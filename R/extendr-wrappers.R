# Calls into the Rust engine (src/rust/src/lib.rs). Internal: use the functions in tok.R.

#' @useDynLib tokr, .registration = TRUE
#' @noRd
NULL

engine_version <- function() .Call(wrap__engine_version)

engine_check <- function(text, lang, level, bpmn) .Call(wrap__engine_check, text, lang, level, bpmn)

engine_fmt <- function(text, lang) .Call(wrap__engine_fmt, text, lang)

engine_bpmn <- function(text, lang) .Call(wrap__engine_bpmn, text, lang)

engine_svg <- function(text, title, lang) .Call(wrap__engine_svg, text, title, lang)

engine_export <- function(text, name, lang) .Call(wrap__engine_export, text, name, lang)

engine_import <- function(xml, keep_ids) .Call(wrap__engine_import, xml, keep_ids)

engine_eval <- function(text, decision, values, lang) .Call(wrap__engine_eval, text, decision, values, lang)

engine_sim <- function(text, cases, seed, lang) .Call(wrap__engine_sim, text, cases, seed, lang)

engine_sim_xes <- function(text, cases, seed, lang) .Call(wrap__engine_sim_xes, text, cases, seed, lang)
