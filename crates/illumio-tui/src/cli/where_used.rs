//! `where-used`: what names one label, label group, IP list or service in
//! the draft policy and the label groups (`pce::where_used`), and for a
//! label how many workloads carry it. Every read is cached: the object's
//! own list (to find it by name), the draft rulesets, the label groups
//! (labels and label groups only) and, for a label, one workloads read of
//! at most one row filtered by it.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{OutputFormat, TableRows, WhereUsedMeta};
use super::write::{ip_list_in, is_href, label_group_in, label_in, service_in};
use clap::{Args, Subcommand};
use pce::models::PolicyScope;
use pce::where_used::{
    Inputs, NOT_SEARCHED, ObjectType, Reference, Target, WorkloadCount, where_used,
};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Subcommand, Debug)]
pub enum WhereUsedCmd {
    /// Where a label is used: ruleset scopes, rules, label groups, and how many workloads carry it
    Label(TargetArg),
    /// Where a label group is used: scopes, rules, and the groups holding it
    LabelGroup(TargetArg),
    /// Where an IP list is used: rule providers and consumers
    Iplist(TargetArg),
    /// Where a service is used: rule services
    Service(TargetArg),
}

#[derive(Args, Debug)]
pub struct TargetArg {
    /// key=value for a label, else the exact name; or the object's href
    pub name: String,
}

impl WhereUsedCmd {
    fn parts(&self) -> (ObjectType, &str) {
        match self {
            WhereUsedCmd::Label(a) => (ObjectType::Label, &a.name),
            WhereUsedCmd::LabelGroup(a) => (ObjectType::LabelGroup, &a.name),
            WhereUsedCmd::Iplist(a) => (ObjectType::IpList, &a.name),
            WhereUsedCmd::Service(a) => (ObjectType::Service, &a.name),
        }
    }
}

/// The CSV columns, in order (`columns::csv_columns`).
pub const HEADERS: [&str; 12] = [
    "kind", "ruleset", "rule", "side", "action", "pending", "disabled", "via", "name", "count",
    "exact", "href",
];

/// One row per reference, the workload count of a label last.
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct Rows(pub Vec<Reference>);

fn opt(s: &Option<String>) -> String {
    s.clone().unwrap_or_default()
}

fn yes(b: bool) -> String {
    if b { "yes".into() } else { String::new() }
}

impl TableRows for Rows {
    fn headers(&self) -> Vec<&'static str> {
        HEADERS.to_vec()
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .map(|r| {
                let count = r.count.map(|count| {
                    WorkloadCount {
                        count,
                        exact: r.exact.unwrap_or(true),
                    }
                    .text()
                });
                vec![
                    r.kind.as_str().to_string(),
                    opt(&r.ruleset),
                    r.rule.map(|n| format!("#{n}")).unwrap_or_default(),
                    r.side.map(|s| s.as_str().to_string()).unwrap_or_default(),
                    r.action
                        .map(|a| pce::where_used::action_text(a).to_string())
                        .unwrap_or_default(),
                    yes(r.pending),
                    yes(r.disabled),
                    opt(&r.via),
                    opt(&r.name),
                    count.unwrap_or_default(),
                    match r.exact {
                        Some(true) => "yes".into(),
                        Some(false) => "no".into(),
                        None => String::new(),
                    },
                    r.href.clone(),
                ]
            })
            .collect()
    }
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .map(|r| {
                vec![
                    r.kind.as_str().to_string(),
                    opt(&r.ruleset),
                    r.rule.map(|n| n.to_string()).unwrap_or_default(),
                    r.side.map(|s| s.as_str().to_string()).unwrap_or_default(),
                    r.action
                        .map(|a| crate::tui::model::rule_kind_name(a).to_string())
                        .unwrap_or_default(),
                    r.pending.to_string(),
                    r.disabled.to_string(),
                    opt(&r.via),
                    opt(&r.name),
                    r.count.map(|n| n.to_string()).unwrap_or_default(),
                    r.exact.map(|b| b.to_string()).unwrap_or_default(),
                    r.href.clone(),
                ]
            })
            .collect()
    }
}

/// The object found by `spec` in `items`: by href (compared as a draft
/// href), else by `by_name`'s rules. Its href and display name.
fn find<T>(
    items: &[T],
    spec: &str,
    word: &str,
    href_of: impl Fn(&T) -> &str,
    name_of: impl Fn(&T) -> String,
    by_name: impl Fn(&[T], &str) -> Result<String, CliError>,
) -> Result<(String, String), CliError> {
    let href = if is_href(spec) {
        let want = pce::explain::canon_href(spec);
        items
            .iter()
            .map(&href_of)
            .find(|h| pce::explain::canon_href(h) == want)
            .map(str::to_string)
            .ok_or_else(|| CliError::Args(format!("no {word} with href {}", spec.trim())))?
    } else {
        by_name(items, spec)?
    };
    let name = items
        .iter()
        .find(|i| href_of(i) == href)
        .map(&name_of)
        .unwrap_or_else(|| href.clone());
    Ok((href, name))
}

pub async fn run(ctx: &Context, cmd: &WhereUsedCmd) -> Result<(), CliError> {
    let (kind, spec) = cmd.parts();
    let mut truncated = false;
    // The object's own list: it is found there, by name or href.
    let (href, name, hit) = match kind {
        ObjectType::Label => {
            let c = ctx.pce.labels(ctx.cache_mode).await?;
            truncated |= c.truncated;
            let (h, n) = find(
                &c.value,
                spec,
                kind.word(),
                |l| &l.href,
                |l| format!("{}={}", l.key, l.value),
                label_in,
            )?;
            (h, n, c.hit)
        }
        ObjectType::LabelGroup => {
            let c = ctx.pce.label_groups(ctx.cache_mode).await?;
            truncated |= c.truncated;
            let (h, n) = find(
                &c.value,
                spec,
                kind.word(),
                |g| &g.href,
                |g| g.name.clone(),
                label_group_in,
            )?;
            (h, n, c.hit)
        }
        ObjectType::IpList => {
            let c = ctx.pce.ip_lists(ctx.cache_mode).await?;
            truncated |= c.truncated;
            let (h, n) = find(
                &c.value,
                spec,
                kind.word(),
                |l| &l.href,
                |l| l.name.clone(),
                ip_list_in,
            )?;
            (h, n, c.hit)
        }
        ObjectType::Service => {
            let c = ctx.pce.services(ctx.cache_mode).await?;
            truncated |= c.truncated;
            let (h, n) = find(
                &c.value,
                spec,
                kind.word(),
                |s| &s.href,
                |s| s.name.clone(),
                service_in,
            )?;
            (h, n, c.hit)
        }
    };
    let sets = ctx
        .pce
        .rule_sets(PolicyScope::Draft, ctx.cache_mode)
        .await?;
    truncated |= sets.truncated;
    // Only labels and label groups sit in label groups.
    let groups = if matches!(kind, ObjectType::Label | ObjectType::LabelGroup) {
        let g = ctx.pce.label_groups(ctx.cache_mode).await?;
        truncated |= g.truncated;
        g.value
    } else {
        Vec::new()
    };
    let target = Target::new(kind, href.clone());
    // Deny rules as the list carries them: the per-ruleset endpoint would
    // cost a read each.
    let none = HashMap::new();
    let found = where_used(
        &target,
        &Inputs {
            rule_sets: &sets.value,
            fallback_deny: &none,
            groups: &groups,
            deny_apart: sets.value.iter().all(|rs| rs.deny_rules.is_empty()),
        },
    );
    let total = found.refs.len();
    let mut rows = found.refs;
    let workloads = if kind == ObjectType::Label {
        let n = ctx.pce.label_workload_probe(&href, ctx.cache_mode).await?;
        rows.push(Reference::workloads(&href, n.value));
        Some(n.value)
    } else {
        None
    };
    let mut meta = ctx.meta(cache_label(hit, ctx.cache_mode), truncated, None);
    meta.where_used = Some(WhereUsedMeta {
        object: kind,
        name,
        href,
        total,
        workloads,
        deny_unread: found.deny_unread,
        not_searched: NOT_SEARCHED,
    });
    if found.deny_unread > 0 {
        let n = found.deny_unread;
        let w = format!(
            "{n} ruleset{} may hold deny rules not searched: this PCE lists none inline (policy rules <ruleset> reads them)",
            if n == 1 { "" } else { "s" }
        );
        if ctx.format != OutputFormat::Json {
            eprintln!("warning: {w}");
        }
        meta.warnings = Some(vec![w]);
    }
    ctx.emit_meta(&Rows(rows), &meta)
}
