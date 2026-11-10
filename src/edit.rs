//! Column-exact editing of a drawing, for agents and scripts: a text editor
//! cannot easily insert three spaces at column 87 of a line that is mostly
//! spaces. Lines and columns are 1-based file positions, as in messages.

use std::path::Path;

fn read(file: &Path) -> Result<(Vec<String>, bool), String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let trailing = text.ends_with('\n');
    Ok((text.lines().map(str::to_string).collect(), trailing))
}

fn write(file: &Path, lines: &[String], trailing: bool) -> Result<(), String> {
    let mut text: String = lines.iter().map(|l| l.trim_end()).collect::<Vec<_>>().join("\n");
    if trailing || !lines.is_empty() {
        text.push('\n');
    }
    std::fs::write(file, text).map_err(|e| format!("{}: {e}", file.display()))
}

fn line_range(spec: &str, max: usize) -> Result<(usize, usize), String> {
    let (a, b) = match spec.split_once(':') {
        Some((a, b)) => (a, b),
        None => (spec, spec),
    };
    let a: usize = a.parse().map_err(|_| format!("line {spec:?}: use N or FIRST:LAST"))?;
    let b: usize = if b.is_empty() { max } else { b.parse().map_err(|_| format!("line {spec:?}: use N or FIRST:LAST"))? };
    if a == 0 || b < a {
        return Err(format!("line {spec:?}: 1-based and FIRST <= LAST"));
    }
    Ok((a, b))
}

/// Every run of non-space characters with its first column.
pub fn tokens(file: &Path, lines: Option<&str>) -> Result<String, String> {
    let (all, _) = read(file)?;
    let (a, b) = match lines {
        Some(s) => line_range(s, all.len())?,
        None => (1, all.len()),
    };
    let mut out = String::new();
    for (i, l) in all.iter().enumerate().take(b.min(all.len())).skip(a - 1) {
        let mut toks = Vec::new();
        let bytes = l.as_bytes();
        let mut c = 0;
        while c < bytes.len() {
            if bytes[c] == b' ' {
                c += 1;
                continue;
            }
            let s = c;
            while c < bytes.len() && bytes[c] != b' ' {
                c += 1;
            }
            toks.push(format!("{}:{}", s + 1, &l[s..c]));
        }
        if !toks.is_empty() {
            out.push_str(&format!("{}: {}\n", i + 1, toks.join("  ")));
        }
    }
    Ok(out)
}

/// Insert `by` spaces at column `col` of each line in the range (by > 0),
/// or delete -by characters there, which must all be spaces (by < 0).
/// Lines shorter than `col` are left alone.
pub fn shift(file: &Path, lines: &str, col: usize, by: i64) -> Result<String, String> {
    if col == 0 {
        return Err("columns are 1-based".into());
    }
    let (mut all, trailing) = read(file)?;
    let (a, b) = line_range(lines, all.len())?;
    let mut changed = 0;
    for i in a - 1..b.min(all.len()) {
        let l = &mut all[i];
        if l.len() < col {
            continue;
        }
        if by > 0 {
            l.insert_str(col - 1, &" ".repeat(by as usize));
        } else if by < 0 {
            let n = (-by) as usize;
            let end = (col - 1 + n).min(l.len());
            if let Some(k) = l[col - 1..end].find(|ch| ch != ' ') {
                return Err(format!("line {}: column {} holds '{}', not a space; nothing was changed", i + 1, col + k, &l[col - 1 + k..col + k]));
            }
            l.replace_range(col - 1..end, "");
        }
        changed += 1;
    }
    write(file, &all, trailing)?;
    Ok(format!("shifted {changed} line(s)"))
}

/// Write `text` (one or more lines) with its top-left character at
/// line:col. Refuses to overwrite a non-space character with a different
/// one unless `force`.
pub fn put(file: &Path, at: &str, text: &str, force: bool) -> Result<String, String> {
    let (l, c) = at.split_once(':').ok_or("--at LINE:COL")?;
    let line: usize = l.parse().map_err(|_| "--at LINE:COL")?;
    let col: usize = c.parse().map_err(|_| "--at LINE:COL")?;
    if line == 0 || col == 0 {
        return Err("--at is 1-based".into());
    }
    if !text.is_ascii() || text.contains('\t') {
        return Err("the text must be ASCII without tabs".into());
    }
    let (mut all, trailing) = read(file)?;
    let rows: Vec<&str> = text.split('\n').collect();
    // Check everything first so a refused put changes nothing.
    for (k, r) in rows.iter().enumerate() {
        let Some(cur) = all.get(line - 1 + k) else { continue };
        for (j, ch) in r.bytes().enumerate() {
            let old = cur.as_bytes().get(col - 1 + j).copied().unwrap_or(b' ');
            if ch != b' ' && old != b' ' && old != ch && !force {
                return Err(format!(
                    "line {} column {} holds '{}'; put would overwrite it with '{}' (use --force)",
                    line + k,
                    col + j,
                    old as char,
                    ch as char
                ));
            }
        }
    }
    for (k, r) in rows.iter().enumerate() {
        while all.len() < line + k {
            all.push(String::new());
        }
        let cur = &mut all[line - 1 + k];
        let mut b = cur.clone().into_bytes();
        if b.len() < col - 1 + r.len() {
            b.resize(col - 1 + r.len(), b' ');
        }
        for (j, ch) in r.bytes().enumerate() {
            // Spaces in the text are transparent: they keep what is there.
            if ch != b' ' {
                b[col - 1 + j] = ch;
            }
        }
        *cur = String::from_utf8(b).unwrap();
    }
    write(file, &all, trailing)?;
    Ok(format!("wrote {} line(s) at {line}:{col}", rows.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str, text: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("tsch-edit-{}-{name}", std::process::id()));
        std::fs::write(&p, text).unwrap();
        p
    }

    #[test]
    fn shift_inserts_and_deletes_spaces_only() {
        let p = tmp("shift", "ab  cd\nxy\n");
        shift(&p, "1:2", 3, 2).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "ab    cd\nxy\n");
        shift(&p, "1", 3, -4).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "abcd\nxy\n");
        assert!(shift(&p, "1", 2, -1).is_err());
    }

    #[test]
    fn put_overlays_and_refuses_clashes() {
        let p = tmp("put", "a\n");
        put(&p, "2:3", "R1\n10k", false).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "a\n  R1\n  10k\n");
        assert!(put(&p, "2:3", "C1", false).is_err());
        put(&p, "1:1", "a b", false).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "a b\n  R1\n  10k\n");
    }

    #[test]
    fn tokens_list_columns() {
        let p = tmp("tok", "  U1  +--+\n");
        assert_eq!(tokens(&p, None).unwrap(), "1: 3:U1  7:+--+\n");
    }
}
