# The files in fixtures/ were written by `tok sim` (the command line) for the same models:
# NAME.txt is the printed report, *-log.csv and *-cases.csv the files of --log and
# --cases-csv.

example <- function(name) system.file("examples", name, package = "tokr")
fixture <- function(name) test_path("fixtures", name)
report_of <- function(name) {
  paste0(paste(readLines(fixture(name), encoding = "UTF-8"), collapse = "\n"), "\n")
}
read_fixture_csv <- function(name) {
  utils::read.csv(fixture(name), colClasses = "character", fileEncoding = "UTF-8",
                  na.strings = character())
}
iso <- function(t) {
  out <- format(t, "%Y-%m-%dT%H:%M:%S", tz = "UTC")
  out[is.na(t)] <- ""
  out
}
kwerenda <- function() example("kwerenda-symulacja.tok")
no_start <- c('tok 0.3 "No start"', 'task a "A"', 'end e "E"', "flow a -> e")

test_that("the report is the one of tok sim", {
  runs <- list(
    list("kwerenda-symulacja.txt", kwerenda(), NULL, NULL, "en"),
    list("kwerenda-symulacja-pl-20-3.txt", kwerenda(), 20, 3, "pl"),
    list("wypozyczenie.txt", example("wypozyczenie.tok"), NULL, NULL, "en"),
    list("propozycja-zakupu-pl-10-42.txt", example("propozycja-zakupu.tok"), 10L, 42L, "pl"),
    list("loan-desk.txt", fixture("loan-desk.tok"), NULL, NULL, "en"),
    list("loan-desk-pl.txt", fixture("loan-desk.tok"), NULL, NULL, "pl")
  )
  for (r in runs) {
    s <- tok_sim(r[[2]], cases = r[[3]], seed = r[[4]], lang = r[[5]])
    expect_identical(format(s), report_of(r[[1]]), label = r[[1]])
    expect_identical(capture.output(print(s)), readLines(fixture(r[[1]]), encoding = "UTF-8"),
                     label = r[[1]])
  }
})

test_that("the option tok.lang chooses the language of the report", {
  old <- options(tok.lang = "pl")
  on.exit(options(old))
  expect_identical(format(tok_sim(kwerenda(), cases = 20, seed = 3)),
                   report_of("kwerenda-symulacja-pl-20-3.txt"))
})

test_that("a seed gives the same result every time", {
  a <- tok_sim(kwerenda(), cases = 30, seed = 11)
  expect_identical(tok_sim(kwerenda(), cases = 30, seed = 11), a)
  expect_identical(tok_sim(kwerenda(), cases = 30L, seed = 11L), a)
  expect_identical(tok_sim(kwerenda(), cases = 30.7, seed = 11.9), a)
  expect_identical(tok_sim(kwerenda(), cases = 30, seed = a$seed), a)
  expect_equal(a$seed, 11)
  b <- tok_sim(kwerenda(), cases = 30, seed = 12)
  expect_false(identical(a$log$end, b$log$end))
  expect_equal(tok_sim(kwerenda())$seed, 7)
  expect_equal(tok_sim(example("wypozyczenie.tok"))$seed, 1)
})

test_that("the event log is the one of tok sim --log", {
  s <- tok_sim(kwerenda(), cases = 20, seed = 3)
  csv <- read_fixture_csv("kwerenda-log.csv")
  log <- s$log
  expect_equal(nrow(log), nrow(csv))
  expect_identical(log$case, as.integer(csv$case))
  for (col in c("activity", "label", "resource", "status")) {
    expect_identical(log[[col]], csv[[col]], label = col)
  }
  for (col in c("enqueue", "start", "end")) {
    expect_identical(iso(log[[col]]), csv[[col]], label = col)
  }
})

test_that("times with a UTC offset are converted to UTC", {
  s <- tok_sim(fixture("loan-desk.tok"))
  csv <- read_fixture_csv("loan-desk-log.csv")
  expect_true(all(grepl("+02:00$", csv$enqueue)))
  for (col in c("enqueue", "start", "end")) {
    local <- iso(s$log[[col]] + 7200)
    local[is.na(s$log[[col]])] <- ""
    expect_identical(local, sub("+02:00", "", csv[[col]], fixed = TRUE), label = col)
  }
  expect_equal(format(s$cases$start[1], "%Y-%m-%d %H:%M:%S", tz = "UTC"), "2027-05-03 06:00:00")
})

test_that("the event log has the columns of tok and of process mining", {
  s <- tok_sim(fixture("loan-desk.tok"))
  log <- s$log
  expect_s3_class(log, "data.frame")
  expect_named(log, c(
    "case", "activity", "label", "resource", "enqueue", "start", "end", "status",
    "case_id", "activity_id", "activity_instance_id", "resource_id", "schedule", "complete"
  ))
  for (col in c("enqueue", "start", "end", "schedule", "complete")) {
    expect_s3_class(log[[col]], "POSIXct")
    expect_identical(attr(log[[col]], "tzone"), "UTC")
  }
  expect_identical(log$case_id, log$case)
  expect_identical(log$activity_id, log$label)
  expect_identical(log$activity_instance_id, seq_len(nrow(log)))
  expect_identical(log$resource_id, log$resource)
  expect_identical(log$schedule, log$enqueue)
  done <- log$status == "completed"
  expect_true(any(!done))
  expect_identical(log$complete[done], log$end[done])
  expect_true(all(is.na(log$complete[!done])))
  expect_true(all(log$enqueue <= log$end))
  expect_true(all(is.na(log$start) | (log$enqueue <= log$start & log$start <= log$end)))
  expect_identical(as.data.frame(s), log)
})

test_that("activities without a label or a resource are identified by their id", {
  text <- c('tok 0.3 "Plain"', "start s", "task a duration=10m", "end e", "flow s -> a -> e")
  log <- tok_sim(text, cases = 2)$log
  expect_identical(log$activity_id, c("a", "a"))
  expect_identical(log$resource, c("", ""))
  expect_true(all(is.na(log$resource_id)))
  expect_equal(as.numeric(log$end - log$start, units = "secs"), c(600, 600))
})

test_that("the table of cases is the one of tok sim --cases-csv", {
  s <- tok_sim(kwerenda(), cases = 20, seed = 3)
  csv <- read_fixture_csv("kwerenda-cases.csv")
  cases <- s$cases
  expect_identical(cases$case, as.integer(csv$case))
  expect_identical(iso(cases$start), csv$start)
  expect_identical(iso(cases$end), csv$end)
  expect_identical(cases$end_event, csv$end_event)
  expect_identical(cases$cycle_time_s, as.numeric(csv$cycle_time_s))
  expect_equal(nrow(cases), s$kpi$cases$started)
})

test_that("case variables become columns", {
  cases <- tok_sim(fixture("loan-desk.tok"))$cases
  expect_named(cases, c("case", "start", "end", "end_event", "cycle_time_s",
                        "kind", "price", "returned"))
  expect_type(cases$kind, "character")
  expect_true(all(cases$kind %in% c("print", "ebook")))
  expect_type(cases$price, "double")
  expect_true(all(cases$price >= 10 & cases$price <= 200))
  expect_type(cases$returned, "logical")
  lent <- cases$end_event == "done"
  expect_false(anyNA(cases$returned[lent]))
  expect_true(all(is.na(cases$returned[!lent])))
})

test_that("a variable named like a column gets a prefix", {
  text <- c('tok 0.3 "Clash"', "start s", "end e", "flow s -> e",
            "simulate cases=3", 'variable case "fixed(5)"', 'variable when "2h"')
  cases <- tok_sim(text)$cases
  expect_identical(cases$variable_case, c(5, 5, 5))
  expect_identical(cases$when, c(7200, 7200, 7200))
})

test_that("the indicators are those of tok sim --json", {
  s <- tok_sim(kwerenda())
  k <- s$kpi
  expect_named(k, c("seed", "cases", "cycle_time", "activities", "resources",
                    "horizon_s", "warnings"))
  expect_identical(k$cases$by_end_event, c(zakonczona = 252L, odrzucony = 48L))
  expect_identical(k$cases$started, 300L)
  expect_identical(k$cases$finished, 300L)
  expect_identical(k$cycle_time$mean, "17.5 h")
  expect_identical(k$cycle_time$p90, "1.5 d")
  expect_equal(k$cycle_time$mean_s, mean(s$cases$cycle_time_s), tolerance = 1e-6)
  expect_named(k$activities, c("activity", "label", "count", "mean_wait_s",
                               "mean_processing_s", "mean_wait", "mean_processing"))
  expect_identical(k$activities$count, c(300L, 300L, 252L, 252L, 252L, 48L))
  expect_identical(sum(k$activities$count), sum(s$log$status == "completed"))
  expect_identical(k$resources$resource, c("archiwisci", "kancelisci"))
  expect_identical(k$resources$count, c(2, 1))
  expect_identical(k$resources$utilization, c(0.8091, 0.0917))
  expect_identical(s$horizon, k$horizon_s)
  expect_identical(s$seed, 7)
  expect_identical(s$warnings, character())
})

test_that("a run without cases gives empty tables", {
  s <- tok_sim(c('tok 0.3 "Empty"', 'start s "S"', 'end e "E"', "flow s -> e"), cases = 0)
  expect_equal(nrow(s$log), 0L)
  expect_equal(nrow(s$cases), 0L)
  expect_true(is.na(s$kpi$cycle_time$mean_s))
  expect_match(format(s), "0 cases started, 0 finished", fixed = TRUE)
})

test_that("errors are reported as tok sim reports them", {
  expect_error(tok_sim(no_start), "tok.sim.SimError: the process 'process' has no start event",
               fixed = TRUE)
  broken <- c('tok 0.3 "Broken"', 'start s "S"', "flow s -> q")
  expect_error(tok_sim(broken), "line 3: error R002: unknown flow node 'q'", fixed = TRUE)
  expect_error(tok_sim(broken, lang = "pl"), "wiersz 3: błąd R002", fixed = TRUE)
  expect_error(tok_sim(kwerenda(), seed = Inf),
               "OverflowError: cannot convert float infinity to integer", fixed = TRUE)
  expect_error(tok_sim(kwerenda(), seed = NA), "`seed` must be a single number", fixed = TRUE)
  expect_error(tok_sim(kwerenda(), seed = "7"), "`seed` must be a single number", fixed = TRUE)
  expect_error(tok_sim(kwerenda(), cases = 1:2), "`cases` must be a single number", fixed = TRUE)
  expect_error(tok_sim(kwerenda(), lang = "de"), "`lang`")
})

# fixtures/repair-desk.xes was written by `tok sim repair-desk.tok --xes`; the model draws
# only uniform and triangular durations, so the file is the same on every platform.
test_that("tok_xes gives the XES file of tok sim --xes", {
  repair <- fixture("repair-desk.tok")
  want <- paste(readLines(fixture("repair-desk.xes"), encoding = "UTF-8"), collapse = "\n")
  want <- paste0(want, "\n")
  s <- tok_sim(repair)
  xes <- tok_xes(s)
  expect_s3_class(xes, "tok_text")
  expect_identical(as.character(unclass(xes)), want)
  expect_identical(as.character(unclass(tok_xes(repair))), want)
  expect_identical(as.character(unclass(tok_xes(tok_read(repair), cases = 8, seed = 3))), want)
  dir <- withr_tempdir()
  path <- file.path(dir, "log.xes")
  expect_invisible(tok_xes(s, path))
  expect_identical(readBin(path, "raw", file.size(path)), charToRaw(enc2utf8(want)))
})

test_that("the XES log has one trace per case and the events of the event log", {
  s <- tok_sim(kwerenda(), cases = 15, seed = 4)
  xes <- tok_xes(s)
  count <- function(pattern) lengths(regmatches(xes, gregexpr(pattern, xes, fixed = TRUE)))
  expect_true(startsWith(xes, '<?xml version="1.0" encoding="UTF-8"?>\n<log xes.version="1.0">'))
  expect_equal(count("<trace>"), nrow(s$cases))
  expect_equal(count("<event>"), nrow(s$log) + sum(!is.na(s$log$start)) + sum(!is.na(s$log$end)))
  expect_equal(count('value="schedule"'), nrow(s$log))
  expect_equal(count('value="complete"'), sum(s$log$status == "completed") + 1L)
  expect_identical(tok_xes(kwerenda(), cases = 15, seed = 4), xes)
  expect_match(xes, '<date key="start" value="2027-01-04T08:00:00.000"/>', fixed = TRUE)
  loan <- tok_xes(fixture("loan-desk.tok"))
  expect_match(loan, '<date key="time:timestamp" value="2027-05-03T08:00:00.000+02:00"/>',
               fixed = TRUE)
  expect_match(loan, '<string key="org:resource" value="staff"/>', fixed = TRUE)
})

test_that("tok_xes reports its errors", {
  s <- tok_sim(kwerenda(), cases = 5)
  expect_error(tok_xes(s, cases = 3), "`cases` and `seed` are those of the simulation")
  expect_error(tok_xes(no_start), "tok.sim.SimError: the process 'process' has no start event",
               fixed = TRUE)
  expect_error(tok_xes(kwerenda(), seed = "7"), "`seed` must be a single number", fixed = TRUE)
  expect_error(tok_xes(kwerenda(), lang = "de"), "`lang`")
})
