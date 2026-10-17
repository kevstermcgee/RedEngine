//! A tiny expression language for rule conditions and values: `score >= 3 && !has_key`, `score + 1`, `time > 30`.
//!
//! Numbers are `f64` (IEEE, so results are identical on every platform); booleans are `1.0` / `0.0` and anything
//! non-zero is true. Operators, loosest to tightest: `||`, `&&`, `== !=`, `< <= > >=`, `+ -`, `* / %`, unary `- !`.
//! Division and remainder by zero give `0` (never NaN or infinity), so a rule can never poison the simulation state.
//! Identifiers must be **declared variables** (or the built-ins `time`, `tick`, `players`): an unknown name is a
//! parse error with a did-you-mean, which is how a typo in a rule is caught by `validate` instead of at play time.
//! Built-in **functions** read the world: `prop_y(crate)`, `tilt(domino_3) > 60`, `held(parcel)`, `mass(barrel)`,
//! `moved(bell)`, `props_in(pit)`, `in_zone(crate, pit)`; their argument is a loose prop's (or a zone's) id from the [`Scope`] the expression
//! is compiled in, and a [`World`] answers them at evaluation time (ADR 2026-09-29-prop-aware-rules-and-scenarios). In a scope with loose props,
//! `self` stands for the prop that triggered the rule (`held(self)`, `in_zone(self, pit)`): it compiles to [`SUBJECT`] and the world answers for
//! whichever prop that is when the rule runs (ADR 2026-10-13-aimed-fields-and-the-rule-s-subject-prop).
//! **Math** over expressions: `floor(x) round(x) abs(x) sqrt(x) min(a, b) max(a, b)`. **`random(a, b)`** is a whole number from `a` to `b` inclusive,
//! drawn from the simulation's seeded generator through the [`World`] (so a run with the same seed and inputs draws the same numbers; ADR
//! 2026-10-10-story-is-rules-and-tables-not-a-story-system).

use std::fmt;

/// The prop index `self` compiles to: the rule's subject, whichever prop that is when it runs ([`World::call`] receives it as the index).
pub const SUBJECT: usize = usize::MAX;

/// A built-in function over the world. The prop functions take a loose prop's object id; `props_in` takes a zone id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Func {
    /// `prop_y(id)`: height of the prop's origin, m.
    PropY,
    /// `tilt(id)`: degrees the prop's up axis leans from vertical (0 upright, 90 on its side).
    Tilt,
    /// `held(id)`: 1 while a player carries it, else 0.
    Held,
    /// `mass(id)`: kg.
    Mass,
    /// `moved(id)`: metres its origin is from where the map author put it.
    Moved,
    /// `props_in(zone)`: how many loose props are inside the zone.
    PropsIn,
    /// `in_zone(prop, zone)`: 1 when that one loose prop is inside the zone, else 0.
    InZone,
}

impl Func {
    /// Every built-in: `(name, function, help line)`, for the parser and `describe rules`.
    pub const ALL: &[(&str, Func, &str)] = &[
        ("prop_y", Func::PropY, "prop_y(id)      height of a loose prop's origin, m"),
        ("tilt", Func::Tilt, "tilt(id)        degrees its up axis leans from vertical (0 upright, 90 on its side)"),
        ("held", Func::Held, "held(id)        1 while a player carries it"),
        ("mass", Func::Mass, "mass(id)        its mass, kg"),
        ("moved", Func::Moved, "moved(id)       metres its origin is from where the map put it"),
        ("props_in", Func::PropsIn, "props_in(zone)  how many loose props have their origin inside the zone"),
        ("in_zone", Func::InZone, "in_zone(id, zone)  1 when that loose prop has its origin inside the zone (same test as props_in, for one prop)"),
    ];

    fn by_name(name: &str) -> Option<Func> {
        Func::ALL.iter().find(|(n, _, _)| *n == name).map(|(_, f, _)| *f)
    }

    /// The function's name in the language.
    pub fn name(self) -> &'static str {
        Func::ALL.iter().find(|(_, f, _)| *f == self).map_or("?", |(n, _, _)| n)
    }

    /// True when the argument is a zone id rather than a loose prop id.
    pub fn takes_zone(self) -> bool {
        matches!(self, Func::PropsIn)
    }

    /// True when the call names a loose prop and then a zone (`in_zone(prop, zone)`).
    pub fn takes_prop_and_zone(self) -> bool {
        matches!(self, Func::InZone)
    }
}

/// A math function over expressions (no world needed): `(name, function, arguments, help line)` in [`MathFn::ALL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathFn {
    Floor,
    Round,
    Abs,
    Sqrt,
    Min,
    Max,
}

impl MathFn {
    /// Every math function: `(name, function, argument count, help line)`, for the parser and `describe rules` / `describe 2d`.
    pub const ALL: &[(&str, MathFn, usize, &str)] = &[
        ("floor", MathFn::Floor, 1, "floor(x)        the whole number at or below x (floor(2.7) = 2, floor(-0.5) = -1)"),
        ("round", MathFn::Round, 1, "round(x)        the nearest whole number (halves away from zero)"),
        ("abs", MathFn::Abs, 1, "abs(x)          x without its sign"),
        ("sqrt", MathFn::Sqrt, 1, "sqrt(x)         square root (0 for a negative x)"),
        ("min", MathFn::Min, 2, "min(a, b)       the smaller"),
        ("max", MathFn::Max, 2, "max(a, b)       the larger"),
    ];

    fn by_name(name: &str) -> Option<(MathFn, usize)> {
        MathFn::ALL.iter().find(|(n, _, _, _)| *n == name).map(|(_, f, a, _)| (*f, *a))
    }

    fn apply(self, a: &[f64]) -> f64 {
        let x = a.first().copied().unwrap_or(0.0);
        let y = a.get(1).copied().unwrap_or(0.0);
        match self {
            MathFn::Floor => x.floor(),
            MathFn::Round => x.round(),
            MathFn::Abs => x.abs(),
            MathFn::Sqrt => {
                if x > 0.0 {
                    x.sqrt()
                } else {
                    0.0
                }
            }
            MathFn::Min => x.min(y),
            MathFn::Max => x.max(y),
        }
    }
}

/// The help line of `random`, for `describe rules` / `describe 2d`.
pub const RANDOM_HELP: &str = "random(a, b)    a whole number from a to b inclusive, from the game's seeded generator (same seed, same draws)";

/// One draw of `random(a, b)`: a whole number from `round(min(a,b))` to `round(max(a,b))` inclusive, advancing `state` (xorshift64*). The simulations keep
/// `state` and call this through their [`World`]; a range wider than 2^31 or a non-finite bound gives the low end.
pub fn draw_int(state: &mut u64, a: f64, b: f64) -> f64 {
    let (lo, hi) = (a.min(b).round(), a.max(b).round());
    if !lo.is_finite() || !hi.is_finite() || hi - lo > 2_147_483_647.0 {
        return if lo.is_finite() { lo } else { 0.0 };
    }
    let mut x = if *state == 0 { 0x9E37_79B9_7F4A_7C15 } else { *state };
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *state = x;
    let r = x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11;
    let n = (hi - lo) as u64 + 1;
    lo + (r % n) as f64
}

/// What a function call may name: the loose props and the zones of the scene, by id. A call is compiled to an index
/// into the matching list, so evaluating it never touches a string.
#[derive(Debug, Clone, Copy, Default)]
pub struct Scope<'a> {
    /// Loose prop object ids (`Func` prop arguments resolve to an index into this).
    pub props: &'a [String],
    /// Zone ids (`props_in` resolves to an index into this).
    pub zones: &'a [String],
    /// Per-player variable names (`me.name` resolves to an index into this).
    pub player_vars: &'a [String],
    /// Per-prop variable names (`self.name` resolves to an index into this).
    pub prop_vars: &'a [String],
}

/// Answers the built-in functions at evaluation time: `index` is the position in the [`Scope`] the expression was
/// compiled against (a prop for the prop functions, a zone for `props_in`). `()` answers 0 to everything.
pub trait World {
    /// The value of `f` for the prop or zone at `index` ([`SUBJECT`] for `self`: the rule's subject prop, 0 when it has none).
    fn call(&self, f: Func, index: usize) -> f64;

    /// The value of the two-argument `f` (`in_zone`) for the prop at `prop` and the zone at `zone`; 0 by default.
    fn call2(&self, _f: Func, _prop: usize, _zone: usize) -> f64 {
        0.0
    }

    /// The value of the per-player variable at `index` (`me.name`) for the player that triggered the rule; 0 by default and when no player did.
    fn me(&self, _index: usize) -> f64 {
        0.0
    }

    /// The value of the per-prop variable at `index` (`self.name`) of the rule's subject prop; 0 by default and when the rule has none.
    fn its(&self, _index: usize) -> f64 {
        0.0
    }

    /// `random(lo, hi)`: a simulation draws from its seeded generator ([`draw_int`]); with no generator (a static check) it is the low end.
    fn random(&self, lo: f64, hi: f64) -> f64 {
        lo.min(hi).round()
    }
}

impl World for () {
    fn call(&self, _: Func, _: usize) -> f64 {
        0.0
    }
}

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
    /// A per-player variable of the player that triggered the rule (`me.name`), by index into the scene's `player_vars`.
    Me(usize),
    /// A per-prop variable of the rule's subject prop (`self.name`), by index into the scene's `prop_vars`.
    Its(usize),
    /// Unary minus.
    Neg(Box<Expr>),
    /// Logical not.
    Not(Box<Expr>),
    /// A binary operation.
    Bin(Op, Box<Expr>, Box<Expr>),
    /// A built-in function of the prop (or zone) at this index of the [`Scope`] it was compiled in.
    Call(Func, usize),
    /// A built-in of a prop and a zone (`in_zone`): the prop's index in the scope's props, then the zone's in its zones.
    Call2(Func, usize, usize),
    /// A math function of its arguments.
    Math(MathFn, Vec<Expr>),
    /// `random(a, b)`: asks the [`World`] for a draw.
    Random(Box<Expr>, Box<Expr>),
}

impl Expr {
    /// Evaluates against `vars` with no world (every function call reads as `0`); see [`eval_in`](Self::eval_in).
    pub fn eval(&self, vars: &[f64]) -> f64 {
        self.eval_in(vars, &())
    }

    /// Evaluates against `vars` (indexes out of range read as `0`), asking `world` for the built-in functions.
    pub fn eval_in(&self, vars: &[f64], world: &dyn World) -> f64 {
        match self {
            Expr::Num(n) => *n,
            Expr::Var(i) => vars.get(*i).copied().unwrap_or(0.0),
            Expr::Me(i) => world.me(*i),
            Expr::Its(i) => world.its(*i),
            Expr::Call(f, i) => world.call(*f, *i),
            Expr::Call2(f, p, z) => world.call2(*f, *p, *z),
            Expr::Math(f, args) => {
                let a: Vec<f64> = args.iter().map(|e| e.eval_in(vars, world)).collect();
                f.apply(&a)
            }
            Expr::Random(lo, hi) => {
                let (a, c) = (lo.eval_in(vars, world), hi.eval_in(vars, world));
                world.random(a, c)
            }
            Expr::Neg(e) => -e.eval_in(vars, world),
            Expr::Not(e) => b(e.eval_in(vars, world) == 0.0),
            Expr::Bin(op, l, r) => {
                let a = l.eval_in(vars, world);
                match op {
                    Op::Or => return b(a != 0.0 || r.eval_in(vars, world) != 0.0),
                    Op::And => return b(a != 0.0 && r.eval_in(vars, world) != 0.0),
                    _ => {}
                }
                let c = r.eval_in(vars, world);
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

    /// True when the value is non-zero (no world: function calls read as `0`).
    pub fn truthy(&self, vars: &[f64]) -> bool {
        self.eval(vars) != 0.0
    }

    /// True when the value is non-zero, asking `world` for the built-in functions.
    pub fn truthy_in(&self, vars: &[f64], world: &dyn World) -> bool {
        self.eval_in(vars, world) != 0.0
    }

    /// Whether the expression reads a per-player variable (`me.name`), so it needs a triggering player.
    pub fn uses_me(&self) -> bool {
        match self {
            Expr::Me(_) => true,
            Expr::Num(_) | Expr::Var(_) | Expr::Its(_) | Expr::Call(..) | Expr::Call2(..) => false,
            Expr::Neg(e) | Expr::Not(e) => e.uses_me(),
            Expr::Bin(_, l, r) | Expr::Random(l, r) => l.uses_me() || r.uses_me(),
            Expr::Math(_, args) => args.iter().any(Expr::uses_me),
        }
    }

    /// Whether the expression reads the rule's subject prop (`self`), so it needs a trigger that has one.
    pub fn uses_subject(&self) -> bool {
        match self {
            Expr::Call(_, i) | Expr::Call2(_, i, _) => *i == SUBJECT,
            Expr::Its(_) => true,
            Expr::Num(_) | Expr::Var(_) | Expr::Me(_) => false,
            Expr::Neg(e) | Expr::Not(e) => e.uses_subject(),
            Expr::Bin(_, l, r) | Expr::Random(l, r) => l.uses_subject() || r.uses_subject(),
            Expr::Math(_, args) => args.iter().any(Expr::uses_subject),
        }
    }

    /// Pushes the index of every variable the expression reads (into the table it was compiled against) onto `out`, in reading order, repeats included.
    pub fn read_vars(&self, out: &mut Vec<usize>) {
        match self {
            Expr::Var(i) => out.push(*i),
            Expr::Num(_) | Expr::Me(_) | Expr::Call(..) | Expr::Call2(..) => {}
            Expr::Neg(e) | Expr::Not(e) => e.read_vars(out),
            Expr::Bin(_, l, r) | Expr::Random(l, r) => {
                l.read_vars(out);
                r.read_vars(out);
            }
            Expr::Math(_, args) => args.iter().for_each(|a| a.read_vars(out)),
        }
    }

    /// Whether the expression calls any built-in function (so evaluating it needs a real [`World`]).
    pub fn reads_world(&self) -> bool {
        match self {
            Expr::Num(_) | Expr::Var(_) => false,
            Expr::Me(_) | Expr::Its(_) => false,
            Expr::Call(..) | Expr::Call2(..) | Expr::Random(..) => true,
            Expr::Neg(e) | Expr::Not(e) => e.reads_world(),
            Expr::Bin(_, l, r) => l.reads_world() || r.reads_world(),
            Expr::Math(_, args) => args.iter().any(Expr::reads_world),
        }
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
    Comma,
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
            // `me.name` (a per-player variable) and `self.name` (the subject prop's): one identifier.
            let head: String = cs[start..i].iter().collect();
            if (head == "me" || head == "self") && cs.get(i) == Some(&'.') && cs.get(i + 1).is_some_and(|d| d.is_alphabetic() || *d == '_') {
                i += 1;
                while i < cs.len() && (cs[i].is_alphanumeric() || cs[i] == '_') {
                    i += 1;
                }
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
                    ',' => (Tok::Comma, 1),
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
    scope: Scope<'a>,
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
                if self.peek() == Some(&Tok::LParen) {
                    return self.call(&name, at);
                }
                if let Some(pv) = name.strip_prefix("me.") {
                    return match self.scope.player_vars.iter().position(|n| n == pv) {
                        Some(i) => Ok(Expr::Me(i)),
                        None => Err(ParseError { message: unknown_player_var(pv, self.scope.player_vars), at }),
                    };
                }
                if let Some(pv) = name.strip_prefix("self.") {
                    return match self.scope.prop_vars.iter().position(|n| n == pv) {
                        Some(i) => Ok(Expr::Its(i)),
                        None => Err(ParseError { message: unknown_prop_var(pv, self.scope.prop_vars), at }),
                    };
                }
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

    /// `name(` was read: parse `id )` and resolve the call against the scope; math functions and `random` take expressions instead.
    fn call(&mut self, name: &str, at: usize) -> Result<Expr, ParseError> {
        if name == "random" {
            let mut args = self.expr_args(name, 2, at)?.into_iter();
            if let (Some(lo), Some(hi)) = (args.next(), args.next()) {
                return Ok(Expr::Random(Box::new(lo), Box::new(hi)));
            }
            return Err(ParseError { message: "`random(a, b)` takes 2 arguments".into(), at });
        }
        if let Some((f, n)) = MathFn::by_name(name) {
            return Ok(Expr::Math(f, self.expr_args(name, n, at)?));
        }
        let Some(f) = Func::by_name(name) else {
            let mut known: Vec<&str> = Func::ALL.iter().map(|(n, _, _)| *n).collect();
            known.extend(MathFn::ALL.iter().map(|(n, _, _, _)| *n));
            known.push("random");
            let hint = crate::suggest::suggest(name, known.iter().copied()).first().map(|n| format!(" — did you mean `{n}`?")).unwrap_or_default();
            return Err(ParseError { message: format!("unknown function `{name}`{hint} (built-ins: {})", known.join(", ")), at });
        };
        self.pos += 1; // the `(`
        let arg_at = self.at();
        let Some(Tok::Ident(arg)) = self.toks.get(self.pos).map(|t| t.0.clone()) else {
            return Err(ParseError { message: format!("`{name}(` needs a {} id, then `)`", if f.takes_zone() { "zone" } else { "loose prop" }), at: arg_at });
        };
        self.pos += 1;
        if f.takes_prop_and_zone() {
            return self.call_prop_and_zone(f, name, &arg, arg_at);
        }
        if self.peek() != Some(&Tok::RParen) {
            return Err(ParseError { message: format!("missing `)` after `{name}({arg}`"), at: self.at() });
        }
        self.pos += 1;
        let (kind, list) = if f.takes_zone() { ("zone", self.scope.zones) } else { ("loose prop", self.scope.props) };
        if arg == "self" && !f.takes_zone() && !list.is_empty() {
            return Ok(Expr::Call(f, SUBJECT));
        }
        match list.iter().position(|n| *n == arg) {
            Some(i) => Ok(Expr::Call(f, i)),
            None => {
                let hint =
                    crate::suggest::suggest(&arg, list.iter().map(String::as_str)).first().map(|n| format!(" — did you mean `{n}`?")).unwrap_or_default();
                let known =
                    if list.is_empty() { "none in this scene".to_string() } else { list.iter().take(12).map(String::as_str).collect::<Vec<_>>().join(", ") };
                Err(ParseError { message: format!("`{name}({arg})`: no {kind} `{arg}`{hint} ({kind}s: {known})"), at: arg_at })
            }
        }
    }
}

impl Parser<'_> {
    /// `name(` is next: parse exactly `n` comma-separated expressions and the `)`.
    fn expr_args(&mut self, name: &str, n: usize, at: usize) -> Result<Vec<Expr>, ParseError> {
        self.pos += 1; // the `(`
        let mut args = Vec::new();
        loop {
            if self.peek() == Some(&Tok::RParen) && args.is_empty() {
                break;
            }
            args.push(self.binary(0)?);
            match self.peek() {
                Some(Tok::Comma) => self.pos += 1,
                Some(Tok::RParen) => break,
                _ => return Err(ParseError { message: format!("missing `)` or `,` in `{name}(...)`"), at: self.at() }),
            }
        }
        self.pos += 1; // the `)`
        if args.len() != n {
            let shape = if n == 1 { format!("`{name}(x)`") } else { format!("`{name}(a, b)`") };
            return Err(ParseError { message: format!("{shape} takes {n} argument(s), got {}", args.len()), at });
        }
        Ok(args)
    }

    /// `in_zone(prop` was read: parse `, zone )` and resolve both ids against the scope.
    fn call_prop_and_zone(&mut self, f: Func, name: &str, prop: &str, prop_at: usize) -> Result<Expr, ParseError> {
        if self.peek() != Some(&Tok::Comma) {
            return Err(ParseError { message: format!("`{name}(` needs a loose prop id, a comma, then a zone id: `{name}({prop}, zone)`"), at: self.at() });
        }
        self.pos += 1;
        let zone_at = self.at();
        let Some(Tok::Ident(zone)) = self.toks.get(self.pos).map(|t| t.0.clone()) else {
            return Err(ParseError { message: format!("`{name}({prop}, ` needs a zone id, then `)`"), at: zone_at });
        };
        self.pos += 1;
        if self.peek() != Some(&Tok::RParen) {
            return Err(ParseError { message: format!("missing `)` after `{name}({prop}, {zone}`"), at: self.at() });
        }
        self.pos += 1;
        let lookup = |kind: &str, list: &[String], id: &str, at: usize| match list.iter().position(|n| n == id) {
            Some(i) => Ok(i),
            None => {
                let hint = crate::suggest::suggest(id, list.iter().map(String::as_str)).first().map(|n| format!(" — did you mean `{n}`?")).unwrap_or_default();
                let known =
                    if list.is_empty() { "none in this scene".to_string() } else { list.iter().take(12).map(String::as_str).collect::<Vec<_>>().join(", ") };
                Err(ParseError { message: format!("`{name}({prop}, {zone})`: no {kind} `{id}`{hint} ({kind}s: {known})"), at })
            }
        };
        let p = if prop == "self" && !self.scope.props.is_empty() { SUBJECT } else { lookup("loose prop", self.scope.props, prop, prop_at)? };
        let z = lookup("zone", self.scope.zones, &zone, zone_at)?;
        Ok(Expr::Call2(f, p, z))
    }
}

/// The message for a name that is not a declared variable, with a did-you-mean when one is close.
pub fn unknown_var(name: &str, names: &[String]) -> String {
    let near = crate::suggest::suggest(name, names.iter().map(String::as_str));
    let hint = near.first().map(|n| format!(" — did you mean `{n}`?")).unwrap_or_default();
    format!("unknown variable `{name}`{hint} (declare it in `vars`; known: {})", names.join(", "))
}

/// The message for `self.name` where `name` is not a declared per-prop variable.
pub fn unknown_prop_var(name: &str, names: &[String]) -> String {
    let near = crate::suggest::suggest(name, names.iter().map(String::as_str));
    let hint = near.first().map(|n| format!(" — did you mean `self.{n}`?")).unwrap_or_default();
    let known = if names.is_empty() { "none declared".to_string() } else { names.join(", ") };
    format!("unknown per-prop variable `self.{name}`{hint} (declare it in `prop_vars`; known: {known})")
}

/// The message for `me.name` where `name` is not a declared per-player variable.
pub fn unknown_player_var(name: &str, names: &[String]) -> String {
    let near = crate::suggest::suggest(name, names.iter().map(String::as_str));
    let hint = near.first().map(|n| format!(" — did you mean `me.{n}`?")).unwrap_or_default();
    let known = if names.is_empty() { "none declared".to_string() } else { names.join(", ") };
    format!("unknown per-player variable `me.{name}`{hint} (declare it in `player_vars`; known: {known})")
}

/// Compiles `src` against the variable `names` (a variable's index is its position in `names`) with no props or zones
/// in scope: any function call is an error. See [`parse_in`].
pub fn parse(src: &str, names: &[String]) -> Result<Expr, ParseError> {
    parse_in(src, names, Scope::default())
}

/// Compiles `src` against the variable `names` and the loose props / zones of `scope` (a call's index is the id's
/// position in the scope's list).
pub fn parse_in(src: &str, names: &[String], scope: Scope<'_>) -> Result<Expr, ParseError> {
    let toks = lex(src)?;
    if toks.is_empty() {
        return Err(ParseError { message: "the expression is empty".to_string(), at: 0 });
    }
    let mut p = Parser { toks, pos: 0, names, scope, end: src.chars().count() };
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

    struct Fake;
    impl World for Fake {
        fn call(&self, f: Func, index: usize) -> f64 {
            match (f, index) {
                (Func::PropY, 1) => -2.5,
                (Func::Tilt, 0) => 72.0,
                (Func::Held, 0) => 1.0,
                (Func::Mass, 1) => 21.0,
                (Func::Moved, 1) => 3.2,
                (Func::PropsIn, 0) => 2.0,
                _ => 0.0,
            }
        }
        fn call2(&self, f: Func, prop: usize, zone: usize) -> f64 {
            // the crate (1) is in the pit (0); the bell (0) is not
            if f == Func::InZone && prop == 1 && zone == 0 {
                7.0
            } else {
                0.0
            }
        }
    }

    #[test]
    fn built_in_functions_resolve_ids_in_scope_and_read_the_world() {
        let props = vec!["bell".to_string(), "crate".to_string()];
        let zones = vec!["pit".to_string()];
        let scope = Scope { props: &props, zones: &zones, ..Default::default() };
        let e = |src: &str| parse_in(src, &names(), scope).unwrap_or_else(|e| panic!("{src}: {e}"));
        assert_eq!(e("prop_y(crate)").eval_in(&[0.0; 3], &Fake), -2.5);
        assert_eq!(e("tilt(bell) > 60 && held(bell)").eval_in(&[0.0; 3], &Fake), 1.0);
        assert_eq!(e("mass(crate) * 2 + props_in(pit)").eval_in(&[0.0; 3], &Fake), 44.0);
        assert_eq!(e("moved(crate) > 1 && score == 3").eval_in(&[3.0, 0.0, 0.0], &Fake), 1.0);
        assert_eq!(e("prop_y(crate)").eval(&[0.0; 3]), 0.0, "without a world every call reads as 0");
        assert!(e("prop_y(crate) < 0").reads_world() && !e("score > 1").reads_world());
        assert_eq!(e("prop_y(crate)"), Expr::Call(Func::PropY, 1));
        assert_eq!(e("in_zone(crate, pit)"), Expr::Call2(Func::InZone, 1, 0));
        assert_eq!(e("3 * in_zone(bell, pit) + 1").eval_in(&[0.0; 3], &Fake), 1.0, "bell is not in the pit");
        assert_eq!(e("2 * in_zone(crate, pit)").eval_in(&[0.0; 3], &Fake), 14.0);
        assert!(e("in_zone(crate, pit)").reads_world());
    }

    #[test]
    fn a_bad_function_call_names_the_fix() {
        let props = vec!["bell".to_string()];
        let zones = vec!["pit".to_string()];
        let scope = Scope { props: &props, zones: &zones, ..Default::default() };
        let err = |src: &str| parse_in(src, &names(), scope).unwrap_err().message;
        assert!(err("prop_y(bel)").contains("no loose prop `bel` — did you mean `bell`?"), "{}", err("prop_y(bel)"));
        assert!(err("props_in(bell)").contains("no zone `bell`"), "{}", err("props_in(bell)"));
        assert!(err("tilted(bell)").contains("unknown function `tilted` — did you mean `tilt`?"), "{}", err("tilted(bell)"));
        assert!(err("tilt(bell").contains("missing `)`"), "{}", err("tilt(bell"));
        assert!(err("tilt()").contains("needs a loose prop id"), "{}", err("tilt()"));
        assert!(err("in_zone(bel, pit)").contains("no loose prop `bel` — did you mean `bell`?"), "{}", err("in_zone(bel, pit)"));
        assert!(err("in_zone(bell, bell)").contains("no zone `bell`"), "{}", err("in_zone(bell, bell)"));
        assert!(err("in_zone(bell)").contains("a comma, then a zone id"), "{}", err("in_zone(bell)"));
        assert!(err("in_zone(bell, pit").contains("missing `)`"), "{}", err("in_zone(bell, pit"));
        assert!(parse("prop_y(bell)", &names()).unwrap_err().message.contains("none in this scene"), "no scope: nothing to call");
    }

    #[test]
    fn math_functions_take_expressions() {
        let names: Vec<String> = vec!["x".into(), "y".into()];
        let ev = |src: &str| parse(src, &names).unwrap().eval(&[2.7, -3.2]);
        assert_eq!(ev("floor(x)"), 2.0);
        assert_eq!(ev("floor(y)"), -4.0);
        assert_eq!(ev("round(x)"), 3.0);
        assert_eq!(ev("abs(y)"), 3.2);
        assert_eq!(ev("min(x, y)"), -3.2);
        assert_eq!(ev("max(x + 1, y * -1)"), 3.7);
        assert_eq!(ev("sqrt(y)"), 0.0, "never NaN");
        assert_eq!(ev("floor(x) % 2 == 0"), 1.0);
        let e = parse("min(x)", &names).unwrap_err();
        assert!(e.message.contains("takes 2 argument(s), got 1"), "{e:?}");
        let e = parse("flor(x)", &names).unwrap_err();
        assert!(e.message.contains("did you mean `floor`"), "{e:?}");
    }

    struct Dice(std::cell::Cell<u64>);
    impl World for Dice {
        fn call(&self, _: Func, _: usize) -> f64 {
            0.0
        }
        fn random(&self, lo: f64, hi: f64) -> f64 {
            let mut s = self.0.get();
            let v = draw_int(&mut s, lo, hi);
            self.0.set(s);
            v
        }
    }

    #[test]
    fn random_draws_whole_numbers_in_range_from_the_world_and_repeats_under_the_same_seed() {
        let names: Vec<String> = vec!["n".into()];
        let e = parse("random(1, n)", &names).unwrap();
        assert!(e.reads_world(), "a draw needs the simulation's generator");
        let run = |seed: u64| {
            let w = Dice(std::cell::Cell::new(seed));
            (0..200).map(|_| e.eval_in(&[6.0], &w)).collect::<Vec<f64>>()
        };
        let a = run(7);
        assert!(a.iter().all(|v| v.fract() == 0.0 && (1.0..=6.0).contains(v)), "{a:?}");
        for face in 1..=6 {
            assert!(a.contains(&(face as f64)), "200 throws show every face: {face} missing");
        }
        assert_eq!(a, run(7), "same seed, same draws");
        assert_ne!(a, run(8), "another seed, other draws");
        assert_eq!(e.eval(&[6.0]), 1.0, "with no generator (validate) it is the low end");
        let mut s = 1;
        assert_eq!(draw_int(&mut s, 5.0, 5.0), 5.0);
        let (mut s1, mut s2) = (42u64, 42u64);
        assert_eq!(draw_int(&mut s1, 9.0, 3.0), draw_int(&mut s2, 3.0, 9.0), "bounds in either order are the same range");
    }
}
