//! The spreadsheet formula engine: one authority for the Calc window, the AI
//! tools and the `.ods` export.
//!
//! A sheet is a map `"A1" -> text`. A text starting with `=` is a formula;
//! anything else is a value, read the way a person typed it (`2.50`, `10%`,
//! `€ 5`, `1,234.5`). `evaluate_sheet` computes every cell at once: formulas
//! are parsed once, ordered by their dependencies, and evaluated in that
//! order, so a long chain of references (`B2=B1+A2` down 500 rows) never
//! recurses, and a loop of references is reported as `#CIRC!` instead of a
//! number.
//!
//! Formulas read like a pocket spreadsheet: `+ - * / ^`, `&` (join text),
//! `= <> < > <= >=`, a trailing `%`, cell refs (`A1`, `$A$1`), ranges
//! (`A1:B3`), `"text"`, `TRUE` / `FALSE`, and functions (see `call`). Both
//! `,` and `;` separate arguments, and the common Italian names (`SOMMA`,
//! `SE`, `ARROTONDA`, …) are read as their English ones.
//!
//! Errors are values, as in every spreadsheet: `#DIV/0!`, `#VALUE!`,
//! `#NAME?`, `#REF!`, `#NUM!`, `#N/A`, `#CIRC!`, `#ERROR!` (cannot be read).

use std::collections::{BTreeMap, HashMap};

/* ── values ─────────────────────────────────────────────────── */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Div0,
    Value,
    Name,
    Ref,
    Num,
    NA,
    Circ,
    Parse,
}

impl ErrorKind {
    pub fn code(self) -> &'static str {
        match self {
            ErrorKind::Div0 => "#DIV/0!",
            ErrorKind::Value => "#VALUE!",
            ErrorKind::Name => "#NAME?",
            ErrorKind::Ref => "#REF!",
            ErrorKind::Num => "#NUM!",
            ErrorKind::NA => "#N/A",
            ErrorKind::Circ => "#CIRC!",
            ErrorKind::Parse => "#ERROR!",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Empty,
    Number(f64),
    Text(String),
    Bool(bool),
    Error(ErrorKind),
}

impl Value {
    /// The value as a spreadsheet shows it in text (`&`, `CONCAT`, exports).
    pub fn to_text(&self) -> String {
        match self {
            Value::Empty => String::new(),
            Value::Number(n) => format_number(*n),
            Value::Text(s) => s.clone(),
            Value::Bool(b) => if *b { "TRUE".into() } else { "FALSE".into() },
            Value::Error(e) => e.code().into(),
        }
    }
}

/// Drop binary floating-point noise: 15 significant digits, so `0.1+0.2` is
/// 0.3 and `1.2*3` is 3.6, as a spreadsheet shows them.
pub fn clean(n: f64) -> f64 {
    if n == 0.0 || !n.is_finite() {
        return if n == 0.0 { 0.0 } else { n };
    }
    format!("{n:.14e}").parse().unwrap_or(n)
}

/// A number as plain canonical text: `3.6`, `1000`, `1e+21`.
pub fn format_number(n: f64) -> String {
    let n = clean(n);
    if n == n.trunc() && n.abs() < 1e15 {
        return format!("{}", n as i64);
    }
    format!("{n}")
}

/// Read a typed value: a number (with a sign, `%`, a currency symbol, and
/// `.` or `,` as either the decimal mark or thousands grouping), `TRUE` /
/// `FALSE`, or text. The window stores numbers with a `.` decimal mark;
/// AI-written and older values may carry a `,`, read leniently: `1,250` with
/// groups of three is a thousand-two-hundred-fifty, `2,50` is two and a half.
pub fn parse_value(raw: &str) -> Value {
    let s = raw.trim();
    if s.is_empty() {
        return Value::Empty;
    }
    match s.to_ascii_uppercase().as_str() {
        "TRUE" | "VERO" => return Value::Bool(true),
        "FALSE" | "FALSO" => return Value::Bool(false),
        _ => {}
    }
    match parse_number_text(s) {
        Some(n) => Value::Number(n),
        None => Value::Text(raw.to_string()),
    }
}

const CURRENCY: &[char] = &['€', '$', '£', '¥'];

fn parse_number_text(s: &str) -> Option<f64> {
    let mut t = s.trim();
    // One currency symbol, before or after the number.
    if let Some(rest) = t.strip_prefix(CURRENCY) {
        t = rest.trim_start();
    } else if let Some(rest) = t.strip_suffix(CURRENCY) {
        t = rest.trim_end();
    }
    let mut percent = false;
    if let Some(rest) = t.strip_suffix('%') {
        percent = true;
        t = rest.trim_end();
    }
    let (sign, body) = match t.as_bytes().first() {
        Some(b'-') => (-1.0, &t[1..]),
        Some(b'+') => (1.0, &t[1..]),
        _ => (1.0, t),
    };
    // A currency symbol may also sit after the sign: `-€5`.
    let body = body.strip_prefix(CURRENCY).unwrap_or(body).trim_start();
    if body.is_empty() || !body.chars().all(|c| c.is_ascii_digit() || c == '.' || c == ',') {
        return plain_float(body).map(|n| finish(sign * n, percent));
    }
    if !body.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let canonical = canonical_digits(body)?;
    let n: f64 = canonical.parse().ok()?;
    Some(finish(sign * n, percent))
}

/// Scientific notation and the like (`1e3`), with no grouping.
fn plain_float(body: &str) -> Option<f64> {
    if body.is_empty() || !body.chars().next()?.is_ascii_digit() && !body.starts_with('.') {
        return None;
    }
    if !body.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-')) {
        return None;
    }
    body.parse::<f64>().ok().filter(|n| n.is_finite())
}

fn finish(n: f64, percent: bool) -> f64 {
    if percent { clean(n / 100.0) } else { n }
}

/// Digits with `.` and `,` to a Rust float literal. Both present: the last
/// one is the decimal mark. One of them: groups of exactly three after it,
/// more than once or with a leading 1–3 digits, mean grouping only when that
/// mark is `,`; a `.` is always decimal here (the window already turned an
/// Italian `1.000` into `1000`).
fn canonical_digits(body: &str) -> Option<String> {
    let has_dot = body.contains('.');
    let has_comma = body.contains(',');
    let (group, decimal) = match (has_dot, has_comma) {
        (true, true) => {
            if body.rfind('.') > body.rfind(',') { (',', '.') } else { ('.', ',') }
        }
        (false, true) => {
            if is_grouped(body, ',') { (',', '.') } else { ('\0', ',') }
        }
        _ => ('\0', '.'),
    };
    if body.matches(decimal).count() > 1 {
        return None;
    }
    if group != '\0' && !is_grouped(body.split(decimal).next().unwrap_or(""), group) {
        return None;
    }
    let mut out = String::with_capacity(body.len());
    for c in body.chars() {
        if c == group {
            continue;
        }
        out.push(if c == decimal { '.' } else { c });
    }
    if out.starts_with('.') {
        out.insert(0, '0');
    }
    Some(out)
}

/// `1,234,567` / `12,345`: a 1–3 digit head, then groups of three.
fn is_grouped(int_part: &str, mark: char) -> bool {
    let parts: Vec<&str> = int_part.split(mark).collect();
    parts.len() > 1
        && (1..=3).contains(&parts[0].len())
        && parts[0].chars().all(|c| c.is_ascii_digit())
        && parts[1..].iter().all(|p| p.len() == 3 && p.chars().all(|c| c.is_ascii_digit()))
}

/* ── cell references ────────────────────────────────────────── */

/// A cell position: 1-based row, 0-based column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Pos {
    pub row: u32,
    pub col: u32,
}

/// `A1`, `$B$7`, `ab12` → a position (columns A–ZZ, rows 1–99999).
pub fn parse_ref(text: &str) -> Option<Pos> {
    let s = text.replace('$', "");
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut col: u32 = 0;
    while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
        if i == 2 {
            return None; // columns stop at ZZ
        }
        col = col * 26 + (bytes[i].to_ascii_uppercase() - b'A' + 1) as u32;
        i += 1;
    }
    if i == 0 || i >= bytes.len() {
        return None;
    }
    let digits = &s[i..];
    if digits.len() > 5 || !digits.chars().all(|c| c.is_ascii_digit()) || digits.starts_with('0') {
        return None;
    }
    Some(Pos { row: digits.parse().ok()?, col: col - 1 })
}

pub fn ref_name(p: Pos) -> String {
    let mut n = p.col + 1;
    let mut s = String::new();
    while n > 0 {
        let rem = (n - 1) % 26;
        s.insert(0, (b'A' + rem as u8) as char);
        n = (n - 1) / 26;
    }
    format!("{s}{}", p.row)
}

/* ── tokens ─────────────────────────────────────────────────── */

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Word(String),
    Op(&'static str),
}

const MAX_FORMULA_LEN: usize = 8 * 1024;

fn tokenize(src: &str) -> Result<Vec<Tok>, ErrorKind> {
    if src.len() > MAX_FORMULA_LEN {
        return Err(ErrorKind::Parse);
    }
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            // Scientific notation: 1.5e3, 2E-4.
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let mut j = i + 1;
                if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                    j += 1;
                }
                if j < chars.len() && chars[j].is_ascii_digit() {
                    while j < chars.len() && chars[j].is_ascii_digit() {
                        j += 1;
                    }
                    i = j;
                }
            }
            let text: String = chars[start..i].iter().collect();
            out.push(Tok::Num(text.parse().map_err(|_| ErrorKind::Parse)?));
            continue;
        }
        if c == '"' {
            // "text", with "" for a quote inside.
            let mut s = String::new();
            i += 1;
            loop {
                match chars.get(i) {
                    None => return Err(ErrorKind::Parse),
                    Some('"') if chars.get(i + 1) == Some(&'"') => {
                        s.push('"');
                        i += 2;
                    }
                    Some('"') => {
                        i += 1;
                        break;
                    }
                    Some(ch) => {
                        s.push(*ch);
                        i += 1;
                    }
                }
            }
            out.push(Tok::Str(s));
            continue;
        }
        if c.is_alphabetic() || c == '_' || c == '$' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '.' | '$')) {
                i += 1;
            }
            out.push(Tok::Word(chars[start..i].iter().collect()));
            continue;
        }
        let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
        let op: &'static str = match two.as_str() {
            "<=" => "<=",
            ">=" => ">=",
            "<>" => "<>",
            _ => match c {
                '+' => "+",
                '-' => "-",
                '*' => "*",
                '/' => "/",
                '^' => "^",
                '&' => "&",
                '=' => "=",
                '<' => "<",
                '>' => ">",
                '(' => "(",
                ')' => ")",
                ',' | ';' => ",",
                ':' => ":",
                '%' => "%",
                _ => return Err(ErrorKind::Parse),
            },
        };
        i += op.len();
        out.push(Tok::Op(op));
    }
    Ok(out)
}

/* ── syntax tree ────────────────────────────────────────────── */

#[derive(Debug, Clone)]
enum Expr {
    Num(f64),
    Str(String),
    Bool(bool),
    Ref(Pos),
    Range(Pos, Pos),
    Neg(Box<Expr>),
    Percent(Box<Expr>),
    Bin(&'static str, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    Error(ErrorKind),
}

const MAX_DEPTH: u32 = 96;

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    depth: u32,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn peek_op(&self, op: &str) -> bool {
        matches!(self.peek(), Some(Tok::Op(o)) if *o == op)
    }
    fn bump(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }
    fn enter(&mut self) -> Result<(), ErrorKind> {
        self.depth += 1;
        if self.depth > MAX_DEPTH { Err(ErrorKind::Parse) } else { Ok(()) }
    }

    fn expr(&mut self) -> Result<Expr, ErrorKind> {
        self.enter()?;
        let r = self.comparison();
        self.depth -= 1;
        r
    }

    fn comparison(&mut self) -> Result<Expr, ErrorKind> {
        let mut left = self.concat()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Op(o)) if matches!(*o, "=" | "<>" | "<" | ">" | "<=" | ">=") => *o,
                _ => break,
            };
            self.bump();
            let right = self.concat()?;
            left = Expr::Bin(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn concat(&mut self) -> Result<Expr, ErrorKind> {
        let mut left = self.additive()?;
        while self.peek_op("&") {
            self.bump();
            let right = self.additive()?;
            left = Expr::Bin("&", Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn additive(&mut self) -> Result<Expr, ErrorKind> {
        let mut left = self.term()?;
        loop {
            let op = if self.peek_op("+") { "+" } else if self.peek_op("-") { "-" } else { break };
            self.bump();
            let right = self.term()?;
            left = Expr::Bin(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn term(&mut self) -> Result<Expr, ErrorKind> {
        let mut left = self.power()?;
        loop {
            let op = if self.peek_op("*") { "*" } else if self.peek_op("/") { "/" } else { break };
            self.bump();
            let right = self.power()?;
            left = Expr::Bin(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn power(&mut self) -> Result<Expr, ErrorKind> {
        let mut left = self.unary()?;
        while self.peek_op("^") {
            self.bump();
            let right = self.unary()?;
            left = Expr::Bin("^", Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// As in Excel and LibreOffice, a sign binds tighter than `^`: `-2^2` is 4.
    fn unary(&mut self) -> Result<Expr, ErrorKind> {
        if self.peek_op("-") {
            self.bump();
            self.enter()?;
            let inner = self.unary();
            self.depth -= 1;
            return Ok(Expr::Neg(Box::new(inner?)));
        }
        if self.peek_op("+") {
            self.bump();
            self.enter()?;
            let inner = self.unary();
            self.depth -= 1;
            return inner;
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Result<Expr, ErrorKind> {
        let mut e = self.primary()?;
        while self.peek_op("%") {
            self.bump();
            e = Expr::Percent(Box::new(e));
        }
        Ok(e)
    }

    fn primary(&mut self) -> Result<Expr, ErrorKind> {
        match self.bump() {
            Some(Tok::Num(n)) => Ok(Expr::Num(n)),
            Some(Tok::Str(s)) => Ok(Expr::Str(s)),
            Some(Tok::Op("(")) => {
                let e = self.expr()?;
                if !self.peek_op(")") {
                    return Err(ErrorKind::Parse);
                }
                self.bump();
                Ok(e)
            }
            Some(Tok::Word(w)) => {
                if self.peek_op("(") {
                    self.bump();
                    let mut args = Vec::new();
                    if self.peek_op(")") {
                        self.bump();
                    } else {
                        loop {
                            // An empty argument (`IF(A1;;0)`) is an empty value.
                            if self.peek_op(",") || self.peek_op(")") {
                                args.push(Expr::Str(String::new()));
                            } else {
                                args.push(self.expr()?);
                            }
                            match self.bump() {
                                Some(Tok::Op(",")) => continue,
                                Some(Tok::Op(")")) => break,
                                _ => return Err(ErrorKind::Parse),
                            }
                        }
                    }
                    return Ok(Expr::Call(canonical_name(&w), args));
                }
                if let Some(a) = parse_ref(&w) {
                    if self.peek_op(":") {
                        self.bump();
                        return match self.bump() {
                            Some(Tok::Word(w2)) => match parse_ref(&w2) {
                                Some(b) => Ok(Expr::Range(a, b)),
                                None => Ok(Expr::Error(ErrorKind::Ref)),
                            },
                            _ => Err(ErrorKind::Parse),
                        };
                    }
                    return Ok(Expr::Ref(a));
                }
                match w.to_uppercase().as_str() {
                    "TRUE" | "VERO" => Ok(Expr::Bool(true)),
                    "FALSE" | "FALSO" => Ok(Expr::Bool(false)),
                    _ => Ok(Expr::Error(ErrorKind::Name)),
                }
            }
            _ => Err(ErrorKind::Parse),
        }
    }
}

fn parse_formula(src: &str) -> Result<Expr, ErrorKind> {
    let toks = tokenize(src)?;
    if toks.is_empty() {
        return Err(ErrorKind::Parse);
    }
    let mut p = Parser { toks, pos: 0, depth: 0 };
    let e = p.expr()?;
    if p.pos != p.toks.len() {
        return Err(ErrorKind::Parse);
    }
    Ok(e)
}

/// The English name of a function, for the Italian names people type.
fn canonical_name(name: &str) -> String {
    let up = name.to_uppercase();
    let english = match up.as_str() {
        "SOMMA" => "SUM",
        "MEDIA" => "AVERAGE",
        "CONTA.NUMERI" => "COUNT",
        "CONTA.VALORI" => "COUNTA",
        "PRODOTTO" => "PRODUCT",
        "MEDIANA" => "MEDIAN",
        "ARROTONDA" => "ROUND",
        "ARROTONDA.PER.ECC" => "ROUNDUP",
        "ARROTONDA.PER.DIF" => "ROUNDDOWN",
        "ASS" => "ABS",
        "RADQ" => "SQRT",
        "POTENZA" => "POWER",
        "RESTO" => "MOD",
        "SE" => "IF",
        "SE.ERRORE" => "IFERROR",
        "E" => "AND",
        "O" => "OR",
        "NON" => "NOT",
        "SOMMA.SE" => "SUMIF",
        "CONTA.SE" => "COUNTIF",
        "MEDIA.SE" => "AVERAGEIF",
        "CONCATENA" => "CONCATENATE",
        "LUNGHEZZA" => "LEN",
        "MAIUSC" => "UPPER",
        "MINUSC" => "LOWER",
        "ANNULLA.SPAZI" => "TRIM",
        "SINISTRA" => "LEFT",
        "DESTRA" => "RIGHT",
        other => other,
    };
    english.to_string()
}

/// The functions `call` knows, by English name.
pub const FUNCTIONS: &[&str] = &[
    "SUM", "AVERAGE", "MIN", "MAX", "COUNT", "COUNTA", "PRODUCT", "MEDIAN",
    "ROUND", "ROUNDUP", "ROUNDDOWN", "INT", "ABS", "SQRT", "POWER", "MOD",
    "IF", "IFERROR", "AND", "OR", "NOT", "SUMIF", "COUNTIF", "AVERAGEIF",
    "CONCAT", "CONCATENATE", "LEN", "UPPER", "LOWER", "TRIM", "LEFT", "RIGHT",
];

/// A formula as LibreOffice and Excel read it: English function names, `,`
/// between arguments, refs untouched. Used for the `.ods` export.
pub fn canonical_formula(formula: &str) -> String {
    let body = formula.strip_prefix('=').unwrap_or(formula);
    let chars: Vec<char> = body.chars().collect();
    let mut out = String::with_capacity(body.len() + 8);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            out.push(c);
            i += 1;
            while i < chars.len() {
                out.push(chars[i]);
                if chars[i] == '"' {
                    if chars.get(i + 1) == Some(&'"') {
                        out.push('"');
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if c.is_alphabetic() || c == '_' || c == '$' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '.' | '$')) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let next = chars[i..].iter().find(|c| !c.is_whitespace());
            if next == Some(&'(') {
                out.push_str(&canonical_name(&word));
            } else {
                out.push_str(&word);
            }
            continue;
        }
        out.push(if c == ';' { ',' } else { c });
        i += 1;
    }
    format!("={out}")
}

/// Every cell a formula reads: single refs, and the corners of ranges.
fn collect_refs(e: &Expr, singles: &mut Vec<Pos>, ranges: &mut Vec<(Pos, Pos)>) {
    match e {
        Expr::Ref(p) => singles.push(*p),
        Expr::Range(a, b) => ranges.push((*a, *b)),
        Expr::Neg(x) | Expr::Percent(x) => collect_refs(x, singles, ranges),
        Expr::Bin(_, a, b) => {
            collect_refs(a, singles, ranges);
            collect_refs(b, singles, ranges);
        }
        Expr::Call(_, args) => args.iter().for_each(|a| collect_refs(a, singles, ranges)),
        _ => {}
    }
}

fn in_range(p: Pos, a: Pos, b: Pos) -> bool {
    let (r1, r2) = (a.row.min(b.row), a.row.max(b.row));
    let (c1, c2) = (a.col.min(b.col), a.col.max(b.col));
    (r1..=r2).contains(&p.row) && (c1..=c2).contains(&p.col)
}

/* ── evaluation ─────────────────────────────────────────────── */

/// Compute every non-empty cell of a sheet.
pub fn evaluate_sheet(cells: &BTreeMap<String, String>) -> BTreeMap<String, Value> {
    let mut plain: HashMap<Pos, Value> = HashMap::new();
    let mut formulas: Vec<(Pos, String, Result<Expr, ErrorKind>)> = Vec::new();
    for (name, raw) in cells {
        let Some(pos) = parse_ref(name) else { continue };
        let text = raw.trim();
        if let Some(src) = text.strip_prefix('=') {
            formulas.push((pos, name.clone(), parse_formula(src)));
        } else {
            plain.insert(pos, parse_value(raw));
        }
    }

    // Who each formula depends on (formula cells only: plain values are known).
    let index: HashMap<Pos, usize> = formulas.iter().enumerate().map(|(i, f)| (f.0, i)).collect();
    let formula_positions: Vec<Pos> = formulas.iter().map(|f| f.0).collect();
    let mut deps: Vec<Vec<usize>> = vec![Vec::new(); formulas.len()];
    for (i, (_, _, parsed)) in formulas.iter().enumerate() {
        let Ok(expr) = parsed else { continue };
        let (mut singles, mut ranges) = (Vec::new(), Vec::new());
        collect_refs(expr, &mut singles, &mut ranges);
        let mut d: Vec<usize> = singles.iter().filter_map(|p| index.get(p).copied()).collect();
        for (a, b) in ranges {
            d.extend(formula_positions.iter().enumerate().filter(|(_, p)| in_range(**p, a, b)).map(|(j, _)| j));
        }
        d.sort_unstable();
        d.dedup();
        deps[i] = d;
    }

    // Kahn's order: a formula runs after every formula it reads.
    let mut readers: Vec<Vec<usize>> = vec![Vec::new(); formulas.len()];
    let mut waiting: Vec<usize> = vec![0; formulas.len()];
    for (i, d) in deps.iter().enumerate() {
        waiting[i] = d.len();
        for &j in d {
            readers[j].push(i);
        }
    }
    let mut ready: Vec<usize> = (0..formulas.len()).filter(|&i| waiting[i] == 0).collect();
    let mut order = Vec::with_capacity(formulas.len());
    while let Some(i) = ready.pop() {
        order.push(i);
        for &r in &readers[i] {
            waiting[r] -= 1;
            if waiting[r] == 0 {
                ready.push(r);
            }
        }
    }

    let mut values: HashMap<Pos, Value> = plain;
    // What is left over reads itself, directly or around a loop.
    let ordered: std::collections::HashSet<usize> = order.iter().copied().collect();
    for (i, f) in formulas.iter().enumerate() {
        if !ordered.contains(&i) {
            values.insert(f.0, Value::Error(ErrorKind::Circ));
        }
    }
    for i in order {
        let (pos, _, parsed) = &formulas[i];
        let v = match parsed {
            Ok(expr) => {
                let ev = Eval { values: &values };
                match ev.eval(expr) {
                    Arg::Scalar(v) => finish_value(v),
                    // `=A1:A3` on its own: a range is not one value.
                    Arg::Range(_) => Value::Error(ErrorKind::Value),
                }
            }
            Err(e) => Value::Error(*e),
        };
        values.insert(*pos, v);
    }

    formulas
        .iter()
        .map(|f| f.0)
        .chain(cells.keys().filter_map(|k| parse_ref(k)))
        .map(|p| (ref_name(p), values.get(&p).cloned().unwrap_or(Value::Empty)))
        .collect()
}

/// A formula's result as stored: numbers cleaned, a reference to an empty
/// cell is 0.
fn finish_value(v: Value) -> Value {
    match v {
        Value::Number(n) if !n.is_finite() => Value::Error(ErrorKind::Num),
        Value::Number(n) => Value::Number(clean(n)),
        Value::Empty => Value::Number(0.0),
        other => other,
    }
}

enum Arg {
    Scalar(Value),
    /// The values of a range, row by row (empty cells included).
    Range(Vec<Value>),
}

struct Eval<'a> {
    values: &'a HashMap<Pos, Value>,
}

/// Biggest range a formula may read, in cells (the sheet is at most 500×52).
const MAX_RANGE_CELLS: u64 = 26_000;

impl Eval<'_> {
    fn cell(&self, p: Pos) -> Value {
        self.values.get(&p).cloned().unwrap_or(Value::Empty)
    }

    fn eval(&self, e: &Expr) -> Arg {
        match e {
            Expr::Range(a, b) => {
                let (r1, r2) = (a.row.min(b.row), a.row.max(b.row));
                let (c1, c2) = (a.col.min(b.col), a.col.max(b.col));
                let count = (r2 - r1 + 1) as u64 * (c2 - c1 + 1) as u64;
                if count > MAX_RANGE_CELLS {
                    return Arg::Scalar(Value::Error(ErrorKind::Ref));
                }
                let mut out = Vec::with_capacity(count as usize);
                for r in r1..=r2 {
                    for c in c1..=c2 {
                        out.push(self.cell(Pos { row: r, col: c }));
                    }
                }
                Arg::Range(out)
            }
            other => Arg::Scalar(self.scalar(other)),
        }
    }

    fn scalar(&self, e: &Expr) -> Value {
        match e {
            Expr::Num(n) => Value::Number(*n),
            Expr::Str(s) => Value::Text(s.clone()),
            Expr::Bool(b) => Value::Bool(*b),
            Expr::Error(k) => Value::Error(*k),
            Expr::Ref(p) => self.cell(*p),
            Expr::Range(..) => Value::Error(ErrorKind::Value),
            Expr::Neg(x) => match num(&self.scalar(x)) {
                Ok(n) => Value::Number(-n),
                Err(k) => Value::Error(k),
            },
            Expr::Percent(x) => match num(&self.scalar(x)) {
                Ok(n) => Value::Number(n / 100.0),
                Err(k) => Value::Error(k),
            },
            Expr::Bin(op, a, b) => self.binary(op, &self.scalar(a), &self.scalar(b)),
            Expr::Call(name, args) => self.call(name, args),
        }
    }

    fn binary(&self, op: &str, a: &Value, b: &Value) -> Value {
        if let Value::Error(k) = a {
            return Value::Error(*k);
        }
        if let Value::Error(k) = b {
            return Value::Error(*k);
        }
        match op {
            "&" => Value::Text(a.to_text() + &b.to_text()),
            "=" | "<>" | "<" | ">" | "<=" | ">=" => {
                let ord = compare(a, b);
                Value::Bool(match op {
                    "=" => ord == std::cmp::Ordering::Equal,
                    "<>" => ord != std::cmp::Ordering::Equal,
                    "<" => ord == std::cmp::Ordering::Less,
                    ">" => ord == std::cmp::Ordering::Greater,
                    "<=" => ord != std::cmp::Ordering::Greater,
                    _ => ord != std::cmp::Ordering::Less,
                })
            }
            _ => {
                let (x, y) = match (num(a), num(b)) {
                    (Ok(x), Ok(y)) => (x, y),
                    (Err(k), _) | (_, Err(k)) => return Value::Error(k),
                };
                let r = match op {
                    "+" => x + y,
                    "-" => x - y,
                    "*" => x * y,
                    "/" => {
                        if y == 0.0 {
                            return Value::Error(ErrorKind::Div0);
                        }
                        x / y
                    }
                    _ => x.powf(y),
                };
                if r.is_finite() { Value::Number(r) } else { Value::Error(ErrorKind::Num) }
            }
        }
    }

    fn args(&self, args: &[Expr]) -> Vec<Arg> {
        args.iter().map(|a| self.eval(a)).collect()
    }

    fn call(&self, name: &str, raw: &[Expr]) -> Value {
        // IF and IFERROR only look at the branch they take.
        match name {
            "IF" => {
                if raw.len() < 2 || raw.len() > 3 {
                    return Value::Error(ErrorKind::Value);
                }
                let cond = match truth(&self.scalar(&raw[0])) {
                    Ok(b) => b,
                    Err(k) => return Value::Error(k),
                };
                return if cond {
                    self.scalar(&raw[1])
                } else if raw.len() == 3 {
                    self.scalar(&raw[2])
                } else {
                    Value::Bool(false)
                };
            }
            "IFERROR" => {
                if raw.len() != 2 {
                    return Value::Error(ErrorKind::Value);
                }
                return match self.scalar(&raw[0]) {
                    Value::Error(_) => self.scalar(&raw[1]),
                    v => v,
                };
            }
            _ => {}
        }
        let args = self.args(raw);
        match name {
            "SUM" | "AVERAGE" | "MIN" | "MAX" | "COUNT" | "PRODUCT" | "MEDIAN" => {
                let nums = match numbers(&args) {
                    Ok(n) => n,
                    Err(k) => return Value::Error(k),
                };
                aggregate(name, &nums)
            }
            "COUNTA" => Value::Number(
                args.iter()
                    .map(|a| match a {
                        Arg::Scalar(v) => usize::from(*v != Value::Empty),
                        Arg::Range(vs) => vs.iter().filter(|v| **v != Value::Empty).count(),
                    })
                    .sum::<usize>() as f64,
            ),
            "ROUND" | "ROUNDUP" | "ROUNDDOWN" => {
                if args.is_empty() || args.len() > 2 {
                    return Value::Error(ErrorKind::Value);
                }
                let (x, digits) = match (scalar_num(&args, 0), opt_num(&args, 1, 0.0)) {
                    (Ok(x), Ok(d)) => (x, d.trunc()),
                    (Err(k), _) | (_, Err(k)) => return Value::Error(k),
                };
                let f = 10f64.powf(digits);
                let scaled = clean(x * f);
                let r = match name {
                    "ROUND" => scaled.abs().round().copysign(scaled),
                    "ROUNDUP" => scaled.abs().ceil().copysign(scaled),
                    _ => scaled.abs().floor().copysign(scaled),
                };
                Value::Number(r / f)
            }
            "INT" => one(&args, |x| Ok(x.floor())),
            "ABS" => one(&args, |x| Ok(x.abs())),
            "SQRT" => one(&args, |x| if x < 0.0 { Err(ErrorKind::Num) } else { Ok(x.sqrt()) }),
            "POWER" => two(&args, |x, y| Ok(x.powf(y))),
            "MOD" => two(&args, |x, y| {
                if y == 0.0 {
                    Err(ErrorKind::Div0)
                } else {
                    // The sign follows the divisor, as in spreadsheets.
                    Ok(x - y * (x / y).floor())
                }
            }),
            "AND" | "OR" => {
                let mut seen = false;
                let mut acc = name == "AND";
                for a in &args {
                    let vals: Vec<&Value> = match a {
                        Arg::Scalar(v) => vec![v],
                        Arg::Range(vs) => vs.iter().filter(|v| matches!(v, Value::Number(_) | Value::Bool(_) | Value::Error(_))).collect(),
                    };
                    for v in vals {
                        match truth(v) {
                            Ok(b) => {
                                seen = true;
                                acc = if name == "AND" { acc && b } else { acc || b };
                            }
                            Err(k) => return Value::Error(k),
                        }
                    }
                }
                if seen { Value::Bool(acc) } else { Value::Error(ErrorKind::Value) }
            }
            "NOT" => match args.as_slice() {
                [Arg::Scalar(v)] => match truth(v) {
                    Ok(b) => Value::Bool(!b),
                    Err(k) => Value::Error(k),
                },
                _ => Value::Error(ErrorKind::Value),
            },
            "SUMIF" | "COUNTIF" | "AVERAGEIF" => self.conditional(name, &args),
            "CONCAT" | "CONCATENATE" => {
                let mut s = String::new();
                for a in &args {
                    match a {
                        Arg::Scalar(Value::Error(k)) => return Value::Error(*k),
                        Arg::Scalar(v) => s.push_str(&v.to_text()),
                        Arg::Range(vs) => vs.iter().for_each(|v| s.push_str(&v.to_text())),
                    }
                }
                Value::Text(s)
            }
            "LEN" => text_fn(&args, |s| Value::Number(s.chars().count() as f64)),
            "UPPER" => text_fn(&args, |s| Value::Text(s.to_uppercase())),
            "LOWER" => text_fn(&args, |s| Value::Text(s.to_lowercase())),
            "TRIM" => text_fn(&args, |s| Value::Text(s.split_whitespace().collect::<Vec<_>>().join(" "))),
            "LEFT" | "RIGHT" => {
                let text = match args.first() {
                    Some(Arg::Scalar(Value::Error(k))) => return Value::Error(*k),
                    Some(Arg::Scalar(v)) => v.to_text(),
                    _ => return Value::Error(ErrorKind::Value),
                };
                let n = match opt_num(&args, 1, 1.0) {
                    Ok(n) if n >= 0.0 => n as usize,
                    Ok(_) => return Value::Error(ErrorKind::Value),
                    Err(k) => return Value::Error(k),
                };
                let chars: Vec<char> = text.chars().collect();
                let n = n.min(chars.len());
                let part: String = if name == "LEFT" {
                    chars[..n].iter().collect()
                } else {
                    chars[chars.len() - n..].iter().collect()
                };
                Value::Text(part)
            }
            _ => Value::Error(ErrorKind::Name),
        }
    }

    fn conditional(&self, name: &str, args: &[Arg]) -> Value {
        if args.len() < 2 || args.len() > 3 || (name == "COUNTIF" && args.len() != 2) {
            return Value::Error(ErrorKind::Value);
        }
        let range = match &args[0] {
            Arg::Range(vs) => vs.clone(),
            Arg::Scalar(v) => vec![v.clone()],
        };
        let criterion = match &args[1] {
            Arg::Scalar(Value::Error(k)) => return Value::Error(*k),
            Arg::Scalar(v) => Criterion::new(v),
            Arg::Range(_) => return Value::Error(ErrorKind::Value),
        };
        let target = match args.get(2) {
            Some(Arg::Range(vs)) => vs.clone(),
            Some(Arg::Scalar(v)) => vec![v.clone()],
            None => range.clone(),
        };
        let mut sum = 0.0;
        let mut count = 0usize;
        for (i, v) in range.iter().enumerate() {
            if !criterion.matches(v) {
                continue;
            }
            if name == "COUNTIF" {
                count += 1;
                continue;
            }
            match target.get(i) {
                Some(Value::Number(n)) => {
                    sum += n;
                    count += 1;
                }
                Some(Value::Error(k)) => return Value::Error(*k),
                _ => {}
            }
        }
        match name {
            "COUNTIF" => Value::Number(count as f64),
            "SUMIF" => Value::Number(sum),
            _ if count == 0 => Value::Error(ErrorKind::Div0),
            _ => Value::Number(sum / count as f64),
        }
    }
}

/* ── helpers ────────────────────────────────────────────────── */

/// A value as a number for arithmetic: empty is 0, TRUE is 1, text that
/// reads as a number counts, other text is `#VALUE!`.
fn num(v: &Value) -> Result<f64, ErrorKind> {
    match v {
        Value::Empty => Ok(0.0),
        Value::Number(n) => Ok(*n),
        Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
        Value::Text(s) => match parse_value(s) {
            Value::Number(n) => Ok(n),
            _ => Err(ErrorKind::Value),
        },
        Value::Error(k) => Err(*k),
    }
}

fn truth(v: &Value) -> Result<bool, ErrorKind> {
    match v {
        Value::Bool(b) => Ok(*b),
        Value::Empty => Ok(false),
        Value::Number(n) => Ok(*n != 0.0),
        Value::Text(s) => match s.to_uppercase().as_str() {
            "TRUE" | "VERO" => Ok(true),
            "FALSE" | "FALSO" => Ok(false),
            _ => Err(ErrorKind::Value),
        },
        Value::Error(k) => Err(*k),
    }
}

fn compare(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let rank = |v: &Value| match v {
        Value::Number(_) | Value::Empty => 0,
        Value::Text(_) => 1,
        Value::Bool(_) => 2,
        Value::Error(_) => 3,
    };
    match (a, b) {
        (Value::Empty, Value::Text(s)) | (Value::Text(s), Value::Empty) if s.is_empty() => Ordering::Equal,
        _ if rank(a) != rank(b) => rank(a).cmp(&rank(b)),
        (Value::Text(x), Value::Text(y)) => x.to_lowercase().cmp(&y.to_lowercase()),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        _ => {
            let x = num(a).unwrap_or(0.0);
            let y = num(b).unwrap_or(0.0);
            clean(x).partial_cmp(&clean(y)).unwrap_or(Ordering::Equal)
        }
    }
}

/// The numbers aggregate functions add up: from ranges, only numeric cells
/// (text and empty cells are skipped, an error stops); a value written
/// straight into the call must be a number.
fn numbers(args: &[Arg]) -> Result<Vec<f64>, ErrorKind> {
    let mut out = Vec::new();
    for a in args {
        match a {
            Arg::Range(vs) => {
                for v in vs {
                    match v {
                        Value::Number(n) => out.push(*n),
                        Value::Error(k) => return Err(*k),
                        _ => {}
                    }
                }
            }
            Arg::Scalar(Value::Empty) => {}
            Arg::Scalar(v) => out.push(num(v)?),
        }
    }
    Ok(out)
}

fn aggregate(name: &str, nums: &[f64]) -> Value {
    let n = nums.len();
    match name {
        "SUM" => Value::Number(nums.iter().sum()),
        "COUNT" => Value::Number(n as f64),
        "PRODUCT" => Value::Number(if n == 0 { 0.0 } else { nums.iter().product() }),
        _ if n == 0 => match name {
            "AVERAGE" => Value::Error(ErrorKind::Div0),
            "MEDIAN" => Value::Error(ErrorKind::Num),
            _ => Value::Number(0.0),
        },
        "AVERAGE" => Value::Number(nums.iter().sum::<f64>() / n as f64),
        "MIN" => Value::Number(nums.iter().copied().fold(f64::INFINITY, f64::min)),
        "MAX" => Value::Number(nums.iter().copied().fold(f64::NEG_INFINITY, f64::max)),
        _ => {
            let mut v = nums.to_vec();
            v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            Value::Number(if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 })
        }
    }
}

fn scalar_num(args: &[Arg], i: usize) -> Result<f64, ErrorKind> {
    match args.get(i) {
        Some(Arg::Scalar(v)) => num(v),
        _ => Err(ErrorKind::Value),
    }
}

fn opt_num(args: &[Arg], i: usize, default: f64) -> Result<f64, ErrorKind> {
    match args.get(i) {
        None => Ok(default),
        Some(Arg::Scalar(Value::Text(s))) if s.is_empty() => Ok(default),
        _ => scalar_num(args, i),
    }
}

fn one(args: &[Arg], f: impl Fn(f64) -> Result<f64, ErrorKind>) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    match scalar_num(args, 0).and_then(f) {
        Ok(n) => Value::Number(n),
        Err(k) => Value::Error(k),
    }
}

fn two(args: &[Arg], f: impl Fn(f64, f64) -> Result<f64, ErrorKind>) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    match scalar_num(args, 0).and_then(|x| scalar_num(args, 1).and_then(|y| f(x, y))) {
        Ok(n) => Value::Number(n),
        Err(k) => Value::Error(k),
    }
}

fn text_fn(args: &[Arg], f: impl Fn(&str) -> Value) -> Value {
    match args {
        [Arg::Scalar(Value::Error(k))] => Value::Error(*k),
        [Arg::Scalar(v)] => f(&v.to_text()),
        _ => Value::Error(ErrorKind::Value),
    }
}

/// A SUMIF / COUNTIF criterion: `">5"`, `"<>x"`, `"=3"`, `"apple"`, or 5.
struct Criterion {
    op: &'static str,
    value: Value,
}

impl Criterion {
    fn new(v: &Value) -> Self {
        let Value::Text(s) = v else {
            return Criterion { op: "=", value: v.clone() };
        };
        let s = s.trim();
        for op in ["<=", ">=", "<>", "<", ">", "="] {
            if let Some(rest) = s.strip_prefix(op) {
                return Criterion { op, value: parse_value(rest) };
            }
        }
        Criterion { op: "=", value: parse_value(s) }
    }

    fn matches(&self, v: &Value) -> bool {
        use std::cmp::Ordering::*;
        // Text against a number never matches `<`/`>`; `=` compares as text
        // ignoring case.
        let comparable = matches!(
            (v, &self.value),
            (Value::Number(_), Value::Number(_)) | (Value::Text(_), Value::Text(_)) | (Value::Bool(_), Value::Bool(_))
        ) || matches!((v, &self.value), (Value::Empty, Value::Empty));
        let ord = compare(v, &self.value);
        match self.op {
            "=" => comparable && ord == Equal,
            "<>" => !comparable || ord != Equal,
            "<" => comparable && ord == Less,
            ">" => comparable && ord == Greater,
            "<=" => comparable && ord != Greater,
            _ => comparable && ord != Less,
        }
    }
}

/* ── JSON for the window and the AI ─────────────────────────── */

/// A computed value as the window reads it: `{"t":"n","v":2.5}`,
/// `{"t":"s","v":"Pane"}`, `{"t":"b","v":true}`, `{"t":"e","v":"#DIV/0!"}`.
pub fn value_json(v: &Value) -> serde_json::Value {
    use serde_json::json;
    match v {
        Value::Empty => json!({ "t": "z" }),
        Value::Number(n) => json!({ "t": "n", "v": n }),
        Value::Text(s) => json!({ "t": "s", "v": s }),
        Value::Bool(b) => json!({ "t": "b", "v": b }),
        Value::Error(k) => json!({ "t": "e", "v": k.code() }),
    }
}

/// Every computed cell of a sheet, keyed by ref.
pub fn sheet_values_json(cells: &BTreeMap<String, String>) -> serde_json::Map<String, serde_json::Value> {
    evaluate_sheet(cells).iter().map(|(k, v)| (k.clone(), value_json(v))).collect()
}

/// What a formula cell computes to, as plain JSON for the AI: a number, a
/// string, a boolean, or the error code.
pub fn formula_results_json(cells: &BTreeMap<String, String>) -> serde_json::Map<String, serde_json::Value> {
    let values = evaluate_sheet(cells);
    cells
        .iter()
        .filter(|(_, raw)| raw.trim_start().starts_with('='))
        .filter_map(|(k, _)| {
            let v = values.get(&k.to_uppercase())?;
            let j = match v {
                Value::Number(n) => serde_json::json!(n),
                Value::Bool(b) => serde_json::json!(b),
                other => serde_json::json!(other.to_text()),
            };
            Some((k.clone(), j))
        })
        .collect()
}

/* ── tests ──────────────────────────────────────────────────── */

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(pairs: &[(&str, &str)]) -> BTreeMap<String, Value> {
        let cells: BTreeMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        evaluate_sheet(&cells)
    }

    fn n(v: &Value) -> f64 {
        match v {
            Value::Number(n) => *n,
            other => panic!("not a number: {other:?}"),
        }
    }

    #[test]
    fn values_read_as_typed() {
        assert_eq!(parse_value("2.50"), Value::Number(2.5));
        assert_eq!(parse_value("2,50"), Value::Number(2.5));
        assert_eq!(parse_value("1,250"), Value::Number(1250.0));
        assert_eq!(parse_value("1,234,567.5"), Value::Number(1_234_567.5));
        assert_eq!(parse_value("1.234.567,5"), Value::Number(1_234_567.5));
        assert_eq!(parse_value("10%"), Value::Number(0.1));
        assert_eq!(parse_value("12.5%"), Value::Number(0.125));
        assert_eq!(parse_value("€ 5"), Value::Number(5.0));
        assert_eq!(parse_value("5,00 €"), Value::Number(5.0));
        assert_eq!(parse_value("-€3.20"), Value::Number(-3.2));
        assert_eq!(parse_value("$1,000"), Value::Number(1000.0));
        assert_eq!(parse_value("1e3"), Value::Number(1000.0));
        assert_eq!(parse_value("VERO"), Value::Bool(true));
        assert_eq!(parse_value("Pane"), Value::Text("Pane".into()));
        assert_eq!(parse_value("15/03/2026"), Value::Text("15/03/2026".into()));
        assert_eq!(parse_value("1.2.3"), Value::Text("1.2.3".into()));
        assert_eq!(parse_value("02115"), Value::Number(2115.0));
    }

    #[test]
    fn the_shopping_list_adds_up() {
        let v = sheet(&[("B2", "2.50"), ("B3", "1.20"), ("B4", "3.10"), ("B5", "=SUM(B2:B4)"), ("C2", "=0.1+0.2"), ("C3", "=B3*3")]);
        assert_eq!(n(&v["B5"]), 6.8);
        assert_eq!(n(&v["C2"]), 0.3);
        assert_eq!(n(&v["C3"]), 3.6);
    }

    #[test]
    fn errors_say_what_went_wrong() {
        let v = sheet(&[
            ("A1", "Pane"), ("B1", "=A1+1"), ("B2", "=1/0"), ("B3", "=NOPE(1)"), ("B4", "=(1+"),
            ("B5", "=B5+1"), ("C1", "=C2"), ("C2", "=C1"), ("B6", "=SQRT(-1)"), ("B7", "=B2+1"),
            ("B8", "=foo"),
        ]);
        assert_eq!(v["B1"], Value::Error(ErrorKind::Value));
        assert_eq!(v["B2"], Value::Error(ErrorKind::Div0));
        assert_eq!(v["B3"], Value::Error(ErrorKind::Name));
        assert_eq!(v["B4"], Value::Error(ErrorKind::Parse));
        assert_eq!(v["B5"], Value::Error(ErrorKind::Circ));
        assert_eq!(v["C1"], Value::Error(ErrorKind::Circ));
        assert_eq!(v["C2"], Value::Error(ErrorKind::Circ));
        assert_eq!(v["B6"], Value::Error(ErrorKind::Num));
        assert_eq!(v["B7"], Value::Error(ErrorKind::Div0));
        assert_eq!(v["B8"], Value::Error(ErrorKind::Name));
    }

    #[test]
    fn percent_and_currency_cells_compute() {
        let v = sheet(&[("B11", "10%"), ("B12", "=B11*200"), ("B13", "€ 5"), ("B14", "=B13+1"), ("B15", "=200*15%")]);
        assert_eq!(n(&v["B12"]), 20.0);
        assert_eq!(n(&v["B14"]), 6.0);
        assert_eq!(n(&v["B15"]), 30.0);
    }

    #[test]
    fn functions_and_separators() {
        let v = sheet(&[
            ("A1", "1.25"), ("A2", "3"), ("A3", "x"), ("A4", "5"),
            ("B1", "=ROUND(A1;1)"), ("B2", "=ROUND(A1,1)"), ("B3", "=IF(A2>2;\"big\";\"small\")"),
            ("B4", "=SOMMA(A1:A4)"), ("B5", "=SE(A2>5;1;0)"), ("B6", "=ARROTONDA(2.345;2)"),
            ("B7", "=AVERAGE(A1:A4)"), ("B8", "=COUNT(A1:A4)"), ("B9", "=COUNTA(A1:A4)"),
            ("C1", "=SUMIF(A1:A4;\">2\")"), ("C2", "=COUNTIF(A1:A4;\"x\")"), ("C3", "=IFERROR(1/0;\"-\")"),
            ("C4", "=\"Tot: \"&A2"), ("C5", "=AND(A2>1;A4>4)"), ("C6", "=MOD(-7;3)"), ("C7", "=-2^2"),
            ("C8", "=ROUNDUP(1.21;1)"), ("C9", "=ROUNDDOWN(-1.29;1)"), ("D1", "=MEDIAN(A1:A4)"),
            ("D2", "=IF(A2>2;1;1/0)"), ("D3", "=LEFT(\"Calc\";2)&UPPER(\"x\")"), ("D4", "=A1:A2"),
            ("D5", "=$A$2*2"), ("D6", "=MAX(A1:A4)-MIN(A1:A4)"), ("D7", "=ROUND(2.5)"), ("D8", "=ROUND(-2.5)"),
        ]);
        assert_eq!(n(&v["B1"]), 1.3);
        assert_eq!(n(&v["B2"]), 1.3);
        assert_eq!(v["B3"], Value::Text("big".into()));
        assert_eq!(n(&v["B4"]), 9.25);
        assert_eq!(n(&v["B5"]), 0.0);
        assert_eq!(n(&v["B6"]), 2.35);
        assert!((n(&v["B7"]) - 9.25 / 3.0).abs() < 1e-12);
        assert_eq!(n(&v["B8"]), 3.0);
        assert_eq!(n(&v["B9"]), 4.0);
        assert_eq!(n(&v["C1"]), 8.0);
        assert_eq!(n(&v["C2"]), 1.0);
        assert_eq!(v["C3"], Value::Text("-".into()));
        assert_eq!(v["C4"], Value::Text("Tot: 3".into()));
        assert_eq!(v["C5"], Value::Bool(true));
        assert_eq!(n(&v["C6"]), 2.0);
        assert_eq!(n(&v["C7"]), 4.0);
        assert_eq!(n(&v["C8"]), 1.3);
        assert_eq!(n(&v["C9"]), -1.2);
        assert_eq!(n(&v["D1"]), 3.0);
        assert_eq!(n(&v["D2"]), 1.0);
        assert_eq!(v["D3"], Value::Text("CaX".into()));
        assert_eq!(v["D4"], Value::Error(ErrorKind::Value));
        assert_eq!(n(&v["D5"]), 6.0);
        assert_eq!(n(&v["D6"]), 3.75);
        assert_eq!(n(&v["D7"]), 3.0);
        assert_eq!(n(&v["D8"]), -3.0);
    }

    #[test]
    fn a_long_running_total_does_not_recurse() {
        let mut cells = BTreeMap::new();
        cells.insert("A1".to_string(), "1".to_string());
        for r in 2..=500 {
            cells.insert(format!("A{r}"), format!("=A{}+1", r - 1));
        }
        let v = evaluate_sheet(&cells);
        assert_eq!(n(&v["A500"]), 500.0);
    }

    #[test]
    fn empty_references_and_text_in_ranges() {
        let v = sheet(&[("A1", "=Z99"), ("A2", "=SUM(B1:B3)"), ("B1", "hello"), ("B2", "4"), ("A3", "=AVERAGE(C1:C3)")]);
        assert_eq!(n(&v["A1"]), 0.0);
        assert_eq!(n(&v["A2"]), 4.0);
        assert_eq!(v["A3"], Value::Error(ErrorKind::Div0));
    }

    #[test]
    fn deep_nesting_is_an_error_not_a_crash() {
        let deep = format!("={}1{}", "(".repeat(10_000), ")".repeat(10_000));
        let v = sheet(&[("A1", deep.as_str())]);
        assert_eq!(v["A1"], Value::Error(ErrorKind::Parse));
        let signs = format!("={}1", "-".repeat(10_000));
        let v = sheet(&[("A1", signs.as_str())]);
        assert_eq!(v["A1"], Value::Error(ErrorKind::Parse));
    }

    #[test]
    fn canonical_formula_for_export() {
        assert_eq!(canonical_formula("=SOMMA(A1:A3;2)"), "=SUM(A1:A3,2)");
        assert_eq!(canonical_formula("=SE(A1>1;\"a;b\";0)"), "=IF(A1>1,\"a;b\",0)");
        assert_eq!(canonical_formula("=E1+E(A1;B1)"), "=E1+AND(A1,B1)");
    }

    #[test]
    fn numbers_print_plainly() {
        assert_eq!(format_number(0.1 + 0.2), "0.3");
        assert_eq!(format_number(1_000_000.0), "1000000");
        assert_eq!(format_number(-2.5), "-2.5");
    }
}
