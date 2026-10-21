//! Document store — a document is a single picture with two **live,
//! non-destructive** states on top of its pixels (`images.bytes`):
//!
//! * `images.adjust` — the dial settings (brightness, contrast, …), rendered as
//!   an op batch over the base pixels;
//! * `images.look` — the selected look.
//!
//! The rendered picture is `look(settings(bytes))`. Both are replaceable
//! selections: dials stay where you put them and fine-tuning **replaces** the
//! value instead of stacking, and picking another look swaps it. Any
//! *destructive* edit (one-clicks, geometry, AI operations) bakes the live
//! state into `bytes` first and clears it, so real edits accumulate on exactly
//! what the user sees. `images.original` is the untouched upload (the target of
//! `reset`).

use photon_rs::PhotonImage;
use serde_json::{Map, Value as Json};

use shiny_plugin_sdk::db::{Db, Value};
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::util::{as_opt_text, as_text};

/// The dial keys, in render order. Unknown keys in a settings update are
/// ignored; the window's dial specs mirror these units exactly.
pub const DIAL_ORDER: &[&str] = &[
    "brightness", "contrast", "exposure", "vibrance", "saturation", "blur", "sharpen", "vignette",
];

/// Translate one dial value into the engine op it drives.
fn dial_op(key: &str, v: f64) -> Option<Json> {
    Some(match key {
        "brightness" => serde_json::json!({ "op": "brightness", "amount": v.clamp(-255.0, 255.0) }),
        "contrast" => serde_json::json!({ "op": "contrast", "amount": v.clamp(-255.0, 255.0) }),
        "exposure" => serde_json::json!({ "op": "exposure", "exposure": v.clamp(-3.0, 3.0) }),
        "vibrance" => serde_json::json!({ "op": "vibrance", "amount": v.clamp(-100.0, 100.0) }),
        "saturation" => serde_json::json!({ "op": "hue_saturation", "saturation": v.clamp(-100.0, 100.0) }),
        "blur" => serde_json::json!({ "op": "blur", "radius": v.clamp(0.0, 100.0) }),
        "sharpen" => serde_json::json!({ "op": "sharpen", "radius": 1, "amount": (v / 100.0).clamp(0.0, 5.0) }),
        "vignette" => serde_json::json!({ "op": "vignette", "amount": v.clamp(0.0, 100.0) }),
        _ => return None,
    })
}

/// The op batch a settings map renders to (zero/absent keys skipped).
fn settings_ops(settings: &Map<String, Json>) -> Vec<Json> {
    let mut ops = Vec::new();
    for key in DIAL_ORDER {
        let Some(v) = settings.get(*key).and_then(|v| v.as_f64()) else {
            continue;
        };
        if v.abs() < 1e-9 {
            continue;
        }
        if let Some(op) = dial_op(key, v) {
            ops.push(op);
        }
    }
    ops
}

pub struct Doc {
    pub title: String,
    pub raw: Vec<u8>,
    pub w: u32,
    pub h: u32,
    /// The untouched upload, decoded to raw RGBA.
    pub original: Vec<u8>,
    pub orig_w: u32,
    pub orig_h: u32,
    /// Selected look key, applied on top of the settings for display.
    pub look: Option<String>,
    /// Live dial settings, applied on top of `raw` for display.
    pub adjust: Map<String, Json>,
}

/// Outcome of an edit: the pixels plus the live state after the run.
pub struct EditResult {
    pub raw: Vec<u8>,
    pub w: u32,
    pub h: u32,
    pub look: Option<String>,
    /// The full effective dial settings after the run ({} = neutral).
    pub adjust: Json,
}
fn as_int(v: &Value) -> i64 {
    match v {
        Value::Int(n) => *n,
        _ => 0,
    }
}

fn as_blob(v: &Value) -> Vec<u8> {
    match v {
        Value::Blob(b) => b.clone(),
        _ => Vec::new(),
    }
}
/// Load a document with both buffers decoded to raw RGBA (legacy `png` rows
/// are decoded on read; the first edit rewrites them as `rgba`).
pub fn load(db: &Db, uid: &str, image_id: &str) -> Result<Doc, AppError> {
    let rows = db.query(
        "SELECT title, width, height, bytes, original, orig_width, orig_height, format, look, adjust \
         FROM images WHERE id = ?1 AND user_id = ?2",
        &[Value::text(image_id), Value::text(uid)],
    )?;
    let r = rows
        .first()
        .ok_or_else(|| AppError::NotFound("Image not found".into()))?;

    let title = as_text(&r[0]);
    let format = as_text(&r[7]);
    let bytes = as_blob(&r[3]);
    let (raw, w, h) = if format == "rgba" {
        (
            bytes,
            as_int(&r[1]).max(1) as u32,
            as_int(&r[2]).max(1) as u32,
        )
    } else {
        let img = crate::ops::decode(&bytes)?;
        (img.get_raw_pixels(), img.get_width(), img.get_height())
    };

    let original_raw = as_blob(&r[4]);
    let (original, orig_w, orig_h) = if original_raw.is_empty() {
        (raw.clone(), w, h)
    } else if format == "rgba" {
        let ow = as_int(&r[5]).max(1) as u32;
        let oh = as_int(&r[6]).max(1) as u32;
        (original_raw, ow, oh)
    } else {
        let img = crate::ops::decode(&original_raw)?;
        (img.get_raw_pixels(), img.get_width(), img.get_height())
    };

    let adjust = serde_json::from_str::<Map<String, Json>>(&as_text(&r[9])).unwrap_or_default();

    Ok(Doc {
        title,
        raw,
        w,
        h,
        original,
        orig_w,
        orig_h,
        look: as_opt_text(&r[8]),
        adjust,
    })
}

/// Apply a look over some pixels (used for display and baking).
fn apply_look(raw: &[u8], w: u32, h: u32, look: &str) -> Result<Vec<u8>, AppError> {
    let mut img = PhotonImage::new(raw.to_vec(), w, h);
    crate::looks::apply(&mut img, look).map_err(AppError::BadRequest)?;
    Ok(img.get_raw_pixels())
}

/// Render base pixels + settings + look (the visible picture).
fn render_with(
    d: &Doc,
    settings: &Map<String, Json>,
    look: Option<&str>,
) -> Result<Vec<u8>, AppError> {
    let ops = settings_ops(settings);
    let mut base = if ops.is_empty() {
        d.raw.clone()
    } else {
        crate::ops::apply_raw(&d.raw, d.w, d.h, &d.raw, d.w, d.h, &ops)?.0
    };
    if let Some(key) = look {
        base = apply_look(&base, d.w, d.h, key)?;
    }
    Ok(base)
}

/// The picture as rendered: base pixels + settings + the selected look.
pub fn visible(d: &Doc) -> Result<Vec<u8>, AppError> {
    render_with(d, &d.adjust, d.look.as_deref())
}

/// Base pixels with the live settings applied, **no look** — the source the
/// look-preview strip renders each look over.
pub fn settings_base(d: &Doc) -> Result<Vec<u8>, AppError> {
    render_with(d, &d.adjust, None)
}

/// Write edited pixels back (raw RGBA), bumping the timestamp.
pub fn save(
    db: &Db,
    uid: &str,
    image_id: &str,
    raw: &[u8],
    w: u32,
    h: u32,
) -> Result<(), AppError> {
    crate::ops::checked_layer_len(w, h)?;
    let changed = db.execute(
        "UPDATE images SET bytes = ?1, width = ?2, height = ?3, format = 'rgba', \
         updated_at = datetime('now') WHERE id = ?4 AND user_id = ?5",
        &[
            Value::blob(raw.to_vec()),
            Value::Int(w as i64),
            Value::Int(h as i64),
            Value::text(image_id),
            Value::text(uid),
        ],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound("Image not found".into()));
    }
    Ok(())
}

fn set_look(db: &Db, uid: &str, image_id: &str, look: Option<&str>) -> Result<(), AppError> {
    let changed = db.execute(
        "UPDATE images SET look = ?1, updated_at = datetime('now') \
         WHERE id = ?2 AND user_id = ?3",
        &[
            match look {
                Some(k) => Value::text(k),
                None => Value::Null,
            },
            Value::text(image_id),
            Value::text(uid),
        ],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound("Image not found".into()));
    }
    Ok(())
}

fn set_adjust(
    db: &Db,
    uid: &str,
    image_id: &str,
    settings: &Map<String, Json>,
) -> Result<(), AppError> {
    let text = if settings.is_empty() {
        None
    } else {
        Some(serde_json::to_string(settings).unwrap_or_else(|_| "{}".into()))
    };
    let changed = db.execute(
        "UPDATE images SET adjust = ?1, updated_at = datetime('now') \
         WHERE id = ?2 AND user_id = ?3",
        &[
            match text {
                Some(t) => Value::text(t),
                None => Value::Null,
            },
            Value::text(image_id),
            Value::text(uid),
        ],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound("Image not found".into()));
    }
    Ok(())
}

/// Merge a partial dial update into the live settings and render. Zero values
/// clear their key, so the stored object stays minimal.
pub fn apply_settings(
    db: &Db,
    uid: &str,
    image_id: &str,
    update: &Map<String, Json>,
    commit: bool,
) -> Result<EditResult, AppError> {
    let d = load(db, uid, image_id)?;
    let res = apply_settings_pixels(d, update)?;
    if commit {
        commit_settings(db, uid, image_id, &res.adjust)?;
    }
    Ok(res)
}

/// The pure render half of a dial update. The route moves the loaded document
/// into `spawn_blocking`, so the image plugin's serial worker stays responsive
/// while the full-resolution pass runs. No database access happens here.
pub fn apply_settings_pixels(
    d: Doc,
    update: &Map<String, Json>,
) -> Result<EditResult, AppError> {
    let mut settings = d.adjust.clone();
    for key in DIAL_ORDER {
        if let Some(v) = update.get(*key) {
            let n = v.as_f64().unwrap_or(0.0);
            if n.abs() < 1e-9 {
                settings.remove(*key);
            } else {
                settings.insert((*key).to_string(), serde_json::json!(n));
            }
        }
    }
    let raw = render_with(&d, &settings, d.look.as_deref())?;
    Ok(EditResult {
        raw,
        w: d.w,
        h: d.h,
        look: d.look,
        adjust: Json::Object(settings),
    })
}

/// Persist the effective dial settings returned by [`apply_settings_pixels`].
pub fn commit_settings(
    db: &Db,
    uid: &str,
    image_id: &str,
    adjust: &Json,
) -> Result<(), AppError> {
    let map = adjust.as_object().cloned().unwrap_or_default();
    set_adjust(db, uid, image_id, &map)
}

/// Apply operations to a document. A lone `look` op changes the selected look
/// (replaceable; no `name` clears it). Everything else is a **destructive**
/// edit: the live settings + look are baked into the base first, the state is
/// cleared, and the operations run over the result.
pub fn apply_operations(
    db: &Db,
    uid: &str,
    image_id: &str,
    operations: &[Json],
    commit: bool,
) -> Result<EditResult, AppError> {
    let d = load(db, uid, image_id)?;

    let pure_look =
        operations.len() == 1 && operations[0].get("op").and_then(|v| v.as_str()) == Some("look");
    if pure_look {
        let name = operations[0]
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if let Some(n) = name {
            // Validate before doing any work, so a typo fails cleanly.
            if crate::looks::find(n).is_none() {
                let keys: Vec<&str> = crate::looks::LOOKS.iter().map(|l| l.key).collect();
                return Err(AppError::BadRequest(format!(
                    "unknown look \"{n}\" — choose one of: {}",
                    keys.join(", ")
                )));
            }
        }
        let raw = match name {
            Some(n) => render_with(&d, &d.adjust, Some(n))?,
            None => render_with(&d, &d.adjust, None)?,
        };
        if commit {
            set_look(db, uid, image_id, name)?;
        }
        return Ok(EditResult {
            raw,
            w: d.w,
            h: d.h,
            look: name.map(str::to_string),
            adjust: Json::Object(d.adjust),
        });
    }

    // Bake the live state (settings + look) into the base so destructive edits
    // accumulate on exactly what the user sees.
    let baked = !d.adjust.is_empty() || d.look.is_some();
    let base = if baked {
        render_with(&d, &d.adjust, d.look.as_deref())?
    } else {
        d.raw.clone()
    };
    let (raw, w, h) = crate::ops::apply_raw(
        &base,
        d.w,
        d.h,
        &d.original,
        d.orig_w,
        d.orig_h,
        operations,
    )?;
    let has_reset = operations
        .iter()
        .any(|o| o.get("op").and_then(|v| v.as_str()) == Some("reset"));
    let clear_state = baked || has_reset;
    if commit {
        save(db, uid, image_id, &raw, w, h)?;
        if clear_state {
            set_look(db, uid, image_id, None)?;
            set_adjust(db, uid, image_id, &Map::new())?;
        }
    }
    Ok(EditResult {
        raw,
        w,
        h,
        look: if clear_state { None } else { d.look },
        adjust: if clear_state {
            serde_json::json!({})
        } else {
            Json::Object(d.adjust)
        },
    })
}
