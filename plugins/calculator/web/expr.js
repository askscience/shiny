/**
 * expr.js — the Calculator window's expression text, without the DOM.
 *
 * The window keeps what was typed as plain evaluator text (`* / -`, `pi`);
 * these helpers turn it into what the screen shows, fix it up before it is
 * sent, flip the sign of the number being typed, and name an evaluator error
 * so the window can say it in the interface language.
 */

/**
 * An expression as the keys show it: × ÷ − instead of * / -, π for pi,
 * √ and ∛ for sqrt and cbrt.
 * Display only; what is evaluated and stored keeps the plain characters.
 */
export function prettyExpr(src) {
  return String(src ?? '')
    .replace(/\*/g, '×')
    .replace(/\//g, '÷')
    .replace(/-/g, '−')
    .replace(/pi/g, 'π')
    .replace(/sqrt\(/g, '√(')
    .replace(/cbrt\(/g, '∛(');
}

/** Close every bracket left open: `sin(30` → `sin(30)`, `(2+3` → `(2+3)`. */
export function closeParens(src) {
  let open = 0;
  for (const ch of String(src ?? '')) {
    if (ch === '(') open += 1;
    else if (ch === ')' && open > 0) open -= 1;
  }
  return src + ')'.repeat(open);
}

/** Index of the `(` matching the `)` at `close`, or -1. */
function openingOf(src, close) {
  let depth = 0;
  for (let i = close; i >= 0; i -= 1) {
    if (src[i] === ')') depth += 1;
    else if (src[i] === '(') {
      depth -= 1;
      if (depth === 0) return i;
    }
  }
  return -1;
}

/** A unary minus can stand at `i`: the start, or after an operator or `(`. */
const unaryAt = (src, i) => i === 0 || /[+\-*/^%(]/.test(src[i - 1]);

const NUMBER_TAIL = /(\d*\.?\d+(?:e[+-]?\d+)?|\d+\.|pi|e)$/;

/**
 * Flip the sign of the last number (or bracket) in `src`, the way ± works on
 * a pocket calculator: `5` → `-5`, `5+3` → `5+(-3)`, and back again. An
 * expression that ends in an operator gets a minus to start the next number.
 */
export function toggleSign(src) {
  const s = String(src ?? '');
  // `(-3)` at the end → `3`.
  const wrapped = s.match(/\(-(\d*\.?\d+(?:e[+-]?\d+)?|pi|e)\)$/);
  if (wrapped) return s.slice(0, wrapped.index) + wrapped[1];

  if (s.endsWith(')')) {
    const open = openingOf(s, s.length - 1);
    if (open < 0) return `${s}-`;
    const group = s.slice(open);
    // `-(2+3)` (a unary minus right before the bracket) → `(2+3)`.
    if (open > 0 && s[open - 1] === '-' && unaryAt(s, open - 1)) {
      return s.slice(0, open - 1) + group;
    }
    // `(-(2+3))` → `(2+3)`.
    const inner = group.slice(1, -1);
    if (inner.startsWith('-(') && openingOf(inner, inner.length - 1) === 1) {
      return s.slice(0, open) + inner.slice(1);
    }
    if (unaryAt(s, open) && (open === 0 || s[open - 1] === '(')) {
      return `${s.slice(0, open)}-${group}`;
    }
    return `${s.slice(0, open)}(-${group})`;
  }

  const tail = s.match(NUMBER_TAIL);
  if (!tail) return `${s}-`;
  const at = tail.index;
  // A unary minus right before it (`-5`, `(-5`, `2*-5`) → drop the minus.
  if (at > 0 && s[at - 1] === '-' && unaryAt(s, at - 1)) {
    return s.slice(0, at - 1) + tail[1];
  }
  if (at === 0) return `-${s}`;
  if (s[at - 1] === '(') return `${s.slice(0, at)}-${tail[1]}`;
  return `${s.slice(0, at)}(-${tail[1]})`;
}

/** The negation of a result's text: `8` → `-8`, `-8` → `8`. */
export function negateText(text) {
  const s = String(text ?? '');
  return s.startsWith('-') ? s.slice(1) : `-${s}`;
}

/**
 * Which kind of mistake an evaluator error message describes, as the last
 * part of a `plugin.calculator.error.*` key. The evaluator speaks English to
 * the AI; the window says it in the user's language.
 */
export function errorKind(message) {
  const m = String(message ?? '').toLowerCase();
  if (/division by zero|modulo by zero/.test(m)) return 'divideByZero';
  if (/too long|too many tokens|nested too deeply/.test(m)) return 'tooLong';
  if (/too large|not a finite/.test(m)) return 'tooLarge';
  if (/unexpected end|expected '\)'|expected ','/.test(m)) return 'incomplete';
  if (/non-positive|negative number|log base|not defined|non-negative integer/.test(m)) return 'domain';
  return 'invalid';
}
