//! Hardware probes for `red_engine2 doctor` that need the graphics/audio crates (so they live in a `gfx`-gated module, like
//! the rest of the windowed code; the headless build reports these as "not compiled in").

use crate::tools::doctor::{check, Check, Status};

/// The sentence about GPU device files that exist but this user cannot open (empty when there are none): on Linux the usual reason a machine with a GPU renders in
/// software is that the user is not in the `render`/`video` group. `denied` are the file names, like `renderD128`.
pub fn gpu_permission_hint(denied: &[String]) -> String {
    if denied.is_empty() {
        return String::new();
    }
    format!(
        " This machine has a GPU it cannot use ({} in /dev/dri is not accessible to this user): add yourself to the `render` and `video` groups (`sudo usermod -aG render,video $USER`, then log in again) to draw, and time, on the real GPU.",
        denied.join(", ")
    )
}

/// The `/dev/dri` device files this user cannot open (Linux; empty elsewhere).
fn denied_gpu_devices() -> Vec<String> {
    #[cfg(target_os = "linux")]
    {
        let mut out: Vec<String> = std::fs::read_dir("/dev/dri")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                (name.starts_with("renderD") || name.starts_with("card")).then_some((name, e.path()))
            })
            .filter(|(_, path)| std::fs::OpenOptions::new().read(true).write(true).open(path).is_err_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied))
            .map(|(name, _)| name)
            .collect();
        out.sort();
        out
    }
    #[cfg(not(target_os = "linux"))]
    Vec::new()
}

/// Whether a GPU adapter is available: hardware first, then a software fallback (Mesa lavapipe, Windows WARP). The line says what the adapter IS (`GPU` or
/// `software rasteriser`, from its device type), never which probe attempt found it: wgpu answers the first, non-forced request with llvmpipe when the real GPU cannot be opened.
pub fn gpu() -> Check {
    let instance = wgpu::Instance::default();
    for force in [false, true] {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { force_fallback_adapter: force, ..Default::default() }));
        if let Ok(a) = adapter {
            let i = a.get_info();
            let soft = matches!(i.device_type, wgpu::DeviceType::Cpu);
            let status = if soft { Status::Warn } else { Status::Ok };
            let note = if soft {
                format!(
                    " (software rendering: correct but slow, fine for `frame`/`tour`/`verify` views; its timings say little about a player's GPU.{})",
                    gpu_permission_hint(&denied_gpu_devices())
                )
            } else {
                String::new()
            };
            return check(
                "gpu",
                status,
                format!("{} via {:?}, {:?} [{}]{note}", i.name, i.backend, i.device_type, if soft { "software rasteriser" } else { "GPU" }),
            );
        }
    }
    check(
        "gpu",
        Status::Warn,
        "no GPU adapter (not even a software one): `frame`, `tour`, `storyboard`, `render` and golden views will fail; everything else works. \
         Linux: install `mesa-vulkan-drivers` (lavapipe) for software rendering; containers: use the headless build",
    )
}

/// Whether an audio output device can be opened.
pub fn audio() -> Check {
    match rodio::OutputStream::try_default() {
        Ok(_) => check("audio", Status::Ok, "an output device opened (the game client plays sounds)"),
        Err(e) => check("audio", Status::Warn, format!("no audio output ({e}): the game client runs silently; nothing else needs audio")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gpu_the_user_cannot_open_is_named_with_its_fix_and_a_machine_without_one_says_nothing() {
        assert_eq!(gpu_permission_hint(&[]), "");
        let hint = gpu_permission_hint(&["card0".to_string(), "renderD128".to_string()]);
        assert!(hint.contains("card0, renderD128") && hint.contains("usermod -aG render,video"), "{hint}");
    }
}
