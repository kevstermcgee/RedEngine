//! Genre-neutral presentation of a scene's live rule state in the standard client.
//!
//! The rules engine remains authoritative and renderer-free. This module only turns its public
//! state (game variables, a recent event and terminal outcome) into the same audited [`Layout`]
//! used by the rest of the client UI.

use super::{fit_scale, text_height, Layout};

const TEXT: [u8; 4] = [236, 238, 245, 255];
const DIM: [u8; 4] = [160, 166, 188, 255];
const GOLD: [u8; 4] = [255, 210, 74, 255];

fn value(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{v:.0}")
    } else {
        let s = format!("{v:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Builds the standard in-game rules HUD. All scene-defined variables are shown (up to eight),
/// the newest event may be supplied transiently, and an ended match gets a central banner.
/// Empty state produces an empty transparent layout.
pub fn hud_layout(w: u32, h: u32, vars: &[(&str, f64)], event: Option<&str>, outcome: Option<&str>) -> Layout {
    let mut l = Layout::new(w, h);
    let (wi, hi) = (w as i32, h as i32);
    let s = (hi / 240).max(1);

    if !vars.is_empty() || event.is_some() {
        let shown = vars.len().min(8);
        let rows = shown + usize::from(vars.len() > shown) + usize::from(event.is_some());
        let panel_w = (125 * s).min(wi - 8);
        let row_h = text_height(s) + 3 * s;
        let panel_h = 8 * s + rows as i32 * row_h;
        let panel = l.panel("rule_state", (4 * s, 4 * s, 4 * s + panel_w, 4 * s + panel_h), None, Some([8, 11, 20, 185]), Some(([75, 82, 110, 220], 1)));
        let x = 9 * s;
        let max_w = panel_w - 10 * s;
        let mut y = 8 * s;
        for (i, (name, n)) in vars.iter().take(shown).enumerate() {
            let text = format!("{}: {}", name.to_uppercase(), value(*n));
            l.label_fit(&format!("var_{i}"), Some(panel), x + max_w / 2, y, &text, s, max_w, TEXT);
            y += row_h;
        }
        if vars.len() > shown {
            let text = format!("+{} MORE", vars.len() - shown);
            l.label_fit("vars_more", Some(panel), x + max_w / 2, y, &text, s, max_w, DIM);
            y += row_h;
        }
        if let Some(name) = event {
            let text = format!("EVENT: {}", name.to_uppercase());
            l.label_fit("rule_event", Some(panel), x + max_w / 2, y, &text, s, max_w, GOLD);
        }
    }

    if let Some(outcome) = outcome {
        let text = outcome.to_uppercase();
        let bw = (240 * s).min(wi - 8);
        let bh = 34 * s;
        let x0 = (wi - bw) / 2;
        let y0 = (hi - bh) / 2;
        let banner = l.panel("outcome", (x0, y0, x0 + bw, y0 + bh), None, Some([8, 11, 20, 225]), Some((GOLD, (s / 2).max(2))));
        let scale = fit_scale(&text, bw - 12 * s, s * 2);
        l.label_fit("outcome_text", Some(banner), wi / 2, y0 + (bh - text_height(scale)) / 2, &text, scale, bw - 12 * s, GOLD);
    }
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_hud_is_auditable_at_supported_sizes_and_caps_long_state() {
        let owned: Vec<(String, f64)> = (0..12).map(|i| (format!("a_very_long_variable_name_{i}"), i as f64 + 0.25)).collect();
        let vars: Vec<(&str, f64)> = owned.iter().map(|(n, v)| (n.as_str(), *v)).collect();
        for (w, h) in crate::ui::screens::CHECK_SIZES {
            let l = hud_layout(w, h, &vars, Some("collected_the_last_thing"), Some("a_wonderful_victory"));
            assert!(l.check().is_empty(), "{w}x{h}: {:?}", l.check());
            assert!(l.widgets.iter().any(|x| x.id == "vars_more"));
            assert!(l.widgets.iter().any(|x| x.id == "outcome_text"));
        }
    }

    #[test]
    fn values_are_compact_and_empty_state_draws_nothing() {
        assert_eq!(value(3.0), "3");
        assert_eq!(value(1.25), "1.25");
        assert!(hud_layout(640, 360, &[], None, None).widgets.is_empty());
    }
}
