//! A voice (a sound as data: `audio export`, a 2D game's `sounds`, a score's `instruments`). Checker: `voice_spec::parse_voice`.

use super::common::Any;
use crate::strict_struct;

strict_struct! {
    /// One synthesized sound.
    pub struct Voice { seconds: Any, level: Any, seed: Any, attack: Any, pitched: bool, layers: Vec<VoiceLayer> }
}

strict_struct! {
    /// One layer of a voice.
    pub struct VoiceLayer {
        sine: Any,
        harmonics: Any,
        glide: Any,
        sweep: Any,
        noise: String,
        decay: Any,
        fade_in: Any,
        delay: Any,
        attack: Any,
        release: Any,
        gain: Any,
    }
}
