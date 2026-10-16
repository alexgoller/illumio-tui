//! `illumio-tui gen <app>`: the allow rules the app's observed traffic
//! needs, as the TUI's generator would plan them, printed and never
//! written. The same `pce::generate` code, the TUI's row defaults, one
//! global granularity.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{TableRows, join_list};
use super::traffic::{ByArg, DirectionArg, FilterArgs, cli_focus, fetch};
use clap::{Args, ValueEnum};
use pce::generate::{
    Coverage, GenInput, GenRow, Proposal, RuleCounts, Stop, app_ruleset, keys_refusal, propose,
    rows,
};
use pce::models::PolicyScope;
use pce::write::PortSpec;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GranularityArg {
    /// All workloads of the app, All Services
    AppAll,
    /// All workloads of the app, the services seen (default)
    #[default]
    AppPorts,
    /// One rule per role pair, the services seen
    RolePorts,
    /// One rule per role pair, All Services
    RoleAll,
}

impl From<GranularityArg> for Stop {
    fn from(g: GranularityArg) -> Self {
        match g {
            GranularityArg::AppAll => Stop::AppAll,
            GranularityArg::AppPorts => Stop::AppPorts,
            GranularityArg::RolePorts => Stop::RolePorts,
            GranularityArg::RoleAll => Stop::RoleAll,
        }
    }
}

#[derive(Args, Debug, Clone, Default)]
pub struct GenArgs {
    /// The app: one value per scope key, e.g. shop/prod
    pub app: String,
    /// How coarse the rules are (default app-ports)
    #[arg(long, value_enum, default_value_t)]
    pub granularity: GranularityArg,
    /// Also the outbound rules (into the peers' rulesets; IP peers into the app's): a second
    /// traffic query
    #[arg(long)]
    pub outbound: bool,
    /// Relative window ending now, e.g. 1h, 24h, 7d (default from profile, 24h)
    #[arg(long)]
    pub window: Option<String>,
    /// Row cap per traffic query, up to 200000 (default from profile; > 10000 needs --yes)
    #[arg(long)]
    pub max_results: Option<u32>,
    /// Confirm an expensive query (window > 7d, max_results > 10000)
    #[arg(long)]
    pub yes: bool,
}

#[derive(Debug, Serialize)]
pub struct TargetOut {
    pub name: String,
    pub href: Option<String>,
    /// `existing`, `create`, `ambiguous` or `none`.
    pub action: &'static str,
}

#[derive(Debug, Serialize)]
pub struct RowOut {
    pub side: pce::generate::Side,
    pub peer: String,
    pub kind: &'static str,
    pub checked: bool,
    pub checkable: bool,
    pub granularity: &'static str,
    pub flows: u64,
    pub services: Vec<String>,
    pub decisions: pce::generate::DecisionCounts,
    pub warnings: Vec<String>,
    /// An `ip_list` row's list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip_list: Option<IpListRef>,
    /// An `ip_list` row's IP peers seen, by address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ips: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct IpListRef {
    pub name: String,
    pub href: String,
}

#[derive(Debug, Serialize)]
pub struct IpListOut {
    pub name: String,
    pub ranges: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct RuleOut {
    pub ruleset: String,
    /// `create`, `add_services` or `exists`.
    pub action: &'static str,
    pub providers: Vec<String>,
    pub consumers: Vec<String>,
    pub unscoped_consumers: bool,
    pub services: Vec<String>,
    /// 1-based, into `rows`.
    pub rows: Vec<usize>,
    pub existing_rule: Option<String>,
    pub description: String,
    /// Override deny rules of the ruleset that block some of its flows
    /// anyway (they stay uncovered).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub override_deny: Vec<String>,
    /// The PCE JSON a create sends (`{step:N}` for what the batch creates
    /// first); for `add_services`, the services the update adds.
    pub body: serde_json::Value,
}

/// The uncovered flow records by reason.
#[derive(Debug, Serialize)]
pub struct UncoveredBy {
    pub unchecked: u64,
    pub folded_ips: u64,
    pub unsupported_proto: u64,
    pub broadcast_multicast: u64,
    /// Behind a rule whose ruleset is ambiguous or cannot be named.
    pub no_target: u64,
    /// Behind a rule, but an override deny rule blocks them anyway.
    pub override_deny: u64,
}

impl From<Coverage> for UncoveredBy {
    fn from(c: Coverage) -> Self {
        UncoveredBy {
            unchecked: c.unchecked,
            folded_ips: c.folded_ips,
            unsupported_proto: c.unsupported_proto,
            broadcast_multicast: c.broadcast_multicast,
            no_target: c.no_target,
            override_deny: c.override_deny,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Preview {
    #[serde(flatten)]
    pub counts: RuleCounts,
    pub covered_flows: u64,
    pub uncovered_flows: u64,
    pub uncovered_by: UncoveredBy,
    /// Why the rules cannot be written as shown (a ruleset is ambiguous or
    /// cannot be named).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GenReport {
    pub app: String,
    pub keys: Vec<String>,
    pub window: String,
    pub granularity: &'static str,
    pub outbound: bool,
    pub target: TargetOut,
    pub rows: Vec<RowOut>,
    pub ip_lists: Vec<IpListOut>,
    pub rules: Vec<RuleOut>,
    pub preview: Preview,
}

impl TableRows for GenReport {
    fn units(&self) -> Option<&'static str> {
        Some("rules proposed from flow records; nothing is written")
    }
    fn headers(&self) -> Vec<&'static str> {
        vec![
            "ruleset",
            "action",
            "consumers → providers",
            "services",
            "unscoped",
        ]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.rules
            .iter()
            .map(|r| {
                vec![
                    r.ruleset.clone(),
                    r.action.to_string(),
                    format!("{} → {}", r.consumers.join(", "), r.providers.join(", ")),
                    r.services.join(", "),
                    r.unscoped_consumers.to_string(),
                ]
            })
            .collect()
    }
    fn full_headers(&self) -> Vec<&'static str> {
        vec![
            "ruleset",
            "action",
            "consumers",
            "providers",
            "services",
            "unscoped",
            "existing_rule",
            "description",
        ]
    }
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.rules
            .iter()
            .map(|r| {
                vec![
                    r.ruleset.clone(),
                    r.action.to_string(),
                    join_list(&r.consumers),
                    join_list(&r.providers),
                    join_list(&r.services),
                    r.unscoped_consumers.to_string(),
                    r.existing_rule.clone().unwrap_or_default(),
                    r.description.clone(),
                ]
            })
            .collect()
    }
}

fn service_texts(services: &std::collections::BTreeSet<PortSpec>) -> Vec<String> {
    services.iter().map(PortSpec::display).collect()
}

fn row_out(r: &GenRow, global: Stop, scope_keys: &[String]) -> RowOut {
    let stop = r.effective(global);
    let mut warnings = r.flag_texts(stop, scope_keys);
    let (ip_list, ips) = match &r.peer {
        pce::generate::PeerKind::IpList { href, name, ips } => {
            // As before #38, for scripts that read the warnings.
            warnings.insert(0, format!("uses IP list {name}"));
            let list = IpListRef {
                name: name.clone(),
                href: href.clone(),
            };
            (Some(list), Some(ips.clone()))
        }
        _ => (None, None),
    };
    RowOut {
        side: r.side,
        peer: r.peer_text(),
        kind: r.kind_text(),
        checked: r.proposed(),
        checkable: r.checkable,
        granularity: stop.cli(),
        flows: r.flows,
        services: if stop.is_all() {
            vec!["all services".into()]
        } else {
            service_texts(&r.services)
        },
        decisions: r.decisions,
        warnings,
        ip_list,
        ips,
    }
}

fn rules_out(p: &Proposal, idx: &pce::NameIndex) -> Vec<RuleOut> {
    use pce::generate::RuleAction;
    let all = idx.all_services_href();
    p.rules
        .iter()
        .map(|r| RuleOut {
            ruleset: p.targets[r.target].name(),
            action: r.action.as_str(),
            providers: r.providers.iter().map(|a| a.text(p, idx)).collect(),
            consumers: r.consumers.iter().map(|a| a.text(p, idx)).collect(),
            unscoped_consumers: r.unscoped_consumers,
            services: r
                .services
                .iter()
                .map(|s| match s {
                    pce::write::ServiceRef::Href(h) if Some(h.as_str()) == all => {
                        "all services".to_string()
                    }
                    pce::write::ServiceRef::Href(h) => idx.service_name(h).unwrap_or(h).to_string(),
                    pce::write::ServiceRef::Port(port) => port.display(),
                })
                .collect(),
            rows: r.rows.iter().map(|i| i + 1).collect(),
            existing_rule: match &r.action {
                RuleAction::Create => None,
                RuleAction::AddServices { rule_href, .. } | RuleAction::Exists { rule_href } => {
                    Some(rule_href.clone())
                }
            },
            description: r.description.clone(),
            override_deny: r.denied_by.clone(),
            body: r.body(p),
        })
        .collect()
}

pub async fn run(ctx: &Context, args: &GenArgs) -> Result<(), CliError> {
    let keys = ctx.config.scope_keys.clone();
    let global = Stop::from(args.granularity);
    let filter = FilterArgs {
        window: args.window.clone(),
        max_results: args.max_results,
        yes: args.yes,
        app: Some(args.app.clone()),
        direction: Some(if args.outbound {
            DirectionArg::Both
        } else {
            DirectionArg::In
        }),
        ..FilterArgs::default()
    };
    let focus = cli_focus(&filter, &keys, ByArg::Pairs)?
        .ok_or_else(|| CliError::Other("--app is set".into()))?;
    if let Some(e) = keys_refusal(&focus.focus.keys, &keys) {
        return Err(CliError::Args(e));
    }
    if !focus.focus.is_label_group() {
        return Err(CliError::Args(format!(
            "{} is not an app of labels: write it as {}",
            args.app,
            keys.iter()
                .map(|k| format!("<{k}>"))
                .collect::<Vec<_>>()
                .join("/")
        )));
    }
    let Some(fetched) = fetch(ctx, &filter, Some(&focus)).await? else {
        return Ok(());
    };
    let focus = focus.focus;
    let app = focus.app_text();
    if fetched.truncated {
        return Err(CliError::Args(format!(
            "traffic capped at {} for {app}: raise --max-results or narrow --window; no rules proposed",
            fetched.cap
        )));
    }
    if fetched.incomplete {
        return Err(CliError::Incomplete(format!(
            "a PCE region did not answer the traffic query for {app}: run it again; no rules proposed"
        )));
    }
    let idx = fetched.index.clone();
    if global.is_all() && idx.all_services_href().is_none() {
        return Err(CliError::Args(format!(
            "--granularity {}: the PCE has no All Services object; use app-ports or role-ports",
            global.cli()
        )));
    }
    let drafts = ctx
        .pce
        .rule_sets(PolicyScope::Draft, ctx.cache_mode)
        .await?;
    if drafts.truncated {
        return Err(CliError::Args(
            "too many draft rulesets to check the rules against; no rules proposed".into(),
        ));
    }
    let tier = ctx.config.tier_key.clone();
    let window = args
        .window
        .clone()
        .unwrap_or_else(|| ctx.config.traffic_default_window.clone());
    let target = app_ruleset(
        &fetched.flows,
        &focus,
        &idx,
        &drafts.value,
        &ctx.config.generator_ruleset_name,
        None,
    );
    let gen_rows = rows(&fetched.flows, &focus, &tier, &idx, target);
    let input = GenInput {
        flows: &fetched.flows,
        focus: &focus,
        tier_key: &tier,
        drafts: &drafts.value,
        idx: &idx,
        ruleset_name: &ctx.config.generator_ruleset_name,
        window: &window,
        outbound: args.outbound,
        picked: None,
    };
    let p = propose(&gen_rows, global, &input);
    // Each row behind a rule an override deny undercuts says so.
    let mut denied: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for r in p.rules.iter().filter(|r| r.denied_flows > 0) {
        for &i in &r.rows {
            denied.entry(i).or_default().push(format!(
                "override deny blocks {} of the rule's flows",
                r.denied_flows
            ));
        }
    }
    let blocker = p.blocker();
    if let Some(b) = &blocker {
        eprintln!("warning: {b}: those rules cannot be written as shown");
    }
    let t = &p.targets[0];
    let report = GenReport {
        app: app.clone(),
        keys: keys.clone(),
        window,
        granularity: global.cli(),
        outbound: args.outbound,
        target: TargetOut {
            name: t.name(),
            href: match t {
                pce::generate::Target::Existing { href, .. } => Some(href.clone()),
                _ => None,
            },
            action: t.action(),
        },
        rows: gen_rows
            .iter()
            .enumerate()
            .filter(|(_, r)| args.outbound || r.side == pce::generate::Side::Inbound)
            .map(|(i, r)| {
                let mut out = row_out(r, global, &keys);
                out.warnings
                    .extend(denied.get(&i).into_iter().flatten().cloned());
                out
            })
            .collect(),
        ip_lists: p
            .ip_lists
            .iter()
            .map(|d| IpListOut {
                name: d.name.clone(),
                ranges: d.ranges.iter().map(|r| r.display()).collect(),
            })
            .collect(),
        rules: rules_out(&p, &idx),
        preview: Preview {
            counts: p.counts(),
            covered_flows: p.coverage.covered,
            uncovered_flows: p.coverage.uncovered,
            uncovered_by: p.coverage.into(),
            blocker,
        },
    };
    let mut meta = fetched.meta(ctx);
    meta.cache = cache_label(fetched.cache == "hit" && drafts.hit, ctx.cache_mode);
    ctx.emit_meta(&report, &meta)
}
