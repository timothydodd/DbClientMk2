//! Deterministic, theme-aware accent colors derived from a name (database / connection): the
//! hue is a stable hash of the lower-cased name, saturation and lightness are tuned per theme so
//! the same database keeps its hue across themes while staying legible on each background.

pub fn rgb_for_name(name: &str, theme: &str) -> (u8, u8, u8) {
    if name.is_empty() {
        return (189, 147, 249);
    }
    let mut hash: i32 = 0;
    for c in name.chars().flat_map(|c| c.to_lowercase()) {
        hash = hash.wrapping_mul(31).wrapping_add(c as i32);
    }
    let hue = (hash % 360).abs() as f64 / 360.0;
    let (s, l) = match theme {
        "Light" => (0.62, 0.40),
        "Dracula" => (0.78, 0.70),
        _ => (0.62, 0.62),
    };
    hsl_to_rgb(hue, s, l)
}

/// Connection tab color: hue from the name, fixed saturation/lightness.
pub fn rgb_for_connection(name: &str) -> (u8, u8, u8) {
    if name.is_empty() {
        return (189, 147, 249);
    }
    let mut hash: i32 = 0;
    for c in name.chars() {
        hash = hash.wrapping_mul(31).wrapping_add(c as i32);
    }
    let hue = (hash % 360).abs() as f64 / 360.0;
    hsl_to_rgb(hue, 0.65, 0.6)
}

pub fn hex(rgb: (u8, u8, u8)) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb.0, rgb.1, rgb.2)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let (r, g, b) = if s == 0.0 {
        (l, l, l)
    } else {
        let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
        let p = 2.0 * l - q;
        (hue_to_rgb(p, q, h + 1.0 / 3.0), hue_to_rgb(p, q, h), hue_to_rgb(p, q, h - 1.0 / 3.0))
    };
    ((r * 255.0).round() as u8, (g * 255.0).round() as u8, (b * 255.0).round() as u8)
}

fn hue_to_rgb(p: f64, q: f64, mut t: f64) -> f64 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        return p + (q - p) * 6.0 * t;
    }
    if t < 0.5 {
        return q;
    }
    if t < 2.0 / 3.0 {
        return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_for_a_name_and_different_between_names() {
        let a = rgb_for_name("Northwind", "Dark");
        assert_eq!(a, rgb_for_name("Northwind", "Dark"));
        assert_ne!(a, rgb_for_name("AdventureWorks", "Dark"));
        assert_eq!(a, rgb_for_name("northwind", "Dark"));
    }

    #[test]
    fn hex_formats() {
        assert_eq!(hex((189, 147, 249)), "#bd93f9");
    }
}
