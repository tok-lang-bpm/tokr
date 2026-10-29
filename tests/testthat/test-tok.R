example <- function(name) system.file("examples", name, package = "tokr")

test_that("a valid model has no diagnostics", {
  d <- tok_check(example("wypozyczenie.tok"))
  expect_s3_class(d, "tok_diagnostics")
  expect_equal(nrow(d), 0L)
  expect_named(d, c("line", "code", "severity", "id", "message"))
})

test_that("errors are reported with line, code and message in both languages", {
  text <- c('tok 0.3 "Broken"', "start s", "flow s -> q")
  en <- tok_check(text)
  expect_true("R002" %in% en$code)
  expect_equal(en$line[en$code == "R002"][1], 3L)
  expect_equal(en$severity[en$code == "R002"][1], "error")
  pl <- tok_check(text, lang = "pl")
  expect_false(identical(en$message, pl$message))
  expect_error(tok_bpmn(text), "R002")
})

test_that("levels reject statements of higher levels", {
  d <- tok_check(example("wypozyczenie.tok"), level = "basic")
  expect_true("R040" %in% d$code)
})

test_that("canonical text is stable", {
  text <- tok_fmt(example("wypozyczenie.tok"))
  expect_identical(unclass(tok_fmt(unclass(text))), unclass(text))
})

test_that("BPMN and SVG are produced", {
  xml <- tok_bpmn(example("wypozyczenie.tok"))
  expect_match(xml, "<bpmndi:BPMNDiagram", fixed = TRUE)
  svg <- tok_svg(example("wypozyczenie.tok"), title = "Loan")
  expect_match(svg, "<title>Loan</title>", fixed = TRUE)
})

test_that("export writes BPMN, SVG, DMN and forms", {
  dir <- withr_tempdir()
  paths <- tok_export(example("propozycja-zakupu.tok"), dir = dir)
  expect_true(all(file.exists(paths)))
  expect_setequal(tools::file_ext(paths), c("bpmn", "svg", "dmn", "form"))
  expect_true(file.exists(file.path(dir, "propozycja-zakupu.bpmn")))
})

test_that("BPMN import round-trips", {
  dir <- withr_tempdir()
  tok_export(example("wypozyczenie.tok"), dir = dir)
  m <- tok_import(file.path(dir, "wypozyczenie.bpmn"))
  expect_s3_class(m, "tok_model")
  expect_equal(nrow(tok_check(m)[tok_check(m)$severity == "error", ]), 0L)
})

test_that("the knitr engine draws a diagram", {
  skip_if_not_installed("knitr")
  dir <- withr_tempdir()
  rmd <- file.path(dir, "model.Rmd")
  writeLines(c(
    "```{tok, echo=FALSE}",
    readLines(example("wypozyczenie.tok"), encoding = "UTF-8"),
    "```"
  ), rmd, useBytes = TRUE)
  out <- knitr::knit(rmd, output = file.path(dir, "model.md"), quiet = TRUE, envir = new.env())
  md <- readLines(out)
  expect_true(any(grepl(".svg", md, fixed = TRUE)))
})

test_that("decision tables are evaluated like tok eval", {
  f <- example("propozycja-zakupu.tok")
  expect_equal(tok_eval(f, "tryb-akceptacji", list(rodzaj = "drukowana", cena = 120)),
               list(tryb = "automatyczny"))
  expect_equal(tok_eval(f, "tryb-akceptacji", list(rodzaj = "drukowana", cena = 150.5)),
               list(tryb = "kierownik"))
  expect_equal(tok_eval(f, "tryb-akceptacji", list(rodzaj = "e-book", cena = 10L)),
               list(tryb = "kierownik"))
  expect_error(tok_eval(f, "nie-ma"), "unknown decision")
  expect_error(tok_eval(f, "tryb-akceptacji", list("e-book")), "named")
})

test_that("collect returns one row per matched rule", {
  text <- c(
    'tok 0.3 "Collect"',
    "decision d hit=collect",
    "  | x: number || a: string | b: number ||",
    "  | > 1       || \"big\"   | 1         ||",
    "  | > 5       || \"huge\"  | 2         ||",
    "  | < 0       || \"neg\"   | 3         ||"
  )
  r <- tok_eval(text, "d", list(x = 10))
  expect_s3_class(r, "data.frame")
  expect_equal(r$a, c("big", "huge"))
  expect_equal(r$b, c(1, 2))
})
