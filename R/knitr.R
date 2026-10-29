#' knitr engine for tok
#'
#' Loading the package registers the engine `tok`, so a chunk written as
#' ```` ```{tok} ```` in R Markdown or Quarto holds a model. Chunk options:
#'
#' * `tok.output`: `"svg"` (default) draws the diagram; `"check"` prints the diagnostics;
#'   `"fmt"` prints the canonical text; `"bpmn"` prints the BPMN XML.
#' * `tok.title`: title drawn above the diagram.
#' * `tok.lang`: language of the messages (default: the option `tok.lang`, then `"en"`).
#' * `tok.export`: a directory; every file of `tok export` is also written there.
#'
#' The diagram is written as an SVG file and included like a figure (`fig.cap` and the
#' other figure options apply), which suits HTML output; for PDF output use
#' `tok.output = "check"` or include an exported picture.
#'
#' @param options Chunk options, passed by knitr.
#' @return The chunk output, for knitr.
#' @examples
#' # the engine is registered when the package is loaded
#' if (requireNamespace("knitr", quietly = TRUE)) {
#'   is.function(knitr::knit_engines$get("tok"))
#' }
#' @export
eng_tok <- function(options) {
  code <- paste(options$code, collapse = "\n")
  if (isFALSE(options$eval)) return(knitr::engine_output(options, code, ""))
  lang <- if (is.null(options$tok.lang)) getOption("tok.lang", "en") else options$tok.lang
  what <- if (is.null(options$tok.output)) "svg" else options$tok.output
  if (!is.null(options$tok.export)) tok_export(code, dir = options$tok.export, lang = lang)
  if (identical(what, "check")) {
    text <- utils::capture.output(print(tok_check(code, lang = lang)))
    return(knitr::engine_output(options, code, text))
  }
  if (what %in% c("fmt", "bpmn")) {
    text <- if (what == "fmt") tok_fmt(code, lang = lang) else tok_bpmn(code, lang = lang)
    return(knitr::engine_output(options, code, unclass(text)))
  }
  if (!identical(what, "svg")) {
    stop("tok.output must be \"svg\", \"check\", \"fmt\" or \"bpmn\"", call. = FALSE)
  }
  path <- knitr::fig_path(".svg", options)
  dir.create(dirname(path), showWarnings = FALSE, recursive = TRUE)
  tok_svg(code, file = path, title = options$tok.title, lang = lang)
  out <- list(knitr::include_graphics(path))
  if (isTRUE(options$echo)) out <- c(list(structure(list(src = code), class = "source")), out)
  knitr::engine_output(options, out = out)
}

.onLoad <- function(libname, pkgname) {
  if (requireNamespace("knitr", quietly = TRUE)) {
    knitr::knit_engines$set(tok = eng_tok)
  }
}
