//! In-window local map travel; rebuilds scene state while retaining the native window/device.
use super::*;
impl App {
    pub(crate) fn open_maps(&mut self) {
        if self.net.is_some() || self.project_maps.is_empty() {
            return;
        }
        self.enter_pause();
        self.map_selection = Some(0);
        self.repaint_maps();
    }
    pub(crate) fn repaint_maps(&mut self) {
        let Some(selected) = self.map_selection else { return };
        if let Some(g) = self.gpu.as_mut() {
            if let Some(live) = g.live.as_mut() {
                let l = red_engine2::project_browser::layout(g.win.config.width, g.win.config.height, &self.project_maps, selected);
                live.overlay.set(&g.win.device, &g.win.queue, g.win.config.width, g.win.config.height, &l.paint().px);
            }
        }
    }
    pub(crate) fn map_key(&mut self, key: KeyCode) {
        let Some(selected) = self.map_selection else { return };
        let count = self.project_maps.len();
        match key {
            KeyCode::Escape | KeyCode::KeyM => {
                self.map_selection = None;
                self.leave_pause();
                self.controller.reset();
            }
            KeyCode::ArrowDown => self.map_selection = Some((selected + 1) % count),
            KeyCode::ArrowUp => self.map_selection = Some((selected + count - 1) % count),
            KeyCode::ArrowRight => self.map_selection = Some((selected + 8).min(count - 1)),
            KeyCode::ArrowLeft => self.map_selection = Some(selected.saturating_sub(8)),
            KeyCode::Enter => {
                self.travel_to(selected);
                return;
            }
            _ => {}
        }
        self.repaint_maps();
    }
    pub(crate) fn map_click(&mut self) {
        let (Some(g), Some(selected)) = (&self.gpu, self.map_selection) else { return };
        let l = red_engine2::project_browser::layout(g.win.config.width, g.win.config.height, &self.project_maps, selected);
        let index = l.button_at(self.cursor.0, self.cursor.1).and_then(|id| id.strip_prefix("map")).and_then(|s| s.parse().ok());
        if let Some(index) = index {
            self.travel_to(index);
        }
    }
    fn travel_to(&mut self, index: usize) {
        let Some(entry) = self.project_maps.get(index) else { return };
        let path = entry.path.clone();
        let scene = match red_engine2::load_scene(&path) {
            Ok(scene) => scene,
            Err(errors) => {
                eprintln!("Map load failed: {}", errors.join("; "));
                return;
            }
        };
        let who = scene.player.character.unwrap_or(self.character);
        let mut next = App::new(scene, path, Some(who), None, None);
        next.window = self.window.take();
        next.gpu = self.gpu.take();
        next.view_mode = self.view_mode;
        next.start_game(who);
        if let Some(w) = &next.window {
            w.set_title(&format!("Red Engine 2 - {}", next.scene_path.display()));
        }
        *self = next;
    }
}
