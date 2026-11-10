//! KiCad 10 schematic writer. Every symbol is generated from the drawing,
//! instances are placed at angle 0, and every text is written at the cell
//! where it was drawn.

use std::collections::BTreeMap;

use crate::model::*;
use crate::sexpr::{fmt_num, q, uuid, S};
use crate::sx;

/// Center of grid cell (0, 0) on the page, in mm. On the 1.27 mm grid and
/// inside the drawing frame (which starts 12 mm from the page edge).
pub const ORIGIN: f64 = 15.24;
/// Font height and width. KiCad's stroke font advances 0.86755 x width per
/// character for every glyph, so this width makes one character exactly one
/// 1.27 mm column.
pub const FONT_H: f64 = 1.27;
pub const FONT_W: f64 = 1.4639;
/// Pin numbers.
pub const SMALL: f64 = 1.0;
/// Anchor offsets below the row line that center the capital letters on it
/// (fields and symbol text; schematic text items). Measured on kicad-cli
/// 10.0.8 plots; `tsch verify` re-measures them on every build.
pub const FIELD_DY: f64 = 0.09;
pub const NOTE_DY: f64 = 0.72;
pub const SMALL_DY: f64 = 0.07;
/// Height of a raised pin number's center above its pin line.
pub const RAISE: f64 = 0.76;

pub const SCHEMA_VERSION: &str = "20260306";
/// Symbol library files carry their own format version.
pub const SYMLIB_VERSION: &str = "20251024";
pub const LIB: &str = "tsch";

pub fn x(c: i32) -> f64 {
    ORIGIN + c as f64 * CELL_W
}

pub fn y(r: i32) -> f64 {
    ORIGIN + r as f64 * CELL_H
}

/// Left edge of a cell: the anchor of left-justified text starting there.
pub fn left(c: i32) -> f64 {
    x(c) - CELL_W / 2.0
}

#[derive(Clone, Copy, Debug)]
pub struct Paper {
    pub name: &'static str,
    pub w: f64,
    pub h: f64,
}

pub const PAPERS: [Paper; 5] = [
    Paper { name: "A4", w: 297.0, h: 210.0 },
    Paper { name: "A3", w: 420.0, h: 297.0 },
    Paper { name: "A2", w: 594.0, h: 420.0 },
    Paper { name: "A1", w: 841.0, h: 594.0 },
    Paper { name: "A0", w: 1189.0, h: 841.0 },
];

/// Whether the drawing fits inside the frame and clear of the title block
/// (bottom right, 108 x 32 mm inside the frame).
pub fn fits(sheet: &Sheet, p: &Paper) -> bool {
    let frame_r = p.w - 13.0;
    let frame_b = p.h - 13.0;
    let tb_l = p.w - 121.0;
    let tb_t = p.h - 45.0;
    for (r, &end) in sheet.row_ends.iter().enumerate() {
        if end < 0 {
            continue;
        }
        let bottom = y(r as i32) + CELL_H / 2.0;
        let right = left(end) + CELL_W;
        if bottom > frame_b || right > frame_r {
            return false;
        }
        let top = y(r as i32) - CELL_H / 2.0;
        if bottom > tb_t && top < frame_b && right > tb_l {
            return false;
        }
    }
    true
}

/// Columns and rows that fit on a page, for messages.
pub fn capacity(p: &Paper) -> (i32, i32) {
    let cols = ((p.w - 13.0 - left(0)) / CELL_W).floor() as i32;
    let rows = ((p.h - 13.0 - (ORIGIN - CELL_H / 2.0)) / CELL_H).floor() as i32;
    (cols, rows)
}

pub fn choose_paper(sheet: &Sheet) -> Result<Paper, String> {
    if let Some(name) = &sheet.meta.paper {
        let p = PAPERS
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("unknown paper {name:?}; use A4, A3, A2, A1 or A0"))?;
        if !fits(sheet, p) {
            let (c, r) = capacity(p);
            return Err(format!(
                "the drawing does not fit on {} ({c} columns x {r} rows, less the title block at the bottom right: the last 11 rows end before column {})",
                p.name,
                c - 85
            ));
        }
        return Ok(*p);
    }
    PAPERS
        .iter()
        .find(|p| fits(sheet, p))
        .copied()
        .ok_or_else(|| "the drawing is larger than A0; split it into several sheets".to_string())
}

fn font(h: f64, w: f64) -> S {
    sx!("font", sx!("size", h, w))
}

fn effects(h: f64, w: f64, justify: Option<&str>) -> S {
    let mut v = vec![font(h, w)];
    if let Some(j) = justify {
        v.push(crate::sexpr::list("justify", j.split(' ').map(S::from).collect()));
    }
    crate::sexpr::list("effects", v)
}

fn at(x: f64, y: f64, angle: f64) -> S {
    sx!("at", x, y, angle)
}

fn yes(b: bool) -> S {
    S::Atom(if b { "yes" } else { "no" }.into())
}

fn stroke(w: f64) -> S {
    sx!("stroke", sx!("width", w), sx!("type", "default"))
}

fn fill(f: Fill) -> S {
    let t = match f {
        Fill::None => "none",
        Fill::Background => "background",
        Fill::Solid => "outline",
    };
    sx!("fill", sx!("type", t))
}

fn prop(name: &str, value: &str, x: f64, y: f64, hide: bool, eff: S) -> S {
    let mut v = vec![q(name), q(value), at(x, y, 0.0)];
    if hide {
        v.push(sx!("hide", "yes"));
    }
    v.push(eff);
    crate::sexpr::list("property", v)
}

/// Symbol graphics: our shapes are y-down relative to the origin; symbol
/// space is y-up.
fn shape_sexpr(s: &Shape) -> S {
    match s {
        Shape::Rect { x0, y0, x1, y1, width, fill: f } => {
            sx!("rectangle", sx!("start", *x0, -*y0), sx!("end", *x1, -*y1), stroke(*width), fill(*f))
        }
        Shape::Poly { pts, width, fill: f } => {
            let xy: Vec<S> = pts.iter().map(|(a, b)| sx!("xy", *a, -*b)).collect();
            sx!("polyline", crate::sexpr::list("pts", xy), stroke(*width), fill(*f))
        }
        Shape::Arc { start, mid, end, width } => sx!(
            "arc",
            sx!("start", start.0, -start.1),
            sx!("mid", mid.0, -mid.1),
            sx!("end", end.0, -end.1),
            stroke(*width),
            fill(Fill::None)
        ),
    }
}

fn pin_angle(d: Dir) -> f64 {
    // The angle points from the tip toward the body, in y-up symbol space.
    match d {
        Dir::L => 0.0,
        Dir::R => 180.0,
        Dir::U => 270.0,
        Dir::D => 90.0,
    }
}

/// Text placed on its cells, as (anchor x, anchor y, font size h, w,
/// justify) in schematic coordinates.
pub fn sym_text_anchor(t: &SymText) -> (f64, f64, f64, f64, Option<&'static str>) {
    let n = t.text.chars().count() as i32;
    match t.kind {
        TextKind::Normal => (left(t.col), y(t.row) + FIELD_DY, FONT_H, FONT_W, Some("left")),
        TextKind::PinNumberRaised => {
            let cx = (x(t.col) + x(t.col + n - 1)) / 2.0;
            (cx, y(t.row) - RAISE + SMALL_DY, SMALL, SMALL, None)
        }
        TextKind::PinNumberSide => {
            let cx = (x(t.col) + x(t.col + n - 1)) / 2.0;
            (cx, y(t.row) + SMALL_DY, SMALL, SMALL, None)
        }
    }
}

/// The body of a library symbol (graphics, texts, pins), without its name,
/// used both to write it and to find identical drawings.
fn symbol_units(part: &Part, unit_name: &str) -> (S, S) {
    let (ox, oy) = (x(part.origin.1), y(part.origin.0));
    let mut g: Vec<S> = vec![q(format!("{unit_name}_0_1"))];
    for s in &part.shapes {
        g.push(shape_sexpr(s));
    }
    for t in &part.texts {
        let (ax, ay, h, w, j) = sym_text_anchor(t);
        g.push(sx!("text", q(t.text.clone()), at(ax - ox, -(ay - oy), 0.0), effects(h, w, j)));
    }
    let mut p: Vec<S> = vec![q(format!("{unit_name}_1_1"))];
    for pin in &part.pins {
        let tx = x(pin.tip.1) - ox;
        let ty = -(y(pin.tip.0) - oy);
        let name = if pin.name.is_empty() { "~".to_string() } else { pin.name.clone() };
        p.push(sx!(
            "pin",
            pin.etype.as_str(),
            "line",
            at(tx, ty, pin_angle(pin.dir)),
            sx!("length", pin.length),
            sx!("name", q(name), effects(FONT_H, FONT_W, None)),
            sx!("number", q(pin.number.clone()), effects(SMALL, SMALL, None))
        ));
    }
    (crate::sexpr::list("symbol", g), crate::sexpr::list("symbol", p))
}

fn lib_symbol(name: &str, part: &Part) -> S {
    let (g, p) = symbol_units(part, name);
    let refp: String = part.reference.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let eff = || effects(FONT_H, FONT_W, None);
    sx!(
        "symbol",
        q(format!("{LIB}:{name}")),
        sx!("pin_numbers", sx!("hide", "yes")),
        sx!("pin_names", sx!("offset", 0.0), sx!("hide", "yes")),
        sx!("exclude_from_sim", "no"),
        sx!("in_bom", "yes"),
        sx!("on_board", "yes"),
        prop("Reference", &refp, 0.0, 0.0, false, eff()),
        prop("Value", name, 0.0, 0.0, false, eff()),
        prop("Footprint", "", 0.0, 0.0, true, eff()),
        prop("Datasheet", "", 0.0, 0.0, true, eff()),
        prop("Description", "", 0.0, 0.0, true, eff()),
        g,
        p,
        sx!("embedded_fonts", "no")
    )
}

fn sanitize(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || "_-.+".contains(c) { c } else { '_' })
        .collect();
    if t.is_empty() { "PART".into() } else { t }
}

/// Library symbols of a whole project: one per distinct drawing, shared by
/// identical parts on any sheet.
#[derive(Default)]
pub struct Library {
    /// Symbol name per (sheet index, part index).
    pub names: BTreeMap<(usize, usize), String>,
    pub symbols: BTreeMap<String, S>,
}

impl Library {
    pub fn build(sheets: &[&Sheet]) -> Library {
        let mut lib = Library::default();
        let all: Vec<(usize, usize, &Part)> = sheets
            .iter()
            .enumerate()
            .flat_map(|(si, s)| s.parts.iter().enumerate().map(move |(pi, p)| (si, pi, p)))
            .collect();
        let parts: Vec<&Part> = all.iter().map(|x| x.2).collect();
        let names = symbol_names(&parts);
        for ((si, pi, part), name) in all.iter().zip(names) {
            lib.symbols.entry(name.clone()).or_insert_with(|| lib_symbol(&name, part));
            lib.names.insert((*si, *pi), name);
        }
        for s in sheets {
            for p in &s.ports {
                lib.symbols.entry(power_name(p)).or_insert_with(|| power_symbol(p));
            }
        }
        lib
    }

    /// The symbols as a KiCad library file.
    pub fn file_text(&self) -> String {
        let mut v = vec![
            sx!("version", SYMLIB_VERSION),
            sx!("generator", q("tsch")),
            sx!("generator_version", q(env!("CARGO_PKG_VERSION"))),
        ];
        for s in self.symbols.values() {
            // In a library file the name carries no library prefix.
            v.push(strip_lib(s));
        }
        crate::sexpr::list("kicad_symbol_lib", v).to_text()
    }
}

fn strip_lib(s: &S) -> S {
    match s {
        S::List(v) => {
            let mut v = v.clone();
            if let Some(S::Str(n)) = v.get(1) {
                if let Some(rest) = n.strip_prefix(&format!("{LIB}:")) {
                    v[1] = S::Str(rest.to_string());
                }
            }
            S::List(v)
        }
        other => other.clone(),
    }
}

pub const SYM_LIB_TABLE: &str = "(sym_lib_table\n\t(version 7)\n\t(lib (name \"tsch\") (type \"KiCad\") (uri \"${KIPRJMOD}/tsch.kicad_sym\") (options \"\") (descr \"Symbols generated by tsch\"))\n)\n";

/// Library symbol names per part, sharing one symbol between identical
/// drawings.
pub fn symbol_names(parts: &[&Part]) -> Vec<String> {
    let mut by_body: BTreeMap<String, String> = BTreeMap::new();
    let mut used: BTreeMap<String, String> = BTreeMap::new();
    let mut out = Vec::new();
    for part in parts {
        let (g, p) = symbol_units(part, "X");
        let key = format!("{}{}", g.to_text(), p.to_text());
        if let Some(n) = by_body.get(&key) {
            out.push(n.clone());
            continue;
        }
        let base = match part.kind {
            PartKind::Box => sanitize(if part.value().is_empty() { &part.reference } else { part.value() }),
            PartKind::Glyph(_) => part.shape_name.clone(),
        };
        let mut name = base.clone();
        let mut k = 2;
        while used.get(&name).is_some_and(|other| other != &key) {
            name = format!("{base}_{k}");
            k += 1;
        }
        used.insert(name.clone(), key.clone());
        by_body.insert(key, name.clone());
        out.push(name);
    }
    out
}

fn power_name(p: &Port) -> String {
    format!("PWR_{}_{}", if p.side == Dir::U { "UP" } else { "DN" }, sanitize(&p.name))
}

/// Power port symbol: a stem from the wire end to the name, a bar (supply)
/// or three shrinking bars (ground), all inside the wire end's cell.
fn power_symbol(p: &Port) -> S {
    let name = power_name(p);
    let s = if p.side == Dir::U { 1.0 } else { -1.0 };
    let line = |pts: &[(f64, f64)]| {
        let xy: Vec<S> = pts.iter().map(|(a, b)| sx!("xy", *a, *b * s)).collect();
        sx!("polyline", crate::sexpr::list("pts", xy), stroke(0.0), fill(Fill::None))
    };
    let mut g: Vec<S> = vec![q(format!("{name}_0_1"))];
    if p.ground {
        g.push(line(&[(0.0, 0.0), (0.0, 0.508)]));
        g.push(line(&[(-0.635, 0.508), (0.635, 0.508)]));
        g.push(line(&[(-0.381, 0.762), (0.381, 0.762)]));
        g.push(line(&[(-0.127, 1.016), (0.127, 1.016)]));
    } else {
        g.push(line(&[(0.0, 0.0), (0.0, 1.016)]));
        g.push(line(&[(-0.508, 1.016), (0.508, 1.016)]));
    }
    let eff = || effects(FONT_H, FONT_W, None);
    sx!(
        "symbol",
        q(format!("{LIB}:{name}")),
        sx!("power", "global"),
        sx!("pin_numbers", sx!("hide", "yes")),
        sx!("pin_names", sx!("offset", 0.0), sx!("hide", "yes")),
        sx!("exclude_from_sim", "no"),
        sx!("in_bom", "yes"),
        sx!("on_board", "yes"),
        prop("Reference", "#PWR", 0.0, 0.0, true, eff()),
        prop("Value", &p.name, 0.0, 0.0, false, eff()),
        prop("Footprint", "", 0.0, 0.0, true, eff()),
        prop("Datasheet", "", 0.0, 0.0, true, eff()),
        prop("Description", &format!("Power port {}", p.name), 0.0, 0.0, true, eff()),
        crate::sexpr::list("symbol", g),
        sx!(
            "symbol",
            q(format!("{name}_1_1")),
            sx!(
                "pin",
                "power_in",
                "line",
                at(0.0, 0.0, if p.side == Dir::U { 90.0 } else { 270.0 }),
                sx!("length", 0.0),
                sx!("name", q(""), eff()),
                sx!("number", q("1"), eff())
            )
        ),
        sx!("embedded_fonts", "no")
    )
}

pub struct SheetCtx<'a> {
    pub project: &'a str,
    pub lib: &'a Library,
    /// Index of this sheet in the library's part names.
    pub sheet_index: usize,
    pub sheet_uuid: String,
    /// Instance path of symbols on this sheet: "/root" or "/root/block".
    pub path: String,
    /// First number for #PWR references.
    pub pwr_start: usize,
    pub paper: Paper,
    pub page: String,
    /// Root sheet of a hierarchy: sheet blocks to add.
    pub extra: Vec<S>,
    pub is_root: bool,
}

fn instances(ctx: &SheetCtx, reference: &str) -> S {
    sx!(
        "instances",
        sx!("project", q(ctx.project), sx!("path", q(ctx.path.clone()), sx!("reference", q(reference)), sx!("unit", 1.0)))
    )
}

/// Write one sheet. Returns the file text and the number of #PWR
/// references used.
pub fn write_sheet(sheet: &Sheet, ctx: &SheetCtx) -> (String, usize) {
    let key = |k: &str| uuid(&format!("{}/{}", sheet.name, k));
    let mut items: Vec<S> = Vec::new();
    items.push(sx!("version", SCHEMA_VERSION));
    items.push(sx!("generator", q("tsch")));
    items.push(sx!("generator_version", q(env!("CARGO_PKG_VERSION"))));
    items.push(sx!("uuid", q(ctx.sheet_uuid.clone())));
    items.push(sx!("paper", q(ctx.paper.name)));
    let m = &sheet.meta;
    let mut tb: Vec<S> = vec![sx!("title", q(m.title.clone().unwrap_or_else(|| sheet.name.clone())))];
    if let Some(d) = &m.date {
        tb.push(sx!("date", q(d.clone())));
    }
    if let Some(r) = &m.rev {
        tb.push(sx!("rev", q(r.clone())));
    }
    if let Some(c) = &m.company {
        tb.push(sx!("company", q(c.clone())));
    }
    for (i, c) in m.comments.iter().enumerate().take(9) {
        tb.push(sx!("comment", (i + 1) as f64, q(c.clone())));
    }
    items.push(crate::sexpr::list("title_block", tb));

    // Library symbols used on this sheet.
    let names: Vec<String> = (0..sheet.parts.len()).map(|i| ctx.lib.names[&(ctx.sheet_index, i)].clone()).collect();
    let mut used: BTreeMap<String, S> = BTreeMap::new();
    for n in names.iter().cloned().chain(sheet.ports.iter().map(power_name)) {
        if let Some(s) = ctx.lib.symbols.get(&n) {
            used.entry(n).or_insert_with(|| s.clone());
        }
    }
    items.push(crate::sexpr::list("lib_symbols", used.into_values().collect()));

    // Wires, junctions, no-connects.
    for w in &sheet.wires {
        items.push(sx!(
            "wire",
            sx!("pts", sx!("xy", x(w.a.1), y(w.a.0)), sx!("xy", x(w.b.1), y(w.b.0))),
            stroke(0.0),
            sx!("uuid", q(key(&format!("wire/{:?}/{:?}", w.a, w.b))))
        ));
    }
    for j in &sheet.junctions {
        items.push(sx!(
            "junction",
            at2(x(j.1), y(j.0)),
            sx!("diameter", 0.0),
            sx!("color", 0.0, 0.0, 0.0, 0.0),
            sx!("uuid", q(key(&format!("junction/{j:?}"))))
        ));
    }
    for n in &sheet.no_connects {
        items.push(sx!("no_connect", at2(x(n.1), y(n.0)), sx!("uuid", q(key(&format!("nc/{n:?}"))))));
    }
    // Labels.
    for l in &sheet.labels {
        let (angle, just) = if l.side == Dir::R { (0.0, "left") } else { (180.0, "right") };
        if l.global {
            let a = l.side.step(l.anchor);
            items.push(sx!(
                "global_label",
                q(l.name.clone()),
                sx!("shape", "passive"),
                at(x(a.1), y(a.0), angle),
                sx!("fields_autoplaced", "yes"),
                effects(FONT_H, FONT_W, Some(just)),
                sx!("uuid", q(key(&format!("glabel/{:?}", l.anchor)))),
                prop("Intersheetrefs", "${INTERSHEET_REFS}", x(a.1), y(a.0), true, effects(FONT_H, FONT_W, Some(just)))
            ));
        } else {
            items.push(sx!(
                "label",
                q(l.name.clone()),
                at(x(l.anchor.1), y(l.anchor.0), angle),
                effects(FONT_H, FONT_W, Some(just)),
                sx!("uuid", q(key(&format!("label/{:?}", l.anchor))))
            ));
        }
    }
    // Free text.
    for n in &sheet.notes {
        items.push(sx!(
            "text",
            q(n.text.clone()),
            sx!("exclude_from_sim", "no"),
            at(left(n.at.1), y(n.at.0) + NOTE_DY, 0.0),
            effects(FONT_H, FONT_W, Some("left")),
            sx!("uuid", q(key(&format!("note/{:?}", n.at))))
        ));
    }
    // Part instances.
    for (part, name) in sheet.parts.iter().zip(&names) {
        let (ox, oy) = (x(part.origin.1), y(part.origin.0));
        let mut v: Vec<S> = vec![
            sx!("lib_id", q(format!("{LIB}:{name}"))),
            at(ox, oy, 0.0),
            sx!("unit", 1.0),
            sx!("exclude_from_sim", "no"),
            sx!("in_bom", yes(part.meta.in_bom.unwrap_or(true))),
            sx!("on_board", yes(part.meta.on_board.unwrap_or(true))),
            sx!("dnp", yes(part.meta.dnp.unwrap_or(false))),
            sx!("fields_autoplaced", "no"),
            sx!("uuid", q(key(&format!("part/{}", part.reference)))),
        ];
        for f in &part.fields {
            match f.at {
                Some((r, c)) => v.push(prop(&f.name, &f.value, left(c), y(r) + FIELD_DY, false, effects(FONT_H, FONT_W, Some("left")))),
                None => v.push(prop(&f.name, &f.value, ox, oy, true, effects(FONT_H, FONT_W, None))),
            }
        }
        for pin in &part.pins {
            v.push(sx!("pin", q(pin.number.clone()), sx!("uuid", q(key(&format!("pin/{}/{}", part.reference, pin.number))))));
        }
        v.push(instances(ctx, &part.reference));
        items.push(crate::sexpr::list("symbol", v));
    }
    // Power ports, numbered in reading order.
    let mut ports: Vec<&Port> = sheet.ports.iter().collect();
    ports.sort_by_key(|p| (p.anchor.0, p.anchor.1));
    for (i, p) in ports.iter().enumerate() {
        let reference = format!("#PWR{:03}", ctx.pwr_start + i);
        let (ax, ay) = (x(p.anchor.1), y(p.anchor.0));
        let eff = || effects(FONT_H, FONT_W, None);
        items.push(sx!(
            "symbol",
            sx!("lib_id", q(format!("{LIB}:{}", power_name(p)))),
            at(ax, ay, 0.0),
            sx!("unit", 1.0),
            sx!("exclude_from_sim", "no"),
            sx!("in_bom", "yes"),
            sx!("on_board", "yes"),
            sx!("dnp", "no"),
            sx!("fields_autoplaced", "no"),
            sx!("uuid", q(key(&format!("pwr/{:?}", p.anchor)))),
            prop("Reference", &reference, ax, ay, true, eff()),
            prop("Value", &p.name, left(p.text_at.1), y(p.text_at.0) + FIELD_DY, false, effects(FONT_H, FONT_W, Some("left"))),
            prop("Footprint", "", ax, ay, true, eff()),
            prop("Datasheet", "", ax, ay, true, eff()),
            prop("Description", "", ax, ay, true, eff()),
            sx!("pin", q("1"), sx!("uuid", q(key(&format!("pwrpin/{:?}", p.anchor))))),
            instances(ctx, &reference)
        ));
    }
    items.extend(ctx.extra.iter().cloned());
    if ctx.is_root {
        items.push(sx!("sheet_instances", sx!("path", q("/"), sx!("page", q(ctx.page.clone())))));
    }
    items.push(sx!("embedded_fonts", "no"));
    (crate::sexpr::list("kicad_sch", items).to_text(), ports.len())
}

fn at2(x: f64, y: f64) -> S {
    sx!("at", x, y)
}

/// A sheet block on the root page for a hierarchy.
pub fn sheet_block(project: &str, root_uuid: &str, block_uuid: &str, name: &str, file: &str, page: usize, xy: (f64, f64), size: (f64, f64)) -> S {
    let (bx, by) = xy;
    let (w, h) = size;
    sx!(
        "sheet",
        at2(bx, by),
        sx!("size", w, h),
        sx!("exclude_from_sim", "no"),
        sx!("in_bom", "yes"),
        sx!("on_board", "yes"),
        sx!("dnp", "no"),
        sx!("fields_autoplaced", "yes"),
        sx!("stroke", sx!("width", 0.1524), sx!("type", "solid")),
        sx!("fill", sx!("color", 0.0, 0.0, 0.0, 0.0)),
        sx!("uuid", q(block_uuid)),
        prop("Sheetname", name, bx, by - 0.7116, false, effects(FONT_H, FONT_H, Some("left bottom"))),
        prop("Sheetfile", file, bx, by + h + 0.5884, false, effects(FONT_H, FONT_H, Some("left top"))),
        sx!(
            "instances",
            sx!("project", q(project), sx!("path", q(format!("/{root_uuid}")), sx!("page", q(page.to_string()))))
        )
    )
}

/// Project file settings this tool owns. An existing project file keeps
/// everything else (design rules, ERC exclusions set in the GUI).
pub fn project_json(existing: Option<&str>, file_name: &str) -> String {
    let mut v: serde_json::Value = existing
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let obj = v.as_object_mut().unwrap();
    let meta = obj.entry("meta").or_insert_with(|| serde_json::json!({}));
    meta["filename"] = serde_json::json!(file_name);
    meta["version"] = serde_json::json!(3);
    let erc = obj.entry("erc").or_insert_with(|| serde_json::json!({}));
    if !erc.get("rule_severities").is_some_and(|r| r.is_object()) {
        erc["rule_severities"] = serde_json::json!({});
    }
    // Rails from connectors, batteries or through passives have no
    // power_out pin, so this rule only produces noise.
    erc["rule_severities"]["power_pin_not_driven"] = serde_json::json!("ignore");
    let sch = obj.entry("schematic").or_insert_with(|| serde_json::json!({}));
    if !sch.get("drawing").is_some_and(|d| d.is_object()) {
        sch["drawing"] = serde_json::json!({});
    }
    // Fixed so that rendered positions match what `tsch verify` measured.
    sch["drawing"]["label_size_ratio"] = serde_json::json!(LABEL_SIZE_RATIO);
    sch["drawing"]["text_offset_ratio"] = serde_json::json!(TEXT_OFFSET_RATIO);
    serde_json::to_string_pretty(&v).unwrap() + "\n"
}

/// Margin of a global label's flag around its name, as a fraction of the
/// font width (KiCad sizes the flag from the width). 0.25 keeps the flags
/// of labels on adjacent rows 0.16 mm apart; the default 0.375 overlaps
/// them.
pub const LABEL_SIZE_RATIO: f64 = 0.25;
pub const TEXT_OFFSET_RATIO: f64 = 0.15;

/// For messages: "x, y" of a cell in mm.
#[allow(dead_code)]
pub fn mm(c: Cell) -> String {
    format!("{}, {}", fmt_num(x(c.1)), fmt_num(y(c.0)))
}
