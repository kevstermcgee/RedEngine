use crate::easing::Ease;
use glam::Vec3;

/// Anything a `Track<T>` can interpolate between two keyframe values.
pub trait Lerp: Copy {
    fn lerp(self, other: Self, t: f32) -> Self;
}

impl Lerp for f32 {
    fn lerp(self, other: Self, t: f32) -> Self {
        self + (other - self) * t
    }
}

impl Lerp for Vec3 {
    fn lerp(self, other: Self, t: f32) -> Self {
        Vec3::lerp(self, other, t)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Keyframe<T: Lerp> {
    pub t: f32,
    pub value: T,
    pub ease: Ease,
}

/// A value that is either constant for the whole clip or driven by sparse, eased keyframes.
/// Sampling before the first keyframe holds the first value; sampling after the last holds the
/// last value (no extrapolation).
#[derive(Clone, Debug)]
pub enum Track<T: Lerp> {
    Constant(T),
    Keyframed(Vec<Keyframe<T>>),
}

impl<T: Lerp> Track<T> {
    pub fn constant(value: T) -> Self {
        Track::Constant(value)
    }

    pub fn sample(&self, t: f32) -> T {
        match self {
            Track::Constant(v) => *v,
            Track::Keyframed(kfs) => {
                if kfs.is_empty() {
                    unreachable!("validated tracks always have at least one keyframe");
                }
                if t <= kfs[0].t {
                    return kfs[0].value;
                }
                let last = kfs.len() - 1;
                if t >= kfs[last].t {
                    return kfs[last].value;
                }
                for w in kfs.windows(2) {
                    let (a, b) = (&w[0], &w[1]);
                    if t >= a.t && t <= b.t {
                        let span = (b.t - a.t).max(1e-6);
                        let local = (t - a.t) / span;
                        let shaped = b.ease.apply(local);
                        return a.value.lerp(b.value, shaped);
                    }
                }
                kfs[last].value
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_track_ignores_t() {
        let tr = Track::constant(5.0f32);
        assert_eq!(tr.sample(0.0), 5.0);
        assert_eq!(tr.sample(100.0), 5.0);
    }

    #[test]
    fn keyframed_track_interpolates_linearly() {
        let tr = Track::Keyframed(vec![
            Keyframe { t: 0.0, value: 0.0f32, ease: Ease::Linear },
            Keyframe { t: 2.0, value: 10.0, ease: Ease::Linear },
        ]);
        assert_eq!(tr.sample(-1.0), 0.0);
        assert_eq!(tr.sample(1.0), 5.0);
        assert_eq!(tr.sample(3.0), 10.0);
    }

    #[test]
    fn keyframed_track_holds_before_first_and_after_last() {
        let tr = Track::Keyframed(vec![
            Keyframe { t: 1.0, value: 1.0f32, ease: Ease::Linear },
            Keyframe { t: 2.0, value: 2.0, ease: Ease::Linear },
        ]);
        assert_eq!(tr.sample(0.0), 1.0);
        assert_eq!(tr.sample(5.0), 2.0);
    }

    #[test]
    fn vec3_track_lerps_componentwise() {
        let tr = Track::Keyframed(vec![
            Keyframe { t: 0.0, value: Vec3::ZERO, ease: Ease::Linear },
            Keyframe { t: 1.0, value: Vec3::new(2.0, 4.0, -2.0), ease: Ease::Linear },
        ]);
        let mid = tr.sample(0.5);
        assert!((mid - Vec3::new(1.0, 2.0, -1.0)).length() < 1e-5);
    }
}
