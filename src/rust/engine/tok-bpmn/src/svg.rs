//! SVG rendering of a laid-out model (`render_svg()` of `bpmn.py`).

use std::f64::consts::PI;

use crate::Error;
use crate::layout::{OMap, Pt, key};
use crate::model::{Model, is_act, is_choreo_act, is_conv, is_evt, is_gw, is_subp, m};
use crate::pyfmt::{f0, f1};
use crate::svg06;
use crate::xml::{BPMNDI_NS, DC_NS, DI_NS, Id, qn};

pub(crate) const STROKE: &str = "#1f2328";

pub(crate) fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `_wrap(text, maxw, fs)`: greedy line breaking by character count.
#[must_use]
pub fn wrap(text: &str, maxw: f64, fs: f64) -> Vec<String> {
    let cw = fs * 0.56;
    #[allow(clippy::cast_possible_truncation)]
    let maxc = ((maxw / cw).trunc() as i64).max(4);
    let maxc = usize::try_from(maxc).unwrap_or(4);
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for w in text.split_whitespace() {
        let mut w: Vec<char> = w.chars().collect();
        while w.len() > maxc {
            if !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
            }
            let mut piece: String = w[..maxc - 1].iter().collect();
            piece.push('-');
            lines.push(piece);
            w = w[maxc - 1..].to_vec();
        }
        let ws: String = w.iter().collect();
        if cur.is_empty() {
            cur = ws;
        } else if cur.chars().count() + 1 + w.len() <= maxc {
            cur.push(' ');
            cur.push_str(&ws);
        } else {
            lines.push(std::mem::take(&mut cur));
            cur = ws;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

pub(crate) fn text(lines: &[String], x: f64, y: f64, fs: i64, anchor: &str, extra: &str) -> String {
    if lines.is_empty() {
        return String::new();
    }
    #[allow(clippy::cast_precision_loss)]
    let lh = fs as f64 * 1.2;
    let mut out = format!(
        "<text x=\"{}\" y=\"{}\" font-size=\"{}\" text-anchor=\"{}\" {}>",
        f1(x),
        f1(y),
        fs,
        anchor,
        extra
    );
    for (k, ln) in lines.iter().enumerate() {
        let dy = if k == 0 { 0.0 } else { lh };
        out.push_str(&format!(
            "<tspan x=\"{}\" dy=\"{}\">{}</tspan>",
            f1(x),
            f1(dy),
            esc(ln)
        ));
    }
    out.push_str("</text>");
    out
}

fn pts_str(pts: &[Pt]) -> String {
    pts.iter()
        .map(|(x, y)| format!("{},{}", f1(*x), f1(*y)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn pentagon(cx: f64, cy: f64, r: f64) -> String {
    (0..5)
        .map(|k| {
            let a = -PI / 2.0 + f64::from(k) * 2.0 * PI / 5.0;
            format!("{},{}", f1(cx + r * a.cos()), f1(cy + r * a.sin()))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Outline of a plus sign (parallel multiple events, parallel event-based gateways).
fn plus_outline(cx: f64, cy: f64, r: f64, a: f64) -> String {
    [
        (-a, -r),
        (a, -r),
        (a, -a),
        (r, -a),
        (r, a),
        (a, a),
        (a, r),
        (-a, r),
        (-a, a),
        (-r, a),
        (-r, -a),
        (-a, -a),
    ]
    .iter()
    .map(|(dx, dy)| format!("{},{}", f1(cx + dx), f1(cy + dy)))
    .collect::<Vec<_>>()
    .join(" ")
}

/// Marker of an event: `defs` are its event definitions, `filled` for throwing events and
/// `parallel` for a parallel multiple event (all triggers are needed).
#[allow(clippy::too_many_lines)]
fn event_icon(defs: &[String], cx: f64, cy: f64, filled: bool, parallel: bool) -> String {
    let fill = if filled { STROKE } else { "#fff" };
    let ln = if filled { "#fff" } else { STROKE };
    let mut s = String::new();
    if defs.len() == 1 {
        match defs[0].as_str() {
            "messageEventDefinition" => {
                s.push_str(&format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"16\" height=\"11\" fill=\"{}\" stroke=\"{}\" stroke-width=\"1.2\"/>",
                    f1(cx - 8.0),
                    f1(cy - 5.5),
                    fill,
                    STROKE
                ));
                s.push_str(&format!(
                    "<polyline points=\"{},{} {},{} {},{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"1.2\"/>",
                    f1(cx - 8.0),
                    f1(cy - 5.5),
                    f1(cx),
                    f1(cy + 1.0),
                    f1(cx + 8.0),
                    f1(cy - 5.5),
                    ln
                ));
            }
            "timerEventDefinition" => {
                s.push_str(&format!(
                    "<circle cx=\"{}\" cy=\"{}\" r=\"11\" fill=\"#fff\" stroke=\"{}\" stroke-width=\"1.5\"/>",
                    f1(cx),
                    f1(cy),
                    STROKE
                ));
                for k in 0..12 {
                    let a = f64::from(k) * PI / 6.0;
                    let r1 = if k % 3 != 0 { 8.5 } else { 7.5 };
                    s.push_str(&format!(
                        "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"1\"/>",
                        f1(cx + r1 * a.cos()),
                        f1(cy + r1 * a.sin()),
                        f1(cx + 10.5 * a.cos()),
                        f1(cy + 10.5 * a.sin()),
                        STROKE
                    ));
                }
                s.push_str(&format!(
                    "<polyline points=\"{},{} {},{} {},{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"1.5\"/>",
                    f1(cx),
                    f1(cy - 7.0),
                    f1(cx),
                    f1(cy),
                    f1(cx + 5.0),
                    f1(cy + 2.0),
                    STROKE
                ));
            }
            "signalEventDefinition" => s.push_str(&format!(
                "<polygon points=\"{},{} {},{} {},{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"1.3\"/>",
                f1(cx),
                f1(cy - 9.0),
                f1(cx + 8.5),
                f1(cy + 6.0),
                f1(cx - 8.5),
                f1(cy + 6.0),
                fill,
                STROKE
            )),
            "errorEventDefinition" => s.push_str(&format!(
                "<polygon points=\"{},{} {},{} {},{} {},{} {},{} {},{}\" fill=\"{}\" stroke=\"{}\" \
                 stroke-width=\"1.3\" stroke-linejoin=\"round\"/>",
                f1(cx - 8.0),
                f1(cy + 8.0),
                f1(cx - 3.0),
                f1(cy - 6.0),
                f1(cx + 2.0),
                f1(cy + 2.0),
                f1(cx + 8.0),
                f1(cy - 8.0),
                f1(cx + 3.0),
                f1(cy + 6.0),
                f1(cx - 2.0),
                f1(cy - 2.0),
                fill,
                STROKE
            )),
            "escalationEventDefinition" => s.push_str(&format!(
                "<polygon points=\"{},{} {},{} {},{} {},{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"1.3\" \
                 stroke-linejoin=\"round\"/>",
                f1(cx),
                f1(cy - 9.0),
                f1(cx + 6.5),
                f1(cy + 8.0),
                f1(cx),
                f1(cy + 2.0),
                f1(cx - 6.5),
                f1(cy + 8.0),
                fill,
                STROKE
            )),
            "conditionalEventDefinition" => {
                s.push_str(&format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"13\" height=\"16\" fill=\"#fff\" stroke=\"{}\" stroke-width=\"1.2\"/>",
                    f1(cx - 6.5),
                    f1(cy - 8.0),
                    STROKE
                ));
                for k in 0..4 {
                    let yy = cy - 5.0 + 3.5 * f64::from(k);
                    s.push_str(&format!(
                        "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"1\"/>",
                        f1(cx - 4.0),
                        f1(yy),
                        f1(cx + 4.0),
                        f1(yy),
                        STROKE
                    ));
                }
            }
            "linkEventDefinition" => s.push_str(&format!(
                "<polygon points=\"{},{} {},{} {},{} {},{} {},{} {},{} {},{}\" fill=\"{}\" stroke=\"{}\" \
                 stroke-width=\"1.2\"/>",
                f1(cx - 8.0),
                f1(cy - 3.0),
                f1(cx + 1.0),
                f1(cy - 3.0),
                f1(cx + 1.0),
                f1(cy - 7.0),
                f1(cx + 8.0),
                f1(cy),
                f1(cx + 1.0),
                f1(cy + 7.0),
                f1(cx + 1.0),
                f1(cy + 3.0),
                f1(cx - 8.0),
                f1(cy + 3.0),
                fill,
                STROKE
            )),
            "terminateEventDefinition" => s.push_str(&format!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"10\" fill=\"{}\"/>",
                f1(cx),
                f1(cy),
                STROKE
            )),
            "compensateEventDefinition" => {
                s.push_str(&format!(
                    "<polygon points=\"{},{} {},{} {},{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"1.2\"/>",
                    f1(cx - 9.0),
                    f1(cy),
                    f1(cx - 1.0),
                    f1(cy - 6.0),
                    f1(cx - 1.0),
                    f1(cy + 6.0),
                    fill,
                    STROKE
                ));
                s.push_str(&format!(
                    "<polygon points=\"{},{} {},{} {},{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"1.2\"/>",
                    f1(cx - 1.0),
                    f1(cy),
                    f1(cx + 7.0),
                    f1(cy - 6.0),
                    f1(cx + 7.0),
                    f1(cy + 6.0),
                    fill,
                    STROKE
                ));
            }
            "cancelEventDefinition" => s.push_str(&format!(
                "<path d=\"M{},{} L{},{} M{},{} L{},{}\" stroke=\"{}\" stroke-width=\"4\"/>",
                f1(cx - 7.0),
                f1(cy - 7.0),
                f1(cx + 7.0),
                f1(cy + 7.0),
                f1(cx + 7.0),
                f1(cy - 7.0),
                f1(cx - 7.0),
                f1(cy + 7.0),
                STROKE
            )),
            _ => {}
        }
    }
    if defs.len() > 1 && parallel {
        s.push_str(&format!(
            "<polygon points=\"{}\" fill=\"#fff\" stroke=\"{}\" stroke-width=\"1.2\"/>",
            plus_outline(cx, cy, 9.0, 3.0),
            STROKE
        ));
    } else if defs.len() > 1 {
        s.push_str(&format!(
            "<polygon points=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"1.2\"/>",
            pentagon(cx, cy, 9.0),
            fill,
            STROKE
        ));
    }
    s
}

fn task_icon(t: &str, x: f64, y: f64) -> String {
    let mut s = String::new();
    match t {
        "userTask" => {
            s.push_str(&format!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"4\" fill=\"#fff\" stroke=\"{}\" stroke-width=\"1.2\"/>",
                f1(x + 14.0),
                f1(y + 12.0),
                STROKE
            ));
            s.push_str(&format!(
                "<path d=\"M{},{} Q{},{} {},{} Q{},{} {},{} Z\" fill=\"#fff\" stroke=\"{}\" stroke-width=\"1.2\"/>",
                f1(x + 6.0),
                f1(y + 26.0),
                f1(x + 6.0),
                f1(y + 17.0),
                f1(x + 14.0),
                f1(y + 17.0),
                f1(x + 22.0),
                f1(y + 17.0),
                f1(x + 22.0),
                f1(y + 26.0),
                STROKE
            ));
        }
        "serviceTask" => {
            s.push_str(&format!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"6.5\" fill=\"none\" stroke=\"{}\" stroke-width=\"3.4\" \
                 stroke-dasharray=\"2.6 2.5\"/>",
                f1(x + 14.0),
                f1(y + 15.0),
                STROKE
            ));
            s.push_str(&format!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"4.6\" fill=\"#fff\" stroke=\"{}\" stroke-width=\"1.2\"/>",
                f1(x + 14.0),
                f1(y + 15.0),
                STROKE
            ));
            s.push_str(&format!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"1.8\" fill=\"none\" stroke=\"{}\" stroke-width=\"1\"/>",
                f1(x + 14.0),
                f1(y + 15.0),
                STROKE
            ));
        }
        "manualTask" => s.push_str(&format!(
            "<path d=\"M{},{} h12 a3,3 0 0 1 0,6 h-1 M{},{} h-12 v-8 l5,-5 h2 v5 h8 a2,2 0 0 1 0,4 h-3 M{},{} h3 \
             a2,2 0 0 1 0,4 h-2\" fill=\"#fff\" stroke=\"{}\" stroke-width=\"1.2\" stroke-linejoin=\"round\"/>",
            f1(x + 9.0),
            f1(y + 17.0),
            f1(x + 18.0),
            f1(y + 23.0),
            f1(x + 16.0),
            f1(y + 17.0),
            STROKE
        )),
        "scriptTask" => {
            s.push_str(&format!(
                "<path d=\"M{},{} h11 c-4,4 4,9 0,13 h-11 c4,-4 -4,-9 0,-13 z\" fill=\"#fff\" stroke=\"{}\" \
                 stroke-width=\"1.2\"/>",
                f1(x + 9.0),
                f1(y + 7.0),
                STROKE
            ));
            for k in 0..3 {
                let yy = y + 10.5 + 3.0 * f64::from(k);
                s.push_str(&format!(
                    "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"1\"/>",
                    f1(x + 9.5),
                    f1(yy),
                    f1(x + 16.5),
                    f1(yy),
                    STROKE
                ));
            }
        }
        "businessRuleTask" => {
            s.push_str(&format!(
                "<rect x=\"{}\" y=\"{}\" width=\"18\" height=\"13\" fill=\"#fff\" stroke=\"{}\" stroke-width=\"1.2\"/>",
                f1(x + 6.0),
                f1(y + 8.0),
                STROKE
            ));
            s.push_str(&format!(
                "<rect x=\"{}\" y=\"{}\" width=\"18\" height=\"4\" fill=\"{}\"/>",
                f1(x + 6.0),
                f1(y + 8.0),
                "#9aa1a9"
            ));
            s.push_str(&format!(
                "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"1\"/>",
                f1(x + 6.0),
                f1(y + 16.5),
                f1(x + 24.0),
                f1(y + 16.5),
                STROKE
            ));
            s.push_str(&format!(
                "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"1\"/>",
                f1(x + 11.0),
                f1(y + 12.0),
                f1(x + 11.0),
                f1(y + 21.0),
                STROKE
            ));
        }
        "sendTask" | "receiveTask" => {
            let filled = t == "sendTask";
            let fill = if filled { STROKE } else { "#fff" };
            let ln = if filled { "#fff" } else { STROKE };
            s.push_str(&format!(
                "<rect x=\"{}\" y=\"{}\" width=\"17\" height=\"12\" fill=\"{}\" stroke=\"{}\" stroke-width=\"1.2\"/>",
                f1(x + 6.0),
                f1(y + 8.0),
                fill,
                STROKE
            ));
            s.push_str(&format!(
                "<polyline points=\"{},{} {},{} {},{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"1.2\"/>",
                f1(x + 6.0),
                f1(y + 8.0),
                f1(x + 14.5),
                f1(y + 15.0),
                f1(x + 23.0),
                f1(y + 8.0),
                ln
            ));
        }
        _ => {}
    }
    s
}

/// Marker of an instantiating receive task: the envelope in a circle, like the marker of a
/// message start event (BPMN 2.0.2, 10.3.3).
fn instantiating_receive_icon(x: f64, y: f64) -> String {
    let (cx, cy) = (x + 15.0, y + 15.0);
    let mut s = format!(
        "<circle cx=\"{}\" cy=\"{}\" r=\"10\" fill=\"#fff\" stroke=\"{}\" stroke-width=\"1.2\"/>",
        f1(cx),
        f1(cy),
        STROKE
    );
    s.push_str(&format!(
        "<rect x=\"{}\" y=\"{}\" width=\"13\" height=\"9\" fill=\"#fff\" stroke=\"{}\" stroke-width=\"1.1\"/>",
        f1(cx - 6.5),
        f1(cy - 4.5),
        STROKE
    ));
    s.push_str(&format!(
        "<polyline points=\"{},{} {},{} {},{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"1.1\"/>",
        f1(cx - 6.5),
        f1(cy - 4.5),
        f1(cx),
        f1(cy + 0.5),
        f1(cx + 6.5),
        f1(cy - 4.5),
        STROKE
    ));
    s
}

/// The task type of the global task a call activity calls (BPMN 2.0.2, 10.3.6: such a call
/// activity has a thick border and the marker of the task type, not the plus sign).
fn called_global_task(md: &Model, e: Id) -> Option<&'static str> {
    if md.tag(e) != "callActivity" {
        return None;
    }
    let called = md.get(md.attr(e, "calledElement"))?;
    Some(match md.tag(called).as_str() {
        "globalTask" => "task",
        "globalUserTask" => "userTask",
        "globalManualTask" => "manualTask",
        "globalScriptTask" => "scriptTask",
        "globalBusinessRuleTask" => "businessRuleTask",
        _ => return None,
    })
}

#[allow(clippy::too_many_lines)]
fn markers(md: &Model, e: Id, x: f64, y: f64, w: f64, h: f64) -> String {
    let t = md.tag(e);
    let d = &md.doc;
    let mut mk: Vec<&str> = Vec::new();
    if d.find(e, &m("standardLoopCharacteristics")).is_some() {
        mk.push("loop");
    }
    if let Some(mi) = d.find(e, &m("multiInstanceLoopCharacteristics")) {
        mk.push(if md.attr(mi, "isSequential") == Some("true") {
            "mi_seq"
        } else {
            "mi_par"
        });
    }
    if is_subp(&t) && !md.has_flow_children(e) {
        mk.push("plus");
    }
    if t == "callActivity"
        && md.attr(e, "calledElement").is_some_and(|c| !c.is_empty())
        && called_global_task(md, e).is_none()
    {
        mk.push("plus");
    }
    if t == "adHocSubProcess" {
        mk.push("adhoc");
    }
    if md.attr(e, "isForCompensation") == Some("true") {
        mk.push("comp");
    }
    draw_markers(&mk, x, y, w, h)
}

/// Activity markers centred at the bottom of the box (x, y, w, h).
pub(crate) fn draw_markers(mk: &[&str], x: f64, y: f64, w: f64, h: f64) -> String {
    if mk.is_empty() {
        return String::new();
    }
    let mut s = String::new();
    #[allow(clippy::cast_precision_loss)]
    let total = (mk.len() * 16) as f64;
    let bx = x + w / 2.0 - total / 2.0;
    let by = y + h - 18.0;
    for (k, m_) in mk.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let cx = bx + 16.0 * k as f64 + 8.0;
        let cy = by + 7.0;
        match *m_ {
            "loop" => {
                s.push_str(&format!(
                    "<path d=\"M{},{} A5.5,5.5 0 1 1 {},{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"1.3\"/>",
                    f1(cx - 4.0),
                    f1(cy + 4.0),
                    f1(cx + 4.0),
                    f1(cy + 4.0),
                    STROKE
                ));
                s.push_str(&format!(
                    "<path d=\"M{},{} l-1,4 l4,-1\" fill=\"none\" stroke=\"{}\" stroke-width=\"1.3\"/>",
                    f1(cx - 4.0),
                    f1(cy + 0.0),
                    STROKE
                ));
            }
            "mi_par" => {
                for dd in [-4.0, 0.0, 4.0] {
                    s.push_str(&format!(
                        "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"2\"/>",
                        f1(cx + dd),
                        f1(cy - 6.0),
                        f1(cx + dd),
                        f1(cy + 6.0),
                        STROKE
                    ));
                }
            }
            "mi_seq" => {
                for dd in [-4.0, 0.0, 4.0] {
                    s.push_str(&format!(
                        "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"2\"/>",
                        f1(cx - 6.0),
                        f1(cy + dd),
                        f1(cx + 6.0),
                        f1(cy + dd),
                        STROKE
                    ));
                }
            }
            "plus" => {
                s.push_str(&format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"12\" height=\"12\" fill=\"#fff\" stroke=\"{}\" stroke-width=\"1.2\"/>",
                    f1(cx - 6.0),
                    f1(cy - 6.0),
                    STROKE
                ));
                s.push_str(&format!(
                    "<path d=\"M{},{} h8 M{},{} v8\" stroke=\"{}\" stroke-width=\"1.4\"/>",
                    f1(cx - 4.0),
                    f1(cy),
                    f1(cx),
                    f1(cy - 4.0),
                    STROKE
                ));
            }
            "adhoc" => s.push_str(&format!(
                "<text x=\"{}\" y=\"{}\" font-size=\"16\" text-anchor=\"middle\">~</text>",
                f1(cx),
                f1(cy + 5.0)
            )),
            _ => {
                s.push_str(&format!(
                    "<polygon points=\"{},{} {},{} {},{}\" fill=\"none\" stroke=\"{}\"/>",
                    f1(cx - 7.0),
                    f1(cy),
                    f1(cx),
                    f1(cy - 5.0),
                    f1(cx),
                    f1(cy + 5.0),
                    STROKE
                ));
                s.push_str(&format!(
                    "<polygon points=\"{},{} {},{} {},{}\" fill=\"none\" stroke=\"{}\"/>",
                    f1(cx),
                    f1(cy),
                    f1(cx + 7.0),
                    f1(cy - 5.0),
                    f1(cx + 7.0),
                    f1(cy + 5.0),
                    STROKE
                ));
            }
        }
    }
    s
}

fn seg_len(a: Pt, b: Pt) -> f64 {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    (dx * dx + dy * dy).powf(0.5)
}

/// Anchor (start of the first baseline) of the label of a sequence flow drawn without label
/// bounds: next to the first long segment.
pub(crate) fn flow_label_anchor(pts: &[Pt]) -> Pt {
    let ((x1, y1), (x2, y2)) = (pts[0], pts[1]);
    let first_vertical = (x2 - x1).abs() < (y2 - y1).abs();
    if first_vertical && pts.len() >= 3 && (y2 - y1).abs() > 40.0 {
        return (x2 + 8.0, y2 - 7.0);
    }
    if !first_vertical
        && pts.len() >= 3
        && (pts[2].0 - x2).abs() < 0.6
        && (pts[2].1 - y2).abs() > 30.0
    {
        if pts[2].1 > y2 {
            return (x2 + 6.0, y2 + 18.0);
        }
        return (x2 + 6.0, y2 - 8.0);
    }
    let mut k = 0;
    while k + 2 < pts.len() && seg_len(pts[k], pts[k + 1]) < 30.0 {
        k += 1;
    }
    let ((x1, y1), (x2, y2)) = (pts[k], pts[k + 1]);
    if (x2 - x1).abs() >= (y2 - y1).abs() {
        return (x1 + 8.0, y1 - 7.0);
    }
    if y2 > y1 {
        return (x1 + 6.0, y1 + 16.0);
    }
    (x1 + 6.0, y1 - 8.0)
}

/// Anchor (start of the first baseline) of the label of a message flow drawn without label
/// bounds: next to the middle segment.
pub(crate) fn msg_label_anchor(pts: &[Pt]) -> Pt {
    let k = pts.len() / 2;
    let ((x1, y1), (x2, y2)) = (pts[k - 1], pts[k]);
    ((x1 + x2) / 2.0 + 6.0, (y1 + y2) / 2.0 - 4.0)
}

fn flow_label(nm: &str, pts: &[Pt]) -> String {
    let (x, y) = flow_label_anchor(pts);
    text(&wrap(nm, 90.0, 10.0), x, y, 10, "start", "")
}

fn parse_f(v: Option<&str>) -> f64 {
    v.and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(0.0)
}

/// An external label drawn in its BPMNDI bounds: wrapped to the width of the bounds,
/// centred, first baseline 8 below the top.
fn bounded_label(nm: &str, b: [f64; 4], extra: &str) -> String {
    text(
        &wrap(nm, b[2] + 1.0, 10.0),
        b[0] + b[2] / 2.0,
        b[1] + 8.0,
        10,
        "middle",
        extra,
    )
}

/// Bounds of the `BPMNLabel` of a shape or an edge.
fn label_bounds(md: &Model, di: Id) -> Option<[f64; 4]> {
    let l = md.doc.find(di, &qn(BPMNDI_NS, "BPMNLabel"))?;
    let b = md.doc.find(l, &qn(DC_NS, "Bounds"))?;
    Some([
        parse_f(md.attr(b, "x")),
        parse_f(md.attr(b, "y")),
        parse_f(md.attr(b, "width")),
        parse_f(md.attr(b, "height")),
    ])
}

/// Render a model that already has BPMNDI as SVG.
///
/// # Errors
/// [`Error::NoDiagram`] when the model has no shapes.
#[allow(clippy::too_many_lines)]
pub fn render_model(md: &Model, title: Option<&str>) -> Result<String, Error> {
    let d = &md.doc;
    let r = d.root;
    let mut shapes: OMap<[f64; 4]> = OMap::default();
    let mut lbounds: OMap<[f64; 4]> = OMap::default();
    // participant bands of choreography activities (language 0.6), by activity shape
    let mut bands: svg06::Bands = std::collections::HashMap::new();
    let mut shape_ids: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for s in d.iter_tag(r, &qn(BPMNDI_NS, "BPMNShape")) {
        let Some(b) = d.find(s, &qn(DC_NS, "Bounds")) else {
            continue;
        };
        if let Some(act) = md.attr(s, "choreographyActivityShape") {
            bands.entry(act.to_string()).or_default().push(svg06::Band {
                participant: key(md.attr(s, "bpmnElement")),
                bounds: [
                    parse_f(md.attr(b, "x")),
                    parse_f(md.attr(b, "y")),
                    parse_f(md.attr(b, "width")),
                    parse_f(md.attr(b, "height")),
                ],
                kind: md.attr(s, "participantBandKind").unwrap_or("").to_string(),
                visible: md.attr(s, "isMessageVisible") == Some("true"),
            });
            continue;
        }
        if let Some(di) = md.attr(s, "id") {
            shape_ids.insert(key(md.attr(s, "bpmnElement")), di.to_string());
        }
        if let Some(lb) = label_bounds(md, s) {
            lbounds.insert(&key(md.attr(s, "bpmnElement")), lb);
        }
        shapes.insert(
            &key(md.attr(s, "bpmnElement")),
            [
                parse_f(md.attr(b, "x")),
                parse_f(md.attr(b, "y")),
                parse_f(md.attr(b, "width")),
                parse_f(md.attr(b, "height")),
            ],
        );
    }
    let mut edges: OMap<Vec<Pt>> = OMap::default();
    for e in d.iter_tag(r, &qn(BPMNDI_NS, "BPMNEdge")) {
        let pts: Vec<Pt> = d
            .findall(e, &qn(DI_NS, "waypoint"))
            .iter()
            .map(|p| (parse_f(md.attr(*p, "x")), parse_f(md.attr(*p, "y"))))
            .collect();
        if let Some(lb) = label_bounds(md, e) {
            lbounds.insert(&key(md.attr(e, "bpmnElement")), lb);
        }
        edges.insert(&key(md.attr(e, "bpmnElement")), pts);
    }
    if shapes.keys.is_empty() {
        return Err(Error::NoDiagram);
    }
    let mut xs: Vec<f64> = Vec::new();
    let mut ys: Vec<f64> = Vec::new();
    for (_, [x, y, w, h]) in shapes.items() {
        xs.extend([x, x + w, x + w / 2.0 - 62.0, x + w / 2.0 + 62.0]);
        ys.extend([y, y + h + 36.0]);
    }
    for (_, pts) in edges.items() {
        for (x, y) in pts {
            xs.push(x);
            ys.push(y);
        }
    }
    for (_, [x, y, w, h]) in lbounds.items() {
        xs.extend([x, x + w]);
        ys.extend([y, y + h]);
    }
    let fmin = |v: &[f64]| v.iter().copied().fold(f64::INFINITY, f64::min);
    let fmax = |v: &[f64]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let (minx, miny) = (fmin(&xs) - 20.0, fmin(&ys) - 20.0);
    let (maxx, maxy) = (fmax(&xs) + 60.0, fmax(&ys) + 20.0);
    let (w_all, h_all) = (maxx - minx, maxy - miny);
    let mut p: Vec<String> = Vec::new();
    p.push(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{} {} {} {}\" width=\"{}\" height=\"{}\" \
         font-family=\"Arial, Helvetica, sans-serif\">",
        f0(minx),
        f0(miny),
        f0(w_all),
        f0(h_all),
        f0(w_all),
        f0(h_all)
    ));
    if let Some(t) = title.filter(|t| !t.is_empty()) {
        p.push(format!("<title>{}</title>", esc(t)));
    }
    p.push(format!(
        "<defs><marker id=\"seqEnd\" viewBox=\"0 0 10 10\" refX=\"10\" refY=\"5\" markerUnits=\"userSpaceOnUse\" \
         markerWidth=\"11\" markerHeight=\"11\" orient=\"auto\"><path d=\"M0,0 L10,5 L0,10 z\" fill=\"{STROKE}\"/></marker>\
         <marker id=\"msgEnd\" viewBox=\"0 0 10 10\" refX=\"10\" refY=\"5\" markerUnits=\"userSpaceOnUse\" \
         markerWidth=\"11\" markerHeight=\"11\" orient=\"auto\"><path d=\"M0.5,0.5 L9.5,5 L0.5,9.5 z\" fill=\"#fff\" \
         stroke=\"{STROKE}\"/></marker><marker id=\"msgStart\" viewBox=\"0 0 10 10\" refX=\"5\" refY=\"5\" \
         markerUnits=\"userSpaceOnUse\" markerWidth=\"9\" markerHeight=\"9\"><circle cx=\"5\" cy=\"5\" r=\"4\" \
         fill=\"#fff\" stroke=\"{STROKE}\"/></marker><marker id=\"assocEnd\" viewBox=\"0 0 10 10\" refX=\"10\" \
         refY=\"5\" markerUnits=\"userSpaceOnUse\" markerWidth=\"10\" markerHeight=\"10\" orient=\"auto\"><path \
         d=\"M0,0 L10,5 L0,10\" fill=\"none\" stroke=\"{STROKE}\"/></marker></defs>"
    ));
    p.push(format!(
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#fff\"/>",
        f0(minx),
        f0(miny),
        f0(w_all),
        f0(h_all)
    ));
    let mut labels: Vec<String> = Vec::new();
    let el = |i: &str| md.get(Some(i));

    // pools and lanes
    for (i, [x, y, w, h]) in shapes.items() {
        let Some(e) = el(&i) else {
            continue;
        };
        let t = md.tag(e);
        if t != "participant" && t != "lane" {
            continue;
        }
        if t == "participant" && md.attr(e, "processRef").is_none_or(str::is_empty) {
            p.push(format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#f2f4f7\" stroke=\"{}\" stroke-width=\"1.3\"/>",
                f1(x),
                f1(y),
                f1(w),
                f1(h),
                STROKE
            ));
            p.push(text(
                &[md.name(Some(e))],
                x + w / 2.0,
                y + h / 2.0 + 5.0,
                13,
                "middle",
                "font-weight=\"bold\"",
            ));
            if svg06::multiple(md, e) {
                p.push(svg06::multiplicity_marker(x + w / 2.0, y + h));
            }
            continue;
        }
        p.push(format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#fff\" stroke=\"{}\" stroke-width=\"1.3\"/>",
            f1(x),
            f1(y),
            f1(w),
            f1(h),
            STROKE
        ));
        p.push(format!(
            "<rect x=\"{}\" y=\"{}\" width=\"30\" height=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"1.3\"/>",
            f1(x),
            f1(y),
            f1(h),
            if t == "participant" { "#f2f4f7" } else { "#f8f9fb" },
            STROKE
        ));
        let nm = md.name(Some(e));
        if !nm.is_empty() {
            let mut lines = wrap(&nm, (h - 10.0).max(40.0), 12.0);
            lines.truncate(3);
            let (cx, cy) = (x + 15.0, y + h / 2.0);
            let lh = 14.4;
            let mut tx = format!("<g transform=\"rotate(-90 {} {})\">", f1(cx), f1(cy));
            #[allow(clippy::cast_precision_loss)]
            let n = lines.len() as f64;
            for (k, ln) in lines.iter().enumerate() {
                #[allow(clippy::cast_precision_loss)]
                let off = (k as f64 - (n - 1.0) / 2.0) * lh;
                tx.push_str(&format!(
                    "<text x=\"{}\" y=\"{}\" font-size=\"12\" text-anchor=\"middle\" dominant-baseline=\"middle\" \
                     transform=\"translate(0 {})\">{}</text>",
                    f1(cx),
                    f1(cy),
                    f1(off),
                    esc(ln)
                ));
            }
            tx.push_str("</g>");
            p.push(tx);
        }
        if t == "participant" && svg06::multiple(md, e) {
            p.push(svg06::multiplicity_marker(x + w / 2.0, y + h));
        }
    }
    let bands_of = |i: &str| -> Vec<svg06::Band> {
        shape_ids
            .get(i)
            .and_then(|di| bands.get(di))
            .cloned()
            .unwrap_or_default()
    };

    // expanded sub-processes (backgrounds)
    for (i, [x, y, w, h]) in shapes.items() {
        let Some(e) = el(&i) else {
            continue;
        };
        let t = md.tag(e);
        if t == "subChoreography" && md.has_flow_children(e) {
            p.push(svg06::choreography_activity(
                md,
                e,
                &t,
                [x, y, w, h],
                &bands_of(&i),
                &mut labels,
            ));
            continue;
        }
        if !is_subp(&t) || !md.has_flow_children(e) {
            continue;
        }
        let dash = if md.is_event_sub(e) {
            " stroke-dasharray=\"2 3\""
        } else {
            ""
        };
        p.push(format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"10\" fill=\"#fff\" stroke=\"{}\" \
             stroke-width=\"1.5\"{}/>",
            f1(x),
            f1(y),
            f1(w),
            f1(h),
            STROKE,
            dash
        ));
        if t == "transaction" {
            p.push(format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"8\" fill=\"none\" stroke=\"{}\"/>",
                f1(x + 3.0),
                f1(y + 3.0),
                f1(w - 6.0),
                f1(h - 6.0),
                STROKE
            ));
        }
        let nm = md.name(Some(e));
        if !nm.is_empty() {
            let mut lines = wrap(&nm, w - 20.0, 11.0);
            lines.truncate(2);
            p.push(text(
                &lines,
                x + 10.0,
                y + 18.0,
                11,
                "start",
                "font-weight=\"bold\"",
            ));
        }
        p.push(markers(md, e, x, y, w, h));
    }

    // edges
    for (i, pts) in edges.items() {
        let Some(e) = el(&i) else {
            continue;
        };
        if pts.len() < 2 {
            continue;
        }
        let t = md.tag(e);
        let pstr = pts_str(&pts);
        if t == "sequenceFlow" {
            p.push(format!(
                "<polyline points=\"{pstr}\" fill=\"none\" stroke=\"{STROKE}\" stroke-width=\"1.5\" \
                 stroke-linejoin=\"round\" marker-end=\"url(#seqEnd)\"/>"
            ));
            let src = md.get(md.attr(e, "sourceRef"));
            if src.is_some_and(|s| md.attr(s, "default") == Some(i.as_str())) {
                let ((x1, y1), (x2, y2)) = (pts[0], pts[1]);
                let (dx, dy) = (x2 - x1, y2 - y1);
                let l = (dx * dx + dy * dy).powf(0.5).max(1.0);
                let (ux, uy) = (dx / l, dy / l);
                let (mx, my) = (x1 + ux * 10.0, y1 + uy * 10.0);
                p.push(format!(
                    "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"1.5\"/>",
                    f1(mx - 5.0 * ux - 5.0 * uy),
                    f1(my - 5.0 * uy + 5.0 * ux),
                    f1(mx + 5.0 * ux + 5.0 * uy),
                    f1(my + 5.0 * uy - 5.0 * ux),
                    STROKE
                ));
            }
            if src.is_some_and(|s| is_act(&md.tag(s)))
                && d.find(e, &m("conditionExpression")).is_some()
            {
                let ((x1, y1), (x2, y2)) = (pts[0], pts[1]);
                let (dx, dy) = (x2 - x1, y2 - y1);
                let l = (dx * dx + dy * dy).powf(0.5).max(1.0);
                let (ux, uy) = (dx / l, dy / l);
                let pd: Vec<Pt> = vec![
                    (x1, y1),
                    (x1 + ux * 7.0 - uy * 5.0, y1 + uy * 7.0 + ux * 5.0),
                    (x1 + ux * 14.0, y1 + uy * 14.0),
                    (x1 + ux * 7.0 + uy * 5.0, y1 + uy * 7.0 - ux * 5.0),
                ];
                p.push(format!(
                    "<polygon points=\"{}\" fill=\"#fff\" stroke=\"{}\" stroke-width=\"1.2\"/>",
                    pts_str(&pd),
                    STROKE
                ));
            }
            let nm = md.name(Some(e));
            if !nm.is_empty() {
                labels.push(match lbounds.get(&i) {
                    Some(b) => bounded_label(&nm, *b, ""),
                    None => flow_label(&nm, &pts),
                });
            }
        } else if t == "messageFlow" {
            p.push(format!(
                "<polyline points=\"{pstr}\" fill=\"none\" stroke=\"{STROKE}\" stroke-width=\"1.3\" \
                 stroke-dasharray=\"7 4\" marker-start=\"url(#msgStart)\" marker-end=\"url(#msgEnd)\"/>"
            ));
            let nm = md.name(Some(e));
            if let (false, Some(b)) = (nm.is_empty(), lbounds.get(&i)) {
                labels.push(bounded_label(&nm, *b, "font-style=\"italic\""));
            } else if !nm.is_empty() {
                let (x, y) = msg_label_anchor(&pts);
                labels.push(text(
                    &wrap(&nm, 110.0, 10.0),
                    x,
                    y,
                    10,
                    "start",
                    "font-style=\"italic\"",
                ));
            }
        } else if t == "conversationLink" {
            p.push(svg06::link(&pts));
        } else if t == "association" {
            let me = if matches!(md.attr(e, "associationDirection"), Some("One" | "Both")) {
                " marker-end=\"url(#assocEnd)\""
            } else {
                ""
            };
            p.push(format!(
                "<polyline points=\"{pstr}\" fill=\"none\" stroke=\"{STROKE}\" stroke-width=\"1.2\" \
                 stroke-dasharray=\"2 3\"{me}/>"
            ));
        } else if t == "dataInputAssociation" || t == "dataOutputAssociation" {
            p.push(format!(
                "<polyline points=\"{pstr}\" fill=\"none\" stroke=\"{STROKE}\" stroke-width=\"1.2\" \
                 stroke-dasharray=\"2 3\" marker-end=\"url(#assocEnd)\"/>"
            ));
        }
    }

    // nodes
    let mut boundary_draw: Vec<(Id, [f64; 4], Option<[f64; 4]>)> = Vec::new();
    for (i, bx) in shapes.items() {
        let Some(e) = el(&i) else {
            continue;
        };
        let t = md.tag(e);
        if t == "participant" || t == "lane" {
            continue;
        }
        if t == "boundaryEvent" {
            boundary_draw.push((e, bx, lbounds.get(&i).copied()));
            continue;
        }
        if (is_subp(&t) || t == "subChoreography") && md.has_flow_children(e) {
            continue;
        }
        if is_conv(&t) {
            p.push(svg06::conversation(md, e, &t, bx, &mut labels));
            continue;
        }
        if t == "group" {
            p.push(svg06::group(md, e, bx, &mut labels));
            continue;
        }
        if is_choreo_act(&t) {
            p.push(svg06::choreography_activity(
                md,
                e,
                &t,
                bx,
                &bands_of(&i),
                &mut labels,
            ));
            continue;
        }
        let lb = lbounds.get(&i).copied();
        p.push(draw_node(md, e, &t, bx, lb, &mut labels));
    }
    for (e, bx, lb) in boundary_draw {
        p.push(draw_node(md, e, "boundaryEvent", bx, lb, &mut labels));
    }
    // the name of each choreography above its diagram (language 0.6)
    for plane in d.iter_tag(r, &qn(BPMNDI_NS, "BPMNPlane")) {
        let Some(c) = md.get(md.attr(plane, "bpmnElement")) else {
            continue;
        };
        if md.tag(c) != "choreography" {
            continue;
        }
        let nm = md.name(Some(c));
        let mut top: Option<(f64, f64)> = None;
        for s in d.findall(plane, &qn(BPMNDI_NS, "BPMNShape")) {
            if let Some(b) = d.find(s, &qn(DC_NS, "Bounds")) {
                let (x, y) = (parse_f(md.attr(b, "x")), parse_f(md.attr(b, "y")));
                top = Some(top.map_or((x, y), |(tx, ty)| (tx.min(x), ty.min(y))));
            }
        }
        if let (false, Some((x, y))) = (nm.is_empty(), top) {
            labels.push(svg06::title(&nm, x, y));
        }
    }
    p.extend(labels);
    p.push("</svg>".to_string());
    Ok(p.join("\n"))
}

/// Outline of a data object (a page with a folded corner).
fn data_object_shape(x: f64, y: f64, w: f64, h: f64) -> String {
    format!(
        "<path d=\"M{},{} h{} l10,10 v{} h-{} z M{},{} v10 h10\" fill=\"#fff\" stroke=\"{}\" \
         stroke-width=\"1.3\" stroke-linejoin=\"round\"/>",
        f1(x),
        f1(y),
        f1(w - 10.0),
        f1(h - 10.0),
        f1(w),
        f1(x + w - 10.0),
        f1(y),
        STROKE
    )
}

/// Collection marker (three vertical bars) at the bottom centre of a data shape.
fn collection_marker(cx: f64, bottom: f64) -> String {
    let mut s = String::new();
    for dd in [-4.0, 0.0, 4.0] {
        s.push_str(&format!(
            "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"2\"/>",
            f1(cx + dd),
            f1(bottom - 13.0),
            f1(cx + dd),
            f1(bottom - 4.0),
            STROKE
        ));
    }
    s
}

#[allow(clippy::too_many_lines)]
fn draw_node(
    md: &Model,
    e: Id,
    t: &str,
    b: [f64; 4],
    lb: Option<[f64; 4]>,
    labels: &mut Vec<String>,
) -> String {
    let [x, y, w, h] = b;
    let mut s = String::new();
    let nm = md.name(Some(e));
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    if is_act(t) {
        let sw = if t == "callActivity" { "4" } else { "1.5" };
        s.push_str(&format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"10\" fill=\"#fff\" stroke=\"{}\" stroke-width=\"{}\"/>",
            f1(x),
            f1(y),
            f1(w),
            f1(h),
            STROKE,
            sw
        ));
        if t == "transaction" {
            // collapsed transaction: double border, as the expanded one
            s.push_str(&format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"8\" fill=\"none\" stroke=\"{}\"/>",
                f1(x + 3.0),
                f1(y + 3.0),
                f1(w - 6.0),
                f1(h - 6.0),
                STROKE
            ));
        }
        // a call activity calling a global task shows the marker of the task type
        let kind = called_global_task(md, e).unwrap_or(t);
        if t == "receiveTask" && md.attr(e, "instantiate") == Some("true") {
            s.push_str(&instantiating_receive_icon(x, y));
        } else {
            s.push_str(&task_icon(kind, x, y));
        }
        let has_icon = matches!(
            kind,
            "userTask"
                | "serviceTask"
                | "manualTask"
                | "scriptTask"
                | "businessRuleTask"
                | "sendTask"
                | "receiveTask"
        );
        let mut fs = 11i64;
        let mut lines = Vec::new();
        for f in [11i64, 10, 9] {
            fs = f;
            #[allow(clippy::cast_precision_loss)]
            let ff = f as f64;
            lines = wrap(&nm, w - 12.0, ff);
            #[allow(clippy::cast_precision_loss)]
            let need = lines.len() as f64 * ff * 1.2;
            if need <= h - if has_icon { 30.0 } else { 14.0 } {
                break;
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let fsf = fs as f64;
        let lh = fsf * 1.2;
        #[allow(clippy::cast_precision_loss)]
        let top =
            cy - (lines.len() as f64 * lh) / 2.0 + fsf * 0.85 + if has_icon { 6.0 } else { 0.0 };
        s.push_str(&text(&lines, cx, top, fs, "middle", ""));
        s.push_str(&markers(md, e, x, y, w, h));
    } else if is_evt(t) {
        let defs: Vec<String> = md.eventdefs(e).into_iter().filter(|d| d != "ref").collect();
        let nonint = (t == "boundaryEvent" && md.attr(e, "cancelActivity") == Some("false"))
            || (t == "startEvent" && md.attr(e, "isInterrupting") == Some("false"));
        let dash = if nonint {
            " stroke-dasharray=\"5 3\""
        } else {
            ""
        };
        let r = 18;
        if t == "startEvent" {
            s.push_str(&format!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"#fff\" stroke=\"{}\" stroke-width=\"1.6\"{}/>",
                f1(cx),
                f1(cy),
                r,
                STROKE,
                dash
            ));
        } else if t == "endEvent" {
            s.push_str(&format!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"#fff\" stroke=\"{}\" stroke-width=\"3.5\"/>",
                f1(cx),
                f1(cy),
                r - 1,
                STROKE
            ));
        } else {
            s.push_str(&format!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"#fff\" stroke=\"{}\" stroke-width=\"1.3\"{}/>",
                f1(cx),
                f1(cy),
                r,
                STROKE,
                dash
            ));
            s.push_str(&format!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"1.3\"{}/>",
                f1(cx),
                f1(cy),
                r - 3,
                STROKE,
                dash
            ));
        }
        let filled = t == "endEvent" || t == "intermediateThrowEvent";
        let parallel = !filled && md.attr(e, "parallelMultiple") == Some("true");
        s.push_str(&event_icon(&defs, cx, cy, filled, parallel));
        if let (false, Some(b)) = (nm.is_empty(), lb) {
            labels.push(bounded_label(&nm, b, ""));
        } else if !nm.is_empty() {
            labels.push(text(
                &wrap(&nm, 110.0, 10.0),
                cx,
                y + h + 13.0,
                10,
                "middle",
                "",
            ));
        }
    } else if is_gw(t) {
        s.push_str(&format!(
            "<polygon points=\"{},{} {},{} {},{} {},{}\" fill=\"#fff\" stroke=\"{}\" stroke-width=\"1.5\"/>",
            f1(cx),
            f1(y),
            f1(x + w),
            f1(cy),
            f1(cx),
            f1(y + h),
            f1(x),
            f1(cy),
            STROKE
        ));
        match t {
            "exclusiveGateway" => s.push_str(&format!(
                "<path d=\"M{},{} L{},{} M{},{} L{},{}\" stroke=\"{}\" stroke-width=\"4\"/>",
                f1(cx - 8.0),
                f1(cy - 8.0),
                f1(cx + 8.0),
                f1(cy + 8.0),
                f1(cx + 8.0),
                f1(cy - 8.0),
                f1(cx - 8.0),
                f1(cy + 8.0),
                STROKE
            )),
            "parallelGateway" => s.push_str(&format!(
                "<path d=\"M{},{} L{},{} M{},{} L{},{}\" stroke=\"{}\" stroke-width=\"4\"/>",
                f1(cx - 11.0),
                f1(cy),
                f1(cx + 11.0),
                f1(cy),
                f1(cx),
                f1(cy - 11.0),
                f1(cx),
                f1(cy + 11.0),
                STROKE
            )),
            "inclusiveGateway" => s.push_str(&format!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"10\" fill=\"none\" stroke=\"{}\" stroke-width=\"2.5\"/>",
                f1(cx),
                f1(cy),
                STROKE
            )),
            "complexGateway" => s.push_str(&format!(
                "<path d=\"M{},{} L{},{} M{},{} L{},{} M{},{} L{},{} M{},{} L{},{}\" stroke=\"{}\" stroke-width=\"3\"/>",
                f1(cx - 11.0),
                f1(cy),
                f1(cx + 11.0),
                f1(cy),
                f1(cx),
                f1(cy - 11.0),
                f1(cx),
                f1(cy + 11.0),
                f1(cx - 8.0),
                f1(cy - 8.0),
                f1(cx + 8.0),
                f1(cy + 8.0),
                f1(cx + 8.0),
                f1(cy - 8.0),
                f1(cx - 8.0),
                f1(cy + 8.0),
                STROKE
            )),
            "eventBasedGateway" => {
                s.push_str(&format!(
                    "<circle cx=\"{}\" cy=\"{}\" r=\"12\" fill=\"none\" stroke=\"{}\" stroke-width=\"1.2\"/>",
                    f1(cx),
                    f1(cy),
                    STROKE
                ));
                // a gateway that starts the process has a single circle (BPMN 2.0.2, 10.6.6);
                // the parallel type exists only for such gateways
                let parallel = md.attr(e, "eventGatewayType") == Some("Parallel");
                if !parallel && md.attr(e, "instantiate") != Some("true") {
                    s.push_str(&format!(
                        "<circle cx=\"{}\" cy=\"{}\" r=\"9.5\" fill=\"none\" stroke=\"{}\" stroke-width=\"1.2\"/>",
                        f1(cx),
                        f1(cy),
                        STROKE
                    ));
                }
                if parallel {
                    s.push_str(&format!(
                        "<polygon points=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"1.2\"/>",
                        plus_outline(cx, cy, 7.5, 2.5),
                        STROKE
                    ));
                } else {
                    s.push_str(&format!(
                        "<polygon points=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"1.2\"/>",
                        pentagon(cx, cy, 6.0),
                        STROKE
                    ));
                }
            }
            _ => {}
        }
        if let (false, Some(b)) = (nm.is_empty(), lb) {
            labels.push(bounded_label(&nm, b, ""));
        } else if !nm.is_empty() {
            labels.push(text(
                &wrap(&nm, 120.0, 10.0),
                cx,
                y + h + 13.0,
                10,
                "middle",
                "",
            ));
        }
    } else if t == "dataObjectReference" {
        s.push_str(&data_object_shape(x, y, w, h));
        let dobj = md.get(md.attr(e, "dataObjectRef"));
        if dobj.is_some_and(|o| md.attr(o, "isCollection") == Some("true")) {
            s.push_str(&collection_marker(cx, y + h));
        }
        let st = md.doc.find(e, &m("dataState"));
        let mut lab = if nm.is_empty() {
            dobj.map(|o| md.name(Some(o))).unwrap_or_default()
        } else {
            nm.clone()
        };
        if let Some(stn) = st
            .and_then(|x| md.attr(x, "name"))
            .filter(|v| !v.is_empty())
        {
            lab.push_str(&format!(" [{stn}]"));
        }
        if let (false, Some(b)) = (lab.is_empty(), lb) {
            labels.push(bounded_label(&lab, b, ""));
        } else if !lab.is_empty() {
            labels.push(text(
                &wrap(&lab, 110.0, 10.0),
                cx,
                y + h + 13.0,
                10,
                "middle",
                "",
            ));
        }
    } else if t == "dataInput" || t == "dataOutput" {
        // a data object with an arrow in the upper left corner: unfilled for an input,
        // filled for an output
        s.push_str(&data_object_shape(x, y, w, h));
        s.push_str(&format!(
            "<path d=\"M{},{} h9 v-3 l5,5 l-5,5 v-3 h-9 z\" fill=\"{}\" stroke=\"{}\" stroke-width=\"1\" \
             stroke-linejoin=\"round\"/>",
            f1(x + 5.0),
            f1(y + 9.0),
            if t == "dataOutput" { STROKE } else { "#fff" },
            STROKE
        ));
        let item = md.get(md.attr(e, "itemSubjectRef"));
        if md.attr(e, "isCollection") == Some("true")
            || item.is_some_and(|i| md.attr(i, "isCollection") == Some("true"))
        {
            s.push_str(&collection_marker(cx, y + h));
        }
        if let (false, Some(b)) = (nm.is_empty(), lb) {
            labels.push(bounded_label(&nm, b, ""));
        } else if !nm.is_empty() {
            labels.push(text(
                &wrap(&nm, 110.0, 10.0),
                cx,
                y + h + 13.0,
                10,
                "middle",
                "",
            ));
        }
    } else if t == "dataStoreReference" {
        let rx = w / 2.0;
        s.push_str(&format!(
            "<path d=\"M{},{} a{},6 0 0 0 {},0 a{},6 0 0 0 -{},0 v{} a{},6 0 0 0 {},0 v-{}\" fill=\"#fff\" \
             stroke=\"{}\" stroke-width=\"1.3\"/>",
            f1(x),
            f1(y + 6.0),
            f1(rx),
            f1(w),
            f1(rx),
            f1(w),
            f1(h - 12.0),
            f1(rx),
            f1(w),
            f1(h - 12.0),
            STROKE
        ));
        s.push_str(&format!(
            "<path d=\"M{},{} a{},6 0 0 0 {},0 M{},{} a{},6 0 0 0 {},0\" fill=\"none\" stroke=\"{}\" \
             stroke-width=\"1.1\"/>",
            f1(x),
            f1(y + 11.0),
            f1(rx),
            f1(w),
            f1(x),
            f1(y + 16.0),
            f1(rx),
            f1(w),
            STROKE
        ));
        let ds = md.get(md.attr(e, "dataStoreRef"));
        let lab = if nm.is_empty() {
            ds.map(|o| md.name(Some(o))).unwrap_or_default()
        } else {
            nm.clone()
        };
        if let (false, Some(b)) = (lab.is_empty(), lb) {
            labels.push(bounded_label(&lab, b, ""));
        } else if !lab.is_empty() {
            labels.push(text(
                &wrap(&lab, 110.0, 10.0),
                cx,
                y + h + 13.0,
                10,
                "middle",
                "",
            ));
        }
    } else if t == "textAnnotation" {
        s.push_str(&format!(
            "<path d=\"M{},{} h-10 v{} h10\" fill=\"none\" stroke=\"{}\" stroke-width=\"1.3\"/>",
            f1(x + 10.0),
            f1(y),
            f1(h),
            STROKE
        ));
        let txt = md.doc.findtext(e, &m("text")).unwrap_or_default();
        s.push_str(&text(
            &wrap(txt.trim(), w - 10.0, 11.0),
            x + 6.0,
            y + 15.0,
            11,
            "start",
            "",
        ));
    }
    s
}
