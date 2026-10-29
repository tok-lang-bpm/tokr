//! Canonical form of the statements of language 0.7: DMN types with components, input
//! data, knowledge sources, business knowledge models and decisions with their boxed
//! expressions.

use super::{Printer, join, q, tail};
use crate::model::*;

/// Keys of language 0.7 whose values are always quoted (lists, expressions and texts).
const QUOTED_07: &[&str] = &[
    "requires", "params", "with", "question", "answers", "uri", "allowed", "expr", "default",
];

fn tail07(el: &Base, keys: &[&str], flags: &[&str]) -> Vec<String> {
    tail(el, keys, flags, false)
        .into_iter()
        .map(|p| match p.split_once('=') {
            Some((k, v)) if QUOTED_07.contains(&k) && !v.starts_with('"') => {
                format!("{k}={}", q(v))
            }
            _ => p,
        })
        .collect()
}

/// A name as a token: a word when it reads back as one, otherwise a quoted string.
fn name_token(n: &str) -> String {
    let mut chars = n.chars();
    let word = match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' || (c as u32) > 127 => chars.all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '_' | '-' | '.' | ':' | '/' | '+')
                || (c as u32) > 127
        }),
        _ => false,
    };
    if word && !n.contains("->") {
        n.to_string()
    } else {
        q(n)
    }
}

fn header_cell(name: &str, typ: &str, values: Option<&String>) -> String {
    match values {
        Some(v) => format!("{name}: {typ} = {v}"),
        None => format!("{name}: {typ}"),
    }
}

impl Printer<'_> {
    /// The components of a DMN type, indented below it.
    pub(super) fn components(&mut self, list: &[Component], indent: usize) {
        for c in list {
            let text = join(
                &["item", &name_token(c.base.id())],
                tail07(&c.base, COMPONENT_KEYS, ITEM_FLAGS),
            );
            self.emit_base(indent, &text, &c.base);
            self.components(&c.children, indent + 1);
        }
    }

    /// The decision requirements graph and the decisions of a 0.7 document.
    pub(super) fn drg(&mut self) {
        let d = self.doc;
        if !d.inputdata.is_empty() || !d.sources.is_empty() {
            self.blank();
            for b in d.inputdata.values() {
                let text = join(&["inputdata", b.id()], tail07(b, INPUTDATA_KEYS, &[]));
                self.emit_base(0, &text, b);
            }
            for b in d.sources.values() {
                let text = join(&["source", b.id()], tail07(b, SOURCE_KEYS, &[]));
                self.emit_base(0, &text, b);
            }
        }
        for b in d.bkms.values() {
            self.blank();
            let head = join(&["bkm", b.base.id()], tail07(&b.base, BKM_KEYS, &[]));
            self.boxed(b, 0, head);
        }
        for dec in d.decisions.values() {
            self.blank();
            let head = join(
                &["decision", dec.base.id()],
                tail07(&dec.base, DECISION_KEYS_07, &[]),
            );
            self.boxed(dec, 0, head);
        }
    }

    fn boxed(&mut self, b: &Decision, indent: usize, head: String) {
        self.emit_base(indent, &head, &b.base);
        if b.has_table() {
            self.table07(b, indent + 1);
        }
        for line in &b.lines {
            let mut parts = vec![line.base.kind.clone()];
            if line.base.kind == "entry" {
                parts.push(name_token(line.base.id()));
            }
            let mut rest = tail07(&line.base, BOX_LINE_KEYS, &[]);
            parts.append(&mut rest);
            self.boxed(line, indent + 1, parts.join(" "));
        }
    }

    /// Rows of a decision table or a relation, columns aligned.
    fn table07(&mut self, dec: &Decision, indent: usize) {
        let pad = "  ".repeat(indent);
        let hin: Vec<String> = dec
            .inputs
            .iter()
            .enumerate()
            .map(|(k, (a, b))| header_cell(a, b, dec.input_values.get(k).and_then(Option::as_ref)))
            .collect();
        let hout: Vec<String> = dec
            .outputs
            .iter()
            .enumerate()
            .map(|(k, (a, b))| header_cell(a, b, dec.output_values.get(k).and_then(Option::as_ref)))
            .collect();
        let ann = !dec.relation
            && (dec
                .rules
                .iter()
                .any(|r| r.annotation.as_deref().is_some_and(|a| !a.is_empty()))
                || dec.has_annotations);
        type Row07<'r> = (Vec<String>, Vec<String>, Option<String>, Option<&'r Rule>);
        let mut rows: Vec<Row07> = vec![(
            hin,
            hout,
            ann.then(|| dec.annotation_title.clone().unwrap_or_default()),
            None,
        )];
        for r in &dec.rules {
            rows.push((
                r.inputs.clone(),
                r.outputs.clone(),
                ann.then(|| r.annotation.clone().unwrap_or_default()),
                Some(r),
            ));
        }
        let nin = dec.inputs.len();
        let ncol = nin + dec.outputs.len();
        let len = |s: &str| s.chars().count();
        let mut widths = vec![0usize; ncol];
        let mut awidth = 0usize;
        for (ins, outs, a, _) in &rows {
            for (k, c) in ins.iter().chain(outs.iter()).enumerate() {
                if k < ncol {
                    widths[k] = widths[k].max(len(c));
                }
            }
            if let Some(a) = a {
                awidth = awidth.max(len(a));
            }
        }
        let ljust = |s: &str, w: usize| {
            let n = len(s);
            if n >= w {
                s.to_string()
            } else {
                format!("{s}{}", " ".repeat(w - n))
            }
        };
        for (ins, outs, a, r) in &rows {
            let cells: Vec<&String> = ins.iter().chain(outs.iter()).collect();
            let cell = |k: usize| cells.get(k).map(|s| s.as_str()).unwrap_or("");
            let left: Vec<String> = (0..nin).map(|k| ljust(cell(k), widths[k])).collect();
            let mut text = if dec.relation {
                format!("| {} |", left.join(" | "))
            } else {
                let right: Vec<String> = (nin..ncol).map(|k| ljust(cell(k), widths[k])).collect();
                super::table_row(dec, &left, &right)
            };
            if ann {
                text.push_str(&format!(
                    "| {} |",
                    ljust(a.as_deref().unwrap_or(""), awidth)
                ));
            }
            if let Some(r) = r {
                for c in &r.comments {
                    self.out.push(format!("{pad}{}", super::comment(c)));
                }
                if let Some(t) = r.trailing.as_ref().filter(|t| !t.is_empty()) {
                    text.push_str("  # ");
                    text.push_str(t);
                }
            }
            self.out.push(format!(
                "{pad}{}",
                text.trim_end_matches(crate::pyutil::is_space)
            ));
        }
    }
}
