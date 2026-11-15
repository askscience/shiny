//! Minimal OpenDocument Presentation (ODP) codec — slides ↔ `.odp` bytes.
//!
//! ODP (`.odp`) is the ISO-standard open presentation format used by
//! LibreOffice Impress / OpenOffice. An `.odp` is a ZIP containing a
//! `mimetype` entry (first, uncompressed), `META-INF/manifest.xml`, and
//! `content.xml` whose body is `<office:presentation>` with one `<draw:page>`
//! per slide.
//!
//! This codec is deliberately small but valid, and fully self-contained —
//! no office suite is involved. Each slide becomes a `<draw:page>` in one of
//! six layouts (title / section / content / two-column / quote / blank).
//! How a layout looks — every box, type size, colour, accent rule and
//! background — is [`crate::odp_spec`], the same description the Impress
//! window draws from, so the file looks like the editor. Backgrounds are page
//! fills (gradients in `styles.xml`), bullets are real `text:list`s, and
//! speaker notes are `presentation:notes`. LibreOffice Impress opens the
//! files produced here directly.
//!
//! The layout is round-tripped through the ODF-standard `presentation:class`
//! attribute (`title` / `subtitle` / `outline` / `text` / `blank`) plus frame
//! structure, and text roles are recovered from the paragraph style names we
//! emit (`PTitle*` / `PSub*` / `PBody*` / `PAttrib*`).
//!
//! **Known limitations:** the bullet reveal mode (`Slide::reveal`) is
//! app-side only, and the shrink scale (`Slide::fit`) is not read back (the
//! editor measures it again). Slide transitions are written (and read back)
//! as ODF `presentation:transition-style` on the page's drawing-page style;
//! speaker notes round-trip.
//!
//! Both the core binary (REST import/export for the impress plugin) and plugin
//! code link this module — no runtime state crosses the dlopen boundary
//! because everything is plain bytes.

use std::io::{Cursor, Write};

use serde::{Deserialize, Serialize};

use crate::errors::AppError;

const MIMETYPE: &str = "application/vnd.oasis.opendocument.presentation";

// Namespace URIs used for roxmltree tag/attribute lookups (the `(uri, local)`
// tuple form).
const NS_OFFICE: &str = "urn:oasis:names:tc:opendocument:xmlns:office:1.0";
const NS_DRAW: &str = "urn:oasis:names:tc:opendocument:xmlns:drawing:1.0";
const NS_TEXT: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
const NS_PRESENTATION: &str = "urn:oasis:names:tc:opendocument:xmlns:presentation:1.0";
const NS_STYLE: &str = "urn:oasis:names:tc:opendocument:xmlns:style:1.0";

/// Bullet glyph emitted before each bullet line (also stripped on import).
const BULLET: &str = "• ";

// ─────────────────────────────────────────────────────────────
// Slide model
// ─────────────────────────────────────────────────────────────

/// One slide in a presentation. All fields are optional except `layout`,
/// which defaults to "content". The LLM writes this shape through the
/// `slide_*` tools; the core `presentations` table stores it as JSON.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Slide {
    #[serde(default = "default_layout")]
    pub layout: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    #[serde(default)]
    pub bullets: Vec<String>,
    #[serde(default)]
    pub columns: Vec<Vec<String>>,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub attribution: String,
    #[serde(default)]
    pub notes: String,
    /// Transition played when this slide enters: see `TRANSITIONS`.
    /// `"none"` (the default) means the slide just cuts in.
    #[serde(default = "default_transition")]
    pub transition: String,
    /// How the bullets arrive: `"all"` (default, together with the slide) or
    /// `"bullets"` (one more bullet per advance). App-side only — see the
    /// module docs.
    #[serde(default = "default_reveal")]
    pub reveal: String,
    /// Shrink-on-overflow scale of the slide's shrinkable text boxes (1 = as
    /// designed, down to `odp_spec::MIN_FIT`). The Impress editor measures it;
    /// the writer multiplies those boxes' type sizes by it.
    #[serde(default = "default_fit")]
    pub fit: f64,
}

fn default_fit() -> f64 {
    1.0
}

fn default_layout() -> String {
    "content".into()
}

fn default_transition() -> String {
    "none".into()
}

fn default_reveal() -> String {
    "all".into()
}

/// Known slide transitions. `none` (a hard cut) is the default.
pub const TRANSITIONS: &[&str] = &["none", "fade", "slide", "push", "zoom"];

/// Normalise a transition name to a known value ("none" when unknown).
pub fn normalize_transition(transition: &str) -> String {
    let t = transition.trim().to_ascii_lowercase();
    if TRANSITIONS.contains(&t.as_str()) {
        t
    } else {
        "none".into()
    }
}

/// Known bullet-reveal modes. `all` (bullets arrive with the slide) is default.
pub const REVEALS: &[&str] = &["all", "bullets"];

/// Normalise a reveal mode to a known value ("all" when unknown).
pub fn normalize_reveal(reveal: &str) -> String {
    let r = reveal.trim().to_ascii_lowercase();
    if REVEALS.contains(&r.as_str()) {
        r
    } else {
        "all".into()
    }
}

/// Known layout names. Unknown values are normalised to "content".
pub const LAYOUTS: &[&str] = &["title", "section", "content", "two-column", "quote", "blank"];

/// Normalise a layout string to a known value ("content" when unknown).
pub fn normalize_layout(layout: &str) -> String {
    let l = layout.trim().to_ascii_lowercase();
    if LAYOUTS.contains(&l.as_str()) {
        l
    } else {
        "content".into()
    }
}

/// Known theme names. Unknown values fall back to "aurora".
pub const THEMES: &[&str] = &["aurora", "slate", "ocean", "mono", "ember"];

pub fn normalize_theme(theme: &str) -> String {
    let t = theme.trim().to_ascii_lowercase();
    if THEMES.contains(&t.as_str()) {
        t
    } else {
        "aurora".into()
    }
}

// ─────────────────────────────────────────────────────────────
// slides → .odp
// ─────────────────────────────────────────────────────────────
//
// Everything visual comes from `odp_spec`: the same boxes, sizes, colours,
// rules and backgrounds the Impress window draws. The package is a complete
// ODF 1.2 presentation: `mimetype`, `manifest.xml`, `meta.xml`, `styles.xml`
// (page layout, master page, gradients, fonts) and `content.xml` (one
// `draw:page` per slide, with its speaker notes).

use crate::odp_spec::{self as spec, Background, LayoutSpec, Palette, TextBox, VAlign};

/// Build a valid `.odp` file from a list of slides.
pub fn slides_to_odp(theme: &str, slides: &[Slide]) -> Result<Vec<u8>, AppError> {
    let palette = spec::palette(&normalize_theme(theme));
    let mut styles = StyleSet::default();
    let body = body_xml(slides, &mut styles);
    let content = format!(
        "{CONTENT_HEADER}{fonts}<office:automatic-styles>{auto}</office:automatic-styles>\
         <office:body><office:presentation>{body}</office:presentation></office:body></office:document-content>",
        fonts = font_face_decls(),
        auto = styles.xml(&palette),
    );

    let err = |e: &dyn std::fmt::Display| AppError::Internal(format!("ODP write failed: {e}"));
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));

    // `mimetype` MUST be the first entry and MUST be stored uncompressed.
    let stored = zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("mimetype", stored).map_err(|e| err(&e))?;
    zip.write_all(MIMETYPE.as_bytes()).map_err(|e| err(&e))?;

    let deflated = zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in [
        ("META-INF/manifest.xml", MANIFEST.to_string()),
        ("meta.xml", META.to_string()),
        ("styles.xml", styles_xml(&palette)),
        ("content.xml", content),
    ] {
        zip.start_file(name, deflated).map_err(|e| err(&e))?;
        zip.write_all(data.as_bytes()).map_err(|e| err(&e))?;
    }

    Ok(zip.finish().map_err(|e| err(&e))?.into_inner())
}

const NAMESPACES: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
  xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"
  xmlns:presentation="urn:oasis:names:tc:opendocument:xmlns:presentation:1.0"
  xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
  xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"
  xmlns:xlink="http://www.w3.org/1999/xlink"
  xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0"
  xmlns:dc="http://purl.org/dc/elements/1.1/""#;

const CONTENT_HEADER: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-content\n  ",
    r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
  xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"
  xmlns:presentation="urn:oasis:names:tc:opendocument:xmlns:presentation:1.0"
  xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
  xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"
  xmlns:xlink="http://www.w3.org/1999/xlink"
  office:version="1.2">
"#
);

const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2">
 <manifest:file-entry manifest:full-path="/" manifest:version="1.2" manifest:media-type="application/vnd.oasis.opendocument.presentation"/>
 <manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>
 <manifest:file-entry manifest:full-path="styles.xml" manifest:media-type="text/xml"/>
 <manifest:file-entry manifest:full-path="meta.xml" manifest:media-type="text/xml"/>
</manifest:manifest>
"#;

const META: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-meta xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0" office:version="1.2"><office:meta><meta:generator>LiteAura Impress</meta:generator></office:meta></office:document-meta>
"#;

fn font_face_decls() -> String {
    let face = |name: &str| {
        format!(
            "<style:font-face style:name=\"{name}\" svg:font-family=\"&apos;{name}&apos;\" style:font-family-generic=\"swiss\" style:font-pitch=\"variable\"/>"
        )
    };
    format!(
        "<office:font-face-decls>{}{}{}</office:font-face-decls>",
        face(spec::FONT_DISPLAY),
        face(spec::FONT_TEXT),
        face(spec::FONT_FALLBACK)
    )
}

/// CSS angle (clockwise, 180 = downwards) → ODF 1.2 `draw:angle` (tenths of
/// a degree, counter-clockwise, 0 = downwards).
fn odf_angle(css: f64) -> i64 {
    (((180.0 - css) % 360.0 + 360.0) % 360.0 * 10.0).round() as i64
}

/// The gradients a deck's backgrounds use, named by their colour pair.
fn gradient_name(from: &str, to: &str) -> String {
    format!("g-{from}-{to}")
}

fn styles_xml(p: &Palette) -> String {
    let mut gradients = String::new();
    let mut seen = Vec::new();
    for name in spec::LAYOUTS {
        if let Background::Gradient { from, to, angle } = spec::layout(name).background {
            let id = gradient_name(from, to);
            if seen.contains(&id) {
                continue;
            }
            gradients.push_str(&format!(
                "<draw:gradient draw:name=\"{id}\" draw:style=\"linear\" draw:start-color=\"{}\" draw:end-color=\"{}\" \
                 draw:start-intensity=\"100%\" draw:end-intensity=\"100%\" draw:angle=\"{}\" draw:border=\"0%\"/>",
                p.get(from),
                p.get(to),
                odf_angle(angle)
            ));
            seen.push(id);
        }
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-styles {NAMESPACES} office:version=\"1.2\">{fonts}\
         <office:styles>{gradients}\
         <style:default-style style:family=\"graphic\"><style:graphic-properties draw:stroke=\"none\"/>\
         <style:text-properties style:font-name=\"{text}\" fo:font-family=\"&apos;{text}&apos;\" fo:font-size=\"18pt\"/></style:default-style>\
         </office:styles>\
         <office:automatic-styles>\
         <style:page-layout style:name=\"PM1\"><style:page-layout-properties fo:margin-top=\"0cm\" fo:margin-bottom=\"0cm\" \
         fo:margin-left=\"0cm\" fo:margin-right=\"0cm\" fo:page-width=\"{w}cm\" fo:page-height=\"{h}cm\" style:print-orientation=\"landscape\"/></style:page-layout>\
         <style:style style:name=\"Mdp1\" style:family=\"drawing-page\"><style:drawing-page-properties draw:background-size=\"full\" draw:fill=\"solid\" draw:fill-color=\"{light}\"/></style:style>\
         </office:automatic-styles>\
         <office:master-styles><style:master-page style:name=\"Default\" style:page-layout-name=\"PM1\" draw:style-name=\"Mdp1\"/></office:master-styles>\
         </office:document-styles>",
        fonts = font_face_decls(),
        text = spec::FONT_TEXT,
        w = spec::PAGE_CM.0,
        h = spec::PAGE_CM.1,
        light = p.light,
    )
}

/// The automatic styles a deck needs, collected while its pages are written.
#[derive(Default)]
struct StyleSet {
    /// (background key, transition) → drawing-page style name.
    pages: Vec<(String, String, String)>,
    /// Paragraph style name → (box, fit, dark).
    paragraphs: Vec<(String, TextBox, f64)>,
    list: bool,
}

impl StyleSet {
    fn page(&mut self, bg: &str, transition: &str) -> String {
        if let Some((_, _, name)) = self.pages.iter().find(|(b, t, _)| b == bg && t == transition) {
            return name.clone();
        }
        let name = format!("dp{}", self.pages.len() + 1);
        self.pages.push((bg.into(), transition.into(), name.clone()));
        name
    }

    /// `PTitleOnDark-title-100`: the role prefix (read back on import), the
    /// background, the layout and the fit, so each distinct look is one style.
    fn paragraph(&mut self, layout: &str, dark: bool, t: &TextBox, fit: f64) -> String {
        let fit = if t.shrink { fit } else { 1.0 };
        let name = format!(
            "{}On{}-{}-{}-{}",
            t.role.prefix(),
            if dark { "Dark" } else { "Light" },
            layout,
            t.name.replace(' ', ""),
            (fit * 100.0).round() as i64
        );
        if !self.paragraphs.iter().any(|(n, _, _)| *n == name) {
            self.paragraphs.push((name.clone(), *t, fit));
        }
        if t.list {
            self.list = true;
        }
        name
    }

    fn xml(&self, p: &Palette) -> String {
        let mut s = String::new();
        for (bg, transition, name) in &self.pages {
            let fill = match bg.split_once(':') {
                Some(("g", pair)) => {
                    let (from, to) = pair.split_once('/').unwrap_or(("darkA", "darkB"));
                    format!("draw:fill=\"gradient\" draw:fill-gradient-name=\"{}\"", gradient_name(from, to))
                }
                _ => format!("draw:fill=\"solid\" draw:fill-color=\"{}\"", p.get(bg)),
            };
            let trans = match odf_transition_style(transition) {
                Some(style) => format!(
                    " presentation:transition-type=\"manual\" presentation:transition-style=\"{style}\" presentation:transition-speed=\"medium\""
                ),
                None => String::new(),
            };
            s.push_str(&format!(
                "<style:style style:name=\"{name}\" style:family=\"drawing-page\"><style:drawing-page-properties \
                 presentation:background-visible=\"true\" presentation:background-objects-visible=\"true\" \
                 draw:background-size=\"full\" {fill}{trans}/></style:style>"
            ));
        }
        for v in [VAlign::Top, VAlign::Middle, VAlign::Bottom] {
            s.push_str(&format!(
                "<style:style style:name=\"grText{n}\" style:family=\"graphic\"><style:graphic-properties draw:stroke=\"none\" \
                 draw:fill=\"none\" draw:textarea-vertical-align=\"{v}\" draw:auto-grow-height=\"false\" draw:auto-grow-width=\"false\" \
                 fo:min-height=\"0cm\" fo:padding-top=\"0cm\" fo:padding-bottom=\"0cm\" fo:padding-left=\"0cm\" fo:padding-right=\"0cm\" \
                 fo:wrap-option=\"wrap\"/></style:style>",
                n = v.name(),
                v = v.name()
            ));
        }
        s.push_str(&format!(
            "<style:style style:name=\"grRule\" style:family=\"graphic\"><style:graphic-properties draw:stroke=\"none\" \
             draw:fill=\"solid\" draw:fill-color=\"{}\"/></style:style>",
            p.accent
        ));
        for (name, t, fit) in &self.paragraphs {
            let size = t.size * fit;
            let font = if t.display { spec::FONT_DISPLAY } else { spec::FONT_TEXT };
            let tracking = if t.tracking != 0.0 {
                format!(" fo:letter-spacing=\"{}\"", spec::cm(t.tracking * size))
            } else {
                String::new()
            };
            s.push_str(&format!(
                "<style:style style:name=\"{name}\" style:family=\"paragraph\"><style:paragraph-properties fo:margin-top=\"0cm\" \
                 fo:margin-bottom=\"{gap}\" fo:margin-left=\"0cm\" fo:text-indent=\"0cm\" fo:line-height=\"{lh}\" fo:text-align=\"{align}\"/>\
                 <style:text-properties style:font-name=\"{font}\" fo:font-family=\"&apos;{font}&apos;\" style:font-family-generic=\"swiss\" \
                 fo:font-size=\"{fs}\" fo:font-weight=\"{weight}\" fo:color=\"{color}\"{tracking}/></style:style>",
                gap = spec::cm(t.gap * fit),
                lh = spec::cm(size * t.line),
                align = if t.center { "center" } else { "start" },
                fs = spec::pt(size),
                weight = t.weight,
                color = p.get(t.color),
            ));
        }
        if self.list {
            s.push_str(&format!(
                "<text:list-style style:name=\"LBullet\"><text:list-level-style-bullet text:level=\"1\" text:bullet-char=\"{ch}\" \
                 text:bullet-relative-size=\"{rel}%\"><style:list-level-properties text:space-before=\"0cm\" text:min-label-width=\"{indent}\"/>\
                 <style:text-properties fo:font-family=\"&apos;{fb}&apos;\" style:use-window-font-color=\"false\" fo:color=\"{accent}\"/>\
                 </text:list-level-style-bullet></text:list-style>",
                ch = spec::BULLET_CHAR,
                rel = (spec::BULLET_RELATIVE * 100.0).round() as i64,
                indent = spec::cm(spec::BULLET_INDENT_PX),
                fb = spec::FONT_FALLBACK,
                accent = p.accent,
            ));
        }
        s
    }
}

/// Map an app transition to an ODF 1.2 `presentation:transition-style` value.
/// ODF has no cross-fade, no "push" and no true zoom, so those fall back to
/// their nearest standard effect (`dissolve`, `move-from-right`,
/// `fade-from-center`). `none` maps to `None` (no properties element).
fn odf_transition_style(transition: &str) -> Option<&'static str> {
    match normalize_transition(transition).as_str() {
        "fade" => Some("dissolve"),
        "slide" | "push" => Some("move-from-right"),
        "zoom" => Some("fade-from-center"),
        _ => None,
    }
}

/// Map an ODF `presentation:transition-style` back to an app transition.
/// Foreign effects land on the closest app equivalent (`fade` when we simply
/// don't know it); a plain `none`/missing style yields `None` so the caller
/// keeps the `"none"` default.
fn app_transition(style: &str) -> Option<&'static str> {
    let s = style.trim().to_ascii_lowercase();
    if s.is_empty() || s == "none" {
        None
    } else if s == "dissolve" || s.starts_with("fade-over") {
        Some("fade")
    } else if s.starts_with("fade-from-center") || s.starts_with("fade-to-center") {
        Some("zoom")
    } else if s.starts_with("move-from") || s.starts_with("uncover-to") {
        Some("slide")
    } else {
        Some("fade")
    }
}

/// Map a layout name to the `presentation:class` the reader recognises.
fn layout_class(layout: &str) -> &'static str {
    match normalize_layout(layout).as_str() {
        "title" => "title",
        "section" => "subtitle",
        "quote" => "text",
        "blank" => "blank",
        _ => "outline", // content + two-column are bullet outlines
    }
}

fn body_xml(slides: &[Slide], styles: &mut StyleSet) -> String {
    let mut body = String::new();
    for (i, slide) in slides.iter().enumerate() {
        body.push_str(&page_xml(slide, i + 1, styles));
    }
    body
}

fn background_key(bg: Background) -> String {
    match bg {
        Background::Solid(c) => c.to_string(),
        Background::Gradient { from, to, .. } => format!("g:{from}/{to}"),
    }
}

/// The text a box shows for a slide, as paragraphs.
fn field_paragraphs(slide: &Slide, t: &TextBox) -> Vec<String> {
    let clean = |v: &[String]| v.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect::<Vec<_>>();
    let lines = |s: &str| {
        let s = s.trim_matches('\n');
        if s.trim().is_empty() {
            Vec::new()
        } else {
            s.split('\n').map(|l| l.trim_end().to_string()).collect()
        }
    };
    match t.field {
        "title" => lines(&slide.title.replace('\n', " ")),
        "subtitle" => lines(&slide.subtitle),
        "body" => lines(&slide.body),
        "attribution" => lines(&slide.attribution),
        "bullets" => clean(&slide.bullets),
        "columns.0" => clean(slide.columns.first().map(Vec::as_slice).unwrap_or(&[])),
        "columns.1" => clean(slide.columns.get(1).map(Vec::as_slice).unwrap_or(&[])),
        _ => Vec::new(),
    }
}

fn field_filled(slide: &Slide, field: &str) -> bool {
    match field {
        "title" => !slide.title.trim().is_empty(),
        "subtitle" => !slide.subtitle.trim().is_empty(),
        "body" => !slide.body.trim().is_empty(),
        "attribution" => !slide.attribution.trim().is_empty(),
        _ => false,
    }
}

fn page_xml(slide: &Slide, index: usize, styles: &mut StyleSet) -> String {
    let layout_name = normalize_layout(&slide.layout);
    let spec: LayoutSpec = spec::layout(&layout_name);
    let name = if slide.title.trim().is_empty() {
        format!("Slide {index}")
    } else {
        format!("Slide {index} — {}", slide.title.trim())
    };
    let page_style = styles.page(&background_key(spec.background), &normalize_transition(&slide.transition));
    let mut page = format!(
        "<draw:page draw:name=\"{}\" draw:style-name=\"{page_style}\" draw:master-page-name=\"Default\" presentation:class=\"{}\">",
        xml_escape_attr(&name),
        layout_class(&layout_name)
    );

    for r in &spec.rules {
        if field_filled(slide, r.with) {
            page.push_str(&format!(
                "<draw:rect draw:name=\"Accent\" draw:style-name=\"grRule\" draw:corner-radius=\"{}\" svg:x=\"{}\" svg:y=\"{}\" svg:width=\"{}\" svg:height=\"{}\"/>",
                spec::cm(r.radius),
                spec::cm(r.x),
                spec::cm(r.y),
                spec::cm(r.w),
                spec::cm(r.h)
            ));
        }
    }

    let fit = slide.fit.clamp(spec::MIN_FIT, 1.0);
    for t in &spec.texts {
        let paras = match t.literal {
            Some(text) if t.with.is_some_and(|w| field_filled(slide, w)) => vec![text.to_string()],
            Some(_) => continue,
            None => field_paragraphs(slide, t),
        };
        // Columns keep their (possibly empty) frames: the reader counts them.
        if paras.is_empty() && !t.field.starts_with("columns.") {
            continue;
        }
        let pstyle = styles.paragraph(&layout_name, spec.dark, t, fit);
        let mut text = String::new();
        for para in &paras {
            text.push_str(&format!("<text:p text:style-name=\"{pstyle}\">{}</text:p>", xml_escape(para)));
        }
        if t.list && !paras.is_empty() {
            let items: String = paras
                .iter()
                .map(|para| format!("<text:list-item><text:p text:style-name=\"{pstyle}\">{}</text:p></text:list-item>", xml_escape(para)))
                .collect();
            text = format!("<text:list text:style-name=\"LBullet\">{items}</text:list>");
        }
        page.push_str(&format!(
            "<draw:frame draw:name=\"{}\" draw:style-name=\"grText{}\" presentation:style-name=\"{}\" svg:x=\"{}\" svg:y=\"{}\" svg:width=\"{}\" svg:height=\"{}\"><draw:text-box>{text}</draw:text-box></draw:frame>",
            t.name,
            t.valign.name(),
            t.role.prefix(),
            spec::cm(t.x),
            spec::cm(t.y),
            spec::cm(t.w),
            spec::cm(t.h)
        ));
    }

    let notes: Vec<&str> = slide.notes.split('\n').collect();
    if !slide.notes.trim().is_empty() {
        let paras: String = notes.iter().map(|l| format!("<text:p>{}</text:p>", xml_escape(l.trim_end()))).collect();
        page.push_str(&format!(
            "<presentation:notes><draw:page-thumbnail presentation:class=\"page\" svg:x=\"2cm\" svg:y=\"2cm\" svg:width=\"17cm\" svg:height=\"9.6cm\"/>\
             <draw:frame presentation:class=\"notes\" svg:x=\"2cm\" svg:y=\"12.5cm\" svg:width=\"17cm\" svg:height=\"13cm\"><draw:text-box>{paras}</draw:text-box></draw:frame></presentation:notes>"
        ));
    }

    page.push_str("</draw:page>");
    page
}

// ─────────────────────────────────────────────────────────────
// .odp → slides
// ─────────────────────────────────────────────────────────────

/// Read slides back out of a `.odp` file. Handles our own writer losslessly
/// (layout + title/subtitle/bullets/columns/body/attribution + transition) and
/// makes a best-effort attempt at foreign LibreOffice files (title + bullets).
pub fn odp_to_slides(odp: &[u8]) -> Result<Vec<Slide>, AppError> {
    let xml = read_content_xml(odp)?;
    let doc = roxmltree::Document::parse(&xml)
        .map_err(|e| AppError::BadRequest(format!("Invalid ODP content.xml: {}", e)))?;

    let mut slides = Vec::new();
    let presentation = doc
        .descendants()
        .find(|n| n.has_tag_name((NS_OFFICE, "presentation")) && n.is_element());

    let Some(presentation) = presentation else {
        return Ok(slides); // empty deck
    };

    // `draw:style-name` on a page points at a drawing-page style that holds the
    // transition (ODF 1.2). Collect them once up front.
    let transitions = drawing_page_transitions(&doc);

    for page in presentation.children() {
        if !page.has_tag_name((NS_DRAW, "page")) {
            continue;
        }
        // Skip speaker-notes pages (we don't emit them, but foreign files do).
        if page.attribute((NS_PRESENTATION, "class")) == Some("notes") {
            continue;
        }
        // Prefer the transition on the referenced drawing-page style; older
        // files (and some LibreOffice exports) put it straight on the page.
        let style = page
            .attribute((NS_DRAW, "style-name"))
            .and_then(|name| transitions.get(name).cloned())
            .or_else(|| page.attribute((NS_PRESENTATION, "transition-style")).map(str::to_string))
            .unwrap_or_default();
        let mut slide = page_to_slide(&page);
        slide.notes = page_notes(&page);
        if let Some(app) = app_transition(&style) {
            slide.transition = app.to_string();
        }
        slides.push(slide);
    }

    Ok(slides)
}

/// The speaker notes of a page: the `notes` frame of its `presentation:notes`.
fn page_notes(page: &roxmltree::Node) -> String {
    let Some(notes) = page.children().find(|c| c.has_tag_name((NS_PRESENTATION, "notes"))) else {
        return String::new();
    };
    let mut lines = Vec::new();
    for frame in notes.children().filter(|c| c.has_tag_name((NS_DRAW, "frame"))) {
        if frame.attribute((NS_PRESENTATION, "class")).is_some_and(|c| c != "notes") {
            continue;
        }
        for p in frame.descendants().filter(|n| n.has_tag_name((NS_TEXT, "p"))) {
            lines.push(paragraph_text(&p));
        }
    }
    lines.join("\n").trim_matches('\n').to_string()
}

/// Map every `style:style[family=drawing-page]` name to its
/// `presentation:transition-style` value.
fn drawing_page_transitions(doc: &roxmltree::Document) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for node in doc.descendants() {
        if !node.has_tag_name((NS_STYLE, "style"))
            || node.attribute((NS_STYLE, "family")) != Some("drawing-page")
        {
            continue;
        }
        let Some(name) = node.attribute((NS_STYLE, "name")) else { continue };
        let style = node
            .children()
            .find(|c| c.has_tag_name((NS_STYLE, "drawing-page-properties")))
            .and_then(|p| p.attribute((NS_PRESENTATION, "transition-style")))
            .unwrap_or("none");
        map.insert(name.to_string(), style.to_string());
    }
    map
}

#[derive(Clone, Copy, PartialEq)]
enum PClass {
    Title,
    Sub,
    Body,
    Attrib,
    Other,
}

fn classify(style_name: &str) -> PClass {
    if style_name.starts_with("PTitle") {
        PClass::Title
    } else if style_name.starts_with("PSub") {
        PClass::Sub
    } else if style_name.starts_with("PAttrib") {
        PClass::Attrib
    } else if style_name.starts_with("PBody") {
        PClass::Body
    } else {
        PClass::Other
    }
}

/// A frame's paragraphs, each tagged with its role, plus the frame's own
/// `presentation:style-name` (used to recognise bullet frames even when empty).
struct FrameText {
    pstyle: String,
    paras: Vec<(PClass, String)>,
}

fn page_to_slide(page: &roxmltree::Node) -> Slide {
    let class = page.attribute((NS_PRESENTATION, "class")).unwrap_or("");

    let mut frames: Vec<FrameText> = Vec::new();
    for child in page.children() {
        if !child.has_tag_name((NS_DRAW, "frame")) {
            continue;
        }
        frames.push(frame_to_text(&child));
    }

    // A file "we wrote" is recognised by our frame/paragraph style names. Frames
    // may be empty (e.g. a two-column slide with no bullets), so also inspect
    // the frame's own `presentation:style-name`.
    let own_styles = frames.iter().any(|f| {
        f.pstyle.starts_with("PTitle")
            || f.pstyle.starts_with("PSub")
            || f.pstyle.starts_with("PBody")
            || f.pstyle.starts_with("PAttrib")
            || f.paras.iter().any(|(c, _)| *c != PClass::Other)
    });

    if !own_styles {
        // Foreign file (no PTitle/PBody/… styles): best-effort title+bullets.
        return foreign_slide(class, &frames);
    }

    let mut slide = Slide {
        layout: class_to_layout(class, &frames),
        ..Slide::default_blank()
    };

    for frame in &frames {
        for (role, text) in &frame.paras {
            let text = text.trim().to_string();
            if text.is_empty() {
                continue;
            }
            match role {
                PClass::Title => {
                    if slide.title.is_empty() {
                        slide.title = text;
                    }
                }
                PClass::Sub => {
                    if slide.subtitle.is_empty() {
                        slide.subtitle = text;
                    }
                }
                PClass::Attrib => {
                    if slide.attribution.is_empty() {
                        slide.attribution = text;
                    }
                }
                PClass::Body => match slide.layout.as_str() {
                    // A multi-line body is one paragraph per line.
                    "quote" | "blank" => {
                        if !slide.body.is_empty() {
                            slide.body.push('\n');
                        }
                        slide.body.push_str(&text);
                    }
                    "two-column" => {
                        // Body paragraphs from the left frame fill column 0,
                        // right frame fills column 1. We rebuild columns below.
                    }
                    _ => slide.bullets.push(strip_bullet(&text)),
                },
                PClass::Other => {}
            }
        }
    }

    // Rebuild two-column: group body paragraphs by frame.
    if slide.layout == "two-column" {
        let mut columns: Vec<Vec<String>> = Vec::new();
        for frame in &frames {
            let items: Vec<String> = frame
                .paras
                .iter()
                .filter(|(c, _)| *c == PClass::Body)
                .map(|(_, t)| strip_bullet(t.trim()))
                .filter(|s| !s.is_empty())
                .collect();
            if !items.is_empty() {
                columns.push(items);
            }
        }
        if !columns.is_empty() {
            slide.columns = columns;
        }
    }

    slide
}

fn frame_to_text(frame: &roxmltree::Node) -> FrameText {
    let pstyle = frame.attribute((NS_PRESENTATION, "style-name")).unwrap_or("").to_string();
    let mut paras = Vec::new();
    for node in frame.descendants() {
        if node.has_tag_name((NS_TEXT, "p")) {
            let style = node.attribute((NS_TEXT, "style-name")).unwrap_or("");
            let text = paragraph_text(&node);
            paras.push((classify(style), text));
        }
    }
    FrameText { pstyle, paras }
}

/// All text inside a `<text:p>` (spans, line breaks and tabs flattened).
fn paragraph_text(node: &roxmltree::Node) -> String {
    let mut out = String::new();
    for child in node.children() {
        if child.is_text() {
            out.push_str(child.text().unwrap_or(""));
        } else if child.is_element() {
            if child.has_tag_name((NS_TEXT, "line-break"))
                || child.has_tag_name((NS_TEXT, "tab"))
                || child.has_tag_name((NS_TEXT, "s"))
            {
                out.push(' ');
            } else {
                out.push_str(&paragraph_text(&child));
            }
        }
    }
    out
}

fn strip_bullet(text: &str) -> String {
    let t = text.trim();
    for prefix in [BULLET, "- ", "– ", "· "] {
        if let Some(rest) = t.strip_prefix(prefix) {
            return rest.trim().to_string();
        }
    }
    t.to_string()
}

fn class_to_layout(class: &str, frames: &[FrameText]) -> String {
    match class {
        "title" => "title".into(),
        "subtitle" => "section".into(),
        "text" => "quote".into(),
        "blank" => "blank".into(),
        _ => {
            let bullet_frames = frames
                .iter()
                .filter(|f| {
                    f.pstyle.starts_with("PBody")
                        || f.paras.iter().any(|(c, _)| *c == PClass::Body)
                })
                .count();
            if bullet_frames >= 2 {
                "two-column".into()
            } else {
                "content".into()
            }
        }
    }
}

fn foreign_slide(class: &str, frames: &[FrameText]) -> Slide {
    let mut slide = Slide {
        layout: match class {
            "title" => "title".into(),
            "subtitle" => "section".into(),
            "text" => "quote".into(),
            "blank" => "blank".into(),
            _ => "content".into(),
        },
        ..Slide::default_blank()
    };

    let mut all: Vec<String> = Vec::new();
    for frame in frames {
        for (_, text) in &frame.paras {
            let t = text.trim().to_string();
            if !t.is_empty() {
                all.push(t);
            }
        }
    }

    if slide.layout == "quote" {
        if let Some(first) = all.first() {
            slide.body = first.clone();
        }
        if all.len() > 1 {
            slide.attribution = all[1].clone();
        }
        return slide;
    }

    if let Some(first) = all.first() {
        slide.title = first.clone();
    }
    for t in all.iter().skip(1) {
        slide.bullets.push(strip_bullet(t));
    }
    slide
}

impl Slide {
    fn default_blank() -> Self {
        Slide {
            layout: "content".into(),
            title: String::new(),
            subtitle: String::new(),
            bullets: Vec::new(),
            columns: Vec::new(),
            body: String::new(),
            attribution: String::new(),
            notes: String::new(),
            transition: "none".into(),
            reveal: "all".into(),
            fit: 1.0,
        }
    }
}

// ─────────────────────────────────────────────────────────────
// helpers
// ─────────────────────────────────────────────────────────────

fn read_content_xml(odp: &[u8]) -> Result<String, AppError> {
    // Size-capped: a small `content.xml` entry can inflate to gigabytes.
    crate::read_zip_text_capped(odp, "content.xml", crate::MAX_ODF_CONTENT_XML)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn xml_escape_attr(s: &str) -> String {
    xml_escape(s).replace('"', "&quot;")
}

// ─────────────────────────────────────────────────────────────
// tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn slide(
        layout: &str,
        title: &str,
        subtitle: &str,
        bullets: &[&str],
        body: &str,
        attribution: &str,
    ) -> Slide {
        let columns = if layout == "two-column" {
            vec![
                vec!["alpha".into(), "beta".into()],
                vec!["gamma".into(), "delta".into()],
            ]
        } else {
            vec![]
        };
        Slide {
            layout: layout.into(),
            title: title.into(),
            subtitle: subtitle.into(),
            bullets: bullets.iter().map(|s| s.to_string()).collect(),
            columns,
            body: body.into(),
            attribution: attribution.into(),
            notes: String::new(),
            transition: "none".into(),
            reveal: "all".into(),
            fit: 1.0,
        }
    }

    /// Round-trip must preserve every field except the app-only ones
    /// (`reveal`, and `fit`, which the editor measures again).
    fn strip_app_only(mut s: Vec<Slide>) -> Vec<Slide> {
        for sl in &mut s {
            sl.fit = 1.0;
            sl.reveal = "all".into();
        }
        s
    }

    #[test]
    fn round_trip_all_layouts() {
        let slides = vec![
            slide("title", "Q3 Review", "Product · 2026", &[], "", ""),
            slide("section", "What shipped", "", &[], "", ""),
            slide("content", "Highlights", "", &["Faster startup", "New editor", "ODP export"], "", ""),
            slide("two-column", "Before / After", "", &[], "", ""),
            slide("quote", "", "", &[], "Simplicity is the ultimate sophistication.", "— attributed"),
            slide("blank", "", "", &[], "", ""),
        ];
        let odt = slides_to_odp("aurora", &slides).expect("write");
        let back = odp_to_slides(&odt).expect("read");
        assert_eq!(strip_app_only(back), slides, "round trip must preserve slides");
    }

    #[test]
    fn transitions_survive_round_trip() {
        let mut slides = vec![
            slide("content", "One", "", &["a"], "", ""),
            slide("content", "Two", "", &["b"], "", ""),
            slide("content", "Three", "", &["c"], "", ""),
            slide("content", "Four", "", &["d"], "", ""),
        ];
        for (s, t) in slides.iter_mut().zip(["fade", "slide", "push", "zoom"]) {
            s.transition = t.into();
        }
        let odt = slides_to_odp("ocean", &slides).expect("write");
        let content = read_content_xml(&odt).expect("content.xml");

        // Each transition gets a drawing-page style with a standard ODF effect.
        assert!(content.contains("presentation:transition-style=\"dissolve\""), "fade → dissolve");
        assert!(content.contains("presentation:transition-style=\"move-from-right\""), "slide/push → move-from-right");
        assert!(content.contains("presentation:transition-style=\"fade-from-center\""), "zoom → fade-from-center");
        assert!(content.contains("presentation:transition-type=\"manual\""), "click-advanced");

        let back = odp_to_slides(&odt).expect("read");
        let got: Vec<&str> = back.iter().map(|s| s.transition.as_str()).collect();
        // `push` degrades to `slide`: ODF has no push effect.
        assert_eq!(got, ["fade", "slide", "slide", "zoom"]);
    }

    #[test]
    fn only_used_transition_styles_are_emitted() {
        // A deck with no animation must stay free of transition metadata.
        let slides = vec![slide("content", "One", "", &["a"], "", "")];
        let odt = slides_to_odp("aurora", &slides).expect("write");
        let content = read_content_xml(&odt).expect("content.xml");
        assert!(content.contains("style:name=\"dp1\""), "plain deck uses dp1");
        assert!(!content.contains("dpFade"), "unused fade style must not be emitted");
        assert!(!content.contains("presentation:transition-style"), "no transitions in a plain deck");
    }

    #[test]
    fn foreign_transition_styles_map_to_app_effects() {
        assert_eq!(app_transition("none"), None);
        assert_eq!(app_transition(""), None);
        assert_eq!(app_transition("dissolve"), Some("fade"));
        assert_eq!(app_transition("fade-over-black"), Some("fade"));
        assert_eq!(app_transition("wipe-right"), Some("fade"));
        assert_eq!(app_transition("fade-from-center"), Some("zoom"));
        assert_eq!(app_transition("move-from-bottom"), Some("slide"));
        assert_eq!(app_transition("uncover-to-left"), Some("slide"));
        assert_eq!(odf_transition_style("none"), None);
        assert_eq!(odf_transition_style("bogus"), None);
    }

    #[test]
    fn round_trip_empty() {
        let odt = slides_to_odp("aurora", &[]).expect("write");
        let back = odp_to_slides(&odt).expect("read");
        assert!(back.is_empty());
    }

    #[test]
    fn zip_is_well_formed_odp() {
        let slides = vec![slide("title", "Hello", "Sub", &[], "", "")];
        let odt = slides_to_odp("ocean", &slides).expect("write");

        let mut archive = zip::ZipArchive::new(Cursor::new(odt.clone())).expect("zip");
        let first = archive.by_index(0).expect("first entry");
        assert_eq!(first.name(), "mimetype");
        assert_eq!(first.compression(), zip::CompressionMethod::Stored);
        drop(first);

        let mut mt = String::new();
        archive.by_name("mimetype").unwrap().read_to_string(&mut mt).unwrap();
        assert_eq!(mt, MIMETYPE);

        let content = read_content_xml(&odt).expect("content.xml");
        assert!(content.contains("office:presentation"), "has presentation body");
        assert!(content.contains("presentation:class=\"title\""), "title slide class");
        assert!(content.contains("draw:rect"), "has the accent rule");
        assert!(content.contains("text:style-name=\"PTitleOnDark-"), "title paragraph style");

        let mut styles = String::new();
        archive.by_name("styles.xml").expect("styles.xml").read_to_string(&mut styles).unwrap();
        assert!(styles.contains("fo:page-width=\"28cm\"") && styles.contains("fo:page-height=\"15.75cm\""), "16:9 page");
        assert!(styles.contains("<style:master-page style:name=\"Default\""), "master page");
        assert!(styles.contains("draw:gradient"), "background gradients");
        roxmltree::Document::parse(&styles).expect("styles.xml parses");
        roxmltree::Document::parse(&content).expect("content.xml parses");
        let mut manifest = String::new();
        archive.by_name("META-INF/manifest.xml").unwrap().read_to_string(&mut manifest).unwrap();
        assert!(manifest.contains("styles.xml") && manifest.contains("meta.xml"));
    }

    #[test]
    fn notes_and_multiline_bodies_round_trip() {
        let mut quote = slide("quote", "", "", &[], "First line\nSecond line", "Someone");
        quote.notes = "Say this slowly.\nThen pause.".into();
        let odp = slides_to_odp("aurora", &[quote.clone()]).expect("write");
        let back = odp_to_slides(&odp).expect("read");
        assert_eq!(back[0].notes, quote.notes);
        assert_eq!(back[0].body, quote.body);
    }

    #[test]
    fn bullets_are_a_real_list_in_the_accent_colour() {
        let odp = slides_to_odp("ember", &[slide("content", "T", "", &["a", "b"], "", "")]).expect("write");
        let content = read_content_xml(&odp).unwrap();
        assert!(content.contains("<text:list text:style-name=\"LBullet\">"));
        assert!(content.contains("text:bullet-char=\"\u{25CF}\""));
        assert!(content.contains("fo:color=\"#f97316\""), "bullet in the ember accent");
        assert!(!content.contains(BULLET), "no typed bullet glyphs");
    }

    #[test]
    fn fit_scales_the_shrinkable_text_only() {
        let mut s = slide("content", "Title", "", &["a"], "", "");
        s.fit = 0.5;
        let content = read_content_xml(&slides_to_odp("aurora", &[s]).unwrap()).unwrap();
        // Bullets: 19px → 23.6pt at 100 %, 11.8pt at 50 %; the title keeps 32px → 39.7pt.
        assert!(content.contains("fo:font-size=\"11.8pt\""), "bullets shrunk");
        assert!(content.contains("fo:font-size=\"39.7pt\""), "title untouched");
    }

    #[test]
    fn css_angles_map_to_odf() {
        assert_eq!(odf_angle(180.0), 0);
        assert_eq!(odf_angle(90.0), 900);
        assert_eq!(odf_angle(150.0), 300);
        assert_eq!(odf_angle(270.0), 2700);
    }

    #[test]
    fn empty_two_column_keeps_layout() {
        let slide = Slide { layout: "two-column".into(), ..Slide::default_blank() };
        let odt = slides_to_odp("aurora", &[slide]).expect("write");
        let back = odp_to_slides(&odt).expect("read");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].layout, "two-column");
    }

    #[test]
    fn normalizes_unknown_layout_and_theme() {
        assert_eq!(normalize_layout("BOGUS"), "content");
        assert_eq!(normalize_layout("Two-Column"), "two-column");
        assert_eq!(normalize_theme("nope"), "aurora");
        assert_eq!(normalize_theme("EMBER"), "ember");
        assert_eq!(normalize_transition("Slide"), "slide");
        assert_eq!(normalize_transition("nope"), "none");
        assert_eq!(normalize_reveal("Bullets"), "bullets");
        assert_eq!(normalize_reveal("later"), "all");
    }
}
