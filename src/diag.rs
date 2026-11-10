//! Diagnostics: errors and warnings tied to a file line and column.

use std::fmt::Write as _;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
}

/// One finding. `line` and `col` are 1-based positions in the source file;
/// `line == 0` means the finding is not tied to a position.
#[derive(Clone, Debug)]
pub struct Diag {
    pub sev: Severity,
    pub code: &'static str,
    pub line: usize,
    pub col: usize,
    pub msg: String,
    pub hint: Option<String>,
}

impl Diag {
    pub fn error(code: &'static str, line: usize, col: usize, msg: impl Into<String>) -> Self {
        Diag { sev: Severity::Error, code, line, col, msg: msg.into(), hint: None }
    }

    pub fn warning(code: &'static str, line: usize, col: usize, msg: impl Into<String>) -> Self {
        Diag { sev: Severity::Warning, code, line, col, msg: msg.into(), hint: None }
    }

    pub fn hint(mut self, h: impl Into<String>) -> Self {
        self.hint = Some(h.into());
        self
    }

    /// Set the hint on a finding already pushed.
    pub fn note(&mut self, h: impl Into<String>) {
        self.hint = Some(h.into());
    }
}

/// Collected findings for one file.
#[derive(Default, Debug)]
pub struct Diags {
    pub items: Vec<Diag>,
}

impl Diags {
    pub fn push(&mut self, d: Diag) {
        self.items.push(d);
    }

    pub fn has_errors(&self) -> bool {
        self.items.iter().any(|d| d.sev == Severity::Error)
    }

    pub fn count(&self, sev: Severity) -> usize {
        self.items.iter().filter(|d| d.sev == sev).count()
    }

    pub fn sort(&mut self) {
        self.items.sort_by(|a, b| (a.line, a.col, a.sev).cmp(&(b.line, b.col, b.sev)));
    }

    /// Render all findings in the compiler style: location, message, the
    /// source line with a caret under the column, and the hint.
    pub fn render(&self, path: &str, source_lines: &[&str], limit: usize) -> String {
        let mut out = String::new();
        for (i, d) in self.items.iter().enumerate() {
            if i == limit {
                let _ = writeln!(out, "... {} more findings not shown", self.items.len() - limit);
                break;
            }
            let kind = match d.sev {
                Severity::Error => "error",
                Severity::Warning => "warning",
            };
            if d.line == 0 {
                let _ = writeln!(out, "{path}: {kind}[{}]: {}", d.code, d.msg);
            } else {
                let _ = writeln!(out, "{path}:{}:{}: {kind}[{}]: {}", d.line, d.col, d.code, d.msg);
                if let Some(src) = source_lines.get(d.line - 1) {
                    let num = d.line.to_string();
                    let pad = " ".repeat(num.len());
                    let _ = writeln!(out, "{num} | {src}");
                    let caret = " ".repeat(d.col.saturating_sub(1));
                    let _ = writeln!(out, "{pad} | {caret}^");
                }
            }
            if let Some(h) = &d.hint {
                let _ = writeln!(out, "  = hint: {h}");
            }
        }
        out
    }
}
