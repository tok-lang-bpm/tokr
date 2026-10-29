# Writes src/Makevars (src/Makevars.win on Windows) from its template.
#
# A source archive made by r/build.sh contains every Rust crate the package needs
# (src/rust/vendor.tar.xz) and builds without network access; a checkout of the repository
# builds against the engine crates next to it and downloads the external crates once.

desc <- read.dcf("DESCRIPTION")
rustc_min <- sub(".*rustc >= ([0-9.]+).*", "\\1", desc[, "SystemRequirements"])

home <- Sys.getenv(if (.Platform$OS.type == "windows") "USERPROFILE" else "HOME")
Sys.setenv(PATH = paste(Sys.getenv("PATH"), file.path(home, ".cargo", "bin"), sep = .Platform$path.sep))

version_of <- function(tool) {
  out <- tryCatch(system2(tool, "--version", stdout = TRUE, stderr = TRUE), error = function(e) NULL)
  if (is.null(out) || !length(out)) return(NA_character_)
  sub("^[^ ]+ ([0-9]+\\.[0-9]+(\\.[0-9]+)?).*", "\\1", out[[1]])
}

rustc <- version_of("rustc")
if (is.na(version_of("cargo")) || is.na(rustc)) {
  stop(
    "The Rust compiler (rustc) and cargo were not found.\n",
    "Install Rust ", rustc_min, " or newer from https://www.rust-lang.org/tools/install ",
    "(on Windows also run: rustup target add x86_64-pc-windows-gnu).",
    call. = FALSE
  )
}
if (utils::compareVersion(rustc, rustc_min) < 0) {
  stop("Rust ", rustc_min, " or newer is required; found ", rustc, ".", call. = FALSE)
}
message("Using rustc ", rustc)

offline <- file.exists("src/rust/vendor.tar.xz")
debug <- nzchar(Sys.getenv("DEBUG"))

# the archive is unpacked by R itself (its own tar and xz), so the xz program is not needed
offline_setup <- if (offline) {
  paste(
    "\"$(R_HOME)/bin$(R_ARCH_BIN)/Rscript\" -e 'untar(\"rust/vendor.tar.xz\", tar = \"internal\")' &&",
    "mkdir -p $(CARGOTMP) &&",
    "cp rust/vendor-config.toml $(CARGOTMP)/config.toml"
  )
} else {
  ""
}

windows <- .Platform$OS.type == "windows"
template <- if (windows) "src/Makevars.win.in" else "src/Makevars.in"
target <- if (windows) "src/Makevars.win" else "src/Makevars"

text <- readLines(template)
text <- gsub("@OFFLINE_SETUP@", offline_setup, text, fixed = TRUE)
text <- gsub("@CARGO_FLAGS@", if (offline) "--offline -j 2" else "", text, fixed = TRUE)
text <- gsub("@CLEAN_TARGET@", if (debug) "" else "$(TARGET_DIR) vendor", text, fixed = TRUE)
con <- file(target, open = "wb")
writeLines(text, con, sep = "\n")
close(con)
