/**
 * sheet.js — the Calc window's cell text, without the DOM.
 *
 * Cells are stored the way the formula engine (plugin's `engine.rs`) and the
 * AI read them: numbers with a `.` decimal mark and no grouping (`2.50`,
 * `1000`, `10%`, `€ 5.00`), formulas with `,` between arguments and `.` in
 * numbers. People type and read them in their own language: `2,50`,
 * `1.000`, `=ROUND(B5;1)`, `=B2*1,22`. These helpers convert between the
 * two, format computed values for a cell, move formula references when a
 * cell is pasted elsewhere, and turn a block of cells into tab-separated
 * text and back.
 */

const sepCache = new Map();

/** The decimal mark and grouping separator of a locale. */
export function separators(locale) {
  if (sepCache.has(locale)) return sepCache.get(locale);
  const found = readSeparators(locale);
  sepCache.set(locale, found);
  return found;
}

function readSeparators(locale) {
  try {
    const parts = new Intl.NumberFormat(locale).formatToParts(12345.6);
    return {
      decimal: parts.find((p) => p.type === 'decimal')?.value || '.',
      group: parts.find((p) => p.type === 'group')?.value || ',',
    };
  } catch {
    return { decimal: '.', group: ',' };
  }
}

const CURRENCY = '€$£¥';
// [lead currency][sign][currency][number][%][trail currency], spaces around.
const DECORATED = new RegExp(
  `^(\\s*(?:[${CURRENCY}]\\s*)?)([+-]?)(\\s*(?:[${CURRENCY}]\\s*)?)`
  + `([0-9][0-9.,\\s\\u00a0\\u202f']*[0-9]|[0-9])`
  + `(\\s*%?\\s*(?:[${CURRENCY}])?\\s*)$`,
);

const GROUP_SPACES = /[\s  ']/g;

function isGrouped(int, mark) {
  const parts = int.split(mark);
  return parts.length > 1 && /^\d{1,3}$/.test(parts[0]) && parts.slice(1).every((p) => /^\d{3}$/.test(p));
}

/**
 * The digits of a number as a person typed them, in canonical form
 * (`1234.50`), or null when they do not read as one number. In the locale
 * the locale's decimal mark is the decimal mark; the other mark is grouping
 * when it groups by three (`1.000`), and a lone other mark that does not
 * group (`1.20` typed in Italian) is read as the decimal mark too, which is
 * what the person meant.
 */
export function canonicalNumber(part, seps) {
  const raw = String(part);
  // Spaces (or ') only as thousands grouping: `1 000`, `12 345,5` — a phone
  // number like `333 1234567` stays text.
  if (GROUP_SPACES.test(raw) && !/^\d{1,3}([\s\u00a0\u202f']\d{3})+([.,]\d+)?$/.test(raw)) return null;
  GROUP_SPACES.lastIndex = 0;
  const s = raw.replace(GROUP_SPACES, '');
  if (!/^[0-9.,]+$/.test(s) || !/[0-9]/.test(s)) return null;
  const dec = seps.decimal === ',' ? ',' : '.';
  const other = dec === ',' ? '.' : ',';
  let int = s;
  let frac = null;
  if (s.includes(dec)) {
    const bits = s.split(dec);
    if (bits.length !== 2) return null;
    [int, frac] = bits;
    if (frac.includes(other)) return null;
    if (int.includes(other)) {
      if (!isGrouped(int, other)) return null;
      int = int.split(other).join('');
    }
  } else if (s.includes(other)) {
    if (isGrouped(s, other)) int = s.split(other).join('');
    else {
      const bits = s.split(other);
      if (bits.length !== 2) return null;
      [int, frac] = bits;
    }
  }
  if (!/^\d*$/.test(int) || (frac != null && !/^\d+$/.test(frac))) return null;
  if (!int && frac == null) return null;
  return frac != null ? `${int || '0'}.${frac}` : int;
}

/** A typed value or formula, in the form it is stored in. */
export function toStored(input, seps) {
  const text = String(input ?? '');
  const trimmed = text.trim();
  if (trimmed.startsWith('=')) return formulaToStored(trimmed, seps);
  const m = trimmed.match(DECORATED);
  if (!m) return trimmed;
  const number = canonicalNumber(m[4], seps);
  if (number == null) return trimmed;
  return `${m[1]}${m[2]}${m[3]}${number}${m[5]}`.trim();
}

/** A stored value or formula, as the person edits it in their language. */
export function toEditable(stored, seps) {
  const text = String(stored ?? '');
  if (text.trim().startsWith('=')) return formulaToEditable(text.trim(), seps);
  if (seps.decimal !== ',') return text;
  const m = text.match(DECORATED);
  if (!m || !/^\d+(\.\d+)?$/.test(m[4])) return text;
  return `${m[1]}${m[2]}${m[3]}${m[4].replace('.', ',')}${m[5]}`;
}

/** Walk a formula outside its "strings", calling `visit(i)` per position. */
function mapFormula(src, onNumber, onChar) {
  let out = '';
  let i = 0;
  const isWord = (ch) => /[\p{L}\p{N}_$.]/u.test(ch || '');
  while (i < src.length) {
    const ch = src[i];
    if (ch === '"') {
      const end = (() => {
        let j = i + 1;
        while (j < src.length) {
          if (src[j] === '"') {
            if (src[j + 1] === '"') { j += 2; continue; }
            return j + 1;
          }
          j += 1;
        }
        return src.length;
      })();
      out += src.slice(i, end);
      i = end;
      continue;
    }
    // A word (a ref like B5, a function name like SOMMA.SE): copied whole,
    // so its digits are never read as a number.
    if (/[\p{L}_$]/u.test(ch)) {
      let j = i;
      while (j < src.length && isWord(src[j])) j += 1;
      out += src.slice(i, j);
      i = j;
      continue;
    }
    if (/[0-9]/.test(ch)) {
      const [text, end] = onNumber(src, i);
      out += text;
      i = end;
      continue;
    }
    out += onChar(ch);
    i += 1;
  }
  return out;
}

function formulaToStored(src, seps) {
  const comma = seps.decimal === ',';
  return mapFormula(
    src,
    (s, i) => {
      let j = i;
      while (j < s.length && /[0-9.]/.test(s[j])) j += 1;
      let text = s.slice(i, j);
      // `1,22` in a decimal-comma language is one number.
      if (comma && s[j] === ',' && /[0-9]/.test(s[j + 1] || '') && !text.includes('.')) {
        let k = j + 1;
        while (k < s.length && /[0-9]/.test(s[k])) k += 1;
        text = `${text}.${s.slice(j + 1, k)}`;
        j = k;
      }
      return [text, j];
    },
    (ch) => (ch === ';' ? ',' : ch),
  );
}

function formulaToEditable(src, seps) {
  if (seps.decimal !== ',') return src;
  return mapFormula(
    src,
    (s, i) => {
      let j = i;
      while (j < s.length && /[0-9.]/.test(s[j])) j += 1;
      return [s.slice(i, j).replace('.', ','), j];
    },
    (ch) => (ch === ',' ? ';' : ch),
  );
}

/** Spreadsheet error codes and the catalogue key that explains each. */
export const ERROR_KEYS = {
  '#DIV/0!': 'div0',
  '#VALUE!': 'value',
  '#NAME?': 'name',
  '#REF!': 'ref',
  '#NUM!': 'num',
  '#N/A': 'na',
  '#CIRC!': 'circ',
  '#ERROR!': 'parse',
};

const formatCache = new Map();

function numberFormat(locale, options) {
  const key = `${locale}|${JSON.stringify(options)}`;
  let f = formatCache.get(key);
  if (!f) {
    try { f = new Intl.NumberFormat(locale, options); } catch { f = new Intl.NumberFormat('en', options); }
    formatCache.set(key, f);
  }
  return f;
}

/** A computed number for a cell: grouped, up to 10 decimals, in the locale. */
export function formatNumber(n, locale) {
  if (!Number.isFinite(n)) return '#NUM!';
  const a = Math.abs(n);
  if (a !== 0 && (a >= 1e15 || a < 1e-9)) {
    const [mant, exp] = n.toExponential(6).split('e');
    const m = numberFormat(locale, { maximumFractionDigits: 6 }).format(Number(mant));
    return `${m}E${exp.replace('+', '')}`;
  }
  return numberFormat(locale, { maximumFractionDigits: 10 }).format(n);
}

/**
 * What a cell shows. `value` is the engine's `{ t, v }` (`n` number, `s`
 * text, `b` boolean, `e` error, `z` empty) or undefined before it arrives.
 * A typed number keeps what was typed around it (`€`, `%`) and its number of
 * decimals (`2,50`), grouped in the locale; a formula's number is formatted
 * plainly. `bools` names TRUE and FALSE in the interface language.
 */
export function cellText(raw, value, locale, seps, bools = ['TRUE', 'FALSE']) {
  const text = String(raw ?? '');
  if (!text) return '';
  const formula = text.trim().startsWith('=');
  if (!value) return formula ? '' : localizePlain(text, locale, seps);
  switch (value.t) {
    case 'e': return String(value.v);
    case 'b': return value.v ? bools[0] : bools[1];
    case 's': return String(value.v);
    case 'z': return '';
    case 'n': return formula ? formatNumber(value.v, locale) : localizePlain(text, locale, seps);
    default: return text;
  }
}

/** A typed number in the locale (`1000000` → `1.000.000`, `2.50` → `2,50`). */
export function localizePlain(text, locale, seps) {
  const m = text.match(DECORATED);
  if (!m) return text;
  // Stored values are canonical; older or AI-written ones are read leniently.
  const number = canonicalNumber(m[4], { decimal: '.' }) ?? canonicalNumber(m[4], seps);
  if (number == null) return text;
  const decimals = number.includes('.') ? number.split('.')[1].length : 0;
  // A leading zero (a postcode, a code) is not a quantity: leave it alone.
  if (/^0\d/.test(number)) return text;
  const formatted = numberFormat(locale, {
    minimumFractionDigits: decimals,
    maximumFractionDigits: decimals,
  }).format(Number(number));
  return `${m[1]}${m[2]}${m[3]}${formatted}${m[5]}`;
}

/* ── references ─────────────────────────────────────────────── */

export function colName(col) {
  let n = col + 1;
  let s = '';
  while (n > 0) {
    const rem = (n - 1) % 26;
    s = String.fromCharCode(65 + rem) + s;
    n = Math.floor((n - 1) / 26);
  }
  return s;
}

function colIndex(letters) {
  let col = 0;
  for (const ch of letters.toUpperCase()) col = col * 26 + (ch.charCodeAt(0) - 64);
  return col - 1;
}

/**
 * A formula moved by (dRow, dCol), as when it is pasted elsewhere: relative
 * refs follow, `$`-fixed parts stay. A ref pushed off the sheet is `#REF!`.
 */
export function shiftFormula(formula, dRow, dCol) {
  const src = String(formula);
  if (!src.trim().startsWith('=') || (!dRow && !dCol)) return src;
  let out = '';
  let i = 0;
  while (i < src.length) {
    const ch = src[i];
    if (ch === '"') {
      let j = i + 1;
      while (j < src.length && !(src[j] === '"' && src[j + 1] !== '"')) j += src[j] === '"' ? 2 : 1;
      out += src.slice(i, j + 1);
      i = j + 1;
      continue;
    }
    if (/[\p{L}_$]/u.test(ch)) {
      let j = i;
      while (j < src.length && /[\p{L}\p{N}_$.]/u.test(src[j])) j += 1;
      const word = src.slice(i, j);
      const rest = src.slice(j).trimStart();
      const m = word.match(/^(\$?)([A-Za-z]{1,2})(\$?)([1-9]\d*)$/);
      if (m && !rest.startsWith('(')) {
        const col = m[1] ? colIndex(m[2]) : colIndex(m[2]) + dCol;
        const row = m[3] ? Number(m[4]) : Number(m[4]) + dRow;
        out += col < 0 || row < 1 ? '#REF!' : `${m[1]}${colName(col)}${m[3]}${row}`;
      } else {
        out += word;
      }
      i = j;
      continue;
    }
    out += ch;
    i += 1;
  }
  return out;
}

/* ── clipboard text ─────────────────────────────────────────── */

/** Rows of cell texts as tab-separated lines (quoted where needed). */
export function toTsv(rows) {
  const field = (s) => {
    const v = String(s ?? '');
    return /[\t\n\r"]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v;
  };
  return rows.map((r) => r.map(field).join('\t')).join('\n');
}

/** Tab-separated text (from a spreadsheet, a table, a plain list) as rows. */
export function parseTsv(text) {
  return parseDelimited(String(text ?? ''), '\t');
}

/**
 * Delimited text as rows: quoted fields with `""` for a quote, CRLF or LF
 * line ends; one trailing line end is not an extra empty row.
 */
export function parseDelimited(text, delimiter) {
  const rows = [];
  let row = [];
  let field = '';
  let quoted = false;
  for (let i = 0; i < text.length; i += 1) {
    const ch = text[i];
    if (quoted) {
      if (ch === '"') {
        if (text[i + 1] === '"') { field += '"'; i += 1; } else quoted = false;
      } else field += ch;
    } else if (ch === '"' && field === '') {
      quoted = true;
    } else if (ch === delimiter) {
      row.push(field);
      field = '';
    } else if (ch === '\n' || ch === '\r') {
      if (ch === '\r' && text[i + 1] === '\n') i += 1;
      row.push(field);
      rows.push(row);
      row = [];
      field = '';
    } else {
      field += ch;
    }
  }
  if (field !== '' || row.length) {
    row.push(field);
    rows.push(row);
  }
  return rows;
}

/**
 * The delimiter of a CSV file: `;` when the first line has more of them
 * than commas outside quotes (Italian, German, French spreadsheets export
 * that way, with a decimal comma), else `,`.
 */
export function csvDelimiter(text) {
  const line = String(text ?? '').split(/\r?\n/, 1)[0] || '';
  let quoted = false;
  let commas = 0;
  let semis = 0;
  for (const ch of line) {
    if (ch === '"') quoted = !quoted;
    else if (!quoted && ch === ',') commas += 1;
    else if (!quoted && ch === ';') semis += 1;
  }
  return semis > commas ? ';' : ',';
}
