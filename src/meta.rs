//! The TOML header: sheet title block, extra power net names, and the part
//! table with the properties the drawing does not show.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::diag::{Diag, Diags};

#[derive(Deserialize, Default, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Meta {
    pub title: Option<String>,
    pub date: Option<String>,
    pub rev: Option<String>,
    pub company: Option<String>,
    #[serde(default)]
    pub comments: Vec<String>,
    /// A4, A3, A2, A1 or A0; chosen from the drawing size when absent.
    pub paper: Option<String>,
    /// Net names that may be power ports besides the built-in rail patterns.
    #[serde(default)]
    pub power: Vec<String>,
    /// Properties per reference designator.
    #[serde(default)]
    pub part: BTreeMap<String, PartMeta>,
    /// Properties shared by every part whose reference starts with the key
    /// (letters only, e.g. "R" or "LED"); a part's own entry wins.
    #[serde(default)]
    pub default: BTreeMap<String, PartMeta>,
}

#[derive(Deserialize, Default, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct PartMeta {
    pub value: Option<String>,
    pub footprint: Option<String>,
    pub datasheet: Option<String>,
    pub description: Option<String>,
    /// Extra fields (MPN, Manufacturer, ...). Hidden unless named in `visible`.
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    /// Names of the field lines drawn after Reference and Value, in order.
    pub visible: Option<Vec<String>>,
    /// Pin electrical type by pin number or pin name.
    #[serde(default)]
    pub pins: BTreeMap<String, String>,
    /// Drawing variant: "led", "zener", "schottky" for diodes, "ferrite" for
    /// inductors, "fuse" for resistors.
    pub symbol: Option<String>,
    pub dnp: Option<bool>,
    pub in_bom: Option<bool>,
    pub on_board: Option<bool>,
}

impl PartMeta {
    /// Merge: `self` wins, `base` fills the gaps.
    pub fn over(&self, base: &PartMeta) -> PartMeta {
        let mut fields = base.fields.clone();
        fields.extend(self.fields.clone());
        let mut pins = base.pins.clone();
        pins.extend(self.pins.clone());
        PartMeta {
            value: self.value.clone().or_else(|| base.value.clone()),
            footprint: self.footprint.clone().or_else(|| base.footprint.clone()),
            datasheet: self.datasheet.clone().or_else(|| base.datasheet.clone()),
            description: self.description.clone().or_else(|| base.description.clone()),
            fields,
            visible: self.visible.clone().or_else(|| base.visible.clone()),
            pins,
            symbol: self.symbol.clone().or_else(|| base.symbol.clone()),
            dnp: self.dnp.or(base.dnp),
            in_bom: self.in_bom.or(base.in_bom),
            on_board: self.on_board.or(base.on_board),
        }
    }
}

pub const PIN_TYPES: &[&str] = &[
    "input",
    "output",
    "bidirectional",
    "tri_state",
    "passive",
    "free",
    "unspecified",
    "power_in",
    "power_out",
    "open_collector",
    "open_emitter",
    "no_connect",
];

impl Meta {
    pub fn parse(text: &str, header_line: usize, diags: &mut Diags) -> Meta {
        if text.trim().is_empty() {
            return Meta::default();
        }
        match toml::from_str::<Meta>(text) {
            Ok(m) => {
                for (r, p) in m.part.iter().chain(m.default.iter()) {
                    for (pin, t) in &p.pins {
                        if !PIN_TYPES.contains(&t.as_str()) {
                            diags.push(
                                Diag::error("E010", header_line, 1, format!("part {r}: pin {pin} has unknown type {t:?}"))
                                    .hint(format!("one of: {}", PIN_TYPES.join(", "))),
                            );
                        }
                    }
                }
                m
            }
            Err(e) => {
                let line = e
                    .span()
                    .map(|s| header_line + text[..s.start.min(text.len())].matches('\n').count())
                    .unwrap_or(header_line);
                diags.push(Diag::error("E011", line, 1, format!("header: {}", e.message())));
                Meta::default()
            }
        }
    }

    /// The effective properties of a part: its own entry over the default
    /// for its longest matching reference prefix.
    pub fn part_meta(&self, reference: &str) -> PartMeta {
        let prefix: String = reference.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        let base = self.default.get(&prefix).cloned().unwrap_or_default();
        match self.part.get(reference) {
            Some(p) => p.over(&base),
            None => base,
        }
    }
}
