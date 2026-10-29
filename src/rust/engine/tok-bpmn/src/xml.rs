//! A small mutable element tree that reproduces the behaviour of Python's
//! `xml.etree.ElementTree` as used by the course tool `bpmn.py`: parsing
//! (text/tail model, comments dropped), the global namespace registry,
//! `_indent`, `save` and `tostring` serialization.

use std::collections::HashMap;

use crate::Error;

/// Namespaces known to `bpmn.py` (`NS`), in declaration order.
pub const NS: [(&str, &str); 5] = [
    ("bpmn", "http://www.omg.org/spec/BPMN/20100524/MODEL"),
    ("bpmndi", "http://www.omg.org/spec/BPMN/20100524/DI"),
    ("dc", "http://www.omg.org/spec/DD/20100524/DC"),
    ("di", "http://www.omg.org/spec/DD/20100524/DI"),
    ("xsi", "http://www.w3.org/2001/XMLSchema-instance"),
];

pub const BPMN_NS: &str = NS[0].1;
pub const BPMNDI_NS: &str = NS[1].1;
pub const DC_NS: &str = NS[2].1;
pub const DI_NS: &str = NS[3].1;
pub const XSI_NS: &str = NS[4].1;
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

/// The deepest nesting of elements read. The BPMN files tok writes for documents whose
/// blocks are nested as deep as the parser reads stay well within it.
pub const MAX_DEPTH: usize = 256;

/// Qualified name in ElementTree notation: `{uri}local` or `local`.
pub fn qn(uri: &str, local: &str) -> String {
    format!("{{{uri}}}{local}")
}

/// Element id inside a [`Doc`] arena.
pub type Id = usize;

#[derive(Debug, Clone)]
pub struct Elem {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub text: Option<String>,
    pub tail: Option<String>,
    pub children: Vec<Id>,
}

impl Elem {
    fn new(tag: String) -> Self {
        Self {
            tag,
            attrs: Vec::new(),
            text: None,
            tail: None,
            children: Vec::new(),
        }
    }
}

/// Element tree plus the prefix map of the input document (`in_map`).
#[derive(Debug, Clone)]
pub struct Doc {
    pub elems: Vec<Elem>,
    pub root: Id,
    /// `prefix -> uri` in the order of first declaration (ElementTree `start-ns`).
    pub in_map: Vec<(String, String)>,
    /// ElementTree `_namespace_map` after `load` (uri -> prefix).
    registry: HashMap<String, String>,
}

fn is_ns_digits(prefix: &str) -> bool {
    prefix.len() > 2 && prefix.starts_with("ns") && prefix[2..].bytes().all(|b| b.is_ascii_digit())
}

fn default_registry() -> HashMap<String, String> {
    [
        (XML_NS, "xml"),
        ("http://www.w3.org/1999/xhtml", "html"),
        ("http://www.w3.org/1999/02/22-rdf-syntax-ns#", "rdf"),
        ("http://schemas.xmlsoap.org/wsdl/", "wsdl"),
        ("http://www.w3.org/2001/XMLSchema", "xs"),
        ("http://www.w3.org/2001/XMLSchema-instance", "xsi"),
        ("http://purl.org/dc/elements/1.1/", "dc"),
    ]
    .iter()
    .map(|(u, p)| ((*u).to_string(), (*p).to_string()))
    .collect()
}

fn register(registry: &mut HashMap<String, String>, prefix: &str, uri: &str) {
    if is_ns_digits(prefix) {
        return;
    }
    registry.retain(|k, v| k != uri && v != prefix);
    registry.insert(uri.to_string(), prefix.to_string());
}

/// Namespace declarations (`xmlns`, `xmlns:p`) written in a start tag, in order.
fn declared_namespaces(start_tag: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = start_tag.as_bytes();
    let mut i = 0;
    // skip `<name`
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' && bytes[i] != b'/'
    {
        i += 1;
    }
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] == b'>' || bytes[i] == b'/' {
            break;
        }
        let ns = i;
        while i < bytes.len() && bytes[i] != b'=' && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let name = &start_tag[ns..i];
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'=') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let quote = bytes[i];
        i += 1;
        let vs = i;
        while i < bytes.len() && bytes[i] != quote {
            i += 1;
        }
        let value = &start_tag[vs..i.min(bytes.len())];
        i += 1;
        if name == "xmlns" {
            out.push((String::new(), unescape(value)));
        } else if let Some(p) = name.strip_prefix("xmlns:") {
            out.push((p.to_string(), unescape(value)));
        }
    }
    out
}

fn unescape(v: &str) -> String {
    v.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

impl Doc {
    /// Parse XML text like `bpmn.py load()`: comments and processing instructions
    /// are dropped, the input prefixes are registered as ElementTree would.
    pub fn parse(text: &str) -> Result<Doc, Error> {
        let opt = roxmltree::ParsingOptions {
            allow_dtd: true,
            ..roxmltree::ParsingOptions::default()
        };
        let rx = roxmltree::Document::parse_with_options(text, opt)
            .map_err(|e| Error::Xml(e.to_string()))?;
        // `build` and everything that walks the tree recurse into elements; nodes come
        // in document order, so a parent comes before its children
        let mut depths = vec![0usize; rx.descendants().count() + 1];
        for n in rx.descendants() {
            let d =
                n.parent().map_or(0, |p| depths[p.id().get_usize()]) + usize::from(n.is_element());
            if d > MAX_DEPTH {
                return Err(Error::Xml(format!(
                    "the elements are nested more than {MAX_DEPTH} levels deep"
                )));
            }
            depths[n.id().get_usize()] = d;
        }
        let mut doc = Doc {
            elems: Vec::new(),
            root: 0,
            in_map: Vec::new(),
            registry: default_registry(),
        };
        let root = doc.build(rx.root_element(), text);
        doc.root = root;
        // namespace registration done by load()
        let known: Vec<&str> = NS.iter().map(|(_, u)| *u).collect();
        let pairs = doc.in_map.clone();
        for (prefix, uri) in &pairs {
            if known.contains(&uri.as_str()) || prefix.is_empty() || is_ns_digits(prefix) {
                continue;
            }
            register(&mut doc.registry, prefix, uri);
        }
        for (p, u) in NS {
            register(&mut doc.registry, p, u);
        }
        Ok(doc)
    }

    fn build(&mut self, node: roxmltree::Node<'_, '_>, text: &str) -> Id {
        let range = node.range();
        let start = &text[range.start..];
        let end = find_tag_end(start);
        for (p, u) in declared_namespaces(&start[..end]) {
            if !self.in_map.iter().any(|(q, _)| *q == p) {
                self.in_map.push((p, u));
            }
        }
        let tag = match node.tag_name().namespace() {
            Some(uri) => qn(uri, node.tag_name().name()),
            None => node.tag_name().name().to_string(),
        };
        let mut el = Elem::new(tag);
        for a in node.attributes() {
            let key = match a.namespace() {
                Some(uri) => qn(uri, a.name()),
                None => a.name().to_string(),
            };
            el.attrs.push((key, a.value().to_string()));
        }
        let id = self.elems.len();
        self.elems.push(el);
        let mut buf: Option<String> = None;
        let mut last_child: Option<Id> = None;
        for c in node.children() {
            if c.is_element() {
                self.flush(id, last_child, &mut buf);
                let cid = self.build(c, text);
                self.elems[id].children.push(cid);
                last_child = Some(cid);
            } else if c.is_text()
                && let Some(t) = c.text()
                && !t.is_empty()
            {
                buf.get_or_insert_with(String::new).push_str(t);
            }
        }
        self.flush(id, last_child, &mut buf);
        id
    }

    fn flush(&mut self, parent: Id, last_child: Option<Id>, buf: &mut Option<String>) {
        if let Some(t) = buf.take() {
            match last_child {
                Some(c) => self.elems[c].tail = Some(t),
                None => self.elems[parent].text = Some(t),
            }
        }
    }

    pub fn tag(&self, e: Id) -> &str {
        &self.elems[e].tag
    }

    pub fn get(&self, e: Id, key: &str) -> Option<&str> {
        self.elems[e]
            .attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn set(&mut self, e: Id, key: &str, value: &str) {
        let attrs = &mut self.elems[e].attrs;
        if let Some(a) = attrs.iter_mut().find(|(k, _)| k == key) {
            a.1 = value.to_string();
        } else {
            attrs.push((key.to_string(), value.to_string()));
        }
    }

    pub fn children(&self, e: Id) -> &[Id] {
        &self.elems[e].children
    }

    pub fn text(&self, e: Id) -> Option<&str> {
        self.elems[e].text.as_deref()
    }

    /// Pre-order iteration over `e` and all its descendants.
    pub fn iter(&self, e: Id) -> Vec<Id> {
        let mut out = Vec::new();
        let mut stack = vec![e];
        while let Some(x) = stack.pop() {
            out.push(x);
            for c in self.elems[x].children.iter().rev() {
                stack.push(*c);
            }
        }
        out
    }

    /// `elem.iter(tag)`.
    pub fn iter_tag(&self, e: Id, tag: &str) -> Vec<Id> {
        self.iter(e)
            .into_iter()
            .filter(|x| self.elems[*x].tag == tag)
            .collect()
    }

    /// `elem.findall(tag)` (direct children).
    pub fn findall(&self, e: Id, tag: &str) -> Vec<Id> {
        self.elems[e]
            .children
            .iter()
            .copied()
            .filter(|c| self.elems[*c].tag == tag)
            .collect()
    }

    /// `elem.find(tag)` (first direct child).
    pub fn find(&self, e: Id, tag: &str) -> Option<Id> {
        self.elems[e]
            .children
            .iter()
            .copied()
            .find(|c| self.elems[*c].tag == tag)
    }

    /// `elem.findtext(tag)`: `None` without the child, `""` for a child without text.
    pub fn findtext(&self, e: Id, tag: &str) -> Option<String> {
        self.find(e, tag)
            .map(|c| self.elems[c].text.clone().unwrap_or_default())
    }

    /// `ET.SubElement(parent, tag, attrs)`.
    pub fn sub_element(&mut self, parent: Id, tag: &str, attrs: &[(&str, &str)]) -> Id {
        let mut el = Elem::new(tag.to_string());
        for (k, v) in attrs {
            el.attrs.push(((*k).to_string(), (*v).to_string()));
        }
        let id = self.elems.len();
        self.elems.push(el);
        self.elems[parent].children.push(id);
        id
    }

    pub fn remove_child(&mut self, parent: Id, child: Id) {
        self.elems[parent].children.retain(|c| *c != child);
    }

    // ------------------------------------------------------------- writing

    fn fix_qnames(&mut self) {
        let xsi_type = qn(XSI_NS, "type");
        let out_prefix: HashMap<&str, &str> = NS.iter().map(|(p, u)| (*u, *p)).collect();
        for e in self.iter(self.root) {
            let Some(v) = self.get(e, &xsi_type).map(str::to_string) else {
                continue;
            };
            if v.is_empty() {
                continue;
            }
            let (p, loc) = match v.split_once(':') {
                Some((p, l)) => (p.to_string(), l.to_string()),
                None => (String::new(), v.clone()),
            };
            let uri = self
                .in_map
                .iter()
                .find(|(q, _)| *q == p)
                .map(|(_, u)| u.as_str());
            if let Some(op) = uri.and_then(|u| out_prefix.get(u)) {
                let nv = format!("{op}:{loc}");
                self.set(e, &xsi_type, &nv);
            }
        }
    }

    fn indent(&mut self, e: Id, level: usize) {
        let pad = format!("\n{}", "  ".repeat(level));
        if !self.elems[e].children.is_empty() {
            if blank(&self.elems[e].text) {
                self.elems[e].text = Some(format!("{pad}  "));
            }
            let kids = self.elems[e].children.clone();
            for c in &kids {
                self.indent(*c, level + 1);
            }
            if let Some(last) = kids.last()
                && blank(&self.elems[*last].tail)
            {
                self.elems[*last].tail = Some(pad.clone());
            }
        }
        if level > 0 && blank(&self.elems[e].tail) {
            self.elems[e].tail = Some(pad);
        }
    }

    /// `bpmn.py save()` without writing a file: returns the file content.
    pub fn save(&mut self) -> String {
        self.fix_qnames();
        for e in self.iter(self.root) {
            if !self.elems[e].children.is_empty() {
                if self.elems[e]
                    .text
                    .as_deref()
                    .is_some_and(|t| t.trim().is_empty())
                {
                    self.elems[e].text = None;
                }
                let kids = self.elems[e].children.clone();
                for c in kids {
                    if self.elems[c]
                        .tail
                        .as_deref()
                        .is_some_and(|t| t.trim().is_empty())
                    {
                        self.elems[c].tail = None;
                    }
                }
            }
        }
        let root = self.root;
        self.indent(root, 0);
        let mut text = self.tostring();
        let head_end = if text.starts_with('<') {
            text.find('>')
        } else {
            None
        };
        if let Some(he) = head_end {
            let head = text[..=he].to_string();
            let mut extra = String::new();
            let known: Vec<&str> = NS.iter().map(|(_, u)| *u).collect();
            for (prefix, uri) in &self.in_map {
                if prefix.is_empty() || is_ns_digits(prefix) {
                    continue;
                }
                if !head.contains(&format!("xmlns:{prefix}="))
                    && !known.contains(&uri.as_str())
                    && text.contains(&format!("=\"{prefix}:"))
                {
                    extra.push_str(&format!(" xmlns:{prefix}=\"{uri}\""));
                }
            }
            if !extra.is_empty() {
                text = format!(
                    "{}{}{}{}",
                    &head[..head.len() - 1],
                    extra,
                    &head[head.len() - 1..],
                    &text[he + 1..]
                );
            }
        }
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{text}\n")
    }

    /// `ET.tostring(root, encoding="unicode")`.
    pub fn tostring(&self) -> String {
        let mut qnames: HashMap<String, String> = HashMap::new();
        let mut namespaces: Vec<(String, String)> = Vec::new(); // uri -> prefix, insertion order
        let add = |name: &str,
                   qnames: &mut HashMap<String, String>,
                   namespaces: &mut Vec<(String, String)>| {
            if qnames.contains_key(name) {
                return;
            }
            if let Some(rest) = name.strip_prefix('{') {
                let (uri, local) = rest.rsplit_once('}').unwrap_or((rest, ""));
                let prefix = match namespaces.iter().find(|(u, _)| u == uri) {
                    Some((_, p)) => p.clone(),
                    None => {
                        let p = self
                            .registry
                            .get(uri)
                            .cloned()
                            .unwrap_or_else(|| format!("ns{}", namespaces.len()));
                        if p != "xml" {
                            namespaces.push((uri.to_string(), p.clone()));
                        }
                        p
                    }
                };
                let q = if prefix.is_empty() {
                    local.to_string()
                } else {
                    format!("{prefix}:{local}")
                };
                qnames.insert(name.to_string(), q);
            } else {
                qnames.insert(name.to_string(), name.to_string());
            }
        };
        for e in self.iter(self.root) {
            add(&self.elems[e].tag, &mut qnames, &mut namespaces);
            for (k, _) in &self.elems[e].attrs {
                add(k, &mut qnames, &mut namespaces);
            }
        }
        let mut sorted = namespaces.clone();
        sorted.sort_by(|a, b| a.1.cmp(&b.1));
        let mut out = String::new();
        self.write_elem(self.root, &qnames, Some(&sorted), &mut out);
        out
    }

    fn write_elem(
        &self,
        e: Id,
        qnames: &HashMap<String, String>,
        ns: Option<&[(String, String)]>,
        out: &mut String,
    ) {
        let el = &self.elems[e];
        let tag = &qnames[&el.tag];
        out.push('<');
        out.push_str(tag);
        if let Some(ns) = ns {
            for (uri, prefix) in ns {
                if prefix.is_empty() {
                    out.push_str(&format!(" xmlns=\"{}\"", escape_attrib(uri)));
                } else {
                    out.push_str(&format!(" xmlns:{prefix}=\"{}\"", escape_attrib(uri)));
                }
            }
        }
        for (k, v) in &el.attrs {
            out.push_str(&format!(" {}=\"{}\"", qnames[k], escape_attrib(v)));
        }
        let has_text = el.text.as_deref().is_some_and(|t| !t.is_empty());
        if has_text || !el.children.is_empty() {
            out.push('>');
            if let Some(t) = &el.text {
                out.push_str(&escape_cdata(t));
            }
            for c in &el.children {
                self.write_elem(*c, qnames, None, out);
            }
            out.push_str("</");
            out.push_str(tag);
            out.push('>');
        } else {
            out.push_str(" />");
        }
        if let Some(t) = &el.tail {
            out.push_str(&escape_cdata(t));
        }
    }
}

fn find_tag_end(s: &str) -> usize {
    let mut quote: Option<u8> = None;
    for (i, b) in s.bytes().enumerate() {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == b'>' => return i + 1,
            None => {}
        }
    }
    s.len()
}

fn blank(t: &Option<String>) -> bool {
    t.as_deref().is_none_or(|t| t.trim().is_empty())
}

fn escape_cdata(t: &str) -> String {
    t.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attrib(t: &str) -> String {
    t.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\r', "&#13;")
        .replace('\n', "&#10;")
        .replace('\t', "&#09;")
}
