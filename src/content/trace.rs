//! A match trace (`--record`, `replay`): written by the engine, read back by `sim::trace::Trace::from_json` (which tolerates extra keys: the
//! drift test holds the repository's traces to both definitions but does not probe keys; see the ADR).

use super::common::Any;
use crate::strict_struct;

strict_struct! {
    /// A recorded match.
    pub struct Trace {
        trace_version: u32,
        engine: String,
        tick_rate: u32,
        scene: String,
        map_hash: u64,
        seed: String,
        spawn_group: String,
        platform: String,
        checkpoint_every: u32,
        dump_every: u32,
        entries: Vec<Any>,
        checkpoints: Vec<Any>,
        events: Vec<Any>,
        dumps: Vec<Any>,
        final_tick: u64,
    }
}
