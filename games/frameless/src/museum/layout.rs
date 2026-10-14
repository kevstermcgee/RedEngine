//! The floor plan of an endless museum: a grid of rooms, the doors between them and the wings they belong to, all pure functions of a seed.
//!
//! Room `(x, z)` stands in the cell centred on `(x * CELL, 0, z * CELL)` (north is -z). Neighbouring rooms are joined, where there is a door, by a short
//! hall that runs from one room's wall to the other's with the door itself at the cell border. Every room has at least one door: each room's "parent"
//! door (to the north or to the east, drawn from the seed) always exists, which ties the whole infinite grid together (a binary-tree maze, in which any two
//! rooms are connected), and about a third of the other walls get a door too, so the plan has loops to get lost in. The first room has all four.
//!
//! Rooms come in wings of about three by three that share a theme (an emotion); now and then a room borrows a neighbouring theme.

use super::text::Draw;
use glam::{Vec2, Vec3};

/// Metres between the centres of neighbouring rooms.
pub const CELL: f32 = 26.0;
/// Wall thickness.
pub const WALL: f32 = 0.5;
/// Inside width of the hall between two rooms.
pub const HALL_W: f32 = 3.6;
/// Inside height of the hall.
pub const HALL_H: f32 = 4.4;
/// Width of the doorway at the cell border (where the door hangs).
pub const DOOR_W: f32 = 2.6;
/// Height of that doorway.
pub const DOOR_H: f32 = 3.5;
/// Rooms per wing along each axis.
pub const WING: i32 = 3;

/// A room's place on the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct RoomId {
    pub x: i32,
    pub z: i32,
}

impl RoomId {
    pub const ORIGIN: RoomId = RoomId { x: 0, z: 0 };

    pub fn new(x: i32, z: i32) -> RoomId {
        RoomId { x, z }
    }

    /// The centre of the room's floor.
    pub fn centre(self) -> Vec3 {
        Vec3::new(self.x as f32 * CELL, 0.0, self.z as f32 * CELL)
    }

    /// The room whose cell holds the point (halls belong to the cell they are in).
    pub fn at(p: Vec3) -> RoomId {
        RoomId { x: (p.x / CELL).round() as i32, z: (p.z / CELL).round() as i32 }
    }

    /// The neighbour across `side`.
    pub fn step(self, side: Side) -> RoomId {
        let (dx, dz) = side.delta();
        RoomId { x: self.x + dx, z: self.z + dz }
    }

    /// The gallery's number on the plan and at its door: 1 for the first room, then outward in a square spiral, so every room has its own for ever.
    pub fn number(self) -> u64 {
        let (x, z) = (self.x as i64, -(self.z as i64));
        let k = x.abs().max(z.abs());
        if k == 0 {
            return 1;
        }
        // The ring k holds the numbers (2k-1)^2 + 1 ..= (2k+1)^2, starting just above the bottom-right corner and going round anticlockwise.
        let base = (2 * k - 1) * (2 * k - 1);
        let along = if x == k && z > -k {
            z + k
        } else if z == k {
            2 * k + (k - x)
        } else if x == -k {
            4 * k + (k - z)
        } else {
            6 * k + (x + k)
        };
        (base + along) as u64
    }

    /// The room with gallery number `n` (the inverse of [`Self::number`]; 0 is taken as 1).
    pub fn from_number(n: u64) -> RoomId {
        let n = n.max(1) as i64;
        if n == 1 {
            return RoomId::ORIGIN;
        }
        let mut k = ((((n as f64).sqrt() - 1.0) / 2.0).ceil()) as i64;
        while (2 * k + 1) * (2 * k + 1) < n {
            k += 1;
        }
        while k > 1 && (2 * k - 1) * (2 * k - 1) >= n {
            k -= 1;
        }
        let along = n - (2 * k - 1) * (2 * k - 1);
        let (x, z) = if along <= 2 * k {
            (k, along - k)
        } else if along <= 4 * k {
            (k - (along - 2 * k), k)
        } else if along <= 6 * k {
            (-k, k - (along - 4 * k))
        } else {
            ((along - 6 * k) - k, -k)
        };
        RoomId { x: x as i32, z: -z as i32 }
    }
}

/// A side of a room. North is -z, east +x.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Side {
    North,
    East,
    South,
    West,
}

impl Side {
    pub const ALL: [Side; 4] = [Side::North, Side::East, Side::South, Side::West];

    /// The grid step toward this side.
    pub fn delta(self) -> (i32, i32) {
        match self {
            Side::North => (0, -1),
            Side::East => (1, 0),
            Side::South => (0, 1),
            Side::West => (-1, 0),
        }
    }

    /// The unit vector out of the room through this side.
    pub fn out(self) -> Vec3 {
        let (dx, dz) = self.delta();
        Vec3::new(dx as f32, 0.0, dz as f32)
    }

    pub fn opposite(self) -> Side {
        match self {
            Side::North => Side::South,
            Side::East => Side::West,
            Side::South => Side::North,
            Side::West => Side::East,
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }

    /// Whether a wall on this side runs along x (north and south walls do).
    pub fn runs_along_x(self) -> bool {
        matches!(self, Side::North | Side::South)
    }
}

/// The border between two neighbouring rooms: east of `(x, z)` or south of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EdgeId {
    pub x: i32,
    pub z: i32,
    /// Between `(x, z)` and `(x + 1, z)`; otherwise between `(x, z)` and `(x, z + 1)`.
    pub east: bool,
}

impl EdgeId {
    /// The border on `side` of `room`.
    pub fn of(room: RoomId, side: Side) -> EdgeId {
        match side {
            Side::East => EdgeId { x: room.x, z: room.z, east: true },
            Side::West => EdgeId { x: room.x - 1, z: room.z, east: true },
            Side::South => EdgeId { x: room.x, z: room.z, east: false },
            Side::North => EdgeId { x: room.x, z: room.z - 1, east: false },
        }
    }

    /// The two rooms it joins (west/north one first).
    pub fn rooms(self) -> (RoomId, RoomId) {
        let a = RoomId::new(self.x, self.z);
        (a, if self.east { a.step(Side::East) } else { a.step(Side::South) })
    }

    /// The middle of the doorway, on the floor.
    pub fn centre(self) -> Vec3 {
        let a = RoomId::new(self.x, self.z).centre();
        if self.east {
            a + Vec3::new(CELL * 0.5, 0.0, 0.0)
        } else {
            a + Vec3::new(0.0, 0.0, CELL * 0.5)
        }
    }

    /// The direction one walks through it from the first room to the second.
    pub fn through(self) -> Vec3 {
        if self.east {
            Vec3::X
        } else {
            Vec3::Z
        }
    }
}

/// How a room's floor space is arranged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// Open floor.
    Plain,
    /// Four columns near the corners.
    Pillars,
    /// A free-standing wall across the middle, hung on both faces; `along_x` when it runs east-west.
    Partition { along_x: bool },
}

/// The floor's finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Floor {
    /// Two-tone marble squares.
    Checker,
    /// Long boards.
    Boards,
    /// Polished stone slabs with a border.
    Slabs,
    /// A herringbone parquet.
    Parquet,
}

/// One room of the plan.
#[derive(Debug, Clone)]
pub struct Room {
    pub id: RoomId,
    /// The theme (index into the collection's themes).
    pub theme: usize,
    /// The theme of the wing it stands in (a room may borrow another).
    pub wing_theme: usize,
    /// Inside size: width along x, depth along z.
    pub size: Vec2,
    /// Inside height.
    pub height: f32,
    pub plan: Plan,
    pub floor: Floor,
    /// A glass lantern in the ceiling with the sky above it.
    pub skylight: bool,
    /// Ceiling beams.
    pub beams: bool,
    /// Which sides have a door, by [`Side::index`].
    pub doors: [bool; 4],
}

impl Room {
    pub fn has_door(&self, side: Side) -> bool {
        self.doors[side.index()]
    }

    /// The inside face of the wall on `side`, as (a point at its middle on the floor, the unit vector into the room).
    pub fn wall_face(&self, side: Side) -> (Vec3, Vec3) {
        let half = Vec3::new(self.size.x * 0.5, 0.0, self.size.y * 0.5);
        let out = side.out();
        (self.id.centre() + out * half, -out)
    }

    /// The length of the wall on `side` (inside).
    pub fn wall_length(&self, side: Side) -> f32 {
        if side.runs_along_x() {
            self.size.x
        } else {
            self.size.y
        }
    }

    /// Whether a point (x, z) is inside the room's floor (not the halls).
    pub fn contains(&self, p: Vec3) -> bool {
        let d = p - self.id.centre();
        d.x.abs() <= self.size.x * 0.5 && d.z.abs() <= self.size.y * 0.5
    }
}

/// The plan of one museum: everything is a function of `seed` and the number of themes.
#[derive(Debug, Clone)]
pub struct Layout {
    pub seed: u32,
    pub themes: usize,
    /// The first room's theme, when the museum names one.
    pub start_theme: Option<usize>,
}

fn draw(seed: u32, x: i32, z: i32, what: i64) -> Draw {
    Draw::new(&[seed as i64, x as i64, z as i64, what])
}

impl Layout {
    pub fn new(seed: u32, themes: usize, start_theme: Option<usize>) -> Layout {
        Layout { seed, themes: themes.max(1), start_theme }
    }

    /// The parent door of a room: north or east.
    fn parent(&self, r: RoomId) -> Side {
        if draw(self.seed, r.x, r.z, 1).chance(0.5) {
            Side::North
        } else {
            Side::East
        }
    }

    /// Whether there is a door on this border.
    pub fn has_door(&self, e: EdgeId) -> bool {
        let (a, b) = e.rooms();
        if a == RoomId::ORIGIN || b == RoomId::ORIGIN {
            return true;
        }
        // The parent door of the room on the far (east/south) side, or of the near one.
        let parent = if e.east { self.parent(a) == Side::East } else { self.parent(b) == Side::North };
        parent || draw(self.seed, e.x, e.z, if e.east { 2 } else { 3 }).chance(0.36)
    }

    /// The wing a room belongs to (its coordinates on the grid of wings).
    pub fn wing_of(&self, r: RoomId) -> (i32, i32) {
        let (ox, oz) = (draw(self.seed, 0, 0, 7).below(WING as usize) as i32, draw(self.seed, 0, 0, 8).below(WING as usize) as i32);
        ((r.x + ox).div_euclid(WING), (r.z + oz).div_euclid(WING))
    }

    /// The theme of the wing a room stands in.
    pub fn wing_theme(&self, r: RoomId) -> usize {
        let (ox, oz) = (draw(self.seed, 0, 0, 7).below(WING as usize) as i32, draw(self.seed, 0, 0, 8).below(WING as usize) as i32);
        let (wx, wz) = self.wing_of(r);
        if let Some(t) = self.start_theme.filter(|_| (wx, wz) == (ox.div_euclid(WING), oz.div_euclid(WING))) {
            return t;
        }
        let base = |wx: i32, wz: i32| draw(self.seed, wx, wz, 9).below(self.themes);
        let mut t = base(wx, wz);
        // Two neighbouring wings rarely share a mood: step past the west and north wings' (unadjusted) themes.
        for _ in 0..2 {
            if self.themes > 2 && (t == base(wx - 1, wz) || t == base(wx, wz - 1)) {
                t = (t + 1 + draw(self.seed, wx, wz, 10).below(self.themes - 1)) % self.themes;
            }
        }
        t
    }

    /// One room of the plan.
    pub fn room(&self, id: RoomId) -> Room {
        let mut d = draw(self.seed, id.x, id.z, 20);
        let wing_theme = self.wing_theme(id);
        let theme = if id != RoomId::ORIGIN && self.themes > 1 && d.chance(0.14) {
            // A guest from a neighbouring wing.
            let side = *d.pick(&Side::ALL);
            let n = id.step(side);
            self.wing_theme(RoomId::new(n.x + side.delta().0 * 2, n.z + side.delta().1 * 2))
        } else {
            wing_theme
        };
        let sizes: [f32; 5] = [12.0, 14.0, 16.0, 18.0, 20.0];
        let (w, depth): (f32, f32) = if id == RoomId::ORIGIN { (16.0, 16.0) } else { (*d.pick(&sizes), *d.pick(&sizes)) };
        let height = [5.5, 6.5, 7.5, 9.0][d.weighted(&[3.0, 3.0, 2.0, 1.2])];
        let plan = if id == RoomId::ORIGIN {
            Plan::Plain
        } else {
            match d.weighted(&[4.0, 2.0, 2.4]) {
                1 if w.min(depth) >= 14.0 => Plan::Pillars,
                2 if w.max(depth) >= 16.0 => Plan::Partition { along_x: w >= depth },
                _ => Plan::Plain,
            }
        };
        let floor = [Floor::Checker, Floor::Boards, Floor::Slabs, Floor::Parquet][d.below(4)];
        let skylight = height >= 6.5 && d.chance(0.45);
        let beams = !skylight && d.chance(0.5);
        let mut doors = [false; 4];
        for side in Side::ALL {
            doors[side.index()] = self.has_door(EdgeId::of(id, side));
        }
        Room { id, theme, wing_theme, size: Vec2::new(w, depth), height, plan, floor, skylight, beams, doors }
    }

    /// The rooms joined to `id` by a door.
    pub fn neighbours(&self, id: RoomId) -> Vec<(Side, RoomId)> {
        Side::ALL.into_iter().filter(|s| self.has_door(EdgeId::of(id, *s))).map(|s| (s, id.step(s))).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashSet, VecDeque};

    #[test]
    fn every_room_has_a_door_and_the_plan_is_one_piece() {
        let l = Layout::new(42, 12, None);
        // Breadth-first from the first room over a 15 x 15 block: every room in the inner 7 x 7 must be reached (a binary-tree maze connects everything,
        // though the path may leave the block, so the search may roam a little wider).
        let mut seen = HashSet::new();
        let mut queue = VecDeque::from([RoomId::ORIGIN]);
        seen.insert(RoomId::ORIGIN);
        while let Some(r) = queue.pop_front() {
            assert!(!l.neighbours(r).is_empty(), "{r:?} has no door");
            for (_, n) in l.neighbours(r) {
                if n.x.abs() <= 12 && n.z.abs() <= 12 && seen.insert(n) {
                    queue.push_back(n);
                }
            }
        }
        for x in -3..=3 {
            for z in -3..=3 {
                assert!(seen.contains(&RoomId::new(x, z)), "({x}, {z}) cannot be reached");
            }
        }
    }

    #[test]
    fn doors_agree_from_both_sides_and_the_first_room_has_four() {
        let l = Layout::new(7, 12, None);
        for x in -5..5 {
            for z in -5..5 {
                let r = l.room(RoomId::new(x, z));
                for s in Side::ALL {
                    let n = l.room(r.id.step(s));
                    assert_eq!(r.has_door(s), n.has_door(s.opposite()), "({x},{z}) {s:?}");
                }
            }
        }
        assert_eq!(l.room(RoomId::ORIGIN).doors, [true; 4]);
    }

    #[test]
    fn wings_share_themes_and_rooms_fit_their_cells() {
        let l = Layout::new(3, 12, Some(5));
        assert_eq!(l.room(RoomId::ORIGIN).theme, 5);
        let mut same = 0;
        let mut pairs = 0;
        for x in -6..6 {
            for z in -6..6 {
                let r = l.room(RoomId::new(x, z));
                assert!(r.size.x + 2.0 * 2.0 <= CELL && r.size.y + 2.0 * 2.0 <= CELL, "a hall of at least 2 m each side");
                pairs += 1;
                if l.wing_theme(RoomId::new(x + 1, z)) == r.wing_theme {
                    same += 1;
                }
            }
        }
        assert!(same * 2 > pairs, "most east neighbours share the wing: {same}/{pairs}");
    }

    #[test]
    fn gallery_numbers_are_unique_and_start_at_one() {
        assert_eq!(RoomId::ORIGIN.number(), 1);
        let mut seen = HashSet::new();
        for x in -10..=10 {
            for z in -10..=10 {
                assert!(seen.insert(RoomId::new(x, z).number()), "({x},{z}) repeats a number");
            }
        }
        assert_eq!(seen.iter().max(), Some(&(21 * 21)));
        for n in 1..2000u64 {
            assert_eq!(RoomId::from_number(n).number(), n);
        }
    }
}
