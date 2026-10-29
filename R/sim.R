#' Simulate a process
#'
#' Runs the discrete-event simulation of `tok sim`: cases arrive, wait for the resources
#' of `by=`, take the durations of `duration=` and choose flows by their conditions and
#' probabilities, as the `simulate`, `resource` and `variable` statements of the model
#' say. The same model and seed always give the same result, number by number the result
#' of `tok sim` and of the Python package.
#'
#' Times of the simulation are calendar times counted from `start=` of `simulate`. In the
#' event log and the table of cases they are `POSIXct` values in UTC; a `start=` without a
#' UTC offset is taken as UTC, so the clock times are those of `tok sim --log`.
#'
#' The event log has one row per activity instance, as `tok sim --log` writes it: the
#' columns `case`, `activity` (identifier), `label`, `resource` (`""` when the activity has
#' no `by=`), `enqueue` (when the case reached the activity), `start` (when a performer
#' started the work; `NA` when the activity was interrupted while waiting), `end` (when it
#' completed or was interrupted) and `status` (`"completed"` or `"interrupted"`). The
#' columns `case_id`, `activity_id` (the label, or the identifier when the activity has no
#' label), `activity_instance_id` (the row number), `resource_id` (`NA` when none),
#' `schedule` (= `enqueue`) and `complete` (= `end` of completed instances, `NA` for
#' interrupted ones) repeat them under the names process-mining packages expect. For
#' example, with bupaR:
#'
#' ```
#' bupaR::activitylog(s$log, case_id = "case_id", activity_id = "activity_id",
#'                    resource_id = "resource_id",
#'                    timestamps = c("schedule", "start", "complete"))
#' ```
#'
#' @inheritParams tok_check
#' @param cases Number of cases; `NULL` takes `cases=` of `simulate` (default 100).
#' @param seed Seed of the random number generator; `NULL` takes `seed=` of `simulate`
#'   (default 1). Like `tok sim`, the simulator takes the integer part of the number, so
#'   `seed = 7.9` is the seed 7.
#' @return An object of class `tok_sim`, a list with the elements:
#'   * `kpi`: the indicators of `tok sim --json`, a list with `seed`; `cases` (a list of
#'     `started`, `finished` and `by_end_event`, a named integer vector of finished cases
#'     per end event); `cycle_time` (a list of `mean_s`, `median_s` and `p90_s` in seconds,
#'     `NA` without finished cases, and the same as short texts `mean`, `median`, `p90`);
#'     `activities`, a data frame with one row per activity (`activity`, `label`, `count`
#'     of completed instances, `mean_wait_s`, `mean_processing_s`, `mean_wait`,
#'     `mean_processing`); `resources`, a data frame with one row per resource
#'     (`resource`, `label`, `count`, `utilization` as a fraction); `horizon_s`, the
#'     simulated period in seconds; and `warnings`.
#'   * `log`: the event log, a data frame described above.
#'   * `cases`: a data frame with one row per case: `case`, `start`, `end` (`NA` for an
#'     unfinished case), `end_event` (`NA` for an unfinished case), `cycle_time_s`, and
#'     one column per case variable. A variable that holds a single value in every case
#'     is a vector (logical, numeric or character, `NA` where the case has no value;
#'     durations in seconds), any other a list; a variable named like one of the first
#'     five columns gets the prefix `variable_`.
#'   * `warnings`: conditions or expressions the simulator could not evaluate.
#'   * `seed`: the seed used, as a number.
#'   * `horizon`: the simulated period in seconds, from the first arrival to the last
#'     event.
#'   * `report`: the text report of `tok sim` in the language `lang`, which `print()`
#'     shows.
#'
#'   The object also keeps the model, the number of cases and the seed of the run, so
#'   [tok_xes()] can write its event log as XES.
#'
#'   `as.data.frame()` returns the event log. The function stops with the errors of the
#'   model (see [tok_check()]) and with the errors of the simulator as `tok sim` reports
#'   them, for example `tok.sim.SimError: the process 'process' has no start event`.
#' @examples
#' f <- system.file("examples", "kwerenda-symulacja.tok", package = "tokr")
#' s <- tok_sim(f)
#' s
#' s$kpi$resources
#' head(s$log)
#' table(tok_sim(f, cases = 50, seed = 3)$cases$end_event)
#' @export
tok_sim <- function(x, cases = NULL, seed = NULL, lang = getOption("tok.lang", "en")) {
  m <- as_model(x)
  lang <- check_lang(lang)
  res <- engine_sim(m$text, sim_param(cases, "cases"), sim_param(seed, "seed"), lang)
  seed <- as.numeric(res$seed)
  kpi <- list(
    seed = seed,
    cases = list(
      started = res$started,
      finished = res$finished,
      by_end_event = stats::setNames(res$by_end_event$cases, res$by_end_event$end_event)
    ),
    cycle_time = res$cycle_time,
    activities = as.data.frame(res$activities, stringsAsFactors = FALSE),
    resources = as.data.frame(res$resources, stringsAsFactors = FALSE),
    horizon_s = res$horizon_s,
    warnings = res$warnings
  )
  structure(
    list(
      kpi = kpi, log = sim_log(res$log), cases = sim_cases(res$cases),
      warnings = res$warnings, seed = seed, horizon = res$horizon_s,
      report = res$report
    ),
    class = "tok_sim",
    run = list(text = m$text, cases = res$started, seed = seed)
  )
}

#' XES event log of a simulation
#'
#' Writes the event log of [tok_sim()] as an XES document (IEEE 1849-2016, eXtensible
#' Event Stream), the exchange format of process-mining tools such as ProM, pm4py,
#' bupaR, Disco and Celonis; it is the file of `tok sim --xes`.
#'
#' The log has one trace per case and up to three events per activity instance, with the
#' transitions of the standard lifecycle model: `schedule` when the case reached the
#' activity, `start` when a performer started the work and `complete` when it completed;
#' an instance interrupted by a boundary timer ends with `ati_abort` when it was running
#' and with `withdraw` when it was still waiting for a performer. Events carry
#' `concept:name` (the label of the activity), `concept:instance`, `org:resource`,
#' `time:timestamp` (with milliseconds and the UTC offset of `start=` of `simulate`) and
#' `activity` (the identifier). Traces carry the case number, `start`, `end`,
#' `end_event`, `cycle_time_s` and the case variables as typed attributes. See
#' `docs/simulation.md` in the repository for the details.
#'
#' @param x A result of [tok_sim()], whose run is repeated with the same model, number of
#'   cases and seed (which gives the same log), or a model: a `tok_model`, the path of a
#'   `.tok` file, or the text of a model, which is simulated with `cases` and `seed`.
#' @param file Optional path to write to.
#' @param cases,seed As in [tok_sim()]; only for a model, not for a result of
#'   [tok_sim()].
#' @inheritParams tok_check
#' @return The XES text; invisibly when written to `file`.
#' @examples
#' f <- system.file("examples", "kwerenda-symulacja.tok", package = "tokr")
#' s <- tok_sim(f, cases = 20)
#' xes <- tempfile(fileext = ".xes")
#' tok_xes(s, xes)
#' substr(tok_xes(f, cases = 2, seed = 7), 1, 200)
#' @export
tok_xes <- function(x, file = NULL, cases = NULL, seed = NULL,
                    lang = getOption("tok.lang", "en")) {
  lang <- check_lang(lang)
  if (inherits(x, "tok_sim")) {
    if (!is.null(cases) || !is.null(seed)) {
      stop("`cases` and `seed` are those of the simulation `x`", call. = FALSE)
    }
    run <- attr(x, "run")
    if (is.null(run)) stop("`x` does not keep its model; run tok_sim() again", call. = FALSE)
    text <- run$text
    cases <- run$cases
    seed <- run$seed
  } else {
    text <- as_model(x)$text
    cases <- sim_param(cases, "cases")
    seed <- sim_param(seed, "seed")
  }
  out(engine_sim_xes(text, cases, seed, lang), file)
}

sim_param <- function(x, name) {
  if (is.null(x)) return(NULL)
  if (!is.numeric(x) || length(x) != 1L || is.na(x)) {
    stop("`", name, "` must be a single number", call. = FALSE)
  }
  x
}

utc <- function(seconds) .POSIXct(as.numeric(seconds), tz = "UTC")

sim_log <- function(lg) {
  activity_id <- lg$label
  unlabelled <- !nzchar(activity_id)
  activity_id[unlabelled] <- lg$activity[unlabelled]
  resource_id <- lg$resource
  resource_id[!nzchar(resource_id)] <- NA_character_
  complete <- as.numeric(lg$end)
  complete[lg$status != "completed"] <- NA_real_
  data.frame(
    case = lg$case, activity = lg$activity, label = lg$label, resource = lg$resource,
    enqueue = utc(lg$enqueue), start = utc(lg$start), end = utc(lg$end),
    status = lg$status,
    case_id = lg$case, activity_id = activity_id,
    activity_instance_id = seq_along(lg$case), resource_id = resource_id,
    schedule = utc(lg$enqueue), complete = utc(complete),
    stringsAsFactors = FALSE
  )
}

sim_cases <- function(cs) {
  d <- data.frame(
    case = cs$case, start = utc(cs$start), end = utc(cs$end),
    end_event = cs$end_event, cycle_time_s = as.numeric(cs$cycle_time_s),
    stringsAsFactors = FALSE
  )
  vars <- cs$variables
  keys <- unique(unlist(lapply(vars, names), use.names = FALSE))
  for (k in keys) {
    values <- lapply(vars, function(v) v[[k]])
    col <- if (k %in% names(d)) paste0("variable_", k) else k
    d[[col]] <- simplify_values(values)
  }
  d
}

# One value per case: an atomic vector when every value is a single value of one type
# (NULL becomes NA), otherwise the list.
simplify_values <- function(values) {
  present <- !vapply(values, is.null, logical(1))
  single <- vapply(values[present], function(v) is.atomic(v) && length(v) == 1L, logical(1))
  types <- unique(vapply(values[present], typeof, character(1)))
  if (!all(single) || length(types) > 1L) return(values)
  out <- rep(NA, length(values))
  if (any(present)) out[present] <- unlist(values[present], use.names = FALSE)
  out
}

#' @export
format.tok_sim <- function(x, ...) x$report

#' @export
print.tok_sim <- function(x, ...) {
  cat(x$report)
  invisible(x)
}

#' @export
as.data.frame.tok_sim <- function(x, row.names = NULL, optional = FALSE, ...) x$log
