//! Running a [`RuleSet`]: the deterministic state machine behind a scene's `rules`.
//!
//! [`RulesEngine::step_props`] is called once per simulation tick with where every player and every loose prop is; it
//! reports what the world must do as [`Effect`]s (teleport a player, shove / reset / place a prop) and keeps everything
//! else — variables, which rules have fired, hidden objects, the event log, whether the match ended — as plain data that
//! [`RulesEngine::checksum`] folds into the match checksum, so a replay that diverges in game logic is detected.
//! Nothing here knows about sockets, windows or physics: it is fed positions and returns effects. Props are described
//! as [`RuleProp`]s in the physics world's order; [`RulesEngine::bind_props`] maps the ids a rule set names to that order once.

use super::clock::TICK_RATE_HZ;
use super::match_sim::MAX_PLAYERS;
use super::rules::{Action, ResetTarget, Rule, RuleSet, Target, Volume, When, Who};
use super::rules_expr::{Func, World};
use crate::player::Character;
use glam::Vec3;
use std::collections::{BTreeMap, BTreeSet};

/// Most events kept in [`RulesEngine::history`] (the oldest are dropped).
pub const MAX_HISTORY: usize = 4096;
/// How many times events may trigger further events within one tick (a rule that emits what it listens for stops here).
pub const MAX_CHAIN: usize = 4;

/// A player as the rules see them.
#[derive(Debug, Clone, Copy)]
pub struct RulePlayer {
    /// The player's slot.
    pub slot: usize,
    /// Feet position.
    pub pos: Vec3,
    /// Body radius, m.
    pub radius: f32,
    /// Body height, m.
    pub height: f32,
    /// Which body.
    pub character: Character,
    /// Team 1 or 2, or `0` outside a teamed match (see `MatchSim::teams_enabled`).
    pub team: u8,
}

/// A loose prop as the rules see it (one per prop, in the physics world's order).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RuleProp {
    /// Its origin (the frame of its authored `position`), world space.
    pub origin: Vec3,
    /// Height of its box above the origin, m (its band for volume tests).
    pub height: f32,
    /// Degrees its up axis leans from how the map author placed it (0 upright, 90 on its side).
    pub tilt_deg: f32,
    /// The player carrying it, if any.
    pub held_by: Option<usize>,
    /// Mass, kg.
    pub mass: f32,
    /// Metres its origin is from where the map author put it.
    pub moved: f32,
    /// Its linear velocity, m/s (what a `field` pushes toward its target from).
    pub vel: Vec3,
}

impl RuleProp {
    /// How the rules see physics prop `prop` of `props` right now.
    pub fn of(props: &crate::physics::PropWorld, prop: usize) -> RuleProp {
        let pose = props.prop_pose(prop);
        RuleProp {
            origin: pose.w_axis.truncate(),
            height: props.props()[prop].shape.extents.y,
            tilt_deg: props.tilt_deg(prop),
            held_by: props.holder_of(prop),
            mass: props.mass(prop),
            moved: props.moved(prop),
            vel: props.prop_velocity(prop),
        }
    }
}

/// Whether a loose prop counts as inside a volume: its origin inside in x/z, and its height band (with a 0.1 m
/// allowance for a prop settled a hair into the floor) overlapping the volume in y.
pub fn prop_inside(v: &Volume, p: &RuleProp) -> bool {
    p.origin.x >= v.min.x
        && p.origin.x <= v.max.x
        && p.origin.z >= v.min.z
        && p.origin.z <= v.max.z
        && p.origin.y + p.height.max(0.0) >= v.min.y - 0.1
        && p.origin.y <= v.max.y
}

/// What the expression built-ins read while a tick's rules run: the props, the binding from a rule set's prop ids to
/// physics props, and the zones.
struct Ctx<'a> {
    props: &'a [RuleProp],
    slot: &'a [Option<usize>],
    zones: &'a [Volume],
}

impl Ctx<'_> {
    fn prop(&self, index: usize) -> Option<&RuleProp> {
        self.slot.get(index).copied().flatten().and_then(|k| self.props.get(k))
    }
}

impl World for Ctx<'_> {
    fn call(&self, f: Func, index: usize) -> f64 {
        if f.takes_zone() {
            let Some(v) = self.zones.get(index) else { return 0.0 };
            return self.props.iter().filter(|p| prop_inside(v, p)).count() as f64;
        }
        let Some(p) = self.prop(index) else { return 0.0 };
        match f {
            Func::PropY => p.origin.y as f64,
            Func::Tilt => p.tilt_deg as f64,
            Func::Held => p.held_by.is_some() as u8 as f64,
            Func::Mass => p.mass as f64,
            Func::Moved => p.moved as f64,
            Func::PropsIn | Func::InZone => 0.0,
        }
    }

    fn call2(&self, f: Func, prop: usize, zone: usize) -> f64 {
        if f != Func::InZone {
            return 0.0;
        }
        match (self.prop(prop), self.zones.get(zone)) {
            (Some(p), Some(v)) => prop_inside(v, p) as u8 as f64,
            _ => 0.0,
        }
    }
}

/// A [`Ctx`] plus the per-player variables of the player a rule is running for: answers `me.name` from that row.
struct MeView<'a> {
    base: &'a dyn World,
    row: &'a [f64],
}

impl World for MeView<'_> {
    fn call(&self, f: Func, index: usize) -> f64 {
        self.base.call(f, index)
    }
    fn call2(&self, f: Func, prop: usize, zone: usize) -> f64 {
        self.base.call2(f, prop, zone)
    }
    fn me(&self, index: usize) -> f64 {
        self.row.get(index).copied().unwrap_or(0.0)
    }
}

/// Something that happened because a rule ran.
#[derive(Debug, Clone, PartialEq)]
pub struct GameEvent {
    /// The tick it happened on.
    pub tick: u64,
    /// The rule that emitted it.
    pub rule: String,
    /// The event name (`emit`), or `end:<reason>` when the match ended.
    pub name: String,
    /// The player that triggered the rule, if any.
    pub slot: Option<usize>,
}

/// A change the rules ask the world to make (the engine cannot do it itself).
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Move a player.
    Teleport {
        /// Which player.
        slot: usize,
        /// Where.
        target: Target,
    },
    /// Give a prop a shove.
    Impulse {
        /// Object id of the prop.
        object: String,
        /// Direction.
        dir: Vec3,
        /// Speed, m/s.
        speed: f32,
    },
    /// Put a loose prop back where the map author placed it, at rest (physics prop index).
    Reset {
        /// The prop.
        prop: usize,
    },
    /// Move a loose prop's origin to a point, upright as authored, at rest (physics prop index).
    Place {
        /// The prop.
        prop: usize,
        /// Where.
        at: Vec3,
    },
}

/// The live state of a scene's rules. See the module docs.
#[derive(Debug, Clone)]
pub struct RulesEngine {
    set: RuleSet,
    vars: Vec<f64>,
    /// Per-player variables: `MAX_PLAYERS` rows of `pn` values, one row per player slot.
    pvars: Vec<f64>,
    /// How many per-player variables the scene declares (a row's length). Kept here because `self.set` is taken out of the engine while a tick runs.
    pn: usize,
    fired: Vec<bool>,
    next_ok: Vec<u64>,
    inside: BTreeMap<(usize, usize), bool>,
    /// Physics prop index of each of the set's `prop_ids` (see [`bind_props`](Self::bind_props)).
    prop_slot: Vec<Option<usize>>,
    /// `(rule, physics prop) -> inside` for `prop_enter` / `prop_exit` rules.
    prop_inside: BTreeMap<(usize, usize), bool>,
    /// `rule -> below` for `prop_below` rules.
    below: BTreeMap<usize, bool>,
    hidden: BTreeSet<String>,
    collision_disabled: BTreeSet<String>,
    ended: Option<String>,
    started: bool,
    history: Vec<GameEvent>,
    dropped: u64,
    pending: Vec<GameEvent>,
    /// Engine-raised events waiting to trigger `when: {event}` rules on the next step.
    injected: Vec<(String, Option<usize>)>,
    /// The scene's looping axis: a player near the seam is inside a volume just past the other edge (see `expanse`).
    wrap: Option<crate::expanse::Wrap>,
}

impl RulesEngine {
    /// Starts a match with `set`.
    pub fn new(set: RuleSet) -> Self {
        RulesEngine {
            vars: set.var_init.clone(),
            pvars: set.player_var_init.iter().copied().cycle().take(set.player_var_init.len() * MAX_PLAYERS).collect(),
            pn: set.player_var_init.len(),
            fired: vec![false; set.rules.len()],
            next_ok: vec![0; set.rules.len()],
            prop_slot: vec![None; set.prop_ids.len()],
            set,
            inside: BTreeMap::new(),
            prop_inside: BTreeMap::new(),
            below: BTreeMap::new(),
            hidden: BTreeSet::new(),
            collision_disabled: BTreeSet::new(),
            ended: None,
            started: false,
            history: Vec::new(),
            dropped: 0,
            pending: Vec::new(),
            injected: Vec::new(),
            wrap: None,
        }
    }

    /// Measures `enter` / `exit` volumes the short way round a looping world (the scene's `world.wrap`).
    pub fn with_wrap(mut self, wrap: Option<crate::expanse::Wrap>) -> Self {
        self.wrap = wrap;
        self
    }

    /// True when the scene declared any rule (a match without rules never ends and has no events).
    pub fn has_rules(&self) -> bool {
        !self.set.rules.is_empty()
    }

    /// The parsed rule set (its variables, loose prop ids and zones).
    pub fn set(&self) -> &RuleSet {
        &self.set
    }

    /// True when some rule looks at loose props, so [`step_props`](Self::step_props) needs them described every tick.
    pub fn needs_props(&self) -> bool {
        self.set.needs_props
    }

    /// Maps every loose prop id the rule set names to its index in the physics world (`None`: no such prop, so rules
    /// about it never fire and its built-ins read 0). Call once, when the world exists.
    pub fn bind_props(&mut self, resolve: impl Fn(&str) -> Option<usize>) {
        self.prop_slot = self.set.prop_ids.iter().map(|id| resolve(id)).collect();
    }

    /// The outcome the match ended with (`end` action), if it has.
    pub fn ended(&self) -> Option<&str> {
        self.ended.as_deref()
    }

    /// The value of a variable by name (built-ins included).
    pub fn var(&self, name: &str) -> Option<f64> {
        self.set.var_names.iter().position(|n| n == name).map(|i| self.vars[i])
    }

    /// Player `slot`'s per-player variables as `(name, value)` (empty when the scene declares none or the slot is out of range).
    pub fn player_vars(&self, slot: usize) -> Vec<(&str, f64)> {
        let row = self.row(Some(slot));
        self.set.player_var_names.iter().zip(row).map(|(n, v)| (n.as_str(), *v)).collect()
    }

    /// One per-player variable of player `slot`.
    pub fn player_var(&self, slot: usize, name: &str) -> Option<f64> {
        let k = self.set.player_var_names.iter().position(|n| n == name)?;
        self.row(Some(slot)).get(k).copied()
    }

    /// Puts player `slot`'s per-player variables back to their declared starting values: a player joined that slot.
    pub fn reset_player(&mut self, slot: usize) {
        let n = self.pn;
        if slot < MAX_PLAYERS && n > 0 {
            self.pvars[slot * n..(slot + 1) * n].copy_from_slice(&self.set.player_var_init);
        }
    }

    /// The row of per-player variables for `slot` (empty without a player or a declaration).
    fn row(&self, slot: Option<usize>) -> &[f64] {
        let n = self.pn;
        match slot {
            Some(s) if n > 0 && s < MAX_PLAYERS => &self.pvars[s * n..(s + 1) * n],
            _ => &[],
        }
    }

    /// The scene's own variables (not the built-ins) as `(name, value)`.
    pub fn vars(&self) -> Vec<(&str, f64)> {
        self.set.var_names.iter().zip(&self.vars).skip(super::rules::BUILTIN_VARS.len()).map(|(n, v)| (n.as_str(), *v)).collect()
    }

    /// Sets a scene variable (not a built-in) to a value; false when there is no such variable.
    pub fn set_var(&mut self, name: &str, value: f64) -> bool {
        match self.set.var_names.iter().position(|n| n == name).filter(|i| *i >= super::rules::BUILTIN_VARS.len()) {
            Some(i) => {
                self.vars[i] = value;
                true
            }
            None => false,
        }
    }

    /// The variables the scene keeps between sessions (`persist`) with their values.
    pub fn persisted(&self) -> Vec<(String, f64)> {
        self.set.persist.iter().filter_map(|n| self.var(n).map(|v| (n.clone(), v))).collect()
    }

    /// Ids of objects a rule has hidden.
    pub fn hidden(&self) -> impl Iterator<Item = &str> {
        self.hidden.iter().map(String::as_str)
    }

    /// Top-level objects whose authored static collision is currently disabled.
    pub fn collision_disabled(&self) -> impl Iterator<Item = &str> {
        self.collision_disabled.iter().map(String::as_str)
    }

    /// Every event so far (at most [`MAX_HISTORY`]; the oldest are dropped).
    pub fn history(&self) -> &[GameEvent] {
        &self.history
    }

    /// How many old events were dropped from [`RulesEngine::history`].
    pub fn dropped_events(&self) -> u64 {
        self.dropped
    }

    /// Records an event the *engine* raised (`pickup`, `hit`, `kill`, ...: see [`super::rules::ENGINE_EVENTS`]): it goes
    /// into the history/new-event lists like an `emit`, and any `when: {event}` rule reacts on the next [`step`](Self::step).
    pub fn inject(&mut self, tick: u64, name: &str, slot: Option<usize>) {
        self.record(tick, "engine", name.to_string(), slot);
        if self.injected.len() < 64 {
            self.injected.push((name.to_string(), slot));
        }
    }

    /// Events since the last call (a server logs and clears these each tick).
    pub fn take_new_events(&mut self) -> Vec<GameEvent> {
        std::mem::take(&mut self.pending)
    }

    fn inside(v: &Volume, p: &RulePlayer, wrap: Option<crate::expanse::Wrap>) -> bool {
        let mut pos = p.pos;
        if let Some(w) = wrap {
            // The copy of the player nearest to the volume: across the seam of a looping world it is one period away.
            let centre = glam::Vec2::new((v.min.x + v.max.x) * 0.5, (v.min.z + v.max.z) * 0.5);
            let near = w.nearest_image(centre, glam::Vec2::new(pos.x, pos.z));
            pos.x = near.x;
            pos.z = near.y;
        }
        let cx = pos.x.clamp(v.min.x, v.max.x);
        let cz = pos.z.clamp(v.min.z, v.max.z);
        let (dx, dz) = (pos.x - cx, pos.z - cz);
        dx * dx + dz * dz <= p.radius * p.radius && pos.y + p.height >= v.min.y && pos.y <= v.max.y
    }

    fn who_ok(who: Who, c: Character, team: u8) -> bool {
        match who {
            Who::Any => true,
            Who::Human => c != Character::Rat,
            Who::Rat => c == Character::Rat,
            Who::Team1 => team == 1,
            Who::Team2 => team == 2,
        }
    }

    fn record(&mut self, tick: u64, rule: &str, name: String, slot: Option<usize>) {
        let ev = GameEvent { tick, rule: rule.to_string(), name, slot };
        if self.history.len() >= MAX_HISTORY {
            self.history.remove(0);
            self.dropped += 1;
        }
        self.history.push(ev.clone());
        if self.pending.len() < 256 {
            self.pending.push(ev);
        }
    }

    /// Runs the actions of rule `ri` of `set` if its guards allow it. Emitted event names are pushed onto `queue`.
    #[allow(clippy::too_many_arguments)]
    fn fire(
        &mut self,
        set: &RuleSet,
        ri: usize,
        tick: u64,
        slot: Option<usize>,
        queue: &mut Vec<(String, Option<usize>)>,
        effects: &mut Vec<Effect>,
        ctx: &Ctx<'_>,
    ) {
        let rule: &Rule = &set.rules[ri];
        if (rule.once && self.fired[ri]) || tick < self.next_ok[ri] || self.ended.is_some() {
            return;
        }
        if let Some(c) = &rule.cond {
            let me = MeView { base: ctx, row: self.row(slot) };
            if !c.truthy_in(&self.vars, &me) {
                return;
            }
        }
        self.fired[ri] = true;
        self.next_ok[ri] = tick + rule.cooldown_ticks;
        for a in &rule.actions {
            match a {
                Action::Set { var, value } => {
                    let v = value.eval_in(&self.vars, &MeView { base: ctx, row: self.row(slot) });
                    if let Some(slot_v) = self.vars.get_mut(*var) {
                        *slot_v = v;
                    }
                }
                Action::SetMe { var, value } => {
                    let n = self.pn;
                    if let Some(s) = slot.filter(|s| *s < MAX_PLAYERS) {
                        let v = value.eval_in(&self.vars, &MeView { base: ctx, row: self.row(slot) });
                        if let Some(cell) = self.pvars.get_mut(s * n + *var) {
                            *cell = v;
                        }
                    }
                }
                Action::Emit(name) => {
                    self.record(tick, &rule.id, name.clone(), slot);
                    queue.push((name.clone(), slot));
                }
                Action::Hide(o) => {
                    self.hidden.insert(o.clone());
                }
                Action::Show(o) => {
                    self.hidden.remove(o);
                }
                Action::Deactivate(o) => {
                    self.hidden.insert(o.clone());
                    self.collision_disabled.insert(o.clone());
                }
                Action::Activate(o) => {
                    self.hidden.remove(o);
                    self.collision_disabled.remove(o);
                }
                Action::Collision { object, enabled } => {
                    if *enabled {
                        self.collision_disabled.remove(object);
                    } else {
                        self.collision_disabled.insert(object.clone());
                    }
                }
                Action::Teleport(target) => {
                    if let Some(slot) = slot {
                        effects.push(Effect::Teleport { slot, target: target.clone() });
                    }
                }
                Action::End(reason) => {
                    self.record(tick, &rule.id, format!("end:{reason}"), slot);
                    self.ended = Some(reason.clone());
                }
                Action::Impulse { object, dir, speed } => effects.push(Effect::Impulse { object: object.clone(), dir: *dir, speed: *speed }),
                Action::Reset(ResetTarget::Props(list)) => {
                    effects.extend(list.iter().filter_map(|&k| ctx.slot.get(k).copied().flatten()).map(|prop| Effect::Reset { prop }));
                }
                Action::Reset(ResetTarget::Zone(z)) => {
                    if let Some(v) = set.zone_volumes.get(*z) {
                        effects.extend(ctx.props.iter().enumerate().filter(|(_, p)| prop_inside(v, p)).map(|(prop, _)| Effect::Reset { prop }));
                    }
                }
                Action::Place { prop, at } => {
                    if let Some(prop) = ctx.slot.get(*prop).copied().flatten() {
                        effects.push(Effect::Place { prop, at: *at });
                    }
                }
            }
        }
    }

    /// [`step_props`](Self::step_props) with no loose props in view: prop triggers never fire and the prop built-ins read 0.
    pub fn step(&mut self, tick: u64, players: &[RulePlayer]) -> Vec<Effect> {
        self.step_props(tick, players, &[])
    }

    /// Advances the rules one tick (`tick` = ticks completed so far) given where the players and the loose props are;
    /// returns the effects the world must apply. Deterministic: rules run in declaration order, players in slot order,
    /// props in physics order. `props` may be empty when [`needs_props`](Self::needs_props) is false.
    pub fn step_props(&mut self, tick: u64, players: &[RulePlayer], props: &[RuleProp]) -> Vec<Effect> {
        let mut effects = Vec::new();
        if self.ended.is_some() || self.set.rules.is_empty() {
            self.injected.clear();
            return effects;
        }
        self.vars[0] = tick as f64 / TICK_RATE_HZ as f64;
        self.vars[1] = tick as f64;
        self.vars[2] = players.len() as f64;
        self.inside.retain(|(_, slot), _| players.iter().any(|p| p.slot == *slot));
        // The set and the binding are read-only while the rules run; lending them out lets `fire` borrow `self` mutably.
        let set = std::mem::take(&mut self.set);
        let slots = std::mem::take(&mut self.prop_slot);
        let ctx = Ctx { props, slot: &slots, zones: &set.zone_volumes };
        let mut queue: Vec<(String, Option<usize>)> = std::mem::take(&mut self.injected);
        for ri in 0..set.rules.len() {
            let rule = &set.rules[ri];
            match &rule.when {
                When::Start => {
                    if !self.started {
                        self.fire(&set, ri, tick, None, &mut queue, &mut effects, &ctx);
                    }
                }
                When::Enter(v) | When::Exit(v) => {
                    let entering = matches!(rule.when, When::Enter(_));
                    for p in players.iter().filter(|p| Self::who_ok(rule.who, p.character, p.team)) {
                        let now = Self::inside(v, p, self.wrap);
                        let was = self.inside.insert((ri, p.slot), now);
                        if let Some(was) = was {
                            if (entering && now && !was) || (!entering && !now && was) {
                                self.fire(&set, ri, tick, Some(p.slot), &mut queue, &mut effects, &ctx);
                            }
                        }
                    }
                }
                When::Every(n) => {
                    if tick > 0 && tick.is_multiple_of(*n) {
                        self.fire(&set, ri, tick, None, &mut queue, &mut effects, &ctx);
                    }
                }
                When::After(n) => {
                    if tick == *n {
                        self.fire(&set, ri, tick, None, &mut queue, &mut effects, &ctx);
                    }
                }
                When::Event(_) => {}
                When::PropEnter { volume, prop } | When::PropExit { volume, prop } => {
                    let entering = matches!(rule.when, When::PropEnter { .. });
                    let (lo, hi) = match prop {
                        Some(k) => slots.get(*k).copied().flatten().map_or((0, 0), |i| (i, i + 1)),
                        None => (0, props.len()),
                    };
                    for (k, p) in props.iter().enumerate().take(hi).skip(lo) {
                        let now = prop_inside(volume, p);
                        let was = self.prop_inside.insert((ri, k), now);
                        if let Some(was) = was {
                            if (entering && now && !was) || (!entering && !now && was) {
                                self.fire(&set, ri, tick, None, &mut queue, &mut effects, &ctx);
                            }
                        }
                    }
                }
                When::PropBelow { prop, y } => {
                    if let Some(p) = slots.get(*prop).copied().flatten().and_then(|i| props.get(i)) {
                        let now = p.origin.y < *y;
                        let was = self.below.insert(ri, now);
                        if was == Some(false) && now {
                            self.fire(&set, ri, tick, None, &mut queue, &mut effects, &ctx);
                        }
                    }
                }
            }
        }
        self.started = true;
        for _ in 0..MAX_CHAIN {
            if queue.is_empty() {
                break;
            }
            let batch = std::mem::take(&mut queue);
            for (name, slot) in batch {
                for ri in 0..set.rules.len() {
                    let matches_event = matches!(&set.rules[ri].when, When::Event(n) if *n == name);
                    let who_ok = slot.is_none_or(|s| players.iter().find(|p| p.slot == s).is_none_or(|p| Self::who_ok(set.rules[ri].who, p.character, p.team)));
                    if matches_event && who_ok {
                        self.fire(&set, ri, tick, slot, &mut queue, &mut effects, &ctx);
                    }
                }
            }
        }
        self.set = set;
        self.prop_slot = slots;
        effects
    }

    /// A 64-bit fold of all rule state (variables, fired flags, cooldowns, occupancy, hidden objects, outcome, event count).
    pub fn checksum(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut mix = |v: u64| {
            h ^= v;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        };
        for v in &self.vars {
            mix(v.to_bits());
        }
        // Only a scene with `player_vars` folds them in, so the checksum of every older scene (and its recorded traces) is unchanged.
        for v in &self.pvars {
            mix(v.to_bits());
        }
        for (f, n) in self.fired.iter().zip(&self.next_ok) {
            mix(*f as u64);
            mix(*n);
        }
        for ((r, s), v) in &self.inside {
            mix((*r as u64) << 32 | (*s as u64) << 1 | *v as u64);
        }
        for o in &self.hidden {
            o.bytes().for_each(|b| mix(b as u64));
            mix(0xff);
        }
        // Preserve pre-v5 checksums while no dynamic collision state exists, so old traces remain
        // replayable. The marker makes a non-empty collision set distinct from hidden-object data.
        if !self.collision_disabled.is_empty() {
            mix(0xfe);
            for o in &self.collision_disabled {
                o.bytes().for_each(|b| mix(b as u64));
                mix(0xff);
            }
        }
        // Prop occupancy exists only in scenes with prop rules, so older checksums are unchanged elsewhere.
        if !self.prop_inside.is_empty() {
            mix(0xfd);
            for ((r, p), v) in &self.prop_inside {
                mix((*r as u64) << 32 | (*p as u64) << 1 | *v as u64);
            }
        }
        if !self.below.is_empty() {
            mix(0xfc);
            for (r, v) in &self.below {
                mix((*r as u64) << 1 | *v as u64);
            }
        }
        if let Some(e) = &self.ended {
            e.bytes().for_each(|b| mix(b as u64));
        }
        mix(self.history.len() as u64 + self.dropped);
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::rules::{parse_rules, Refs};
    use serde_json::{json, Value};

    fn engine(v: Value) -> RulesEngine {
        let mut refs = Refs::default();
        refs.object_ids.insert("coin".into());
        refs.top_level_ids.insert("coin".into());
        refs.bounds.insert("coin".into(), (Vec3::new(2.0, 0.0, 0.0), Vec3::new(2.4, 0.4, 0.4)));
        refs.zones.insert("exit".into(), (Vec3::new(8.0, 0.0, -1.0), Vec3::new(10.0, 0.0, 1.0)));
        refs.spawn_ids.insert("start".into());
        RulesEngine::new(parse_rules(v.as_object().unwrap_or_else(|| panic!("object")), &refs).unwrap_or_else(|e| panic!("{e:?}")))
    }

    fn at(slot: usize, x: f32, z: f32, c: Character) -> RulePlayer {
        RulePlayer { slot, pos: Vec3::new(x, 0.0, z), radius: 0.3, height: 1.8, character: c, team: 0 }
    }

    fn at_team(slot: usize, x: f32, z: f32, c: Character, team: u8) -> RulePlayer {
        RulePlayer { team, ..at(slot, x, z, c) }
    }

    fn coin_game() -> RulesEngine {
        engine(json!({
            "vars": {"score": 0},
            "rules": [
                {"id": "take", "when": {"enter": {"object": "coin", "pad": 0.2}}, "once": true, "do": [{"add": ["score", 1]}, {"hide": "coin"}, {"emit": "coin"}]},
                {"id": "win", "when": {"enter": {"zone": "exit"}}, "if": "score >= 1", "do": [{"emit": "victory"}, {"end": "victory"}]}
            ]
        }))
    }

    #[test]
    fn walking_into_a_volume_fires_once_and_then_the_conditional_win_rule_ends_the_match() {
        let mut e = coin_game();
        let h = Character::Human;
        // Far from everything: nothing happens.
        for t in 1..=5 {
            assert!(e.step(t, &[at(0, -5.0, 0.0, h)]).is_empty());
        }
        assert_eq!(e.var("score"), Some(0.0));
        // Enter the coin: score 1, coin hidden, event logged. Standing in it does not fire again.
        e.step(6, &[at(0, 2.2, 0.2, h)]);
        e.step(7, &[at(0, 2.2, 0.2, h)]);
        assert_eq!(e.var("score"), Some(1.0));
        assert_eq!(e.hidden().collect::<Vec<_>>(), ["coin"]);
        assert_eq!(e.history().iter().filter(|x| x.name == "coin").count(), 1);
        // Leave and re-enter: `once` holds.
        e.step(8, &[at(0, 5.0, 0.0, h)]);
        e.step(9, &[at(0, 2.2, 0.2, h)]);
        assert_eq!(e.var("score"), Some(1.0));
        // The exit: conditional on score >= 1, ends the match.
        e.step(10, &[at(0, 6.0, 0.0, h)]);
        assert_eq!(e.ended(), None);
        e.step(11, &[at(0, 8.5, 0.0, h)]);
        assert_eq!(e.ended(), Some("victory"));
        let names: Vec<&str> = e.history().iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["coin", "victory", "end:victory"]);
        // Once ended, nothing runs any more.
        assert!(e.step(12, &[at(0, 2.2, 0.2, h)]).is_empty());
        assert_eq!(e.var("score"), Some(1.0));
    }

    #[test]
    fn the_exit_does_nothing_without_the_coin() {
        let mut e = coin_game();
        e.step(1, &[at(0, 5.0, 0.0, Character::Human)]);
        e.step(2, &[at(0, 8.5, 0.0, Character::Human)]);
        assert_eq!(e.ended(), None, "score 0: the `if` blocks the win");
    }

    #[test]
    fn who_filters_players_and_events_chain_within_a_tick() {
        let mut e = engine(json!({
            "vars": {"n": 0},
            "rules": [
                {"id": "a", "when": {"enter": {"zone": "exit"}}, "who": "rat", "do": [{"emit": "rat_in"}]},
                {"id": "b", "when": {"event": "rat_in"}, "do": [{"add": ["n", 10]}, {"emit": "second"}]},
                {"id": "c", "when": {"event": "second"}, "do": [{"add": ["n", 1]}, {"teleport": "start"}]}
            ]
        }));
        let far = [at(0, 0.0, 0.0, Character::Human), at(1, 0.0, 3.0, Character::Rat)];
        e.step(1, &far);
        let near_human = [at(0, 9.0, 0.0, Character::Human), at(1, 0.0, 3.0, Character::Rat)];
        e.step(2, &near_human);
        assert_eq!(e.var("n"), Some(0.0), "a human entering does not fire a rat rule");
        let near_rat = [at(0, 9.0, 0.0, Character::Human), at(1, 9.0, 0.0, Character::Rat)];
        let fx = e.step(3, &near_rat);
        assert_eq!(e.var("n"), Some(11.0), "rat_in -> second, both handled in the same tick");
        assert_eq!(fx, vec![Effect::Teleport { slot: 1, target: Target::Spawn("start".into()) }]);
    }

    #[test]
    fn who_filters_by_team_too() {
        let mut e = engine(json!({
            "vars": {"team1_in": 0, "team2_in": 0},
            "rules": [
                {"id": "a", "when": {"enter": {"zone": "exit"}}, "who": "team1", "do": [{"add": ["team1_in", 1]}]},
                {"id": "b", "when": {"enter": {"zone": "exit"}}, "who": "team2", "do": [{"add": ["team2_in", 1]}]}
            ]
        }));
        let far = [at_team(0, 0.0, 0.0, Character::Human, 1), at_team(1, 0.0, 3.0, Character::Human, 2)];
        e.step(1, &far);
        let both_in = [at_team(0, 9.0, 0.0, Character::Human, 1), at_team(1, 9.0, 0.0, Character::Human, 2)];
        e.step(2, &both_in);
        assert_eq!((e.var("team1_in"), e.var("team2_in")), (Some(1.0), Some(1.0)), "each team's own rule fires once for its own team");
    }

    #[test]
    fn timers_start_rules_cooldowns_and_exits_work() {
        let mut e = engine(json!({
            "vars": {"boot": 0, "tick_n": 0, "left": 0},
            "rules": [
                {"id": "s", "when": {"start": true}, "do": [{"set": ["boot", 1]}]},
                {"id": "t", "when": {"every": 0.5}, "do": [{"add": ["tick_n", 1]}]},
                {"id": "x", "when": {"exit": {"zone": "exit"}}, "do": [{"add": ["left", 1]}]},
                {"id": "cd", "when": {"every": 0.5}, "cooldown": 2.0, "do": [{"emit": "slow"}]}
            ]
        }));
        let h = Character::Human;
        for t in 1..=120 {
            let x = if (30..60).contains(&t) { 9.0 } else { 0.0 };
            e.step(t, &[at(0, x, 0.0, h)]);
        }
        assert_eq!(e.var("boot"), Some(1.0));
        assert_eq!(e.var("tick_n"), Some(4.0), "every 0.5 s over 2 s");
        assert_eq!(e.var("left"), Some(1.0), "one exit at tick 60");
        assert_eq!(e.history().iter().filter(|x| x.name == "slow").count(), 1, "the 2 s cooldown suppressed the rest");
        assert_eq!(e.var("time"), Some(2.0));
    }

    #[test]
    fn an_event_loop_is_bounded_and_state_is_checksummed() {
        let mut e = engine(json!({"vars": {"n": 0}, "rules": [
            {"id": "ping", "when": {"start": true}, "do": [{"emit": "p"}]},
            {"id": "loop", "when": {"event": "p"}, "do": [{"add": ["n", 1]}, {"emit": "p"}]}]}));
        e.step(1, &[]);
        assert_eq!(e.var("n"), Some(MAX_CHAIN as f64), "the chain stops at MAX_CHAIN instead of spinning");
        let (a, mut b) = (e.checksum(), coin_game());
        assert_ne!(a, b.checksum());
        let c1 = b.checksum();
        b.step(1, &[at(0, 2.2, 0.2, Character::Human)]);
        assert_eq!(b.step(2, &[at(0, 2.2, 0.2, Character::Human)]), vec![]);
        assert_ne!(c1, b.checksum(), "firing a rule changes the checksum");
        let (mut x, mut y) = (coin_game(), coin_game());
        for t in 1..50 {
            let p = [at(0, t as f32 * 0.2, 0.0, Character::Human)];
            x.step(t, &p);
            y.step(t, &p);
        }
        assert_eq!(x.checksum(), y.checksum(), "same inputs, same state");
    }

    #[test]
    fn in_zone_tests_one_prop_while_props_in_counts_them_all() {
        let mut refs = Refs::default();
        refs.prop_ids.extend(["bell".to_string(), "crate".to_string()]);
        refs.zones.insert("pan".into(), (Vec3::new(0.0, 0.0, 0.0), Vec3::new(4.0, 0.0, 4.0)));
        let set = parse_rules(
            json!({"vars": {"mine": 0, "all": 0, "weighed": 0}, "rules": [
                {"id": "look", "when": {"every": 0.5}, "do": [
                    {"set": ["mine", "in_zone(crate, pan)"]},
                    {"set": ["all", "props_in(pan)"]},
                    {"set": ["weighed", "in_zone(crate, pan) * mass(crate) + in_zone(bell, pan) * mass(bell)"]}]}
            ]})
            .as_object()
            .unwrap(),
            &refs,
        )
        .unwrap();
        let mut e = RulesEngine::new(set);
        assert!(e.needs_props(), "in_zone reads the props");
        e.bind_props(|id| match id {
            "crate" => Some(0),
            "bell" => Some(1),
            _ => None,
        });
        let prop =
            |x: f32, z: f32, mass: f32| RuleProp { origin: Vec3::new(x, 0.3, z), height: 0.5, tilt_deg: 0.0, held_by: None, mass, moved: 0.0, vel: Vec3::ZERO };
        // Only the bell (mass 5) is on the pan: in_zone(crate) is 0, props_in counts 1, and the weighed sum is the bell alone.
        e.step_props(30, &[], &[prop(9.0, 9.0, 20.0), prop(2.0, 2.0, 5.0)]);
        assert_eq!((e.var("mine"), e.var("all"), e.var("weighed")), (Some(0.0), Some(1.0), Some(5.0)));
        // The crate (mass 20) joins: both on the pan.
        e.step_props(60, &[], &[prop(1.0, 1.0, 20.0), prop(2.0, 2.0, 5.0)]);
        assert_eq!((e.var("mine"), e.var("all"), e.var("weighed")), (Some(1.0), Some(2.0), Some(25.0)));
        // A bounce out of the pan is undone by the next reading: no bookkeeping to drift.
        e.step_props(90, &[], &[prop(9.0, 9.0, 20.0), prop(2.0, 2.0, 5.0)]);
        assert_eq!((e.var("mine"), e.var("weighed")), (Some(0.0), Some(5.0)));
    }

    #[test]
    fn deactivate_hides_and_unblocks_in_one_step_and_activate_undoes_it() {
        let mut refs = Refs::default();
        refs.object_ids.insert("door".into());
        refs.top_level_ids.insert("door".into());
        let set = parse_rules(
            json!({"vars": {"open": 0}, "rules": [
                {"id": "open", "when": {"start": true}, "do": [{"deactivate": "door"}]},
                {"id": "shut", "when": {"after": 1.0}, "do": [{"activate": "door"}]}
            ]})
            .as_object()
            .unwrap(),
            &refs,
        )
        .unwrap();
        let mut e = RulesEngine::new(set);
        e.step(1, &[]);
        assert!(e.hidden().any(|h| h == "door") && e.collision_disabled().any(|h| h == "door"), "one action did both");
        e.step(60, &[]);
        assert!(!e.hidden().any(|h| h == "door") && !e.collision_disabled().any(|h| h == "door"), "activate put both back");
        let bad = parse_rules(json!({"rules": [{"id": "x", "when": {"start": true}, "do": [{"deactivate": "dor"}]}]}).as_object().unwrap(), &refs);
        assert!(bad.unwrap_err().iter().any(|m| m.contains("no top-level object `dor` — did you mean `door`?")), "a typo names the fix");
    }

    #[test]
    fn prop_triggers_and_built_ins_see_the_props_the_world_describes() {
        let mut refs = Refs::default();
        refs.prop_ids.extend(["bell".to_string(), "crate".to_string()]);
        refs.zones.insert("pit".into(), (Vec3::new(0.0, -3.0, 0.0), Vec3::new(4.0, -3.0, 4.0)));
        let set = parse_rules(
            json!({"vars": {"score": 0, "n": 0}, "rules": [
                {"id": "in_pit", "when": {"prop_enter": {"zone": "pit"}}, "do": [{"add": ["score", 1]}, {"set": ["n", "props_in(pit)"]}]},
                {"id": "out", "when": {"prop_exit": {"zone": "pit"}, "prop": "crate"}, "do": [{"emit": "crate_out"}]},
                {"id": "fell", "when": {"prop_below": ["bell", 0.5]}, "if": "tilt(bell) > 60 && mass(bell) > 1", "do": [{"emit": "bell_down"}, {"reset": "bell"}, {"place": ["crate", [1, 0, 1]]}]}
            ]})
            .as_object()
            .unwrap(),
            &refs,
        )
        .unwrap();
        let mut e = RulesEngine::new(set);
        assert!(e.needs_props());
        // The physics world numbers its props crate = 0, bell = 1 (not the sorted id order).
        e.bind_props(|id| match id {
            "crate" => Some(0),
            "bell" => Some(1),
            _ => None,
        });
        let prop = |x: f32, y: f32, z: f32, tilt: f32| RuleProp {
            origin: Vec3::new(x, y, z),
            height: 0.5,
            tilt_deg: tilt,
            held_by: None,
            mass: 20.0,
            moved: 0.0,
            vel: Vec3::ZERO,
        };
        let c0 = e.checksum();
        // Tick 1: both props up on a deck above the pit: nothing.
        assert!(e.step_props(1, &[], &[prop(2.0, 1.0, 2.0, 0.0), prop(2.0, 1.0, 3.0, 0.0)]).is_empty());
        // Tick 2: the crate drops into the pit: score 1, n = props_in(pit) = 1.
        e.step_props(2, &[], &[prop(2.0, -3.0, 2.0, 0.0), prop(2.0, 1.0, 3.0, 0.0)]);
        assert_eq!((e.var("score"), e.var("n")), (Some(1.0), Some(1.0)));
        // Tick 3: the crate leaves the pit (crate_out); the bell lies below 0.5 m on its side: bell_down, reset, place.
        let fx = e.step_props(3, &[], &[prop(2.0, 1.0, 2.0, 0.0), prop(2.0, 0.2, 3.0, 80.0)]);
        assert_eq!(fx, vec![Effect::Reset { prop: 1 }, Effect::Place { prop: 0, at: Vec3::new(1.0, 0.0, 1.0) }]);
        let names: Vec<&str> = e.history().iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["crate_out", "bell_down"]);
        // Staying below is not a new crossing (edge-triggered).
        assert!(e.step_props(4, &[], &[prop(2.0, 1.0, 2.0, 0.0), prop(2.0, 0.2, 3.0, 80.0)]).is_empty());
        assert_eq!(e.history().len(), 2);
        assert_ne!(e.checksum(), c0, "prop occupancy is part of the rule state");
        // The same scene stepped with no props in view never fires a prop rule.
        let mut quiet = RulesEngine::new(e.set().clone());
        quiet.step(1, &[]);
        quiet.step(2, &[]);
        assert!(quiet.history().is_empty() && quiet.var("score") == Some(0.0));
    }

    #[test]
    fn per_player_variables_belong_to_the_player_that_triggered_the_rule() {
        let mut e = engine(json!({
            "vars": {"total": 0},
            "player_vars": {"laps": 0, "lives": 3},
            "rules": [
                {"id": "lap", "when": {"enter": {"zone": "exit"}}, "do": [{"add": ["me.laps", 1]}, {"add": ["total", 1]}]},
                {"id": "win", "when": {"event": "lap_done"}, "if": "me.laps >= 2", "do": [{"set": ["total", 100]}]},
                {"id": "mark", "when": {"enter": {"zone": "exit"}}, "if": "me.laps >= 2", "do": [{"emit": "lap_done"}, {"set": ["me.lives", "me.lives - 1"]}]}
            ]
        }));
        let away = |slot| at(slot, 0.0, 0.0, Character::Human);
        let on = |slot| at(slot, 9.0, 0.0, Character::Human);
        // Both start outside (a player's first sighting only records where they are); player 1 then laps twice, player 0 never moves.
        e.step(0, &[away(0), away(1)]);
        for _ in 0..2 {
            e.step(1, &[away(0), on(1)]);
            e.step(2, &[away(0), away(1)]);
        }
        assert_eq!((e.player_var(0, "laps"), e.player_var(1, "laps")), (Some(0.0), Some(2.0)));
        assert_eq!((e.player_var(0, "lives"), e.player_var(1, "lives")), (Some(3.0), Some(2.0)), "the second lap marked player 1 only");
        assert_eq!(e.var("total"), Some(100.0), "the event rule saw player 1's laps (the emitter's slot carries over)");
        assert_eq!(e.player_vars(1), vec![("laps", 2.0), ("lives", 2.0)]);
        e.reset_player(1);
        assert_eq!(e.player_vars(1), vec![("laps", 0.0), ("lives", 3.0)], "a player joining the slot starts from the declared values");
        assert_eq!(e.player_var(1, "nope"), None);
    }

    #[test]
    fn per_player_variables_are_part_of_the_checksum_and_a_scene_without_them_is_unchanged() {
        let scene = json!({"player_vars": {"n": 0}, "rules": [{"id": "r", "when": {"enter": {"zone": "exit"}}, "do": [{"add": ["me.n", 1]}]}]});
        let (mut a, mut b) = (engine(scene.clone()), engine(scene));
        assert_eq!(a.checksum(), b.checksum());
        a.step(1, &[at(0, 9.0, 0.0, Character::Human)]);
        b.step(1, &[at(1, 9.0, 0.0, Character::Human)]);
        assert_ne!(a.checksum(), b.checksum(), "who got the lap matters");
        assert_eq!(engine(json!({"vars": {}})).checksum(), RulesEngine::new(RuleSet::default()).checksum());
    }
}
