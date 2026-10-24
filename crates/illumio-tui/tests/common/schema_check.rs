//! Every envelope a CLI test reads is checked against the schema it names
//! in `meta.schema` (`illumio-tui schema NAME`), so a schema file cannot
//! drift from the code: a new, renamed or removed field fails the test that
//! printed it. The validator knows the subset of JSON Schema the files use
//! and refuses any other keyword.
#![allow(dead_code)]
use serde_json::Value;

/// Parses a command's stdout as JSON; an envelope is checked against its
/// schema.
pub fn envelope(stdout: &[u8]) -> Value {
    let v: Value = serde_json::from_slice(stdout).expect("stdout is JSON");
    check(&v);
    v
}

/// [`envelope`] of a command run with `--fields`: `data` holds fewer
/// fields, so `required` is not checked; every field left must still be
/// the schema's.
pub fn picked(stdout: &[u8]) -> Value {
    let v: Value = serde_json::from_slice(stdout).expect("stdout is JSON");
    check_as(&v, Mode::Picked);
    v
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Whole,
    Picked,
}

/// Checks `v` when it is an envelope (`data` and `meta`): it must name its
/// schema and follow it.
pub fn check(v: &Value) {
    check_as(v, Mode::Whole)
}

fn check_as(v: &Value, mode: Mode) {
    let Some(meta) = v.get("meta").filter(|_| v.get("data").is_some()) else {
        return;
    };
    let name = meta["schema"]
        .as_str()
        .unwrap_or_else(|| panic!("envelope without meta.schema: {v}"));
    let doc = illumio_tui::cli::schema::document(name)
        .unwrap_or_else(|| panic!("meta.schema {name:?} is no schema"));
    let errors = run(&doc, v, mode);
    assert!(
        errors.is_empty(),
        "output does not follow schema {name}:\n  {}\noutput: {}",
        errors.join("\n  "),
        serde_json::to_string_pretty(v).unwrap()
    );
}

/// The errors of `v` against the schema document `doc`; empty when valid.
pub fn validate(doc: &Value, v: &Value) -> Vec<String> {
    run(doc, v, Mode::Whole)
}

fn run(doc: &Value, v: &Value, mode: Mode) -> Vec<String> {
    let mut errors = Vec::new();
    node(&Ctx { doc, mode }, doc, v, "$", &mut errors);
    errors
}

struct Ctx<'a> {
    doc: &'a Value,
    mode: Mode,
}

const ANNOTATIONS: &[&str] = &[
    "$schema",
    "$defs",
    "$comment",
    "title",
    "description",
    "examples",
];

fn type_ok(t: &str, v: &Value) -> bool {
    match t {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "integer" => v.is_i64() || v.is_u64(),
        "number" => v.is_number(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        other => panic!("unknown type {other:?}"),
    }
}

fn resolve<'a>(doc: &'a Value, r: &str) -> &'a Value {
    let name = r
        .strip_prefix("#/$defs/")
        .unwrap_or_else(|| panic!("only #/$defs/ refs: {r}"));
    doc.get("$defs")
        .and_then(|d| d.get(name))
        .unwrap_or_else(|| panic!("no $defs/{name}"))
}

fn node(cx: &Ctx, s: &Value, v: &Value, at: &str, errors: &mut Vec<String>) {
    let doc = cx.doc;
    let s = match s {
        Value::Bool(true) => return,
        Value::Bool(false) => {
            errors.push(format!("{at}: not allowed"));
            return;
        }
        Value::Object(o) => o,
        other => panic!("schema at {at} is {other}"),
    };
    for (k, kv) in s {
        match k.as_str() {
            "$ref" => node(cx, resolve(doc, kv.as_str().unwrap()), v, at, errors),
            "type" => {
                let ok = match kv {
                    Value::String(t) => type_ok(t, v),
                    Value::Array(ts) => ts.iter().any(|t| type_ok(t.as_str().unwrap(), v)),
                    _ => panic!("type at {at}"),
                };
                if !ok {
                    errors.push(format!("{at}: {v} is not of type {kv}"));
                }
            }
            "enum" => {
                if !kv.as_array().unwrap().contains(v) {
                    errors.push(format!("{at}: {v} not in {kv}"));
                }
            }
            "const" => {
                if kv != v {
                    errors.push(format!("{at}: {v} is not {kv}"));
                }
            }
            "properties" => {
                if let Some(o) = v.as_object() {
                    for (name, sub) in kv.as_object().unwrap() {
                        if let Some(x) = o.get(name) {
                            node(cx, sub, x, &format!("{at}.{name}"), errors);
                        }
                    }
                }
            }
            "required" if cx.mode == Mode::Picked && at.starts_with("$.data") => {}
            "required" => {
                if let Some(o) = v.as_object() {
                    for name in kv.as_array().unwrap() {
                        let name = name.as_str().unwrap();
                        if !o.contains_key(name) {
                            errors.push(format!("{at}: missing required {name:?}"));
                        }
                    }
                }
            }
            "additionalProperties" => {
                if let Some(o) = v.as_object() {
                    let known = s.get("properties").and_then(Value::as_object);
                    for (name, x) in o {
                        if known.is_some_and(|p| p.contains_key(name)) {
                            continue;
                        }
                        match kv {
                            Value::Bool(false) => {
                                errors.push(format!("{at}: unexpected field {name:?}"))
                            }
                            sub => node(cx, sub, x, &format!("{at}.{name}"), errors),
                        }
                    }
                }
            }
            "items" => {
                if let Some(a) = v.as_array() {
                    for (i, x) in a.iter().enumerate() {
                        node(cx, kv, x, &format!("{at}[{i}]"), errors);
                    }
                }
            }
            "oneOf" | "anyOf" => {
                let branches = kv.as_array().unwrap();
                let passing = branches
                    .iter()
                    .filter(|b| {
                        let mut e = Vec::new();
                        node(cx, b, v, at, &mut e);
                        e.is_empty()
                    })
                    .count();
                let ok = if k == "oneOf" {
                    passing == 1
                } else {
                    passing >= 1
                };
                if !ok {
                    // The closest branch's errors say most.
                    let detail = branches
                        .iter()
                        .map(|b| {
                            let mut e = Vec::new();
                            node(cx, b, v, at, &mut e);
                            e
                        })
                        .min_by_key(Vec::len)
                        .unwrap_or_default();
                    errors.push(format!(
                        "{at}: {passing} of the {k} branches match; closest: {}",
                        detail.join("; ")
                    ));
                }
            }
            a if ANNOTATIONS.contains(&a) => {}
            other => panic!("keyword {other:?} at {at} is outside the validator's subset"),
        }
    }
}
