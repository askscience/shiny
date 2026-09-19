//! Layer blend modes — the same nine Compositor compositing modes (plus a
//! normal default), applied to straight (non-premultiplied) 0..=1 channels by
//! the compositor in `composite.rs`.

/// How a layer's colours combine with the backdrop beneath it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    Difference,
    ColorDodge,
    ColorBurn,
    /// Linear dodge (`backdrop + source`), a common extra.
    Add,
    /// Linear burn (`backdrop - source`).
    Subtract,
}

impl BlendMode {
    /// Parse a wire name. Accepts `color_dodge`, `color-dodge` and
    /// `colordodge` spellings; unknown names fall back to `Normal`.
    pub fn parse(s: &str) -> BlendMode {
        let n = s.trim().to_ascii_lowercase().replace(['-', ' '], "_");
        match n.as_str() {
            "multiply" => BlendMode::Multiply,
            "screen" => BlendMode::Screen,
            "overlay" => BlendMode::Overlay,
            "darken" => BlendMode::Darken,
            "lighten" => BlendMode::Lighten,
            "difference" => BlendMode::Difference,
            "color_dodge" | "colordodge" => BlendMode::ColorDodge,
            "color_burn" | "colorburn" => BlendMode::ColorBurn,
            "add" | "linear_dodge" | "lineardodge" => BlendMode::Add,
            "subtract" | "linear_burn" | "linearburn" => BlendMode::Subtract,
            _ => BlendMode::Normal,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            BlendMode::Normal => "normal",
            BlendMode::Multiply => "multiply",
            BlendMode::Screen => "screen",
            BlendMode::Overlay => "overlay",
            BlendMode::Darken => "darken",
            BlendMode::Lighten => "lighten",
            BlendMode::Difference => "difference",
            BlendMode::ColorDodge => "color_dodge",
            BlendMode::ColorBurn => "color_burn",
            BlendMode::Add => "add",
            BlendMode::Subtract => "subtract",
        }
    }

    /// The canonical list advertised to the window and the agent.
    pub fn all() -> &'static [&'static str] {
        &[
            "normal",
            "multiply",
            "screen",
            "overlay",
            "darken",
            "lighten",
            "difference",
            "color_dodge",
            "color_burn",
            "add",
            "subtract",
        ]
    }

    /// Blend one straight channel. `cb` is the backdrop, `cs` the source.
    #[inline]
    pub fn channel(self, cb: f32, cs: f32) -> f32 {
        match self {
            BlendMode::Normal => cs,
            BlendMode::Multiply => cb * cs,
            BlendMode::Screen => cb + cs - cb * cs,
            BlendMode::Overlay => {
                // Hard-light with source and backdrop swapped.
                if cb <= 0.5 {
                    2.0 * cb * cs
                } else {
                    1.0 - 2.0 * (1.0 - cb) * (1.0 - cs)
                }
            }
            BlendMode::Darken => cb.min(cs),
            BlendMode::Lighten => cb.max(cs),
            BlendMode::Difference => (cb - cs).abs(),
            BlendMode::ColorDodge => {
                if cb <= 0.0 {
                    0.0
                } else if cs >= 1.0 {
                    1.0
                } else {
                    (cb / (1.0 - cs)).min(1.0)
                }
            }
            BlendMode::ColorBurn => {
                if cb >= 1.0 {
                    1.0
                } else if cs <= 0.0 {
                    0.0
                } else {
                    1.0 - ((1.0 - cb) / cs).min(1.0)
                }
            }
            BlendMode::Add => (cb + cs).min(1.0),
            BlendMode::Subtract => (cb - cs).max(0.0),
        }
    }
}

impl Default for BlendMode {
    fn default() -> Self {
        BlendMode::Normal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_aliases() {
        assert_eq!(BlendMode::parse("Color Dodge"), BlendMode::ColorDodge);
        assert_eq!(BlendMode::parse("color-dodge"), BlendMode::ColorDodge);
        assert_eq!(BlendMode::parse("COLOR_BURN"), BlendMode::ColorBurn);
        assert_eq!(BlendMode::parse("multiply"), BlendMode::Multiply);
        assert_eq!(BlendMode::parse("nonsense"), BlendMode::Normal);
    }

    #[test]
    fn multiply_and_screen_are_duals() {
        for i in 0..=10 {
            let v = i as f32 / 10.0;
            assert!((BlendMode::Multiply.channel(v, v) - v * v).abs() < 1e-6);
            let s = BlendMode::Screen.channel(v, v);
            let m = BlendMode::Multiply.channel(1.0 - v, 1.0 - v);
            assert!((s - (1.0 - m)).abs() < 1e-6, "screen/multiply dual at {v}");
        }
    }

    #[test]
    fn dodge_and_burn_bounds() {
        assert_eq!(BlendMode::ColorDodge.channel(0.0, 0.5), 0.0);
        assert_eq!(BlendMode::ColorDodge.channel(0.5, 1.0), 1.0);
        assert_eq!(BlendMode::ColorBurn.channel(1.0, 0.5), 1.0);
        assert_eq!(BlendMode::ColorBurn.channel(0.5, 0.0), 0.0);
    }

    #[test]
    fn every_mode_stays_in_unit_range() {
        for &name in BlendMode::all() {
            let mode = BlendMode::parse(name);
            for a in 0..=10 {
                for b in 0..=10 {
                    let out = mode.channel(a as f32 / 10.0, b as f32 / 10.0);
                    assert!((0.0..=1.0).contains(&out), "{name} out of range: {out}");
                }
            }
        }
    }
}
