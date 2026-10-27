//! What changed between two snapshots: per section, the objects added,
//! removed and changed (matched by href), with the changed fields. The
//! rulesets, IP lists, label groups and services go through the policy
//! diff's own line diffs (`tui::diff`), so a rule reads as it does in
//! `policy pending --diff`; labels, boundaries and workloads have theirs
//! here. Names come from both snapshots' lists (the newer first), so an
//! object gone from one still has its name.
use super::{Section, Snapshot};
use crate::tui::diff::{
    DenySide, DiffLine, Sign, diff_ip_list, diff_label_group, diff_rule_set_with, diff_service,
    entries, field, inlines_deny,
};
use crate::tui::edit::{actor_tokens, service_tokens};
use pce::models::{EnforcementBoundary, IpList, Label, LabelGroup, RuleSet, Service, Workload};
use pce::{KeyOrder, NameIndex};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// The sections `diff` compares, as `--section` names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, clap::ValueEnum)]
pub enum DiffSection {
    Rulesets,
    Labels,
    LabelGroups,
    Iplists,
    Services,
    Boundaries,
    Workloads,
}

impl DiffSection {
    pub const ALL: [DiffSection; 7] = [
        DiffSection::Rulesets,
        DiffSection::Labels,
        DiffSection::LabelGroups,
        DiffSection::Iplists,
        DiffSection::Services,
        DiffSection::Boundaries,
        DiffSection::Workloads,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            DiffSection::Rulesets => "rulesets",
            DiffSection::Labels => "labels",
            DiffSection::LabelGroups => "label-groups",
            DiffSection::Iplists => "iplists",
            DiffSection::Services => "services",
            DiffSection::Boundaries => "boundaries",
            DiffSection::Workloads => "workloads",
        }
    }

    /// The snapshot section that holds it.
    pub fn source(self) -> Section {
        match self {
            DiffSection::Labels => Section::Labels,
            DiffSection::Workloads => Section::Workloads,
            _ => Section::Policy,
        }
    }
}

/// One side of the diff.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct End {
    /// The snapshot's id, or `now` for a live read.
    pub id: String,
    pub taken_at: jiff::Timestamp,
    pub label: Option<String>,
}

/// One section's counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Count {
    pub section: &'static str,
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
}

/// One object added, removed or changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    pub section: &'static str,
    /// `added`, `removed` or `changed`.
    pub change: &'static str,
    /// `draft` or `active` for a ruleset; `None` elsewhere.
    pub scope: Option<&'static str>,
    pub name: String,
    pub href: String,
    /// The fields that changed (`labels`, `enforcement_mode`, `rules` …);
    /// empty when added or removed.
    pub fields: Vec<String>,
    /// The changed lines, `-` before and `+` after (`+label env=prod`,
    /// `-enforcement_mode: visibility_only`).
    pub details: Vec<String>,
}

/// A section asked for that is not in both snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NotCompared {
    pub section: &'static str,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SnapshotDiff {
    pub from: End,
    pub to: End,
    /// Anything differs in the sections compared.
    pub changed: bool,
    pub sections: Vec<Count>,
    pub changes: Vec<Change>,
    pub not_compared: Vec<NotCompared>,
}

/// Refuses two snapshots of different profiles, PCEs or orgs, naming both.
pub fn check_same(a: &Snapshot, a_id: &str, b: &Snapshot, b_id: &str) -> Result<(), String> {
    let who = |s: &Snapshot| format!("profile {:?}, PCE {} org {}", s.profile, s.pce, s.org);
    if a.profile != b.profile || a.pce != b.pce || a.org != b.org {
        return Err(format!(
            "snapshots of different PCEs are never compared: {a_id} is {}, {b_id} is {}",
            who(a),
            who(b)
        ));
    }
    Ok(())
}

/// Every object of each list by href, the newer first.
fn union<T: Clone>(newer: Option<&[T]>, older: Option<&[T]>, href: fn(&T) -> &str) -> Vec<T> {
    let newer = newer.unwrap_or_default();
    let seen: HashSet<&str> = newer.iter().map(href).collect();
    let mut out: Vec<T> = newer.to_vec();
    out.extend(
        older
            .unwrap_or_default()
            .iter()
            .filter(|o| !seen.contains(href(o)))
            .cloned(),
    );
    out
}

/// The names both snapshots know, the newer's first.
fn index(a: &Snapshot, b: &Snapshot, order: &KeyOrder) -> NameIndex {
    let (pa, pb) = (a.sections.policy.as_ref(), b.sections.policy.as_ref());
    let labels = union(
        b.sections.labels.as_deref(),
        a.sections.labels.as_deref(),
        |l| &l.href,
    );
    let groups = union(
        pb.map(|p| p.label_groups.as_slice()),
        pa.map(|p| p.label_groups.as_slice()),
        |g| &g.href,
    );
    let lists = union(
        pb.map(|p| p.ip_lists.as_slice()),
        pa.map(|p| p.ip_lists.as_slice()),
        |l| &l.href,
    );
    let services = union(
        pb.map(|p| p.services.as_slice()),
        pa.map(|p| p.services.as_slice()),
        |s| &s.href,
    );
    NameIndex::new(&labels, &groups, &lists, &services).with_key_order(order.clone())
}

/// The field a changed line is about: `name: …` names it, `label …` is
/// `labels` (and so on for the list entries); any other line is a rule.
fn field_of(text: &str) -> String {
    if let Some((f, _)) = text.split_once(": ")
        && !f.contains(' ')
    {
        return f.to_string();
    }
    let first = text.split(' ').next().unwrap_or_default();
    match first {
        "range" => "ranges",
        "fqdn" => "fqdns",
        "label" => "labels",
        "sub-group" => "sub_groups",
        "port" => "ports",
        "provider" => "providers",
        "consumer" => "consumers",
        "service" => "services",
        _ => "rules",
    }
    .to_string()
}

/// The changed lines of a line diff as `details`, and the fields they name.
fn changed_lines(lines: &[DiffLine]) -> (Vec<String>, Vec<String>) {
    let mut fields: Vec<String> = Vec::new();
    let mut details = Vec::new();
    for l in lines {
        let sign = match l.sign {
            Sign::Same => continue,
            Sign::Added => '+',
            Sign::Removed => '-',
        };
        let f = field_of(&l.text);
        if !fields.contains(&f) {
            fields.push(f);
        }
        details.push(format!("{sign}{}", l.text));
    }
    (fields, details)
}

/// How to compare one kind of object.
struct Kind<'a, T> {
    section: &'static str,
    scope: Option<&'static str>,
    href: fn(&T) -> &str,
    name: &'a dyn Fn(&T) -> String,
    lines: &'a dyn Fn(&T, &T) -> Vec<DiffLine>,
}

/// `older` against `newer` by href (each side keyed once: O(n)), changed
/// and added in the newer's order, then removed in the older's.
fn compare<T>(k: &Kind<T>, older: &[T], newer: &[T]) -> Vec<Change> {
    let old_by: HashMap<&str, &T> = older.iter().map(|o| ((k.href)(o), o)).collect();
    let new_hrefs: HashSet<&str> = newer.iter().map(|n| (k.href)(n)).collect();
    let change = |what: &'static str, x: &T, fields, details| Change {
        section: k.section,
        change: what,
        scope: k.scope,
        name: (k.name)(x),
        href: (k.href)(x).to_string(),
        fields,
        details,
    };
    let mut out = Vec::new();
    for n in newer {
        match old_by.get((k.href)(n)) {
            Some(o) => {
                let (fields, details) = changed_lines(&(k.lines)(o, n));
                if !details.is_empty() {
                    out.push(change("changed", n, fields, details));
                }
            }
            None => out.push(change("added", n, Vec::new(), Vec::new())),
        }
    }
    for o in older {
        if !new_hrefs.contains((k.href)(o)) {
            out.push(change("removed", o, Vec::new(), Vec::new()));
        }
    }
    out
}

fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

fn label_lines(a: &Label, b: &Label) -> Vec<DiffLine> {
    let mut out = Vec::new();
    field(&mut out, "key", Some(&a.key), Some(&b.key));
    field(&mut out, "value", Some(&a.value), Some(&b.value));
    field(
        &mut out,
        "deleted",
        Some(yes_no(a.is_deleted())),
        Some(yes_no(b.is_deleted())),
    );
    out
}

fn boundary_lines(
    a: &EnforcementBoundary,
    b: &EnforcementBoundary,
    idx: &NameIndex,
) -> Vec<DiffLine> {
    let mut out = Vec::new();
    field(&mut out, "name", Some(&a.name), Some(&b.name));
    entries(
        &mut out,
        "provider",
        &actor_tokens(&a.providers, idx),
        &actor_tokens(&b.providers, idx),
    );
    entries(
        &mut out,
        "consumer",
        &actor_tokens(&a.consumers, idx),
        &actor_tokens(&b.consumers, idx),
    );
    entries(
        &mut out,
        "service",
        &service_tokens(&a.ingress_services, idx),
        &service_tokens(&b.ingress_services, idx),
    );
    out
}

/// A workload's labels as `key=value` (through the index when the list
/// gave only the href).
fn workload_labels(w: &Workload, idx: &NameIndex) -> Vec<String> {
    let mut v: Vec<String> = w
        .labels
        .iter()
        .map(|l| match (&l.key, &l.value) {
            (Some(k), Some(v)) => format!("{k}={v}"),
            _ => idx.label_text(&l.href),
        })
        .collect();
    v.sort();
    v
}

fn workload_lines(a: &Workload, b: &Workload, idx: &NameIndex) -> Vec<DiffLine> {
    let mut out = Vec::new();
    field(&mut out, "name", a.name.as_deref(), b.name.as_deref());
    field(
        &mut out,
        "hostname",
        a.hostname.as_deref(),
        b.hostname.as_deref(),
    );
    entries(
        &mut out,
        "label",
        &workload_labels(a, idx),
        &workload_labels(b, idx),
    );
    field(
        &mut out,
        "enforcement_mode",
        a.enforcement_mode.as_deref(),
        b.enforcement_mode.as_deref(),
    );
    field(
        &mut out,
        "managed",
        a.managed.map(yes_no),
        b.managed.map(yes_no),
    );
    let (sa, sb) = (a.agent_status(), b.agent_status());
    field(&mut out, "ven_status", sa.as_deref(), sb.as_deref());
    field(
        &mut out,
        "ven_version",
        a.agent_version(),
        b.agent_version(),
    );
    out
}

/// What changed on a workload from `a` (older) to `b`, one short
/// transition per field, from [`workload_lines`]: `app: shop→cart` (a
/// label key; `-` where there is none), `mode: visibility→selective`,
/// `ven: active→stopped`, `ven 23.2→23.5`, `name: a→b`. Empty: nothing
/// changed. Workloads compare's `was→now` (#85).
pub fn workload_transitions(a: &Workload, b: &Workload, idx: &NameIndex) -> Vec<String> {
    // (a label key?, the field or key, before, after), first seen first.
    let mut out: Vec<(bool, String, Option<String>, Option<String>)> = Vec::new();
    for l in workload_lines(a, b, idx) {
        if l.sign == Sign::Same {
            continue;
        }
        let (label, f, v) = match l.text.strip_prefix("label ") {
            Some(kv) => {
                let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                (true, k, v)
            }
            None => match l.text.split_once(": ") {
                Some((f, v)) => (false, f, v),
                None => continue,
            },
        };
        let at = match out.iter().position(|e| e.0 == label && e.1 == f) {
            Some(i) => i,
            None => {
                out.push((label, f.to_string(), None, None));
                out.len() - 1
            }
        };
        let v = Some(if !label && f == "enforcement_mode" {
            v.replace("visibility_only", "visibility")
        } else {
            v.to_string()
        });
        if l.sign == Sign::Removed {
            out[at].2 = v;
        } else {
            out[at].3 = v;
        }
    }
    out.into_iter()
        .map(|(label, f, was, now)| {
            let (was, now) = (
                was.unwrap_or_else(|| "-".into()),
                now.unwrap_or_else(|| "-".into()),
            );
            match (label, f.as_str()) {
                (false, "enforcement_mode") => format!("mode: {was}→{now}"),
                (false, "ven_status") => format!("ven: {was}→{now}"),
                (false, "ven_version") => format!("ven {was}→{now}"),
                _ => format!("{f}: {was}→{now}"),
            }
        })
        .collect()
}

fn rule_set_changes(
    scope: &'static str,
    older: &[RuleSet],
    newer: &[RuleSet],
    idx: &NameIndex,
) -> Vec<Change> {
    // Either list inlining deny rules means this PCE inlines them: an
    // empty list on the other side is then zero rules (every deny rule
    // deleted), not unfetched.
    let inl = inlines_deny(older) || inlines_deny(newer);
    let lines = |a: &RuleSet, b: &RuleSet| {
        diff_rule_set_with(
            Some(a),
            Some(b),
            idx,
            DenySide::of(a, inl, None),
            DenySide::of(b, inl, None),
        )
    };
    let name = |r: &RuleSet| r.name.clone();
    compare(
        &Kind {
            section: "rulesets",
            scope: Some(scope),
            href: |r: &RuleSet| r.href.as_str(),
            name: &name,
            lines: &lines,
        },
        older,
        newer,
    )
}

fn changes_of(s: DiffSection, a: &Snapshot, b: &Snapshot, idx: &NameIndex) -> Vec<Change> {
    let (pa, pb) = (a.sections.policy.as_ref(), b.sections.policy.as_ref());
    match s {
        DiffSection::Rulesets => {
            let (Some(pa), Some(pb)) = (pa, pb) else {
                return Vec::new();
            };
            let mut v = rule_set_changes("draft", &pa.draft_rule_sets, &pb.draft_rule_sets, idx);
            v.extend(rule_set_changes(
                "active",
                &pa.active_rule_sets,
                &pb.active_rule_sets,
                idx,
            ));
            v
        }
        DiffSection::LabelGroups => {
            let (Some(pa), Some(pb)) = (pa, pb) else {
                return Vec::new();
            };
            compare(
                &Kind {
                    section: s.as_str(),
                    scope: None,
                    href: |g: &LabelGroup| g.href.as_str(),
                    name: &|g: &LabelGroup| g.name.clone(),
                    lines: &|x: &LabelGroup, y: &LabelGroup| {
                        diff_label_group(Some(x), Some(y), idx)
                    },
                },
                &pa.label_groups,
                &pb.label_groups,
            )
        }
        DiffSection::Iplists => {
            let (Some(pa), Some(pb)) = (pa, pb) else {
                return Vec::new();
            };
            compare(
                &Kind {
                    section: s.as_str(),
                    scope: None,
                    href: |l: &IpList| l.href.as_str(),
                    name: &|l: &IpList| l.name.clone(),
                    lines: &|x: &IpList, y: &IpList| diff_ip_list(Some(x), Some(y)),
                },
                &pa.ip_lists,
                &pb.ip_lists,
            )
        }
        DiffSection::Services => {
            let (Some(pa), Some(pb)) = (pa, pb) else {
                return Vec::new();
            };
            compare(
                &Kind {
                    section: s.as_str(),
                    scope: None,
                    href: |x: &Service| x.href.as_str(),
                    name: &|x: &Service| x.name.clone(),
                    lines: &|x: &Service, y: &Service| diff_service(Some(x), Some(y)),
                },
                &pa.services,
                &pb.services,
            )
        }
        DiffSection::Boundaries => {
            let (Some(pa), Some(pb)) = (pa, pb) else {
                return Vec::new();
            };
            compare(
                &Kind {
                    section: s.as_str(),
                    scope: None,
                    href: |x: &EnforcementBoundary| x.href.as_str(),
                    name: &|x: &EnforcementBoundary| x.name.clone(),
                    lines: &|x: &EnforcementBoundary, y: &EnforcementBoundary| {
                        boundary_lines(x, y, idx)
                    },
                },
                pa.enforcement_boundaries.as_deref().unwrap_or_default(),
                pb.enforcement_boundaries.as_deref().unwrap_or_default(),
            )
        }
        DiffSection::Labels => {
            let (Some(la), Some(lb)) = (&a.sections.labels, &b.sections.labels) else {
                return Vec::new();
            };
            compare(
                &Kind {
                    section: s.as_str(),
                    scope: None,
                    href: |x: &Label| x.href.as_str(),
                    name: &|x: &Label| x.display(),
                    lines: &label_lines,
                },
                la,
                lb,
            )
        }
        DiffSection::Workloads => {
            let (Some(wa), Some(wb)) = (&a.sections.workloads, &b.sections.workloads) else {
                return Vec::new();
            };
            compare(
                &Kind {
                    section: s.as_str(),
                    scope: None,
                    href: |x: &Workload| x.href.as_str(),
                    name: &|x: &Workload| x.display_name(),
                    lines: &|x: &Workload, y: &Workload| workload_lines(x, y, idx),
                },
                wa,
                wb,
            )
        }
    }
}

/// `older` (`a`) against `newer` (`b`) in the `wanted` sections (none:
/// all). A section not in both snapshots is listed as not compared.
pub fn diff(
    a: (&Snapshot, End),
    b: (&Snapshot, End),
    wanted: &[DiffSection],
    order: &KeyOrder,
) -> SnapshotDiff {
    let ((a, from), (b, to)) = (a, b);
    let idx = index(a, b, order);
    let (has_a, has_b) = (a.sections.names(), b.sections.names());
    let mut sections = Vec::new();
    let mut changes = Vec::new();
    let mut not_compared = Vec::new();
    for s in DiffSection::ALL {
        if !wanted.is_empty() && !wanted.contains(&s) {
            continue;
        }
        let src = s.source();
        let missing: Vec<&str> = [(&from.id, &has_a), (&to.id, &has_b)]
            .into_iter()
            .filter(|(_, has)| !has.contains(&src))
            .map(|(id, _)| id.as_str())
            .collect();
        if !missing.is_empty() {
            not_compared.push(NotCompared {
                section: s.as_str(),
                reason: format!("no {} section in {}", src.as_str(), missing.join(" and ")),
            });
            continue;
        }
        // Boundaries the PCE did not let a side read are not "all removed".
        if s == DiffSection::Boundaries {
            let unread: Vec<&str> = [(&from.id, a), (&to.id, b)]
                .into_iter()
                .filter(|(_, x)| {
                    x.sections
                        .policy
                        .as_ref()
                        .is_some_and(|p| p.enforcement_boundaries.is_none())
                })
                .map(|(id, _)| id.as_str())
                .collect();
            if !unread.is_empty() {
                not_compared.push(NotCompared {
                    section: s.as_str(),
                    reason: format!("boundaries not read in {}", unread.join(" and ")),
                });
                continue;
            }
        }
        let c = changes_of(s, a, b, &idx);
        let n = |what: &str| c.iter().filter(|x| x.change == what).count();
        sections.push(Count {
            section: s.as_str(),
            added: n("added"),
            removed: n("removed"),
            changed: n("changed"),
        });
        changes.extend(c);
    }
    SnapshotDiff {
        from,
        to,
        changed: !changes.is_empty(),
        sections,
        changes,
        not_compared,
    }
}
