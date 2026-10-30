//! Diagnostic codes, severities and messages (English and Polish). Mirrors `diagnostics.py`.

use crate::json::{Json, obj};

/// Severity of a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Note,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        }
    }
    fn polish(self) -> &'static str {
        match self {
            Severity::Error => "błąd",
            Severity::Warning => "ostrzeżenie",
            Severity::Note => "uwaga",
        }
    }
}

use Severity::{Error as E, Warning as W};

/// (code, severity, English template, Polish template)
pub const MESSAGES: &[(&str, Severity, &str, &str)] = &[
    (
        "T001",
        E,
        "invalid token: {detail}",
        "niepoprawny znak lub tekst: {detail}",
    ),
    (
        "T002",
        E,
        "tab character in indentation; use spaces",
        "tabulator we wcięciu; używaj spacji",
    ),
    (
        "T003",
        E,
        "indentation does not match any open block",
        "wcięcie nie pasuje do żadnego otwartego bloku",
    ),
    (
        "P001",
        E,
        "unknown keyword '{word}'",
        "nieznane słowo kluczowe „{word}”",
    ),
    (
        "P002",
        E,
        "'{keyword}' needs an identifier",
        "„{keyword}” wymaga identyfikatora",
    ),
    (
        "P003",
        E,
        "'{keyword}' cannot have an indented block",
        "„{keyword}” nie może mieć wciętego bloku",
    ),
    (
        "P004",
        E,
        "'{keyword}' is not allowed inside {context}",
        "„{keyword}” nie może wystąpić w bloku {context}",
    ),
    (
        "P005",
        E,
        "the document must start with a header such as 'tok 1.0'",
        "dokument musi zaczynać się nagłówkiem, na przykład „tok 1.0”",
    ),
    (
        "P006",
        W,
        "unknown language version {version}; this tool reads versions 0.1, 0.3, 0.5, 0.6, 0.7 and 1.0",
        "nieznana wersja języka {version}; to narzędzie czyta wersje 0.1, 0.3, 0.5, 0.6, 0.7 i 1.0",
    ),
    (
        "P007",
        E,
        "malformed attribute near '{near}'",
        "niepoprawny atrybut w pobliżu „{near}”",
    ),
    (
        "P008",
        E,
        "malformed {keyword} line: {detail}",
        "niepoprawny wiersz {keyword}: {detail}",
    ),
    (
        "P009",
        E,
        "malformed decision table row: {detail}",
        "niepoprawny wiersz tabeli decyzyjnej: {detail}",
    ),
    ("P010", E, "a second header", "drugi nagłówek dokumentu"),
    (
        "R001",
        E,
        "identifier '{id}' is already used on line {other}",
        "identyfikator „{id}” jest już użyty w wierszu {other}",
    ),
    (
        "R002",
        E,
        "unknown {what} '{id}'",
        "nieznany element ({what_pl}): „{id}”",
    ),
    (
        "R003",
        E,
        "'{id}' is a '{actual}' element; required here: {expected}",
        "„{id}” to {actual}, a tu wymagany jest element: {expected_pl}",
    ),
    (
        "R004",
        E,
        "identifier '{id}' must use only ASCII letters, digits, '_' and '-'",
        "identyfikator „{id}” może zawierać tylko litery ASCII, cyfry, „_” i „-”",
    ),
    (
        "R010",
        W,
        "unknown attribute '{key}' on {kind}",
        "nieznany atrybut „{key}” w elemencie {kind}",
    ),
    (
        "R011",
        E,
        "unknown flag '{flag}' on {kind}",
        "nieznana flaga „{flag}” w elemencie {kind}",
    ),
    (
        "R012",
        E,
        "an event can have only one trigger ({triggers})",
        "zdarzenie może mieć tylko jeden wyzwalacz ({triggers})",
    ),
    (
        "R013",
        E,
        "'{key}' requires {requirement}",
        "„{key}” wymaga: {requirement}",
    ),
    (
        "R020",
        E,
        "a flow line with a label or attributes must contain exactly one arrow",
        "wiersz flow z etykietą lub atrybutami musi mieć dokładnie jedną strzałkę",
    ),
    (
        "R021",
        E,
        "sequence flow {source} -> {target} crosses a process or sub-process boundary",
        "przepływ sekwencji {source} -> {target} przekracza granicę procesu lub podprocesu",
    ),
    (
        "R022",
        E,
        "message flow {source} -> {target} must connect two different pools",
        "przepływ komunikatów {source} -> {target} musi łączyć dwa różne baseny",
    ),
    (
        "R030",
        E,
        "'{id}' is in a pool with lanes but outside every lane",
        "„{id}” leży w basenie z liniami, ale poza liniami",
    ),
    (
        "R031",
        E,
        "'{id}' is declared at top level, but the document has pools; move it into a pool",
        "„{id}” zadeklarowano poza basenem, a dokument ma baseny; przenieś go do basenu",
    ),
    (
        "R032",
        E,
        "boundary event '{id}' needs on=ACTIVITY",
        "zdarzenie graniczne „{id}” wymaga on=CZYNNOŚĆ",
    ),
    (
        "R050",
        E,
        "ad-hoc sub-process '{id}' must contain at least one activity and no start or end events",
        "podproces ad hoc „{id}” musi zawierać co najmniej jedną czynność i żadnych zdarzeń początkowych ani końcowych",
    ),
    ("R051", E, "{detail}", "{detail_pl}"),
    (
        "R052",
        E,
        "events= is not allowed on the root-level start event '{id}'",
        "events= nie może wystąpić na zdarzeniu początkowym procesu „{id}”",
    ),
    (
        "R053",
        E,
        "malformed {key}=\"…\" list: '{item}' (expected {expected})",
        "niepoprawna lista {key}=\"…\": „{item}” (oczekiwano: {expected_pl})",
    ),
    ("R054", E, "{detail}", "{detail_pl}"),
    (
        "R055",
        W,
        "'{key}' belongs to the executable layer and is ignored because the document is not executable",
        "„{key}” należy do warstwy wykonywalnej i jest pomijany, bo dokument nie jest wykonywalny",
    ),
    ("R060", E, "{detail}", "{detail_pl}"),
    ("R061", E, "{detail}", "{detail_pl}"),
    ("R062", E, "{detail}", "{detail_pl}"),
    ("R063", E, "{detail}", "{detail_pl}"),
    (
        "R064",
        W,
        "activity '{activity}' reads and writes data object '{data}' in the same state '{state}'; a state transition writes another state",
        "czynność „{activity}” odczytuje i zapisuje obiekt danych „{data}” w tym samym stanie „{state}”; przejście stanu zapisuje inny stan",
    ),
    ("R065", E, "{detail}", "{detail_pl}"),
    (
        "R066",
        E,
        "unknown {what} '{id}'; declare it with '{keyword}'",
        "nieznany element ({what_pl}): „{id}”; zadeklaruj go słowem „{keyword}”",
    ),
    ("R070", E, "{detail}", "{detail_pl}"),
    (
        "R071",
        E,
        "message flow '{flow}' named by '{id}' does not connect two of its pools",
        "przepływ komunikatów „{flow}” wskazany w „{id}” nie łączy dwóch basenów tej konwersacji",
    ),
    ("R072", E, "{detail}", "{detail_pl}"),
    ("R073", E, "{detail}", "{detail_pl}"),
    (
        "R074",
        E,
        "participant multiplicity of pool '{id}' needs whole numbers with 0 <= min <= max and max >= 1 (min is 2 when only max is given, max is 1 when only min is given)",
        "krotność uczestnika w basenie „{id}” wymaga liczb całkowitych, dla których 0 <= min <= max i max >= 1 (gdy podano tylko max, min wynosi 2; gdy podano tylko min, max wynosi 1)",
    ),
    (
        "R075",
        E,
        "'{id}' ({what}) is outside the BPMN conformance class {class}; it belongs to the class {needed}",
        "„{id}” ({what}) wykracza poza klasę zgodności BPMN {class}; należy do klasy {needed}",
    ),
    (
        "R076",
        E,
        "'{id}' requires itself through {path}; the requirements of a decision requirements graph form no cycle",
        "„{id}” wymaga samego siebie przez {path}; wymagania w diagramie wymagań decyzyjnych nie mogą tworzyć cyklu",
    ),
    ("R077", E, "{detail}", "{detail_pl}"),
    ("R078", E, "{detail}", "{detail_pl}"),
    (
        "R079",
        W,
        "'{id}' uses '{name}' ({kind} '{other}') without requiring it; add it to requires=",
        "„{id}” używa „{name}” ({kind_pl} „{other}”), choć nie wymaga tego elementu; dodaj go do requires=",
    ),
    (
        "R080",
        E,
        "the name '{name}' is used twice ('{a}' and '{b}'); the names of a decision requirements graph, of the entries of one context and of the parameters of one function are unique",
        "nazwa „{name}” występuje dwa razy („{a}” i „{b}”); nazwy w diagramie wymagań decyzyjnych, wpisów jednego kontekstu i parametrów jednej funkcji muszą być różne",
    ),
    ("R081", E, "{detail}", "{detail_pl}"),
    (
        "R090",
        W,
        "'{key}' is given more than once on this line; only the last value counts (deprecated in language 1.0)",
        "„{key}” występuje w tym wierszu więcej niż raz; liczy się tylko ostatnia wartość (przestarzałe w języku 1.0)",
    ),
    (
        "C100",
        E,
        "transaction '{id}' is not executed by Camunda 8.10",
        "transakcja „{id}” nie jest wykonywana przez Camunda 8.10",
    ),
    (
        "C101",
        E,
        "cancel event '{id}' is not executed by Camunda 8.10",
        "zdarzenie anulowania „{id}” nie jest wykonywane przez Camunda 8.10",
    ),
    (
        "C102",
        E,
        "complex gateway '{id}' is not executed by Camunda 8.10",
        "bramka złożona „{id}” nie jest wykonywana przez Camunda 8.10",
    ),
    (
        "C103",
        E,
        "standard loop of '{id}' (while=, test-before, loop-max) is not executed by Camunda 8.10; use multi-instance",
        "pętla standardowa „{id}” (while=, test-before, loop-max) nie jest wykonywana przez Camunda 8.10; użyj wielu instancji",
    ),
    (
        "C104",
        E,
        "multiple event '{id}' (several triggers) is not executed by Camunda 8.10",
        "zdarzenie wielokrotne „{id}” (kilka wyzwalaczy) nie jest wykonywane przez Camunda 8.10",
    ),
    (
        "C105",
        E,
        "parallel multiple event '{id}' is not executed by Camunda 8.10",
        "równoległe zdarzenie wielokrotne „{id}” nie jest wykonywane przez Camunda 8.10",
    ),
    (
        "C106",
        E,
        "instantiating receive task '{id}' is not executed by Camunda 8.10; use a message start event",
        "zadanie odbioru „{id}” rozpoczynające proces nie jest wykonywane przez Camunda 8.10; użyj zdarzenia początkowego wiadomości",
    ),
    (
        "C107",
        E,
        "instantiating event-based gateway '{id}' is not executed by Camunda 8.10",
        "bramka oparta na zdarzeniach „{id}” rozpoczynająca proces nie jest wykonywana przez Camunda 8.10",
    ),
    (
        "C108",
        E,
        "call activity '{id}' calls the global task '{task}', which Camunda 8.10 does not execute",
        "czynność wywołania „{id}” wywołuje zadanie globalne „{task}”, którego Camunda 8.10 nie wykonuje",
    ),
    (
        "C109",
        W,
        "operation '{operation}' of '{id}' is ignored by Camunda 8.10 (workers take jobs by type=)",
        "operacja „{operation}” w „{id}” jest pomijana przez Camunda 8.10 (workery pobierają zadania według type=)",
    ),
    (
        "C110",
        W,
        "correlation key '{id}' is ignored by Camunda 8.10 (messages are correlated by correlation=)",
        "klucz korelacji „{id}” jest pomijany przez Camunda 8.10 (wiadomości koreluje correlation=)",
    ),
    (
        "C120",
        W,
        "conversation '{id}' is ignored by Camunda 8.10 (conversations describe the collaboration, not its execution)",
        "konwersacja „{id}” jest pomijana przez Camunda 8.10 (konwersacje opisują współpracę, a nie jej wykonanie)",
    ),
    (
        "C121",
        W,
        "call conversation '{id}' is ignored by Camunda 8.10",
        "wywołanie konwersacji „{id}” jest pomijane przez Camunda 8.10",
    ),
    (
        "C122",
        E,
        "choreography '{id}' is not executed by Camunda 8.10",
        "choreografia „{id}” nie jest wykonywana przez Camunda 8.10",
    ),
    (
        "C123",
        W,
        "participant multiplicity of pool '{id}' is ignored by Camunda 8.10",
        "krotność uczestnika w basenie „{id}” jest pomijana przez Camunda 8.10",
    ),
    (
        "C130",
        E,
        "business knowledge model '{id}' whose logic is an invocation is not executed by Camunda 8.10",
        "model wiedzy biznesowej „{id}”, którego logiką jest wywołanie, nie jest wykonywany przez Camunda 8.10",
    ),
    (
        "C131",
        E,
        "a function definition in '{id}' whose body is not a literal expression is not executed by Camunda 8.10",
        "definicja funkcji w „{id}”, której treścią nie jest wyrażenie literalne, nie jest wykonywana przez Camunda 8.10",
    ),
    (
        "C132",
        E,
        "decision '{id}' has no decision logic; Camunda 8.10 does not deploy it",
        "decyzja „{id}” nie ma logiki decyzyjnej; Camunda 8.10 jej nie wdroży",
    ),
    (
        "C022",
        E,
        "message {kind} event '{id}' needs type=\"…\" (Camunda executes it as a job)",
        "zdarzenie {kind_pl} wiadomości „{id}” wymaga type=\"…\" (Camunda wykonuje je jako zadanie dla workera)",
    ),
    (
        "C090",
        E,
        "{key}=\"{value}\" must be an ISO 8601 date-time with a zone (e.g. 2027-01-15T12:00:00+01:00) or an expression starting with =",
        "{key}=\"{value}\" musi być datą i godziną ISO 8601 ze strefą (np. 2027-01-15T12:00:00+01:00) albo wyrażeniem zaczynającym się od =",
    ),
    (
        "C091",
        E,
        "priority must be an integer from 0 to 100 or an expression starting with =, not '{value}'",
        "priority musi być liczbą całkowitą od 0 do 100 albo wyrażeniem zaczynającym się od =, a nie „{value}”",
    ),
    (
        "R040",
        E,
        "'{keyword}' is not available at level {level}",
        "„{keyword}” nie jest dostępne na poziomie {level}",
    ),
    (
        "D001",
        E,
        "decision '{id}' has no header row",
        "decyzja „{id}” nie ma wiersza nagłówka",
    ),
    (
        "D002",
        E,
        "rule has {got} cells, the header has {expected}",
        "reguła ma {got} komórek, a nagłówek {expected}",
    ),
    (
        "D003",
        E,
        "unknown hit policy '{hit}'",
        "nieznana polityka trafień „{hit}”",
    ),
    (
        "D004",
        E,
        "header cell '{cell}' must be 'name: type' with type string, number, boolean or date",
        "komórka nagłówka „{cell}” musi mieć postać „nazwa: typ” (string, number, boolean, date)",
    ),
    (
        "D005",
        W,
        "decision '{id}' has no rules",
        "decyzja „{id}” nie ma reguł",
    ),
    (
        "D010",
        W,
        "rules {a} and {b} can both match {example} (hit policy unique)",
        "reguły {a} i {b} mogą pasować jednocześnie, np. {example} (polityka unique)",
    ),
    (
        "D006",
        E,
        "hit policy {hit} orders the matching rules by output values; give them for at least one output (name: type = …)",
        "polityka trafień {hit} porządkuje pasujące reguły według wartości wyników; podaj je co najmniej dla jednego wyniku (nazwa: typ = …)",
    ),
    ("D007", E, "{detail}", "{detail_pl}"),
    (
        "D008",
        E,
        "row has {got} cells, the relation has {expected} columns",
        "wiersz ma {got} komórek, a relacja ma {expected} kolumn",
    ),
    (
        "D009",
        E,
        "header cell '{cell}' must be 'name: type' or 'name: type = values' (language 0.7)",
        "komórka nagłówka „{cell}” musi mieć postać „nazwa: typ” albo „nazwa: typ = wartości” (język 0.7)",
    ),
    (
        "M001",
        E,
        "unknown field type '{type}'",
        "nieznany typ pola „{type}”",
    ),
    (
        "M002",
        E,
        "field '{id}' of type {type} needs options=\"value:Label, …\"",
        "pole „{id}” typu {type} wymaga options=\"wartość:Etykieta, …\"",
    ),
    (
        "F001",
        W,
        "FEEL expression outside the supported subset: {detail}",
        "wyrażenie FEEL spoza obsługiwanego podzbioru: {detail_pl}",
    ),
    (
        "F002",
        E,
        "FEEL syntax error: {detail}",
        "błąd składni FEEL: {detail_pl}",
    ),
    (
        "S001",
        E,
        "invalid distribution '{text}': {detail}",
        "niepoprawny rozkład „{text}”: {detail}",
    ),
    (
        "S002",
        W,
        "probabilities of the flows leaving '{id}' sum to {total}, not 1",
        "prawdopodobieństwa przepływów z „{id}” sumują się do {total}, a nie do 1",
    ),
    (
        "S003",
        W,
        "gateway '{id}' has neither conditions nor probabilities; the simulator takes the first flow",
        "bramka „{id}” nie ma ani warunków, ani prawdopodobieństw; symulator wybierze pierwszy przepływ",
    ),
];

/// English text for the BPMN rules (the validator's own messages are Polish).
pub const BPMN_EN: &[(&str, &str)] = &[
    ("E001", "the file is not well-formed XML"),
    ("E002", "the root element is not bpmn:definitions"),
    ("E003", "an id is used more than once"),
    ("E004", "reference to a non-existent id"),
    (
        "E005",
        "sequence flow without both ends or connecting data, a pool or an annotation",
    ),
    ("E006", "sequence flow between pools or into a sub-process"),
    ("E007", "message flow from an element outside the pools"),
    (
        "E008",
        "message flow inside one pool (use a sequence flow between lanes)",
    ),
    (
        "E009",
        "message flow to a gateway, data or a non-message event",
    ),
    ("E010", "trigger not allowed for this event position"),
    (
        "E011",
        "incoming flow to a start event or outgoing flow from an end event",
    ),
    (
        "E012",
        "invalid boundary event (on a gateway, with incoming flow, non-interrupting error, …)",
    ),
    (
        "E013",
        "event sub-process with sequence flows or without a triggered start event",
    ),
    ("E014", "compensation activity with sequence flows"),
    (
        "E015",
        "default flow that does not leave the element or has a condition",
    ),
    (
        "E016",
        "event-based gateway followed by something other than catch events or receive tasks",
    ),
    ("E017", "condition or default flow on a parallel gateway"),
    ("E018", "reference to a declaration of the wrong type"),
    ("E030", "element in two lanes"),
    ("W010", "timer event without a time"),
    (
        "W016",
        "event after an event-based gateway has other incoming flows",
    ),
    (
        "W020",
        "gateway with one incoming and one outgoing flow is unnecessary",
    ),
    (
        "W021",
        "branch of an XOR/OR split without label, condition or default status",
    ),
    (
        "W022",
        "branch without a condition at an element that splits conditionally (it has a default flow or other branches have conditions)",
    ),
    (
        "W023",
        "sequence flow repeats another one (same source, target and condition)",
    ),
    ("W030", "lane refers to an element inside a sub-process"),
    ("W031", "element outside every lane"),
    ("W040", "process without a start or end event"),
    ("W041", "element not reachable from a start event"),
    (
        "W042",
        "path without an end event (element without outgoing flow)",
    ),
    ("W043", "sequence flow from an element back to itself"),
    (
        "W044",
        "activity or event with several outgoing flows without conditions (implicit split; all of them are taken)",
    ),
    ("W045", "sub-process without a start event"),
    ("W046", "sub-process without an end event"),
    (
        "W047",
        "event with several triggers (multiple event), reported as an error by bpmn.io linters",
    ),
    ("W050", "OR split without a default flow"),
    (
        "W051",
        "join of a different type than the split (e.g. AND split joined by XOR)",
    ),
    (
        "W052",
        "boundary escalation/error on a sub-process that nothing inside throws",
    ),
    ("W053", "escalation thrown but never caught"),
    (
        "W054",
        "message task or event without a message flow in a model with several pools",
    ),
    ("L010", "message event without messageRef"),
    ("L020", "name of a splitting gateway is not a question"),
    ("L021", "gateway both joins and splits"),
    ("L030", "task without a name"),
];

/// English text for what the BPMN validator reports only for BPMN files (`tok check
/// FILE.bpmn`): the Camunda 8 profile (`--camunda`; the codes of the executable profile of
/// tok documents), the diagram (D) and the step numbers (K, `--steps`, `--steps-from`).
pub const BPMN_FILE_EN: &[(&str, &str)] = &[
    (
        "C001",
        "no modeler:executionPlatform=\"Camunda Cloud\" on <bpmn:definitions>; Camunda Modeler opens the file as a Camunda 7 model",
    ),
    (
        "C002",
        "no process has isExecutable=\"true\"; the engine will not deploy the model",
    ),
    (
        "C010",
        "user task without <zeebe:userTask/>; it does not reach Tasklist as a Camunda user task",
    ),
    (
        "C011",
        "user task without a form (zeebe:formDefinition); in Tasklist it can only be completed without data",
    ),
    (
        "C012",
        "the form of the user task is in no .form file of the model's folder",
    ),
    (
        "C020",
        "service or send task without <zeebe:taskDefinition type=\"…\"/>; the deployment fails",
    ),
    (
        "C021",
        "service or send task waits for a worker of its job type; without one the instance stops there",
    ),
    (
        "C030",
        "business rule task without <zeebe:calledDecision decisionId=\"…\" resultVariable=\"…\"/>",
    ),
    (
        "C031",
        "zeebe:calledDecision without decisionId or resultVariable",
    ),
    (
        "C032",
        "the decision of the business rule task is in no .dmn file of the model's folder",
    ),
    (
        "C040",
        "script task without <zeebe:script expression=\"=…\" resultVariable=\"…\"/>",
    ),
    (
        "C041",
        "zeebe:script: the FEEL expression must start with '=' and resultVariable is required",
    ),
    ("C050", "message event or task without messageRef"),
    (
        "C051",
        "message without a name; the engine correlates messages by name",
    ),
    (
        "C052",
        "message caught by an event or receive task without <zeebe:subscription correlationKey=\"=…\"/>",
    ),
    ("C060", "element not executed by Camunda 8"),
    (
        "C070",
        "flow leaving a gateway without a condition; the engine needs one on every outgoing flow except the default",
    ),
    (
        "C071",
        "condition of a flow is not a FEEL expression starting with '=' (e.g. =approved = true)",
    ),
    (
        "C080",
        "timer event without a value (timeDuration, timeDate or timeCycle)",
    ),
    (
        "D01",
        "diagram coordinates of an element that is no longer in the model; run tok bpmn on the file to lay it out again",
    ),
    (
        "D02",
        "elements without diagram coordinates (BPMNDI); run tok bpmn on the file to lay it out",
    ),
    (
        "K01",
        "name does not start with the step number of the process description (e.g. '12. Request for quotation')",
    ),
    (
        "K02",
        "a step of the process description has no element in the model",
    ),
    (
        "K03",
        "an element carries a step number that is not in the process description",
    ),
];

pub const LEVELS: &[&str] = &["basic", "analytic", "full"];

fn entry(code: &str) -> Option<&'static (&'static str, Severity, &'static str, &'static str)> {
    MESSAGES.iter().find(|e| e.0 == code)
}

/// One diagnostic. Arguments are stored as text, as Python formats them.
#[derive(Debug, Clone)]
pub struct Diag {
    pub code: String,
    pub line: usize,
    pub id: Option<String>,
    pub args: Vec<(String, String)>,
    pub text_pl: Option<String>,
    pub text_en: Option<String>,
    pub severity_override: Option<Severity>,
}

impl Diag {
    pub fn new(code: &str, line: usize, id: Option<&str>, args: &[(&str, String)]) -> Self {
        Diag {
            code: code.to_string(),
            line,
            id: id.map(str::to_string),
            args: args
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
            text_pl: None,
            text_en: None,
            severity_override: None,
        }
    }

    /// A diagnostic with ready texts (executable profile and BPMN rules).
    pub fn text(
        code: &str,
        line: usize,
        id: Option<&str>,
        sev: Severity,
        pl: String,
        en: Option<String>,
    ) -> Self {
        Diag {
            code: code.to_string(),
            line,
            id: id.map(str::to_string),
            args: Vec::new(),
            text_pl: Some(pl),
            text_en: en,
            severity_override: Some(sev),
        }
    }

    pub fn severity(&self) -> Severity {
        if let Some(s) = self.severity_override {
            return s;
        }
        entry(&self.code).map(|e| e.1).unwrap_or(Severity::Error)
    }

    pub fn is_error(&self) -> bool {
        self.severity() == Severity::Error
    }

    pub fn message(&self, lang: &str) -> String {
        if let Some(pl) = &self.text_pl {
            if lang == "en" {
                if let Some(en) = self.text_en.as_deref().filter(|t| !t.is_empty()) {
                    return en.to_string();
                }
                if let Some((_, t)) = BPMN_EN
                    .iter()
                    .chain(BPMN_FILE_EN)
                    .find(|e| e.0 == self.code)
                {
                    return match self.id.as_deref() {
                        Some(id) if !id.is_empty() => format!("{t} ({id})"),
                        _ => t.to_string(),
                    };
                }
            }
            return pl.clone();
        }
        let Some(e) = entry(&self.code) else {
            return self.code.clone();
        };
        let template = if lang == "en" { e.2 } else { e.3 };
        format_template(template, &self.args).unwrap_or_else(|| template.to_string())
    }

    pub fn to_json(&self, lang: &str) -> Json {
        obj(vec![
            ("code", Json::Str(self.code.clone())),
            ("severity", Json::Str(self.severity().as_str().into())),
            ("line", Json::Int(self.line as i64)),
            ("id", self.id.clone().map(Json::Str).unwrap_or(Json::Null)),
            ("message", Json::Str(self.message(lang))),
        ])
    }
}

/// Python `str.format(**args)` for `{name}` placeholders; None when a name is missing.
fn format_template(template: &str, args: &[(String, String)]) -> Option<String> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find('}')?;
        let name = &after[..end];
        let value = args.iter().find(|(k, _)| k == name)?;
        out.push_str(&value.1);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// One line of `tok check` output.
pub fn format_diag(d: &Diag, lang: &str, path: Option<&str>) -> String {
    let sev = if lang == "en" {
        d.severity().as_str()
    } else {
        d.severity().polish()
    };
    let place = match path {
        Some(p) => format!("{p}:{}", d.line),
        None if lang == "en" => format!("line {}", d.line),
        None => format!("wiersz {}", d.line),
    };
    format!("{place}: {sev} {}: {}", d.code, d.message(lang))
}

/// Stable sort by (line, code), as the Python CLI does.
pub fn sort(diags: &mut [Diag]) {
    diags.sort_by(|a, b| (a.line, &a.code).cmp(&(b.line, &b.code)));
}
