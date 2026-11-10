//! Two-pin part glyphs: the ASCII forms the parser recognizes and the KiCad
//! graphics drawn inside the same cells.
//!
//! Geometry is in mm relative to the glyph origin cell center, y down. Each
//! pin's tip is the cell just outside the glyph along its axis.

use crate::model::{Cell, Dir, Fill, GlyphKind, Shape};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// `[=]` .. `[====]`, n = number of `=`.
    RH(i32),
    /// `#` stacked n rows.
    RV(i32),
    /// `||`
    CH,
    /// `===`, origin at the middle `=`.
    CV,
    /// `+||` (plus_left) or `||+`.
    CPH { plus_left: bool },
    /// `+` above (plus_top) or below `===`; origin at the middle `=`.
    CPV { plus_top: bool },
    /// `~~` .. `~~~~~~`
    LH(i32),
    /// `)` or `(` stacked n rows.
    LV { n: i32, right: bool },
    /// `>|` (anode_left) or `|<`.
    DH { anode_left: bool },
    /// `V` (anode_top) or `^`.
    DV { anode_top: bool },
}

pub struct GlyphPin {
    pub number: &'static str,
    pub name: &'static str,
    pub tip: Cell,
    pub dir: Dir,
    pub length: f64,
}

pub struct Built {
    pub shapes: Vec<Shape>,
    pub pins: Vec<GlyphPin>,
}

const W: f64 = 1.27;
const H: f64 = 2.54;
/// Half the body thickness across the wire axis (KiCad's R is 2.032 wide).
const HALF: f64 = 1.016;
const PLATE: f64 = 0.508;

impl Form {
    pub fn kind(self) -> GlyphKind {
        match self {
            Form::RH(_) | Form::RV(_) => GlyphKind::Resistor,
            Form::CH | Form::CV => GlyphKind::Capacitor,
            Form::CPH { .. } | Form::CPV { .. } => GlyphKind::CapPolarized,
            Form::LH(_) | Form::LV { .. } => GlyphKind::Inductor,
            Form::DH { .. } | Form::DV { .. } => GlyphKind::Diode,
        }
    }

    pub fn vertical(self) -> bool {
        matches!(self, Form::RV(_) | Form::CV | Form::CPV { .. } | Form::LV { .. } | Form::DV { .. })
    }

    /// Name used for the library symbol, e.g. "R_H2".
    pub fn name(self, variant: Option<&str>) -> String {
        let base = match self {
            Form::RH(n) => format!("R_H{n}"),
            Form::RV(n) => format!("R_V{n}"),
            Form::CH => "C_H".into(),
            Form::CV => "C_V".into(),
            Form::CPH { plus_left: true } => "CP_H_PL".into(),
            Form::CPH { plus_left: false } => "CP_H_PR".into(),
            Form::CPV { plus_top: true } => "CP_V_PT".into(),
            Form::CPV { plus_top: false } => "CP_V_PB".into(),
            Form::LH(n) => format!("L_H{n}"),
            Form::LV { n, right: true } => format!("L_V{n}R"),
            Form::LV { n, right: false } => format!("L_V{n}L"),
            Form::DH { anode_left: true } => "D_H_AL".into(),
            Form::DH { anode_left: false } => "D_H_AR".into(),
            Form::DV { anode_top: true } => "D_V_AT".into(),
            Form::DV { anode_top: false } => "D_V_AB".into(),
        };
        match variant {
            Some(v) => format!("{base}_{}", v.to_ascii_uppercase()),
            None => base,
        }
    }

    pub fn default_reference(self) -> &'static str {
        match self.kind() {
            GlyphKind::Resistor => "R",
            GlyphKind::Capacitor | GlyphKind::CapPolarized => "C",
            GlyphKind::Inductor => "L",
            GlyphKind::Diode => "D",
        }
    }

    /// Variants allowed for this glyph.
    pub fn variants(self) -> &'static [&'static str] {
        match self.kind() {
            GlyphKind::Resistor => &["fuse"],
            GlyphKind::Inductor => &["ferrite"],
            GlyphKind::Diode => &["led", "zener", "schottky"],
            _ => &[],
        }
    }

    /// Cells holding the glyph characters, given the origin.
    pub fn cells(self, o: Cell) -> Vec<Cell> {
        let (r, c) = o;
        match self {
            Form::RH(n) => (0..n + 2).map(|i| (r, c + i)).collect(),
            Form::RV(n) => (0..n).map(|i| (r + i, c)).collect(),
            Form::CH => vec![(r, c), (r, c + 1)],
            Form::CV => vec![(r, c - 1), (r, c), (r, c + 1)],
            Form::CPH { .. } => vec![(r, c), (r, c + 1), (r, c + 2)],
            Form::CPV { plus_top } => {
                let p = if plus_top { r - 1 } else { r + 1 };
                vec![(p, c), (r, c - 1), (r, c), (r, c + 1)]
            }
            Form::LH(n) => (0..n).map(|i| (r, c + i)).collect(),
            Form::LV { n, .. } => (0..n).map(|i| (r + i, c)).collect(),
            Form::DH { .. } => vec![(r, c), (r, c + 1)],
            Form::DV { .. } => vec![(r, c)],
        }
    }

    /// Cells that must stay blank because the graphics reach into them.
    pub fn halo(self, o: Cell) -> Vec<Cell> {
        let (r, c) = o;
        let rows: Vec<i32> = match self {
            Form::RV(n) | Form::LV { n, .. } => (0..n).map(|i| r + i).collect(),
            Form::DV { .. } => vec![r],
            Form::CPV { plus_top } => vec![if plus_top { r - 1 } else { r + 1 }],
            _ => vec![],
        };
        rows.into_iter().flat_map(|rr| [(rr, c - 1), (rr, c + 1)]).collect()
    }

    /// Pin tips with the outward direction, first and second end of the axis.
    pub fn tips(self, o: Cell) -> [(Cell, Dir); 2] {
        let (r, c) = o;
        match self {
            Form::RH(n) => [((r, c - 1), Dir::L), ((r, c + n + 2), Dir::R)],
            Form::RV(n) => [((r - 1, c), Dir::U), ((r + n, c), Dir::D)],
            Form::CH | Form::DH { .. } => [((r, c - 1), Dir::L), ((r, c + 2), Dir::R)],
            Form::CV | Form::DV { .. } => [((r - 1, c), Dir::U), ((r + 1, c), Dir::D)],
            Form::CPH { .. } => [((r, c - 1), Dir::L), ((r, c + 3), Dir::R)],
            Form::CPV { plus_top: true } => [((r - 2, c), Dir::U), ((r + 1, c), Dir::D)],
            Form::CPV { plus_top: false } => [((r - 1, c), Dir::U), ((r + 2, c), Dir::D)],
            Form::LH(n) => [((r, c - 1), Dir::L), ((r, c + n), Dir::R)],
            Form::LV { n, .. } => [((r - 1, c), Dir::U), ((r + n, c), Dir::D)],
        }
    }

    pub fn build(self, o: Cell, variant: Option<&str>) -> Built {
        let mut s: Vec<Shape> = Vec::new();
        let rect = |x0: f64, y0: f64, x1: f64, y1: f64, width: f64, fill: Fill| Shape::Rect { x0, y0, x1, y1, width, fill };
        let line = |pts: &[(f64, f64)], width: f64| Shape::Poly { pts: pts.to_vec(), width, fill: Fill::None };
        let [(t1, d1), (t2, d2)] = self.tips(o);
        // Pin length from the tip to the body edge at `edge` along the axis.
        let len = |tip: Cell, edge: f64| -> f64 {
            let tx = (tip.1 - o.1) as f64 * W;
            let ty = (tip.0 - o.0) as f64 * H;
            if self.vertical() { (ty - edge).abs() } else { (tx - edge).abs() }
        };
        let pin = |number: &'static str, name: &'static str, tip: Cell, dir: Dir, length: f64| GlyphPin {
            number,
            name,
            tip,
            dir,
            length,
        };
        let plus = |cx: f64, cy: f64| -> [Shape; 2] {
            let a = 0.3;
            [
                Shape::Poly { pts: vec![(cx - a, cy), (cx + a, cy)], width: 0.0, fill: Fill::None },
                Shape::Poly { pts: vec![(cx, cy - a), (cx, cy + a)], width: 0.0, fill: Fill::None },
            ]
        };
        let pins = match self {
            Form::RH(n) => {
                let x1 = (n + 1) as f64 * W;
                s.push(rect(0.0, -HALF, x1, HALF, 0.254, Fill::None));
                if variant == Some("fuse") {
                    s.push(line(&[(0.0, 0.0), (x1, 0.0)], 0.0));
                }
                vec![pin("1", "", t1, d1, len(t1, 0.0)), pin("2", "", t2, d2, len(t2, x1))]
            }
            Form::RV(n) => {
                let half = if n == 1 { 0.508 } else { HALF };
                let y1 = (n - 1) as f64 * H + HALF;
                s.push(rect(-half, -HALF, half, y1, 0.254, Fill::None));
                if variant == Some("fuse") {
                    s.push(line(&[(0.0, -HALF), (0.0, y1)], 0.0));
                }
                vec![pin("1", "", t1, d1, len(t1, -HALF)), pin("2", "", t2, d2, len(t2, y1))]
            }
            Form::CH => {
                s.push(line(&[(0.254, -HALF), (0.254, HALF)], PLATE));
                s.push(line(&[(1.016, -HALF), (1.016, HALF)], PLATE));
                vec![pin("1", "", t1, d1, len(t1, 0.254)), pin("2", "", t2, d2, len(t2, 1.016))]
            }
            Form::CV => {
                s.push(line(&[(-HALF, -0.381), (HALF, -0.381)], PLATE));
                s.push(line(&[(-HALF, 0.381), (HALF, 0.381)], PLATE));
                vec![pin("1", "", t1, d1, len(t1, -0.381)), pin("2", "", t2, d2, len(t2, 0.381))]
            }
            Form::CPH { plus_left } => {
                // Plates straddle the boundary between the two `|` cells.
                let mid = if plus_left { 1.905 } else { 0.635 };
                let (px, nx) = if plus_left { (mid - 0.381, mid + 0.381) } else { (mid + 0.381, mid - 0.381) };
                s.push(rect(px - 0.127, -HALF, px + 0.127, HALF, 0.0, Fill::None));
                s.push(rect(nx - 0.127, -HALF, nx + 0.127, HALF, 0.0, Fill::Solid));
                let sign_x = if plus_left { 0.0 } else { 2.0 * W };
                s.extend(plus(sign_x, -0.75));
                let (pos_edge, neg_edge) = if plus_left { (px - 0.127, nx + 0.127) } else { (px + 0.127, nx - 0.127) };
                let (pt, pd, nt, nd) = if plus_left { (t1, d1, t2, d2) } else { (t2, d2, t1, d1) };
                vec![pin("1", "", pt, pd, len(pt, pos_edge)), pin("2", "", nt, nd, len(nt, neg_edge))]
            }
            Form::CPV { plus_top } => {
                let (py, ny) = if plus_top { (-0.381, 0.381) } else { (0.381, -0.381) };
                s.push(rect(-HALF, py - 0.127, HALF, py + 0.127, 0.0, Fill::None));
                s.push(rect(-HALF, ny - 0.127, HALF, ny + 0.127, 0.0, Fill::Solid));
                s.extend(plus(-1.1, if plus_top { -1.9 } else { 1.9 }));
                let (pos_edge, neg_edge) = if plus_top { (py - 0.127, ny + 0.127) } else { (py + 0.127, ny - 0.127) };
                let (pt, pd, nt, nd) = if plus_top { (t1, d1, t2, d2) } else { (t2, d2, t1, d1) };
                vec![pin("1", "", pt, pd, len(pt, pos_edge)), pin("2", "", nt, nd, len(nt, neg_edge))]
            }
            Form::LH(n) => {
                let x0 = -W / 2.0;
                let x1 = (n - 1) as f64 * W + W / 2.0;
                if variant == Some("ferrite") {
                    s.push(rect(x0, -0.762, x1, 0.762, 0.254, Fill::None));
                    s.push(line(&[(x0, 0.762), (x1, -0.762)], 0.0));
                } else {
                    for i in 0..n {
                        let cx = i as f64 * W;
                        s.push(Shape::Arc {
                            start: (cx - W / 2.0, 0.0),
                            mid: (cx, -W / 2.0),
                            end: (cx + W / 2.0, 0.0),
                            width: 0.0,
                        });
                    }
                }
                vec![pin("1", "", t1, d1, len(t1, x0)), pin("2", "", t2, d2, len(t2, x1))]
            }
            Form::LV { n, right } => {
                let y0 = -H / 2.0;
                let y1 = (n - 1) as f64 * H + H / 2.0;
                if variant == Some("ferrite") {
                    s.push(rect(-0.762, y0, 0.762, y1, 0.254, Fill::None));
                    s.push(line(&[(-0.762, y1), (0.762, y0)], 0.0));
                } else {
                    let bx = if right { W / 2.0 } else { -W / 2.0 };
                    for i in 0..(2 * n) {
                        let ya = y0 + i as f64 * W;
                        s.push(Shape::Arc {
                            start: (0.0, ya),
                            mid: (bx, ya + W / 2.0),
                            end: (0.0, ya + W),
                            width: 0.0,
                        });
                    }
                }
                vec![pin("1", "", t1, d1, len(t1, y0)), pin("2", "", t2, d2, len(t2, y1))]
            }
            Form::DH { anode_left } => {
                let led = variant == Some("led");
                let h = if led { 0.762 } else { HALF };
                // Triangle base and bar positions along x.
                let (base, bar) = if anode_left { (-0.381, W) } else { (W + 0.381, 0.0) };
                s.push(Shape::Poly { pts: vec![(base, -h), (base, h), (bar, 0.0), (base, -h)], width: 0.254, fill: Fill::None });
                s.extend(diode_bar(variant, bar, h, false, anode_left));
                if led {
                    let x = (base + bar) / 2.0;
                    s.extend(arrows(x, -0.8, 0.0, -1.0, true));
                }
                let (at, ad, kt, kd) = if anode_left { (t1, d1, t2, d2) } else { (t2, d2, t1, d1) };
                vec![pin("1", "K", kt, kd, len(kt, bar)), pin("2", "A", at, ad, len(at, base))]
            }
            Form::DV { anode_top } => {
                let (base, bar) = if anode_top { (-0.889, 0.635) } else { (0.889, -0.635) };
                s.push(Shape::Poly {
                    pts: vec![(-HALF, base), (HALF, base), (0.0, bar), (-HALF, base)],
                    width: 0.254,
                    fill: Fill::None,
                });
                s.extend(diode_bar(variant, bar, HALF, true, anode_top));
                if variant == Some("led") {
                    s.extend(arrows(1.2, 0.0, 1.0, 0.0, false));
                }
                let (at, ad, kt, kd) = if anode_top { (t1, d1, t2, d2) } else { (t2, d2, t1, d1) };
                vec![pin("1", "K", kt, kd, len(kt, bar)), pin("2", "A", at, ad, len(at, base))]
            }
        };
        Built { shapes: s, pins }
    }
}

/// The cathode bar, with zener or Schottky tails. `at` is the bar position
/// along the axis, `h` its half length. `forward` says the current flows
/// toward +axis (anode on the low side).
fn diode_bar(variant: Option<&str>, at: f64, h: f64, vertical: bool, forward: bool) -> Vec<Shape> {
    let t = 0.381 * if forward { -1.0 } else { 1.0 };
    let pts: Vec<(f64, f64)> = match variant {
        Some("zener") => vec![(at + t, -h), (at, -h), (at, h), (at - t, h)],
        Some("schottky") => vec![(at - t, -h + 0.3), (at - t, -h), (at, -h), (at, h), (at + t, h), (at + t, h - 0.3)],
        _ => vec![(at, -h), (at, h)],
    };
    let pts = if vertical { pts.into_iter().map(|(a, b)| (b, a)).collect() } else { pts };
    vec![Shape::Poly { pts, width: 0.254, fill: Fill::None }]
}

/// Two small emission arrows for an LED. Horizontal glyphs put them above the
/// body, vertical ones to its right.
fn arrows(x: f64, y: f64, _dx: f64, _dy: f64, above: bool) -> Vec<Shape> {
    let mut v = Vec::new();
    for k in [-0.35, 0.35] {
        let (sx, sy, ex, ey) = if above {
            (x + k - 0.2, y, x + k + 0.2, y - 0.4)
        } else {
            (x, y + k + 0.2, x + 0.4, y + k - 0.2)
        };
        v.push(Shape::Poly { pts: vec![(sx, sy), (ex, ey)], width: 0.0, fill: Fill::None });
        v.push(Shape::Poly { pts: vec![(ex - 0.2, ey), (ex, ey), (ex, ey + 0.2)], width: 0.0, fill: Fill::None });
    }
    v
}
