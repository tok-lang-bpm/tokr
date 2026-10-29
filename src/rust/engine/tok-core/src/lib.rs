//! Parser, checker, printer and exporters of the tok process language.
//!
//! This crate mirrors the Python reference implementation (`tok/python/src/tok`)
//! module by module; the conformance corpus in `tok/conformance` is the contract.

pub mod check;
pub mod classes;
pub mod dec;
pub mod diagnostics;
pub mod dmn;
pub mod drg;
pub mod export;
pub mod feel;
pub mod feel3;
pub mod feel_eval;
pub mod forms;
pub mod import;
pub mod json;
pub mod lexer;
pub mod model;
pub mod parser;
pub mod printer;
pub mod pydec;
pub mod pytime;
pub mod pyutil;
pub mod simcheck;
pub mod xml;

pub use check::{BpmnIssue, BpmnRules, check};
pub use diagnostics::{Diag, Severity};
pub use model::Document;
pub use parser::parse;
pub use printer::fmt;

/// Newest language version read by this implementation (documents of 0.1, 0.3, 0.5, 0.6
/// and 0.7 stay valid).
pub const LANGUAGE_VERSION: &str = "1.0";

/// Parse and, when the parser found no errors, check a document (`tok check`).
/// Diagnostics are sorted by line and code like the Python command line.
pub fn load(text: &str, level: &str, bpmn: Option<&dyn BpmnRules>) -> (Document, Vec<Diag>) {
    let (mut doc, mut diags) = parse(text);
    if !diags.iter().any(Diag::is_error) {
        let more = check(&mut doc, level, bpmn);
        diags.extend(more);
    }
    diagnostics::sort(&mut diags);
    (doc, diags)
}
