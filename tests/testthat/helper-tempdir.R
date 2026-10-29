withr_tempdir <- function(envir = parent.frame()) {
  dir <- tempfile("tok-test")
  dir.create(dir)
  do.call(on.exit, list(substitute(unlink(d, recursive = TRUE), list(d = dir)), add = TRUE), envir = envir)
  dir
}
