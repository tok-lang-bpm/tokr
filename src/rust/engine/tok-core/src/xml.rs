//! A tiny element tree and a writer that reproduces Python's `xml.etree.ElementTree` output.

pub const BPMN_NS: &str = "http://www.omg.org/spec/BPMN/20100524/MODEL";
pub const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";
pub const ZEEBE_NS: &str = "http://camunda.org/schema/zeebe/1.0";
pub const MODELER_NS: &str = "http://camunda.org/schema/modeler/1.0";
pub const DMN_NS: &str = "https://www.omg.org/spec/DMN/20191111/MODEL/";
/// DMN 1.5 (language 0.7) and its diagram interchange.
pub const DMN15_NS: &str = "https://www.omg.org/spec/DMN/20230324/MODEL/";
pub const DMNDI15_NS: &str = "https://www.omg.org/spec/DMN/20230324/DMNDI/";
pub const DMN_DC_NS: &str = "http://www.omg.org/spec/DMN/20180521/DC/";
pub const DMN_DI_NS: &str = "http://www.omg.org/spec/DMN/20180521/DI/";
/// XML Schema, the namespace of `xsd:` type names in `structureRef` (language 0.5).
pub const XSD_NS: &str = "http://www.w3.org/2001/XMLSchema";

/// The deepest nesting of elements the importers read. They walk the element tree
/// recursively, so the nesting is bounded to keep them within the stack. A file within
/// it gives a document whose blocks are within the nesting the parser reads
/// (`parser::MAX_BLOCK_DEPTH`): every block of tok stands for at least one element.
pub const MAX_DEPTH: usize = 64;

/// The error of a file whose elements are nested deeper than [`MAX_DEPTH`], or `None`.
pub fn too_deep(tree: &roxmltree::Document<'_>) -> Option<String> {
    (depth(tree) > MAX_DEPTH)
        .then(|| format!("the XML elements are nested more than {MAX_DEPTH} levels deep"))
}

/// The nesting of the elements of a tree, without recursion: nodes come in document
/// order, so a parent comes before its children.
pub fn depth(tree: &roxmltree::Document<'_>) -> usize {
    let mut depths = vec![0usize; tree.descendants().count() + 1];
    let mut deepest = 0;
    for n in tree.descendants() {
        let d = n.parent().map_or(0, |p| depths[p.id().get_usize()]) + usize::from(n.is_element());
        depths[n.id().get_usize()] = d;
        deepest = deepest.max(d);
    }
    deepest
}

/// Registered namespace prefixes (empty prefix = default namespace).
pub fn prefix(ns: &str) -> &'static str {
    match ns {
        BPMN_NS => "bpmn",
        XSI_NS => "xsi",
        ZEEBE_NS => "zeebe",
        MODELER_NS => "modeler",
        DMN_NS => "",
        DMN15_NS => "",
        DMNDI15_NS => "dmndi",
        DMN_DC_NS => "dc",
        DMN_DI_NS => "di",
        XSD_NS => "xsd",
        _ => "ns0",
    }
}

/// One element: namespace URI, local name, attributes (namespace, name, value), children, text, tail.
#[derive(Debug, Clone, Default)]
pub struct XElem {
    pub ns: &'static str,
    pub name: String,
    pub attrs: Vec<(&'static str, String, String)>,
    pub children: Vec<XElem>,
    pub text: Option<String>,
    pub tail: Option<String>,
    /// Namespaces declared on this element although no tag or attribute name uses them
    /// (prefixes in attribute values, such as `structureRef="xsd:string"`).
    pub declare: Vec<&'static str>,
    /// Further namespaces with their prefixes, declared on this element when it is the root
    /// (`structure="feel:string" namespace="…"` of language 0.5).
    pub prefixed: Vec<(String, String)>,
}

impl XElem {
    pub fn new(ns: &'static str, name: &str) -> Self {
        XElem {
            ns,
            name: name.to_string(),
            ..Default::default()
        }
    }

    /// Set (or replace in place) an attribute.
    pub fn set_ns(&mut self, ns: &'static str, name: &str, value: impl Into<String>) {
        let value = value.into();
        for a in &mut self.attrs {
            if a.0 == ns && a.1 == name {
                a.2 = value;
                return;
            }
        }
        self.attrs.push((ns, name.to_string(), value));
    }

    pub fn set(&mut self, name: &str, value: impl Into<String>) {
        self.set_ns("", name, value);
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.0.is_empty() && a.1 == name)
            .map(|a| a.2.as_str())
    }

    /// Append a child and return its index.
    pub fn push(&mut self, child: XElem) -> usize {
        self.children.push(child);
        self.children.len() - 1
    }

    /// Python `list.insert` (index clamped to the end).
    pub fn insert(&mut self, pos: usize, child: XElem) {
        let pos = pos.min(self.children.len());
        self.children.insert(pos, child);
    }

    /// Clark name `{ns}name` used for sorting attributes like Python.
    fn clark(ns: &str, name: &str) -> String {
        if ns.is_empty() {
            name.to_string()
        } else {
            format!("{{{ns}}}{name}")
        }
    }

    /// Sort the attributes of every element by Clark name (as `semantic_xml` does).
    pub fn sort_attrs(&mut self) {
        self.attrs.sort_by_key(|a| Self::clark(a.0, &a.1));
        for c in &mut self.children {
            c.sort_attrs();
        }
    }

    /// Path of child indices to the first element (pre-order) whose `id` equals `id`.
    pub fn find_path(&self, id: &str) -> Option<Vec<usize>> {
        if self.get("id") == Some(id) {
            return Some(Vec::new());
        }
        for (k, c) in self.children.iter().enumerate() {
            if let Some(mut p) = c.find_path(id) {
                p.insert(0, k);
                return Some(p);
            }
        }
        None
    }

    pub fn at_mut(&mut self, path: &[usize]) -> Option<&mut XElem> {
        let mut e = self;
        for &k in path {
            e = e.children.get_mut(k)?;
        }
        Some(e)
    }

    /// ElementTree-style pretty indentation (`_indent` in the Python code).
    pub fn indent(&mut self, level: usize) {
        let pad = format!("\n{}", "  ".repeat(level));
        if !self.children.is_empty() {
            if blank(&self.text) {
                self.text = Some(format!("{pad}  "));
            }
            for c in &mut self.children {
                c.indent(level + 1);
            }
            if let Some(last) = self.children.last_mut()
                && blank(&last.tail)
            {
                last.tail = Some(pad.clone());
            }
        }
        if level > 0 && blank(&self.tail) {
            self.tail = Some(pad);
        }
    }

    fn used_namespaces(&self, out: &mut Vec<&'static str>) {
        if !self.ns.is_empty() && !out.contains(&self.ns) {
            out.push(self.ns);
        }
        for ns in &self.declare {
            if !out.contains(ns) {
                out.push(ns);
            }
        }
        for a in &self.attrs {
            if !a.0.is_empty() && !out.contains(&a.0) {
                out.push(a.0);
            }
        }
        for c in &self.children {
            c.used_namespaces(out);
        }
    }

    fn qname(ns: &str, name: &str) -> String {
        let p = prefix(ns);
        if ns.is_empty() || p.is_empty() {
            name.to_string()
        } else {
            format!("{p}:{name}")
        }
    }

    /// Serialize like `ET.tostring(root, encoding="unicode")`.
    pub fn to_string_root(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, true);
        out
    }

    fn write(&self, out: &mut String, root: bool) {
        out.push('<');
        out.push_str(&Self::qname(self.ns, &self.name));
        if root {
            let mut used = Vec::new();
            self.used_namespaces(&mut used);
            let mut decls: Vec<(&str, &str)> = used.iter().map(|ns| (prefix(ns), *ns)).collect();
            for (p, ns) in &self.prefixed {
                if !decls.iter().any(|(q, _)| q == p) {
                    decls.push((p.as_str(), ns.as_str()));
                }
            }
            decls.sort_by_key(|d| d.0);
            for (p, ns) in decls {
                if p.is_empty() {
                    out.push_str(&format!(" xmlns=\"{}\"", escape_attrib(ns)));
                } else {
                    out.push_str(&format!(" xmlns:{p}=\"{}\"", escape_attrib(ns)));
                }
            }
        }
        for (ns, name, v) in &self.attrs {
            out.push_str(&format!(
                " {}=\"{}\"",
                Self::qname(ns, name),
                escape_attrib(v)
            ));
        }
        let has_text = self.text.as_deref().map(|t| !t.is_empty()).unwrap_or(false);
        if has_text || !self.children.is_empty() {
            out.push('>');
            if let Some(t) = &self.text {
                out.push_str(&escape_cdata(t));
            }
            for c in &self.children {
                c.write(out, false);
            }
            out.push_str("</");
            out.push_str(&Self::qname(self.ns, &self.name));
            out.push('>');
        } else {
            out.push_str(" />");
        }
        if let Some(t) = &self.tail {
            out.push_str(&escape_cdata(t));
        }
    }
}

fn blank(s: &Option<String>) -> bool {
    match s {
        None => true,
        Some(t) => crate::pyutil::strip(t).is_empty(),
    }
}

pub fn escape_cdata(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn escape_attrib(s: &str) -> String {
    escape_cdata(s)
        .replace('"', "&quot;")
        .replace('\r', "&#13;")
        .replace('\n', "&#10;")
        .replace('\t', "&#09;")
}
