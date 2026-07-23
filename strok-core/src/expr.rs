//! Arithmetic scalar expressions (C13).
//!
//! A tiny recursive-descent evaluator for **space-free** scalar expressions that
//! may appear anywhere a plain number is accepted on a scene-node or shape-op
//! line: coordinates, dimensions, `rotation=`, `gap=`, `round-corners`,
//! `addpoint … at=`.
//!
//! Grammar (standard precedence, left-associative):
//!
//! ```text
//! expr   := term (('+' | '-') term)*
//! term   := factor (('*' | '/' | '%') factor)*
//! factor := '-' factor | '(' expr ')' | number | '$' name
//! ```
//!
//! Expressions are **space-free** because the line tokenizer splits on spaces —
//! so `40+$i*60` arrives as a single token. `$name` resolves from an [`Env`]
//! (built from `let` bindings and `repeat` loop variables). An undefined name is
//! an error that lists / suggests the known names; division (or modulo) by zero
//! is an error.
//!
//! Expressions are evaluated **eagerly at parse time** — the [`crate::scene`]
//! stores plain `f64` numbers, never the expression source (except a `let`,
//! which keeps its source verbatim for round-trip).

use crate::diagnostics::suggest;
use crate::error::{Result, StrokError};
use std::collections::HashMap;

/// The evaluation environment: `$name → f64`. Holds `let` bindings and the
/// active `repeat` loop variables.
#[derive(Debug, Clone, Default)]
pub struct Env {
    vars: HashMap<String, f64>,
}

impl Env {
    pub fn new() -> Self {
        Env {
            vars: HashMap::new(),
        }
    }

    /// Bind (or rebind) `name` to `value`.
    pub fn set(&mut self, name: impl Into<String>, value: f64) {
        self.vars.insert(name.into(), value);
    }

    /// Look up a bound name.
    pub fn get(&self, name: &str) -> Option<f64> {
        self.vars.get(name).copied()
    }

    /// True if `name` is already bound (used to detect `repeat` var shadowing).
    pub fn contains(&self, name: &str) -> bool {
        self.vars.contains_key(name)
    }

    /// A child env with one extra binding (used per `repeat` iteration).
    pub fn child(&self, name: impl Into<String>, value: f64) -> Env {
        let mut c = self.clone();
        c.set(name, value);
        c
    }

    /// The known names, sorted (for stable "did you mean" / listing output).
    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.vars.keys().cloned().collect();
        v.sort();
        v
    }
}

/// Evaluate a scalar expression against `env`.
///
/// **Fast path:** a bare number (anything `f64::from_str` accepts) is returned
/// as-is, so plain numbers parse exactly as they did before expressions existed
/// (and re-emit byte-identically). Only non-numeric input runs the parser.
pub fn eval_scalar(s: &str, env: &Env) -> Result<f64> {
    let trimmed = s.trim();
    // Fast path: preserve pre-expression behavior for plain numbers exactly.
    if let Ok(n) = trimmed.parse::<f64>() {
        return Ok(n);
    }
    if trimmed.is_empty() {
        return Err(StrokError::ParseError(
            "empty scalar expression".to_string(),
        ));
    }
    let mut parser = ExprParser {
        chars: trimmed.chars().collect(),
        pos: 0,
        env,
    };
    let v = parser.expr()?;
    parser.skip_ws();
    if parser.pos != parser.chars.len() {
        return Err(StrokError::ParseError(format!(
            "unexpected trailing input in expression '{}'",
            trimmed
        )));
    }
    Ok(v)
}

struct ExprParser<'a> {
    chars: Vec<char>,
    pos: usize,
    env: &'a Env,
}

impl ExprParser<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.pos += 1;
        }
    }

    fn expr(&mut self) -> Result<f64> {
        let mut acc = self.term()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some('+') => {
                    self.pos += 1;
                    acc += self.term()?;
                }
                Some('-') => {
                    self.pos += 1;
                    acc -= self.term()?;
                }
                _ => break,
            }
        }
        Ok(acc)
    }

    fn term(&mut self) -> Result<f64> {
        let mut acc = self.factor()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some('*') => {
                    self.pos += 1;
                    acc *= self.factor()?;
                }
                Some('/') => {
                    self.pos += 1;
                    let rhs = self.factor()?;
                    if rhs == 0.0 {
                        return Err(StrokError::ParseError(
                            "division by zero in expression".to_string(),
                        ));
                    }
                    acc /= rhs;
                }
                Some('%') => {
                    self.pos += 1;
                    let rhs = self.factor()?;
                    if rhs == 0.0 {
                        return Err(StrokError::ParseError(
                            "modulo by zero in expression".to_string(),
                        ));
                    }
                    acc %= rhs;
                }
                _ => break,
            }
        }
        Ok(acc)
    }

    fn factor(&mut self) -> Result<f64> {
        self.skip_ws();
        match self.peek() {
            Some('-') => {
                self.pos += 1;
                Ok(-self.factor()?)
            }
            Some('+') => {
                self.pos += 1;
                self.factor()
            }
            Some('(') => {
                self.pos += 1;
                let v = self.expr()?;
                self.skip_ws();
                if self.peek() == Some(')') {
                    self.pos += 1;
                    Ok(v)
                } else {
                    Err(StrokError::ParseError(
                        "unbalanced parenthesis in expression".to_string(),
                    ))
                }
            }
            Some('$') => self.name_ref(),
            Some(c) if c.is_ascii_digit() || c == '.' => self.number(),
            Some(c) => Err(StrokError::ParseError(format!(
                "unexpected character '{}' in expression",
                c
            ))),
            None => Err(StrokError::ParseError(
                "unexpected end of expression".to_string(),
            )),
        }
    }

    fn number(&mut self) -> Result<f64> {
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit() || c == '.') {
            self.pos += 1;
        }
        let s: String = self.chars[start..self.pos].iter().collect();
        s.parse::<f64>()
            .map_err(|_| StrokError::ParseError(format!("invalid number '{}' in expression", s)))
    }

    fn name_ref(&mut self) -> Result<f64> {
        // Consume '$'
        self.pos += 1;
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            self.pos += 1;
        }
        let name: String = self.chars[start..self.pos].iter().collect();
        if name.is_empty() {
            return Err(StrokError::ParseError(
                "expected a name after '$' in expression".to_string(),
            ));
        }
        match self.env.get(&name) {
            Some(v) => Ok(v),
            None => {
                let known = self.env.names();
                let known_refs: Vec<&str> = known.iter().map(|s| s.as_str()).collect();
                let mut msg = format!("unknown name '${}' in expression", name);
                if let Some(s) = suggest(&name, &known_refs) {
                    msg.push_str(&format!(" (did you mean `${}`?)", s));
                } else if known_refs.is_empty() {
                    msg.push_str(" (no names are in scope)");
                } else {
                    msg.push_str(&format!(" (known: {})", known.join(", ")));
                }
                Err(StrokError::ParseError(msg))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(s: &str) -> f64 {
        eval_scalar(s, &Env::new()).unwrap()
    }

    #[test]
    fn plain_numbers_fast_path() {
        assert_eq!(ev("40"), 40.0);
        assert_eq!(ev("-5"), -5.0);
        assert_eq!(ev("3.5"), 3.5);
        assert_eq!(ev("0"), 0.0);
    }

    #[test]
    fn precedence_and_parens() {
        assert_eq!(ev("40+2*60"), 160.0);
        assert_eq!(ev("(40+2)*60"), 2520.0);
        assert_eq!(ev("100-10-5"), 85.0);
        assert_eq!(ev("10/2/5"), 1.0);
        assert_eq!(ev("7%3"), 1.0);
    }

    #[test]
    fn unary_minus() {
        assert_eq!(ev("-5*2"), -10.0);
        assert_eq!(ev("-(3+2)"), -5.0);
        assert_eq!(ev("10*-2"), -20.0);
    }

    #[test]
    fn let_refs() {
        let mut env = Env::new();
        env.set("col", 310.0);
        env.set("i", 2.0);
        assert_eq!(eval_scalar("$col", &env).unwrap(), 310.0);
        assert_eq!(eval_scalar("$col+$i*60", &env).unwrap(), 430.0);
        assert_eq!(eval_scalar("40+$i*60", &env).unwrap(), 160.0);
    }

    #[test]
    fn undefined_name_suggests() {
        let mut env = Env::new();
        env.set("col", 1.0);
        let err = eval_scalar("$cel", &env).unwrap_err().to_string();
        assert!(err.contains("unknown name '$cel'"), "{err}");
        assert!(err.contains("did you mean `$col`?"), "{err}");
    }

    #[test]
    fn division_by_zero() {
        let err = eval_scalar("5/0", &Env::new()).unwrap_err().to_string();
        assert!(err.contains("division by zero"), "{err}");
        let err = eval_scalar("5%0", &Env::new()).unwrap_err().to_string();
        assert!(err.contains("modulo by zero"), "{err}");
    }

    #[test]
    fn trailing_garbage_errors() {
        assert!(eval_scalar("40+", &Env::new()).is_err());
        assert!(eval_scalar("(40", &Env::new()).is_err());
        assert!(eval_scalar("40)", &Env::new()).is_err());
    }
}
