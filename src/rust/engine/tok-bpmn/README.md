# tok-bpmn

BPMN 2.0 tools of the tok process language: the BPMN rules of the course validator `bpmn.py`
(errors, warnings and style notes with their codes), an automatic diagram layout (left to
right, pools stacked, lanes as rows, routed sequence and message flows, placed labels) that
writes BPMN diagram interchange, a layout sidecar that keeps the drawing of imported files,
and SVG rendering. It works on BPMN 2.0 XML, so it serves any BPMN file, not only those
exported from tok.

```rust
let issues = tok_bpmn::validate(xml)?;
```

`PARITY.md` explains what is identical to the Python reference and what the layout engine
does beyond it. The tok language and its other crates: `tok-core`, `tok-sim`, `tok-cli`.

License: GPL-3.0-or-later.
