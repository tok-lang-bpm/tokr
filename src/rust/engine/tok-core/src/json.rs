//! Minimal JSON values with output identical to Python's
//! `json.dumps(value, ensure_ascii=False, indent=2)`.

use crate::pyutil::float_repr;

/// A JSON value; objects keep insertion order.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    /// A number printed as given (Python int of any size).
    Num(String),
    Float(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

/// Build an object from (key, value) pairs.
pub fn obj(pairs: Vec<(&str, Json)>) -> Json {
    Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn escape(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

impl Json {
    /// Pretty text with two-space indentation (no trailing newline).
    pub fn pretty(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0, true);
        out
    }

    /// Compact text as `json.dumps(value, ensure_ascii=False)`.
    pub fn compact(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0, false);
        out
    }

    fn write(&self, out: &mut String, level: usize, pretty: bool) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Int(i) => out.push_str(&i.to_string()),
            Json::Num(t) => out.push_str(t),
            Json::Float(f) => {
                if f.is_nan() {
                    out.push_str("NaN");
                } else if f.is_infinite() {
                    out.push_str(if *f > 0.0 { "Infinity" } else { "-Infinity" });
                } else {
                    out.push_str(&float_repr(*f));
                }
            }
            Json::Str(s) => escape(s, out),
            Json::Arr(items) => {
                if items.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push('[');
                for (k, v) in items.iter().enumerate() {
                    if k > 0 {
                        out.push_str(if pretty { "," } else { ", " });
                    }
                    if pretty {
                        out.push('\n');
                        out.push_str(&"  ".repeat(level + 1));
                    }
                    v.write(out, level + 1, pretty);
                }
                if pretty {
                    out.push('\n');
                    out.push_str(&"  ".repeat(level));
                }
                out.push(']');
            }
            Json::Obj(items) => {
                if items.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push('{');
                for (k, (key, v)) in items.iter().enumerate() {
                    if k > 0 {
                        out.push_str(if pretty { "," } else { ", " });
                    }
                    if pretty {
                        out.push('\n');
                        out.push_str(&"  ".repeat(level + 1));
                    }
                    escape(key, out);
                    out.push_str(": ");
                    v.write(out, level + 1, pretty);
                }
                if pretty {
                    out.push('\n');
                    out.push_str(&"  ".repeat(level));
                }
                out.push('}');
            }
        }
    }
}
