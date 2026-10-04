//! The engine's sound synthesis that needs no sound card: the sample rate, the WAV encoder and the one-off clips that live outside `sfx`.
//!
//! This is the part of audio that is pure arithmetic, so it also builds in the headless server and the analysis tools (`audio report`, `audio render`):
//! [`crate::audio::Audio`] (the `rodio` player) is the only thing that needs a device, and it stays behind the `gfx` feature.

/// Output sample rate of the synthesized clips, in Hz.
pub const SAMPLE_RATE: u32 = 44100;

/// Encodes `interleaved` (`f32` in -1..1, `channels` per frame) as a 16-bit PCM WAV file in memory: a 44-byte
/// RIFF/WAVE header followed by little-endian `i16` samples. No new dependency for this — the format is this
/// simple. Used to let a player save the engine's generated music (`crate::music::loop_samples`) as a file.
pub fn wav_bytes_i16(interleaved: &[f32], sample_rate: u32, channels: u16) -> Vec<u8> {
    let bits_per_sample: u16 = 16;
    let block_align = channels * bits_per_sample / 8;
    let byte_rate = sample_rate * block_align as u32;
    let data_size = (interleaved.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_size as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_size).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM subchunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // audio format: 1 = PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits_per_sample.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_size.to_le_bytes());
    for s in interleaved {
        let sample = (s.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

/// [`wav_bytes_i16`], written straight to `path`.
pub fn write_wav_i16(path: &std::path::Path, interleaved: &[f32], sample_rate: u32, channels: u16) -> Result<(), String> {
    std::fs::write(path, wav_bytes_i16(interleaved, sample_rate, channels)).map_err(|e| format!("{}: {e}", path.display()))
}

/// A short wooden "thock" for the bat connecting: a fast-decaying low body thump (the impact), a
/// dry woody resonance a little above it, and a brief noise crack at the very start. Purely
/// synthesized, same reasoning as the engine's procedural meshes: no sample file to import.
pub fn synth_bat_hit() -> Vec<f32> {
    let duration_s = 0.20_f32;
    let n = (SAMPLE_RATE as f32 * duration_s) as usize;
    let mut noise = crate::dsp::Noise(0x9E3779B9);
    let mut lp = 0.0f32;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SAMPLE_RATE as f32;
        let thump = (2.0 * std::f32::consts::PI * (95.0 - 30.0 * t) * t).sin() * (-t * 22.0).exp();
        let wood = (2.0 * std::f32::consts::PI * 340.0 * t).sin() * (-t * 38.0).exp();
        let wood2 = (2.0 * std::f32::consts::PI * 610.0 * t).sin() * (-t * 55.0).exp();
        lp += 0.35 * (noise.white() - lp); // crude low-pass: a dull crack, not a hiss
        let crack = lp * (-t * 90.0).exp();
        out.push((thump * 0.75 + wood * 0.45 + wood2 * 0.18 + crack * 0.9) * 0.9);
    }
    // The layers add up past full scale at the crack: bring the peak under it (no clipping once mixed) and fade the last 2 ms so it ends on zero.
    let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs())).max(1e-6);
    let scale = (0.95 / peak).min(1.0);
    let fade = (SAMPLE_RATE as f32 * 0.002) as usize;
    let len = out.len();
    for (i, s) in out.iter_mut().enumerate() {
        *s *= scale * if i + fade > len { (len - i) as f32 / fade as f32 } else { 1.0 };
    }
    out
}

/// A dry metallic click: the hammer falling on an empty magazine, or raising a firearm.
pub fn synth_weapon_click() -> Vec<f32> {
    let n = (SAMPLE_RATE as f32 * 0.05) as usize;
    let mut noise = crate::dsp::Noise(0xC11C4B);
    (0..n)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            let ring = (2.0 * std::f32::consts::PI * 2400.0 * t).sin() * (-t * 140.0).exp();
            let tick = noise.white() * (-t * 400.0).exp();
            ((ring * 0.5 + tick * 0.6) * 0.8).clamp(-1.0, 1.0)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_weapon_click_is_short_and_audible() {
        let click = synth_weapon_click();
        assert!(click.len() < 3000, "a click, not a sound effect: {} samples", click.len());
        assert!(click.iter().fold(0.0f32, |m, s| m.max(s.abs())) > 0.3);
    }

    #[test]
    fn wav_bytes_round_trip_the_header_and_sample_count() {
        let samples = vec![0.0, 0.5, -1.0, 1.0, -0.25, 0.25]; // 3 stereo frames
        let bytes = wav_bytes_i16(&samples, SAMPLE_RATE, 2);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"fmt ");
        let channels = u16::from_le_bytes([bytes[22], bytes[23]]);
        let rate = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
        let bits = u16::from_le_bytes([bytes[34], bytes[35]]);
        assert_eq!((channels, rate, bits), (2, SAMPLE_RATE, 16));
        assert_eq!(&bytes[36..40], b"data");
        let data_size = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]) as usize;
        assert_eq!(data_size, samples.len() * 2);
        assert_eq!(bytes.len(), 44 + data_size);
        // A full-scale sample must round-trip to the i16 extreme, not clip into noise or silence.
        let last_two = &bytes[bytes.len() - 2..];
        assert_eq!(i16::from_le_bytes([last_two[0], last_two[1]]), (0.25 * i16::MAX as f32).round() as i16);
    }

    #[test]
    fn write_wav_writes_exactly_the_encoded_bytes() {
        let dir = std::env::temp_dir().join(format!("re2_audio_wav_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.wav");
        let samples = vec![0.1, -0.2, 0.3, -0.4];
        write_wav_i16(&path, &samples, SAMPLE_RATE, 2).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), wav_bytes_i16(&samples, SAMPLE_RATE, 2));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bat_hit_is_short_audible_and_decays() {
        let clip = synth_bat_hit();
        assert!(clip.len() > 4000 && clip.len() < 12000);
        let peak = clip.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.3 && peak <= 1.0, "peak {peak}");
        let tail = clip[clip.len() - 500..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(tail < 0.02, "clip should have decayed by the end: {tail}");
    }
}
