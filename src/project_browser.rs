//! Local project map discovery and a paginated, keyboard/gamepad-friendly map picker.
use std::path::{Path, PathBuf};

/// One playable map within the containing project's declared map list.
#[derive(Clone, Debug)]
pub struct MapEntry {
    pub name: String,
    pub path: PathBuf,
}

/// Discover the nearest game.json, accepting only existing maps inside that project directory.
pub fn maps_for(scene: &Path) -> Vec<MapEntry> {
    let Ok(scene) = scene.canonicalize() else {
        return vec![];
    };
    for ancestor in scene.ancestors().skip(1).take(5) {
        if !ancestor.join("game.json").is_file() {
            continue;
        }
        let Ok(game) = crate::tools::game::load(ancestor) else {
            return vec![];
        };
        return game
            .maps
            .iter()
            .filter_map(|name| {
                let path = ancestor.join(name).canonicalize().ok()?;
                if !path.starts_with(ancestor) || !path.is_file() {
                    return None;
                }
                Some(MapEntry { name: path.file_stem()?.to_string_lossy().replace(['-', '_'], " "), path })
            })
            .collect();
    }
    vec![]
}

/// Eight maps per page, with stable action ids for mouse input and a highlighted pad selection.
pub fn layout(w: u32, h: u32, maps: &[MapEntry], selected: usize) -> crate::ui::Layout {
    let mut l = crate::ui::Layout::new(w, h);
    let (wi, hi) = (w as i32, h as i32);
    let s = (hi / 360).max(1);
    l.panel("background", (0, 0, wi, hi), None, Some([12, 18, 28, 245]), None);
    l.label_fit("title", None, wi / 2, hi / 20, "PROJECT MAPS", s * 2, wi - 16, [247, 204, 113, 255]);
    let start = selected / 8 * 8;
    for (index, map) in maps.iter().enumerate().skip(start).take(8) {
        let y = hi / 6 + (index - start) as i32 * hi / 12;
        let picked = index == selected;
        l.button(
            &format!("map{index}"),
            (wi / 12, y, wi * 11 / 12, y + hi / 14),
            None,
            &map.name,
            s,
            if picked { [45, 72, 88, 255] } else { [24, 36, 48, 255] },
            (if picked { [247, 204, 113, 255] } else { [60, 85, 102, 255] }, 1),
            [238, 243, 246, 255],
        );
    }
    l.label_fit("help", None, wi / 2, hi * 89 / 100, "UP/DOWN SELECT - ENTER/A OPEN - ESC/B BACK", s, wi - 12, [175, 194, 206, 255]);
    l.label_fit(
        "pages",
        None,
        wi / 2,
        hi * 95 / 100,
        &format!("PAGE {} / {} - LEFT/RIGHT OR LB/RB", selected / 8 + 1, maps.len().div_ceil(8).max(1)),
        s,
        wi - 12,
        [175, 194, 206, 255],
    );
    l
}

#[cfg(test)]
mod tests {
    #[test]
    fn map_pages_have_unique_actions_and_fit_small_windows() {
        let maps: Vec<_> = (0..19).map(|i| super::MapEntry { name: format!("map {i}"), path: format!("map{i}.json").into() }).collect();
        for (w, h) in [(640, 480), (1280, 720), (1920, 1080)] {
            for selected in [0, 8, 18] {
                let l = super::layout(w, h, &maps, selected);
                assert!(l.check().is_empty(), "{:?}", l.check());
            }
        }
    }
}
