//! `hygiene`: the labels, label groups, IP lists and services nothing in
//! the draft policy, the label groups, the pairing profiles or the
//! workloads names (`pce::hygiene`, where-used's reach in one pass), and the
//! workloads missing label keys. Every read is cached and made only for
//! the kinds asked: the object lists, the draft rulesets, the label groups,
//! the pairing profiles and the workload list. An unused row is a
//! candidate: `meta.not_searched` says what may still name it.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{HygieneMeta, KeyCount, KindCount, OutputFormat, TableRows, join_list};
use clap::{Args, ValueEnum};
use pce::hygiene::{Finding, Gaps, Kind, LABELS_BY_FLAGS, NOT_SEARCHED, Objects, Used};
use pce::models::{PolicyScope, WorkloadFilter};
use pce::where_used::Inputs;
use serde::Serialize;
use std::collections::HashMap;

/// A `--kind` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum KindArg {
    Label,
    #[value(alias = "label-group")]
    LabelGroup,
    #[value(alias = "ip-list")]
    IpList,
    Service,
    Workload,
}

impl From<KindArg> for Kind {
    fn from(k: KindArg) -> Self {
        match k {
            KindArg::Label => Kind::Label,
            KindArg::LabelGroup => Kind::LabelGroup,
            KindArg::IpList => Kind::IpList,
            KindArg::Service => Kind::Service,
            KindArg::Workload => Kind::Workload,
        }
    }
}

#[derive(Args, Debug, Default)]
pub struct HygieneArgs {
    /// Only these kinds (comma-separated; default every kind); each kind reads only what it needs
    #[arg(long, value_enum, value_delimiter = ',')]
    pub kind: Vec<KindArg>,
}

impl HygieneArgs {
    /// The kinds asked, in report order (repeats once).
    pub fn kinds(&self) -> Vec<Kind> {
        if self.kind.is_empty() {
            return Kind::ALL.to_vec();
        }
        Kind::ALL
            .into_iter()
            .filter(|k| self.kind.iter().any(|a| Kind::from(*a) == *k))
            .collect()
    }
}

/// The CSV columns, in order (`columns::csv_columns`).
pub const HEADERS: [&str; 6] = ["kind", "name", "href", "reason", "missing", "certain"];

/// One row per unused object, then one per workload missing labels.
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct Rows(pub Vec<Finding>);

impl TableRows for Rows {
    fn headers(&self) -> Vec<&'static str> {
        HEADERS.to_vec()
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .map(|r| {
                vec![
                    r.kind.as_str().to_string(),
                    r.name.clone(),
                    r.href.clone(),
                    r.reason.clone(),
                    r.missing.as_deref().unwrap_or_default().join(", "),
                    if r.certain { "yes" } else { "no" }.to_string(),
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
                    r.name.clone(),
                    r.href.clone(),
                    r.reason.clone(),
                    join_list(r.missing.as_deref().unwrap_or_default()),
                    r.certain.to_string(),
                ]
            })
            .collect()
    }
}

/// `label groups, IP lists`: kinds in words.
fn kinds_text(kinds: &[Kind]) -> String {
    kinds
        .iter()
        .map(|k| k.plural())
        .collect::<Vec<_>>()
        .join(", ")
}

pub async fn run(ctx: &Context, args: &HygieneArgs) -> Result<(), CliError> {
    let kinds = args.kinds();
    let has = |k: Kind| kinds.contains(&k);
    let objects_asked = [Kind::Label, Kind::LabelGroup, Kind::IpList, Kind::Service]
        .into_iter()
        .any(has);
    let mode = ctx.cache_mode;
    let mut truncated = false;
    let mut hit = true;
    let mut warnings = Vec::new();
    let mut not_searched = NOT_SEARCHED.to_string();
    let mut gaps = Gaps::default();
    macro_rules! read {
        ($e:expr) => {{
            let c = $e.await?;
            truncated |= c.truncated;
            hit &= c.hit;
            c
        }};
    }
    // With the PCE's usage flags, in the same one request: they name the
    // holders this tool cannot read (container workloads, permissions,
    // mapping rules, firewall settings ...).
    let labels = if has(Kind::Label) {
        Some(read!(ctx.pce.labels_with_usage(mode)).value)
    } else {
        None
    };
    // Labels and groups sit in label groups.
    let groups = if has(Kind::Label) || has(Kind::LabelGroup) {
        let c = read!(ctx.pce.label_groups(mode));
        gaps.groups = c.truncated;
        Some(c.value)
    } else {
        None
    };
    let ip_lists = if has(Kind::IpList) {
        Some(read!(ctx.pce.ip_lists(mode)).value)
    } else {
        None
    };
    let services = if has(Kind::Service) {
        Some(read!(ctx.pce.services(mode)).value)
    } else {
        None
    };
    let sets = if objects_asked {
        let c = read!(ctx.pce.rule_sets(PolicyScope::Draft, mode));
        gaps.rule_sets = c.truncated;
        c.value
    } else {
        Vec::new()
    };
    // Only labels sit in pairing profiles. Best effort, as where-used: a
    // role that may not list them (403) or a failed read keeps the report;
    // labels the PCE's flags do not settle are then withheld.
    let mut profiles = Vec::new();
    if has(Kind::Label) {
        match ctx.pce.pairing_profiles(mode).await {
            Ok(p) => {
                truncated |= p.truncated;
                hit &= p.hit;
                gaps.profiles = p.truncated;
                profiles = p.value;
            }
            Err(e) => {
                gaps.profiles = true;
                warnings.push(format!("pairing profiles not searched: {e}"));
                not_searched.push_str(&format!(", pairing profiles (read failed: {e})"));
            }
        }
    }
    let workloads = if has(Kind::Label) || has(Kind::Workload) {
        let c = read!(ctx.pce.workloads(&WorkloadFilter::default(), mode));
        gaps.workloads = c.truncated;
        c.value
    } else {
        Vec::new()
    };
    if gaps.workloads && has(Kind::Workload) {
        warnings.push(format!(
            "the workload list was cut short at {}: workloads not read are not checked",
            workloads.len()
        ));
    }
    let none = HashMap::new();
    let used = Used::new(
        &Inputs {
            rule_sets: &sets,
            fallback_deny: &none,
            groups: groups.as_deref().unwrap_or_default(),
            // Deny rules as the list carries them: the per-ruleset endpoint
            // would cost a read each.
            deny_apart: !sets.is_empty() && sets.iter().all(|rs| rs.deny_rules.is_empty()),
        },
        &profiles,
        &workloads,
    );
    let found = pce::hygiene::unused(
        &Objects {
            labels: labels.as_deref(),
            groups: groups.as_deref().filter(|_| has(Kind::LabelGroup)),
            ip_lists: ip_lists.as_deref(),
            services: services.as_deref(),
        },
        &used,
        gaps,
    );
    if labels.iter().flatten().any(|l| l.usage_flags().is_some()) {
        not_searched.push_str(&format!("; {LABELS_BY_FLAGS}"));
    }
    if !found.withheld.is_empty() {
        warnings.push(format!(
            "not listed: {} (a list that may hold them was not read in full: {}; never guessed unused)",
            kinds_text(&found.withheld),
            gaps.text()
        ));
    }
    if !found.uncertain.is_empty() {
        let n = used.deny_unread;
        warnings.push(format!(
            "uncertain (certain: false): {}: {n} ruleset{} may hold deny rules not searched; this PCE lists none inline (policy rules <ruleset> reads them)",
            kinds_text(&found.uncertain),
            if n == 1 { "" } else { "s" }
        ));
    }
    // `loc` is expected only when the PCE has that key: the labels read,
    // else the workloads' own labels say.
    let has_loc = match &labels {
        Some(ls) => ls.iter().any(|l| !l.is_deleted() && l.key == "loc"),
        None => workloads
            .iter()
            .any(|w| w.labels.iter().any(|l| l.key.as_deref() == Some("loc"))),
    };
    let keys = ctx.config.hygiene_keys(has_loc);
    let missing = if has(Kind::Workload) {
        pce::hygiene::missing_labels(&workloads, &keys)
    } else {
        Vec::new()
    };
    let object_kinds: Vec<Kind> = kinds
        .iter()
        .copied()
        .filter(|k| *k != Kind::Workload && !found.withheld.contains(k))
        .collect();
    let mut rows = found.rows;
    let unused_counts = pce::hygiene::counts_by_kind(&rows, &object_kinds)
        .into_iter()
        .map(|(kind, count)| KindCount { kind, count })
        .collect();
    let missing_counts = has(Kind::Workload).then(|| {
        pce::hygiene::counts_by_key(&missing, &keys)
            .into_iter()
            .map(|(key, count)| KeyCount { key, count })
            .collect()
    });
    rows.extend(missing);
    let mut meta = ctx.meta(cache_label(hit, mode), truncated, None);
    meta.not_searched = Some(not_searched);
    meta.hygiene = Some(HygieneMeta {
        kinds: kinds.clone(),
        unused: unused_counts,
        withheld: found.withheld,
        uncertain: found.uncertain,
        keys: has(Kind::Workload).then_some(keys),
        missing: missing_counts,
        workloads: (has(Kind::Workload) || has(Kind::Label)).then_some(workloads.len()),
        deny_unread: used.deny_unread,
    });
    if !warnings.is_empty() {
        if ctx.format != OutputFormat::Json {
            for w in &warnings {
                eprintln!("warning: {w}");
            }
        }
        meta.warnings = Some(warnings);
    }
    ctx.emit_meta(&Rows(rows), &meta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_are_in_report_order_and_default_to_all() {
        let a = HygieneArgs {
            kind: vec![KindArg::Workload, KindArg::IpList, KindArg::IpList],
        };
        assert_eq!(a.kinds(), [Kind::IpList, Kind::Workload]);
        assert_eq!(HygieneArgs::default().kinds(), Kind::ALL);
    }

    #[test]
    fn the_table_joins_missing_keys_and_csv_lists_them() {
        let rows = Rows(vec![Finding {
            kind: Kind::Workload,
            name: "web01".into(),
            href: "/orgs/5/workloads/1".into(),
            reason: "missing labels: app,env".into(),
            missing: Some(vec!["app".into(), "env".into()]),
            certain: true,
        }]);
        assert_eq!(rows.rows()[0][4], "app, env");
        assert_eq!(rows.full_rows()[0][4], join_list(["app", "env"]));
        assert_eq!(rows.headers(), HEADERS);
    }
}
