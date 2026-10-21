//! Content-stream text editing (lopdf).
//!
//! Why this exists next to `ops.rs`: a PDF content stream is append-only, so
//! pdf_oxide's `modify_text` re-emits a run *without* removing the original
//! glyphs — find & replace on an existing document produced overlapping text.
//! Faithful editing needs the raw operator stream, which is what lopdf exposes.
//!
//! Both the agent's `pdf_replace_text` tool and the window's inline editor go
//! through here, so the AI and the human have exactly the same capability.
//!
//! Two traps this handles that a naive implementation misses:
//!
//! 1. **Text often lives in Form XObjects, not the page stream.** A page's own
//!    stream may be a few hundred bytes while its header is a `/TPL1` XObject
//!    with thousands of operators. lopdf's own `replace_text` only scans the
//!    page stream, so it silently does nothing on such files.
//! 2. **Text is usually not one literal string.** It arrives as a `TJ` array
//!    with the glyphs split by kerning numbers — `[(T) -3 (I) 10 (M)]` — so
//!    byte matching finds nothing. We flatten the array, edit the text, then
//!    re-emit it with a kerning correction so the line keeps its original width.

use std::collections::HashMap;

use lopdf::content::Content;
use lopdf::{Document, Object, ObjectId};

use shiny_plugin_sdk::errors::AppError;

fn err(e: impl std::fmt::Display) -> AppError {
    AppError::BadRequest(format!("PDF text edit failed: {e}"))
}

/// How many replacements a call made, per page.
#[derive(Debug, Default, Clone, Copy)]
pub struct ReplaceReport {
    pub replaced: usize,
    pub streams_touched: usize,
}

/// Glyph widths for every font on the page, in 1000-unit em space, plus the
/// average width used when a code point is missing from `/Widths`.
#[derive(Default)]
struct FontMetrics {
    widths: HashMap<u8, f32>,
    default: f32,
}

impl FontMetrics {
    /// Advance width of `s` at `size`, in points.
    fn width_of(&self, s: &str, size: f32) -> f32 {
        s.bytes()
            .map(|b| self.widths.get(&b).copied().unwrap_or(self.default) / 1000.0 * size)
            .sum()
    }
}

fn page_font_metrics(doc: &Document, page_id: ObjectId) -> FontMetrics {
    let mut m = FontMetrics { default: 500.0, ..Default::default() };
    let Ok(fonts) = doc.get_page_fonts(page_id) else { return m };
    for (_name, d) in fonts {
        // A missing FirstChar means the array is indexed from 0.
        let first = d
            .get(b"FirstChar")
            .ok()
            .and_then(|o| o.as_i64().ok())
            .unwrap_or(0)
            .clamp(0, 255) as u8;
        if let Ok(Object::Array(ws)) = d.get(b"Widths") {
            for (i, o) in ws.iter().enumerate() {
                let v = o.as_float().or_else(|_| o.as_i64().map(|x| x as f32));
                if let Ok(v) = v {
                    let code = first.wrapping_add(i as u8);
                    m.widths.insert(code, v);
                }
            }
        }
    }
    m
}

/// Every XObject on the page that is a Form (the only kind that carries its own
/// content stream).
fn page_form_xobjects(doc: &Document, page_id: ObjectId) -> Vec<ObjectId> {
    let mut ids = Vec::new();
    let Ok((_, resource_ids)) = doc.get_page_resources(page_id) else { return ids };
    for rid in resource_ids {
        let Ok(d) = doc.get_dictionary(rid) else { continue };
        let Ok(xo) = d.get(b"XObject") else { continue };
        let Ok(xd) = xo.as_dict() else { continue };
        for (_name, v) in xd.iter() {
            if let Object::Reference(id) = v {
                if let Ok(Object::Stream(s)) = doc.get_object(*id) {
                    let is_form = s
                        .dict
                        .get(b"Subtype")
                        .ok()
                        .and_then(|o| o.as_name().ok())
                        .map(|n| n == b"Form")
                        .unwrap_or(false);
                    if is_form {
                        ids.push(*id);
                    }
                }
            }
        }
    }
    ids
}

/// Rewrite one content stream, replacing `old` with `new` in its text runs.
///
/// Returns `(new_stream_bytes, replacements)`.
fn rewrite_stream(
    raw: &[u8],
    old: &str,
    new: &str,
    size: f32,
    metrics: &FontMetrics,
) -> Result<(Vec<u8>, usize), AppError> {
    let content = Content::decode(raw).map_err(err)?;
    let mut ops = content.operations.clone();
    let mut hits = 0usize;

    for op in ops.iter_mut() {
        if op.operator == "Tj" {
            for o in op.operands.iter_mut() {
                if let Object::String(bs, _) = o {
                    let t = String::from_utf8_lossy(bs).to_string();
                    if t.contains(old) {
                        *bs = t.replace(old, new).into_bytes();
                        hits += 1;
                    }
                }
            }
            continue;
        }
        if op.operator != "TJ" {
            continue;
        }
        for o in op.operands.iter_mut() {
            let Object::Array(a) = o else { continue };
            let text = flatten_tj(a);
            if !text.contains(old) {
                continue;
            }
            let replaced = text.replace(old, new);

            // Keep the line the same length. In a TJ array a positive number
            // advances the pen, so the correction goes *after* the string: the
            // replacement then starts at the original x and the width
            // difference is absorbed before whatever follows. Putting it first
            // shifts the replacement left of its own margin.
            let ow = metrics.width_of(&text, size);
            let nw = metrics.width_of(&replaced, size);
            let mut rebuilt: Vec<Object> =
                vec![Object::string_literal(replaced.clone().into_bytes())];
            if (nw - ow).abs() > 0.01 {
                let kern = ((nw - ow) / size * 1000.0).round() as i64;
                if kern != 0 {
                    rebuilt.push(Object::Integer(kern));
                }
            }
            *a = rebuilt;
            hits += 1;
        }
    }

    if hits == 0 {
        return Ok((raw.to_vec(), 0));
    }
    let encoded = Content { operations: ops }.encode().map_err(err)?;
    Ok((encoded, hits))
}

/// Flatten a `TJ` operand array into its visible text.
fn flatten_tj(arr: &[Object]) -> String {
    let mut s = String::new();
    for x in arr {
        if let Object::String(b, _) = x {
            s.push_str(&String::from_utf8_lossy(b));
        }
    }
    s
}


/// Find & replace `old` with `new` on `page`, across the page stream and every
/// Form XObject it draws.
///
/// Text replacement only. Restyling a run is deliberately not offered: a run's
/// `Tf`/`rg` are inherited graphics state that stay in force for every following
/// text object, and most runs in a real document never set their own `Tf` at
/// all, so a naive style edit either does nothing or repaints the rest of the
/// page. Doing it correctly needs save/restore of the inherited state around the
/// edit, which is separate work.
pub fn replace_text(
    bytes: &[u8],
    page: usize,
    old: &str,
    new: &str,
) -> Result<(Vec<u8>, ReplaceReport), AppError> {
    if old.is_empty() {
        return Err(AppError::BadRequest("old text must not be empty".into()));
    }
    let mut doc = Document::load_mem(bytes).map_err(err)?;
    let pages = doc.get_pages();
    // Both engines number pages from 0; lopdf's page map is 1-based.
    let page_id = *pages
        .get(&((page + 1) as u32))
        .ok_or_else(|| AppError::NotFound("page out of range".into()))?;

    let metrics = page_font_metrics(&doc, page_id);
    let mut report = ReplaceReport::default();

    // The page's own stream first, then every Form XObject it draws — most
    // files keep their stationary text (headers, footers, letterheads) in an
    // XObject, so skipping those would make find & replace look broken.
    let mut targets: Vec<ObjectId> = doc.get_page_contents(page_id);
    targets.extend(page_form_xobjects(&doc, page_id));

    // A single point size is enough to size the kerning correction; per-run
    // sizes are not tracked because the correction only has to keep the line
    // from colliding with what follows, not be metrically exact.
    let size = page_default_size(&doc, page_id);
    for id in targets {
        let Ok(Object::Stream(s)) = doc.get_object(id) else { continue };
        let raw = s.decompressed_content().unwrap_or_default();
        let (out, hits) = rewrite_stream(&raw, old, new, size, &metrics)?;
        if hits == 0 {
            continue;
        }
        report.replaced += hits;
        report.streams_touched += 1;
        if let Ok(Object::Stream(s)) = doc.get_object_mut(id) {
            // The bytes are now uncompressed, so the old /Filter must go.
            s.set_plain_content(out);
            s.dict.remove(b"Filter");
        }
    }

    if report.replaced == 0 {
        return Ok((bytes.to_vec(), report));
    }

    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(err)?;
    Ok((out, report))
}

/// The first `Tf` size found on the page, used to scale the kern correction.
fn page_default_size(doc: &Document, page_id: ObjectId) -> f32 {
    for id in doc.get_page_contents(page_id).into_iter().chain(page_form_xobjects(doc, page_id)) {
        let Ok(Object::Stream(s)) = doc.get_object(id) else { continue };
        let Ok(c) = Content::decode(&s.decompressed_content().unwrap_or_default()) else { continue };
        for op in &c.operations {
            if op.operator == "Tf" {
                if let Some(v) = op
                    .operands
                    .get(1)
                    .and_then(|o| o.as_float().ok().or_else(|| o.as_i64().ok().map(|x| x as f32)))
                {
                    return v;
                }
            }
        }
    }
    12.0
}
