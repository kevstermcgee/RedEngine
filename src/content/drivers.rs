//! The kart drivers (`assets/data/drivers.json`). Checker: `sim::kart::Roster::parse` (its own serde structs, also `deny_unknown_fields`).

use crate::strict_struct;

strict_struct! {
    /// The roster.
    pub struct DriverRoster { roster: u32, about: String, base: KartSpec, drivers: Vec<Driver> }
}

strict_struct! {
    /// One driver: a name, an ability and the numbers it changes.
    pub struct Driver { name: String, ability: String, spec: KartSpec }
}

strict_struct! {
    /// Kart numbers (each optional: laid over the base).
    pub struct KartSpec {
        top_speed: f64,
        accel: f64,
        brake: f64,
        reverse_speed: f64,
        steer_rate: f64,
        grip: f64,
        mass: f64,
        dirt: f64,
        mud: f64,
        water: f64,
        boost_mult: f64,
        drift_rate: f64,
        hop_speed: f64,
    }
}
