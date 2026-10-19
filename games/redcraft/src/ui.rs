//! The screens, painted on the CPU as the original lays them out (in its "GUI pixels", times a scale that fits the window): the HUD (hotbar,
//! hearts, air, crosshair, the name of what is in hand, the debug lines), the inventory with its 2 x 2 grid, the crafting table, the furnace, the
//! chest, the creative list, the pause menu, the title screen and its forms, and the death screen. Every paint also says where its buttons and
//! slots are ([`Hit`]), so a click is found on exactly what was drawn.

use crate::icons::{icon, Icon};
use crate::item::{Item, Stack};
use red_engine2::ui::Canvas;
use std::collections::HashMap;

/// A slot a screen shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SlotRef {
    /// The player's own (0..9 the hotbar).
    Inv(usize),
    /// The crafting grid (2 x 2 or 3 x 3, row by row).
    Craft(usize),
    CraftOut,
    FurnaceIn,
    FurnaceFuel,
    FurnaceOut,
    Chest(usize),
    /// The creative list: an item to take.
    Creative(usize),
}

/// A button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Button {
    Resume,
    Music,
    Sound,
    View,
    AutoJump,
    Fullscreen,
    QuitTitle,
    QuitGame,
    Continue,
    NewWorld,
    LoadWorld,
    World(usize),
    Create,
    Cancel,
    ModeToggle,
    NameField,
    SeedField,
    Respawn,
    Title,
    PageUp,
    PageDown,
}

/// What is under a point of the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Slot(SlotRef),
    Button(Button),
    /// Inside a panel, on nothing.
    Panel,
}

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x0 as f32 && y >= self.y0 as f32 && x < self.x1 as f32 && y < self.y1 as f32
    }
}

/// A painted screen: its pixels and where its parts are.
pub struct Painted {
    pub canvas: Canvas,
    pub hits: Vec<(Rect, Hit)>,
}

impl Painted {
    /// The topmost thing at a point.
    pub fn at(&self, x: f32, y: f32) -> Option<Hit> {
        self.hits.iter().rev().find(|(r, _)| r.contains(x, y)).map(|(_, h)| *h)
    }
}

/// The scale of a GUI pixel for a window (as the original's "auto": as big as fits 400 x 300).
pub fn gui_scale(w: u32, h: u32) -> i32 {
    ((w / 400).min(h / 300) as i32).clamp(1, 6)
}

/// Icon pictures, made once per size.
#[derive(Default)]
pub struct IconCache(HashMap<(Item, u32), Icon>);

impl IconCache {
    pub fn get(&mut self, item: Item, size: u32) -> &Icon {
        self.0.entry((item, size)).or_insert_with(|| icon(item, size))
    }
}

const BLACK: [u8; 4] = [0, 0, 0, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];
const PANEL: [u8; 4] = [198, 198, 198, 255];
const SLOT: [u8; 4] = [139, 139, 139, 255];
const DARK: [u8; 4] = [55, 55, 55, 255];
const TEXT_DARK: [u8; 4] = [64, 64, 64, 255];

/// The painter: a canvas, a scale and the hits found so far.
pub struct Gfx<'a> {
    pub c: Canvas,
    pub g: i32,
    pub hits: Vec<(Rect, Hit)>,
    pub icons: &'a mut IconCache,
}

impl<'a> Gfx<'a> {
    pub fn new(w: u32, h: u32, icons: &'a mut IconCache) -> Gfx<'a> {
        Gfx { c: Canvas::new(w, h), g: gui_scale(w, h), hits: Vec::new(), icons }
    }

    pub fn done(self) -> Painted {
        Painted { canvas: self.c, hits: self.hits }
    }

    /// A rectangle in screen pixels: opaque colours are written straight, others blended.
    pub fn fill(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, col: [u8; 4]) {
        if col[3] == 255 {
            let (x0, x1) = (x0.max(0), x1.min(self.c.w));
            let (y0, y1) = (y0.max(0), y1.min(self.c.h));
            for y in y0..y1 {
                let row = (y * self.c.w) as usize * 4;
                for x in x0..x1 {
                    let i = row + x as usize * 4;
                    self.c.px[i..i + 4].copy_from_slice(&col);
                }
            }
        } else if col[3] > 0 {
            self.c.rect(x0, y0, x1, y1, col);
        }
    }

    /// A rectangle in GUI pixels from `(ox, oy)` (screen).
    #[allow(clippy::too_many_arguments)]
    pub fn gfill(&mut self, ox: i32, oy: i32, x: i32, y: i32, w: i32, h: i32, col: [u8; 4]) {
        let g = self.g;
        self.fill(ox + x * g, oy + y * g, ox + (x + w) * g, oy + (y + h) * g, col);
    }

    /// Text in GUI scale with a drop shadow; returns its width in screen pixels.
    pub fn text(&mut self, x: i32, y: i32, s: &str, col: [u8; 4], shadow: bool) -> i32 {
        let g = self.g;
        if shadow {
            let sh = [col[0] / 4, col[1] / 4, col[2] / 4, col[3]];
            self.c.ink(x + g, y + g, s, g, sh);
        }
        self.c.ink(x, y, s, g, col);
        red_engine2::ui::text_width(s, g)
    }

    pub fn text_width(&self, s: &str) -> i32 {
        red_engine2::ui::text_width(s, self.g)
    }

    pub fn centered(&mut self, cx: i32, y: i32, s: &str, col: [u8; 4]) {
        let w = self.text_width(s);
        self.text(cx - w / 2, y, s, col, true);
    }

    /// An icon at screen (x, y), `size` square.
    pub fn icon(&mut self, item: Item, x: i32, y: i32, size: u32) {
        let ic = self.icons.get(item, size);
        let (w, h) = (self.c.w, self.c.h);
        for yy in 0..size as i32 {
            let sy = y + yy;
            if sy < 0 || sy >= h {
                continue;
            }
            for xx in 0..size as i32 {
                let sx = x + xx;
                if sx < 0 || sx >= w {
                    continue;
                }
                let s = ((yy as u32 * size + xx as u32) * 4) as usize;
                if ic.px[s + 3] == 0 {
                    continue;
                }
                let d = ((sy * w + sx) * 4) as usize;
                self.c.px[d..d + 4].copy_from_slice(&ic.px[s..s + 4]);
            }
        }
    }

    /// A stack at screen (x, y) in a 16-GUI-pixel square: its icon, its count, a tool's wear.
    pub fn stack(&mut self, s: &Stack, x: i32, y: i32) {
        if s.is_empty() {
            return;
        }
        let g = self.g;
        self.icon(s.item, x, y, (16 * g) as u32);
        if let Some(t) = s.item.tool() {
            if s.wear > 0 {
                let left = 1.0 - s.wear as f32 / t.durability() as f32;
                let bw = (13.0 * left).round() as i32;
                let colour = [((1.0 - left) * 255.0) as u8, (left * 255.0) as u8, 0, 255];
                self.fill(x + 2 * g, y + 13 * g, x + 15 * g, y + 15 * g, BLACK);
                self.fill(x + 2 * g, y + 13 * g, x + (2 + bw) * g, y + 14 * g, colour);
            }
        }
        if s.count > 1 {
            let t = s.count.to_string();
            let w = self.text_width(&t);
            self.text(x + 17 * g - w - g, y + 9 * g, &t, WHITE, true);
        }
    }

    /// A slot (18 GUI pixels square) with `(gx, gy)` its top-left in GUI pixels from `(ox, oy)`.
    #[allow(clippy::too_many_arguments)]
    pub fn slot(&mut self, ox: i32, oy: i32, gx: i32, gy: i32, s: &Stack, r: SlotRef, hover: bool) {
        let g = self.g;
        self.gfill(ox, oy, gx, gy, 18, 18, DARK);
        self.gfill(ox, oy, gx + 1, gy + 1, 17, 17, WHITE);
        self.gfill(ox, oy, gx + 1, gy + 1, 16, 16, SLOT);
        let (x, y) = (ox + (gx + 1) * g, oy + (gy + 1) * g);
        self.stack(s, x, y);
        if hover {
            self.fill(x, y, x + 16 * g, y + 16 * g, [255, 255, 255, 110]);
        }
        self.hits.push((Rect { x0: ox + gx * g, y0: oy + gy * g, x1: ox + (gx + 18) * g, y1: oy + (gy + 18) * g }, Hit::Slot(r)));
    }

    /// The original's raised grey panel, `w` x `h` GUI pixels, centred; returns its top-left on screen.
    pub fn panel(&mut self, w: i32, h: i32) -> (i32, i32) {
        let g = self.g;
        let (ox, oy) = ((self.c.w - w * g) / 2, (self.c.h - h * g) / 2);
        self.gfill(ox, oy, 1, 0, w - 2, h, BLACK);
        self.gfill(ox, oy, 0, 1, w, h - 2, BLACK);
        self.gfill(ox, oy, 1, 1, w - 2, h - 2, WHITE);
        self.gfill(ox, oy, 3, 3, w - 4, h - 4, [85, 85, 85, 255]);
        self.gfill(ox, oy, 3, 3, w - 6, h - 6, PANEL);
        self.hits.push((Rect { x0: ox, y0: oy, x1: ox + w * g, y1: oy + h * g }, Hit::Panel));
        (ox, oy)
    }

    /// A button `w` GUI pixels wide and 20 high at screen (x, y).
    #[allow(clippy::too_many_arguments)]
    pub fn button(&mut self, x: i32, y: i32, w: i32, label: &str, b: Button, hover: bool, enabled: bool) {
        let g = self.g;
        let base = if !enabled {
            [70, 70, 70, 255]
        } else if hover {
            [126, 138, 196, 255]
        } else {
            [111, 111, 111, 255]
        };
        self.fill(x, y, x + w * g, y + 20 * g, BLACK);
        self.fill(x + g, y + g, x + (w - 1) * g, y + 19 * g, base);
        self.fill(x + g, y + g, x + (w - 1) * g, y + 2 * g, [170, 170, 170, 255]);
        self.fill(x + g, y + 17 * g, x + (w - 1) * g, y + 19 * g, [76, 76, 76, 255]);
        if hover && enabled {
            self.fill(x, y, x + w * g, y + g, WHITE);
            self.fill(x, y + 19 * g, x + w * g, y + 20 * g, WHITE);
        }
        let colour = if !enabled {
            [160, 160, 160, 255]
        } else if hover {
            [255, 255, 160, 255]
        } else {
            [224, 224, 224, 255]
        };
        let tw = self.text_width(label);
        self.text(x + (w * g - tw) / 2, y + 6 * g, label, colour, true);
        self.hits.push((Rect { x0: x, y0: y, x1: x + w * g, y1: y + 20 * g }, Hit::Button(b)));
    }

    /// A text field `w` GUI pixels wide at screen (x, y).
    #[allow(clippy::too_many_arguments)]
    pub fn field(&mut self, x: i32, y: i32, w: i32, text: &str, b: Button, focused: bool, blink: bool) {
        let g = self.g;
        self.fill(x, y, x + w * g, y + 20 * g, if focused { WHITE } else { [160, 160, 160, 255] });
        self.fill(x + g, y + g, x + (w - 1) * g, y + 19 * g, BLACK);
        let shown = if focused && blink { format!("{text}_") } else { text.to_string() };
        self.text(x + 4 * g, y + 6 * g, &shown, [224, 224, 224, 255], false);
        self.hits.push((Rect { x0: x, y0: y, x1: x + w * g, y1: y + 20 * g }, Hit::Button(b)));
    }
}

/// What the HUD shows.
pub struct HudState<'a> {
    pub slots: &'a [Stack],
    pub selected: usize,
    pub survival: bool,
    pub health: f32,
    /// Air left 0..1, while under water.
    pub air: Option<f32>,
    /// The name of what was just taken in hand, and how visible it is (0..1).
    pub held_name: Option<(&'a str, f32)>,
    pub debug: Option<&'a [String]>,
    /// The hurt flash: hearts blink.
    pub blink: bool,
    /// A line in the middle (a message: "Saved", "Day 3").
    pub message: Option<(&'a str, f32)>,
}

pub fn paint_hud(w: u32, h: u32, icons: &mut IconCache, s: &HudState) -> Painted {
    let mut gx = Gfx::new(w, h, icons);
    let g = gx.g;
    let (cw, ch) = (w as i32, h as i32);
    // The crosshair.
    let (mx, my) = (cw / 2, ch / 2);
    let arm = 5 * g;
    gx.fill(mx - arm - g / 2, my - g / 2 - (g + 1) / 2 + 1, mx + arm + g / 2 + 1, my + (g + 1) / 2, [16, 16, 16, 150]);
    gx.fill(mx - g / 2 - (g + 1) / 2 + 1, my - arm - g / 2, mx + (g + 1) / 2, my + arm + g / 2 + 1, [16, 16, 16, 150]);
    gx.fill(mx - arm, my - g / 2, mx + arm + 1, my + (g + 1) / 2 - (g > 1) as i32 + 1, [235, 235, 235, 230]);
    gx.fill(mx - g / 2, my - arm, mx + (g + 1) / 2 - (g > 1) as i32 + 1, my + arm + 1, [235, 235, 235, 230]);
    // The hotbar: 182 x 22.
    let (ox, oy) = ((cw - 182 * g) / 2, ch - 23 * g);
    gx.gfill(ox, oy, 0, 0, 182, 22, [0, 0, 0, 160]);
    gx.gfill(ox, oy, 1, 1, 180, 20, [88, 88, 88, 200]);
    for i in 0..9 {
        gx.gfill(ox, oy, 2 + i * 20, 2, 18, 18, [40, 40, 40, 200]);
        gx.gfill(ox, oy, 3 + i * 20, 3, 16, 16, [104, 104, 104, 190]);
    }
    let sel = s.selected as i32;
    gx.gfill(ox, oy, sel * 20 - 1, -1, 24, 24, BLACK);
    gx.gfill(ox, oy, sel * 20, 0, 22, 22, [230, 230, 230, 255]);
    gx.gfill(ox, oy, sel * 20 + 2, 2, 18, 18, [40, 40, 40, 200]);
    gx.gfill(ox, oy, sel * 20 + 3, 3, 16, 16, [120, 120, 120, 200]);
    for i in 0..9 {
        let st = s.slots[i];
        gx.stack(&st, ox + (3 + i as i32 * 20) * g, oy + 3 * g);
    }
    if s.survival {
        // Ten hearts over the left half.
        let hy = oy - 10 * g;
        for k in 0..10 {
            let x = ox + k * 8 * g;
            let full = s.health - k as f32 * 2.0;
            heart(
                &mut gx,
                x,
                hy,
                if full >= 2.0 {
                    2
                } else if full >= 1.0 {
                    1
                } else {
                    0
                },
                s.blink,
            );
        }
        if let Some(air) = s.air {
            let bubbles = (air * 10.0).ceil() as i32;
            for k in 0..bubbles {
                let x = ox + (182 - 9 - k * 8) * g;
                bubble(&mut gx, x, hy);
            }
        }
    }
    if let Some((name, a)) = s.held_name {
        let alpha = (a * 255.0) as u8;
        let tw = gx.text_width(name);
        let y = oy - if s.survival { 22 } else { 12 } * g;
        gx.text((cw - tw) / 2, y, name, [255, 255, 255, alpha], true);
    }
    if let Some((m, a)) = s.message {
        let tw = gx.text_width(m);
        gx.text((cw - tw) / 2, ch / 3, m, [255, 255, 255, (a * 255.0) as u8], true);
    }
    if let Some(lines) = s.debug {
        for (i, l) in lines.iter().enumerate() {
            let y = 2 * g + i as i32 * 9 * g;
            let tw = gx.text_width(l);
            gx.fill(g, y - g, 3 * g + tw, y + 8 * g, [80, 80, 80, 140]);
            gx.text(2 * g, y, l, [224, 224, 224, 255], false);
        }
    }
    gx.done()
}

fn heart(gx: &mut Gfx, x: i32, y: i32, fill: u8, blink: bool) {
    const SHAPE: [&str; 9] = [".##...##.", "#rr#.#rr#", "#rRr#rrr#", "#rrrrrrr#", "#rrrrrrr#", ".#rrrrr#.", "..#rrr#..", "...#r#...", "....#...."];
    let g = gx.g;
    for (j, row) in SHAPE.iter().enumerate() {
        for (i, ch) in row.chars().enumerate() {
            let c = match ch {
                '#' => {
                    if blink {
                        [255, 255, 255, 255]
                    } else {
                        [20, 0, 0, 255]
                    }
                }
                'r' | 'R' => {
                    let lit = fill == 2 || (fill == 1 && i < 4);
                    if !lit {
                        [60, 30, 30, 255]
                    } else if ch == 'R' {
                        [255, 190, 190, 255]
                    } else {
                        [220, 20, 30, 255]
                    }
                }
                _ => continue,
            };
            gx.fill(x + i as i32 * g, y + j as i32 * g, x + (i as i32 + 1) * g, y + (j as i32 + 1) * g, c);
        }
    }
}

fn bubble(gx: &mut Gfx, x: i32, y: i32) {
    const SHAPE: [&str; 9] = ["..###....", ".#bbb#...", "#bwbbb#..", "#bbbbb#..", "#bbbbb#..", ".#bbb#...", "..###....", ".........", "........."];
    let g = gx.g;
    for (j, row) in SHAPE.iter().enumerate() {
        for (i, ch) in row.chars().enumerate() {
            let c = match ch {
                '#' => [10, 40, 120, 255],
                'b' => [70, 140, 255, 255],
                'w' => [255, 255, 255, 255],
                _ => continue,
            };
            gx.fill(x + i as i32 * g, y + j as i32 * g, x + (i as i32 + 1) * g, y + (j as i32 + 1) * g, c);
        }
    }
}

/// Which container a screen shows (besides the player's own slots).
pub enum Container<'a> {
    /// The inventory: its 2 x 2 grid and what it makes.
    Inventory {
        grid: &'a [Stack],
        out: Stack,
    },
    Table {
        grid: &'a [Stack],
        out: Stack,
    },
    Furnace {
        f: &'a crate::craft::Furnace,
    },
    Chest {
        slots: &'a [Stack],
    },
    Creative {
        items: &'a [Item],
        page: usize,
    },
}

/// A screen with slots: the container's part, then the player's 27 + 9, the stack on the mouse drawn last.
#[allow(clippy::too_many_arguments)]
pub fn paint_slots(
    w: u32,
    h: u32,
    icons: &mut IconCache,
    inv: &[Stack],
    cont: &Container,
    cursor: &Stack,
    mouse: (f32, f32),
    hover_title: Option<&str>,
) -> Painted {
    let mut gx = Gfx::new(w, h, icons);
    let g = gx.g;
    gx.fill(0, 0, w as i32, h as i32, [16, 16, 16, 150]);
    let tall = matches!(cont, Container::Creative { .. });
    let (ox, oy) = gx.panel(176, if tall { 184 } else { 166 });
    let hover_slot = |gx: &Gfx, gxx: i32, gyy: i32| {
        let r = Rect { x0: ox + gxx * gx.g, y0: oy + gyy * gx.g, x1: ox + (gxx + 18) * gx.g, y1: oy + (gyy + 18) * gx.g };
        r.contains(mouse.0, mouse.1)
    };
    let inv_y = match cont {
        Container::Chest { .. } => 84,
        Container::Creative { .. } => 160,
        _ => 84,
    };
    match cont {
        Container::Inventory { grid, out } => {
            gx.text(ox + 97 * g, oy + 6 * g, "Crafting", TEXT_DARK, false);
            // Where the original shows the player: the RedCraft mark.
            gx.gfill(ox, oy, 26, 8, 52, 62, [0, 0, 0, 255]);
            gx.gfill(ox, oy, 27, 9, 50, 60, [40, 40, 52, 255]);
            gx.centered(ox + 52 * g, oy + 30 * g, "RED", [230, 70, 60, 255]);
            gx.centered(ox + 52 * g, oy + 40 * g, "CRAFT", [220, 220, 220, 255]);
            for i in 0..4 {
                let (sx, sy) = (98 + (i % 2) as i32 * 18, 18 + (i / 2) as i32 * 18);
                let hv = hover_slot(&gx, sx, sy);
                gx.slot(ox, oy, sx, sy, &grid[i], SlotRef::Craft(i), hv);
            }
            arrow(&mut gx, ox + 136 * g, oy + 28 * g, 1.0);
            let hv = hover_slot(&gx, 152, 26);
            gx.slot(ox, oy, 152, 26, out, SlotRef::CraftOut, hv);
        }
        Container::Table { grid, out } => {
            gx.text(ox + 28 * g, oy + 6 * g, "Crafting", TEXT_DARK, false);
            for i in 0..9 {
                let (sx, sy) = (29 + (i % 3) as i32 * 18, 16 + (i / 3) as i32 * 18);
                let hv = hover_slot(&gx, sx, sy);
                gx.slot(ox, oy, sx, sy, &grid[i], SlotRef::Craft(i), hv);
            }
            arrow(&mut gx, ox + 90 * g, oy + 35 * g, 1.0);
            let hv = hover_slot(&gx, 123, 34);
            gx.slot(ox, oy, 123, 34, out, SlotRef::CraftOut, hv);
        }
        Container::Furnace { f } => {
            gx.text(ox + 60 * g, oy + 6 * g, "Furnace", TEXT_DARK, false);
            let hv = hover_slot(&gx, 55, 16);
            gx.slot(ox, oy, 55, 16, &f.input, SlotRef::FurnaceIn, hv);
            let hv = hover_slot(&gx, 55, 52);
            gx.slot(ox, oy, 55, 52, &f.fuel, SlotRef::FurnaceFuel, hv);
            let hv = hover_slot(&gx, 115, 34);
            gx.slot(ox, oy, 115, 34, &f.output, SlotRef::FurnaceOut, hv);
            flame(&mut gx, ox + 56 * g, oy + 36 * g, if f.burn_total > 0.0 { f.burn / f.burn_total } else { 0.0 });
            arrow(&mut gx, ox + 79 * g, oy + 35 * g, f.cook / crate::craft::SMELT_SECS);
        }
        Container::Chest { slots } => {
            gx.text(ox + 8 * g, oy + 6 * g, "Chest", TEXT_DARK, false);
            for i in 0..27 {
                let (sx, sy) = (7 + (i % 9) as i32 * 18, 17 + (i / 9) as i32 * 18);
                let hv = hover_slot(&gx, sx, sy);
                gx.slot(ox, oy, sx, sy, &slots[i], SlotRef::Chest(i), hv);
            }
        }
        Container::Creative { items, page } => {
            let pages = items.len().div_ceil(63).max(1);
            gx.text(ox + 8 * g, oy + 6 * g, &format!("Creative  {}/{}", page + 1, pages), TEXT_DARK, false);
            for k in 0..63 {
                let n = page * 63 + k;
                let (sx, sy) = (7 + (k % 9) as i32 * 18, 16 + (k / 9) as i32 * 18);
                let s = items.get(n).map_or(Stack::EMPTY, |it| Stack::new(*it, 1));
                let hv = hover_slot(&gx, sx, sy);
                gx.slot(ox, oy, sx, sy, &s, SlotRef::Creative(n), hv);
            }
            if pages > 1 {
                gx.button(ox + 120 * g, oy + 2 * g, 24, "<", Button::PageUp, false, *page > 0);
                gx.button(ox + 146 * g, oy + 2 * g, 24, ">", Button::PageDown, false, page + 1 < pages);
            }
        }
    }
    if !tall {
        gx.text(ox + 8 * g, oy + (inv_y - 11) * g, "Inventory", TEXT_DARK, false);
        for i in 0..27 {
            let (sx, sy) = (7 + (i % 9) as i32 * 18, inv_y + (i / 9) as i32 * 18);
            let hv = hover_slot(&gx, sx, sy);
            gx.slot(ox, oy, sx, sy, &inv[9 + i], SlotRef::Inv(9 + i), hv);
        }
    }
    let hot_y = if tall { inv_y } else { inv_y + 58 };
    for (i, st) in inv.iter().take(9).enumerate() {
        let sx = 7 + i as i32 * 18;
        let hv = hover_slot(&gx, sx, hot_y);
        gx.slot(ox, oy, sx, hot_y, st, SlotRef::Inv(i), hv);
    }
    if let Some(t) = hover_title.filter(|_| cursor.is_empty()) {
        let (mx, my) = (mouse.0 as i32 + 8 * g, mouse.1 as i32 - 12 * g);
        let tw = gx.text_width(t);
        gx.fill(mx - 3 * g, my - 3 * g, mx + tw + 3 * g, my + 10 * g, [16, 0, 16, 235]);
        gx.fill(mx - 2 * g, my - 2 * g, mx + tw + 2 * g, my - g, [80, 0, 200, 255]);
        gx.text(mx, my, t, WHITE, true);
    }
    // The stack on the mouse, last, over everything.
    let c = *cursor;
    gx.stack(&c, mouse.0 as i32 - 8 * g, mouse.1 as i32 - 8 * g);
    gx.done()
}

fn arrow(gx: &mut Gfx, x: i32, y: i32, progress: f32) {
    let g = gx.g;
    for i in 0..22 {
        let half = if i < 15 { 2 } else { 7 - (i - 15) };
        let lit = (i as f32) < progress.clamp(0.0, 1.0) * 22.0;
        let c = if lit { [255, 255, 255, 255] } else { [139, 139, 139, 255] };
        gx.fill(x + i * g, y + (8 - half) * g, x + (i + 1) * g, y + (8 + half) * g, c);
    }
}

fn flame(gx: &mut Gfx, x: i32, y: i32, left: f32) {
    let g = gx.g;
    let lit_from = 14 - (left.clamp(0.0, 1.0) * 14.0).round() as i32;
    for j in 0..14 {
        let half = [1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 6, 5, 4][j as usize];
        let c = if j >= lit_from {
            if j > 9 {
                [255, 210, 60, 255]
            } else {
                [255, 120, 20, 255]
            }
        } else {
            [139, 139, 139, 255]
        };
        gx.fill(x + (7 - half) * g, y + j * g, x + (7 + half) * g, y + (j + 1) * g, c);
    }
}

/// The pause menu's settings as it shows them.
pub struct PauseState {
    pub music: bool,
    pub sound: bool,
    pub view: i32,
    pub auto_jump: bool,
    pub mode: &'static str,
    pub world: String,
}

pub fn paint_pause(w: u32, h: u32, icons: &mut IconCache, s: &PauseState, mouse: (f32, f32)) -> Painted {
    let mut gx = Gfx::new(w, h, icons);
    let g = gx.g;
    gx.fill(0, 0, w as i32, h as i32, [10, 10, 16, 150]);
    let cx = w as i32 / 2;
    let top = h as i32 / 2 - 100 * g;
    gx.centered(cx, top, "Game Menu", WHITE);
    gx.centered(cx, top + 12 * g, &format!("{} - {}", s.world, s.mode), [190, 190, 190, 255]);
    let on = |b: bool| if b { "ON" } else { "OFF" };
    let rows: [(String, Button); 8] = [
        ("Back to Game".to_string(), Button::Resume),
        (format!("Music: {}", on(s.music)), Button::Music),
        (format!("Sound: {}", on(s.sound)), Button::Sound),
        (format!("Render Distance: {} chunks", s.view), Button::View),
        (format!("Auto-Jump: {}", on(s.auto_jump)), Button::AutoJump),
        ("Fullscreen".to_string(), Button::Fullscreen),
        ("Save and Quit to Title".to_string(), Button::QuitTitle),
        ("Save and Quit Game".to_string(), Button::QuitGame),
    ];
    for (i, (label, b)) in rows.iter().enumerate() {
        let (x, y) = (cx - 100 * g, top + (28 + i as i32 * 24) * g);
        let hover = Rect { x0: x, y0: y, x1: x + 200 * g, y1: y + 20 * g }.contains(mouse.0, mouse.1);
        gx.button(x, y, 200, label, *b, hover, true);
    }
    gx.done()
}

/// The title screen and its forms.
pub enum TitleView<'a> {
    Main { last: Option<&'a str>, splash: &'a str },
    NewWorld { name: &'a str, seed: &'a str, creative: bool, focus: Button, blink: bool },
    Worlds { list: &'a [(String, String)], page: usize },
}

/// Draws the big blocky logo: each pixel of each letter a little stone (or red) block with a lit top-left edge.
fn logo(gx: &mut Gfx, cx: i32, y: i32, px: i32) {
    let word = "REDCRAFT";
    let char_w = (5 + 1) * px;
    let w = word.len() as i32 * char_w - px;
    let mut x = cx - w / 2;
    let stone = crate::icons::texture(crate::block::tex::STONE);
    let brick = crate::icons::texture(crate::block::tex::BRICKS);
    for (k, ch) in word.chars().enumerate() {
        let tex = if k < 3 { &brick } else { &stone };
        let mut glyph = Canvas::new(5, 7);
        glyph.ink(0, 0, &ch.to_string(), 1, WHITE);
        for gy in 0..7 {
            for gxx in 0..5 {
                if glyph.px[((gy * 5 + gxx) * 4 + 3) as usize] == 0 {
                    continue;
                }
                let (bx, by) = (x + gxx * px, y + gy * px);
                // The block's shadow, then its face sampled from the texture.
                gx.fill(bx + px / 4, by + px / 4, bx + px + px / 4, by + px + px / 4, [0, 0, 0, 160]);
                for yy in 0..px {
                    for xx in 0..px {
                        let t = tex[((yy * 16 / px) * 16 + xx * 16 / px) as usize];
                        let edge = if xx < px / 8 || yy < px / 8 {
                            1.25
                        } else if xx >= px - px / 8 || yy >= px - px / 8 {
                            0.6
                        } else {
                            1.0
                        };
                        let c = [(t[0] as f32 * edge).min(255.0) as u8, (t[1] as f32 * edge).min(255.0) as u8, (t[2] as f32 * edge).min(255.0) as u8, 255];
                        gx.fill(bx + xx, by + yy, bx + xx + 1, by + yy + 1, c);
                    }
                }
            }
        }
        x += char_w;
    }
}

pub fn paint_title(w: u32, h: u32, icons: &mut IconCache, view: &TitleView, mouse: (f32, f32)) -> Painted {
    let mut gx = Gfx::new(w, h, icons);
    let g = gx.g;
    gx.fill(0, 0, w as i32, h as i32, [0, 0, 0, 60]);
    let cx = w as i32 / 2;
    let px = (w as i32 / 70).clamp(4, 22);
    logo(&mut gx, cx, h as i32 / 7, px);
    let below = h as i32 / 7 + 8 * px + 10 * g;
    let hover = |x: i32, y: i32, bw: i32| Rect { x0: x, y0: y, x1: x + bw * g, y1: y + 20 * g }.contains(mouse.0, mouse.1);
    match view {
        TitleView::Main { last, splash } => {
            gx.centered(cx + 60 * g, h as i32 / 7 + 7 * px, splash, [255, 255, 0, 255]);
            let mut y = below + 14 * g;
            let x = cx - 100 * g;
            if let Some(name) = last {
                let label = format!("Play: {name}");
                let hv = hover(x, y, 200);
                gx.button(x, y, 200, &label, Button::Continue, hv, true);
                y += 24 * g;
            }
            let hv = hover(x, y, 200);
            gx.button(x, y, 200, "New World", Button::NewWorld, hv, true);
            y += 24 * g;
            let hv = hover(x, y, 200);
            gx.button(x, y, 200, "Load World", Button::LoadWorld, hv, last.is_some());
            y += 24 * g;
            let hv = hover(x, y, 200);
            gx.button(x, y, 200, "Quit Game", Button::QuitGame, hv, true);
            gx.text(2 * g, h as i32 - 10 * g, "RedCraft 1.0 on Red Engine 2", WHITE, true);
        }
        TitleView::NewWorld { name, seed, creative, focus, blink } => {
            let x = cx - 100 * g;
            let mut y = below;
            gx.text(x, y, "World Name", [160, 160, 160, 255], true);
            y += 10 * g;
            gx.field(x, y, 200, name, Button::NameField, *focus == Button::NameField, *blink);
            y += 30 * g;
            gx.text(x, y, "Seed (leave blank for a random world)", [160, 160, 160, 255], true);
            y += 10 * g;
            gx.field(x, y, 200, seed, Button::SeedField, *focus == Button::SeedField, *blink);
            y += 28 * g;
            let label = format!("Game Mode: {}", if *creative { "Creative" } else { "Survival" });
            let hv = hover(x, y, 200);
            gx.button(x, y, 200, &label, Button::ModeToggle, hv, true);
            y += 12 * g + 20 * g;
            let note = if *creative { "Fly, build with every block, nothing hurts" } else { "Gather, craft and survive the nights" };
            gx.centered(cx, y - 10 * g, note, [190, 190, 190, 255]);
            y += 4 * g;
            let hv = hover(x, y, 98);
            gx.button(x, y, 98, "Create World", Button::Create, hv, true);
            let hv = hover(x + 102 * g, y, 98);
            gx.button(x + 102 * g, y, 98, "Cancel", Button::Cancel, hv, true);
        }
        TitleView::Worlds { list, page } => {
            let x = cx - 110 * g;
            let mut y = below;
            gx.centered(cx, y, "Select World", WHITE);
            y += 14 * g;
            for (i, (name, line)) in list.iter().enumerate().skip(page * 6).take(6) {
                let hv = hover(x, y, 220);
                gx.button(x, y, 220, &format!("{name}  ({line})"), Button::World(i), hv, true);
                y += 24 * g;
            }
            let hv = hover(x, y, 70);
            gx.button(x, y, 70, "<", Button::PageUp, hv, *page > 0);
            let hv = hover(x + 75 * g, y, 70);
            gx.button(x + 75 * g, y, 70, "Back", Button::Cancel, hv, true);
            let hv = hover(x + 150 * g, y, 70);
            gx.button(x + 150 * g, y, 70, ">", Button::PageDown, hv, (page + 1) * 6 < list.len());
        }
    }
    gx.done()
}

pub fn paint_death(w: u32, h: u32, icons: &mut IconCache, cause: &str, mouse: (f32, f32)) -> Painted {
    let mut gx = Gfx::new(w, h, icons);
    let g = gx.g;
    gx.fill(0, 0, w as i32, h as i32, [120, 0, 0, 110]);
    let cx = w as i32 / 2;
    let y = h as i32 / 3;
    let old = gx.g;
    gx.g = old * 2;
    gx.centered(cx, y, "You died!", WHITE);
    gx.g = old;
    gx.centered(cx, y + 24 * g, cause, [230, 230, 230, 255]);
    let x = cx - 100 * g;
    let by = y + 44 * g;
    let hv = Rect { x0: x, y0: by, x1: x + 200 * g, y1: by + 20 * g }.contains(mouse.0, mouse.1);
    gx.button(x, by, 200, "Respawn", Button::Respawn, hv, true);
    let by2 = by + 24 * g;
    let hv = Rect { x0: x, y0: by2, x1: x + 200 * g, y1: by2 + 20 * g }.contains(mouse.0, mouse.1);
    gx.button(x, by2, 200, "Title Screen", Button::Title, hv, true);
    gx.done()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_finds_what_was_drawn_there() {
        let mut icons = IconCache::default();
        let inv = vec![Stack::EMPTY; 36];
        let grid = vec![Stack::EMPTY; 4];
        let p = paint_slots(1280, 720, &mut icons, &inv, &Container::Inventory { grid: &grid, out: Stack::EMPTY }, &Stack::EMPTY, (0.0, 0.0), None);
        let g = gui_scale(1280, 720);
        let (ox, oy) = ((1280 - 176 * g) / 2, (720 - 166 * g) / 2);
        // The first hotbar slot of the inventory screen is at GUI (7, 142).
        let at = |gx: i32, gy: i32| p.at((ox + gx * g + g * 9) as f32, (oy + gy * g + g * 9) as f32);
        assert_eq!(at(7, 142), Some(Hit::Slot(SlotRef::Inv(0))));
        assert_eq!(at(7 + 18 * 8, 84 + 36), Some(Hit::Slot(SlotRef::Inv(35))));
        assert_eq!(at(98, 18), Some(Hit::Slot(SlotRef::Craft(0))));
        assert_eq!(at(152, 26), Some(Hit::Slot(SlotRef::CraftOut)));
        let pause = paint_pause(
            1280,
            720,
            &mut icons,
            &PauseState { music: true, sound: true, view: 10, auto_jump: true, mode: "Survival", world: "W".into() },
            (0.0, 0.0),
        );
        assert!(pause.hits.iter().any(|(_, h)| *h == Hit::Button(Button::QuitGame)));
    }

    #[test]
    fn the_hud_paints_the_hotbar_and_hearts() {
        let mut icons = IconCache::default();
        let mut slots = vec![Stack::EMPTY; 36];
        slots[0] = Stack::new(Item::of(crate::block::Block::DIRT), 12);
        let p = paint_hud(
            1280,
            720,
            &mut icons,
            &HudState { slots: &slots, selected: 0, survival: true, health: 15.0, air: None, held_name: None, debug: None, blink: false, message: None },
        );
        let opaque = p.canvas.px.chunks(4).filter(|c| c[3] > 0).count();
        assert!(opaque > 10_000, "{opaque}");
        assert_eq!(gui_scale(1920, 1080), 3);
        assert_eq!(gui_scale(1280, 720), 2);
    }
}
