//! Print part of a file with line numbers and a column ruler, so positions
//! in a wide drawing can be read off instead of counted.

use std::path::Path;

fn range(spec: Option<&str>, max: usize) -> Result<(usize, usize), String> {
    let Some(s) = spec else { return Ok((1, max.max(1))) };
    let (a, b) = s.split_once(':').ok_or_else(|| format!("range {s:?}: use FIRST:LAST, e.g. 10:40"))?;
    let a: usize = if a.is_empty() { 1 } else { a.parse().map_err(|_| format!("range {s:?}: not a number"))? };
    let b: usize = if b.is_empty() { max } else { b.parse().map_err(|_| format!("range {s:?}: not a number"))? };
    if a == 0 || b < a {
        return Err(format!("range {s:?}: 1-based and FIRST <= LAST"));
    }
    Ok((a, b))
}

pub fn show(file: &Path, lines: Option<&str>, cols: Option<&str>) -> Result<String, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let all: Vec<&str> = text.lines().collect();
    let widest = all.iter().map(|l| l.len()).max().unwrap_or(1);
    let (l0, l1) = range(lines, all.len())?;
    let (c0, c1) = range(cols, widest)?;
    let gutter = l1.to_string().len();
    let pad = " ".repeat(gutter);
    let mut tens = String::new();
    let mut ones = String::new();
    for c in c0..=c1 {
        tens.push(if c % 10 == 0 { char::from(b'0' + ((c / 10) % 10) as u8) } else { ' ' });
        ones.push(char::from(b'0' + (c % 10) as u8));
    }
    let mut out = format!("{pad} | {}\n{pad} | {}\n", tens.trim_end(), ones);
    for (i, l) in all.iter().enumerate().take(l1.min(all.len())).skip(l0 - 1) {
        let slice: String = l.chars().skip(c0 - 1).take(c1 + 1 - c0).collect();
        out.push_str(&format!("{:>gutter$} | {}\n", i + 1, slice.trim_end()));
    }
    out.push_str(&format!("{pad} | {}\n", ones));
    Ok(out)
}
