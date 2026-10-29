//! Drawing of conversations, choreographies and participant multiplicity (BPMN 2.0.2
//! §9.4, §11.5, §12.2): hexagons for conversation nodes, double lines for conversation
//! links, choreography activities with their participant bands and message envelopes, and
//! the multi-instance marker of a pool.

use std::collections::HashMap;

use crate::layout::Pt;
use crate::model::{Model, m};
use crate::pyfmt::f1;
use crate::svg::{STROKE, draw_markers, esc, text, wrap};
use crate::xml::Id;

/// Fill of the bands of participants that do not initiate an activity.
const SHADE: &str = "#e6e8ec";

/// A participant band of a choreography activity.
#[derive(Debug, Clone)]
pub struct Band {
    pub participant: String,
    pub bounds: [f64; 4],
    pub kind: String,
    pub visible: bool,
}

/// Bands by the identifier of the shape of their activity.
pub type Bands = HashMap<String, Vec<Band>>;

fn pts_str(pts: &[Pt]) -> String {
    pts.iter()
        .map(|(x, y)| format!("{},{}", f1(*x), f1(*y)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A conversation link: two parallel lines.
pub fn link(pts: &[Pt]) -> String {
    let p = pts_str(pts);
    format!(
        "<polyline points=\"{p}\" fill=\"none\" stroke=\"{STROKE}\" stroke-width=\"4.4\"/>\
         <polyline points=\"{p}\" fill=\"none\" stroke=\"#fff\" stroke-width=\"2\"/>"
    )
}

/// A conversation node: a hexagon (thick for a call conversation, with a `+` for a
/// sub-conversation) and its name below.
pub fn conversation(md: &Model, e: Id, t: &str, b: [f64; 4], labels: &mut Vec<String>) -> String {
    let [x, y, w, h] = b;
    let cy = y + h / 2.0;
    let pts = [
        (x, cy),
        (x + w * 0.25, y),
        (x + w * 0.75, y),
        (x + w, cy),
        (x + w * 0.75, y + h),
        (x + w * 0.25, y + h),
    ];
    let sw = if t == "callConversation" { "4" } else { "1.5" };
    let mut s = format!(
        "<polygon points=\"{}\" fill=\"#fff\" stroke=\"{STROKE}\" stroke-width=\"{sw}\"/>",
        pts_str(&pts)
    );
    if t == "subConversation" {
        s.push_str(&draw_markers(&["plus"], x, y + 3.0, w, h));
    }
    let nm = md.name(Some(e));
    if !nm.is_empty() {
        let lines = wrap(&nm, 110.0, 11.0);
        #[allow(clippy::cast_precision_loss)]
        let n = lines.len() as f64;
        labels.push(text(
            &lines,
            x + w + 6.0,
            cy - (n - 1.0) * 6.6 + 4.0,
            11,
            "start",
            "",
        ));
    }
    s
}

/// The three bars of a multi-instance participant, centred above `bottom`.
pub fn multiplicity_marker(cx: f64, bottom: f64) -> String {
    let mut s = String::new();
    for dd in [-4.0, 0.0, 4.0] {
        s.push_str(&format!(
            "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{STROKE}\" stroke-width=\"2\"/>",
            f1(cx + dd),
            f1(bottom - 14.0),
            f1(cx + dd),
            f1(bottom - 3.0)
        ));
    }
    s
}

/// Whether a participant (of a collaboration or a choreography) may stand for several:
/// its multiplicity has a maximum greater than 1 (the default maximum is 1).
pub fn multiple(md: &Model, e: Id) -> bool {
    md.doc
        .find(e, &m("participantMultiplicity"))
        .and_then(|pm| md.attr(pm, "maximum"))
        .and_then(|v| v.trim().parse::<i64>().ok())
        .is_some_and(|v| v > 1)
}

/// Name of the message of the message flow that `participant` sends in the activity.
fn message_name(md: &Model, act: Id, participant: &str) -> String {
    for r in md.doc.findall(act, &m("messageFlowRef")) {
        let fid = md.doc.text(r).unwrap_or("").trim().to_string();
        let Some(f) = md.get(Some(&fid)) else {
            continue;
        };
        if md.attr(f, "sourceRef") != Some(participant) {
            continue;
        }
        let msg = md.get(md.attr(f, "messageRef"));
        return match msg {
            Some(mm) => md.name(Some(mm)),
            None => md.name(Some(f)),
        };
    }
    String::new()
}

/// A band: square corners, or rounded on the side where it meets the outline.
fn band_path(b: [f64; 4], top: bool, bottom: bool, fill: &str) -> String {
    let [x, y, w, h] = b;
    let r = 10.0_f64.min(h).min(w / 2.0);
    let d = if top && !bottom {
        format!(
            "M{},{} v{} a{r},{r} 0 0 1 {r},-{r} h{} a{r},{r} 0 0 1 {r},{r} v{} z",
            f1(x),
            f1(y + h),
            f1(-(h - r)),
            f1(w - 2.0 * r),
            f1(h - r)
        )
    } else if bottom && !top {
        format!(
            "M{},{} v{} a{r},{r} 0 0 0 {r},{r} h{} a{r},{r} 0 0 0 {r},-{r} v{} z",
            f1(x),
            f1(y),
            f1(h - r),
            f1(w - 2.0 * r),
            f1(-(h - r))
        )
    } else {
        format!("M{},{} h{} v{} h{} z", f1(x), f1(y), f1(w), f1(h), f1(-w))
    };
    format!("<path d=\"{d}\" fill=\"{fill}\" stroke=\"{STROKE}\" stroke-width=\"1.2\"/>")
}

/// An envelope (BPMN message marker) centred at (cx, cy).
fn envelope(cx: f64, cy: f64, fill: &str) -> String {
    let (w, h) = (24.0, 16.0);
    let (x, y) = (cx - w / 2.0, cy - h / 2.0);
    format!(
        "<rect x=\"{}\" y=\"{}\" width=\"{w}\" height=\"{h}\" fill=\"{fill}\" stroke=\"{STROKE}\" stroke-width=\"1.2\"/>\
         <path d=\"M{},{} L{},{} L{},{}\" fill=\"none\" stroke=\"{STROKE}\" stroke-width=\"1.2\"/>",
        f1(x),
        f1(y),
        f1(x),
        f1(y),
        f1(cx),
        f1(y + h * 0.55),
        f1(x + w),
        f1(y)
    )
}

/// A choreography activity: outline, participant bands with names, the name, markers and
/// the envelopes of visible messages. `expanded` sub-choreographies show their name at the
/// top (their flow nodes are drawn on top of them).
pub fn choreography_activity(
    md: &Model,
    e: Id,
    t: &str,
    b: [f64; 4],
    bands: &[Band],
    labels: &mut Vec<String>,
) -> String {
    let [x, y, w, h] = b;
    let expanded = t == "subChoreography" && md.has_flow_children(e);
    let sw = if t == "callChoreography" { "4" } else { "1.5" };
    let mut s = format!(
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"10\" fill=\"#fff\" stroke=\"{STROKE}\" stroke-width=\"1.5\"/>",
        f1(x),
        f1(y),
        f1(w),
        f1(h)
    );
    let mut top_h = 0.0_f64;
    let mut bottom_h = 0.0_f64;
    for band in bands {
        let [bx, by, bw, bh] = band.bounds;
        let is_top = band.kind.starts_with("top");
        let is_bottom = band.kind.starts_with("bottom");
        if is_top {
            top_h = top_h.max(by + bh - y);
        } else {
            bottom_h = bottom_h.max(y + h - by);
        }
        let fill = if band.kind.ends_with("_initiating") && !band.kind.contains("non") {
            "#fff"
        } else {
            SHADE
        };
        s.push_str(&band_path(band.bounds, is_top, is_bottom, fill));
        let pe = md.get(Some(&band.participant));
        let pn = pe.map(|p| md.name(Some(p))).unwrap_or_default();
        if !pn.is_empty() {
            let mut lines = wrap(&pn, bw - 8.0, 10.0);
            lines.truncate(1);
            s.push_str(&text(
                &lines,
                bx + bw / 2.0,
                by + bh / 2.0 + 3.5,
                10,
                "middle",
                "",
            ));
        }
        if pe.is_some_and(|p| multiple(md, p)) {
            s.push_str(&multiplicity_marker(bx + bw / 2.0, by + bh + 1.0));
        }
        if band.visible && t == "choreographyTask" {
            let msg = message_name(md, e, &band.participant);
            let (ey, ly) = if is_top {
                (y - 22.0, y)
            } else {
                (y + h + 22.0, y + h)
            };
            s.push_str(&format!(
                "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{STROKE}\" stroke-width=\"1.2\" stroke-dasharray=\"2 3\"/>",
                f1(x + w / 2.0),
                f1(if is_top { ey + 8.0 } else { ey - 8.0 }),
                f1(x + w / 2.0),
                f1(ly)
            ));
            s.push_str(&envelope(x + w / 2.0, ey, fill));
            if !msg.is_empty() {
                labels.push(format!(
                    "<text x=\"{}\" y=\"{}\" font-size=\"10\" text-anchor=\"start\" font-style=\"italic\">{}</text>",
                    f1(x + w / 2.0 + 16.0),
                    f1(ey + 3.5),
                    esc(&msg)
                ));
            }
        }
    }
    // the outline again over the bands, with the border of a call
    s.push_str(&format!(
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"10\" fill=\"none\" stroke=\"{STROKE}\" stroke-width=\"{sw}\"/>",
        f1(x),
        f1(y),
        f1(w),
        f1(h)
    ));
    let mut mk: Vec<&str> = Vec::new();
    match md.attr(e, "loopType") {
        Some("Standard") => mk.push("loop"),
        Some("MultiInstanceParallel") => mk.push("mi_par"),
        Some("MultiInstanceSequential") => mk.push("mi_seq"),
        _ => {}
    }
    if (t == "subChoreography" && !expanded) || t == "callChoreography" {
        mk.push("plus");
    }
    let marker_h = if mk.is_empty() { 0.0 } else { 16.0 };
    let nm = md.name(Some(e));
    if expanded {
        if !nm.is_empty() {
            let mut lines = wrap(&nm, w - 20.0, 11.0);
            lines.truncate(2);
            s.push_str(&text(
                &lines,
                x + 10.0,
                y + top_h + 16.0,
                11,
                "start",
                "font-weight=\"bold\"",
            ));
        }
    } else if !nm.is_empty() {
        let inner = (h - top_h - bottom_h - marker_h).max(12.0);
        let mut lines = wrap(&nm, w - 12.0, 11.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let max_lines = ((inner - 4.0) / 13.2).floor().max(1.0) as usize;
        lines.truncate(max_lines);
        #[allow(clippy::cast_precision_loss)]
        let n = lines.len() as f64;
        let mid = y + top_h + inner / 2.0;
        s.push_str(&text(
            &lines,
            x + w / 2.0,
            mid - n * 13.2 / 2.0 + 9.5,
            11,
            "middle",
            "",
        ));
    }
    if !mk.is_empty() {
        s.push_str(&draw_markers(&mk, x, y, w, h - bottom_h));
    }
    s
}

/// The name of a choreography above its diagram, whose topmost shape starts at (x, y).
pub fn title(name: &str, x: f64, y: f64) -> String {
    text(
        &[name.to_string()],
        x,
        y - 44.0,
        13,
        "start",
        "font-weight=\"bold\"",
    )
}

/// A group: a dashed rounded rectangle with the value of its category at the top.
pub fn group(md: &Model, e: Id, b: [f64; 4], labels: &mut Vec<String>) -> String {
    let [x, y, w, h] = b;
    let s = format!(
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"8\" fill=\"none\" stroke=\"{STROKE}\" \
         stroke-width=\"1.3\" stroke-dasharray=\"8 4 2 4\"/>",
        f1(x),
        f1(y),
        f1(w),
        f1(h)
    );
    let name = md
        .get(md.attr(e, "categoryValueRef"))
        .and_then(|v| md.attr(v, "value"))
        .unwrap_or("")
        .trim()
        .to_string();
    if !name.is_empty() {
        labels.push(text(
            &wrap(&name, (w - 16.0).max(40.0), 11.0),
            x + 8.0,
            y + 16.0,
            11,
            "start",
            "",
        ));
    }
    s
}
