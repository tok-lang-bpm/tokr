# tokr 1.0.3

* First release on CRAN. The package gives R the tok engine of release 1.0.3: reading,
  checking (diagnostics as a data frame, in English or Polish), formatting, BPMN 2.0 and
  SVG export, export of every file for Camunda 8, BPMN import, decision evaluation,
  simulation with the event log and the cases as data frames, XES event logs, the diagram
  in the viewer and the knitr engine `tok` for R Markdown and Quarto.
* Every exported function has examples.
* The package builds with Rust 1.88 or newer.
* The engine of 1.0.2 added three warnings of the BPMN rules: `W022` (a branch without a
  condition at an element that splits conditionally), `W023` (a sequence flow that repeats
  another one) and `W043` (a sequence flow from an element back to itself); `tok_check()`
  returns them like every other diagnostic.
* The engine of 1.0.3 adds four more: `W044` (an activity or event with several outgoing
  flows without conditions, an implicit split), `W045` and `W046` (a sub-process without its
  own start or end event) and `W047` (an event with several triggers).

# tokr 1.0.2

* Released on R-universe with the engine of release 1.0.2.

# tokr 1.0.1

* Released on R-universe (https://tok-lang-bpm.r-universe.dev) with the engine of release
  1.0.1.
