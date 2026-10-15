//! Story steps in a scenario script: a scripted player talks, moves on and chooses through the game's story exactly as a client does
//! (an `Intent` to the match's game, naming the revision it was shown), so a scenario proves the real story under the real match
//! (ADR 2026-10-15-narrative-is-compiled-content-run-by-one-story-system).
//!
//! `{"player": "p1", "story": "talk", "to": "bram"}` opens a conversation; `"advance"` moves past one line; `"until_choice"` moves on until
//! a line offers choices or the conversation ends; `{"story": "choose", "choice": "lend"}` picks one (`"*"`: every offered choice, one per
//! run, under `story walk`; with no conversation open, a `*` step is passed: that path's choices ended it sooner); `"leave"`, `"skip"` (a
//! bark). `"revision": N` sends that revision instead of the current one (a stale answer).
//! A step waits while a bark suspends the conversation; a refusal fails the scenario with the story's reason.

use super::super::game::Intent;
use super::super::match_sim::MatchSim;
use serde_json::{Map, Value};

/// What a story step does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verb {
    /// Talk to a speaker.
    Talk(String),
    /// Move past one line.
    Advance,
    /// Move on until a line offers choices, or the conversation ends.
    UntilChoice,
    /// Pick a choice (`*`: the walk's policy picks).
    Choose(String),
    /// Leave the conversation.
    Leave,
    /// Skip the bark being presented.
    Skip,
}

/// A story step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// What it does.
    pub verb: Verb,
    /// The revision to send instead of the current one (to prove a stale answer is refused).
    pub revision: Option<i64>,
}

/// One point where a walk can branch: the choices offered there and the ones held back, with why.
#[derive(Debug, Clone, PartialEq)]
pub struct Offer {
    /// The tick.
    pub tick: u64,
    /// The line shown.
    pub line: String,
    /// The choices offered, in order.
    pub offered: Vec<String>,
    /// The choices not offered now, with why.
    pub held_back: Vec<(String, String)>,
}

/// Who picks at a `"choice": "*"` step.
pub trait Policy {
    /// The choice to take at `offer`.
    fn choose(&mut self, offer: &Offer) -> Result<String, String>;
}

/// A plain scenario: `*` is for `story walk`.
pub struct Scripted;

impl Policy for Scripted {
    fn choose(&mut self, offer: &Offer) -> Result<String, String> {
        Err(format!("`\"choice\": \"*\"` at `{}` branches over {:?}: run it with `story walk`", offer.line, offer.offered))
    }
}

/// Parses a step's story keys (`story`, `to`, `choice`, `revision`).
pub fn parse(so: &Map<String, Value>, at: &str) -> Result<Step, String> {
    let text = |k: &str| so.get(k).and_then(Value::as_str).map(str::to_string);
    let verb = match so.get("story").and_then(Value::as_str) {
        Some("talk") => Verb::Talk(text("to").ok_or_else(|| format!("{at}.to: `talk` needs `to` (a speaker id)"))?),
        Some("advance") => Verb::Advance,
        Some("until_choice") => Verb::UntilChoice,
        Some("choose") => Verb::Choose(text("choice").ok_or_else(|| format!("{at}.choice: `choose` needs `choice` (a choice id, or \"*\")"))?),
        Some("leave") => Verb::Leave,
        Some("skip") => Verb::Skip,
        other => return Err(format!("{at}.story: {other:?} is not a story step (talk, advance, until_choice, choose, leave, skip)")),
    };
    let revision = match so.get("revision") {
        None => None,
        Some(r) => Some(r.as_i64().ok_or_else(|| format!("{at}.revision: a whole number"))?),
    };
    Ok(Step { verb, revision })
}

fn view(sim: &MatchSim) -> Result<Value, String> {
    sim.game_inspect("story").ok_or_else(|| "this scene's game runs no story".to_string())
}

/// Runs `step` for the player in `slot` now (between ticks). `Ok(true)`: done; `Ok(false)`: waiting (a bark suspends the conversation).
pub fn run(sim: &mut MatchSim, slot: usize, step: &Step, policy: &mut dyn Policy) -> Result<bool, String> {
    for _ in 0..64 {
        let v = view(sim)?;
        let revision = step.revision.unwrap_or_else(|| v.get("revision").and_then(Value::as_i64).unwrap_or(0));
        let talk = v.get("view").and_then(|w| w.get("talk")).filter(|t| !t.is_null());
        let suspended = talk.and_then(|t| t.get("suspended")).and_then(Value::as_bool).unwrap_or(false);
        let choices: Vec<String> = talk
            .and_then(|t| t.get("choices"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|c| c.get(0)?.as_str().map(str::to_string))
            .collect();
        let intent = |name: &str, subject: Option<String>| Intent { name: format!("story:{name}"), subject, value: revision };
        let send = |sim: &mut MatchSim, i: Intent| sim.intend(slot, &i);
        if suspended && !matches!(step.verb, Verb::Talk(_) | Verb::Skip) {
            return Ok(false);
        }
        match &step.verb {
            Verb::Talk(who) => return send(sim, intent("talk", Some(who.clone()))).map(|_| true),
            Verb::Advance => return send(sim, intent("advance", None)).map(|_| true),
            Verb::Leave => return send(sim, intent("leave", None)).map(|_| true),
            Verb::Skip => return send(sim, intent("skip", None)).map(|_| true),
            Verb::UntilChoice => {
                if talk.is_none() || !choices.is_empty() {
                    return Ok(true);
                }
                send(sim, intent("advance", None))?;
            }
            Verb::Choose(c) => {
                // A branch point no path of this walk reached (an earlier choice ended the conversation) is passed.
                let Some(t) = talk else {
                    return if c == "*" { Ok(true) } else { Err(format!("choose `{c}`: no conversation is open")) };
                };
                let pick = if c == "*" {
                    let line = t.get("line").and_then(|l| l.get("id")).and_then(Value::as_str).unwrap_or("").to_string();
                    let held_back = t
                        .get("unavailable")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|u| Some((u.get(0)?.as_str()?.to_string(), u.get(1)?.as_str()?.to_string())))
                        .collect();
                    policy.choose(&Offer { tick: sim.tick(), line, offered: choices.clone(), held_back })?
                } else {
                    c.clone()
                };
                return send(sim, intent("choose", Some(pick))).map(|_| true);
            }
        }
    }
    Err("`until_choice`: 64 lines without a choice or an end in one step".into())
}
