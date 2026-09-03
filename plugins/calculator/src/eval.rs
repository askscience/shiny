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

/// Evaluate a math expression and return a finite result.
pub fn evaluate(input: &str) -> Result<f64, String> {
    let tokens = tokenize(input)?;
    if tokens.is_empty() {
        return Err("empty expression".into());
    }
    let mut p = Parser { tokens, pos: 0 };
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

// ---------- Parser -----------------------------------------------------------

struct Parser {
    tokens: Vec<Tok>,
    pos: usize,
}

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
        self.parse_add()
    }

    fn parse_add(&mut self) -> Result<f64, String> {
        let mut v = self.parse_mul()?;
        loop {
            match self.peek() {
                Some(Tok::Op('+')) => {
                    self.next();
                    v += self.parse_mul()?;
                }
                Some(Tok::Op('-')) => {
                    self.next();
                    v -= self.parse_mul()?;
                }
                _ => break,
            }
        }
        Ok(v)
    }

    fn parse_mul(&mut self) -> Result<f64, String> {
        let mut v = self.parse_unary()?;
        loop {
            match self.peek() {
                Some(Tok::Op('*')) => {
                    self.next();
                    v *= self.parse_unary()?;
                }
                Some(Tok::Op('/')) => {
                    self.next();
                    let d = self.parse_unary()?;
                    if d == 0.0 {
                        return Err("division by zero".into());
                    }
                    v /= d;
                }
                Some(Tok::Op('%')) => {
                    self.next();
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
            return Ok(base.powf(exp));
        }
        Ok(base)
    }

    fn parse_postfix(&mut self) -> Result<f64, String> {
        let v = self.parse_primary()?;
        if let Some(Tok::Op('!')) = self.peek() {
            self.next();
            return factorial(v);
        }
        Ok(v)
    }

    fn parse_primary(&mut self) -> Result<f64, String> {
        match self.next() {
            Some(Tok::Num(n)) => Ok(n),
            Some(Tok::Op('(')) => {
                let v = self.parse_expr()?;
                self.expect_close_paren()?;
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
                    call_function(&name, &args)
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
        "phi" => Some(1.618_033_988_749_895),
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

fn call_function(name: &str, args: &[f64]) -> Result<f64, String> {
    let need = |n: usize| -> Result<(), String> {
        if args.len() != n {
            return Err(format!("{name} expects {n} argument(s), got {}", args.len()));
        }
        Ok(())
    };
    match name {
        "sin" => { need(1)?; Ok(args[0].sin()) }
        "cos" => { need(1)?; Ok(args[0].cos()) }
        "tan" => { need(1)?; Ok(args[0].tan()) }
        "asin" => { need(1)?; Ok(args[0].asin()) }
        "acos" => { need(1)?; Ok(args[0].acos()) }
        "atan" => { need(1)?; Ok(args[0].atan()) }
        "sinh" => { need(1)?; Ok(args[0].sinh()) }
        "cosh" => { need(1)?; Ok(args[0].cosh()) }
        "tanh" => { need(1)?; Ok(args[0].tanh()) }
        "asinh" => { need(1)?; Ok(args[0].asinh()) }
        "acosh" => { need(1)?; Ok(args[0].acosh()) }
        "atanh" => { need(1)?; Ok(args[0].atanh()) }
        "sind" => { need(1)?; Ok(args[0].to_radians().sin()) }
        "cosd" => { need(1)?; Ok(args[0].to_radians().cos()) }
        "tand" => { need(1)?; Ok(args[0].to_radians().tan()) }
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
    fn rejects_bad_input() {
        assert!(evaluate("1/0").is_err());
        assert!(evaluate("sqrt(-1)").is_err());
        assert!(evaluate("1 +").is_err());
        assert!(evaluate("").is_err());
    }
}
