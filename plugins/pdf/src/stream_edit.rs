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


/* ── Styling: point size, bold, colour ───────────────────────── */

/// What to change about a run's appearance. `None` leaves that attribute alone.
#[derive(Debug, Clone, Default)]
pub struct TextStyle {
    pub size: Option<f32>,
    pub bold: Option<bool>,
    /// `#rrggbb`
    pub color: Option<String>,
}

/// Graphics state that governs how text is painted.
///
/// This is stream-scoped, not per-run: `Tf` and the fill colour stay in force
/// for every following text object until something changes them.
#[derive(Debug, Clone, Default)]
struct Paint {
    font: Option<Vec<u8>>,
    size: Option<f32>,
    fill: Option<lopdf::content::Operation>,
}

fn fill_color_op(op: &lopdf::content::Operation) -> bool {
    matches!(
        (op.operator.as_str(), op.operands.len()),
        ("rg", 3) | ("g", 1) | ("k", 4)
    )
}

fn hex_to_rgb(s: &str) -> Option<(f32, f32, f32)> {
    let h = s.trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let b = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(|v| v as f32 / 255.0);
    Some((b(0)?, b(2)?, b(4)?))
}

/// Font resources reachable from a stream's own `/Resources`, as
/// `resource name -> BaseFont`.
fn resource_fonts(doc: &Document, id: ObjectId) -> Vec<(Vec<u8>, String)> {
    let mut out = Vec::new();
    let Ok(Object::Stream(s)) = doc.get_object(id) else { return out };
    let Ok(res) = s.dict.get(b"Resources") else { return out };
    let rd = match res {
        Object::Reference(r) => match doc.get_dictionary(*r) {
            Ok(d) => d.clone(),
            Err(_) => return out,
        },
        Object::Dictionary(d) => d.clone(),
        _ => return out,
    };
    let Ok(f) = rd.get(b"Font") else { return out };
    let fd = match f {
        Object::Reference(r) => match doc.get_dictionary(*r) {
            Ok(d) => d.clone(),
            Err(_) => return out,
        },
        Object::Dictionary(d) => d.clone(),
        _ => return out,
    };
    for (fname, fv) in fd.iter() {
        let base = match fv {
            Object::Reference(fid) => doc
                .get_dictionary(*fid)
                .ok()
                .and_then(|x| x.get(b"BaseFont").ok().map(|o| format!("{o:?}")))
                .unwrap_or_default(),
            _ => String::new(),
        };
        out.push((fname.clone(), base));
    }
    out
}

/// Fonts declared on the page dictionary itself.
fn page_stream_fonts(doc: &Document, page_id: ObjectId) -> Vec<(Vec<u8>, String)> {
    let mut out = Vec::new();
    let Ok(fonts) = doc.get_page_fonts(page_id) else { return out };
    for (name, d) in fonts {
        let base = d.get(b"BaseFont").ok().map(|o| format!("{o:?}")).unwrap_or_default();
        out.push((name.clone(), base));
    }
    out
}

fn base_of(fonts: &[(Vec<u8>, String)], n: &[u8]) -> Option<String> {
    let want = String::from_utf8_lossy(n).trim_start_matches('/').to_string();
    fonts
        .iter()
        .find(|(rn, _)| String::from_utf8_lossy(rn).trim_start_matches('/') == want)
        .map(|(_, b)| b.clone())
}

/// A bold resource for the run's own family, if the file carries one.
fn bold_resource(fonts: &[(Vec<u8>, String)], cur: Option<&[u8]>) -> Option<Vec<u8>> {
    let is_bold = |b: &str| b.to_lowercase().contains("bold");
    if let Some(c) = cur {
        if let Some(base) = base_of(fonts, c) {
            let stem = base.split(',').next().unwrap_or(&base).trim().to_lowercase();
            if let Some((n, _)) =
                fonts.iter().find(|(_, b)| is_bold(b) && b.to_lowercase().contains(&stem))
            {
                return Some(n.clone());
            }
        }
    }
    fonts.iter().find(|(_, b)| is_bold(b)).map(|(n, _)| n.clone())
}

/// A regular resource for the same family, to turn bold back off.
fn regular_resource(fonts: &[(Vec<u8>, String)], cur: Option<&[u8]>) -> Option<Vec<u8>> {
    let plain = |b: &str| {
        let l = b.to_lowercase();
        !l.contains("bold") && !l.contains("black") && !l.contains("heavy")
    };
    if let Some(c) = cur {
        if let Some(base) = base_of(fonts, c) {
            let stem = base.split(',').next().unwrap_or(&base).trim().to_lowercase();
            if let Some((n, _)) =
                fonts.iter().find(|(_, b)| plain(b) && b.to_lowercase().contains(&stem))
            {
                return Some(n.clone());
            }
        }
    }
    fonts.iter().find(|(_, b)| plain(b)).map(|(n, _)| n.clone())
}

/// The visible text of a `Tj`/`TJ` operation.
fn run_text(op: &lopdf::content::Operation) -> String {
    let mut s = String::new();
    for o in &op.operands {
        match o {
            Object::String(b, _) => s.push_str(&String::from_utf8_lossy(b)),
            Object::Array(a) => s.push_str(&flatten_tj(a)),
            _ => {}
        }
    }
    s
}

/// Restyle every text run on `page` whose text contains `old`.
///
/// Returns `(new_bytes, runs_restyled)`. Zero means nothing matched or nothing
/// could be changed — the caller must report that rather than claim success.
///
/// ## How it works, and the three things that went wrong before
///
/// `Tf` and the fill colour are **inherited graphics state**: they stay in force
/// for all following text objects. So the only safe edit is to change the
/// operator that already governs the run, and to put the inherited values back
/// afterwards.
///
/// 1. **The governing `Tf` is the last one *inside* the run's text object.**
///    This document emits `BT /F2 11.52 Tf Td TJ ET`, so a `Tf` inserted right
///    after `BT` is dead on arrival — the object's own one overrides it. Where
///    the object has no `Tf` (the run inherits from outside), one is injected
///    after `BT` and the inherited value is re-emitted after `ET`.
/// 2. **Never insert a second `Tf` in front of the first.** Doing so made the
///    *older* edit win, so `size:22` could not undo `size:30` and `bold:false`
///    could not undo `bold:true`. Overwriting the same slot is idempotent.
/// 3. **`q`/`Q` around the object is not the answer.** `Q` restores the state
///    from *before* the object, throwing away a `Tf` that a preceding sibling
///    set and the rest of the block inherits; that garbles the block.
///
/// This rebuilds the stream into a fresh vector and never mutates the original
/// operator list, so positions cannot drift.
///
/// ## Status: withdrawn — not reachable from the API
///
/// A single edit behaves correctly (verified: one changed band, correct size and
/// colour, neighbours untouched, logo and page intact). But it is **not
/// idempotent**: each call appends another `Tf`/undo pair instead of reusing the
/// slot, so calling it twice on the same document grows the operator stream and
/// the later value stops winning. There is no route registered for it.
///
/// Making it idempotent needs a way to recognise a previously injected operator
/// on re-parse — the stream alone cannot distinguish our `Tf` from the
/// document's own. Marking injections, or recording the run's byte offsets in
/// the stored document, would work; neither is implemented.
#[allow(dead_code)] // withdrawn from the API: see the note in its doc comment
pub fn style_text(
    bytes: &[u8],
    page: usize,
    old: &str,
    style: &TextStyle,
) -> Result<(Vec<u8>, usize), AppError> {
    if style.size.is_none() && style.bold.is_none() && style.color.is_none() {
        return Err(AppError::BadRequest("no style change requested".into()));
    }
    if old.is_empty() {
        return Err(AppError::BadRequest("old text must not be empty".into()));
    }
    let mut doc = Document::load_mem(bytes).map_err(err)?;
    let pages = doc.get_pages();
    let page_id = *pages
        .get(&((page + 1) as u32))
        .ok_or_else(|| AppError::NotFound("page out of range".into()))?;

    let mut targets: Vec<ObjectId> = doc.get_page_contents(page_id);
    targets.extend(page_form_xobjects(&doc, page_id));
    let page_fonts = page_stream_fonts(&doc, page_id);

    let mut restyled = 0usize;
    for id in targets {
        let Ok(Object::Stream(s)) = doc.get_object(id) else { continue };
        let raw = s.decompressed_content().unwrap_or_default();
        let Ok(content) = Content::decode(&raw) else { continue };
        let ops = content.operations;

        let mut fonts = resource_fonts(&doc, id);
        if fonts.is_empty() {
            fonts = page_fonts.clone();
        }

        // Which blocks hold a matching run, the `Tf` that governs it, and what
        // the state was when it was drawn.
        struct Block {
            et: usize,
            own_tf: Option<usize>,
            at_run: Paint,
            hits: Vec<usize>,
        }
        let mut blocks: Vec<Block> = Vec::new();
        let mut paint = Paint::default();
        let mut saves: Vec<Paint> = Vec::new();
        let mut cur: Option<Block> = None;
        for idx in 0..ops.len() {
            match ops[idx].operator.as_str() {
                "q" => saves.push(paint.clone()),
                "Q" => {
                    if let Some(p) = saves.pop() {
                        paint = p;
                    }
                }
                "Tf" => {
                    if let Some(Object::Name(n)) = ops[idx].operands.first() {
                        paint.font = Some(n.clone());
                    }
                    paint.size = ops[idx]
                        .operands
                        .get(1)
                        .and_then(|o| o.as_float().ok().or_else(|| o.as_i64().ok().map(|x| x as f32)))
                        .or(paint.size);
                    if let Some(b) = cur.as_mut() {
                        b.own_tf = Some(idx);
                    }
                }
                "rg" | "g" | "k" if fill_color_op(&ops[idx]) => {
                    paint.fill = Some(ops[idx].clone());
                }
                "BT" => {
                    cur = Some(Block {
                        et: usize::MAX,
                        own_tf: None,
                        at_run: paint.clone(),
                        hits: Vec::new(),
                    });
                }
                "ET" => {
                    if let Some(mut b) = cur.take() {
                        b.et = idx;
                        blocks.push(b);
                    }
                }
                "Tj" | "TJ" => {
                    if let Some(b) = cur.as_mut() {
                        if run_text(&ops[idx]).contains(old) {
                            if b.hits.is_empty() {
                                b.at_run = paint.clone();
                            }
                            b.hits.push(idx);
                        }
                    }
                }
                _ => {}
            }
        }
        let hit_blocks: Vec<&Block> =
            blocks.iter().filter(|b| !b.hits.is_empty() && b.et != usize::MAX).collect();
        if hit_blocks.is_empty() {
            continue;
        }

        // Per-operator overrides and insertions, keyed by original position so
        // the rewrite below stays a single clean pass.
        let mut insert_before: HashMap<usize, Vec<lopdf::content::Operation>> = HashMap::new();
        let mut insert_after: HashMap<usize, Vec<lopdf::content::Operation>> = HashMap::new();

        for b in &hit_blocks {
            // Style from the state at the run, not at `BT`: a run that sets its
            // own `Tf` must be able to un-bold back to *its* font.
            let base = &b.at_run;
            let want_size = style.size.or(base.size);
            let want_font = match style.bold {
                Some(true) => bold_resource(&fonts, base.font.as_deref()).or_else(|| base.font.clone()),
                Some(false) => regular_resource(&fonts, base.font.as_deref()).or_else(|| base.font.clone()),
                None => base.font.clone(),
            };
            let want_tf = if style.bold.is_some() || style.size.is_some() {
                match (want_font.clone(), want_size) {
                    (Some(f), Some(sz)) => Some((f, sz)),
                    _ => None,
                }
            } else {
                None
            };
            let color_op = style.color.as_deref().and_then(hex_to_rgb).map(|(r, g, b)| {
                let mut o = lopdf::content::Operation::new(
                    "rg",
                    vec![Object::Real(r), Object::Real(g), Object::Real(b)],
                );
                o.operator = "rg".into();
                o
            });

            // One uniform rule for both shapes of text object: leave any `Tf`
            // the object already has exactly where it is, and write the run's
            // state immediately before the run itself.
            //
            // Why not overwrite the object's `Tf`, tempting as it looks: that
            // operator is also what every following sibling text object
            // inherits, so changing it restyles the rest of the block. Why not
            // insert a *new* `Tf` in front of it: a later edit would then land in
            // front of the previous one and the *older* value would win, so
            // `size:22` could never undo a `size:30`.
            //
            // Writing immediately before the run sidesteps both, and the undo
            // after `ET` puts the inherited state back. Because the undo sits
            // directly after the matching `ET`, repeating the request overwrites
            // the same slot instead of accumulating operators.
            let mut ins: Vec<lopdf::content::Operation> = Vec::new();
            if let Some((f, sz)) = want_tf.clone() {
                let mut o = lopdf::content::Operation::new(
                    "Tf",
                    vec![Object::Name(f), Object::Real(sz)],
                );
                o.operator = "Tf".into();
                ins.push(o);
            }
            if let Some(c) = color_op.clone() {
                ins.push(c);
            }
            if ins.is_empty() {
                continue;   // this run cannot honour the request
            }
            let first = b.hits[0];
            insert_before.entry(first).or_default().extend(ins);

            let mut undo: Vec<lopdf::content::Operation> = Vec::new();
            if want_tf.is_some() {
                if let (Some(pf), Some(psz)) = (base.font.clone(), base.size) {
                    let mut o = lopdf::content::Operation::new(
                        "Tf",
                        vec![Object::Name(pf), Object::Real(psz)],
                    );
                    o.operator = "Tf".into();
                    undo.push(o);
                }
            }
            if color_op.is_some() {
                if let Some(pf) = base.fill.clone() {
                    undo.push(pf);
                }
            }
            if !undo.is_empty() {
                insert_after.entry(b.et).or_default().extend(undo);
            }

            restyled += b.hits.len();
        }

        // Rebuild the stream front-to-back; the original operator list is never
        // mutated, so no index can drift.
        let mut out_ops: Vec<lopdf::content::Operation> = Vec::with_capacity(ops.len() + 8);
        for idx in 0..ops.len() {
            if let Some(extra) = insert_before.get(&idx) {
                out_ops.extend(extra.iter().cloned());
            }
            out_ops.push(ops[idx].clone());
            if let Some(extra) = insert_after.get(&idx) {
                out_ops.extend(extra.iter().cloned());
            }
        }

        let out = Content { operations: out_ops }.encode().map_err(err)?;
        if let Ok(Object::Stream(s)) = doc.get_object_mut(id) {
            s.set_plain_content(out);
            s.dict.remove(b"Filter");
        }
    }

    if restyled == 0 {
        return Ok((bytes.to_vec(), 0));
    }
    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(err)?;
    Ok((out, restyled))
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

/// The text of a page as plain reading-order text, for the agent to inspect
/// before it edits anything.
pub fn page_text(bytes: &[u8], page: usize) -> Result<String, AppError> {
    let doc = Document::load_mem(bytes).map_err(err)?;
    doc.extract_text(&[(page + 1) as u32]).map_err(err)
}
