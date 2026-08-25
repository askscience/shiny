//! Minimal OpenDocument Text (ODT) codec — HTML ↔ ODT.
//!
//! ODT (`.odt`) is the ISO-standard open-source document format used by
//! LibreOffice / OpenOffice / Google Docs. A `.odt` is a ZIP containing a
//! `mimetype` entry (first, uncompressed), `META-INF/manifest.xml`, and
//! `content.xml` with the document body.
//!
//! The codec here is deliberately small but valid: automatic character
//! styles for bold/italic/underline (T1–T7), paragraphs (`text:p`),
//! headings (`text:h`, outline level), bullet/numbered lists, and links.
//! LibreOffice opens files produced by this codec directly.
//!
//! Both the core binary (REST routes for the word plugin) and plugin code
//! (AI document tools) link this module — no runtime state crosses the
//! dlopen boundary because everything is plain bytes.

use std::io::{Cursor, Read, Write};

use crate::errors::AppError;

const MIMETYPE: &str = "application/vnd.oasis.opendocument.text";

/// Namespace URIs used for attribute lookups (roxmltree requires the
/// `(uri, local)` tuple form for namespaced attributes).
const NS_TEXT: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
const NS_XLINK: &str = "http://www.w3.org/1999/xlink";

const CONTENT_HEADER: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content
  xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
  xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"
  xmlns:xlink="http://www.w3.org/1999/xlink"
  office:version="1.2">
"#;

const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2">
 <manifest:file-entry manifest:full-path="/" manifest:version="1.2" manifest:media-type="application/vnd.oasis.opendocument.text"/>
 <manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>
</manifest:manifest>
"#;

/// Build a valid `.odt` file from editor HTML.
pub fn html_to_odt(title: &str, html: &str) -> Result<Vec<u8>, AppError> {
    let body = convert_blocks(html);
    let content = format!(
        "{CONTENT_HEADER} {automatic_styles} <office:body><office:text>{body}</office:text></office:body></office:document-content>",
        automatic_styles = automatic_styles(),
    );

    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));

    // `mimetype` MUST be the first entry and MUST be stored uncompressed.
    let stored = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);
    zip.start_file("mimetype", stored)
        .map_err(|e| AppError::Internal(format!("ODT write failed: {}", e)))?;
    zip.write_all(MIMETYPE.as_bytes())
        .map_err(|e| AppError::Internal(format!("ODT write failed: {}", e)))?;

    let deflated = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("META-INF/manifest.xml", deflated)
        .map_err(|e| AppError::Internal(format!("ODT write failed: {}", e)))?;
    zip.write_all(MANIFEST.as_bytes())
        .map_err(|e| AppError::Internal(format!("ODT write failed: {}", e)))?;

    zip.start_file("content.xml", deflated)
        .map_err(|e| AppError::Internal(format!("ODT write failed: {}", e)))?;
    zip.write_all(content.as_bytes())
        .map_err(|e| AppError::Internal(format!("ODT write failed: {}", e)))?;

    let inner = zip
        .finish()
        .map_err(|e| AppError::Internal(format!("ODT write failed: {}", e)))?
        .into_inner();
    let _ = title; // title lives in the DB row, not the file
    Ok(inner)
}

/// Read editor HTML back out of a `.odt` file.
pub fn odt_to_html(odt: &[u8]) -> Result<String, AppError> {
    let xml = read_content_xml(odt)?;
    let doc = roxmltree::Document::parse(&xml)
        .map_err(|e| AppError::Internal(format!("Invalid ODT content.xml: {}", e)))?;

    let mut out = String::new();
    let mut first = true;
    for node in doc.descendants() {
        if node.has_tag_name("text") {
            for child in node.children() {
                if child.is_element() {
                    if !first {
                        out.push('\n');
                    }
                    first = false;
                    render_block(&child, &mut out);
                }
            }
            break;
        }
    }
    Ok(out)
}

/// Plain text version of a `.odt` (used by the AI `doc_read` tool).
pub fn odt_to_plain_text(odt: &[u8]) -> Result<String, AppError> {
    let html = odt_to_html(odt)?;
    Ok(strip_html(&html))
}

// ─────────────────────────────────────────────────────────────
// HTML → ODT body (small recursive-descent converter)
// ─────────────────────────────────────────────────────────────

struct P {
    c: Vec<char>,
    i: usize,
}

fn is_block_tag(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "blockquote"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "pre"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "ul"
            | "ol"
            | "li"
            | "table"
            | "tr"
            | "td"
            | "th"
            | "dl"
            | "dt"
            | "dd"
    )
}

fn style_name(mask: u8) -> &'static str {
    match mask {
        1 => "T1", // bold
        2 => "T2", // italic
        3 => "T4", // bold italic
        4 => "T3", // underline
        5 => "T5", // bold underline
        6 => "T6", // italic underline
        7 => "T7", // bold italic underline
        _ => "T1",
    }
}

fn automatic_styles() -> &'static str {
    r#"<office:automatic-styles>
<style:style style:name="T1" style:family="text"><style:text-properties fo:font-weight="bold"/></style:style>
<style:style style:name="T2" style:family="text"><style:text-properties fo:font-style="italic"/></style:style>
<style:style style:name="T3" style:family="text"><style:text-properties text:underline-style="solid" text:underline-width="auto" text:underline-color="font-color"/></style:style>
<style:style style:name="T4" style:family="text"><style:text-properties fo:font-weight="bold" fo:font-style="italic"/></style:style>
<style:style style:name="T5" style:family="text"><style:text-properties fo:font-weight="bold" text:underline-style="solid" text:underline-width="auto" text:underline-color="font-color"/></style:style>
<style:style style:name="T6" style:family="text"><style:text-properties fo:font-style="italic" text:underline-style="solid" text:underline-width="auto" text:underline-color="font-color"/></style:style>
<style:style style:name="T7" style:family="text"><style:text-properties fo:font-weight="bold" fo:font-style="italic" text:underline-style="solid" text:underline-width="auto" text:underline-color="font-color"/></style:style>
<style:style style:name="WWNumber1" style:family="list">
<style:list-level-properties text:list-level-position-and-space-mode="label-alignment"/>
<style:list-level-style-number text:level="1" style:num-format="1"/>
<style:list-level-style-number text:level="2" style:num-format="1"/>
<style:list-level-style-number text:level="3" style:num-format="1"/>
</style:style>
</office:automatic-styles>"#
}

fn convert_blocks(html: &str) -> String {
    let mut p = P { c: html.chars().collect(), i: 0 };
    let mut out = String::new();
    p.parse_blocks(&mut out, None);
    escape_and_collapse(&out)
}

impl P {
    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }

    /// Inspect the tag at the cursor without consuming. Returns
    /// (closing, name, raw-inner).
    fn peek_tag(&self) -> Option<(bool, String, String)> {
        if self.peek() != Some('<') {
            return None;
        }
        let mut j = self.i + 1;
        while j < self.c.len() && self.c[j] != '>' {
            j += 1;
        }
        if j >= self.c.len() {
            return None;
        }
        let raw: String = self.c[self.i + 1..j].iter().collect();
        let raw = raw.trim();
        if raw.is_empty() || raw.starts_with("!--") {
            return Some((false, String::new(), raw.to_string()));
        }
        let closing = raw.starts_with('/');
        let body = raw.trim_start_matches('/');
        let name_end = body
            .find(|ch: char| ch.is_whitespace() || ch == '/')
            .unwrap_or(body.len());
        Some((closing, body[..name_end].to_lowercase(), body.to_string()))
    }

    fn consume_tag(&mut self) {
        let mut j = self.i + 1;
        while j < self.c.len() && self.c[j] != '>' {
            j += 1;
        }
        self.i = (j + 1).min(self.c.len());
    }

    /// Raw text of the tag at the cursor (including brackets), then consume.
    fn raw_tag(&mut self) -> String {
        let mut j = self.i + 1;
        while j < self.c.len() && self.c[j] != '>' {
            j += 1;
        }
        let raw: String = self.c[self.i..(j + 1).min(self.c.len())].iter().collect();
        self.i = (j + 1).min(self.c.len());
        raw
    }

    /// Collect the inner content of a (possibly nested) element up to its
    /// matching closing tag, which is consumed.
    fn collect_until_closing(&mut self, name: &str) -> String {
        let mut depth = 1usize;
        let mut out = String::new();
        loop {
            if self.i >= self.c.len() {
                break;
            }
            if self.peek() == Some('<') {
                if let Some((closing, tname, _)) = self.peek_tag() {
                    if tname == name {
                        if closing {
                            depth -= 1;
                            self.consume_tag();
                            if depth == 0 {
                                break;
                            }
                            continue;
                        }
                        depth += 1;
                        self.consume_tag();
                        continue;
                    }
                    if tname.is_empty() {
                        // comment or malformed — skip it wholesale
                        self.consume_tag();
                        continue;
                    }
                }
            }
            out.push(self.c[self.i]);
            self.i += 1;
        }
        out
    }

    fn parse_blocks(&mut self, out: &mut String, stop: Option<&str>) {
        let mut para = String::new();
        loop {
            if self.i >= self.c.len() {
                break;
            }
            if self.peek() == Some('<') {
                let Some((closing, name, _)) = self.peek_tag() else {
                    para.push(self.c[self.i]);
                    self.i += 1;
                    continue;
                };
                if closing {
                    if stop == Some(name.as_str()) {
                        self.consume_tag();
                        break;
                    }
                    self.consume_tag();
                    continue;
                }
                if name.is_empty() {
                    self.consume_tag();
                    continue;
                }
                if is_block_tag(&name) {
                    if !para.trim().is_empty() {
                        out.push_str("<text:p>");
                        self.inline_into(&para, out);
                        out.push_str("</text:p>");
                        para.clear();
                    }
                    self.consume_tag();
                    match name.as_str() {
                        "ul" | "ol" => self.list_into(&name, out),
                        "li" => {
                            let inner = self.collect_until_closing("li");
                            out.push_str("<text:p>");
                            self.inline_into(&inner, out);
                            out.push_str("</text:p>");
                        }
                        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                            let lvl: u32 = name[1..].parse().unwrap_or(1);
                            let inner = self.collect_until_closing(&name);
                            out.push_str(&format!(
                                "<text:h text:outline-level=\"{}\">",
                                lvl.clamp(1, 6)
                            ));
                            self.inline_into(&inner, out);
                            out.push_str("</text:h>");
                        }
                        _ => {
                            // p, div, blockquote, section, pre, table cells…
                            let inner = self.collect_until_closing(&name);
                            out.push_str("<text:p>");
                            self.inline_into(&inner, out);
                            out.push_str("</text:p>");
                        }
                    }
                    continue;
                }
                // inline tag — accumulate raw for paragraph-level inline parse
                para.push_str(&self.raw_tag());
                continue;
            }
            para.push(self.c[self.i]);
            self.i += 1;
        }
        if !para.trim().is_empty() {
            out.push_str("<text:p>");
            self.inline_into(&para, out);
            out.push_str("</text:p>");
        }
    }

    fn list_into(&mut self, kind: &str, out: &mut String) {
        let numbered = kind == "ol";
        out.push_str(if numbered {
            "<text:list text:style-name=\"WWNumber1\">"
        } else {
            "<text:list>"
        });
        loop {
            if self.i >= self.c.len() {
                break;
            }
            if let Some((closing, name, _)) = self.peek_tag() {
                if closing && name == kind {
                    self.consume_tag();
                    break;
                }
                if name == "li" {
                    self.consume_tag();
                    let inner = self.collect_until_closing("li");
                    // The item content is block-ish: reuse block parsing so
                    // nested lists survive as <text:list> inside the item.
                    let mut blocks = String::new();
                    let mut sub = P { c: inner.chars().collect(), i: 0 };
                    sub.parse_blocks(&mut blocks, None);
                    let blocks = escape_and_collapse(&blocks);
                    if blocks.trim().is_empty() {
                        out.push_str("<text:list-item><text:p/></text:list-item>");
                    } else {
                        let mut item = String::new();
                        let mut found_block = false;
                        for tag in split_top_blocks(&blocks) {
                            found_block = true;
                            item.push_str("<text:list-item>");
                            item.push_str(tag);
                            item.push_str("</text:list-item>");
                        }
                        if !found_block {
                            out.push_str("<text:list-item><text:p/>");
                            out.push_str("</text:list-item>");
                        } else {
                            out.push_str(&item);
                        }
                    }
                    continue;
                }
            }
            // Anything else inside a list — consume until the next list tag.
            if self.peek() == Some('<') {
                if let Some((_closing, n, _)) = self.peek_tag() {
                    if !n.is_empty() {
                        self.consume_tag();
                        continue;
                    }
                }
            }
            self.i += 1;
        }
        out.push_str("</text:list>");
    }

    /// Parse a raw inline fragment into ODT spans/text.
    fn inline_into(&mut self, raw: &str, out: &mut String) {
        let mut sub = P { c: raw.chars().collect(), i: 0 };
        sub.parse_inline(out, None);
    }

    fn parse_inline(&mut self, out: &mut String, stop: Option<&str>) {
        loop {
            if self.i >= self.c.len() {
                break;
            }
            if self.peek() == Some('<') {
                let Some((closing, name, attrs)) = self.peek_tag() else {
                    out.push(self.c[self.i]);
                    self.i += 1;
                    continue;
                };
                if closing {
                    if stop == Some(name.as_str()) {
                        self.consume_tag();
                        return;
                    }
                    self.consume_tag();
                    continue;
                }
                if name.is_empty() {
                    self.consume_tag();
                    continue;
                }
                match name.as_str() {
                    "br" => {
                        self.consume_tag();
                        out.push_str("<text:line-break/>");
                    }
                    "b" | "strong" => {
                        self.consume_tag();
                        out.push_str("<text:span text:style-name=\"T1\">");
                        self.parse_inline(out, Some("b"));
                        out.push_str("</text:span>");
                    }
                    "i" | "em" => {
                        self.consume_tag();
                        out.push_str("<text:span text:style-name=\"T2\">");
                        self.parse_inline(out, Some("i"));
                        out.push_str("</text:span>");
                    }
                    "u" => {
                        self.consume_tag();
                        out.push_str("<text:span text:style-name=\"T3\">");
                        self.parse_inline(out, Some("u"));
                        out.push_str("</text:span>");
                    }
                    "a" => {
                        self.consume_tag();
                        let href = attr_value(&attrs, "href")
                            .map(|h| escape_attr(&h))
                            .unwrap_or_default();
                        out.push_str(&format!("<text:a xlink:href=\"{}\">", href));
                        self.parse_inline(out, Some("a"));
                        out.push_str("</text:a>");
                    }
                    "span" => {
                        self.consume_tag();
                        let mask = mask_from_style_attr(&attrs);
                        if mask > 0 {
                            out.push_str(&format!(
                                "<text:span text:style-name=\"{}\">",
                                style_name(mask)
                            ));
                            self.parse_inline(out, Some("span"));
                            out.push_str("</text:span>");
                        } else {
                            self.parse_inline(out, Some("span"));
                        }
                    }
                    "img" | "script" | "style" | "iframe" | "svg" | "video" | "audio" => {
                        // skip the whole element (self-closing or paired)
                        if attrs.trim_end().ends_with('/') {
                            self.consume_tag();
                        } else {
                            let inner = self.collect_until_closing(&name);
                            let _ = inner;
                        }
                    }
                    _ => {
                        // unknown inline: flatten its content
                        let is_self_closing = attrs.trim_end().ends_with('/');
                        self.consume_tag();
                        if !is_self_closing {
                            self.parse_inline(out, Some(&name));
                        }
                    }
                }
                continue;
            }
            out.push(self.c[self.i]);
            self.i += 1;
        }
        if let Some(stop) = stop {
            // Unclosed element — nothing left to do; caller drops it.
            let _ = stop;
        }
    }
}

fn attr_value(raw: &str, key: &str) -> Option<String> {
    let lower = raw.to_lowercase();
    let mut rest = lower.as_str();
    while let Some(pos) = rest.find(key) {
        let after = &rest[pos + key.len()..];
        let after = after.trim_start();
        if after.starts_with('=') {
            let val = after[1..].trim_start();
            let val = val.trim_start_matches('"').trim_start_matches('\'');
            let end = val.find(['"', '\'']).unwrap_or(val.len());
            return Some(val[..end].to_string());
        }
        rest = after;
    }
    None
}

fn mask_from_style_attr(raw: &str) -> u8 {
    let lower = raw.to_lowercase();
    let mut mask = 0u8;
    if lower.contains("font-weight") && (lower.contains("bold") || lower.contains("700")) {
        mask |= 1;
    }
    if lower.contains("font-style:italic") || lower.contains("font-style: oblique") {
        mask |= 2;
    }
    if lower.contains("text-decoration") && lower.contains("underline") {
        mask |= 4;
    }
    mask
}

fn split_top_blocks(xml: &str) -> Vec<&str> {
    // Splits a string of <text:p>/<text:h>/<text:list> elements on top-level
    // element boundaries.
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let bytes = xml.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let end = xml[i..].find('>').map(|d| i + d + 1).unwrap_or(xml.len());
            let tag = &xml[i..end];
            if tag.starts_with("</") {
                if depth == 1 {
                    out.push(&xml[start..end]);
                    start = end;
                }
                if depth > 0 {
                    depth -= 1;
                }
            } else if !tag.ends_with("/>") && !tag.starts_with("<!--") {
                depth += 1;
                if depth == 1 {
                    start = i;
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }
    if start < xml.len() && !xml[start..].trim().is_empty() {
        out.push(&xml[start..]);
    }
    out
}

/// Escape XML text and collapse whitespace runs in text segments, leaving
/// markup untouched.
fn escape_and_collapse(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_tag = false;
    let mut ws_run = false;
    let mut prev_was_tag = false;
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '<' {
            in_tag = true;
            prev_was_tag = true;
            out.push('<');
            i += 1;
            continue;
        }
        if in_tag {
            out.push(ch);
            if ch == '>' {
                in_tag = false;
                ws_run = false;
            }
            i += 1;
            continue;
        }
        if ch.is_whitespace() || ch == '\u{00a0}' {
            // Collapse runs of whitespace to a single space, but NEVER eat
            // the space between two words that happen to straddle a tag
            // (e.g. "bold</span> paragraph").
            if !ws_run {
                out.push(' ');
                ws_run = true;
            }
            prev_was_tag = false;
            i += 1;
            continue;
        }
        ws_run = false;
        match ch {
            '&' => {
                // Keep known entities intact.
                let rest: String = chars[i + 1..].iter().take(24).collect();
                let ent_end = rest.find(';');
                if let Some(end) = ent_end {
                    let entity: String = rest.chars().take(end + 1).collect();
                    let name: String = rest.chars().take(end).collect();
                    let looks_ok = !name.is_empty()
                        && name
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '#' || c == 'x' || c == 'X');
                    if looks_ok && (name.starts_with('#') || name.len() <= 12) {
                        out.push('&');
                        out.push_str(&entity);
                        i += 1 + end + 1;
                        continue;
                    }
                }
                out.push_str("&amp;");
            }
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
        prev_was_tag = false;
        i += 1;
    }
    out
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ─────────────────────────────────────────────────────────────
// ODT → HTML
// ─────────────────────────────────────────────────────────────

fn read_content_xml(odt: &[u8]) -> Result<String, AppError> {
    let reader = Cursor::new(odt.to_vec());
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|e| AppError::Internal(format!("Not a valid .odt file: {}", e)))?;
    let mut content = String::new();
    archive
        .by_name("content.xml")
        .map_err(|e| AppError::Internal(format!("Missing content.xml in .odt: {}", e)))?
        .read_to_string(&mut content)
        .map_err(|e| AppError::Internal(format!("Failed to read content.xml: {}", e)))?;
    Ok(content)
}

fn render_block(node: &roxmltree::Node, out: &mut String) {
    if node.has_tag_name("p") {
        out.push_str("<p>");
        render_inline(node, out);
        out.push_str("</p>");
    } else if node.has_tag_name("h") {
        let lvl = node
            .attribute((NS_TEXT, "outline-level"))
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(1)
            .clamp(1, 6);
        out.push_str(&format!("<h{}>", lvl));
        render_inline(node, out);
        out.push_str(&format!("</h{}>", lvl));
    } else if node.has_tag_name("list") {
        let ordered = node
            .attribute((NS_TEXT, "style-name"))
            .map(|s| s.to_lowercase().contains("number"))
            .unwrap_or(false);
        out.push_str(if ordered { "<ol>" } else { "<ul>" });
        for item in node.children() {
            if item.has_tag_name("list-item") {
                out.push_str("<li>");
                let mut item_html = String::new();
                for child in item.children() {
                    if child.is_element() {
                        if child.has_tag_name("p") || child.has_tag_name("h") {
                            render_inline(&child, &mut item_html);
                        } else if child.has_tag_name("list") {
                            render_block(&child, &mut item_html);
                        }
                    }
                }
                if item_html.trim().is_empty() {
                    render_inline(&item, &mut item_html);
                }
                out.push_str(&item_html);
                out.push_str("</li>");
            }
        }
        out.push_str(if ordered { "</ol>" } else { "</ul>" });
    }
}

fn render_inline(node: &roxmltree::Node, out: &mut String) {
    for child in node.children() {
        if child.is_text() {
            out.push_str(&escape_html_text(child.text().unwrap_or("")));
        } else if child.is_element() {
            if child.has_tag_name("span") {
                // roxmltree matches attributes by (namespace URI, local name).
                let mask = child
                    .attribute((NS_TEXT, "style-name"))
                    .map(style_mask)
                    .unwrap_or(0);
                let mut inner = String::new();
                render_inline(&child, &mut inner);
                wrap_style(mask, &inner, out);
            } else if child.has_tag_name("a") {
                let href = child
                    .attribute((NS_XLINK, "href"))
                    .unwrap_or("");
                out.push_str(&format!("<a href=\"{}\">", escape_html_attr(href)));
                render_inline(&child, out);
                out.push_str("</a>");
            } else if child.has_tag_name("line-break") {
                out.push_str("<br>");
            } else if child.has_tag_name("s") {
                let n = child
                    .attribute((NS_TEXT, "c"))
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(1);
                out.push_str(&" ".repeat(n.clamp(1, 12)));
            } else if child.has_tag_name("tab") {
                out.push('\t');
            } else if child.has_tag_name("p") || child.has_tag_name("h") || child.has_tag_name("list")
            {
                render_block(&child, out);
            } else {
                // note, soft-page-break, bookmark, etc. — flatten content
                render_inline(&child, out);
            }
        }
    }
}

fn style_mask(style_name: &str) -> u8 {
    match style_name {
        "T1" => 1,
        "T2" => 2,
        "T3" => 4,
        "T4" => 3,
        "T5" => 5,
        "T6" => 6,
        "T7" => 7,
        _ => 0,
    }
}

fn wrap_style(mask: u8, inner: &str, out: &mut String) {
    let mut s = inner.to_string();
    if mask & 1 != 0 {
        s = format!("<b>{}</b>", s);
    }
    if mask & 2 != 0 {
        s = format!("<i>{}</i>", s);
    }
    if mask & 4 != 0 {
        s = format!("<u>{}</u>", s);
    }
    out.push_str(&s);
}

fn escape_html_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_html_attr(s: &str) -> String {
    escape_html_text(s).replace('"', "&quot;")
}

// ─────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_keeps_formatting() {
        let html = "<h1>Hello</h1><p>This is a <b>bold</b> paragraph with <i>italics</i>.</p><ul><li>item one</li><li>item two</li></ul>";
        let odt = html_to_odt("t", html).unwrap();
        let xml = read_content_xml(&odt).unwrap();
        let body: String = xml.chars().skip(xml.find("<office:text>").unwrap()).take(500).collect();
        println!("XML BODY: {}", body);
        let back = odt_to_html(&odt).unwrap();
        println!("BACK: {}", back);
        assert!(back.contains("<h1>Hello</h1>"), "h1 lost: {back}");
        assert!(back.contains("<b>bold</b>"), "bold lost: {back}");
        assert!(back.contains("<i>italics</i>"), "italics lost: {back}");
        assert!(back.contains("bold</b> paragraph"), "space after span lost: {back}");
        assert!(back.contains("<ul><li>item one</li><li>item two</li></ul>"), "list lost: {back}");
    }
}

/// Minimal tag stripper for the plain-text reader. Keeps paragraph breaks.
fn strip_html(html: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = html.chars().collect();
    let mut i = 0;
    let mut newline_pending = false;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '<' {
            let end = html[i..].find('>').map(|d| i + d + 1).unwrap_or(html.len());
            let tag = &html[i..end];
            let lower = tag.to_lowercase();
            if lower.starts_with("</p") || lower.starts_with("</h") || lower.starts_with("</li") {
                newline_pending = true;
            } else if lower.starts_with("<li") {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str("- ");
            } else if lower.starts_with("<br") {
                newline_pending = true;
            }
            i = end;
            continue;
        }
        if newline_pending {
            newline_pending = false;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
        }
        if ch == '\u{00a0}' {
            out.push(' ');
        } else {
            out.push(ch);
        }
        i += 1;
    }
    out.lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}