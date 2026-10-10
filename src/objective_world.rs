//! The objective modes' world as the client draws it: capture-the-flag flags, the search-and-destroy bomb and the bomb sites
//! (ADR 2026-10-07-killchain-game-modes-free-for-all-capture-the-flag).
//!
//! Like [`ShooterWorld`](crate::shooter_world::ShooterWorld), everything that can appear is added to the scene once, hidden
//! ([`ObjectiveWorld::add_to_scene`], before the renderer is built), and [`ObjectiveWorld::update`] moves and shows what the newest
//! [`ObjSnap`] says. It is added whenever a map has flags or sites, not only when its own `shooter.mode` asks for them, because the host
//! chooses the mode and the map file does not change with it. Pure: no window, no GPU, tested on scenes.

use crate::net::protocol::ObjSnap;
use crate::scene_pool::HIDDEN_SCALE;
use crate::schema::{Material, Object, ObjectKind, PrimKind, Scene};
use crate::sim::objective::MAX_SITES;
use crate::track::Track;
use glam::Vec3;

/// Ridgeback's colour in the world (linear).
pub const TEAM1: Vec3 = Vec3::new(0.30, 0.50, 0.12);
/// Nightfall's colour in the world (linear).
pub const TEAM2: Vec3 = Vec3::new(0.10, 0.22, 0.75);
/// Height of a flag's pole.
pub const POLE_H: f32 = 3.4;

const PER_FLAG: usize = 4; // pole, cloth, base, beam
const BOMB_PARTS: usize = 3; // body, light, beam
const PER_SITE: usize = 2; // ring, marker

fn prim(id: String, kind: PrimKind, color: Vec3, emissive: Vec3, opacity: f32) -> Object {
    Object {
        id,
        position: Track::constant(Vec3::ZERO),
        rotation: Track::constant(Vec3::ZERO),
        scale: Track::constant(Vec3::splat(HIDDEN_SCALE)),
        material: Some(Material { color: Track::constant(color), metallic: 0.2, roughness: 0.7, emissive, opacity }),
        collide: false,
        prefab: None,
        movable: Some(false),
        kind: ObjectKind::Prim(kind),
    }
}

fn place(scene: &mut Scene, index: usize, pos: Vec3, rot_deg: Vec3, scale: Vec3) {
    if let Some(o) = scene.objects.get_mut(index) {
        o.position = Track::constant(pos);
        o.rotation = Track::constant(rot_deg);
        o.scale = Track::constant(scale);
    }
}

fn hide(scene: &mut Scene, index: usize) {
    if let Some(o) = scene.objects.get_mut(index) {
        o.scale = Track::constant(Vec3::splat(HIDDEN_SCALE));
    }
}

/// The objective scene objects and where they start.
pub struct ObjectiveWorld {
    first: usize,
    flags: usize,
    bomb_at: usize,
    sites_at: usize,
    sites: Vec<(Vec3, f32)>,
    flag_homes: Vec<(u8, Vec3)>,
    time: f32,
}

impl ObjectiveWorld {
    /// Adds the objective objects to `scene` (hidden) if the map has flags or bomb sites. Call it before the renderer is built.
    pub fn add_to_scene(scene: &mut Scene) -> Option<ObjectiveWorld> {
        let cfg = scene.shooter.clone()?;
        let obj = &cfg.objective;
        if obj.flags.is_empty() && obj.sites.is_empty() {
            return None;
        }
        let first = scene.objects.len();
        let flags = obj.flags.len().min(2);
        for (i, f) in obj.flags.iter().take(2).enumerate() {
            let team = if f.team == 1 { TEAM1 } else { TEAM2 };
            let id = format!("obj_flag{i}");
            scene.objects.push(prim(format!("{id}_pole"), PrimKind::Cylinder { radius: 0.06, height: POLE_H }, Vec3::splat(0.75), Vec3::ZERO, 1.0));
            scene.objects.push(prim(format!("{id}_cloth"), PrimKind::Box { size: Vec3::new(1.25, 0.8, 0.04) }, team, team * 0.6, 1.0));
            scene.objects.push(prim(format!("{id}_base"), PrimKind::Cylinder { radius: 1.3, height: 0.08 }, team, team * 0.35, 1.0));
            scene.objects.push(prim(format!("{id}_beam"), PrimKind::Cylinder { radius: 0.14, height: 40.0 }, team, team * 2.5, 0.28));
        }
        let bomb_at = scene.objects.len();
        if !obj.sites.is_empty() {
            scene.objects.push(prim("obj_bomb_body".into(), PrimKind::Box { size: Vec3::new(0.55, 0.22, 0.34) }, Vec3::new(0.12, 0.12, 0.13), Vec3::ZERO, 1.0));
            scene.objects.push(prim("obj_bomb_light".into(), PrimKind::Sphere { radius: 0.06 }, Vec3::new(1.0, 0.1, 0.05), Vec3::new(6.0, 0.3, 0.1), 1.0));
            scene.objects.push(prim(
                "obj_bomb_beam".into(),
                PrimKind::Cylinder { radius: 0.12, height: 40.0 },
                Vec3::new(1.0, 0.2, 0.1),
                Vec3::new(4.0, 0.4, 0.2),
                0.3,
            ));
        }
        let sites_at = scene.objects.len();
        let sites: Vec<(Vec3, f32)> = obj.sites.iter().take(MAX_SITES).map(|s| (s.at, s.radius)).collect();
        for (i, (_, radius)) in sites.iter().enumerate() {
            scene.objects.push(prim(
                format!("obj_site{i}_ring"),
                PrimKind::Cylinder { radius: *radius, height: 0.06 },
                Vec3::new(1.0, 0.7, 0.1),
                Vec3::new(1.6, 1.0, 0.1),
                0.22,
            ));
            scene.objects.push(prim(
                format!("obj_site{i}_post"),
                PrimKind::Cylinder { radius: 0.1, height: 9.0 },
                Vec3::new(1.0, 0.7, 0.1),
                Vec3::new(2.5, 1.5, 0.1),
                0.45,
            ));
        }
        Some(ObjectiveWorld { first, flags, bomb_at, sites_at, sites, flag_homes: obj.flags.iter().take(2).map(|f| (f.team, f.at)).collect(), time: 0.0 })
    }

    /// How many scene objects it added.
    pub fn object_count(&self) -> usize {
        self.flags * PER_FLAG + if self.sites.is_empty() { 0 } else { BOMB_PARTS } + self.sites.len() * PER_SITE
    }

    fn flag_index(&self, flag: usize, part: usize) -> usize {
        self.first + flag * PER_FLAG + part
    }

    /// Ids of this world's objects that should not be drawn right now.
    pub fn hidden_ids<'a>(&'a self, scene: &'a Scene) -> Vec<&'a str> {
        scene
            .objects
            .iter()
            .skip(self.first)
            .take(self.sites_at - self.first + self.sites.len() * PER_SITE)
            .filter(|o| o.scale.sample(0.0).x <= HIDDEN_SCALE * 2.0)
            .map(|o| o.id.as_str())
            .collect()
    }

    /// Shows what `obj` says (nothing when it is `None` or of the other mode) and animates it by `dt`.
    pub fn update(&mut self, scene: &mut Scene, obj: Option<&ObjSnap>, dt: f32) {
        self.time += dt;
        let t = self.time;
        let end = self.sites_at + self.sites.len() * PER_SITE;
        let kind = obj.map_or(0, |o| o.kind);
        // Hide everything, then show what is wanted.
        for i in self.first..end {
            hide(scene, i);
        }
        let Some(o) = obj else { return };
        if kind == 1 {
            for f in 0..self.flags {
                let snap = o.flags[f];
                let team = self.flag_homes.get(f).map_or(1, |h| h.0);
                let pos = Vec3::from(snap.pos);
                // Carried flags ride over the carrier's shoulder and tilt; dropped ones lean; a flag at home stands.
                let (base, tilt) = match snap.state {
                    1 => (pos + Vec3::Y * 0.1, 18.0),
                    2 => (pos, 12.0),
                    _ => (pos, 0.0),
                };
                let wave = (t * 3.0 + f as f32).sin() * 6.0;
                place(scene, self.flag_index(f, 0), base + Vec3::Y * POLE_H / 2.0, Vec3::new(0.0, 0.0, tilt), Vec3::ONE);
                place(scene, self.flag_index(f, 1), base + Vec3::new(0.65, POLE_H - 0.5, 0.0), Vec3::new(0.0, wave, tilt), Vec3::ONE);
                if snap.state != 1 {
                    place(scene, self.flag_index(f, 2), base + Vec3::Y * 0.04, Vec3::ZERO, Vec3::ONE);
                }
                // A beam you can see from across the map, brighter while the flag is away from home.
                let strength = if snap.state == 0 { 0.6 } else { 1.0 };
                place(scene, self.flag_index(f, 3), base + Vec3::Y * 20.0, Vec3::ZERO, Vec3::new(strength, 1.0, strength));
                let _ = team;
            }
        }
        if kind == 2 {
            let s = o.snd;
            // The sites: a ring on the ground and a tall marker; they hide once the bomb is down.
            if s.bomb < 2 {
                for (i, (at, radius)) in self.sites.iter().enumerate() {
                    let pulse = 1.0 + 0.04 * (t * 2.0 + i as f32).sin();
                    place(scene, self.sites_at + i * PER_SITE, *at + Vec3::Y * 0.04, Vec3::ZERO, Vec3::new(pulse, 1.0, pulse));
                    let _ = radius;
                    place(scene, self.sites_at + i * PER_SITE + 1, *at + Vec3::Y * 4.5, Vec3::ZERO, Vec3::ONE);
                }
            }
            // The bomb: carried bombs are on the carrier (the HUD says so), so draw it only on the ground, planted or lying.
            if matches!(s.bomb, 1 | 2) {
                let pos = Vec3::from(s.pos);
                place(scene, self.bomb_at, pos + Vec3::Y * 0.11, Vec3::ZERO, Vec3::ONE);
                // Beeping light: faster as the fuse runs down.
                let fuse = (s.fuse_ticks as f32 / 60.0).max(0.5);
                let rate = if s.bomb == 2 { 2.0 + 14.0 / fuse.min(14.0) } else { 0.6 };
                let on = (t * rate).fract() < 0.35;
                place(scene, self.bomb_at + 1, pos + Vec3::new(0.0, 0.25, 0.0), Vec3::ZERO, if on { Vec3::ONE * 1.4 } else { Vec3::ONE * 0.3 });
                if s.bomb == 2 {
                    place(
                        scene,
                        self.bomb_at + 2,
                        pos + Vec3::Y * 20.0,
                        Vec3::ZERO,
                        Vec3::new(1.0 + 0.5 * on as i32 as f32, 1.0, 1.0 + 0.5 * on as i32 as f32),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::protocol::{FlagSnap, SndSnap};

    fn scene(shooter_extra: &str) -> Scene {
        crate::schema::parse_scene(&format!(
            r##"{{"camera":{{"position":[0,1.7,0],"target":[0,1.7,-10]}},
            "shooter":{{"pickups":[],{shooter_extra}}},
            "objects":[{{"id":"floor","type":"plane","size":[80,80]}}]}}"##
        ))
        .unwrap_or_else(|e| panic!("{e:?}"))
    }

    const CTF: &str = r#""mode":"ctf","flags":[{"team":1,"at":[-20,0,0]},{"team":2,"at":[20,0,0]}]"#;
    const SND: &str = r#""mode":"snd","sites":[{"name":"A","at":[0,0,0],"radius":4},{"name":"B","at":[10,0,5],"radius":4}]"#;

    #[test]
    fn a_map_without_objectives_has_no_objective_world() {
        let mut s = scene(r#""mode":"tdm""#);
        assert!(ObjectiveWorld::add_to_scene(&mut s).is_none());
    }

    #[test]
    fn flags_are_added_hidden_and_shown_where_the_snapshot_says() {
        let mut s = scene(CTF);
        let before = s.objects.len();
        let mut w = ObjectiveWorld::add_to_scene(&mut s).unwrap();
        assert_eq!(s.objects.len() - before, w.object_count());
        assert_eq!(w.hidden_ids(&s).len(), w.object_count(), "everything starts hidden");
        let snap = ObjSnap {
            kind: 1,
            flags: [
                FlagSnap { state: 0, carrier: 255, pos: [-20.0, 0.0, 0.0], left_ticks: 0 },
                FlagSnap { state: 1, carrier: 3, pos: [5.0, 0.0, 2.0], left_ticks: 0 },
            ],
            ..Default::default()
        };
        w.update(&mut s, Some(&snap), 0.016);
        let pole1 = s.objects[w.flag_index(1, 0)].position.sample(0.0);
        assert!((pole1.x - 5.0).abs() < 1e-3 && (pole1.y - POLE_H / 2.0).abs() < 0.2, "the carried flag follows its carrier: {pole1:?}");
        let hidden = w.hidden_ids(&s).len();
        assert!(hidden < w.object_count(), "the flags are drawn");
        // The other mode's snapshot, or none, hides it all again.
        w.update(&mut s, None, 0.016);
        assert_eq!(w.hidden_ids(&s).len(), w.object_count());
    }

    #[test]
    fn sites_show_until_the_bomb_is_down_and_the_planted_bomb_beeps() {
        let mut s = scene(SND);
        let mut w = ObjectiveWorld::add_to_scene(&mut s).unwrap();
        let live = ObjSnap { kind: 2, snd: SndSnap { phase: 1, round: 1, attackers: 1, bomb: 0, carrier: 2, ..Default::default() }, ..Default::default() };
        w.update(&mut s, Some(&live), 0.016);
        let shown = w.object_count() - w.hidden_ids(&s).len();
        assert_eq!(shown, 4, "two sites, a ring and a marker each; the carried bomb is not drawn on the floor");
        let planted =
            ObjSnap { kind: 2, snd: SndSnap { phase: 1, bomb: 2, pos: [3.0, 0.0, 1.0], fuse_ticks: 600, ..Default::default() }, ..Default::default() };
        w.update(&mut s, Some(&planted), 0.016);
        let body = s.objects[w.bomb_at].position.sample(0.0);
        assert!((body.x - 3.0).abs() < 1e-3);
        assert_eq!(w.object_count() - w.hidden_ids(&s).len(), 3, "the sites go away; the bomb, its light and its beam show");
    }
}
