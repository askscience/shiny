//! The standard 14 fonts: metrics and WinAnsi encoding.
//!
//! Everything this plugin *writes* (new documents, added text, notes,
//! watermarks) uses Base-14 fonts with `WinAnsiEncoding`: no font has to be
//! embedded, every viewer has them, and Western European text — accents, €,
//! curly quotes, dashes — is covered.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::afm;

/// The Base-14 families this plugin writes with.
pub const WRITABLE: [&str; 12] = [
    "Helvetica", "Helvetica-Bold", "Helvetica-Oblique", "Helvetica-BoldOblique",
    "Times-Roman", "Times-Bold", "Times-Italic", "Times-BoldItalic",
    "Courier", "Courier-Bold", "Courier-Oblique", "Courier-BoldOblique",
];

/// Is `name` one of the standard 14 fonts?
pub fn is_base14(name: &str) -> bool {
    WRITABLE.contains(&name) || name == "Symbol" || name == "ZapfDingbats"
}

fn table(font: &str) -> Option<&'static [u16; 256]> {
    Some(match font {
        "Helvetica" => &afm::HELVETICA,
        "Helvetica-Bold" => &afm::HELVETICA_BOLD,
        "Helvetica-Oblique" => &afm::HELVETICA_OBLIQUE,
        "Helvetica-BoldOblique" => &afm::HELVETICA_BOLDOBLIQUE,
        "Times-Roman" => &afm::TIMES_ROMAN,
        "Times-Bold" => &afm::TIMES_BOLD,
        "Times-Italic" => &afm::TIMES_ITALIC,
        "Times-BoldItalic" => &afm::TIMES_BOLDITALIC,
        _ => return None,
    })
}

fn reverse() -> &'static HashMap<char, u8> {
    static R: OnceLock<HashMap<char, u8>> = OnceLock::new();
    R.get_or_init(|| {
        let mut m = HashMap::new();
        for (code, &cp) in afm::WINANSI.iter().enumerate() {
            if cp != 0 {
                if let Some(c) = char::from_u32(cp) {
                    m.entry(c).or_insert(code as u8);
                }
            }
        }
        m
    })
}

/// WinAnsi code for `c`, if the encoding has it.
pub fn winansi_code(c: char) -> Option<u8> {
    reverse().get(&c).copied()
}

/// Unicode for a WinAnsi code.
pub fn winansi_char(code: u8) -> Option<char> {
    let cp = afm::WINANSI[code as usize];
    (cp != 0).then(|| char::from_u32(cp)).flatten()
}

/// Encode text as WinAnsi; characters the encoding lacks become `?`.
pub fn encode_winansi(text: &str) -> Vec<u8> {
    text.chars().map(|c| winansi_code(c).unwrap_or(b'?')).collect()
}

/// The first character of `text` WinAnsi cannot represent, if any.
pub fn first_unencodable(text: &str) -> Option<char> {
    text.chars().find(|&c| c != '\n' && winansi_code(c).is_none())
}

/// Width of one WinAnsi code in `font`, in 1/1000 em.
pub fn code_width(font: &str, code: u8) -> f32 {
    if font.starts_with("Courier") {
        return 600.0;
    }
    let t = table(font).unwrap_or(&afm::HELVETICA);
    match t[code as usize] {
        0 => 500.0,
        w => w as f32,
    }
}

/// Width of `text` in `font` at `size`, in points.
pub fn text_width(text: &str, font: &str, size: f32) -> f32 {
    text.chars()
        .map(|c| code_width(font, winansi_code(c).unwrap_or(b'?')))
        .sum::<f32>()
        / 1000.0
        * size
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adobe_metrics() {
        assert_eq!(text_width("A", "Helvetica", 1000.0), 667.0);
        assert_eq!(text_width("é", "Times-Roman", 1000.0), 444.0);
        assert_eq!(text_width("xyz", "Courier", 10.0), 18.0);
        assert_eq!(encode_winansi("è€"), vec![0xE8, 0x80]);
        assert_eq!(first_unencodable("ok 漢"), Some('漢'));
    }
}
