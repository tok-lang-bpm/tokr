# tokr 1.0.2

* First release on CRAN. The package gives R the tok engine of release 1.0.2: reading,
  checking (diagnostics as a data frame, in English or Polish), formatting, BPMN 2.0 and
  SVG export, export of every file for Camunda 8, BPMN import, decision evaluation,
  simulation with the event log and the cases as data frames, XES event logs, the diagram
  in the viewer and the knitr engine `tok` for R Markdown and Quarto.
* Every exported function has examples.
* The engine of 1.0.2 adds three warnings of the BPMN rules: `W022` (a branch without a
  condition at an element that splits conditionally), `W023` (a sequence flow that repeats
  another one) and `W043` (a sequence flow from an element back to itself); `tok_check()`
  returns them like every other diagnostic.

# tokr 1.0.1

* Released on R-universe (https://tok-lang-bpm.r-universe.dev) with the engine of release
  1.0.1.
