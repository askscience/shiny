//! The one description of how an Impress slide looks — shared by the `.odp`
//! writer ([`crate::odp`]) and the Impress window, so what the editor shows is
//! what the file contains.
//!
//! Geometry is given on the editor's logical slide, **640 × 360 px**, which is
//! the ODP page of **28 × 15.75 cm** (the standard 16:9 Impress page): one px
//! is 28/640 cm and 1.2402 pt. Every text block is a fixed box with its own
//! vertical anchor, type size, weight, fixed line height, tracking and colour;
//! the accent rules and the backgrounds are part of the layout too. The
//! window reads the same numbers from `plugins/impress/web/slide-spec.js`,
//! which [`spec_js`] generates (a test fails when the two drift apart).
//!
//! Text that does not fit its box is shrunk the way DeckCraft (MIT/Apache)
//! does it for "shrink on overflow": the editor searches the largest scale at
//! which the slide's shrinkable boxes fit, stores it on the slide
//! (`Slide::fit`), and the writer multiplies the type sizes by it, so the file
//! shows the same size the editor did.

use serde_json::{json, Map, Value};

/// The editor's logical slide, in px.
pub const PAGE_PX: (f64, f64) = (640.0, 360.0);
/// The ODP page, in cm.
pub const PAGE_CM: (f64, f64) = (28.0, 15.75);

/// Font of titles and quotes.
pub const FONT_DISPLAY: &str = "Inter Display";
/// Font of everything else.
pub const FONT_TEXT: &str = "Inter";
/// What both fall back to (bundled with every Linux LibreOffice).
pub const FONT_FALLBACK: &str = "DejaVu Sans";

/// Bullet glyph and its size relative to the paragraph.
pub const BULLET_CHAR: &str = "\u{25CF}";
pub const BULLET_RELATIVE: f64 = 0.7;
/// Distance from the bullet to the text (the hanging indent), in px.
pub const BULLET_INDENT_PX: f64 = 24.0;
/// Smallest shrink scale.
pub const MIN_FIT: f64 = 0.4;

pub fn px_to_cm(px: f64) -> f64 {
    px * PAGE_CM.0 / PAGE_PX.0
}

pub fn px_to_pt(px: f64) -> f64 {
    px * (PAGE_CM.0 / 2.54 * 72.0) / PAGE_PX.0
}

/// `"1.234cm"` with three decimals.
pub fn cm(px: f64) -> String {
    format!("{:.3}cm", px_to_cm(px))
}

/// `"57.0pt"`.
pub fn pt(px: f64) -> String {
    format!("{:.1}pt", px_to_pt(px))
}

// ── colours ──────────────────────────────────────────────────────────────

/// A deck theme's colours. The text colours over dark backgrounds are
/// precomputed (ODF has no alpha on text), so the editor uses the same hex.
#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    pub accent: String,
    pub dark_a: String,
    pub dark_b: String,
    pub light: String,
    pub section_b: String,
    pub ink: String,
    pub body: String,
    pub on_dark: String,
    pub sub_on_dark: String,
    pub attrib_on_dark: String,
}

impl Palette {
    pub fn get(&self, key: &str) -> &str {
        match key {
            "accent" => &self.accent,
            "darkA" => &self.dark_a,
            "darkB" => &self.dark_b,
            "light" => &self.light,
            "sectionB" => &self.section_b,
            "ink" => &self.ink,
            "body" => &self.body,
            "onDark" => &self.on_dark,
            "subOnDark" => &self.sub_on_dark,
            "attribOnDark" => &self.attrib_on_dark,
            _ => "#000000",
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "accent": self.accent, "darkA": self.dark_a, "darkB": self.dark_b,
            "light": self.light, "sectionB": self.section_b, "ink": self.ink,
            "body": self.body, "onDark": self.on_dark, "subOnDark": self.sub_on_dark,
            "attribOnDark": self.attrib_on_dark,
        })
    }
}

fn rgb(hex: &str) -> (f64, f64, f64) {
    let h = hex.trim_start_matches('#');
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(0) as f64;
    (p(0), p(2), p(4))
}

/// `a` over `b` at `t` (0 = all `b`, 1 = all `a`).
fn mix(a: &str, b: &str, t: f64) -> String {
    let (ar, ag, ab) = rgb(a);
    let (br, bg, bb) = rgb(b);
    let c = |x: f64, y: f64| (x * t + y * (1.0 - t)).round().clamp(0.0, 255.0) as u8;
    format!("#{:02x}{:02x}{:02x}", c(ar, br), c(ag, bg), c(ab, bb))
}

/// Theme names, in the order the window lists them.
pub const THEMES: &[&str] = &["aurora", "slate", "ocean", "mono", "ember"];

pub fn palette(theme: &str) -> Palette {
    let (accent, dark_a, dark_b, light) = match theme {
        "slate" => ("#475569", "#0f172a", "#1e293b", "#f8fafc"),
        "ocean" => ("#0ea5e9", "#082f49", "#0c4a6e", "#f0f9ff"),
        "mono" => ("#18181b", "#111111", "#1c1c1c", "#ffffff"),
        "ember" => ("#f97316", "#1c1917", "#2d1f14", "#fff7ed"),
        _ => ("#6366f1", "#1a1a2e", "#2b2b52", "#ffffff"),
    };
    let dark_mid = mix(dark_a, dark_b, 0.5);
    Palette {
        accent: accent.into(),
        dark_a: dark_a.into(),
        dark_b: dark_b.into(),
        light: light.into(),
        section_b: mix(accent, "#000000", 0.55),
        ink: "#0f172a".into(),
        body: "#334155".into(),
        on_dark: "#ffffff".into(),
        sub_on_dark: mix("#ffffff", &dark_mid, 0.72),
        attrib_on_dark: mix("#ffffff", &dark_mid, 0.6),
    }
}

// ── layouts ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Background {
    Solid(&'static str),
    /// CSS angle convention (`180` = top to bottom).
    Gradient { from: &'static str, to: &'static str, angle: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VAlign {
    Top,
    Middle,
    Bottom,
}

impl VAlign {
    pub fn name(self) -> &'static str {
        match self {
            VAlign::Top => "top",
            VAlign::Middle => "middle",
            VAlign::Bottom => "bottom",
        }
    }
}

/// The paragraph-style family a text box writes: the `.odp` reader tells the
/// slide's fields apart by these prefixes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Role {
    Title,
    Sub,
    Body,
    Attrib,
    /// Decoration (the quote mark): never read back.
    Mark,
}

impl Role {
    pub fn prefix(self) -> &'static str {
        match self {
            Role::Title => "PTitle",
            Role::Sub => "PSub",
            Role::Body => "PBody",
            Role::Attrib => "PAttrib",
            Role::Mark => "PMark",
        }
    }
}

/// One text box. `field` names the slide field it shows (`title`,
/// `subtitle`, `body`, `attribution`, `bullets`, `columns.0`, `columns.1`);
/// a decoration has `literal` text and shows only when `with` is not empty.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextBox {
    pub field: &'static str,
    pub literal: Option<&'static str>,
    pub with: Option<&'static str>,
    /// Shape name in the file (LibreOffice's navigator shows it).
    pub name: &'static str,
    pub role: Role,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub size: f64,
    pub weight: u16,
    /// Line height as a multiple of the size.
    pub line: f64,
    /// Letter spacing in em.
    pub tracking: f64,
    pub color: &'static str,
    pub valign: VAlign,
    pub center: bool,
    pub display: bool,
    pub list: bool,
    /// Space after each paragraph, px.
    pub gap: f64,
    pub shrink: bool,
}

const fn tb(field: &'static str, name: &'static str, role: Role, x: f64, y: f64, w: f64, h: f64) -> TextBox {
    TextBox {
        field,
        literal: None,
        with: None,
        name,
        role,
        x,
        y,
        w,
        h,
        size: 19.0,
        weight: 400,
        line: 1.35,
        tracking: 0.0,
        color: "body",
        valign: VAlign::Top,
        center: false,
        display: false,
        list: false,
        gap: 0.0,
        shrink: false,
    }
}

/// An accent bar, drawn only when field `with` has text (the editor always
/// draws it, as part of the empty field it introduces).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rule {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub radius: f64,
    pub color: &'static str,
    pub with: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutSpec {
    pub background: Background,
    /// Whether the text sits on a dark background (style names say so).
    pub dark: bool,
    pub rules: Vec<Rule>,
    pub texts: Vec<TextBox>,
}

fn content_title() -> TextBox {
    TextBox {
        size: 32.0,
        weight: 700,
        line: 1.1,
        tracking: -0.01,
        color: "ink",
        valign: VAlign::Bottom,
        display: true,
        ..tb("title", "Title", Role::Title, 52.0, 16.0, 536.0, 70.0)
    }
}

fn content_rule() -> Rule {
    Rule { x: 52.0, y: 96.0, w: 46.0, h: 5.0, radius: 2.5, color: "accent", with: "title" }
}

fn bullets(field: &'static str, name: &'static str, x: f64, w: f64) -> TextBox {
    TextBox { list: true, gap: 9.0, shrink: true, ..tb(field, name, Role::Body, x, 120.0, w, 212.0) }
}

/// The geometry of one layout (unknown names get `content`).
pub fn layout(name: &str) -> LayoutSpec {
    let dark = Background::Gradient { from: "darkA", to: "darkB", angle: 150.0 };
    match name {
        "title" => LayoutSpec {
            background: dark,
            dark: true,
            rules: vec![Rule { x: 64.0, y: 202.0, w: 64.0, h: 6.0, radius: 3.0, color: "accent", with: "title" }],
            texts: vec![
                TextBox {
                    size: 46.0,
                    weight: 800,
                    line: 1.05,
                    tracking: -0.02,
                    color: "onDark",
                    valign: VAlign::Bottom,
                    display: true,
                    ..tb("title", "Title", Role::Title, 64.0, 40.0, 512.0, 150.0)
                },
                TextBox {
                    size: 22.0,
                    weight: 500,
                    line: 1.3,
                    color: "subOnDark",
                    shrink: true,
                    ..tb("subtitle", "Subtitle", Role::Sub, 64.0, 226.0, 512.0, 96.0)
                },
            ],
        },
        "section" => LayoutSpec {
            background: Background::Gradient { from: "accent", to: "sectionB", angle: 140.0 },
            dark: true,
            rules: vec![],
            texts: vec![TextBox {
                size: 54.0,
                weight: 800,
                line: 1.05,
                tracking: -0.02,
                color: "onDark",
                valign: VAlign::Middle,
                display: true,
                shrink: true,
                ..tb("title", "Title", Role::Title, 64.0, 60.0, 512.0, 240.0)
            }],
        },
        "two-column" => LayoutSpec {
            background: Background::Solid("light"),
            dark: false,
            rules: vec![content_rule()],
            texts: vec![
                content_title(),
                bullets("columns.0", "Left column", 52.0, 250.0),
                bullets("columns.1", "Right column", 338.0, 250.0),
            ],
        },
        "quote" => LayoutSpec {
            background: dark,
            dark: true,
            rules: vec![],
            texts: vec![
                TextBox {
                    literal: Some("\u{201C}"),
                    with: Some("body"),
                    size: 54.0,
                    weight: 600,
                    line: 1.0,
                    color: "accent",
                    valign: VAlign::Bottom,
                    display: true,
                    ..tb("", "Quote mark", Role::Mark, 64.0, 52.0, 80.0, 60.0)
                },
                TextBox {
                    size: 30.0,
                    weight: 600,
                    line: 1.4,
                    tracking: -0.01,
                    color: "onDark",
                    display: true,
                    shrink: true,
                    ..tb("body", "Quote", Role::Body, 64.0, 118.0, 512.0, 150.0)
                },
                TextBox {
                    size: 16.0,
                    line: 1.3,
                    color: "attribOnDark",
                    ..tb("attribution", "Attribution", Role::Attrib, 64.0, 280.0, 512.0, 40.0)
                },
            ],
        },
        "blank" => LayoutSpec {
            background: Background::Solid("light"),
            dark: false,
            rules: vec![],
            texts: vec![TextBox {
                size: 24.0,
                valign: VAlign::Middle,
                center: true,
                shrink: true,
                ..tb("body", "Text", Role::Body, 52.0, 36.0, 536.0, 288.0)
            }],
        },
        _ => LayoutSpec {
            background: Background::Solid("light"),
            dark: false,
            rules: vec![content_rule()],
            texts: vec![content_title(), bullets("bullets", "Bullets", 52.0, 536.0)],
        },
    }
}

/// Layout names, in the order the window lists them.
pub const LAYOUTS: &[&str] = &["title", "section", "content", "two-column", "quote", "blank"];

// ── the editor's copy ────────────────────────────────────────────────────

fn background_json(bg: Background) -> Value {
    match bg {
        Background::Solid(c) => json!({ "kind": "solid", "color": c }),
        Background::Gradient { from, to, angle } => json!({ "kind": "gradient", "from": from, "to": to, "angle": angle }),
    }
}

fn text_json(t: &TextBox) -> Value {
    let mut o = Map::new();
    o.insert("field".into(), json!(t.field));
    if let Some(l) = t.literal {
        o.insert("literal".into(), json!(l));
    }
    if let Some(w) = t.with {
        o.insert("with".into(), json!(w));
    }
    for (k, v) in [
        ("x", t.x),
        ("y", t.y),
        ("w", t.w),
        ("h", t.h),
        ("size", t.size),
        ("line", t.line),
        ("tracking", t.tracking),
        ("gap", t.gap),
    ] {
        o.insert(k.into(), json!(v));
    }
    o.insert("weight".into(), json!(t.weight));
    o.insert("color".into(), json!(t.color));
    o.insert("valign".into(), json!(t.valign.name()));
    o.insert("center".into(), json!(t.center));
    o.insert("display".into(), json!(t.display));
    o.insert("list".into(), json!(t.list));
    o.insert("shrink".into(), json!(t.shrink));
    Value::Object(o)
}

/// Everything the editor needs to draw a slide exactly as the writer does.
pub fn spec_json() -> Value {
    let layouts: Map<String, Value> = LAYOUTS
        .iter()
        .map(|name| {
            let l = layout(name);
            (
                (*name).to_string(),
                json!({
                    "background": background_json(l.background),
                    "dark": l.dark,
                    "rules": l.rules.iter().map(|r| json!({
                        "x": r.x, "y": r.y, "w": r.w, "h": r.h, "radius": r.radius,
                        "color": r.color, "with": r.with,
                    })).collect::<Vec<_>>(),
                    "texts": l.texts.iter().map(text_json).collect::<Vec<_>>(),
                }),
            )
        })
        .collect();
    let themes: Map<String, Value> = THEMES.iter().map(|t| ((*t).to_string(), palette(t).to_json())).collect();
    json!({
        "page": { "w": PAGE_PX.0, "h": PAGE_PX.1 },
        "fonts": { "display": FONT_DISPLAY, "text": FONT_TEXT, "fallback": FONT_FALLBACK },
        "bullet": { "char": BULLET_CHAR, "relative": BULLET_RELATIVE, "indent": BULLET_INDENT_PX },
        "minFit": MIN_FIT,
        "themes": themes,
        "layouts": layouts,
    })
}

/// `plugins/impress/web/slide-spec.js`: the spec as an ES module.
pub fn spec_js() -> String {
    format!(
        "// Generated from crates/shiny-plugin-sdk/src/odp_spec.rs — do not edit.\n\
         // Regenerate: UPDATE_SLIDE_SPEC=1 cargo test -p shiny-plugin-sdk odp_spec\n\
         export default {};\n",
        serde_json::to_string_pretty(&spec_json()).unwrap_or_else(|_| "{}".into())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_convert() {
        assert!((px_to_cm(640.0) - 28.0).abs() < 1e-9);
        assert!((px_to_pt(640.0) - 793.7).abs() < 0.1);
        assert_eq!(cm(32.0), "1.400cm");
    }

    #[test]
    fn colours_mix() {
        assert_eq!(mix("#ffffff", "#000000", 0.5), "#808080");
        assert_eq!(palette("aurora").section_b, mix("#6366f1", "#000000", 0.55));
    }

    #[test]
    fn every_box_is_on_the_page() {
        for name in LAYOUTS {
            for t in layout(name).texts {
                assert!(t.x >= 0.0 && t.y >= 0.0 && t.x + t.w <= PAGE_PX.0 && t.y + t.h <= PAGE_PX.1, "{name}/{}", t.name);
            }
        }
    }

    /// The editor's copy matches this file. `UPDATE_SLIDE_SPEC=1` rewrites it.
    #[test]
    fn odp_spec_web_copy_is_current() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../plugins/impress/web/slide-spec.js");
        let want = spec_js();
        if std::env::var_os("UPDATE_SLIDE_SPEC").is_some() {
            std::fs::write(path, &want).unwrap();
        }
        let have = std::fs::read_to_string(path).unwrap_or_default();
        assert!(have == want, "plugins/impress/web/slide-spec.js is stale: run UPDATE_SLIDE_SPEC=1 cargo test -p shiny-plugin-sdk odp_spec");
    }
}
