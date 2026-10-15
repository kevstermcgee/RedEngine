//! The watch (`tw:watch`): Tidewatch's gameplay as one game system. Night watches on a tidal island: while a watch runs the signal brazier
//! burns a brick of peat every few seconds, the player carries bricks one at a time from the shed to feed it, and a boat comes in at the
//! end of the watch if the light was not dark too long. Peat is a ledger resource in stores (`tw:shed`, `tw:hands`, `tw:brazier`,
//! `tw:dock_lamp`, `tw:reserve`), changed only by transactions, so a story choice that lends two bricks to the dock lamp or a director's
//! "Odile brings one from her reserve" moves real peat and can never make any.
//!
//! What the story reads are its typed queries (`tw:shed_fuel`, `tw:light_out`, ...); what it may do are its commands (`tw:lend`; the
//! director only `tw:odile_helps`, the one command it lets be issued unattended: help from a finite reserve, never taking from the player).

use red_engine2::sim::defs::{code, Diagnostic, Id};
use red_engine2::sim::ledger::{Fnv, Ledger, Reason, Tx};
use red_engine2::sim::systems::{Command, Cx, Inspection, Look, QueryDecl, System, Ty};
use serde_json::{json, Value};

fn id(s: &str) -> Id {
    Id::parse(s).unwrap_or_else(|e| panic!("{e}"))
}

/// The resource.
pub fn peat() -> Id {
    id("tw:peat")
}

/// The stores, by name: `tw:<name>`.
pub const STORES: &[(&str, u32)] = &[("shed", 8), ("hands", 1), ("brazier", 4), ("dock_lamp", 2), ("reserve", 2)];

/// How a watch is played (the scene's `tidewatch` block sets these).
#[derive(Debug, Clone, PartialEq)]
pub struct Rules {
    /// Watches in a night.
    pub watches: u32,
    /// Ticks a watch lasts.
    pub watch_ticks: u32,
    /// Ticks between two bricks burnt.
    pub burn_ticks: u32,
    /// Bricks the shed holds when a watch begins (it is restocked to this).
    pub restock: u32,
    /// Bricks in the brazier when a watch begins, at least.
    pub kindling: u32,
    /// Ticks of darkness in a watch after which the boat is late.
    pub late_ticks: u32,
}

impl Default for Rules {
    fn default() -> Rules {
        Rules { watches: 2, watch_ticks: 60 * 60, burn_ticks: 10 * 60, restock: 5, kindling: 2, late_ticks: 6 * 60 }
    }
}

/// The phase of the night.
pub mod phase {
    /// Ashore: before a watch, or between two (the bell starts the next).
    pub const ASHORE: i64 = 0;
    /// A watch is running.
    pub const WATCH: i64 = 1;
    /// Every watch is over.
    pub const DONE: i64 = 2;
}

/// The watch, running.
#[derive(Debug, Clone, PartialEq)]
pub struct Watch {
    rules: Rules,
    /// [`phase`].
    pub phase: i64,
    /// Watches begun.
    pub watch: u32,
    /// Ticks left in this watch.
    pub left: u32,
    /// Ticks to the next brick burnt.
    pub burn: u32,
    /// Ticks the light was out this watch.
    pub dark: u32,
    /// Boats that came in, and boats that were late.
    pub boats: (u32, u32),
    /// The last watch that ended brought its boat in.
    pub last_safe: bool,
}

impl Watch {
    /// The first watch not yet begun.
    pub fn new(rules: Rules) -> Watch {
        Watch { rules, phase: phase::ASHORE, watch: 0, left: 0, burn: 0, dark: 0, boats: (0, 0), last_safe: false }
    }

    /// The ledger the watch runs on: the stores, the shed's first stock and the reserve, every reason peat may appear or vanish.
    pub fn ledger(rules: &Rules) -> Result<Ledger, Diagnostic> {
        let mut l = Ledger::new();
        l.resource(peat());
        for r in [Reason::Source(id("tw:start")), Reason::Source(id("tw:delivery")), Reason::Sink(id("tw:burn")), Reason::Sink(id("tw:dock_burn"))] {
            l.declare(r);
        }
        for (name, cap) in STORES {
            l.add_store(id(&format!("tw:{name}")), id("tw:watch"), *cap)?;
        }
        let start = Reason::Source(id("tw:start"));
        l.apply(&Tx::new().producing(&id("tw:shed"), &peat(), rules.restock, &start).producing(&id("tw:reserve"), &peat(), 2, &start))?;
        Ok(l)
    }

    fn count(l: &Ledger, store: &str) -> u32 {
        l.count(&id(&format!("tw:{store}")), &peat())
    }

    fn ev(cx: &mut Cx, name: &str, value: i64) {
        let _ = cx.emit(&id(name), None, value);
    }

    fn start(&mut self, cx: &mut Cx) {
        let delivery = Reason::Source(id("tw:delivery"));
        let shed = Watch::count(cx.ledger, "shed");
        let brazier = Watch::count(cx.ledger, "brazier");
        let mut tx = Tx::new();
        if shed < self.rules.restock {
            tx = tx.producing(&id("tw:shed"), &peat(), self.rules.restock - shed, &delivery);
        }
        if brazier < self.rules.kindling {
            tx = tx.producing(&id("tw:brazier"), &peat(), self.rules.kindling - brazier, &delivery);
        }
        let _ = cx.ledger.apply(&tx);
        self.watch += 1;
        self.phase = phase::WATCH;
        (self.left, self.burn, self.dark) = (self.rules.watch_ticks, self.rules.burn_ticks, 0);
        Watch::ev(cx, "tw:watch_started", self.watch as i64);
    }
}

impl System for Watch {
    fn id(&self) -> Id {
        id("tw:watch")
    }

    fn emits(&self) -> Vec<Id> {
        ["tw:watch_started", "tw:fed", "tw:low", "tw:out", "tw:relit", "tw:watch_ended", "tw:lent", "tw:lamp_stocked", "tw:odile_helped"]
            .iter()
            .map(|e| id(e))
            .collect()
    }

    fn commands(&self) -> Vec<Id> {
        ["tw:take", "tw:feed", "tw:ring", "tw:lend", "tw:stock_lamp", "tw:odile_helps"].iter().map(|c| id(c)).collect()
    }

    fn unattended(&self) -> Vec<Id> {
        vec![id("tw:odile_helps")]
    }

    fn queries(&self) -> Vec<QueryDecl> {
        [
            ("tw:phase", Ty::Int, "0 ashore (before or between watches), 1 a watch is running, 2 every watch is over"),
            ("tw:watch", Ty::Int, "watches begun (1 during the first)"),
            ("tw:secs_left", Ty::Int, "seconds left in the running watch"),
            ("tw:shed_fuel", Ty::Int, "bricks of peat in the shed"),
            ("tw:brazier_fuel", Ty::Int, "bricks in the signal brazier (0: the light is out)"),
            ("tw:carrying", Ty::Bool, "the player carries a brick"),
            ("tw:light_out", Ty::Bool, "a watch is running and the brazier is empty"),
            ("tw:dark_secs", Ty::Int, "seconds the light was out in this watch"),
            ("tw:dock_lit", Ty::Bool, "the dock lamp has peat"),
            ("tw:reserve", Ty::Int, "bricks left in Odile's own reserve"),
            ("tw:boats_in", Ty::Int, "boats that came in safely"),
            ("tw:boats_late", Ty::Int, "boats that were late because the light was out too long"),
            ("tw:last_safe", Ty::Bool, "the last watch that ended brought its boat in (false before any has ended)"),
        ]
        .iter()
        .map(|(q, t, d)| QueryDecl::new(id(q), *t, d))
        .collect()
    }

    fn query(&self, q: &Id, look: &Look) -> Option<f64> {
        let l = look.ledger;
        let b = |x: bool| x as u8 as f64;
        Some(match q.name() {
            "phase" => self.phase as f64,
            "watch" => self.watch as f64,
            "secs_left" => (self.left / 60) as f64,
            "shed_fuel" => Watch::count(l, "shed") as f64,
            "brazier_fuel" => Watch::count(l, "brazier") as f64,
            "carrying" => b(Watch::count(l, "hands") > 0),
            "light_out" => b(self.phase == phase::WATCH && Watch::count(l, "brazier") == 0),
            "dark_secs" => (self.dark / 60) as f64,
            "dock_lit" => b(Watch::count(l, "dock_lamp") > 0),
            "reserve" => Watch::count(l, "reserve") as f64,
            "boats_in" => self.boats.0 as f64,
            "boats_late" => self.boats.1 as f64,
            "last_safe" => b(self.last_safe),
            _ => return None,
        })
    }

    fn tick(&mut self, cx: &mut Cx) {
        if self.phase != phase::WATCH {
            return;
        }
        self.left = self.left.saturating_sub(1);
        self.burn = self.burn.saturating_sub(1);
        let brazier = Watch::count(cx.ledger, "brazier");
        if brazier == 0 {
            self.dark += 1;
        }
        if self.burn == 0 {
            self.burn = self.rules.burn_ticks;
            if brazier > 0 && cx.ledger.apply(&Tx::new().consuming(&id("tw:brazier"), &peat(), 1, &Reason::Sink(id("tw:burn")))).is_ok() {
                match brazier - 1 {
                    0 => Watch::ev(cx, "tw:out", 0),
                    1 => Watch::ev(cx, "tw:low", 1),
                    _ => {}
                }
            }
            // The dock lamp burns at half the rate.
            if self.left % (self.rules.burn_ticks * 2) < self.rules.burn_ticks && Watch::count(cx.ledger, "dock_lamp") > 0 {
                let _ = cx.ledger.apply(&Tx::new().consuming(&id("tw:dock_lamp"), &peat(), 1, &Reason::Sink(id("tw:dock_burn"))));
            }
        }
        if self.left == 0 {
            let safe = self.dark < self.rules.late_ticks;
            self.last_safe = safe;
            if safe {
                self.boats.0 += 1;
            } else {
                self.boats.1 += 1;
            }
            self.phase = if self.watch >= self.rules.watches { phase::DONE } else { phase::ASHORE };
            Watch::ev(cx, "tw:watch_ended", safe as i64);
        }
    }

    fn command(&mut self, c: &Command, cx: &mut Cx) -> Result<(), Diagnostic> {
        let refuse = |m: String| Err(Diagnostic::new(code::MISSING, c.name.as_str(), m));
        let held = Watch::count(cx.ledger, "hands");
        match c.name.name() {
            "take" => {
                if held > 0 {
                    return refuse("your hands are full".into());
                }
                cx.ledger.apply(&Tx::new().moving(&id("tw:shed"), &id("tw:hands"), &peat(), 1))
            }
            "feed" => {
                if held == 0 {
                    return refuse("you carry no peat".into());
                }
                let was = Watch::count(cx.ledger, "brazier");
                cx.ledger.apply(&Tx::new().moving(&id("tw:hands"), &id("tw:brazier"), &peat(), 1))?;
                Watch::ev(cx, "tw:fed", was as i64 + 1);
                if was == 0 && self.phase == phase::WATCH {
                    Watch::ev(cx, "tw:relit", 1);
                }
                Ok(())
            }
            "ring" => {
                if self.phase != phase::ASHORE {
                    return refuse(if self.phase == phase::WATCH { "a watch is already running".into() } else { "the night is over".into() });
                }
                self.start(cx);
                Ok(())
            }
            "lend" => {
                let n = u32::try_from(c.value)
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or_else(|| Diagnostic::new(code::MISSING, "tw:lend", "lend how many? (a positive value)"))?;
                cx.ledger.apply(&Tx::new().moving(&id("tw:shed"), &id("tw:dock_lamp"), &peat(), n))?;
                Watch::ev(cx, "tw:lent", n as i64);
                Ok(())
            }
            "stock_lamp" => {
                if held == 0 {
                    return refuse("you carry no peat".into());
                }
                cx.ledger.apply(&Tx::new().moving(&id("tw:hands"), &id("tw:dock_lamp"), &peat(), 1))?;
                Watch::ev(cx, "tw:lamp_stocked", 1);
                Ok(())
            }
            "odile_helps" => {
                cx.ledger.apply(&Tx::new().moving(&id("tw:reserve"), &id("tw:brazier"), &peat(), 1))?;
                Watch::ev(cx, "tw:odile_helped", 1);
                Ok(())
            }
            other => refuse(format!("the watch takes no `{other}`")),
        }
    }

    fn version(&self) -> u32 {
        1
    }

    fn save(&self) -> Value {
        json!({"phase": self.phase, "watch": self.watch, "left": self.left, "burn": self.burn, "dark": self.dark, "boats": [self.boats.0, self.boats.1], "last_safe": self.last_safe})
    }

    fn load(&mut self, s: Value) -> Result<(), String> {
        let n = |k: &str| s.get(k).and_then(Value::as_u64).ok_or(format!("watch: no `{k}`"));
        let boats = s.get("boats").and_then(Value::as_array).filter(|b| b.len() == 2).ok_or("watch: no `boats`")?;
        (self.phase, self.watch, self.left, self.burn, self.dark) =
            (n("phase")? as i64, n("watch")? as u32, n("left")? as u32, n("burn")? as u32, n("dark")? as u32);
        self.boats = (boats[0].as_u64().unwrap_or(0) as u32, boats[1].as_u64().unwrap_or(0) as u32);
        self.last_safe = s.get("last_safe").and_then(Value::as_bool).unwrap_or(false);
        Ok(())
    }

    fn hash(&self) -> u64 {
        let mut h = Fnv::new();
        for v in [
            self.phase as u64,
            self.watch as u64,
            self.left as u64,
            self.burn as u64,
            self.dark as u64,
            self.boats.0 as u64,
            self.boats.1 as u64,
            self.last_safe as u64,
        ] {
            h.u64(v);
        }
        h.0
    }

    fn instances(&self) -> Vec<Id> {
        vec![id("tw:night")]
    }

    fn inspect(&self, target: &Id, look: &Look) -> Option<Inspection> {
        (target.as_str() == "tw:night").then(|| {
            let status = match self.phase {
                phase::ASHORE => "ashore",
                phase::WATCH if Watch::count(look.ledger, "brazier") == 0 => "dark",
                phase::WATCH => "watching",
                _ => "done",
            };
            let blocked_by = (status == "dark")
                .then(|| Diagnostic::new(code::MISSING, "tw:brazier", "the brazier is empty").fix("carry a brick from the shed to the brazier"));
            Inspection {
                id: target.clone(),
                system: self.id(),
                instance_at: None,
                definition: None,
                defined_at: None,
                status: status.into(),
                blocked_by,
                state: self.save(),
            }
        })
    }
}
