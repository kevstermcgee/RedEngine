//! A tiny expression language for rule conditions and values: `score >= 3 && !has_key`, `score + 1`, `time > 30`.
//!
//! Numbers are `f64` (IEEE, so results are identical on every platform); booleans are `1.0` / `0.0` and anything
//! non-zero is true. Operators, loosest to tightest: `||`, `&&`, `== !=`, `< <= > >=`, `+ -`, `* / %`, unary `- !`.
//! Division and remainder by zero give `0` (never NaN or infinity), so a rule can never poison the simulation state.
//! Identifiers must be **declared variables** (or the built-ins `time`, `tick`, `players`): an unknown name is a
//! parse error with a did-you-mean, which is how a typo in a rule is caught by `validate` instead of at play time.

use std::fmt;

/// A binary operator.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Op {
    /// `||`
    Or,
    /// `&&`
    And,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Rem,
}

/// A compiled expression over a variable table (variables are addressed by index).
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A literal (`true`/`false` are `1`/`0`).
    Num(f64),
    /// A variable, by index into the table it was compiled against.
    Var(usize),
    /// Unary minus.
    Neg(Box<Expr>),
    /// Logical not.
    Not(Box<Expr>),
    /// A binary operation.
    Bin(Op, Box<Expr>, Box<Expr>),
}

impl Expr {
    /// Evaluates against `vars` (indexes out of range read as `0`).
    pub fn eval(&self, vars: &[f64]) -> f64 {
        match self {
            Expr::Num(n) => *n,
            Expr::Var(i) => vars.get(*i).copied().unwrap_or(0.0),
            Expr::Neg(e) => -e.eval(vars),
            Expr::Not(e) => b(e.eval(vars) == 0.0),
            Expr::Bin(op, l, r) => {
                let a = l.eval(vars);
                match op {
                    Op::Or => return b(a != 0.0 || r.eval(vars) != 0.0),
                    Op::And => return b(a != 0.0 && r.eval(vars) != 0.0),
                    _ => {}
                }
                let c = r.eval(vars);
                match op {
                    Op::Eq => b(a == c),
                    Op::Ne => b(a != c),
                    Op::Lt => b(a < c),
                    Op::Le => b(a <= c),
                    Op::Gt => b(a > c),
                    Op::Ge => b(a >= c),
                    Op::Add => a + c,
                    Op::Sub => a - c,
                    Op::Mul => a * c,
                    Op::Div => {
                        if c == 0.0 {
                            0.0
                        } else {
                            a / c
                        }
                    }
                    Op::Rem => {
                        if c == 0.0 {
                            0.0
                        } else {
                            a % c
                        }
                    }
                    Op::Or | Op::And => 0.0, // handled above
                }
            }
        }
    }

    /// True when the value is non-zero.
    pub fn truthy(&self, vars: &[f64]) -> bool {
        self.eval(vars) != 0.0
    }
}

fn b(x: bool) -> f64 {
    if x {
        1.0
    } else {
        0.0
    }
}

/// A parse failure with the character offset it was found at.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    /// What is wrong (and, for a variable, the fix).
    pub message: String,
    /// Offset in the source text.
    pub at: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at character {})", self.message, self.at)
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Op(Op),
    Not,
    LParen,
    RParen,
}

fn lex(src: &str) -> Result<Vec<(Tok, usize)>, ParseError> {
    let cs: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        let at = i;
        let two = |a: char, b: char| c == a && cs.get(i + 1) == Some(&b);
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || (c == '.' && cs.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            while i < cs.len() && (cs[i].is_ascii_digit() || cs[i] == '.') {
                i += 1;
            }
            let text: String = cs[start..i].iter().collect();
            let n = text.parse::<f64>().map_err(|_| ParseError { message: format!("`{text}` is not a number"), at })?;
            out.push((Tok::Num(n), at));
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < cs.len() && (cs[i].is_alphanumeric() || cs[i] == '_') {
                i += 1;
            }
            out.push((Tok::Ident(cs[start..i].iter().collect()), at));
        } else {
            let (tok, len) = if two('&', '&') {
                (Tok::Op(Op::And), 2)
            } else if two('|', '|') {
                (Tok::Op(Op::Or), 2)
            } else if two('=', '=') {
                (Tok::Op(Op::Eq), 2)
            } else if two('!', '=') {
                (Tok::Op(Op::Ne), 2)
            } else if two('<', '=') {
                (Tok::Op(Op::Le), 2)
            } else if two('>', '=') {
                (Tok::Op(Op::Ge), 2)
            } else {
                match c {
                    '<' => (Tok::Op(Op::Lt), 1),
                    '>' => (Tok::Op(Op::Gt), 1),
                    '+' => (Tok::Op(Op::Add), 1),
                    '-' => (Tok::Op(Op::Sub), 1),
                    '*' => (Tok::Op(Op::Mul), 1),
                    '/' => (Tok::Op(Op::Div), 1),
                    '%' => (Tok::Op(Op::Rem), 1),
                    '!' => (Tok::Not, 1),
                    '(' => (Tok::LParen, 1),
                    ')' => (Tok::RParen, 1),
                    other => {
                        return Err(ParseError { message: format!("unexpected `{other}` (operators: || && == != < <= > >= + - * / % ! and parentheses)"), at })
                    }
                }
            };
            i += len;
            out.push((tok, at));
        }
    }
    Ok(out)
}

struct Parser<'a> {
    toks: Vec<(Tok, usize)>,
    pos: usize,
    names: &'a [String],
    end: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.0)
    }
    fn at(&self) -> usize {
        self.toks.get(self.pos).map_or(self.end, |t| t.1)
    }
    fn binary(&mut self, level: usize) -> Result<Expr, ParseError> {
        const LEVELS: &[&[Op]] =
            &[&[Op::Or], &[Op::And], &[Op::Eq, Op::Ne], &[Op::Lt, Op::Le, Op::Gt, Op::Ge], &[Op::Add, Op::Sub], &[Op::Mul, Op::Div, Op::Rem]];
        let Some(ops) = LEVELS.get(level) else { return self.unary() };
        let mut left = self.binary(level + 1)?;
        while let Some(Tok::Op(op)) = self.peek().cloned() {
            if !ops.contains(&op) {
                break;
            }
            self.pos += 1;
            let right = self.binary(level + 1)?;
            left = Expr::Bin(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    fn unary(&mut self) -> Result<Expr, ParseError> {
        match self.peek() {
            Some(Tok::Not) => {
                self.pos += 1;
                Ok(Expr::Not(Box::new(self.unary()?)))
            }
            Some(Tok::Op(Op::Sub)) => {
                self.pos += 1;
                Ok(Expr::Neg(Box::new(self.unary()?)))
            }
            _ => self.atom(),
        }
    }
    fn atom(&mut self) -> Result<Expr, ParseError> {
        let at = self.at();
        match self.toks.get(self.pos).map(|t| t.0.clone()) {
            Some(Tok::Num(n)) => {
                self.pos += 1;
                Ok(Expr::Num(n))
            }
            Some(Tok::Ident(name)) => {
                self.pos += 1;
                match name.as_str() {
                    "true" => Ok(Expr::Num(1.0)),
                    "false" => Ok(Expr::Num(0.0)),
                    _ => match self.names.iter().position(|n| *n == name) {
                        Some(i) => Ok(Expr::Var(i)),
                        None => Err(ParseError { message: unknown_var(&name, self.names), at }),
                    },
                }
            }
            Some(Tok::LParen) => {
                self.pos += 1;
                let e = self.binary(0)?;
                if self.peek() == Some(&Tok::RParen) {
                    self.pos += 1;
                    Ok(e)
                } else {
                    Err(ParseError { message: "missing `)`".to_string(), at: self.at() })
                }
            }
            Some(_) => Err(ParseError { message: "expected a number, a variable or `(`".to_string(), at }),
            None => Err(ParseError { message: "the expression ends too soon".to_string(), at }),
        }
    }
}

/// The message for a name that is not a declared variable, with a did-you-mean when one is close.
pub fn unknown_var(name: &str, names: &[String]) -> String {
    let near = crate::prefabs::suggest(name, names.iter().map(String::as_str));
    let hint = near.first().map(|n| format!(" — did you mean `{n}`?")).unwrap_or_default();
    format!("unknown variable `{name}`{hint} (declare it in `vars`; known: {})", names.join(", "))
}

/// Compiles `src` against the variable `names` (a variable's index is its position in `names`).
pub fn parse(src: &str, names: &[String]) -> Result<Expr, ParseError> {
    let toks = lex(src)?;
    if toks.is_empty() {
        return Err(ParseError { message: "the expression is empty".to_string(), at: 0 });
    }
    let mut p = Parser { toks, pos: 0, names, end: src.chars().count() };
    let e = p.binary(0)?;
    if p.pos < p.toks.len() {
        return Err(ParseError { message: "unexpected trailing text (missing an operator?)".to_string(), at: p.at() });
    }
    Ok(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        ["score", "has_key", "time"].iter().map(|s| s.to_string()).collect()
    }
    fn eval(src: &str, vars: &[f64]) -> f64 {
        parse(src, &names()).unwrap_or_else(|e| panic!("{src}: {e}")).eval(vars)
    }

    #[test]
    fn arithmetic_comparison_and_logic_follow_the_usual_precedence() {
        let v = [3.0, 0.0, 12.5];
        assert_eq!(eval("1 + 2 * 3", &v), 7.0);
        assert_eq!(eval("(1 + 2) * 3", &v), 9.0);
        assert_eq!(eval("score >= 3 && !has_key", &v), 1.0);
        assert_eq!(eval("score < 3 || has_key", &v), 0.0);
        assert_eq!(eval("score == 3", &v), 1.0);
        assert_eq!(eval("score != 3", &v), 0.0);
        assert_eq!(eval("-score + 10 % 4", &v), -1.0);
        assert_eq!(eval("time / 2", &v), 6.25);
        assert_eq!(eval("true && !false", &v), 1.0);
    }

    #[test]
    fn division_by_zero_is_zero_not_nan() {
        assert_eq!(eval("score / 0", &[3.0, 0.0, 0.0]), 0.0);
        assert_eq!(eval("score % 0", &[3.0, 0.0, 0.0]), 0.0);
    }

    #[test]
    fn a_misspelled_variable_names_the_fix() {
        let e = parse("scor >= 3", &names()).unwrap_err();
        assert!(e.message.contains("unknown variable `scor`") && e.message.contains("did you mean `score`"), "{e}");
    }

    #[test]
    fn syntax_errors_say_what_and_where() {
        assert!(parse("score >=", &names()).unwrap_err().message.contains("ends too soon"));
        assert!(parse("(score", &names()).unwrap_err().message.contains("missing `)`"));
        assert!(parse("score score", &names()).unwrap_err().message.contains("trailing"));
        assert!(parse("", &names()).unwrap_err().message.contains("empty"));
        assert!(parse("score $ 3", &names()).unwrap_err().message.contains("unexpected `$`"));
    }

    #[test]
    fn evaluation_is_deterministic_and_short_circuits() {
        let e = parse("score > 0 && time / score > 2", &names()).unwrap();
        assert_eq!(e.eval(&[0.0, 0.0, 9.0]), 0.0);
        assert_eq!(e.eval(&[2.0, 0.0, 9.0]), 1.0);
    }
}
