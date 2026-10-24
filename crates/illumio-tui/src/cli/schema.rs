//! `illumio-tui schema`: the JSON Schema (draft 2020-12) of every
//! command's envelope, and the field names `--fields` accepts. The schemas
//! are hand-kept files under `schemas/`, embedded here; the CLI tests check
//! every command's real output against them, so they cannot drift.
use super::output::{Meta, OutputFormat, TableRows, choose_format, meta_line, write_json};
use super::{CliError, Command, GlobalArgs};
use clap::Args;
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

#[derive(Args, Debug, Clone, Default)]
pub struct SchemaArgs {
    /// A command's words (`workloads ven-summary`) or a schema name
    /// (`write`, `envelope`, `meta`, `error`); none lists them all
    pub command: Vec<String>,
}

/// The shapes every command shares, by name.
pub const SHARED: &[(&str, &str)] = &[
    (
        "envelope",
        "what every command prints with --json: {\"data\": ..., \"meta\": ...}",
    ),
    (
        "meta",
        "the envelope's meta: profile, counts, cache, warnings …",
    ),
    (
        "error",
        "what a failed command prints to stderr: {\"error\": {kind, message, exit_code}}",
    ),
];

/// Every command that prints an envelope, and the schema of its `data`.
/// Commands that print one of several shapes by flag list each.
pub const COMMANDS: &[(&str, &str)] = &[
    ("health", "health"),
    ("labels", "labels"),
    ("labels add", "write"),
    ("labels update", "write"),
    ("labels delete", "write"),
    ("label-groups", "label-groups"),
    ("iplists", "iplists"),
    ("iplists add", "write"),
    ("iplists update", "write"),
    ("iplists delete", "write"),
    ("services", "services"),
    ("services add", "write"),
    ("services update", "write"),
    ("services delete", "write"),
    ("events", "events"),
    ("events --save-filter", "saved-filter"),
    ("workloads", "workloads"),
    ("workloads --save-filter", "saved-filter"),
    ("workloads add", "write"),
    ("workloads update", "write"),
    ("workloads delete", "write"),
    ("workloads enforce", "bulk"),
    ("workloads relabel", "bulk"),
    ("workloads ven-summary", "workloads-ven-summary"),
    ("workloads ven-summary --save-filter", "saved-filter"),
    ("app", "app"),
    ("gen", "gen"),
    ("readiness", "readiness"),
    ("enforce", "enforce"),
    ("enforce --rollback", "enforce-rollback"),
    ("enforce --watch", "enforce-watch"),
    ("policy rulesets", "policy-rulesets"),
    ("policy rulesets add", "write"),
    ("policy rulesets update", "write"),
    ("policy rulesets delete", "write"),
    ("policy rulesets enable", "write"),
    ("policy rulesets disable", "write"),
    ("policy rules", "policy-rules"),
    ("policy rules add", "write"),
    ("policy rules update", "write"),
    ("policy rules delete", "write"),
    ("policy rules impact", "policy-rules-impact"),
    ("policy rules enable", "write"),
    ("policy rules disable", "write"),
    ("policy deny-rules add", "write"),
    ("policy deny-rules update", "write"),
    ("policy deny-rules delete", "write"),
    ("policy search", "policy-search"),
    ("policy search --save-filter", "saved-filter"),
    ("policy pending", "policy-pending"),
    ("policy provision", "write"),
    ("policy revert", "write"),
    ("traffic query", "traffic-query"),
    ("traffic query --dry-run", "traffic-dry-run"),
    ("traffic query --save-filter", "saved-filter"),
    ("traffic groups", "traffic-groups"),
    ("traffic groups --dry-run", "traffic-dry-run"),
    ("traffic groups --save-filter", "saved-filter"),
    ("traffic mesh", "traffic-mesh"),
    ("traffic mesh --dry-run", "traffic-dry-run"),
    ("traffic mesh --save-filter", "saved-filter"),
    ("traffic explain", "traffic-explain"),
    ("profile list", "profile-list"),
    ("profile use", "profile-use"),
    ("profile add", "profile-add"),
    ("marks list", "marks-list"),
    ("marks add", "marks-add"),
    ("marks rm", "marks-rm"),
    ("marks import", "marks-import"),
    ("filters list", "filters-list"),
    ("filters show traffic", "filters-show"),
    ("filters show policy", "filters-show"),
    ("filters show events", "filters-show"),
    ("filters show workloads", "filters-show"),
    ("filters rm traffic", "filters-rm"),
    ("filters rm policy", "filters-rm"),
    ("filters rm events", "filters-rm"),
    ("filters rm workloads", "filters-rm"),
    ("filters default traffic", "filters-default"),
    ("filters default policy", "filters-default"),
    ("filters default events", "filters-default"),
    ("filters default workloads", "filters-default"),
    ("completions refresh", "completions-refresh"),
    ("completions refresh --clear", "completions-refresh"),
    ("schema", "schema-list"),
];

/// Commands that print no envelope: a script, a file, a schema.
pub const NO_ENVELOPE: &[&str] = &["completions", "marks export", "schema COMMAND"];

/// The embedded files: each a JSON Schema of `data` (`meta.json` of
/// `meta`, `error.json` of the error), with `$defs` of its own;
/// `defs.json` holds the `$defs` several share.
const FILES: &[(&str, &str)] = &[
    ("app", include_str!("../../schemas/app.json")),
    ("bulk", include_str!("../../schemas/bulk.json")),
    (
        "completions-refresh",
        include_str!("../../schemas/completions-refresh.json"),
    ),
    ("defs", include_str!("../../schemas/defs.json")),
    ("enforce", include_str!("../../schemas/enforce.json")),
    (
        "enforce-rollback",
        include_str!("../../schemas/enforce-rollback.json"),
    ),
    (
        "enforce-watch",
        include_str!("../../schemas/enforce-watch.json"),
    ),
    ("error", include_str!("../../schemas/error.json")),
    ("events", include_str!("../../schemas/events.json")),
    (
        "filters-default",
        include_str!("../../schemas/filters-default.json"),
    ),
    (
        "filters-list",
        include_str!("../../schemas/filters-list.json"),
    ),
    ("filters-rm", include_str!("../../schemas/filters-rm.json")),
    (
        "filters-show",
        include_str!("../../schemas/filters-show.json"),
    ),
    ("gen", include_str!("../../schemas/gen.json")),
    ("health", include_str!("../../schemas/health.json")),
    ("iplists", include_str!("../../schemas/iplists.json")),
    (
        "label-groups",
        include_str!("../../schemas/label-groups.json"),
    ),
    ("labels", include_str!("../../schemas/labels.json")),
    ("marks-add", include_str!("../../schemas/marks-add.json")),
    (
        "marks-import",
        include_str!("../../schemas/marks-import.json"),
    ),
    ("marks-list", include_str!("../../schemas/marks-list.json")),
    ("marks-rm", include_str!("../../schemas/marks-rm.json")),
    ("meta", include_str!("../../schemas/meta.json")),
    (
        "policy-pending",
        include_str!("../../schemas/policy-pending.json"),
    ),
    (
        "policy-rules",
        include_str!("../../schemas/policy-rules.json"),
    ),
    (
        "policy-rules-impact",
        include_str!("../../schemas/policy-rules-impact.json"),
    ),
    (
        "policy-rulesets",
        include_str!("../../schemas/policy-rulesets.json"),
    ),
    (
        "policy-search",
        include_str!("../../schemas/policy-search.json"),
    ),
    (
        "profile-add",
        include_str!("../../schemas/profile-add.json"),
    ),
    (
        "profile-list",
        include_str!("../../schemas/profile-list.json"),
    ),
    (
        "profile-use",
        include_str!("../../schemas/profile-use.json"),
    ),
    ("readiness", include_str!("../../schemas/readiness.json")),
    (
        "saved-filter",
        include_str!("../../schemas/saved-filter.json"),
    ),
    (
        "schema-list",
        include_str!("../../schemas/schema-list.json"),
    ),
    ("services", include_str!("../../schemas/services.json")),
    (
        "traffic-dry-run",
        include_str!("../../schemas/traffic-dry-run.json"),
    ),
    (
        "traffic-explain",
        include_str!("../../schemas/traffic-explain.json"),
    ),
    (
        "traffic-groups",
        include_str!("../../schemas/traffic-groups.json"),
    ),
    (
        "traffic-mesh",
        include_str!("../../schemas/traffic-mesh.json"),
    ),
    (
        "traffic-query",
        include_str!("../../schemas/traffic-query.json"),
    ),
    ("workloads", include_str!("../../schemas/workloads.json")),
    (
        "workloads-ven-summary",
        include_str!("../../schemas/workloads-ven-summary.json"),
    ),
    ("write", include_str!("../../schemas/write.json")),
];

const DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

fn file(name: &str) -> Option<Value> {
    FILES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, text)| serde_json::from_str(text).expect("embedded schema is JSON"))
}

/// A file's schema without its `$defs`, and the `$defs`.
fn split_defs(mut v: Value) -> (Value, Map<String, Value>) {
    let defs = v
        .as_object_mut()
        .and_then(|o| o.remove("$defs"))
        .and_then(|d| match d {
            Value::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default();
    (v, defs)
}

/// The `#/$defs/NAME` names `v` refers to.
fn refs(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::Object(o) => {
            for (k, v) in o {
                match (k.as_str(), v) {
                    ("$ref", Value::String(r)) => {
                        if let Some(name) = r.strip_prefix("#/$defs/") {
                            out.insert(name.to_string());
                        }
                    }
                    _ => refs(v, out),
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|v| refs(v, out)),
        _ => {}
    }
}

/// `root` with the `$defs` it refers to (transitively), from `own` first,
/// then `defs.json`.
fn with_defs(mut root: Value, own: Map<String, Value>) -> Value {
    let (_, shared) = split_defs(file("defs").expect("defs.json"));
    let mut picked = Map::new();
    let mut todo = BTreeSet::new();
    refs(&root, &mut todo);
    while let Some(name) = todo.pop_first() {
        if picked.contains_key(&name) {
            continue;
        }
        let def = own
            .get(&name)
            .or_else(|| shared.get(&name))
            .unwrap_or_else(|| panic!("schema refers to unknown $defs/{name}"))
            .clone();
        refs(&def, &mut todo);
        picked.insert(name, def);
    }
    if !picked.is_empty()
        && let Some(o) = root.as_object_mut()
    {
        o.insert("$defs".into(), Value::Object(picked));
    }
    root
}

/// The schema name of a command path or a schema name, if either.
pub fn resolve(words: &str) -> Option<&'static str> {
    COMMANDS
        .iter()
        .find(|(c, _)| *c == words)
        .map(|(_, s)| *s)
        .or_else(|| {
            COMMANDS
                .iter()
                .map(|(_, s)| *s)
                .chain(SHARED.iter().map(|(s, _)| *s))
                .find(|s| *s == words)
        })
}

/// The whole schema `name` prints: an envelope with that `data` and the
/// shared `meta`, or a shared shape.
pub fn document(name: &str) -> Option<Value> {
    let (title, root, own) = match name {
        "meta" | "error" => {
            let (s, own) = split_defs(file(name)?);
            (format!("illumio-tui {name}"), s, own)
        }
        "envelope" => (
            "illumio-tui envelope".to_string(),
            envelope_of(
                json!({"description": "the command's result: see `illumio-tui schema COMMAND`"}),
            ),
            Map::new(),
        ),
        _ => {
            let (data, own) = split_defs(file(name)?);
            (format!("illumio-tui {name}"), envelope_of(data), own)
        }
    };
    let mut doc = json!({"$schema": DIALECT, "title": title});
    if let (Some(d), Value::Object(r)) = (doc.as_object_mut(), root) {
        d.extend(r);
    }
    let mut own = own;
    if name != "meta" && name != "error" {
        let (meta, meta_defs) = split_defs(file("meta").expect("meta.json"));
        own.insert("meta".into(), meta);
        own.extend(meta_defs);
    }
    Some(with_defs(doc, own))
}

fn envelope_of(data: Value) -> Value {
    json!({
        "type": "object",
        "properties": {"data": data, "meta": {"$ref": "#/$defs/meta"}},
        "required": ["data", "meta"],
        "additionalProperties": false,
    })
}

/// Resolves a `$ref` (one level at a time) in `doc`.
fn deref<'a>(doc: &'a Value, v: &'a Value) -> &'a Value {
    match v.get("$ref").and_then(Value::as_str) {
        Some(r) => r
            .strip_prefix("#/$defs/")
            .and_then(|n| doc.get("$defs")?.get(n))
            .map(|d| deref(doc, d))
            .unwrap_or(v),
        None => v,
    }
}

/// The dotted field names of `data` in schema `name`, and the prefixes
/// under which any name goes (a raw PCE object passed through).
pub fn fields(name: &str) -> Option<(BTreeSet<String>, BTreeSet<String>)> {
    let doc = document(name)?;
    let data = doc.get("properties")?.get("data")?;
    let mut names = BTreeSet::new();
    let mut open = BTreeSet::new();
    collect(&doc, data, "", &mut names, &mut open, 0);
    Some((names, open))
}

fn collect(
    doc: &Value,
    v: &Value,
    prefix: &str,
    names: &mut BTreeSet<String>,
    open: &mut BTreeSet<String>,
    depth: usize,
) {
    // A recursive shape stops somewhere.
    if depth > 12 {
        return;
    }
    let v = deref(doc, v);
    for key in ["oneOf", "anyOf", "allOf"] {
        for b in v.get(key).and_then(Value::as_array).into_iter().flatten() {
            collect(doc, b, prefix, names, open, depth + 1);
        }
    }
    if let Some(items) = v.get("items") {
        collect(doc, items, prefix, names, open, depth + 1);
    }
    if let Some(props) = v.get("properties").and_then(Value::as_object) {
        for (k, sub) in props {
            let path = if prefix.is_empty() {
                k.clone()
            } else {
                format!("{prefix}.{k}")
            };
            names.insert(path.clone());
            collect(doc, sub, &path, names, open, depth + 1);
        }
    }
    if is_open(v) {
        open.insert(prefix.to_string());
    }
}

/// A schema that takes fields it does not name: an object without
/// `additionalProperties: false`, or any value (a raw PCE answer).
fn is_open(v: &Value) -> bool {
    const SHAPE: &[&str] = &[
        "type",
        "$ref",
        "properties",
        "items",
        "anyOf",
        "oneOf",
        "enum",
        "const",
    ];
    let any_value = v
        .as_object()
        .is_some_and(|o| !SHAPE.iter().any(|k| o.contains_key(*k)));
    let open_object = v.get("type").is_some_and(|t| {
        t == "object"
            || t.as_array()
                .is_some_and(|a| a.iter().any(|t| t == "object"))
    }) && v.get("additionalProperties") != Some(&Value::Bool(false));
    any_value || open_object
}

/// `--fields` against schema `name`: every name a field of `data`.
pub fn check_fields(name: &str, wanted: &[String]) -> Result<(), CliError> {
    let (names, open) = fields(name).unwrap_or_default();
    for f in wanted {
        let known = names.contains(f)
            || open.contains("")
            || open.iter().any(|p| {
                f.strip_prefix(p.as_str())
                    .is_some_and(|r| r.starts_with('.'))
            });
        if !known {
            let valid = if names.is_empty() {
                "none (data has no fields)".to_string()
            } else {
                names.iter().cloned().collect::<Vec<_>>().join(", ")
            };
            return Err(CliError::Args(format!(
                "--fields: no field {f:?} in {name}; valid: {valid}"
            )));
        }
    }
    Ok(())
}

/// The schema of the envelope `command` prints; `None` when it prints none.
pub fn schema_of(command: &Command) -> Option<&'static str> {
    use super::filters::FiltersCmd as F;
    use super::marks::MarksCmd as M;
    use super::policy::{PolicyCmd as P, RulesAction as RA, RulesetsAction as RS};
    use super::profile::ProfileCmd as PR;
    use super::traffic::TrafficCmd as T;
    use super::workloads::WorkloadsAction as W;
    if super::filters::saves_filter(command) {
        return Some("saved-filter");
    }
    Some(match command {
        Command::Health => "health",
        Command::Labels(c) => match c.action {
            None => "labels",
            Some(_) => "write",
        },
        Command::LabelGroups => "label-groups",
        Command::Iplists(c) => match c.action {
            None => "iplists",
            Some(_) => "write",
        },
        Command::Services(c) => match c.action {
            None => "services",
            Some(_) => "write",
        },
        Command::Events(_) => "events",
        Command::Workloads(c) => match &c.action {
            None => "workloads",
            Some(W::Add(_) | W::Update(_) | W::Delete(_)) => "write",
            Some(W::Enforce(_) | W::Relabel(_)) => "bulk",
            Some(W::VenSummary(_)) => "workloads-ven-summary",
        },
        Command::App(_) => "app",
        Command::Gen(_) => "gen",
        Command::Readiness(_) => "readiness",
        Command::Enforce(a) if a.watch => "enforce-watch",
        Command::Enforce(a) if a.rollback => "enforce-rollback",
        Command::Enforce(_) => "enforce",
        Command::Policy(p) => match p {
            P::Rulesets(c) => match &c.action {
                None => "policy-rulesets",
                Some(
                    RS::Add(_) | RS::Update(_) | RS::Delete(_) | RS::Enable(_) | RS::Disable(_),
                ) => "write",
            },
            P::Rules(c) => match &c.action {
                None => "policy-rules",
                Some(RA::Impact(_)) => "policy-rules-impact",
                Some(
                    RA::Add(_) | RA::Update(_) | RA::Delete(_) | RA::Enable(_) | RA::Disable(_),
                ) => "write",
            },
            P::DenyRules(_) | P::Provision(_) | P::Revert(_) => "write",
            P::Search(_) => "policy-search",
            P::Pending(_) => "policy-pending",
        },
        Command::Traffic(t) => match t {
            _ if t.filter().is_some_and(|f| f.dry_run) => "traffic-dry-run",
            T::Query(_) => "traffic-query",
            T::Groups(_) => "traffic-groups",
            T::Mesh(_) => "traffic-mesh",
            T::Explain(_) => "traffic-explain",
        },
        Command::Profile(p) => match p {
            PR::List => "profile-list",
            PR::Use { .. } => "profile-use",
            PR::Add(_) => "profile-add",
        },
        Command::Marks(m) => match m {
            M::List(_) => "marks-list",
            M::Add(_) => "marks-add",
            M::Rm(_) => "marks-rm",
            M::Import(_) => "marks-import",
            M::Export(_) => return None,
        },
        Command::Filters(f) => match f {
            F::List(_) => "filters-list",
            F::Show(_) => "filters-show",
            F::Rm(_) => "filters-rm",
            F::Default(_) => "filters-default",
        },
        Command::Completions(c) => match &c.action {
            None => return None,
            Some(super::completions::CompletionsAction::Refresh(_)) => "completions-refresh",
        },
        Command::Schema(a) if a.command.is_empty() => "schema-list",
        Command::Schema(_) => return None,
    })
}

/// A row of `schema`'s list.
#[derive(Debug, Serialize)]
pub struct SchemaRow {
    pub kind: &'static str,
    pub command: Option<&'static str>,
    pub schema: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct SchemaList(pub Vec<SchemaRow>);

impl TableRows for SchemaList {
    fn headers(&self) -> Vec<&'static str> {
        vec!["kind", "command", "schema"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .map(|r| {
                vec![
                    r.kind.to_string(),
                    r.command.unwrap_or("-").to_string(),
                    r.schema.to_string(),
                ]
            })
            .collect()
    }
}

fn list() -> SchemaList {
    let shared = SHARED.iter().map(|(s, _)| SchemaRow {
        kind: "shared",
        command: None,
        schema: s,
    });
    let commands = COMMANDS.iter().map(|(c, s)| SchemaRow {
        kind: "command",
        command: Some(c),
        schema: s,
    });
    SchemaList(shared.chain(commands).collect())
}

/// `illumio-tui schema [COMMAND...]`: local, no profile, no request.
pub fn run(global: &GlobalArgs, args: &SchemaArgs) -> Result<(), CliError> {
    let format = choose_format(global.json, global.table, global.csv);
    if args.command.is_empty() {
        let rows = list();
        let meta = Meta {
            cache: "none",
            ..Meta::default()
        };
        let mut out = std::io::stdout().lock();
        match format {
            OutputFormat::Json => write_json(&mut out, &rows, &meta)?,
            OutputFormat::Table => {
                super::output::write_table(&mut out, &rows)?;
                eprintln!("{}", meta_line(&meta));
            }
            OutputFormat::Csv => {
                super::output::write_table_csv(&mut out, &rows)?;
                eprintln!("{}", meta_line(&meta));
            }
        }
        return Ok(());
    }
    let words = args.command.join(" ");
    let name = resolve(&words).ok_or_else(|| {
        let mut valid: Vec<&str> = COMMANDS.iter().map(|(c, _)| *c).collect();
        valid.dedup();
        CliError::Args(format!(
            "schema: no command or schema {words:?}; commands: {}; shared: {}",
            valid.join(", "),
            SHARED
                .iter()
                .map(|(s, _)| *s)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })?;
    let doc = document(name).expect("every listed schema is embedded");
    let mut out = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, &doc).map_err(std::io::Error::from)?;
    use std::io::Write;
    writeln!(out)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// The leaf command paths clap knows (a subcommand that takes
    /// arguments without one is a leaf too).
    fn leaves(cmd: &clap::Command, prefix: &str, out: &mut Vec<String>) {
        for sub in cmd.get_subcommands().filter(|s| s.get_name() != "help") {
            let path = if prefix.is_empty() {
                sub.get_name().to_string()
            } else {
                format!("{prefix} {}", sub.get_name())
            };
            let has_subs = sub.get_subcommands().any(|s| s.get_name() != "help");
            let runs_alone = !has_subs || !sub.is_subcommand_required_set();
            if runs_alone {
                out.push(path.clone());
            }
            leaves(sub, &path, out);
        }
    }

    #[test]
    fn every_command_has_a_schema_or_prints_no_envelope() {
        let mut paths = Vec::new();
        leaves(&super::super::Cli::command(), "", &mut paths);
        assert!(paths.len() > 50, "{paths:?}");
        for p in &paths {
            let listed = COMMANDS.iter().any(|(c, _)| c == p);
            let none = NO_ENVELOPE.iter().any(|c| c == p);
            assert!(
                listed || none,
                "{p}: add it to schema::COMMANDS (and a schema file)"
            );
        }
        // And nothing listed that clap does not know.
        for (c, _) in COMMANDS {
            let words = c.split(" --").next().unwrap();
            assert!(paths.iter().any(|p| p == words), "{c} is no command");
        }
    }

    #[test]
    fn every_listed_schema_is_embedded_and_resolves() {
        let names = COMMANDS
            .iter()
            .map(|(_, s)| s)
            .chain(SHARED.iter().map(|(s, _)| s));
        for name in names {
            let doc = document(name).unwrap_or_else(|| panic!("{name}: no schema file"));
            assert_eq!(doc["$schema"], DIALECT);
        }
        for (name, _) in FILES {
            if *name != "defs" {
                assert!(document(name).is_some(), "{name}");
            }
        }
    }

    #[test]
    fn a_command_resolves_by_words_or_schema_name() {
        assert_eq!(
            resolve("workloads ven-summary"),
            Some("workloads-ven-summary")
        );
        assert_eq!(resolve("labels add"), Some("write"));
        assert_eq!(resolve("write"), Some("write"));
        assert_eq!(resolve("envelope"), Some("envelope"));
        assert_eq!(resolve("nope"), None);
    }

    #[test]
    fn fields_reach_into_nested_objects_and_arrays() {
        let doc = json!({
            "type": "object",
            "properties": {
                "a": {"type": "string"},
                "b": {"type": "array", "items": {"$ref": "#/$defs/x"}},
                "raw": {"type": "object", "additionalProperties": true}
            },
            "additionalProperties": false,
            "$defs": {"x": {"type": "object", "properties": {"c": {"type": "integer"}}, "additionalProperties": false}}
        });
        let mut names = BTreeSet::new();
        let mut open = BTreeSet::new();
        collect(&doc, &doc, "", &mut names, &mut open, 0);
        assert_eq!(
            names.into_iter().collect::<Vec<_>>(),
            vec!["a", "b", "b.c", "raw"]
        );
        assert_eq!(open.into_iter().collect::<Vec<_>>(), vec!["raw"]);
    }

    #[test]
    fn unknown_fields_name_the_valid_ones() {
        let e = check_fields("health", &["nope".into()]).unwrap_err();
        let msg = e.to_string();
        assert!(msg.contains("no field \"nope\" in health"), "{msg}");
        assert!(msg.contains("host"), "{msg}");
        assert!(check_fields("health", &["host".into(), "ok".into()]).is_ok());
    }
}
