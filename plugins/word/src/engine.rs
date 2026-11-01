//! The Word plugin's document engine, built on WordCraft's libraries
//! (`wordcraft-doc`, `-docx`, `-formats`, `-layout`, `-pdf`; MIT OR Apache-2.0).
//!
//! Two layers:
//!
//! - **Content codecs.** A document is stored as `.odt` content. The window
//!   edits an HTML fragment and the agent reads and writes Markdown; both go
//!   through WordCraft's [`Document`] model, so tables, alignment, colours,
//!   sizes, lists and fonts survive every round trip. Import accepts `.docx`,
//!   `.odt`, `.rtf`, `.md`, `.html` and `.txt`.
//! - **Composition.** A [`Design`] (stored beside the content) turns that
//!   content into a finished document on export: a theme (fonts, colours,
//!   table style), the page size and margins, a cover page, a table of
//!   contents with real page numbers, a running header and a page-number
//!   footer. Export writes `.docx`, `.pdf`, `.odt`, `.md` or `.html`.
//!
//! The editor never sees the composition: the cover, contents, header and
//! footer are generated on every export from the current content, so they
//! can't drift from it.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use shiny_plugin_sdk::errors::AppError;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::{
    Align, Border, BorderStyle, Borders, CharProps, LineSpacing, Rgb, TabAlign, TabLeader, TabStop,
    TextColor,
};
use wordcraft_doc::{para_block, Block, Document, Paragraph, PartKind, Pos};

/* ── Formats ────────────────────────────────────────────────── */

/// A file format the plugin reads or writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Odt,
    Docx,
    Pdf,
    Rtf,
    Markdown,
    Html,
    Txt,
}

impl Format {
    /// The format named by an extension or a short name (`"docx"`, `".PDF"`, `"markdown"`).
    pub fn parse(name: &str) -> Option<Format> {
        Some(match name.trim().trim_start_matches('.').to_ascii_lowercase().as_str() {
            "odt" => Format::Odt,
            "docx" | "docm" | "dotx" | "word" => Format::Docx,
            "pdf" => Format::Pdf,
            "rtf" => Format::Rtf,
            "md" | "markdown" => Format::Markdown,
            "html" | "htm" | "xhtml" => Format::Html,
            "txt" | "text" => Format::Txt,
            _ => return None,
        })
    }

    pub fn ext(self) -> &'static str {
        match self {
            Format::Odt => "odt",
            Format::Docx => "docx",
            Format::Pdf => "pdf",
            Format::Rtf => "rtf",
            Format::Markdown => "md",
            Format::Html => "html",
            Format::Txt => "txt",
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            Format::Odt => "application/vnd.oasis.opendocument.text",
            Format::Docx => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            Format::Pdf => "application/pdf",
            Format::Rtf => "application/rtf",
            Format::Markdown => "text/markdown; charset=utf-8",
            Format::Html => "text/html; charset=utf-8",
            Format::Txt => "text/plain; charset=utf-8",
        }
    }

    /// Formats a file can be imported from.
    pub fn importable(self) -> bool {
        !matches!(self, Format::Pdf)
    }
}

/// Guess the format of `bytes` from their content (ZIP packages, RTF, HTML).
fn sniff(bytes: &[u8]) -> Option<Format> {
    if bytes.starts_with(b"PK") {
        let head = &bytes[..bytes.len().min(256)];
        if head.windows(39).any(|w| w == b"application/vnd.oasis.opendocument.text") {
            return Some(Format::Odt);
        }
        return Some(Format::Docx);
    }
    if bytes.starts_with(b"{\\rtf") {
        return Some(Format::Rtf);
    }
    let start = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_ascii_lowercase();
    let start = start.trim_start();
    if start.starts_with("<!doctype html") || start.starts_with("<html") {
        return Some(Format::Html);
    }
    None
}

fn internal(what: &str, e: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!("{what}: {e}"))
}

/// Read a file into a document. `name` is the file name (its extension picks
/// the format); when it names none the content is sniffed.
pub fn import(name: &str, bytes: &[u8]) -> Result<Document, AppError> {
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    let format = Format::parse(ext)
        .or_else(|| sniff(bytes))
        .ok_or_else(|| AppError::BadRequest(format!("Unsupported file type: {name}")))?;
    if !format.importable() {
        return Err(AppError::BadRequest("PDF files open in the PDF app, not in Word".into()));
    }
    let mut doc = match format {
        Format::Docx => wordcraft_docx::read(bytes)
            .map_err(|e| AppError::BadRequest(format!("Not a readable Word document: {e}")))?,
        f => wordcraft_formats::import(f.ext(), bytes)
            .ok_or_else(|| AppError::BadRequest(format!("Unsupported file type: {name}")))?
            .map_err(|e| AppError::BadRequest(format!("Could not read {name}: {e}")))?,
    };
    doc.ensure_nonempty();
    Ok(doc)
}

/* ── Content codecs ─────────────────────────────────────────── */

/// The stored `.odt` bytes as a document.
pub fn odt_to_doc(odt: &[u8]) -> Result<Document, AppError> {
    let mut doc = wordcraft_formats::odt::import(odt).map_err(|e| internal("Not a valid .odt file", e))?;
    doc.ensure_nonempty();
    Ok(doc)
}

/// A document as the `.odt` bytes the database stores.
pub fn doc_to_odt(doc: &Document) -> Result<Vec<u8>, AppError> {
    wordcraft_formats::odt::export(doc).map_err(|e| internal("ODT write failed", e))
}

/// The editor's HTML fragment as a document.
pub fn html_to_doc(html: &str) -> Document {
    let mut doc = wordcraft_formats::html::import(html);
    doc.ensure_nonempty();
    doc
}

/// A document as the editor's HTML fragment.
pub fn doc_to_html(doc: &Document) -> String {
    let html = wordcraft_formats::html::export_fragment(doc);
    html.strip_prefix("<meta charset=\"utf-8\">").map(str::to_string).unwrap_or(html)
}

/// A document as Markdown: what the agent reads.
pub fn doc_to_markdown(doc: &Document) -> String {
    wordcraft_formats::markdown::export(doc).trim().to_string()
}

/// The agent's Markdown as a document.
pub fn markdown_to_doc(md: &str) -> Document {
    let mut doc = wordcraft_formats::markdown::import(&separate_lines(md));
    doc.ensure_nonempty();
    doc
}

pub fn html_to_odt(html: &str) -> Result<Vec<u8>, AppError> {
    doc_to_odt(&html_to_doc(html))
}

pub fn odt_to_html(odt: &[u8]) -> Result<String, AppError> {
    Ok(doc_to_html(&odt_to_doc(odt)?))
}

pub fn odt_to_markdown(odt: &[u8]) -> Result<String, AppError> {
    Ok(doc_to_markdown(&odt_to_doc(odt)?))
}

pub fn markdown_to_html(md: &str) -> String {
    doc_to_html(&markdown_to_doc(md))
}

/// Models often write one paragraph per line, where Markdown would join the
/// lines into one paragraph. Put a blank line between two text lines, but
/// keep tables, list items and fenced code as Markdown expects them.
fn separate_lines(md: &str) -> String {
    let is_table = |l: &str| l.trim_start().starts_with('|');
    let is_item = |l: &str| {
        let t = l.trim_start();
        t.starts_with("- ")
            || t.starts_with("* ")
            || t.starts_with("+ ")
            || t.split_once(". ").is_some_and(|(n, _)| !n.is_empty() && n.len() <= 3 && n.bytes().all(|b| b.is_ascii_digit()))
    };
    let mut out = String::with_capacity(md.len() + md.len() / 8);
    let mut fenced = false;
    let mut prev: Option<&str> = None;
    for line in md.lines() {
        let fence = line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~");
        if let Some(p) = prev {
            let blank = line.trim().is_empty() || p.trim().is_empty();
            let together = fenced
                || blank
                || (is_table(p) && is_table(line))
                || (is_item(p) && (is_item(line) || line.starts_with(' ') || line.starts_with('\t')))
                || line.starts_with("    ");
            if !together {
                out.push('\n');
            }
        }
        if fence {
            fenced = !fenced;
        }
        out.push_str(line);
        out.push('\n');
        prev = Some(line);
    }
    out
}

/* ── Design ─────────────────────────────────────────────────── */

/// How a document is composed on export. Stored as JSON beside the content.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Design {
    /// Theme: `plain` (or empty), `classic`, `modern`, `minimal`.
    pub template: String,
    /// `a4` (default) or `letter`.
    pub paper: String,
    /// A cover page with the title, subtitle, author and date.
    pub cover: bool,
    /// A table of contents (headings 1–3) with page numbers.
    pub toc: bool,
    /// A running header with the title.
    pub header: bool,
    /// A "n / N" page-number footer.
    pub page_numbers: bool,
    pub subtitle: String,
    pub author: String,
    pub date: String,
    /// Language of the text (`it`, `en-GB`, …): labels and hyphenation.
    pub lang: String,
    /// Accent colour `#RRGGBB`; empty uses the theme's.
    pub accent: String,
    /// Page margins in millimetres; 0 = the default (25 mm).
    pub margin_top: f32,
    pub margin_right: f32,
    pub margin_bottom: f32,
    pub margin_left: f32,
}

/// Default page margin, millimetres.
pub const DEFAULT_MARGIN_MM: f32 = 25.0;
const PT_PER_MM: f32 = 72.0 / 25.4;

/// The themes [`Design::template`] accepts.
pub const TEMPLATES: &[&str] = &["plain", "classic", "modern", "minimal"];

impl Design {
    /// Nothing to compose: export the content as it is.
    pub fn is_plain(&self) -> bool {
        theme(&self.template).is_none() && !self.cover && !self.toc && !self.header && !self.page_numbers
    }

    /// The defaults a theme brings when it is first chosen: themed documents
    /// get a header and page numbers; covers and contents stay opt-in.
    pub fn with_template_defaults(mut self, template: &str) -> Design {
        self.template = template.to_string();
        if theme(template).is_some() {
            self.header = true;
            self.page_numbers = true;
        }
        self
    }

    /// Check values from the outside (agent or window).
    pub fn validate(&self) -> Result<(), AppError> {
        let t = self.template.trim().to_ascii_lowercase();
        if !t.is_empty() && !TEMPLATES.contains(&t.as_str()) {
            return Err(AppError::BadRequest(format!(
                "Unknown template \"{}\" — use one of: {}",
                self.template,
                TEMPLATES.join(", ")
            )));
        }
        let p = self.paper.trim().to_ascii_lowercase();
        if !p.is_empty() && p != "a4" && p != "letter" {
            return Err(AppError::BadRequest("paper must be \"a4\" or \"letter\"".into()));
        }
        if !self.accent.trim().is_empty() && parse_hex(&self.accent).is_none() {
            return Err(AppError::BadRequest("accent must be a #RRGGBB colour".into()));
        }
        for m in [self.margin_top, self.margin_right, self.margin_bottom, self.margin_left] {
            if !m.is_finite() || (m != 0.0 && !(5.0..=80.0).contains(&m)) {
                return Err(AppError::BadRequest("margins must be between 5 and 80 mm".into()));
            }
        }
        for (name, v) in [("subtitle", &self.subtitle), ("author", &self.author), ("date", &self.date)] {
            if v.chars().count() > 200 {
                return Err(AppError::BadRequest(format!("{name} is too long (max 200 characters)")));
            }
        }
        Ok(())
    }

    /// Margins in points: top, right, bottom, left.
    pub fn margins_pt(&self) -> [f32; 4] {
        let m = |v: f32| if v > 0.0 { v } else { DEFAULT_MARGIN_MM } * PT_PER_MM;
        [m(self.margin_top), m(self.margin_right), m(self.margin_bottom), m(self.margin_left)]
    }

    pub fn from_json(s: &str) -> Design {
        serde_json::from_str(s).unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }
}

/// A theme's typography and colours.
struct Theme {
    body_font: &'static str,
    heading_font: &'static str,
    body_size: f32,
    /// Heading 1–3 sizes.
    heading_sizes: [f32; 3],
    title_size: f32,
    accent: Rgb,
    text: Rgb,
    muted: Rgb,
    line_spacing: f32,
    /// Title and cover centred (classic) or flush left.
    centred: bool,
    /// A rule under Heading 1.
    heading_rule: bool,
}

// Faces WordCraft bundles, so PDF output looks the same on every machine.
const SERIF: &str = "Source Serif 4";
const SANS: &str = "Source Sans 3";

fn theme(name: &str) -> Option<Theme> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "classic" => Theme {
            body_font: SERIF,
            heading_font: SERIF,
            body_size: 11.0,
            heading_sizes: [18.0, 14.0, 12.0],
            title_size: 30.0,
            accent: Rgb(0x1F, 0x38, 0x64),
            text: Rgb(0x1A, 0x1A, 0x1A),
            muted: Rgb(0x59, 0x59, 0x59),
            line_spacing: 1.2,
            centred: true,
            heading_rule: true,
        },
        "modern" => Theme {
            body_font: SANS,
            heading_font: SANS,
            body_size: 11.0,
            heading_sizes: [20.0, 15.0, 12.5],
            title_size: 34.0,
            accent: Rgb(0x1D, 0x5F, 0xBF),
            text: Rgb(0x1F, 0x23, 0x28),
            muted: Rgb(0x5B, 0x63, 0x6E),
            line_spacing: 1.25,
            centred: false,
            heading_rule: false,
        },
        "minimal" => Theme {
            body_font: SANS,
            heading_font: SANS,
            body_size: 10.5,
            heading_sizes: [16.0, 13.0, 11.0],
            title_size: 26.0,
            accent: Rgb(0x11, 0x11, 0x11),
            text: Rgb(0x22, 0x22, 0x22),
            muted: Rgb(0x6B, 0x72, 0x80),
            line_spacing: 1.3,
            centred: false,
            heading_rule: false,
        },
        _ => return None,
    })
}

fn parse_hex(s: &str) -> Option<Rgb> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let n = u32::from_str_radix(h, 16).ok()?;
    Some(Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

/// `c` mixed with white, `t` = share of `c` (0–1).
fn tint(c: Rgb, t: f32) -> Rgb {
    let m = |v: u8| (255.0 - (255.0 - v as f32) * t).round().clamp(0.0, 255.0) as u8;
    Rgb(m(c.0), m(c.1), m(c.2))
}

fn rgb(c: Rgb) -> Option<TextColor> {
    Some(TextColor::Rgb(c))
}

fn rule(c: Rgb, width: f32, space: f32) -> Border {
    Border { style: BorderStyle::Single, width, color: Some(c), space }
}

/// Localised labels: the contents heading.
fn contents_label(lang: &str) -> &'static str {
    let code = lang.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    match code.as_str() {
        "bg" => "Съдържание",
        "cs" | "sk" => "Obsah",
        "da" => "Indhold",
        "de" => "Inhalt",
        "el" => "Περιεχόμενα",
        "es" | "pt" => "Índice",
        "it" => "Indice",
        "et" => "Sisukord",
        "fi" => "Sisällys",
        "fr" => "Sommaire",
        "ga" => "Clár ábhair",
        "hr" => "Sadržaj",
        "hu" => "Tartalom",
        "lt" => "Turinys",
        "lv" => "Saturs",
        "mt" => "Werrej",
        "nl" => "Inhoud",
        "pl" => "Spis treści",
        "ro" => "Cuprins",
        "ru" => "Содержание",
        "sl" => "Kazalo",
        "sv" => "Innehåll",
        "uk" => "Зміст",
        _ => "Contents",
    }
}

/// The heading level (1–9) of a paragraph styled with a built-in heading style.
fn heading_level(p: &Paragraph) -> Option<u8> {
    p.props.style.as_deref()?.strip_prefix("Heading")?.trim().parse::<u8>().ok().filter(|l| (1..=9).contains(l))
}

fn para(text: &str, style: &str) -> Paragraph {
    Paragraph::with_text(text, CharProps::default()).styled(style)
}

fn field(p: &mut Paragraph, instr: &str, shown: &str) {
    let at = p.len();
    let _ = p.insert_object(
        at,
        InlineObject::Field { instr: instr.into(), result: shown.into(), locked: false, dirty: false, code: false, form: String::new() },
        &CharProps::default(),
    );
}

fn text(p: &mut Paragraph, s: &str) {
    let at = p.len();
    let _ = p.insert_text(at, s, &CharProps::default());
}

/// Restyle the built-in styles for `t`.
fn apply_theme(doc: &mut Document, t: &Theme, accent: Rgb) {
    let st = &mut doc.styles;
    st.default_chr.font = Some(t.body_font.into());
    st.default_chr.size = Some(t.body_size);
    st.default_chr.color = rgb(t.text);
    st.default_para.line_spacing = Some(LineSpacing::Multiple(t.line_spacing));
    st.default_para.space_after = Some(t.body_size * 0.6);

    for (i, size) in t.heading_sizes.iter().enumerate() {
        let n = i + 1;
        if let Some(s) = st.get_mut(&format!("Heading{n}")) {
            s.chr.font = Some(t.heading_font.into());
            s.chr.size = Some(*size);
            s.chr.bold = Some(true);
            s.chr.italic = None;
            s.chr.color = rgb(if n == 3 { t.text } else { accent });
            s.para.space_before = Some(if n == 1 { 20.0 } else { 14.0 });
            s.para.space_after = Some(if n == 1 { 8.0 } else { 4.0 });
            if n == 1 && t.heading_rule {
                s.para.borders = Some(Borders { bottom: Some(rule(tint(accent, 0.35), 0.75, 3.0)), ..Default::default() });
            }
        }
    }
    for n in 4..=6 {
        if let Some(s) = st.get_mut(&format!("Heading{n}")) {
            s.chr.font = Some(t.heading_font.into());
            s.chr.color = rgb(t.text);
        }
    }
    if let Some(s) = st.get_mut("Title") {
        s.chr.font = Some(t.heading_font.into());
        s.chr.size = Some(t.title_size);
        s.chr.bold = Some(true);
        s.chr.color = rgb(accent);
        s.para.align = Some(if t.centred { Align::Center } else { Align::Left });
        s.para.space_after = Some(6.0);
    }
    if let Some(s) = st.get_mut("Subtitle") {
        s.chr.font = Some(t.body_font.into());
        s.chr.size = Some(t.body_size + 3.0);
        s.chr.color = rgb(t.muted);
        s.chr.italic = Some(t.centred);
        s.para.align = Some(if t.centred { Align::Center } else { Align::Left });
    }
    if let Some(s) = st.get_mut("Quote") {
        s.chr.color = rgb(t.muted);
        s.para.borders = Some(Borders { left: Some(rule(accent, 2.25, 8.0)), ..Default::default() });
        s.para.align = Some(Align::Left);
        s.para.indent_left = Some(18.0);
        s.para.indent_right = Some(18.0);
    }

    // The table style: the accent on the header row, a light band.
    if let Some(mut s) = st.get("GridTable4AccentBlue").cloned() {
        s.id = "ShinyTable".into();
        s.name = "Shiny Table".into();
        s.builtin = false;
        if let Some(parts) = s.table.as_mut() {
            parts.borders = Some(Borders::all(rule(tint(accent, 0.3), 0.5, 0.0)));
            parts.header_fill = Some(accent);
            parts.band_fill = Some(tint(accent, 0.08));
        }
        st.upsert(s);
    }
    let style_tables = |blocks: &mut Vec<Arc<Block>>| {
        for b in blocks.iter_mut() {
            if let Block::Table(tb) = Arc::make_mut(b) {
                if tb.props.style.as_deref().is_none_or(|s| s == "TableGrid" || s == "TableNormal") {
                    tb.props.style = Some("ShinyTable".into());
                }
            }
        }
    };
    style_tables(&mut doc.body);
}

/// Page size in points for `paper`.
fn page_size(paper: &str) -> (f32, f32) {
    match paper.trim().to_ascii_lowercase().as_str() {
        "letter" => (612.0, 792.0),
        _ => (595.3, 841.9),
    }
}

/// Compose `content` into a finished document for export.
pub fn compose(content: &Document, title: &str, design: &Design) -> Document {
    let mut d = content.clone();
    let title = title.trim();
    d.core.title = title.to_string();
    if !design.author.trim().is_empty() {
        d.core.creator = design.author.trim().to_string();
    }
    if !design.subtitle.trim().is_empty() {
        d.core.subject = design.subtitle.trim().to_string();
    }
    if !design.lang.trim().is_empty() {
        d.styles.default_chr.lang = Some(design.lang.trim().to_string());
    }

    // Page: paper and margins (25 mm unless the design sets them).
    let (w, h) = page_size(&design.paper);
    let themed = theme(&design.template);
    let is_themed = themed.is_some();
    let [mt, mr, mb, ml] = design.margins_pt();
    {
        let s = &mut d.last_section;
        s.page_w = w;
        s.page_h = h;
        s.landscape = false;
        s.margin_left = ml;
        s.margin_right = mr;
        s.margin_top = mt;
        s.margin_bottom = mb;
        s.header = 36.0;
        s.footer = 36.0;
    }
    if design.is_plain() {
        return d;
    }

    let t = themed.unwrap_or(Theme {
        body_font: SANS,
        heading_font: SANS,
        body_size: 11.0,
        heading_sizes: [16.0, 13.0, 12.0],
        title_size: 28.0,
        accent: Rgb(0x15, 0x60, 0x82),
        text: Rgb(0, 0, 0),
        muted: Rgb(0x59, 0x59, 0x59),
        line_spacing: 1.15,
        centred: false,
        heading_rule: false,
    });
    let accent = parse_hex(&design.accent).unwrap_or(t.accent);
    if is_themed {
        apply_theme(&mut d, &t, accent);
    }
    let text_w = w - ml - mr;

    // The title is generated: drop a leading heading that repeats it.
    let repeats_title = d.body.first().and_then(|b| b.as_para()).is_some_and(|p| {
        let s = p.props.style.as_deref().unwrap_or("");
        (s == "Title" || s == "Heading1") && p.plain_text().trim().eq_ignore_ascii_case(title)
    });
    if repeats_title && d.body.len() > 1 {
        d.body.remove(0);
    }

    let mut front: Vec<Paragraph> = Vec::new();
    let align = if t.centred { Align::Center } else { Align::Left };
    let meta: Vec<&str> = [design.author.trim(), design.date.trim()].into_iter().filter(|s| !s.is_empty()).collect();
    if design.cover {
        let mut tp = para(title, "Title");
        tp.props.space_before = Some(h * 0.28);
        tp.props.align = Some(align);
        tp.props.borders = Some(Borders { bottom: Some(rule(accent, 1.5, 10.0)), ..Default::default() });
        front.push(tp);
        if !design.subtitle.trim().is_empty() {
            let mut sp = para(design.subtitle.trim(), "Subtitle");
            sp.props.space_before = Some(12.0);
            sp.props.align = Some(align);
            front.push(sp);
        }
        for (i, m) in meta.iter().enumerate() {
            let mut mp = Paragraph::with_text(m, CharProps { color: rgb(t.muted), ..Default::default() });
            mp.props.align = Some(align);
            mp.props.space_before = Some(if i == 0 { h * 0.22 } else { 0.0 });
            mp.props.space_after = Some(2.0);
            front.push(mp);
        }
    } else if is_themed {
        front.push(para(title, "Title"));
        if !design.subtitle.trim().is_empty() {
            front.push(para(design.subtitle.trim(), "Subtitle"));
        }
        if !meta.is_empty() {
            let mut mp = Paragraph::with_text(&meta.join("  ·  "), CharProps { color: rgb(t.muted), size: Some(t.body_size - 1.0), ..Default::default() });
            mp.props.align = Some(align);
            mp.props.space_after = Some(18.0);
            front.push(mp);
        }
    }

    // Contents: one entry per heading 1–3, page numbers filled after layout.
    let headings: Vec<(usize, u8, String)> = d
        .body
        .iter()
        .enumerate()
        .filter_map(|(i, b)| {
            let p = b.as_para()?;
            let l = heading_level(p).filter(|l| *l <= 3)?;
            let s = p.plain_text().replace(['\t', '\n', '\u{000C}'], " ").trim().to_string();
            (!s.is_empty()).then_some((i, l, s))
        })
        .collect();
    let toc_at = front.len();
    let want_toc = design.toc && !headings.is_empty();
    if want_toc {
        let mut th = para(contents_label(&design.lang), "TOCHeading");
        th.props.page_break_before = Some(design.cover);
        th.props.space_after = Some(12.0);
        front.push(th);
        for (_, level, s) in &headings {
            front.push(toc_entry(s, *level, "", text_w, accent));
        }
    }

    // After a cover or contents the content starts on a new page.
    let break_after_front = design.cover || want_toc;
    let mut body: Vec<Arc<Block>> = front.into_iter().map(para_block).collect();
    let offset = body.len();
    let mut content_blocks = std::mem::take(&mut d.body);
    if break_after_front {
        match content_blocks.first_mut().map(Arc::make_mut) {
            Some(Block::Para(p)) => p.props.page_break_before = Some(true),
            _ => {
                let mut p = Paragraph::new();
                p.props.page_break_before = Some(true);
                body.push(para_block(p));
            }
        }
    }
    let offset = offset + usize::from(body.len() > offset);
    body.extend(content_blocks);
    d.body = body;

    // Header and footer; none on the cover.
    d.last_section.title_page = design.cover;
    if design.header {
        let mut hp = Paragraph::with_text(title, CharProps { size: Some(9.0), color: rgb(t.muted), ..Default::default() });
        // Not the "Header" style: its Letter-width tab stops would catch the tab.
        hp.props.space_after = Some(0.0);
        let right = if !design.subtitle.trim().is_empty() { design.subtitle.trim() } else { design.author.trim() };
        if !right.is_empty() {
            let at = hp.len();
            let _ = hp.insert_text(at, &format!("\t{right}"), &CharProps { size: Some(9.0), color: rgb(t.muted), ..Default::default() });
        }
        hp.props.tabs = Some(vec![TabStop { pos: text_w, align: TabAlign::Right, leader: TabLeader::None }]);
        hp.props.borders = Some(Borders { bottom: Some(rule(tint(accent, 0.5), 0.5, 4.0)), ..Default::default() });
        let id = d.add_part(PartKind::Header, vec![para_block(hp)]);
        d.last_section.headers.default = Some(id);
    }
    if design.page_numbers {
        let mut fp = Paragraph::new().styled("Footer");
        fp.props.align = Some(if t.centred { Align::Center } else { Align::Right });
        field(&mut fp, "PAGE", "1");
        text(&mut fp, " / ");
        field(&mut fp, "NUMPAGES", "1");
        let id = d.add_part(PartKind::Footer, vec![para_block(fp)]);
        d.last_section.footers.default = Some(id);
    }
    if design.cover {
        // An empty first-page header and footer keep the cover clean in Word too.
        let eh = d.add_part(PartKind::Header, vec![para_block(Paragraph::new())]);
        let ef = d.add_part(PartKind::Footer, vec![para_block(Paragraph::new())]);
        d.last_section.headers.first = Some(eh);
        d.last_section.footers.first = Some(ef);
    }

    if want_toc {
        fill_toc_pages(&mut d, toc_at + 1, offset, &headings, text_w, accent);
    }
    d
}

fn toc_entry(text: &str, level: u8, page: &str, text_w: f32, accent: Rgb) -> Paragraph {
    let style = format!("TOC{level}");
    let props = if level == 1 { CharProps { bold: Some(true), ..Default::default() } } else { CharProps::default() };
    let mut p = Paragraph::with_text(&format!("{text}\t"), props).styled(&style);
    let at = p.len();
    let _ = p.insert_text(at, page, &CharProps { color: rgb(accent), ..Default::default() });
    p.props.tabs = Some(vec![TabStop { pos: text_w, align: TabAlign::Right, leader: TabLeader::Dot }]);
    p.props.indent_left = Some(f32::from(level.saturating_sub(1)) * 14.0);
    p.props.space_before = Some(if level == 1 { 6.0 } else { 0.0 });
    p.props.space_after = Some(2.0);
    p
}

/// Lay the document out once and write each heading's page into its entry.
fn fill_toc_pages(d: &mut Document, first_entry: usize, offset: usize, headings: &[(usize, u8, String)], text_w: f32, accent: Rgb) {
    let lay = wordcraft_layout::layout(d, &mut wordcraft_layout::LayoutCache::new(), &wordcraft_layout::LayoutOptions::default());
    for (k, (i, level, s)) in headings.iter().enumerate() {
        let Some(page) = lay.page_of(&Pos::body(offset + i, 0)) else { continue };
        if let Some(slot) = d.body.get_mut(first_entry + k) {
            *slot = para_block(toc_entry(s, *level, &(page + 1).to_string(), text_w, accent));
        }
    }
}

/// Write a composed document in `format`.
pub fn export(doc: &Document, format: Format) -> Result<Vec<u8>, AppError> {
    match format {
        Format::Docx => wordcraft_docx::write(doc).map_err(|e| internal("Word export failed", e)),
        Format::Pdf => {
            let opts = wordcraft_pdf::PdfOptions {
                title: Some(doc.core.title.clone()).filter(|s| !s.is_empty()),
                author: Some(doc.core.creator.clone()).filter(|s| !s.is_empty()),
                ..Default::default()
            };
            wordcraft_pdf::export(doc, &opts).map_err(|e| internal("PDF export failed", e))
        }
        Format::Odt => doc_to_odt(doc),
        f => wordcraft_formats::export(f.ext(), doc)
            .ok_or_else(|| AppError::BadRequest(format!("Cannot export as .{}", f.ext())))?
            .map_err(|e| internal("Export failed", e)),
    }
}

/* ── Fonts for the window ───────────────────────────────────── */

/// The faces WordCraft bundles, in `wordcraft_fonts::bundled()` order:
/// (file name, CSS family, weight, italic). The window loads them from the
/// plugin's own route (same origin, no network), so the page it shows is set
/// in the faces the PDF embeds.
pub const FONT_FILES: &[(&str, &str, u16, bool)] = &[
    ("SourceSans3-Regular.ttf", "Source Sans 3", 400, false),
    ("SourceSans3-Semibold.ttf", "Source Sans 3", 600, false),
    ("SourceSans3-Bold.ttf", "Source Sans 3", 700, false),
    ("SourceSans3-It.ttf", "Source Sans 3", 400, true),
    ("SourceSerif4-Regular.ttf", "Source Serif 4", 400, false),
    ("SourceSerif4-It.ttf", "Source Serif 4", 400, true),
    ("SourceSerif4-Semibold.ttf", "Source Serif 4", 600, false),
    ("SourceSerif4-Bold.ttf", "Source Serif 4", 700, false),
    ("SourceSerif4-BoldIt.ttf", "Source Serif 4", 700, true),
    ("Inter-Regular.ttf", "Inter", 400, false),
    ("Inter-Medium.ttf", "Inter", 500, false),
    ("Inter-SemiBold.ttf", "Inter", 600, false),
    ("JetBrainsMono-Regular.ttf", "JetBrains Mono", 400, false),
];

/// A bundled font file by name.
pub fn font_file(name: &str) -> Option<&'static [u8]> {
    let i = FONT_FILES.iter().position(|(f, ..)| *f == name)?;
    wordcraft_fonts::bundled().get(i).copied()
}

/* ── Page preview ───────────────────────────────────────────── */

/// A composed document laid out into pages: what export will produce.
pub struct Preview {
    pub doc: Document,
    pub layout: wordcraft_layout::DocLayout,
}

impl Preview {
    pub fn new(content: &Document, title: &str, design: &Design) -> Preview {
        let doc = compose(content, title, design);
        let layout = wordcraft_layout::layout(&doc, &mut wordcraft_layout::LayoutCache::new(), &wordcraft_layout::LayoutOptions::default());
        Preview { doc, layout }
    }

    /// Page sizes in points.
    pub fn pages(&self) -> Vec<(f32, f32)> {
        self.layout.pages.iter().map(|p| (p.w, p.h)).collect()
    }

    /// Page `index` as a PNG, `scale` pixels per point (clamped 0.25–4).
    pub fn page_png(&self, index: usize, scale: f32) -> Result<Vec<u8>, AppError> {
        let page = self.layout.pages.get(index).ok_or_else(|| AppError::NotFound(format!("No page {}", index + 1)))?;
        let scale = if scale.is_finite() { scale.clamp(0.25, 4.0) } else { 1.5 };
        let r = wordcraft_render::render_page(&self.doc, page, scale, &wordcraft_render::RenderOptions::default());
        let img = image::RgbaImage::from_raw(r.width, r.height, r.to_straight())
            .ok_or_else(|| AppError::Internal("Page render failed".into()))?;
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| internal("PNG encode failed", e))?;
        Ok(out.into_inner())
    }
}

/* ── Pictures ───────────────────────────────────────────────── */

/// Widest picture in the text, CSS px (≈ the A4 text width).
pub const PICTURE_MAX_W: u32 = 600;
/// Longest side kept in the file.
const PICTURE_MAX_SIDE: u32 = 1600;

/// A picture file as an `<img>` the editor and the codecs take: small PNG and
/// JPEG are kept as they are, anything else is scaled and re-encoded
/// (JPEG, or PNG when it has transparency). `width_pct` (10–100) sizes it
/// against the text width.
pub fn picture_html(bytes: &[u8], alt: &str, width_pct: f32) -> Result<String, AppError> {
    let fmt = image::guess_format(bytes).map_err(|_| AppError::BadRequest("Not a picture Word can read".into()))?;
    let img = image::load_from_memory_with_format(bytes, fmt).map_err(|e| AppError::BadRequest(format!("Could not read the picture: {e}")))?;
    let (w0, h0) = (img.width().max(1), img.height().max(1));
    let keep = matches!(fmt, image::ImageFormat::Png | image::ImageFormat::Jpeg) && w0.max(h0) <= PICTURE_MAX_SIDE && bytes.len() <= 1_500_000;
    let (mime, data) = if keep {
        (if fmt == image::ImageFormat::Png { "image/png" } else { "image/jpeg" }, bytes.to_vec())
    } else {
        let scaled = if w0.max(h0) > PICTURE_MAX_SIDE { img.resize(PICTURE_MAX_SIDE, PICTURE_MAX_SIDE, image::imageops::FilterType::Lanczos3) } else { img };
        let mut out = std::io::Cursor::new(Vec::new());
        if scaled.color().has_alpha() {
            scaled.write_to(&mut out, image::ImageFormat::Png).map_err(|e| internal("PNG encode failed", e))?;
            ("image/png", out.into_inner())
        } else {
            let rgb = image::DynamicImage::ImageRgb8(scaled.to_rgb8());
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 86).encode_image(&rgb).map_err(|e| internal("JPEG encode failed", e))?;
            ("image/jpeg", out.into_inner())
        }
    };
    let pct = if width_pct.is_finite() { width_pct.clamp(10.0, 100.0) } else { 100.0 };
    let max_w = (PICTURE_MAX_W as f32 * pct / 100.0).round() as u32;
    let w = w0.min(max_w).max(1);
    let h = ((h0 as f32) * (w as f32 / w0 as f32)).round().max(1.0) as u32;
    let alt = wordcraft_formats::html::esc(alt);
    Ok(format!("<img src=\"data:{mime};base64,{}\" alt=\"{alt}\" width=\"{w}\" height=\"{h}\">", base64_encode(&data)))
}

fn base64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Insert `block_html` after the block containing `after` (case-insensitive,
/// matched on the escaped text), else at the end. Returns whether `after`
/// was found.
pub fn insert_after_block(html: &str, after: Option<&str>, block_html: &str) -> (String, bool) {
    if let Some(needle) = after.map(str::trim).filter(|s| !s.is_empty()) {
        let esc = wordcraft_formats::html::esc(needle);
        // ASCII folding keeps byte offsets aligned with `html`.
        let lower = html.to_ascii_lowercase();
        if let Some(at) = lower.find(&esc.to_ascii_lowercase()) {
            // The end of the enclosing block: the first closing block tag after the match.
            let ends = ["</p>", "</h1>", "</h2>", "</h3>", "</h4>", "</h5>", "</h6>", "</li>", "</blockquote>", "</table>"];
            if let Some(end) = ends.iter().filter_map(|t| lower[at..].find(t).map(|i| at + i + t.len())).min() {
                // Inside a list or a table: go past the whole list/table.
                let mut cut = end;
                for (open, close) in [("<ul", "</ul>"), ("<ol", "</ol>"), ("<table", "</table>")] {
                    let opened = lower[..at].rfind(open);
                    let closed = lower[..at].rfind(close);
                    if opened.is_some() && opened > closed {
                        if let Some(i) = lower[at..].find(close) {
                            cut = cut.max(at + i + close.len());
                        }
                    }
                }
                let mut out = String::with_capacity(html.len() + block_html.len());
                out.push_str(&html[..cut]);
                out.push_str(block_html);
                out.push_str(&html[cut..]);
                return (out, true);
            }
        }
        return (format!("{html}{block_html}"), false);
    }
    (format!("{html}{block_html}"), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Document {
        markdown_to_doc(
            "# Report\n\nIntro paragraph with **bold**.\n\n## Findings\n\nOne.\nTwo.\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n## Next steps\n\n- first\n- second\n",
        )
    }

    #[test]
    fn markdown_roundtrip_keeps_structure() {
        let md = doc_to_markdown(&sample());
        assert!(md.contains("## Findings"), "{md}");
        assert!(md.contains("**bold**"), "{md}");
        assert!(md.contains("| A"), "{md}");
        // "One.\nTwo." are two paragraphs, not one.
        assert!(md.contains("One.\n\nTwo."), "{md}");
    }

    #[test]
    fn odt_storage_roundtrip_keeps_tables_and_fonts() {
        let html = "<h2>Plan</h2><p>Hello <b>world</b> <span style=\"font-family:'Source Serif 4'\">serif</span></p>\
                    <table><tr><td>a</td><td>b</td></tr></table><ul><li>x</li></ul>";
        let back = odt_to_html(&html_to_odt(html).unwrap()).unwrap();
        assert!(back.contains("<h2"), "{back}");
        assert!(back.contains("<strong>world</strong>") || back.contains("<b>world</b>"), "{back}");
        assert!(back.contains("Source Serif 4"), "{back}");
        assert!(back.contains("<table"), "{back}");
        assert!(back.contains("<li"), "{back}");
    }

    #[test]
    fn reads_documents_written_by_the_old_codec() {
        let old = shiny_plugin_sdk::odt::html_to_odt("t", "<h1>Old</h1><p>Some <b>bold</b> text</p>").unwrap();
        let md = odt_to_markdown(&old).unwrap();
        assert!(md.contains("# Old"), "{md}");
        assert!(md.contains("**bold**"), "{md}");
    }

    #[test]
    fn compose_and_export_every_format() {
        let design = Design {
            template: "modern".into(),
            cover: true,
            toc: true,
            header: true,
            page_numbers: true,
            subtitle: "Q3".into(),
            author: "Ada".into(),
            lang: "it".into(),
            ..Default::default()
        };
        let d = compose(&sample(), "Report", &design);
        // Cover title, subtitle, author, contents heading, three entries.
        let toc: Vec<String> = d.body.iter().filter_map(|b| b.as_para()).filter(|p| p.props.style.as_deref().is_some_and(|s| s.starts_with("TOC"))).map(|p| p.plain_text()).collect();
        assert_eq!(toc.first().map(String::as_str), Some("Indice"));
        assert_eq!(toc.len(), 3, "{toc:?}");
        assert!(toc[1].starts_with("Findings\t"), "{toc:?}");
        assert!(toc[1].trim_end().ends_with(char::is_numeric), "{toc:?}");
        for f in [Format::Docx, Format::Pdf, Format::Odt, Format::Markdown, Format::Html] {
            let bytes = export(&d, f).unwrap();
            assert!(!bytes.is_empty(), "{f:?}");
        }
        let pdf = export(&d, Format::Pdf).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        // The .docx reads back with its heading and table.
        let back = import("x.docx", &export(&d, Format::Docx).unwrap()).unwrap();
        let md = doc_to_markdown(&back);
        assert!(md.contains("Findings"), "{md}");
    }

    #[test]
    fn plain_design_leaves_content_alone() {
        let d = compose(&sample(), "Report", &Design::default());
        assert_eq!(d.body.len(), sample().body.len());
        assert!(d.last_section.headers.default.is_none());
    }

    #[test]
    fn pictures_survive_storage_and_export() {
        // A 2×2 red PNG, as the window inserts it.
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAFklEQVQI12P8z8DAwMDAxMDAwMDAAAANHQEDK+mmyAAAAABJRU5ErkJggg==";
        let html = format!("<p>Logo:</p><p><img src=\"data:image/png;base64,{PNG}\" alt=\"logo\" width=\"120\" height=\"120\"></p>");
        let odt = html_to_odt(&html).unwrap();
        let back = odt_to_html(&odt).unwrap();
        assert!(back.contains("<img src=\"data:image/png;base64,"), "{back}");
        assert!(back.contains("width=\"120\""), "{back}");
        let d = compose(&odt_to_doc(&odt).unwrap(), "Logo", &Design { template: "modern".into(), ..Default::default() });
        let docx = export(&d, Format::Docx).unwrap();
        assert!(docx.windows(11).any(|w| w == b"word/media/"), "no picture in the .docx");
        assert!(export(&d, Format::Pdf).unwrap().windows(6).any(|w| w == b"/Image"), "no picture in the PDF");
    }

    #[test]
    fn preview_renders_the_composed_pages() {
        let design = Design { template: "classic".into(), cover: true, margin_left: 30.0, ..Default::default() };
        let p = Preview::new(&sample(), "Report", &design);
        let pages = p.pages();
        assert!(pages.len() >= 2, "cover + content: {pages:?}");
        assert!((pages[0].0 - 595.3).abs() < 0.5, "A4 width");
        let png = p.page_png(1, 1.0).unwrap();
        assert!(png.starts_with(b"\x89PNG"));
        assert!(p.page_png(99, 1.0).is_err());
    }

    #[test]
    fn margins_reach_the_section() {
        let design = Design { margin_top: 20.0, margin_left: 30.0, ..Default::default() };
        let d = compose(&sample(), "R", &design);
        assert!((d.last_section.margin_left - 30.0 * 72.0 / 25.4).abs() < 0.01);
        assert!((d.last_section.margin_right - 25.0 * 72.0 / 25.4).abs() < 0.01);
        assert!(Design { margin_top: 2.0, ..Default::default() }.validate().is_err());
    }

    #[test]
    fn bundled_fonts_are_served() {
        assert_eq!(FONT_FILES.len(), wordcraft_fonts::bundled().len());
        let f = font_file("SourceSerif4-Regular.ttf").unwrap();
        assert!(f.starts_with(&[0, 1, 0, 0]) || f.starts_with(b"OTTO") || f.starts_with(b"true"));
        assert!(font_file("../etc/passwd").is_none());
    }

    #[test]
    fn pictures_are_sized_and_placed() {
        let mut big = image::RgbImage::new(2400, 1200);
        big.put_pixel(0, 0, image::Rgb([200, 10, 10]));
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(big).write_to(&mut png, image::ImageFormat::Png).unwrap();
        let tag = picture_html(png.get_ref(), "A & B", 50.0).unwrap();
        assert!(tag.starts_with("<img src=\"data:image/jpeg;base64,"), "re-encoded: {}", &tag[..40]);
        assert!(tag.contains("width=\"300\" height=\"150\""), "{}", &tag[tag.len() - 60..]);
        assert!(tag.contains("alt=\"A &amp; B\""));
        assert!(picture_html(b"not a picture", "", 100.0).is_err());

        let html = "<h2>Intro</h2><p>One</p><ul><li>Point</li><li>Two</li></ul><p>End</p>";
        let (out, found) = insert_after_block(html, Some("intro"), "<p>X</p>");
        assert!(found && out.starts_with("<h2>Intro</h2><p>X</p>"), "{out}");
        let (out, _) = insert_after_block(html, Some("Point"), "<p>X</p>");
        assert!(out.contains("</ul><p>X</p><p>End"), "after the whole list: {out}");
        let (out, found) = insert_after_block(html, Some("missing"), "<p>X</p>");
        assert!(!found && out.ends_with("<p>X</p>"));
        // The picture survives storage.
        let back = odt_to_html(&html_to_odt(&format!("<p>{tag}</p>")).unwrap()).unwrap();
        assert!(back.contains("<img"), "{back}");
    }

    #[test]
    fn import_sniffs_and_rejects() {
        let odt = html_to_odt("<p>hi</p>").unwrap();
        assert!(import("noext", &odt).is_ok());
        assert!(import("x.pdf", b"%PDF-1.7").is_err());
        assert!(import("x.exe", b"MZ").is_err());
    }

    #[test]
    fn design_validation() {
        assert!(Design { template: "fancy".into(), ..Default::default() }.validate().is_err());
        assert!(Design { accent: "blue".into(), ..Default::default() }.validate().is_err());
        assert!(Design { template: "classic".into(), accent: "#336699".into(), ..Default::default() }.validate().is_ok());
    }
}

/// `WORD_SAMPLE_DIR=/some/dir cargo test -p shiny-word-plugin --lib write_samples -- --ignored`
/// writes a composed sample in every theme there, to look at.
#[cfg(test)]
mod samples {
    use super::*;

    #[test]
    #[ignore]
    fn write_samples() {
        let Ok(dir) = std::env::var("WORD_SAMPLE_DIR") else { return };
        let md = "# Piano trimestrale\n\nQuesto documento riassume **obiettivi**, risultati e prossimi passi del trimestre.\n\n\
                  ## Obiettivi\n\nRidurre i tempi di risposta e migliorare la qualità del servizio.\n\n\
                  - Tempo medio sotto le 4 ore\n- Soddisfazione oltre il 90%\n\n\
                  ## Risultati\n\n| Indicatore | Obiettivo | Risultato |\n|---|---|---|\n| Tempo medio | 4 h | 3,2 h |\n| Soddisfazione | 90% | 93% |\n| Ticket chiusi | 1200 | 1315 |\n\n\
                  ### Dettaglio\n\nI risultati superano gli obiettivi su tutti gli indicatori.\n\n\
                  > Un buon servizio è quello che non si nota.\n\n\
                  ## Prossimi passi\n\n1. Estendere il turno serale\n2. Nuova base di conoscenza\n";
        let content = markdown_to_doc(md);
        for t in ["classic", "modern", "minimal"] {
            let design = Design {
                template: t.into(),
                cover: true,
                toc: true,
                header: true,
                page_numbers: true,
                subtitle: "Terzo trimestre 2026".into(),
                author: "Ufficio Operazioni".into(),
                date: "1 novembre 2026".into(),
                lang: "it".into(),
                ..Default::default()
            };
            let d = compose(&content, "Piano trimestrale", &design);
            for f in [Format::Pdf, Format::Docx] {
                std::fs::write(format!("{dir}/{t}.{}", f.ext()), export(&d, f).unwrap()).unwrap();
            }
        }
    }
}

