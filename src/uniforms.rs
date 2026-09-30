//! Team uniforms: the colours a soldier, their sleeves and their gloves are made of (ADR 2026-09-30-killchain-loadout-shooter).
//!
//! One place decides what a team looks like, so the third-person body (`characters`), the first-person hands and forearms (`firearms`,
//! `viewer::HeldPart::skin`), the lobby and the scoreboard cannot drift apart: a player's own sleeve and glove are the very colours other
//! players see on their arms. Colours are linear RGB, like the rest of the renderer.

use glam::Vec3;

/// Team 1 and team 2 names, index = team - 1.
pub const TEAM_NAMES: [&str; 2] = ["Ridgeback", "Nightfall"];

/// What one team wears.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Uniform {
    /// The team's name.
    pub name: &'static str,
    /// Tunic and sleeves.
    pub jacket: Vec3,
    /// Trousers.
    pub trousers: Vec3,
    /// Plate carrier and webbing.
    pub vest: Vec3,
    /// Combat helmet shell.
    pub helmet: Vec3,
    /// Boots.
    pub boots: Vec3,
    /// Gloves (what the hand looks like in first person).
    pub glove: Vec3,
    /// First-person sleeve (the jacket's colour).
    pub sleeve: Vec3,
    /// A small accent (shoulder patch, helmet band) that tells the teams apart at a glance.
    pub accent: Vec3,
    /// Skin of the face and neck.
    pub skin: Vec3,
}

/// sRGB bytes to linear RGB.
pub fn srgb(r: u8, g: u8, b: u8) -> Vec3 {
    let f = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    Vec3::new(f(r), f(g), f(b))
}

/// Team 1: army green with coyote-tan kit.
pub fn ridgeback() -> Uniform {
    let jacket = srgb(0x55, 0x5e, 0x2b);
    Uniform {
        name: TEAM_NAMES[0],
        jacket,
        trousers: srgb(0x48, 0x50, 0x27),
        vest: srgb(0x9a, 0x86, 0x5c),
        helmet: srgb(0x5d, 0x66, 0x33),
        boots: srgb(0x4a, 0x38, 0x24),
        glove: srgb(0x8c, 0x76, 0x4b),
        sleeve: jacket,
        accent: srgb(0xd0, 0xb0, 0x5a),
        skin: srgb(0xc8, 0x9a, 0x74),
    }
}

/// Team 2: navy and black.
pub fn nightfall() -> Uniform {
    let jacket = srgb(0x1f, 0x2d, 0x4d);
    Uniform {
        name: TEAM_NAMES[1],
        jacket,
        trousers: srgb(0x16, 0x1e, 0x33),
        vest: srgb(0x17, 0x18, 0x1c),
        helmet: srgb(0x1c, 0x1e, 0x24),
        boots: srgb(0x0d, 0x0d, 0x10),
        glove: srgb(0x12, 0x13, 0x16),
        sleeve: jacket,
        accent: srgb(0x6f, 0xa0, 0xd8),
        skin: srgb(0x9a, 0x6e, 0x50),
    }
}

/// The uniform of a hand skin: `1` Ridgeback, `2` Nightfall, anything else none.
pub fn by_skin(skin: u8) -> Option<Uniform> {
    match skin {
        1 => Some(ridgeback()),
        2 => Some(nightfall()),
        _ => None,
    }
}

/// The uniform of a team number (`1` or `2`).
pub fn for_team(team: u8) -> Option<Uniform> {
    by_skin(team)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_teams_are_easy_to_tell_apart() {
        let (a, b) = (ridgeback(), nightfall());
        let lum = |c: Vec3| 0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z;
        assert!(lum(a.jacket) > lum(b.jacket) * 1.5, "green is clearly lighter than navy");
        assert!(a.jacket.y > a.jacket.x && a.jacket.y > a.jacket.z, "army green is green");
        assert!(b.jacket.z > b.jacket.x && b.jacket.z > b.jacket.y, "navy is blue");
        assert_eq!(a.sleeve, a.jacket, "the sleeve in first person is the jacket others see");
        assert!(by_skin(0).is_none() && by_skin(3).is_none());
    }
}
