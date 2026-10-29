//! Parser: tokens to [`Document`]. Mirrors `parser.py` statement by statement.

use crate::diagnostics::Diag;
use crate::lexer::{self, Kind, Line, Token};
use crate::model::*;
use crate::pyutil::strip;

#[path = "parser_07.rs"]
mod lang07;
pub use lang07::split_header_cell_07;

const NODE_LIKE_EXTRA: &[&str] = &[
    "flow", "message", "data", "store", "reads", "writes", "note", "input", "output", "property",
];

fn allowed(ctx: &str, kw: &str) -> bool {
    let node_like = is_node(kw) || NODE_LIKE_EXTRA.contains(&kw);
    match ctx {
        "top" => {
            node_like
                || [
                    "pool",
                    "group",
                    "decision",
                    "form",
                    "simulate",
                    "resource",
                    "variable",
                    "item",
                    "global",
                    "interface",
                    "key",
                    "conversation",
                    "subconversation",
                    "callconversation",
                    "choreography",
                    "inputdata",
                    "bkm",
                    "source",
                ]
                .contains(&kw)
        }
        // language 0.7: the block of a decision, a business knowledge model, an entry, a
        // result or an element, and the components of a DMN type
        "box" => ["entry", "result", "element"].contains(&kw),
        "itemdef" => kw == "item",
        // language 0.6: the block of a sub-conversation, of a choreography and of a
        // sub-choreography (the checker reports flow nodes a choreography cannot contain)
        "subconversation" => is_conversation(kw),
        "choreography" | "subchoreo" => is_node(kw) || is_choreo(kw) || kw == "flow",
        "pool" | "lane" => node_like || kw == "lane",
        "sub" | "eventsub" | "adhoc" | "transaction" => node_like,
        "form" => kw == "text" || kw == "field",
        "interface" => kw == "operation",
        "key" => kw == "prop",
        _ => false,
    }
}

fn is_keyword(kw: &str) -> bool {
    kw == "tok"
        || allowed("top", kw)
        || allowed("pool", kw)
        || allowed("form", kw)
        || allowed("interface", kw)
        || allowed("key", kw)
        || allowed("choreography", kw)
        || allowed("box", kw)
}

/// A decision or a business knowledge model and the path of nested lines to one box of
/// it (language 0.7).
#[derive(Debug, Clone)]
struct BoxRef {
    bkm: bool,
    id: String,
    path: Vec<usize>,
}

/// `[A-Za-z_][A-Za-z0-9_-]*`
pub fn is_id(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[derive(Debug, Clone)]
enum Owner {
    None,
    Pool(String),
    Lane(String),
    Node(String),
    Decision(String),
    Form(String),
    Interface(String),
    Key(String),
    Conversation(String),
    Choreography(String),
    /// Language 0.7: a box of a decision or a business knowledge model.
    Box(BoxRef),
    /// Language 0.7: a DMN type and the path to a component of it.
    Item(String, Vec<usize>),
}

#[derive(Debug, Clone)]
struct Ctx {
    kind: &'static str,
    scope: Option<String>,
    lane: Option<String>,
    pool: Option<String>,
    owner: Owner,
    /// The choreography whose block this is (language 0.6).
    choreography: Option<String>,
}

impl Ctx {
    fn owner_id(&self) -> Option<&str> {
        match &self.owner {
            Owner::Pool(i)
            | Owner::Lane(i)
            | Owner::Node(i)
            | Owner::Decision(i)
            | Owner::Form(i)
            | Owner::Interface(i)
            | Owner::Key(i)
            | Owner::Conversation(i)
            | Owner::Choreography(i)
            | Owner::Item(i, _) => Some(i),
            Owner::Box(b) => Some(&b.id),
            Owner::None => None,
        }
    }
}

/// Result of one statement: element kind, whether it accepts a block, and the block context.
struct Prev {
    kind: String,
    accepts: bool,
    ctx: Option<Ctx>,
}

impl Prev {
    fn plain(kind: &str) -> Self {
        Prev {
            kind: kind.to_string(),
            accepts: false,
            ctx: None,
        }
    }
}

struct Tail {
    label: Option<String>,
    attrs: Attrs,
    extra: Vec<(String, String)>,
    flags: Vec<String>,
}

struct Parser {
    doc: Document,
    diags: Vec<Diag>,
    lines: Vec<Line>,
    pending: Vec<String>,
    counter: std::collections::HashMap<String, usize>,
    /// Blocks open around the line parsed.
    depth: usize,
}

/// The deepest nesting of blocks the parser reads. Everything that walks a document
/// (checker, printer, exporters, layout) recurses into blocks, so the nesting is bounded
/// to keep them within the stack; deeper blocks are `P008`.
pub const MAX_BLOCK_DEPTH: usize = 64;

/// Parse tok text. Returns the document and parser diagnostics.
pub fn parse(text: &str) -> (Document, Vec<Diag>) {
    let (lines, errors) = lexer::tokenize(text);
    let mut p = Parser {
        doc: Document::default(),
        diags: Vec::new(),
        lines,
        pending: Vec::new(),
        counter: Default::default(),
        depth: 0,
    };
    for (code, line, msg) in errors {
        p.diags
            .push(Diag::new(code, line, None, &[("detail", msg)]));
    }
    p.run();
    (p.doc, p.diags)
}

impl Parser {
    fn err(&mut self, code: &str, line: usize, id: Option<&str>, args: &[(&str, String)]) {
        self.diags.push(Diag::new(code, line, id, args));
    }

    /// Note a key or flag given again on line `line_no`; the checker reports it in a
    /// document of language 1.0 (`R090`).
    fn repeated(&mut self, line_no: usize, key: &str) {
        let entry = (line_no, key.to_string());
        if !self.doc.repeated.contains(&entry) {
            self.doc.repeated.push(entry);
        }
    }

    fn take_comments(&mut self, base: &mut Base, line: &Line) {
        base.comments = std::mem::take(&mut self.pending);
        base.trailing = line.comment.clone();
    }

    fn register(&mut self, kind: &str, id: &str, line_no: usize) -> bool {
        if !is_id(id) {
            self.err("R004", line_no, Some(id), &[("id", id.to_string())]);
        }
        if let Some(other) = self.doc.ids.get(id) {
            let other = other.line.to_string();
            self.err(
                "R001",
                line_no,
                Some(id),
                &[("id", id.to_string()), ("other", other)],
            );
            return false;
        }
        self.doc.ids.insert(
            id.to_string(),
            IdEntry {
                kind: kind.to_string(),
                line: line_no,
            },
        );
        true
    }

    /// Label, attributes and flags; a repeated key replaces the earlier value.
    fn parse_tail(
        &mut self,
        toks: &[Token],
        start: usize,
        line_no: usize,
        allow_label: bool,
    ) -> Tail {
        self.parse_tail_with(toks, start, line_no, allow_label, false)
    }

    /// Like `parse_tail`, but repeated `REPEATABLE_KEYS` keep every value (language 0.5).
    fn parse_tail_multi(&mut self, toks: &[Token], start: usize, line_no: usize) -> Tail {
        self.parse_tail_with(toks, start, line_no, true, true)
    }

    fn parse_tail_with(
        &mut self,
        toks: &[Token],
        start: usize,
        line_no: usize,
        allow_label: bool,
        repeat: bool,
    ) -> Tail {
        let mut tail = Tail {
            label: None,
            attrs: Attrs::new(),
            extra: Vec::new(),
            flags: Vec::new(),
        };
        let mut i = start;
        if allow_label && i < toks.len() && toks[i].kind == Kind::Str {
            tail.label = Some(toks[i].value.clone());
            i += 1;
        }
        while i < toks.len() {
            let t = &toks[i];
            if t.kind == Kind::Word && i + 1 < toks.len() && toks[i + 1].kind == Kind::Equals {
                if i + 2 >= toks.len()
                    || !matches!(toks[i + 2].kind, Kind::Word | Kind::Number | Kind::Str)
                {
                    let near = t.value.clone();
                    self.err("P007", line_no, None, &[("near", near)]);
                    return tail;
                }
                if repeat
                    && REPEATABLE_KEYS.contains(&t.value.as_str())
                    && tail.attrs.contains_key(&t.value)
                {
                    tail.extra
                        .push((t.value.clone(), toks[i + 2].value.clone()));
                } else {
                    if tail.attrs.contains_key(&t.value) {
                        self.repeated(line_no, &t.value);
                    }
                    tail.attrs
                        .insert(t.value.clone(), toks[i + 2].value.clone());
                }
                i += 3;
            } else if t.kind == Kind::Word {
                if tail.flags.contains(&t.value) {
                    self.repeated(line_no, &t.value);
                }
                tail.flags.push(t.value.clone());
                i += 1;
            } else {
                let near = t.value.clone();
                self.err("P007", line_no, None, &[("near", near)]);
                return tail;
            }
        }
        tail
    }

    fn run(&mut self) {
        let mut i = 0;
        while i < self.lines.len() && self.lines[i].standalone {
            let c = self.lines[i].comment.clone().unwrap_or_default();
            self.pending.push(c);
            i += 1;
        }
        let is_header = i < self.lines.len()
            && self.lines[i]
                .tokens
                .first()
                .map(|t| t.value == "tok")
                .unwrap_or(false);
        if !is_header {
            let line = if i < self.lines.len() {
                self.lines[i].number
            } else {
                1
            };
            self.err("P005", line, None, &[]);
        } else {
            let line = self.lines[i].clone();
            self.header(&line);
            i += 1;
        }
        let ctx = Ctx {
            kind: "top",
            scope: Some(self.doc.default_process_id()),
            lane: None,
            pool: None,
            owner: Owner::None,
            choreography: None,
        };
        self.block(i, -1, &ctx);
        self.doc.end_comments = std::mem::take(&mut self.pending);
    }

    fn block(&mut self, mut i: usize, parent_indent: i64, ctx: &Ctx) -> usize {
        let mut indent: Option<usize> = None;
        let mut prev: Option<Prev> = None;
        while i < self.lines.len() {
            let line = self.lines[i].clone();
            if line.standalone {
                self.pending.push(line.comment.clone().unwrap_or_default());
                i += 1;
                continue;
            }
            if (line.indent as i64) <= parent_indent {
                return i;
            }
            let ind = *indent.get_or_insert(line.indent);
            if line.indent > ind {
                match prev.take() {
                    Some(p) if p.accepts && p.ctx.is_none() => {
                        i = self.skip_block(i, ind);
                        prev = Some(Prev::plain(&p.kind));
                    }
                    Some(p) if p.accepts && self.depth >= MAX_BLOCK_DEPTH => {
                        let detail =
                            format!("its block is nested more than {MAX_BLOCK_DEPTH} levels deep");
                        self.err(
                            "P008",
                            line.number,
                            None,
                            &[("keyword", p.kind.clone()), ("detail", detail)],
                        );
                        i = self.skip_block(i, ind);
                        prev = Some(Prev::plain(&p.kind));
                    }
                    Some(p) if p.accepts => {
                        let child = p.ctx.clone().unwrap_or_else(|| ctx.clone());
                        if p.kind == "pool"
                            && let Some(pid) = child.owner_id().map(str::to_string)
                            && let Some(pool) = self.doc.pool_mut(&pid)
                        {
                            pool.blackbox = false;
                        }
                        self.depth += 1;
                        i = self.block(i, ind as i64, &child);
                        self.depth -= 1;
                        prev = Some(Prev::plain(&p.kind));
                    }
                    other => {
                        let kw = other
                            .as_ref()
                            .map(|p| p.kind.clone())
                            .unwrap_or_else(|| "?".to_string());
                        self.err("P003", line.number, None, &[("keyword", kw)]);
                        i = self.skip_block(i, ind);
                        prev = other.map(|p| Prev {
                            kind: p.kind,
                            accepts: p.accepts,
                            ctx: p.ctx,
                        });
                    }
                }
                continue;
            }
            if line.indent < ind {
                self.err("T003", line.number, None, &[]);
                return i;
            }
            prev = Some(self.statement(&line, ctx));
            i += 1;
        }
        i
    }

    fn skip_block(&self, mut i: usize, indent: usize) -> usize {
        while i < self.lines.len() && (self.lines[i].standalone || self.lines[i].indent > indent) {
            i += 1;
        }
        i
    }

    fn header(&mut self, line: &Line) {
        let toks = &line.tokens;
        self.doc.header_line = line.number;
        self.doc.header_comments = std::mem::take(&mut self.pending);
        self.doc.header_trailing = line.comment.clone();
        if toks.len() < 2 || toks[1].kind != Kind::Number {
            self.err("P005", line.number, None, &[]);
            return;
        }
        let version = toks[1].value.clone();
        if !LANGUAGE_VERSIONS.contains(&version.as_str()) {
            self.err("P006", line.number, None, &[("version", version.clone())]);
        }
        self.doc.version = Some(version);
        let tail = self.parse_tail(toks, 2, line.number, true);
        self.doc.title = tail.label;
        self.doc.attrs = tail.attrs;
        self.doc.flags = tail.flags;
    }

    fn statement(&mut self, line: &Line, ctx: &Ctx) -> Prev {
        let first = &line.tokens[0];
        if first.kind == Kind::Row {
            if ctx.kind == "decision" {
                if let Owner::Decision(did) = &ctx.owner {
                    let did = did.clone();
                    self.row(line, &did);
                }
            } else if ctx.kind == "box" {
                if let Owner::Box(b) = &ctx.owner {
                    let b = b.clone();
                    self.row07(line, &b);
                }
            } else {
                self.err(
                    "P004",
                    line.number,
                    None,
                    &[("keyword", "|".into()), ("context", ctx.kind.into())],
                );
            }
            return Prev::plain("row");
        }
        let kw = if first.kind == Kind::Word {
            first.value.as_str()
        } else {
            ""
        };
        if first.kind == Kind::Word && kw == "tok" {
            self.err("P010", line.number, None, &[]);
            return Prev::plain("tok");
        }
        if first.kind != Kind::Word
            || !is_keyword(kw)
            || (KEYWORDS_07.contains(&kw) && !self.doc.lang_07())
        {
            self.err("P001", line.number, None, &[("word", first.value.clone())]);
            return Prev::plain("?");
        }
        if !allowed(ctx.kind, kw) {
            self.err(
                "P004",
                line.number,
                None,
                &[("keyword", kw.to_string()), ("context", ctx.kind.into())],
            );
            return Prev::plain(kw);
        }
        match kw {
            "pool" => self.st_pool(line),
            "lane" => self.st_lane(line, ctx),
            "flow" => self.st_flow(line),
            "message" => self.st_message(line),
            "data" | "store" | "input" | "output" | "property" => self.st_data(line, ctx, kw),
            "item" if ctx.kind == "itemdef" => self.st_component(line, ctx),
            "item" if self.doc.lang_07() => self.st_item07(line),
            "item" | "global" => self.st_declaration(line, kw),
            "inputdata" | "source" => self.st_drg_element(line, kw),
            "bkm" => self.st_box_root(line, true),
            "decision" if self.doc.lang_07() => self.st_box_root(line, false),
            "entry" | "result" | "element" => self.st_box_line(line, ctx, kw),
            "interface" | "key" => self.st_block_declaration(line, kw),
            "operation" | "prop" => self.st_member(line, ctx, kw),
            "conversation" | "subconversation" | "callconversation" => {
                self.st_conversation(line, ctx, kw)
            }
            "choreography" => self.st_choreography(line),
            "reads" | "writes" => self.st_assoc(line, kw),
            "note" | "group" => self.st_note_group(line, kw),
            "decision" => self.st_decision(line),
            "form" => self.st_form(line),
            "text" => self.st_text(line, ctx),
            "field" => self.st_field(line, ctx),
            "simulate" => self.st_simulate(line),
            "resource" => self.st_resource(line),
            "variable" => self.st_variable(line),
            _ => self.st_node(line, ctx),
        }
    }

    fn need_id(&mut self, line: &Line, pos: usize) -> Option<String> {
        let toks = &line.tokens;
        if toks.len() <= pos || !matches!(toks[pos].kind, Kind::Word | Kind::Number) {
            let kw = toks[0].value.clone();
            self.err("P002", line.number, None, &[("keyword", kw)]);
            return None;
        }
        Some(toks[pos].value.clone())
    }

    fn add_child(&mut self, ctx: &Ctx, id: &str) {
        match (&ctx.kind, &ctx.owner) {
            (&"top", _) => self.doc.top.push(id.to_string()),
            (_, Owner::Pool(p)) => {
                let p = p.clone();
                if let Some(pool) = self.doc.pool_mut(&p) {
                    pool.children.push(id.to_string());
                }
            }
            (_, Owner::Lane(l)) => {
                if let Some(lane) = self.doc.lanes.get_mut(l) {
                    lane.children.push(id.to_string());
                }
            }
            (_, Owner::Node(n)) => {
                if let Some(node) = self.doc.nodes.get_mut(n) {
                    node.children.push(id.to_string());
                }
            }
            (_, Owner::Choreography(c)) => {
                if let Some(ch) = self.doc.choreographies.get_mut(c) {
                    ch.children.push(id.to_string());
                }
            }
            _ => {}
        }
    }

    fn st_node(&mut self, line: &Line, ctx: &Ctx) -> Prev {
        let kind = line.tokens[0].value.clone();
        let sub = is_sub(&kind) || kind == "subchoreo";
        let Some(nid) = self.need_id(line, 1) else {
            return Prev {
                kind,
                accepts: sub,
                ctx: None,
            };
        };
        // a key written twice in a 0.1 or 0.3 document keeps its last value, as before
        let tail = if self.doc.lang_05() {
            self.parse_tail_multi(&line.tokens, 2, line.number)
        } else {
            self.parse_tail(&line.tokens, 2, line.number, true)
        };
        let mut base = Base::new(
            &kind,
            Some(nid.clone()),
            tail.label,
            tail.attrs,
            tail.flags,
            line.number,
        );
        base.extra = tail.extra;
        self.take_comments(&mut base, line);
        let node = Node {
            base,
            scope: ctx.scope.clone(),
            pool: ctx.pool.clone(),
            lane: if ctx.kind == "lane" {
                ctx.lane.clone()
            } else {
                None
            },
            children: Vec::new(),
            choreography: ctx.choreography.clone(),
        };
        if !self.register(&kind, &nid, line.number) {
            return Prev {
                kind,
                accepts: sub,
                ctx: None,
            };
        }
        self.doc.nodes.insert(nid.clone(), node);
        self.add_child(ctx, &nid);
        if sub {
            let child = Ctx {
                kind: match kind.as_str() {
                    "sub" => "sub",
                    "adhoc" => "adhoc",
                    "transaction" => "transaction",
                    "subchoreo" => "subchoreo",
                    _ => "eventsub",
                },
                scope: Some(nid.clone()),
                lane: None,
                pool: ctx.pool.clone(),
                owner: Owner::Node(nid),
                choreography: ctx.choreography.clone(),
            };
            return Prev {
                kind,
                accepts: true,
                ctx: Some(child),
            };
        }
        Prev::plain(&kind)
    }

    fn st_pool(&mut self, line: &Line) -> Prev {
        let Some(pid) = self.need_id(line, 1) else {
            return Prev::plain("pool");
        };
        let tail = self.parse_tail(&line.tokens, 2, line.number, true);
        let mut base = Base::new(
            "pool",
            Some(pid.clone()),
            tail.label,
            tail.attrs,
            tail.flags,
            line.number,
        );
        self.take_comments(&mut base, line);
        let pool = Pool {
            base,
            blackbox: true,
            children: Vec::new(),
        };
        if !self.register("pool", &pid, line.number) {
            return Prev {
                kind: "pool".into(),
                accepts: true,
                ctx: None,
            };
        }
        let scope = pool.process_id();
        self.doc.pools.push(pool);
        let child = Ctx {
            kind: "pool",
            scope: Some(scope),
            lane: None,
            pool: Some(pid.clone()),
            owner: Owner::Pool(pid),
            choreography: None,
        };
        Prev {
            kind: "pool".into(),
            accepts: true,
            ctx: Some(child),
        }
    }

    fn st_lane(&mut self, line: &Line, ctx: &Ctx) -> Prev {
        let Some(lid) = self.need_id(line, 1) else {
            return Prev::plain("lane");
        };
        let tail = self.parse_tail(&line.tokens, 2, line.number, true);
        let mut base = Base::new(
            "lane",
            Some(lid.clone()),
            tail.label,
            tail.attrs,
            tail.flags,
            line.number,
        );
        self.take_comments(&mut base, line);
        let lane = Lane {
            base,
            pool: ctx.pool.clone(),
            parent: ctx.lane.clone(),
            children: Vec::new(),
            sub: Vec::new(),
        };
        if !self.register("lane", &lid, line.number) {
            return Prev {
                kind: "lane".into(),
                accepts: true,
                ctx: None,
            };
        }
        self.doc.lanes.insert(lid.clone(), lane);
        match &ctx.owner {
            Owner::Pool(p) => {
                let p = p.clone();
                if let Some(pool) = self.doc.pool_mut(&p) {
                    pool.children.push(lid.clone());
                }
            }
            Owner::Lane(l) => {
                if let Some(parent) = self.doc.lanes.get_mut(l) {
                    parent.children.push(lid.clone());
                }
            }
            _ => {}
        }
        let child = Ctx {
            kind: "lane",
            scope: ctx.scope.clone(),
            lane: Some(lid.clone()),
            pool: ctx.pool.clone(),
            owner: Owner::Lane(lid),
            choreography: None,
        };
        Prev {
            kind: "lane".into(),
            accepts: true,
            ctx: Some(child),
        }
    }

    fn st_flow(&mut self, line: &Line) -> Prev {
        let toks = &line.tokens;
        let mut ids = Vec::new();
        let mut i = 1;
        loop {
            if i >= toks.len() || !matches!(toks[i].kind, Kind::Word | Kind::Number) {
                self.err(
                    "P008",
                    line.number,
                    None,
                    &[
                        ("keyword", "flow".into()),
                        ("detail", "expected an identifier".into()),
                    ],
                );
                return Prev::plain("flow");
            }
            ids.push(toks[i].value.clone());
            i += 1;
            if i < toks.len() && toks[i].kind == Kind::Arrow {
                i += 1;
                continue;
            }
            break;
        }
        if ids.len() < 2 {
            self.err(
                "P008",
                line.number,
                None,
                &[
                    ("keyword", "flow".into()),
                    ("detail", "expected '->'".into()),
                ],
            );
            return Prev::plain("flow");
        }
        let mut tail = self.parse_tail(toks, i, line.number, true);
        if (tail.label.is_some() || !tail.attrs.is_empty() || !tail.flags.is_empty())
            && ids.len() > 2
        {
            self.err("R020", line.number, None, &[]);
            tail = Tail {
                label: None,
                attrs: Attrs::new(),
                extra: Vec::new(),
                flags: Vec::new(),
            };
        }
        for k in 0..ids.len() - 1 {
            let (src, tgt) = (ids[k].clone(), ids[k + 1].clone());
            let explicit = if k == 0 {
                tail.attrs.get("id").cloned()
            } else {
                None
            };
            let fid = match explicit.as_ref().filter(|e| !e.is_empty()) {
                Some(e) => e.clone(),
                None => self.derive_id(&format!("f-{src}-{tgt}")),
            };
            let mut attrs = tail.attrs.clone();
            attrs.shift_remove("id");
            let mut base = Base::new(
                "flow",
                Some(fid.clone()),
                tail.label.clone(),
                attrs,
                tail.flags.clone(),
                line.number,
            );
            if k == 0 {
                self.take_comments(&mut base, line);
            }
            let f = Flow {
                base,
                source: src,
                target: tgt,
                explicit_id: explicit.is_some(),
            };
            self.register("flow", &fid, line.number);
            self.doc.flows.push(f);
        }
        Prev::plain("flow")
    }

    fn derive_id(&mut self, base: &str) -> String {
        let mut n = self.counter.get(base).copied().unwrap_or(0) + 1;
        self.counter.insert(base.to_string(), n);
        let mut fid = if n == 1 {
            base.to_string()
        } else {
            format!("{base}-{n}")
        };
        while self.doc.ids.contains_key(&fid) {
            n += 1;
            self.counter.insert(base.to_string(), n);
            fid = format!("{base}-{n}");
        }
        fid
    }

    fn st_message(&mut self, line: &Line) -> Prev {
        let toks = &line.tokens;
        let ok = toks.len() >= 4
            && matches!(toks[1].kind, Kind::Word | Kind::Number)
            && toks[2].kind == Kind::Arrow
            && matches!(toks[3].kind, Kind::Word | Kind::Number);
        if !ok {
            self.err(
                "P008",
                line.number,
                None,
                &[
                    ("keyword", "message".into()),
                    ("detail", "expected 'message A -> B'".into()),
                ],
            );
            return Prev::plain("message");
        }
        let mut tail = self.parse_tail(toks, 4, line.number, true);
        let (src, tgt) = (toks[1].value.clone(), toks[3].value.clone());
        let mid = tail.attrs.shift_remove("id");
        let explicit = mid.is_some();
        let id = match mid.filter(|m| !m.is_empty()) {
            Some(m) => m,
            None => self.derive_id(&format!("m-{src}-{tgt}")),
        };
        for fl in &tail.flags {
            self.err(
                "R011",
                line.number,
                None,
                &[("flag", fl.clone()), ("kind", "message".into())],
            );
        }
        let mut base = Base::new(
            "message",
            Some(id.clone()),
            tail.label,
            tail.attrs,
            Vec::new(),
            line.number,
        );
        self.take_comments(&mut base, line);
        let m = MessageFlow {
            base,
            source: src,
            target: tgt,
            explicit_id: explicit,
        };
        self.register("message", &id, line.number);
        self.doc.messages.push(m);
        Prev::plain("message")
    }

    fn st_data(&mut self, line: &Line, ctx: &Ctx, kind: &str) -> Prev {
        let Some(did) = self.need_id(line, 1) else {
            return Prev::plain(kind);
        };
        let tail = self.parse_tail(&line.tokens, 2, line.number, true);
        let mut base = Base::new(
            kind,
            Some(did.clone()),
            tail.label,
            tail.attrs,
            tail.flags,
            line.number,
        );
        self.take_comments(&mut base, line);
        let (scope, block) = if ctx.kind == "top" {
            (None, None)
        } else if ctx.kind == "lane" {
            (ctx.scope.clone(), ctx.pool.clone())
        } else {
            (ctx.scope.clone(), ctx.owner_id().map(str::to_string))
        };
        let d = Data { base, scope, block };
        if !self.register(kind, &did, line.number) {
            return Prev::plain(kind);
        }
        self.doc.data.insert(did.clone(), d);
        if ctx.kind == "lane" {
            if let Some(pid) = ctx.pool.clone()
                && let Some(pool) = self.doc.pool_mut(&pid)
            {
                pool.children.push(did);
            }
        } else {
            self.add_child(ctx, &did);
        }
        Prev::plain(kind)
    }

    fn st_assoc(&mut self, line: &Line, kind: &str) -> Prev {
        let toks = &line.tokens;
        if toks.len() < 3 || toks[1].kind != Kind::Word || toks[2].kind != Kind::Word {
            self.err(
                "P008",
                line.number,
                None,
                &[
                    ("keyword", kind.into()),
                    ("detail", format!("expected '{kind} ACTIVITY DATA'")),
                ],
            );
            return Prev::plain(kind);
        }
        let (act, data) = (toks[1].value.clone(), toks[2].value.clone());
        // language 0.5: `input=` / `output=` name the data input or output of the activity
        let tail = self.parse_tail(toks, 3, line.number, false);
        let mut base = Base::new(
            kind,
            Some(format!("{kind}-{act}-{data}")),
            None,
            tail.attrs,
            tail.flags,
            line.number,
        );
        self.take_comments(&mut base, line);
        self.doc.assocs.push(Association {
            base,
            activity: act,
            data,
        });
        Prev::plain(kind)
    }

    fn st_note_group(&mut self, line: &Line, kind: &str) -> Prev {
        let Some(nid) = self.need_id(line, 1) else {
            return Prev::plain(kind);
        };
        let tail = self.parse_tail(&line.tokens, 2, line.number, true);
        let mut base = Base::new(
            kind,
            Some(nid.clone()),
            tail.label,
            tail.attrs,
            tail.flags,
            line.number,
        );
        self.take_comments(&mut base, line);
        self.register(kind, &nid, line.number);
        if kind == "note" {
            self.doc.notes.push(base);
        } else {
            self.doc.groups.push(base);
        }
        Prev::plain(kind)
    }

    fn st_decision(&mut self, line: &Line) -> Prev {
        let Some(did) = self.need_id(line, 1) else {
            return Prev::plain("decision");
        };
        let tail = self.parse_tail(&line.tokens, 2, line.number, true);
        for fl in &tail.flags {
            self.err(
                "R011",
                line.number,
                None,
                &[("flag", fl.clone()), ("kind", "decision".into())],
            );
        }
        let mut base = Base::new(
            "decision",
            Some(did.clone()),
            tail.label,
            tail.attrs,
            Vec::new(),
            line.number,
        );
        self.take_comments(&mut base, line);
        self.register("decision", &did, line.number);
        self.doc.decisions.insert(
            did.clone(),
            Decision {
                base,
                ..Default::default()
            },
        );
        let child = Ctx {
            kind: "decision",
            scope: None,
            lane: None,
            pool: None,
            owner: Owner::Decision(did),
            choreography: None,
        };
        Prev {
            kind: "decision".into(),
            accepts: true,
            ctx: Some(child),
        }
    }

    fn row(&mut self, line: &Line, did: &str) {
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
        if groups.len() < 2 || groups.len() > 3 {
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
        let ann: Option<String> = if groups.len() == 3 {
            Some(
                groups[2]
                    .iter()
                    .filter(|x| !x.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        } else {
            None
        };
        let ins = groups[0].clone();
        let outs = groups[1].clone();
        let Some(dec) = self.doc.decisions.get_mut(did) else {
            return;
        };
        if !dec.header_done {
            dec.header_done = true;
            let mut errs = Vec::new();
            for cell in &ins {
                match split_header_cell(cell) {
                    Some(p) => dec.inputs.push(p),
                    None => errs.push(cell.clone()),
                }
            }
            for cell in &outs {
                match split_header_cell(cell) {
                    Some(p) => dec.outputs.push(p),
                    None => errs.push(cell.clone()),
                }
            }
            dec.has_annotations = groups.len() == 3;
            dec.annotation_title = ann;
            dec.header_line = Some(line.number);
            dec.no_inputs = ins.is_empty();
            dec.no_outputs = outs.is_empty();
            for cell in errs {
                self.err("D004", line.number, Some(did), &[("cell", cell)]);
            }
            return;
        }
        if ins.len() != dec.inputs.len() || outs.len() != dec.outputs.len() {
            let got = (ins.len() + outs.len()).to_string();
            let expected = (dec.inputs.len() + dec.outputs.len()).to_string();
            self.err(
                "D002",
                line.number,
                Some(did),
                &[("got", got), ("expected", expected)],
            );
            return;
        }
        let rule = Rule {
            inputs: ins,
            outputs: outs,
            annotation: ann.filter(|a| !a.is_empty()),
            line: line.number,
            comments: std::mem::take(&mut self.pending),
            trailing: line.comment.clone(),
        };
        if let Some(dec) = self.doc.decisions.get_mut(did) {
            dec.rules.push(rule);
        }
    }

    fn st_form(&mut self, line: &Line) -> Prev {
        let Some(fid) = self.need_id(line, 1) else {
            return Prev::plain("form");
        };
        let tail = self.parse_tail(&line.tokens, 2, line.number, true);
        let mut base = Base::new(
            "form",
            Some(fid.clone()),
            tail.label,
            tail.attrs,
            Vec::new(),
            line.number,
        );
        self.take_comments(&mut base, line);
        self.register("form", &fid, line.number);
        self.doc.forms.insert(
            fid.clone(),
            Form {
                base,
                items: Vec::new(),
            },
        );
        let child = Ctx {
            kind: "form",
            scope: None,
            lane: None,
            pool: None,
            owner: Owner::Form(fid),
            choreography: None,
        };
        Prev {
            kind: "form".into(),
            accepts: true,
            ctx: Some(child),
        }
    }

    fn push_item(&mut self, ctx: &Ctx, item: FormItem) {
        if let Owner::Form(f) = &ctx.owner
            && let Some(form) = self.doc.forms.get_mut(f)
        {
            form.items.push(item);
        }
    }

    fn st_text(&mut self, line: &Line, ctx: &Ctx) -> Prev {
        let toks = &line.tokens;
        if toks.len() != 2 || toks[1].kind != Kind::Str {
            self.err(
                "P008",
                line.number,
                None,
                &[
                    ("keyword", "text".into()),
                    ("detail", "expected text \"…\"".into()),
                ],
            );
            return Prev::plain("text");
        }
        let mut base = Base::new(
            "text",
            None,
            Some(toks[1].value.clone()),
            Attrs::new(),
            Vec::new(),
            line.number,
        );
        self.take_comments(&mut base, line);
        self.push_item(ctx, FormItem { base });
        Prev::plain("text")
    }

    fn st_field(&mut self, line: &Line, ctx: &Ctx) -> Prev {
        let Some(key) = self.need_id(line, 1) else {
            return Prev::plain("field");
        };
        let tail = self.parse_tail(&line.tokens, 2, line.number, true);
        let mut base = Base::new(
            "field",
            Some(key),
            tail.label,
            tail.attrs,
            tail.flags,
            line.number,
        );
        self.take_comments(&mut base, line);
        self.push_item(ctx, FormItem { base });
        Prev::plain("field")
    }

    fn st_simulate(&mut self, line: &Line) -> Prev {
        let tail = self.parse_tail(&line.tokens, 1, line.number, false);
        let mut base = Base::new("simulate", None, None, tail.attrs, tail.flags, line.number);
        self.take_comments(&mut base, line);
        self.doc.simulate = Some(base);
        Prev::plain("simulate")
    }

    fn st_resource(&mut self, line: &Line) -> Prev {
        let Some(rid) = self.need_id(line, 1) else {
            return Prev::plain("resource");
        };
        let tail = self.parse_tail(&line.tokens, 2, line.number, true);
        let mut base = Base::new(
            "resource",
            Some(rid.clone()),
            tail.label,
            tail.attrs,
            tail.flags,
            line.number,
        );
        self.take_comments(&mut base, line);
        self.register("resource", &rid, line.number);
        self.doc.resources.insert(rid, base);
        Prev::plain("resource")
    }

    fn st_variable(&mut self, line: &Line) -> Prev {
        let toks = &line.tokens;
        if toks.len() != 3 || toks[1].kind != Kind::Word || toks[2].kind != Kind::Str {
            self.err(
                "P008",
                line.number,
                None,
                &[
                    ("keyword", "variable".into()),
                    ("detail", "expected variable NAME \"DIST\"".into()),
                ],
            );
            return Prev::plain("variable");
        }
        let mut base = Base::new(
            "variable",
            Some(toks[1].value.clone()),
            Some(toks[2].value.clone()),
            Attrs::new(),
            Vec::new(),
            line.number,
        );
        self.take_comments(&mut base, line);
        self.doc.variables.push(base);
        Prev::plain("variable")
    }

    /// `item` and `global` (language 0.5): top-level declarations without a block.
    fn st_declaration(&mut self, line: &Line, kind: &str) -> Prev {
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
            if kind == "item" {
                self.doc.items.insert(id, base);
            } else {
                self.doc.globals.insert(id, base);
            }
        }
        Prev::plain(kind)
    }

    /// `interface` and `key` (language 0.5): declarations with a block of `operation` or
    /// `prop` lines.
    fn st_block_declaration(&mut self, line: &Line, kind: &str) -> Prev {
        let Some(id) = self.need_id(line, 1) else {
            return Prev {
                kind: kind.to_string(),
                accepts: true,
                ctx: None,
            };
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
                kind: kind.to_string(),
                accepts: true,
                ctx: None,
            };
        }
        let (ctx_kind, owner) = if kind == "interface" {
            self.doc.interfaces.insert(
                id.clone(),
                Interface {
                    base,
                    operations: Vec::new(),
                },
            );
            ("interface", Owner::Interface(id))
        } else {
            self.doc.keys.insert(
                id.clone(),
                CorrelationKey {
                    base,
                    props: Vec::new(),
                },
            );
            ("key", Owner::Key(id))
        };
        Prev {
            kind: kind.to_string(),
            accepts: true,
            ctx: Some(Ctx {
                kind: ctx_kind,
                scope: None,
                lane: None,
                pool: None,
                owner,
                choreography: None,
            }),
        }
    }

    /// `conversation`, `subconversation` (with a block of conversation nodes) and
    /// `callconversation` (language 0.6).
    fn st_conversation(&mut self, line: &Line, ctx: &Ctx, kind: &str) -> Prev {
        let block = kind == "subconversation";
        let Some(id) = self.need_id(line, 1) else {
            return Prev {
                kind: kind.to_string(),
                accepts: block,
                ctx: None,
            };
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
                kind: kind.to_string(),
                accepts: block,
                ctx: None,
            };
        }
        let parent = match &ctx.owner {
            Owner::Conversation(p) => Some(p.clone()),
            _ => None,
        };
        if let Some(p) = &parent
            && let Some(x) = self.doc.conversations.get_mut(p)
        {
            x.children.push(id.clone());
        }
        self.doc.conversations.insert(
            id.clone(),
            Conversation {
                base,
                parent,
                children: Vec::new(),
            },
        );
        if !block {
            return Prev::plain(kind);
        }
        Prev {
            kind: kind.to_string(),
            accepts: true,
            ctx: Some(Ctx {
                kind: "subconversation",
                scope: None,
                lane: None,
                pool: None,
                owner: Owner::Conversation(id),
                choreography: None,
            }),
        }
    }

    /// `choreography ID ["Label"]` with a block of flow nodes and flows (language 0.6).
    fn st_choreography(&mut self, line: &Line) -> Prev {
        let Some(id) = self.need_id(line, 1) else {
            return Prev {
                kind: "choreography".into(),
                accepts: true,
                ctx: None,
            };
        };
        let tail = self.parse_tail(&line.tokens, 2, line.number, true);
        let mut base = Base::new(
            "choreography",
            Some(id.clone()),
            tail.label,
            tail.attrs,
            tail.flags,
            line.number,
        );
        self.take_comments(&mut base, line);
        if !self.register("choreography", &id, line.number) {
            return Prev {
                kind: "choreography".into(),
                accepts: true,
                ctx: None,
            };
        }
        self.doc.choreographies.insert(
            id.clone(),
            Choreography {
                base,
                children: Vec::new(),
            },
        );
        Prev {
            kind: "choreography".into(),
            accepts: true,
            ctx: Some(Ctx {
                kind: "choreography",
                scope: Some(id.clone()),
                lane: None,
                pool: None,
                owner: Owner::Choreography(id.clone()),
                choreography: Some(id),
            }),
        }
    }

    /// `operation` of an interface and `prop` of a correlation key (language 0.5).
    fn st_member(&mut self, line: &Line, ctx: &Ctx, kind: &str) -> Prev {
        let Some(id) = self.need_id(line, 1) else {
            return Prev::plain(kind);
        };
        let tail = if kind == "operation" {
            self.parse_tail_multi(&line.tokens, 2, line.number)
        } else {
            self.parse_tail(&line.tokens, 2, line.number, true)
        };
        let mut base = Base::new(
            kind,
            Some(id.clone()),
            tail.label,
            tail.attrs,
            tail.flags,
            line.number,
        );
        base.extra = tail.extra;
        self.take_comments(&mut base, line);
        match &ctx.owner {
            Owner::Interface(i) => {
                if self.register(kind, &id, line.number)
                    && let Some(x) = self.doc.interfaces.get_mut(i)
                {
                    x.operations.push(base);
                }
            }
            Owner::Key(k) => {
                if !is_id(&id) {
                    self.err("R004", line.number, Some(&id), &[("id", id.clone())]);
                }
                if let Some(x) = self.doc.keys.get_mut(k) {
                    x.props.push(base);
                }
            }
            _ => {}
        }
        Prev::plain(kind)
    }
}

/// Split a table row into cells by `|` outside quotes; empty cells mark `||`.
pub fn split_row(text: &str) -> Option<Vec<String>> {
    let mut s = strip(text);
    if let Some(r) = s.strip_prefix('|') {
        s = r;
    }
    if s.ends_with('|') && !s.ends_with("\\|") {
        s = &s[..s.len() - 1];
    }
    let ch: Vec<char> = s.chars().collect();
    let mut cells = Vec::new();
    let mut buf = String::new();
    let mut in_str = false;
    let mut i = 0;
    while i < ch.len() {
        let c = ch[i];
        if c == '\\' && in_str && i + 1 < ch.len() {
            buf.push(c);
            buf.push(ch[i + 1]);
            i += 2;
            continue;
        }
        if c == '"' {
            in_str = !in_str;
        }
        if c == '|' && !in_str {
            cells.push(std::mem::take(&mut buf));
        } else {
            buf.push(c);
        }
        i += 1;
    }
    if in_str {
        return None;
    }
    cells.push(buf);
    Some(cells)
}

fn split_header_cell(cell: &str) -> Option<(String, String)> {
    let (name, typ) = cell.rsplit_once(':')?;
    let (name, typ) = (strip(name), strip(typ));
    if name.is_empty() || !["string", "number", "boolean", "date"].contains(&typ) {
        return None;
    }
    Some((name.to_string(), typ.to_string()))
}
