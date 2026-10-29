//! Canonical printer (`tok fmt`, spec §14). Mirrors `printer.py`.

use crate::model::*;

#[path = "printer_07.rs"]
mod lang07;

const QUOTED_KEYS: &[&str] = &[
    "when",
    "expr",
    "condition",
    "correlation",
    "duration",
    "assign",
    "delay",
    "arrival",
    "options",
    "members",
    "message",
    "signal",
    "error",
    "escalation",
    "link",
    "code",
    "start",
    // language 0.3: expressions, lists and values of the executable layer
    "collection",
    "output-item",
    "until",
    "active",
    "inputs",
    "outputs",
    "headers",
    "listeners",
    "events",
    "url",
    "body",
    "query",
    "headers-in",
    "assignee",
    "candidate-users",
    "candidate-groups",
    "due",
    "follow-up",
    "business-id",
    "version",
    "version-tag",
    // language 0.5
    "while",
    "activation",
    "structure",
    "in",
    "out",
    "path",
    "doc",
    "process-name",
    "object",
    // language 0.6
    "reply",
    "pools",
    "messages",
    "map",
];

const EXECUTION_LISTENER_EVENTS: &[&str] = &["start", "end"];

/// Execution listeners first, then task listeners, each group in written order (canonical form).
pub fn normalize_listeners(text: &str) -> String {
    let items: Vec<&str> = text
        .split(';')
        .map(crate::pyutil::strip)
        .filter(|x| !x.is_empty())
        .collect();
    let event = |x: &str| crate::pyutil::strip(x.split(':').next().unwrap_or("")).to_string();
    let first: Vec<&str> = items
        .iter()
        .copied()
        .filter(|x| EXECUTION_LISTENER_EVENTS.contains(&event(x).as_str()))
        .collect();
    let rest: Vec<&str> = items
        .iter()
        .copied()
        .filter(|x| !first.contains(x))
        .collect();
    let mut all = first;
    all.extend(rest);
    all.join("; ")
}

fn bare_core(v: &str) -> bool {
    let ch: Vec<char> = v.chars().collect();
    if ch.is_empty() {
        return false;
    }
    let rest_ok = |s: &[char]| {
        s.iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '/' | '+' | '-'))
    };
    // [A-Za-z_][A-Za-z0-9_.:/+-]*
    if (ch[0].is_ascii_alphabetic() || ch[0] == '_') && rest_ok(&ch[1..]) {
        return true;
    }
    // [0-9][A-Za-z0-9_.:/+-]*
    if ch[0].is_ascii_digit() && rest_ok(&ch[1..]) {
        return true;
    }
    // -?[0-9]+(\.[0-9]+)?
    let body = if ch[0] == '-' { &ch[1..] } else { &ch[..] };
    let mut i = 0;
    while i < body.len() && body[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 {
        return false;
    }
    if i == body.len() {
        return true;
    }
    if body[i] != '.' {
        return false;
    }
    let frac = &body[i + 1..];
    !frac.is_empty() && frac.iter().all(|c| c.is_ascii_digit())
}

/// Python `_BARE.match(v)` (`$` also matches before a final newline).
fn bare(v: &str) -> bool {
    bare_core(v) || v.strip_suffix('\n').map(bare_core).unwrap_or(false)
}

/// Quoted string with tok escapes.
pub fn q(text: &str) -> String {
    format!(
        "\"{}\"",
        text.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    )
}

fn value(key: &str, v: &str) -> String {
    let normalized;
    let v = if key == "listeners" {
        normalized = normalize_listeners(v);
        normalized.as_str()
    } else {
        v
    };
    if QUOTED_KEYS.contains(&key) || !bare(v) || v.contains("->") {
        q(v)
    } else {
        v.to_string()
    }
}

fn tail(el: &Base, keys: &[&str], flags: &[&str], quote_type: bool) -> Vec<String> {
    let mut parts = Vec::new();
    if let Some(l) = &el.label {
        parts.push(q(l));
    }
    for f in flags {
        if el.has(f) {
            parts.push((*f).to_string());
        }
    }
    for f in &el.flags {
        if !flags.contains(&f.as_str()) {
            parts.push(f.clone());
        }
    }
    let repeated = |parts: &mut Vec<String>, k: &str| {
        for (_, v) in el.extra.iter().filter(|(x, _)| x == k) {
            parts.push(format!("{k}={}", value(k, v)));
        }
    };
    for k in keys {
        if let Some(v) = el.attrs.get(*k) {
            let shown = if *k == "type" && quote_type {
                q(v)
            } else {
                value(k, v)
            };
            parts.push(format!("{k}={shown}"));
            repeated(&mut parts, k);
        }
    }
    let mut rest: Vec<&String> = el
        .attrs
        .keys()
        .filter(|k| !keys.contains(&k.as_str()))
        .collect();
    rest.sort();
    for k in rest {
        parts.push(format!("{k}={}", value(k, &el.attrs[k])));
        repeated(&mut parts, k);
    }
    parts
}

/// Attribute keys and flags of a data statement in canonical order.
fn data_tables(kind: &str) -> (&'static [&'static str], &'static [&'static str]) {
    if IO_KINDS.contains(&kind) {
        (IO_KEYS, IO_FLAGS)
    } else {
        (DATA_KEYS, DATA_FLAGS)
    }
}

fn comment(c: &str) -> String {
    if c.is_empty() {
        "#".to_string()
    } else {
        format!("# {c}")
    }
}

/// One decision table row: inputs, outputs, annotation and the rule it prints.
type Row<'r> = (Vec<String>, Vec<String>, Option<String>, Option<&'r Rule>);

struct Printer<'a> {
    doc: &'a Document,
    out: Vec<String>,
}

/// Canonical text of a parsed document.
pub fn fmt(doc: &Document) -> String {
    let mut p = Printer {
        doc,
        out: Vec::new(),
    };
    p.run()
}

fn join(head: &[&str], rest: Vec<String>) -> String {
    let mut v: Vec<String> = head.iter().map(|s| s.to_string()).collect();
    v.extend(rest);
    v.join(" ")
}

impl Printer<'_> {
    fn emit(&mut self, indent: usize, text: &str, el: Option<(&[String], Option<&String>)>) {
        let pad = "  ".repeat(indent);
        let mut line = format!("{pad}{text}");
        if let Some((comments, trailing)) = el {
            for c in comments {
                self.out.push(format!("{pad}{}", comment(c)));
            }
            if let Some(t) = trailing.filter(|t| !t.is_empty()) {
                line.push_str("  # ");
                line.push_str(t);
            }
        }
        self.out.push(line);
    }

    fn emit_base(&mut self, indent: usize, text: &str, b: &Base) {
        self.emit(indent, text, Some((&b.comments, b.trailing.as_ref())));
    }

    fn blank(&mut self) {
        if self.out.last().map(|l| !l.is_empty()).unwrap_or(false) {
            self.out.push(String::new());
        }
    }

    fn run(&mut self) -> String {
        let d = self.doc;
        for c in &d.header_comments {
            self.out.push(comment(c));
        }
        let mut head = vec![
            "tok".to_string(),
            d.version.clone().unwrap_or_else(|| "0.1".into()),
        ];
        if let Some(t) = &d.title {
            head.push(q(t));
        }
        for f in HEADER_FLAGS {
            if d.flags.iter().any(|x| x == f) {
                head.push((*f).to_string());
            }
        }
        for f in &d.flags {
            if !HEADER_FLAGS.contains(&f.as_str()) {
                head.push(f.clone());
            }
        }
        for k in HEADER_KEYS {
            if let Some(v) = d.attrs.get(*k) {
                head.push(format!("{k}={}", value(k, v)));
            }
        }
        let mut rest: Vec<&String> = d
            .attrs
            .keys()
            .filter(|k| !HEADER_KEYS.contains(&k.as_str()))
            .collect();
        rest.sort();
        for k in rest {
            head.push(format!("{k}={}", value(k, &d.attrs[k])));
        }
        let mut line = head.join(" ");
        if let Some(t) = d.header_trailing.as_ref().filter(|t| !t.is_empty()) {
            line.push_str("  # ");
            line.push_str(t);
        }
        self.out.push(line);

        if !d.items.is_empty() || !d.globals.is_empty() {
            self.blank();
            for it in d.items.values() {
                let keys = if d.lang_07() { ITEM_KEYS_07 } else { ITEM_KEYS };
                self.emit_base(
                    0,
                    &join(&["item", it.id()], tail(it, keys, ITEM_FLAGS, false)),
                    it,
                );
                if let Some(list) = d.components.get(it.id()) {
                    self.components(list, 1);
                }
            }
            for g in d.globals.values() {
                self.emit_base(
                    0,
                    &join(&["global", g.id()], tail(g, GLOBAL_KEYS, &[], false)),
                    g,
                );
            }
        }
        for x in d.interfaces.values() {
            self.blank();
            self.emit_base(
                0,
                &join(
                    &["interface", x.base.id()],
                    tail(&x.base, INTERFACE_KEYS, &[], false),
                ),
                &x.base,
            );
            for op in &x.operations {
                self.emit_base(
                    1,
                    &join(
                        &["operation", op.id()],
                        tail(op, OPERATION_KEYS, &[], false),
                    ),
                    op,
                );
            }
        }

        if !d.top.is_empty() {
            self.blank();
            self.children(&d.top, 0);
        }
        for p in &d.pools {
            self.blank();
            self.emit_base(
                0,
                &join(
                    &["pool", p.base.id()],
                    tail(&p.base, d.pool_keys(), POOL_FLAGS, false),
                ),
                &p.base,
            );
            self.children(&p.children, 1);
        }
        // language 0.6: each choreography with its nodes and its sequence flows
        for c in d.choreographies.values() {
            self.blank();
            self.emit_base(
                0,
                &join(
                    &["choreography", c.base.id()],
                    tail(&c.base, CHOREOGRAPHY_KEYS, &[], false),
                ),
                &c.base,
            );
            self.children(&c.children, 1);
            let own: Vec<&Flow> = d
                .flows
                .iter()
                .filter(|f| self.flow_choreography(f) == Some(c.base.id()))
                .collect();
            self.flows(&own, 1);
        }
        let process_flows: Vec<&Flow> = d
            .flows
            .iter()
            .filter(|f| self.flow_choreography(f).is_none())
            .collect();
        if !process_flows.is_empty() {
            self.blank();
            self.flows(&process_flows, 0);
        }
        // message flows named by a conversation print their identifier (language 0.6)
        let referenced: std::collections::HashSet<String> = d
            .conversations
            .values()
            .flat_map(|c| words(c.base.attr("messages").unwrap_or("")))
            .collect();
        if !d.messages.is_empty() {
            self.blank();
            for m in &d.messages {
                let mut parts = vec![
                    "message".to_string(),
                    m.source.clone(),
                    "->".into(),
                    m.target.clone(),
                ];
                parts.extend(tail(&m.base, MESSAGE_KEYS, &[], false));
                if m.explicit_id || referenced.contains(m.base.id()) {
                    parts.push(format!("id={}", m.base.id()));
                }
                self.emit_base(0, &parts.join(" "), &m.base);
            }
        }
        let top: Vec<&String> = d
            .conversations
            .iter()
            .filter(|(_, c)| c.parent.is_none())
            .map(|(k, _)| k)
            .collect();
        if !top.is_empty() {
            self.blank();
            for k in top {
                self.conversation(k, 0);
            }
        }
        for k in d.keys.values() {
            self.blank();
            self.emit_base(
                0,
                &join(&["key", k.base.id()], tail(&k.base, &[], &[], false)),
                &k.base,
            );
            for p in &k.props {
                self.emit_base(
                    1,
                    &join(&["prop", p.id()], tail(p, PROP_KEYS, &[], false)),
                    p,
                );
            }
        }
        if !d.assocs.is_empty() {
            self.blank();
            for a in &d.assocs {
                let keys = if a.base.kind == "reads" {
                    READS_KEYS
                } else {
                    WRITES_KEYS
                };
                let text = join(
                    &[a.base.kind.as_str(), a.activity.as_str(), a.data.as_str()],
                    tail(&a.base, keys, &[], false),
                );
                self.emit_base(0, &text, &a.base);
            }
        }
        if !d.notes.is_empty() || !d.groups.is_empty() {
            self.blank();
            for n in &d.notes {
                self.emit_base(
                    0,
                    &join(&["note", n.id()], tail(n, NOTE_KEYS, &[], false)),
                    n,
                );
            }
            for g in &d.groups {
                self.emit_base(
                    0,
                    &join(&["group", g.id()], tail(g, GROUP_KEYS, &[], false)),
                    g,
                );
            }
        }
        if d.lang_07() {
            self.drg();
        } else {
            for dec in d.decisions.values() {
                self.blank();
                self.decision(dec);
            }
        }
        for f in d.forms.values() {
            self.blank();
            self.emit_base(
                0,
                &join(&["form", f.base.id()], tail(&f.base, &[], &[], false)),
                &f.base,
            );
            for it in &f.items {
                if it.base.kind == "text" {
                    let text = format!("text {}", q(it.base.label.as_deref().unwrap_or("")));
                    self.emit_base(1, &text, &it.base);
                } else {
                    let text = join(
                        &["field", it.base.id()],
                        tail(&it.base, FIELD_KEYS, FIELD_FLAGS, false),
                    );
                    self.emit_base(1, &text, &it.base);
                }
            }
        }
        if d.simulate.is_some() || !d.resources.is_empty() || !d.variables.is_empty() {
            self.blank();
            if let Some(s) = &d.simulate {
                self.emit_base(
                    0,
                    &join(&["simulate"], tail(s, SIMULATE_KEYS, &[], false)),
                    s,
                );
            }
            for r in d.resources.values() {
                self.emit_base(
                    0,
                    &join(&["resource", r.id()], tail(r, RESOURCE_KEYS, &[], false)),
                    r,
                );
            }
            for v in &d.variables {
                let text = format!(
                    "variable {} {}",
                    v.id(),
                    q(v.label.as_deref().unwrap_or(""))
                );
                self.emit_base(0, &text, v);
            }
        }
        if !d.end_comments.is_empty() {
            self.blank();
            for c in &d.end_comments {
                self.out.push(comment(c));
            }
        }
        while self.out.last().map(|l| l.is_empty()).unwrap_or(false) {
            self.out.pop();
        }
        format!("{}\n", self.out.join("\n"))
    }

    fn node(&mut self, n: &Node, indent: usize) {
        let kind = n.base.kind.as_str();
        let text = join(
            &[kind, n.base.id()],
            tail(&n.base, node_keys(kind), node_flags(kind), true),
        );
        self.emit_base(indent, &text, &n.base);
        if is_sub(kind) || kind == "subchoreo" {
            self.children(&n.children, indent + 1);
        }
    }

    /// A conversation node and, for a sub-conversation, the nodes of its block.
    fn conversation(&mut self, id: &str, indent: usize) {
        let d = self.doc;
        let Some(c) = d.conversations.get(id) else {
            return;
        };
        let kind = c.base.kind.as_str();
        let text = join(
            &[kind, c.base.id()],
            tail(&c.base, conversation_keys(kind), &[], false),
        );
        self.emit_base(indent, &text, &c.base);
        for k in &c.children {
            self.conversation(k, indent + 1);
        }
    }

    /// The choreography a sequence flow belongs to: that of its source, else of its target.
    fn flow_choreography(&self, f: &Flow) -> Option<&str> {
        let d = self.doc;
        let of = |id: &str| d.nodes.get(id).and_then(|n| n.choreography.as_deref());
        match d.nodes.get(&f.source) {
            Some(n) => n.choreography.as_deref(),
            None => of(&f.target),
        }
    }

    fn children(&mut self, ids: &[String], indent: usize) {
        let d = self.doc;
        let mut datas = Vec::new();
        for i in ids {
            if let Some(n) = d.nodes.get(i) {
                self.node(n, indent);
            } else if let Some(l) = d.lanes.get(i) {
                self.emit_base(
                    indent,
                    &join(&["lane", l.base.id()], tail(&l.base, LANE_KEYS, &[], false)),
                    &l.base,
                );
                self.children(&l.children, indent + 1);
            } else if let Some(x) = d.data.get(i) {
                datas.push(x);
            }
        }
        for x in datas {
            let (keys, flags) = data_tables(&x.base.kind);
            let text = join(
                &[x.base.kind.as_str(), x.base.id()],
                tail(&x.base, keys, flags, false),
            );
            self.emit_base(indent, &text, &x.base);
        }
    }

    fn flows(&mut self, flows: &[&Flow], indent: usize) {
        let mut out_count: std::collections::HashMap<&str, usize> = Default::default();
        for f in flows {
            *out_count.entry(f.source.as_str()).or_insert(0) += 1;
        }
        let plain = |f: &Flow| {
            f.base.label.is_none()
                && f.base.attrs.is_empty()
                && f.base.flags.is_empty()
                && !f.explicit_id
        };
        let mut chain: Option<(Vec<String>, &Flow)> = None;
        for &f in flows {
            if let Some((c, _)) = &mut chain
                && plain(f)
                && c.last() == Some(&f.source)
                && out_count.get(f.source.as_str()).copied().unwrap_or(0) == 1
                && f.base.comments.is_empty()
                && f.base.trailing.as_deref().unwrap_or("").is_empty()
            {
                c.push(f.target.clone());
                continue;
            }
            if let Some((c, el)) = chain.take() {
                self.emit_base(indent, &format!("flow {}", c.join(" -> ")), &el.base);
            }
            if plain(f) {
                chain = Some((vec![f.source.clone(), f.target.clone()], f));
            } else {
                let mut parts = vec![
                    "flow".to_string(),
                    f.source.clone(),
                    "->".into(),
                    f.target.clone(),
                ];
                parts.extend(tail(&f.base, FLOW_KEYS, FLOW_FLAGS, false));
                if f.explicit_id {
                    parts.push(format!("id={}", f.base.id()));
                }
                self.emit_base(indent, &parts.join(" "), &f.base);
            }
        }
        if let Some((c, el)) = chain {
            self.emit_base(indent, &format!("flow {}", c.join(" -> ")), &el.base);
        }
    }

    fn decision(&mut self, dec: &Decision) {
        let head = join(
            &["decision", dec.base.id()],
            tail(&dec.base, DECISION_KEYS, &[], false),
        );
        self.emit_base(0, &head, &dec.base);
        let hin: Vec<String> = dec
            .inputs
            .iter()
            .map(|(a, b)| format!("{a}: {b}"))
            .collect();
        let hout: Vec<String> = dec
            .outputs
            .iter()
            .map(|(a, b)| format!("{a}: {b}"))
            .collect();
        let ann = dec.rules.iter().any(|r| {
            r.annotation
                .as_deref()
                .map(|a| !a.is_empty())
                .unwrap_or(false)
        }) || dec.has_annotations;
        let mut rows: Vec<Row> = vec![(
            hin,
            hout,
            if dec.has_annotations {
                Some(dec.annotation_title.clone().unwrap_or_default())
            } else {
                None
            },
            None,
        )];
        for r in &dec.rules {
            rows.push((
                r.inputs.clone(),
                r.outputs.clone(),
                if ann {
                    Some(r.annotation.clone().unwrap_or_default())
                } else {
                    None
                },
                Some(r),
            ));
        }
        let nin = dec.inputs.len();
        let ncol = nin + dec.outputs.len();
        let mut widths = vec![0usize; ncol];
        let mut awidth = 0usize;
        let len = |s: &str| s.chars().count();
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
            let right: Vec<String> = (nin..ncol).map(|k| ljust(cell(k), widths[k])).collect();
            let mut text = table_row(dec, &left, &right);
            if ann {
                text.push_str(&format!(
                    "| {} |",
                    ljust(a.as_deref().unwrap_or(""), awidth)
                ));
            }
            if let Some(r) = r {
                for c in &r.comments {
                    self.out.push(format!("  # {c}"));
                }
                if let Some(t) = r.trailing.as_ref().filter(|t| !t.is_empty()) {
                    text.push_str("  # ");
                    text.push_str(t);
                }
            }
            self.out.push(format!(
                "  {}",
                text.trim_end_matches(crate::pyutil::is_space)
            ));
        }
    }
}

/// A row of a decision table: the input cells, `||`, the output cells. A side the header
/// has no cells on is left out (`|| out |`, `| in ||`), so that the row reads back the same.
pub(crate) fn table_row(dec: &Decision, left: &[String], right: &[String]) -> String {
    let mut text = if dec.no_inputs && left.is_empty() {
        "||".to_string()
    } else {
        format!("| {} ||", left.join(" | "))
    };
    if !(dec.no_outputs && right.is_empty()) {
        text.push_str(&format!(" {} |", right.join(" | ")));
    }
    text
}
