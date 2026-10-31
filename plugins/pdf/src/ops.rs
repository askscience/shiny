//! The PDF operations engine, used by both the agent tools and the REST
//! routes so the AI and the window always agree. Everything here is
//! synchronous and CPU-bound; callers run it off the async runtime.
//!
//! One model backs the whole plugin: lopdf for the object graph, plus the
//! plugin's own modules for the hard parts — `compose` (creation), `stream_edit`
//! (reading and editing text), `doc` (page tree, resources, appended content)
//! and `fonts` (Base-14 metrics and WinAnsi).
//!
//! Comments (highlights, notes, text boxes, drawings, links), page marks
//! (watermark, header & footer) and page organisation go through PdfCraft's
//! engine (`craft.rs`): real annotations with appearance streams that stay
//! editable and deletable.

use std::collections::HashSet;

use shiny_plugin_sdk::errors::AppError;

use crate::doc;

pub use crate::compose::CreateOptions;
pub use crate::doc::MAX_PAGES;


/* ── Read ───────────────────────────────────────────────────── */

pub fn page_count(bytes: &[u8]) -> Result<usize, AppError> {
    Ok(doc::load(bytes)?.get_pages().len())
}

/// Plain text of one page, in reading order (PdfCraft's text layout).
pub fn extract_text(bytes: &[u8], page: usize) -> Result<String, AppError> {
    let n = page_count(bytes)?;
    if page >= n {
        return Err(AppError::NotFound(format!("page {} out of range", page + 1)));
    }
    Ok(crate::craft::text(bytes, Some(&[page]))?.pop().map(|(_, t)| t).unwrap_or_default())
}

/// Plain text of every page, opening the document once.
pub fn extract_all_text(bytes: &[u8]) -> Result<Vec<String>, AppError> {
    Ok(crate::craft::text(bytes, None)?.into_iter().map(|(_, t)| t).collect())
}

/// Page size in points, as displayed (rotation applied).
pub fn page_size_pt(bytes: &[u8], page: usize) -> Result<(f32, f32), AppError> {
    let d = doc::load(bytes)?;
    let id = doc::page_id(&d, page)?;
    let mb = doc::media_box(&d, id);
    let (w, h) = (mb[2] - mb[0], mb[3] - mb[1]);
    Ok(if doc::rotation(&d, id) % 180 == 90 { (h, w) } else { (w, h) })
}

/// Reject files we cannot work with, and return the page count.
pub fn validate(bytes: &[u8]) -> Result<usize, AppError> {
    let n = page_count(bytes)?;
    if n == 0 || n > MAX_PAGES {
        Err(AppError::BadRequest(format!("unsupported page count: {n}")))
    } else {
        Ok(n)
    }
}

/* ── Create ─────────────────────────────────────────────────── */

/// Create a PDF. `format` = `"markdown"`, `"html"` or `"plain"`. `css` is
/// accepted for API compatibility but ignored — styling comes from structure.
pub fn create(content: &str, _css: &str, format: &str) -> Result<Vec<u8>, AppError> {
    crate::compose::create(content, format, &CreateOptions::default())
}

/// Create a PDF with options (title metadata, page numbers).
pub fn create_with(content: &str, format: &str, opts: &CreateOptions) -> Result<Vec<u8>, AppError> {
    crate::compose::create(content, format, opts)
}

/* ── Page operations ────────────────────────────────────────── */

/// Rotate pages clockwise by a multiple of 90° (`None` = every page).
pub fn rotate(bytes: &[u8], pages: Option<&[usize]>, degrees: i32) -> Result<Vec<u8>, AppError> {
    crate::craft::rotate(bytes, pages, degrees as i64)
}

/// The document becomes exactly `order` (0-based). A permutation goes through
/// PdfCraft; keeping a subset or repeating pages uses the page-tree rebuild.
pub fn select_pages(bytes: &[u8], order: &[usize]) -> Result<Vec<u8>, AppError> {
    let n = page_count(bytes)?;
    let mut sorted = order.to_vec();
    sorted.sort_unstable();
    if sorted == (0..n).collect::<Vec<_>>() {
        return crate::craft::reorder(bytes, order);
    }
    let mut d = doc::load(bytes)?;
    let ids = doc::page_ids(&d);
    let new: Vec<_> = order
        .iter()
        .map(|&p| ids.get(p).copied().ok_or_else(|| AppError::NotFound(format!("page {} out of range", p + 1))))
        .collect::<Result<_, _>>()?;
    doc::set_page_order(&mut d, &new)?;
    doc::save(&mut d)
}

pub fn delete_pages(bytes: &[u8], remove: &[usize]) -> Result<Vec<u8>, AppError> {
    let total = page_count(bytes)?;
    let remove: HashSet<usize> = remove.iter().copied().collect();
    if (0..total).all(|i| remove.contains(&i)) {
        return Err(AppError::BadRequest("cannot delete every page".into()));
    }
    let mut list: Vec<usize> = remove.into_iter().filter(|&p| p < total).collect();
    list.sort_unstable();
    crate::craft::delete_pages(bytes, &list)
}

/// A new document made of `pages` (0-based, in this order).
pub fn extract_pages(bytes: &[u8], pages: &[usize]) -> Result<Vec<u8>, AppError> {
    crate::craft::extract(bytes, pages)
}

/// Duplicate page `page`; the copy lands right after it.
pub fn duplicate_page(bytes: &[u8], page: usize) -> Result<Vec<u8>, AppError> {
    if page >= page_count(bytes)? {
        return Err(AppError::NotFound(format!("page {} out of range", page + 1)));
    }
    crate::craft::duplicate_page(bytes, page)
}

/// Insert `count` blank pages after page `after` (`None` = at the start),
/// sized like the neighbouring page.
pub fn insert_blank(bytes: &[u8], after: Option<usize>, count: usize) -> Result<Vec<u8>, AppError> {
    let n = page_count(bytes)?;
    let at = after.map(|a| (a + 1).min(n)).unwrap_or(0);
    let like = after.unwrap_or(0).min(n.saturating_sub(1));
    let mut out = bytes.to_vec();
    for _ in 0..count.clamp(1, 50) {
        out = crate::craft::insert_blank(&out, at, like)?;
    }
    Ok(out)
}

/// Append every page of `other` to `bytes`.
pub fn merge(bytes: &[u8], other: &[u8]) -> Result<Vec<u8>, AppError> {
    crate::craft::merge(bytes, other)
}

/* ── Inline text editing ────────────────────────────────────── */

/// One editable run of text on a page, as the viewer needs it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TextRun {
    pub text: String,
    /// `[x, y, w, h]` in PDF points, bottom-left origin; `y` is the bottom of
    /// the glyph box (baseline minus descent).
    pub rect: [f32; 4],
    /// Baseline y — what the editor sends back to pick this exact run.
    pub baseline: f32,
    pub font: String,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    /// `#rrggbb`, so the viewer can draw the overlay in the real colour.
    pub color: String,
}

fn hex(c: [f32; 3]) -> String {
    let to = |v: f32| -> u8 { (v.clamp(0.0, 1.0) * 255.0).round() as u8 };
    format!("#{:02x}{:02x}{:02x}", to(c[0]), to(c[1]), to(c[2]))
}

/// Every text run on a page. These come from the same scanner the editor
/// matches with, so every run the viewer offers can actually be edited.
pub fn page_text_runs(bytes: &[u8], page: usize) -> Result<Vec<TextRun>, AppError> {
    let d = doc::load(bytes)?;
    let id = doc::page_id(&d, page)?;
    Ok(crate::stream_edit::page_runs(&d, id)?
        .into_iter()
        .map(|r| {
            let lower = r.font.to_ascii_lowercase();
            TextRun {
                rect: [r.x, r.y - r.size * 0.22, r.w.max(r.size * 0.3), r.size * 1.15],
                baseline: r.y,
                bold: lower.contains("bold") || lower.contains("black") || lower.contains("heavy"),
                italic: lower.contains("italic") || lower.contains("oblique"),
                size: r.size,
                color: hex(r.color),
                font: r.font,
                text: r.text,
            }
        })
        .collect())
}

/// A single in-place edit coming back from the viewer.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TextEdit {
    /// The run's text as the viewer last saw it — the find side of the replace.
    pub match_text: String,
    /// `[x, y, w, h]` of that run: only the occurrence nearest to it changes,
    /// so editing one "Total" does not rewrite every "Total" on the page.
    pub match_rect: [f32; 4],
    /// The run's baseline, when the viewer has it (more precise than the rect).
    #[serde(default)]
    pub baseline: Option<f32>,
    /// Replacement text.
    pub text: String,
}

/// Apply in-place text edits to one page and return the new bytes.
pub fn edit_text_runs(bytes: &[u8], page: usize, edits: &[TextEdit]) -> Result<(Vec<u8>, usize), AppError> {
    if edits.is_empty() {
        return Err(AppError::BadRequest("no edits supplied".into()));
    }
    let mut out = bytes.to_vec();
    let mut applied = 0usize;
    for edit in edits {
        if edit.match_text.trim().is_empty() || edit.match_text == edit.text {
            continue;
        }
        let [x, y, _, h] = edit.match_rect;
        let baseline = edit.baseline.unwrap_or(y + h * 0.19);
        let (new_bytes, report) = crate::stream_edit::replace_text_target(
            &out,
            page,
            edit.match_text.trim(),
            edit.text.trim(),
            crate::stream_edit::Target::Near(x, baseline),
        )?;
        if report.replaced > 0 {
            out = new_bytes;
            applied += report.replaced;
        }
    }
    Ok((out, applied))
}

/* ── Marks: comments, links, watermark ──────────────────────── */

/// Parse `#rrggbb` (0..1 RGB).
pub fn parse_text_color(s: &str) -> Option<[f32; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let b = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(|v| v as f32 / 255.0);
    Some([b(0)?, b(2)?, b(4)?])
}

/// Add a comment through PdfCraft: real annotations with appearance streams,
/// which every viewer draws and which stay editable and deletable.
///
/// `rects` are `[x, y, width, height]` in PDF points (bottom-left origin); text
/// markup takes one rect per line. Kinds: `highlight`, `underline`,
/// `strikeout`, `squiggly`, `note`, `box`, `oval`, `free_text` (a text box),
/// `link` (`text` is the URL).
#[allow(clippy::too_many_arguments)]
pub fn annotate(
    bytes: &[u8],
    page: usize,
    kind: &str,
    rects: &[[f32; 4]],
    text: &str,
    color: Option<[f32; 3]>,
    font_size: f32,
    author: &str,
) -> Result<Vec<u8>, AppError> {
    use crate::craft::Mark;
    use pdfcraft_annot::Markup;
    let r = |v: &[f32; 4]| [v[0] as f64, v[1] as f64, v[2] as f64, v[3] as f64];
    let first = rects.first().map(r).ok_or_else(|| AppError::BadRequest("rect required".into()))?;
    let markup = |k| Mark::Markup { kind: k, rects: rects.iter().map(r).collect() };
    let mark = match kind.trim().to_ascii_lowercase().as_str() {
        "highlight" => markup(Markup::Highlight),
        "underline" => markup(Markup::Underline),
        "strikeout" | "strike" => markup(Markup::StrikeOut),
        "squiggly" => markup(Markup::Squiggly),
        "note" => {
            if text.trim().is_empty() {
                return Err(AppError::BadRequest("a note needs text".into()));
            }
            Mark::Note { at: [first[0], first[1] + first[3]] }
        }
        "box" | "rectangle" => Mark::Rectangle { rect: first },
        "oval" | "circle" => Mark::Oval { rect: first },
        "free_text" | "text" => {
            // A comfortable box when the caller only gave a point.
            let w = if first[2] > 20.0 { first[2] } else { (text.lines().map(|l| l.chars().count()).max().unwrap_or(10) as f64 * font_size as f64 * 0.55).max(60.0) };
            let lines = text.lines().count().max(1) as f64;
            let h = if first[3] > lines * font_size as f64 { first[3] } else { lines * font_size as f64 * 1.3 + 6.0 };
            let top = first[1] + first[3];
            Mark::TextBox { rect: [first[0], top - h, w, h], font_size: font_size as f64 }
        }
        "link" => Mark::Link { rect: first, url: text.to_string() },
        other => return Err(AppError::BadRequest(format!("unknown annotation kind: {other}"))),
    };
    let color = color.map(|c| [c[0] as f64, c[1] as f64, c[2] as f64]);
    let note = if kind == "link" { "" } else { text };
    Ok(crate::craft::add_mark(bytes, page, mark, note, color, author)?.0)
}

/// Freehand drawing (one or more strokes of `[x, y]` points).
pub fn ink(bytes: &[u8], page: usize, strokes: Vec<Vec<[f64; 2]>>, color: Option<[f32; 3]>, author: &str) -> Result<Vec<u8>, AppError> {
    let color = color.map(|c| [c[0] as f64, c[1] as f64, c[2] as f64]);
    Ok(crate::craft::add_mark(bytes, page, crate::craft::Mark::Ink { strokes }, "", color, author)?.0)
}

/// A translucent diagonal watermark on every page (replaces an earlier one).
pub fn add_watermark(bytes: &[u8], _pages: Option<&[usize]>, text: &str) -> Result<Vec<u8>, AppError> {
    crate::craft::watermark(bytes, text, 0.25)
}
