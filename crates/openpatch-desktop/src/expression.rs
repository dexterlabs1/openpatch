//! Bounded, original parameter-expression evaluator. Powers associate right;
//! parentheses retain their usual meaning (including `(-2)^2 == 4`).
//! NaN is an error; infinities are preserved for compositions and validated by
//! the parameter caller. Conventional exponent signs deliberately differ from
//! tinyexpr (`2^-2^2 == 1/16`).
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExpressionError {
    Syntax,
    UnknownName,
    Domain,
    Limit,
}
impl fmt::Display for ExpressionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Syntax => "Invalid expression",
            Self::UnknownName => "Unknown note or function",
            Self::Domain => "Value outside mathematical domain",
            Self::Limit => "Expression is too complex",
        })
    }
}

type Result<T> = std::result::Result<T, ExpressionError>;
#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f64),
    Name(String),
    Symbol(u8),
    End,
}

pub(crate) fn evaluate(input: &str) -> Result<f64> {
    if input.len() > 512 {
        return Err(ExpressionError::Limit);
    }
    let input = input.replace('−', "-").to_ascii_lowercase();
    let bytes = input.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        let token = if c.is_ascii_digit() || c == b'.' {
            if c == b'0' && bytes.get(i + 1) == Some(&b'x') {
                let (value, end) = hex_number(&input, i)?;
                i = end;
                tokens.push(Token::Number(value));
                if tokens.len() > 256 {
                    return Err(ExpressionError::Limit);
                }
                continue;
            }
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'e' {
                i += 1;
                if i < bytes.len() && matches!(bytes[i], b'+' | b'-') {
                    i += 1;
                }
                let exponent = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if i == exponent {
                    return Err(ExpressionError::Syntax);
                }
            }
            let value = input[start..i]
                .parse::<f64>()
                .map_err(|_| ExpressionError::Syntax)?;
            Token::Number(value)
        } else if c.is_ascii_alphabetic() {
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'#') {
                i += 1;
            }
            Token::Name(input[start..i].replace('#', "s"))
        } else if b"+-*/%^(),".contains(&c) {
            i += 1;
            Token::Symbol(c)
        } else {
            return Err(ExpressionError::Syntax);
        };
        tokens.push(token);
        if tokens.len() > 256 {
            return Err(ExpressionError::Limit);
        }
    }
    tokens.push(Token::End);
    let mut parser = Parser {
        tokens,
        cursor: 0,
        operations: 0,
    };
    let value = parser.list(0)?;
    if parser.peek() != &Token::End {
        return Err(ExpressionError::Syntax);
    }
    if value.is_nan() {
        return Err(ExpressionError::Domain);
    }
    Ok(value)
}

struct Parser {
    tokens: Vec<Token>,
    cursor: usize,
    operations: usize,
}
impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.cursor]
    }
    fn take(&mut self, c: u8) -> bool {
        if self.peek() == &Token::Symbol(c) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }
    fn operation(&mut self) -> Result<()> {
        self.operations += 1;
        if self.operations > 256 {
            Err(ExpressionError::Limit)
        } else {
            Ok(())
        }
    }
    fn list(&mut self, depth: usize) -> Result<f64> {
        let mut value = self.expr(0, depth)?;
        while self.take(b',') {
            self.operation()?;
            value = self.expr(0, depth)?;
        }
        Ok(value)
    }
    fn expr(&mut self, minimum: u8, depth: usize) -> Result<f64> {
        if depth > 32 {
            return Err(ExpressionError::Limit);
        }
        let mut left = self.primary(depth)?;
        loop {
            let (operator, precedence) = match self.peek() {
                Token::Symbol(c @ (b'+' | b'-')) => (*c, 1),
                Token::Symbol(c @ (b'*' | b'/' | b'%')) => (*c, 2),
                Token::Symbol(b'^') => (b'^', 4),
                _ => break,
            };
            if precedence < minimum {
                break;
            }
            self.cursor += 1;
            self.operation()?;
            let right = self.expr(
                if operator == b'^' {
                    precedence
                } else {
                    precedence + 1
                },
                depth + 1,
            )?;
            left = valid(match operator {
                b'+' => left + right,
                b'-' => left - right,
                b'*' => left * right,
                b'/' => left / right,
                b'%' => left % right,
                _ => left.powf(right),
            })?;
        }
        Ok(left)
    }
    fn primary(&mut self, depth: usize) -> Result<f64> {
        if depth > 32 {
            return Err(ExpressionError::Limit);
        }
        if self.take(b'+') {
            self.operation()?;
            return self.expr(3, depth + 1);
        }
        if self.take(b'-') {
            self.operation()?;
            return Ok(-self.expr(3, depth + 1)?);
        }
        if self.take(b'(') {
            let value = self.list(depth + 1)?;
            if !self.take(b')') {
                return Err(ExpressionError::Syntax);
            }
            return Ok(value);
        }
        let token = self.peek().clone();
        match token {
            Token::Number(value) => {
                self.cursor += 1;
                Ok(value)
            }
            Token::Name(name) => {
                self.cursor += 1;
                if name == "inf" {
                    return Ok(f64::INFINITY);
                }
                if name == "pi" {
                    if self.take(b'(') && !self.take(b')') {
                        return Err(ExpressionError::Syntax);
                    }
                    return Ok(std::f64::consts::PI);
                }
                if let Some(value) = note(&name) {
                    return Ok(value);
                }
                let arity = arity(&name).ok_or(ExpressionError::UnknownName)?;
                self.operation()?;
                let mut args = [0.0; 2];
                if arity == 1 {
                    // Bare unary functions bind to a signed primary, as in tinyexpr.
                    args[0] = self.bare_argument(depth + 1)?;
                } else {
                    if !self.take(b'(') {
                        return Err(ExpressionError::Syntax);
                    }
                    args[0] = self.expr(0, depth + 1)?;
                    if !self.take(b',') {
                        return Err(ExpressionError::Syntax);
                    }
                    args[1] = self.expr(0, depth + 1)?;
                    if !self.take(b')') {
                        return Err(ExpressionError::Syntax);
                    }
                }
                function(&name, args)
            }
            _ => Err(ExpressionError::Syntax),
        }
    }
    fn bare_argument(&mut self, depth: usize) -> Result<f64> {
        if depth > 32 {
            return Err(ExpressionError::Limit);
        }
        if self.take(b'+') {
            self.operation()?;
            return self.bare_argument(depth + 1);
        }
        if self.take(b'-') {
            self.operation()?;
            return Ok(-self.bare_argument(depth + 1)?);
        }
        self.primary(depth)
    }
}
fn valid(value: f64) -> Result<f64> {
    if !value.is_nan() {
        Ok(value)
    } else {
        Err(ExpressionError::Domain)
    }
}
fn note(name: &str) -> Option<f64> {
    let bytes = name.as_bytes();
    let mut semitone = match *bytes.first()? {
        b'c' => 0,
        b'd' => 2,
        b'e' => 4,
        b'f' => 5,
        b'g' => 7,
        b'a' => 9,
        b'b' => 11,
        _ => return None,
    };
    let mut i = 1;
    if bytes.get(i) == Some(&b's') {
        semitone += 1;
        i += 1;
    } else if bytes.get(i) == Some(&b'b') {
        semitone -= 1;
        i += 1;
    }
    let mut octave = 4;
    if let Some(digit @ b'0'..=b'9') = bytes.get(i) {
        octave = (digit - b'0') as i32;
        i += 1;
    }
    let voltage = (octave - 4) as f64 + semitone as f64 / 12.0;
    let volts = bytes.get(i) == Some(&b'v');
    if volts {
        i += 1;
    }
    if i != bytes.len() {
        return None;
    }
    Some(if volts {
        voltage
    } else {
        440.0 * (voltage - 0.75).exp2()
    })
}
fn arity(name: &str) -> Option<usize> {
    Some(match name {
        "atan2" | "pow" | "ncr" | "npr" => 2,
        "abs" | "acos" | "asin" | "atan" | "ceil" | "cos" | "cosh" | "exp" | "fac" | "floor"
        | "ln" | "log" | "log10" | "log2" | "sin" | "sinh" | "sqrt" | "tan" | "tanh"
        | "gaintodb" | "dbtogain" | "vtof" | "ftov" | "vtobpm" | "bpmtov" => 1,
        _ => return None,
    })
}
fn function(name: &str, [x, y]: [f64; 2]) -> Result<f64> {
    if name == "dbtogain" && x == f64::NEG_INFINITY {
        return Ok(0.0);
    }
    if name == "gaintodb" && x == 0.0 {
        return Ok(f64::NEG_INFINITY);
    }
    const C4: f64 = 261.6256_f32 as f64;
    valid(match name {
        "abs" => x.abs(),
        "acos" => x.acos(),
        "asin" => x.asin(),
        "atan" => x.atan(),
        "atan2" => x.atan2(y),
        "ceil" => x.ceil(),
        "cos" => x.cos(),
        "cosh" => x.cosh(),
        "exp" => x.exp(),
        "floor" => x.floor(),
        "ln" | "log" => x.ln(),
        "log10" => x.log10(),
        "log2" => x.log2(),
        "pow" => x.powf(y),
        "sin" => x.sin(),
        "sinh" => x.sinh(),
        "sqrt" => x.sqrt(),
        "tan" => x.tan(),
        "tanh" => x.tanh(),
        "gaintodb" => x.log10() * 20.0,
        "dbtogain" => 10.0_f64.powf(x / 20.0),
        "vtof" => x.exp2() * C4,
        "ftov" => (x / C4).log2(),
        "vtobpm" => x.exp2() * 120.0,
        "bpmtov" => (x / 120.0).log2(),
        "fac" => factorial(x)?,
        "ncr" | "npr" => combinations(x, y, name == "npr")?,
        _ => return Err(ExpressionError::UnknownName),
    })
}
// Preserve tinyexpr's truncation and 64-bit unsigned intermediate overflow.
// Bound combinatorial work independently of the numeric result.
fn factorial(x: f64) -> Result<f64> {
    if x.is_nan() || x < 0.0 {
        return Err(ExpressionError::Domain);
    }
    if x >= 21.0 {
        return Ok(f64::INFINITY);
    }
    Ok((1..=x as u64).product::<u64>() as f64)
}
fn combinations(n: f64, r: f64, permutation: bool) -> Result<f64> {
    if n.is_nan() || r.is_nan() || n < 0.0 || r < 0.0 || r > n {
        return Err(ExpressionError::Domain);
    }
    if n > u32::MAX as f64 || r > u32::MAX as f64 {
        return Ok(f64::INFINITY);
    }
    let (n, r) = (n as u64, r as u64);
    let steps = r.min(n - r);
    if steps > 256 {
        return Err(ExpressionError::Limit);
    }
    let mut result = 1_u64;
    for i in 1..=steps {
        match result.checked_mul(n - steps + i) {
            Some(value) => result = value / i,
            None => return Ok(f64::INFINITY),
        }
    }
    valid(if permutation {
        result as f64 * factorial(r as f64)?
    } else {
        result as f64
    })
}

// Hexadecimal strtod-style numbers, including optional binary exponents.
fn hex_number(input: &str, start: usize) -> Result<(f64, usize)> {
    let bytes = input.as_bytes();
    let mut i = start + 2;
    let mut digits = Vec::new();
    while let Some(digit) = bytes.get(i).and_then(|b| (*b as char).to_digit(16)) {
        digits.push(digit);
        i += 1;
    }
    let integer_digits = digits.len();
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        while let Some(digit) = bytes.get(i).and_then(|b| (*b as char).to_digit(16)) {
            digits.push(digit);
            i += 1;
        }
    }
    if digits.is_empty() {
        return Err(ExpressionError::Syntax);
    }
    let first = digits.iter().position(|digit| *digit != 0);
    let mut value = 0.0;
    let mut binary_exponent = 0_i32;
    if let Some(first) = first {
        binary_exponent = 4 * (integer_digits as i32 - first as i32 - 1);
        let mut place = 1.0;
        for digit in &digits[first..] {
            value += *digit as f64 * place;
            place /= 16.0;
        }
    }
    if bytes.get(i) == Some(&b'p') {
        i += 1;
        let negative = bytes.get(i) == Some(&b'-');
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let exponent_start = i;
        let mut exponent = 0_i32;
        while let Some(digit @ b'0'..=b'9') = bytes.get(i) {
            exponent = exponent
                .saturating_mul(10)
                .saturating_add((digit - b'0') as i32);
            i += 1;
        }
        if i == exponent_start {
            return Err(ExpressionError::Syntax);
        }
        if negative {
            exponent = -exponent;
        }
        binary_exponent = binary_exponent.saturating_add(exponent);
    }
    // Scale in bounded chunks so a representable subnormal or large result is
    // not lost to an overflowing/underflowing intermediate power of two.
    if value != 0.0 {
        if binary_exponent > 4096 {
            value = f64::INFINITY;
        } else if binary_exponent < -4096 {
            value = 0.0;
        } else {
            while binary_exponent > 512 {
                value *= 2.0_f64.powi(512);
                binary_exponent -= 512;
            }
            while binary_exponent < -512 {
                value *= 2.0_f64.powi(-512);
                binary_exponent += 512;
            }
            value *= 2.0_f64.powi(binary_exponent);
        }
    }
    Ok((value, i))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn near(expression: &str, expected: f64) {
        let value = evaluate(expression).unwrap();
        assert!(
            (value - expected).abs() < 1e-9 * expected.abs().max(1.0),
            "{expression}: {value} != {expected}"
        );
    }
    #[test]
    fn precedence_and_grammar() {
        for (text, value) in [
            ("-2^2", -4.0),
            ("(-2)^2", 4.0),
            ("(-1)^0", 1.0),
            ("2^3^2", 512.0),
            ("2^-2", 0.25),
            ("2^-2^2", 0.0625),
            ("0x10+0x1.8p2", 22.0),
            ("1+2*3%4", 3.0),
            ("1,2,(3,4)", 4.0),
            ("1e-3 + 2E+2", 200.001),
            ("−2 + +3", 1.0),
            ("sin 2^2", 2.0_f64.sin().powi(2)),
        ] {
            near(text, value);
        }
        for text in [
            "",
            "1 2",
            "1+",
            "pow(2)",
            "pow(2,3,4)",
            "sqrt()",
            "1e",
            "1.2.3",
            "pi(1)",
            "2pi",
        ] {
            assert!(evaluate(text).is_err(), "{text}");
        }
    }
    #[test]
    fn notes_and_conversions() {
        near("A4", 440.0);
        near("e", 440.0 * (-5.0_f64 / 12.0).exp2());
        for letter in ["a", "b", "c", "d", "e", "f", "g"] {
            for octave in 0..=9 {
                near(
                    &format!("{letter}#{octave}v"),
                    evaluate(&format!("{letter}s{octave}v")).unwrap(),
                );
            }
        }
        near("Cb4v", -1.0 / 12.0);
        near("Bs4v", 1.0);
        near("c", evaluate("C4").unwrap());
        near("ftov(vtof(2))", 2.0);
        near("bpmtov(vtobpm(-2))", -2.0);
        near("dbtogain(gaintodb(2))", 2.0);
        assert!(evaluate("c10").is_err());
    }
    #[test]
    fn functions_and_domains() {
        near("log(exp(1))", 1.0);
        near("ln(exp(1))", 1.0);
        near("log2(8)+log10(100)", 5.0);
        near("fac(5.9)+ncr(5,2)+npr(5,2)", 150.0);
        near("atan2(1,1)", std::f64::consts::FRAC_PI_4);
        near("abs -2 + ceil(1.2) + floor(1.9) + sqrt(4)", 7.0);
        near(
            "cos(0)+cosh(0)+sin(0)+sinh(0)+tan(0)+tanh(0)+asin(0)+acos(1)+atan(0)",
            2.0,
        );
        assert_eq!(evaluate("-INF"), Ok(f64::NEG_INFINITY));
        assert_eq!(evaluate("gaintodb(0)"), Ok(f64::NEG_INFINITY));
        near("dbtogain(-inf)", 0.0);
        near("exp(log(0))", 0.0);
        near("0x0p9999", 0.0);
        assert_eq!(evaluate("0x0.1p1024"), Ok(2.0_f64.powi(1020)));
        assert_eq!(evaluate("0x10p-1078"), Ok(f64::from_bits(1)));
        near("1/fac(21)", 0.0);
        near("npr(30,20)", 73_096_577_329_197_271_449_600_000.0);
        assert_eq!(evaluate("1/0"), Ok(f64::INFINITY));
        assert_eq!(evaluate("log(0)"), Ok(f64::NEG_INFINITY));
        for text in [
            "0/0",
            "sqrt(-1)",
            "fac(-1)",
            "ncr(2,3)",
            "ncr(1000,500)",
            "(-2)^0.5",
        ] {
            assert!(evaluate(text).is_err(), "{text}");
        }
    }
    #[test]
    fn complete_note_aliases_and_reference_constants() {
        for (letter, semitone) in [
            ('c', 0),
            ('d', 2),
            ('e', 4),
            ('f', 5),
            ('g', 7),
            ('a', 9),
            ('b', 11),
        ] {
            for (accidental, offset) in [("", 0), ("s", 1), ("#", 1), ("b", -1)] {
                for octave in 0..=9 {
                    let voltage = (octave - 4) as f64 + (semitone + offset) as f64 / 12.0;
                    let name = format!("{letter}{accidental}{octave}").to_uppercase();
                    near(&name, 440.0 * (voltage - 0.75).exp2());
                    near(&format!("{name}V"), voltage);
                    if octave == 4 {
                        near(&format!("{letter}{accidental}"), evaluate(&name).unwrap());
                        near(&format!("{letter}{accidental}v"), voltage);
                    }
                }
            }
        }
        assert_eq!(evaluate("vtof(0)"), Ok(261.6256_f32 as f64));
        assert_ne!(evaluate("vtof(0)"), evaluate("c4"));
    }
    #[test]
    fn reference_compositions_and_overflow() {
        near("sin(1,2)", 2.0_f64.sin());
        near("pow(2,(1,3))", 8.0);
        near("pi()", std::f64::consts::PI);
        for text in ["e()", "c4()", "ncr(5.1,5.9)"] {
            assert!(evaluate(text).is_err(), "{text}");
        }
        for text in ["fac(21)", "ncr(100,50)", "npr(30,21)", "1e999"] {
            assert_eq!(evaluate(text), Ok(f64::INFINITY), "{text}");
        }
        near("fac(20.9)", 2_432_902_008_176_640_000.0);
        assert_eq!(evaluate("abs(-inf)"), Ok(f64::INFINITY));
    }
    #[test]
    fn bounded_and_repeatable() {
        assert_eq!(evaluate(&"1".repeat(513)), Err(ExpressionError::Limit));
        assert_eq!(
            evaluate(&format!("{}1{}", "(".repeat(34), ")".repeat(34))),
            Err(ExpressionError::Limit)
        );
        assert_eq!(
            evaluate(&vec!["1"; 130].join("+")),
            Err(ExpressionError::Limit)
        );
        for _ in 0..100 {
            near("pow(2,3)+A4v+log(exp(1))", 9.75);
        }
    }
}
