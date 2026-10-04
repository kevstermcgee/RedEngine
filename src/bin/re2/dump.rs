//! The client's state as JSON (`re2 --dump state.json`, a script's `snapshot` and `expect` steps): the answer to "what would the player see and hear?" without looking at a
//! screen. Which remote players are drawn (and in which avatar), the HUD's text, the sounds played, the crosshair's state, and a counter for every way something can fail
//! to be drawn (ADR 2026-09-28-seeing-what-the-player-sees).

use super::*;
use serde_json::{json, Value};

/// What the crosshair looks like this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CrosshairState {
    /// White: nothing in particular.
    Plain,
    /// Gold: something a swing would hit is within reach.
    Reach,
    /// Green: a loose prop can be picked up.
    Pickup,
    /// Red: an enemy is in the sights.
    Enemy,
}

impl CrosshairState {
    fn name(self) -> &'static str {
        match self {
            CrosshairState::Plain => "plain",
            CrosshairState::Reach => "reach",
            CrosshairState::Pickup => "pickup",
            CrosshairState::Enemy => "enemy",
        }
    }
}

fn vec3(v: Vec3) -> Value {
    json!([round2(v.x), round2(v.y), round2(v.z)])
}

fn round2(x: f32) -> f64 {
    (f64::from(x) * 100.0).round() / 100.0
}

impl App {
    /// The crosshair's state, by the same precedence the renderer draws it with (enemy over pick-up over reach).
    pub(crate) fn crosshair_state(&self) -> CrosshairState {
        if self.aim_enemy {
            CrosshairState::Enemy
        } else if self.pickup_target.is_some() {
            CrosshairState::Pickup
        } else if self.target_index.is_some() {
            CrosshairState::Reach
        } else {
            CrosshairState::Plain
        }
    }

    fn phase_name(&self) -> &'static str {
        match self.phase {
            Phase::Connect => "connect",
            Phase::Playing => "playing",
        }
    }

    /// The whole state as one JSON document (schema `re2-dump/1`).
    pub(crate) fn state_dump(&self) -> Value {
        let size = self.window_size().map(|(w, h)| json!([w, h]));
        let online = self.net.as_ref().map(|n| {
            let c = &n.client;
            let roster: Vec<Value> = c
                .status()
                .map(|s| {
                    s.roster
                        .iter()
                        .map(|e| {
                            json!({"id": e.id, "name": e.name, "score": e.score, "bot": e.flags & red_engine2::net::protocol::ROSTER_BOT != 0,
                                   "in_round": e.flags & red_engine2::net::protocol::ROSTER_IN_ROUND != 0, "ready": e.flags & red_engine2::net::protocol::ROSTER_READY != 0,
                                   "character": red_engine2::net::protocol::character_from_wire(e.character).name()})
                        })
                        .collect()
                })
                .unwrap_or_default();
            json!({
                "connected": c.state() == red_engine2::net::client::ConnState::Connected,
                "id": c.my_id(),
                "status": n.status,
                "ping_ms": (c.stats().rtt_ms * 10.0).round() / 10.0,
                "phase": format!("{:?}", c.phase()),
                "in_round": c.in_round(),
                "round": c.status().map(|s| s.round),
                "roster": roster,
                "snapshots": c.stats().snapshots,
                "snapshots_missed": c.stats().snapshots_missed,
            })
        });
        let remote = self.net.as_ref().map(|n| {
            let s = n.stats();
            let c = n.counters();
            json!({
                "in_view": s.in_view, "drawn": s.drawn, "standins": s.standins, "undrawn": s.undrawn, "undrawn_ids": s.undrawn_ids,
                "unposed": s.unposed, "roster_others": s.roster_others, "hidden_by_interest": s.hidden_by_interest,
                "pool": {"capacity": s.pool_capacity, "in_use": s.pool_in_use},
                "bodies": n.avatar_pool_stats().iter().map(|(b, p)| json!({"body": b.name(), "capacity": p.capacity, "in_use": p.in_use, "high_water": p.high_water, "failed_claims": p.failed_claims})).collect::<Vec<_>>(),
                "counters": {"frames": c.frames, "undrawn_frames": c.undrawn_frames, "standin_frames": c.standin_frames, "hidden_frames": c.hidden_frames, "unknown_props": c.unknown_props},
                "players": n.drawn_players().iter().map(|d| json!({
                    "id": d.id, "body": d.body.name(), "avatar": d.avatar, "avatar_body": d.avatar_body.name(), "pos": vec3(d.pos), "dead": d.dead,
                })).collect::<Vec<_>>(),
            })
        });
        let own = self.net.as_ref().and_then(|n| n.own);
        let mut failures: Vec<String> = self.failures.clone();
        failures.extend(self.shots.errors.iter().cloned());
        json!({
            "schema": "re2-dump/1",
            "map": self.scene_path.display().to_string(),
            "frame": self.frame_no,
            "secs": round2(self.play_secs),
            "headless": self.headless,
            "gpu": self.gpu_kind,
            "size": size,
            "phase": self.phase_name(),
            "view": if self.view_mode == ViewMode::ThirdPerson { "third" } else { "first" },
            "player": {
                "character": self.character.name(),
                "pos": vec3(Vec3::new(self.physics_pos.x, self.foot_y, self.physics_pos.y)),
                "yaw_deg": round2(self.camera.yaw.to_degrees().rem_euclid(360.0)),
                "pitch_deg": round2(self.camera.pitch.to_degrees()),
                "weapon": self.shown_weapon().name(),
                "hp": own.map(|o| o.hp),
                "dead": self.own_dead(),
                "carrying": self.carrying(),
            },
            "players": self.players_state(),
            "audio": self.ambient_state(),
            "online": online,
            "remote": remote,
            "card": self.card_content().map(|(c, button)| json!({"kind": self.card.map(|k| format!("{:?}", k.kind).to_lowercase()), "title": c.title, "text": c.text, "button": c.button.map(|label| json!({"id": button, "label": label}))})),
            "hud": {"lines": self.hud_lines.iter().map(|(id, text)| json!({"id": id, "text": text})).collect::<Vec<_>>()},
            "rules": {
                "vars": self.rules.vars().iter().map(|(n, v)| json!({"name": n, "value": v})).collect::<Vec<_>>(),
                "ended": self.rules.ended(),
                "last_event": self.rule_event,
                "hidden": self.rules.hidden().collect::<Vec<_>>(),
            },
            // Every loose prop of the offline world (online the server owns them: null); the same view the rules get.
            "props": self.props.as_ref().map(|p| {
                (0..p.props().len())
                    .map(|k| {
                        let v = red_engine2::sim::rules_run::RuleProp::of(p, k);
                        json!({
                            "id": self.scene.objects[p.props()[k].object_index].id, "pos": vec3(v.origin), "tilt_deg": round2(v.tilt_deg),
                            "moved": round2(v.moved), "asleep": p.is_asleep(k), "held_by": v.held_by,
                        })
                    })
                    .collect::<Vec<_>>()
            }),
            "cues": {
                "counts": self.cue_counts,
                "recent": self.cue_log.iter().map(|(t, cue)| json!({"t": round2(*t), "cue": cue})).collect::<Vec<_>>(),
            },
            "crosshair": {
                "state": self.crosshair_state().name(),
                "enemy": self.aim_enemy,
                "pickup": self.pickup_target.is_some(),
                "in_reach": self.target_index.is_some(),
            },
            "streaks": {"showing": self.streaks.as_ref().map_or(0, |s| s.len())},
            "shots": self.shots.taken.iter().map(|s| json!({"name": s.name, "file": s.file.display().to_string(), "secs": round2(s.secs), "camera": s.camera, "flat_fraction": round2(s.flat)})).collect::<Vec<_>>(),
            "snapshots": self.snapshots.iter().map(|(n, v)| (n.clone(), v.clone())).collect::<serde_json::Map<String, Value>>(),
            "failures": failures,
        })
    }

    /// The lines of the `F3` overlay: frame rate and, online, ping and every counter that says something was not drawn.
    pub(crate) fn debug_lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "{:.0} fps  {:.1} ms/frame  {} scene objects  {:.0} s",
            self.fps_avg,
            1000.0 / self.fps_avg.max(1.0),
            self.scene.objects.len(),
            self.play_secs
        )];
        if let Some(net) = &self.net {
            let (s, c, cs) = (net.stats(), net.counters(), net.client.stats());
            lines.push(format!("ping {:.0} ms  snapshots {} (missed {})", cs.rtt_ms, cs.snapshots, cs.snapshots_missed));
            lines.push(format!("others: {} in view, {} drawn, {} UNDRAWN, {} stand-in, {} unposed", s.in_view, s.drawn, s.undrawn, s.standins, s.unposed));
            lines
                .push(format!("roster others {}  hidden by interest {}  avatars {}/{}", s.roster_others, s.hidden_by_interest, s.pool_in_use, s.pool_capacity));
            lines.push(format!(
                "bad frames: {} undrawn, {} stand-in, {} hidden; unknown props {}",
                c.undrawn_frames, c.standin_frames, c.hidden_frames, c.unknown_props
            ));
        }
        if let Some(last) = self.failures.last() {
            lines.push(format!("{} FAILURE(S): {last}", self.failures.len()));
        }
        lines
    }

    /// Writes [`state_dump`](Self::state_dump) to `path` (creating the folder).
    pub(crate) fn write_dump(&self, path: &std::path::Path) -> Result<(), String> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let text = serde_json::to_string_pretty(&self.state_dump()).map_err(|e| e.to_string())?;
        std::fs::write(path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))
    }
}
