#' Read a tok model
#'
#' Reads a `.tok` file. The result can be passed to every other function of the package;
#' they also accept the path of a `.tok` file or the text of a model directly.
#'
#' @param path Path of a `.tok` file.
#' @return An object of class `tok_model`: a list with the elements `text`, `path` and
#'   `name` (the file name without the extension).
#' @examples
#' m <- tok_read(system.file("examples", "wypozyczenie.tok", package = "tokr"))
#' m
#' @export
tok_read <- function(path) {
  stopifnot(is.character(path), length(path) == 1L)
  if (!file.exists(path)) stop("file not found: ", path, call. = FALSE)
  text <- paste(readLines(path, encoding = "UTF-8", warn = FALSE), collapse = "\n")
  structure(
    list(text = enc2utf8(text), path = path, name = sub("\\.tok$", "", basename(path))),
    class = "tok_model"
  )
}

#' @export
print.tok_model <- function(x, ...) {
  header <- strsplit(x$text, "\n", fixed = TRUE)[[1]]
  cat("<tok model", if (!is.null(x$path)) paste0(" ", x$path), ">\n", sep = "")
  cat(utils::head(header, 12L), sep = "\n")
  if (length(header) > 12L) cat("... (", length(header) - 12L, " more lines)\n", sep = "")
  invisible(x)
}

# A model given as a tok_model, the path of a .tok file, or text (one or more lines).
as_model <- function(x) {
  if (inherits(x, "tok_model")) return(x)
  if (!is.character(x) || !length(x)) {
    stop("`x` must be a tok model, the path of a .tok file, or the text of a model", call. = FALSE)
  }
  if (length(x) == 1L && !grepl("\n", x, fixed = TRUE) && grepl("\\.tok$", x)) {
    return(tok_read(x))
  }
  structure(list(text = enc2utf8(paste(x, collapse = "\n")), path = NULL, name = "model"),
            class = "tok_model")
}

check_lang <- function(lang) {
  if (!identical(lang, "en") && !identical(lang, "pl")) {
    stop("`lang` must be \"en\" or \"pl\"", call. = FALSE)
  }
  lang
}

#' Check a model
#'
#' Runs the checks of `tok check`: syntax, identifiers, flows, pools and lanes, decision
#' tables, forms, simulation parameters and the BPMN 2.0 rules.
#'
#' @param x A model: a `tok_model`, the path of a `.tok` file, or the text of a model.
#' @param lang Language of the messages, `"en"` or `"pl"`. The default comes from the
#'   option `tok.lang`.
#' @param level `"full"` (default), `"analytic"` or `"basic"`: the lower levels also report
#'   the use of statements that belong to a higher one.
#' @param bpmn Whether to run the BPMN 2.0 rules on the exported model.
#' @return A data frame of class `tok_diagnostics` with the columns `line`, `code`,
#'   `severity` (`"error"`, `"warning"` or `"note"`), `id` and `message`, one row per
#'   problem; no rows when the model has no problems.
#' @examples
#' tok_check('tok 0.3 "Example"
#' start s "Start"
#' task a "Work"
#' end e "End"
#' flow s -> a -> e')
#' @export
tok_check <- function(x, lang = getOption("tok.lang", "en"),
                      level = c("full", "analytic", "basic"), bpmn = TRUE) {
  m <- as_model(x)
  level <- match.arg(level)
  cols <- engine_check(m$text, check_lang(lang), level, isTRUE(bpmn))
  d <- data.frame(
    line = cols$line, code = cols$code, severity = cols$severity,
    id = cols$id, message = cols$message, stringsAsFactors = FALSE
  )
  structure(d, class = c("tok_diagnostics", "data.frame"), source = m$path)
}

#' @export
print.tok_diagnostics <- function(x, ...) {
  src <- attr(x, "source")
  prefix <- if (is.null(src)) "line" else src
  if (!nrow(x)) {
    cat("no problems\n")
  } else {
    for (i in seq_len(nrow(x))) {
      cat(sprintf("%s:%d: %s %s: %s\n", prefix, x$line[i], x$severity[i], x$code[i], x$message[i]))
    }
  }
  n <- table(factor(x$severity, levels = c("error", "warning", "note")))
  cat(sprintf("%d error(s), %d warning(s), %d note(s)\n", n[["error"]], n[["warning"]], n[["note"]]))
  invisible(x)
}

#' Canonical text of a model
#'
#' Prints the model as `tok fmt` does: one statement per line, canonical order of
#' attributes, comments kept.
#'
#' @inheritParams tok_check
#' @param write When `TRUE` and `x` is a file, the file is rewritten.
#' @return The canonical text (invisibly when the file is rewritten).
#' @examples
#' tok_fmt(system.file("examples", "wypozyczenie.tok", package = "tokr"))
#' @export
tok_fmt <- function(x, write = FALSE, lang = getOption("tok.lang", "en")) {
  m <- as_model(x)
  text <- engine_fmt(m$text, check_lang(lang))
  if (isTRUE(write)) {
    if (is.null(m$path)) stop("`write = TRUE` needs a model read from a file", call. = FALSE)
    write_utf8(text, m$path)
    return(invisible(text))
  }
  structure(text, class = c("tok_text", "character"))
}

#' @export
print.tok_text <- function(x, ...) {
  cat(x, if (!endsWith(x, "\n")) "\n", sep = "")
  invisible(x)
}

#' BPMN 2.0 XML and SVG of a model
#'
#' `tok_bpmn()` exports BPMN 2.0 XML with diagram layout, which opens in Camunda Modeler
#' and bpmn.io; `tok_svg()` draws the diagram. Both stop with the list of errors when the
#' model has errors (see [tok_check()]).
#'
#' @inheritParams tok_check
#' @param file Optional path to write to.
#' @param title Title drawn above the diagram (SVG only).
#' @return The XML or SVG text; invisibly when written to `file`.
#' @examples
#' model <- system.file("examples", "wypozyczenie.tok", package = "tokr")
#' xml <- tok_bpmn(model)
#' substr(xml, 1, 120)
#' svg <- tok_svg(model, title = "Interlibrary loan")
#' nchar(svg)
#' @export
tok_bpmn <- function(x, file = NULL, lang = getOption("tok.lang", "en")) {
  out(engine_bpmn(as_model(x)$text, check_lang(lang)), file)
}

#' @rdname tok_bpmn
#' @export
tok_svg <- function(x, file = NULL, title = NULL, lang = getOption("tok.lang", "en")) {
  out(engine_svg(as_model(x)$text, if (is.null(title)) "" else title, check_lang(lang)), file)
}

out <- function(text, file) {
  if (is.null(file)) return(structure(text, class = c("tok_text", "character")))
  write_utf8(text, file)
  invisible(text)
}

write_utf8 <- function(text, path) {
  con <- file(path, open = "wb")
  on.exit(close(con))
  writeBin(charToRaw(enc2utf8(text)), con)
}

#' Draw a model
#'
#' Shows the diagram in the RStudio or Positron viewer, or in the web browser.
#'
#' @inheritParams tok_svg
#' @return The path of the temporary SVG file, invisibly.
#' @examples
#' if (interactive()) {
#'   tok_plot(system.file("examples", "wypozyczenie.tok", package = "tokr"))
#' }
#' @export
tok_plot <- function(x, title = NULL, lang = getOption("tok.lang", "en")) {
  m <- as_model(x)
  svg <- tok_svg(m, title = title, lang = lang)
  dir <- tempfile("tok")
  dir.create(dir)
  path <- file.path(dir, paste0(m$name, ".svg"))
  write_utf8(svg, path)
  page <- file.path(dir, "index.html")
  write_utf8(paste0(
    "<!doctype html><meta charset=\"utf-8\"><title>", m$name, "</title>",
    "<body style=\"margin:0;background:#fff\">", svg, "</body>"
  ), page)
  viewer <- getOption("viewer")
  if (is.function(viewer)) viewer(page) else utils::browseURL(page)
  invisible(path)
}

#' Export every file of a model
#'
#' Writes what `tok export` writes: `NAME.bpmn`, `NAME.svg`, one `.dmn` file per decision
#' table and one `.form` file per form, ready to deploy to Camunda 8.
#'
#' @inheritParams tok_check
#' @param dir Output directory, created when missing.
#' @param name Base name of the BPMN and SVG files; by default the name of the `.tok` file.
#' @return The paths of the written files, invisibly.
#' @examples
#' out <- file.path(tempdir(), "tok-export")
#' files <- tok_export(system.file("examples", "propozycja-zakupu.tok", package = "tokr"),
#'                     dir = out)
#' basename(files)
#' unlink(out, recursive = TRUE)
#' @export
tok_export <- function(x, dir = ".", name = NULL, lang = getOption("tok.lang", "en")) {
  m <- as_model(x)
  files <- engine_export(m$text, if (is.null(name)) m$name else name, check_lang(lang))
  dir.create(dir, showWarnings = FALSE, recursive = TRUE)
  paths <- file.path(dir, names(files))
  for (i in seq_along(files)) write_utf8(files[[i]], paths[[i]])
  invisible(paths)
}

#' Convert BPMN 2.0 to tok
#'
#' Reads a BPMN 2.0 file (for example drawn in Camunda Modeler) and returns the model as tok
#' text, as `tok import` does.
#'
#' @param file Path of a `.bpmn` file.
#' @param keep_ids Keep the identifiers of the BPMN file; `FALSE` derives readable ones
#'   from the labels.
#' @return A `tok_model`; warnings about elements tok does not keep are in its element
#'   `warnings` and are also signalled.
#' @examples
#' bpmn <- tempfile(fileext = ".bpmn")
#' tok_bpmn(system.file("examples", "wypozyczenie.tok", package = "tokr"), file = bpmn)
#' m <- tok_import(bpmn)
#' m
#' unlink(bpmn)
#' @export
tok_import <- function(file, keep_ids = TRUE) {
  xml <- paste(readLines(file, encoding = "UTF-8", warn = FALSE), collapse = "\n")
  res <- engine_import(enc2utf8(xml), isTRUE(keep_ids))
  for (w in res$warnings) warning(w, call. = FALSE)
  structure(
    list(text = res$text, path = NULL, name = sub("\\.bpmn$", "", basename(file)),
         warnings = res$warnings),
    class = "tok_model"
  )
}

#' Evaluate a decision
#'
#' Evaluates a decision of the model for the given inputs, as `tok eval` does. In a
#' model of language 0.7 the decision is evaluated with the decisions, input data and
#' business knowledge models it requires, and the inputs are the input data by FEEL
#' name; in earlier models the decision table alone is evaluated with the hit policy of
#' the table.
#'
#' @inheritParams tok_check
#' @param decision Identifier of the decision.
#' @param inputs Named list of inputs: numbers, strings, logical values, `NA` for null;
#'   a vector longer than one is a list, a named list is a context.
#' @return In a model of language 0.7 the value of the decision (a number, string,
#'   logical value, list or named list; `NULL` for null), a list of named lists as a data
#'   frame. In earlier models: for the hit policies `unique`, `first`, `any` and
#'   `priority` a named list output name → value (`NULL` when no rule matches); for
#'   `collect` a data frame with one row per matched rule.
#' @examples
#' f <- system.file("examples", "propozycja-zakupu.tok", package = "tokr")
#' tok_eval(f, "tryb-akceptacji", list(rodzaj = "drukowana", cena = 120))
#' @export
tok_eval <- function(x, decision, inputs = list(), lang = getOption("tok.lang", "en")) {
  inputs <- as.list(inputs)
  if (length(inputs) && (is.null(names(inputs)) || any(!nzchar(names(inputs))))) {
    stop("`inputs` must be a named list: list(name = value, ...)", call. = FALSE)
  }
  res <- engine_eval(as_model(x)$text, decision, inputs, check_lang(lang))
  if (is.list(res) && is.null(names(res)) && length(res) &&
      all(vapply(res, function(r) is.list(r) && !is.null(names(r)), logical(1)))) {
    cols <- names(res[[1]])
    if (all(vapply(res, function(r) identical(names(r), cols), logical(1)))) {
      return(as.data.frame(
        lapply(stats::setNames(cols, cols), function(k) {
          unlist(lapply(res, function(r) if (is.null(r[[k]])) NA else r[[k]]))
        }),
        stringsAsFactors = FALSE
      ))
    }
  }
  res
}

#' Version of the tok engine
#'
#' @return The version of the Rust engine compiled into the package.
#' @examples
#' tok_version()
#' @export
tok_version <- function() engine_version()
