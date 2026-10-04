//! Time of day: the sun, moon and sky as a pure function of a clock.
//!
//! A scene with a `clock` block has a day that turns into night and back: the sun and moon travel across the sky, the sky changes colour with the sun's
//! height (night, a long dawn, day, a long golden dusk), the sun light and the ambient follow it, the stars come out as the sun sets (and turn
//! slowly overhead through the night), the moon has its phase, and a haze can blend the distance into the horizon colour.
//!
//! ```json
//! "clock": { "day_secs": 720, "start": 0.27, "sun_max_deg": 62, "fog": 0.0035, "stars": 1.0, "moon": true }
//! ```
//!
//! `start` is the time of day when the scene begins: 0 is midnight, 0.25 sunrise, 0.5 noon, 0.75 sunset. Everything here is arithmetic on that one number and
//! a day count, so the same moment looks the same on every machine, `frame --hour 18.5` can draw any moment, and a test can sweep a whole day and check that
//! nothing jumps. The look is deliberately stylised and soft (a pink-gold dawn and dusk, a deep indigo night) rather than physically exact.
//!
//! Conventions: +X is east (the sun rises there), +Z is south (the sun culminates there in the northern hemisphere), +Y is up.

use crate::color::parse_hex_to_linear;
use crate::strict::check_keys;
use glam::Vec3;
use serde_json::{Map, Value};
use std::f32::consts::TAU;

/// `clock` keys.
pub const CLOCK_KEYS: &[&str] = &["day_secs", "start", "sun_max_deg", "latitude_deg", "fog", "stars", "moon", "moon_phase", "night_light", "light"];

/// The synodic month in days: how long the moon takes to go through its phases.
const MOON_MONTH: f32 = 29.53;

/// The clock block.
#[derive(Debug, Clone, PartialEq)]
pub struct Clock {
    /// Real seconds in one in-game day.
    pub day_secs: f32,
    /// Time of day (0..1, 0 midnight, 0.25 sunrise, 0.5 noon, 0.75 sunset) at scene time zero.
    pub start: f32,
    /// How high the sun climbs at noon, degrees (the path is tilted from the zenith by the rest).
    pub sun_max_deg: f32,
    /// Latitude for the turning of the stars, degrees (the pole star sits this high in the north).
    pub latitude_deg: f32,
    /// Haze density per metre (0 for none): the distance fades into the horizon colour.
    pub fog: f32,
    /// Star brightness, 0 (none) to 2.
    pub stars: f32,
    /// Whether the moon shows.
    pub moon: bool,
    /// The moon's phase on day 0 (0 new, 0.5 full); it advances a month's worth over 29.5 days.
    pub moon_phase: f32,
    /// How bright the night is under the moon, 0 (black) to 1.
    pub night_light: f32,
    /// The id of the directional light the sun drives (default: the first directional light).
    pub light: Option<String>,
}

impl Default for Clock {
    fn default() -> Clock {
        Clock {
            day_secs: 720.0,
            start: 0.27,
            sun_max_deg: 62.0,
            latitude_deg: 40.0,
            fog: 0.0,
            stars: 1.0,
            moon: true,
            moon_phase: 0.55,
            night_light: 0.5,
            light: None,
        }
    }
}

/// Everything the renderer needs to know about one moment. Colours are linear RGB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DayState {
    /// Time of day, 0..1.
    pub time: f32,
    /// Whole days since the clock began (0 on the first).
    pub day: u64,
    /// Direction toward the sun and its height above the horizon in degrees.
    pub sun_dir: Vec3,
    /// Sun height, degrees (negative below the horizon).
    pub sun_elev_deg: f32,
    /// Apparent radius of the disc, degrees (larger near the horizon).
    pub sun_radius_deg: f32,
    /// Colour of the disc and its light.
    pub sun_color: Vec3,
    /// Direction toward the moon and its phase (0 new, 0.5 full, 1 new again).
    pub moon_dir: Vec3,
    /// See `moon_dir`.
    pub moon_phase: f32,
    /// Sky colour overhead.
    pub zenith: Vec3,
    /// Sky colour at the horizon (and what distant things fade into).
    pub horizon: Vec3,
    /// Colour and strength of the glow low on the sun's side of the sky.
    pub glow: Vec3,
    /// See `glow`.
    pub glow_strength: f32,
    /// The direction the sun's light shines in (from the sun toward the world). It keeps the sun's own direction and fades out as the sun sets, so shadows
    /// lengthen at dusk and nothing sweeps round.
    pub light_dir: Vec3,
    /// Colour of the sun's light times its intensity (zero at night).
    pub light_color: Vec3,
    /// The direction moonlight shines in, and its colour times intensity: a dim cool fill from the moon's own place in the sky (no shadows).
    pub moon_light_dir: Vec3,
    /// See `moon_light_dir`.
    pub moon_light_color: Vec3,
    /// Ambient light colour times intensity.
    pub ambient: Vec3,
    /// How visible the stars are, 0 (day) to 1 (full night).
    pub star_visibility: f32,
    /// Rotation of the celestial sphere (rows), applied to a view direction before looking up a star.
    pub celestial: [Vec3; 3],
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn hex(s: &str) -> Vec3 {
    parse_hex_to_linear(s).unwrap_or(Vec3::ZERO)
}

/// Linear interpolation through `(sun height in degrees, value)` keys, smoothed between them.
fn keyed(keys: &[(f32, Vec3)], elev: f32) -> Vec3 {
    if elev <= keys[0].0 {
        return keys[0].1;
    }
    for w in keys.windows(2) {
        if elev <= w[1].0 {
            return w[0].1.lerp(w[1].1, smooth(w[0].0, w[1].0, elev));
        }
    }
    keys[keys.len() - 1].1
}

impl Clock {
    /// The scene time (seconds) at which the clock reads `hour` (0..24) on its first day.
    pub fn t_for_hour(&self, hour: f32) -> f32 {
        ((hour / 24.0).rem_euclid(1.0) - self.start).rem_euclid(1.0) * self.day_secs
    }

    /// The hour (0..24) at scene time `t`.
    pub fn hour_at(&self, t: f32) -> f32 {
        ((self.start + t / self.day_secs).rem_euclid(1.0)) * 24.0
    }

    /// The state of the sky at scene time `t` seconds, `day_offset` whole days into the game (saved days played before this session).
    pub fn state(&self, t: f32, day_offset: u64) -> DayState {
        let cycles = self.start + t / self.day_secs;
        let time = cycles.rem_euclid(1.0);
        let day = cycles.floor().max(0.0) as u64 + day_offset;
        // The sun: a great circle tilted from the vertical, rising in the east (+X) and culminating toward +Z.
        let a = TAU * (time - 0.25);
        let tilt = (90.0 - self.sun_max_deg).to_radians();
        let sun_dir = Vec3::new(a.cos(), a.sin() * tilt.cos(), a.sin() * tilt.sin()).normalize();
        let elev = sun_dir.y.asin().to_degrees();
        // Morning or evening: the dawn is cooler and pinker, the dusk warmer and golder.
        let dusk = smooth(0.4, 0.6, time);
        let dawn_tint = Vec3::new(1.0, 0.93, 1.06);
        let dusk_tint = Vec3::new(1.06, 0.98, 0.88);
        let tint = dawn_tint.lerp(dusk_tint, dusk);

        let zenith = keyed(
            &[
                (-18.0, hex("#03051a")),
                (-8.0, hex("#101649")),
                (-2.0, hex("#263479")),
                (3.0, hex("#4067b3")),
                (10.0, hex("#4f86d0")),
                (25.0, hex("#5b9be2")),
                (50.0, hex("#4f94e6")),
            ],
            elev,
        );
        let horizon = keyed(
            &[
                (-18.0, hex("#070c28")),
                (-8.0, hex("#2f2d66")),
                (-2.0, hex("#d9768a")),
                (3.0, hex("#ff9d68")),
                (10.0, hex("#ffd2a2")),
                (25.0, hex("#cfe7f6")),
                (50.0, hex("#bfe3f8")),
            ],
            elev,
        ) * tint;
        let glow =
            keyed(&[(-10.0, hex("#8a4a96")), (-2.0, hex("#ff8566")), (4.0, hex("#ff9a55")), (14.0, hex("#ffc680")), (30.0, hex("#ffe2b0"))], elev) * tint;
        // The glow rises as the sun nears the horizon, peaks just under and just over it, and is gone at night and at noon.
        let glow_strength = smooth(-12.0, -2.0, elev) * (1.0 - smooth(5.0, 30.0, elev)) * 0.72;
        let sun_color = keyed(&[(-4.0, hex("#ff6a3a")), (2.0, hex("#ff9150")), (10.0, hex("#ffc98a")), (30.0, hex("#fff1d2"))], elev);
        let sun_radius_deg = 2.4 + 1.6 * (1.0 - smooth(0.0, 25.0, elev.max(0.0)));

        // The moon sits opposite the sun, a little off the ecliptic, and goes through its phases over the days.
        let moon_phase = (self.moon_phase + day as f32 / MOON_MONTH).rem_euclid(1.0);
        let moon_dir = Vec3::new(-sun_dir.x, -sun_dir.y * 0.93 + 0.12, -sun_dir.z).normalize();

        // Light: the sun by day (its own direction, fading out as it sets) and, separately, a dim cool fill from the moon at night.
        let sun_up = smooth(-3.0, 8.0, elev);
        let light_color = sun_color * (sun_up * (0.35 + 0.75 * smooth(2.0, 28.0, elev)));
        let moon_illum = 0.5 - 0.5 * (TAU * moon_phase).cos();
        let moon_light_color = Vec3::new(0.55, 0.68, 1.0) * (0.20 + 0.34 * moon_illum) * self.night_light * (1.0 - sun_up);
        let ambient_day = Vec3::new(0.50, 0.60, 0.78) * 0.62;
        // At twilight the ground takes the colour of the sky near the horizon, so it glows with the sunset instead of going black.
        let ambient_dusk = (horizon * 0.55 + zenith * 0.45) * 0.62 + Vec3::new(0.06, 0.04, 0.08);
        let ambient_night = Vec3::new(0.13, 0.19, 0.38) * (0.30 + 0.9 * self.night_light * 0.5);
        let ambient = ambient_night.lerp(ambient_dusk, smooth(-12.0, -1.0, elev)).lerp(ambient_day, smooth(2.0, 22.0, elev));

        // The stars appear once the sun is well under the horizon and are gone before it clears it; the sphere turns once a day around the pole.
        let lat = self.latitude_deg.to_radians();
        let axis = Vec3::new(0.0, lat.sin(), -lat.cos()).normalize();
        let celestial = rotation_rows(axis, -TAU * time);
        DayState {
            time,
            day,
            sun_dir,
            sun_elev_deg: elev,
            sun_radius_deg,
            sun_color,
            moon_dir,
            moon_phase,
            zenith,
            horizon,
            glow,
            glow_strength,
            light_dir: -sun_dir,
            light_color,
            moon_light_dir: -moon_dir,
            moon_light_color,
            ambient,
            star_visibility: (1.0 - smooth(-16.0, -4.0, elev)) * self.stars,
            celestial,
        }
    }
}

/// The rows of the rotation by `angle` radians about the unit `axis` (Rodrigues).
fn rotation_rows(axis: Vec3, angle: f32) -> [Vec3; 3] {
    let m = glam::Mat3::from_axis_angle(axis, angle);
    let t = m.transpose();
    [t.x_axis, t.y_axis, t.z_axis]
}

/// Parses the scene's `clock` block (`None` when absent).
pub fn parse_clock(root: &Map<String, Value>) -> Result<Option<Clock>, Vec<String>> {
    let Some(raw) = root.get("clock") else { return Ok(None) };
    let Some(o) = raw.as_object() else { return Err(vec!["clock: must be an object like {\"day_secs\": 720, \"start\": 0.27}".to_string()]) };
    let mut errs = Vec::new();
    check_keys(&mut errs, "clock", o, CLOCK_KEYS);
    let d = Clock::default();
    let mut num = |key: &str, default: f32, lo: f32, hi: f32| -> f32 {
        match o.get(key) {
            None => default,
            Some(v) => match v.as_f64().map(|x| x as f32) {
                Some(x) if (lo..=hi).contains(&x) => x,
                Some(x) => {
                    errs.push(format!("clock.{key}: {x} is outside {lo} to {hi}"));
                    default
                }
                None => {
                    errs.push(format!("clock.{key}: must be a number"));
                    default
                }
            },
        }
    };
    let clock = Clock {
        day_secs: num("day_secs", d.day_secs, 20.0, 86400.0),
        start: num("start", d.start, 0.0, 1.0).rem_euclid(1.0),
        sun_max_deg: num("sun_max_deg", d.sun_max_deg, 20.0, 89.0),
        latitude_deg: num("latitude_deg", d.latitude_deg, 0.0, 89.0),
        fog: num("fog", d.fog, 0.0, 0.05),
        stars: num("stars", d.stars, 0.0, 2.0),
        moon: o.get("moon").map_or(d.moon, |v| v.as_bool().unwrap_or(d.moon)),
        moon_phase: num("moon_phase", d.moon_phase, 0.0, 1.0),
        night_light: num("night_light", d.night_light, 0.0, 1.0),
        light: o.get("light").and_then(Value::as_str).map(str::to_string),
    };
    if errs.is_empty() {
        Ok(Some(clock))
    } else {
        Err(errs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lum(c: Vec3) -> f32 {
        0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z
    }

    fn at(hour: f32) -> DayState {
        let c = Clock::default();
        c.state(c.t_for_hour(hour), 0)
    }

    #[test]
    fn the_sun_rises_in_the_east_peaks_in_the_south_and_sets_in_the_west() {
        let (rise, noon, set, midnight) = (at(6.0), at(12.0), at(18.0), at(0.0));
        assert!(rise.sun_dir.x > 0.99 && rise.sun_elev_deg.abs() < 0.5, "{:?}", rise.sun_dir);
        assert!(set.sun_dir.x < -0.99 && set.sun_elev_deg.abs() < 0.5);
        assert!((noon.sun_elev_deg - 62.0).abs() < 0.5 && noon.sun_dir.z > 0.3, "culminates toward +Z at the configured height: {:?}", noon.sun_dir);
        assert!(midnight.sun_elev_deg < -55.0);
        assert!((noon.sun_dir.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_day_is_continuous_nothing_jumps_between_one_minute_and_the_next() {
        let c = Clock { day_secs: 1440.0, ..Clock::default() };
        let mut prev = c.state(0.0, 0);
        for s in 1..=1440 {
            let st = c.state(s as f32, 0);
            for (name, a, b) in [
                ("zenith", prev.zenith, st.zenith),
                ("horizon", prev.horizon, st.horizon),
                ("light", prev.light_color, st.light_color),
                ("ambient", prev.ambient, st.ambient),
                ("glow", prev.glow, st.glow),
                ("sun", prev.sun_dir, st.sun_dir),
            ] {
                assert!((a - b).length() < 0.06, "{name} jumped by {} at minute {s}", (a - b).length());
            }
            assert!((prev.star_visibility - st.star_visibility).abs() < 0.05, "stars at minute {s}");
            assert!(
                (prev.light_dir - st.light_dir).length() < 0.01 && (prev.moon_light_dir - st.moon_light_dir).length() < 0.01,
                "a light direction jumped at minute {s}"
            );
            assert!((prev.moon_light_color - st.moon_light_color).length() < 0.02, "moonlight jumped at minute {s}");
            prev = st;
        }
    }

    #[test]
    fn day_is_brighter_than_night_and_the_sky_goes_from_blue_to_gold_to_indigo() {
        let (day, dusk, night) = (at(12.0), at(18.4), at(0.0));
        assert!(
            lum(day.light_color) > 0.5 && lum(night.light_color) < 0.001 && lum(night.moon_light_color) < 0.3 && lum(night.moon_light_color) > 0.05,
            "{} {}",
            lum(day.light_color),
            lum(night.light_color)
        );
        assert!(lum(day.ambient) > lum(night.ambient) * 3.0);
        assert!(lum(day.horizon) > 0.4 && lum(night.horizon) < 0.02);
        assert!(day.zenith.z > day.zenith.x, "a blue sky by day");
        assert!(dusk.horizon.x > dusk.horizon.z, "a warm horizon at dusk: {:?}", dusk.horizon);
        assert!(dusk.glow_strength > 0.5 && night.glow_strength < 0.01 && day.glow_strength < 0.01, "the glow belongs to twilight");
        assert!(night.zenith.z > night.zenith.x, "an indigo night");
        assert!(day.sun_color.x > day.sun_color.z * 0.9 && dusk.sun_color.x > dusk.sun_color.z * 1.5, "the sun is warmer low down");
        assert!(dusk.sun_radius_deg > day.sun_radius_deg, "bigger near the horizon");
    }

    #[test]
    fn dawn_and_dusk_are_not_the_same_picture() {
        let (dawn, dusk) = (at(5.8), at(18.2));
        assert!((dawn.horizon - dusk.horizon).length() > 0.02, "dawn is cooler, dusk golder");
        assert!(dawn.horizon.z / dawn.horizon.x > dusk.horizon.z / dusk.horizon.x);
    }

    #[test]
    fn the_stars_come_out_after_sunset_and_are_gone_by_morning() {
        let vis = |h: f32| at(h).star_visibility;
        assert_eq!(vis(12.0), 0.0);
        assert!(vis(18.0) < 0.1, "not at the horizon crossing: {}", vis(18.0));
        assert!(vis(19.5) > 0.4, "{}", vis(19.5));
        assert!(vis(0.0) > 0.99);
        assert!(vis(5.0) > 0.3 && vis(6.5) < 0.05, "fading before sunrise: {} {}", vis(5.0), vis(6.5));
        let none = Clock { stars: 0.0, ..Clock::default() };
        assert_eq!(none.state(none.t_for_hour(0.0), 0).star_visibility, 0.0);
    }

    #[test]
    fn the_celestial_sphere_turns_once_a_day_and_rows_are_a_rotation() {
        let (a, b, c) = (at(0.0), at(6.0), at(24.0 - 1e-3));
        for st in [a, b] {
            let [r0, r1, r2] = st.celestial;
            assert!((r0.length() - 1.0).abs() < 1e-4 && r0.dot(r1).abs() < 1e-4 && (r0.cross(r1) - r2).length() < 1e-3, "orthonormal");
        }
        assert!((a.celestial[0] - b.celestial[0]).length() > 0.5, "the stars have moved by morning");
        assert!((a.celestial[0] - c.celestial[0]).length() < 0.01, "and are back after a day");
    }

    #[test]
    fn days_count_up_across_sunrises_and_saved_days_are_added() {
        let c = Clock { day_secs: 100.0, start: 0.3, ..Clock::default() };
        assert_eq!(c.state(0.0, 0).day, 0);
        assert_eq!(c.state(69.0, 0).day, 0);
        assert_eq!(c.state(71.0, 0).day, 1, "the day number turns over at midnight (time 1.0)");
        assert_eq!(c.state(171.0, 4).day, 6);
        assert!((c.hour_at(c.t_for_hour(18.5)) - 18.5).abs() < 1e-3);
        assert!((c.t_for_hour(c.hour_at(33.0)) - 33.0).abs() < 1e-2);
    }

    #[test]
    fn the_moon_goes_through_its_phases_over_a_month() {
        let c = Clock::default();
        let phase = |day| c.state(c.t_for_hour(23.5), day).moon_phase;
        assert!((phase(0) - 0.55).abs() < 0.01, "{}", phase(0));
        assert!(phase(8) > phase(0) && (phase(0) - phase(30)).abs() < 0.06, "back near the start after a month: {} {}", phase(0), phase(30));
        assert!(at(0.0).moon_dir.y > 0.5, "up at midnight");
    }

    #[test]
    fn the_block_parses_with_defaults_and_names_every_mistake() {
        let ok = parse_clock(serde_json::json!({"clock": {"day_secs": 600, "start": 0.5, "fog": 0.003}}).as_object().unwrap()).unwrap().unwrap();
        assert_eq!((ok.day_secs, ok.start, ok.fog, ok.moon), (600.0, 0.5, 0.003, true));
        assert!(parse_clock(&Map::new()).unwrap().is_none());
        let e = parse_clock(serde_json::json!({"clock": {"day_sec": 600, "start": "dawn", "fog": 9, "stars": 5}}).as_object().unwrap()).unwrap_err().join("\n");
        for want in ["clock.day_sec: unknown field", "clock.start: must be a number", "clock.fog: 9 is outside", "clock.stars: 5 is outside"] {
            assert!(e.contains(want), "missing `{want}` in:\n{e}");
        }
    }
}
