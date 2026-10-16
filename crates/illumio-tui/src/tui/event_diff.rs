//! An audit event as a readable change diff: a header (when, what, who,
//! which request), one block per resource change and the notifications.
//! Pure: everything comes from the event and the names already loaded
//! ([`Names`]), never a request. The lines are [`DiffLine`]s, so the TUI
//! colours them as the pending diff does and `events --diff` prints them
//! as `policy pending --diff` does. A shape this does not read stays
//! visible as its JSON, never an error. Names are today's: an href reads
//! as what the index calls it now, not at the time of the event.
use crate::tui::diff::{DiffLine, Sign};
use pce::NameIndex;
use pce::models::{Event, IpRange, Ref, RuleSet, ServicePort};
use pce::write::IpRangeSpec;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::HashMap;

/// What turns hrefs into names: the name index (labels, label groups, IP
/// lists, services), the loaded rulesets, and the names the event itself
/// carries (`local`, see [`Names::for_event`]). Any may be missing; an
/// href nothing names stays an href.
#[derive(Clone, Default)]
pub struct Names<'a> {
    pub idx: Option<&'a NameIndex>,
    pub rule_sets: &'a [RuleSet],
    /// Draft href → name, from `{href, name}` / `{href, key, value}` objects in the event.
    pub local: HashMap<String, String>,
}

/// One changed field, as text: a value's `before` and `after` (either
/// absent when it was or became unset), or a list's `added` and
/// `removed` elements. `value` holds a change whose shape is not read,
/// or a note (`(order changed)`, `(empty)`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FieldChange {
    pub field: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// The note of a list that is empty on both sides (`null` → `[]`).
const EMPTY: &str = "(empty)";

impl FieldChange {
    /// Nothing set, or only an empty list: a create or delete leaves it out.
    fn is_empty(&self) -> bool {
        self.before.is_none()
            && self.after.is_none()
            && self.added.is_empty()
            && self.removed.is_empty()
            && self.value.as_deref().is_none_or(|v| v == EMPTY)
    }
}

/// One entry of `resource_changes`: a create, update or delete of one
/// resource. A create or delete without `changes` lists the resource's
/// fields as all after or all before. `raw` is an entry not read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResourceChange {
    pub change_type: String,
    pub resource_type: String,
    /// `rule_set Shop-Prod`, `workload web-01.example.com`, `label app=shop`.
    pub resource: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    pub fields: Vec<FieldChange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

/// One line of the detail: a block title or a diff line (header and
/// notification lines are `Sign::Same`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailLine {
    Title(String),
    Line(DiffLine),
}

/// Fields of a resource that only identify it; not listed as changed.
const IDENTITY: &[&str] = &[
    "href",
    "created_at",
    "updated_at",
    "created_by",
    "updated_by",
];

/// The org id from an event href, `/orgs/5/events/…` → `5`.
fn org_of(event: &Event) -> Option<&str> {
    event.href.strip_prefix("/orgs/")?.split('/').next()
}

/// `{href, name}` or `{href, key, value}` anywhere under `v`, into `out`.
fn learn(v: &Value, out: &mut HashMap<String, String>) {
    match v {
        Value::Array(items) => items.iter().for_each(|i| learn(i, out)),
        Value::Object(o) => {
            let s = |k: &str| o.get(k).and_then(Value::as_str).filter(|v| !v.is_empty());
            let name = match (s("key"), s("value")) {
                (Some(k), Some(v)) => Some(format!("{k}={v}")),
                _ => s("name").map(str::to_string),
            };
            if let (Some(h), Some(n)) = (s("href"), name) {
                out.entry(draft(h)).or_insert(n);
            }
            o.values().for_each(|i| learn(i, out));
        }
        _ => {}
    }
}

impl<'a> Names<'a> {
    /// These names plus the ones `event`'s resources carry (a sec_rule's
    /// `rule_set {href, name}` names its `rule_list` href). Loaded names win.
    pub fn for_event(&self, event: &Event) -> Names<'a> {
        let mut local = self.local.clone();
        for c in &event.resource_changes {
            if let Some(r) = c.get("resource") {
                learn(r, &mut local);
            }
        }
        Names {
            idx: self.idx,
            rule_sets: self.rule_sets,
            local,
        }
    }

    /// The bare name of `href` (`app=shop`, `Prod`, `Corp`), if anything knows it.
    fn base_name(&self, href: &str) -> Option<String> {
        let kind = Ref::new(href).object_type().to_string();
        let idx = self.idx.and_then(|idx| match kind.as_str() {
            "labels" => idx.label(href).map(|l| l.display()),
            "label_groups" => idx.label_group_name(href).map(str::to_string),
            "ip_lists" => idx.ip_list_name(href).map(str::to_string),
            "services" => idx.service_name(href).map(str::to_string),
            _ => None,
        });
        let key = draft(href);
        idx.or_else(|| {
            (kind == "rule_sets")
                .then(|| self.rule_sets.iter().find(|rs| draft(&rs.href) == key))
                .flatten()
                .map(|rs| rs.name.clone())
        })
        .or_else(|| self.local.get(&key).cloned())
    }

    /// A name for `href` by its object type: `app=shop`, `group:Prod`,
    /// `iplist:Corp`, a service or ruleset name; the href when unknown.
    fn href(&self, href: &str) -> String {
        let prefix = match Ref::new(href).object_type() {
            "label_groups" => "group:",
            "ip_lists" => "iplist:",
            _ => "",
        };
        self.base_name(href)
            .map_or_else(|| href.to_string(), |n| format!("{prefix}{n}"))
    }

    /// One value as text. An object reads as the thing it names (an actor,
    /// a port, a range, an href), any other as `key=value, …`.
    fn text(&self, v: &Value) -> String {
        match v {
            Value::Null => "null".into(),
            Value::String(s) => s.clone(),
            Value::Bool(_) | Value::Number(_) => v.to_string(),
            Value::Array(items) if items.is_empty() => "[]".into(),
            Value::Array(items) => items
                .iter()
                .map(|i| self.text(i))
                .collect::<Vec<_>>()
                .join(", "),
            Value::Object(o) => self.object(o).unwrap_or_else(|| self.pairs(o)),
        }
    }

    /// `consumers=workloads, providers=workloads`; a list of more than one
    /// in brackets, `{}` when empty.
    fn pairs(&self, o: &Map<String, Value>) -> String {
        if o.is_empty() {
            return "{}".into();
        }
        o.iter()
            .map(|(k, v)| match v {
                Value::Array(a) if a.len() > 1 => format!("{k}=[{}]", self.text(v)),
                _ => format!("{k}={}", self.text(v)),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn object(&self, o: &Map<String, Value>) -> Option<String> {
        let s = |k: &str| o.get(k).and_then(Value::as_str).filter(|v| !v.is_empty());
        let excl = o.get("exclusion").and_then(Value::as_bool) == Some(true);
        let not = |t: String| if excl { format!("!{t}") } else { t };
        // An actor: `{"label": {...}}`, `{"ip_list": {...}}`, `{"actors": "ams"}`.
        if let Some(a) = s("actors") {
            return Some(if a == "ams" {
                "All Workloads".into()
            } else {
                a.into()
            });
        }
        for (key, prefix) in [
            ("label", ""),
            ("label_group", "group:"),
            ("ip_list", "iplist:"),
            ("workload", "workload:"),
            ("virtual_service", "vs:"),
            ("virtual_server", "vserver:"),
            ("service", ""),
        ] {
            if let Some(inner) = o.get(key).and_then(Value::as_object) {
                let is = |k: &str| {
                    inner
                        .get(k)
                        .and_then(Value::as_str)
                        .filter(|v| !v.is_empty())
                };
                let href = is("href");
                // The name the event carries wins over the index.
                let own = match (is("key"), is("value")) {
                    (Some(k), Some(v)) => Some(format!("{k}={v}")),
                    _ => is("name").or_else(|| is("hostname")).map(str::to_string),
                };
                let Some(name) = own
                    .or_else(|| href.and_then(|h| self.base_name(h)))
                    .or(href.map(str::to_string))
                else {
                    continue;
                };
                return Some(not(format!("{prefix}{name}")));
            }
        }
        if let (Some(k), Some(v)) = (s("key"), s("value")) {
            return Some(format!("{k}={v}"));
        }
        if o.contains_key("from_ip") {
            let r: IpRange = serde_json::from_value(Value::Object(o.clone())).ok()?;
            let mut t = IpRangeSpec::from_range(&r).display();
            if let Some(d) = s("description") {
                t.push_str(&format!(" ({d})"));
            }
            return Some(t);
        }
        if o.contains_key("proto") || o.contains_key("port") {
            let p: ServicePort = serde_json::from_value(Value::Object(o.clone())).ok()?;
            // `display` leaves out the ICMP type and code and a Windows service's names.
            let mut t = p.display();
            for (k, label) in [("icmp_type", "type"), ("icmp_code", "code")] {
                if let Some(n) = o.get(k).filter(|v| !v.is_null()) {
                    t.push_str(&format!(" {label} {n}"));
                }
            }
            for k in ["service_name", "process_name"] {
                if let Some(n) = s(k) {
                    t.push_str(&format!(" {n}"));
                }
            }
            return Some(t);
        }
        // A workload interface: `eth0 10.0.0.5`.
        if let Some(addr) = s("address") {
            return Some(match s("name") {
                Some(n) => format!("{n} {addr}"),
                None => addr.to_string(),
            });
        }
        if let Some(n) = s("hostname").or_else(|| s("name")) {
            return Some(n.to_string());
        }
        s("href").map(|h| self.href(h))
    }

    /// An element's identity across before and after: its href (or the
    /// href of the object it wraps) with its exclusion flag, else the
    /// value itself.
    fn identity(v: &Value) -> String {
        let href = |v: &Value| v.get("href").and_then(Value::as_str).map(str::to_string);
        let excl = v.get("exclusion").and_then(Value::as_bool) == Some(true);
        href(v)
            .or_else(|| {
                v.as_object()?
                    .values()
                    .find_map(|inner| inner.is_object().then(|| href(inner)).flatten())
            })
            .map(|h| format!("{}{}", if excl { "!" } else { "" }, draft(&h)))
            .unwrap_or_else(|| v.to_string())
    }
}

/// The draft form of a policy href: active and draft differ only there.
fn draft(href: &str) -> String {
    href.replacen("/sec_policy/active/", "/sec_policy/draft/", 1)
}

/// The resource's one key and its object, `("rule_set", {...})`.
fn resource_of(change: &Map<String, Value>) -> Option<(&String, &Map<String, Value>)> {
    let (kind, v) = change.get("resource")?.as_object()?.iter().next()?;
    Some((kind, v.as_object()?))
}

/// `label app=shop`, `workload web-01`, `sec_rule 7 in Shop-Prod`,
/// `rule_set Shop-Prod`; the kind alone without a name.
fn resource_text(kind: &str, r: &Map<String, Value>, names: &Names) -> String {
    let s = |k: &str| r.get(k).and_then(Value::as_str).filter(|v| !v.is_empty());
    let href = s("href");
    let name = match kind {
        "label" => s("key").zip(s("value")).map(|(k, v)| format!("{k}={v}")),
        "sec_rule" | "deny_rule" => href.map(|h| {
            let id = Ref::new(h).id().to_string();
            match r.get("rule_set").map(|rs| names.text(rs)) {
                Some(rs) => format!("{id} in {rs}"),
                None => id,
            }
        }),
        _ => None,
    }
    .or_else(|| s("hostname").or_else(|| s("name")).map(str::to_string))
    .or_else(|| {
        href.map(|h| {
            let n = names.href(h);
            if n == h {
                Ref::new(h).id().to_string()
            } else {
                n
            }
        })
    });
    match name {
        Some(n) => format!("{kind} {n}"),
        None => kind.to_string(),
    }
}

/// Elements whose text is on both sides (a lossy text, e.g. a range whose
/// only change is not shown) read as their JSON instead, so a real change
/// never looks like a no-op.
fn distinct(
    added: Vec<(String, &Value)>,
    removed: Vec<(String, &Value)>,
) -> (Vec<String>, Vec<String>) {
    let clash = |t: &String, other: &[(String, &Value)]| other.iter().any(|(o, _)| o == t);
    let fix = |side: &[(String, &Value)], other: &[(String, &Value)]| {
        side.iter()
            .map(|(t, v)| {
                if clash(t, other) {
                    v.to_string()
                } else {
                    t.clone()
                }
            })
            .collect()
    };
    (fix(&added, &removed), fix(&removed, &added))
}

/// One `changes` entry: `{before, after}` (lists diffed by element) or
/// `{created, deleted}` (a list's added and removed elements).
fn field_change(field: &str, entry: &Value, names: &Names, org: Option<&str>) -> FieldChange {
    let mut fc = FieldChange {
        field: field.to_string(),
        ..FieldChange::default()
    };
    // `label_ids_array` holds label ids, not hrefs: named through the org's label hrefs.
    let text = |v: &Value| match (field, v, org) {
        ("label_ids_array", Value::Number(n), Some(org)) => {
            names.href(&format!("/orgs/{org}/labels/{n}"))
        }
        _ => element(field, v, names),
    };
    let Some(o) = entry.as_object() else {
        fc.value = Some(names.text(entry));
        return fc;
    };
    let known = o
        .keys()
        .all(|k| matches!(k.as_str(), "before" | "after" | "created" | "deleted"));
    if !known {
        fc.value = Some(entry.to_string());
        return fc;
    }
    let empty = Vec::new();
    let list = |k: &str| o.get(k).and_then(Value::as_array).unwrap_or(&empty);
    let before = o.get("before").filter(|v| !v.is_null());
    let after = o.get("after").filter(|v| !v.is_null());
    let listy = |v: Option<&Value>| v.is_none_or(Value::is_array);
    let (mut added, mut removed) = (Vec::new(), Vec::new());
    if (before.is_some() || after.is_some()) && listy(before) && listy(after) {
        let (b, a) = (list("before"), list("after"));
        let ids = |l: &[Value]| l.iter().map(Names::identity).collect::<Vec<_>>();
        let (bi, ai) = (ids(b), ids(a));
        added.extend(
            a.iter()
                .zip(&ai)
                .filter(|(_, id)| !bi.contains(id))
                .map(|(v, _)| v),
        );
        removed.extend(
            b.iter()
                .zip(&bi)
                .filter(|(_, id)| !ai.contains(id))
                .map(|(v, _)| v),
        );
        if added.is_empty() && removed.is_empty() {
            fc.value = Some(if bi != ai && !ai.is_empty() {
                "(order changed)".into()
            } else {
                EMPTY.into()
            });
        }
    } else {
        fc.before = before.map(text);
        fc.after = after.map(text);
    }
    added.extend(list("created"));
    removed.extend(list("deleted"));
    (fc.added, fc.removed) = distinct(
        added.into_iter().map(|v| (text(v), v)).collect(),
        removed.into_iter().map(|v| (text(v), v)).collect(),
    );
    fc
}

/// The resource's own fields as all after (a create) or all before (a
/// delete): what a create or delete without `changes` carries.
fn resource_fields(r: &Map<String, Value>, created: bool, names: &Names) -> Vec<FieldChange> {
    r.iter()
        .filter(|(k, v)| !IDENTITY.contains(&k.as_str()) && !v.is_null())
        .filter_map(|(k, v)| {
            let mut fc = FieldChange {
                field: k.clone(),
                ..FieldChange::default()
            };
            match v {
                Value::Array(items) if items.is_empty() => return None,
                Value::Array(items) => {
                    let texts = items.iter().map(|i| element(k, i, names)).collect();
                    if created {
                        fc.added = texts;
                    } else {
                        fc.removed = texts;
                    }
                }
                _ if created => fc.after = Some(names.text(v)),
                _ => fc.before = Some(names.text(v)),
            }
            Some(fc)
        })
        .collect()
}

/// A list element; an empty scope reads as `All` (every workload).
fn element(field: &str, v: &Value, names: &Names) -> String {
    match (field, v) {
        ("scopes", Value::Array(a)) if a.is_empty() => "All".into(),
        _ => names.text(v),
    }
}

/// Every resource change of `event`, read.
pub fn resource_changes(event: &Event, names: &Names) -> Vec<ResourceChange> {
    let names = &names.for_event(event);
    let org = org_of(event);
    event
        .resource_changes
        .iter()
        .map(|c| {
            let Some(o) = c.as_object() else {
                return unread(c);
            };
            let change_type = o
                .get("change_type")
                .and_then(Value::as_str)
                .unwrap_or("change")
                .to_string();
            let (kind, res) = match resource_of(o) {
                Some((k, r)) => (k.clone(), Some(r)),
                None => ("resource".to_string(), None),
            };
            let resource = res.map_or_else(|| kind.clone(), |r| resource_text(&kind, r, names));
            let href = res
                .and_then(|r| r.get("href"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let mut fields: Vec<FieldChange> = match o.get("changes") {
                Some(Value::Object(ch)) if !ch.is_empty() => ch
                    .iter()
                    .map(|(k, v)| field_change(k, v, names, org))
                    .collect(),
                None | Some(Value::Null) | Some(Value::Object(_)) => {
                    match (change_type.as_str(), res) {
                        ("create", Some(r)) => resource_fields(r, true, names),
                        ("delete", Some(r)) => resource_fields(r, false, names),
                        _ => Vec::new(),
                    }
                }
                Some(_) => return unread(c),
            };
            // A create or delete lists what the object holds: unset and empty fields say nothing.
            if matches!(change_type.as_str(), "create" | "delete") {
                fields.retain(|f| !f.is_empty());
            }
            ResourceChange {
                change_type,
                resource_type: kind,
                resource,
                href,
                fields,
                raw: None,
            }
        })
        .collect()
}

fn unread(c: &Value) -> ResourceChange {
    ResourceChange {
        change_type: "change".into(),
        resource_type: "resource".into(),
        resource: "resource".into(),
        href: None,
        fields: Vec::new(),
        raw: Some(c.to_string()),
    }
}

/// `update rule_set Shop-Prod (2 fields)`, ` +1` per further change;
/// empty without resource changes.
pub fn summary(changes: &[ResourceChange]) -> String {
    let Some(first) = changes.first() else {
        return String::new();
    };
    let mut s = format!("{} {}", first.change_type, first.resource);
    match first.fields.len() {
        0 => {}
        1 => s.push_str(" (1 field)"),
        n => s.push_str(&format!(" ({n} fields)")),
    }
    if changes.len() > 1 {
        s.push_str(&format!(" +{}", changes.len() - 1));
    }
    s
}

/// A field's lines, each `field: value`: `-` before and `+` after, one
/// `-`/`+` per list element, unsigned for a note or a shape not read.
pub fn field_lines(fc: &FieldChange) -> Vec<DiffLine> {
    let line = |sign, v: &str| DiffLine {
        sign,
        text: format!("{}: {v}", fc.field),
    };
    let mut out = Vec::new();
    out.extend(fc.before.iter().map(|b| line(Sign::Removed, b)));
    out.extend(fc.after.iter().map(|a| line(Sign::Added, a)));
    out.extend(fc.removed.iter().map(|r| line(Sign::Removed, r)));
    out.extend(fc.added.iter().map(|a| line(Sign::Added, a)));
    out.extend(fc.value.iter().map(|v| line(Sign::Same, v)));
    if out.is_empty() {
        // `{"before": null, "after": null}`: unset on both sides.
        out.push(line(Sign::Same, "(unset)"));
    }
    out
}

/// The diff lines of every resource change (no header).
pub fn change_lines(changes: &[ResourceChange]) -> Vec<DetailLine> {
    let mut out = Vec::new();
    for c in changes {
        out.push(DetailLine::Title(format!(
            "{} {}",
            c.change_type, c.resource
        )));
        if let Some(raw) = &c.raw {
            out.push(same(raw.clone()));
        }
        if c.fields.is_empty() && c.raw.is_none() {
            out.push(same("(no fields recorded)".into()));
        }
        out.extend(c.fields.iter().flat_map(field_lines).map(DetailLine::Line));
    }
    out
}

fn same(text: String) -> DetailLine {
    DetailLine::Line(DiffLine {
        sign: Sign::Same,
        text,
    })
}

/// A notification: its type, then one `key  value` line per info field.
fn notification_lines(n: &Value, names: &Names) -> Vec<DetailLine> {
    let kind = n
        .get("notification_type")
        .and_then(Value::as_str)
        .unwrap_or("notification");
    let mut out = vec![same(kind.to_string())];
    if let Some(info) = n.get("info").and_then(Value::as_object) {
        out.extend(
            info.iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| same(format!("  {k:<13}{}", names.text(v)))),
        );
    }
    out
}

fn action(event: &Event) -> Option<&Map<String, Value>> {
    event.extra.get("action").and_then(Value::as_object)
}

/// A failed request's `action.errors`, as red `error  token: message` lines.
fn error_lines(event: &Event, names: &Names) -> Vec<DetailLine> {
    action(event)
        .and_then(|a| a.get("errors"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|e| {
            let msg = e.get("message").and_then(Value::as_str).unwrap_or("");
            let text = match e.get("token").and_then(Value::as_str) {
                Some(t) if !msg.is_empty() => format!("{t}: {msg}"),
                Some(t) => t.to_string(),
                None => names.text(e),
            };
            DetailLine::Line(DiffLine {
                sign: Sign::Removed,
                text: format!("error    {text}"),
            })
        })
        .collect()
}

/// What happened, without the header: errors, the change blocks and the
/// notifications; what `events --diff` prints under each row.
pub fn body_lines(event: &Event, changes: &[ResourceChange], names: &Names) -> Vec<DetailLine> {
    let mut out = error_lines(event, names);
    out.extend(change_lines(changes));
    if !event.notifications.is_empty() {
        out.push(DetailLine::Title("notifications".into()));
        out.extend(
            event
                .notifications
                .iter()
                .flat_map(|n| notification_lines(n, names)),
        );
    }
    out
}

/// The whole detail: header, then [`body_lines`]. An event with nothing
/// in its body adds its JSON, so nothing is hidden.
pub fn detail_lines(event: &Event, names: &Names) -> Vec<DetailLine> {
    let names = &names.for_event(event);
    let changes = resource_changes(event, names);
    let mut out = Vec::new();
    let mut head = |k: &str, v: String| {
        if !v.is_empty() {
            out.push(same(format!("{k:<9}{v}")));
        }
    };
    head("time", event.timestamp.clone().unwrap_or_default());
    head("type", event.event_type.clone().unwrap_or_default());
    let sev = event.severity.clone().unwrap_or_default();
    let status = event.status.clone().unwrap_or_default();
    head(
        "severity",
        match (sev.is_empty(), status.is_empty()) {
            (_, true) => sev,
            (true, false) => format!("status {status}"),
            (false, false) => format!("{sev} · {status}"),
        },
    );
    head("actor", event.actor());
    let a = |k: &str| {
        action(event)
            .and_then(|a| a.get(k))
            .filter(|v| !v.is_null())
    };
    // The status code goes with the source address: a long endpoint is cut at 80 columns.
    let code = a("http_status_code").map(|c| format!("HTTP {c}"));
    let ip = a("src_ip").and_then(Value::as_str);
    if let Some(ep) = a("api_endpoint").and_then(Value::as_str) {
        let method = a("api_method").and_then(Value::as_str).unwrap_or("");
        head("request", format!("{method} {ep}").trim_start().to_string());
    }
    head(
        "from",
        match (ip, code) {
            (Some(ip), Some(c)) => format!("{ip} ({c})"),
            (Some(ip), None) => ip.to_string(),
            (None, Some(c)) => c,
            (None, None) => String::new(),
        },
    );
    // Which API key: often a shared service account's.
    let key = a("info")
        .and_then(|i| i.get("api_key_id"))
        .and_then(Value::as_str);
    head("api key", key.unwrap_or_default().to_string());
    // One change has its title right below; the summary only adds for several.
    if changes.len() > 1 {
        head("changes", summary(&changes));
    }
    let body = body_lines(event, &changes, names);
    let bare = body.is_empty();
    out.extend(body);
    if bare {
        out.push(DetailLine::Title("event (no changes recorded)".into()));
        let json = serde_json::to_string_pretty(event).unwrap_or_default();
        out.extend(json.lines().map(|l| same(l.to_string())));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::fixture_index;
    use serde_json::json;

    fn event(v: Value) -> Event {
        serde_json::from_value(v).unwrap()
    }

    fn rule_sets() -> Vec<RuleSet> {
        serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap()
    }

    /// The lines as `+ text`, `- text`, `  text`, `# title`.
    fn plain(lines: &[DetailLine]) -> Vec<String> {
        lines
            .iter()
            .map(|l| match l {
                DetailLine::Title(t) => format!("# {t}"),
                DetailLine::Line(d) => {
                    let g = match d.sign {
                        Sign::Added => "+",
                        Sign::Removed => "-",
                        Sign::Same => " ",
                    };
                    format!("{g} {}", d.text)
                }
            })
            .collect()
    }

    fn sec_rule_update() -> Event {
        event(json!({
            "href": "/orgs/5/events/aa-1",
            "timestamp": "2026-10-10T07:40:00.000Z",
            "event_type": "sec_rule.update",
            "severity": "info",
            "status": "success",
            "created_by": {"user": {"href": "/users/1", "username": "ops@example.com"}},
            "notifications": [],
            "resource_changes": [{
                "change_type": "update",
                "changes": {
                    "providers": {
                        "created": [{"label": {"href": "/orgs/5/labels/2"}}],
                        "deleted": [{"label": {"href": "/orgs/5/labels/1"}}]
                    },
                    "consumers": {
                        "before": [{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}],
                        "after": [
                            {"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}},
                            {"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/10"}},
                            {"actors": "ams"}
                        ]
                    },
                    "enabled": {"before": true, "after": false}
                },
                "resource": {"sec_rule": {
                    "href": "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/7",
                    "rule_set": {"href": "/orgs/5/sec_policy/draft/rule_sets/100", "name": "Shop-Prod"},
                    "providers": [{"label": {"href": "/orgs/5/labels/2"}}]
                }},
                "uuid": "u-1"
            }],
            "action": {
                "api_endpoint": "/api/v2/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/7",
                "api_method": "PUT",
                "http_status_code": 204,
                "src_ip": "192.0.2.10"
            }
        }))
    }

    #[test]
    fn update_lists_only_changed_elements_by_name() {
        let idx = fixture_index();
        let names = Names {
            idx: Some(&idx),
            ..Names::default()
        };
        let e = sec_rule_update();
        let c = resource_changes(&e, &names);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].resource, "sec_rule 7 in Shop-Prod");
        assert_eq!(summary(&c), "update sec_rule 7 in Shop-Prod (3 fields)");
        assert_eq!(
            plain(&change_lines(&c)),
            [
                "# update sec_rule 7 in Shop-Prod",
                "+ consumers: group:All prod envs",
                "+ consumers: All Workloads",
                "- enabled: true",
                "+ enabled: false",
                "- providers: role=web",
                "+ providers: role=db",
            ]
        );
        let head = plain(&detail_lines(&e, &names));
        assert!(
            head.contains(&"  actor    user:ops@example.com".to_string()),
            "{head:?}"
        );
        assert!(
            head.contains(
                &"  request  PUT /api/v2/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/7"
                    .to_string()
            ),
            "{head:?}"
        );
        assert!(
            head.contains(&"  from     192.0.2.10 (HTTP 204)".to_string()),
            "{head:?}"
        );
        assert!(
            head.contains(&"  severity info · success".to_string()),
            "{head:?}"
        );
    }

    #[test]
    fn without_an_index_hrefs_stay_hrefs() {
        let c = resource_changes(&sec_rule_update(), &Names::default());
        let lines = plain(&change_lines(&c));
        assert!(
            lines.contains(&"+ providers: /orgs/5/labels/2".to_string()),
            "{lines:?}"
        );
    }

    #[test]
    fn create_and_delete_without_changes_list_the_resource() {
        let idx = fixture_index();
        let names = Names {
            idx: Some(&idx),
            ..Names::default()
        };
        let e = event(json!({
            "href": "/orgs/5/events/aa-2",
            "event_type": "rule_set.delete",
            "resource_changes": [{
                "change_type": "delete",
                "changes": null,
                "resource": {"rule_set": {
                    "href": "/orgs/5/sec_policy/draft/rule_sets/100",
                    "name": "Shop-Prod",
                    "scopes": [[{"label": {"href": "/orgs/5/labels/3"}}, {"label": {"href": "/orgs/5/labels/4"}}], []]
                }}
            }]
        }));
        assert_eq!(
            plain(&change_lines(&resource_changes(&e, &names))),
            [
                "# delete rule_set Shop-Prod",
                "- name: Shop-Prod",
                "- scopes: app=shop, env=prod",
                "- scopes: All",
            ]
        );
    }

    #[test]
    fn workload_labels_by_href_and_by_id() {
        let idx = fixture_index();
        let names = Names {
            idx: Some(&idx),
            ..Names::default()
        };
        let e = event(json!({
            "href": "/orgs/5/events/aa-3",
            "event_type": "workload.update",
            "resource_changes": [{
                "change_type": "update",
                "changes": {
                    "labels": {"created": [{"href": "/orgs/5/labels/2"}], "deleted": [{"href": "/orgs/5/labels/1"}]},
                    "label_ids_array": {"before": [1, 3], "after": [2, 3]}
                },
                "resource": {"workload": {"href": "/orgs/5/workloads/w-1", "hostname": "web-01.example.com", "name": "web-01"}}
            }]
        }));
        assert_eq!(
            plain(&change_lines(&resource_changes(&e, &names))),
            [
                "# update workload web-01.example.com",
                "- label_ids_array: role=web",
                "+ label_ids_array: role=db",
                "- labels: role=web",
                "+ labels: role=db",
            ]
        );
    }

    #[test]
    fn ruleset_names_come_from_loaded_rulesets() {
        let sets = rule_sets();
        let names = Names {
            idx: None,
            rule_sets: &sets,
            ..Names::default()
        };
        let e = event(json!({
            "href": "/orgs/5/events/aa-4",
            "resource_changes": [{
                "change_type": "create",
                "changes": {"rule_list": {"before": null, "after": {"href": "/orgs/5/sec_policy/draft/rule_sets/101"}}},
                "resource": {"sec_rule": {"href": "/orgs/5/sec_policy/draft/rule_sets/101/sec_rules/9"}}
            }]
        }));
        let lines = plain(&change_lines(&resource_changes(&e, &names)));
        assert_eq!(lines[1], "+ rule_list: Global-Infra", "{lines:?}");
    }

    #[test]
    fn ports_ranges_and_interfaces_read_as_text() {
        let names = Names::default();
        let e = event(json!({
            "href": "/orgs/5/events/aa-5",
            "resource_changes": [{
                "change_type": "update",
                "changes": {
                    "service_ports": {"created": [{"port": 8443, "proto": 6}], "deleted": [{"port": 8000, "to_port": 8099, "proto": 6}]},
                    "ip_ranges": {"created": [{"from_ip": "10.2.0.0/16", "exclusion": false}]},
                    "workload_interfaces": {"created": [{"name": "eth0", "address": "192.0.2.5"}]}
                },
                "resource": {"service": {"href": "/orgs/5/sec_policy/draft/services/32", "name": "Web"}}
            }]
        }));
        let lines = plain(&change_lines(&resource_changes(&e, &names)));
        assert_eq!(
            lines,
            [
                "# update service Web",
                "+ ip_ranges: 10.2.0.0/16",
                "- service_ports: 8000-8099/tcp",
                "+ service_ports: 8443/tcp",
                "+ workload_interfaces: eth0 192.0.2.5",
            ]
        );
    }

    #[test]
    fn notifications_and_errors_read_as_lines() {
        let names = Names::default();
        let e = event(json!({
            "href": "/orgs/5/events/aa-6",
            "event_type": "request.authorization_failed",
            "severity": "err",
            "status": "failure",
            "created_by": {"user": {"href": "/users/9", "username": "svc@example.com"}},
            "notifications": [{
                "notification_type": "request.authorization_failed",
                "info": {"api_endpoint": "/api/v2/orgs/5/sec_policy", "api_method": "POST", "src_ip": "198.51.100.7"},
                "uuid": "n-1"
            }],
            "resource_changes": [],
            "action": {"api_endpoint": "/api/v2/orgs/5/x", "api_method": "POST", "errors": [{"message": "Not allowed", "token": "forbidden"}]}
        }));
        let lines = plain(&detail_lines(&e, &names));
        assert!(
            lines.contains(&"- error    forbidden: Not allowed".to_string()),
            "{lines:?}"
        );
        assert!(lines.contains(&"# notifications".to_string()), "{lines:?}");
        let at = lines
            .iter()
            .position(|l| l == "  request.authorization_failed")
            .expect("notification type");
        assert_eq!(
            lines[at + 1..at + 4],
            [
                "    api_endpoint /api/v2/orgs/5/sec_policy",
                "    api_method   POST",
                "    src_ip       198.51.100.7",
            ]
        );
        assert!(
            !lines.iter().any(|l| l.contains("\"href\"")),
            "no JSON fallback: {lines:?}"
        );
    }

    #[test]
    fn unknown_shapes_fall_back_to_json() {
        let names = Names::default();
        let e = event(json!({
            "href": "/orgs/5/events/aa-7",
            "event_type": "system.license_expiring",
            "resource_changes": [
                "not an object",
                {"change_type": "update", "changes": [1, 2], "resource": {"x": {}}},
                {"change_type": "update", "changes": {"odd": {"updated": [1]}, "scalar": 5}, "resource": {"org_setting": {"format": "x"}}}
            ]
        }));
        let lines = plain(&change_lines(&resource_changes(&e, &names)));
        assert_eq!(
            lines,
            [
                "# change resource",
                "  \"not an object\"",
                "# change resource",
                "  {\"change_type\":\"update\",\"changes\":[1,2],\"resource\":{\"x\":{}}}",
                "# update org_setting",
                "  odd: {\"updated\":[1]}",
                "  scalar: 5",
            ]
        );
        // Nothing readable at all: the event's JSON follows the header.
        let bare = event(json!({"href": "/orgs/5/events/aa-8", "event_type": "system.x"}));
        let lines = plain(&detail_lines(&bare, &names));
        assert!(
            lines.contains(&"# event (no changes recorded)".to_string()),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains("\"event_type\": \"system.x\"")),
            "{lines:?}"
        );
    }

    #[test]
    fn create_leaves_out_empty_fields_and_update_names_reorders() {
        let idx = fixture_index();
        let names = Names {
            idx: Some(&idx),
            ..Names::default()
        };
        let created = event(json!({
            "href": "/orgs/5/events/b-1",
            "resource_changes": [{
                "change_type": "create",
                "changes": {
                    "label_ids_array": {"after": [], "before": null},
                    "server_roles": {"after": [], "before": null},
                    "name": {"after": "web-02", "before": null},
                    "network_type": {"after": 2, "before": null},
                    "description": {"after": null, "before": null}
                },
                "resource": {"workload": {"href": "/orgs/5/workloads/w-2", "name": "web-02"}}
            }]
        }));
        assert_eq!(
            plain(&change_lines(&resource_changes(&created, &names))),
            [
                "# create workload web-02",
                "+ name: web-02",
                "+ network_type: 2"
            ]
        );
        let updated = event(json!({
            "href": "/orgs/5/events/b-2",
            "resource_changes": [{
                "change_type": "update",
                "changes": {
                    "labels": {"before": [{"href": "/orgs/5/labels/1"}, {"href": "/orgs/5/labels/3"}],
                               "after": [{"href": "/orgs/5/labels/3"}, {"href": "/orgs/5/labels/1"}]},
                    "server_roles": {"after": [], "before": null},
                    "kind": {"before": "x", "after": ["y"]}
                },
                "resource": {"workload": {"href": "/orgs/5/workloads/w-2", "name": "web-02"}}
            }]
        }));
        assert_eq!(
            plain(&change_lines(&resource_changes(&updated, &names))),
            [
                "# update workload web-02",
                "- kind: x",
                "+ kind: y",
                "  labels: (order changed)",
                "  server_roles: (empty)",
            ]
        );
    }

    #[test]
    fn exclusion_flips_and_lossy_texts_stay_visible() {
        let idx = fixture_index();
        let names = Names {
            idx: Some(&idx),
            ..Names::default()
        };
        let e = event(json!({
            "href": "/orgs/5/events/b-3",
            "resource_changes": [{
                "change_type": "update",
                "changes": {
                    "providers": {"before": [{"label": {"href": "/orgs/5/labels/1"}}],
                                  "after": [{"label": {"href": "/orgs/5/labels/1"}, "exclusion": true}]},
                    "service_ports": {"deleted": [{"port": 445, "proto": 6, "process_name": "svchost.exe"}, {"proto": 1, "icmp_type": 8}],
                                      "created": [{"port": 445, "proto": 6, "process_name": "lsass.exe"}, {"proto": 1, "icmp_type": 0}]},
                    "ip_ranges": {"deleted": [{"from_ip": "10.0.0.0/8", "tag": "a"}],
                                  "created": [{"from_ip": "10.0.0.0/8", "tag": "b"}]}
                },
                "resource": {"service": {"href": "/orgs/5/sec_policy/draft/services/31", "name": "PostgreSQL"}}
            }]
        }));
        assert_eq!(
            plain(&change_lines(&resource_changes(&e, &names))),
            [
                "# update service PostgreSQL",
                "- ip_ranges: {\"from_ip\":\"10.0.0.0/8\",\"tag\":\"a\"}",
                "+ ip_ranges: {\"from_ip\":\"10.0.0.0/8\",\"tag\":\"b\"}",
                "- providers: role=web",
                "+ providers: !role=web",
                "- service_ports: 445/tcp svchost.exe",
                "- service_ports: icmp type 8",
                "+ service_ports: 445/tcp lsass.exe",
                "+ service_ports: icmp type 0",
            ]
        );
    }

    #[test]
    fn nested_objects_and_the_events_own_names_read_as_text() {
        // No index and no rulesets loaded: the rule's ruleset name comes from the event.
        let names = Names::default();
        let e = event(json!({
            "href": "/orgs/5/events/b-4",
            "created_by": {"service_account": {"name": "ci-bot"}},
            "resource_changes": [{
                "change_type": "create",
                "changes": {
                    "resolve_labels": {"after": {"consumers": ["workloads"], "providers": ["workloads", "virtual_services"]}, "before": null},
                    "rule_list": {"after": {"href": "/orgs/5/sec_policy/draft/rule_sets/100"}, "before": null}
                },
                "resource": {"sec_rule": {
                    "href": "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9",
                    "rule_set": {"href": "/orgs/5/sec_policy/draft/rule_sets/100", "name": "Shop-Prod"}
                }}
            }],
            "action": {"api_endpoint": "/api/v2/x", "api_method": "POST", "info": {"api_key_id": "k-9"}}
        }));
        let lines = plain(&detail_lines(&e, &names));
        assert_eq!(
            lines[lines.len() - 3..],
            [
                "# create sec_rule 9 in Shop-Prod",
                "+ resolve_labels: consumers=workloads, providers=[workloads, virtual_services]",
                "+ rule_list: Shop-Prod",
            ]
        );
        assert!(
            lines.contains(&"  actor    service_account:ci-bot".to_string()),
            "{lines:?}"
        );
        assert!(lines.contains(&"  api key  k-9".to_string()), "{lines:?}");
        assert!(
            !lines.iter().any(|l| l.contains("changes ")),
            "one change, no summary: {lines:?}"
        );
    }
}
