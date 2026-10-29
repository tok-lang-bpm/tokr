//! Discrete-event simulator of the tok process language.
//!
//! This crate ports `sim.py` of the Python implementation. Every random draw of
//! `sim.py` comes from one `random.Random(seed)`; [`PyRandom`] reproduces that
//! generator exactly, the simulator draws in the same order, and the results are
//! formatted as Python formats them, so the same model and seed give the same event
//! log, indicators and reports (on the same platform: see `DEPENDENCIES.md`).
//! [`xes`] writes the event log as XES (IEEE 1849-2016), which `sim.py` does not, and
//! [`describe`] gives the simulation model as data for simulators outside this crate.
//!
//! Errors are [`PyError`]s: the exceptions `sim.py` lets through (for example
//! `tok.sim.SimError` for a process without a start event), as the last line of a
//! Python traceback prints them.

pub mod describe;
pub mod fallback;
pub mod pyrandom;
pub mod pyval;
pub mod result;
pub mod sim;
pub mod xes;

pub use pyrandom::{PyRandom, SeedError};
pub use result::{Kpi, SimResult};
pub use sim::{Param, Simulator};
pub use tok_core::pyutil::PyError;

use tok_core::Document;

/// Stack of the thread that runs a simulation. Tokens move through gateways and events
/// that take no time by nested calls, as in `sim.py`, so a long chain of such elements
/// (up to `MAX_STEPS` per case) needs a deep stack.
const STACK_SIZE: usize = 256 * 1024 * 1024;

/// Run the simulation of a checked document (`sim.simulate`). `cases` and `seed`
/// override the attributes of `simulate`.
pub fn simulate(
    doc: &Document,
    cases: Option<Param>,
    seed: Option<Param>,
) -> Result<SimResult, PyError> {
    let job = move || -> Result<SimResult, PyError> {
        // the expressions of a 0.7 document are FEEL of conformance level 3
        let _feel = fallback::Feel07::set(doc.lang_07());
        let outcome = Simulator::new(doc, cases, seed)?.run()?;
        Ok(SimResult { outcome })
    };
    if cfg!(target_family = "wasm") {
        // no threads: the current stack
        return job();
    }
    std::thread::scope(|s| {
        let spawned = std::thread::Builder::new()
            .name("tok-sim".into())
            .stack_size(STACK_SIZE)
            .spawn_scoped(s, job);
        match spawned {
            Ok(handle) => match handle.join() {
                Ok(r) => r,
                Err(panic) => std::panic::resume_unwind(panic),
            },
            Err(e) => Err(PyError::new(
                "RuntimeError",
                format!("cannot start the simulation thread: {e}"),
            )),
        }
    })
}
