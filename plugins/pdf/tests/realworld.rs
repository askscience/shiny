//! Real-world smoke test (ignored by default): point `PDF_SAMPLES` at a
//! directory of PDFs and `PDF_OUT` at an output directory.
//!
//! For each file: read runs on the first pages, edit up to 12 runs on page 1
//! (rewriting each with a permutation of its own characters, so the font has
//! every glyph), then do page ops. Results are written for an independent
//! check with pdf.js (`scripts/verify_pdfjs.mjs`).
use shiny_pdf_plugin::ops;

#[test]
#[ignore]
fn real_world_samples() {
    let dir = std::env::var("PDF_SAMPLES").expect("PDF_SAMPLES");
    let out = std::env::var("PDF_OUT").expect("PDF_OUT");
    let mut report = String::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().map_or(true, |e| e != "pdf") {
            continue;
        }
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let bytes = std::fs::read(&path).unwrap();
        let t0 = std::time::Instant::now();
        let n = match ops::page_count(&bytes) {
            Ok(n) => n,
            Err(e) => {
                report.push_str(&format!("{name}: LOAD FAIL {e:?}\n"));
                continue;
            }
        };
        let page = (0..n.min(6)).find(|&p| ops::page_text_runs(&bytes, p).map_or(false, |r| r.len() > 3)).unwrap_or(0);
        let runs = ops::page_text_runs(&bytes, page).unwrap_or_default();
        let mut cur = bytes.clone();
        let (mut ok, mut tried, mut errs) = (0, 0, Vec::new());
        let mut expect = Vec::new();
        for r in runs.iter().filter(|r| r.text.trim().chars().count() >= 4).take(12) {
            tried += 1;
            let t = r.text.trim();
            // Rotate the words: same glyphs, visibly different text.
            let mut words: Vec<&str> = t.split_whitespace().collect();
            if words.len() > 1 { words.rotate_left(1); } else { continue; }
            let new = words.join(" ");
            let e = ops::TextEdit { match_text: r.text.clone(), match_rect: r.rect, baseline: Some(r.baseline), text: new.clone() };
            match ops::edit_text_runs(&cur, page, &[e]) {
                Ok((b, 1)) => { cur = b; ok += 1; expect.push(new); }
                Ok((_, k)) => errs.push(format!("applied {k} for {t:?}")),
                Err(e) => errs.push(format!("{t:?}: {e:?}")),
            }
        }
        std::fs::write(format!("{out}/{name}.edited.pdf"), &cur).unwrap();
        std::fs::write(format!("{out}/{name}.expect.txt"), format!("{page}\n{}", expect.join("\n"))).unwrap();
        let pages_ok = ops::rotate(&cur, Some(&[0]), 90)
            .and_then(|b| ops::duplicate_page(&b, 0))
            .and_then(|b| ops::insert_blank(&b, Some(0), 1))
            .and_then(|b| ops::merge(&b, &bytes))
            .and_then(|b| ops::annotate(&b, 0, "highlight", &[[50.0, 50.0, 100.0, 20.0]], "", None, 12.0, ""))
            .and_then(|b| ops::add_watermark(&b, Some(&[0]), "BOZZA"));
        match &pages_ok {
            Ok(b) => {
                std::fs::write(format!("{out}/{name}.ops.pdf"), b).unwrap();
                report.push_str(&format!("{name}: pages={n} runs(p{page})={} edits {ok}/{tried} ops OK ({} pages) {:?}\n", runs.len(), ops::page_count(b).unwrap(), t0.elapsed()));
            }
            Err(e) => report.push_str(&format!("{name}: pages={n} edits {ok}/{tried} OPS FAIL {e:?}\n")),
        }
        for e in errs.iter().take(3) {
            report.push_str(&format!("    {e}\n"));
        }
    }
    println!("{report}");
}

/// Created documents, for the same independent pdf.js check.
#[test]
#[ignore]
fn created_samples() {
    let out = std::env::var("PDF_OUT").expect("PDF_OUT");
    let md = "# Fattura n. 42\n\nCliente: **Mario Rossi** — via Garibaldi 12, Milano.\n\nPerché sì: è così, costa 1.250,00 € «circa».\n\n## Voci\n\n| Voce | Quantità | Prezzo |\n|---|---:|---:|\n| Consulenza tecnica con descrizione molto lunga che deve andare a capo nella cella | 3 | 900 € |\n| Licenza | 1 | 350 € |\n\n1. primo\n2. secondo\n3. terzo\n\n> Nota: pagamento a 30 giorni.\n\nVedi [il sito](https://example.com) per i dettagli.\n";
    let b = ops::create_with(md, "markdown", &ops::CreateOptions { title: Some("Fattura".into()), page_numbers: true }).unwrap();
    std::fs::write(format!("{out}/created-md.edited.pdf"), &b).unwrap();
    std::fs::write(
        format!("{out}/created-md.expect.txt"),
        "0\nFattura n. 42\nCliente: Mario Rossi — via Garibaldi 12, Milano.\nPerché sì: è così, costa 1.250,00 € «circa».\n3.\nterzo\nConsulenza tecnica\nNota: pagamento a 30 giorni.\nVedi il sito per i dettagli.\n1 / 1\n",
    ).unwrap();
    let html = "<h1 style=\"color:red\">Relazione</h1><p>Testo con <b>grassetto</b> &amp; <em>corsivo</em>.</p><ol start=\"5\"><li>cinque</li><li>sei</li></ol>";
    let b = ops::create(html, "", "html").unwrap();
    std::fs::write(format!("{out}/created-html.edited.pdf"), &b).unwrap();
    std::fs::write(format!("{out}/created-html.expect.txt"), "0\nRelazione\nTesto con grassetto & corsivo.\n6.\nsei\n").unwrap();
}
