//! A small S-expression builder that writes KiCad's tab-indented layout.

#[derive(Clone, Debug)]
pub enum S {
    /// Bare token: a keyword, a number, yes/no.
    Atom(String),
    /// Quoted string.
    Str(String),
    List(Vec<S>),
}

pub fn atom(s: impl Into<String>) -> S {
    S::Atom(s.into())
}

pub fn q(s: impl Into<String>) -> S {
    S::Str(s.into())
}

/// A number in mm, written the way KiCad does: up to 4 decimals, no
/// trailing zeros, no negative zero.
pub fn num(v: f64) -> S {
    S::Atom(fmt_num(v))
}

pub fn fmt_num(v: f64) -> String {
    let r = (v * 10000.0).round() / 10000.0;
    let r = if r == 0.0 { 0.0 } else { r };
    let mut s = format!("{r:.4}");
    while s.contains('.') && (s.ends_with('0') || s.ends_with('.')) {
        s.pop();
    }
    s
}

/// `(head items...)`
pub fn list(head: &str, items: Vec<S>) -> S {
    let mut v = vec![atom(head)];
    v.extend(items);
    S::List(v)
}

#[macro_export]
macro_rules! sx {
    ($head:expr $(, $item:expr)* $(,)?) => {
        $crate::sexpr::list($head, vec![$($item.into()),*])
    };
}

impl From<&str> for S {
    fn from(s: &str) -> S {
        S::Atom(s.to_string())
    }
}

impl From<f64> for S {
    fn from(v: f64) -> S {
        num(v)
    }
}

fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    for ch in s.chars() {
        match ch {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c => o.push(c),
        }
    }
    o
}

impl S {
    fn is_short(&self) -> bool {
        match self {
            S::List(v) => v.iter().all(|x| !matches!(x, S::List(_))) && v.len() <= 8,
            _ => true,
        }
    }

    pub fn write(&self, out: &mut String, depth: usize) {
        match self {
            S::Atom(a) => out.push_str(a),
            S::Str(s) => {
                out.push('"');
                out.push_str(&escape(s));
                out.push('"');
            }
            S::List(v) => {
                out.push('(');
                if self.is_short() {
                    for (i, x) in v.iter().enumerate() {
                        if i > 0 {
                            out.push(' ');
                        }
                        x.write(out, depth + 1);
                    }
                    out.push(')');
                    return;
                }
                // Head and leading atoms on the first line, nested lists below.
                let mut i = 0;
                while i < v.len() && !matches!(v[i], S::List(_)) {
                    if i > 0 {
                        out.push(' ');
                    }
                    v[i].write(out, depth + 1);
                    i += 1;
                }
                for x in &v[i..] {
                    out.push('\n');
                    for _ in 0..depth + 1 {
                        out.push('\t');
                    }
                    x.write(out, depth + 1);
                }
                out.push('\n');
                for _ in 0..depth {
                    out.push('\t');
                }
                out.push(')');
            }
        }
    }

    pub fn to_text(&self) -> String {
        let mut s = String::new();
        self.write(&mut s, 0);
        s.push('\n');
        s
    }
}

/// Deterministic UUIDs: the same input text gives the same UUID, so
/// regenerating a sheet keeps symbol identities (and the PCB link) stable.
pub fn uuid(key: &str) -> String {
    let h1 = fnv(key.as_bytes(), 0xcbf29ce484222325);
    let h2 = fnv(key.as_bytes(), 0x84222325cbf29ce4 ^ h1.rotate_left(17));
    let b: Vec<u8> = h1.to_be_bytes().iter().chain(h2.to_be_bytes().iter()).copied().collect();
    let mut b: [u8; 16] = b.try_into().unwrap();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}

fn fnv(data: &[u8], seed: u64) -> u64 {
    let mut h = seed;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_print_like_kicad() {
        assert_eq!(fmt_num(1.27), "1.27");
        assert_eq!(fmt_num(-0.0), "0");
        assert_eq!(fmt_num(2.0), "2");
        assert_eq!(fmt_num(1.46389), "1.4639");
    }

    #[test]
    fn uuids_are_stable_and_distinct() {
        assert_eq!(uuid("a"), uuid("a"));
        assert_ne!(uuid("a"), uuid("b"));
        assert_eq!(uuid("a").len(), 36);
    }
}
