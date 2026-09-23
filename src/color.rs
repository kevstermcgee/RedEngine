use glam::Vec3;

/// Parses a `#rrggbb` or `#rrggbbaa` hex color into linear-space RGB (alpha, if present, is
/// dropped — materials in this engine are opaque). Shading math happens in linear space so the
/// sRGB→linear conversion happens once here, at scene-load time.
pub fn parse_hex_to_linear(s: &str) -> Result<Vec3, String> {
    let s = s.trim();
    let hex = s.strip_prefix('#').ok_or_else(|| format!("color '{s}' must start with '#'"))?;
    if hex.len() != 6 && hex.len() != 8 {
        return Err(format!("color '{s}' must be '#rrggbb' or '#rrggbbaa'"));
    }
    let byte = |i: usize| -> Result<f32, String> {
        u8::from_str_radix(&hex[i..i + 2], 16)
            .map(|b| b as f32 / 255.0)
            .map_err(|_| format!("color '{s}' has invalid hex digits"))
    };
    let r = srgb_to_linear(byte(0)?);
    let g = srgb_to_linear(byte(2)?);
    let b = srgb_to_linear(byte(4)?);
    Ok(Vec3::new(r, g, b))
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_black_and_white() {
        assert_eq!(parse_hex_to_linear("#000000").unwrap(), Vec3::ZERO);
        let w = parse_hex_to_linear("#ffffff").unwrap();
        assert!((w.x - 1.0).abs() < 1e-5 && (w.y - 1.0).abs() < 1e-5 && (w.z - 1.0).abs() < 1e-5);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse_hex_to_linear("ff0000").is_err());
        assert!(parse_hex_to_linear("#ff00").is_err());
        assert!(parse_hex_to_linear("#gg0000").is_err());
    }

    #[test]
    fn accepts_alpha_suffix_and_ignores_it() {
        let a = parse_hex_to_linear("#ff0000ff").unwrap();
        let b = parse_hex_to_linear("#ff0000").unwrap();
        assert_eq!(a, b);
    }
}
