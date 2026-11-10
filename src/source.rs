//! Loading a text schematic: the optional TOML header between `+++` lines,
//! then the drawing grid. Grid row 0 is the first line after the header.

use crate::diag::{Diag, Diags};

/// The drawing as a grid of ASCII bytes. Cells outside the drawn text read
/// as a space.
#[derive(Clone, Debug, Default)]
pub struct Grid {
    pub rows: Vec<Vec<u8>>,
    pub width: usize,
}

impl Grid {
    pub fn get(&self, r: i32, c: i32) -> u8 {
        if r < 0 || c < 0 {
            return b' ';
        }
        match self.rows.get(r as usize) {
            Some(row) => *row.get(c as usize).unwrap_or(&b' '),
            None => b' ',
        }
    }

    pub fn height(&self) -> usize {
        self.rows.len()
    }

    pub fn from_lines<S: AsRef<str>>(lines: &[S]) -> Grid {
        let rows: Vec<Vec<u8>> = lines
            .iter()
            .map(|l| l.as_ref().trim_end().as_bytes().to_vec())
            .collect();
        let width = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        Grid { rows, width }
    }
}

#[derive(Debug)]
pub struct Source {
    pub path: String,
    pub text: String,
    /// TOML header text (empty when the file has no header).
    pub header: String,
    /// 1-based file line of the first header line.
    pub header_line: usize,
    /// 1-based file line of grid row 0.
    pub first_line: usize,
    pub grid: Grid,
}

impl Source {
    /// Split a file into header and grid. Tabs and non-ASCII characters in
    /// the drawing are errors because they break the column mapping.
    pub fn parse(path: &str, text: &str, diags: &mut Diags) -> Source {
        let lines: Vec<&str> = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
        let mut header = String::new();
        let mut header_line = 0;
        let mut start = 0;
        if lines.first().map(|l| l.trim_end()) == Some("+++") {
            match lines.iter().skip(1).position(|l| l.trim_end() == "+++") {
                Some(end) => {
                    header = lines[1..end + 1].join("\n");
                    header_line = 2;
                    start = end + 2;
                }
                None => {
                    diags.push(
                        Diag::error("E001", 1, 1, "header opened with +++ is never closed")
                            .hint("end the TOML header with a line holding only +++"),
                    );
                    start = lines.len();
                }
            }
        }
        let mut body: Vec<&str> = lines[start.min(lines.len())..].to_vec();
        // A trailing newline leaves one empty element; drop trailing blank lines.
        while body.last().is_some_and(|l| l.trim().is_empty()) {
            body.pop();
        }
        for (i, l) in body.iter().enumerate() {
            for (j, ch) in l.chars().enumerate() {
                if ch == '\t' {
                    diags.push(
                        Diag::error("E002", start + i + 1, j + 1, "tab in the drawing")
                            .hint("use spaces: every character is one grid column"),
                    );
                } else if !ch.is_ascii() {
                    diags.push(
                        Diag::error("E003", start + i + 1, j + 1, format!("non-ASCII character {ch:?} in the drawing"))
                            .hint("the drawing is ASCII only"),
                    );
                }
            }
        }
        let clean: Vec<String> = body
            .iter()
            .map(|l| l.chars().map(|c| if c.is_ascii() && c != '\t' { c } else { ' ' }).collect())
            .collect();
        Source {
            path: path.to_string(),
            text: text.to_string(),
            header,
            header_line,
            first_line: start + 1,
            grid: Grid::from_lines(&clean),
        }
    }

    /// File line (1-based) of a grid row.
    pub fn line_of(&self, row: i32) -> usize {
        (self.first_line as i64 + row as i64).max(1) as usize
    }
}
