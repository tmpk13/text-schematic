//! `tsch box`: print a correctly aligned box (IC, connector) from a pin
//! list, so large parts do not have to be counted out by hand.
//!
//! A pin is `NUM`, `NUM:NAME` or `NUM:NAME=NET`. On the left and right a net
//! becomes a local label (`[NET]` a global one); on the top and bottom a net
//! becomes a power port. A pin without a net gets `x` (unused).

pub struct PinSpec {
    pub num: String,
    pub name: String,
    pub net: Option<String>,
}

pub fn parse_list(s: &str) -> Result<Vec<PinSpec>, String> {
    let mut v = Vec::new();
    for item in s.split(',').map(str::trim).filter(|x| !x.is_empty()) {
        let (pin, net) = match item.split_once('=') {
            Some((p, n)) => (p, Some(n.trim().to_string()).filter(|n| !n.is_empty())),
            None => (item, None),
        };
        let (num, name) = match pin.split_once(':') {
            Some((a, b)) => (a.trim(), b.trim()),
            None => (pin.trim(), ""),
        };
        if num.is_empty() || num.contains(' ') || name.contains(' ') {
            return Err(format!("pin {item:?}: use NUM, NUM:NAME or NUM:NAME=NET without spaces"));
        }
        v.push(PinSpec { num: num.into(), name: name.into(), net });
    }
    Ok(v)
}

struct Canvas {
    rows: Vec<Vec<u8>>,
}

impl Canvas {
    fn put(&mut self, r: usize, c: usize, t: &str) {
        while self.rows.len() <= r {
            self.rows.push(Vec::new());
        }
        let row = &mut self.rows[r];
        if row.len() < c + t.len() {
            row.resize(c + t.len(), b' ');
        }
        row[c..c + t.len()].copy_from_slice(t.as_bytes());
    }

    fn text(&self) -> String {
        let mut s = String::new();
        for r in &self.rows {
            s.push_str(String::from_utf8_lossy(r).trim_end());
            s.push('\n');
        }
        s
    }
}

/// Side pins carry nets as labels; a rail name there must be a global
/// label to join the rail's power ports.
fn side_net(n: &str) -> String {
    if !n.starts_with('[') && crate::parse::is_power_name(n, &[]) { format!("[{n}]") } else { n.to_string() }
}

pub fn generate(reference: &str, value: &str, left: &[PinSpec], right: &[PinSpec], top: &[PinSpec], bottom: &[PinSpec]) -> String {
    let side_len = 1 + left.iter().chain(right).map(|p| p.num.len()).max().unwrap_or(0);
    let left: Vec<PinSpec> = left.iter().map(|p| PinSpec { num: p.num.clone(), name: p.name.clone(), net: p.net.as_deref().map(side_net) }).collect();
    let right: Vec<PinSpec> = right.iter().map(|p| PinSpec { num: p.num.clone(), name: p.name.clone(), net: p.net.as_deref().map(side_net) }).collect();
    let (left, right) = (&left[..], &right[..]);
    let lw = left.iter().filter_map(|p| p.net.as_ref()).map(|n| n.len()).max().unwrap_or(0);
    // Columns: labels, two wire cells, the pin, then the edge.
    let tip_l = if lw > 0 { lw + 2 } else { 0 };
    let l = tip_l + side_len;
    // Top and bottom pins: right of the value text, one pitch apart.
    let pitch = top
        .iter()
        .chain(bottom)
        .map(|p| p.name.len().max(p.num.len() + 1).max(p.net.as_ref().map_or(0, |n| n.len())) + 2)
        .max()
        .unwrap_or(0)
        .max(4);
    let first_v = l + value.len().max(reference.len()) + 2 + top.iter().chain(bottom).map(|p| p.num.len()).max().unwrap_or(0);
    let nv = top.len().max(bottom.len());
    let vcols: Vec<usize> = (0..nv).map(|i| first_v + i * pitch).collect();
    let rows_lr = left.len().max(right.len());
    let inner_lr = left
        .iter()
        .zip(right.iter().map(Some).chain(std::iter::repeat(None)))
        .map(|(a, b)| a.name.len() + b.map_or(0, |b| b.name.len()) + 3)
        .chain(right.iter().map(|b| b.name.len() + 2))
        .max()
        .unwrap_or(0);
    let inner_v = vcols.last().map_or(0, |&c| c + pitch / 2 + 1 - l);
    let width = inner_lr.max(inner_v).max(value.len().min(12)).max(6);
    let r = l + width + 1;
    // Rows: ports and wires above the top pins, fields, the box.
    let top_extra = if top.iter().any(|p| p.net.is_some()) { 1 } else { 0 };
    let t = 2 + top_extra; // reference and value sit in rows t-2 and t-1
    let first_pin_row = t + 1 + usize::from(!top.is_empty());
    let b = first_pin_row + rows_lr + usize::from(!bottom.is_empty());
    let b = b.max(t + 2);
    let mut cv = Canvas { rows: Vec::new() };
    cv.put(t - 2, l, reference);
    cv.put(t - 1, l, value);
    cv.put(t, l, &format!("+{}+", "-".repeat(width)));
    cv.put(b, l, &format!("+{}+", "-".repeat(width)));
    for row in t + 1..b {
        cv.put(row, l, "|");
        cv.put(row, r, "|");
    }
    for (i, p) in left.iter().enumerate() {
        let row = first_pin_row + i;
        let pad = "-".repeat(side_len - 1 - p.num.len());
        match &p.net {
            Some(n) => cv.put(row, tip_l - 2 - n.len(), &format!("{n}---{pad}{}", p.num)),
            None => cv.put(row, tip_l, &format!("x{pad}{}", p.num)),
        }
        cv.put(row, l + 1, &p.name);
    }
    for (i, p) in right.iter().enumerate() {
        let row = first_pin_row + i;
        let pad = "-".repeat(side_len - 1 - p.num.len());
        match &p.net {
            Some(n) => cv.put(row, r + 1, &format!("{}{pad}---{n}", p.num)),
            None => cv.put(row, r + 1, &format!("{}{pad}x", p.num)),
        }
        cv.put(row, r - p.name.len(), &p.name);
    }
    for (side, pins) in [(true, top), (false, bottom)] {
        for (p, &c) in pins.iter().zip(&vcols) {
            let (tip, name_row) = if side { (t - 1, t + 1) } else { (b + 1, b - 1) };
            cv.put(tip, c - p.num.len(), &p.num);
            cv.put(tip, c, if p.net.is_some() { "|" } else { "x" });
            if !p.name.is_empty() {
                cv.put(name_row, c - (p.name.len() - 1) / 2, &p.name);
            }
            if let Some(n) = &p.net {
                let (wire, text) = if side { (tip - 1, tip - 2) } else { (tip + 1, tip + 2) };
                cv.put(wire, c, "|");
                cv.put(text, c - (n.len() - 1) / 2, n);
            }
        }
    }
    cv.text()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_boxes_check_clean() {
        let left = parse_list("1:VBAT=+3V3,7:NRST=[NRST],10:PA0=SDA,11:PA1").unwrap();
        let right = parse_list("30:PA9=TX,31:PA10=RX,34:SWDIO=[SWDIO]").unwrap();
        let top = parse_list("24:VDD=+3V3,36:VDDA=+3V3").unwrap();
        let bottom = parse_list("23:VSS=GND,35:VSSA=GND,47").unwrap();
        let text = generate("U1", "STM32F103C8T6", &left, &right, &top, &bottom);
        let (s, d) = crate::parse_text(&text);
        let errors: Vec<_> = d.items.iter().filter(|x| x.sev == crate::diag::Severity::Error).collect();
        assert!(errors.is_empty(), "{errors:#?}\n{text}");
        let u1 = &s.parts[0];
        assert_eq!(u1.reference, "U1");
        assert_eq!(u1.value(), "STM32F103C8T6");
        assert_eq!(u1.pins.len(), 12);
        let vdd = u1.pins.iter().find(|p| p.number == "24").unwrap();
        assert_eq!(vdd.name, "VDD");
    }

    #[test]
    fn box_without_vertical_pins() {
        let left = parse_list("1:A=X,2:B").unwrap();
        let text = generate("J1", "CONN", &left, &[], &[], &[]);
        let (_, d) = crate::parse_text(&text);
        assert!(!d.has_errors(), "{:#?}\n{text}", d.items);
    }
}
