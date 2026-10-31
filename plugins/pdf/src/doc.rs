//! Low-level document plumbing on top of lopdf: loading, saving, the page
//! tree, inherited page attributes, page resources, and appending content.
//!
//! Every engine module (creation, editing, annotations, page operations)
//! goes through here, so one PDF model backs the whole plugin.

use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};

use shiny_plugin_sdk::errors::AppError;

/// Hard ceiling on page count — guards against pathological inputs.
pub const MAX_PAGES: usize = 2000;

pub fn err(e: impl std::fmt::Display) -> AppError {
    AppError::BadRequest(format!("PDF error: {e}"))
}

/// Parse a PDF. Files encrypted with an empty user password (the common
/// "no copying" restriction) are opened transparently; a real password is an
/// explicit error rather than garbage.
pub fn load(bytes: &[u8]) -> Result<Document, AppError> {
    let mut doc = Document::load_mem(bytes).map_err(|e| {
        AppError::BadRequest(format!("This file could not be read as a PDF ({e})"))
    })?;
    if doc.is_encrypted() && doc.decrypt("").is_err() {
        return Err(AppError::BadRequest(
            "This PDF is password-protected; remove the password and import it again".into(),
        ));
    }
    Ok(doc)
}

/// Serialize, compressing streams that are not compressed yet. lopdf's own
/// `compress()` uses the slowest deflate level, which made edits on large
/// vector pages take seconds; the default level is ~5x faster for a few
/// percent of size.
pub fn save(doc: &mut Document) -> Result<Vec<u8>, AppError> {
    use std::io::Write;
    for obj in doc.objects.values_mut() {
        let Object::Stream(s) = obj else { continue };
        if s.dict.has(b"Filter") || !s.allows_compression || s.content.len() < 64 {
            continue;
        }
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        if enc.write_all(&s.content).is_err() {
            continue;
        }
        if let Ok(z) = enc.finish() {
            if z.len() + 19 < s.content.len() {
                s.dict.set("Filter", "FlateDecode");
                s.set_content(z);
            }
        }
    }
    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(err)?;
    Ok(out)
}

pub fn num(o: &Object) -> Option<f32> {
    o.as_float().ok().or_else(|| o.as_i64().ok().map(|v| v as f32))
}

pub fn deref<'a>(doc: &'a Document, o: &'a Object) -> &'a Object {
    match o {
        Object::Reference(id) => doc.get_object(*id).unwrap_or(o),
        _ => o,
    }
}

/* ── Pages ─────────────────────────────────────────────────────── */

/// Page object ids in document order.
pub fn page_ids(doc: &Document) -> Vec<ObjectId> {
    doc.get_pages().into_values().collect()
}

pub fn page_id(doc: &Document, page: usize) -> Result<ObjectId, AppError> {
    // Pages are 0-based everywhere in the plugin; lopdf's map is 1-based.
    doc.get_pages()
        .get(&((page + 1) as u32))
        .copied()
        .ok_or_else(|| AppError::NotFound(format!("page {} out of range", page + 1)))
}

/// An attribute a page may inherit from its ancestors (`Resources`,
/// `MediaBox`, `CropBox`, `Rotate`).
pub fn inherited(doc: &Document, page_id: ObjectId, key: &[u8]) -> Option<Object> {
    let mut id = page_id;
    for _ in 0..64 {
        let d = doc.get_dictionary(id).ok()?;
        if let Ok(v) = d.get(key) {
            return Some(v.clone());
        }
        id = d.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

fn rect_of(doc: &Document, o: &Object) -> Option<[f32; 4]> {
    let a = deref(doc, o).as_array().ok()?;
    if a.len() != 4 {
        return None;
    }
    let v: Vec<f32> = a.iter().filter_map(|x| num(deref(doc, x))).collect();
    (v.len() == 4).then(|| [v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])])
}

/// `[x0, y0, x1, y1]` of the page's MediaBox (A4 if missing).
pub fn media_box(doc: &Document, page_id: ObjectId) -> [f32; 4] {
    inherited(doc, page_id, b"MediaBox")
        .and_then(|o| rect_of(doc, &o))
        .unwrap_or([0.0, 0.0, 595.0, 842.0])
}

pub fn rotation(doc: &Document, page_id: ObjectId) -> i64 {
    inherited(doc, page_id, b"Rotate")
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(0)
        .rem_euclid(360)
}

/// Copy inherited attributes onto the page itself, so it keeps them when the
/// page tree is rebuilt or the page moves to another document.
pub fn materialize(doc: &mut Document, page_id: ObjectId) -> Result<(), AppError> {
    let mut found = Vec::new();
    for key in [b"Resources".as_slice(), b"MediaBox", b"CropBox", b"Rotate"] {
        if let Some(v) = inherited(doc, page_id, key) {
            found.push((key.to_vec(), v));
        }
    }
    let d = doc.get_dictionary_mut(page_id).map_err(err)?;
    for (k, v) in found {
        if !d.has(&k) {
            d.set(k, v);
        }
    }
    if !d.has(b"MediaBox") {
        d.set("MediaBox", vec![0.into(), 0.into(), 595.into(), 842.into()]);
    }
    Ok(())
}

/// Deep-enough copy of a page: a new page dictionary with its own content
/// streams (so editing one copy never edits the other); resources are shared.
pub fn clone_page(doc: &mut Document, page_id: ObjectId) -> Result<ObjectId, AppError> {
    materialize(doc, page_id)?;
    let mut d = doc.get_dictionary(page_id).map_err(err)?.clone();
    let mut contents = Vec::new();
    for cid in doc.get_page_contents(page_id) {
        if let Ok(Object::Stream(s)) = doc.get_object(cid) {
            let copy = s.clone();
            contents.push(Object::Reference(doc.add_object(copy)));
        }
    }
    d.set("Contents", Object::Array(contents));
    // Annotations belong to one page each (their /P points back at it).
    d.remove(b"Annots");
    Ok(doc.add_object(d))
}

/// Make the document's pages exactly `order` (ids may repeat — repeats are
/// cloned). The page tree is rebuilt flat and orphaned objects are dropped.
pub fn set_page_order(doc: &mut Document, order: &[ObjectId]) -> Result<(), AppError> {
    if order.is_empty() {
        return Err(AppError::BadRequest("a PDF needs at least one page".into()));
    }
    if order.len() > MAX_PAGES {
        return Err(AppError::BadRequest(format!("too many pages (max {MAX_PAGES})")));
    }
    let root = doc
        .catalog()
        .map_err(err)?
        .get(b"Pages")
        .and_then(Object::as_reference)
        .map_err(err)?;
    for id in order {
        materialize(doc, *id)?;
    }
    let mut seen = std::collections::HashSet::new();
    let mut kids = Vec::with_capacity(order.len());
    for &id in order {
        let id = if seen.insert(id) { id } else { clone_page(doc, id)? };
        if let Ok(d) = doc.get_dictionary_mut(id) {
            d.set("Parent", Object::Reference(root));
        }
        kids.push(Object::Reference(id));
    }
    let count = kids.len() as i64;
    doc.objects.insert(
        root,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }),
    );
    doc.prune_objects();
    Ok(())
}

/// A fresh blank page (not yet in the tree).
pub fn new_blank_page(doc: &mut Document, mb: [f32; 4]) -> ObjectId {
    let content = doc.add_object(Stream::new(dictionary! {}, Vec::new()));
    doc.add_object(dictionary! {
        "Type" => "Page",
        "MediaBox" => vec![mb[0].into(), mb[1].into(), mb[2].into(), mb[3].into()],
        "Resources" => dictionary! {},
        "Contents" => content,
    })
}

/* ── Resources & appended content ──────────────────────────────── */

/// The page's resources merged into one inline dictionary (inherited and
/// referenced entries resolved), suitable for reading or for replacing.
pub fn resources(doc: &Document, page_id: ObjectId) -> Dictionary {
    let mut out = Dictionary::new();
    if let Some(r) = inherited(doc, page_id, b"Resources") {
        if let Ok(d) = deref(doc, &r).as_dict() {
            out = d.clone();
        }
    }
    out
}

/// A sub-dictionary of resources (`Font`, `ExtGState`…), resolved and owned.
pub fn sub_dict(doc: &Document, res: &Dictionary, key: &[u8]) -> Dictionary {
    res.get(key)
        .ok()
        .map(|o| deref(doc, o))
        .and_then(|o| o.as_dict().ok())
        .cloned()
        .unwrap_or_default()
}

/// What appended content needs from the page's resources.
#[derive(Default)]
pub struct Needs {
    /// Base-14 fonts by name; they are registered as `/ShF_<name>`.
    pub fonts: Vec<&'static str>,
    /// Graphics states: (name, fill alpha, multiply blend).
    pub gstates: Vec<(String, f32, bool)>,
}

/// Resource name used for a Base-14 font in appended content.
pub fn font_res(name: &str) -> Vec<u8> {
    format!("ShF_{}", name.replace('-', "_")).into_bytes()
}

/// Append a self-contained content stream (`ops`) to a page.
///
/// The page's existing content is wrapped in `q … Q` once, so a page that
/// leaves its graphics state transformed cannot skew what is added; repeated
/// appends reuse that wrapper instead of nesting it.
pub fn append_content(
    doc: &mut Document,
    page_id: ObjectId,
    ops: Vec<Operation>,
    needs: &Needs,
) -> Result<(), AppError> {
    let mut res = resources(doc, page_id);
    if !needs.fonts.is_empty() {
        let mut fonts = sub_dict(doc, &res, b"Font");
        for &f in &needs.fonts {
            let key = font_res(f);
            if !fonts.has(&key) {
                let id = doc.add_object(dictionary! {
                    "Type" => "Font", "Subtype" => "Type1",
                    "BaseFont" => Object::Name(f.as_bytes().to_vec()),
                    "Encoding" => "WinAnsiEncoding",
                });
                fonts.set(key, Object::Reference(id));
            }
        }
        res.set("Font", Object::Dictionary(fonts));
    }
    if !needs.gstates.is_empty() {
        let mut gs = sub_dict(doc, &res, b"ExtGState");
        for (name, alpha, multiply) in &needs.gstates {
            let mut d = dictionary! { "Type" => "ExtGState", "ca" => *alpha, "CA" => *alpha };
            if *multiply {
                d.set("BM", Object::Name(b"Multiply".to_vec()));
            }
            let id = doc.add_object(d);
            gs.set(name.as_bytes().to_vec(), Object::Reference(id));
        }
        res.set("ExtGState", Object::Dictionary(gs));
    }
    doc.get_dictionary_mut(page_id).map_err(err)?.set("Resources", Object::Dictionary(res));

    // Wrap existing content once.
    let ids = doc.get_page_contents(page_id);
    let wrapped = ids
        .first()
        .and_then(|id| doc.get_object(*id).ok())
        .and_then(|o| o.as_stream().ok())
        .map(|s| s.dict.has(b"ShinyWrap"))
        .unwrap_or(false);
    let mut arr: Vec<Object> = ids.iter().map(|id| Object::Reference(*id)).collect();
    if !wrapped && !ids.is_empty() {
        let pre = doc.add_object(Stream::new(dictionary! { "ShinyWrap" => true }, b"q\n".to_vec()));
        let post = doc.add_object(Stream::new(dictionary! {}, b"\nQ\n".to_vec()));
        arr.insert(0, Object::Reference(pre));
        arr.push(Object::Reference(post));
    }
    let mut all = vec![Operation::new("q", vec![])];
    all.extend(ops);
    all.push(Operation::new("Q", vec![]));
    let bytes = Content { operations: all }.encode().map_err(err)?;
    let id = doc.add_object(Stream::new(dictionary! {}, bytes));
    arr.push(Object::Reference(id));
    doc.get_dictionary_mut(page_id).map_err(err)?.set("Contents", Object::Array(arr));
    Ok(())
}

/// Add an annotation dictionary to a page's `/Annots`.
pub fn add_annotation(doc: &mut Document, page_id: ObjectId, mut annot: Dictionary) -> Result<(), AppError> {
    annot.set("P", Object::Reference(page_id));
    let id = doc.add_object(annot);
    let existing = doc
        .get_dictionary(page_id)
        .map_err(err)?
        .get(b"Annots")
        .ok()
        .cloned();
    let mut arr = match existing {
        Some(Object::Array(a)) => a,
        Some(Object::Reference(r)) => doc.get_object(r).ok().and_then(|o| o.as_array().ok()).cloned().unwrap_or_default(),
        _ => Vec::new(),
    };
    arr.push(Object::Reference(id));
    doc.get_dictionary_mut(page_id).map_err(err)?.set("Annots", Object::Array(arr));
    Ok(())
}

/* ── Operator helpers ──────────────────────────────────────────── */

pub fn op(name: &str, args: Vec<Object>) -> Operation {
    Operation::new(name, args)
}

pub fn reals(v: &[f32]) -> Vec<Object> {
    v.iter().map(|x| Object::Real(*x)).collect()
}

/// `BT /font size Tf a b c d x y Tm (text) Tj ET` with WinAnsi bytes.
pub fn text_ops(font: &str, size: f32, matrix: [f32; 6], text: &str, rgb: [f32; 3]) -> Vec<Operation> {
    vec![
        op("BT", vec![]),
        op("rg", reals(&rgb)),
        op("Tf", vec![Object::Name(font_res(font)), Object::Real(size)]),
        op("Tm", reals(&matrix)),
        op("Tj", vec![Object::string_literal(crate::fonts::encode_winansi(text))]),
        op("ET", vec![]),
    ]
}
