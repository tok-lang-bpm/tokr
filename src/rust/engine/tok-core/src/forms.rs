//! Forms: export to Camunda form JSON (spec §10.2). Mirrors `forms.py`.

use crate::json::{Json, obj};
use crate::model::Form;
use crate::pyutil::strip;

fn is_number(t: &str) -> bool {
    let body = t.strip_prefix('-').unwrap_or(t);
    let (int, frac) = match body.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (body, None),
    };
    !int.is_empty()
        && int.chars().all(|c| c.is_ascii_digit())
        && frac
            .map(|f| !f.is_empty() && f.chars().all(|c| c.is_ascii_digit()))
            .unwrap_or(true)
}

/// Option or default value: true/false, numbers, else the text.
fn convert(text: &str) -> Json {
    let t = strip(text);
    match t {
        "true" => Json::Bool(true),
        "false" => Json::Bool(false),
        _ if is_number(t) => {
            if t.contains('.') {
                Json::Float(t.parse().unwrap_or(0.0))
            } else {
                let digits = t.trim_start_matches('-').trim_start_matches('0');
                let digits = if digits.is_empty() { "0" } else { digits };
                let neg = t.starts_with('-') && digits != "0";
                Json::Num(if neg {
                    format!("-{digits}")
                } else {
                    digits.to_string()
                })
            }
        }
        _ => Json::Str(t.to_string()),
    }
}

/// `'value:Label, value:Label'` to Camunda option objects.
pub fn parse_options(text: &str) -> Vec<Json> {
    let mut out = Vec::new();
    for part in text.split(',') {
        let part = strip(part);
        if part.is_empty() {
            continue;
        }
        match part.split_once(':') {
            Some((value, label)) => out.push(obj(vec![
                ("label", Json::Str(strip(label).to_string())),
                ("value", convert(value)),
            ])),
            None => out.push(obj(vec![
                ("label", Json::Str(part.to_string())),
                ("value", convert(part)),
            ])),
        }
    }
    out
}

/// The Camunda form as JSON.
pub fn form_json(form: &Form) -> Json {
    let mut components = Vec::new();
    let mut ntext = 0;
    for item in &form.items {
        let b = &item.base;
        if b.kind == "text" {
            ntext += 1;
            components.push(obj(vec![
                ("type", Json::Str("text".into())),
                ("id", Json::Str(format!("Text_{ntext}"))),
                ("text", Json::Str(b.label.clone().unwrap_or_default())),
            ]));
            continue;
        }
        let key = b.id().to_string();
        let typ = b
            .attrs
            .get("type")
            .cloned()
            .unwrap_or_else(|| "textfield".into());
        let label = b
            .label
            .clone()
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| key.clone());
        let mut c: Vec<(&str, Json)> = vec![
            ("type", Json::Str(typ.clone())),
            ("id", Json::Str(format!("Field_{key}"))),
            ("key", Json::Str(key.clone())),
        ];
        if typ == "datetime" {
            c.push(("subtype", Json::Str("date".into())));
            c.push(("dateLabel", Json::Str(label)));
        } else {
            c.push(("label", Json::Str(label)));
        }
        if (typ == "select" || typ == "radio")
            && let Some(o) = b.attrs.get("options")
        {
            c.push(("values", Json::Arr(parse_options(o))));
        }
        if let Some(d) = b.attrs.get("default") {
            c.push(("defaultValue", convert(d)));
        }
        let mut validate: Vec<(&str, Json)> = Vec::new();
        if b.has("required") {
            validate.push(("required", Json::Bool(true)));
        }
        if typ == "number" {
            for k in ["min", "max"] {
                if let Some(v) = b.attrs.get(k) {
                    validate.push((k, convert(v)));
                }
            }
        }
        if !validate.is_empty() {
            c.push(("validate", obj(validate)));
        }
        components.push(obj(c));
    }
    obj(vec![
        ("id", Json::Str(form.base.id().to_string())),
        ("type", Json::Str("default".into())),
        ("executionPlatform", Json::Str("Camunda Cloud".into())),
        ("executionPlatformVersion", Json::Str("8.10.0".into())),
        (
            "exporter",
            obj(vec![
                ("name", Json::Str("tok".into())),
                ("version", Json::Str("0.1.0".into())),
            ]),
        ),
        ("components", Json::Arr(components)),
        ("schemaVersion", Json::Int(18)),
    ])
}

/// Camunda form JSON text (file `<form id>.form`).
pub fn export_form(form: &Form) -> String {
    format!("{}\n", form_json(form).pretty())
}
