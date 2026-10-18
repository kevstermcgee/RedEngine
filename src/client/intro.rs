//! The "RedEngine Games" title card before the game (see [`crate::ui::title_card`]): asked for with `--intro`, or by a player's launch
//! (`RE2_PLAYER=1`). While it plays the window shows only the card, nothing updates and no input reaches the game; any key, click or the end of the
//! card starts it.

use super::*;
use crate::overlay::Overlay;
use crate::ui::title_card::TitleCard;
use winit::event::{ElementState, WindowEvent};

/// The card, its clock, and the overlay it is drawn with.
pub(crate) struct Intro {
    card: TitleCard,
    start: Instant,
    overlay: Overlay,
    px: Vec<u8>,
}

impl App {
    /// Starts the card (once the window and the GPU exist).
    pub(crate) fn start_intro(&mut self) {
        let Some(gpu) = self.gpu.as_ref() else { return };
        self.intro = Some(Intro { card: TitleCard::default(), start: Instant::now(), overlay: Overlay::new(&gpu.device, gpu.config.format), px: Vec::new() });
    }

    /// The game begins: everything that paints the overlay paints it afresh, and the frame clock starts from now.
    pub(crate) fn end_intro(&mut self) {
        self.intro = None;
        self.online.painted = None;
        self.rule_hud_painted = None;
        self.split_hud_painted.clear();
        self.last_frame = Instant::now();
        if self.paused {
            self.repaint_pause();
        }
    }

    /// While the card plays: a key press (other than F11, which `window_event` handles first) or a click skips it. True when the event was the card's.
    pub(crate) fn intro_input(&mut self, event: &WindowEvent) -> bool {
        if self.intro.is_none() {
            return false;
        }
        match event {
            WindowEvent::KeyboardInput { event: k, .. } => {
                use winit::keyboard::{KeyCode, PhysicalKey};
                if k.physical_key == PhysicalKey::Code(KeyCode::F11) {
                    return false;
                }
                if k.state == ElementState::Pressed && !k.repeat {
                    self.end_intro();
                }
                true
            }
            WindowEvent::MouseInput { state, .. } => {
                if *state == ElementState::Pressed {
                    self.end_intro();
                }
                true
            }
            _ => false,
        }
    }

    /// Draws this frame of the card; false when there is no card (it never started, or it just ended).
    pub(crate) fn intro_frame(&mut self) -> bool {
        let Some(intro) = self.intro.as_mut() else { return false };
        let Some((w, h)) = self.gpu.as_ref().map(|g| (g.config.width, g.config.height)) else { return true };
        let (cw, ch) = TitleCard::size_for(w, h);
        if !intro.card.paint(cw, ch, intro.start.elapsed().as_secs_f32(), &mut intro.px) {
            self.end_intro();
            return false;
        }
        let Some(gpu) = self.gpu.as_mut() else { return true };
        let Some(surface) = gpu.surface.as_ref() else { return true };
        let Some((frame, reconfigure)) = acquire_frame(surface, &gpu.device, &gpu.config) else { return true };
        intro.overlay.set(&gpu.device, &gpu.queue, cw, ch, &intro.px);
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("title-card") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("title-card"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            intro.overlay.draw(&mut pass);
        }
        gpu.queue.submit([encoder.finish()]);
        gpu.queue.present(frame);
        if reconfigure {
            surface.configure(&gpu.device, &gpu.config);
        }
        true
    }
}
