//! Drawing parser and rule checker: turns the character grid into a sheet
//! model, reporting every rule it breaks.
//!
//! Order of recognition: boxes, two-pin glyphs, box pins and pin names, then
//! text phrases (labels, power ports, fields, notes), then wires. Each cell
//! belongs to at most one element; that is what keeps the KiCad drawing free
//! of overlaps.

use std::collections::{BTreeMap, HashMap};

use crate::diag::{Diag, Diags};
use crate::glyphs::Form;
use crate::meta::Meta;
use crate::model::*;
use crate::source::{Grid, Source};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Own {
    Free,
    /// Box outline.
    Edge(usize),
    /// Inside a box.
    Inside(usize),
    /// Glyph character.
    Glyph(usize),
    /// Blank cell a glyph needs beside it.
    Halo(usize),
    /// Pin number or pin padding of a box.
    PinText(usize),
    Phrase(usize),
    /// `x` on a pin tip.
    NoConnect,
}

#[derive(Clone, Debug)]
enum BodyKind {
    Box { t: i32, l: i32, b: i32, r: i32 },
    Glyph(Form),
}

#[derive(Clone, Debug)]
struct Body {
    kind: BodyKind,
    origin: Cell,
    bbox: (i32, i32, i32, i32),
    /// Box pins (glyph pins are built once the variant is known).
    pins: Vec<PinDef>,
    texts: Vec<SymText>,
}

#[derive(Clone, Debug)]
struct Phrase {
    row: i32,
    c0: i32,
    c1: i32,
    text: String,
    /// Wire cell touching the first character (text right of a wire end).
    left_wire: Option<Cell>,
    /// Wire cell touching the last character.
    right_wire: Option<Cell>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Att {
    Pin(usize, usize),
    Label(usize),
    Port(usize),
}

const WIRE_CHARS: &[u8] = b"-|+*";

fn has_alnum(s: &[u8]) -> bool {
    s.iter().any(|b| b.is_ascii_alphanumeric())
}

pub fn is_ground(name: &str) -> bool {
    let u = name.to_ascii_uppercase();
    u.contains("GND") || u.starts_with("VSS") || u == "0V" || u.contains("EARTH") || u.contains("CHASSIS")
}

/// Names a power port may carry without being declared in `power = [...]`.
pub fn is_power_name(name: &str, extra: &[String]) -> bool {
    if extra.iter().any(|e| e == name) || is_ground(name) {
        return true;
    }
    let u = name.to_ascii_uppercase();
    let bare = u.trim_start_matches(['+', '-']);
    if bare.chars().next().is_some_and(|c| c.is_ascii_digit()) && bare.contains('V') {
        return true;
    }
    const PREFIX: &[&str] = &[
        "VCC", "VDD", "VEE", "VBAT", "VBUS", "VSYS", "VIN", "VPP", "VREF", "VIO", "VMOT", "VM", "AVDD", "DVDD",
        "AVCC", "PVDD", "VCORE", "VLED", "VUSB", "VPWR", "VSUP",
    ];
    PREFIX.iter().any(|p| bare.starts_with(p))
}

fn is_reference(s: &str) -> bool {
    let b = s.as_bytes();
    let letters = b.iter().take_while(|c| c.is_ascii_uppercase() || **c == b'_').count();
    letters > 0 && letters < b.len() && b[letters..].iter().all(|c| c.is_ascii_digit())
}

struct Parser<'a> {
    src: &'a Source,
    g: &'a Grid,
    meta: &'a Meta,
    d: &'a mut Diags,
    own: Vec<Vec<Own>>,
    bodies: Vec<Body>,
    phrases: Vec<Phrase>,
    labels: Vec<Label>,
    ports: Vec<Port>,
    notes: Vec<Note>,
    no_connects: Vec<Cell>,
    att: HashMap<(Cell, Dir), Att>,
    /// Pin tips of glyphs, checked once the wire cells are known.
    glyph_tips: Vec<(usize, Cell, Dir)>,
    /// Field phrases attached to each body.
    attached: Vec<Vec<usize>>,
    /// Part index of each body (None when the body has no valid reference).
    body_part: Vec<Option<usize>>,
}

pub fn parse(src: &Source, meta: &Meta, name: &str, diags: &mut Diags) -> Sheet {
    let g = &src.grid;
    let own = vec![vec![Own::Free; g.width + 2]; g.height() + 2];
    let mut p = Parser {
        src,
        g,
        meta,
        d: diags,
        own,
        bodies: Vec::new(),
        phrases: Vec::new(),
        labels: Vec::new(),
        ports: Vec::new(),
        notes: Vec::new(),
        no_connects: Vec::new(),
        att: HashMap::new(),
        glyph_tips: Vec::new(),
        attached: Vec::new(),
        body_part: Vec::new(),
    };
    p.find_boxes();
    p.find_glyphs();
    for bi in 0..p.bodies.len() {
        if matches!(p.bodies[bi].kind, BodyKind::Box { .. }) {
            p.box_pins(bi);
        }
    }
    p.claim_glyph_nc();
    p.tokenize();
    let parts = p.assemble_parts();
    let (wires, junctions, nets) = p.wires_and_nets(&parts);
    Sheet {
        name: name.to_string(),
        path: src.path.clone(),
        meta: meta.clone(),
        parts,
        wires,
        junctions,
        no_connects: p.no_connects.clone(),
        labels: p.labels.clone(),
        ports: p.ports.clone(),
        notes: p.notes.clone(),
        nets,
        rows: g.height(),
        cols: g.width,
        row_ends: g.rows.iter().map(|r| r.iter().rposition(|&b| b != b' ').map(|i| i as i32).unwrap_or(-1)).collect(),
    }
}

impl<'a> Parser<'a> {
    fn ch(&self, c: Cell) -> u8 {
        self.g.get(c.0, c.1)
    }

    fn own(&self, c: Cell) -> Own {
        if c.0 < 0 || c.1 < 0 {
            return Own::Free;
        }
        self.own
            .get(c.0 as usize)
            .and_then(|r| r.get(c.1 as usize))
            .copied()
            .unwrap_or(Own::Free)
    }

    fn set(&mut self, c: Cell, o: Own) {
        if c.0 >= 0 && c.1 >= 0 && (c.0 as usize) < self.own.len() && (c.1 as usize) < self.own[0].len() {
            self.own[c.0 as usize][c.1 as usize] = o;
        }
    }

    fn free(&self, c: Cell) -> bool {
        self.own(c) == Own::Free
    }

    fn line(&self, c: Cell) -> usize {
        self.src.line_of(c.0)
    }

    fn err(&mut self, code: &'static str, c: Cell, msg: impl Into<String>) -> &mut Diag {
        let d = Diag::error(code, self.line(c), (c.1 + 1).max(1) as usize, msg);
        self.d.push(d);
        self.d.items.last_mut().unwrap()
    }

    fn warn(&mut self, code: &'static str, c: Cell, msg: impl Into<String>) -> &mut Diag {
        let d = Diag::warning(code, self.line(c), (c.1 + 1).max(1) as usize, msg);
        self.d.push(d);
        self.d.items.last_mut().unwrap()
    }

    fn pos(&self, c: Cell) -> String {
        format!("{}:{}", self.line(c), c.1 + 1)
    }

    fn body_desc(&self, bi: usize) -> String {
        let b = &self.bodies[bi];
        let what = match &b.kind {
            BodyKind::Box { .. } => "box",
            BodyKind::Glyph(f) => match f.kind() {
                GlyphKind::Resistor => "resistor",
                GlyphKind::Capacitor => "capacitor",
                GlyphKind::CapPolarized => "polarized capacitor",
                GlyphKind::Inductor => "inductor",
                GlyphKind::Diode => "diode",
            },
        };
        format!("the {what} at {}", self.pos((b.bbox.0, b.bbox.1)))
    }

    // ---------------------------------------------------------------- boxes

    fn find_boxes(&mut self) {
        let h = self.g.height() as i32;
        let w = self.g.width as i32;
        for r in 0..h {
            for c in 0..w {
                if self.ch((r, c)) != b'+' || !self.free((r, c)) {
                    continue;
                }
                let mut c2 = c + 1;
                while self.ch((r, c2)) == b'-' {
                    c2 += 1;
                }
                if self.ch((r, c2)) != b'+' || c2 < c + 2 {
                    continue;
                }
                let mut r2 = r + 1;
                while self.ch((r2, c)) == b'|' {
                    r2 += 1;
                }
                let closed = self.ch((r2, c)) == b'+'
                    && r2 >= r + 2
                    && (r + 1..r2).all(|rr| self.ch((rr, c2)) == b'|')
                    && self.ch((r2, c2)) == b'+'
                    && (c + 1..c2).all(|cc| self.ch((r2, cc)) == b'-');
                if !closed {
                    self.broken_box(r, c, c2);
                    continue;
                }
                if (r..=r2).any(|rr| (c..=c2).any(|cc| !self.free((rr, cc)))) {
                    self.err("E101", (r, c), "box overlaps another box").note("boxes may not nest or touch");
                    continue;
                }
                let bi = self.bodies.len();
                for rr in r..=r2 {
                    for cc in c..=c2 {
                        let edge = rr == r || rr == r2 || cc == c || cc == c2;
                        self.set((rr, cc), if edge { Own::Edge(bi) } else { Own::Inside(bi) });
                    }
                }
                self.bodies.push(Body {
                    kind: BodyKind::Box { t: r, l: c, b: r2, r: c2 },
                    origin: (r, c),
                    bbox: (r, c, r2, c2),
                    pins: Vec::new(),
                    texts: Vec::new(),
                });
            }
        }
    }

    /// A top edge `+---+` with a matching bottom edge further down whose
    /// sides do not line up: report the first cell of the broken side,
    /// instead of the many wire and text errors the loose pieces cause.
    fn broken_box(&mut self, r: i32, c: i32, c2: i32) {
        let h = self.g.height() as i32;
        let bottom = (r + 2..h.min(r + 80)).find(|&rr| {
            self.ch((rr, c)) == b'+' && self.ch((rr, c2)) == b'+' && (c + 1..c2).all(|cc| self.ch((rr, cc)) == b'-')
        });
        let Some(r2) = bottom else { return };
        for rr in r + 1..r2 {
            for cc in [c, c2] {
                if self.ch((rr, cc)) != b'|' {
                    let side = if cc == c { "left" } else { "right" };
                    let got = self.ch((rr, cc)) as char;
                    let hint = format!("the corners are at {} and {}; every row between them needs '|' in both corner columns", self.pos((r, c)), self.pos((r2, c2)));
                    self.err("E102", (rr, cc), format!("box outline broken: expected the {side} side '|' here, found '{got}'")).note(hint);
                    return;
                }
            }
        }
    }

    /// A run of pin-number characters going outward from `start` in `dir`.
    /// Returns the cells in reading order (left to right).
    fn number_run(&self, start: Cell, dir: Dir) -> Vec<Cell> {
        let mut cells = Vec::new();
        let mut c = start;
        loop {
            let ch = self.ch(c);
            if ch == b' ' || WIRE_CHARS.contains(&ch) || !self.free(c) {
                break;
            }
            cells.push(c);
            c = dir.step(c);
        }
        if dir == Dir::L {
            cells.reverse();
        }
        cells
    }

    fn text_of(&self, cells: &[Cell]) -> String {
        cells.iter().map(|&c| self.ch(c) as char).collect()
    }

    fn box_pins(&mut self, bi: usize) {
        let BodyKind::Box { t, l, b, r } = self.bodies[bi].kind else { return };
        // Left and right pins: (side, row, number cells, no-connect cell).
        let mut side_pins: Vec<(Dir, i32, Vec<Cell>, Option<Cell>)> = Vec::new();
        for rr in t + 1..b {
            for (dir, edge) in [(Dir::L, l), (Dir::R, r)] {
                let first = dir.step((rr, edge));
                let mut cells = self.number_run(first, dir);
                if cells.is_empty() {
                    let ch = self.ch(first);
                    if ch != b' ' && self.free(first) {
                        self.err("E110", first, format!("'{}' touches the {} edge of the box without a pin number", ch as char, dir.name().replace("above", "top")))
                            .note("a pin is its number written against the edge, e.g. --3| ; a wire may not touch a box elsewhere");
                    }
                    continue;
                }
                // An x at the outer end, with nothing beyond it, marks the pin unused.
                let outer = if dir == Dir::L { cells[0] } else { *cells.last().unwrap() };
                let mut nc = None;
                if cells.len() >= 2 && self.ch(outer) == b'x' && self.ch(dir.step(outer)) == b' ' {
                    nc = Some(outer);
                    cells.retain(|&c| c != outer);
                }
                side_pins.push((dir, rr, cells, nc));
            }
        }
        let max_len = side_pins.iter().map(|p| p.2.len() as i32).max().unwrap_or(0);
        let plen = max_len + 1;
        for (dir, rr, cells, nc) in side_pins {
            let edge = if dir == Dir::L { l } else { r };
            let number = self.text_of(&cells);
            let n = cells.len() as i32;
            let tip = (rr, if dir == Dir::L { edge - plen } else { edge + plen });
            // One error per pin, the most specific first: a misplaced x,
            // then missing padding, then an unconnected tip.
            let mut ok = true;
            if let Some(x) = nc {
                if x != tip {
                    let eg = if dir == Dir::L { format!("x{}{number}|", "-".repeat((plen - n - 1) as usize)) } else { format!("|{number}{}x", "-".repeat((plen - n - 1) as usize)) };
                    self.err("E112", x, format!("pin {number}: the x must sit on the pin tip, {plen} cells from the box edge (pins of this box are its longest number + 1 long)"))
                        .note(format!("pad with '-' between the number and the x: {eg}"));
                    ok = false;
                }
            }
            if ok {
                for k in n + 1..plen {
                    let c = (rr, if dir == Dir::L { edge - k } else { edge + k });
                    if self.ch(c) != b'-' || !self.free(c) {
                        let pad = "-".repeat((plen - n - 1) as usize);
                        let eg = if dir == Dir::L { format!("-{pad}{number}|") } else { format!("|{number}{pad}-") };
                        self.err("E111", c, format!("pin {number}: expected '-' here; pins of this box are {plen} cells long (longest number + 1)"))
                            .note(format!("pad short pin numbers with '-' toward the tip: {eg}"));
                        ok = false;
                        break;
                    }
                }
            }
            for &c in &cells {
                self.set(c, Own::PinText(bi));
            }
            for k in n + 1..plen {
                let c = (rr, if dir == Dir::L { edge - k } else { edge + k });
                if self.free(c) && self.ch(c) == b'-' {
                    self.set(c, Own::PinText(bi));
                }
            }
            let tch = self.ch(tip);
            if ok && !(tch == b'x' || (WIRE_CHARS.contains(&tch) && tch != b'|')) {
                self.err("E113", tip, format!("pin {number} is not connected: its tip ({plen} cells from the edge) holds '{}'", tch as char))
                    .note("continue a wire from the tip, or put x on the tip to mark the pin unused");
                ok = false;
            }
            if !ok {
                // Keep the pin on the part but leave its tip alone, so the
                // wire checks do not report the same mistake again.
                if let Some(x) = nc {
                    self.set(x, Own::PinText(bi));
                }
                self.bodies[bi].pins.push(PinDef {
                    number: number.clone(),
                    name: String::new(),
                    tip,
                    dir,
                    length: plen as f64 * CELL_W,
                    etype: String::new(),
                });
                self.bodies[bi].texts.push(SymText { text: number, row: rr, col: cells[0].1, kind: TextKind::PinNumberRaised });
                continue;
            }
            let pin_idx = self.bodies[bi].pins.len();
            self.bodies[bi].pins.push(PinDef {
                number: number.clone(),
                name: String::new(),
                tip,
                dir,
                length: plen as f64 * CELL_W,
                etype: String::new(),
            });
            self.bodies[bi].texts.push(SymText { text: number, row: rr, col: cells[0].1, kind: TextKind::PinNumberRaised });
            self.attach_tip(bi, pin_idx, tip, dir);
        }
        // Top and bottom pins.
        for cc in l + 1..r {
            for (dir, row) in [(Dir::U, t - 1), (Dir::D, b + 1)] {
                let tip = (row, cc);
                let tch = self.ch(tip);
                if !(tch == b'|' || tch == b'x') || !self.free(tip) {
                    continue;
                }
                let cells = self.number_run((row, cc - 1), Dir::L);
                if cells.is_empty() {
                    self.err("E114", tip, format!("'{}' meets the {} edge of the box but has no pin number", tch as char, if dir == Dir::U { "top" } else { "bottom" }))
                        .note("write the pin number immediately left of the pin, e.g. 5|");
                    continue;
                }
                let number = self.text_of(&cells);
                for &c in &cells {
                    self.set(c, Own::PinText(bi));
                }
                let pin_idx = self.bodies[bi].pins.len();
                self.bodies[bi].pins.push(PinDef {
                    number: number.clone(),
                    name: String::new(),
                    tip,
                    dir,
                    length: CELL_H,
                    etype: String::new(),
                });
                self.bodies[bi].texts.push(SymText { text: number, row, col: cells[0].1, kind: TextKind::PinNumberSide });
                self.attach_tip(bi, pin_idx, tip, dir);
            }
        }
        self.box_interior(bi, t, l, b, r);
        // Duplicate pin numbers.
        let mut seen: BTreeMap<String, Cell> = BTreeMap::new();
        let pins = self.bodies[bi].pins.clone();
        for p in &pins {
            if let Some(first) = seen.insert(p.number.clone(), p.tip) {
                let fp = self.pos(first);
                self.err("E115", p.tip, format!("pin number {} is used twice in this box (also at {fp})", p.number));
            }
        }
    }

    /// Registers a pin tip: an `x` there becomes a no-connect, otherwise the
    /// wire cell at the tip gets the pin as a connection on the body side.
    fn attach_tip(&mut self, bi: usize, pin: usize, tip: Cell, dir: Dir) {
        if self.ch(tip) == b'x' && matches!(self.own(tip), Own::Free | Own::NoConnect) {
            if self.own(tip) == Own::NoConnect {
                self.err("E116", tip, "two pins end on the same x");
            }
            self.set(tip, Own::NoConnect);
            self.no_connects.push(tip);
        }
        if let Some(prev) = self.att.insert((tip, dir.opposite()), Att::Pin(bi, pin)) {
            let _ = prev;
            self.err("E117", tip, "two pins end on the same cell from the same side");
        }
    }

    /// Phrases inside a box: pin names against the edges or over top and
    /// bottom pins, anything else is drawn as text inside the symbol.
    fn box_interior(&mut self, bi: usize, t: i32, l: i32, b: i32, r: i32) {
        for rr in t + 1..b {
            let words = self.words_in((rr, l + 1), r - 1, |_| true);
            for (c0, c1, text) in merge_words(words) {
                let touches_l = c0 == l + 1;
                let touches_r = c1 == r - 1;
                let pins = &self.bodies[bi].pins;
                let left_pin = pins.iter().position(|p| p.dir == Dir::L && p.tip.0 == rr);
                let right_pin = pins.iter().position(|p| p.dir == Dir::R && p.tip.0 == rr);
                let vert: Vec<usize> = pins
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| {
                        ((p.dir == Dir::U && rr == t + 1) || (p.dir == Dir::D && rr == b - 1)) && p.tip.1 >= c0 && p.tip.1 <= c1
                    })
                    .map(|(i, _)| i)
                    .collect();
                let mut owner: Option<usize> = None;
                if touches_l && touches_r {
                    self.err("E120", (rr, c0), format!("'{text}' fills the whole row of the box"))
                        .note("leave at least one space between a left pin name and a right pin name");
                    continue;
                }
                if touches_l {
                    match left_pin {
                        Some(i) => owner = Some(i),
                        None => {
                            self.err("E121", (rr, c0), format!("'{text}' touches the left edge but this row has no left pin"))
                                .note("indent text that is not a pin name by one space");
                            continue;
                        }
                    }
                } else if touches_r {
                    match right_pin {
                        Some(i) => owner = Some(i),
                        None => {
                            self.err("E121", (rr, c1), format!("'{text}' touches the right edge but this row has no right pin"))
                                .note("leave one space between text and the edge when it is not a pin name");
                            continue;
                        }
                    }
                }
                if !vert.is_empty() {
                    if owner.is_some() || vert.len() > 1 {
                        self.err("E122", (rr, c0), format!("'{text}' is over more than one pin"))
                            .note("a top or bottom pin's name sits in the row inside the edge, covering only that pin's column");
                        continue;
                    }
                    owner = Some(vert[0]);
                }
                match owner {
                    Some(i) => {
                        if !self.bodies[bi].pins[i].name.is_empty() {
                            self.err("E123", (rr, c0), format!("pin {} already has a name", self.bodies[bi].pins[i].number));
                        }
                        self.bodies[bi].pins[i].name = text.clone();
                    }
                    None => {}
                }
                self.bodies[bi].texts.push(SymText { text, row: rr, col: c0, kind: TextKind::Normal });
            }
        }
    }

    /// Runs of non-space characters on one row from `start` to column
    /// `end` inclusive, limited to cells for which `keep` holds.
    fn words_in(&self, start: Cell, end: i32, keep: impl Fn(Cell) -> bool) -> Vec<(i32, i32, String)> {
        let mut out = Vec::new();
        let mut c = start.1;
        let r = start.0;
        while c <= end {
            if self.ch((r, c)) != b' ' && keep((r, c)) {
                let c0 = c;
                let mut s = String::new();
                while c <= end && self.ch((r, c)) != b' ' && keep((r, c)) {
                    s.push(self.ch((r, c)) as char);
                    c += 1;
                }
                out.push((c0, c - 1, s));
            } else {
                c += 1;
            }
        }
        out
    }

    // --------------------------------------------------------------- glyphs

    fn tip_ok(&self, c: Cell, vertical: bool) -> bool {
        let ch = self.ch(c);
        self.free(c) && (ch == b'x' || ch == b'+' || ch == b'*' || ch == if vertical { b'|' } else { b'-' })
    }

    fn find_glyphs(&mut self) {
        let h = self.g.height() as i32;
        let w = self.g.width as i32;
        // Row forms.
        for r in 0..h {
            let mut c = 0;
            while c < w {
                if !self.free((r, c)) {
                    c += 1;
                    continue;
                }
                let at = |k: i32| self.ch((r, c + k));
                let form = match at(0) {
                    b'[' => {
                        let n = (1..).take_while(|&k| at(k) == b'=').count() as i32;
                        if (1..=4).contains(&n) && at(n + 1) == b']' { Some(Form::RH(n)) } else { None }
                    }
                    b'+' if at(1) == b'|' && at(2) == b'|' => Some(Form::CPH { plus_left: true }),
                    b'|' if at(1) == b'|' && at(2) == b'+' => Some(Form::CPH { plus_left: false }),
                    b'|' if at(1) == b'|' => Some(Form::CH),
                    b'|' if at(1) == b'<' => Some(Form::DH { anode_left: false }),
                    b'>' if at(1) == b'|' => Some(Form::DH { anode_left: true }),
                    b'~' => {
                        let n = (0..).take_while(|&k| at(k) == b'~').count() as i32;
                        if (2..=8).contains(&n) { Some(Form::LH(n)) } else { None }
                    }
                    b'=' if at(1) == b'=' && at(2) == b'=' && at(3) != b'=' && self.ch((r, c - 1)) != b'=' => {
                        let m = (r, c + 1);
                        let up = (r - 1, c + 1);
                        let dn = (r + 1, c + 1);
                        let marker = |p: Cell| self.ch(p) == b'+' && self.ch((p.0, p.1 - 1)) == b' ' && self.ch((p.0, p.1 + 1)) == b' ';
                        let _ = m;
                        match (marker(up), marker(dn)) {
                            (true, true) => {
                                self.err("E130", up, "capacitor with + on both sides");
                                None
                            }
                            (true, false) => Some(Form::CPV { plus_top: true }),
                            (false, true) => Some(Form::CPV { plus_top: false }),
                            _ => Some(Form::CV),
                        }
                    }
                    _ => None,
                };
                let Some(form) = form else {
                    c += 1;
                    continue;
                };
                let origin = match form {
                    Form::CV | Form::CPV { .. } => (r, c + 1),
                    _ => (r, c),
                };
                if self.try_glyph(form, origin) {
                    let width = form.cells(origin).iter().filter(|x| x.0 == r).count() as i32;
                    c += width;
                } else {
                    c += 1;
                }
            }
        }
        // Column forms.
        for c in 0..w {
            let mut r = 0;
            while r < h {
                if !self.free((r, c)) {
                    r += 1;
                    continue;
                }
                let ch = self.ch((r, c));
                let run = |want: u8| (0..).take_while(|&k| self.ch((r + k, c)) == want && self.free((r + k, c))).count() as i32;
                let form = match ch {
                    b'#' => {
                        let n = run(b'#');
                        if n <= 4 { Some(Form::RV(n)) } else { None }
                    }
                    b')' | b'(' => {
                        let n = run(ch);
                        if n <= 4 { Some(Form::LV { n, right: ch == b')' }) } else { None }
                    }
                    b'V' | b'^' => Some(Form::DV { anode_top: ch == b'V' }),
                    _ => None,
                };
                match form {
                    Some(f) if self.try_glyph(f, (r, c)) => r += f.cells((r, c)).len() as i32,
                    _ => r += 1,
                }
            }
        }
    }

    /// Accepts a glyph when at least one of its ends is a wire cell or x.
    fn try_glyph(&mut self, form: Form, origin: Cell) -> bool {
        let cells = form.cells(origin);
        if cells.iter().any(|&c| !self.free(c)) {
            return false;
        }
        let tips = form.tips(origin);
        let v = form.vertical();
        if !tips.iter().any(|(t, _)| self.tip_ok(*t, v)) {
            return false;
        }
        // A lone V or ^ next to other characters is text, not a diode.
        if let Form::DV { .. } = form {
            let (r, c) = origin;
            if self.ch((r, c - 1)) != b' ' || self.ch((r, c + 1)) != b' ' {
                if !(self.tip_ok(tips[0].0, true) && self.tip_ok(tips[1].0, true)) {
                    return false;
                }
            }
        }
        let bi = self.bodies.len();
        for &c in &cells {
            self.set(c, Own::Glyph(bi));
        }
        for h in form.halo(origin) {
            if self.ch(h) != b' ' {
                self.err("E131", h, format!("'{}' is too close to a vertical part body", self.ch(h) as char))
                    .note("keep the cell on each side of a vertical resistor, inductor, diode or + marker blank");
            } else {
                self.set(h, Own::Halo(bi));
            }
        }
        let rows: Vec<i32> = cells.iter().map(|c| c.0).collect();
        let cols: Vec<i32> = cells.iter().map(|c| c.1).collect();
        let bbox = (*rows.iter().min().unwrap(), *cols.iter().min().unwrap(), *rows.iter().max().unwrap(), *cols.iter().max().unwrap());
        self.bodies.push(Body { kind: BodyKind::Glyph(form), origin, bbox, pins: Vec::new(), texts: Vec::new() });
        for (t, d) in tips {
            self.glyph_tips.push((bi, t, d));
            if self.att.insert((t, d.opposite()), Att::Pin(bi, 0)).is_some() {
                self.err("E117", t, "two pins end on the same cell from the same side");
            }
        }
        true
    }

    fn claim_glyph_nc(&mut self) {
        let tips = self.glyph_tips.clone();
        for (bi, t, d) in tips {
            if self.ch(t) == b'x' && matches!(self.own(t), Own::Free | Own::NoConnect) {
                if self.own(t) == Own::NoConnect {
                    self.err("E116", t, "two pins end on the same x");
                }
                self.set(t, Own::NoConnect);
                self.no_connects.push(t);
            }
            let _ = (bi, d);
        }
    }

    // ----------------------------------------------------------------- text

    fn tokenize(&mut self) {
        let h = self.g.height() as i32;
        let w = self.g.width as i32;
        for r in 0..h {
            // Runs of free, non-space, non-'|' cells.
            let mut words: Vec<(i32, i32, String, Option<Cell>, Option<Cell>)> = Vec::new();
            let mut c = 0;
            while c < w {
                let ok = |p: &Parser, cc: i32| {
                    let ch = p.ch((r, cc));
                    ch != b' ' && ch != b'|' && p.free((r, cc))
                };
                if !ok(self, c) {
                    c += 1;
                    continue;
                }
                let a = c;
                while c < w && ok(self, c) {
                    c += 1;
                }
                let b = c - 1;
                let s: Vec<u8> = (a..=b).map(|k| self.ch((r, k))).collect();
                if !has_alnum(&s) {
                    continue; // all wire characters, checked later
                }
                let is_w = |x: u8| x == b'-' || x == b'+' || x == b'*';
                let n = s.len();
                let mut wire = vec![false; n];
                let mut i = 0;
                while i < n {
                    if !is_w(s[i]) {
                        i += 1;
                        continue;
                    }
                    let i0 = i;
                    while i < n && is_w(s[i]) {
                        i += 1;
                    }
                    let i1 = i - 1;
                    if i0 == 0 && i1 == n - 1 {
                        continue; // cannot happen: the run has a letter or digit
                    }
                    if i0 == 0 {
                        // Prefix: leading wire characters ending in '-'. A lone
                        // minus with nothing before it is text ("-5V").
                        let mut k = i1 + 1;
                        while k > 0 && s[k - 1] != b'-' {
                            k -= 1;
                        }
                        if !(k == 1 && self.ch((r, a - 1)) == b' ') {
                            wire[..k].iter_mut().for_each(|x| *x = true);
                        }
                    } else if i1 == n - 1 {
                        // Suffix: trailing wire characters starting with '-'.
                        // A lone trailing minus with nothing after it is text ("D-").
                        let mut j = i0;
                        while j < n && s[j] != b'-' {
                            j += 1;
                        }
                        if !(n - j == 1 && self.ch((r, b + 1)) == b' ') {
                            wire[j..].iter_mut().for_each(|x| *x = true);
                        }
                    } else {
                        // Inside a run: two or more dashes, or a dash against a
                        // global label bracket, is a wire between two texts.
                        let dashes = s[i0] == b'-' && s[i1] == b'-';
                        let bracket = s[i0 - 1] == b']' || s[i1 + 1] == b'[';
                        if dashes && (i1 > i0 || bracket) {
                            wire[i0..=i1].iter_mut().for_each(|x| *x = true);
                        }
                    }
                }
                let mut i = 0;
                while i < n {
                    if wire[i] {
                        i += 1;
                        continue;
                    }
                    let i0 = i;
                    while i < n && !wire[i] {
                        i += 1;
                    }
                    let i1 = i - 1;
                    let left_wire = if i0 > 0 { Some((r, a + i0 as i32 - 1)) } else { None };
                    let right_wire = if i1 + 1 < n { Some((r, a + i1 as i32 + 1)) } else { None };
                    let text = String::from_utf8_lossy(&s[i0..=i1]).to_string();
                    if !has_alnum(text.as_bytes()) {
                        // Punctuation between wires: left for the wire checks.
                        continue;
                    }
                    words.push((a + i0 as i32, a + i1 as i32, text, left_wire, right_wire));
                }
            }
            // Join words separated by a single space into one phrase.
            let mut i = 0;
            while i < words.len() {
                let (c0, mut c1, mut text, lw, mut rw) = words[i].clone();
                let mut j = i + 1;
                while j < words.len() && rw.is_none() && words[j].3.is_none() && words[j].0 == c1 + 2 {
                    text.push(' ');
                    text.push_str(&words[j].2);
                    c1 = words[j].1;
                    rw = words[j].4;
                    j += 1;
                }
                let pi = self.phrases.len();
                for cc in c0..=c1 {
                    self.set((r, cc), Own::Phrase(pi));
                }
                self.phrases.push(Phrase { row: r, c0, c1, text, left_wire: lw, right_wire: rw });
                i = j;
            }
        }
        self.classify_phrases();
    }

    fn wire_cell(&self, c: Cell) -> bool {
        self.free(c) && WIRE_CHARS.contains(&self.ch(c))
    }

    fn classify_phrases(&mut self) {
        let mut candidates: Vec<usize> = Vec::new();
        for pi in 0..self.phrases.len() {
            let p = self.phrases[pi].clone();
            let start = (p.row, p.c0);
            // Text may touch a wire only as a label; anything else needs a space.
            for (side, nb, wired) in [(Dir::L, (p.row, p.c0 - 1), p.left_wire.is_some()), (Dir::R, (p.row, p.c1 + 1), p.right_wire.is_some())] {
                if wired {
                    continue;
                }
                let ch = self.ch(nb);
                if ch != b' ' {
                    let what = match self.own(nb) {
                        Own::Glyph(b) | Own::Edge(b) | Own::Halo(b) | Own::PinText(b) | Own::Inside(b) => self.body_desc(b),
                        _ => format!("'{}'", ch as char),
                    };
                    self.err("E140", if side == Dir::L { start } else { (p.row, p.c1) }, format!("'{}' touches {what}", p.text))
                        .note("leave a space between text and drawing; only labels touch the end of a wire");
                }
            }
            if p.left_wire.is_some() && p.right_wire.is_some() {
                self.err("E141", start, format!("'{}' sits between two wires", p.text))
                    .note("a label goes at the end of a wire; a wire cannot run through text");
                continue;
            }
            if let Some(w) = p.left_wire.or(p.right_wire) {
                self.make_label(pi, w);
                continue;
            }
            // Power port: the name directly above or below a vertical wire end.
            let mut ends: Vec<(Cell, Dir)> = Vec::new();
            for cc in p.c0..=p.c1 {
                for (dr, side) in [(1, Dir::U), (-1, Dir::D)] {
                    let c = (p.row + dr, cc);
                    if self.wire_cell(c) && self.ch(c) != b'-' {
                        ends.push((c, side));
                    }
                }
            }
            if ends.len() > 1 {
                let list: Vec<String> = ends.iter().map(|(c, _)| self.pos(*c)).collect();
                self.err("E142", start, format!("'{}' touches {} wire ends ({})", p.text, ends.len(), list.join(", ")))
                    .note("a power port name sits on exactly one vertical wire end");
                continue;
            }
            if let Some(&(anchor, side)) = ends.first() {
                // A bad name is reported once; the port still connects so the
                // wire end is not reported too.
                if !is_power_name(&p.text, &self.meta.power) {
                    self.err("E143", start, format!("'{}' sits on the end of a vertical wire, which makes it a power port, but it is not a rail name", p.text))
                        .note("power ports are rails (GND, +3V3, VCC...) or names listed in power = [...]; give a signal a label at a horizontal wire end, and keep fields off wire ends");
                } else if p.text.contains(' ') {
                    self.err("E144", start, format!("power port name '{}' contains a space", p.text));
                }
                let idx = self.ports.len();
                self.ports.push(Port {
                    name: p.text.clone(),
                    anchor,
                    side,
                    ground: is_ground(&p.text),
                    text_at: start,
                });
                if self.att.insert((anchor, side), Att::Port(idx)).is_some() {
                    self.err("E145", anchor, "two things connect to this wire end from the same side");
                }
                continue;
            }
            candidates.push(pi);
        }
        self.attach_fields(candidates);
    }

    fn make_label(&mut self, pi: usize, wire: Cell) {
        let p = self.phrases[pi].clone();
        let start = (p.row, p.c0);
        let side = if p.left_wire.is_some() { Dir::R } else { Dir::L };
        let (name, global) = if p.text.len() > 2 && p.text.starts_with('[') && p.text.ends_with(']') {
            (p.text[1..p.text.len() - 1].to_string(), true)
        } else {
            (p.text.clone(), false)
        };
        if name.contains(' ') || name.contains('[') || name.contains(']') || name.is_empty() {
            self.err("E146", start, format!("label '{}' is not a valid net name", p.text))
                .note("net names have no spaces (two words one space apart are one text); a global label is written [NAME] against the wire end");
            // Still connect it, so the wire is not reported a second time.
        }
        if !global && is_power_name(&name, &[]) && is_ground(&name) {
            self.warn("W147", start, format!("label '{name}' looks like a ground net"))
                .note("connect ground with a power port: the name directly below a vertical wire end");
        }
        let idx = self.labels.len();
        self.labels.push(Label { name, anchor: wire, side, global, text_at: start });
        if self.att.insert((wire, side), Att::Label(idx)).is_some() {
            self.err("E145", wire, "two things connect to this wire end from the same side");
        }
    }

    // --------------------------------------------------------------- fields

    /// Fields: each text line goes to the part it sits beside (sharing a
    /// row, at most 2 blank columns away) or directly above or below
    /// (sharing a column, in the adjacent row). Lines that reach no part
    /// join a left-aligned stack whose neighbor line already belongs to one.
    fn attach_fields(&mut self, cands: Vec<usize>) {
        let gap = |a0: i32, a1: i32, b0: i32, b1: i32| -> i32 {
            if a1 < b0 {
                b0 - a1 - 1
            } else if b1 < a0 {
                a0 - b1 - 1
            } else {
                -1
            }
        };
        let mut owner: BTreeMap<usize, usize> = BTreeMap::new();
        let mut orphans: Vec<usize> = Vec::new();
        for &pi in &cands {
            let p = self.phrases[pi].clone();
            // Strength 2: touching the body's own rows or columns; 1: only
            // beside a vertical glyph's pin stub rows. The strongest wins.
            let strength = |bi: usize| -> u8 {
                let (t, l, b, r) = self.bodies[bi].bbox;
                let gr = gap(p.row, p.row, t, b);
                let gc = gap(p.c0, p.c1, l, r);
                if (gr < 0 && gc <= 2) || (gc < 0 && gr == 0) {
                    return 2;
                }
                if let BodyKind::Glyph(f) = self.bodies[bi].kind {
                    if f.vertical() && gap(p.row, p.row, t - 1, b + 1) < 0 && gc <= 2 {
                        return 1;
                    }
                }
                0
            };
            let scored: Vec<(usize, u8)> = (0..self.bodies.len()).map(|bi| (bi, strength(bi))).filter(|x| x.1 > 0).collect();
            let best = scored.iter().map(|x| x.1).max().unwrap_or(0);
            let near: Vec<usize> = scored.iter().filter(|x| x.1 == best).map(|x| x.0).collect();
            let mut cands_b = near.clone();
            // A reference goes to a part that takes its letter; a glyph of
            // that kind wins over a box.
            if cands_b.len() > 1 && is_reference(&p.text) {
                cands_b.retain(|&bi| self.accepts(bi, &p.text));
                if cands_b.iter().any(|&bi| matches!(self.bodies[bi].kind, BodyKind::Glyph(_))) {
                    cands_b.retain(|&bi| matches!(self.bodies[bi].kind, BodyKind::Glyph(_)));
                }
            }
            let cell = (p.row, p.c0);
            match cands_b.len() {
                0 if near.is_empty() => orphans.push(pi),
                0 => {
                    let names: Vec<String> = near.iter().map(|&b| self.body_desc(b)).collect();
                    self.err("E150", cell, format!("reference '{}' is next to {}, which takes a different letter", p.text, names.join(" and ")))
                        .note("resistor glyphs take R/F/TH, capacitors C, inductors L/FB, diodes D/LED/TVS/ZD; boxes take any reference");
                }
                1 => {
                    owner.insert(pi, cands_b[0]);
                }
                _ => {
                    let names: Vec<String> = cands_b.iter().map(|&b| self.body_desc(b)).collect();
                    self.err("E151", cell, format!("text '{}' is next to more than one part: {}", p.text, names.join(" and ")))
                        .note("a field shares a row with its part (at most 2 blank columns away) or sits in the row directly above or below it, over its columns; move it clear of the other part");
                }
            }
        }
        // A value line aligned directly under another part's field line
        // reads as that part's; refuse to guess.
        let owned: Vec<(usize, usize)> = owner.iter().map(|(&p, &b)| (p, b)).collect();
        for &(pi, bi) in &owned {
            let p = self.phrases[pi].clone();
            if is_reference(&p.text) {
                continue;
            }
            let above = owned.iter().find(|(qi, qb)| {
                let q = &self.phrases[*qi];
                q.row == p.row - 1 && q.c0 == p.c0 && *qb != bi
            });
            if let Some(&(qi, qb)) = above {
                let qtext = self.phrases[qi].text.clone();
                let (mine, theirs) = (self.body_desc(bi), self.body_desc(qb));
                self.err("E156", (p.row, p.c0), format!("'{}' is beside {mine} but sits under '{qtext}', a field of {theirs}", p.text))
                    .note("move it so it is next to only one part, or out of line with the other part's fields");
                owner.remove(&pi);
            }
        }
        // Stacks: an orphan directly above or below an owned line with the
        // same first column joins that line's part.
        loop {
            let mut changed = false;
            for &pi in &orphans.clone() {
                let p = self.phrases[pi].clone();
                let mut found: Vec<usize> = owner
                    .iter()
                    .filter(|(q, _)| {
                        let q = &self.phrases[**q];
                        (q.row - p.row).abs() == 1 && q.c0 == p.c0
                    })
                    .map(|(_, b)| *b)
                    .collect();
                found.sort();
                found.dedup();
                if found.len() == 1 {
                    owner.insert(pi, found[0]);
                    orphans.retain(|&o| o != pi);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        for &pi in &orphans {
            let p = self.phrases[pi].clone();
            if is_reference(&p.text) {
                self.warn("W150", (p.row, p.c0), format!("'{}' looks like a reference but is not next to any part", p.text))
                    .note("a field shares a row with its part (at most 2 blank columns away), sits in the row directly above or below it over its columns, or stacks left-aligned on such a line");
            }
            self.notes.push(Note { text: p.text.clone(), at: (p.row, p.c0) });
        }
        let mut attached: Vec<Vec<usize>> = vec![Vec::new(); self.bodies.len()];
        for (pi, bi) in owner {
            attached[bi].push(pi);
        }
        self.attached = attached;
    }
}

impl<'a> Parser<'a> {
    /// Hint for a body without fields: where they go, and the closest
    /// stray reference if one is nearby.
    fn nearest_ref_hint(&self, bbox: (i32, i32, i32, i32)) -> String {
        let (t, l, b, r) = bbox;
        let mut best: Option<(i32, &Note)> = None;
        for n in &self.notes {
            if !is_reference(&n.text) {
                continue;
            }
            let (nr, nc) = n.at;
            let dr = if nr < t { t - nr - 1 } else if nr > b { nr - b - 1 } else { 0 };
            let end = nc + n.text.len() as i32 - 1;
            let dc = if end < l { l - end - 1 } else if nc > r { nc - r - 1 } else { 0 };
            let d = dr.max(dc);
            if d <= 24 && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, n));
            }
        }
        let base = "write the reference (R1, U3...) in a row of the body or the row just above or below it, at most 2 blank columns away: above a box's top-left corner, above a horizontal part, right of a vertical one";
        match best {
            Some((_, n)) => {
                let (nr, nc) = n.at;
                let dr = if nr < t { t - nr - 1 } else if nr > b { nr - b - 1 } else { 0 };
                let end = nc + n.text.len() as i32 - 1;
                let dc = if end < l { l - end - 1 } else if nc > r { nc - r - 1 } else { 0 };
                format!("'{}' at {} is {dr} blank row(s) and {dc} blank column(s) away; {base}", n.text, self.pos(n.at))
            }
            None => base.to_string(),
        }
    }

    /// Whether a body may carry this reference: boxes take any, glyphs only
    /// the letters of their kind.
    fn accepts(&self, bi: usize, reference: &str) -> bool {
        let prefix: String = reference.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        match &self.bodies[bi].kind {
            BodyKind::Box { .. } => true,
            BodyKind::Glyph(f) => match f.kind() {
                GlyphKind::Resistor => prefix.starts_with('R') || prefix.starts_with('F') || prefix.starts_with("TH"),
                GlyphKind::Capacitor | GlyphKind::CapPolarized => prefix.starts_with('C'),
                GlyphKind::Inductor => prefix.starts_with('L') || prefix.starts_with("FB"),
                GlyphKind::Diode => {
                    prefix.starts_with('D') || prefix.starts_with("LED") || prefix.starts_with("TVS") || prefix.starts_with("ZD")
                }
            },
        }
    }
}

fn merge_words(words: Vec<(i32, i32, String)>) -> Vec<(i32, i32, String)> {
    let mut out: Vec<(i32, i32, String)> = Vec::new();
    for (c0, c1, s) in words {
        if let Some(last) = out.last_mut() {
            if c0 == last.1 + 2 {
                last.1 = c1;
                last.2.push(' ');
                last.2.push_str(&s);
                continue;
            }
        }
        out.push((c0, c1, s));
    }
    out
}

// ------------------------------------------------------------- parts, wires

impl<'a> Parser<'a> {
    fn assemble_parts(&mut self) -> Vec<Part> {
        let mut parts: Vec<Part> = Vec::new();
        let mut refs: HashMap<String, Cell> = HashMap::new();
        self.body_part = vec![None; self.bodies.len()];
        for bi in 0..self.bodies.len() {
            let body = self.bodies[bi].clone();
            let body_cell = (body.bbox.0, body.bbox.1);
            let mut list = self.attached.get(bi).cloned().unwrap_or_default();
            list.sort_by_key(|&i| (self.phrases[i].row, self.phrases[i].c0));
            let ph: Vec<Phrase> = list.iter().map(|&i| self.phrases[i].clone()).collect();
            if ph.is_empty() {
                let desc = self.body_desc(bi);
                let hint = self.nearest_ref_hint(body.bbox);
                self.err("E152", body_cell, format!("{desc} has no reference")).note(hint);
                continue;
            }
            let reference = ph[0].text.clone();
            let ref_cell = (ph[0].row, ph[0].c0);
            if !is_reference(&reference) {
                let desc = self.body_desc(bi);
                self.err("E153", ref_cell, format!("'{reference}' is the first field of {desc}; the first field must be a reference like R1 or U3"))
                    .note("fields read top to bottom: reference, value, then the names in visible = [...]");
                continue;
            }
            if let Some(prev) = refs.insert(reference.clone(), ref_cell) {
                let pp = self.pos(prev);
                self.err("E154", ref_cell, format!("reference {reference} is used twice (also at {pp})"));
                continue;
            }
            let meta = self.meta.part_meta(&reference);
            let variant = meta.symbol.clone();
            let (shapes, mut pins, shape_name, kind) = match body.kind {
                BodyKind::Box { t, l, b, r } => {
                    if variant.is_some() {
                        self.err("E155", ref_cell, format!("{reference}: symbol variants apply to two-pin glyphs, not boxes"));
                    }
                    let rect = Shape::Rect {
                        x0: 0.0,
                        y0: 0.0,
                        x1: (r - l) as f64 * CELL_W,
                        y1: (b - t) as f64 * CELL_H,
                        width: 0.254,
                        fill: Fill::Background,
                    };
                    (vec![rect], body.pins.clone(), "BOX".to_string(), PartKind::Box)
                }
                BodyKind::Glyph(f) => {
                    let v = variant.as_deref();
                    let v = match v {
                        Some(x) if !f.variants().contains(&x) => {
                            let allowed = f.variants();
                            self.err("E155", ref_cell, format!("{reference}: symbol = {x:?} does not apply to this glyph"))
                                .note(if allowed.is_empty() { "this glyph has no variants".to_string() } else { format!("allowed: {}", allowed.join(", ")) });
                            None
                        }
                        other => other,
                    };
                    let built = f.build(body.origin, v);
                    let pins: Vec<PinDef> = built
                        .pins
                        .iter()
                        .map(|p| PinDef {
                            number: p.number.to_string(),
                            name: p.name.to_string(),
                            tip: p.tip,
                            dir: p.dir,
                            length: p.length,
                            etype: String::new(),
                        })
                        .collect();
                    (built.shapes, pins, f.name(v), PartKind::Glyph(f.kind()))
                }
            };
            for p in pins.iter_mut() {
                p.etype = meta
                    .pins
                    .get(&p.number)
                    .or_else(|| if p.name.is_empty() { None } else { meta.pins.get(&p.name) })
                    .cloned()
                    .unwrap_or_else(|| "passive".to_string());
            }
            for k in meta.pins.keys() {
                if !pins.iter().any(|p| &p.number == k || &p.name == k) {
                    self.warn("W157", ref_cell, format!("{reference}: pins.{k} in the header matches no pin number or name"));
                }
            }
            // Fields: reference, value, then the visible extras in order.
            let mut fields = vec![Field { name: "Reference".into(), value: reference.clone(), at: Some(ref_cell) }];
            match ph.get(1) {
                Some(v) => {
                    if let Some(mv) = &meta.value {
                        if mv != &v.text {
                            self.err("E158", (v.row, v.c0), format!("{reference}: drawn value '{}' differs from value = {mv:?} in the header", v.text));
                        }
                    }
                    fields.push(Field { name: "Value".into(), value: v.text.clone(), at: Some((v.row, v.c0)) });
                }
                None => {
                    let value = meta.value.clone().unwrap_or_default();
                    if value.is_empty() {
                        self.warn("W159", ref_cell, format!("{reference} has no value"))
                            .note("draw it on the line under the reference, or set value in the header");
                    }
                    fields.push(Field { name: "Value".into(), value, at: None });
                }
            }
            let names = meta.visible.clone().unwrap_or_else(|| vec!["MPN".to_string()]);
            for (k, p) in ph.iter().skip(2).enumerate() {
                let Some(name) = names.get(k) else {
                    self.err("E160", (p.row, p.c0), format!("{reference}: field line '{}' has no name", p.text))
                        .note("list the names of the lines after the value: visible = [\"MPN\", ...] in the part's header entry");
                    continue;
                };
                if let Some(mv) = meta.fields.get(name) {
                    if mv != &p.text {
                        self.err("E158", (p.row, p.c0), format!("{reference}: drawn {name} '{}' differs from the header value {mv:?}", p.text));
                    }
                }
                if fields.iter().any(|f| &f.name == name) {
                    self.err("E161", (p.row, p.c0), format!("{reference}: field {name} is drawn twice"));
                    continue;
                }
                fields.push(Field { name: name.clone(), value: p.text.clone(), at: Some((p.row, p.c0)) });
            }
            for (name, value) in [
                ("Footprint", meta.footprint.clone()),
                ("Datasheet", meta.datasheet.clone()),
                ("Description", meta.description.clone()),
            ] {
                fields.push(Field { name: name.into(), value: value.unwrap_or_default(), at: None });
            }
            for (name, value) in &meta.fields {
                if !fields.iter().any(|f| &f.name == name) {
                    fields.push(Field { name: name.clone(), value: value.clone(), at: None });
                }
            }
            self.body_part[bi] = Some(parts.len());
            parts.push(Part {
                reference,
                kind,
                shape_name,
                origin: body.origin,
                bbox: body.bbox,
                shapes,
                pins,
                texts: body.texts.clone(),
                fields,
                meta,
                line: self.line(body_cell),
                col: (body_cell.1 + 1) as usize,
            });
        }
        for r in self.meta.part.keys() {
            if !refs.contains_key(r) {
                self.d.push(Diag::warning("W162", self.src.header_line, 1, format!("part {r} is in the header but not drawn")));
            }
        }
        parts
    }

    fn pin_of<'p>(&self, parts: &'p [Part], bi: usize, tip: Cell) -> Option<(&'p Part, &'p PinDef)> {
        let pi = self.body_part.get(bi).copied().flatten()?;
        let part = &parts[pi];
        part.pins.iter().find(|p| p.tip == tip).map(|p| (part, p))
    }

    fn wires_and_nets(&mut self, parts: &[Part]) -> (Vec<Wire>, Vec<Cell>, Vec<Net>) {
        let h = self.g.height() as i32;
        let w = self.g.width as i32;
        let mut wc: BTreeMap<Cell, u8> = BTreeMap::new();
        for r in 0..h {
            for c in 0..w {
                let ch = self.ch((r, c));
                if ch == b' ' || !self.free((r, c)) {
                    continue;
                }
                if WIRE_CHARS.contains(&ch) {
                    wc.insert((r, c), ch);
                } else {
                    let hint = match ch {
                        b'=' => "'=' only appears in [==] resistors and === capacitors",
                        b'[' | b']' => "brackets form [==] resistors and [NAME] global labels at a wire end",
                        b'~' => "an inductor is 2 to 8 '~' between two wire cells",
                        b'#' | b')' | b'(' | b'V' | b'^' => "a vertical part needs a wire cell (| + *) or x directly above or below it",
                        b'>' | b'<' => "a diode is >| or |< between two wire cells",
                        _ => "only wires (- | + *), parts and text (anything with a letter or digit) may appear",
                    };
                    self.err("E170", (r, c), format!("'{}' is not part of any element", ch as char)).note(hint);
                }
            }
        }
        let get = |c: Cell| wc.get(&c).copied();
        let crossing = |c: Cell| -> bool {
            match get(c) {
                Some(b'-') => get((c.0 - 1, c.1)) == Some(b'|') && get((c.0 + 1, c.1)) == Some(b'|'),
                Some(b'|') => get((c.0, c.1 - 1)) == Some(b'-') && get((c.0, c.1 + 1)) == Some(b'-'),
                _ => false,
            }
        };
        let opens = |c: Cell, d: Dir| -> bool {
            match get(c) {
                Some(b'-') => d.horizontal() || crossing(c),
                Some(b'|') => !d.horizontal() || crossing(c),
                Some(_) => true,
                None => false,
            }
        };
        let wire_link = |c: Cell, d: Dir| -> bool { opens(c, d) && opens(d.step(c), d.opposite()) };
        let att = self.att.clone();
        let linked = |c: Cell, d: Dir| -> bool { att.contains_key(&(c, d)) || wire_link(c, d) };

        // Pins whose tip is neither a wire cell nor an x.
        let mut pin_atts: Vec<(Cell, Dir, usize)> = att
            .iter()
            .filter_map(|(&(c, d), a)| if let Att::Pin(bi, _) = a { Some((c, d, *bi)) } else { None })
            .collect();
        pin_atts.sort();
        for &(tip, side, bi) in &pin_atts {
            if get(tip).is_none() && self.own(tip) != Own::NoConnect {
                let Some((part, pin)) = self.pin_of(parts, bi, tip) else { continue };
                let ch = self.ch(tip) as char;
                let msg = format!("pin {}.{} is not connected: the cell {} the part holds '{ch}'", part.reference, pin.number, side.opposite().name().replace("left", "left of").replace("right", "right of"));
                let hint = if pin.dir.horizontal() { "put a wire cell (- + *) or x there" } else { "put a wire cell (| + *) or x there" };
                self.err("E171", tip, msg).note(hint);
            }
        }
        // The pin (if any) that ends on a cell, for messages.
        let tip_pin = |c: Cell| -> Option<String> {
            pin_atts.iter().find(|(t, _, _)| *t == c).and_then(|&(t, _, bi)| self.pin_of(parts, bi, t)).map(|(p, pin)| format!("{}.{}", p.reference, pin.number))
        };
        let mut errs: Vec<(Cell, &'static str, String, String)> = Vec::new();
        for (&c, &ch) in &wc {
            let need: Vec<Dir> = match ch {
                b'-' => {
                    if crossing(c) { Dir::ALL.to_vec() } else { vec![Dir::L, Dir::R] }
                }
                b'|' => {
                    if crossing(c) { Dir::ALL.to_vec() } else { vec![Dir::U, Dir::D] }
                }
                _ => vec![],
            };
            for d in need {
                if linked(c, d) {
                    continue;
                }
                let n = d.step(c);
                let nch = self.ch(n);
                let (what, hint) = match self.own(n) {
                    _ if nch == b' ' => (format!("nothing is {} it", d.name().replace("left", "to the left of").replace("right", "to the right of")), "a wire must end on a pin, a label, a power port, a junction or a corner".to_string()),
                    Own::Phrase(pi) => (format!("it runs into the text '{}'", self.phrases[pi].text), "leave a space between the wire and the text, or make the text a label at the wire end".to_string()),
                    Own::Glyph(b) | Own::Edge(b) | Own::Inside(b) | Own::Halo(b) | Own::PinText(b) => (format!("it runs into {}", self.body_desc(b)), "wires reach a part only at its pin tips".to_string()),
                    Own::NoConnect => ("it runs into an x".to_string(), "an x marks an unused pin; nothing else may connect to it".to_string()),
                    Own::Free => {
                        if d.horizontal() && nch == b'|' {
                            ("it meets a vertical wire without a junction".to_string(), "use * where wires join, or draw - through | on both sides to cross without connecting".to_string())
                        } else if !d.horizontal() && nch == b'-' {
                            ("it meets a horizontal wire without a junction".to_string(), "use * where wires join, or draw | through - on both sides to cross without connecting".to_string())
                        } else {
                            (format!("it does not connect to '{}'", nch as char), "check the wire characters".to_string())
                        }
                    }
                };
                let head = match tip_pin(c) {
                    Some(p) => format!("pin {p} is not connected:"),
                    None => "wire end is not connected:".to_string(),
                };
                let hint = if tip_pin(c).is_some() && nch == b' ' { "continue the wire from the tip, or put x on the tip to mark the pin unused".to_string() } else { hint };
                errs.push((c, "E172", format!("{head} {what}"), hint));
            }
            if ch == b'+' || ch == b'*' {
                let arms: Vec<Dir> = Dir::ALL.iter().copied().filter(|&d| linked(c, d)).collect();
                if ch == b'+' && arms.len() != 2 {
                    let msg = if arms.len() > 2 {
                        format!("'+' joins {} wires", arms.len())
                    } else {
                        format!("'+' corner has {} connection{}", arms.len(), if arms.len() == 1 { "" } else { "s" })
                    };
                    let hint = if arms.len() > 2 { "use * for a junction of three or four wires" } else { "a corner joins exactly two wires (or a wire and a pin)" };
                    errs.push((c, "E173", msg, hint.to_string()));
                }
                if ch == b'*' && arms.len() < 3 {
                    errs.push((c, "E174", format!("junction '*' has {} connections", arms.len()), "a junction joins three or four wires; use + for a corner".to_string()));
                }
            }
        }
        for (c, code, msg, hint) in errs {
            self.err(code, c, msg).note(hint);
        }
        // Connectivity: one node per wire cell, two for a crossing.
        let mut ids: HashMap<(Cell, u8), usize> = HashMap::new();
        let mut uf: Vec<usize> = Vec::new();
        let node = |c: Cell, d: Dir, ids: &mut HashMap<(Cell, u8), usize>, uf: &mut Vec<usize>| -> usize {
            let axis = if crossing(c) { if d.horizontal() { 1 } else { 2 } } else { 0 };
            *ids.entry((c, axis)).or_insert_with(|| {
                uf.push(uf.len());
                uf.len() - 1
            })
        };
        fn find(uf: &mut [usize], x: usize) -> usize {
            let mut x = x;
            while uf[x] != x {
                uf[x] = uf[uf[x]];
                x = uf[x];
            }
            x
        }
        fn union(uf: &mut [usize], a: usize, b: usize) {
            let (ra, rb) = (find(uf, a), find(uf, b));
            if ra != rb {
                uf[ra] = rb;
            }
        }
        for &c in wc.keys() {
            for d in [Dir::R, Dir::D] {
                if wire_link(c, d) {
                    let a = node(c, d, &mut ids, &mut uf);
                    let b = node(d.step(c), d.opposite(), &mut ids, &mut uf);
                    union(&mut uf, a, b);
                }
            }
        }
        let mut by_name: HashMap<(bool, String), usize> = HashMap::new();
        let mut names_at: Vec<(usize, String, bool)> = Vec::new();
        for l in &self.labels {
            let n = node(l.anchor, l.side, &mut ids, &mut uf);
            let key = (l.global, l.name.clone());
            match by_name.get(&key) {
                Some(&m) => union(&mut uf, n, m),
                None => {
                    by_name.insert(key, n);
                }
            }
            names_at.push((n, l.name.clone(), l.global));
        }
        for p in &self.ports {
            let n = node(p.anchor, p.side, &mut ids, &mut uf);
            let key = (true, p.name.clone());
            match by_name.get(&key) {
                Some(&m) => union(&mut uf, n, m),
                None => {
                    by_name.insert(key, n);
                }
            }
            names_at.push((n, p.name.clone(), true));
        }
        let mut nets_by_root: BTreeMap<usize, Net> = BTreeMap::new();
        for part in parts {
            for pin in &part.pins {
                if self.own(pin.tip) == Own::NoConnect {
                    continue;
                }
                let n = node(pin.tip, pin.dir.opposite(), &mut ids, &mut uf);
                let root = find(&mut uf, n);
                nets_by_root.entry(root).or_default().pins.push((part.reference.clone(), pin.number.clone()));
            }
        }
        for (n, name, global) in &names_at {
            let root = find(&mut uf, *n);
            let net = nets_by_root.entry(root).or_default();
            if !net.names.contains(name) {
                net.names.push(name.clone());
            }
            if *global && !net.globals.contains(name) {
                net.globals.push(name.clone());
            }
        }
        let mut nets: Vec<Net> = nets_by_root.into_values().collect();
        for n in nets.iter_mut() {
            n.names.sort();
            n.globals.sort();
            n.pins.sort();
        }
        nets.sort_by(|a, b| (a.names.first(), a.pins.first()).cmp(&(b.names.first(), b.pins.first())));
        // Warnings on the net level.
        let mut local_count: HashMap<String, usize> = HashMap::new();
        for l in self.labels.iter().filter(|l| !l.global) {
            *local_count.entry(l.name.clone()).or_default() += 1;
        }
        let labels = self.labels.clone();
        for l in labels.iter().filter(|l| !l.global) {
            if local_count[&l.name] == 1 {
                self.warn("W175", l.text_at, format!("label '{}' appears only once on this sheet", l.name))
                    .note("a local label connects only to labels of the same name on the same sheet; use [NAME] for a net that continues on another sheet");
            }
        }
        for n in &nets {
            if n.names.len() > 1 {
                self.d.push(Diag::warning("W176", 0, 0, format!("one net carries several names: {}", n.names.join(", "))));
            }
        }
        // KiCad wires: maximal straight runs, split wherever anything attaches.
        let pass = |c: Cell, horiz: bool| -> bool {
            if crossing(c) {
                return true;
            }
            let has_att = Dir::ALL.iter().any(|&d| att.contains_key(&(c, d)));
            !has_att && get(c) == Some(if horiz { b'-' } else { b'|' })
        };
        let mut wires: Vec<Wire> = Vec::new();
        for &c in wc.keys() {
            for (fwd, back, horiz) in [(Dir::R, Dir::L, true), (Dir::D, Dir::U, false)] {
                if !wire_link(c, fwd) || (wire_link(c, back) && pass(c, horiz)) {
                    continue;
                }
                let mut e = fwd.step(c);
                while pass(e, horiz) && wire_link(e, fwd) {
                    e = fwd.step(e);
                }
                wires.push(Wire { a: c, b: e });
            }
        }
        for l in self.labels.iter().filter(|l| l.global) {
            wires.push(Wire { a: l.anchor, b: l.side.step(l.anchor) });
        }
        wires.sort();
        let junctions: Vec<Cell> = wc.iter().filter(|(_, ch)| **ch == b'*').map(|(c, _)| *c).collect();
        (wires, junctions, nets)
    }
}
