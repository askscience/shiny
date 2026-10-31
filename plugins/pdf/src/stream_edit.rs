//! Content-stream text editing (lopdf).
//!
//! A PDF content stream is append-only from pdf_oxide's point of view: its
//! `modify_text` re-emits a run without removing the original glyphs. Faithful
//! editing needs the raw operator stream, which is what lopdf exposes. Both the
//! agent's `pdf_replace_text` tool and the window's inline editor go through
//! here, so the AI and the human have exactly the same capability.
//!
//! What a naive "find the string in a `Tj`" implementation gets wrong, and what
//! this engine does instead:
//!
//! 1. **A visible line is rarely one operator.** Generators (including this
//!    plugin's own creator) emit one `Tj` per word with absolute `Tm`
//!    positioning, or a `TJ` array split by kerning numbers. The engine decodes
//!    every text-showing operator, groups them into *lines* (same baseline) and
//!    matches across operators within a line.
//! 2. **Bytes are not text.** Strings are font-encoded (WinAnsi, Differences,
//!    Identity-H + ToUnicode…). Each code is decoded with the font's encoding,
//!    and the replacement is encoded back with the same encoding. A character
//!    the font cannot draw (subset fonts!) is reported instead of silently
//!    becoming a blank.
//! 3. **Text often lives in Form XObjects**, not the page stream (letterheads,
//!    headers, templates). Their streams are scanned too.
//! 4. **Changing a word changes the line's width.** Words that follow on the
//!    same line and are placed with absolute positions are shifted by the width
//!    difference, stopping at the first wide gap (a table column, a tab stop),
//!    so the rest of the page is untouched.
//! 5. **Editing one line must not edit its twins.** The inline editor sends the
//!    position of the run it clicked; only the occurrence nearest to it changes.
//!
//! Styling existing text (bold/size/colour) is deliberately not offered here:
//! `Tf`/`rg` are inherited graphics state that leak into every following text
//! object. Added text (see `ops::annotate` `free_text`) carries its own style.

use std::collections::HashMap;
use std::rc::Rc;

use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Dictionary, Document, Encoding, Object, ObjectId, Stream, StringFormat};

use crate::doc::{self, deref, num};

use shiny_plugin_sdk::errors::AppError;

fn err(e: impl std::fmt::Display) -> AppError {
    AppError::BadRequest(format!("PDF text edit failed: {e}"))
}

/// How many replacements a call made.
#[derive(Debug, Default, Clone, Copy)]
pub struct ReplaceReport {
    pub replaced: usize,
    pub streams_touched: usize,
    pub pages_touched: usize,
}

/// Which occurrences to replace.
#[derive(Debug, Clone, Copy)]
pub enum Target {
    /// Every occurrence on the page.
    All,
    /// Only the occurrence whose start is nearest to this point (PDF points,
    /// bottom-left origin) — what the inline editor sends for a clicked line.
    Near(f32, f32),
}

/* ── Fonts ─────────────────────────────────────────────────────── */

/// Everything needed to decode, re-encode and measure one font's strings.
struct FontInfo {
    /// One-byte fonts: the text each code decodes to.
    table: Option<Vec<String>>,
    /// One-byte fonts: the code for each decoded character.
    rev: HashMap<String, u8>,
    /// Multi-byte fonts: the ToUnicode map, owned.
    umap: Option<Encoding<'static>>,
    /// Embedded (often subset) font: a code with no width has no glyph.
    embedded: bool,
    /// Bytes per character code: 2 for Type0 (Identity-H and friends), else 1.
    code_len: usize,
    /// Glyph widths by code, in 1000-unit glyph space.
    widths: HashMap<u32, f32>,
    default_w: f32,
    /// Base-14 name when the font has no `/Widths` (metrics come from the AFM).
    base14: Option<String>,
    /// `/BaseFont` without a subset prefix ("ABCDEF+Arial" → "Arial").
    name: String,
}

impl FontInfo {
    fn plain() -> Self {
        Self {
            table: None,
            rev: HashMap::new(),
            umap: None,
            embedded: false,
            code_len: 1,
            widths: HashMap::new(),
            default_w: 500.0,
            base14: None,
            name: String::new(),
        }
    }

    /// Resolve a font dictionary. A Type0 font's encoding comes from its
    /// ToUnicode map (lopdf cannot decode `Identity-H` by itself); one-byte
    /// fonts get a 256-entry decode table, so nothing borrows the document.
    fn new(doc: &Document, dict: &Dictionary) -> Self {
        let mut f = Self::build(doc, dict);
        if let Some(t) = &f.table {
            for (code, text) in t.iter().enumerate() {
                if !text.is_empty() {
                    f.rev.entry(text.clone()).or_insert(code as u8);
                }
            }
            // Prefer codes that actually have a glyph in a subset font.
            let better: Vec<(String, u8)> = f
                .rev
                .iter()
                .filter(|(_, c)| !f.has_glyph(&[**c]))
                .filter_map(|(t, _)| {
                    f.table.as_ref().unwrap().iter().enumerate()
                        .find(|(c, tt)| *tt == t && f.has_glyph(&[*c as u8]))
                        .map(|(c, _)| (t.clone(), c as u8))
                })
                .collect();
            for (t, c) in better {
                f.rev.insert(t, c);
            }
        }
        f
    }

    fn build(doc: &Document, dict: &Dictionary) -> Self {
        let mut f = Self::plain();
        if let Some(n) = dict.get(b"BaseFont").ok().and_then(|o| o.as_name().ok()) {
            let n = String::from_utf8_lossy(n).to_string();
            f.name = match n.split_once('+') {
                Some((tag, rest)) if tag.len() == 6 => rest.to_string(),
                _ => n,
            };
        }
        let is_type0 = dict
            .get(b"Subtype")
            .ok()
            .and_then(|o| o.as_name().ok())
            .map(|n| n == b"Type0")
            .unwrap_or(false);
        let enc_dict = encoding_dict(dict);
        let enc = enc_dict.get_font_encoding(doc).ok();
        f.embedded = font_is_embedded(doc, dict);
        match enc {
            Some(Encoding::UnicodeMapEncoding(m)) if is_type0 => f.umap = Some(Encoding::UnicodeMapEncoding(m)),
            Some(e) if !is_type0 => {
                let table: Vec<String> = (0..=255u8)
                    .map(|b| match e.bytes_to_string(&[b]) {
                        Ok(t) if !t.is_empty() && !t.contains('\u{FFFD}') && !t.contains('\0') => t,
                        _ => String::new(),
                    })
                    .collect();
                f.table = Some(table);
            }
            _ => {}
        }
        if is_type0 {
            f.code_len = 2;
            f.default_w = 1000.0;
            // Widths live in the descendant CIDFont's /W array.
            let desc = dict
                .get(b"DescendantFonts")
                .ok()
                .map(|o| deref(doc, o))
                .and_then(|o| o.as_array().ok())
                .and_then(|a| a.first())
                .map(|o| deref(doc, o))
                .and_then(|o| o.as_dict().ok());
            if let Some(d) = desc {
                if let Some(dw) = d.get(b"DW").ok().and_then(num) {
                    f.default_w = dw;
                }
                if let Some(w) = d.get(b"W").ok().map(|o| deref(doc, o)).and_then(|o| o.as_array().ok()) {
                    let mut i = 0;
                    while i < w.len() {
                        let Some(first) = num(deref(doc, &w[i])) else { break };
                        match w.get(i + 1).map(|o| deref(doc, o)) {
                            Some(Object::Array(ws)) => {
                                for (k, v) in ws.iter().enumerate() {
                                    if let Some(v) = num(deref(doc, v)) {
                                        f.widths.insert(first as u32 + k as u32, v);
                                    }
                                }
                                i += 2;
                            }
                            Some(last) => {
                                let (Some(last), Some(v)) =
                                    (num(last), w.get(i + 2).map(|o| deref(doc, o)).and_then(num))
                                else {
                                    break;
                                };
                                for c in first as u32..=last as u32 {
                                    f.widths.insert(c, v);
                                }
                                i += 3;
                            }
                            None => break,
                        }
                    }
                }
            }
            return f;
        }
        let first = dict.get(b"FirstChar").ok().and_then(num).unwrap_or(0.0) as u32;
        if let Some(ws) = dict.get(b"Widths").ok().map(|o| deref(doc, o)).and_then(|o| o.as_array().ok()) {
            for (i, o) in ws.iter().enumerate() {
                if let Some(v) = num(deref(doc, o)) {
                    f.widths.insert(first + i as u32, v);
                }
            }
            if let Some(avg) = average(f.widths.values()) {
                f.default_w = avg;
            }
        } else if crate::fonts::is_base14(&f.name) {
            f.base14 = Some(f.name.clone());
        }
        f
    }

    /// Decode one character code to text. Falls back to Latin-1 so an unknown
    /// encoding still yields *something* to match against.
    fn decode(&self, code: &[u8]) -> String {
        if let (Some(t), [b]) = (&self.table, code) {
            if !t[*b as usize].is_empty() {
                return t[*b as usize].clone();
            }
        }
        if let Some(enc) = &self.umap {
            if let Ok(s) = enc.bytes_to_string(code) {
                if !s.is_empty() && !s.contains('\u{FFFD}') {
                    return s;
                }
            }
        }
        code.iter().map(|&b| b as char).collect()
    }

    /// Does this code have a glyph? Embedded fonts are usually subsets, and a
    /// code outside the font's widths draws nothing.
    fn has_glyph(&self, code: &[u8]) -> bool {
        // CID fonts: a CID missing from /W uses /DW and still has a glyph;
        // their ToUnicode round trip is the check that matters.
        if !self.embedded || self.widths.is_empty() || self.code_len != 1 {
            return true;
        }
        let c = code.iter().fold(0u32, |acc, &b| (acc << 8) | b as u32);
        self.widths.get(&c).map_or(false, |w| *w > 0.0)
    }

    /// Encode text with this font, verifying every character can be drawn,
    /// so a missing glyph is an error rather than an invisible gap.
    fn encode(&self, text: &str) -> Result<Vec<u8>, AppError> {
        if text.is_empty() {
            return Ok(Vec::new());
        }
        if self.table.is_some() || self.umap.is_none() {
            if self.rev_is_empty() {
                // Unknown one-byte encoding: Latin-1.
                let mut v = Vec::new();
                for ch in text.chars() {
                    let c = ch as u32;
                    if c > 255 || !self.has_glyph(&[c as u8]) {
                        return Err(missing_glyph(ch));
                    }
                    v.push(c as u8);
                }
                return Ok(v);
            }
            let mut v = Vec::new();
            for ch in text.chars() {
                let code = self
                    .rev
                    .get(&ch.to_string())
                    .copied()
                    .or_else(|| (ch == ' ').then_some(32))
                    .filter(|c| self.has_glyph(&[*c]))
                    .ok_or_else(|| missing_glyph(ch))?;
                v.push(code);
            }
            return Ok(v);
        }
        let enc = self.umap.as_ref().unwrap();
        let bytes = enc.string_to_bytes(text);
        // Round trip, code by code: what we would draw must read back as `text`.
        let back: String = bytes.chunks(self.code_len).map(|c| self.decode(c)).collect();
        if back != text || !bytes.chunks(self.code_len).all(|c| self.has_glyph(c)) {
            let missing = text
                .chars()
                .find(|c| !back.contains(*c))
                .unwrap_or_else(|| text.chars().next().unwrap_or('?'));
            return Err(missing_glyph(missing));
        }
        Ok(bytes)
    }

    /// Encode `text` as units, falling back to a positional word space when
    /// the font has no space glyph (TeX fonts usually do not: their spaces are
    /// kerning). A kerned space is the sentinel unit `([], " ")`.
    fn encode_units(&self, text: &str) -> Result<Vec<(Vec<u8>, String)>, AppError> {
        let to_units = |bytes: Vec<u8>| -> Vec<(Vec<u8>, String)> {
            bytes.chunks(self.code_len).map(|c| (c.to_vec(), self.decode(c))).collect()
        };
        match self.encode(text) {
            Ok(b) => Ok(to_units(b)),
            Err(e) if text.contains(' ') && self.encode(" ").is_err() => {
                let mut out = Vec::new();
                for (k, word) in text.split(' ').enumerate() {
                    if k > 0 {
                        out.push((Vec::new(), " ".to_string()));
                    }
                    out.extend(to_units(self.encode(word).map_err(|_| e.clone_msg())?));
                }
                Ok(out)
            }
            Err(e) => Err(e),
        }
    }

    fn rev_is_empty(&self) -> bool {
        self.rev.is_empty()
    }

    /// Advance width of `bytes` (whose decoded text is `text`) at `size`, in
    /// unscaled text-space units.
    fn width(&self, bytes: &[u8], text: &str, size: f32) -> f32 {
        if let Some(name) = &self.base14 {
            return crate::fonts::text_width(text, name, size);
        }
        bytes
            .chunks(self.code_len)
            .map(|c| {
                let code = c.iter().fold(0u32, |acc, &b| (acc << 8) | b as u32);
                self.widths.get(&code).copied().unwrap_or(self.default_w)
            })
            .sum::<f32>()
            / 1000.0
            * size
    }
}

fn font_is_embedded(doc: &Document, dict: &Dictionary) -> bool {
    let has_file = |d: &Dictionary| {
        d.get(b"FontDescriptor")
            .ok()
            .map(|o| deref(doc, o))
            .and_then(|o| o.as_dict().ok())
            .map_or(false, |fd| fd.has(b"FontFile") || fd.has(b"FontFile2") || fd.has(b"FontFile3"))
    };
    if has_file(dict) {
        return true;
    }
    dict.get(b"DescendantFonts")
        .ok()
        .map(|o| deref(doc, o))
        .and_then(|o| o.as_array().ok())
        .and_then(|a| a.first())
        .map(|o| deref(doc, o))
        .and_then(|o| o.as_dict().ok())
        .map_or(false, has_file)
}

fn average<'a>(it: impl Iterator<Item = &'a f32>) -> Option<f32> {
    let (sum, n) = it.filter(|v| **v > 0.0).fold((0.0, 0), |(s, n), v| (s + v, n + 1));
    (n > 0).then(|| sum / n as f32)
}

trait CloneMsg {
    fn clone_msg(&self) -> AppError;
}

impl CloneMsg for AppError {
    fn clone_msg(&self) -> AppError {
        AppError::BadRequest(self.to_string())
    }
}

/// Word space used when a font has no space glyph, in thousandths of an em.
const KERNED_SPACE: i64 = 300;

fn missing_glyph(ch: char) -> AppError {
    AppError::BadRequest(format!(
        "The font used by this text does not contain the character '{ch}'. \
         PDFs often embed only the letters they already use — try a different \
         wording, or add new text with the Add text tool instead."
    ))
}

/// A Type0 font's encoding is resolved from its ToUnicode map, so drop the
/// `/Encoding` (usually `Identity-H`, which lopdf cannot decode by itself).
fn encoding_dict(d: &Dictionary) -> Dictionary {
    let mut e = d.clone();
    let is_type0 = d
        .get(b"Subtype")
        .ok()
        .and_then(|o| o.as_name().ok())
        .map(|n| n == b"Type0")
        .unwrap_or(false);
    if is_type0 && d.has(b"ToUnicode") {
        e.remove(b"Encoding");
    }
    e
}

/* ── Text geometry ─────────────────────────────────────────────── */

type Mat = [f32; 6];
const IDENT: Mat = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `m1 × m2` in PDF row-vector convention.
fn mul(m1: Mat, m2: Mat) -> Mat {
    [
        m1[0] * m2[0] + m1[1] * m2[2],
        m1[0] * m2[1] + m1[1] * m2[3],
        m1[2] * m2[0] + m1[3] * m2[2],
        m1[2] * m2[1] + m1[3] * m2[3],
        m1[4] * m2[0] + m1[5] * m2[2] + m2[4],
        m1[4] * m2[1] + m1[5] * m2[3] + m2[5],
    ]
}

fn mat_of(ops: &[Object]) -> Option<Mat> {
    if ops.len() != 6 {
        return None;
    }
    let mut m = [0.0; 6];
    for (i, o) in ops.iter().enumerate() {
        m[i] = num(o)?;
    }
    Some(m)
}

/* ── Scanning a stream ─────────────────────────────────────────── */

/// One string inside a text-showing operator.
#[derive(Debug, Clone)]
struct Seg {
    /// Index of the operand holding the string (or the TJ array).
    operand: usize,
    /// Index inside the TJ array; `None` for a plain string operand.
    elem: Option<usize>,
    format: StringFormat,
    /// `(code bytes, decoded text)` per character code.
    units: Vec<(Vec<u8>, String)>,
    /// A kerning gap before this string wide enough to read as a word space
    /// (TeX and many generators never draw a space glyph: `(This)-250(is)`).
    space_before: bool,
}

/// A gap between two pieces of text wider than this many ems is a column or
/// tab stop, not a word space: runs split there and reflow stops there.
const COLUMN_GAP: f32 = 0.8;

/// A kerning number at or below this (thousandths of an em) is a word space.
const KERN_SPACE: f32 = -150.0;

/// One text-showing operator (`Tj`, `TJ`, `'`, `"`).
#[derive(Debug, Clone)]
struct Show {
    op: usize,
    font: Vec<u8>,
    size: f32,
    /// Start point in user space (after the stream's base matrix).
    x: f32,
    y: f32,
    /// End of the drawn text, user space x.
    end_x: f32,
    /// Horizontal scale text space → user space (Tm·CTM `a`).
    hscale: f32,
    /// The positioning operator that placed this show, if it was one of
    /// `Tm`/`Td`/`TD` directly before it.
    pos_op: Option<usize>,
    /// Fill colour when the text was drawn.
    color: [f32; 3],
    segs: Vec<Seg>,
}

/// A run of shows sharing a baseline: the unit matching works within.
struct Line {
    shows: Vec<usize>,
}

struct Scan {
    ops: Vec<Operation>,
    shows: Vec<Show>,
    lines: Vec<Line>,
    /// Every `Do` with the CTM in force there (to find Form XObjects).
    dos: Vec<(Vec<u8>, Mat)>,
}

fn scan_stream(raw: &[u8], fonts: &HashMap<Vec<u8>, Rc<FontInfo>>, base: Mat) -> Result<Scan, AppError> {
    let content = Content::decode(raw).map_err(err)?;
    let ops = content.operations;
    let plain = FontInfo::plain();

    let mut shows: Vec<Show> = Vec::new();
    let mut ctm = base;
    let mut stack: Vec<Mat> = Vec::new();
    let mut tm = IDENT;
    let mut tlm = IDENT;
    let mut leading = 0.0f32;
    let mut font: Vec<u8> = Vec::new();
    let mut size = 12.0f32;
    let mut char_sp = 0.0f32;
    let mut word_sp = 0.0f32;
    let mut hz = 1.0f32;
    let mut last_pos: Option<usize> = None;
    let mut fill = [0.0f32; 3];
    let mut fill_stack: Vec<[f32; 3]> = Vec::new();
    let mut dos: Vec<(Vec<u8>, Mat)> = Vec::new();

    for (i, op) in ops.iter().enumerate() {
        let o = &op.operands;
        match op.operator.as_str() {
            "q" => {
                stack.push(ctm);
                fill_stack.push(fill);
            }
            "Q" => {
                ctm = stack.pop().unwrap_or(base);
                fill = fill_stack.pop().unwrap_or([0.0; 3]);
            }
            "cm" => {
                if let Some(m) = mat_of(o) {
                    ctm = mul(m, ctm);
                }
            }
            "Do" => {
                if let Some(n) = o.first().and_then(|x| x.as_name().ok()) {
                    dos.push((n.to_vec(), ctm));
                }
            }
            "rg" | "sc" | "scn" if o.len() == 3 => {
                for (k, v) in o.iter().enumerate() {
                    fill[k] = num(v).unwrap_or(0.0);
                }
            }
            "g" => {
                let v = o.first().and_then(num).unwrap_or(0.0);
                fill = [v, v, v];
            }
            "k" if o.len() == 4 => {
                let c: Vec<f32> = o.iter().map(|v| num(v).unwrap_or(0.0)).collect();
                fill = [(1.0 - c[0]) * (1.0 - c[3]), (1.0 - c[1]) * (1.0 - c[3]), (1.0 - c[2]) * (1.0 - c[3])];
            }
            "BT" => {
                tm = IDENT;
                tlm = IDENT;
                last_pos = None;
            }
            "Tf" => {
                if let Some(n) = o.first().and_then(|x| x.as_name().ok()) {
                    font = n.to_vec();
                }
                if let Some(s) = o.get(1).and_then(num) {
                    size = s;
                }
            }
            "Tc" => char_sp = o.first().and_then(num).unwrap_or(0.0),
            "Tw" => word_sp = o.first().and_then(num).unwrap_or(0.0),
            "Tz" => hz = o.first().and_then(num).unwrap_or(100.0) / 100.0,
            "TL" => leading = o.first().and_then(num).unwrap_or(0.0),
            "Tm" => {
                if let Some(m) = mat_of(o) {
                    tm = m;
                    tlm = m;
                    last_pos = Some(i);
                }
            }
            "Td" | "TD" => {
                let tx = o.first().and_then(num).unwrap_or(0.0);
                let ty = o.get(1).and_then(num).unwrap_or(0.0);
                if op.operator == "TD" {
                    leading = -ty;
                }
                tlm = mul([1.0, 0.0, 0.0, 1.0, tx, ty], tlm);
                tm = tlm;
                last_pos = Some(i);
            }
            "T*" => {
                tlm = mul([1.0, 0.0, 0.0, 1.0, 0.0, -leading], tlm);
                tm = tlm;
                last_pos = None;
            }
            "Tj" | "TJ" | "'" | "\"" => {
                if op.operator == "'" || op.operator == "\"" {
                    tlm = mul([1.0, 0.0, 0.0, 1.0, 0.0, -leading], tlm);
                    tm = tlm;
                    last_pos = None;
                    if op.operator == "\"" {
                        word_sp = o.first().and_then(num).unwrap_or(word_sp);
                        char_sp = o.get(1).and_then(num).unwrap_or(char_sp);
                    }
                }
                let fi = fonts.get(&font).map(|f| &**f).unwrap_or(&plain);
                let start = mul(tm, ctm);
                let mut segs = Vec::new();
                let mut adv = 0.0f32; // text-space advance
                let mut gap_space = false;
                let take = |operand: usize, elem: Option<usize>, bytes: &[u8], format: StringFormat, segs: &mut Vec<Seg>, space_before: bool| -> f32 {
                    let units: Vec<(Vec<u8>, String)> = bytes
                        .chunks(fi.code_len)
                        .map(|c| (c.to_vec(), fi.decode(c)))
                        .collect();
                    let text: String = units.iter().map(|u| u.1.as_str()).collect();
                    let n = units.len() as f32;
                    let spaces = text.matches(' ').count() as f32;
                    segs.push(Seg { operand, elem, format, units, space_before });
                    (fi.width(bytes, &text, size) + char_sp * n + word_sp * spaces) * hz
                };
                for (k, x) in o.iter().enumerate() {
                    match x {
                        Object::String(b, f) => adv += take(k, None, b, *f, &mut segs, false),
                        Object::Array(a) => {
                            for (j, e) in a.iter().enumerate() {
                                match e {
                                    Object::String(b, f) => {
                                        adv += take(k, Some(j), b, *f, &mut segs, std::mem::take(&mut gap_space));
                                    }
                                    other => {
                                        if let Some(v) = num(other) {
                                            adv -= v / 1000.0 * size * hz;
                                            if v <= KERN_SPACE {
                                                gap_space = true;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                tm = mul([1.0, 0.0, 0.0, 1.0, adv, 0.0], tm);
                let end = mul(tm, ctm);
                let hscale = (start[0] * start[0] + start[1] * start[1]).sqrt().max(1e-6);
                shows.push(Show {
                    op: i,
                    font: font.clone(),
                    size,
                    x: start[4],
                    y: start[5],
                    end_x: end[4],
                    hscale,
                    pos_op: last_pos.take(),
                    color: fill,
                    segs,
                });
            }
            _ => {}
        }
    }

    // Lines: consecutive shows on the same baseline (a few % of the size).
    let mut lines: Vec<Line> = Vec::new();
    for (idx, s) in shows.iter().enumerate() {
        let tol = (s.size * s.hscale * 0.3).max(1.0);
        match lines.last_mut() {
            Some(l) if (shows[*l.shows.last().unwrap()].y - s.y).abs() <= tol => l.shows.push(idx),
            _ => lines.push(Line { shows: vec![idx] }),
        }
    }
    Ok(Scan { ops, shows, lines, dos })
}

/* ── Matching ──────────────────────────────────────────────────── */

/// Where a character of a line came from.
#[derive(Debug, Clone, Copy)]
struct Loc {
    show: usize,
    seg: usize,
    unit: usize,
}

/// A found occurrence: an inclusive range of units, start..=end.
#[derive(Debug, Clone, Copy)]
struct Hit {
    start: Loc,
    end: Loc,
    x: f32,
    y: f32,
}

/// Find `needle` in each line. Exact first; if that finds nothing anywhere,
/// whitespace-insensitive (spacing in PDFs is often positional, not a byte).
fn find_hits(scan: &Scan, needle: &str) -> Vec<Hit> {
    let exact = find_hits_with(scan, needle, false);
    if !exact.is_empty() {
        return exact;
    }
    find_hits_with(scan, needle, true)
}

/// The characters of a line with where each came from. Word spaces that are
/// only *positional* (a kerning gap inside a `TJ`, or a gap between two
/// operators) are included as virtual characters (`true`), so text reads and
/// matches the way it looks.
fn line_chars(scan: &Scan, line: &Line) -> Vec<(char, Loc, bool)> {
    let mut chars: Vec<(char, Loc, bool)> = Vec::new();
    let mut prev_end: Option<f32> = None;
    for &si in &line.shows {
        let s = &scan.shows[si];
        let size = s.size * s.hscale;
        let mut first = true;
        for (gi, seg) in s.segs.iter().enumerate() {
            let starts_space = seg.units.first().map_or(false, |u| u.1.starts_with(char::is_whitespace));
            let ends_space = chars.last().map_or(true, |c| c.0.is_whitespace());
            let gap_between_ops = first && prev_end.map_or(false, |e| s.x - e > size * 0.15);
            if (seg.space_before || gap_between_ops) && !starts_space && !ends_space {
                if let Some(&(_, loc, _)) = chars.last() {
                    chars.push((' ', loc, true));
                }
            }
            if !seg.units.is_empty() {
                first = false;
            }
            for (ui, (_, t)) in seg.units.iter().enumerate() {
                for c in t.chars() {
                    chars.push((c, Loc { show: si, seg: gi, unit: ui }, false));
                }
            }
        }
        prev_end = Some(s.end_x);
    }
    chars
}

fn find_hits_with(scan: &Scan, needle: &str, loose: bool) -> Vec<Hit> {
    let keep = |c: char| !(loose && c.is_whitespace());
    let pat: Vec<char> = needle.chars().filter(|c| keep(*c)).collect();
    if pat.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for line in &scan.lines {
        let chars: Vec<(char, Loc, bool)> = line_chars(scan, line).into_iter().filter(|c| keep(c.0)).collect();
        let mut i = 0;
        while i + pat.len() <= chars.len() {
            if chars[i..i + pat.len()].iter().map(|c| c.0).eq(pat.iter().copied()) {
                // A virtual space has no bytes of its own: never let one be
                // the edge of the edited range.
                let span = &chars[i..i + pat.len()];
                let (Some(a), Some(b)) = (span.iter().find(|c| !c.2), span.iter().rev().find(|c| !c.2)) else {
                    i += 1;
                    continue;
                };
                let (start, end) = (a.1, b.1);
                let s = &scan.shows[start.show];
                hits.push(Hit { start, end, x: s.x, y: s.y });
                i += pat.len();
            } else {
                i += 1;
            }
        }
    }
    hits
}

fn loc_key(l: Loc) -> (usize, usize, usize) {
    (l.show, l.seg, l.unit)
}

/* ── Rewriting ─────────────────────────────────────────────────── */

/// Apply `hits` (non-overlapping, in order) to the scanned stream.
fn apply_hits(
    scan: &mut Scan,
    hits: &[Hit],
    new: &str,
    fonts: &HashMap<Vec<u8>, Rc<FontInfo>>,
) -> Result<(), AppError> {
    let plain = FontInfo::plain();
    // Edited segment units, keyed by (show, seg); start from the scanned ones.
    let mut edited: HashMap<(usize, usize), Vec<(Vec<u8>, String)>> = HashMap::new();
    // Positioning shifts to apply: op index → dx in user space.
    let mut shifts: HashMap<usize, f32> = HashMap::new();
    // Per show, the TJ element ranges a hit covers (exclusive bounds; `None`
    // = open): kerning numbers inside them belong to the replaced text.
    let mut spans: HashMap<usize, Vec<(Option<usize>, Option<usize>)>> = HashMap::new();

    for hit in hits {
        let s0 = &scan.shows[hit.start.show];
        let fi = fonts.get(&s0.font).map(|f| &**f).unwrap_or(&plain);
        let new_units = fi.encode_units(new)?;

        // Width of what is removed vs. what is inserted, measured per show.
        let mut old_w = 0.0f32;
        let in_range = |l: Loc| loc_key(l) >= loc_key(hit.start) && loc_key(l) <= loc_key(hit.end);
        for si in hit.start.show..=hit.end.show {
            let s = &scan.shows[si];
            let f = fonts.get(&s.font).map(|f| &**f).unwrap_or(&plain);
            for (gi, seg) in s.segs.iter().enumerate() {
                for (ui, (b, t)) in seg.units.iter().enumerate() {
                    if in_range(Loc { show: si, seg: gi, unit: ui }) {
                        old_w += f.width(b, t, s.size) * s.hscale;
                    }
                }
            }
        }
        let new_w: f32 = new_units
            .iter()
            .map(|(b, t)| if b.is_empty() { KERNED_SPACE as f32 / 1000.0 * s0.size } else { fi.width(b, t, s0.size) })
            .sum::<f32>()
            * s0.hscale;
        let delta = new_w - old_w;

        for si in hit.start.show..=hit.end.show {
            let lo = (si == hit.start.show).then(|| scan.shows[si].segs[hit.start.seg].elem).flatten();
            let hi = (si == hit.end.show).then(|| scan.shows[si].segs[hit.end.seg].elem).flatten();
            spans.entry(si).or_default().push((lo, hi));
        }

        // Rebuild every touched segment's units.
        for si in hit.start.show..=hit.end.show {
            for gi in 0..scan.shows[si].segs.len() {
                let key = (si, gi);
                let units = edited
                    .entry(key)
                    .or_insert_with(|| scan.shows[si].segs[gi].units.clone())
                    .clone();
                let mut out = Vec::with_capacity(units.len());
                let mut touched = false;
                for (ui, u) in units.into_iter().enumerate() {
                    let loc = Loc { show: si, seg: gi, unit: ui };
                    if loc_key(loc) == loc_key(hit.start) {
                        // The replacement takes the place of the first unit.
                        out.extend(new_units.iter().cloned());
                        touched = true;
                    } else if in_range(loc) {
                        touched = true;
                    } else {
                        out.push(u);
                    }
                }
                if touched {
                    edited.insert(key, out);
                }
            }
        }

        // Reflow: shift the words that follow on the same line, up to the
        // first gap wider than a couple of spaces (a column or tab stop).
        if delta.abs() > 0.01 {
            if let Some(line) = scan.lines.iter().find(|l| l.shows.contains(&hit.end.show)) {
                let mut prev_end = scan.shows[hit.end.show].end_x;
                let mut chain_shifted = false;
                for &si in line.shows.iter().filter(|&&si| si > hit.end.show) {
                    let s = &scan.shows[si];
                    let gap = s.x - prev_end;
                    if gap > s.size * s.hscale * COLUMN_GAP || gap < -s.size * s.hscale {
                        break;
                    }
                    prev_end = s.end_x;
                    let Some(p) = s.pos_op else { continue };
                    match scan.ops[p].operator.as_str() {
                        "Tm" => {
                            *shifts.entry(p).or_default() += delta;
                            chain_shifted = true;
                        }
                        _ if !chain_shifted => {
                            *shifts.entry(p).or_default() += delta;
                            chain_shifted = true;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Write the segments back into the operators.
    let mut by_show: HashMap<usize, Vec<(usize, Vec<(Vec<u8>, String)>)>> = HashMap::new();
    for ((si, gi), units) in edited {
        by_show.entry(si).or_default().push((gi, units));
    }
    for (si, mut segs) in by_show {
        segs.sort_by_key(|(gi, _)| *gi);
        let show = scan.shows[si].clone();
        let op = &mut scan.ops[show.op];
        let mut drop: Vec<(usize, usize)> = Vec::new();
        // TJ elements replaced by several objects (text split by kerned spaces).
        let mut expand: HashMap<(usize, usize), Vec<Object>> = HashMap::new();
        for (gi, units) in segs {
            let seg = &show.segs[gi];
            // Strings separated by kerned-space sentinels.
            let mut pieces: Vec<Object> = Vec::new();
            let mut cur: Vec<u8> = Vec::new();
            for (b, _) in &units {
                if b.is_empty() {
                    pieces.push(Object::String(std::mem::take(&mut cur), seg.format));
                    pieces.push(Object::Integer(-KERNED_SPACE));
                } else {
                    cur.extend_from_slice(b);
                }
            }
            pieces.push(Object::String(cur, seg.format));
            let empty = pieces.len() == 1 && matches!(&pieces[0], Object::String(b, _) if b.is_empty());
            match seg.elem {
                None if pieces.len() == 1 => op.operands[seg.operand] = pieces.pop().unwrap(),
                None if op.operator == "Tj" => {
                    // A plain string cannot carry kerning: switch to TJ.
                    op.operator = "TJ".into();
                    op.operands[seg.operand] = Object::Array(pieces);
                }
                None => {
                    // `'` / `"` take a string only: drop the kerned spaces.
                    let joined: Vec<u8> = pieces.iter().filter_map(|p| p.as_str().ok()).flatten().copied().collect();
                    op.operands[seg.operand] = Object::String(joined, seg.format);
                }
                Some(j) => {
                    if empty {
                        drop.push((seg.operand, j));
                    } else if pieces.len() == 1 {
                        if let Object::Array(a) = &mut op.operands[seg.operand] {
                            a[j] = pieces.pop().unwrap();
                        }
                    } else {
                        expand.insert((seg.operand, j), pieces);
                    }
                }
            }
        }
        // Kerning inside a replaced span would leave gaps in the new text.
        let inside = |j: usize| {
            spans.get(&si).map_or(false, |v| {
                v.iter().any(|(lo, hi)| lo.map_or(true, |l| j > l) && hi.map_or(true, |h| j < h))
            })
        };
        for (k, operand) in op.operands.iter().enumerate() {
            if let Object::Array(a) = operand {
                for (j, e) in a.iter().enumerate() {
                    if num(e).is_some() && inside(j) {
                        drop.push((k, j));
                    }
                }
            }
        }
        drop.sort();
        drop.dedup();
        // Rebuild touched arrays in one pass (indices refer to the original).
        for k in 0..op.operands.len() {
            let Object::Array(a) = &op.operands[k] else { continue };
            if !drop.iter().any(|d| d.0 == k) && !expand.keys().any(|e| e.0 == k) {
                continue;
            }
            let mut out = Vec::with_capacity(a.len());
            for (j, e) in a.iter().enumerate() {
                if drop.contains(&(k, j)) {
                    continue;
                }
                match expand.remove(&(k, j)) {
                    Some(p) => out.extend(p),
                    None => out.push(e.clone()),
                }
            }
            op.operands[k] = Object::Array(out);
        }
    }

    for (p, dx) in shifts {
        let op = &mut scan.ops[p];
        match op.operator.as_str() {
            "Tm" => {
                if let Some(e) = op.operands.get(4).and_then(num) {
                    // Tm's translation is in user space units of the CTM; the
                    // delta is measured after CTM, so undo the CTM scale.
                    let a = op.operands.first().and_then(num).unwrap_or(1.0);
                    let hs = scan.shows.iter().find(|s| s.pos_op == Some(p)).map(|s| s.hscale).unwrap_or(1.0);
                    let unit = if a.abs() > 1e-6 { hs / a.abs() } else { 1.0 };
                    op.operands[4] = Object::Real(e + dx / unit.max(1e-6));
                }
            }
            "Td" | "TD" => {
                if let Some(tx) = op.operands.first().and_then(num) {
                    let hs = scan.shows.iter().find(|s| s.pos_op == Some(p)).map(|s| s.hscale).unwrap_or(1.0);
                    // Td is in line-matrix units: one unit moves `hscale`
                    // points in user space after the CTM.
                    op.operands[0] = Object::Real(tx + dx / hs.max(1e-6));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/* ── Page driver ───────────────────────────────────────────────── */

/// One stream to scan: the page's content (all its streams as one logical
/// stream — graphics state carries across them) or a Form XObject placed with
/// the CTM in force at its `Do`.
struct Job {
    /// `None` = the page content itself.
    form: Option<ObjectId>,
    scan: Scan,
    fonts: HashMap<Vec<u8>, Rc<FontInfo>>,
}

/// Decompression cap for a single content stream (guards against bombs).
const MAX_STREAM: usize = 256 * 1024 * 1024;

fn stream_bytes(s: &lopdf::Stream) -> Vec<u8> {
    s.decompressed_content_with_limit(MAX_STREAM).unwrap_or_else(|_| {
        if s.dict.has(b"Filter") { Vec::new() } else { s.content.clone() }
    })
}

fn fonts_in(doc: &Document, res: &Dictionary, cache: &mut HashMap<ObjectId, Rc<FontInfo>>) -> HashMap<Vec<u8>, Rc<FontInfo>> {
    let fonts = doc::sub_dict(doc, res, b"Font");
    let mut out = HashMap::new();
    for (n, v) in fonts.iter() {
        let info = match v {
            Object::Reference(id) => {
                if let Some(f) = cache.get(id) {
                    f.clone()
                } else {
                    let Ok(d) = doc.get_dictionary(*id) else { continue };
                    let f = Rc::new(FontInfo::new(doc, d));
                    cache.insert(*id, f.clone());
                    f
                }
            }
            Object::Dictionary(d) => Rc::new(FontInfo::new(doc, d)),
            _ => continue,
        };
        out.insert(n.clone(), info);
    }
    out
}

/// Scan a page: its content, then every Form XObject it (transitively)
/// draws. Each stream is parsed exactly once.
fn scan_page(doc: &Document, page_id: ObjectId) -> Vec<Job> {
    let mut cache = HashMap::new();
    let res = doc::resources(doc, page_id);
    let page_fonts = fonts_in(doc, &res, &mut cache);
    let mut raw = Vec::new();
    for id in doc.get_page_contents(page_id) {
        if let Ok(Object::Stream(s)) = doc.get_object(id) {
            raw.extend(stream_bytes(s));
            raw.push(b'\n');
        }
    }
    let mut jobs: Vec<Job> = Vec::new();
    let mut job_res: Vec<Dictionary> = Vec::new();
    if let Ok(scan) = scan_stream(&raw, &page_fonts, IDENT) {
        jobs.push(Job { form: None, scan, fonts: page_fonts.clone() });
        job_res.push(res.clone());
    }
    let mut seen: Vec<ObjectId> = Vec::new();
    let mut i = 0;
    while i < jobs.len() && jobs.len() < 64 {
        let xobjects = doc::sub_dict(doc, &job_res[i], b"XObject");
        let dos = jobs[i].scan.dos.clone();
        for (name, ctm) in dos {
            let Some(Object::Reference(id)) = xobjects.get(&name).ok() else { continue };
            if seen.contains(id) {
                continue;
            }
            let Ok(Object::Stream(s)) = doc.get_object(*id) else { continue };
            if s.dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok()) != Some(b"Form".as_slice()) {
                continue;
            }
            seen.push(*id);
            let own = s.dict.get(b"Resources").ok().map(|o| deref(doc, o)).and_then(|o| o.as_dict().ok()).cloned();
            let m = s.dict.get(b"Matrix").ok().and_then(|o| o.as_array().ok()).and_then(|a| mat_of(a)).unwrap_or(IDENT);
            let mut fonts = own.as_ref().map(|r| fonts_in(doc, r, &mut cache)).unwrap_or_default();
            for (k, v) in &page_fonts {
                fonts.entry(k.clone()).or_insert_with(|| v.clone());
            }
            if let Ok(scan) = scan_stream(&stream_bytes(s), &fonts, mul(m, ctm)) {
                jobs.push(Job { form: Some(*id), scan, fonts });
                job_res.push(own.unwrap_or_default());
            }
        }
        i += 1;
    }
    jobs
}

/// Replace text on one page of a loaded document. Returns the replacement
/// count and how many streams changed.
fn replace_on_page(
    doc: &mut Document,
    page_id: ObjectId,
    old: &str,
    new: &str,
    target: Target,
) -> Result<(usize, usize), AppError> {
    let mut jobs = scan_page(doc, page_id);
    let mut chosen: Vec<(usize, Vec<Hit>)> = jobs
        .iter()
        .enumerate()
        .map(|(ji, j)| (ji, find_hits(&j.scan, old)))
        .filter(|(_, h)| !h.is_empty())
        .collect();
    if let Target::Near(nx, ny) = target {
        // Vertical distance matters most: lines are what differ.
        let mut best: Option<(usize, Hit, f32)> = None;
        for (k, (_, hits)) in chosen.iter().enumerate() {
            for h in hits {
                let d = (h.x - nx).abs() + (h.y - ny).abs() * 4.0;
                if best.as_ref().map_or(true, |b| d < b.2) {
                    best = Some((k, *h, d));
                }
            }
        }
        chosen = match best {
            Some((k, h, _)) => vec![(chosen[k].0, vec![h])],
            None => Vec::new(),
        };
    }
    let mut replaced = 0;
    let streams = chosen.len();
    for (ji, hits) in chosen {
        let job = &mut jobs[ji];
        apply_hits(&mut job.scan, &hits, new, &job.fonts)?;
        let bytes = Content { operations: std::mem::take(&mut job.scan.ops) }.encode().map_err(err)?;
        replaced += hits.len();
        match job.form {
            Some(id) => {
                if let Ok(Object::Stream(s)) = doc.get_object_mut(id) {
                    s.dict.remove(b"Filter");
                    s.dict.remove(b"DecodeParms");
                    s.set_plain_content(bytes);
                }
            }
            None => {
                // The page's streams were edited as one: store them as one.
                let id = doc.add_object(Stream::new(dictionary! {}, bytes));
                doc.get_dictionary_mut(page_id).map_err(err)?.set("Contents", Object::Reference(id));
            }
        }
    }
    Ok((replaced, streams))
}

/* ── Reading: runs and plain text ──────────────────────────────── */

/// A run of text as the viewer shows it: one line segment, split at wide
/// gaps (table columns), with geometry in PDF points (bottom-left origin).
#[derive(Debug, Clone)]
pub struct RunGeom {
    pub text: String,
    pub x: f32,
    /// Baseline.
    pub y: f32,
    pub w: f32,
    /// Font size in user-space points.
    pub size: f32,
    pub font: String,
    pub color: [f32; 3],
}

/// Every visible run of text on a page — the same model the editor matches
/// against, so whatever the viewer offers to edit can be edited.
pub fn page_runs(doc: &Document, page_id: ObjectId) -> Result<Vec<RunGeom>, AppError> {
    let per_job: Vec<Vec<RunGeom>> = scan_page(doc, page_id).into_iter().map(|job| {
        let (scan, fonts) = (job.scan, job.fonts);
        let plain = FontInfo::plain();
        let mut runs: Vec<RunGeom> = Vec::new();
        for line in &scan.lines {
            // Split the line into runs at wide gaps (columns, tab stops).
            let mut groups: Vec<Vec<usize>> = Vec::new();
            for &si in &line.shows {
                let s = &scan.shows[si];
                let size = s.size * s.hscale;
                match groups.last_mut() {
                    Some(g) => {
                        let prev = &scan.shows[*g.last().unwrap()];
                        let gap = s.x - prev.end_x;
                        if gap > size * COLUMN_GAP || gap < -size * 2.0 {
                            groups.push(vec![si]);
                        } else {
                            g.push(si);
                        }
                    }
                    None => groups.push(vec![si]),
                }
            }
            for g in groups {
                let sub = Line { shows: g };
                let text: String = line_chars(&scan, &sub).into_iter().map(|c| c.0).collect();
                if text.trim().is_empty() {
                    continue;
                }
                let first = &scan.shows[sub.shows[0]];
                let last = &scan.shows[*sub.shows.last().unwrap()];
                // The run's font/colour: those of its widest show.
                let main = sub
                    .shows
                    .iter()
                    .map(|&i| &scan.shows[i])
                    .max_by(|a, b| (a.end_x - a.x).partial_cmp(&(b.end_x - b.x)).unwrap_or(std::cmp::Ordering::Equal))
                    .unwrap_or(first);
                let fi = fonts.get(&main.font).map(|f| &**f).unwrap_or(&plain);
                runs.push(RunGeom {
                    text,
                    x: first.x,
                    y: first.y,
                    w: (last.end_x - first.x).max(0.0),
                    size: main.size * main.hscale,
                    font: fi.name.clone(),
                    color: main.color,
                });
            }
        }
        runs
    }).collect();
    Ok(per_job
        .into_iter()
        .flatten()
        .filter(|r| !r.text.trim().is_empty())
        .collect())
}

/// Plain text of a page in reading order (top to bottom, left to right).
pub fn page_text(doc: &Document, page_id: ObjectId) -> Result<String, AppError> {
    let mut runs = page_runs(doc, page_id)?;
    runs.sort_by(|a, b| b.y.partial_cmp(&a.y).unwrap_or(std::cmp::Ordering::Equal));
    let mut lines: Vec<Vec<RunGeom>> = Vec::new();
    for r in runs {
        match lines.last_mut() {
            Some(l) if (l[0].y - r.y).abs() <= r.size.max(l[0].size) * 0.4 => l.push(r),
            _ => lines.push(vec![r]),
        }
    }
    let mut out = String::new();
    for mut l in lines {
        l.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
        let line: Vec<String> = l.iter().map(|r| r.text.trim().to_string()).collect();
        out.push_str(&line.join(" "));
        out.push('\n');
    }
    Ok(out)
}

/// Find & replace every occurrence of `old` on `page`.
pub fn replace_text(bytes: &[u8], page: usize, old: &str, new: &str) -> Result<(Vec<u8>, ReplaceReport), AppError> {
    replace_text_target(bytes, page, old, new, Target::All)
}

/// Find & replace on `page`, either everywhere or only near a point.
pub fn replace_text_target(
    bytes: &[u8],
    page: usize,
    old: &str,
    new: &str,
    target: Target,
) -> Result<(Vec<u8>, ReplaceReport), AppError> {
    if old.trim().is_empty() {
        return Err(AppError::BadRequest("old text must not be empty".into()));
    }
    let mut doc = doc::load(bytes)?;
    let page_id = doc::page_id(&doc, page)?;
    let (replaced, streams) = replace_on_page(&mut doc, page_id, old, new, target)?;
    let report = ReplaceReport { replaced, streams_touched: streams, pages_touched: (replaced > 0) as usize };
    if replaced == 0 {
        return Ok((bytes.to_vec(), report));
    }
    Ok((doc::save(&mut doc)?, report))
}

/// Find & replace across `pages` (every page when `None`).
pub fn replace_text_pages(
    bytes: &[u8],
    pages: Option<&[usize]>,
    old: &str,
    new: &str,
) -> Result<(Vec<u8>, ReplaceReport), AppError> {
    if old.trim().is_empty() {
        return Err(AppError::BadRequest("old text must not be empty".into()));
    }
    let mut doc = doc::load(bytes)?;
    let all: Vec<usize> = (0..doc.get_pages().len()).collect();
    let mut report = ReplaceReport::default();
    for &p in pages.unwrap_or(&all) {
        let page_id = doc::page_id(&doc, p)?;
        let (n, s) = replace_on_page(&mut doc, page_id, old, new, Target::All)?;
        report.replaced += n;
        report.streams_touched += s;
        report.pages_touched += (n > 0) as usize;
    }
    if report.replaced == 0 {
        return Ok((bytes.to_vec(), report));
    }
    Ok((doc::save(&mut doc)?, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_multiply_translates() {
        let m = mul([1.0, 0.0, 0.0, 1.0, 10.0, 20.0], [2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
        assert_eq!(m, [2.0, 0.0, 0.0, 2.0, 20.0, 40.0]);
    }
}
