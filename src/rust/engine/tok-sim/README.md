# tok-sim

The discrete-event simulator of the tok process language: process instances arrive, tokens
move along sequence flows, activities wait for performers of their resources and hold them
for their durations, gateways choose by conditions or probabilities, timers, messages and
boundary events fire, decisions are evaluated. The result is an event log (CSV or XES, IEEE
1849-2016, for process-mining tools), a table of cases and indicators: cases per end event,
cycle time, waiting and processing time per activity, utilization per resource. With a fixed
seed a run is reproducible.

The semantics and the distributions are described in `docs/simulation.md` of the repository;
the `tok` command (`tok sim`) is in the crate `tok-cli`.

License: GPL-3.0-or-later.
