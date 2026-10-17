//! What enters and leaves the workshop: sources (`ws:supply`: a mine delivers a random amount into a store every few seconds, as much as
//! fits) and sinks (`ws:shipping`: takes a shipment from a store every few seconds, when the whole shipment is there). Both are content
//! (`sources`, `sinks`); these are the two behaviors, each with per-definition counters.

use crate::content::Catalog;
use red_engine2::sim::defs::{code, Diagnostic, Id};
use red_engine2::sim::ledger::{Fnv, Op, Reason, Tx};
use red_engine2::sim::systems::{Cx, Inspection, Look, System};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

fn id(s: &str) -> Id {
    Id::parse(s).unwrap_or_else(|e| panic!("{e}"))
}

/// The sources.
pub struct Supply {
    cat: Arc<Catalog>,
    /// Per source: units delivered, and units that did not fit (never made).
    pub delivered: BTreeMap<Id, (u64, u64)>,
    /// Scratch kept between ticks (a tick allocates nothing).
    tx: Tx,
}

impl Supply {
    /// Every source of the catalog, nothing delivered yet.
    pub fn new(cat: Arc<Catalog>) -> Supply {
        let delivered = cat.sources.iter().map(|(i, _)| (*i, (0, 0))).collect();
        Supply { cat, delivered, tx: Tx::new() }
    }
}

impl System for Supply {
    fn id(&self) -> Id {
        id("ws:supply")
    }

    fn tick(&mut self, cx: &mut Cx) {
        let Supply { cat, delivered, tx } = self;
        for (sid, src) in cat.sources.iter() {
            if !cx.tick.is_multiple_of(src.every as u64) {
                continue;
            }
            let why = Reason::Source(*sid);
            for (res, (lo, hi)) in &src.gives {
                let want = lo + cx.random(hi - lo + 1);
                let k = want.min(cx.ledger.free(&src.into));
                tx.ops.clear();
                tx.ops.push(Op::Produce { to: src.into, res: *res, n: k, why });
                if k > 0 && cx.ledger.try_apply(tx).is_ok() {
                    delivered.entry(*sid).or_default().0 += k as u64;
                }
                delivered.entry(*sid).or_default().1 += (want - k) as u64;
            }
        }
    }

    fn save(&self) -> Value {
        json!(self.delivered)
    }

    fn load(&mut self, state: Value) -> Result<(), String> {
        self.delivered = serde_json::from_value(state).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn hash(&self) -> u64 {
        let mut h = Fnv::new();
        for (i, (a, b)) in &self.delivered {
            h.str(i.as_str());
            h.u64(*a);
            h.u64(*b);
        }
        h.0
    }

    fn instances(&self) -> Vec<Id> {
        self.delivered.keys().cloned().collect()
    }

    fn inspect(&self, target: &Id, look: &Look) -> Option<Inspection> {
        let src = self.cat.sources.get(target)?;
        let (made, refused) = self.delivered.get(target).copied().unwrap_or((0, 0));
        let full = look.ledger.free(&src.into) == 0;
        Some(Inspection {
            id: *target,
            system: self.id(),
            instance_at: None,
            definition: Some(*target),
            defined_at: self.cat.sources.provenance(target).cloned(),
            status: if full { "blocked" } else { "working" }.into(),
            blocked_by: full.then(|| {
                Diagnostic::new(code::FULL, target, format!("`{}` is full: deliveries wait", src.into)).fix(format!("use or ship what `{}` holds", src.into))
            }),
            state: json!({"into": src.into, "every_ticks": src.every, "delivered": made, "did_not_fit": refused}),
        })
    }
}

/// The sinks.
pub struct Shipping {
    cat: Arc<Catalog>,
    /// Per sink: units taken.
    pub shipped: BTreeMap<Id, u64>,
    /// Scratch kept between ticks (a tick allocates nothing).
    tx: Tx,
    shipped_event: Id,
}

impl Shipping {
    /// Every sink of the catalog, nothing shipped yet.
    pub fn new(cat: Arc<Catalog>) -> Shipping {
        let shipped = cat.sinks.iter().map(|(i, _)| (*i, 0)).collect();
        Shipping { cat, shipped, tx: Tx::new(), shipped_event: id("ws:shipped") }
    }
}

impl System for Shipping {
    fn id(&self) -> Id {
        id("ws:shipping")
    }

    fn after(&self) -> Vec<Id> {
        vec![id("ws:machines")]
    }

    fn emits(&self) -> Vec<Id> {
        vec![id("ws:shipped")]
    }

    fn tick(&mut self, cx: &mut Cx) {
        let Shipping { cat, shipped, tx, shipped_event } = self;
        for (sid, sink) in cat.sinks.iter() {
            if !cx.tick.is_multiple_of(sink.every as u64) {
                continue;
            }
            let why = Reason::Sink(*sid);
            tx.ops.clear();
            tx.ops.extend(sink.takes.iter().map(|(res, n)| Op::Consume { from: sink.from, res: *res, n: *n, why }));
            if cx.ledger.try_apply(tx).is_ok() {
                let n: u32 = sink.takes.values().sum();
                *shipped.entry(*sid).or_default() += n as u64;
                let _ = cx.emit(shipped_event, Some(sid), n as i64);
            }
        }
    }

    fn save(&self) -> Value {
        json!(self.shipped)
    }

    fn load(&mut self, state: Value) -> Result<(), String> {
        self.shipped = serde_json::from_value(state).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn hash(&self) -> u64 {
        let mut h = Fnv::new();
        for (i, n) in &self.shipped {
            h.str(i.as_str());
            h.u64(*n);
        }
        h.0
    }

    fn instances(&self) -> Vec<Id> {
        self.shipped.keys().cloned().collect()
    }

    fn inspect(&self, target: &Id, look: &Look) -> Option<Inspection> {
        let sink = self.cat.sinks.get(target)?;
        let missing = sink.takes.iter().find(|(r, n)| look.ledger.count(&sink.from, r) < **n);
        Some(Inspection {
            id: *target,
            system: self.id(),
            instance_at: None,
            definition: Some(*target),
            defined_at: self.cat.sinks.provenance(target).cloned(),
            status: if missing.is_some() { "waiting" } else { "working" }.into(),
            blocked_by: missing.map(|(r, n)| {
                Diagnostic::new(code::MISSING, target, format!("a shipment takes {n} `{r}`; `{}` holds {}", sink.from, look.ledger.count(&sink.from, r)))
                    .fix(format!("send `{r}` to `{}`", sink.from))
            }),
            state: json!({"from": sink.from, "takes": sink.takes, "every_ticks": sink.every, "shipped": self.shipped.get(target).copied().unwrap_or(0)}),
        })
    }
}
