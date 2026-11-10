//! tsch: text schematics to KiCad. A drawing is a grid of characters; each
//! character is one 1.27 mm wide, 2.54 mm tall cell of the KiCad sheet.

pub mod boxgen;
pub mod diag;
pub mod glyphs;
pub mod kicad;
pub mod meta;
pub mod model;
pub mod parse;
pub mod project;
pub mod sexpr;
pub mod show;
pub mod source;
pub mod verify;

/// Parse one sheet from text (for tests and tools).
pub fn parse_text(text: &str) -> (model::Sheet, diag::Diags) {
    let mut d = diag::Diags::default();
    let src = source::Source::parse("test.tsch", text, &mut d);
    let meta = meta::Meta::parse(&src.header, src.header_line, &mut d);
    let sheet = parse::parse(&src, &meta, "test", &mut d);
    d.sort();
    (sheet, d)
}
