//! Statements of language 0.7: input data, knowledge sources, business knowledge models,
//! decisions with boxed expressions (`entry`, `result`, `element`, tables and relations)
//! and DMN types with components.

use super::{BoxRef, Ctx, Owner, Parser, Prev, split_row};
use crate::lexer::{Kind, Line};
use crate::model::*;
use crate::pyutil::strip;

/// `name: type` or `name: type = values` of a header cell of language 0.7: the name, the
/// type and the unary tests of the values.
pub fn split_header_cell_07(cell: &str) -> Option<(String, String, Option<String>)> {
    let mut in_str = false;
    let mut eq = None;
    let chars: Vec<char> = cell.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' if in_str => i += 1,
            '"' => in_str = !in_str,
            '=' if !in_str => {
                eq = Some(i);
                break;
            }
            _ => {}
        }
        i += 1;
    }
    let (head, values) = match eq {
        Some(k) => {
            let head: String = chars[..k].iter().collect();
            let values: String = chars[k + 1..].iter().collect();
            (head, Some(strip(&values).to_string()))
        }
        None => (cell.to_string(), None),
    };
    let (name, typ) = head.rsplit_once(':')?;
    let (name, typ) = (strip(name), strip(typ));
    if name.is_empty() || typ.is_empty() || values.as_deref() == Some("") {
        return None;
    }
    Some((name.to_string(), typ.to_string(), values))
}

impl Parser {
    fn box_mut(&mut self, r: &BoxRef) -> Option<&mut Decision> {
        let mut d = if r.bkm {
            self.doc.bkms.get_mut(&r.id)?
        } else {
            self.doc.decisions.get_mut(&r.id)?
        };
        for &k in &r.path {
            d = d.lines.get_mut(k)?;
        }
        Some(d)
    }

    fn box_ctx(owner: Owner) -> Ctx {
        Ctx {
            kind: "box",
            scope: None,
            lane: None,
            pool: None,
            owner,
            choreography: None,
        }
    }

    /// `inputdata` and `source`.
    pub(super) fn st_drg_element(&mut self, line: &Line, kind: &str) -> Prev {
        let Some(id) = self.need_id(line, 1) else {
            return Prev::plain(kind);
        };
        let tail = self.parse_tail(&line.tokens, 2, line.number, true);
        let mut base = Base::new(
            kind,
            Some(id.clone()),
            tail.label,
            tail.attrs,
            tail.flags,
            line.number,
        );
        self.take_comments(&mut base, line);
        if self.register(kind, &id, line.number) {
            if kind == "inputdata" {
                self.doc.inputdata.insert(id, base);
            } else {
                self.doc.sources.insert(id, base);
            }
        }
        Prev::plain(kind)
    }

    /// `decision` of language 0.7 and `bkm`: a box whose block holds its logic.
    pub(super) fn st_box_root(&mut self, line: &Line, bkm: bool) -> Prev {
        let kind = if bkm { "bkm" } else { "decision" };
        let Some(id) = self.need_id(line, 1) else {
            return Prev::plain(kind);
        };
        let tail = self.parse_tail(&line.tokens, 2, line.number, true);
        let mut base = Base::new(
            kind,
            Some(id.clone()),
            tail.label,
            tail.attrs,
            tail.flags,
            line.number,
        );
        self.take_comments(&mut base, line);
        if !self.register(kind, &id, line.number) {
            return Prev {
                kind: kind.into(),
                accepts: true,
                ctx: None,
            };
        }
        let d = Decision {
            base,
            ..Default::default()
        };
        if bkm {
            self.doc.bkms.insert(id.clone(), d);
        } else {
            self.doc.decisions.insert(id.clone(), d);
        }
        Prev {
            kind: kind.into(),
            accepts: true,
            ctx: Some(Parser::box_ctx(Owner::Box(BoxRef {
                bkm,
                id,
                path: Vec::new(),
            }))),
        }
    }

    /// `entry NAME …`, `result …` and `element …` in the block of a box.
    pub(super) fn st_box_line(&mut self, line: &Line, ctx: &Ctx, kind: &str) -> Prev {
        let Owner::Box(parent) = &ctx.owner else {
            return Prev::plain(kind);
        };
        let toks = &line.tokens;
        let (name, start) = if kind == "entry" {
            match toks.get(1) {
                Some(t)
                    if matches!(t.kind, Kind::Word | Kind::Str)
                        && !(t.kind == Kind::Word
                            && toks.get(2).is_some_and(|x| x.kind == Kind::Equals)) =>
                {
                    (Some(t.value.clone()), 2)
                }
                _ => {
                    self.err("P002", line.number, None, &[("keyword", kind.to_string())]);
                    return Prev::plain(kind);
                }
            }
        } else {
            (None, 1)
        };
        let tail = self.parse_tail(toks, start, line.number, false);
        let mut base = Base::new(kind, name, None, tail.attrs, tail.flags, line.number);
        self.take_comments(&mut base, line);
        let parent = parent.clone();
        let Some(b) = self.box_mut(&parent) else {
            return Prev::plain(kind);
        };
        b.lines.push(Decision {
            base,
            ..Default::default()
        });
        let mut path = parent.path.clone();
        path.push(b.lines.len() - 1);
        Prev {
            kind: kind.into(),
            accepts: true,
            ctx: Some(Parser::box_ctx(Owner::Box(BoxRef {
                bkm: parent.bkm,
                id: parent.id,
                path,
            }))),
        }
    }

    /// A row of a table or a relation in the block of a box (language 0.7).
    pub(super) fn row07(&mut self, line: &Line, r: &BoxRef) {
        let Some(cells) = split_row(&line.tokens[0].value) else {
            self.err(
                "P009",
                line.number,
                None,
                &[("detail", "unbalanced quotes".into())],
            );
            return;
        };
        let mut groups: Vec<Vec<String>> = vec![Vec::new()];
        for c in cells {
            if c.is_empty() {
                groups.push(Vec::new());
            } else if let Some(g) = groups.last_mut() {
                g.push(strip(&c).to_string());
            }
        }
        let root = r.id.clone();
        let comments = std::mem::take(&mut self.pending);
        let Some(dec) = self.box_mut(r) else {
            return;
        };
        let relation_header = !dec.header_done && groups.len() == 1;
        if groups.len() > 3 || (dec.header_done && !dec.relation && groups.len() < 2) {
            self.pending = comments;
            self.err(
                "P009",
                line.number,
                None,
                &[(
                    "detail",
                    "expected inputs || outputs [|| annotation]".into(),
                )],
            );
            return;
        }
        if !dec.header_done {
            dec.header_done = true;
            dec.relation = relation_header;
            dec.header_line = Some(line.number);
            dec.no_inputs = groups[0].is_empty();
            dec.no_outputs = groups.len() >= 2 && groups[1].is_empty();
            let mut errs = Vec::new();
            for cell in &groups[0] {
                match split_header_cell_07(cell) {
                    Some((n, t, v)) => {
                        dec.inputs.push((n, t));
                        dec.input_values.push(v);
                    }
                    None => errs.push(cell.clone()),
                }
            }
            if groups.len() >= 2 {
                for cell in &groups[1] {
                    match split_header_cell_07(cell) {
                        Some((n, t, v)) => {
                            dec.outputs.push((n, t));
                            dec.output_values.push(v);
                        }
                        None => errs.push(cell.clone()),
                    }
                }
            }
            dec.has_annotations = groups.len() == 3;
            dec.annotation_title = (groups.len() == 3).then(|| {
                groups[2]
                    .iter()
                    .filter(|x| !x.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ")
            });
            for cell in errs {
                self.err("D009", line.number, Some(&root), &[("cell", cell)]);
            }
            self.pending = comments;
            return;
        }
        if dec.relation {
            if groups.len() != 1 || groups[0].len() != dec.inputs.len() {
                let got = groups.iter().map(Vec::len).sum::<usize>().to_string();
                let expected = dec.inputs.len().to_string();
                self.pending = comments;
                self.err(
                    "D008",
                    line.number,
                    Some(&root),
                    &[("got", got), ("expected", expected)],
                );
                return;
            }
            dec.rules.push(Rule {
                inputs: groups[0].clone(),
                outputs: Vec::new(),
                annotation: None,
                line: line.number,
                comments,
                trailing: line.comment.clone(),
            });
            return;
        }
        let ann: Option<String> = (groups.len() == 3).then(|| {
            groups[2]
                .iter()
                .filter(|x| !x.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join(" ")
        });
        if groups[0].len() != dec.inputs.len() || groups[1].len() != dec.outputs.len() {
            let got = (groups[0].len() + groups[1].len()).to_string();
            let expected = (dec.inputs.len() + dec.outputs.len()).to_string();
            self.pending = comments;
            self.err(
                "D002",
                line.number,
                Some(&root),
                &[("got", got), ("expected", expected)],
            );
            return;
        }
        dec.rules.push(Rule {
            inputs: groups[0].clone(),
            outputs: groups[1].clone(),
            annotation: ann.filter(|a| !a.is_empty()),
            line: line.number,
            comments,
            trailing: line.comment.clone(),
        });
    }

    /// `item` of language 0.7: a BPMN item definition, or a DMN type with an optional
    /// block of components.
    pub(super) fn st_item07(&mut self, line: &Line) -> Prev {
        let Some(id) = self.need_id(line, 1) else {
            return Prev::plain("item");
        };
        let tail = self.parse_tail(&line.tokens, 2, line.number, true);
        let mut base = Base::new(
            "item",
            Some(id.clone()),
            tail.label,
            tail.attrs,
            tail.flags,
            line.number,
        );
        self.take_comments(&mut base, line);
        if !self.register("item", &id, line.number) {
            return Prev {
                kind: "item".into(),
                accepts: true,
                ctx: None,
            };
        }
        self.doc.items.insert(id.clone(), base);
        Prev {
            kind: "item".into(),
            accepts: true,
            ctx: Some(Ctx {
                kind: "itemdef",
                scope: None,
                lane: None,
                pool: None,
                owner: Owner::Item(id, Vec::new()),
                choreography: None,
            }),
        }
    }

    /// `item NAME …` in the block of a DMN type: a component (its name is not a document
    /// identifier).
    pub(super) fn st_component(&mut self, line: &Line, ctx: &Ctx) -> Prev {
        let Owner::Item(root, path) = &ctx.owner else {
            return Prev::plain("item");
        };
        let toks = &line.tokens;
        let name = match toks.get(1) {
            Some(t)
                if matches!(t.kind, Kind::Word | Kind::Str | Kind::Number)
                    && !(t.kind == Kind::Word
                        && toks.get(2).is_some_and(|x| x.kind == Kind::Equals)) =>
            {
                t.value.clone()
            }
            _ => {
                self.err("P002", line.number, None, &[("keyword", "item".into())]);
                return Prev::plain("item");
            }
        };
        let tail = self.parse_tail(toks, 2, line.number, false);
        let mut base = Base::new(
            "component",
            Some(name),
            None,
            tail.attrs,
            tail.flags,
            line.number,
        );
        self.take_comments(&mut base, line);
        let (root, mut path) = (root.clone(), path.clone());
        let list = self.doc.components.entry(root.clone()).or_default();
        let mut cur: &mut Vec<Component> = list;
        for &k in &path {
            let Some(c) = cur.get_mut(k) else {
                return Prev::plain("item");
            };
            cur = &mut c.children;
        }
        cur.push(Component {
            base,
            children: Vec::new(),
        });
        path.push(cur.len() - 1);
        Prev {
            kind: "item".into(),
            accepts: true,
            ctx: Some(Ctx {
                kind: "itemdef",
                scope: None,
                lane: None,
                pool: None,
                owner: Owner::Item(root, path),
                choreography: None,
            }),
        }
    }
}
