//! `traffic explain`: which rule allows or blocks the flows of one
//! source, destination and port. One narrowed Explorer query evaluated
//! against the draft policy (`update_rules`), its flows' rule hrefs
//! resolved through the draft rulesets.
use super::context::Context;
use super::error::CliError;
use super::output::{TableRows, join_list};
use super::traffic::{FilterArgs, fetch};
use clap::{Args, ValueEnum};
use pce::explain::{Explanation, MatchKind, MatchedRule, explain, ruleset_of};
use pce::models::{DenyRule, PolicyScope, RuleSet};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};

/// What `traffic explain` costs: `--help` and `meta.cost` say it.
pub const EXPLAIN_COST: &str = "1 Explorer query evaluated against the draft (1 POST, status polls, 1 PUT update_rules, polls, 1 download); the draft rulesets (1 GET) when a flow names a rule; 1 GET per ruleset whose deny rules are not inline; the object lists for names (cached)";

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ExplainProto {
    Tcp,
    Udp,
}

/// `traffic explain`: one source, one destination, one port.
#[derive(Args, Debug, Clone)]
pub struct ExplainArgs {
    /// Source: key=value, ip:ADDR, ipl:NAME, wl:HOST, fqdn:NAME. Comma = AND, repeat = OR
    #[arg(long, required = true)]
    pub src: Vec<String>,
    /// Destination, same syntax as --src
    #[arg(long, required = true)]
    pub dst: Vec<String>,
    /// Destination port
    #[arg(long)]
    pub port: u16,
    /// tcp or udp
    #[arg(long, value_enum)]
    pub proto: ExplainProto,
    /// Relative window ending now, e.g. 1h, 24h, 7d (default from profile, 24h)
    #[arg(long)]
    pub window: Option<String>,
    /// Row cap, up to 200000 (default from profile, 10000; > 10000 needs --yes)
    #[arg(long)]
    pub max_results: Option<u32>,
    /// Confirm an expensive query (window > 7d or max_results > 10000)
    #[arg(long)]
    pub yes: bool,
}

impl ExplainArgs {
    /// The Explorer filter it runs: always with the draft evaluation, no marks.
    fn filter(&self) -> FilterArgs {
        FilterArgs {
            window: self.window.clone(),
            src: self.src.clone(),
            dst: self.dst.clone(),
            port: Some(self.port),
            proto: Some(
                match self.proto {
                    ExplainProto::Tcp => "tcp",
                    ExplainProto::Udp => "udp",
                }
                .into(),
            ),
            max_results: self.max_results,
            yes: self.yes,
            draft: true,
            no_marks: true,
            ..FilterArgs::default()
        }
    }
}

/// One flow record and what decides it.
#[derive(Debug, Serialize)]
pub struct ExplainedFlow {
    pub src: String,
    pub dst: String,
    pub service: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process: Option<String>,
    pub connections: u64,
    #[serde(flatten)]
    pub explanation: Explanation,
}

#[derive(Debug, Serialize)]
pub struct Explained {
    pub flows: Vec<ExplainedFlow>,
}

fn rule_cell(r: &MatchedRule) -> String {
    if r.resolved {
        format!("{} · {}", r.ruleset.as_deref().unwrap_or("?"), r.summary())
    } else {
        r.href.clone()
    }
}

impl TableRows for Explained {
    fn headers(&self) -> Vec<&'static str> {
        vec![
            "src", "dst", "service", "decision", "draft", "verdict", "kind", "rule", "on",
        ]
    }

    /// One row per rule; a flow no rule names gets one row with empty
    /// rule cells.
    fn rows(&self) -> Vec<Vec<String>> {
        let mut out = Vec::new();
        for f in &self.flows {
            let e = &f.explanation;
            let head = vec![
                f.src.clone(),
                f.dst.clone(),
                f.service.clone(),
                e.reported.as_str().to_string(),
                e.draft
                    .as_ref()
                    .map(|d| d.as_str().to_string())
                    .unwrap_or_default(),
                e.verdict.clone(),
            ];
            if e.rules.is_empty() {
                let mut row = head.clone();
                row.extend([String::new(), String::new(), String::new()]);
                out.push(row);
            }
            for r in &e.rules {
                let mut row = head.clone();
                row.extend([
                    r.kind.as_str().to_string(),
                    rule_cell(r),
                    match r.enabled {
                        Some(true) => "✓".into(),
                        Some(false) => "✗".into(),
                        None => String::new(),
                    },
                ]);
                out.push(row);
            }
        }
        out
    }

    fn full_headers(&self) -> Vec<&'static str> {
        vec![
            "src",
            "dst",
            "service",
            "decision",
            "draft",
            "verdict",
            "kind",
            "rule href",
            "ruleset",
            "scopes",
            "consumers",
            "providers",
            "services",
            "enabled",
        ]
    }

    fn full_rows(&self) -> Vec<Vec<String>> {
        let mut out = Vec::new();
        for f in &self.flows {
            let e = &f.explanation;
            let head = vec![
                f.src.clone(),
                f.dst.clone(),
                f.service.clone(),
                e.reported.as_str().to_string(),
                e.draft
                    .as_ref()
                    .map(|d| d.as_str().to_string())
                    .unwrap_or_default(),
                e.verdict.clone(),
            ];
            if e.rules.is_empty() {
                let mut row = head.clone();
                row.extend(std::iter::repeat_n(String::new(), 8));
                out.push(row);
            }
            for r in &e.rules {
                let mut row = head.clone();
                row.extend([
                    r.kind.as_str().to_string(),
                    r.href.clone(),
                    r.ruleset.clone().unwrap_or_default(),
                    join_list(&r.scopes),
                    join_list(&r.consumers),
                    join_list(&r.providers),
                    join_list(&r.services),
                    r.enabled.map(|b| b.to_string()).unwrap_or_default(),
                ]);
                out.push(row);
            }
        }
        out
    }
}

/// The rulesets (by href) whose deny rules a flow names but that carry
/// none inline: their deny rules are one GET each.
fn deny_fallbacks(flows: &[pce::traffic::ResolvedFlow], sets: &[RuleSet]) -> BTreeSet<String> {
    flows
        .iter()
        .flat_map(|f| &f.rules)
        .filter(|h| MatchKind::of_href(h).is_deny())
        .filter_map(|h| ruleset_of(h))
        .filter_map(|rs| {
            sets.iter()
                .find(|s| pce::explain::canon_href(&s.href) == rs)
        })
        .filter(|s| s.deny_rules.is_empty())
        .map(|s| s.href.clone())
        .collect()
}

pub async fn run(ctx: &Context, args: &ExplainArgs) -> Result<(), CliError> {
    let filter = args.filter();
    let Some(f) = fetch(ctx, &filter, None).await? else {
        return Ok(());
    };
    let names_a_rule = f
        .flows
        .iter()
        .flat_map(|f| &f.rules)
        .any(|h| ruleset_of(h).is_some());
    let sets: Vec<RuleSet> = if names_a_rule {
        ctx.pce
            .rule_sets(PolicyScope::Draft, ctx.cache_mode)
            .await?
            .value
    } else {
        Vec::new()
    };
    let mut extra: HashMap<String, Vec<DenyRule>> = HashMap::new();
    for rs in deny_fallbacks(&f.flows, &sets) {
        let d = ctx.pce.deny_rules(&rs, ctx.cache_mode).await?.value;
        extra.insert(rs, d);
    }
    let data = Explained {
        flows: f
            .flows
            .iter()
            .map(|fl| ExplainedFlow {
                src: fl.src.display_name(),
                dst: fl.dst.display_name(),
                service: fl.service_text(),
                process: fl.process_name.clone(),
                connections: fl.num_connections,
                explanation: explain(fl, &sets, &extra, &f.index),
            })
            .collect(),
    };
    let mut meta = f.meta(ctx);
    meta.cost = Some(EXPLAIN_COST);
    ctx.emit_meta(&data, &meta)
}
