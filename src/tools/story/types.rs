//! The type checker of story expressions: every expression is the rules' own language after lowering, and its type is inferred from the
//! variable table (a fact's declared type, a query's type in the game's bindings, a scene variable's start value). A condition is a yes/no,
//! an int fact takes a whole number, a yes/no counts as 0 or 1 in a sum or a product; literals 0 and 1 are both.

use crate::sim::rules_expr::{self, Expr, MathFn, Op};
use crate::sim::story::package as pkg;
use crate::sim::systems::Ty;
use std::collections::BTreeMap;

/// An expression's type while it is checked: literals 0 and 1 are both yes/no and numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum T {
    Bool,
    Int,
    Num,
    Lit01,
}

impl T {
    pub fn of(t: Ty) -> T {
        match t {
            Ty::Bool => T::Bool,
            Ty::Int => T::Int,
            Ty::Num => T::Num,
        }
    }
    pub fn boolish(self) -> bool {
        matches!(self, T::Bool | T::Lit01)
    }
    pub fn numeric(self) -> bool {
        !matches!(self, T::Bool)
    }
    pub fn intish(self) -> bool {
        matches!(self, T::Int | T::Lit01)
    }
    pub fn name(self) -> &'static str {
        match self {
            T::Bool => "a yes/no",
            T::Int => "a whole number",
            T::Num => "a number",
            T::Lit01 => "0 or 1",
        }
    }
}

/// How many `&&`-joined parts a lowered condition has (its specificity).
pub fn conjuncts(lowered: &str, nvars: usize) -> u32 {
    fn count(e: &Expr) -> u32 {
        match e {
            Expr::Bin(Op::And, a, b) => count(a) + count(b),
            _ => 1,
        }
    }
    rules_expr::parse(lowered, &pkg::var_names(nvars)).map_or(0, |e| count(&e))
}

pub fn type_of(e: &Expr, vars: &[Ty]) -> Result<T, String> {
    Ok(match e {
        Expr::Num(n) if *n == 0.0 || *n == 1.0 => T::Lit01,
        Expr::Num(n) if n.fract() == 0.0 => T::Int,
        Expr::Num(_) => T::Num,
        Expr::Var(i) => T::of(vars.get(*i).copied().unwrap_or(Ty::Num)),
        Expr::Not(x) => {
            let t = type_of(x, vars)?;
            if !t.boolish() {
                return Err(format!("`!` needs a yes/no, not {}", t.name()));
            }
            T::Bool
        }
        Expr::Neg(x) => {
            let t = type_of(x, vars)?;
            if !t.numeric() {
                return Err("`-` needs a number, not a yes/no".into());
            }
            if t == T::Lit01 {
                T::Int
            } else {
                t
            }
        }
        Expr::Bin(op, a, b) => {
            let (x, y) = (type_of(a, vars)?, type_of(b, vars)?);
            match op {
                Op::And | Op::Or => {
                    if !x.boolish() || !y.boolish() {
                        return Err(format!("`&&` and `||` join yes/no values, not {} and {}", x.name(), y.name()));
                    }
                    T::Bool
                }
                Op::Eq | Op::Ne => {
                    if !((x.boolish() && y.boolish()) || (x.numeric() && y.numeric())) {
                        return Err(format!("comparing {} with {}", x.name(), y.name()));
                    }
                    T::Bool
                }
                Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                    if !x.numeric() || !y.numeric() {
                        return Err("`<`, `<=`, `>`, `>=` compare numbers, not yes/no values".into());
                    }
                    T::Bool
                }
                Op::Div => {
                    if !x.numeric() || !y.numeric() {
                        return Err("arithmetic on a yes/no".into());
                    }
                    T::Num
                }
                // A yes/no counts as 0 or 1 in a sum or a product (`tw:light_out * 3 + ...`, a tension score).
                Op::Add | Op::Sub | Op::Mul => {
                    let int = |t: T| t.intish() || t == T::Bool;
                    if int(x) && int(y) {
                        T::Int
                    } else {
                        T::Num
                    }
                }
                _ => {
                    if !x.numeric() || !y.numeric() {
                        return Err("`%` of a yes/no".into());
                    }
                    if x.intish() && y.intish() {
                        T::Int
                    } else {
                        T::Num
                    }
                }
            }
        }
        Expr::Math(f, args) => {
            let ts = args.iter().map(|a| type_of(a, vars)).collect::<Result<Vec<_>, _>>()?;
            if ts.iter().any(|t| !t.numeric()) {
                return Err("a math function of a yes/no".into());
            }
            match f {
                MathFn::Floor | MathFn::Round => T::Int,
                MathFn::Sqrt => T::Num,
                _ if ts.iter().all(|t| t.intish()) => T::Int,
                _ => T::Num,
            }
        }
        Expr::Random(..) => return Err("`random` is not allowed in a story".into()),
        Expr::Me(_) | Expr::Call(..) | Expr::Call2(..) => return Err("not something a story reads".into()),
    })
}

/// The scene's declared variables and their types (from each start value: `true`/`false` a yes/no, a whole number an int).
pub fn scene_types(scene: &serde_json::Value) -> BTreeMap<String, Ty> {
    scene
        .get("vars")
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flatten()
        .map(|(k, v)| {
            let ty = match v {
                serde_json::Value::Bool(_) => Ty::Bool,
                serde_json::Value::Number(n) if n.as_f64().is_some_and(|f| f.fract() == 0.0) => Ty::Int,
                _ => Ty::Num,
            };
            (k.clone(), ty)
        })
        .collect()
}
