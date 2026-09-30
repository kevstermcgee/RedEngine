//! Kart race sounds: which cue to play when the race HUD changes (the countdown beeps, the green light, a pickup, a boost, a lap, the finish).
//!
//! Pure: [`KartSound::step`] takes the HUD of this frame and says what to play, so the rules are tested without a sound device. The clips are the
//! engine's own synthesized ones ([`crate::sfx`]); the windowed client plays what comes out.

use crate::sfx;
use crate::sim::kart::Item;
use crate::ui::race::RaceHud;

/// One sound the race asks for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KartCue {
    /// One beep of the countdown (3, 2, 1).
    CountBeep,
    /// The light went green.
    Go,
    /// A pickup went into the hand.
    Pickup,
    /// A boost began (Mushroom, drift release, slipstream).
    Boost,
    /// A drift charge reached a higher tier.
    DriftTier,
    /// A lap was completed.
    Lap,
    /// Our kart crossed the line for the last time; `won` when first.
    Finish {
        /// Whether we finished in first place.
        won: bool,
    },
}

impl KartCue {
    /// The clip for this cue (mono samples).
    pub fn clip(self) -> Vec<f32> {
        match self {
            KartCue::CountBeep => sfx::beep(880.0, 0.18),
            KartCue::Go => sfx::go(),
            KartCue::Pickup => sfx::kill_ding(),
            KartCue::Boost => sfx::pad_launch(),
            KartCue::DriftTier => sfx::hit_tick(),
            KartCue::Lap => sfx::level_up(false),
            KartCue::Finish { won: true } => sfx::victory(),
            KartCue::Finish { won: false } => sfx::defeat(),
        }
    }
}

/// Remembers the last HUD so it can tell what just happened.
#[derive(Debug, Default)]
pub struct KartSound {
    prev: Option<RaceHud>,
}

impl KartSound {
    /// The cues for this frame's HUD (`None` = not in a race: forget everything).
    pub fn step(&mut self, hud: Option<&RaceHud>) -> Vec<KartCue> {
        let mut cues = Vec::new();
        let Some(now) = hud else {
            self.prev = None;
            return cues;
        };
        if let Some(prev) = &self.prev {
            if now.phase == 0 {
                let (a, b) = (prev.countdown_secs.ceil() as i32, now.countdown_secs.ceil() as i32);
                if prev.phase == 0 && b < a && b >= 1 {
                    cues.push(KartCue::CountBeep);
                }
            }
            if prev.phase == 0 && now.phase == 1 {
                cues.push(KartCue::Go);
            }
            if prev.item == Item::None && now.item != Item::None {
                cues.push(KartCue::Pickup);
            }
            if !prev.boosting && now.boosting {
                cues.push(KartCue::Boost);
            }
            if now.drift_tier > prev.drift_tier {
                cues.push(KartCue::DriftTier);
            }
            if now.lap > prev.lap && !now.finished {
                cues.push(KartCue::Lap);
            }
            if !prev.finished && now.finished {
                cues.push(KartCue::Finish { won: now.place == 1 });
            }
        }
        self.prev = Some(now.clone());
        cues
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::race::demo_racing;

    fn hud() -> RaceHud {
        RaceHud { item: Item::None, boosting: false, drift_tier: 0, finished: false, lap: 1, phase: 1, ..demo_racing() }
    }

    #[test]
    fn the_countdown_beeps_each_second_and_the_light_says_go() {
        let mut s = KartSound::default();
        let at = |secs: f32| RaceHud { phase: 0, countdown_secs: secs, ..hud() };
        assert!(s.step(Some(&at(3.0))).is_empty(), "the first frame only remembers");
        assert!(s.step(Some(&at(2.9))).is_empty(), "still in the third second");
        assert_eq!(s.step(Some(&at(2.0))), vec![KartCue::CountBeep]);
        assert_eq!(s.step(Some(&at(1.0))), vec![KartCue::CountBeep]);
        assert_eq!(s.step(Some(&hud())), vec![KartCue::Go]);
    }

    #[test]
    fn pickups_boosts_drift_tiers_laps_and_the_finish_each_sound_once() {
        let mut s = KartSound::default();
        s.step(Some(&hud()));
        assert_eq!(s.step(Some(&RaceHud { item: Item::Acorn, ..hud() })), vec![KartCue::Pickup]);
        assert!(s.step(Some(&RaceHud { item: Item::Acorn, ..hud() })).is_empty(), "holding it is silent");
        assert_eq!(s.step(Some(&RaceHud { boosting: true, ..hud() })), vec![KartCue::Boost]);
        assert_eq!(s.step(Some(&RaceHud { boosting: true, drift_tier: 1, ..hud() })), vec![KartCue::DriftTier]);
        assert_eq!(s.step(Some(&RaceHud { lap: 2, ..hud() })), vec![KartCue::Lap]);
        assert_eq!(
            s.step(Some(&RaceHud { lap: 3, finished: true, place: 1, ..hud() })),
            vec![KartCue::Finish { won: true }],
            "the last lap is the finish, not a lap"
        );
    }

    #[test]
    fn leaving_the_race_forgets_it_and_every_cue_has_a_clip() {
        let mut s = KartSound::default();
        s.step(Some(&hud()));
        assert!(s.step(None).is_empty());
        assert!(s.step(Some(&RaceHud { item: Item::Bubble, ..hud() })).is_empty(), "a fresh race starts by only remembering");
        for cue in [
            KartCue::CountBeep,
            KartCue::Go,
            KartCue::Pickup,
            KartCue::Boost,
            KartCue::DriftTier,
            KartCue::Lap,
            KartCue::Finish { won: true },
            KartCue::Finish { won: false },
        ] {
            assert!(!cue.clip().is_empty(), "{cue:?}");
        }
    }
}
