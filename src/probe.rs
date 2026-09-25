//! Hardware probes for `red_engine2 doctor` that need the graphics/audio crates (so they live in a `gfx`-gated module, like
//! the rest of the windowed code; the headless build reports these as "not compiled in").

use crate::tools::doctor::{check, Check, Status};

/// Whether a GPU adapter is available: hardware first, then a software fallback (Mesa lavapipe, Windows WARP).
pub fn gpu() -> Check {
    let instance = wgpu::Instance::default();
    for (force, label) in [(false, "hardware"), (true, "software fallback")] {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { force_fallback_adapter: force, ..Default::default() }));
        if let Ok(a) = adapter {
            let i = a.get_info();
            let soft = matches!(i.device_type, wgpu::DeviceType::Cpu);
            let status = if soft { Status::Warn } else { Status::Ok };
            let note = if soft { " (software rendering: correct but slow, fine for `frame`/`tour`/`verify` views)" } else { "" };
            return check("gpu", status, format!("{} via {:?}, {:?} [{label}]{note}", i.name, i.backend, i.device_type));
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
