//! The graphical client's online front end (ADR 0029): the connect form, the lobby / results screens that take over the window, and the HUD
//! over the world. The screens themselves are `red_engine2::ui::online` (headless, audited by `ui-check`); this file only feeds them what
//! the `NetClient` knows and turns clicks and keys into `NetClient` calls.

use super::*;
use red_engine2::net::client::{ClientConfig, ConnState};
use red_engine2::ui::online::{
    action_at, connect_action_for, connect_layout, hud_layout, lobby_layout, results_layout, screen_for, ConnectAction, ConnectForm, OnlineAction,
    OnlineScreen, OnlineView,
};
use red_engine2::ui::Layout;
use std::hash::{Hash, Hasher};

/// What the online screens currently show and where the pointer is.
pub(crate) struct OnlineUi {
    /// The connect form's text.
    pub form: ConnectForm,
    /// The button under the cursor on the current online screen.
    pub hover: Option<String>,
    /// Hash of the layout the overlay currently shows (repaint only when it changes).
    pub painted: Option<u64>,
    /// A lobby or results screen owns the window (mouse free, world frozen).
    pub takeover: bool,
}

impl OnlineUi {
    pub(crate) fn new(address: &str, key: &str, name: &str) -> Self {
        OnlineUi { form: ConnectForm::new(address, key, name), hover: None, painted: None, takeover: false }
    }
}

/// A fingerprint of everything a layout draws (rectangles, text, colours), so the overlay is only re-uploaded when something changed.
fn layout_hash(l: &Layout) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (l.w, l.h).hash(&mut h);
    for w in &l.widgets {
        (&w.id, w.rect, &w.text, w.scale, w.color, w.fill, w.frame.map(|f| (f.0, f.1))).hash(&mut h);
    }
    h.finish()
}

impl App {
    fn map_stem(&self) -> String {
        self.scene_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
    }

    fn window_size(&self) -> Option<(u32, u32)> {
        self.gpu.as_ref().map(|g| (g.config.width, g.config.height))
    }

    /// What the client knows, in the shape the online screens take (`None` offline, or before the first `Status`).
    pub(crate) fn online_view(&self) -> Option<OnlineView> {
        let c = &self.net.as_ref()?.client;
        let st = c.status()?;
        Some(OnlineView {
            phase: st.phase,
            round: st.round,
            secs_left: (st.ticks_left != u32::MAX).then(|| st.ticks_left.div_ceil(60)),
            min_players: st.min_players,
            me: c.my_id().unwrap_or(0),
            in_round: c.in_round(),
            roster: st.roster.clone(),
            winner: st.winner,
            end_code: st.end_code,
            end_text: st.end_text.clone(),
            ping_ms: c.stats().rtt_ms,
            map: self.map_stem(),
            reconnecting: matches!(c.state(), ConnState::Reconnecting | ConnState::Connecting),
            message: None,
        })
    }

    /// Whether the local body is held still (a lobby, a countdown, the results, or watching a round in progress).
    pub(crate) fn online_frozen(&self) -> bool {
        self.net.as_ref().is_some_and(|n| n.client.state() == ConnState::Connected && !n.client.in_round())
    }

    /// Each frame while online: decides which screen the phase needs, frees or captures the mouse when a screen takes over, and repaints
    /// the overlay when its content changed.
    pub(crate) fn sync_online_ui(&mut self) {
        if self.paused {
            return;
        }
        let Some(view) = self.online_view() else { return };
        let screen = screen_for(&view);
        let takeover = screen != OnlineScreen::Hud;
        if takeover != self.online.takeover {
            self.online.takeover = takeover;
            self.online.hover = None;
            self.keys.clear();
            self.sprint_held = false;
            self.set_grab(!takeover);
        }
        let Some((w, h)) = self.window_size() else { return };
        let hover = self.online.hover.clone();
        let layout = match screen {
            OnlineScreen::Lobby => lobby_layout(w, h, &view, hover.as_deref()),
            OnlineScreen::Results => results_layout(w, h, &view, hover.as_deref()),
            OnlineScreen::Hud => hud_layout(w, h, &view),
        };
        let hash = layout_hash(&layout);
        if self.online.painted == Some(hash) {
            return;
        }
        self.online.painted = Some(hash);
        if let Some(gpu) = self.gpu.as_mut() {
            if let Some(live) = gpu.live.as_mut() {
                live.overlay.set(&gpu.device, &gpu.queue, w, h, &layout.paint().px);
            }
        }
    }

    /// The layout currently under the pointer (lobby or results), for hit-testing.
    fn takeover_layout(&self) -> Option<Layout> {
        let view = self.online_view()?;
        let (w, h) = self.window_size()?;
        match screen_for(&view) {
            OnlineScreen::Lobby => Some(lobby_layout(w, h, &view, None)),
            OnlineScreen::Results => Some(results_layout(w, h, &view, None)),
            OnlineScreen::Hud => None,
        }
    }

    pub(crate) fn online_hover(&mut self, x: f32, y: f32) {
        self.cursor = (x, y);
        let hover = self.takeover_layout().and_then(|l| l.button_at(x, y).map(str::to_string));
        if hover != self.online.hover {
            self.online.hover = hover;
        }
    }

    pub(crate) fn online_click(&mut self, event_loop: &ActiveEventLoop) {
        let action = self.takeover_layout().and_then(|l| action_at(&l, self.cursor.0, self.cursor.1));
        if let Some(a) = action {
            self.online_action(a, event_loop);
        }
    }

    pub(crate) fn online_action(&mut self, a: OnlineAction, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let Some(net) = self.net.as_mut() else { return };
        match a {
            OnlineAction::ToggleReady => {
                let want = !net.client.is_ready();
                net.client.set_ready(want, now);
            }
            OnlineAction::ToggleCharacter => {
                let other = 1 - net.client.character();
                net.client.set_character(other, now);
            }
            OnlineAction::Leave => {
                net.client.disconnect();
                event_loop.exit();
            }
        }
    }

    /// Keys while a lobby or results screen has the window: R / Enter ready, C character, Escape leave.
    pub(crate) fn online_key(&mut self, code: KeyCode, event_loop: &ActiveEventLoop) {
        match code {
            KeyCode::KeyR | KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => self.online_action(OnlineAction::ToggleReady, event_loop),
            KeyCode::KeyC => self.online_action(OnlineAction::ToggleCharacter, event_loop),
            KeyCode::Escape => self.online_action(OnlineAction::Leave, event_loop),
            KeyCode::KeyF => self.toggle_fullscreen(),
            _ => {}
        }
    }

    // ---- the connect form --------------------------------------------------------------------------------------------------------

    /// Opens the connect form over the launch menu's backdrop.
    pub(crate) fn open_connect(&mut self) {
        self.phase = Phase::Connect;
        self.online.form.message = None;
        self.online.hover = None;
        self.online.painted = None;
    }

    fn close_connect(&mut self) {
        self.phase = Phase::Menu;
        self.menu_painted = None;
    }

    pub(crate) fn connect_key(&mut self, event: &winit::event::KeyEvent, event_loop: &ActiveEventLoop) {
        let _ = event_loop;
        if event.state != ElementState::Pressed {
            return;
        }
        match event.physical_key {
            PhysicalKey::Code(KeyCode::Escape) => self.close_connect(),
            PhysicalKey::Code(KeyCode::Enter | KeyCode::NumpadEnter) => self.try_connect(),
            PhysicalKey::Code(KeyCode::Tab) => self.online.form.next_field(),
            PhysicalKey::Code(KeyCode::Backspace) => self.online.form.backspace(),
            _ => {
                if let Some(text) = &event.text {
                    for c in text.chars() {
                        self.online.form.type_char(c);
                    }
                }
            }
        }
    }

    pub(crate) fn connect_hover(&mut self, x: f32, y: f32) {
        self.cursor = (x, y);
        let hover = self.window_size().and_then(|(w, h)| connect_layout(w, h, &self.online.form, None).button_at(x, y).map(str::to_string));
        self.online.hover = hover;
    }

    pub(crate) fn connect_click(&mut self) {
        let Some((w, h)) = self.window_size() else { return };
        let hit = connect_layout(w, h, &self.online.form, None).button_at(self.cursor.0, self.cursor.1).and_then(connect_action_for);
        match hit {
            Some(ConnectAction::Focus(f)) => self.online.form.focus = f,
            Some(ConnectAction::Connect) => self.try_connect(),
            Some(ConnectAction::Back) => self.close_connect(),
            None => {}
        }
    }

    /// Tries to join the typed server (blocking for up to three seconds); on success starts the game with the connected session, on
    /// failure says what went wrong on the form.
    pub(crate) fn try_connect(&mut self) {
        let addr_text = self.online.form.address_with_port();
        let Some(addr) = addr_text.to_socket_addrs().ok().and_then(|mut i| i.next()) else {
            self.online.form.message = Some(format!("Cannot find '{addr_text}'. Check the address and your internet connection."));
            return;
        };
        let world = match ClientWorld::load(&self.scene_path) {
            Ok((_scene, world)) => world,
            Err(e) => {
                self.online.form.message = Some(format!("Cannot load the map: {e}"));
                return;
            }
        };
        let mut cfg = ClientConfig::new(addr, if self.character == Character::Rat { 1 } else { 0 }, world.map_hash, 0);
        let key = self.online.form.key.trim().to_string();
        cfg.join_key = (!key.is_empty()).then_some(key);
        cfg.name = self.online.form.name.clone();
        let mut session = match NetSession::connect_with(cfg, world) {
            Ok(s) => s,
            Err(e) => {
                self.online.form.message = Some(format!("Cannot open a network socket: {e}"));
                return;
            }
        };
        match session.wait_connected(3.0) {
            Ok(spawn) => {
                session.teleport = spawn; // start_game places the player there (None in a lobby)
                self.join_key = cfg_key(&self.online.form);
                self.player_name = self.online.form.name.clone();
                println!("Connected to {addr}.");
                self.pending_net = Some(session);
                self.net_server = Some(addr);
                let who = self.character;
                self.start_game(who);
            }
            Err(e) => self.online.form.message = Some(e),
        }
    }

    /// One frame of the connect form: the menu's 3-D backdrop, the form over it.
    pub(crate) fn connect_frame(&mut self) {
        let Some((w, h)) = self.window_size() else { return };
        let layout = connect_layout(w, h, &self.online.form, self.online.hover.as_deref());
        let hash = layout_hash(&layout);
        let repaint = self.online.painted != Some(hash);
        if repaint {
            self.online.painted = Some(hash);
        }
        let Some(gpu) = self.gpu.as_mut() else { return };
        let Some(menu_live) = gpu.menu.as_mut() else { return };
        let Some((surface_tex, reconfigure)) = acquire_frame(&gpu.surface, &gpu.device, &gpu.config) else { return };
        let t = self.start.elapsed().as_secs_f32();
        menu::animate(&mut self.menu_scene, w as f32 / h as f32, t, self.character);
        if repaint {
            menu_live.overlay.set(&gpu.device, &gpu.queue, w, h, &layout.paint().px);
        }
        let view = surface_tex.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let hidden = Mat4::from_scale(Vec3::splat(HIDDEN_SCALE));
        menu_live.render_ex(
            &gpu.device,
            &gpu.queue,
            &self.menu_scene,
            t,
            &menu::menu_camera(),
            &view,
            false,
            hidden,
            hidden,
            FrameOptions { crosshair: false, viewmodel: false, pickup: false, ..FrameOptions::default() },
        );
        gpu.queue.present(surface_tex);
        if reconfigure {
            gpu.surface.configure(&gpu.device, &gpu.config);
        }
    }
}

fn cfg_key(form: &ConnectForm) -> Option<String> {
    let k = form.key.trim();
    (!k.is_empty()).then(|| k.to_string())
}
