//! Document creation: Markdown / HTML / plain text → a clean A4 PDF.
//!
//! Two phases, on purpose:
//!
//! 1. **Layout** turns blocks into a display list per page (text, lines,
//!    fills) using Base-14 metrics. Pages are created lazily, only when there is
//!    something to draw, so a document never ends on a blank page, and the
//!    total page count is known before anything is written (page numbers).
//! 2. **Emit** writes the display list as PDF objects (lopdf): one content
//!    stream per page, Base-14 fonts with WinAnsi encoding (nothing embedded,
//!    accents and € included), real link annotations.

use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Dictionary, Document, Object, Stream};

use crate::doc::{op, reals};
use crate::fonts;

use shiny_plugin_sdk::errors::AppError;

// A4, in points.
const PAGE_W: f32 = 595.0;
const PAGE_H: f32 = 842.0;
const MARGIN: f32 = 64.0;
const BOTTOM: f32 = 64.0;
const BODY: f32 = 10.5;
const LEAD: f32 = 1.4;

const C_HEAD: (f32, f32, f32) = (0.086, 0.196, 0.309);
const C_RULE: (f32, f32, f32) = (0.78, 0.80, 0.83);
const C_FILL: (f32, f32, f32) = (0.95, 0.955, 0.96);

/// Creation options beyond the content itself.
#[derive(Debug, Clone, Default)]
pub struct CreateOptions {
    /// Document title (PDF metadata).
    pub title: Option<String>,
    /// Draw "n / N" centred in the footer of every page.
    pub page_numbers: bool,
}

/* ── Inline runs ─────────────────────────────────────────────── */

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Style {
    bold: bool,
    italic: bool,
    code: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Run {
    text: String,
    style: Style,
    link: Option<String>,
    /// A forced line break (`<br>`, two trailing spaces, or a trailing `\`).
    brk: bool,
}

impl Run {
    fn new(text: &str, style: Style) -> Self {
        Self { text: text.to_string(), style, link: None, brk: false }
    }
    fn line_break() -> Self {
        Self { text: String::new(), style: Style::default(), link: None, brk: true }
    }
}

fn font_of(s: Style) -> &'static str {
    match (s.code, s.bold, s.italic) {
        (true, true, _) => "Courier-Bold",
        (true, false, _) => "Courier",
        (false, true, true) => "Helvetica-BoldOblique",
        (false, true, false) => "Helvetica-Bold",
        (false, false, true) => "Helvetica-Oblique",
        (false, false, false) => "Helvetica",
    }
}

/// Map characters Base-14 fonts (WinAnsi) cannot draw to close equivalents,
/// so they do not vanish from the page.
fn sanitize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '→' | '⟶' => out.push_str("->"),
            '←' => out.push_str("<-"),
            '↔' => out.push_str("<->"),
            '⇒' => out.push_str("=>"),
            '≥' => out.push_str(">="),
            '≤' => out.push_str("<="),
            '≠' => out.push_str("!="),
            '✓' | '✔' => out.push('v'),
            '✗' | '✘' => out.push('x'),
            '\u{00A0}' | '\u{2009}' | '\u{202F}' => out.push(' '),
            '\t' => out.push_str("    "),
            '\u{200B}' | '\u{FEFF}' => {}
            c => out.push(c),
        }
    }
    out
}

/// Inline Markdown → runs, scanned left to right: `**bold**`, `*italic*` /
/// `_italic_`, `` `code` ``, `[text](url)`, backslash escapes.
pub(crate) fn parse_inline(s: &str) -> Vec<Run> {
    let chars: Vec<char> = s.chars().collect();
    let mut out: Vec<Run> = Vec::new();
    let mut buf = String::new();
    let mut style = Style::default();
    let mut i = 0;

    fn flush(out: &mut Vec<Run>, buf: &mut String, style: Style) {
        if !buf.is_empty() {
            out.push(Run::new(&sanitize(buf), style));
            buf.clear();
        }
    }
    let closes = |from: usize, pat: &[char]| -> bool {
        let n = pat.len();
        (from..chars.len().saturating_sub(n - 1)).any(|k| chars[k..k + n] == *pat)
    };

    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() && "\\`*_[]()#|-+.!<>".contains(chars[i + 1]) {
            buf.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if style.code {
            if c == '`' {
                flush(&mut out, &mut buf, style);
                style.code = false;
            } else {
                buf.push(c);
            }
            i += 1;
            continue;
        }
        if c == '`' && closes(i + 1, &['`']) {
            flush(&mut out, &mut buf, style);
            style.code = true;
            i += 1;
            continue;
        }
        if c == '*' && chars.get(i + 1) == Some(&'*') {
            if style.bold || closes(i + 2, &['*', '*']) {
                flush(&mut out, &mut buf, style);
                style.bold = !style.bold;
                i += 2;
                continue;
            }
        }
        let word_edge = |k: usize| k >= chars.len() || !chars[k].is_alphanumeric();
        if c == '*' || (c == '_' && (style.italic || word_edge(i.wrapping_sub(1)) || i == 0)) {
            let opening = !style.italic && chars.get(i + 1).map_or(false, |n| !n.is_whitespace()) && closes(i + 1, &[c]);
            if style.italic || opening {
                flush(&mut out, &mut buf, style);
                style.italic = !style.italic;
                i += 1;
                continue;
            }
        }
        if c == '[' {
            // [text](url)
            if let Some(close) = (i + 1..chars.len()).find(|&k| chars[k] == ']') {
                if chars.get(close + 1) == Some(&'(') {
                    if let Some(end) = (close + 2..chars.len()).find(|&k| chars[k] == ')') {
                        flush(&mut out, &mut buf, style);
                        let label: String = chars[i + 1..close].iter().collect();
                        let url: String = chars[close + 2..end].iter().collect();
                        for mut r in parse_inline(&label) {
                            r.link = Some(url.trim().to_string());
                            r.style.bold |= style.bold;
                            r.style.italic |= style.italic;
                            out.push(r);
                        }
                        i = end + 1;
                        continue;
                    }
                }
            }
        }
        buf.push(c);
        i += 1;
    }
    flush(&mut out, &mut buf, style);
    out
}

/* ── Blocks ──────────────────────────────────────────────────── */

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Block {
    Heading(u32, Vec<Run>),
    Para(Vec<Run>),
    /// Items carry their nesting level and their marker ("•", "3.").
    List(Vec<(usize, String, Vec<Run>)>),
    Table { header: Vec<Vec<Run>>, rows: Vec<Vec<Vec<Run>>> },
    Quote(Vec<Run>),
    Code(String),
    Hr,
    PageBreak,
}

fn split_table_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    // Split on unescaped pipes.
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut prev = ' ';
    for c in t.chars() {
        if c == '|' && prev != '\\' {
            cells.push(cur.trim().to_string());
            cur.clear();
        } else {
            if c == '|' {
                cur.pop();
            }
            cur.push(c);
        }
        prev = c;
    }
    cells.push(cur.trim().to_string());
    cells
}

fn is_sep_row(cells: &[String]) -> bool {
    !cells.is_empty()
        && cells.iter().all(|c| {
            let inner = c.trim().trim_matches(':');
            !inner.is_empty() && inner.chars().all(|ch| ch == '-')
        })
}

/// `"12. text"` → `(12, "text")`.
fn ordered_item(t: &str) -> Option<(u32, &str)> {
    let dot = t.find(|c: char| c == '.' || c == ')')?;
    if dot == 0 || dot > 4 {
        return None;
    }
    let n: u32 = t[..dot].parse().ok()?;
    let rest = &t[dot + 1..];
    let rest = rest.strip_prefix(' ')?;
    Some((n, rest))
}

fn bullet_item(t: &str) -> Option<&str> {
    t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ "))
}

fn is_page_break(t: &str) -> bool {
    matches!(t, "\\pagebreak" | "\\newpage" | "<!-- pagebreak -->" | "[pagebreak]")
}

/// Parse Markdown into blocks.
pub(crate) fn parse_blocks(md: &str) -> Vec<Block> {
    let lines: Vec<&str> = md.lines().collect();
    let mut blocks: Vec<Block> = Vec::new();
    let mut para: Vec<Run> = Vec::new();
    let mut i = 0;

    fn flush(blocks: &mut Vec<Block>, para: &mut Vec<Run>) {
        while para.last().map_or(false, |r| r.brk) {
            para.pop();
        }
        if !para.is_empty() {
            blocks.push(Block::Para(std::mem::take(para)));
        }
    }

    while i < lines.len() {
        let line = lines[i];
        let t = line.trim();

        if t.starts_with("```") || t.starts_with("~~~") {
            flush(&mut blocks, &mut para);
            let fence = &t[..3];
            let mut code = String::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with(fence) {
                code.push_str(lines[i]);
                code.push('\n');
                i += 1;
            }
            blocks.push(Block::Code(code));
            i += 1;
            continue;
        }
        if t.is_empty() {
            flush(&mut blocks, &mut para);
            i += 1;
            continue;
        }
        if is_page_break(t) {
            flush(&mut blocks, &mut para);
            blocks.push(Block::PageBreak);
            i += 1;
            continue;
        }
        if t.len() >= 3 && (t.chars().all(|c| c == '-') || t.chars().all(|c| c == '*') || t.chars().all(|c| c == '_')) {
            flush(&mut blocks, &mut para);
            blocks.push(Block::Hr);
            i += 1;
            continue;
        }
        if t.starts_with('|') {
            flush(&mut blocks, &mut para);
            let header = split_table_row(t);
            let mut j = i + 1;
            if j < lines.len() && is_sep_row(&split_table_row(lines[j])) {
                j += 1;
            }
            let mut rows = Vec::new();
            while j < lines.len() && lines[j].trim().starts_with('|') {
                let cells = split_table_row(lines[j]);
                if !is_sep_row(&cells) {
                    rows.push(cells.iter().map(|c| parse_inline(c)).collect());
                }
                j += 1;
            }
            blocks.push(Block::Table { header: header.iter().map(|c| parse_inline(c)).collect(), rows });
            i = j;
            continue;
        }
        let hashes = t.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
            flush(&mut blocks, &mut para);
            let text = t[hashes..].trim().trim_end_matches('#').trim();
            blocks.push(Block::Heading(hashes.min(4) as u32, parse_inline(text)));
            i += 1;
            continue;
        }
        // Lists: consecutive items (any nesting) become one block, so ordered
        // lists keep their own numbers.
        if bullet_item(t).is_some() || ordered_item(t).is_some() {
            flush(&mut blocks, &mut para);
            let mut items: Vec<(usize, String, Vec<Run>)> = Vec::new();
            while i < lines.len() {
                let raw = lines[i];
                let tt = raw.trim();
                let indent = raw.len() - raw.trim_start().len();
                let level = (indent / 2).min(4);
                if let Some(rest) = bullet_item(tt) {
                    let marker = if level % 2 == 0 { "•" } else { "–" };
                    items.push((level, marker.to_string(), parse_inline(rest)));
                } else if let Some((n, rest)) = ordered_item(tt) {
                    items.push((level, format!("{n}."), parse_inline(rest)));
                } else if !tt.is_empty() && indent >= 2 && !items.is_empty() {
                    // Continuation line of the previous item.
                    let last = items.last_mut().unwrap();
                    last.2.push(Run::new(" ", Style::default()));
                    last.2.extend(parse_inline(tt));
                } else {
                    break;
                }
                i += 1;
            }
            blocks.push(Block::List(items));
            continue;
        }
        if let Some(rest) = t.strip_prefix('>') {
            flush(&mut blocks, &mut para);
            let mut runs = parse_inline(rest.trim());
            i += 1;
            while i < lines.len() {
                let Some(more) = lines[i].trim().strip_prefix('>') else { break };
                runs.push(Run::new(" ", Style::default()));
                runs.extend(parse_inline(more.trim()));
                i += 1;
            }
            blocks.push(Block::Quote(runs));
            continue;
        }

        // Paragraph text: lines join with a space; a hard break is two
        // trailing spaces or a trailing backslash.
        let hard = line.ends_with("  ") || t.ends_with('\\');
        let body = t.strip_suffix('\\').unwrap_or(t);
        if !para.is_empty() && !para.last().map_or(false, |r| r.brk) {
            para.push(Run::new(" ", Style::default()));
        }
        para.extend(parse_inline(body));
        if hard {
            para.push(Run::line_break());
        }
        i += 1;
    }
    flush(&mut blocks, &mut para);
    blocks
}

/// Plain text: blank lines separate paragraphs, single newlines are kept.
fn plain_blocks(s: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    for para in s.replace("\r\n", "\n").split("\n\n") {
        let mut runs = Vec::new();
        for (k, line) in para.lines().enumerate() {
            if k > 0 {
                runs.push(Run::line_break());
            }
            runs.push(Run::new(&sanitize(line.trim_end()), Style::default()));
        }
        if runs.iter().any(|r| !r.text.trim().is_empty()) {
            blocks.push(Block::Para(runs));
        }
    }
    blocks
}

/* ── HTML → Markdown ─────────────────────────────────────────── */

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        let tail = &rest[p..];
        let Some(semi) = tail[..tail.len().min(12)].find(';') else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let ent = &tail[1..semi];
        let decoded = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            "euro" => Some('€'),
            "copy" => Some('©'),
            "reg" => Some('®'),
            "hellip" => Some('…'),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "laquo" => Some('«'),
            "raquo" => Some('»'),
            "deg" => Some('°'),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32)
            }
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Escape text so Markdown parsing does not reinterpret it.
fn md_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '*' | '_' | '`' | '[' | ']' | '\\' | '|') {
            o.push('\\');
        }
        o.push(c);
    }
    o
}

/// Semantic HTML → the Markdown dialect above. Tags may carry attributes and
/// any case; CSS is ignored (structure and emphasis drive the styling).
pub(crate) fn html_to_markdown(html: &str) -> String {
    let mut out = String::new();
    let mut lists: Vec<(bool, u32)> = Vec::new(); // (ordered, next number)
    let mut pre = 0usize;
    let mut skip = 0usize; // inside <style>/<script>/<head>
    let mut href: Vec<Option<String>> = Vec::new();
    let mut row_cells = 0usize;
    let mut row_is_header = false;
    let mut header_done = false;
    let mut i = 0;
    let b = html.as_bytes();

    let block = |out: &mut String| {
        let trimmed = out.trim_end_matches([' ', '\t']).len();
        out.truncate(trimmed);
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push_str(if out.ends_with('\n') { "\n" } else { "\n\n" });
        }
    };

    while i < b.len() {
        if b[i] == b'<' {
            let Some(end) = html[i..].find('>').map(|e| i + e) else {
                out.push_str("&lt;");
                i += 1;
                continue;
            };
            let raw = &html[i + 1..end];
            i = end + 1;
            if raw.starts_with("!--") {
                if raw.contains("pagebreak") {
                    block(&mut out);
                    out.push_str("\\pagebreak\n\n");
                }
                if !raw.ends_with("--") {
                    if let Some(e) = html[i..].find("-->") {
                        i += e + 3;
                    }
                }
                continue;
            }
            let closing = raw.starts_with('/');
            let body = raw.trim_start_matches('/').trim_end_matches('/');
            let name: String = body.chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
            let attrs = &body[name.len().min(body.len())..];
            let attr = |key: &str| -> Option<String> {
                let lower = attrs.to_ascii_lowercase();
                let p = lower.find(&format!("{key}="))?;
                let v = attrs[p + key.len() + 1..].trim_start();
                let (q, v) = match v.chars().next()? {
                    '"' => ('"', &v[1..]),
                    '\'' => ('\'', &v[1..]),
                    _ => (' ', v),
                };
                Some(v.split(q).next().unwrap_or("").to_string())
            };
            if matches!(name.as_str(), "style" | "script" | "head" | "title") {
                if closing { skip = skip.saturating_sub(1) } else { skip += 1 }
                continue;
            }
            if skip > 0 {
                continue;
            }
            let attrs_l = attrs.to_ascii_lowercase();
            // CSS breaks, and the editor's own page-break marker.
            let page_break = attrs_l.contains("page-break-before")
                || attrs_l.contains("break-before")
                || (attrs_l.contains("class=\"page-break\"") && !closing);
            if page_break && !closing {
                block(&mut out);
                out.push_str("\\pagebreak\n\n");
            }
            match (name.as_str(), closing) {
                ("h1" | "h2" | "h3" | "h4" | "h5" | "h6", false) => {
                    block(&mut out);
                    let n = name[1..].parse::<usize>().unwrap_or(1).min(4);
                    out.push_str(&"#".repeat(n));
                    out.push(' ');
                }
                ("h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "div" | "section" | "article" | "header" | "footer", true) => block(&mut out),
                ("p" | "div" | "section" | "article" | "header" | "footer", false) => block(&mut out),
                ("br", _) => out.push_str("  \n"),
                ("hr", _) => {
                    block(&mut out);
                    out.push_str("---\n\n");
                }
                ("strong" | "b", _) => out.push_str("**"),
                ("em" | "i", _) => out.push('*'),
                ("code", _) if pre == 0 => out.push('`'),
                ("pre", false) => {
                    block(&mut out);
                    out.push_str("```\n");
                    pre += 1;
                }
                ("pre", true) => {
                    pre = pre.saturating_sub(1);
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str("```\n\n");
                }
                ("blockquote", false) => {
                    block(&mut out);
                    out.push_str("> ");
                }
                ("blockquote", true) => block(&mut out),
                ("ul" | "ol", false) => {
                    if lists.is_empty() {
                        block(&mut out);
                    }
                    let start = attr("start").and_then(|s| s.parse().ok()).unwrap_or(1);
                    lists.push((name == "ol", start));
                }
                ("ul" | "ol", true) => {
                    lists.pop();
                    if lists.is_empty() {
                        block(&mut out);
                    }
                }
                ("li", false) => {
                    if !out.ends_with('\n') && !out.is_empty() {
                        out.push('\n');
                    }
                    let depth = lists.len().saturating_sub(1);
                    out.push_str(&"  ".repeat(depth));
                    match lists.last_mut() {
                        Some((true, n)) => {
                            out.push_str(&format!("{n}. "));
                            *n += 1;
                        }
                        _ => out.push_str("- "),
                    }
                }
                ("li", true) => {
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                }
                ("table", false) => {
                    block(&mut out);
                    header_done = false;
                }
                ("table", true) => block(&mut out),
                ("tr", false) => {
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push('|');
                    row_cells = 0;
                    row_is_header = false;
                }
                ("tr", true) => {
                    out.push('\n');
                    if !header_done {
                        // The first row is the header; Markdown wants a rule.
                        out.push('|');
                        for _ in 0..row_cells.max(1) {
                            out.push_str("---|");
                        }
                        out.push('\n');
                        header_done = true;
                    }
                    let _ = row_is_header;
                }
                ("th", false) => {
                    row_is_header = true;
                    out.push(' ');
                }
                ("td", false) => out.push(' '),
                ("th" | "td", true) => {
                    out.push_str(" |");
                    row_cells += 1;
                }
                ("a", false) => {
                    let h = attr("href");
                    if h.is_some() {
                        out.push('[');
                    }
                    href.push(h);
                }
                ("a", true) => {
                    if let Some(Some(h)) = href.pop() {
                        out.push_str(&format!("]({h})"));
                    }
                }
                _ => {}
            }
            continue;
        }
        // Text node.
        let next = html[i..].find('<').map(|p| i + p).unwrap_or(b.len());
        let text = decode_entities(&html[i..next]);
        i = next;
        if skip > 0 {
            continue;
        }
        if pre > 0 {
            out.push_str(&text);
            continue;
        }
        let mut collapsed = String::new();
        let mut ws = false;
        for c in text.chars() {
            if c.is_whitespace() {
                ws = true;
            } else {
                if ws && !(collapsed.is_empty() && (out.ends_with('\n') || out.ends_with(' ') || out.is_empty() || out.ends_with('|'))) {
                    collapsed.push(' ');
                }
                ws = false;
                collapsed.push(c);
            }
        }
        if ws && !collapsed.is_empty() {
            collapsed.push(' ');
        }
        out.push_str(&md_escape(&collapsed));
    }
    out
}

/* ── Layout ──────────────────────────────────────────────────── */

#[derive(Debug, Clone)]
enum Draw {
    Text { x: f32, y: f32, font: &'static str, size: f32, text: String, link: Option<String> },
    Line { x1: f32, y1: f32, x2: f32, y2: f32, w: f32, c: (f32, f32, f32) },
    Fill { x: f32, y: f32, w: f32, h: f32, c: (f32, f32, f32) },
}

struct Layout {
    pages: Vec<Vec<Draw>>,
    y: f32,
}

/// A laid-out line: pieces with their x offset from the line start.
type LineOut = Vec<(f32, Run)>;

impl Layout {
    fn new() -> Self {
        Self { pages: Vec::new(), y: PAGE_H - MARGIN }
    }

    fn width(&self, text: &str, style: Style, size: f32) -> f32 {
        fonts::text_width(text, font_of(style), size)
    }

    fn page(&mut self) -> &mut Vec<Draw> {
        if self.pages.is_empty() {
            self.pages.push(Vec::new());
            self.y = PAGE_H - MARGIN;
        }
        self.pages.last_mut().unwrap()
    }

    fn new_page(&mut self) {
        self.pages.push(Vec::new());
        self.y = PAGE_H - MARGIN;
    }

    /// Make room for `h` points; break the page only if it already has
    /// content (so no empty pages, ever).
    fn need(&mut self, h: f32) {
        self.page();
        if self.y - h < BOTTOM && !self.pages.last().unwrap().is_empty() {
            self.new_page();
        }
    }

    fn draw(&mut self, d: Draw) {
        self.page().push(d);
    }

    /// Wrap runs to `max_w` at `size`. Words longer than a line are split.
    fn wrap(&self, runs: &[Run], size: f32, max_w: f32) -> Vec<LineOut> {
        // Tokens: words with their style, spaces, and forced breaks.
        enum Tok {
            Word(Vec<Run>),
            Space,
            Break,
        }
        let mut toks: Vec<Tok> = Vec::new();
        let mut cur: Vec<Run> = Vec::new();
        for r in runs {
            if r.brk {
                if !cur.is_empty() {
                    toks.push(Tok::Word(std::mem::take(&mut cur)));
                }
                toks.push(Tok::Break);
                continue;
            }
            let mut piece = String::new();
            for c in r.text.chars() {
                if c == ' ' {
                    if !piece.is_empty() {
                        cur.push(Run { text: std::mem::take(&mut piece), ..r.clone() });
                    }
                    if !cur.is_empty() {
                        toks.push(Tok::Word(std::mem::take(&mut cur)));
                    }
                    toks.push(Tok::Space);
                } else {
                    piece.push(c);
                }
            }
            if !piece.is_empty() {
                cur.push(Run { text: piece, ..r.clone() });
            }
        }
        if !cur.is_empty() {
            toks.push(Tok::Word(cur));
        }

        let space_w = self.width(" ", Style::default(), size);
        let mut lines: Vec<LineOut> = Vec::new();
        let mut line: LineOut = Vec::new();
        let mut x = 0.0f32;
        let mut pending_space = false;
        for tok in toks {
            match tok {
                Tok::Break => {
                    lines.push(std::mem::take(&mut line));
                    x = 0.0;
                    pending_space = false;
                }
                Tok::Space => pending_space = !line.is_empty(),
                Tok::Word(parts) => {
                    let ww: f32 = parts.iter().map(|p| self.width(&p.text, p.style, size)).sum();
                    let sp = if pending_space { space_w } else { 0.0 };
                    if !line.is_empty() && x + sp + ww > max_w {
                        lines.push(std::mem::take(&mut line));
                        x = 0.0;
                    } else {
                        x += sp;
                    }
                    pending_space = false;
                    for p in parts {
                        let mut w = self.width(&p.text, p.style, size);
                        if w <= max_w || !line.is_empty() && x > 0.0 {
                            if x + w > max_w && !line.is_empty() {
                                lines.push(std::mem::take(&mut line));
                                x = 0.0;
                            }
                            line.push((x, p.clone()));
                            x += w;
                            continue;
                        }
                        // A single word wider than the line: split by chars.
                        let mut chunk = String::new();
                        for c in p.text.chars() {
                            let cw = self.width(&c.to_string(), p.style, size);
                            if x + self.width(&chunk, p.style, size) + cw > max_w && !chunk.is_empty() {
                                line.push((x, Run { text: std::mem::take(&mut chunk), ..p.clone() }));
                                lines.push(std::mem::take(&mut line));
                                x = 0.0;
                            }
                            chunk.push(c);
                        }
                        w = self.width(&chunk, p.style, size);
                        line.push((x, Run { text: chunk, ..p.clone() }));
                        x += w;
                    }
                }
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
        lines
    }

    /// Draw one wrapped line. Consecutive pieces with the same style are
    /// merged into a single string *with real space glyphs*: text extraction,
    /// copy/paste and the inline editor all rely on spaces being bytes in the
    /// stream, not just gaps between positioned words.
    fn emit_line(&mut self, line: &LineOut, x0: f32, size: f32, force: Option<Style>) {
        let y = self.y;
        let resolve = |r: &Run| force.map(|f| Style { bold: f.bold || r.style.bold, italic: f.italic || r.style.italic, code: r.style.code }).unwrap_or(r.style);
        // (start x, style, link, text, end x)
        let mut groups: Vec<(f32, Style, Option<String>, String, f32)> = Vec::new();
        for (dx, r) in line {
            if r.text.is_empty() {
                continue;
            }
            let style = resolve(r);
            let w = self.width(&r.text, style, size);
            match groups.last_mut() {
                Some(g) if g.1 == style && g.2 == r.link => {
                    let gap = dx - g.4;
                    if gap > 0.5 {
                        let sp = self.width(" ", style, size).max(0.1);
                        let n = ((gap / sp).round() as usize).max(1);
                        g.3.push_str(&" ".repeat(n));
                    }
                    g.3.push_str(&r.text);
                    g.4 = dx + w;
                }
                _ => {
                    // A space at a style boundary belongs to the next group,
                    // so the copied text keeps its word breaks.
                    let mut text = String::new();
                    let mut x = *dx;
                    if let Some(prev) = groups.last() {
                        if dx - prev.4 > 0.5 {
                            text.push(' ');
                            x -= self.width(" ", style, size);
                        }
                    }
                    text.push_str(&r.text);
                    groups.push((x, style, r.link.clone(), text, dx + w));
                }
            }
        }
        for (dx, style, link, text, end) in groups {
            self.draw(Draw::Text { x: x0 + dx, y, font: font_of(style), size, text, link: link.clone() });
            if link.is_some() {
                self.draw(Draw::Line { x1: x0 + dx, y1: y - 1.5, x2: x0 + end, y2: y - 1.5, w: 0.5, c: (0.05, 0.35, 0.75) });
            }
        }
    }

    fn paragraph(&mut self, runs: &[Run], x0: f32, max_w: f32, size: f32, force: Option<Style>) -> usize {
        let lines = self.wrap(runs, size, max_w);
        let lh = size * LEAD;
        for line in &lines {
            self.need(lh);
            self.emit_line(line, x0, size, force);
            self.y -= lh;
        }
        lines.len()
    }

    fn block(&mut self, block: &Block, next_is_body: bool) {
        let max_w = PAGE_W - MARGIN * 2.0;
        match block {
            Block::Heading(level, runs) => {
                let size = match level {
                    1 => 22.0,
                    2 => 16.0,
                    3 => 13.0,
                    _ => 11.5,
                };
                let space_before = if self.pages.last().map_or(true, |p| p.is_empty()) { 0.0 } else { size * 0.5 };
                // Keep the heading with at least two lines of what follows.
                let keep = size * 1.9 + if next_is_body { BODY * LEAD * 2.0 } else { 0.0 };
                self.need(space_before + keep);
                if self.pages.last().map_or(false, |p| !p.is_empty()) {
                    self.y -= space_before;
                }
                self.y -= size * 0.3;
                let bold = Style { bold: true, ..Style::default() };
                let lines = self.wrap(&styled(runs, bold), size, max_w);
                for line in &lines {
                    self.need(size * 1.25);
                    self.emit_line(line, MARGIN, size, None);
                    self.y -= size * 1.25;
                }
                if *level == 1 {
                    let ry = self.y + size * 0.55;
                    self.draw(Draw::Line { x1: MARGIN, y1: ry, x2: PAGE_W - MARGIN, y2: ry, w: 1.5, c: C_HEAD });
                    self.y -= size * 0.35;
                } else {
                    self.y -= size * 0.15;
                }
            }
            Block::Para(runs) => {
                self.paragraph(runs, MARGIN, max_w, BODY, None);
                self.y -= BODY * 0.6;
            }
            Block::List(items) => {
                let lh = BODY * LEAD;
                for (level, marker, runs) in items {
                    let indent = MARGIN + *level as f32 * 16.0;
                    let mw = self.width(marker, Style::default(), BODY);
                    let gutter = (mw + 6.0).max(14.0);
                    let lines = self.wrap(runs, BODY, max_w - (indent - MARGIN) - gutter);
                    for (k, line) in lines.iter().enumerate() {
                        self.need(lh);
                        if k == 0 {
                            let y = self.y;
                            self.draw(Draw::Text { x: indent, y, font: "Helvetica", size: BODY, text: marker.clone(), link: None });
                        }
                        self.emit_line(line, indent + gutter, BODY, None);
                        self.y -= lh;
                    }
                    if lines.is_empty() {
                        self.need(lh);
                        let y = self.y;
                        self.draw(Draw::Text { x: indent, y, font: "Helvetica", size: BODY, text: marker.clone(), link: None });
                        self.y -= lh;
                    }
                    self.y -= BODY * 0.15;
                }
                self.y -= BODY * 0.5;
            }
            Block::Table { header, rows } => self.table(header, rows, max_w),
            Block::Quote(runs) => {
                let lines = self.wrap(&styled(runs, Style { italic: true, ..Style::default() }), BODY, max_w - 16.0);
                let lh = BODY * LEAD;
                for line in &lines {
                    self.need(lh);
                    let y = self.y;
                    self.draw(Draw::Line { x1: MARGIN + 2.0, y1: y + BODY, x2: MARGIN + 2.0, y2: y - lh + BODY, w: 2.0, c: C_RULE });
                    self.emit_line(line, MARGIN + 16.0, BODY, None);
                    self.y -= lh;
                }
                self.y -= BODY * 0.6;
            }
            Block::Code(code) => {
                let size = 9.0;
                let lh = size * 1.35;
                let code_style = Style { code: true, ..Style::default() };
                self.y -= 2.0;
                for raw in code.trim_end_matches('\n').lines() {
                    let text = sanitize(raw);
                    let lines = self.wrap(&[Run::new(&text, code_style)], size, max_w - 16.0);
                    let lines = if lines.is_empty() { vec![Vec::new()] } else { lines };
                    for line in &lines {
                        self.need(lh);
                        let y = self.y;
                        self.draw(Draw::Fill { x: MARGIN, y: y - lh + size + 1.0, w: max_w, h: lh, c: C_FILL });
                        // Keep leading indentation: wrap() drops leading spaces.
                        let lead = text.len() - text.trim_start().len();
                        let pad = if std::ptr::eq(line, &lines[0]) { self.width(&" ".repeat(lead), code_style, size) } else { 0.0 };
                        self.emit_line(line, MARGIN + 8.0 + pad, size, None);
                        self.y -= lh;
                    }
                }
                self.y -= BODY * 0.8;
            }
            Block::Hr => {
                self.need(14.0);
                self.y -= 4.0;
                let y = self.y;
                self.draw(Draw::Line { x1: MARGIN, y1: y, x2: PAGE_W - MARGIN, y2: y, w: 0.75, c: C_RULE });
                self.y -= 12.0;
            }
            Block::PageBreak => {
                if self.pages.last().map_or(false, |p| !p.is_empty()) {
                    self.new_page();
                }
            }
        }
    }

    fn table(&mut self, header: &[Vec<Run>], rows: &[Vec<Vec<Run>>], max_w: f32) {
        let ncols = header.len().max(rows.iter().map(|r| r.len()).max().unwrap_or(0));
        if ncols == 0 {
            return;
        }
        let size = 9.5;
        let pad = 5.0;
        let lh = size * 1.35;
        let bold = Style { bold: true, ..Style::default() };
        let cell_w = |l: &Layout, runs: &[Run], st: Option<Style>| -> f32 {
            runs.iter().map(|r| l.width(&r.text, st.unwrap_or(r.style), size)).sum()
        };
        // Natural widths, then shrink wide columns first to fit the page.
        let mut nat = vec![24.0f32; ncols];
        for c in 0..ncols {
            if let Some(h) = header.get(c) {
                nat[c] = nat[c].max(cell_w(self, h, Some(bold)) + pad * 2.0);
            }
            for r in rows {
                if let Some(cell) = r.get(c) {
                    nat[c] = nat[c].max(cell_w(self, cell, None) + pad * 2.0);
                }
            }
        }
        let mut widths = nat.clone();
        let total: f32 = widths.iter().sum();
        if total > max_w {
            // Give every column a fair share; columns narrower than it keep
            // their natural width, the rest split what remains.
            let mut fixed = vec![false; ncols];
            loop {
                let free: f32 = max_w - (0..ncols).filter(|&c| fixed[c]).map(|c| widths[c]).sum::<f32>();
                let open: Vec<usize> = (0..ncols).filter(|&c| !fixed[c]).collect();
                if open.is_empty() {
                    break;
                }
                let share = free / open.len() as f32;
                let mut changed = false;
                for &c in &open {
                    if nat[c] <= share {
                        widths[c] = nat[c];
                        fixed[c] = true;
                        changed = true;
                    }
                }
                if !changed {
                    for &c in &open {
                        widths[c] = share.max(30.0);
                    }
                    break;
                }
            }
        }
        let table_w: f32 = widths.iter().sum();

        let draw_row = |l: &mut Layout, cells: &[Vec<Run>], is_header: bool| {
            let st = if is_header { Some(bold) } else { None };
            let wrapped: Vec<Vec<LineOut>> = (0..ncols)
                .map(|c| {
                    let runs: Vec<Run> = cells.get(c).cloned().unwrap_or_default();
                    let runs = match st { Some(f) => styled(&runs, f), None => runs };
                    l.wrap(&runs, size, (widths[c] - pad * 2.0).max(10.0))
                })
                .collect();
            let nlines = wrapped.iter().map(|w| w.len()).max().unwrap_or(1).max(1);
            let row_h = nlines as f32 * lh + pad;
            l.need(row_h);
            let top = l.y + size;
            if is_header {
                l.draw(Draw::Fill { x: MARGIN, y: top - row_h, w: table_w, h: row_h, c: C_FILL });
            }
            let mut x = MARGIN;
            for c in 0..ncols {
                let y0 = l.y;
                for (k, line) in wrapped[c].iter().enumerate() {
                    l.y = y0 - k as f32 * lh - pad * 0.3;
                    l.emit_line(line, x + pad, size, None);
                }
                l.y = y0;
                x += widths[c];
            }
            l.y -= row_h;
            let ly = l.y + size - 1.0;
            l.draw(Draw::Line { x1: MARGIN, y1: ly, x2: MARGIN + table_w, y2: ly, w: if is_header { 0.9 } else { 0.4 }, c: C_RULE });
        };

        self.y -= 2.0;
        let has_header = header.iter().any(|h| !h.is_empty());
        if has_header {
            draw_row(self, header, true);
        }
        for r in rows {
            let page_before = self.pages.len();
            // Repeat the header when a row starts a new page.
            let est = lh * 2.0;
            if self.y - est < BOTTOM && has_header {
                self.need(est + lh * 2.0);
                if self.pages.len() != page_before {
                    draw_row(self, header, true);
                }
            }
            draw_row(self, r, false);
        }
        self.y -= BODY * 0.8;
    }
}

/// Apply a forced style (heading bold, quote italic, header bold) *before*
/// wrapping, so words are measured in the face they are drawn with — measuring
/// bold text as regular made the spaces between words vanish.
fn styled(runs: &[Run], f: Style) -> Vec<Run> {
    runs.iter()
        .map(|r| Run { style: Style { bold: r.style.bold || f.bold, italic: r.style.italic || f.italic, code: r.style.code }, ..r.clone() })
        .collect()
}

fn layout(blocks: &[Block]) -> Vec<Vec<Draw>> {
    let mut l = Layout::new();
    for (k, b) in blocks.iter().enumerate() {
        let next_is_body = matches!(blocks.get(k + 1), Some(Block::Para(_) | Block::List(_) | Block::Table { .. } | Block::Quote(_) | Block::Code(_)));
        l.block(b, next_is_body);
    }
    if l.pages.is_empty() {
        l.pages.push(Vec::new()); // one blank page for an empty document
    }
    l.pages
}

fn emit(pages: Vec<Vec<Draw>>, opts: &CreateOptions) -> Result<Vec<u8>, AppError> {
    let err = |e: lopdf::Error| AppError::BadRequest(format!("PDF error: {e}"));
    let total = pages.len();
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();

    // One font object per Base-14 face actually used.
    let mut font_ids: Vec<(&'static str, lopdf::ObjectId)> = Vec::new();
    let mut font_ref = |doc: &mut Document, name: &'static str| -> Vec<u8> {
        if !font_ids.iter().any(|(n, _)| *n == name) {
            let id = doc.add_object(dictionary! {
                "Type" => "Font", "Subtype" => "Type1",
                "BaseFont" => Object::Name(name.as_bytes().to_vec()),
                "Encoding" => "WinAnsiEncoding",
            });
            font_ids.push((name, id));
        }
        crate::doc::font_res(name)
    };

    let mut kids = Vec::with_capacity(total);
    for (n, page) in pages.into_iter().enumerate() {
        let mut ops: Vec<Operation> = Vec::new();
        let mut annots: Vec<Object> = Vec::new();
        let mut text_mode = false;
        let mut cur_font: Option<(&'static str, f32)> = None;
        let end_text = |ops: &mut Vec<Operation>, text_mode: &mut bool| {
            if *text_mode {
                ops.push(op("ET", vec![]));
                *text_mode = false;
            }
        };
        let mut draws = page;
        if opts.page_numbers {
            let label = format!("{} / {}", n + 1, total);
            let w = fonts::text_width(&label, "Helvetica", 8.5);
            draws.push(Draw::Text { x: (PAGE_W - w) / 2.0, y: BOTTOM / 2.0, font: "Helvetica", size: 8.5, text: label, link: None });
        }
        for d in draws {
            match d {
                Draw::Text { x, y, font, size, text, link } => {
                    if !text_mode {
                        ops.push(op("BT", vec![]));
                        ops.push(op("rg", reals(&[0.0, 0.0, 0.0])));
                        text_mode = true;
                        cur_font = None;
                    }
                    if cur_font != Some((font, size)) {
                        let name = font_ref(&mut doc, font);
                        ops.push(op("Tf", vec![Object::Name(name), Object::Real(size)]));
                        cur_font = Some((font, size));
                    }
                    ops.push(op("Tm", reals(&[1.0, 0.0, 0.0, 1.0, x, y])));
                    ops.push(op("Tj", vec![Object::string_literal(fonts::encode_winansi(&text))]));
                    if let Some(url) = link.filter(|u| !u.is_empty()) {
                        let w = fonts::text_width(&text, font, size);
                        annots.push(Object::Dictionary(dictionary! {
                            "Type" => "Annot", "Subtype" => "Link",
                            "Rect" => reals(&[x, y - size * 0.25, x + w, y + size * 0.9]),
                            "Border" => vec![0.into(), 0.into(), 0.into()],
                            "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal(url.into_bytes()) },
                        }));
                    }
                }
                Draw::Line { x1, y1, x2, y2, w, c } => {
                    end_text(&mut ops, &mut text_mode);
                    ops.push(op("RG", reals(&[c.0, c.1, c.2])));
                    ops.push(op("w", reals(&[w])));
                    ops.push(op("m", reals(&[x1, y1])));
                    ops.push(op("l", reals(&[x2, y2])));
                    ops.push(op("S", vec![]));
                }
                Draw::Fill { x, y, w, h, c } => {
                    end_text(&mut ops, &mut text_mode);
                    ops.push(op("rg", reals(&[c.0, c.1, c.2])));
                    ops.push(op("re", reals(&[x, y, w, h])));
                    ops.push(op("f", vec![]));
                }
            }
        }
        end_text(&mut ops, &mut text_mode);
        let content = Content { operations: ops }.encode().map_err(err)?;
        let content_id = doc.add_object(Stream::new(dictionary! {}, content));
        let mut page_dict = dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => reals(&[0.0, 0.0, PAGE_W, PAGE_H]),
            "Contents" => content_id,
        };
        if !annots.is_empty() {
            let refs: Vec<Object> = annots.into_iter().map(|a| Object::Reference(doc.add_object(a))).collect();
            page_dict.set("Annots", refs);
        }
        kids.push(Object::Reference(doc.add_object(page_dict)));
    }

    let mut font_dict = Dictionary::new();
    for (name, id) in &font_ids {
        font_dict.set(crate::doc::font_res(name), Object::Reference(*id));
    }
    let resources = doc.add_object(dictionary! { "Font" => font_dict });
    doc.objects.insert(pages_id, Object::Dictionary(dictionary! {
        "Type" => "Pages", "Kids" => kids, "Count" => total as i64, "Resources" => resources,
    }));
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    let mut info = dictionary! { "Producer" => Object::string_literal("Shiny PDF") };
    if let Some(t) = opts.title.as_deref().filter(|t| !t.trim().is_empty()) {
        info.set("Title", lopdf::text_string(t));
    }
    let info_id = doc.add_object(info);
    doc.trailer.set("Info", info_id);
    crate::doc::save(&mut doc)
}

/* ── Blocks → editor HTML ─────────────────────────────────────── */

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn runs_html(runs: &[Run]) -> String {
    let mut out = String::new();
    for r in runs {
        if r.brk {
            out.push_str("<br>");
            continue;
        }
        let mut t = esc(&r.text);
        if r.style.code {
            t = format!("<code>{t}</code>");
        }
        if r.style.italic {
            t = format!("<em>{t}</em>");
        }
        if r.style.bold {
            t = format!("<strong>{t}</strong>");
        }
        if let Some(url) = &r.link {
            t = format!("<a href=\"{}\">{t}</a>", esc(url));
        }
        out.push_str(&t);
    }
    out
}

/// The document editor's HTML for Markdown / HTML / plain source, so a PDF the
/// assistant created can be opened and edited visually.
pub fn to_editor_html(content: &str, format: &str) -> String {
    let content = content.replace("\r\n", "\n");
    let blocks = match format.trim().to_ascii_lowercase().as_str() {
        "plain" | "text" | "txt" => plain_blocks(&content),
        "html" if content.contains("</") => parse_blocks(&html_to_markdown(&content)),
        _ => parse_blocks(&content),
    };
    let mut out = String::new();
    for b in &blocks {
        match b {
            Block::Heading(l, runs) => out.push_str(&format!("<h{l}>{}</h{l}>", runs_html(runs))),
            Block::Para(runs) => out.push_str(&format!("<p>{}</p>", runs_html(runs))),
            Block::Quote(runs) => out.push_str(&format!("<blockquote>{}</blockquote>", runs_html(runs))),
            Block::Code(code) => out.push_str(&format!("<pre>{}</pre>", esc(code.trim_end_matches('\n')))),
            Block::Hr => out.push_str("<hr>"),
            Block::PageBreak => out.push_str("<div class=\"page-break\" contenteditable=\"false\"></div>"),
            Block::List(items) => {
                // Nested levels become nested lists; ordered when the first item is numbered.
                let mut depth: Vec<&'static str> = Vec::new();
                for (level, marker, runs) in items {
                    let tag = if marker.ends_with('.') { "ol" } else { "ul" };
                    while depth.len() > level + 1 {
                        out.push_str(&format!("</li></{}>", depth.pop().unwrap()));
                    }
                    if depth.len() < level + 1 {
                        while depth.len() < level + 1 {
                            out.push_str(&format!("<{tag}>"));
                            depth.push(tag);
                        }
                    } else {
                        out.push_str("</li>");
                    }
                    out.push_str(&format!("<li>{}", runs_html(runs)));
                }
                while let Some(t) = depth.pop() {
                    out.push_str(&format!("</li></{t}>"));
                }
            }
            Block::Table { header, rows } => {
                out.push_str("<table><thead><tr>");
                for c in header {
                    out.push_str(&format!("<th>{}</th>", runs_html(c)));
                }
                out.push_str("</tr></thead><tbody>");
                for r in rows {
                    out.push_str("<tr>");
                    for c in r {
                        out.push_str(&format!("<td>{}</td>", runs_html(c)));
                    }
                    out.push_str("</tr>");
                }
                out.push_str("</tbody></table>");
            }
        }
    }
    if out.is_empty() {
        out.push_str("<p><br></p>");
    }
    out
}

/// Create a PDF. `format` = `"markdown"`, `"html"` or `"plain"`; anything else
/// is sniffed (HTML if it looks like markup, Markdown otherwise).
pub fn create(content: &str, format: &str, opts: &CreateOptions) -> Result<Vec<u8>, AppError> {
    let content = content.replace("\r\n", "\n");
    let fmt = format.trim().to_ascii_lowercase();
    let looks_html = {
        let t = content.trim_start().to_ascii_lowercase();
        t.starts_with('<') && (t.contains("</") || t.contains("<br") || t.contains("<hr"))
    };
    let blocks = match fmt.as_str() {
        "plain" | "text" | "txt" => plain_blocks(&content),
        "markdown" | "md" => parse_blocks(&content),
        // "html" is the agent's default, but agents often send Markdown
        // anyway: only treat it as HTML when it actually contains markup.
        "html" if looks_html || content.contains("</") => parse_blocks(&html_to_markdown(&content)),
        _ if looks_html => parse_blocks(&html_to_markdown(&content)),
        _ => parse_blocks(&content),
    };
    emit(layout(&blocks), opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(runs: &[Run]) -> Vec<(&str, Style)> {
        runs.iter().map(|r| (r.text.as_str(), r.style)).collect()
    }

    #[test]
    fn inline_is_left_to_right() {
        let r = parse_inline("a `x*y` and **b** *c*");
        let t = texts(&r);
        assert_eq!(t[1].0, "x*y");
        assert!(t[1].1.code);
        assert!(t.iter().any(|(s, st)| *s == "b" && st.bold));
        assert!(t.iter().any(|(s, st)| *s == "c" && st.italic));
    }

    #[test]
    fn snake_case_is_not_italic() {
        let r = parse_inline("my_var_name");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].text, "my_var_name");
    }

    #[test]
    fn ordered_lists_keep_numbers() {
        let b = parse_blocks("1. a\n2. b\n3. c");
        let Block::List(items) = &b[0] else { panic!() };
        assert_eq!(items.iter().map(|i| i.1.as_str()).collect::<Vec<_>>(), ["1.", "2.", "3."]);
    }

    #[test]
    fn paragraph_lines_join_with_space() {
        let b = parse_blocks("hello\nworld");
        let Block::Para(runs) = &b[0] else { panic!() };
        let s: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(s, "hello world");
    }

    #[test]
    fn html_with_attributes_and_entities() {
        let md = html_to_markdown("<H1 class=\"t\">Titolo</H1><p style=\"x\">A &amp; B</p><ol start=\"3\"><li>x</li><li>y</li></ol>");
        assert!(md.contains("# Titolo"), "{md}");
        assert!(md.contains("A & B"), "{md}");
        assert!(md.contains("3. x") && md.contains("4. y"), "{md}");
    }

    #[test]
    fn editor_html_from_markdown() {
        let h = to_editor_html("# T\n\n- a\n- b\n\n1. x\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n**b** & <i>", "markdown");
        assert!(h.starts_with("<h1>T</h1>"), "{h}");
        assert!(h.contains("<ul><li>a</li><li>b</li></ul>"), "{h}");
        assert!(h.contains("<ol><li>x</li></ol>"), "{h}");
        assert!(h.contains("<th>A</th>") && h.contains("<td>2</td>"), "{h}");
        assert!(h.contains("<strong>b</strong> &amp; &lt;i&gt;"), "{h}");
    }

    #[test]
    fn editor_page_break_marker_breaks_the_page() {
        let html = "<!--pdfdoc theme=modern--><h1>Uno</h1><div class=\"page-break\" contenteditable=\"false\"></div><p>Due</p>";
        let blocks = parse_blocks(&html_to_markdown(html));
        assert!(blocks.iter().any(|b| matches!(b, Block::PageBreak)), "{blocks:?}");
    }

    #[test]
    fn html_table_gets_header_rule() {
        let md = html_to_markdown("<table><tr><th>A</th><th>B</th></tr><tr><td>1</td><td>2</td></tr></table>");
        let b = parse_blocks(&md);
        let Block::Table { header, rows } = &b[0] else { panic!("{md}") };
        assert_eq!(header.len(), 2);
        assert_eq!(rows.len(), 1);
    }
}
