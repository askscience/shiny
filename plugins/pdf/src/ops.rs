//! Shared PDF operations engine (pdf_oxide), used by both the agent tools and
//! the REST routes so they always agree. Everything here is synchronous and
//! CPU-bound; it runs on the plugin's single-thread bridge runtime.
//!
//! Creation deliberately avoids pdf_oxide's HTML/CSS engine (its subset-embedded
//! fonts render overlapping in the tiny-skia rasteriser) and instead lays text
//! out manually with the low-level `DocumentBuilder` + Base-14 fonts, which
//! render correctly everywhere. Annotations are drawn as page *content* for the
//! same reason — pdf_oxide's rasteriser only paints annotations that carry an
//! `/AP` appearance stream, which markup/note annotations don't get.

use std::collections::HashSet;

use pdf_oxide::editor::DocumentEditor;
use pdf_oxide::elements::{FontSpec, PathContent, PathOperation, TextContent, TextStyle};
use pdf_oxide::geometry::Rect;
use pdf_oxide::layout::Color;
use pdf_oxide::rendering::{render_page, RenderOptions};
use pdf_oxide::writer::{DocumentBuilder, LineStyle, PageSize};
use pdf_oxide::PdfDocument;

use shiny_plugin_sdk::errors::AppError;

/// Hard ceiling on page count — guards against pathological inputs.
const MAX_PAGES: usize = 2000;

// Page geometry (A4, in points).
const PAGE_W: f32 = 595.0;
const PAGE_H: f32 = 842.0;
const MARGIN: f32 = 64.0;
const BOTTOM: f32 = 64.0;

// Professional palette (RGB 0..1).
const C_HEAD: [f32; 3] = [0.086, 0.196, 0.309]; // #16324f
const C_RULE: [f32; 3] = [0.70, 0.75, 0.80];
const C_YELLOW: [f32; 3] = [1.0, 0.85, 0.25];
const C_RED: [f32; 3] = [0.85, 0.18, 0.18];
const C_BLUE: [f32; 3] = [0.05, 0.35, 0.75];

fn pdf_err(e: pdf_oxide::Error) -> AppError {
    AppError::BadRequest(format!("PDF error: {e}"))
}

fn open_doc(bytes: &[u8]) -> Result<PdfDocument, AppError> {
    PdfDocument::from_bytes(bytes.to_vec()).map_err(pdf_err)
}

fn open_editor(bytes: &[u8]) -> Result<DocumentEditor, AppError> {
    DocumentEditor::from_bytes(bytes.to_vec()).map_err(pdf_err)
}

/* ── Read / render ──────────────────────────────────────────── */

pub fn page_count(bytes: &[u8]) -> Result<usize, AppError> {
    open_doc(bytes)?.page_count().map_err(pdf_err)
}

pub fn extract_text(bytes: &[u8], page: usize) -> Result<String, AppError> {
    open_doc(bytes)?.extract_text(page).map_err(pdf_err)
}

pub fn render_png(bytes: &[u8], page: usize, dpi: u32) -> Result<Vec<u8>, AppError> {
    let doc = open_doc(bytes)?;
    let img = render_page(&doc, page, &RenderOptions::with_dpi(dpi)).map_err(pdf_err)?;
    Ok(img.data)
}

pub fn page_size_pt(bytes: &[u8], page: usize) -> Result<(f32, f32), AppError> {
    let doc = open_doc(bytes)?;
    let (x0, y0, x1, y1) = doc.get_page_media_box(page).map_err(pdf_err)?;
    Ok((x1 - x0, y1 - y0))
}

/* ── Create (reliable Base-14 layout) ───────────────────────── */

#[derive(Debug, Clone, Copy, PartialEq)]
enum RunStyle {
    Normal,
    Bold,
    Italic,
    Code,
}

#[derive(Debug, Clone)]
struct Run {
    text: String,
    style: RunStyle,
}

#[derive(Debug, Clone)]
enum Block {
    Heading(u32, Vec<Run>),
    Para(Vec<Run>),
    List { ordered: bool, items: Vec<Vec<Run>> },
    Table { header: Vec<Vec<Run>>, rows: Vec<Vec<Vec<Run>>> },
    Quote(Vec<Run>),
    CodeBlock(String),
    Hr,
}

fn font_of(s: RunStyle) -> &'static str {
    match s {
        RunStyle::Bold => "Helvetica-Bold",
        RunStyle::Italic => "Helvetica-Oblique",
        RunStyle::Code => "Courier",
        RunStyle::Normal => "Helvetica",
    }
}

fn run(r: &str, style: RunStyle) -> Run {
    Run { text: r.to_string(), style }
}

/// Inline Markdown → runs: **bold**, `code`, *italic*.
fn parse_inline(s: &str) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    let mut rest = s;
    let push_normal = |out: &mut Vec<Run>, t: &str| {
        if !t.is_empty() {
            out.push(run(t, RunStyle::Normal));
        }
    };
    loop {
        if let Some(pos) = rest.find("**") {
            push_normal(&mut out, &rest[..pos]);
            let after = &rest[pos + 2..];
            match after.find("**") {
                Some(end) => {
                    out.push(run(&after[..end], RunStyle::Bold));
                    rest = &after[end + 2..];
                    continue;
                }
                None => {
                    out.push(run("**", RunStyle::Normal));
                    rest = after;
                    continue;
                }
            }
        }
        if let Some(pos) = rest.find('`') {
            push_normal(&mut out, &rest[..pos]);
            let after = &rest[pos + 1..];
            match after.find('`') {
                Some(end) => {
                    out.push(run(&after[..end], RunStyle::Code));
                    rest = &after[end + 1..];
                    continue;
                }
                None => {
                    out.push(run("`", RunStyle::Normal));
                    rest = after;
                    continue;
                }
            }
        }
        if let Some(pos) = rest.find('*') {
            push_normal(&mut out, &rest[..pos]);
            let after = &rest[pos + 1..];
            match after.find('*') {
                Some(end) => {
                    out.push(run(&after[..end], RunStyle::Italic));
                    rest = &after[end + 1..];
                    continue;
                }
                None => {
                    out.push(run("*", RunStyle::Normal));
                    rest = after;
                    continue;
                }
            }
        }
        push_normal(&mut out, rest);
        break;
    }
    out
}

fn split_table_row(line: &str) -> Vec<String> {
    line.trim()
        .trim_start_matches('|')
        .trim_end_matches('|')
        .split('|')
        .map(|c| c.trim().to_string())
        .collect()
}

fn is_sep_row(cells: &[String]) -> bool {
    !cells.is_empty()
        && cells.iter().all(|c| {
            let inner = c.trim().trim_matches(':');
            !inner.is_empty() && inner.chars().all(|ch| ch == '-')
        })
}

fn strip_ordered_prefix(t: &str) -> Option<&str> {
    let idx = t.find(". ")?;
    if idx == 0 {
        return None;
    }
    let num = &t[..idx];
    if num.is_empty() || !num.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(&t[idx + 2..])
}

/// Parse Markdown into blocks. Also accepts the light Markdown emitted by
/// [`html_to_markdown`], so both inputs share one renderer.
fn parse_blocks(md: &str) -> Vec<Block> {
    let lines: Vec<&str> = md.lines().collect();
    let mut blocks: Vec<Block> = Vec::new();
    let mut para: Vec<Run> = Vec::new();
    let mut in_code = false;
    let mut code: String = String::new();
    let mut i = 0;

    macro_rules! flush_para {
        () => {
            if !para.is_empty() {
                blocks.push(Block::Para(std::mem::take(&mut para)));
            }
        };
    }

    while i < lines.len() {
        let line = lines[i];

        if line.trim_start().starts_with("```") {
            flush_para!();
            if in_code {
                blocks.push(Block::CodeBlock(std::mem::take(&mut code)));
                in_code = false;
            } else {
                in_code = true;
            }
            i += 1;
            continue;
        }
        if in_code {
            code.push_str(line);
            code.push('\n');
            i += 1;
            continue;
        }

        let t = line.trim();
        if t.is_empty() {
            flush_para!();
            i += 1;
            continue;
        }
        if t == "---" || t == "***" || t == "___" {
            flush_para!();
            blocks.push(Block::Hr);
            i += 1;
            continue;
        }

        // Table: a run of `|`-prefixed lines (first = header; optional `|---|`).
        if t.starts_with('|') {
            let header = split_table_row(line);
            let mut rows: Vec<Vec<Vec<Run>>> = Vec::new();
            let mut j = i + 1;
            if j < lines.len() && is_sep_row(&split_table_row(lines[j])) {
                j += 1; // skip the separator row
            }
            while j < lines.len() && lines[j].trim().starts_with('|') {
                let cells = split_table_row(lines[j]);
                if cells.is_empty() {
                    break;
                }
                rows.push(cells.iter().map(|c| parse_inline(c)).collect());
                j += 1;
            }
            flush_para!();
            let header = header.iter().map(|c| parse_inline(c)).collect();
            blocks.push(Block::Table { header, rows });
            i = j;
            continue;
        }

        if let Some(rest) = t.strip_prefix("#### ") {
            flush_para!();
            blocks.push(Block::Heading(4, parse_inline(rest)));
        } else if let Some(rest) = t.strip_prefix("### ") {
            flush_para!();
            blocks.push(Block::Heading(3, parse_inline(rest)));
        } else if let Some(rest) = t.strip_prefix("## ") {
            flush_para!();
            blocks.push(Block::Heading(2, parse_inline(rest)));
        } else if let Some(rest) = t.strip_prefix("# ") {
            flush_para!();
            blocks.push(Block::Heading(1, parse_inline(rest)));
        } else if let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            flush_para!();
            blocks.push(Block::List { ordered: false, items: vec![parse_inline(rest)] });
        } else if let Some(rest) = strip_ordered_prefix(t) {
            flush_para!();
            blocks.push(Block::List { ordered: true, items: vec![parse_inline(rest)] });
        } else if let Some(rest) = t.strip_prefix("> ") {
            flush_para!();
            blocks.push(Block::Quote(parse_inline(rest)));
        } else {
            para.extend(parse_inline(t));
        }
        i += 1;
    }
    if in_code {
        blocks.push(Block::CodeBlock(std::mem::take(&mut code)));
    }
    flush_para!();
    blocks
}

/// A minimal HTML → Markdown conversion so `format:"html"` input (semantic
/// tags) renders through the same reliable Base-14 layout. Arbitrary CSS is
/// ignored — structure and emphasis are what drive the styling.
fn html_to_markdown(html: &str) -> String {
    let mut s = html.to_string();
    s = s.replace("<h1>", "\n# ").replace("</h1>", "\n");
    s = s.replace("<h2>", "\n## ").replace("</h2>", "\n");
    s = s.replace("<h3>", "\n### ").replace("</h3>", "\n");
    s = s.replace("<h4>", "\n#### ").replace("</h4>", "\n");
    s = s.replace("<h5>", "\n#### ").replace("</h5>", "\n");
    s = s.replace("<h6>", "\n#### ").replace("</h6>", "\n");
    s = s.replace("<strong>", "**").replace("</strong>", "**");
    s = s.replace("<b>", "**").replace("</b>", "**");
    s = s.replace("<em>", "*").replace("</em>", "*");
    s = s.replace("<i>", "*").replace("</i>", "*");
    s = s.replace("<pre><code>", "\n```\n").replace("</code></pre>", "\n```\n");
    s = s.replace("<code>", "`").replace("</code>", "`");
    s = s.replace("<blockquote>", "\n> ").replace("</blockquote>", "\n");
    s = s.replace("<li>", "\n- ").replace("</li>", "");
    s = s.replace("<ul>", "\n").replace("</ul>", "\n");
    s = s.replace("<ol>", "\n").replace("</ol>", "\n");
    s = s.replace("<table>", "\n").replace("</table>", "\n");
    s = s.replace("<thead>", "").replace("</thead>", "");
    s = s.replace("<tbody>", "").replace("</tbody>", "");
    s = s.replace("<tr>", "\n| ").replace("</tr>", " |");
    s = s.replace("<th>", " ").replace("</th>", " |");
    s = s.replace("<td>", " ").replace("</td>", " |");
    s = s.replace("<hr>", "\n---\n").replace("<hr/>", "\n---\n").replace("<hr />", "\n---\n");
    s = s.replace("<br>", "\n").replace("<br/>", "\n").replace("<br />", "\n");
    s = s.replace("<p>", "\n").replace("</p>", "\n");

    // Strip remaining tags (and inline style attributes) without a regex.
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// Split a block's runs into words (space-separated), keeping each word's runs.
fn tokenize_words(runs: &[Run]) -> Vec<Vec<Run>> {
    let mut words: Vec<Vec<Run>> = Vec::new();
    let mut cur: Vec<Run> = Vec::new();
    for r in runs {
        for piece in r.text.split_inclusive(' ') {
            let (chunk, has_space) = match piece.strip_suffix(' ') {
                Some(c) => (c, true),
                None => (piece, false),
            };
            if !chunk.is_empty() {
                cur.push(run(chunk, r.style));
            }
            if has_space {
                if !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
            }
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
}

/// Layout all blocks with the `DocumentBuilder` (Base-14 fonts).
fn layout(blocks: &[Block]) -> Result<Vec<u8>, AppError> {
    let mut builder = DocumentBuilder::new();
    {
        let mut page = builder.page(PageSize::A4);
        let mut y = PAGE_H - MARGIN;
        let max_w = PAGE_W - MARGIN * 2.0;
        let body_size = 10.5;

        // Measure a run's width in a given font/size.
        macro_rules! measure {
            ($page:expr, $font:expr, $size:expr, $text:expr) => {{
                let p = $page.font($font, $size);
                let w = p.measure($text);
                (p, w)
            }};
        }

        // Ensure `needed` vertical space; page-break otherwise.
        macro_rules! ensure {
            ($page:expr, $needed:expr) => {{
                let mut p = $page;
                if y - ($needed) < BOTTOM {
                    p = p.new_page_same_size();
                    y = PAGE_H - MARGIN;
                }
                (p)
            }};
        }

        // Wrap `runs` to `max_w`; returns lines of (run, width).
        macro_rules! wrap {
            ($page:expr, $runs:expr, $size:expr, $maxw:expr) => {{
                let mut p = $page;
                let words = tokenize_words($runs);
                let mut lines: Vec<Vec<(Run, f32)>> = Vec::new();
                let mut line: Vec<(Run, f32)> = Vec::new();
                let mut line_w = 0.0;
                let sp = p.measure(" ");
                for word in words {
                    let mut ww = 0.0;
                    let mut mw: Vec<(Run, f32)> = Vec::new();
                    for r in &word {
                        let (pp, w) = measure!(p, font_of(r.style), $size, &r.text);
                        p = pp;
                        ww += w;
                        mw.push((r.clone(), w));
                    }
                    let sep = if line.is_empty() { 0.0 } else { sp };
                    if !line.is_empty() && line_w + sep + ww > $maxw {
                        lines.push(std::mem::take(&mut line));
                        line_w = 0.0;
                    }
                    if !line.is_empty() {
                        line.push((run(" ", RunStyle::Normal), sp));
                        line_w += sp;
                    }
                    line.extend(mw);
                    line_w += ww;
                }
                if !line.is_empty() {
                    lines.push(line);
                }
                (p, lines, sp)
            }};
        }

        for block in blocks {
            match block {
                Block::Heading(level, runs) => {
                    let size = match level {
                        1 => 22.0,
                        2 => 16.0,
                        3 => 13.0,
                        _ => 11.5,
                    };
                    page = ensure!(page, size + 10.0);
                    let mut cx = MARGIN;
                    for r in runs {
                        let (p, w) = measure!(page, "Helvetica-Bold", size, &r.text);
                        page = p;
                        page = page.at(cx, y).text(&r.text);
                        cx += w;
                    }
                    y -= size * 1.3;
                    if *level == 1 {
                        // colored rule under the title
                        let ry = y + size * 0.3;
                        page = page.stroke_line(
                            MARGIN,
                            ry,
                            PAGE_W - MARGIN,
                            ry,
                            LineStyle::new(1.5, C_HEAD[0], C_HEAD[1], C_HEAD[2]),
                        );
                    }
                    y -= size * 0.6;
                }
                Block::Para(runs) => {
                    let (p, lines, sp) = wrap!(page, runs, body_size, max_w);
                    page = p;
                    let lh = body_size * 1.35;
                    for line in &lines {
                        page = ensure!(page, lh);
                        let mut cx = MARGIN;
                        for (r, w) in line {
                            page = page.font(font_of(r.style), body_size);
                            page = page.at(cx, y).text(&r.text);
                            cx += *w;
                        }
                        y -= lh;
                    }
                    let _ = sp;
                    y -= body_size * 0.4;
                }
                Block::List { ordered, items } => {
                    let lh = body_size * 1.35;
                    for (idx, item) in items.iter().enumerate() {
                        let (p, lines, _sp) = wrap!(page, item, body_size, max_w - 18.0);
                        page = p;
                        page = ensure!(page, lh);
                        let marker = if *ordered {
                            format!("{}.", idx + 1)
                        } else {
                            "•".to_string()
                        };
                        page = page.font("Helvetica", body_size);
                        page = page.at(MARGIN, y).text(&marker);
                        for line in &lines {
                            let mut cx = MARGIN + 18.0;
                            for (r, w) in line {
                                page = page.font(font_of(r.style), body_size);
                                page = page.at(cx, y).text(&r.text);
                                cx += *w;
                            }
                            y -= lh;
                            page = ensure!(page, lh);
                        }
                    }
                    y -= body_size * 0.3;
                }
                Block::Table { header, rows } => {
                    let ncols = header.len().max(rows.iter().map(|r| r.len()).max().unwrap_or(0));
                    if ncols > 0 {
                        let cell_size = 9.5;
                        let pad = 6.0;
                        let mut widths = vec![40.0; ncols];
                        for c in 0..ncols {
                            let mut w = 0.0_f32;
                            for cell in std::iter::once(header).chain(rows.iter()) {
                                if let Some(runs) = cell.get(c) {
                                    let mut s = String::new();
                                    for r in runs {
                                        s.push_str(&r.text);
                                    }
                                    let (p, ww) = measure!(page, "Helvetica", cell_size, &s);
                                    page = p;
                                    w = w.max(ww);
                                }
                            }
                            widths[c] = w + pad * 2.0;
                        }
                        let total: f32 = widths.iter().sum();
                        if total > max_w {
                            let scale = max_w / total;
                            for w in widths.iter_mut() {
                                *w *= scale;
                            }
                        }
                        let row_h = cell_size * 1.5;
                        if !header.is_empty() {
                            page = ensure!(page, row_h);
                            let mut x = MARGIN;
                            for c in 0..ncols {
                                let cell = header.get(c).cloned().unwrap_or_default();
                                let mut cx = x + pad;
                                for r in &cell {
                                    let (p, w) = measure!(page, "Helvetica-Bold", cell_size, &r.text);
                                    page = p;
                                    page = page.at(cx, y).text(&r.text);
                                    cx += w;
                                }
                                x += widths[c];
                            }
                            let rule_y = y - 3.0;
                            page = page.stroke_line(
                                MARGIN,
                                rule_y,
                                MARGIN + total.min(max_w),
                                rule_y,
                                LineStyle::new(1.0, C_RULE[0], C_RULE[1], C_RULE[2]),
                            );
                            y -= row_h;
                        }
                        for row in rows {
                            page = ensure!(page, row_h);
                            let mut x = MARGIN;
                            for c in 0..ncols {
                                let cell = row.get(c).cloned().unwrap_or_default();
                                let mut cx = x + pad;
                                for r in &cell {
                                    let (p, w) = measure!(page, "Helvetica", cell_size, &r.text);
                                    page = p;
                                    page = page.at(cx, y).text(&r.text);
                                    cx += w;
                                }
                                x += widths[c];
                            }
                            y -= row_h;
                        }
                        y -= 6.0;
                    }
                }
                Block::Quote(runs) => {
                    let (p, lines, _sp) = wrap!(page, runs, body_size, max_w - 22.0);
                    page = p;
                    let lh = body_size * 1.35;
                    let qx = MARGIN + 12.0;
                    for line in &lines {
                        page = ensure!(page, lh);
                        let mut cx = qx;
                        for (r, w) in line {
                            page = page.font(font_of(r.style), body_size);
                            page = page.at(cx, y).text(&r.text);
                            cx += *w;
                        }
                        y -= lh;
                    }
                    // left rule
                    let rule_top = y + lines.len() as f32 * lh;
                    let rule_bot = y;
                    page = page.stroke_line(
                        MARGIN,
                        rule_top,
                        MARGIN,
                        rule_bot,
                        LineStyle::new(2.0, C_RULE[0], C_RULE[1], C_RULE[2]),
                    );
                    y -= body_size * 0.4;
                }
                Block::CodeBlock(code) => {
                    let lh = 9.0 * 1.3;
                    for cline in code.lines() {
                        page = ensure!(page, lh);
                        page = page.font("Courier", 9.0);
                        page = page.at(MARGIN + 6.0, y).text(cline);
                        y -= lh;
                    }
                    y -= 6.0;
                }
                Block::Hr => {
                    page = ensure!(page, 12.0);
                    page = page.stroke_line(
                        MARGIN,
                        y,
                        PAGE_W - MARGIN,
                        y,
                        LineStyle::new(1.0, C_RULE[0], C_RULE[1], C_RULE[2]),
                    );
                    y -= 12.0;
                }
            }
        }
        page.done();
    }
    builder.build().map_err(pdf_err)
}

/// Create a PDF. `format` = `"markdown"` (default), `"html"`, or `"plain"`.
/// `css` is accepted for API compatibility but ignored — styling comes from
/// structure (headings, emphasis, lists, tables) rendered with Base-14 fonts.
pub fn create(content: &str, _css: &str, format: &str) -> Result<Vec<u8>, AppError> {
    let md = match format.trim().to_lowercase().as_str() {
        "plain" => {
            // Plain text: one paragraph per blank-line-separated chunk.
            let mut s = String::new();
            for para in content.split("\n\n") {
                let p = para.trim().replace('\n', " ");
                if !p.is_empty() {
                    s.push_str(&p);
                    s.push_str("\n\n");
                }
            }
            s
        }
        "html" => html_to_markdown(content),
        _ => content.to_string(),
    };
    let md = if md.trim().is_empty() { " ".to_string() } else { md };
    layout(&parse_blocks(&md))
}

/* ── Page operations ────────────────────────────────────────── */

pub fn rotate(bytes: &[u8], pages: Option<&[usize]>, degrees: i32) -> Result<Vec<u8>, AppError> {
    let mut ed = open_editor(bytes)?;
    match pages {
        None => ed.rotate_all_pages(degrees).map_err(pdf_err)?,
        Some(ps) => {
            for &p in ps {
                ed.rotate_page_by(p, degrees).map_err(pdf_err)?;
            }
        }
    }
    ed.save_to_bytes().map_err(pdf_err)
}

pub fn select_pages(bytes: &[u8], pages: &[usize]) -> Result<Vec<u8>, AppError> {
    let mut ed = open_editor(bytes)?;
    ed.select_pages(pages).map_err(pdf_err)?;
    ed.save_to_bytes().map_err(pdf_err)
}

pub fn delete_pages(bytes: &[u8], remove: &[usize]) -> Result<Vec<u8>, AppError> {
    let mut ed = open_editor(bytes)?;
    let total = ed.current_page_count();
    let remove_set: HashSet<usize> = remove.iter().copied().collect();
    let keep: Vec<usize> = (0..total).filter(|i| !remove_set.contains(i)).collect();
    if keep.is_empty() {
        return Err(AppError::BadRequest("cannot delete every page".into()));
    }
    ed.select_pages(&keep).map_err(pdf_err)?;
    ed.save_to_bytes().map_err(pdf_err)
}

pub fn merge(bytes: &[u8], other: &[u8]) -> Result<Vec<u8>, AppError> {
    let mut ed = open_editor(bytes)?;
    ed.merge_from_bytes(other).map_err(pdf_err)?;
    ed.save_to_bytes().map_err(pdf_err)
}

pub fn extract(bytes: &[u8], pages: &[usize]) -> Result<Vec<u8>, AppError> {
    let mut ed = open_editor(bytes)?;
    ed.extract_pages_to_bytes(pages).map_err(pdf_err)
}

/* ── Content editing & annotations ──────────────────────────── */

pub fn replace_text(
    bytes: &[u8],
    page: usize,
    old: &str,
    new: &str,
) -> Result<(Vec<u8>, usize), AppError> {
    if old.is_empty() {
        return Err(AppError::BadRequest("old text must not be empty".into()));
    }
    let mut ed = open_editor(bytes)?;
    let mut count = 0usize;
    ed.edit_page(page, |p| {
        let ids: Vec<_> = p.find_text_containing(old).into_iter().map(|m| m.id()).collect();
        count = ids.len();
        for id in ids {
            p.modify_text(id, |t| {
                t.text = t.text.replace(old, new);
            })?;
        }
        Ok(())
    })
    .map_err(pdf_err)?;
    let out = ed.save_to_bytes().map_err(pdf_err)?;
    Ok((out, count))
}

fn line_path(rect: [f32; 4], y_at: f32, color: Color, width: f32, dashed: bool) -> PathContent {
    let mut p = PathContent::new(Rect::new(rect[0], rect[1], rect[2], rect[3]));
    let ly = rect[1] + y_at;
    p.operations.push(PathOperation::MoveTo(rect[0], ly));
    p.operations.push(PathOperation::LineTo(rect[0] + rect[2], ly));
    p.stroke_color = Some(color);
    p.stroke_width = width;
    if dashed {
        p.dash_pattern = Some((vec![2.5, 2.0], 0.0));
    }
    p
}

fn rect_outline(rect: [f32; 4], color: Color, width: f32) -> PathContent {
    let mut p = PathContent::new(Rect::new(rect[0], rect[1], rect[2], rect[3]));
    p.operations
        .push(PathOperation::Rectangle(rect[0], rect[1], rect[2], rect[3]));
    p.stroke_color = Some(color);
    p.stroke_width = width;
    p
}

fn rect_fill(rect: [f32; 4], color: Color) -> PathContent {
    let mut p = PathContent::new(Rect::new(rect[0], rect[1], rect[2], rect[3]));
    p.operations
        .push(PathOperation::Rectangle(rect[0], rect[1], rect[2], rect[3]));
    p.fill_color = Some(color);
    p
}

/// Add a visible markup/note to a page. Markups are drawn as page *content*
/// (not annotations) so pdf_oxide's rasteriser — which only paints annotations
/// that carry an `/AP` appearance stream — shows them. `rect` is
/// `[x, y, width, height]` in PDF points (bottom-left origin).
pub fn annotate(
    bytes: &[u8],
    page: usize,
    kind: &str,
    rect: [f32; 4],
    text: &str,
    color: Option<[f32; 3]>,
) -> Result<Vec<u8>, AppError> {
    let kind = kind.trim().to_lowercase();
    let rgb = color.unwrap_or(match kind.as_str() {
        "highlight" | "note" => C_YELLOW,
        "link" => C_BLUE,
        _ => C_RED,
    });
    let c = Color::new(rgb[0], rgb[1], rgb[2]);
    let mut ed = open_editor(bytes)?;

    ed.edit_page(page, |p| {
        match kind.as_str() {
            "highlight" => {
                p.add_path(rect_outline(rect, c, 1.5));
            }
            "underline" => {
                p.add_path(line_path(rect, 0.0, c, 1.2, false));
            }
            "strikeout" => {
                p.add_path(line_path(rect, rect[3] / 2.0, c, 1.2, false));
            }
            "squiggly" => {
                p.add_path(line_path(rect, 0.0, c, 1.2, true));
            }
            "note" => {
                let bw = rect[2].max(150.0);
                let bh = rect[3].max(34.0);
                p.add_path(rect_fill([rect[0], rect[1], bw, bh], Color::new(1.0, 0.96, 0.6)));
                p.add_text(TextContent::new(
                    text,
                    Rect::new(rect[0] + 4.0, rect[1] + bh - 12.0, bw - 8.0, 12.0),
                    FontSpec::new("Helvetica", 9.0),
                    TextStyle::new().with_color(Color::black()),
                ));
            }
            "free_text" => {
                p.add_text(TextContent::new(
                    text,
                    Rect::new(rect[0], rect[1], rect[2], rect[3]),
                    FontSpec::new("Helvetica", 10.0),
                    TextStyle::new().with_color(Color::new(0.0, 0.0, 0.0)),
                ));
            }
            "link" => {
                p.add_path(line_path(rect, 0.0, c, 1.0, false));
                if !text.is_empty() {
                    p.add_annotation(pdf_oxide::writer::LinkAnnotation::uri(
                        Rect::new(rect[0], rect[1], rect[2], rect[3]),
                        text,
                    ));
                }
            }
            other => {
                return Err(pdf_oxide::Error::Unsupported(format!(
                    "unknown annotation kind: {other}"
                )));
            }
        }
        Ok(())
    })
    .map_err(pdf_err)?;

    ed.save_to_bytes().map_err(pdf_err)
}

pub fn add_watermark(
    bytes: &[u8],
    pages: Option<&[usize]>,
    text: &str,
) -> Result<Vec<u8>, AppError> {
    let total = page_count(bytes)?;
    let mut ed = open_editor(bytes)?;
    let targets: Vec<usize> = match pages {
        Some(ps) => ps.to_vec(),
        None => (0..total).collect(),
    };
    for p in targets {
        let media = ed.get_page_media_box(p).map_err(pdf_err)?;
        let rect = Rect::from_points(media[0], media[1], media[2], media[3]);
        ed.edit_page(p, |page| {
            page.add_annotation(
                pdf_oxide::writer::WatermarkAnnotation::new(text)
                    .with_rect(rect)
                    .with_opacity(0.22)
                    .with_rotation(45.0)
                    .with_color(0.5, 0.5, 0.5),
            );
            Ok(())
        })
        .map_err(pdf_err)?;
    }
    ed.save_to_bytes().map_err(pdf_err)
}

/* ── Validation ─────────────────────────────────────────────── */

pub fn validate(bytes: &[u8]) -> Result<usize, AppError> {
    let n = page_count(bytes)?;
    if n == 0 || n > MAX_PAGES {
        Err(AppError::BadRequest(format!("unsupported page count: {n}")))
    } else {
        Ok(n)
    }
}
