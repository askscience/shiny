//! Minimal OpenDocument Spreadsheet (ODS) codec — cell map ↔ `.ods` bytes.
//!
//! ODS (`.ods`) is the ISO-standard open spreadsheet format used by
//! LibreOffice / OpenOffice / Excel. A `.ods` is a ZIP containing a
//! `mimetype` entry (first, uncompressed), `META-INF/manifest.xml`, and
//! `content.xml` with `<office:spreadsheet>` tables.
//!
//! This codec is deliberately small but valid, and fully self-contained —
//! no external office suite is involved. It writes one sheet per file (the
//! calc plugin's model) with string/number/formula cells:
//!
//! - strings  → `office:value-type="string"` + `<text:p>`
//! - numbers  → `office:value-type="float" office:value="…"`
//! - formulas → `table:formula="of:=…"` (OpenFormula). Cached value is 0;
//!   readers recalculate on load. Ranges are written in OpenFormula bracket
//!   syntax (`[.B1:.B3]`) and translated back on import.
//!
//! Both the core binary (REST import/export for the calc plugin) and plugin
//! code link this module — no runtime state crosses the dlopen boundary
//! because everything is plain bytes.

use std::collections::BTreeMap;
use std::io::{Cursor, Write};

use crate::errors::AppError;

const MIMETYPE: &str = "application/vnd.oasis.opendocument.spreadsheet";

const NS_TABLE: &str = "urn:oasis:names:tc:opendocument:xmlns:table:1.0";
const NS_TEXT: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
const NS_OFFICE: &str = "urn:oasis:names:tc:opendocument:xmlns:office:1.0";

const CONTENT_HEADER: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content
  xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  office:version="1.2">
"#;

const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2">
 <manifest:file-entry manifest:full-path="/" manifest:version="1.2" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/>
 <manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>
</manifest:manifest>
"#;

// ─────────────────────────────────────────────────────────────
// cells → .ods
// ─────────────────────────────────────────────────────────────

/// A cell's computed value, for a reader that does not recalculate on load
/// (and for the numbers a person typed with `,`, `%` or a currency sign).
#[derive(Debug, Clone, PartialEq)]
pub enum OdsCached {
    Number(f64),
    Text(String),
    Bool(bool),
}

/// Build a valid `.ods` file from a cell map ("A1" -> "value").
pub fn cells_to_ods(cells: &BTreeMap<String, String>) -> Result<Vec<u8>, AppError> {
    cells_to_ods_with_results(cells, &BTreeMap::new())
}

/// Like `cells_to_ods`, with each cell's computed value: a formula carries
/// its result instead of 0, and a typed number (`2,50`, `10%`, `€ 5`) is
/// written as a number with its text kept.
pub fn cells_to_ods_with_results(
    cells: &BTreeMap<String, String>,
    results: &BTreeMap<String, OdsCached>,
) -> Result<Vec<u8>, AppError> {
    let content = content_xml(cells, results);

    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));

    // `mimetype` MUST be the first entry and MUST be stored uncompressed.
    let stored = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);
    zip.start_file("mimetype", stored)
        .map_err(|e| AppError::Internal(format!("ODS write failed: {}", e)))?;
    zip.write_all(MIMETYPE.as_bytes())
        .map_err(|e| AppError::Internal(format!("ODS write failed: {}", e)))?;

    let deflated = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("META-INF/manifest.xml", deflated)
        .map_err(|e| AppError::Internal(format!("ODS write failed: {}", e)))?;
    zip.write_all(MANIFEST.as_bytes())
        .map_err(|e| AppError::Internal(format!("ODS write failed: {}", e)))?;

    zip.start_file("content.xml", deflated)
        .map_err(|e| AppError::Internal(format!("ODS write failed: {}", e)))?;
    zip.write_all(content.as_bytes())
        .map_err(|e| AppError::Internal(format!("ODS write failed: {}", e)))?;

    let inner = zip
        .finish()
        .map_err(|e| AppError::Internal(format!("ODS write failed: {}", e)))?
        .into_inner();
    Ok(inner)
}

/// Build `content.xml` with one `<table:table>` holding every non-empty cell.
fn content_xml(cells: &BTreeMap<String, String>, results: &BTreeMap<String, OdsCached>) -> String {
    // row (1-based) -> (col (0-based) -> (value, computed))
    let mut by_row: BTreeMap<u32, BTreeMap<u32, (String, Option<OdsCached>)>> = BTreeMap::new();
    for (ref_, value) in cells {
        if let Some(pos) = parse_ref(ref_) {
            by_row
                .entry(pos.0)
                .or_default()
                .insert(pos.1, (value.clone(), results.get(ref_).cloned()));
        }
    }

    let max_row = by_row.keys().last().copied().unwrap_or(0);
    let max_col = by_row
        .values()
        .flat_map(|row| row.keys().copied())
        .max()
        .unwrap_or(0);

    let mut body = String::new();
    body.push_str(&format!(
        "   <table:table table:name=\"Sheet1\">\n    <table:table-column table:number-columns-repeated=\"{}\"/>\n",
        (max_col + 1).max(1)
    ));

    if max_row == 0 {
        // Keep at least one empty row so the table is well-formed.
        body.push_str("    <table:table-row/>\n");
    } else {
        // Emit EVERY row number 1..=max_row — sparse sheets must keep their
        // empty rows, otherwise A3 collapses onto A2 on export→import.
        for row in 1..=max_row {
            let Some(cols) = by_row.get(&row) else {
                body.push_str("    <table:table-row/>\n");
                continue;
            };
            let mut row_xml = String::from("    <table:table-row>");
            for col in 0..=max_col {
                match cols.get(&col) {
                    Some((value, cached)) => row_xml.push_str(&cell_xml(value, cached.as_ref())),
                    None => row_xml.push_str("<table:table-cell/>"),
                }
            }
            row_xml.push_str("</table:table-row>\n");
            body.push_str(&row_xml);
        }
    }
    body.push_str("   </table:table>");

    format!(
        "{CONTENT_HEADER} <office:body><office:spreadsheet>{body}</office:spreadsheet></office:body></office:document-content>"
    )
}

fn cell_xml(value: &str, cached: Option<&OdsCached>) -> String {
    let v = value.trim();
    if let Some(formula) = v.strip_prefix('=') {
        // Formula cell: its computed value when known (0 otherwise); readers
        // recalculate on load anyway.
        let of = xml_escape(&to_openformula(formula));
        match cached {
            Some(OdsCached::Text(t)) => format!(
                "<table:table-cell office:value-type=\"string\" table:formula=\"{of}\"><text:p>{}</text:p></table:table-cell>",
                xml_escape(t)
            ),
            Some(OdsCached::Bool(b)) => format!(
                "<table:table-cell office:value-type=\"boolean\" office:boolean-value=\"{b}\" table:formula=\"{of}\"><text:p>{}</text:p></table:table-cell>",
                if *b { "TRUE" } else { "FALSE" }
            ),
            Some(OdsCached::Number(n)) => format!(
                "<table:table-cell office:value-type=\"float\" office:value=\"{n}\" table:formula=\"{of}\"><text:p>{n}</text:p></table:table-cell>"
            ),
            None => format!(
                "<table:table-cell office:value-type=\"float\" office:value=\"0\" table:formula=\"{of}\"><text:p>0</text:p></table:table-cell>"
            ),
        }
    } else if let (Some(OdsCached::Number(n)), None) = (cached, clean_number(v)) {
        // A number typed with decoration (`2,50`, `10%`, `€ 5`): the value is
        // the number, the text is what the person typed.
        format!(
            "<table:table-cell office:value-type=\"float\" office:value=\"{n}\"><text:p>{}</text:p></table:table-cell>",
            xml_escape(v)
        )
    } else if let Some(num) = clean_number(v) {
        // Numeric only when the canonical form round-trips exactly — values
        // like "02115" (zip codes) or "1e3" stay text to avoid data loss.
        format!(
            "<table:table-cell office:value-type=\"float\" office:value=\"{}\"><text:p>{}</text:p></table:table-cell>",
            num, num
        )
    } else {
        format!(
            "<table:table-cell office:value-type=\"string\"><text:p>{}</text:p></table:table-cell>",
            xml_escape(v)
        )
    }
}

/// Parse as f64 only when `v`'s canonical form equals the trimmed input.
fn clean_number(v: &str) -> Option<f64> {
    let n = v.parse::<f64>().ok()?;
    if n.to_string() == v {
        Some(n)
    } else {
        None
    }
}

/// Translate an app formula body ("SUM(B1:B3;2)+A1") to OpenFormula
/// ("of:=SUM([.B1:.B3];2)+[.A1]"): refs in brackets, `;` between arguments,
/// "strings" and function names (`LOG10(…)`) left alone.
fn to_openformula(body: &str) -> String {
    let chars: Vec<char> = body.chars().collect();
    let word_at = |start: usize| -> usize {
        let mut j = start;
        while j < chars.len() && (chars[j].is_alphanumeric() || matches!(chars[j], '_' | '.' | '$')) {
            j += 1;
        }
        j
    };
    let is_ref = |w: &str| {
        let w = w.replace('$', "");
        let letters = w.chars().take_while(|c| c.is_ascii_alphabetic()).count();
        let digits = &w[letters..];
        (1..=2).contains(&letters)
            && !digits.is_empty()
            && digits.len() <= 5
            && digits.chars().all(|c| c.is_ascii_digit())
            && !digits.starts_with('0')
    };
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            out.push(c);
            i += 1;
            while i < chars.len() {
                out.push(chars[i]);
                if chars[i] == '"' {
                    if chars.get(i + 1) == Some(&'"') {
                        out.push('"');
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if c.is_alphabetic() || c == '_' || c == '$' {
            let end = word_at(i);
            let word: String = chars[i..end].iter().collect();
            let next = chars[end..].iter().find(|c| !c.is_whitespace());
            if is_ref(&word) && next != Some(&'(') {
                let up = word.to_uppercase();
                if chars.get(end) == Some(&':') {
                    let end2 = word_at(end + 1);
                    let second: String = chars[end + 1..end2].iter().collect();
                    if is_ref(&second) {
                        out.push_str(&format!("[.{up}:.{}]", second.to_uppercase()));
                        i = end2;
                        continue;
                    }
                }
                out.push_str(&format!("[.{up}]"));
            } else {
                out.push_str(&word);
            }
            i = end;
            continue;
        }
        out.push(if c == ',' { ';' } else { c });
        i += 1;
    }
    format!("of:={out}")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ─────────────────────────────────────────────────────────────
// .ods → cells
// ─────────────────────────────────────────────────────────────

/// Read the cell map back out of a `.ods` file (first sheet).
pub fn ods_to_cells(ods: &[u8]) -> Result<BTreeMap<String, String>, AppError> {
    let xml = read_content_xml(ods)?;
    let doc = roxmltree::Document::parse(&xml)
        .map_err(|e| AppError::BadRequest(format!("Invalid ODS content.xml: {}", e)))?;

    let mut cells = BTreeMap::new();

    // Find the first sheet: office:spreadsheet > table:table.
    let sheet = doc
        .descendants()
        .find(|n| n.has_tag_name((NS_OFFICE, "spreadsheet")) && n.is_element())
        .and_then(|spreadsheet| {
            spreadsheet.children().find(|n| n.has_tag_name((NS_TABLE, "table")))
        });

    let Some(sheet) = sheet else {
        return Ok(cells); // empty sheet → empty map
    };

    let mut row = 1u32;
    for row_node in sheet.children() {
        if !row_node.has_tag_name((NS_TABLE, "table-row")) {
            continue;
        }
        let repeats = repeat_count(&row_node, "number-rows-repeated").max(1) as u32;

        // Collect this row's cells (col, value) once…
        let mut row_cells: Vec<(u32, String)> = Vec::new();
        let mut col = 0u32;
        for cell in row_node.children() {
            if !cell.has_tag_name((NS_TABLE, "table-cell")) {
                continue;
            }
            // Clamped to the columns that can still be kept: a hostile
            // `number-columns-repeated="4294967295"` would otherwise push
            // billions of copies of the value before the column check below.
            let cell_repeats = repeat_count(&cell, "number-columns-repeated")
                .max(1)
                .min(MAX_COLS + 2 - col);
            if let Some(value) = cell_value(&cell) {
                for i in 0..cell_repeats {
                    row_cells.push((col + i, value.clone()));
                }
            }
            col = col.saturating_add(cell_repeats);
            if col > MAX_COLS + 1 {
                break;
            }
        }

        // …then stamp it into every repeated row.
        for r in 0..repeats {
            let this_row = row.saturating_add(r);
            if this_row > MAX_ROWS {
                break;
            }
            for (c, value) in &row_cells {
                if *c <= MAX_COLS && cells.len() < MAX_CELLS {
                    cells.insert(format!("{}{}", col_name(*c), this_row), value.clone());
                }
            }
        }
        row = row.saturating_add(repeats);
        if row > MAX_ROWS + 1 {
            break;
        }
    }

    Ok(cells)
}

const MAX_ROWS: u32 = 500;
const MAX_COLS: u32 = 51; // A..AZ
const MAX_CELLS: usize = 5000;

/// 0-based column index → "A", "B", …, "Z", "AA", "AB", …
fn col_name(col: u32) -> String {
    let mut n = col + 1;
    let mut s = String::new();
    while n > 0 {
        let rem = (n - 1) % 26;
        s.insert(0, (b'A' + rem as u8) as char);
        n = (n - 1) / 26;
    }
    s
}

fn cell_value(cell: &roxmltree::Node) -> Option<String> {
    let formula = cell
        .attribute((NS_TABLE, "formula"))
        .map(|f| from_openformula(f));
    let value_type = cell.attribute((NS_OFFICE, "value-type"));

    if let Some(f) = formula {
        // Formula wins over the (cached) value — the app evaluates live.
        // `f` is already "=…"; the map value is the formula text itself.
        return Some(f);
    }

    match value_type {
        Some("float") | Some("currency") | Some("percentage") => {
            let v = cell
                .attribute((NS_OFFICE, "value"))
                .unwrap_or("")
                .trim();
            if v.is_empty() {
                None
            } else {
                Some(v.to_string())
            }
        }
        Some("boolean") => {
            let v = cell.attribute((NS_OFFICE, "boolean-value")).unwrap_or("");
            Some(v.to_string())
        }
        _ => {
            // string / default: first <text:p> text content.
            let text = cell
                .children()
                .find(|n| n.has_tag_name((NS_TEXT, "p")))
                .map(|p| node_text(&p))
                .unwrap_or_default();
            let text = text.trim();
            if text.is_empty() {
                None
            } else {
                Some(text.to_string())
            }
        }
    }
}

fn repeat_count(node: &roxmltree::Node, attr: &str) -> u32 {
    node.attribute((NS_TABLE, attr))
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(1)
}

/// All descendant text of a node (spans inside `<text:p>` are flattened).
fn node_text(node: &roxmltree::Node) -> String {
    node.descendants()
        .filter(|n| n.is_text())
        .map(|n| n.text().unwrap_or(""))
        .collect()
}

fn read_content_xml(ods: &[u8]) -> Result<String, AppError> {
    // Size-capped: a small `content.xml` entry can inflate to gigabytes.
    crate::read_zip_text_capped(ods, "content.xml", crate::MAX_ODF_CONTENT_XML)
}

// ─────────────────────────────────────────────────────────────
// cell ref helpers + OpenFormula translation
// ─────────────────────────────────────────────────────────────

/// Parse an A1-style ref ("B3") → (row, col 0-based). None when malformed.
fn parse_ref(ref_: &str) -> Option<(u32, u32)> {
    let bytes = ref_.as_bytes();
    let mut i = 0;
    let mut col: u32 = 0;
    let mut letters = 0;
    while i < bytes.len() && bytes[i].is_ascii_uppercase() {
        col = col * 26 + (bytes[i] - b'A' + 1) as u32;
        letters += 1;
        i += 1;
        if letters > 2 {
            return None;
        }
    }
    if letters == 0 || i >= bytes.len() {
        return None;
    }
    let row: u32 = ref_[i..].parse().ok()?;
    if row == 0 {
        return None;
    }
    Some((row, col - 1))
}

/// Translate an OpenFormula string ("of:=SUM([.B1:.B3];2)+[.A1]") back to the
/// app's plain form ("=SUM(B1:B3,2)+A1"); "strings" stay as they are.
fn from_openformula(src: &str) -> String {
    let body = src.strip_prefix("of:").unwrap_or(src);
    let chars: Vec<char> = body.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            out.push(c);
            i += 1;
            while i < chars.len() {
                out.push(chars[i]);
                if chars[i] == '"' {
                    if chars.get(i + 1) == Some(&'"') {
                        out.push('"');
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if c == '[' {
            if let Some(rel) = chars[i + 1..].iter().position(|ch| *ch == ']') {
                let inner: String = chars[i + 1..i + 1 + rel].iter().filter(|ch| **ch != '.').collect();
                out.push_str(&inner);
                i += rel + 2;
                continue;
            }
        }
        out.push(if c == ';' { ',' } else { c });
        i += 1;
    }
    if out.starts_with('=') {
        out
    } else {
        format!("={out}")
    }
}

// ─────────────────────────────────────────────────────────────
// tests — fully self-contained, no office suite required
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn round_trip_strings_numbers_formulas() {
        let cells = map(&[
            ("A1", "Item"),
            ("B1", "Cost"),
            ("A2", "Coffee"),
            ("B2", "4.5"),
            ("A3", "Lunch"),
            ("B3", "12"),
            ("B4", "=SUM(B1:B3)"),
            ("C1", "=A1+B1"),
        ]);
        let ods = cells_to_ods(&cells).expect("write");
        let back = ods_to_cells(&ods).expect("read");
        assert_eq!(back, cells, "round trip must preserve every cell");
    }

    #[test]
    fn round_trip_empty() {
        let ods = cells_to_ods(&BTreeMap::new()).expect("write");
        let back = ods_to_cells(&ods).expect("read");
        assert!(back.is_empty());
    }

    #[test]
    fn round_trip_keeps_sparse_row_numbers() {
        // Row 2 is empty: A3 must stay A3 after export+import (not collapse
        // onto A2).
        let cells = map(&[("A1", "first"), ("A3", "third")]);
        let ods = cells_to_ods(&cells).expect("write");
        let back = ods_to_cells(&ods).expect("read");
        assert_eq!(back, cells, "empty rows must keep cell row numbers");
    }

    #[test]
    fn zip_is_well_formed_ods() {
        let cells = map(&[("A1", "hello"), ("B2", "=SUM(A1:A2)")]);
        let ods = cells_to_ods(&cells).expect("write");

        let mut archive = zip::ZipArchive::new(Cursor::new(ods.clone())).expect("zip");
        // mimetype must be first and uncompressed
        let first = archive.by_index(0).expect("first entry");
        assert_eq!(first.name(), "mimetype");
        assert_eq!(first.compression(), zip::CompressionMethod::Stored);
        drop(first);

        let mut mt = String::new();
        archive.by_name("mimetype").unwrap().read_to_string(&mut mt).unwrap();
        assert_eq!(mt, MIMETYPE);

        let content = read_content_xml(&ods).expect("content.xml");
        assert!(content.contains("office:spreadsheet"), "has spreadsheet body");
        assert!(content.contains("table:formula=\"of:=SUM([.A1:.A2])\""), "openformula range");
        assert!(content.contains("office:value-type=\"string\""), "string cell");
    }

    #[test]
    fn computed_results_are_written() {
        let cells = map(&[("A1", "2,50"), ("B1", "=A1*2"), ("C1", "=\"a\"&\"b\""), ("D1", "plain")]);
        let mut results = BTreeMap::new();
        results.insert("A1".to_string(), OdsCached::Number(2.5));
        results.insert("B1".to_string(), OdsCached::Number(5.0));
        results.insert("C1".to_string(), OdsCached::Text("ab".into()));
        let xml = content_xml(&cells, &results);
        assert!(xml.contains("office:value=\"2.5\"><text:p>2,50</text:p>"), "{xml}");
        assert!(xml.contains("office:value=\"5\" table:formula"), "{xml}");
        assert!(xml.contains("office:value-type=\"string\" table:formula"), "{xml}");
        let back = ods_to_cells(&cells_to_ods_with_results(&cells, &results).expect("write")).expect("read");
        assert_eq!(back.get("B1").map(String::as_str), Some("=A1*2"));
        assert_eq!(back.get("D1").map(String::as_str), Some("plain"));
    }

    #[test]
    fn leading_zero_stays_text() {
        let cells = map(&[("A1", "02115"), ("B1", "4.0"), ("C1", "1e3")]);
        let ods = cells_to_ods(&cells).expect("write");
        let content = read_content_xml(&ods).expect("content.xml");
        // All three must be written as strings, not floats.
        assert!(content.contains("office:value-type=\"string\"><text:p>02115</text:p>"));
        assert!(content.contains("office:value-type=\"string\"><text:p>4.0</text:p>"));
        assert!(content.contains("office:value-type=\"string\"><text:p>1e3</text:p>"));
        let back = ods_to_cells(&ods).expect("read");
        assert_eq!(back, cells);
    }

    #[test]
    fn formula_translation() {
        assert_eq!(to_openformula("SUM(B1:B3)+A1"), "of:=SUM([.B1:.B3])+[.A1]");
        assert_eq!(to_openformula("ROUND(B5,1)&\"A1,b\""), "of:=ROUND([.B5];1)&\"A1,b\"");
        assert_eq!(to_openformula("LOG10($A$1)"), "of:=LOG10([.$A$1])");
        assert_eq!(from_openformula("of:=ROUND([.B5];1)&\"x;y\""), "=ROUND(B5,1)&\"x;y\"");
        assert_eq!(to_openformula("A1^2"), "of:=[.A1]^2");
        assert_eq!(from_openformula("of:=SUM([.B1:.B3])+A1"), "=SUM(B1:B3)+A1");
        assert_eq!(from_openformula("of:=SUM(B1:B3)"), "=SUM(B1:B3)");
        assert_eq!(from_openformula("[.A1]"), "=A1");
        assert_eq!(from_openformula("=A1+B2"), "=A1+B2"); // no of: prefix
    }

    #[test]
    fn import_reads_repeated_cells_and_sparse_rows() {
        // A LibreOffice-style row: two cells, second repeated 3×, with an
        // explicit empty padded cell — emulated, not generated by our writer.
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.2">
 <office:body><office:spreadsheet>
  <table:table table:name="Sheet1">
   <table:table-row>
    <table:table-cell office:value-type="string"><text:p>alpha</text:p></table:table-cell>
    <table:table-cell office:value-type="string" table:number-columns-repeated="2"><text:p>beta</text:p></table:table-cell>
    <table:table-cell office:value-type="float" office:value="7"/>
   </table:table-row>
   <table:table-row table:number-rows-repeated="2">
    <table:table-cell office:value-type="float" office:value="1.5"/>
   </table:table-row>
  </table:table>
 </office:spreadsheet></office:body></office:document-content>"#;

        // wrap into a real ods zip
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let stored = zip::write::FileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(MIMETYPE.as_bytes()).unwrap();
        let deflated = zip::write::FileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("content.xml", deflated).unwrap();
        zip.write_all(xml.as_bytes()).unwrap();
        let ods = zip.finish().unwrap().into_inner();

        let back = ods_to_cells(&ods).expect("read");
        assert_eq!(back.get("A1").map(String::as_str), Some("alpha"));
        assert_eq!(back.get("B1").map(String::as_str), Some("beta"));
        assert_eq!(back.get("C1").map(String::as_str), Some("beta"));
        assert_eq!(back.get("D1").map(String::as_str), Some("7"));
        assert_eq!(back.get("A2").map(String::as_str), Some("1.5"));
        assert_eq!(back.get("A3").map(String::as_str), Some("1.5"));
    }

    #[test]
    fn huge_repeat_counts_are_clamped() {
        // u32::MAX repeats on a valued cell used to push ~4 billion strings.
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.2">
 <office:body><office:spreadsheet>
  <table:table table:name="Sheet1">
   <table:table-row table:number-rows-repeated="4294967295">
    <table:table-cell office:value-type="string" table:number-columns-repeated="4294967295"><text:p>x</text:p></table:table-cell>
    <table:table-cell office:value-type="string" table:number-columns-repeated="4294967295"><text:p>y</text:p></table:table-cell>
   </table:table-row>
   <table:table-row><table:table-cell office:value-type="string"><text:p>after</text:p></table:table-cell></table:table-row>
  </table:table>
 </office:spreadsheet></office:body></office:document-content>"#;
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let stored = zip::write::FileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(MIMETYPE.as_bytes()).unwrap();
        zip.start_file("content.xml", stored).unwrap();
        zip.write_all(xml.as_bytes()).unwrap();
        let ods = zip.finish().unwrap().into_inner();

        let back = ods_to_cells(&ods).expect("read");
        assert_eq!(back.len(), MAX_CELLS);
        assert_eq!(back.get("A1").map(String::as_str), Some("x"));
        assert!(back.values().all(|v| v == "x"));
    }
}
