//! The seam a game's own server-side rules plug into the match simulation (ADR 2026-10-14-a-game-plugs-into-the-server-s-match-simulation-the): a
//! [`MatchGame`] is told what the match does (a player spawns, ticks, acts; someone dies or scores; the world ticks) where the match does it, and
//! answers what the match asks (whether a teammate's shot hurts, whether input is held, which side a team spawns on, how fast a player may run).
//! Every method has a default that changes nothing, so a match with no game plays exactly as one without.
//!
//! The match holds at most one game. A hook is handed the match itself (`&mut MatchSim`); the match takes its game out while a hook it called
//! runs, so a hook that calls back into the match for something that tells the game again (damage that kills, a respawn) passes itself along:
//! [`MatchSim::damage_in`](super::match_sim::MatchSim::damage_in), [`MatchSim::respawn_in`](super::match_sim::MatchSim::respawn_in). The loadout
//! shooter ([`super::shooter::ShooterGame`]) is the first game.

use super::match_sim::{MatchSim, ServerPlayer};
use super::player::PlayerInput;
use crate::weapons::Weapon;
use glam::Vec3;
use std::any::Any;

/// A game a crate defines for the match simulation: built for a scene that asks for it (its block), as a scene block is defined
/// (`schema::define_blocks`) and a world layer (`world_layer::define_layers`).
pub struct GameDef {
    /// Its name (`shooter`): a name already defined keeps its first definition.
    pub name: &'static str,
    /// The game for a scene, when the scene plays it.
    pub build: fn(&crate::schema::Scene) -> Option<Box<dyn MatchGame>>,
}

static GAMES: std::sync::RwLock<Vec<&'static GameDef>> = std::sync::RwLock::new(Vec::new());

/// Defines games for the match simulation; call it with the scene blocks (the engine's `scene_blocks::register` does for its own).
pub fn define_games(defs: &'static [GameDef]) {
    let mut all = GAMES.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    for d in defs {
        if !all.iter().any(|g| g.name == d.name) {
            all.push(d);
        }
    }
}

/// The game a scene plays: the first defined one that builds for it, `None` for a match with no game of its own.
pub fn game_for(scene: &crate::schema::Scene) -> Option<Box<dyn MatchGame>> {
    let defs = GAMES.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    defs.into_iter().find_map(|d| (d.build)(scene))
}

/// A kill, as the match scored it: who, whom, with what, and whether it counts for the killer and their team.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Kill {
    /// The tick it happened.
    pub tick: u64,
    /// The killer's slot (the victim's own, for a death by their own blast or nobody's bomb).
    pub by: usize,
    /// The victim's slot.
    pub victim: usize,
    /// The killer's team (`0`: none).
    pub by_team: u8,
    /// Whether it scores (not a suicide, not a teammate).
    pub scores: bool,
    /// What killed.
    pub weapon: Weapon,
    /// A headshot.
    pub headshot: bool,
}

/// A game's own rules inside a running match. Every method has a default that changes nothing.
pub trait MatchGame: Any + Send {
    /// Its name (`shooter`).
    fn name(&self) -> &'static str;

    /// A player has joined or respawned in `slot` (a start kit is handed out here).
    fn spawn(&mut self, _sim: &mut MatchSim, _slot: usize) {}

    /// A player has left `slot`.
    fn left(&mut self, _slot: usize) {}

    /// Whether the game runs this player's weapons (then [`MatchGame::player_tick`] and [`MatchGame::actions`] replace the match's own).
    fn runs_weapons(&self, _slot: usize) -> bool {
        false
    }

    /// A living player's weapons for one tick (only when [`MatchGame::runs_weapons`]).
    fn player_tick(&mut self, _sim: &mut MatchSim, _slot: usize) {}

    /// A living player's buttons this tick (only when [`MatchGame::runs_weapons`]).
    fn actions(&mut self, _sim: &mut MatchSim, _slot: usize, _input: &PlayerInput) {}

    /// Whether a teammate's shot or blast hurts.
    fn friendly_fire(&self) -> bool {
        true
    }

    /// `victim` has just died (before the killer is credited).
    fn on_death(&mut self, _sim: &mut MatchSim, _victim: usize) {}

    /// A death has been scored (after the killer is credited, before the rules hear of it).
    fn on_kill(&mut self, _sim: &mut MatchSim, _kill: Kill) {}

    /// The game's world for one tick, after every player has moved.
    fn tick(&mut self, _sim: &mut MatchSim) {}

    /// Whether players are held in place this tick (they may look around).
    fn input_locked(&self) -> bool {
        false
    }

    /// Whether the dead stay dead for now.
    fn respawn_blocked(&self) -> bool {
        false
    }

    /// The spawn group (`team1`, `team2`) a player of `team` starts at.
    fn spawn_side(&self, team: u8) -> u8 {
        team
    }

    /// How much what player `slot` carries slows them (`None`: not at all).
    fn move_mult(&self, _slot: usize) -> Option<f32> {
        None
    }

    /// The game's state for player `slot`, for the match checksum (`None`: nothing).
    fn player_hash(&self, _slot: usize) -> Option<u64> {
        None
    }

    /// The game's part of a new bot's mind (its weapons, its objective, what it picks up), `None`: the bot plays the match as it is.
    fn bot_mind(&self) -> Option<Box<dyn BotMind>> {
        None
    }

    // ---- the server's flow and what a client is sent ----------------------------------------------------------------------------------

    /// Whether players are on teams in the game's mode.
    fn teams(&self) -> bool {
        false
    }

    /// What each team is scoring toward (index = team - 1), when the game keeps team scores: the match is then won by a team.
    fn team_score(&self) -> Option<[u32; 2]> {
        None
    }

    /// Kills scored by each team (index = team - 1), when the game counts them.
    fn team_kills(&self) -> Option<[u32; 2]> {
        None
    }

    /// The score that wins, when the game's mode sets it (otherwise the scene's `score_to_win`).
    fn score_limit(&self) -> Option<u32> {
        None
    }

    /// Most people on one team, when the game sets it (the match then holds two teams of it).
    fn team_size(&self) -> Option<usize> {
        None
    }

    /// The game's mode, as the status carries it.
    fn mode_wire(&self) -> u8 {
        0
    }

    /// Whether players wear the team uniforms ([`body`](Self::body) answers with `Character::soldier`): a client's avatar pool then prepares the
    /// soldiers instead of the other bodies (`net::session::avatar_plan`).
    fn uniforms(&self) -> bool {
        false
    }

    /// The body a player of `team` (`0`: none) in `slot` wears, given the `look` they or the roster chose (`None`: the character they chose).
    fn body(&self, _slot: usize, _team: u8, _look: u8) -> Option<crate::player::Character> {
        None
    }

    /// What happened in the game lately, for the server's log: `(id, line)`, ids rising (a smaller id than one logged before: a new world).
    fn log(&self) -> Vec<(u32, String)> {
        Vec::new()
    }

    /// The game's own part of a snapshot for `viewer` (`protocol::GameMsg`), `None`: the snapshot carries none.
    fn snapshot_part(&self, _sim: &MatchSim, _viewer: usize) -> Option<crate::net::protocol::GameMsg> {
        None
    }

    /// Player `slot`'s game byte in a snapshot that carries a game part (`protocol::PlayerSnap::extra`).
    fn player_byte(&self, _sim: &MatchSim, _slot: usize) -> u8 {
        0
    }

    /// Whether player `slot` is swinging a melee weapon of the game's (a snapshot's swing flag).
    fn swinging(&self, _slot: usize) -> bool {
        false
    }
}

/// Where a bot wants to be for the game's objective, and whether to hold Interact there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotGoal {
    /// Where to go (the ground there).
    pub at: Vec3,
    /// Hold Interact now (planting or defusing).
    pub hold: bool,
    /// Carrying the flag or bomb, or racing to take back one's own: worth running past a fight.
    pub urgent: bool,
}

/// What a bot's game mind is told when asked: who and when it is, whom it sees, and the bot's own random numbers (the same stream its brain
/// draws from, so a bot plays the same match the same way).
pub struct BotCtx<'a> {
    /// The bot's slot.
    pub slot: usize,
    /// The bot.
    pub me: &'a ServerPlayer,
    /// The tick.
    pub now: u64,
    /// The enemy it sees now, if any.
    pub target: Option<usize>,
    /// The bot's own input sequence (for alternating decisions).
    pub seq: u32,
    /// The bot's random numbers.
    pub rng: &'a mut super::ai::Rng,
}

/// A game's part of one bot's mind ([`MatchGame::bot_mind`]), kept in its brain: what it remembers of the game's own play and how it plays it.
/// Every method has a default that does nothing.
pub trait BotMind: Any + Send + std::fmt::Debug {
    /// The bot's weapons this tick, after it aimed and fired: what to hold, whether to aim down the sights, a grenade, a reload.
    fn weapons(&mut self, _sim: &MatchSim, _bot: &mut BotCtx, _input: &mut PlayerInput) {}

    /// Where the bot wants to be for the game's objective now.
    fn objective(&self, _sim: &MatchSim, _slot: usize) -> Option<BotGoal> {
        None
    }

    /// Something on the floor worth walking to, when there is nothing better to do.
    fn pickup(&mut self, _sim: &MatchSim, _bot: &mut BotCtx) -> Option<Vec3> {
        None
    }

    /// The bot has reached the thing it was walking to.
    fn reached_pickup(&mut self) {}
}

impl MatchSim {
    /// Runs `f` on the match's game, if it has one, with the game taken out of the match while it runs.
    pub(crate) fn with_game<R>(&mut self, f: impl FnOnce(&mut dyn MatchGame, &mut MatchSim) -> R) -> Option<R> {
        let mut game = self.game.take()?;
        let r = f(game.as_mut(), self);
        self.game = Some(game);
        Some(r)
    }

    /// Runs `f` with the match's game passed along (taken out of the match while `f` runs): for the match's own calls into what may tell the
    /// game something (`damage_in`, `respawn_in`).
    pub(crate) fn with_game_passed<R>(&mut self, f: impl FnOnce(&mut MatchSim, Option<&mut dyn MatchGame>) -> R) -> R {
        let mut game = self.game.take();
        let r = f(self, game.as_deref_mut());
        if self.game.is_none() {
            self.game = game;
        }
        r
    }

    /// The game's own part of a snapshot for `viewer`, if the match has a game that sends one.
    pub fn game_part(&self, viewer: usize) -> Option<crate::net::protocol::GameMsg> {
        self.game.as_ref().and_then(|g| g.snapshot_part(self, viewer))
    }

    /// Player `slot`'s game byte in a snapshot (`0` without a game).
    pub fn game_player_byte(&self, slot: usize) -> u8 {
        self.game.as_ref().map_or(0, |g| g.player_byte(self, slot))
    }

    /// Whether player `slot` swings a game's melee weapon.
    pub fn game_swinging(&self, slot: usize) -> bool {
        self.game.as_ref().is_some_and(|g| g.swinging(slot))
    }

    /// The body the game gives a player of `team` in `slot` with `look`, if it decides bodies.
    pub fn game_body(&self, slot: usize, team: u8, look: u8) -> Option<crate::player::Character> {
        self.game.as_ref().and_then(|g| g.body(slot, team, look))
    }

    /// The game's recent log lines (see [`MatchGame::log`]).
    pub fn game_log(&self) -> Vec<(u32, String)> {
        self.game.as_ref().map_or_else(Vec::new, |g| g.log())
    }

    /// The game's mode as the status carries it (`0` without a game).
    pub fn game_mode_wire(&self) -> u8 {
        self.game.as_ref().map_or(0, |g| g.mode_wire())
    }

    /// Whether teams score in this match (a game with team scores and teams on): then a team wins it.
    pub fn team_scored(&self) -> bool {
        self.game.as_ref().is_some_and(|g| g.team_score().is_some()) && self.teams_enabled()
    }

    /// The match's game of type `T`, if it has one.
    pub fn game<T: MatchGame>(&self) -> Option<&T> {
        self.game.as_deref().and_then(|g| (g as &dyn Any).downcast_ref::<T>())
    }

    /// The match's game of type `T`, to change.
    pub fn game_mut<T: MatchGame>(&mut self) -> Option<&mut T> {
        self.game.as_deref_mut().and_then(|g| (g as &mut dyn Any).downcast_mut::<T>())
    }
}
