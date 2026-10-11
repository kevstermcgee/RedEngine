//! What hangs and stands in a room, what each piece is called and where it came from, where the room's key is hidden, and the worlds inside paintings.
//!
//! Everything is a function of the museum's seed, the room and the collection, so a room is the same every time it is visited and the same seed is the
//! same museum. A room's key is kept by one of its pieces: in a sculpture or installation itself (touch it), or inside a painting, or inside a painting
//! inside that painting, and so on ([`KeyPlan`]). Every painting can be entered; the world inside ([`Pocket`]) is drawn in the painting's own style and
//! may hang more paintings, up to [`MAX_DEPTH`] worlds deep.

use super::layout::{Layout, Plan, Room, RoomId, Side, HALL_W};
use super::sculpt::Family;
use super::spec::{Collection, Rgb, Theme};
use super::text::{fill, Draw};
use glam::{Vec2, Vec3};
use std::collections::{BTreeMap, HashMap};

/// How many worlds deep paintings go: a room's painting is 1, a painting inside it 2, one inside that 3 (which hangs none).
pub const MAX_DEPTH: usize = 3;

/// What kind of piece.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Painting,
    /// Three panels of one picture.
    Triptych,
    /// A woven hanging that stirs in the air.
    Tapestry,
    Sculpture,
    /// A piece hung from the ceiling.
    Installation,
}

impl Kind {
    /// The word for it on a plaque or in a hint.
    pub fn word(self) -> &'static str {
        match self {
            Kind::Painting => "painting",
            Kind::Triptych => "triptych",
            Kind::Tapestry => "tapestry",
            Kind::Sculpture => "sculpture",
            Kind::Installation => "installation",
        }
    }

    /// Whether a player can step into it.
    pub fn enterable(self) -> bool {
        matches!(self, Kind::Painting | Kind::Triptych | Kind::Tapestry)
    }

    fn media_key(self) -> &'static str {
        match self {
            Kind::Painting | Kind::Triptych => "painting",
            Kind::Tapestry => "tapestry",
            Kind::Sculpture => "sculpture",
            Kind::Installation => "installation",
        }
    }
}

/// A picture frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameStyle {
    /// Gold leaf with a bead and a deep cove.
    Gilded,
    /// Heavy carved gold with corner bosses.
    Baroque,
    /// A narrow black moulding.
    Ebony,
    /// Pale oak, square-cut.
    Oak,
    /// A wide white mat inside a thin black frame.
    Gallery,
    /// Weathered grey wood, a little crooked.
    Driftwood,
    /// No frame: the canvas stands off the wall on hidden pegs.
    Floating,
    /// Brushed copper, gone green at the corners.
    Copper,
    /// A deep box lined with dark velvet.
    Velvet,
    /// Silver with a fine double rim.
    Silver,
}

impl FrameStyle {
    pub const ALL: [FrameStyle; 10] = [
        FrameStyle::Gilded,
        FrameStyle::Baroque,
        FrameStyle::Ebony,
        FrameStyle::Oak,
        FrameStyle::Gallery,
        FrameStyle::Driftwood,
        FrameStyle::Floating,
        FrameStyle::Copper,
        FrameStyle::Velvet,
        FrameStyle::Silver,
    ];

    /// How far the frame reaches out from the canvas edge, metres.
    pub fn border(self, canvas: Vec2) -> f32 {
        let s = canvas.x.min(canvas.y);
        match self {
            FrameStyle::Gilded => 0.09 + 0.04 * s,
            FrameStyle::Baroque => 0.13 + 0.05 * s,
            FrameStyle::Ebony => 0.035,
            FrameStyle::Oak => 0.06,
            FrameStyle::Gallery => 0.16 + 0.06 * s,
            FrameStyle::Driftwood => 0.08,
            FrameStyle::Floating => 0.0,
            FrameStyle::Copper => 0.05,
            FrameStyle::Velvet => 0.12,
            FrameStyle::Silver => 0.06 + 0.02 * s,
        }
    }
}

/// The picture of a painted surface: which of the shader's styles, its seed, its palette and a few numbers the shader may use.
#[derive(Debug, Clone, PartialEq)]
pub struct Art {
    /// The style, 1-based (0 is "not painted").
    pub style: u32,
    /// Its seed (0..1000): with the style it makes the picture.
    pub seed: f32,
    /// Four colours, darkest first.
    pub palette: [Rgb; 4],
}

/// A piece's label.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plaque {
    pub title: String,
    pub artist: String,
    /// How a sentence names the artist ("the Salt Hand" for "Anonymous (the Salt Hand)").
    pub maker: String,
    pub year: String,
    pub medium: String,
    pub story: String,
}

/// One piece in a room.
#[derive(Debug, Clone)]
pub struct Exhibit {
    /// Its number in the room.
    pub index: u8,
    pub kind: Kind,
    /// The picture (paintings) or the surface finish of a sculpture (`style` is then a material, 1-based).
    pub art: Art,
    /// The shape family of a sculpture or installation.
    pub family: Option<Family>,
    pub frame: FrameStyle,
    /// A painting's canvas centre; the top of a sculpture's plinth (its base); an installation's hanging point.
    pub at: Vec3,
    /// The way it faces (horizontal unit vector, toward the viewer).
    pub facing: Vec3,
    /// A painting's canvas size; a sculpture's footprint radius and height.
    pub size: Vec2,
    /// A sculpture's plinth height (0 for none).
    pub plinth: f32,
    pub plaque: Plaque,
    /// The theme it speaks for.
    pub theme: usize,
    /// One of the collection's hand-written pieces.
    pub masterpiece: bool,
}

impl Exhibit {
    /// The point a viewer looks at.
    pub fn focus(&self) -> Vec3 {
        match self.kind {
            Kind::Sculpture => self.at + Vec3::Y * self.size.y * 0.5,
            Kind::Installation => self.at - Vec3::Y * self.size.y * 0.5,
            _ => self.at,
        }
    }

    /// The viewer's right along a painting's face.
    pub fn right(&self) -> Vec3 {
        (-self.facing).cross(Vec3::Y).normalize_or(Vec3::X)
    }
}

/// Where a room's key is: in exhibit `exhibit`, `depth` worlds in (0: in the piece itself), reached through the nested paintings `route`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPlan {
    pub exhibit: u8,
    pub depth: u8,
    /// For each world on the way down after the first, which of its paintings leads on (`depth - 1` entries).
    pub route: Vec<u8>,
}

/// A bench: centre, yaw (the way a sitter faces), length.
#[derive(Debug, Clone, Copy)]
pub struct Bench {
    pub at: Vec3,
    pub facing: Vec3,
    pub length: f32,
}

/// The curator's note beside a doorway: where it hangs and what it says.
#[derive(Debug, Clone)]
pub struct Note {
    pub at: Vec3,
    pub facing: Vec3,
    pub text: String,
}

/// A furnished room.
#[derive(Debug, Clone)]
pub struct Contents {
    pub room: Room,
    pub exhibits: Vec<Exhibit>,
    pub key: KeyPlan,
    pub benches: Vec<Bench>,
    pub note: Note,
}

/// Which room each hand-written masterpiece hangs in (each in exactly one).
#[derive(Debug, Clone, Default)]
pub struct Homes(pub HashMap<RoomId, usize>);

impl Homes {
    /// Places each masterpiece in the first room of its theme, spiralling out from a gallery number of its own, that holds no other.
    pub fn place(layout: &Layout, c: &Collection) -> Homes {
        let mut homes = HashMap::new();
        for (i, m) in c.masterpieces.iter().enumerate() {
            let mut d = Draw::new(&[layout.seed as i64, i as i64, 77]);
            let start = 2 + d.below(6) as u64 + 5 * i as u64;
            for n in start..start + 600 {
                let r = RoomId::from_number(n);
                if !homes.contains_key(&r) && layout.room(r).theme == m.theme {
                    homes.insert(r, i);
                    break;
                }
            }
        }
        Homes(homes)
    }
}

fn jitter_palette(p: &[Rgb; 4], d: &mut Draw) -> [Rgb; 4] {
    // Each colour leans a little warmer or cooler and lighter or darker, so two paintings from one palette are cousins, not twins.
    let warm = d.range(-0.06, 0.06);
    let mut out = *p;
    for c in out.iter_mut() {
        let k = d.range(0.86, 1.14);
        *c = [(c[0] * k * (1.0 + warm)).clamp(0.0, 1.0), (c[1] * k).clamp(0.0, 1.0), (c[2] * k * (1.0 - warm)).clamp(0.0, 1.0)];
    }
    out
}

/// A fresh picture in the theme's manner (now and then any style).
pub fn draw_art(c: &Collection, theme: &Theme, d: &mut Draw) -> Art {
    let style = if d.chance(0.12) || theme.styles.is_empty() { d.below(c.styles.len()) } else { *d.pick(&theme.styles) };
    let palette = jitter_palette(d.pick(&theme.palettes), d);
    Art { style: style as u32 + 1, seed: d.range(0.0, 1000.0), palette }
}

/// A year as a label says it, how a story says it ("in 1934", "around 1890"), and a later moment for a story to happen in.
fn year(d: &mut Draw) -> (String, String, String) {
    let y = match d.weighted(&[1.0, 2.5, 5.0, 3.0]) {
        0 => d.range(1460.0, 1700.0),
        1 => d.range(1700.0, 1900.0),
        2 => d.range(1900.0, 2000.0),
        _ => d.range(2000.0, 2024.0),
    } as i32;
    let (label, when) = match d.weighted(&[6.0, 2.0, 1.5, 0.6]) {
        0 => (y.to_string(), format!("in {y}")),
        1 => {
            let r = y - y % 5;
            (format!("c. {r}"), format!("around {r}"))
        }
        2 => {
            let to = (y + 1 + d.below(9) as i32).min(2026);
            (format!("{y}\u{2013}{to}"), format!("between {y} and {to}"))
        }
        _ => ("undated".to_string(), "in a year nobody wrote down".to_string()),
    };
    let later = if y < 2023 && d.chance(0.8) {
        format!("in {}", (y + 1 + d.below((2026 - y).max(1) as usize) as i32).min(2026))
    } else {
        (*d.pick(&["one winter not long ago", "some years later", "a long time afterwards", "only last spring"])).to_string()
    };
    (label, when, later)
}

/// A title as a sentence quotes it.
pub fn quoted(title: &str) -> String {
    format!("\u{201c}{title}\u{201d}")
}

/// The templates a room has used, so its pieces do not tell the same story twice (cleared when every one has been told).
#[derive(Debug, Clone, Default)]
pub struct Deck {
    used: std::collections::HashSet<(u8, usize)>,
}

impl Deck {
    /// One of `n` templates of list `list` not yet used here.
    fn deal(&mut self, list: u8, n: usize, d: &mut Draw) -> usize {
        if n == 0 {
            return 0;
        }
        let free: Vec<usize> = (0..n).filter(|i| !self.used.contains(&(list, *i))).collect();
        let i = if free.is_empty() {
            self.used.retain(|(l, _)| *l != list);
            d.below(n)
        } else {
            *d.pick(&free)
        };
        self.used.insert((list, i));
        i
    }
}

/// The artists who work in a wing: a handful, so names come back as one walks its rooms. Returns the label and how a story names them.
fn wing_artist(layout: &Layout, c: &Collection, theme: &Theme, room: RoomId, d: &mut Draw) -> (String, String) {
    let (wx, wz) = layout.wing_of(room);
    // Mostly the wing's own residents; sometimes someone from anywhere.
    let k = if d.chance(0.7) { d.below(6) as i64 } else { 1000 + d.below(100_000) as i64 };
    let mut ad = Draw::new(&[layout.seed as i64, wx as i64, wz as i64, k, 31]);
    let tpl = ad.pick(&c.artists).clone();
    let (label, prose) = match tpl.split_once(" // ") {
        Some((a, b)) => (a.to_string(), b.to_string()),
        None => (tpl.clone(), tpl.clone()),
    };
    // The two halves draw the same words in the same order from twin generators.
    let mut twin = ad.clone();
    let label = fill(&label, theme, c, &BTreeMap::new(), &mut ad);
    let prose = fill(&prose, theme, c, &BTreeMap::new(), &mut twin);
    // A story's sentence may begin with the name: keep a lower-case "the" inside sentences.
    (label, prose)
}

/// Writes a piece's plaque; `deck` keeps a room from repeating itself.
pub fn write_plaque(layout: &Layout, c: &Collection, theme_ix: usize, kind: Kind, room: RoomId, d: &mut Draw, deck: &mut Deck) -> Plaque {
    let theme = &c.themes[theme_ix];
    let (year, when, later) = year(d);
    let mut fixed: BTreeMap<&str, String> = BTreeMap::new();
    fixed.insert("year", year.clone());
    fixed.insert("when", when);
    fixed.insert("later", later);
    fixed.insert("kind", kind.word().to_string());
    fixed.insert("theme", theme.name.to_lowercase());
    fixed.insert("wing", theme.wing.clone());
    fixed.insert("n", (2 + d.below(30)).to_string());
    fixed.insert("room", room.number().to_string());
    // Titles with no slots are rarer, and come in numbered series (a museum has more than one "Don't").
    let mut ti = deck.deal(0, theme.titles.len(), d);
    if !theme.titles[ti].contains('{') && d.chance(0.6) {
        ti = deck.deal(0, theme.titles.len(), d);
    }
    let mut title = fill(&theme.titles[ti], theme, c, &fixed, d);
    if !theme.titles[ti].contains('{') && d.chance(0.7) {
        title = format!("{title} ({})", ["II", "III", "IV", "V", "VI", "VII", "IX", "XII"][d.below(8)]);
    }
    let (artist, maker) = wing_artist(layout, c, theme, room, d);
    let medium = c.media.get(kind.media_key()).filter(|m| !m.is_empty()).map(|m| fill(d.pick(m), theme, c, &fixed, d)).unwrap_or_default();
    fixed.insert("title", quoted(&title));
    fixed.insert("artist", maker.clone());
    fixed.insert("medium", medium.to_lowercase());
    let story = if matches!(kind, Kind::Sculpture | Kind::Installation | Kind::Tapestry) && !c.object_stories.is_empty() {
        let i = deck.deal(2, c.object_stories.len(), d);
        fill(&c.object_stories[i], theme, c, &fixed, d)
    } else {
        let n = theme.stories.len() + c.stories.len();
        let i = deck.deal(1, n, d);
        let tpl = if i < theme.stories.len() { &theme.stories[i] } else { &c.stories[i - theme.stories.len()] };
        fill(tpl, theme, c, &fixed, d)
    };
    Plaque { title, artist, maker, year, medium, story }
}

/// A painting's shape: width over height.
fn aspect(d: &mut Draw) -> f32 {
    [0.75, 1.333, 1.0, 1.8, 0.6, 1.5, 0.8][d.weighted(&[3.0, 3.0, 1.6, 1.0, 0.8, 1.2, 1.4])]
}

/// Where a piece hangs on a wall: the wall's point, the way it faces, and the room's height (for the hanging height).
fn hang(base: Vec3, facing: Vec3, size: Vec2, room_h: f32) -> Vec3 {
    let mut y = 1.65f32.max(0.75 + size.y * 0.5);
    y = y.min(room_h - 0.5 - size.y * 0.5).max(size.y * 0.5 + 0.35);
    base + facing * 0.06 + Vec3::Y * y
}

/// Furnishes a room: its pieces, their plaques, the key's hiding place, benches and the curator's note.
pub fn furnish(layout: &Layout, c: &Collection, homes: &Homes, room: &Room) -> Contents {
    let id = room.id;
    let mut d = Draw::new(&[layout.seed as i64, id.x as i64, id.z as i64, 4242]);
    let theme_ix = room.theme;
    let theme = &c.themes[theme_ix];
    let mut exhibits: Vec<Exhibit> = Vec::new();
    let mut benches = Vec::new();
    let h = room.height;
    let corner_inset = if room.plan == Plan::Pillars { 2.4 } else { 0.7 };
    // The curator's note hangs beside the first doorway (on the viewer's right as one walks in).
    let note_side = Side::ALL.into_iter().find(|s| room.has_door(*s)).unwrap_or(Side::North);

    let deck = std::cell::RefCell::new(Deck::default());
    let push =
        |exhibits: &mut Vec<Exhibit>, d: &mut Draw, kind: Kind, at: Vec3, facing: Vec3, size: Vec2, frame: FrameStyle, plinth: f32, family: Option<Family>| {
            let index = exhibits.len() as u8;
            let mut pd = Draw::new(&[layout.seed as i64, id.x as i64, id.z as i64, 500 + index as i64]);
            let mut art = draw_art(c, theme, &mut pd);
            if family.is_some() {
                art.style = 1 + pd.below(c.materials.len().max(1)) as u32;
            }
            let plaque = write_plaque(layout, c, theme_ix, kind, id, &mut pd, &mut deck.borrow_mut());
            // A sculpture is made of what its label says, when the label names one of the collection's materials.
            if family.is_some() {
                let medium = plaque.medium.to_lowercase();
                if let Some(i) = c.materials.iter().position(|m| medium.contains(&m.to_lowercase())) {
                    art.style = i as u32 + 1;
                }
            }
            let _ = d;
            exhibits.push(Exhibit { index, kind, art, family, frame, at, facing, size, plinth, plaque, theme: theme_ix, masterpiece: false });
        };

    // ---- Walls ----
    for side in Side::ALL {
        let (p0, inward) = room.wall_face(side);
        let len = room.wall_length(side);
        let right = (-inward).cross(Vec3::Y).normalize();
        let half = len * 0.5 - corner_inset;
        let gap = HALL_W * 0.5 + 0.7;
        let segments: Vec<(f32, f32)> = if room.has_door(side) {
            // Beside the note the segment starts farther from the doorway.
            let note_gap = if side == note_side { gap + 0.9 } else { gap };
            vec![(-half, -gap), (note_gap, half)]
        } else {
            vec![(-half, half)]
        };
        let door_wall = room.has_door(side);
        for (a, b) in segments {
            let s = b - a;
            if s < 1.5 {
                continue;
            }
            let mid = (a + b) * 0.5;
            if door_wall {
                let asp = aspect(&mut d);
                let w = (s - 0.4).min(d.range(1.5, 3.2));
                let size = Vec2::new(w, (w / asp).min(h - 2.0).min(3.4));
                let frame = *d.pick(&FrameStyle::ALL);
                push(&mut exhibits, &mut d, Kind::Painting, hang(p0 + right * mid, inward, size, h), inward, size, frame, 0.0, None);
                continue;
            }
            // A whole wall: one grand piece, a pair, three, a triptych or a tapestry.
            let choice = d.weighted(&[3.0, 2.5, if s >= 10.0 { 1.6 } else { 0.0 }, if s >= 7.0 { 1.4 } else { 0.0 }, if h >= 6.0 { 1.2 } else { 0.0 }]);
            match choice {
                0 => {
                    let asp = aspect(&mut d).max(0.75);
                    let w = d.range(3.2, (s - 1.0).clamp(3.3, 7.0));
                    let size = Vec2::new(w, (w / asp).min(h - 2.0).clamp(1.6, 4.6));
                    let frame =
                        *d.pick(&[FrameStyle::Gilded, FrameStyle::Baroque, FrameStyle::Gallery, FrameStyle::Floating, FrameStyle::Silver, FrameStyle::Velvet]);
                    let at = hang(p0 + right * mid, inward, size, h);
                    push(&mut exhibits, &mut d, Kind::Painting, at, inward, size, frame, 0.0, None);
                    benches.push(Bench { at: p0 + right * mid + inward * 3.4, facing: -inward, length: 1.9 });
                }
                1 | 2 => {
                    let n = if choice == 1 { 2 } else { 3 };
                    let slot = s / n as f32;
                    for k in 0..n {
                        let u = a + slot * (k as f32 + 0.5);
                        let asp = aspect(&mut d);
                        let w = (slot - 0.8).min(d.range(1.6, 3.4)).max(1.0);
                        let size = Vec2::new(w, (w / asp).min(h - 2.0).min(3.4));
                        let frame = *d.pick(&FrameStyle::ALL);
                        push(&mut exhibits, &mut d, Kind::Painting, hang(p0 + right * u, inward, size, h), inward, size, frame, 0.0, None);
                    }
                }
                3 => {
                    let w = d.range(4.5, (s - 1.0).clamp(4.6, 7.5));
                    let size = Vec2::new(w, (w * d.range(0.36, 0.5)).min(h - 2.4));
                    let frame = *d.pick(&[FrameStyle::Gilded, FrameStyle::Baroque, FrameStyle::Ebony, FrameStyle::Silver, FrameStyle::Oak]);
                    push(&mut exhibits, &mut d, Kind::Triptych, hang(p0 + right * mid, inward, size, h), inward, size, frame, 0.0, None);
                }
                _ => {
                    let w = d.range(1.8, (s - 1.0).clamp(1.9, 3.6));
                    let size = Vec2::new(w, (w * d.range(1.2, 1.7)).min(h - 1.6));
                    let at = p0 + inward * 0.12 + Vec3::Y * (h - 0.6 - size.y * 0.5).min(0.9 + size.y * 0.5).max(size.y * 0.5 + 0.5) + right * mid;
                    push(&mut exhibits, &mut d, Kind::Tapestry, at, inward, size, FrameStyle::Floating, 0.0, None);
                }
            }
        }
    }

    // ---- The partition's two faces ----
    if let Plan::Partition { along_x } = room.plan {
        let long = if along_x { room.size.x } else { room.size.y };
        let len = partition_length(long);
        let normal = if along_x { Vec3::Z } else { Vec3::X };
        for face in [normal, -normal] {
            let base = id.centre() + face * (PARTITION_T * 0.5);
            let asp = aspect(&mut d).max(0.9);
            let w = (len - 1.4).min(d.range(1.6, 3.4));
            let size = Vec2::new(w, (w / asp).min(partition_height(h) - 1.2));
            let frame = *d.pick(&FrameStyle::ALL);
            let mut at = hang(base, face, size, partition_height(h) + 0.6);
            at.y = at.y.min(partition_height(h) - 0.35 - size.y * 0.5);
            push(&mut exhibits, &mut d, Kind::Painting, at, face, size, frame, 0.0, None);
        }
    }

    // ---- The floor: sculptures in the quarters, maybe one in the middle, maybe something hanging ----
    let q = Vec2::new(room.size.x * 0.25, room.size.y * 0.25);
    let mut quarters = [Vec2::new(-q.x, -q.y), Vec2::new(q.x, -q.y), Vec2::new(-q.x, q.y), Vec2::new(q.x, q.y)];
    // Shuffle the quarters.
    for i in (1..4).rev() {
        let j = d.below(i + 1);
        quarters.swap(i, j);
    }
    let n_sculpt = 1 + d.weighted(&[3.0, 3.0, 1.5]);
    let families = if theme.sculptures.is_empty() { Family::ALL.to_vec() } else { theme.sculptures.clone() };
    let standing: Vec<Family> = families.iter().copied().filter(|f| !f.hangs()).collect();
    let hanging: Vec<Family> = families.iter().copied().filter(|f| f.hangs()).collect();
    let standing = if standing.is_empty() { vec![Family::Vessel, Family::Figure, Family::Monolith] } else { standing };
    for qp in quarters.iter().take(n_sculpt) {
        let fam = *d.pick(&standing);
        let at2 = id.centre() + Vec3::new(qp.x, 0.0, qp.y);
        // It faces the middle of the room.
        let facing = (id.centre() - at2).normalize_or(Vec3::Z);
        let (radius, height, plinth) = fam.proportions(&mut d);
        push(&mut exhibits, &mut d, Kind::Sculpture, at2 + Vec3::Y * plinth, facing, Vec2::new(radius, height), FrameStyle::Floating, plinth, Some(fam));
    }
    let centre_free = room.plan != Plan::Partition { along_x: true } && room.plan != Plan::Partition { along_x: false };
    if centre_free {
        if !hanging.is_empty() && h >= 6.0 && d.chance(0.55) {
            let fam = *d.pick(&hanging);
            let (radius, height, _) = fam.proportions(&mut d);
            let height = height.min(h - 3.0);
            push(
                &mut exhibits,
                &mut d,
                Kind::Installation,
                id.centre() + Vec3::Y * (h - 0.2),
                Vec3::Z,
                Vec2::new(radius, height),
                FrameStyle::Floating,
                0.0,
                Some(fam),
            );
        } else if d.chance(0.5) {
            let fam = *d.pick(&standing);
            let (radius, height, plinth) = fam.proportions(&mut d);
            let (radius, height) = (radius * 1.3, height * 1.25);
            push(
                &mut exhibits,
                &mut d,
                Kind::Sculpture,
                id.centre() + Vec3::Y * plinth,
                Vec3::Z,
                Vec2::new(radius, height),
                FrameStyle::Floating,
                plinth,
                Some(fam),
            );
        }
    }

    // ---- A hand-written masterpiece takes the grandest painting's place ----
    if let Some(&mi) = homes.0.get(&id) {
        let m = &c.masterpieces[mi];
        let best = exhibits
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind == Kind::Painting)
            .max_by(|a, b| (a.1.size.x * a.1.size.y).total_cmp(&(b.1.size.x * b.1.size.y)))
            .map(|(i, _)| i);
        if let Some(i) = best {
            let e = &mut exhibits[i];
            e.masterpiece = true;
            e.art.style = m.style as u32 + 1;
            e.theme = m.theme;
            e.frame = FrameStyle::Baroque;
            e.plaque = Plaque {
                title: m.title.clone(),
                artist: m.artist.clone(),
                maker: m.artist.clone(),
                year: m.year.clone(),
                medium: m.medium.clone(),
                story: m.story.clone(),
            };
        }
    }

    // ---- The key ----
    let key = plan_key(id, &exhibits, &mut d);
    let keeper = &exhibits[key.exhibit as usize];
    let mut fixed: BTreeMap<&str, String> = BTreeMap::new();
    fixed.insert("title", quoted(&keeper.plaque.title));
    fixed.insert("artist", keeper.plaque.maker.clone());
    fixed.insert("kind", keeper.kind.word().to_string());
    fixed.insert("theme", theme.name.to_lowercase());
    fixed.insert("wing", theme.wing.clone());
    fixed.insert("room", id.number().to_string());
    let hint = fill(d.pick(&theme.hints), theme, c, &fixed, &mut d);
    let (p0, inward) = room.wall_face(note_side);
    let right = (-inward).cross(Vec3::Y).normalize();
    let note = Note { at: p0 + right * (HALL_W * 0.5 + 0.75) + inward * 0.04 + Vec3::Y * 1.5, facing: inward, text: hint };

    Contents { room: room.clone(), exhibits, key, benches, note }
}

/// The free-standing wall across a room: its length for a room `long` metres along it.
pub fn partition_length(long: f32) -> f32 {
    (long * 0.42).max(6.0)
}

/// Its height in a room `h` high.
pub fn partition_height(h: f32) -> f32 {
    (h - 1.6).min(4.0)
}

/// Its thickness.
pub const PARTITION_T: f32 = 0.4;

/// Where a room hides its key. The first room keeps it one painting deep, so the first thing a player learns is that paintings open.
fn plan_key(id: RoomId, exhibits: &[Exhibit], d: &mut Draw) -> KeyPlan {
    let paintings: Vec<u8> = exhibits.iter().filter(|e| e.kind.enterable()).map(|e| e.index).collect();
    let solids: Vec<u8> = exhibits.iter().filter(|e| !e.kind.enterable()).map(|e| e.index).collect();
    let mut depth = if id == RoomId::ORIGIN { 1 } else { d.weighted(&[2.0, 4.0, 3.0, 1.2]) as u8 };
    if depth == 0 && solids.is_empty() {
        depth = 1;
    }
    if depth > 0 && paintings.is_empty() {
        depth = 0;
    }
    let exhibit = if depth == 0 { *d.pick(&solids) } else { *d.pick(&paintings) };
    let route = (1..depth).map(|_| d.below(3) as u8).collect();
    KeyPlan { exhibit, depth, route }
}

/// A world inside a painting, named by the way in: the room, the painting, and which painting inside each world after that.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PocketPath {
    pub room: RoomId,
    pub exhibit: u8,
    pub nest: Vec<u8>,
}

impl PocketPath {
    /// How many worlds deep (a room's painting is 1).
    pub fn depth(&self) -> usize {
        self.nest.len() + 1
    }

    /// The world inside painting `i` of this one.
    pub fn child(&self, i: u8) -> PocketPath {
        let mut nest = self.nest.clone();
        nest.push(i);
        PocketPath { room: self.room, exhibit: self.exhibit, nest }
    }

    /// The world this one hangs in (`None`: the museum).
    pub fn parent(&self) -> Option<PocketPath> {
        if self.nest.is_empty() {
            None
        } else {
            let mut nest = self.nest.clone();
            nest.pop();
            Some(PocketPath { room: self.room, exhibit: self.exhibit, nest })
        }
    }

    fn keys(&self, seed: u32, what: i64) -> Vec<i64> {
        let mut k = vec![seed as i64, self.room.x as i64, self.room.z as i64, self.exhibit as i64, what];
        k.extend(self.nest.iter().map(|n| *n as i64 + 1));
        k
    }
}

/// How a painting inside a world is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stand {
    /// On a wooden easel.
    Easel,
    /// Hanging in the air, turning a little.
    Floating,
    /// On a slim plinth of stone.
    Pillar,
}

/// A painting inside a world.
#[derive(Debug, Clone)]
pub struct Nested {
    pub index: u8,
    pub art: Art,
    pub frame: FrameStyle,
    /// The canvas centre (world space, the pocket's own origin).
    pub at: Vec3,
    pub facing: Vec3,
    pub size: Vec2,
    pub stand: Stand,
    pub plaque: Plaque,
    /// It is on the way to the room's key (its frame keeps a faint golden shimmer).
    pub on_route: bool,
}

/// What the ground of a world inside a painting is like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ground {
    /// A still mirror of water.
    Mirror,
    /// The painting's own colours underfoot.
    Painted,
    /// Pale sand.
    Dune,
    /// Floating stepping stones over nothing (the walkable disc is invisible between them).
    Stones,
}

/// A world inside a painting.
#[derive(Debug, Clone)]
pub struct Pocket {
    pub path: PocketPath,
    /// The picture the world is made of (the painting stepped into).
    pub art: Art,
    pub theme: usize,
    /// The painting's title (shown on arrival).
    pub title: String,
    pub frames: Vec<Nested>,
    /// Where the key floats, when it is here.
    pub key: Option<Vec3>,
    /// How far one may walk from the centre.
    pub radius: f32,
    pub spawn: Vec3,
    /// The way the player faces on arrival.
    pub spawn_facing: Vec3,
    /// The frame that leads back out, behind the spawn.
    pub exit: Vec3,
    pub ground: Ground,
}

/// The world behind a room's painting `path.exhibit`, or behind a painting inside such a world. `None` when the path leads nowhere.
pub fn pocket(layout: &Layout, c: &Collection, contents: &Contents, path: &PocketPath) -> Option<Pocket> {
    let e = contents.exhibits.get(path.exhibit as usize).filter(|e| e.kind.enterable())?;
    if path.depth() > MAX_DEPTH {
        return None;
    }
    // Walk down: the world behind the room's painting, then each nested painting's.
    let mut art = e.art.clone();
    let mut title = e.plaque.title.clone();
    let mut here = PocketPath { room: path.room, exhibit: path.exhibit, nest: Vec::new() };
    for &i in &path.nest {
        let p = build_pocket(layout, c, contents, &here, art.clone(), title.clone(), e.theme);
        let f = p.frames.get(i as usize)?;
        art = f.art.clone();
        title = f.plaque.title.clone();
        here = here.child(i);
    }
    Some(build_pocket(layout, c, contents, &here, art, title, e.theme))
}

fn build_pocket(layout: &Layout, c: &Collection, contents: &Contents, path: &PocketPath, art: Art, title: String, theme_ix: usize) -> Pocket {
    let mut d = Draw::new(&path.keys(layout.seed, 9001));
    let key = &contents.key;
    let on_key_line = key.exhibit == path.exhibit && key.depth as usize >= path.depth() && key.route.starts_with(&path.nest);
    let key_here = on_key_line && key.depth as usize == path.depth();
    let leads_on = if on_key_line && !key_here { Some(key.route[path.nest.len()]) } else { None };
    let radius = d.range(14.0, 20.0);
    let ground = [Ground::Mirror, Ground::Painted, Ground::Dune, Ground::Stones][d.weighted(&[3.0, 3.0, 1.5, 1.2])];
    let mut count = if path.depth() < MAX_DEPTH { d.weighted(&[1.0, 3.0, 3.0, 2.0]) } else { 0 };
    if let Some(i) = leads_on {
        count = count.max(i as usize + 1);
    }
    let theme = &c.themes[theme_ix];
    let spawn = Vec3::new(0.0, 0.0, radius * 0.55);
    let mut frames = Vec::new();
    // The paintings stand in a loose arc ahead, facing the arrival point.
    let spread = 1.9 / count.max(1) as f32;
    let mut deck = Deck::default();
    for i in 0..count {
        let mut fd = Draw::new(&path.child(i as u8).keys(layout.seed, 9100));
        let a = (i as f32 - (count as f32 - 1.0) * 0.5) * spread + fd.range(-0.12, 0.12);
        let dist = fd.range(radius * 0.35, radius * 0.7);
        let base = Vec3::new(a.sin() * dist, 0.0, -a.cos() * dist + radius * 0.05);
        let facing = Vec3::new(spawn.x - base.x, 0.0, spawn.z - base.z).normalize_or(Vec3::Z);
        let stand = [Stand::Easel, Stand::Floating, Stand::Pillar][fd.weighted(&[2.0, 2.0, 1.4])];
        let asp = aspect(&mut fd);
        let w = fd.range(1.4, 2.4);
        let size = Vec2::new(w, (w / asp).min(2.2));
        let lift = match stand {
            Stand::Easel => 0.55 + size.y * 0.5,
            Stand::Floating => 1.7 + fd.range(0.0, 0.8),
            Stand::Pillar => 1.15 + size.y * 0.5,
        };
        let child_art = if fd.chance(0.7) { draw_art(c, theme, &mut fd) } else { draw_art(c, &c.themes[fd.below(c.themes.len())], &mut fd) };
        let plaque = write_plaque(layout, c, theme_ix, Kind::Painting, path.room, &mut fd, &mut deck);
        frames.push(Nested {
            index: i as u8,
            art: child_art,
            frame: *fd.pick(&FrameStyle::ALL),
            at: base + Vec3::Y * lift,
            facing,
            size,
            stand,
            plaque,
            on_route: leads_on == Some(i as u8),
        });
    }
    // The key floats in the open: well clear of every painting's stand and of the way back, so it can always be walked into.
    let key = key_here.then(|| {
        let mut best = Vec3::new(0.0, 1.25, -radius * 0.6);
        let mut best_gap = -1.0f32;
        for _ in 0..24 {
            let a = d.range(-0.9, 0.9);
            let r = d.range(radius * 0.35, radius * 0.75);
            let at = Vec3::new(a.sin() * r, 1.25, -a.cos() * r);
            let gap = frames.iter().map(|f: &Nested| Vec2::new(f.at.x - at.x, f.at.z - at.z).length()).fold(f32::MAX, f32::min);
            if gap > best_gap {
                best = at;
                best_gap = gap;
            }
            if gap > 3.0 {
                break;
            }
        }
        best
    });
    Pocket { path: path.clone(), art, theme: theme_ix, title, frames, key, radius, spawn, spawn_facing: Vec3::NEG_Z, exit: spawn + Vec3::Z * 3.2, ground }
}

#[cfg(test)]
mod tests {
    use super::super::spec::parse_collection;
    use super::*;

    fn collection() -> Collection {
        let v: serde_json::Value = serde_json::json!({
            "styles": ["ink", "field", "tide"],
            "materials": ["marble", "bronze"],
            "words": {"first": ["Ada", "Ines", "Tomas"], "last": ["Moreau", "Kalnins", "Okafor"], "epithet": ["the Salt Hand"], "noun": ["lamp", "tide", "letter"], "city": ["Riga"]},
            "artists": ["{first} {last}", "Anonymous ({epithet}) // {epithet}"],
            "object_stories": ["{artist} made it, {when}."],
            "media": {"painting": ["oil on linen"], "tapestry": ["wool"], "sculpture": ["bronze"], "installation": ["glass and wire"]},
            "themes": [
                {"id": "longing", "name": "Longing", "wing": "The Tidewater Wing",
                 "colors": {"wall": "#33445a", "floor": "#2a2420", "trim": "#c8a060", "ceiling": "#202830", "light": "#ffd8a8"},
                 "palettes": [["#101820", "#305070", "#80a0c0", "#f0e0c0"]], "styles": ["ink", "tide"], "sculptures": ["vessel", "figure", "mobile"],
                 "titles": ["The {noun}"], "stories": ["{artist} made it in {city}."], "hints": ["Ask {title}."]},
                {"id": "joy", "name": "Joy", "wing": "The Sunlit Wing",
                 "colors": {"wall": "#f0e0c0", "floor": "#806040", "trim": "#c8a060", "ceiling": "#fff8f0", "light": "#fff0d0"},
                 "palettes": [["#402010", "#e06030", "#f0c040", "#fff8e0"]], "styles": ["field"], "sculptures": ["bloom", "orbit"],
                 "titles": ["A {noun} for you"], "stories": ["Made in {city} by {artist}, {year}."], "hints": ["{title} knows."]}
            ],
            "masterpieces": [{"title": "The Last Letter", "artist": "Ines Moreau", "year": "1911", "medium": "oil", "story": "Kept.", "theme": "joy", "style": "field"}]
        });
        parse_collection(&v, None).expect("collection")
    }

    #[test]
    fn rooms_are_furnished_the_same_way_every_time_and_nothing_stands_in_a_doorway() {
        let c = collection();
        let l = Layout::new(11, c.themes.len(), None);
        let homes = Homes::place(&l, &c);
        for x in -4..4 {
            for z in -4..4 {
                let room = l.room(RoomId::new(x, z));
                let a = furnish(&l, &c, &homes, &room);
                let b = furnish(&l, &c, &homes, &room);
                assert_eq!(a.exhibits.len(), b.exhibits.len());
                assert_eq!(a.key, b.key);
                assert!(a.exhibits.len() >= 3, "({x},{z}) has only {} pieces", a.exhibits.len());
                for e in &a.exhibits {
                    assert_eq!(e.plaque, b.exhibits[e.index as usize].plaque);
                    assert!(!e.plaque.title.is_empty() && !e.plaque.artist.is_empty() && !e.plaque.story.is_empty());
                    // Nothing on the floor within a metre and a half of the line from a door to the middle.
                    if e.kind == Kind::Sculpture {
                        let d = (e.at - room.id.centre()) * Vec3::new(1.0, 0.0, 1.0);
                        for s in Side::ALL.into_iter().filter(|s| room.has_door(*s)) {
                            let off = if s.runs_along_x() { d.x.abs() } else { d.z.abs() };
                            assert!(off > 1.5 + e.size.x * 0.5 || d.length() < 0.1, "({x},{z}) a sculpture blocks the {s:?} door");
                        }
                    }
                }
                let keeper = &a.exhibits[a.key.exhibit as usize];
                assert_eq!(keeper.kind.enterable(), a.key.depth > 0);
                assert_eq!(a.key.route.len(), (a.key.depth as usize).saturating_sub(1));
            }
        }
        let first = furnish(&l, &c, &homes, &l.room(RoomId::ORIGIN));
        assert_eq!(first.key.depth, 1, "the first room teaches stepping into a painting");
    }

    #[test]
    fn the_key_is_at_the_end_of_its_route_and_nowhere_else() {
        let c = collection();
        let l = Layout::new(5, c.themes.len(), None);
        let homes = Homes::place(&l, &c);
        let mut deep = 0;
        for x in -5..5 {
            for z in -5..5 {
                let room = l.room(RoomId::new(x, z));
                let k = furnish(&l, &c, &homes, &room);
                if k.key.depth == 0 {
                    continue;
                }
                let mut path = PocketPath { room: room.id, exhibit: k.key.exhibit, nest: Vec::new() };
                for (lvl, step) in k.key.route.iter().enumerate() {
                    let p = pocket(&l, &c, &k, &path).expect("a world on the route");
                    assert!(p.key.is_none(), "level {lvl}: the key is deeper");
                    assert!(p.frames.iter().any(|f| f.index == *step && f.on_route));
                    path = path.child(*step);
                }
                let p = pocket(&l, &c, &k, &path).expect("the key's world");
                assert!(p.key.is_some(), "({x},{z}): the key's world has the key");
                let key = p.key.unwrap();
                for f in &p.frames {
                    assert!(Vec2::new(f.at.x - key.x, f.at.z - key.z).length() > 1.8, "({x},{z}): the key floats clear of every painting's stand");
                }
                deep = deep.max(k.key.depth);
                // Every other painting's world holds no key.
                for e in k.exhibits.iter().filter(|e| e.kind.enterable() && e.index != k.key.exhibit) {
                    let p = pocket(&l, &c, &k, &PocketPath { room: room.id, exhibit: e.index, nest: Vec::new() }).unwrap();
                    assert!(p.key.is_none());
                }
            }
        }
        assert!(deep >= 2, "some keys are paintings within paintings");
    }

    #[test]
    fn each_masterpiece_hangs_once_in_a_room_of_its_theme() {
        let c = collection();
        let l = Layout::new(8, c.themes.len(), None);
        let homes = Homes::place(&l, &c);
        assert_eq!(homes.0.len(), 1);
        let (room, _) = homes.0.iter().next().unwrap();
        let k = furnish(&l, &c, &homes, &l.room(*room));
        assert_eq!(k.exhibits.iter().filter(|e| e.masterpiece).count(), 1);
        assert!(k.exhibits.iter().any(|e| e.plaque.title == "The Last Letter"));
    }
}
