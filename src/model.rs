//! The parsed sheet in grid coordinates. A cell is (row, col); its center
//! sits at x = col * CELL_W, y = row * CELL_H from the grid origin, in mm.

use crate::meta::{Meta, PartMeta};

pub type Cell = (i32, i32);

pub const CELL_W: f64 = 1.27;
pub const CELL_H: f64 = 2.54;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Dir {
    L,
    R,
    U,
    D,
}

impl Dir {
    pub const ALL: [Dir; 4] = [Dir::L, Dir::R, Dir::U, Dir::D];

    pub fn opposite(self) -> Dir {
        match self {
            Dir::L => Dir::R,
            Dir::R => Dir::L,
            Dir::U => Dir::D,
            Dir::D => Dir::U,
        }
    }

    pub fn delta(self) -> (i32, i32) {
        match self {
            Dir::L => (0, -1),
            Dir::R => (0, 1),
            Dir::U => (-1, 0),
            Dir::D => (1, 0),
        }
    }

    pub fn step(self, c: Cell) -> Cell {
        let (dr, dc) = self.delta();
        (c.0 + dr, c.1 + dc)
    }

    pub fn horizontal(self) -> bool {
        matches!(self, Dir::L | Dir::R)
    }

    pub fn name(self) -> &'static str {
        match self {
            Dir::L => "left",
            Dir::R => "right",
            Dir::U => "above",
            Dir::D => "below",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fill {
    None,
    Background,
    Solid,
}

/// Symbol graphics in mm relative to the part origin cell center, y down.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Rect { x0: f64, y0: f64, x1: f64, y1: f64, width: f64, fill: Fill },
    Poly { pts: Vec<(f64, f64)>, width: f64, fill: Fill },
    /// Arc through three points.
    Arc { start: (f64, f64), mid: (f64, f64), end: (f64, f64), width: f64 },
}

#[derive(Clone, Debug)]
pub struct PinDef {
    pub number: String,
    /// Empty when the drawing gives no name.
    pub name: String,
    pub tip: Cell,
    /// From the body toward the tip.
    pub dir: Dir,
    pub length: f64,
    pub etype: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextKind {
    /// Normal text on the row line: pin names and text inside a box.
    Normal,
    /// Pin number of a left or right pin: small, raised above the pin line.
    PinNumberRaised,
    /// Pin number beside a top or bottom pin: small, on the row line.
    PinNumberSide,
}

/// Text drawn as part of a symbol, at its grid cells.
#[derive(Clone, Debug)]
pub struct SymText {
    pub text: String,
    pub row: i32,
    pub col: i32,
    pub kind: TextKind,
}

#[derive(Clone, Debug)]
pub struct Field {
    pub name: String,
    pub value: String,
    /// (row, first column) when drawn; hidden fields have none.
    pub at: Option<Cell>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlyphKind {
    Resistor,
    Capacitor,
    CapPolarized,
    Inductor,
    Diode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PartKind {
    Box,
    Glyph(GlyphKind),
}

#[derive(Clone, Debug)]
pub struct Part {
    pub reference: String,
    pub kind: PartKind,
    /// Short name of the drawing, used to name the library symbol.
    pub shape_name: String,
    pub origin: Cell,
    /// Body cells, inclusive: (top, left, bottom, right).
    pub bbox: (i32, i32, i32, i32),
    pub shapes: Vec<Shape>,
    pub pins: Vec<PinDef>,
    pub texts: Vec<SymText>,
    /// Reference, Value, then the rest.
    pub fields: Vec<Field>,
    pub meta: PartMeta,
    /// Source position of the body (1-based line and column).
    pub line: usize,
    pub col: usize,
}

impl Part {
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }

    pub fn value(&self) -> &str {
        self.field("Value").map(|f| f.value.as_str()).unwrap_or("")
    }
}

/// A straight wire between two cell centers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Wire {
    pub a: Cell,
    pub b: Cell,
}

#[derive(Clone, Debug)]
pub struct Label {
    pub name: String,
    /// Connection point.
    pub anchor: Cell,
    /// Side of the anchor the text extends to (L or R).
    pub side: Dir,
    pub global: bool,
    /// First cell of the drawn text (for a global label: the bracket).
    pub text_at: Cell,
}

#[derive(Clone, Debug)]
pub struct Port {
    pub name: String,
    pub anchor: Cell,
    /// U: the name is above the wire end (a supply); D: below it.
    pub side: Dir,
    pub ground: bool,
    pub text_at: Cell,
}

#[derive(Clone, Debug)]
pub struct Note {
    pub text: String,
    pub at: Cell,
}

#[derive(Clone, Debug, Default)]
pub struct Net {
    /// Label or port names on the net, sorted.
    pub names: Vec<String>,
    /// (reference, pin number), sorted.
    pub pins: Vec<(String, String)>,
    /// Names from global labels and power ports: the net may continue on
    /// another sheet under these names.
    pub globals: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Sheet {
    /// File stem; also the KiCad sheet file name.
    pub name: String,
    pub path: String,
    pub meta: Meta,
    pub parts: Vec<Part>,
    pub wires: Vec<Wire>,
    pub junctions: Vec<Cell>,
    pub no_connects: Vec<Cell>,
    pub labels: Vec<Label>,
    pub ports: Vec<Port>,
    pub notes: Vec<Note>,
    pub nets: Vec<Net>,
    pub rows: usize,
    pub cols: usize,
    /// Last non-space column of each grid row (-1 when the row is empty).
    pub row_ends: Vec<i32>,
}
