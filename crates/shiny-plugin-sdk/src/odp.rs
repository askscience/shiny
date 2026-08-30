//! Minimal OpenDocument Presentation (ODP) codec — slides ↔ `.odp` bytes.
//!
//! ODP (`.odp`) is the ISO-standard open presentation format used by
//! LibreOffice Impress / OpenOffice. An `.odp` is a ZIP containing a
//! `mimetype` entry (first, uncompressed), `META-INF/manifest.xml`, and
//! `content.xml` whose body is `<office:presentation>` with one `<draw:page>`
//! per slide.
//!
//! This codec is deliberately small but valid, and fully self-contained —
//! no office suite is involved. Each slide becomes a `<draw:page>` with a
//! small set of layouts (title / section / content / two-column / quote /
//! blank) rendered with a modern sans-serif type ramp, a theme accent colour
//! and full-bleed background rectangles. LibreOffice Impress opens the files
//! produced here directly.
//!
//! The layout is round-tripped through the ODF-standard `presentation:class`
//! attribute (`title` / `subtitle` / `outline` / `text` / `blank`) plus frame
//! structure, and text roles are recovered from the paragraph style names we
//! emit (`PTitle*` / `PSub*` / `PBody*` / `PAttrib*`).
//!
//! **Known v1 limitation:** speaker notes (`Slide::notes`) are app-side only —
//! they are not written into the `.odp` (proper notes pages are a follow-up),
//! so exporting and re-importing drops them.
//!
//! Both the core binary (REST import/export for the impress plugin) and plugin
//! code link this module — no runtime state crosses the dlopen boundary
//! because everything is plain bytes.

use std::io::{Cursor, Read, Write};

use serde::{Deserialize, Serialize};

use crate::errors::AppError;

const MIMETYPE: &str = "application/vnd.oasis.opendocument.presentation";

// Namespace URIs used for roxmltree tag/attribute lookups (the `(uri, local)`
// tuple form).
const NS_OFFICE: &str = "urn:oasis:names:tc:opendocument:xmlns:office:1.0";
const NS_DRAW: &str = "urn:oasis:names:tc:opendocument:xmlns:drawing:1.0";
const NS_TEXT: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
const NS_PRESENTATION: &str = "urn:oasis:names:tc:opendocument:xmlns:presentation:1.0";

/// Page geometry (16:9, in centimetres — the standard Impress widescreen size).
const PAGE_W: &str = "28cm";
const PAGE_H: &str = "15.75cm";

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
}

fn default_layout() -> String {
    "content".into()
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
// Theme colours
// ─────────────────────────────────────────────────────────────

struct ThemeColors {
    accent: &'static str,
    dark_bg: &'static str,
    light_bg: &'static str,
}

fn theme_colors(theme: &str) -> ThemeColors {
    match normalize_theme(theme).as_str() {
        "slate" => ThemeColors { accent: "#475569", dark_bg: "#0f172a", light_bg: "#f8fafc" },
        "ocean" => ThemeColors { accent: "#0ea5e9", dark_bg: "#082f49", light_bg: "#f0f9ff" },
        "mono" => ThemeColors { accent: "#18181b", dark_bg: "#111111", light_bg: "#ffffff" },
        "ember" => ThemeColors { accent: "#f97316", dark_bg: "#1c1917", light_bg: "#fff7ed" },
        _ => ThemeColors { accent: "#6366f1", dark_bg: "#1a1a2e", light_bg: "#ffffff" },
    }
}

// ─────────────────────────────────────────────────────────────
// slides → .odp
// ─────────────────────────────────────────────────────────────

/// Build a valid `.odp` file from a list of slides.
pub fn slides_to_odp(theme: &str, slides: &[Slide]) -> Result<Vec<u8>, AppError> {
    let colors = theme_colors(theme);
    let content = format!(
        "{CONTENT_HEADER}{automatic_styles}{master_styles}<office:body><office:presentation>{}</office:presentation></office:body></office:document-content>",
        body_xml(&colors, slides),
        automatic_styles = automatic_styles(&colors),
        master_styles = master_styles(),
    );

    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));

    // `mimetype` MUST be the first entry and MUST be stored uncompressed.
    let stored = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);
    zip.start_file("mimetype", stored)
        .map_err(|e| AppError::Internal(format!("ODP write failed: {}", e)))?;
    zip.write_all(MIMETYPE.as_bytes())
        .map_err(|e| AppError::Internal(format!("ODP write failed: {}", e)))?;

    let deflated = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("META-INF/manifest.xml", deflated)
        .map_err(|e| AppError::Internal(format!("ODP write failed: {}", e)))?;
    zip.write_all(MANIFEST.as_bytes())
        .map_err(|e| AppError::Internal(format!("ODP write failed: {}", e)))?;

    zip.start_file("content.xml", deflated)
        .map_err(|e| AppError::Internal(format!("ODP write failed: {}", e)))?;
    zip.write_all(content.as_bytes())
        .map_err(|e| AppError::Internal(format!("ODP write failed: {}", e)))?;

    let inner = zip
        .finish()
        .map_err(|e| AppError::Internal(format!("ODP write failed: {}", e)))?
        .into_inner();
    Ok(inner)
}

const CONTENT_HEADER: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content
  xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
  xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"
  xmlns:presentation="urn:oasis:names:tc:opendocument:xmlns:presentation:1.0"
  xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
  xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"
  xmlns:xlink="http://www.w3.org/1999/xlink"
  office:version="1.2">
"#;

const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2">
 <manifest:file-entry manifest:full-path="/" manifest:version="1.2" manifest:media-type="application/vnd.oasis.opendocument.presentation"/>
 <manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>
</manifest:manifest>
"#;

/// Fixed text styles (role → font/colour). Colours are role-based, not
/// theme-based, so the paragraph styles are constant; only the graphic fills
/// (backgrounds / accent bars) change with the theme.
fn automatic_styles(colors: &ThemeColors) -> String {
    let mut s = String::from("<office:automatic-styles>");

    // Page layout + drawing-page style.
    s.push_str(&format!(
        "<style:page-layout style:name=\"pm1\"><style:page-layout-properties svg:width=\"{PAGE_W}\" svg:height=\"{PAGE_H}\"/></style:page-layout>"
    ));
    s.push_str("<style:style style:name=\"dp1\" style:family=\"drawing-page\"/>");

    // Graphic fills (theme-driven).
    s.push_str(&graphic_style("grFrame", "none", "none"));
    s.push_str(&graphic_style("grBgDark", "solid", colors.dark_bg));
    s.push_str(&graphic_style("grBgLight", "solid", colors.light_bg));
    s.push_str(&graphic_style("grAccent", "solid", colors.accent));

    // Text roles.
    s.push_str(&paragraph_style("PTitleOnDark", "36pt", "#ffffff", Some("bold")));
    s.push_str(&paragraph_style("PTitleOnLight", "30pt", "#0f172a", Some("bold")));
    s.push_str(&paragraph_style("PTitleOnAccent", "40pt", "#ffffff", Some("bold")));
    s.push_str(&paragraph_style("PSubOnDark", "20pt", "#cbd5e1", None));
    s.push_str(&paragraph_style("PSubOnLight", "16pt", "#64748b", None));
    s.push_str(&paragraph_style("PBodyOnDark", "17pt", "#e2e8f0", None));
    s.push_str(&paragraph_style("PBodyOnLight", "15pt", "#334155", None));
    s.push_str(&paragraph_style("PAttribOnDark", "14pt", "#94a3b8", None));
    s.push_str(&paragraph_style("PAttribOnLight", "14pt", "#94a3b8", None));

    s.push_str("</office:automatic-styles>");
    s
}

fn graphic_style(name: &str, fill: &str, color: &str) -> String {
    format!(
        "<style:style style:name=\"{name}\" style:family=\"graphic\"><style:graphic-properties draw:fill=\"{fill}\" draw:fill-color=\"{color}\" draw:stroke=\"none\"/></style:style>"
    )
}

fn paragraph_style(name: &str, size: &str, color: &str, weight: Option<&str>) -> String {
    let weight = match weight {
        Some(w) => format!(" fo:font-weight=\"{w}\""),
        None => String::new(),
    };
    format!(
        "<style:style style:name=\"{name}\" style:family=\"paragraph\"><style:paragraph-properties fo:margin=\"0cm\"/><style:text-properties fo:font-family=\"Inter, DejaVu Sans\" fo:font-size=\"{size}\" fo:color=\"{color}\"{weight}/></style:style>"
    )
}

fn master_styles() -> String {
    format!(
        "<office:master-styles><style:master-page style:name=\"Default\" style:page-layout-name=\"pm1\"/></office:master-styles>"
    )
}

/// Map a layout name to the ODF-standard `presentation:class`.
fn layout_class(layout: &str) -> &'static str {
    match normalize_layout(layout).as_str() {
        "title" => "title",
        "section" => "subtitle",
        "quote" => "text",
        "blank" => "blank",
        _ => "outline", // content + two-column are bullet outlines
    }
}

fn body_xml(colors: &ThemeColors, slides: &[Slide]) -> String {
    let mut body = String::new();
    for (i, slide) in slides.iter().enumerate() {
        body.push_str(&page_xml(colors, slide, i + 1));
    }
    body
}

fn page_xml(colors: &ThemeColors, slide: &Slide, index: usize) -> String {
    let layout = normalize_layout(&slide.layout);
    let class = layout_class(&layout);
    let name = if slide.title.trim().is_empty() {
        format!("Slide {index}")
    } else {
        format!("Slide {index} — {}", slide.title.trim())
    };

    let mut page = format!(
        "<draw:page draw:name=\"{}\" draw:style-name=\"dp1\" draw:master-page-name=\"Default\" presentation:presentation-page-layout-name=\"pm1\" presentation:class=\"{}\">",
        xml_escape_attr(&name),
        class
    );

    match layout.as_str() {
        "title" => page.push_str(&title_slide_xml(colors, slide)),
        "section" => page.push_str(&section_slide_xml(colors, slide)),
        "two-column" => page.push_str(&two_column_slide_xml(colors, slide)),
        "quote" => page.push_str(&quote_slide_xml(colors, slide)),
        "blank" => page.push_str(&blank_slide_xml(colors, slide)),
        _ => page.push_str(&content_slide_xml(colors, slide)),
    }

    page.push_str("</draw:page>");
    page
}

// ── Layout renderers ─────────────────────────────────────────

fn bg_rect(style: &str, x: &str, y: &str, w: &str, h: &str) -> String {
    format!(
        "<draw:rect draw:style-name=\"{style}\" svg:x=\"{x}\" svg:y=\"{y}\" svg:width=\"{w}\" svg:height=\"{h}\"/>"
    )
}

fn text_frame(pstyle: &str, x: &str, y: &str, w: &str, h: &str, lines: &[String]) -> String {
    let mut paras = String::new();
    for line in lines {
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        paras.push_str(&format!(
            "<text:p text:style-name=\"{pstyle}\">{}</text:p>",
            xml_escape(text)
        ));
    }
    format!(
        "<draw:frame draw:style-name=\"grFrame\" presentation:style-name=\"{pstyle}\" svg:x=\"{x}\" svg:y=\"{y}\" svg:width=\"{w}\" svg:height=\"{h}\"><draw:text-box>{paras}</draw:text-box></draw:frame>"
    )
}

fn bullet_lines(items: &[String]) -> Vec<String> {
    items
        .iter()
        .map(|b| b.trim().to_string())
        .filter(|b| !b.is_empty())
        .map(|b| format!("{BULLET}{b}"))
        .collect()
}

fn title_slide_xml(colors: &ThemeColors, slide: &Slide) -> String {
    let mut out = String::new();
    out.push_str(&bg_rect("grBgDark", "0cm", "0cm", PAGE_W, PAGE_H));
    // Small accent rule above the title.
    out.push_str(&bg_rect("grAccent", "3cm", "6.0cm", "2.2cm", "0.12cm"));
    if !slide.title.trim().is_empty() {
        out.push_str(&text_frame(
            "PTitleOnDark",
            "3cm", "6.4cm", "22cm", "3cm",
            &[slide.title.clone()],
        ));
    }
    if !slide.subtitle.trim().is_empty() {
        out.push_str(&text_frame(
            "PSubOnDark",
            "3cm", "9.6cm", "22cm", "2cm",
            &[slide.subtitle.clone()],
        ));
    }
    let _ = colors;
    out
}

fn section_slide_xml(colors: &ThemeColors, slide: &Slide) -> String {
    let mut out = String::new();
    out.push_str(&bg_rect("grAccent", "0cm", "0cm", PAGE_W, PAGE_H));
    if !slide.title.trim().is_empty() {
        out.push_str(&text_frame(
            "PTitleOnAccent",
            "3cm", "6.6cm", "22cm", "3cm",
            &[slide.title.clone()],
        ));
    }
    let _ = colors;
    out
}

fn content_slide_xml(colors: &ThemeColors, slide: &Slide) -> String {
    let mut out = String::new();
    out.push_str(&bg_rect("grBgLight", "0cm", "0cm", PAGE_W, PAGE_H));
    out.push_str(&title_block("PTitleOnLight", slide));
    let bullets = bullet_lines(&slide.bullets);
    if !bullets.is_empty() {
        out.push_str(&text_frame("PBodyOnLight", "2.5cm", "4.4cm", "23cm", "9.5cm", &bullets));
    }
    let _ = colors;
    out
}

fn two_column_slide_xml(colors: &ThemeColors, slide: &Slide) -> String {
    let mut out = String::new();
    out.push_str(&bg_rect("grBgLight", "0cm", "0cm", PAGE_W, PAGE_H));
    out.push_str(&title_block("PTitleOnLight", slide));
    let mut cols = slide.columns.iter();
    let left = cols.next().cloned().unwrap_or_default();
    let right = cols.next().cloned().unwrap_or_default();
    out.push_str(&text_frame(
        "PBodyOnLight",
        "2.5cm", "4.4cm", "11.3cm", "9.5cm",
        &bullet_lines(&left),
    ));
    out.push_str(&text_frame(
        "PBodyOnLight",
        "14.2cm", "4.4cm", "11.3cm", "9.5cm",
        &bullet_lines(&right),
    ));
    let _ = colors;
    out
}

fn title_block(pstyle: &str, slide: &Slide) -> String {
    let mut out = String::new();
    if !slide.title.trim().is_empty() {
        out.push_str(&text_frame(
            pstyle,
            "2.5cm", "1.4cm", "23cm", "2cm",
            &[slide.title.clone()],
        ));
        out.push_str(&bg_rect("grAccent", "2.5cm", "3.4cm", "2cm", "0.1cm"));
    }
    out
}

fn quote_slide_xml(colors: &ThemeColors, slide: &Slide) -> String {
    let mut out = String::new();
    out.push_str(&bg_rect("grBgDark", "0cm", "0cm", PAGE_W, PAGE_H));
    if !slide.body.trim().is_empty() {
        out.push_str(&text_frame(
            "PBodyOnDark",
            "3cm", "4.4cm", "22cm", "7cm",
            &[slide.body.clone()],
        ));
    }
    if !slide.attribution.trim().is_empty() {
        out.push_str(&text_frame(
            "PAttribOnDark",
            "3cm", "11.6cm", "22cm", "2cm",
            &[slide.attribution.clone()],
        ));
    }
    let _ = colors;
    out
}

fn blank_slide_xml(colors: &ThemeColors, slide: &Slide) -> String {
    let mut out = String::new();
    out.push_str(&bg_rect("grBgLight", "0cm", "0cm", PAGE_W, PAGE_H));
    if !slide.body.trim().is_empty() {
        out.push_str(&text_frame(
            "PBodyOnLight",
            "2.5cm", "5.5cm", "23cm", "6cm",
            &[slide.body.clone()],
        ));
    } else {
        // A slim bottom accent strip so a "blank" slide still reads as a
        // designed surface rather than an empty page.
        out.push_str(&bg_rect("grAccent", "0cm", "15.25cm", PAGE_W, "0.5cm"));
    }
    let _ = colors;
    out
}

// ─────────────────────────────────────────────────────────────
// .odp → slides
// ─────────────────────────────────────────────────────────────

/// Read slides back out of a `.odp` file. Handles our own writer losslessly
/// (layout + title/subtitle/bullets/columns/body/attribution) and makes a
/// best-effort attempt at foreign LibreOffice files (title + bullets).
pub fn odp_to_slides(odp: &[u8]) -> Result<Vec<Slide>, AppError> {
    let xml = read_content_xml(odp)?;
    let doc = roxmltree::Document::parse(&xml)
        .map_err(|e| AppError::Internal(format!("Invalid ODP content.xml: {}", e)))?;

    let mut slides = Vec::new();
    let presentation = doc
        .descendants()
        .find(|n| n.has_tag_name((NS_OFFICE, "presentation")) && n.is_element());

    let Some(presentation) = presentation else {
        return Ok(slides); // empty deck
    };

    for page in presentation.children() {
        if !page.has_tag_name((NS_DRAW, "page")) {
            continue;
        }
        // Skip speaker-notes pages (we don't emit them, but foreign files do).
        if page.attribute((NS_PRESENTATION, "class")) == Some("notes") {
            continue;
        }
        slides.push(page_to_slide(&page));
    }

    Ok(slides)
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
                    "quote" | "blank" => {
                        if slide.body.is_empty() {
                            slide.body = text;
                        }
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
        }
    }
}

// ─────────────────────────────────────────────────────────────
// helpers
// ─────────────────────────────────────────────────────────────

fn read_content_xml(odp: &[u8]) -> Result<String, AppError> {
    let reader = Cursor::new(odp.to_vec());
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|e| AppError::Internal(format!("Not a valid .odp file: {}", e)))?;
    let mut content = String::new();
    archive
        .by_name("content.xml")
        .map_err(|e| AppError::Internal(format!("Missing content.xml in .odp: {}", e)))?
        .read_to_string(&mut content)
        .map_err(|e| AppError::Internal(format!("Failed to read content.xml: {}", e)))?;
    Ok(content)
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
        }
    }

    /// Round-trip must preserve every field except `notes` (not in ODP).
    fn strip_notes(mut s: Vec<Slide>) -> Vec<Slide> {
        for sl in &mut s {
            sl.notes.clear();
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
        assert_eq!(strip_notes(back), slides, "round trip must preserve slides");
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
        assert!(content.contains("draw:rect"), "has background rect");
        assert!(content.contains("text:style-name=\"PTitleOnDark\""), "title paragraph style");
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
    }
}
