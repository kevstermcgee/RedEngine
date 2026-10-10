//! A 2D game's sounds and music: rendered to samples from their descriptions (the same voice and score JSON the 3D engine uses) and measured by the shared audio analysis.
//!
//! Rendering happens where the game runs (tests, `verify`, the native window), so the player hears exactly the samples the analysis measured. What this module can
//! prove is the *waveform*: finite, not clipped, not silent, ends that do not click. It cannot prove the sound device started or that anything sounds good.

use crate::audio_analysis::{self, Clip, Kind, Report};
use crate::game::GameDef;
use crate::synth::SAMPLE_RATE;

/// Mono samples of sound `i` at [`SAMPLE_RATE`].
pub fn voice_pcm(def: &GameDef, i: usize) -> Result<Vec<f32>, String> {
    let v = crate::voice_spec::parse_voice_up_to(&def.sounds[i].voice, 6.0).map_err(|e| e.join("; "))?;
    Ok(v.render())
}

/// Interleaved stereo samples of music track `i` (one loop) at [`SAMPLE_RATE`], and its loudness.
pub fn music_pcm(def: &GameDef, i: usize) -> Result<(Vec<f32>, f32), String> {
    let s = crate::score::parse_score(&def.music[i].1).map_err(|e| e.join("; "))?;
    let r = s.render();
    Ok((r.samples, r.lufs))
}

/// One measured sound.
#[derive(Debug, Clone)]
pub struct SoundCheck {
    /// Its name.
    pub name: String,
    /// `sound` or `music`.
    pub kind: &'static str,
    /// The measurements.
    pub report: Report,
    /// Sentences about what is wrong (empty = fine).
    pub problems: Vec<String>,
}

/// Renders and measures every sound and music track of the game.
pub fn check_all(def: &GameDef) -> Vec<SoundCheck> {
    let mut out = Vec::new();
    for (i, s) in def.sounds.iter().enumerate() {
        match voice_pcm(def, i) {
            Ok(pcm) => {
                let report = audio_analysis::analyze(Clip { samples: &pcm, channels: 1, rate: SAMPLE_RATE });
                let problems = audio_analysis::problems(&report, Kind::OneShot);
                out.push(SoundCheck { name: s.name.clone(), kind: "sound", report, problems });
            }
            Err(e) => out.push(SoundCheck { name: s.name.clone(), kind: "sound", report: empty_report(), problems: vec![e] }),
        }
    }
    for (i, (name, _)) in def.music.iter().enumerate() {
        match music_pcm(def, i) {
            Ok((pcm, _)) => {
                let report = audio_analysis::analyze(Clip { samples: &pcm, channels: 2, rate: SAMPLE_RATE });
                let problems = audio_analysis::problems(&report, Kind::Loop);
                out.push(SoundCheck { name: name.clone(), kind: "music", report, problems });
            }
            Err(e) => out.push(SoundCheck { name: name.clone(), kind: "music", report: empty_report(), problems: vec![e] }),
        }
    }
    out
}

fn empty_report() -> Report {
    audio_analysis::analyze(Clip { samples: &[0.0; 4], channels: 1, rate: SAMPLE_RATE })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::tests::game;

    #[test]
    fn a_games_sounds_render_and_are_measured() {
        let def = game("", r##""c":{"shape":{"circle":2}}"##, r#"{"prefab":"c","at":[5,5]}"#);
        let checks = check_all(&def);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].name, "beep");
        assert!(checks[0].report.peak_dbfs > -40.0 && checks[0].report.secs > 0.09, "{:?}", checks[0].report);
        assert!(checks[0].problems.is_empty(), "{:?}", checks[0].problems);
    }
}
