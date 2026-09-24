//! Easing curves (linear/in/out/inout/hold/back/bounce/elastic) used by every keyframe `Track`.

use std::f32::consts::PI;

/// Easing curves shared by every keyframe track. Each function maps a clip-local `x` in
/// `0.0..=1.0` (how far through the segment leading up to a keyframe) to a shaped `0.0..=1.0`.
/// Same name set as the 2D engine (`animengine`), so an AI author only has to learn this once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Ease {
    #[default]
    Linear,
    In,
    Out,
    InOut,
    Hold,
    Back,
    Bounce,
    Elastic,
}

impl Ease {
    /// Parses an easing name from a scene (`"linear"`, `"in"`, `"out"`, `"inout"`, `"hold"`, `"back"`, `"bounce"`, `"elastic"`).
    pub fn parse(name: &str) -> Result<Ease, String> {
        match name {
            "linear" => Ok(Ease::Linear),
            "in" => Ok(Ease::In),
            "out" => Ok(Ease::Out),
            "inout" => Ok(Ease::InOut),
            "hold" => Ok(Ease::Hold),
            "back" => Ok(Ease::Back),
            "bounce" => Ok(Ease::Bounce),
            "elastic" => Ok(Ease::Elastic),
            other => Err(format!(
                "unknown ease '{other}' (expected one of: linear, in, out, inout, hold, back, bounce, elastic)"
            )),
        }
    }

    /// Maps normalized time `x` in 0..=1 through the curve.
    pub fn apply(self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        match self {
            Ease::Linear => x,
            Ease::In => x * x,
            Ease::Out => 1.0 - (1.0 - x) * (1.0 - x),
            Ease::InOut => {
                if x < 0.5 {
                    2.0 * x * x
                } else {
                    1.0 - (-2.0 * x + 2.0).powi(2) / 2.0
                }
            }
            Ease::Hold => 0.0,
            Ease::Back => {
                let c1 = 1.70158;
                let c3 = c1 + 1.0;
                c3 * x * x * x - c1 * x * x
            }
            Ease::Bounce => bounce_out(x),
            Ease::Elastic => {
                if x == 0.0 || x == 1.0 {
                    x
                } else {
                    let c4 = (2.0 * PI) / 3.0;
                    2f32.powf(-10.0 * x) * ((x * 10.0 - 0.75) * c4).sin() + 1.0
                }
            }
        }
    }
}

fn bounce_out(x: f32) -> f32 {
    let n1 = 7.5625;
    let d1 = 2.75;
    let mut x = x;
    if x < 1.0 / d1 {
        n1 * x * x
    } else if x < 2.0 / d1 {
        x -= 1.5 / d1;
        n1 * x * x + 0.75
    } else if x < 2.5 / d1 {
        x -= 2.25 / d1;
        n1 * x * x + 0.9375
    } else {
        x -= 2.625 / d1;
        n1 * x * x + 0.984375
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_are_stable() {
        for ease in [
            Ease::Linear,
            Ease::In,
            Ease::Out,
            Ease::InOut,
            Ease::Back,
            Ease::Bounce,
            Ease::Elastic,
        ] {
            assert!((ease.apply(0.0)).abs() < 1e-4, "{ease:?} at 0");
            assert!((ease.apply(1.0) - 1.0).abs() < 1e-4, "{ease:?} at 1");
        }
    }

    #[test]
    fn hold_is_a_step() {
        assert_eq!(Ease::Hold.apply(0.99), 0.0);
    }

    #[test]
    fn parse_roundtrip() {
        for name in ["linear", "in", "out", "inout", "hold", "back", "bounce", "elastic"] {
            assert!(Ease::parse(name).is_ok());
        }
        assert!(Ease::parse("bogus").is_err());
    }
}
