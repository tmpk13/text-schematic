//! A project: one sheet, or several sheets under a generated root page.
//! Sheets connect through global labels ([NAME]) and power ports.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::diag::{Diag, Diags, Severity};
use crate::kicad;
use crate::meta::Meta;
use crate::model::Sheet;
use crate::parse;
use crate::sexpr::uuid;
use crate::source::Source;

pub struct SheetFile {
    pub src: Source,
    pub sheet: Sheet,
    pub diags: Diags,
}

pub struct Project {
    pub name: String,
    pub title: Option<String>,
    pub sheets: Vec<SheetFile>,
    /// Findings that span sheets.
    pub diags: Diags,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    name: Option<String>,
    title: Option<String>,
    sheets: Vec<String>,
}

/// A net of the whole project, merged across sheets by global names.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProjectNet {
    pub names: Vec<String>,
    pub pins: Vec<(String, String)>,
}

fn stem(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "sheet".into())
}

pub fn load(inputs: &[PathBuf]) -> Result<Project, String> {
    let (name, title, files): (String, Option<String>, Vec<PathBuf>) =
        if inputs.len() == 1 && inputs[0].extension().is_some_and(|e| e == "toml") {
            let text = std::fs::read_to_string(&inputs[0]).map_err(|e| format!("{}: {e}", inputs[0].display()))?;
            let m: Manifest = toml::from_str(&text).map_err(|e| format!("{}: {}", inputs[0].display(), e.message()))?;
            let dir = inputs[0].parent().unwrap_or(Path::new("."));
            let files = m.sheets.iter().map(|s| dir.join(s)).collect();
            (m.name.unwrap_or_else(|| stem(&inputs[0])), m.title, files)
        } else if inputs.len() == 1 {
            (stem(&inputs[0]), None, inputs.to_vec())
        } else {
            ("project".into(), None, inputs.to_vec())
        };
    if files.is_empty() {
        return Err("no sheets given".into());
    }
    let mut sheets = Vec::new();
    let mut stems = BTreeSet::new();
    for f in &files {
        let text = std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?;
        let s = stem(f);
        if !stems.insert(s.clone()) {
            return Err(format!("two sheets are named {s}; sheet file names must differ"));
        }
        if files.len() > 1 && s == name {
            return Err(format!("sheet {s} has the project's name; name the project differently (name = ... in the manifest)"));
        }
        let mut diags = Diags::default();
        let path = f.display().to_string();
        let src = Source::parse(&path, &text, &mut diags);
        let meta = Meta::parse(&src.header, src.header_line, &mut diags);
        let sheet = parse::parse(&src, &meta, &s, &mut diags);
        if let Err(e) = kicad::choose_paper(&sheet) {
            diags.push(Diag::error("E180", 0, 0, e));
        }
        diags.sort();
        sheets.push(SheetFile { src, sheet, diags });
    }
    let mut p = Project { name, title, sheets, diags: Diags::default() };
    p.cross_checks();
    Ok(p)
}

impl Project {
    pub fn has_errors(&self) -> bool {
        self.diags.has_errors() || self.sheets.iter().any(|s| s.diags.has_errors())
    }

    pub fn count(&self, sev: Severity) -> usize {
        self.diags.count(sev) + self.sheets.iter().map(|s| s.diags.count(sev)).sum::<usize>()
    }

    fn cross_checks(&mut self) {
        let mut refs: BTreeMap<String, String> = BTreeMap::new();
        // Units of each multi-unit part: unit -> sheet, and its pin numbers.
        let mut units: BTreeMap<String, BTreeMap<u32, String>> = BTreeMap::new();
        let mut single: BTreeSet<String> = BTreeSet::new();
        let mut pins: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        for s in &self.sheets {
            for p in &s.sheet.parts {
                let shown = p.shown_reference();
                if let Some(other) = refs.insert(shown.clone(), s.sheet.name.clone()) {
                    if other != s.sheet.name {
                        self.diags.push(Diag::error("E181", 0, 0, format!("reference {shown} is on sheets {other} and {}", s.sheet.name)));
                    }
                }
                match p.unit {
                    Some(u) => {
                        units.entry(p.reference.clone()).or_default().insert(u, s.sheet.name.clone());
                        for pin in &p.pins {
                            if let Some(prev) = pins.entry(p.reference.clone()).or_default().insert(pin.number.clone(), shown.clone()) {
                                if prev != shown {
                                    self.diags.push(Diag::error("E183", 0, 0, format!("pin {} of {} is on units {prev} and {shown}", pin.number, p.reference)));
                                }
                            }
                        }
                    }
                    None => {
                        single.insert(p.reference.clone());
                    }
                }
            }
        }
        for base in units.keys() {
            let all: Vec<&crate::model::Part> =
                self.sheets.iter().flat_map(|s| s.sheet.parts.iter()).filter(|p| &p.reference == base && p.unit.is_some()).collect();
            let keys: BTreeSet<&String> = all.iter().flat_map(|p| p.meta.pins.keys()).collect();
            for k in keys {
                if !all.iter().any(|q| q.pins.iter().any(|pin| &pin.number == k || &pin.name == k)) {
                    self.diags.push(Diag::warning("W186", 0, 0, format!("{base}: pins.{k} in the header matches no pin of any unit")));
                }
            }
            for p in &all {
                if p.value() != all[0].value() || p.meta.footprint != all[0].meta.footprint {
                    self.diags.push(Diag::error("E187", 0, 0, format!("units {} and {} of {base} differ in value or footprint", all[0].shown_reference(), p.shown_reference())));
                }
            }
        }
        for (base, us) in &units {
            if single.contains(base) {
                self.diags.push(Diag::error("E184", 0, 0, format!("{base} is drawn both as a whole part and as units ({base}A...)")));
            }
            let n = *us.keys().max().unwrap();
            if us.len() as u32 != n {
                let have: Vec<String> = us.keys().map(|u| format!("{base}{}", (b'A' + (*u - 1) as u8) as char)).collect();
                self.diags.push(Diag::error("E185", 0, 0, format!("units of {base} must run from A without gaps; drawn: {}", have.join(", "))));
            }
        }
        if self.sheets.len() > 1 {
            let mut count: BTreeMap<String, usize> = BTreeMap::new();
            for s in &self.sheets {
                for l in s.sheet.labels.iter().filter(|l| l.global) {
                    *count.entry(l.name.clone()).or_default() += 1;
                }
            }
            for (n, c) in count {
                if c == 1 {
                    self.diags.push(Diag::warning("W182", 0, 0, format!("global label [{n}] appears only once in the project")));
                }
            }
        }
    }

    /// Render every finding, file by file.
    pub fn report(&self, limit: usize) -> String {
        let mut out = String::new();
        for s in &self.sheets {
            let lines: Vec<&str> = s.src.text.lines().collect();
            out.push_str(&s.diags.render(&s.src.path, &lines, limit));
        }
        out.push_str(&self.diags.render(&self.name, &[], limit));
        out
    }

    pub fn netlist(&self) -> Vec<ProjectNet> {
        // Union sheet nets that share a global name.
        let mut nets: Vec<(Vec<String>, BTreeSet<String>, BTreeSet<(String, String)>)> = Vec::new();
        for s in &self.sheets {
            for n in &s.sheet.nets {
                let names: Vec<String> = if self.sheets.len() > 1 {
                    n.names
                        .iter()
                        .map(|x| if n.globals.contains(x) { x.clone() } else { format!("/{}/{x}", s.sheet.name) })
                        .collect()
                } else {
                    n.names.clone()
                };
                nets.push((names, n.globals.iter().cloned().collect(), n.pins.iter().cloned().collect()));
            }
        }
        let mut parent: Vec<usize> = (0..nets.len()).collect();
        fn find(p: &mut [usize], x: usize) -> usize {
            let mut x = x;
            while p[x] != x {
                p[x] = p[p[x]];
                x = p[x];
            }
            x
        }
        let mut by_global: BTreeMap<String, usize> = BTreeMap::new();
        for i in 0..nets.len() {
            for g in nets[i].1.clone() {
                if let Some(&j) = by_global.get(&g) {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    parent[a] = b;
                } else {
                    by_global.insert(g, i);
                }
            }
        }
        let mut merged: BTreeMap<usize, (BTreeSet<String>, BTreeSet<(String, String)>)> = BTreeMap::new();
        for i in 0..nets.len() {
            let r = find(&mut parent, i);
            let e = merged.entry(r).or_default();
            e.0.extend(nets[i].0.iter().cloned());
            e.1.extend(nets[i].2.iter().cloned());
        }
        let mut out: Vec<ProjectNet> = merged
            .into_values()
            .filter(|(_, pins)| !pins.is_empty())
            .map(|(n, p)| ProjectNet { names: n.into_iter().collect(), pins: p.into_iter().collect() })
            .collect();
        out.sort();
        out
    }

    pub fn root_file(&self) -> String {
        format!("{}.kicad_sch", self.name)
    }

    /// Write the KiCad files into `dir`. Returns the paths written.
    pub fn write(&self, dir: &Path) -> Result<Vec<PathBuf>, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let mut written = Vec::new();
        let root_uuid = uuid(&format!("root/{}", self.name));
        let mut pwr = 1;
        let lib = kicad::Library::build(&self.sheets.iter().map(|s| &s.sheet).collect::<Vec<_>>());
        let put = |path: PathBuf, text: &str, written: &mut Vec<PathBuf>| -> Result<(), String> {
            std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
            written.push(path);
            Ok(())
        };
        if self.sheets.len() == 1 {
            let s = &self.sheets[0].sheet;
            let paper = kicad::choose_paper(s)?;
            let ctx = kicad::SheetCtx {
                project: &self.name,
                lib: &lib,
                sheet_index: 0,
                sheet_uuid: root_uuid.clone(),
                path: format!("/{root_uuid}"),
                pwr_start: pwr,
                paper,
                page: "1".into(),
                extra: Vec::new(),
                is_root: true,
            };
            let (text, _) = kicad::write_sheet(s, &ctx);
            put(dir.join(self.root_file()), &text, &mut written)?;
        } else {
            let mut blocks = Vec::new();
            for (i, sf) in self.sheets.iter().enumerate() {
                let s = &sf.sheet;
                let block = uuid(&format!("block/{}/{}", self.name, s.name));
                let file = format!("{}.kicad_sch", s.name);
                let (col, row) = ((i % 3) as f64, (i / 3) as f64);
                let xy = (25.4 + col * 88.9, 30.48 + row * 30.48);
                blocks.push(kicad::sheet_block(&self.name, &root_uuid, &block, &s.name, &file, i + 2, xy, (63.5, 15.24)));
                let paper = kicad::choose_paper(s)?;
                let ctx = kicad::SheetCtx {
                    project: &self.name,
                    lib: &lib,
                    sheet_index: i,
                    sheet_uuid: uuid(&format!("sheet/{}/{}", self.name, s.name)),
                    path: format!("/{root_uuid}/{block}"),
                    pwr_start: pwr,
                    paper,
                    page: (i + 2).to_string(),
                    extra: Vec::new(),
                    is_root: false,
                };
                let (text, used) = kicad::write_sheet(s, &ctx);
                pwr += used;
                put(dir.join(file), &text, &mut written)?;
            }
            let root = Sheet {
                name: self.name.clone(),
                meta: Meta { title: self.title.clone().or_else(|| Some(self.name.clone())), ..Meta::default() },
                ..Sheet::default()
            };
            let empty = kicad::Library::default();
            let ctx = kicad::SheetCtx {
                project: &self.name,
                lib: &empty,
                sheet_index: 0,
                sheet_uuid: root_uuid.clone(),
                path: format!("/{root_uuid}"),
                pwr_start: 0,
                paper: kicad::PAPERS[0],
                page: "1".into(),
                extra: blocks,
                is_root: true,
            };
            let (text, _) = kicad::write_sheet(&root, &ctx);
            put(dir.join(self.root_file()), &text, &mut written)?;
        }
        put(dir.join("tsch.kicad_sym"), &lib.file_text(), &mut written)?;
        let table = dir.join("sym-lib-table");
        let existing_table = std::fs::read_to_string(&table).unwrap_or_default();
        if !existing_table.contains("(name \"tsch\")") {
            let text = if existing_table.trim().is_empty() {
                kicad::SYM_LIB_TABLE.to_string()
            } else {
                // Keep the user's entries; add ours before the closing paren.
                let t = existing_table.trim_end().trim_end_matches(')').to_string();
                format!("{t}\t(lib (name \"tsch\") (type \"KiCad\") (uri \"${{KIPRJMOD}}/tsch.kicad_sym\") (options \"\") (descr \"Symbols generated by tsch\"))\n)\n")
            };
            put(table, &text, &mut written)?;
        }
        let pro = dir.join(format!("{}.kicad_pro", self.name));
        let existing = std::fs::read_to_string(&pro).ok();
        let text = kicad::project_json(existing.as_deref(), &format!("{}.kicad_pro", self.name));
        put(pro, &text, &mut written)?;
        Ok(written)
    }
}
