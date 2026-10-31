//! The bridge to PdfCraft's engine (MIT OR Apache-2.0, pinned in Cargo.toml).
//!
//! PdfCraft does the PDF work it is best at: **real annotations** (comments
//! with generated appearance streams, so every viewer shows them and they can
//! be edited, moved or deleted later), **page marks** (watermark, header and
//! footer — removable as a unit), **page organisation**, and **text in reading
//! order**. Every function here is bytes in → bytes out: the plugin keeps its
//! own undo history (`store.rs`), so a PdfCraft session never outlives a call.

use std::sync::Arc;

use pdfcraft_annot::{self as annot, Markup, Meta, NewAnnotation, NoteIcon, Shape, Style};
use pdfcraft_cos::{Document, SaveOptions};
use serde::Serialize;

use shiny_plugin_sdk::errors::AppError;

fn bad(e: impl std::fmt::Display) -> AppError {
    AppError::BadRequest(e.to_string())
}

/* ── Open / save ─────────────────────────────────────────────── */

pub fn open(bytes: &[u8]) -> Result<Document, AppError> {
    Document::open(Arc::new(bytes.to_vec())).map_err(|e| AppError::BadRequest(format!("This file could not be read as a PDF ({e})")))
}

/// A full, compact save (the plugin's history keeps older versions, so the
/// file does not need an incremental revision chain).
pub fn save(doc: &Document) -> Result<Vec<u8>, AppError> {
    pdfcraft_cos::write_full(doc, &SaveOptions::default()).map_err(bad)
}

fn edit_doc(bytes: &[u8], f: impl FnOnce(&mut Document) -> Result<(), AppError>) -> Result<Vec<u8>, AppError> {
    let mut doc = open(bytes)?;
    f(&mut doc)?;
    save(&doc)
}

pub fn page_count(bytes: &[u8]) -> Result<usize, AppError> {
    pdfcraft_organize::page_count(&open(bytes)?).map_err(bad)
}

fn meta() -> Meta {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Meta { date: Some(pdf_date(now)), id: uuid::Uuid::new_v4().to_string() }
}

/// `D:YYYYMMDDHHmmSSZ` for a Unix time (UTC).
fn pdf_date(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("D:{y:04}{m:02}{d:02}{:02}{:02}{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/* ── Annotations (comments) ──────────────────────────────────── */

/// What the window or the agent asks to add.
#[derive(Debug, Clone)]
pub enum Mark {
    /// Text markup over one rect per line (`[x, y, w, h]`, bottom-left origin).
    Markup { kind: Markup, rects: Vec<[f64; 4]> },
    Note { at: [f64; 2] },
    Rectangle { rect: [f64; 4] },
    Oval { rect: [f64; 4] },
    /// A text box comment (FreeText): its text stays editable.
    TextBox { rect: [f64; 4], font_size: f64 },
    /// Freehand strokes.
    Ink { strokes: Vec<Vec<[f64; 2]>> },
    /// A web link over a rect.
    Link { rect: [f64; 4], url: String },
}

fn quad([x, y, w, h]: [f64; 4]) -> [f64; 8] {
    // Top-left, top-right, bottom-left, bottom-right (Acrobat's order).
    [x, y + h, x + w, y + h, x, y, x + w, y]
}

fn corners([x, y, w, h]: [f64; 4]) -> [f64; 4] {
    [x, y, x + w, y + h]
}

/// Add one comment. `text` is the comment's note (or the text box's text);
/// `color` is 0..1 RGB. Returns the new bytes and the comment's index on its page.
pub fn add_mark(
    bytes: &[u8],
    page: usize,
    mark: Mark,
    text: &str,
    color: Option<[f64; 3]>,
    author: &str,
) -> Result<(Vec<u8>, usize), AppError> {
    let mut doc = open(bytes)?;
    let count = pdfcraft_organize::page_count(&doc).map_err(bad)?;
    if page >= count {
        return Err(AppError::NotFound(format!("page {} out of range", page + 1)));
    }
    let index = if let Mark::Link { rect, url } = &mark {
        let url = url.trim();
        if url.is_empty() {
            return Err(AppError::BadRequest("a link needs an address".into()));
        }
        let url = if url.contains("://") || url.starts_with("mailto:") { url.to_string() } else { format!("https://{url}") };
        let style = annot::links::LinkStyle { visible: false, ..Default::default() };
        annot::links::add(&mut doc, page, corners(*rect), &annot::links::LinkAction::Uri(url), &style).map_err(bad)?
    } else {
        let default_color = match &mark {
            Mark::Markup { kind: Markup::Highlight, .. } | Mark::Note { .. } => [1.0, 0.82, 0.0],
            Mark::Markup { .. } => [0.86, 0.15, 0.15],
            Mark::TextBox { .. } => [0.1, 0.1, 0.1],
            _ => [0.9, 0.25, 0.2],
        };
        let shape = match mark {
            Mark::Markup { kind, rects } => {
                if rects.is_empty() {
                    return Err(AppError::BadRequest("nothing selected".into()));
                }
                Shape::TextMarkup { kind, quads: rects.into_iter().map(quad).collect() }
            }
            Mark::Note { at } => Shape::Note { at, icon: NoteIcon::Comment },
            Mark::Rectangle { rect } => Shape::Rectangle { rect: corners(rect) },
            Mark::Oval { rect } => Shape::Oval { rect: corners(rect) },
            Mark::TextBox { rect, font_size } => {
                if text.trim().is_empty() {
                    return Err(AppError::BadRequest("a text box needs text".into()));
                }
                Shape::TextBox { rect: corners(rect), font_size: font_size.clamp(4.0, 144.0) }
            }
            Mark::Ink { strokes } => {
                let strokes: Vec<Vec<[f64; 2]>> = strokes.into_iter().filter(|s| s.len() > 1).collect();
                if strokes.is_empty() {
                    return Err(AppError::BadRequest("nothing drawn".into()));
                }
                Shape::Ink { strokes }
            }
            Mark::Link { .. } => unreachable!(),
        };
        // A text box reads as text on the page: no frame around it.
        let width = if matches!(shape, Shape::TextBox { .. }) { 0.0 } else { Style::default().width };
        let style = Style { color: color.unwrap_or(default_color), width, ..Style::default() };
        let new = NewAnnotation { page, shape, style, contents: text.to_string(), author: author.to_string() };
        annot::add_annotation(&mut doc, &new, &meta()).map_err(bad)?
    };
    Ok((save(&doc)?, index))
}

/// A comment as the window's comment list shows it.
#[derive(Debug, Clone, Serialize)]
pub struct Comment {
    pub page: usize,
    pub index: usize,
    pub kind: String,
    pub author: Option<String>,
    pub text: Option<String>,
    pub modified: Option<String>,
    /// `[x0, y0, x1, y1]` in PDF points.
    pub rect: [f32; 4],
    pub color: Option<[f32; 3]>,
    /// Replies point at their parent `(page, index)`.
    pub parent: Option<(usize, usize)>,
}

/// Every comment in the document, page order. Popups (the boxes a viewer opens
/// for a note) are part of their comment, not comments of their own.
pub fn comments(bytes: &[u8]) -> Result<Vec<Comment>, AppError> {
    let doc = open(bytes)?;
    Ok(annot::summaries(&doc)
        .into_iter()
        .filter(|s| annot::is_comment_subtype(&s.subtype))
        .map(|s| Comment {
            page: s.page,
            index: s.index,
            kind: s.subtype,
            author: s.author,
            text: s.contents,
            modified: s.modified,
            rect: s.rect,
            color: s.color,
            parent: s.parent,
        })
        .collect())
}

pub fn delete_comment(bytes: &[u8], page: usize, index: usize) -> Result<Vec<u8>, AppError> {
    edit_doc(bytes, |doc| annot::delete_annotation(doc, page, index).map_err(bad))
}

pub fn set_comment_text(bytes: &[u8], page: usize, index: usize, text: &str) -> Result<Vec<u8>, AppError> {
    edit_doc(bytes, |doc| annot::set_contents(doc, page, index, text, &meta()).map_err(bad))
}

pub fn move_comment(bytes: &[u8], page: usize, index: usize, dx: f64, dy: f64) -> Result<Vec<u8>, AppError> {
    edit_doc(bytes, |doc| annot::move_annotation(doc, page, index, dx, dy, &meta()).map_err(bad))
}

pub fn reply(bytes: &[u8], page: usize, index: usize, text: &str, author: &str) -> Result<Vec<u8>, AppError> {
    if text.trim().is_empty() {
        return Err(AppError::BadRequest("a reply needs text".into()));
    }
    edit_doc(bytes, |doc| annot::add_reply(doc, page, index, text, author, &meta()).map(|_| ()).map_err(bad))
}

/* ── Page marks ──────────────────────────────────────────────── */

fn all_pages(doc: &Document) -> Result<Vec<usize>, AppError> {
    Ok((0..pdfcraft_organize::page_count(doc).map_err(bad)?).collect())
}

/// A translucent diagonal watermark on every page (replacing an earlier one).
pub fn watermark(bytes: &[u8], text: &str, opacity: f64) -> Result<Vec<u8>, AppError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(AppError::BadRequest("watermark text must not be empty".into()));
    }
    edit_doc(bytes, |doc| {
        let pages = all_pages(doc)?;
        let wm = pdfcraft_edit::Watermark {
            text: text.to_string(),
            opacity: opacity.clamp(0.05, 1.0),
            ..Default::default()
        };
        if let Some(c) = wm.first_undrawable() {
            return Err(AppError::BadRequest(format!("The character '{c}' cannot be used in a watermark")));
        }
        pdfcraft_edit::add_watermark(doc, &pages, &wm, true).map_err(bad)
    })
}

/// Header and footer text on every page. `slots` = header left / centre /
/// right, footer left / centre / right; tokens such as `<<1>>` (page number),
/// `<<n>>` (page count) and `<<dd.mm.yyyy>>` expand per page.
pub fn header_footer(bytes: &[u8], slots: [String; 6], font_size: f64) -> Result<Vec<u8>, AppError> {
    if slots.iter().all(|s| s.trim().is_empty()) {
        return Err(AppError::BadRequest("header and footer are empty".into()));
    }
    edit_doc(bytes, |doc| {
        let pages = all_pages(doc)?;
        let hf = pdfcraft_edit::HeaderFooter { text: slots, font_size: font_size.clamp(5.0, 24.0), ..Default::default() };
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let date = pdf_date(now);
        let y: i64 = date[2..6].parse().unwrap_or(1970);
        let m: u32 = date[6..8].parse().unwrap_or(1);
        let d: u32 = date[8..10].parse().unwrap_or(1);
        let cx = pdfcraft_edit::Context { date: (y, m, d) };
        pdfcraft_edit::add_header_footer(doc, &pages, &hf, true, &cx).map_err(bad)
    })
}

/// Which marks are present: `"watermark"`, `"header_footer"`, `"background"`.
pub fn marks(bytes: &[u8]) -> Result<Vec<String>, AppError> {
    let doc = open(bytes)?;
    Ok(pdfcraft_edit::marks_present(&doc)
        .into_iter()
        .map(|k| match k {
            pdfcraft_edit::MarkKind::Watermark => "watermark".to_string(),
            pdfcraft_edit::MarkKind::HeaderFooter => "header_footer".to_string(),
            other => format!("{other:?}").to_lowercase(),
        })
        .collect())
}

pub fn remove_marks(bytes: &[u8], kind: &str) -> Result<Vec<u8>, AppError> {
    let kind = match kind {
        "watermark" => pdfcraft_edit::MarkKind::Watermark,
        "header_footer" => pdfcraft_edit::MarkKind::HeaderFooter,
        other => return Err(AppError::BadRequest(format!("unknown mark kind: {other}"))),
    };
    edit_doc(bytes, |doc| {
        let pages = all_pages(doc)?;
        pdfcraft_edit::remove_marks(doc, &pages, kind).map(|_| ()).map_err(bad)
    })
}

/* ── Pages ───────────────────────────────────────────────────── */

pub fn rotate(bytes: &[u8], pages: Option<&[usize]>, degrees: i64) -> Result<Vec<u8>, AppError> {
    if degrees % 90 != 0 {
        return Err(AppError::BadRequest("degrees must be a multiple of 90".into()));
    }
    edit_doc(bytes, |doc| {
        let targets = match pages {
            Some(p) if !p.is_empty() => p.to_vec(),
            _ => all_pages(doc)?,
        };
        pdfcraft_organize::rotate_pages(doc, &targets, degrees).map_err(bad)
    })
}

pub fn delete_pages(bytes: &[u8], pages: &[usize]) -> Result<Vec<u8>, AppError> {
    edit_doc(bytes, |doc| pdfcraft_organize::delete_pages(doc, pages).map_err(bad))
}

pub fn duplicate_page(bytes: &[u8], page: usize) -> Result<Vec<u8>, AppError> {
    edit_doc(bytes, |doc| pdfcraft_organize::duplicate_pages(doc, &[page], Some(page + 1)).map_err(bad))
}

/// Insert a blank page sized like `like` at position `at`.
pub fn insert_blank(bytes: &[u8], at: usize, like: usize) -> Result<Vec<u8>, AppError> {
    edit_doc(bytes, |doc| {
        let boxes = pdfcraft_organize::page_boxes(doc).map_err(bad)?;
        let (w, h) = boxes
            .get(like)
            .map(|b| {
                let m = b[0];
                ((m[2] - m[0]).abs(), (m[3] - m[1]).abs())
            })
            .unwrap_or((595.0, 842.0));
        pdfcraft_organize::insert_blank_page(doc, at, w, h).map(|_| ()).map_err(bad)
    })
}

/// The document becomes exactly `order` (a permutation of its pages).
pub fn reorder(bytes: &[u8], order: &[usize]) -> Result<Vec<u8>, AppError> {
    edit_doc(bytes, |doc| {
        let n = pdfcraft_organize::page_count(doc).map_err(bad)?;
        let mut seen = vec![false; n];
        if order.len() != n || order.iter().any(|&p| p >= n || std::mem::replace(&mut seen[p], true)) {
            return Err(AppError::BadRequest("order must list every page exactly once".into()));
        }
        // Move pages into place one by one, tracking where each original page is.
        let mut current: Vec<usize> = (0..n).collect();
        for (target, &want) in order.iter().enumerate() {
            let pos = current.iter().position(|&p| p == want).unwrap();
            if pos != target {
                pdfcraft_organize::move_pages(doc, &[pos], target).map_err(bad)?;
                let p = current.remove(pos);
                current.insert(target, p);
            }
        }
        Ok(())
    })
}

/// Append every page of `other`.
pub fn merge(bytes: &[u8], other: &[u8]) -> Result<Vec<u8>, AppError> {
    let src = open(other)?;
    edit_doc(bytes, |doc| {
        let at = pdfcraft_organize::page_count(doc).map_err(bad)?;
        let pages = all_pages(&src)?;
        pdfcraft_organize::import_pages(doc, &src, &pages, at).map(|_| ()).map_err(bad)
    })
}

/// A new document made of `pages` (in that order).
pub fn extract(bytes: &[u8], pages: &[usize]) -> Result<Vec<u8>, AppError> {
    edit_doc(bytes, |doc| {
        let n = pdfcraft_organize::page_count(doc).map_err(bad)?;
        if pages.is_empty() || pages.iter().any(|&p| p >= n) {
            return Err(AppError::BadRequest("pages out of range".into()));
        }
        let drop: Vec<usize> = (0..n).filter(|p| !pages.contains(p)).collect();
        if !drop.is_empty() {
            pdfcraft_organize::delete_pages(doc, &drop).map_err(bad)?;
        }
        // Remaining pages are in document order; put them in the asked order.
        let mut kept: Vec<usize> = pages.to_vec();
        kept.sort_unstable();
        kept.dedup();
        let order: Vec<usize> = pages.iter().filter_map(|p| kept.iter().position(|k| k == p)).collect();
        let mut current: Vec<usize> = (0..kept.len()).collect();
        for (target, &want) in order.iter().enumerate() {
            let pos = current.iter().position(|&p| p == want).unwrap();
            if pos != target {
                pdfcraft_organize::move_pages(doc, &[pos], target).map_err(bad)?;
                let p = current.remove(pos);
                current.insert(target, p);
            }
        }
        Ok(())
    })
}

/* ── Text ────────────────────────────────────────────────────── */

/// Plain text of each page, in reading order.
pub fn text(bytes: &[u8], pages: Option<&[usize]>) -> Result<Vec<(usize, String)>, AppError> {
    let n = page_count(bytes)?;
    let mut r = pdfcraft_render::PageRenderer::new(Arc::new(bytes.to_vec()), Default::default());
    let wanted: Vec<usize> = match pages {
        Some(p) => p.iter().copied().filter(|&p| p < n).collect(),
        None => (0..n).collect(),
    };
    Ok(wanted
        .into_iter()
        .map(|page| {
            let req = pdfcraft_render::RenderRequest { page, kind: pdfcraft_render::RequestKind::Text, ..Default::default() };
            let text = r.render(req).text.map(|t| t.plain_text()).unwrap_or_default();
            (page, text)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_dates() {
        assert_eq!(pdf_date(0), "D:19700101000000Z");
        assert_eq!(pdf_date(1_793_448_000), "D:20261031120000Z");
    }
}
