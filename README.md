# tokr: tok for R

The R package `tokr` reads, checks, formats, exports and simulates process models written in [tok](https://github.com/tok-lang-bpm/tok), and draws them in the viewer and in R Markdown or Quarto documents. The work is done by the Rust engine of tok, the same one the `tok` command and the Python package use, so the results are identical.

## Install

The package compiles its Rust engine during installation. You need:

- R 4.2 or newer, and on Windows [Rtools](https://cran.r-project.org/bin/windows/Rtools/) for your R version;
- Rust 1.99 or newer from <https://www.rust-lang.org/tools/install>; on Windows also run `rustup target add x86_64-pc-windows-gnu` once.

From CRAN:

```r
install.packages("tokr")
```

From R-universe, which also has binary packages for Windows and macOS:

```r
install.packages("tokr", repos = c("https://tok-lang-bpm.r-universe.dev", "https://cloud.r-project.org"))
```

From a release archive `tokr_VERSION.tar.gz` (it contains every Rust crate it needs and installs without network access; `r/build.sh` of the repository builds it):

```r
install.packages("tokr_1.0.2.tar.gz", repos = NULL, type = "source")
```

From a checkout of the repository:

```
R CMD INSTALL r/tokr
```

## Use

```r
library(tokr)

m <- tok_read("wypozyczenie.tok")
tok_check(m)                    # diagnostics as a data frame
tok_check(m, lang = "pl")       # messages in Polish
tok_plot(m)                     # diagram in the viewer
tok_export(m, dir = "camunda")  # BPMN, SVG, DMN and forms for Camunda 8
tok_fmt(m)                      # canonical text
tok_import("model.bpmn")        # BPMN 2.0 drawn in Camunda Modeler -> tok
tok_eval("propozycja-zakupu.tok", "tryb-akceptacji",
         list(rodzaj = "drukowana", cena = 120))   # decision table, as tok eval
```

Every function takes a model read with `tok_read()`, the path of a `.tok` file, or the text of a model. `options(tok.lang = "pl")` switches all messages to Polish.

## Simulation

`tok_sim()` runs the simulation of `tok sim` on a model with `simulate`, `resource` and `variable` statements (see `docs/simulation.md` in the repository). The same model and seed give the same numbers as `tok sim` and the Python package.

```r
s <- tok_sim("kwerenda-symulacja.tok")   # cases and seed of the simulate statement
s                                        # the report of tok sim
s <- tok_sim("kwerenda-symulacja.tok", cases = 1000, seed = 3, lang = "pl")
s$kpi$activities                         # count, mean wait and processing per activity
s$kpi$resources                          # utilization per resource
s$cases                                  # one row per case, with the case variables
log <- as.data.frame(s)                  # the event log
```

The event log has one row per activity instance, with the columns of `tok sim --log` (`case`, `activity`, `label`, `resource`, `enqueue`, `start`, `end`, `status`) and the same data under the names process-mining packages use (`case_id`, `activity_id`, `activity_instance_id`, `resource_id`, `schedule`, `start`, `complete`). Times are `POSIXct` in UTC. With bupaR:

```r
bupaR::activitylog(log, case_id = "case_id", activity_id = "activity_id",
                   resource_id = "resource_id",
                   timestamps = c("schedule", "start", "complete"))
```

See `?tok_sim` for every element of the result.

### XES event log

`tok_xes()` writes the event log as an XES document (IEEE 1849-2016), the file of `tok sim --xes`, which process-mining tools such as ProM, pm4py, bupaR and Disco read directly. It takes a result of `tok_sim()` (the run is repeated with the same model, number of cases and seed, so the log is the same) or a model:

```r
tok_xes(s, "kwerenda.xes")                                       # the log of s
tok_xes("kwerenda-symulacja.tok", "kwerenda.xes", cases = 500, seed = 3)
```

Each case is a trace; each activity instance gives the events `schedule`, `start` and `complete` (or `ati_abort` and `withdraw` for instances interrupted by a boundary timer while running or while waiting), with the label, the resource and timestamps with milliseconds. See `?tok_xes`.

## In documents

Loading the package registers a knitr engine. A chunk written as

````
```{tok, fig.cap = "Interlibrary loan"}
tok 0.3 "Interlibrary loan"
start s "Request received"
user check "Check the request"
end e "Request handled"
flow s -> check -> e
```
````

draws the diagram. With `tok.output = "check"` the chunk prints the diagnostics instead, with `"fmt"` the canonical text and with `"bpmn"` the BPMN XML; `tok.export = "dir"` also writes every exported file. See `?eng_tok`.

## Build the release archive

`r/build.sh` (in the repository) copies the engine crates into the package, vendors the external crates and runs `R CMD build`; the archive is written to `dist/`.
