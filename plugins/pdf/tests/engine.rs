//! Engine regression tests: creation, in-place text editing, page ops.
use shiny_pdf_plugin::{craft, ops, stream_edit};

fn page_text(bytes: &[u8], p: usize) -> String {
    ops::extract_text(bytes, p).unwrap()
}

fn all_text(bytes: &[u8]) -> String {
    (0..ops::page_count(bytes).unwrap()).map(|p| page_text(bytes, p)).collect::<Vec<_>>().join("\n")
}

fn md(src: &str) -> Vec<u8> {
    ops::create(src, "", "markdown").unwrap()
}

#[test]
fn ordered_list_numbers_survive() {
    let t = all_text(&md("1. uno\n2. due\n3. tre"));
    assert!(t.contains("3."), "{t}");
}

#[test]
fn html_headings_with_attributes_render() {
    let b = ops::create("<h1 class=\"x\">Titolo</h1><p>Corpo &amp; resto</p>", "", "html").unwrap();
    let t = all_text(&b);
    assert!(t.contains("Corpo & resto"), "{t}");
    assert!(!t.contains("&amp;"), "{t}");
}

#[test]
fn no_trailing_blank_page() {
    for n in 40..90 {
        let mut s = String::new();
        for i in 0..n {
            s.push_str(&format!("- item {i}\n"));
        }
        let b = md(&s);
        let last = ops::page_count(&b).unwrap() - 1;
        assert!(!page_text(&b, last).trim().is_empty(), "n={n}: last page blank");
    }
}

#[test]
fn page_numbers_are_drawn() {
    let mut s = String::new();
    for i in 0..120 {
        s.push_str(&format!("Paragraph {i} with some words.\n\n"));
    }
    let b = ops::create_with(&s, "markdown", &ops::CreateOptions { title: None, page_numbers: true }).unwrap();
    let n = ops::page_count(&b).unwrap();
    assert!(n > 1);
    assert!(page_text(&b, n - 1).contains(&format!("{n} / {n}")));
}

#[test]
fn long_table_cells_wrap() {
    let long = "parola ".repeat(40);
    let b = md(&format!("| A | B |\n|---|---|\n| {long} | x |"));
    assert!(all_text(&b).contains("parola"));
}

/// Every run the viewer offers must be editable — including word-per-Tj lines
/// and accented text in our own documents.
#[test]
fn every_run_of_a_created_pdf_is_editable() {
    let b = md("# Fattura\n\nCliente: Mario Rossi\n\nTotale: 100 euro\n\nPerché sì, è così");
    let runs = ops::page_text_runs(&b, 0).unwrap();
    assert!(runs.len() >= 4);
    for r in &runs {
        let e = ops::TextEdit { match_text: r.text.clone(), match_rect: r.rect, baseline: Some(r.baseline), text: "Nuovo testo è".into() };
        let (out, n) = ops::edit_text_runs(&b, 0, &[e]).unwrap();
        assert_eq!(n, 1, "run {:?} not edited", r.text);
        let t = page_text(&out, 0);
        assert!(t.contains("Nuovo testo è"), "run {:?}: {t}", r.text);
        assert!(!t.contains(r.text.trim()), "run {:?} still present: {t}", r.text);
    }
}

#[test]
fn editing_one_line_leaves_its_twin_alone() {
    let b = md("Totale\n\nAltro\n\nTotale");
    let runs = ops::page_text_runs(&b, 0).unwrap();
    let last = runs.iter().filter(|r| r.text.trim() == "Totale").last().unwrap();
    let e = ops::TextEdit { match_text: last.text.clone(), match_rect: last.rect, baseline: None, text: "Somma".into() };
    let (out, _) = ops::edit_text_runs(&b, 0, &[e]).unwrap();
    let t = page_text(&out, 0);
    assert!(t.contains("Totale") && t.contains("Somma"), "{t}");
    let first_pos = t.find("Totale").unwrap();
    assert!(first_pos < t.find("Somma").unwrap(), "the wrong twin changed: {t}");
}

#[test]
fn replace_word_mid_line_reflows_following_words() {
    let b = md("Cliente: Mario Rossi");
    let (out, rep) = stream_edit::replace_text(&b, 0, "Mario", "Massimiliano").unwrap();
    assert_eq!(rep.replaced, 1);
    let t = page_text(&out, 0);
    assert!(t.contains("Massimiliano Rossi"), "{t}");
}

#[test]
fn replace_across_pages_and_missing_glyph_is_an_error() {
    let b = md("alpha\n\n\\pagebreak\n\nalpha beta");
    let (out, rep) = stream_edit::replace_text_pages(&b, None, "alpha", "gamma").unwrap();
    assert_eq!(rep.replaced, 2);
    assert_eq!(rep.pages_touched, 2);
    assert!(!all_text(&out).contains("alpha"));
    // WinAnsi has no CJK: refuse instead of drawing nothing.
    assert!(stream_edit::replace_text(&b, 0, "alpha", "漢字").is_err());
}

#[test]
fn comments_are_real_annotations_editable_and_deletable() {
    let b = md("Some important text\n\nAnother paragraph");
    let out = ops::annotate(&b, 0, "highlight", &[[60.0, 770.0, 120.0, 14.0], [60.0, 750.0, 80.0, 14.0]], "check this", None, 12.0, "me").unwrap();
    assert!(page_text(&out, 0).contains("Some important text"));
    let out = ops::annotate(&out, 0, "note", &[[300.0, 600.0, 20.0, 20.0]], "una nota è qui", None, 12.0, "me").unwrap();
    let out = ops::annotate(&out, 0, "free_text", &[[100.0, 500.0, 0.0, 16.0]], "riga uno\nriga due", Some([0.6, 0.0, 0.0]), 12.0, "me").unwrap();
    let list = craft::comments(&out).unwrap();
    let kinds: Vec<&str> = list.iter().map(|c| c.kind.as_str()).collect();
    assert!(kinds.contains(&"Highlight") && kinds.contains(&"Text") && kinds.contains(&"FreeText"), "{kinds:?}");
    let note = list.iter().find(|c| c.kind == "Text").unwrap();
    assert_eq!(note.text.as_deref(), Some("una nota è qui"));
    let out = craft::set_comment_text(&out, note.page, note.index, "nota cambiata").unwrap();
    let list = craft::comments(&out).unwrap();
    assert!(list.iter().any(|c| c.text.as_deref() == Some("nota cambiata")));
    let hl = list.iter().find(|c| c.kind == "Highlight").unwrap();
    let out = craft::delete_comment(&out, hl.page, hl.index).unwrap();
    assert!(!craft::comments(&out).unwrap().iter().any(|c| c.kind == "Highlight"));
}

#[test]
fn ink_and_link() {
    let b = md("x");
    let out = ops::ink(&b, 0, vec![vec![[100.0, 100.0], [150.0, 140.0], [200.0, 100.0]]], None, "me").unwrap();
    let out = ops::annotate(&out, 0, "link", &[[64.0, 770.0, 50.0, 12.0]], "example.com", None, 12.0, "me").unwrap();
    let kinds: Vec<String> = craft::comments(&out).unwrap().into_iter().map(|c| c.kind).collect();
    assert!(kinds.iter().any(|k| k == "Ink"), "{kinds:?}");
}

#[test]
fn watermark_and_footer_are_removable_marks() {
    let b = md("uno\n\n\\pagebreak\n\ndue");
    let out = ops::add_watermark(&b, None, "BOZZA").unwrap();
    assert!(craft::marks(&out).unwrap().contains(&"watermark".to_string()));
    let out = craft::header_footer(&out, ["".into(), "".into(), "".into(), "".into(), "Pagina <<1>> di <<n>>".into(), "".into()], 9.0).unwrap();
    assert!(page_text(&out, 1).contains("Pagina 2 di 2"), "{}", page_text(&out, 1));
    let out = craft::remove_marks(&out, "watermark").unwrap();
    assert!(!craft::marks(&out).unwrap().contains(&"watermark".to_string()));
    assert!(craft::marks(&out).unwrap().contains(&"header_footer".to_string()));
}

#[test]
fn reorder_extract_merge_keep_order() {
    let b = md("A1\n\n\\pagebreak\n\nB2\n\n\\pagebreak\n\nC3");
    let r = ops::select_pages(&b, &[2, 0, 1]).unwrap();
    assert!(page_text(&r, 0).contains("C3") && page_text(&r, 1).contains("A1") && page_text(&r, 2).contains("B2"));
    let e = ops::extract_pages(&b, &[2, 0]).unwrap();
    assert_eq!(ops::page_count(&e).unwrap(), 2);
    assert!(page_text(&e, 0).contains("C3") && page_text(&e, 1).contains("A1"));
    let m = ops::merge(&b, &e).unwrap();
    assert_eq!(ops::page_count(&m).unwrap(), 5);
    assert!(page_text(&m, 3).contains("C3"));
    let rot = ops::rotate(&b, Some(&[1]), -90).unwrap();
    assert_eq!(ops::page_count(&rot).unwrap(), 3);
}

#[test]
fn blank_duplicate_extract() {
    let b = md("uno\n\n\\pagebreak\n\ndue");
    let ins = ops::insert_blank(&b, Some(0), 1).unwrap();
    assert_eq!(ops::page_count(&ins).unwrap(), 3);
    assert!(page_text(&ins, 1).trim().is_empty());
    assert!(page_text(&ins, 2).contains("due"));

    let dup = ops::duplicate_page(&b, 0).unwrap();
    assert_eq!(ops::page_count(&dup).unwrap(), 3);
    assert!(page_text(&dup, 1).contains("uno"));
    assert!(page_text(&dup, 2).contains("due"));

    let ex = ops::extract_pages(&b, &[1]).unwrap();
    assert_eq!(ops::page_count(&ex).unwrap(), 1);
    assert!(page_text(&ex, 0).contains("due"));
}

/// A word-per-`Tj` PDF with absolute `Tm` positions (how the previous
/// creator wrote, and how many generators write).
fn word_per_tj() -> Vec<u8> {
    use lopdf::{dictionary, Document, Object, Stream};
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    let content = b"BT\n/F1 10.5 Tf\n1 0 0 1 64 736 Tm\n(Cliente:) Tj\n1 0 0 1 100.75 736 Tm\n( ) Tj\n1 0 0 1 106.866 736 Tm\n(Mario) Tj\n1 0 0 1 134.2815 736 Tm\n( ) Tj\n1 0 0 1 140.39749 736 Tm\n(Rossi) Tj\n1 0 0 1 400 736 Tm\n(Colonna) Tj\nET\n".to_vec();
    let c = doc.add_object(Stream::new(dictionary! {}, content));
    let page = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id, "Contents" => c,
        "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
    });
    doc.objects.insert(pages_id, Object::Dictionary(dictionary! {
        "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1,
    }));
    let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", cat);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

#[test]
fn word_per_tj_line_edit_and_reflow() {
    let b = word_per_tj();
    // Whole-line edit across five operators.
    let (out, rep) = stream_edit::replace_text(&b, 0, "Cliente: Mario Rossi", "Cliente: Luca Bianchi").unwrap();
    assert_eq!(rep.replaced, 1);
    let t = page_text(&out, 0);
    assert!(t.contains("Luca Bianchi") && !t.contains("Mario"), "{t}");

    // Mid-line word: the following word moves, the far column does not.
    let (out, _) = stream_edit::replace_text(&b, 0, "Mario", "Massimiliano").unwrap();
    let runs = ops::page_text_runs(&out, 0).unwrap();
    let rossi = runs.iter().find(|r| r.text.contains("Rossi")).expect("Rossi");
    let col = runs.iter().find(|r| r.text.contains("Colonna")).expect("Colonna");
    let massi = runs.iter().find(|r| r.text.contains("Massimiliano")).expect("Massimiliano");
    if rossi.text.contains("Massimiliano") {
        // Extracted as one run: fine, as long as the words are not overlapping.
        assert!(rossi.text.contains("Massimiliano Rossi"), "{:?}", rossi.text);
    } else {
        assert!(rossi.rect[0] >= massi.rect[0] + massi.rect[2] - 0.5, "Rossi overlaps: {rossi:?} {massi:?}");
    }
    let _ = col;
    // The far column keeps its absolute position in the stream.
    let doc = lopdf::Document::load_mem(&out).unwrap();
    let pid = *doc.get_pages().get(&1).unwrap();
    let content = lopdf::content::Content::decode(&doc.get_page_content(pid)).unwrap();
    let ops = &content.operations;
    let k = ops.iter().position(|o| o.operator == "Tj" && matches!(&o.operands[0], lopdf::Object::String(b, _) if b == b"Colonna")).unwrap();
    let tm = &ops[k - 1];
    assert_eq!(tm.operator, "Tm");
    assert!((tm.operands[4].as_float().unwrap() - 400.0).abs() < 0.01, "column moved: {tm:?}");
    // …while "Rossi" moved right by the width difference.
    let r = ops.iter().position(|o| o.operator == "Tj" && matches!(&o.operands[0], lopdf::Object::String(b, _) if b == b"Rossi")).unwrap();
    assert!(ops[r - 1].operands[4].as_float().unwrap() > 160.0, "Rossi not reflowed: {:?}", ops[r - 1]);
}

#[test]
fn bold_headings_and_table_headers_keep_their_spaces() {
    let b = md("# Invoice no. 1\n\n| Long header name | B |\n|---|---|\n| x | y |\n\n> quoted words here");
    let t = page_text(&b, 0);
    assert!(t.contains("Invoice no. 1"), "{t}");
    assert!(t.contains("Long header name"), "{t}");
    assert!(t.contains("quoted words here"), "{t}");
}

#[test]
fn table_cells_are_separate_runs() {
    let b = md("| Item | Qty |\n|---|---|\n| Service | 1 |");
    let runs = ops::page_text_runs(&b, 0).unwrap();
    assert!(runs.iter().any(|r| r.text == "Service"), "{:?}", runs.iter().map(|r| &r.text).collect::<Vec<_>>());
}
