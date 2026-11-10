//! Proof that the KiCad files say what the drawing says, using kicad-cli:
//!
//! 1. The netlist KiCad exports equals the drawn netlist (as pin sets).
//! 2. KiCad ERC.
//! 3. The plotted SVG: every text lands inside the cells it was drawn in,
//!    no two texts overlap, and no line passes through a text.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::kicad::{self, left, x, y};
use crate::model::*;
use crate::project::Project;

/// Allowed distance (mm) of rendered ink outside its cells.
const TOL: f64 = 0.1;

fn kicad_cli(args: &[&str]) -> Result<String, String> {
    let out = Command::new("kicad-cli")
        .args(args)
        .output()
        .map_err(|e| format!("cannot run kicad-cli: {e} (install KiCad 10)"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if !out.status.success() && !text.contains("violation") {
        return Err(format!("kicad-cli {} failed:\n{text}", args.join(" ")));
    }
    Ok(text)
}

// ------------------------------------------------------------ s-expressions

#[derive(Debug, Clone)]
enum Sx {
    A(String),
    L(Vec<Sx>),
}

fn parse_sx(s: &str) -> Option<Sx> {
    let b = s.as_bytes();
    let mut i = 0;
    fn skip(b: &[u8], i: &mut usize) {
        while *i < b.len() && b[*i].is_ascii_whitespace() {
            *i += 1;
        }
    }
    fn item(b: &[u8], i: &mut usize) -> Option<Sx> {
        skip(b, i);
        if *i >= b.len() {
            return None;
        }
        if b[*i] == b'(' {
            *i += 1;
            let mut v = Vec::new();
            loop {
                skip(b, i);
                if *i >= b.len() {
                    return None;
                }
                if b[*i] == b')' {
                    *i += 1;
                    return Some(Sx::L(v));
                }
                v.push(item(b, i)?);
            }
        } else if b[*i] == b'"' {
            *i += 1;
            let mut s = Vec::new();
            while *i < b.len() && b[*i] != b'"' {
                if b[*i] == b'\\' && *i + 1 < b.len() {
                    *i += 1;
                }
                s.push(b[*i]);
                *i += 1;
            }
            *i += 1;
            Some(Sx::A(String::from_utf8_lossy(&s).to_string()))
        } else {
            let st = *i;
            while *i < b.len() && !b[*i].is_ascii_whitespace() && b[*i] != b'(' && b[*i] != b')' {
                *i += 1;
            }
            Some(Sx::A(String::from_utf8_lossy(&b[st..*i]).to_string()))
        }
    }
    item(b, &mut i)
}

impl Sx {
    fn head(&self) -> Option<&str> {
        match self {
            Sx::L(v) => match v.first() {
                Some(Sx::A(a)) => Some(a),
                _ => None,
            },
            _ => None,
        }
    }

    fn kids(&self, tag: &str) -> Vec<&Sx> {
        match self {
            Sx::L(v) => v.iter().filter(|x| x.head() == Some(tag)).collect(),
            _ => vec![],
        }
    }

    fn val(&self, tag: &str) -> Option<String> {
        match self.kids(tag).first() {
            Some(Sx::L(v)) => match v.get(1) {
                Some(Sx::A(a)) => Some(a.clone()),
                _ => None,
            },
            _ => None,
        }
    }
}

/// Nets of a KiCad netlist as sets of "REF.PIN", power symbols left out.
fn kicad_nets(path: &Path) -> Result<Vec<BTreeSet<String>>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let root = parse_sx(&text).ok_or("cannot parse the exported netlist")?;
    let nets = root.kids("nets");
    let nets = nets.first().ok_or("netlist has no nets")?;
    let mut out = Vec::new();
    for n in nets.kids("net") {
        let pins: BTreeSet<String> = n
            .kids("node")
            .iter()
            .filter_map(|node| Some(format!("{}.{}", node.val("ref")?, node.val("pin")?)))
            .filter(|p| !p.starts_with('#'))
            .collect();
        if !pins.is_empty() {
            out.push(pins);
        }
    }
    Ok(out)
}

// --------------------------------------------------------------------- SVG

#[derive(Debug, Clone)]
struct Ink {
    text: String,
    b: (f64, f64, f64, f64),
}

#[derive(Default)]
struct Plot {
    texts: Vec<Ink>,
    /// Line segments with half their stroke width.
    segs: Vec<((f64, f64), (f64, f64), f64)>,
    file: Option<String>,
}

fn style_of(style: &str, key: &str) -> Option<String> {
    style.split(';').find_map(|kv| {
        let (k, v) = kv.split_once(':')?;
        if k.trim() == key { Some(v.trim().to_string()) } else { None }
    })
}

/// Points of an SVG path as KiCad writes it (absolute M, L, A, Z), arcs
/// sampled along the curve.
fn path_points(d: &str) -> Vec<Vec<(f64, f64)>> {
    let mut toks: Vec<String> = Vec::new();
    let mut cur = String::new();
    for ch in d.chars() {
        if ch.is_ascii_alphabetic() && ch != 'e' {
            if !cur.is_empty() {
                toks.push(std::mem::take(&mut cur));
            }
            toks.push(ch.to_string());
        } else if ch == ',' || ch.is_whitespace() {
            if !cur.is_empty() {
                toks.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        toks.push(cur);
    }
    let mut subs: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut pts: Vec<(f64, f64)> = Vec::new();
    let mut mode = 'M';
    let mut nums: Vec<f64> = Vec::new();
    for t in toks {
        if let Some(c) = t.chars().next().filter(|c| c.is_ascii_alphabetic()) {
            mode = c.to_ascii_uppercase();
            nums.clear();
            if mode == 'M' && !pts.is_empty() {
                subs.push(std::mem::take(&mut pts));
            }
            if mode == 'Z' {
                if let Some(&f) = pts.first() {
                    pts.push(f);
                }
            }
            continue;
        }
        let Ok(v) = t.parse::<f64>() else { continue };
        nums.push(v);
        match mode {
            'A' if nums.len() == 7 => {
                let start = *pts.last().unwrap_or(&(nums[5], nums[6]));
                pts.extend(arc_points(start, nums[0], nums[1], nums[3] != 0.0, nums[4] != 0.0, (nums[5], nums[6])));
                nums.clear();
            }
            'M' | 'L' if nums.len() == 2 => {
                pts.push((nums[0], nums[1]));
                nums.clear();
            }
            _ => {}
        }
    }
    if !pts.is_empty() {
        subs.push(pts);
    }
    subs
}

/// Sample an SVG elliptical arc (circular here) from its endpoint form.
fn arc_points(p0: (f64, f64), rx: f64, _ry: f64, large: bool, sweep: bool, p1: (f64, f64)) -> Vec<(f64, f64)> {
    let r = rx.abs();
    let (dx, dy) = ((p0.0 - p1.0) / 2.0, (p0.1 - p1.1) / 2.0);
    let d2 = dx * dx + dy * dy;
    if r == 0.0 || d2 == 0.0 {
        return vec![p1];
    }
    let r = r.max(d2.sqrt());
    let mut f = ((r * r - d2) / d2).max(0.0).sqrt();
    if large == sweep {
        f = -f;
    }
    let (cx, cy) = ((p0.0 + p1.0) / 2.0 + f * dy, (p0.1 + p1.1) / 2.0 - f * dx);
    let a0 = (p0.1 - cy).atan2(p0.0 - cx);
    let a1 = (p1.1 - cy).atan2(p1.0 - cx);
    let mut da = a1 - a0;
    if sweep && da < 0.0 {
        da += std::f64::consts::TAU;
    }
    if !sweep && da > 0.0 {
        da -= std::f64::consts::TAU;
    }
    (1..=12).map(|k| {
        let a = a0 + da * k as f64 / 12.0;
        (cx + r * a.cos(), cy + r * a.sin())
    }).collect()
}

fn read_svg(path: &Path) -> Result<Plot, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let opts = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    let doc = roxmltree::Document::parse_with_options(&text, opts).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut plot = Plot::default();
    fn walk(n: roxmltree::Node, style: &str, plot: &mut Plot) {
        let own = n.attribute("style").map(|s| s.replace(['\n', ' '], "")).unwrap_or_default();
        let style = if own.is_empty() { style.to_string() } else { format!("{style};{own}") };
        let tag = n.tag_name().name();
        if tag == "g" && n.attribute("class") == Some("stroked-text") {
            let desc = n.children().find(|c| c.tag_name().name() == "desc").and_then(|c| c.text()).unwrap_or("").to_string();
            let mut b = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for p in n.descendants().filter(|c| c.tag_name().name() == "path") {
                for sub in path_points(p.attribute("d").unwrap_or("")) {
                    for (px, py) in sub {
                        b = (b.0.min(px), b.1.min(py), b.2.max(px), b.3.max(py));
                    }
                }
            }
            if b.0 < f64::MAX && !desc.trim().is_empty() {
                if let Some(f) = desc.strip_prefix("File: ") {
                    plot.file.get_or_insert(f.trim().to_string());
                }
                plot.texts.push(Ink { text: desc, b });
            }
            return;
        }
        let stroke = style_of(&style, "stroke");
        let stroked = stroke.as_deref().is_some_and(|s| s != "none");
        let half = style_of(&style, "stroke-width").and_then(|w| w.parse::<f64>().ok()).unwrap_or(0.0) / 2.0;
        let filled = style_of(&style, "fill").is_some_and(|f| f != "none");
        let mut add = |pts: &[(f64, f64)], h: f64| {
            for w in pts.windows(2) {
                plot.segs.push((w[0], w[1], h));
            }
        };
        match tag {
            "path" if stroked || filled => {
                for sub in path_points(n.attribute("d").unwrap_or("")) {
                    add(&sub, if stroked { half } else { 0.0 });
                }
            }
            "circle" => {
                let g = |k: &str| n.attribute(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                let (cx, cy, r) = (g("cx"), g("cy"), g("r"));
                let pts: Vec<(f64, f64)> = (0..=16)
                    .map(|k| {
                        let a = k as f64 * std::f64::consts::TAU / 16.0;
                        (cx + r * a.cos(), cy + r * a.sin())
                    })
                    .collect();
                add(&pts, half);
            }
            _ => {}
        }
        for c in n.children() {
            walk(c, &style, plot);
        }
    }
    walk(doc.root_element(), "", &mut plot);
    Ok(plot)
}

// ---------------------------------------------------------- expected texts

#[derive(Debug, Clone)]
struct Want {
    text: String,
    /// Cells the text may occupy, mm.
    cells: (f64, f64, f64, f64),
    what: String,
}

fn span(row: i32, c0: i32, n: usize) -> (f64, f64, f64, f64) {
    (left(c0), y(row) - CELL_H / 2.0, left(c0) + n as f64 * CELL_W, y(row) + CELL_H / 2.0)
}

fn wanted(s: &Sheet) -> Vec<Want> {
    let mut v = Vec::new();
    for p in &s.parts {
        for f in &p.fields {
            if let Some((r, c)) = f.at {
                v.push(Want { text: f.value.clone(), cells: span(r, c, f.value.len()), what: format!("{} {}", p.reference, f.name) });
            }
        }
        for t in &p.texts {
            let mut cells = span(t.row, t.col, t.text.len());
            if t.kind == TextKind::PinNumberRaised {
                cells.3 = y(t.row);
            }
            let what = match t.kind {
                TextKind::Normal => format!("{} text", p.reference),
                _ => format!("{} pin number", p.reference),
            };
            v.push(Want { text: t.text.clone(), cells, what });
        }
    }
    for l in &s.labels {
        let n = l.name.len();
        let cells = if l.global {
            // The flag spans from the center of `[` to the center of `]`;
            // the name may start inside the `[` cell.
            let mut sp = span(l.text_at.0, l.text_at.1, n + 2);
            sp.0 = x(l.text_at.1);
            sp.2 = x(l.text_at.1 + n as i32 + 1);
            sp
        } else {
            let mut sp = span(l.text_at.0, l.text_at.1, n);
            // A label's text starts at its anchor, inside the wire's end cell.
            if l.side == Dir::R {
                sp.0 = x(l.anchor.1);
            } else {
                sp.2 = x(l.anchor.1);
            }
            sp
        };
        v.push(Want { text: l.name.clone(), cells, what: format!("label {}", l.name) });
    }
    for p in &s.ports {
        v.push(Want { text: p.name.clone(), cells: span(p.text_at.0, p.text_at.1, p.name.len()), what: format!("power port {}", p.name) });
    }
    for n in &s.notes {
        v.push(Want { text: n.text.clone(), cells: span(n.at.0, n.at.1, n.text.len()), what: "note".into() });
    }
    v
}

fn inside(b: (f64, f64, f64, f64), c: (f64, f64, f64, f64)) -> bool {
    b.0 >= c.0 - TOL && b.1 >= c.1 - TOL && b.2 <= c.2 + TOL && b.3 <= c.3 + TOL
}

fn overlap(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64), m: f64) -> bool {
    a.0 + m < b.2 && b.0 + m < a.2 && a.1 + m < b.3 && b.1 + m < a.3
}

/// Whether a thick segment crosses a box shrunk by `m`.
fn seg_hits(p: (f64, f64), q: (f64, f64), half: f64, b: (f64, f64, f64, f64), m: f64) -> bool {
    let b = (b.0 + m - half, b.1 + m - half, b.2 - m + half, b.3 - m + half);
    if b.0 >= b.2 || b.1 >= b.3 {
        return false;
    }
    // Liang-Barsky clip.
    let (dx, dy) = (q.0 - p.0, q.1 - p.1);
    let mut t0: f64 = 0.0;
    let mut t1: f64 = 1.0;
    for (pp, qq) in [(-dx, p.0 - b.0), (dx, b.2 - p.0), (-dy, p.1 - b.1), (dy, b.3 - p.1)] {
        if pp == 0.0 {
            if qq < 0.0 {
                return false;
            }
        } else {
            let r = qq / pp;
            if pp < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
        }
    }
    t0 <= t1
}

pub struct TextReport {
    pub checked: usize,
    pub problems: Vec<String>,
    /// Largest distance of ink outside its cells, mm.
    pub worst: f64,
}

fn check_texts(sheet: &Sheet, plot: &Plot) -> TextReport {
    let wants = wanted(sheet);
    let mut used = vec![false; plot.texts.len()];
    let mut matched: Vec<(usize, &Want)> = Vec::new();
    let mut problems = Vec::new();
    let mut worst: f64 = 0.0;
    let mut by_text: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, t) in plot.texts.iter().enumerate() {
        by_text.entry(t.text.as_str()).or_default().push(i);
    }
    for w in &wants {
        let (cx, cy) = ((w.cells.0 + w.cells.2) / 2.0, (w.cells.1 + w.cells.3) / 2.0);
        let best = by_text
            .get(w.text.as_str())
            .into_iter()
            .flatten()
            .map(|&i| (i, &plot.texts[i]))
            .filter(|(i, _)| !used[*i])
            .min_by(|a, b| {
                let da = ((a.1.b.0 + a.1.b.2) / 2.0 - cx).hypot((a.1.b.1 + a.1.b.3) / 2.0 - cy);
                let db = ((b.1.b.0 + b.1.b.2) / 2.0 - cx).hypot((b.1.b.1 + b.1.b.3) / 2.0 - cy);
                da.total_cmp(&db)
            });
        let Some((i, t)) = best else {
            problems.push(format!("{} '{}' was not found in the plot", w.what, w.text));
            continue;
        };
        used[i] = true;
        let out = [w.cells.0 - t.b.0, w.cells.1 - t.b.1, t.b.2 - w.cells.2, t.b.3 - w.cells.3]
            .into_iter()
            .fold(0.0f64, f64::max);
        worst = worst.max(out);
        if !inside(t.b, w.cells) {
            problems.push(format!(
                "{} '{}' is drawn {:.2} mm outside its cells (ink {:.2},{:.2}..{:.2},{:.2}; cells {:.2},{:.2}..{:.2},{:.2})",
                w.what, w.text, out, t.b.0, t.b.1, t.b.2, t.b.3, w.cells.0, w.cells.1, w.cells.2, w.cells.3
            ));
        }
        matched.push((i, w));
    }
    // Spatial buckets keep the pair checks near linear on large sheets.
    const B: f64 = 5.0;
    let key = |x: f64, y: f64| ((x / B).floor() as i64, (y / B).floor() as i64);
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (m, &(i, _)) in matched.iter().enumerate() {
        let b = plot.texts[i].b;
        let (x0, y0) = key(b.0, b.1);
        let (x1, y1) = key(b.2, b.3);
        for gx in x0..=x1 {
            for gy in y0..=y1 {
                grid.entry((gx, gy)).or_default().push(m);
            }
        }
    }
    let mut seen: HashSet<(usize, usize)> = HashSet::new();
    for cell in grid.values() {
        for (a, &ma) in cell.iter().enumerate() {
            for &mb in &cell[a + 1..] {
                let (lo, hi) = (ma.min(mb), ma.max(mb));
                let (i, wi) = matched[lo];
                let (j, wj) = matched[hi];
                if overlap(plot.texts[i].b, plot.texts[j].b, 0.02) && seen.insert((lo, hi)) {
                    problems.push(format!("{} '{}' overlaps {} '{}'", wi.what, wi.text, wj.what, wj.text));
                }
            }
        }
    }
    let mut crossed: HashSet<usize> = HashSet::new();
    for &(p, q, half) in &plot.segs {
        let (x0, y0) = key(p.0.min(q.0) - half, p.1.min(q.1) - half);
        let (x1, y1) = key(p.0.max(q.0) + half, p.1.max(q.1) + half);
        for gx in x0..=x1 {
            for gy in y0..=y1 {
                let Some(cell) = grid.get(&(gx, gy)) else { continue };
                for &m in cell {
                    let (i, wi) = matched[m];
                    if !crossed.contains(&m) && seg_hits(p, q, half, plot.texts[i].b, 0.05) {
                        crossed.insert(m);
                        problems.push(format!(
                            "a line from ({:.2},{:.2}) to ({:.2},{:.2}) crosses {} '{}'",
                            p.0, p.1, q.0, q.1, wi.what, wi.text
                        ));
                    }
                }
            }
        }
    }
    TextReport { checked: matched.len(), problems, worst }
}

fn svg_for(dir: &Path) -> Result<BTreeMap<String, Plot>, String> {
    let mut out = BTreeMap::new();
    for e in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let p = e.map_err(|e| e.to_string())?.path();
        if p.extension().is_some_and(|x| x == "svg") {
            let plot = read_svg(&p)?;
            if let Some(f) = plot.file.clone() {
                out.insert(f, plot);
            }
        }
    }
    Ok(out)
}

pub fn verify(p: &Project, out: &Path) -> Result<bool, String> {
    let root = out.join(p.root_file());
    let root_s = root.to_string_lossy().to_string();
    let mut ok = true;

    // 1. Netlist round trip.
    let net = out.join(format!("{}.net", p.name));
    kicad_cli(&["sch", "export", "netlist", "--format", "kicadsexpr", "-o", &net.to_string_lossy(), &root_s])?;
    let theirs = kicad_nets(&net)?;
    let ours: Vec<BTreeSet<String>> = p
        .netlist()
        .into_iter()
        .map(|n| n.pins.iter().map(|(r, q)| format!("{r}.{q}")).collect())
        .collect();
    let our_pins: BTreeSet<String> = ours.iter().flatten().cloned().collect();
    // Unused (x) pins come back as one-pin nets; they are not in ours.
    let theirs: BTreeSet<BTreeSet<String>> = theirs
        .into_iter()
        .filter(|n| n.len() > 1 || n.iter().all(|q| our_pins.contains(q)))
        .collect();
    let ours: BTreeSet<BTreeSet<String>> = ours.into_iter().collect();
    if theirs == ours {
        println!("netlist: KiCad reads back the drawn netlist ({} nets)", ours.len());
    } else {
        ok = false;
        println!("netlist: MISMATCH");
        for n in ours.difference(&theirs) {
            println!("  drawn only:  {}", n.iter().cloned().collect::<Vec<_>>().join(" "));
        }
        for n in theirs.difference(&ours) {
            println!("  KiCad only:  {}", n.iter().cloned().collect::<Vec<_>>().join(" "));
        }
    }

    // 2. ERC.
    let erc = out.join(format!("{}-erc.json", p.name));
    kicad_cli(&["sch", "erc", "--format", "json", "--severity-all", "-o", &erc.to_string_lossy(), &root_s])?;
    let ej: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&erc).map_err(|e| e.to_string())?)
        .map_err(|e| format!("ERC report: {e}"))?;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut lines = Vec::new();
    for sh in ej["sheets"].as_array().cloned().unwrap_or_default() {
        for v in sh["violations"].as_array().cloned().unwrap_or_default() {
            let sev = v["severity"].as_str().unwrap_or("?").to_string();
            *counts.entry(sev.clone()).or_default() += 1;
            let items: Vec<String> = v["items"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|i| i["description"].as_str().unwrap_or("").to_string())
                .collect();
            lines.push(format!("  {sev}: {} [{}]: {}", v["description"].as_str().unwrap_or(""), v["type"].as_str().unwrap_or(""), items.join("; ")));
        }
    }
    let errors = counts.get("error").copied().unwrap_or(0);
    println!("ERC: {} error(s), {} warning(s)", errors, counts.get("warning").copied().unwrap_or(0));
    for l in lines {
        println!("{l}");
    }
    if errors > 0 {
        ok = false;
    }

    // 3. Plotted text.
    let svg_dir = out.join("svg");
    let _ = std::fs::remove_dir_all(&svg_dir);
    std::fs::create_dir_all(&svg_dir).map_err(|e| e.to_string())?;
    kicad_cli(&["sch", "export", "svg", "-o", &svg_dir.to_string_lossy(), &root_s])?;
    let plots = svg_for(&svg_dir)?;
    for sf in &p.sheets {
        let file = if p.sheets.len() == 1 { p.root_file() } else { format!("{}.kicad_sch", sf.sheet.name) };
        let Some(plot) = plots.get(&file) else {
            ok = false;
            println!("texts: no plot found for {file}");
            continue;
        };
        let rep = check_texts(&sf.sheet, plot);
        if rep.problems.is_empty() {
            println!(
                "texts ({}): {} drawn texts all inside their cells (worst {:.2} mm, tolerance {TOL}), none overlap, no line crosses them",
                sf.sheet.name, rep.checked, rep.worst
            );
        } else {
            ok = false;
            println!("texts ({}): {} problem(s) in {} texts", sf.sheet.name, rep.problems.len(), rep.checked);
            for pr in rep.problems.iter().take(40) {
                println!("  {pr}");
            }
        }
    }
    println!("{}", if ok { "verify: PASS" } else { "verify: FAIL" });
    Ok(ok)
}

pub fn render(p: &Project, out: &Path, scale: f64) -> Result<Vec<PathBuf>, String> {
    let root = out.join(p.root_file());
    let svg_dir = out.join("svg");
    let _ = std::fs::remove_dir_all(&svg_dir);
    std::fs::create_dir_all(&svg_dir).map_err(|e| e.to_string())?;
    kicad_cli(&["sch", "export", "svg", "-o", &svg_dir.to_string_lossy(), &root.to_string_lossy()])?;
    let mut pngs = Vec::new();
    for e in std::fs::read_dir(&svg_dir).map_err(|e| e.to_string())? {
        let svg = e.map_err(|e| e.to_string())?.path();
        if svg.extension().is_none_or(|x| x != "svg") {
            continue;
        }
        let png = out.join(format!("{}.png", svg.file_stem().unwrap().to_string_lossy()));
        // Without a display inkscape skips GTK desktop settings that make
        // it abort on some GNOME setups.
        let status = Command::new("inkscape")
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .args([
                "--export-type=png",
                &format!("--export-dpi={}", scale * 25.4),
                "--export-background=white",
                "--export-background-opacity=1",
                &format!("--export-filename={}", png.display()),
                &svg.to_string_lossy(),
            ])
            .output()
            .map_err(|e| format!("cannot run inkscape: {e}"))?;
        if !status.status.success() {
            return Err(format!("inkscape failed on {}", svg.display()));
        }
        pngs.push(png);
    }
    pngs.sort();
    let _ = kicad::ORIGIN;
    Ok(pngs)
}
