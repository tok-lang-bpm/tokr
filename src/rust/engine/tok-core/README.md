# tok-core

The core of the tok process language: lexer, parser, checker with diagnostics in English and
Polish, canonical printer, BPMN 2.0 export and import, DMN export and import (DMN 1.3 decision
tables, DMN 1.5 decision requirements graphs and boxed expressions), FEEL (the subset of
languages 0.1–0.6 and DMN 1.5 conformance level 3), the evaluation of decisions and Camunda
form export.

A tok document is plain text; one file describes a BPMN 2.0 process with its decisions, forms
and simulation parameters:

```
tok 1.0 "Wypożyczenie"

start s "Potrzeba książki"
user z "Złożenie zamówienia"
end e "Zamówienie złożone"

flow s -> z -> e
```

```rust
let (mut doc, diags) = tok_core::parse(text);
let more = tok_core::check(&mut doc, "full", None);
let canonical = tok_core::fmt(&doc);
let bpmn = tok_core::export::semantic_xml(&doc, None);
```

The language is specified in `docs/spec.md` of the repository, its diagnostics in
`docs/diagnostics.md`; the conformance corpus in `conformance/` fixes the outputs. The
automatic diagram layout and SVG are in the crate `tok-bpmn`, the simulator in `tok-sim`, the
`tok` command in `tok-cli`.

License: GPL-3.0-or-later.
