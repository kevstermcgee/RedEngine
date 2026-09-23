use anyhow::{Context, Result};
use ffmpeg_sidecar::command::FfmpegCommand;
use ffmpeg_sidecar::event::{FfmpegEvent, LogLevel};
use std::io::Write;
use std::path::Path;
use std::thread::{self, JoinHandle};

/// Downloads a static ffmpeg binary the first time it's needed (no system ffmpeg install
/// required) — same "bundled, boring encoder" philosophy as the 2D engine's `imageio-ffmpeg`.
pub fn ensure_ffmpeg() -> Result<()> {
    ffmpeg_sidecar::download::auto_download().map_err(|e| anyhow::anyhow!("failed to set up ffmpeg: {e}"))
}

pub struct VideoEncoder {
    stdin: std::process::ChildStdin,
    event_thread: JoinHandle<()>,
}

impl VideoEncoder {
    /// Spawns ffmpeg, ready to receive raw `rgb24` frames of `width x height` on stdin.
    pub fn start(out_path: &Path, width: u32, height: u32, fps: u32) -> Result<Self> {
        ensure_ffmpeg()?;
        if let Some(parent) = out_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut child = FfmpegCommand::new()
            .arg("-y")
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24"])
            .args(["-video_size", &format!("{width}x{height}")])
            .args(["-framerate", &fps.to_string()])
            .input("-")
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "18", "-preset", "medium"])
            .args(["-movflags", "+faststart"])
            .output(out_path.to_string_lossy().as_ref())
            .spawn()
            .context("failed to spawn ffmpeg")?;

        let stdin = child.take_stdin().context("ffmpeg stdin was not available")?;
        let iter = child.iter().context("failed to attach to ffmpeg output")?;
        let event_thread = thread::spawn(move || {
            for event in iter {
                if let FfmpegEvent::Log(LogLevel::Error, msg) = event {
                    eprintln!("ffmpeg: {msg}");
                }
            }
        });

        Ok(VideoEncoder { stdin, event_thread })
    }

    pub fn write_frame(&mut self, rgb: &[u8]) -> Result<()> {
        self.stdin.write_all(rgb).context("failed to write a frame to ffmpeg")
    }

    /// Closes ffmpeg's stdin (signals end-of-stream) and waits for it to finish encoding.
    pub fn finish(self) -> Result<()> {
        drop(self.stdin);
        self.event_thread.join().map_err(|_| anyhow::anyhow!("ffmpeg logging thread panicked"))?;
        Ok(())
    }
}
