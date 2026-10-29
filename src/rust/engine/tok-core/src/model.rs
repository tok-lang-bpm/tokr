//! Intermediate representation of a tok document. Mirrors `model.py`: elements refer to
//! each other by identifier, and every map keeps declaration order.

use indexmap::IndexMap;

pub const EVENT_KINDS: &[&str] = &["start", "end", "catch", "throw", "boundary"];
pub const TASK_KINDS: &[&str] = &[
    "task", "user", "service", "manual", "script", "rule", "send", "receive", "call",
];
pub const SUB_KINDS: &[&str] = &["sub", "eventsub", "adhoc", "transaction"];
pub const GATEWAY_KINDS: &[&str] = &["xor", "and", "or", "eventgw", "complex"];

pub const TRIGGERS: &[&str] = &[
    "message",
    "timer",
    "signal",
    "error",
    "escalation",
    "condition",
    "link",
];
pub const TRIGGER_FLAGS: &[&str] = &["terminate", "compensate", "cancel"];
/// Triggers that cannot be combined with other triggers (language 0.5, R062).
pub const SINGLE_TRIGGERS: &[&str] = &["terminate", "cancel", "compensate", "link"];
/// Attribute keys that may repeat on one statement (language 0.5): the triggers of a
/// multiple event and the errors of an operation.
pub const REPEATABLE_KEYS: &[&str] = &[
    "message",
    "timer",
    "signal",
    "error",
    "escalation",
    "condition",
    "link",
];
/// Catch-all flags (no name): catch every error or escalation (language 0.3).
pub const CATCH_ALL_FLAGS: &[&str] = &["error", "escalation"];
/// Triggers written as flags without a value (language 0.5): an event definition without
/// a message, signal, time or condition (`error` and `escalation` are `CATCH_ALL_FLAGS`).
pub const NAMELESS_TRIGGERS: &[&str] = &["message", "timer", "signal", "condition"];
/// Documentation text of the element a statement exports (language 0.5).
pub const DOC_KEY: &str = "doc";

pub const EVENT_KEYS: &[&str] = &[
    "on",
    "message",
    "timer",
    "signal",
    "error",
    "escalation",
    "code",
    "condition",
    "link",
    "correlation",
    "form",
    "binding",
    "version",
    "activity",
    "events",
    "type",
    "retries",
    "headers",
    "outputs",
    "listeners",
    "delay",
    "assign",
    "doc",
];
pub const EVENT_FLAGS: &[&str] = &[
    "noninterrupting",
    "terminate",
    "compensate",
    "cancel",
    "error",
    "escalation",
    "message",
    "timer",
    "signal",
    "condition",
    "parallel",
];
pub const TASK_KEYS: &[&str] = &[
    "message",
    "operation",
    "process",
    "task",
    "form",
    "decision",
    "expr",
    "result",
    "type",
    "binding",
    "version",
    "child-vars",
    "parent-vars",
    "business-id",
    "while",
    "loop-max",
    "collection",
    "item",
    "output",
    "output-item",
    "until",
    "active",
    "assignee",
    "candidate-users",
    "candidate-groups",
    "due",
    "follow-up",
    "priority",
    "retries",
    "headers",
    "connector",
    "method",
    "url",
    "query",
    "headers-in",
    "body",
    "inputs",
    "outputs",
    "compensate",
    "listeners",
    "by",
    "duration",
    "assign",
    "doc",
];
pub const TASK_FLAGS: &[&str] = &[
    "loop",
    "test-before",
    "parallel",
    "sequential",
    "compensation",
    "wait",
    "instantiate",
];
/// Attributes of a complex gateway (language 0.5).
pub const COMPLEX_KEYS: &[&str] = &["activation", "doc"];
/// Attributes of the other gateways.
pub const GATEWAY_KEYS: &[&str] = &["doc"];
/// Flags of an event-based gateway (language 0.5).
pub const EVENTGW_FLAGS: &[&str] = &["instantiate", "parallel"];
/// Values of `method=` on a transaction and their BPMN form.
pub const TRANSACTION_METHODS: &[(&str, &str)] = &[
    ("compensate", "##Compensate"),
    ("store", "##Store"),
    ("image", "##Image"),
];
/// Values of `kind=` on `global` and the BPMN element of each.
pub const GLOBAL_KINDS: &[(&str, &str)] = &[
    ("task", "globalTask"),
    ("user", "globalUserTask"),
    ("manual", "globalManualTask"),
    ("script", "globalScriptTask"),
    ("rule", "globalBusinessRuleTask"),
];

/// The executable layer of language 0.3 (spec §10.3): meaningful only in executable
/// documents, rejected at the levels basic and analytic.
pub const EXEC_KEYS: &[&str] = &[
    "binding",
    "version",
    "child-vars",
    "parent-vars",
    "business-id",
    "collection",
    "item",
    "output",
    "output-item",
    "until",
    "active",
    "assignee",
    "candidate-users",
    "candidate-groups",
    "due",
    "follow-up",
    "priority",
    "retries",
    "headers",
    "connector",
    "method",
    "url",
    "query",
    "headers-in",
    "body",
    "inputs",
    "outputs",
    "listeners",
    "events",
    "version-tag",
];
pub const EXEC_FLAGS: &[&str] = &["wait"];
pub const USER_TASK_KEYS: &[&str] = &[
    "assignee",
    "candidate-users",
    "candidate-groups",
    "due",
    "follow-up",
    "priority",
];
pub const CONNECTOR_KEYS: &[&str] = &["method", "url", "query", "headers-in", "body"];
/// Attributes whose value is always a FEEL expression (written without `=`).
pub const EXPR_KEYS: &[&str] = &[
    "when",
    "expr",
    "condition",
    "correlation",
    "collection",
    "output-item",
    "until",
    "active",
    "while",
    "activation",
];
/// Language versions accepted in the header.
pub const LANGUAGE_VERSIONS: &[&str] = &["0.1", "0.3", "0.5", "0.6", "0.7", "1.0"];

pub const HEADER_KEYS: &[&str] = &[
    "id",
    "process",
    "target",
    "namespace",
    "collaboration",
    "version-tag",
    "process-name",
    "doc",
];
pub const HEADER_FLAGS: &[&str] = &["executable"];
pub const POOL_KEYS: &[&str] = &["process", "version-tag", "process-name", "doc"];
pub const POOL_FLAGS: &[&str] = &["executable"];
/// Attributes of a pool in a document of language 0.6: the participant multiplicity.
pub const POOL_KEYS_06: &[&str] = &[
    "process",
    "version-tag",
    "process-name",
    "min",
    "max",
    "doc",
];

/// Conversation nodes of a collaboration (language 0.6).
pub const CONVERSATION_KINDS: &[&str] = &["conversation", "subconversation", "callconversation"];
pub const CONVERSATION_KEYS: &[&str] = &["pools", "messages", "doc"];
pub const CALLCONVERSATION_KEYS: &[&str] = &["pools", "messages", "collaboration", "map", "doc"];
/// Choreography activities (language 0.6); they stand only in a `choreography` block.
pub const CHOREO_KINDS: &[&str] = &["choreo", "subchoreo", "callchoreo"];
pub const CHOREOGRAPHY_KEYS: &[&str] = &["doc"];
pub const CHOREO_KEYS: &[&str] = &["from", "to", "message", "reply", "doc"];
pub const SUBCHOREO_KEYS: &[&str] = &["pools", "from", "doc"];
pub const CALLCHOREO_KEYS: &[&str] = &["pools", "from", "choreography", "map", "doc"];
pub const CHOREO_FLAGS: &[&str] = &["loop", "parallel", "sequential"];
/// Flow nodes allowed in a choreography besides its activities (BPMN 2.0.2 §11.6, §11.7).
pub const CHOREOGRAPHY_NODE_KINDS: &[&str] = &[
    "start", "end", "catch", "throw", "boundary", "xor", "and", "or", "eventgw", "complex",
];
pub const LANE_KEYS: &[&str] = &["doc"];
pub const FLOW_KEYS: &[&str] = &["id", "when", "p", "doc"];
pub const FLOW_FLAGS: &[&str] = &["default"];
pub const MESSAGE_KEYS: &[&str] = &["id", "message", "delay", "doc"];
pub const DATA_KEYS: &[&str] = &["state", "type", "object", "doc"];
pub const DATA_FLAGS: &[&str] = &["collection"];
/// `reads ACTIVITY DATA input=ID` and `writes ACTIVITY DATA output=ID` (language 0.5).
pub const READS_KEYS: &[&str] = &["input"];
pub const WRITES_KEYS: &[&str] = &["output"];
pub const NOTE_KEYS: &[&str] = &["on", "doc"];
pub const GROUP_KEYS: &[&str] = &["members", "doc"];
pub const DECISION_KEYS: &[&str] = &["hit"];
pub const FIELD_KEYS: &[&str] = &["type", "options", "min", "max", "default"];
pub const FIELD_FLAGS: &[&str] = &["required"];
pub const SIMULATE_KEYS: &[&str] = &["cases", "seed", "arrival", "start"];
pub const RESOURCE_KEYS: &[&str] = &["count"];
/// Process data of language 0.5: `input`, `output`, `property`.
pub const IO_KINDS: &[&str] = &["input", "output", "property"];
pub const IO_KEYS: &[&str] = &["type", "on", "doc"];
pub const IO_FLAGS: &[&str] = &["collection"];
pub const ITEM_KEYS: &[&str] = &["structure", "namespace", "kind"];
pub const ITEM_FLAGS: &[&str] = &["collection"];
pub const ITEM_KINDS: &[&str] = &["information", "physical"];
/// Namespace prefixes the exporter uses itself; `structure=` with `namespace=` cannot
/// declare them (`xsd` always means XML Schema).
pub const RESERVED_PREFIXES: &[&str] = &[
    "bpmn", "bpmndi", "dc", "di", "xsi", "xsd", "zeebe", "modeler", "xml",
];
pub const GLOBAL_KEYS: &[&str] = &["kind", "doc"];
pub const INTERFACE_KEYS: &[&str] = &["doc"];
pub const OPERATION_KEYS: &[&str] = &["in", "out", "error", "doc"];
pub const PROP_KEYS: &[&str] = &["message", "path"];

pub const HIT_POLICIES: &[&str] = &["unique", "first", "any", "priority", "collect"];
/// Hit policies of language 0.7 (DMN 1.5 §8.2.11).
pub const HIT_POLICIES_07: &[&str] = &[
    "unique",
    "first",
    "any",
    "priority",
    "collect",
    "rule-order",
    "output-order",
];
/// Aggregations of `hit=collect` (language 0.7).
pub const AGGREGATIONS: &[&str] = &["sum", "count", "min", "max"];

/// Statements of language 0.7 (decision requirements graphs and boxed expressions); in
/// documents of earlier versions they are unknown keywords.
pub const KEYWORDS_07: &[&str] = &["inputdata", "bkm", "source", "entry", "result", "element"];
/// Attributes of `decision` in a document of language 0.7.
pub const DECISION_KEYS_07: &[&str] = &[
    "name",
    "type",
    "requires",
    "question",
    "answers",
    "hit",
    "aggregation",
    "default",
    "expr",
    "invoke",
    "with",
    "doc",
];
/// Attributes of `bkm` (language 0.7).
pub const BKM_KEYS: &[&str] = &[
    "name",
    "params",
    "type",
    "requires",
    "hit",
    "aggregation",
    "default",
    "expr",
    "invoke",
    "with",
    "doc",
];
/// Attributes of `inputdata` (language 0.7).
pub const INPUTDATA_KEYS: &[&str] = &["name", "type", "doc"];
/// Attributes of `source` (language 0.7).
pub const SOURCE_KEYS: &[&str] = &["name", "requires", "kind", "uri", "doc"];
/// Attributes of `entry`, `result` and `element` lines (language 0.7).
pub const BOX_LINE_KEYS: &[&str] = &[
    "type",
    "params",
    "hit",
    "aggregation",
    "default",
    "expr",
    "invoke",
    "with",
];
/// Attributes of an `item` in a document of language 0.7 (BPMN or DMN type).
pub const ITEM_KEYS_07: &[&str] = &["name", "structure", "namespace", "kind", "type", "allowed"];
/// Attributes of a component of a DMN type (language 0.7).
pub const COMPONENT_KEYS: &[&str] = &["type", "allowed"];
/// Built-in FEEL types usable in `type=` and in table headers (language 0.7).
pub const FEEL_TYPES: &[&str] = &[
    "string",
    "number",
    "boolean",
    "date",
    "time",
    "date and time",
    "days and time duration",
    "years and months duration",
    "list",
    "context",
    "range",
    "function",
    "any",
    "Any",
];
pub const FIELD_TYPES: &[&str] = &[
    "textfield",
    "textarea",
    "number",
    "checkbox",
    "select",
    "radio",
    "datetime",
];

/// Events that throw (data inputs, `reads`) and events that catch (data outputs, `writes`).
pub const THROWING_EVENTS: &[&str] = &["end", "throw"];
pub const CATCHING_EVENTS: &[&str] = &["start", "catch", "boundary"];

pub fn is_event(kind: &str) -> bool {
    EVENT_KINDS.contains(&kind)
}
pub fn is_sub(kind: &str) -> bool {
    SUB_KINDS.contains(&kind)
}
pub fn is_node(kind: &str) -> bool {
    EVENT_KINDS.contains(&kind)
        || TASK_KINDS.contains(&kind)
        || is_sub(kind)
        || GATEWAY_KINDS.contains(&kind)
}
pub fn is_activity(kind: &str) -> bool {
    TASK_KINDS.contains(&kind) || is_sub(kind)
}
/// A choreography activity: `choreo`, `subchoreo` or `callchoreo` (language 0.6).
pub fn is_choreo(kind: &str) -> bool {
    CHOREO_KINDS.contains(&kind)
}
/// A conversation node: `conversation`, `subconversation` or `callconversation`.
pub fn is_conversation(kind: &str) -> bool {
    CONVERSATION_KINDS.contains(&kind)
}

/// Canonical attribute keys of a flow node kind.
pub fn node_keys(kind: &str) -> &'static [&'static str] {
    if is_event(kind) {
        EVENT_KEYS
    } else if TASK_KINDS.contains(&kind) || is_sub(kind) {
        TASK_KEYS
    } else if kind == "complex" {
        COMPLEX_KEYS
    } else if GATEWAY_KINDS.contains(&kind) {
        GATEWAY_KEYS
    } else {
        match kind {
            "choreo" => CHOREO_KEYS,
            "subchoreo" => SUBCHOREO_KEYS,
            "callchoreo" => CALLCHOREO_KEYS,
            _ => &[],
        }
    }
}

/// Canonical flags of a flow node kind.
pub fn node_flags(kind: &str) -> &'static [&'static str] {
    if is_event(kind) {
        EVENT_FLAGS
    } else if TASK_KINDS.contains(&kind) || is_sub(kind) {
        TASK_FLAGS
    } else if kind == "eventgw" {
        EVENTGW_FLAGS
    } else if is_choreo(kind) {
        CHOREO_FLAGS
    } else {
        &[]
    }
}

/// Canonical attribute keys of a conversation node kind.
pub fn conversation_keys(kind: &str) -> &'static [&'static str] {
    if kind == "callconversation" {
        CALLCONVERSATION_KEYS
    } else {
        CONVERSATION_KEYS
    }
}

/// Split a list attribute (`pools="a b"`, `messages="m1 m2"`) into its words.
pub fn words(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || c == ',')
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// `map="inner:outer; …"`: participant associations as (inner, outer) pairs; `None` when
/// an item is malformed.
pub fn participant_map(text: &str) -> Option<Vec<(String, String)>> {
    let mut out = Vec::new();
    for item in text.split(';').map(str::trim).filter(|x| !x.is_empty()) {
        let (a, b) = item.split_once(':')?;
        let (a, b) = (a.trim(), b.trim());
        if a.is_empty() || b.is_empty() || b.contains(':') {
            return None;
        }
        out.push((a.to_string(), b.to_string()));
    }
    Some(out)
}

/// Attribute map in insertion order.
pub type Attrs = IndexMap<String, String>;

/// Fields shared by every statement.
#[derive(Debug, Clone, Default)]
pub struct Base {
    pub kind: String,
    pub id: Option<String>,
    pub label: Option<String>,
    pub attrs: Attrs,
    /// Further values of repeated keys (`REPEATABLE_KEYS`) in written order; the first
    /// value of each key is in `attrs` (language 0.5).
    pub extra: Vec<(String, String)>,
    pub flags: Vec<String>,
    pub line: usize,
    pub comments: Vec<String>,
    pub trailing: Option<String>,
}

impl Base {
    pub fn new(
        kind: &str,
        id: Option<String>,
        label: Option<String>,
        attrs: Attrs,
        flags: Vec<String>,
        line: usize,
    ) -> Self {
        Base {
            kind: kind.to_string(),
            id,
            label,
            attrs,
            extra: Vec::new(),
            flags,
            line,
            comments: Vec::new(),
            trailing: None,
        }
    }
    /// Every value of `key`: the one in `attrs`, then the repeated ones.
    pub fn values(&self, key: &str) -> Vec<&str> {
        let mut out: Vec<&str> = self.attr(key).into_iter().collect();
        out.extend(
            self.extra
                .iter()
                .filter(|(k, _)| k == key)
                .map(|(_, v)| v.as_str()),
        );
        out
    }
    pub fn has(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f == flag)
    }
    pub fn id(&self) -> &str {
        self.id.as_deref().unwrap_or("")
    }
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs.get(key).map(String::as_str)
    }
}

/// A flow node.
#[derive(Debug, Clone, Default)]
pub struct Node {
    pub base: Base,
    pub scope: Option<String>,
    pub lane: Option<String>,
    pub pool: Option<String>,
    pub children: Vec<String>,
    /// The choreography the node belongs to (language 0.6); `None` in processes.
    pub choreography: Option<String>,
}

/// A sequence flow.
#[derive(Debug, Clone, Default)]
pub struct Flow {
    pub base: Base,
    pub source: String,
    pub target: String,
    pub explicit_id: bool,
}

/// A message flow.
#[derive(Debug, Clone, Default)]
pub struct MessageFlow {
    pub base: Base,
    pub source: String,
    pub target: String,
    pub explicit_id: bool,
}

/// A data object (`data`), a data store (`store`), or (language 0.5) a data input
/// (`input`), data output (`output`) or property (`property`).
#[derive(Debug, Clone, Default)]
pub struct Data {
    pub base: Base,
    pub scope: Option<String>,
    pub block: Option<String>,
}

/// `reads` or `writes`.
#[derive(Debug, Clone, Default)]
pub struct Association {
    pub base: Base,
    pub activity: String,
    pub data: String,
}

/// A pool.
#[derive(Debug, Clone, Default)]
pub struct Pool {
    pub base: Base,
    pub blackbox: bool,
    pub children: Vec<String>,
}

impl Pool {
    pub fn process_id(&self) -> String {
        match self.base.attrs.get("process") {
            Some(p) if !p.is_empty() => p.clone(),
            _ => format!("{}-process", self.base.id()),
        }
    }
}

/// A lane.
#[derive(Debug, Clone, Default)]
pub struct Lane {
    pub base: Base,
    pub pool: Option<String>,
    pub parent: Option<String>,
    pub children: Vec<String>,
    /// Child lanes in XML order (BPMN import only).
    pub sub: Vec<String>,
}

/// One rule of a decision table.
#[derive(Debug, Clone, Default)]
pub struct Rule {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub annotation: Option<String>,
    pub line: usize,
    pub comments: Vec<String>,
    pub trailing: Option<String>,
}

/// A decision table.
#[derive(Debug, Clone, Default)]
pub struct Decision {
    pub base: Base,
    pub inputs: Vec<(String, String)>,
    pub outputs: Vec<(String, String)>,
    pub rules: Vec<Rule>,
    pub has_annotations: bool,
    pub annotation_title: Option<String>,
    pub header_line: Option<usize>,
    pub header_done: bool,
    /// The header row has no input cells, or no output cells (`|| out |`, `| in ||`): the
    /// canonical form leaves that side out, where a header whose cells were all malformed
    /// keeps an empty cell.
    pub no_inputs: bool,
    pub no_outputs: bool,
    /// Language 0.7: input values of the input columns and output values of the output
    /// columns (`name: type = tests`), parallel to `inputs` and `outputs`.
    pub input_values: Vec<Option<String>>,
    pub output_values: Vec<Option<String>>,
    /// Language 0.7: the header has no `||`; the rows form a relation whose columns are
    /// in `inputs`.
    pub relation: bool,
    /// Language 0.7: the `entry`, `result` and `element` lines of the block, each with its
    /// own logic (the same structure; `base.kind` names the line and `base.id` is the
    /// name of an entry).
    pub lines: Vec<Decision>,
}

/// A component of a DMN type (language 0.7): `item NAME …` in the block of an `item`.
#[derive(Debug, Clone, Default)]
pub struct Component {
    pub base: Base,
    pub children: Vec<Component>,
}

impl Decision {
    /// The table has a header or rows (a decision table or a relation).
    pub fn has_table(&self) -> bool {
        self.header_done
            || !self.inputs.is_empty()
            || !self.outputs.is_empty()
            || !self.rules.is_empty()
    }
    pub fn hit(&self) -> &str {
        self.base
            .attrs
            .get("hit")
            .map(String::as_str)
            .unwrap_or("unique")
    }
}

/// A form item: `text` or `field`.
#[derive(Debug, Clone, Default)]
pub struct FormItem {
    pub base: Base,
}

/// A form.
#[derive(Debug, Clone, Default)]
pub struct Form {
    pub base: Base,
    pub items: Vec<FormItem>,
}

/// An interface with its operations (language 0.5).
#[derive(Debug, Clone, Default)]
pub struct Interface {
    pub base: Base,
    pub operations: Vec<Base>,
}

/// A correlation key with its `prop` lines (language 0.5).
#[derive(Debug, Clone, Default)]
pub struct CorrelationKey {
    pub base: Base,
    pub props: Vec<Base>,
}

/// A conversation node of the collaboration (language 0.6): `conversation`,
/// `subconversation` (with the nodes of its block in `children`) or `callconversation`.
#[derive(Debug, Clone, Default)]
pub struct Conversation {
    pub base: Base,
    pub parent: Option<String>,
    pub children: Vec<String>,
}

impl Conversation {
    pub fn pools(&self) -> Vec<String> {
        words(self.base.attr("pools").unwrap_or(""))
    }
}

/// A choreography (language 0.6): its flow nodes are in `Document::nodes` with
/// `choreography` set, its top-level nodes listed in `children`.
#[derive(Debug, Clone, Default)]
pub struct Choreography {
    pub base: Base,
    pub children: Vec<String>,
}

/// Kind and line of a registered identifier (for duplicate and reference checks).
#[derive(Debug, Clone)]
pub struct IdEntry {
    pub kind: String,
    pub line: usize,
}

/// A parsed document.
#[derive(Debug, Clone, Default)]
pub struct Document {
    pub version: Option<String>,
    pub title: Option<String>,
    pub attrs: Attrs,
    pub flags: Vec<String>,
    pub header_line: usize,
    pub header_comments: Vec<String>,
    pub header_trailing: Option<String>,
    pub top: Vec<String>,
    pub nodes: IndexMap<String, Node>,
    pub flows: Vec<Flow>,
    pub messages: Vec<MessageFlow>,
    pub data: IndexMap<String, Data>,
    pub assocs: Vec<Association>,
    pub notes: Vec<Base>,
    pub groups: Vec<Base>,
    pub pools: Vec<Pool>,
    pub lanes: IndexMap<String, Lane>,
    pub decisions: IndexMap<String, Decision>,
    pub forms: IndexMap<String, Form>,
    pub simulate: Option<Base>,
    pub resources: IndexMap<String, Base>,
    pub variables: Vec<Base>,
    pub items: IndexMap<String, Base>,
    pub globals: IndexMap<String, Base>,
    pub interfaces: IndexMap<String, Interface>,
    pub keys: IndexMap<String, CorrelationKey>,
    pub conversations: IndexMap<String, Conversation>,
    pub choreographies: IndexMap<String, Choreography>,
    /// Language 0.7: input data, knowledge sources and business knowledge models.
    pub inputdata: IndexMap<String, Base>,
    pub sources: IndexMap<String, Base>,
    pub bkms: IndexMap<String, Decision>,
    /// Language 0.7: the components of DMN types, by item identifier.
    pub components: IndexMap<String, Vec<Component>>,
    pub end_comments: Vec<String>,
    pub ids: IndexMap<String, IdEntry>,
    /// Attribute keys and flags given more than once on one line, as (line, key), in
    /// written order; the earlier values of a key are dropped (deprecated in 1.0, `R090`).
    pub repeated: Vec<(usize, String)>,
}

impl Document {
    /// The header declares language 0.5: `resource` and `by=` also describe BPMN
    /// resources, and an event may have several triggers.
    pub fn lang_05(&self) -> bool {
        !matches!(self.version.as_deref(), Some("0.1" | "0.3") | None)
    }
    /// The header declares language 0.6 (or newer): pools have a multiplicity.
    pub fn lang_06(&self) -> bool {
        !matches!(self.version.as_deref(), Some("0.1" | "0.3" | "0.5") | None)
    }
    /// The header declares language 0.7 (or newer): decision requirements graphs, boxed
    /// expressions and FEEL at conformance level 3.
    pub fn lang_07(&self) -> bool {
        !matches!(
            self.version.as_deref(),
            Some("0.1" | "0.3" | "0.5" | "0.6") | None
        )
    }
    /// The header declares language 1.0: the language of 0.7 with its limits as rules and
    /// the deprecation warnings of 1.0 (`R090`). Unlike the predicates above, an unknown
    /// version (`P006`) does not count, so its diagnostics stay as they were.
    pub fn lang_10(&self) -> bool {
        matches!(self.version.as_deref(), Some("1.0"))
    }
    /// Attribute keys of pools in this document.
    pub fn pool_keys(&self) -> &'static [&'static str] {
        if self.lang_06() {
            POOL_KEYS_06
        } else {
            POOL_KEYS
        }
    }
    /// A flow node of a choreography (language 0.6), not of a process.
    pub fn in_choreography(&self, id: &str) -> bool {
        self.nodes.get(id).is_some_and(|n| n.choreography.is_some())
    }
    pub fn executable(&self) -> bool {
        self.flags.iter().any(|f| f == "executable")
    }
    pub fn executable_any(&self) -> bool {
        self.executable() || self.pools.iter().any(|p| p.base.has("executable"))
    }
    pub fn default_process_id(&self) -> String {
        self.attrs
            .get("process")
            .cloned()
            .unwrap_or_else(|| "process".to_string())
    }
    pub fn pool(&self, id: &str) -> Option<&Pool> {
        self.pools.iter().find(|p| p.base.id() == id)
    }
    pub fn pool_mut(&mut self, id: &str) -> Option<&mut Pool> {
        self.pools.iter_mut().find(|p| p.base.id() == id)
    }
}
