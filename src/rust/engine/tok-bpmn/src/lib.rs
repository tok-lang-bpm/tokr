//! BPMN 2.0 rules, automatic layout and SVG rendering used by tok.
//!
//! The validator is a faithful port of the course validator `bpmn.py` 1.2 (shipped
//! with the Python implementation as `tok.bpmnlib`): the same diagnostics with
//! identical Polish messages and the same semantic BPMN XML. The layout engine
//! started as a port of `bpmn.py` and, from tok 0.4, refines its result: flows are
//! rerouted around shapes, labels and other flows, labels are placed without
//! overlaps and written as `BPMNLabel` bounds, and no quality measure is ever worse
//! than in the classic layout. A model that brings its own diagram is drawn exactly
//! as by `bpmn.py`. See `PARITY.md` for the verification.

mod choreo;
mod layout;
mod model;
mod pyfmt;
mod refine;
mod sidecar;
mod steps;
mod svg;
mod svg06;
mod validate;
mod xml;

use std::path::Path;

pub use sidecar::{Sidecar, extract as extract_sidecar, layout_with, render_svg_with};
pub use steps::read_opis;
pub use validate::{Issue, Level, ValidateOptions};

/// Errors of the BPMN functions.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The input is not well-formed XML.
    #[error("Plik nie jest poprawnym XML: {0}")]
    Xml(String),
    /// The model has no diagram interchange (shapes) to render.
    #[error("Model nie ma współrzędnych (BPMNDI). Najpierw uruchom: bpmn.py layout")]
    NoDiagram,
    /// A layout sidecar (`FILE.tok.layout`) cannot be read.
    #[error("layout sidecar: {0}")]
    Sidecar(String),
}

fn model(xml: &str) -> Result<model::Model, Error> {
    Ok(model::Model::new(xml::Doc::parse(xml)?))
}

/// Check BPMN 2.0 rules (`bpmn.py check`).
///
/// # Errors
/// [`Error::Xml`] when the input is not well-formed XML.
pub fn validate(xml: &str) -> Result<Vec<Issue>, Error> {
    validate_with(xml, &ValidateOptions::default())
}

/// Check BPMN 2.0 rules with the step-number options (`--kroki`, `--opis`).
///
/// # Errors
/// [`Error::Xml`] when the input is not well-formed XML.
pub fn validate_with(xml: &str, opts: &ValidateOptions) -> Result<Vec<Issue>, Error> {
    Ok(validate::validate_model(&model(xml)?, opts))
}

/// Camunda 8 executable profile (`bpmn.py check --camunda`); `folder` holds the
/// `.form` and `.dmn` files the model refers to.
///
/// # Errors
/// [`Error::Xml`] when the input is not well-formed XML.
pub fn validate_camunda(xml: &str, folder: &Path) -> Result<Vec<Issue>, Error> {
    Ok(validate::validate_camunda_model(&model(xml)?, folder))
}

/// One-line summary of the model (`summarize`).
///
/// # Errors
/// [`Error::Xml`] when the input is not well-formed XML.
pub fn summarize(xml: &str) -> Result<String, Error> {
    Ok(validate::summarize_model(&model(xml)?))
}

/// Automatic layout: returns the document with fresh BPMNDI, serialized like
/// `bpmn.py layout` writes it.
///
/// # Errors
/// [`Error::Xml`] when the input is not well-formed XML.
pub fn layout(xml: &str) -> Result<String, Error> {
    let mut md = model(xml)?;
    layout::layout_model(&mut md);
    Ok(md.doc.save())
}

/// Render the diagram as SVG (`bpmn.py svg`): a model without BPMNDI is laid
/// out first.
///
/// # Errors
/// [`Error::Xml`] for malformed XML, [`Error::NoDiagram`] when nothing can be drawn.
pub fn render_svg(xml: &str, title: Option<&str>) -> Result<String, Error> {
    let mut md = model(xml)?;
    let has_shapes = !md
        .doc
        .iter_tag(md.doc.root, &xml::qn(xml::BPMNDI_NS, "BPMNShape"))
        .is_empty();
    if !has_shapes {
        layout::layout_model(&mut md);
    }
    svg::render_model(&md, title)
}
