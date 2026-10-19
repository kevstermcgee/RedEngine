//! Worlds on disk: one folder per world under the game's own state folder (`settings::game_state_path`), holding `level.json` (the seed, the
//! game mode, the player, the inventory, the time of day, the furnaces and chests) and `chunks.bin` (only the chunks the player changed, each
//! run-length coded: the rest is generated again from the seed). Written to a temporary file and renamed, so a crash never leaves half a save.

use crate::chunk::ChunkPos;
use crate::craft::Furnace;
use crate::item::{Item, Stack};
use glam::{IVec3, Vec3};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// How the world is played.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameMode {
    Survival,
    Creative,
}

impl GameMode {
    pub fn name(self) -> &'static str {
        match self {
            GameMode::Survival => "survival",
            GameMode::Creative => "creative",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            GameMode::Survival => "Survival",
            GameMode::Creative => "Creative",
        }
    }
}

/// Everything about a world but its chunks.
#[derive(Clone, Debug, PartialEq)]
pub struct Level {
    pub name: String,
    pub seed: u32,
    pub mode: GameMode,
    pub pos: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub health: f32,
    pub flying: bool,
    pub spawn: Vec3,
    pub slots: Vec<Stack>,
    pub selected: usize,
    /// Time of day, 0..1.
    pub time: f32,
    pub days: u32,
    pub furnaces: HashMap<IVec3, Furnace>,
    pub chests: HashMap<IVec3, Vec<Stack>>,
    /// Seconds played.
    pub played: f64,
    /// When it was last played (seconds since 1970).
    pub last_played: u64,
}

fn stack_json(s: &Stack) -> Value {
    if s.is_empty() {
        Value::Null
    } else if s.wear > 0 {
        json!([s.item.name(), s.count, s.wear])
    } else {
        json!([s.item.name(), s.count])
    }
}

fn stack_of(v: &Value) -> Stack {
    let Some(a) = v.as_array() else { return Stack::EMPTY };
    let Some(item) = a.first().and_then(Value::as_str).and_then(Item::by_name) else { return Stack::EMPTY };
    let count = a.get(1).and_then(Value::as_u64).unwrap_or(1).clamp(1, 64) as u8;
    let wear = a.get(2).and_then(Value::as_u64).unwrap_or(0) as u16;
    Stack { item, count, wear }
}

fn v3(v: Vec3) -> Value {
    json!([v.x, v.y, v.z])
}

fn vec3_of(v: Option<&Value>) -> Option<Vec3> {
    let a = v?.as_array()?;
    Some(Vec3::new(a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32, a.get(2)?.as_f64()? as f32))
}

fn ivec_key(p: IVec3) -> String {
    format!("{},{},{}", p.x, p.y, p.z)
}

fn ivec_of(s: &str) -> Option<IVec3> {
    let v: Vec<i32> = s.split(',').filter_map(|p| p.parse().ok()).collect();
    (v.len() == 3).then(|| IVec3::new(v[0], v[1], v[2]))
}

impl Level {
    pub fn to_json(&self) -> Value {
        let furnaces: serde_json::Map<String, Value> = self
            .furnaces
            .iter()
            .map(|(p, f)| (ivec_key(*p), json!({"input": stack_json(&f.input), "fuel": stack_json(&f.fuel), "output": stack_json(&f.output), "burn": f.burn, "burn_total": f.burn_total, "cook": f.cook})))
            .collect();
        let chests: serde_json::Map<String, Value> =
            self.chests.iter().map(|(p, s)| (ivec_key(*p), Value::Array(s.iter().map(stack_json).collect()))).collect();
        json!({
            "format": "redcraft-level/1",
            "name": self.name,
            "seed": self.seed,
            "mode": self.mode.name(),
            "player": {"pos": v3(self.pos), "yaw": self.yaw, "pitch": self.pitch, "health": self.health, "flying": self.flying, "spawn": v3(self.spawn)},
            "inventory": {"selected": self.selected, "slots": self.slots.iter().map(stack_json).collect::<Vec<_>>()},
            "time": self.time,
            "days": self.days,
            "furnaces": furnaces,
            "chests": chests,
            "played": self.played,
            "last_played": self.last_played,
        })
    }

    pub fn from_json(v: &Value) -> Result<Level, String> {
        if v.get("format").and_then(Value::as_str) != Some("redcraft-level/1") {
            return Err("not a RedCraft level (format redcraft-level/1)".to_string());
        }
        let p = v.get("player").cloned().unwrap_or(Value::Null);
        let inv = v.get("inventory").cloned().unwrap_or(Value::Null);
        let mut slots: Vec<Stack> = inv.get("slots").and_then(Value::as_array).map(|a| a.iter().map(stack_of).collect()).unwrap_or_default();
        slots.resize(crate::inventory::SLOTS, Stack::EMPTY);
        let furnaces = v
            .get("furnaces")
            .and_then(Value::as_object)
            .map(|o| {
                o.iter()
                    .filter_map(|(k, f)| {
                        let num = |key: &str| f.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32;
                        Some((
                            ivec_of(k)?,
                            Furnace {
                                input: stack_of(f.get("input")?),
                                fuel: stack_of(f.get("fuel")?),
                                output: stack_of(f.get("output")?),
                                burn: num("burn"),
                                burn_total: num("burn_total"),
                                cook: num("cook"),
                            },
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let chests = v
            .get("chests")
            .and_then(Value::as_object)
            .map(|o| {
                o.iter()
                    .filter_map(|(k, s)| {
                        let mut slots: Vec<Stack> = s.as_array()?.iter().map(stack_of).collect();
                        slots.resize(27, Stack::EMPTY);
                        Some((ivec_of(k)?, slots))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let pos = vec3_of(p.get("pos")).ok_or("player.pos is missing")?;
        Ok(Level {
            name: v.get("name").and_then(Value::as_str).unwrap_or("World").to_string(),
            seed: v.get("seed").and_then(Value::as_u64).ok_or("seed is missing")? as u32,
            mode: if v.get("mode").and_then(Value::as_str) == Some("creative") { GameMode::Creative } else { GameMode::Survival },
            pos,
            yaw: p.get("yaw").and_then(Value::as_f64).unwrap_or(0.0) as f32,
            pitch: p.get("pitch").and_then(Value::as_f64).unwrap_or(0.0) as f32,
            health: p.get("health").and_then(Value::as_f64).unwrap_or(20.0) as f32,
            flying: p.get("flying").and_then(Value::as_bool).unwrap_or(false),
            spawn: vec3_of(p.get("spawn")).unwrap_or(pos),
            slots,
            selected: inv.get("selected").and_then(Value::as_u64).unwrap_or(0).min(8) as usize,
            time: v.get("time").and_then(Value::as_f64).unwrap_or(0.3) as f32,
            days: v.get("days").and_then(Value::as_u64).unwrap_or(0) as u32,
            furnaces,
            chests,
            played: v.get("played").and_then(Value::as_f64).unwrap_or(0.0),
            last_played: v.get("last_played").and_then(Value::as_u64).unwrap_or(0),
        })
    }
}

/// The chunks file: "RCC1", then for each chunk x, z (i32 LE), length (u32 LE), bytes.
pub fn encode_chunks(chunks: &[(ChunkPos, Vec<u8>)]) -> Vec<u8> {
    let mut out = b"RCC1".to_vec();
    for (p, b) in chunks {
        out.extend_from_slice(&p.x.to_le_bytes());
        out.extend_from_slice(&p.z.to_le_bytes());
        out.extend_from_slice(&(b.len() as u32).to_le_bytes());
        out.extend_from_slice(b);
    }
    out
}

pub fn decode_chunks(bytes: &[u8]) -> Result<HashMap<ChunkPos, Vec<u8>>, String> {
    if bytes.len() < 4 || &bytes[..4] != b"RCC1" {
        return Err("not a RedCraft chunks file".to_string());
    }
    let mut out = HashMap::new();
    let mut i = 4;
    let rd = |b: &[u8], at: usize| -> Option<[u8; 4]> { b.get(at..at + 4)?.try_into().ok() };
    while i < bytes.len() {
        let x = i32::from_le_bytes(rd(bytes, i).ok_or("cut short")?);
        let z = i32::from_le_bytes(rd(bytes, i + 4).ok_or("cut short")?);
        let n = u32::from_le_bytes(rd(bytes, i + 8).ok_or("cut short")?) as usize;
        let body = bytes.get(i + 12..i + 12 + n).ok_or("cut short")?;
        out.insert(ChunkPos::new(x, z), body.to_vec());
        i += 12 + n;
    }
    Ok(out)
}

/// The folder all worlds live in, for the game whose settings key is `key` (`RED_REDCRAFT_SAVES` overrides it, for tests).
pub fn worlds_dir(key: &str) -> Option<PathBuf> {
    if let Ok(d) = std::env::var("RED_REDCRAFT_SAVES") {
        return Some(PathBuf::from(d));
    }
    red_engine2::settings::game_state_path(key, "worlds")
}

/// A folder name for a world's name.
pub fn slug(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "world".to_string()
    } else {
        s
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Writes a world.
pub fn save(dir: &Path, level: &Level, chunks: &[(ChunkPos, Vec<u8>)]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    write_atomic(&dir.join("chunks.bin"), &encode_chunks(chunks))?;
    let text = serde_json::to_string_pretty(&level.to_json()).map_err(|e| e.to_string())?;
    write_atomic(&dir.join("level.json"), text.as_bytes())
}

/// Reads a world.
pub fn load(dir: &Path) -> Result<(Level, HashMap<ChunkPos, Vec<u8>>), String> {
    let text = std::fs::read_to_string(dir.join("level.json")).map_err(|e| format!("{}: {e}", dir.join("level.json").display()))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("level.json: {e}"))?;
    let level = Level::from_json(&v)?;
    let chunks = match std::fs::read(dir.join("chunks.bin")) {
        Ok(b) => decode_chunks(&b)?,
        Err(_) => HashMap::new(),
    };
    Ok((level, chunks))
}

/// The worlds in `dir`, most recently played first: (folder, level).
pub fn list(dir: &Path) -> Vec<(PathBuf, Level)> {
    let mut v: Vec<(PathBuf, Level)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let text = std::fs::read_to_string(p.join("level.json")).ok()?;
            let level = Level::from_json(&serde_json::from_str(&text).ok()?).ok()?;
            Some((p, level))
        })
        .collect();
    v.sort_by_key(|e| std::cmp::Reverse(e.1.last_played));
    v
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::id;

    #[test]
    fn a_world_comes_back_as_it_was_saved() {
        let dir = std::env::temp_dir().join(format!("redcraft-save-{}", std::process::id()));
        let mut slots = vec![Stack::EMPTY; crate::inventory::SLOTS];
        slots[0] = Stack::new(Item(id::IRON_PICKAXE), 1);
        slots[0].wear = 17;
        slots[5] = Stack::new(Item::of(crate::block::Block::TORCH), 12);
        let mut furnaces = HashMap::new();
        furnaces.insert(IVec3::new(4, 70, -3), Furnace { input: Stack::new(Item::of(crate::block::Block::SAND), 3), burn: 12.5, ..Default::default() });
        let mut chests = HashMap::new();
        let mut c = vec![Stack::EMPTY; 27];
        c[26] = Stack::new(Item(id::DIAMOND), 5);
        chests.insert(IVec3::new(-1, 64, 9), c);
        let level = Level {
            name: "My World".to_string(),
            seed: 1234,
            mode: GameMode::Creative,
            pos: Vec3::new(1.5, 70.0, -2.25),
            yaw: 1.0,
            pitch: -0.2,
            health: 13.0,
            flying: true,
            spawn: Vec3::new(0.5, 66.0, 0.5),
            slots,
            selected: 5,
            time: 0.61,
            days: 3,
            furnaces,
            chests,
            played: 100.5,
            last_played: 7,
        };
        let chunks = vec![(ChunkPos::new(-2, 5), vec![1, 2, 3]), (ChunkPos::new(0, 0), vec![9; 40])];
        save(&dir, &level, &chunks).unwrap();
        let (back, back_chunks) = load(&dir).unwrap();
        assert_eq!(back, level);
        assert_eq!(back_chunks.get(&ChunkPos::new(-2, 5)), Some(&vec![1, 2, 3]));
        assert_eq!(back_chunks.len(), 2);
        assert_eq!(list(dir.parent().unwrap()).iter().filter(|(p, _)| *p == dir).count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(slug("My World!"), "my-world");
        assert!(decode_chunks(b"RCC1\x01\x00").is_err());
    }
}
