//! Safe scientific expression evaluator.
//!
//! A small recursive-descent parser + evaluator over `f64` with **no external
//! crates**, so the calculator plugin has one authoritative math engine used
//! by both the AI tool (`calculator_eval`) and the REST route the Calculator
//! window calls. Invalid input returns a `String` error rather than panicking.

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Op(char),
}

/// How the trigonometric functions read and give angles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Angle {
    /// `sin(pi/2)` is 1 — the default, and what the AI tool uses.
    #[default]
    Radians,
    /// `sin(90)` is 1 — the Calculator window's DEG mode.
    Degrees,
}

/// Evaluate a math expression and return a finite result (angles in radians).
pub fn evaluate(input: &str) -> Result<f64, String> {
    evaluate_with(input, Angle::Radians)
}

/// Evaluate with the given angle unit for `sin cos tan asin acos atan atan2`.
///
/// Beyond plain arithmetic it reads the way a pocket calculator does:
/// - a number next to a constant, a function or a parenthesis multiplies
///   (`2pi`, `3(4+1)`, `2sqrt(9)`, `(1+1)(2+2)`);
/// - `%` closing a term is a percentage: `50+10%` is 55, `50-10%` 45,
///   `200*15%` 30, `3%` 0.03. Between two operands it stays the remainder
///   (`10 % 3` is 1).
pub fn evaluate_with(input: &str, angle: Angle) -> Result<f64, String> {
    if input.len() > MAX_EXPR_LEN {
        return Err(format!(
            "expression is too long (max {MAX_EXPR_LEN} bytes)"
        ));
    }
    let tokens = implicit_multiplication(tokenize(input)?);
    if tokens.is_empty() {
        return Err("empty expression".into());
    }
    if tokens.len() > MAX_TOKENS {
        return Err("expression has too many tokens".into());
    }
    let mut p = Parser { tokens, pos: 0, depth: 0, degrees: angle == Angle::Degrees, percent: false };
    let value = p.parse_expr()?;
    if p.pos != p.tokens.len() {
        return Err(format!("unexpected trailing input at position {}", p.pos));
    }
    if !value.is_finite() {
        return Err("result is not a finite number".into());
    }
    Ok(value)
}

/// Format a result for display (trim trailing zeros, avoid `1e7` style noise).
pub fn format_number(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    let a = v.abs();
    if a != 0.0 && (a >= 1e12 || a < 1e-9) {
        return format!("{:e}", v);
    }
    let mut s = format!("{:.10}", v);
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    s
}

// ---------- Tokenizer --------------------------------------------------------

fn tokenize(input: &str) -> Result<Vec<Tok>, String> {
    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || c == '.' {
            let start = i;
            let mut seen_dot = false;
            while i < bytes.len() {
                let ch = bytes[i] as char;
                if ch.is_ascii_digit() {
                    i += 1;
                    continue;
                }
                if ch == '.' && !seen_dot {
                    seen_dot = true;
                    i += 1;
                    continue;
                }
                break;
            }
            // Scientific notation: `1.5e3`, `2E-4`.
            if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
                let mut j = i + 1;
                if j < bytes.len() && (bytes[j] == b'+' || bytes[j] == b'-') {
                    j += 1;
                }
                if j < bytes.len() && (bytes[j] as char).is_ascii_digit() {
                    while j < bytes.len() && (bytes[j] as char).is_ascii_digit() {
                        j += 1;
                    }
                    i = j;
                }
            }
            let text = &input[start..i];
            let n: f64 = text.parse().map_err(|_| format!("invalid number \"{text}\""))?;
            out.push(Tok::Num(n));
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < bytes.len() {
                let ch = bytes[i] as char;
                if ch.is_ascii_alphanumeric() || ch == '_' {
                    i += 1;
                } else {
                    break;
                }
            }
            out.push(Tok::Ident(input[start..i].to_lowercase()));
            continue;
        }
        match c {
            '+' | '-' | '*' | '/' | '%' | '^' | '!' | '(' | ')' | ',' => {
                out.push(Tok::Op(c));
                i += 1;
            }
            other => return Err(format!("unexpected character \"{other}\"")),
        }
    }
    Ok(out)
}

/// Insert the `*` a person leaves out: after a number, a constant, `)` or
/// `!`, when a number, a name or `(` follows. A name followed by `(` is a
/// function call and keeps its parenthesis; two bare numbers (`2 3`) stay an
/// error rather than silently multiplying.
fn implicit_multiplication(tokens: Vec<Tok>) -> Vec<Tok> {
    let mut out: Vec<Tok> = Vec::with_capacity(tokens.len());
    for tok in &tokens {
        if let Some(prev) = out.last() {
            let prev_ends_operand = match prev {
                Tok::Num(_) => true,
                Tok::Op(')') | Tok::Op('!') => true,
                // A name ends an operand unless it is a function being called.
                Tok::Ident(_) => !matches!(tok, Tok::Op('(')),
                _ => false,
            };
            let starts_operand = match tok {
                Tok::Num(_) => !matches!(prev, Tok::Num(_)),
                Tok::Ident(_) | Tok::Op('(') => true,
                _ => false,
            };
            if prev_ends_operand && starts_operand {
                out.push(Tok::Op('*'));
            }
        }
        out.push(tok.clone());
    }
    out
}

// ---------- Parser -----------------------------------------------------------

struct Parser {
    tokens: Vec<Tok>,
    pos: usize,
    /// Recursion depth of the descent. Every nested parenthesis, unary sign
    /// and right-associative power adds a level; without a bound, a few tens of
    /// thousands of characters overflow the stack. A stack overflow is a
    /// SIGSEGV, not a Rust panic, so it would abort the whole host process.
    depth: u32,
    /// Trigonometry in degrees (the window's DEG mode).
    degrees: bool,
    /// The factor just parsed ended in a percentage (`10%`), so a `+` or `-`
    /// before its term takes that share of the left side (`50+10%` = 55).
    percent: bool,
}

/// More nesting than any real expression needs, and far below the ~10k frames
/// that would overflow a worker thread's stack.
const MAX_PARSE_DEPTH: u32 = 128;

/// Upper bounds on the raw input size and token count. The route's body limit
/// is 160 MB, and each `Tok` costs ~16–24 bytes, so without these caps a single
/// request could allocate gigabytes before parsing even starts.
const MAX_EXPR_LEN: usize = 64 * 1024;
const MAX_TOKENS: usize = 16 * 1024;

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.tokens.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn parse_expr(&mut self) -> Result<f64, String> {
        self.depth += 1;
        if self.depth > MAX_PARSE_DEPTH {
            return Err("expression is nested too deeply".into());
        }
        let result = self.parse_add();
        self.depth -= 1;
        result
    }

    fn parse_add(&mut self) -> Result<f64, String> {
        let mut v = self.parse_mul()?;
        loop {
            let sign = match self.peek() {
                Some(Tok::Op('+')) => 1.0,
                Some(Tok::Op('-')) => -1.0,
                _ => break,
            };
            self.next();
            self.percent = false;
            let term = self.parse_mul()?;
            // `a + b%` adds b percent of a, like a pocket calculator.
            let term = if self.percent { v * term } else { term };
            self.percent = false;
            v += sign * term;
        }
        self.percent = false;
        Ok(v)
    }

    fn parse_mul(&mut self) -> Result<f64, String> {
        let mut v = self.parse_unary()?;
        loop {
            match self.peek() {
                Some(Tok::Op('*')) => {
                    self.next();
                    self.percent = false;
                    v *= self.parse_unary()?;
                }
                Some(Tok::Op('/')) => {
                    self.next();
                    self.percent = false;
                    let d = self.parse_unary()?;
                    if d == 0.0 {
                        return Err("division by zero".into());
                    }
                    v /= d;
                }
                Some(Tok::Op('%')) => {
                    // Only reached between two operands (see parse_postfix):
                    // the remainder.
                    self.next();
                    self.percent = false;
                    let d = self.parse_unary()?;
                    if d == 0.0 {
                        return Err("modulo by zero".into());
                    }
                    v %= d;
                }
                _ => break,
            }
        }
        Ok(v)
    }

    fn parse_unary(&mut self) -> Result<f64, String> {
        self.depth += 1;
        if self.depth > MAX_PARSE_DEPTH {
            return Err("expression is nested too deeply".into());
        }
        let result = self.parse_unary_inner();
        self.depth -= 1;
        result
    }

    fn parse_unary_inner(&mut self) -> Result<f64, String> {
        match self.peek() {
            Some(Tok::Op('+')) => {
                self.next();
                self.parse_unary()
            }
            Some(Tok::Op('-')) => {
                self.next();
                Ok(-self.parse_unary()?)
            }
            _ => self.parse_pow(),
        }
    }

    fn parse_pow(&mut self) -> Result<f64, String> {
        let base = self.parse_postfix()?;
        if let Some(Tok::Op('^')) = self.peek() {
            self.next();
            // Right-associative; the exponent may itself be unary (`2^-3`).
            let exp = self.parse_unary()?;
            // A percentage inside the exponent is not a share of the sum.
            self.percent = false;
            return Ok(base.powf(exp));
        }
        Ok(base)
    }

    fn parse_postfix(&mut self) -> Result<f64, String> {
        let mut v = self.parse_primary()?;
        loop {
            match self.peek() {
                Some(Tok::Op('!')) => {
                    self.next();
                    v = factorial(v)?;
                }
                // `%` with no operand after it closes a percentage; with one
                // (`10 % 3`) it is the remainder, left to parse_mul.
                Some(Tok::Op('%')) if !self.operand_at(self.pos + 1) => {
                    self.next();
                    v /= 100.0;
                    self.percent = true;
                }
                _ => break,
            }
        }
        Ok(v)
    }

    /// Whether the token at `i` starts an operand.
    fn operand_at(&self, i: usize) -> bool {
        matches!(self.tokens.get(i), Some(Tok::Num(_) | Tok::Ident(_) | Tok::Op('(')))
    }

    fn parse_primary(&mut self) -> Result<f64, String> {
        match self.next() {
            Some(Tok::Num(n)) => Ok(n),
            Some(Tok::Op('(')) => {
                let v = self.parse_expr()?;
                self.expect_close_paren()?;
                // `50+(10%)` is 50.1: a bracket closes its own percentages.
                self.percent = false;
                Ok(v)
            }
            Some(Tok::Ident(name)) => {
                if let Some(Tok::Op('(')) = self.peek() {
                    self.next();
                    let mut args = Vec::new();
                    if let Some(Tok::Op(')')) = self.peek() {
                        self.next();
                    } else {
                        loop {
                            args.push(self.parse_expr()?);
                            match self.next() {
                                Some(Tok::Op(',')) => continue,
                                Some(Tok::Op(')')) => break,
                                _ => return Err("expected ',' or ')' in function call".into()),
                            }
                        }
                    }
                    self.percent = false;
                    call_function(&name, &args, self.degrees)
                } else {
                    constant(&name).ok_or_else(|| format!("unknown symbol \"{name}\""))
                }
            }
            Some(Tok::Op(other)) => Err(format!("unexpected \"{other}\"")),
            None => Err("unexpected end of expression".into()),
        }
    }

    fn expect_close_paren(&mut self) -> Result<(), String> {
        match self.next() {
            Some(Tok::Op(')')) => Ok(()),
            _ => Err("expected ')'".into()),
        }
    }
}

// ---------- Functions & constants -------------------------------------------

fn constant(name: &str) -> Option<f64> {
    match name {
        "pi" => Some(std::f64::consts::PI),
        "e" => Some(std::f64::consts::E),
        "tau" => Some(std::f64::consts::TAU),
        "phi" => Some(std::f64::consts::GOLDEN_RATIO),
        _ => None,
    }
}

fn factorial(n: f64) -> Result<f64, String> {
    if n < 0.0 || n.fract() != 0.0 {
        return Err("factorial (!) requires a non-negative integer".into());
    }
    if n > 170.0 {
        return Err("factorial argument too large".into());
    }
    let mut acc = 1.0f64;
    let mut i = 2.0f64;
    while i <= n {
        acc *= i;
        i += 1.0;
    }
    Ok(acc)
}

/// A trigonometric result within rounding of zero is zero: `sin(pi)` and
/// `cos(90°)` are 0, not 1.2e-16.
fn snap(v: f64) -> f64 {
    if v.abs() < 1e-12 { 0.0 } else { v }
}

fn call_function(name: &str, args: &[f64], degrees: bool) -> Result<f64, String> {
    // In DEG mode the plain names read and give degrees.
    let name = if degrees {
        match name {
            "sin" => "sind",
            "cos" => "cosd",
            "tan" => "tand",
            "asin" => "asind",
            "acos" => "acosd",
            "atan" => "atand",
            "atan2" => "atan2d",
            other => other,
        }
    } else {
        name
    };
    let need = |n: usize| -> Result<(), String> {
        if args.len() != n {
            return Err(format!("{name} expects {n} argument(s), got {}", args.len()));
        }
        Ok(())
    };
    match name {
        "sin" => { need(1)?; Ok(snap(args[0].sin())) }
        "cos" => { need(1)?; Ok(snap(args[0].cos())) }
        "tan" => { need(1)?; Ok(snap(args[0].tan())) }
        "asin" => { need(1)?; Ok(args[0].asin()) }
        "acos" => { need(1)?; Ok(args[0].acos()) }
        "atan" => { need(1)?; Ok(args[0].atan()) }
        "sinh" => { need(1)?; Ok(args[0].sinh()) }
        "cosh" => { need(1)?; Ok(args[0].cosh()) }
        "tanh" => { need(1)?; Ok(args[0].tanh()) }
        "asinh" => { need(1)?; Ok(args[0].asinh()) }
        "acosh" => { need(1)?; Ok(args[0].acosh()) }
        "atanh" => { need(1)?; Ok(args[0].atanh()) }
        "sind" => { need(1)?; Ok(snap(args[0].to_radians().sin())) }
        "cosd" => { need(1)?; Ok(snap(args[0].to_radians().cos())) }
        "tand" => {
            need(1)?;
            if (args[0] % 180.0).abs() == 90.0 {
                return Err("tan is not defined at 90 degrees".into());
            }
            Ok(snap(args[0].to_radians().tan()))
        }
        "asind" => { need(1)?; Ok(args[0].asin().to_degrees()) }
        "acosd" => { need(1)?; Ok(args[0].acos().to_degrees()) }
        "atand" => { need(1)?; Ok(args[0].atan().to_degrees()) }
        "deg" => { need(1)?; Ok(args[0].to_degrees()) }
        "rad" => { need(1)?; Ok(args[0].to_radians()) }
        "ln" => {
            need(1)?;
            if args[0] <= 0.0 { return Err("ln of a non-positive number".into()); }
            Ok(args[0].ln())
        }
        "log" => match args.len() {
            1 => {
                if args[0] <= 0.0 { return Err("log of a non-positive number".into()); }
                Ok(args[0].log10())
            }
            2 => {
                if args[0] <= 0.0 || args[0] == 1.0 {
                    return Err("log base must be > 0 and != 1".into());
                }
                if args[1] <= 0.0 { return Err("log of a non-positive number".into()); }
                Ok(args[1].log(args[0]))
            }
            _ => Err("log expects 1 or 2 arguments".into()),
        },
        "log2" => {
            need(1)?;
            if args[0] <= 0.0 { return Err("log2 of a non-positive number".into()); }
            Ok(args[0].log2())
        }
        "exp" => { need(1)?; Ok(args[0].exp()) }
        "sqrt" => {
            need(1)?;
            if args[0] < 0.0 { return Err("sqrt of a negative number".into()); }
            Ok(args[0].sqrt())
        }
        "cbrt" => { need(1)?; Ok(args[0].cbrt()) }
        "abs" => { need(1)?; Ok(args[0].abs()) }
        "floor" => { need(1)?; Ok(args[0].floor()) }
        "ceil" => { need(1)?; Ok(args[0].ceil()) }
        "round" => { need(1)?; Ok(args[0].round()) }
        "trunc" => { need(1)?; Ok(args[0].trunc()) }
        "sign" => { need(1)?; Ok(args[0].signum()) }
        "fact" | "factorial" => { need(1)?; factorial(args[0]) }
        "atan2" => { need(2)?; Ok(args[0].atan2(args[1])) }
        "atan2d" => { need(2)?; Ok(args[0].atan2(args[1]).to_degrees()) }
        "pow" => { need(2)?; Ok(args[0].powf(args[1])) }
        "mod" => {
            need(2)?;
            if args[1] == 0.0 { return Err("modulo by zero".into()); }
            Ok(args[0] % args[1])
        }
        "hypot" => { need(2)?; Ok(args[0].hypot(args[1])) }
        _ => Err(format!("unknown function \"{name}\"")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_arithmetic() {
        assert_eq!(evaluate("2+3*4").unwrap(), 14.0);
        assert_eq!(evaluate("(2+3)*4").unwrap(), 20.0);
        assert_eq!(evaluate("2^10").unwrap(), 1024.0);
        assert_eq!(evaluate("10 % 3").unwrap(), 1.0);
        assert_eq!(evaluate("-2^2").unwrap(), -4.0);
        assert_eq!(evaluate("5!").unwrap(), 120.0);
    }

    #[test]
    fn scientific() {
        assert!((evaluate("sin(pi/2)").unwrap() - 1.0).abs() < 1e-12);
        assert!((evaluate("sqrt(81)").unwrap() - 9.0).abs() < 1e-12);
        assert!((evaluate("ln(e)").unwrap() - 1.0).abs() < 1e-12);
        assert!((evaluate("log(1000)").unwrap() - 3.0).abs() < 1e-12);
        assert!((evaluate("deg(pi)").unwrap() - 180.0).abs() < 1e-12);
        assert!((evaluate("pow(2,8)").unwrap() - 256.0).abs() < 1e-12);
    }

    #[test]
    fn percentages_read_like_a_pocket_calculator() {
        assert_eq!(evaluate("50+10%").unwrap(), 55.0);
        assert_eq!(evaluate("50-10%").unwrap(), 45.0);
        assert!((evaluate("200*15%").unwrap() - 30.0).abs() < 1e-12);
        assert!((evaluate("3%").unwrap() - 0.03).abs() < 1e-12);
        assert!((evaluate("50+(10%)").unwrap() - 50.1).abs() < 1e-12);
        assert!((evaluate("80/50%").unwrap() - 160.0).abs() < 1e-12);
        // Between two operands it is still the remainder.
        assert_eq!(evaluate("10%3").unwrap(), 1.0);
        assert_eq!(evaluate("10%3+1").unwrap(), 2.0);
        assert_eq!(evaluate("100-10%+5").unwrap(), 95.0);
    }

    #[test]
    fn a_missing_times_sign_multiplies() {
        assert!((evaluate("2pi").unwrap() - 2.0 * std::f64::consts::PI).abs() < 1e-12);
        assert_eq!(evaluate("3(4+1)").unwrap(), 15.0);
        assert_eq!(evaluate("(1+1)(2+2)").unwrap(), 8.0);
        assert_eq!(evaluate("2sqrt(9)").unwrap(), 6.0);
        assert_eq!(evaluate("3!2").unwrap(), 12.0);
        // Scientific notation is still a number, and `2e` is 2 times e.
        assert_eq!(evaluate("2e3").unwrap(), 2000.0);
        assert!((evaluate("2e").unwrap() - 2.0 * std::f64::consts::E).abs() < 1e-12);
        // A function keeps its call; two bare numbers stay an error.
        assert_eq!(evaluate("sqrt(16)").unwrap(), 4.0);
        assert!(evaluate("2 3").is_err());
    }

    #[test]
    fn degrees_mode() {
        let d = |s: &str| evaluate_with(s, Angle::Degrees).unwrap();
        assert!((d("sin(30)") - 0.5).abs() < 1e-12);
        assert_eq!(d("cos(90)"), 0.0);
        assert_eq!(d("sin(180)"), 0.0);
        assert!((d("asin(1)") - 90.0).abs() < 1e-12);
        assert!((d("atan2(1,1)") - 45.0).abs() < 1e-12);
        assert!(evaluate_with("tan(90)", Angle::Degrees).is_err());
        // Radians stay the default, and sin(pi) is exactly 0.
        assert!((evaluate("sin(30)").unwrap() + 0.988_031_624_092_861_8).abs() < 1e-12);
        assert_eq!(evaluate("sin(pi)").unwrap(), 0.0);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(evaluate("1/0").is_err());
        assert!(evaluate("sqrt(-1)").is_err());
        assert!(evaluate("1 +").is_err());
        assert!(evaluate("").is_err());
    }

    /// Regression: unbounded recursion used to overflow the worker stack
    /// (SIGSEGV, killing the whole host process) instead of returning an error.
    #[test]
    fn deep_nesting_is_rejected_not_crashing() {
        let parens = format!("{}1{}", "(".repeat(100_000), ")".repeat(100_000));
        assert!(evaluate(&parens).is_err(), "nested parentheses must be rejected");
        let signs = format!("{}1", "-".repeat(100_000));
        assert!(evaluate(&signs).is_err(), "unary chain must be rejected");
        let powers = format!("1{}", "^1".repeat(100_000));
        assert!(evaluate(&powers).is_err(), "power chain must be rejected");
    }

    /// Regression: the route's 160 MB body limit must not turn into a
    /// multi-gigabyte token vector. Oversized inputs fail before/at tokenize.
    #[test]
    fn oversized_input_is_rejected_before_tokenizing() {
        let long = "1+".repeat(MAX_EXPR_LEN);
        assert!(evaluate(&long).is_err(), "input beyond MAX_EXPR_LEN must be rejected");

        let many = "1,".repeat(MAX_TOKENS);
        assert!(many.len() < MAX_EXPR_LEN);
        assert!(evaluate(&many).is_err(), "token count beyond MAX_TOKENS must be rejected");

        // an expression at a realistic size still works
        let ok = "1+".repeat(MAX_TOKENS / 2 / 2) + "1";
        assert_eq!(evaluate(&ok).unwrap(), (MAX_TOKENS / 2 / 2 + 1) as f64);
    }
}
