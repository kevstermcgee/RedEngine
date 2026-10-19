//! What the game paints over the frame: the HUD while playing, or the screen that is up (the pause menu, the death screen, the title and its
//! forms, the screens with slots), repainted only when what it shows has changed.

use super::{Game, Mode, Screen, Title};
use crate::item::Stack;
use crate::screens::{crafted, stack_of};
use crate::ui::{self, Container, Hit, HudState, PauseState, TitleView};
use red_engine2::client::ModeContext;

impl Game {
    pub(super) fn hud_lines(&self, cx: &ModeContext) -> Vec<String> {
        let Some(p) = &self.play else { return Vec::new() };
        let b = &p.body;
        let (yaw, _) = cx.camera_angles();
        let facing = ["north (-Z)", "east (+X)", "south (+Z)", "west (-X)"][(((yaw.to_degrees().rem_euclid(360.0)) + 45.0) / 90.0) as usize % 4];
        let col = p.terrain.world.gen.column(b.pos.x.floor() as i32, b.pos.z.floor() as i32);
        let (sky, glow) = p.terrain.world.light(b.pos.x.floor() as i32, (b.pos.y + 0.5).floor() as i32, b.pos.z.floor() as i32);
        let st = p.terrain.stats();
        let tod = cx.time_of_day().unwrap_or(0.0) * 24.0;
        let mut v = vec![
            format!("RedCraft 1.0 ({:.0} fps)", self.fps),
            format!("XYZ: {:.2} / {:.2} / {:.2}", b.pos.x, b.pos.y, b.pos.z),
            format!("Facing: {facing}"),
            format!("Biome: {}", col.biome.name()),
            format!("Light: {sky} sky, {glow} block"),
            format!("Chunks: {} loaded, {} drawn, {} jobs", st.loaded, st.meshed, st.in_flight),
            format!("Time: {:02}:{:02}, day {}", tod as u32, ((tod.fract()) * 60.0) as u32, p.days + 1),
            format!("Seed: {}  {}", p.seed, p.mode.title()),
        ];
        if let Some(t) = p.target {
            v.push(format!("Looking at: {} {} {} {}", t.block.info().title, t.pos.x, t.pos.y, t.pos.z));
        }
        v
    }

    /// Repaints the overlay when what it shows changed.
    pub(super) fn paint(&mut self, cx: &mut ModeContext) {
        let Some((w, h)) = cx.window_size() else { return };
        let debug_lines = if self.debug && matches!(self.screen, Screen::Hud) { self.hud_lines(cx) } else { Vec::new() };
        let blink = ((self.since_start * 2.0) as u32).is_multiple_of(2);
        let key = {
            use std::hash::{Hash, Hasher};
            let mut hs = std::collections::hash_map::DefaultHasher::new();
            (w, h, self.screen.name(), format!("{:?}", self.screen)).hash(&mut hs);
            if self.screen != Screen::Hud {
                ((self.mouse.0 as i32), (self.mouse.1 as i32)).hash(&mut hs);
            }
            if let Some(p) = &self.play {
                format!("{:?}{:?}{:?}{:?}", p.inv, p.cursor, p.grid, (p.health as i32, (p.air.ceil()) as i32, p.hurt_flash > 0.5, p.ready)).hash(&mut hs);
                if let Screen::Furnace(at) = &self.screen {
                    format!("{:?}", p.furnaces.get(at).map(|f| (f.input, f.fuel, f.output, (f.burn * 4.0) as i32, (f.cook * 4.0) as i32))).hash(&mut hs);
                }
                if let Screen::Chest(at) = &self.screen {
                    format!("{:?}", p.chests.get(at)).hash(&mut hs);
                }
            }
            if let Some((n, a)) = &self.held_name {
                (n, (a * 16.0) as i32).hash(&mut hs);
            }
            if let Some((m, a)) = &self.message {
                (m, (a * 16.0) as i32).hash(&mut hs);
            }
            debug_lines.hash(&mut hs);
            if matches!(self.screen, Screen::Title(Title::NewWorld { .. })) {
                blink.hash(&mut hs);
            }
            hs.finish()
        };
        if key == self.painted_key && self.painted.is_some() {
            return;
        }
        let mouse = self.mouse;
        let painted = match &self.screen {
            Screen::Hud => {
                let Some(p) = &self.play else { return };
                let in_water = p.body.head_in_water && p.mode == Mode::Survival;
                let message = self.message.as_ref().map(|(m, a)| (m.as_str(), *a)).or_else(|| (!p.ready).then_some(("Loading terrain...", 1.0)));
                ui::paint_hud(
                    w,
                    h,
                    &mut self.icons,
                    &HudState {
                        slots: &p.inv.slots,
                        selected: p.inv.selected,
                        survival: p.mode == Mode::Survival,
                        health: p.health,
                        air: (in_water || p.air < crate::play::MAX_AIR - 0.01).then_some(p.air / crate::play::MAX_AIR),
                        held_name: self.held_name.as_ref().map(|(n, a)| (n.as_str(), *a)),
                        debug: (!debug_lines.is_empty()).then_some(&debug_lines[..]),
                        blink: p.hurt_flash > 0.5,
                        message,
                    },
                )
            }
            Screen::Pause => {
                let Some(p) = &self.play else { return };
                let s = PauseState {
                    music: cx.music_on(),
                    sound: cx.sfx_on(),
                    view: self.opts.view,
                    auto_jump: self.opts.auto_jump,
                    mode: p.mode.title(),
                    world: p.name.clone(),
                };
                ui::paint_pause(w, h, &mut self.icons, &s, mouse)
            }
            Screen::Death => {
                let cause = self.play.as_ref().and_then(|p| p.dead.clone()).unwrap_or_default();
                ui::paint_death(w, h, &mut self.icons, &cause, mouse)
            }
            Screen::Title(t) => {
                let names: Vec<(String, String)> =
                    self.worlds.iter().map(|(_, l)| (l.name.clone(), format!("{}, day {}", l.mode.title(), l.days + 1))).collect();
                let view = match t {
                    Title::Main => TitleView::Main { last: self.worlds.first().map(|(_, l)| l.name.as_str()), splash: self.splash },
                    Title::NewWorld { name, seed, creative, focus } => TitleView::NewWorld { name, seed, creative: *creative, focus: *focus, blink },
                    Title::Worlds { page } => TitleView::Worlds { list: &names, page: *page },
                };
                ui::paint_title(w, h, &mut self.icons, &view, mouse)
            }
            s => {
                let Some(p) = &self.play else { return };
                let out = crafted(p, s);
                let default_furnace = crate::craft::Furnace::default();
                let empty_chest = vec![Stack::EMPTY; 27];
                let cont = match s {
                    Screen::Inventory => Container::Inventory { grid: &p.grid[..4], out },
                    Screen::Table(_) => Container::Table { grid: &p.grid[..9], out },
                    Screen::Furnace(at) => Container::Furnace { f: p.furnaces.get(at).unwrap_or(&default_furnace) },
                    Screen::Chest(at) => Container::Chest { slots: p.chests.get(at).unwrap_or(&empty_chest) },
                    Screen::Creative { page } => Container::Creative { items: &self.creative_items, page: *page },
                    _ => return,
                };
                // The name of the stack under the mouse.
                let hover = self.painted.as_ref().and_then(|pp| pp.at(mouse.0, mouse.1)).and_then(|h| match h {
                    Hit::Slot(r) => {
                        let st = stack_of(p, s, r, &self.creative_items);
                        (!st.is_empty()).then(|| st.item.title())
                    }
                    _ => None,
                });
                ui::paint_slots(w, h, &mut self.icons, &p.inv.slots, &cont, &p.cursor, mouse, hover)
            }
        };
        cx.set_window_overlay(w, h, Some(&painted.canvas.px));
        self.painted = Some(painted);
        self.painted_key = key;
    }
}
