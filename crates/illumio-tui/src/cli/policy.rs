//! Policy: rulesets (list; add, update, delete), the rules of one ruleset
//! (list; add, update, delete allow rules), deny rules, and pending
//! provisioning changes.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::TableRows;
use super::write::{
    self, ActorToken, Lists, ServiceToken, WriteFlags, is_href, parse_actors, parse_scopes,
    parse_services, resolve_actors, resolve_scopes, resolve_services, run_plan, run_rule_plan,
};
use clap::{Args, Subcommand};
use pce::impact::{
    Impact, ImpactInputs, RuleActors, is_rule_href, rule_impact, rule_impact_with,
    rule_set_href_of, workload_labels,
};
use pce::models::{
    DenyRule, PendingChange, PolicyScope, ResolveLabelsAs, RuleSet, Workload, WorkloadFilter,
};
use pce::write::{
    ActorRef, DenyRuleDraft, DenyRulePatch, IfMissing, Plan, ProvisionRequest, ProvisionSubset,
    RuleDraft, RulePatch, RuleSetDraft, RuleSetPatch, ServiceRef, WriteError, checked_nested_href,
};
use pce::{Cached, PceError, RuleKind, RuleSetView, RuleView};
use serde::Serialize;

#[derive(Subcommand, Debug)]
pub enum PolicyCmd {
    /// List rulesets with resolved scopes; add, update or delete one
    Rulesets(RulesetsCmd),
    /// List the rules of one ruleset (name or href); add, update or delete an allow rule
    Rules(RulesCmd),
    /// Add, update or delete a deny rule
    #[command(subcommand)]
    DenyRules(DenyRulesAction),
    /// List changes waiting to be provisioned
    Pending,
    /// Provision pending changes (needs writes enabled and --allow-writes)
    Provision(ProvisionArgs),
}

/// `policy rulesets` lists by default; `add`, `update` and `delete` write.
#[derive(Args, Debug)]
#[command(args_conflicts_with_subcommands = true)]
pub struct RulesetsCmd {
    #[command(subcommand)]
    pub action: Option<RulesetsAction>,
    #[command(flatten)]
    pub list: RulesetsArgs,
}

#[derive(Subcommand, Debug)]
pub enum RulesetsAction {
    /// Create a ruleset (needs writes enabled and --allow-writes)
    Add(RulesetAddArgs),
    /// Change a ruleset's name, description, state or scopes
    Update(RulesetUpdateArgs),
    /// Delete a ruleset and its rules
    Delete(RulesetDeleteArgs),
}

#[derive(Args, Debug)]
pub struct RulesetAddArgs {
    /// Ruleset name
    pub name: String,
    /// Description
    #[arg(long)]
    pub description: Option<String>,
    /// Create the ruleset disabled
    #[arg(long)]
    pub disabled: bool,
    /// One scope: labels or label groups, comma-separated (key=value, group:NAME, an href); repeat for alternatives; none means everywhere
    #[arg(long = "scope")]
    pub scopes: Vec<String>,
    /// Do nothing when a ruleset with this name (case-insensitive) exists
    #[arg(long)]
    pub if_missing: bool,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct RulesetUpdateArgs {
    /// Ruleset name (case-insensitive) or href
    pub ruleset: String,
    /// New name
    #[arg(long)]
    pub name: Option<String>,
    /// New description
    #[arg(long)]
    pub description: Option<String>,
    /// Enable the ruleset
    #[arg(long, conflicts_with = "disabled")]
    pub enabled: bool,
    /// Disable the ruleset
    #[arg(long)]
    pub disabled: bool,
    /// Replace every scope: one --scope per scope, labels or label groups comma-separated
    #[arg(long = "scope", conflicts_with = "clear_scopes")]
    pub scopes: Vec<String>,
    /// Remove every scope (the ruleset then applies everywhere)
    #[arg(long)]
    pub clear_scopes: bool,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct RulesetDeleteArgs {
    /// Ruleset name (case-insensitive) or href
    pub ruleset: String,
    #[command(flatten)]
    pub write: WriteFlags,
}

/// `policy rules <ruleset>` lists; `add`, `update` and `delete` write; `impact` reports one rule.
#[derive(Args, Debug)]
#[command(args_conflicts_with_subcommands = true)]
pub struct RulesCmd {
    #[command(subcommand)]
    pub action: Option<RulesAction>,
    #[command(flatten)]
    pub list: RulesArgs,
}

#[derive(Subcommand, Debug)]
pub enum RulesAction {
    /// Create an allow rule in a ruleset (needs writes enabled and --allow-writes)
    Add(RuleAddArgs),
    /// Change an allow rule, addressed by href
    Update(RuleUpdateArgs),
    /// Delete an allow rule, addressed by href
    Delete(RuleDeleteArgs),
    /// Report one rule's impact (workloads matched as providers and consumers), addressed by href
    Impact(RuleImpactArgs),
}

#[derive(Subcommand, Debug)]
pub enum DenyRulesAction {
    /// Create a deny rule in a ruleset (needs writes enabled and --allow-writes)
    Add(DenyRuleAddArgs),
    /// Change a deny rule, addressed by href
    Update(DenyRuleUpdateArgs),
    /// Delete a deny rule, addressed by href
    Delete(DenyRuleDeleteArgs),
}

/// The three sides of a new rule.
#[derive(Args, Debug)]
pub struct RuleSides {
    /// Provider: key=value, label:key=value, group:NAME, iplist:NAME, workload:HOST, all or an href; prefix ! to exclude (repeatable)
    #[arg(long = "provider")]
    pub providers: Vec<String>,
    /// Consumer, same forms as --provider (repeatable)
    #[arg(long = "consumer")]
    pub consumers: Vec<String>,
    /// Service: a name (or service:NAME), P/PROTO, P-Q/PROTO, a protocol name, or an href (repeatable)
    #[arg(long = "service")]
    pub services: Vec<String>,
}

/// List edits shared by `rules update` and `deny-rules update`.
#[derive(Args, Debug)]
pub struct SideEdits {
    /// Provider to add, same forms as add --provider (repeatable)
    #[arg(long = "add-provider")]
    pub add_providers: Vec<String>,
    /// Provider to remove; must match an existing provider (repeatable)
    #[arg(long = "remove-provider")]
    pub remove_providers: Vec<String>,
    /// Consumer to add (repeatable)
    #[arg(long = "add-consumer")]
    pub add_consumers: Vec<String>,
    /// Consumer to remove; must match an existing consumer (repeatable)
    #[arg(long = "remove-consumer")]
    pub remove_consumers: Vec<String>,
    /// Service to add, same forms as add --service (repeatable)
    #[arg(long = "add-service")]
    pub add_services: Vec<String>,
    /// Service to remove; must match an existing service (repeatable)
    #[arg(long = "remove-service")]
    pub remove_services: Vec<String>,
}

#[derive(Args, Debug)]
pub struct RuleAddArgs {
    /// Ruleset name (case-insensitive) or href
    pub ruleset: String,
    #[command(flatten)]
    pub sides: RuleSides,
    /// Description
    #[arg(long)]
    pub description: Option<String>,
    /// Create the rule disabled
    #[arg(long)]
    pub disabled: bool,
    /// Consumers may come from outside the ruleset's scopes
    #[arg(long)]
    pub unscoped_consumers: bool,
    /// Resolve provider labels as workloads or virtual_services (repeatable; the other side defaults to workloads)
    #[arg(long = "resolve-providers-as", value_parser = ["workloads", "virtual_services"])]
    pub resolve_providers_as: Vec<String>,
    /// Resolve consumer labels as workloads or virtual_services (repeatable; the other side defaults to workloads)
    #[arg(long = "resolve-consumers-as", value_parser = ["workloads", "virtual_services"])]
    pub resolve_consumers_as: Vec<String>,
    /// Also report the rule's impact (workloads matched as providers and consumers), computed before anything is sent; one workloads read, cached
    #[arg(long)]
    pub impact: bool,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct RuleUpdateArgs {
    /// Rule href: /orgs/N/sec_policy/draft/rule_sets/ID/sec_rules/ID
    pub rule: String,
    /// New description
    #[arg(long)]
    pub description: Option<String>,
    /// Enable the rule
    #[arg(long, conflicts_with = "disabled")]
    pub enabled: bool,
    /// Disable the rule
    #[arg(long)]
    pub disabled: bool,
    #[command(flatten)]
    pub edits: SideEdits,
    /// Consumers may come from outside the ruleset's scopes
    #[arg(long, conflicts_with = "scoped_consumers")]
    pub unscoped_consumers: bool,
    /// Consumers must come from inside the ruleset's scopes
    #[arg(long)]
    pub scoped_consumers: bool,
    /// Also report the rule's impact (workloads matched as providers and consumers), computed before anything is sent; one workloads read, cached
    #[arg(long)]
    pub impact: bool,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct RuleDeleteArgs {
    /// Rule href: /orgs/N/sec_policy/draft/rule_sets/ID/sec_rules/ID
    pub rule: String,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct RuleImpactArgs {
    /// Rule or deny rule href: /orgs/N/sec_policy/draft/rule_sets/ID/sec_rules/ID (or deny_rules/ID)
    pub rule: String,
}

#[derive(Args, Debug)]
pub struct DenyRuleAddArgs {
    /// Ruleset name (case-insensitive) or href
    pub ruleset: String,
    #[command(flatten)]
    pub sides: RuleSides,
    /// Description
    #[arg(long)]
    pub description: Option<String>,
    /// Create the deny rule disabled
    #[arg(long)]
    pub disabled: bool,
    /// Override deny: evaluated before the allow rules
    #[arg(long = "override")]
    pub override_: bool,
    /// Also report the rule's impact (workloads matched as providers and consumers), computed before anything is sent; one workloads read, cached
    #[arg(long)]
    pub impact: bool,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct DenyRuleUpdateArgs {
    /// Deny rule href: /orgs/N/sec_policy/draft/rule_sets/ID/deny_rules/ID
    pub deny_rule: String,
    /// New description
    #[arg(long)]
    pub description: Option<String>,
    /// Enable the deny rule
    #[arg(long, conflicts_with = "disabled")]
    pub enabled: bool,
    /// Disable the deny rule
    #[arg(long)]
    pub disabled: bool,
    #[command(flatten)]
    pub edits: SideEdits,
    /// Make it an override deny
    #[arg(long = "override", conflicts_with = "no_override")]
    pub override_: bool,
    /// Make it a regular deny
    #[arg(long)]
    pub no_override: bool,
    /// Also report the rule's impact (workloads matched as providers and consumers), computed before anything is sent; one workloads read, cached
    #[arg(long)]
    pub impact: bool,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct DenyRuleDeleteArgs {
    /// Deny rule href: /orgs/N/sec_policy/draft/rule_sets/ID/deny_rules/ID
    pub deny_rule: String,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct ProvisionArgs {
    /// Provision every pending change
    #[arg(long, conflicts_with = "targets", required_unless_present = "targets")]
    pub all: bool,
    /// Ruleset names, or hrefs of pending rulesets, rules, IP lists, services or label groups
    pub targets: Vec<String>,
    /// Description recorded with the new policy version
    #[arg(long)]
    pub description: Option<String>,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug, Clone, Default)]
pub struct RulesetsArgs {
    /// Read active policy instead of draft
    #[arg(long, conflicts_with = "draft")]
    pub active: bool,
    /// Read draft policy (the default; explicit for scripts)
    #[arg(long)]
    pub draft: bool,
    /// Case-insensitive substring of the ruleset name
    #[arg(long)]
    pub name: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct RulesArgs {
    /// Ruleset name (exact, case-insensitive) or href
    pub ruleset: Option<String>,
    /// Read active policy instead of draft
    #[arg(long, conflicts_with = "draft")]
    pub active: bool,
    /// Read draft policy (the default; explicit for scripts)
    #[arg(long)]
    pub draft: bool,
    /// Add each rule's impact: workloads matched as providers and consumers (one workloads read, cached)
    #[arg(long)]
    pub impact: bool,
}

/// One rule as `policy rules` prints it; `impact` with `--impact`.
#[derive(Debug, Serialize)]
pub struct RuleOut {
    #[serde(flatten)]
    pub rule: RuleView,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impact: Option<Impact>,
}

#[derive(Debug, Serialize)]
pub struct RulesOutput {
    pub ruleset: RuleSetView,
    pub rules: Vec<RuleOut>,
    /// `--impact` was given: the table gains a `P/C` column.
    #[serde(skip)]
    pub with_impact: bool,
}

fn scope(active: bool) -> PolicyScope {
    if active {
        PolicyScope::Active
    } else {
        PolicyScope::Draft
    }
}

impl TableRows for Vec<RuleSetView> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["name", "enabled", "scopes", "rules", "pending", "href"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .map(|r| {
                vec![
                    r.name.clone(),
                    r.enabled.to_string(),
                    r.scopes.join(" | "),
                    r.rule_count.to_string(),
                    r.update_type.clone().unwrap_or_default(),
                    r.href.clone(),
                ]
            })
            .collect()
    }
}

fn names<'a>(it: impl Iterator<Item = &'a str>) -> String {
    it.collect::<Vec<_>>().join(", ")
}

/// `allow`, `deny`, `override_deny`, as in JSON.
fn kind_text(kind: RuleKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

impl TableRows for RulesOutput {
    fn headers(&self) -> Vec<&'static str> {
        let mut h = vec!["kind", "enabled", "consumers", "providers", "services"];
        if self.with_impact {
            h.push("P/C");
        }
        h.extend(["description", "pending"]);
        h
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.rules
            .iter()
            .map(|o| {
                let r = &o.rule;
                let mut row = vec![
                    kind_text(r.kind),
                    r.enabled.to_string(),
                    names(r.consumers.iter().map(|a| a.name.as_str())),
                    names(r.providers.iter().map(|a| a.name.as_str())),
                    names(r.services.iter().map(|s| s.name.as_str())),
                ];
                if self.with_impact {
                    row.push(
                        o.impact
                            .as_ref()
                            .map_or_else(|| "-".to_string(), Impact::pc),
                    );
                }
                row.extend([
                    r.description.clone().unwrap_or_default(),
                    r.update_type.clone().unwrap_or_default(),
                ]);
                row
            })
            .collect()
    }
}

impl TableRows for RuleOut {
    fn headers(&self) -> Vec<&'static str> {
        vec!["field", "value"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let r = &self.rule;
        let mut rows = vec![
            vec!["href".to_string(), r.href.clone()],
            vec!["kind".to_string(), kind_text(r.kind)],
            vec![
                "providers".to_string(),
                names(r.providers.iter().map(|a| a.name.as_str())),
            ],
            vec![
                "consumers".to_string(),
                names(r.consumers.iter().map(|a| a.name.as_str())),
            ],
            vec![
                "services".to_string(),
                names(r.services.iter().map(|s| s.name.as_str())),
            ],
        ];
        if let Some(i) = &self.impact {
            rows.push(vec!["impact".to_string(), i.summary()]);
            for s in &i.per_scope {
                let scope = if s.scope.is_empty() {
                    "All".to_string()
                } else {
                    s.scope.join(" & ")
                };
                rows.push(vec![
                    format!("scope {scope}"),
                    format!("{}/{}", s.providers, s.consumers),
                ]);
            }
            rows.extend(
                i.ip_lists
                    .iter()
                    .map(|l| vec!["ip list".to_string(), l.line()]),
            );
            rows.extend(
                i.unsupported
                    .iter()
                    .map(|u| vec!["unsupported".to_string(), u.clone()]),
            );
        }
        rows
    }
}

impl TableRows for Vec<PendingChange> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["object_type", "change_type", "name", "href"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .map(|p| {
                vec![
                    p.object_type.clone(),
                    p.change_type.clone(),
                    p.name.clone().unwrap_or_default(),
                    p.href.clone(),
                ]
            })
            .collect()
    }
}

pub async fn run(ctx: &Context, cmd: &PolicyCmd) -> Result<(), CliError> {
    match cmd {
        PolicyCmd::Rulesets(c) => match &c.action {
            None => rulesets(ctx, &c.list).await,
            Some(RulesetsAction::Add(a)) => ruleset_add(ctx, a).await,
            Some(RulesetsAction::Update(a)) => ruleset_update(ctx, a).await,
            Some(RulesetsAction::Delete(a)) => ruleset_delete(ctx, a).await,
        },
        PolicyCmd::Rules(c) => match &c.action {
            None => rules(ctx, &c.list).await,
            Some(RulesAction::Add(a)) => rule_add(ctx, a).await,
            Some(RulesAction::Update(a)) => rule_update(ctx, a).await,
            Some(RulesAction::Delete(a)) => rule_delete(ctx, a).await,
            Some(RulesAction::Impact(a)) => rule_impact_one(ctx, a).await,
        },
        PolicyCmd::DenyRules(action) => match action {
            DenyRulesAction::Add(a) => deny_rule_add(ctx, a).await,
            DenyRulesAction::Update(a) => deny_rule_update(ctx, a).await,
            DenyRulesAction::Delete(a) => deny_rule_delete(ctx, a).await,
        },
        PolicyCmd::Pending => pending(ctx).await,
        PolicyCmd::Provision(a) => provision(ctx, a).await,
    }
}

async fn rulesets(ctx: &Context, args: &RulesetsArgs) -> Result<(), CliError> {
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
    let c = ctx
        .pce
        .rule_sets(scope(args.active), ctx.cache_mode)
        .await?;
    let needle = args.name.as_deref().map(str::to_ascii_lowercase);
    let data: Vec<RuleSetView> = c
        .value
        .iter()
        .filter(|rs| {
            needle
                .as_deref()
                .is_none_or(|n| rs.name.to_ascii_lowercase().contains(n))
        })
        .map(|rs| RuleSetView::from_rule_set(rs, &idx))
        .collect();
    ctx.emit(&data, cache_label(c.hit, ctx.cache_mode), c.truncated, None)
}

fn find_ruleset<'a>(sets: &'a [RuleSet], needle: &str) -> Option<&'a RuleSet> {
    let n = needle.trim();
    sets.iter()
        .find(|rs| rs.href == n || rs.name.eq_ignore_ascii_case(n))
        .or_else(|| sets.iter().find(|rs| rs.href.ends_with(&format!("/{n}"))))
}

async fn rules(ctx: &Context, args: &RulesArgs) -> Result<(), CliError> {
    let Some(needle) = args.ruleset.as_deref() else {
        return Err(CliError::Args(
            "policy rules needs a ruleset name or href (or one of add, update, delete, impact)"
                .into(),
        ));
    };
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
    let c = ctx
        .pce
        .rule_sets(scope(args.active), ctx.cache_mode)
        .await?;
    let rs = find_ruleset(&c.value, needle).ok_or_else(|| {
        let names: Vec<&str> = c.value.iter().map(|r| r.name.as_str()).collect();
        CliError::Args(format!(
            "ruleset {needle:?} not found; known: {}",
            names.join(", ")
        ))
    })?;
    let deny = deny_rules_of(ctx, rs).await?;
    let views: Vec<RuleView> = rs
        .rules
        .iter()
        .map(|r| RuleView::from_rule(r, &idx))
        .chain(deny.iter().map(|d| RuleView::from_deny(d, &idx)))
        .collect();
    let mut truncated = c.truncated;
    let impacts: Vec<Option<Impact>> = if args.impact {
        let w = impact_workloads(ctx).await?;
        truncated |= w.truncated;
        let inputs = ImpactInputs {
            workloads: &w.value,
            index: idx.as_ref(),
            truncated: w.truncated,
        };
        // The workloads' label sets are built once for all the rules.
        let labels = workload_labels(&w.value);
        rs.rules
            .iter()
            .map(RuleActors::of_rule)
            .chain(deny.iter().map(RuleActors::of_deny))
            .map(|a| Some(rule_impact_with(&rs.scopes, &a, &inputs, &labels)))
            .collect()
    } else {
        vec![None; views.len()]
    };
    let out = RulesOutput {
        ruleset: RuleSetView::from_rule_set(rs, &idx),
        rules: views
            .into_iter()
            .zip(impacts)
            .map(|(rule, impact)| RuleOut { rule, impact })
            .collect(),
        with_impact: args.impact,
    };
    ctx.emit(&out, cache_label(c.hit, ctx.cache_mode), truncated, None)
}

/// A ruleset's deny rules: inline, or from the per-ruleset endpoint when the
/// inline list is empty (older PCEs; others have no such endpoint, a 404
/// there means none).
async fn deny_rules_of(ctx: &Context, rs: &RuleSet) -> Result<Vec<DenyRule>, CliError> {
    if !rs.deny_rules.is_empty() {
        return Ok(rs.deny_rules.clone());
    }
    match ctx.pce.deny_rules(&rs.href, ctx.cache_mode).await {
        Ok(d) => Ok(d.value),
        Err(PceError::NotFound { .. }) => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

/// The workload list behind `--impact`: one read, through the cache.
async fn impact_workloads(ctx: &Context) -> Result<Cached<Vec<Workload>>, CliError> {
    Ok(ctx
        .pce
        .workloads(&WorkloadFilter::default(), ctx.cache_mode)
        .await?)
}

/// `--impact` on a rule write: the impact of what the plan would send (its
/// body, an update's untouched sides from the loaded rule) in the target
/// ruleset's scopes over the workloads; the rulesets and the workloads are
/// cached reads, made after the guard and the plan. `None` for a plan with
/// nothing to send.
async fn planned_impact(ctx: &Context, plan: &Plan) -> Result<Option<(Impact, bool)>, CliError> {
    let Some(req) = plan.request() else {
        return Ok(None);
    };
    let not_found = |p: &str| {
        CliError::Pce(PceError::NotFound {
            path: p.to_string(),
        })
    };
    let rs_href = rule_set_href_of(&req.path).ok_or_else(|| not_found(&req.path))?;
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
    let sets = ctx
        .pce
        .rule_sets(PolicyScope::Draft, ctx.cache_mode)
        .await?;
    let rs = sets
        .value
        .iter()
        .find(|r| r.href == rs_href)
        .ok_or_else(|| not_found(rs_href))?;
    let base = if !is_rule_href(&req.path) {
        None
    } else {
        let found = if req.path.contains("/deny_rules/") {
            deny_rules_of(ctx, rs)
                .await?
                .iter()
                .find(|d| d.href == req.path)
                .map(RuleActors::of_deny)
        } else {
            rs.rules
                .iter()
                .find(|r| r.href == req.path)
                .map(RuleActors::of_rule)
        };
        // An update's untouched sides come from the loaded rule: without it
        // the impact would be computed on empty sides.
        Some(found.ok_or_else(|| not_found(&req.path))?)
    };
    let actors = RuleActors::planned(req.body.as_ref(), base.as_ref());
    let w = impact_workloads(ctx).await?;
    let inputs = ImpactInputs {
        workloads: &w.value,
        index: idx.as_ref(),
        truncated: w.truncated,
    };
    let impact = rule_impact(&rs.scopes, &actors, &inputs);
    Ok(Some((impact, sets.truncated || w.truncated)))
}

/// [`planned_impact`] when `--impact` was given, else `None` (no read).
async fn maybe_impact(
    ctx: &Context,
    impact: bool,
    plan: &Plan,
) -> Result<Option<(Impact, bool)>, CliError> {
    if impact {
        planned_impact(ctx, plan).await
    } else {
        Ok(None)
    }
}

/// `policy rules impact <href>`: one rule or deny rule of the draft
/// rulesets (an active href folds to draft) with its impact; exit 2 for an
/// href of the wrong shape, 4 when no such rule is loaded.
async fn rule_impact_one(ctx: &Context, a: &RuleImpactArgs) -> Result<(), CliError> {
    let child = if a.rule.contains("/deny_rules/") {
        "deny_rules"
    } else {
        "sec_rules"
    };
    let href = checked_nested_href(&a.rule, "rule_sets", child, ctx.config.org_id)?;
    let not_found = || CliError::Pce(PceError::NotFound { path: href.clone() });
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
    let c = ctx
        .pce
        .rule_sets(PolicyScope::Draft, ctx.cache_mode)
        .await?;
    let rs = rule_set_href_of(&href)
        .and_then(|h| c.value.iter().find(|r| r.href == h))
        .ok_or_else(not_found)?;
    let (rule, actors) = if child == "sec_rules" {
        let r = rs
            .rules
            .iter()
            .find(|r| r.href == href)
            .ok_or_else(not_found)?;
        (RuleView::from_rule(r, &idx), RuleActors::of_rule(r))
    } else {
        let deny = deny_rules_of(ctx, rs).await?;
        let d = deny.iter().find(|d| d.href == href).ok_or_else(not_found)?;
        (RuleView::from_deny(d, &idx), RuleActors::of_deny(d))
    };
    let w = impact_workloads(ctx).await?;
    let inputs = ImpactInputs {
        workloads: &w.value,
        index: idx.as_ref(),
        truncated: w.truncated,
    };
    let impact = rule_impact(&rs.scopes, &actors, &inputs);
    let out = RuleOut {
        rule,
        impact: Some(impact),
    };
    ctx.emit(
        &out,
        cache_label(c.hit, ctx.cache_mode),
        c.truncated || w.truncated,
        None,
    )
}

async fn pending(ctx: &Context) -> Result<(), CliError> {
    let c = ctx.pce.pending_changes(ctx.cache_mode).await?;
    ctx.emit(&c.value, cache_label(c.hit, ctx.cache_mode), false, None)
}

fn on_off(on: bool, off: bool) -> Option<bool> {
    if on {
        Some(true)
    } else if off {
        Some(false)
    } else {
        None
    }
}

/// Exactly one draft ruleset named `name` (case-insensitive); a pending
/// delete does not count unless `with_deleted` (provisioning that delete).
fn ruleset_named(sets: &[RuleSet], name: &str, with_deleted: bool) -> Result<String, CliError> {
    let live = || {
        sets.iter()
            .filter(move |rs| with_deleted || rs.update_type.as_deref() != Some("delete"))
    };
    let hits: Vec<&str> = live()
        .filter(|rs| rs.name.eq_ignore_ascii_case(name))
        .map(|rs| rs.href.as_str())
        .collect();
    match hits.as_slice() {
        [one] => Ok(one.to_string()),
        [] => Err(CliError::Args(write::with_similar(
            format!("no ruleset named {name:?}"),
            &write::similar(name, live().map(|rs| rs.name.as_str())),
        ))),
        many => Err(CliError::Args(format!(
            "{name:?} names several rulesets: {}; use the href",
            many.join(", ")
        ))),
    }
}

/// An href as given (the planner checks it, no request), or a name
/// resolved with one draft ruleset list GET.
async fn ruleset_href(ctx: &Context, needle: &str) -> Result<String, CliError> {
    if is_href(needle) {
        return Ok(needle.trim().to_string());
    }
    let sets = ctx
        .pce
        .rule_sets(PolicyScope::Draft, ctx.cache_mode)
        .await?
        .value;
    ruleset_named(&sets, needle.trim(), false)
}

/// The library's own messages for a rule with an empty side, raised before
/// the ruleset name or any token costs a request.
fn require_sides(sides: &RuleSides) -> Result<(), CliError> {
    let mut errs = Vec::new();
    if sides.providers.is_empty() {
        errs.push("a rule needs at least one provider".to_string());
    }
    if sides.consumers.is_empty() {
        errs.push("a rule needs at least one consumer".to_string());
    }
    if sides.services.is_empty() {
        errs.push("a rule needs at least one service".to_string());
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(WriteError::Invalid(errs).into())
    }
}

fn resolve_labels_as(providers: &[String], consumers: &[String]) -> Option<ResolveLabelsAs> {
    if providers.is_empty() && consumers.is_empty() {
        return None;
    }
    let side = |v: &[String]| {
        if v.is_empty() {
            vec!["workloads".to_string()]
        } else {
            v.to_vec()
        }
    };
    Some(ResolveLabelsAs {
        providers: side(providers),
        consumers: side(consumers),
    })
}

struct ParsedSides {
    providers: Vec<ActorToken>,
    consumers: Vec<ActorToken>,
    services: Vec<ServiceToken>,
}

impl RuleSides {
    fn parse(&self) -> Result<ParsedSides, CliError> {
        require_sides(self)?;
        Ok(ParsedSides {
            providers: parse_actors(&self.providers)?,
            consumers: parse_actors(&self.consumers)?,
            services: parse_services(&self.services)?,
        })
    }
}

struct ParsedEdits {
    add_providers: Vec<ActorToken>,
    remove_providers: Vec<ActorToken>,
    add_consumers: Vec<ActorToken>,
    remove_consumers: Vec<ActorToken>,
    add_services: Vec<ServiceToken>,
    remove_services: Vec<ServiceToken>,
}

struct ResolvedEdits {
    add_providers: Vec<ActorRef>,
    remove_providers: Vec<ActorRef>,
    add_consumers: Vec<ActorRef>,
    remove_consumers: Vec<ActorRef>,
    add_services: Vec<ServiceRef>,
    remove_services: Vec<ServiceRef>,
}

impl SideEdits {
    fn is_empty(&self) -> bool {
        self.add_providers.is_empty()
            && self.remove_providers.is_empty()
            && self.add_consumers.is_empty()
            && self.remove_consumers.is_empty()
            && self.add_services.is_empty()
            && self.remove_services.is_empty()
    }

    fn parse(&self) -> Result<ParsedEdits, CliError> {
        Ok(ParsedEdits {
            add_providers: parse_actors(&self.add_providers)?,
            remove_providers: parse_actors(&self.remove_providers)?,
            add_consumers: parse_actors(&self.add_consumers)?,
            remove_consumers: parse_actors(&self.remove_consumers)?,
            add_services: parse_services(&self.add_services)?,
            remove_services: parse_services(&self.remove_services)?,
        })
    }
}

impl ParsedEdits {
    async fn resolve(
        &self,
        ctx: &Context,
        lists: &mut Lists<'_>,
    ) -> Result<ResolvedEdits, CliError> {
        Ok(ResolvedEdits {
            add_providers: resolve_actors(ctx, &self.add_providers, lists).await?,
            remove_providers: resolve_actors(ctx, &self.remove_providers, lists).await?,
            add_consumers: resolve_actors(ctx, &self.add_consumers, lists).await?,
            remove_consumers: resolve_actors(ctx, &self.remove_consumers, lists).await?,
            add_services: resolve_services(ctx, &self.add_services, lists).await?,
            remove_services: resolve_services(ctx, &self.remove_services, lists).await?,
        })
    }
}

const NOTHING_FOR_RULE: &str = "nothing to change; pass --description, --enabled, --disabled, --add-provider, --remove-provider, --add-consumer, --remove-consumer, --add-service, --remove-service";

async fn ruleset_add(ctx: &Context, a: &RulesetAddArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    // The name errors need no list: an empty name costs no request.
    RuleSetDraft {
        name: a.name.clone(),
        description: None,
        enabled: true,
        scopes: Vec::new(),
    }
    .validate()
    .map_err(WriteError::Invalid)?;
    let scopes = parse_scopes(&a.scopes)?;
    let mut lists = Lists::new(&guard);
    let draft = RuleSetDraft {
        name: a.name.clone(),
        description: a.description.clone(),
        enabled: !a.disabled,
        scopes: resolve_scopes(ctx, &scopes, &mut lists).await?,
    };
    let if_missing = if a.if_missing {
        IfMissing::SkipExisting
    } else {
        IfMissing::Create
    };
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_create_rule_set(&draft, if_missing)
        .await?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn ruleset_update(ctx: &Context, a: &RulesetUpdateArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let enabled = on_off(a.enabled, a.disabled);
    if a.name.is_none()
        && a.description.is_none()
        && enabled.is_none()
        && a.scopes.is_empty()
        && !a.clear_scopes
    {
        return Err(CliError::Args(
            "nothing to change; pass --name, --description, --enabled, --disabled, --scope or --clear-scopes".into(),
        ));
    }
    RuleSetPatch {
        name: a.name.clone(),
        ..RuleSetPatch::default()
    }
    .validate()
    .map_err(WriteError::Invalid)?;
    let scopes = parse_scopes(&a.scopes)?;
    let href = ruleset_href(ctx, &a.ruleset).await?;
    let mut lists = Lists::new(&guard);
    let set_scopes = if a.clear_scopes {
        Some(Vec::new())
    } else if scopes.is_empty() {
        None
    } else {
        Some(resolve_scopes(ctx, &scopes, &mut lists).await?)
    };
    let patch = RuleSetPatch {
        name: a.name.clone(),
        description: a.description.clone(),
        enabled,
        set_scopes,
    };
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_update_rule_set(&href, &patch)
        .await?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn ruleset_delete(ctx: &Context, a: &RulesetDeleteArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let href = ruleset_href(ctx, &a.ruleset).await?;
    let plan = ctx.pce.writer(&guard).plan_delete_rule_set(&href)?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn rule_add(ctx: &Context, a: &RuleAddArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let sides = a.sides.parse()?;
    let ruleset = ruleset_href(ctx, &a.ruleset).await?;
    let mut lists = Lists::new(&guard);
    let draft = RuleDraft {
        description: a.description.clone(),
        enabled: !a.disabled,
        providers: resolve_actors(ctx, &sides.providers, &mut lists).await?,
        consumers: resolve_actors(ctx, &sides.consumers, &mut lists).await?,
        services: resolve_services(ctx, &sides.services, &mut lists).await?,
        unscoped_consumers: a.unscoped_consumers,
        resolve_labels_as: resolve_labels_as(&a.resolve_providers_as, &a.resolve_consumers_as),
    };
    let plan = ctx.pce.writer(&guard).plan_create_rule(&ruleset, &draft)?;
    let impact = maybe_impact(ctx, a.impact, &plan).await?;
    run_rule_plan(ctx, &a.write, &guard, plan, impact).await
}

async fn rule_update(ctx: &Context, a: &RuleUpdateArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let href = checked_nested_href(&a.rule, "rule_sets", "sec_rules", ctx.config.org_id)?;
    let enabled = on_off(a.enabled, a.disabled);
    let unscoped = on_off(a.unscoped_consumers, a.scoped_consumers);
    if a.description.is_none() && enabled.is_none() && unscoped.is_none() && a.edits.is_empty() {
        return Err(CliError::Args(format!(
            "{NOTHING_FOR_RULE}, --unscoped-consumers or --scoped-consumers"
        )));
    }
    let edits = a.edits.parse()?;
    let mut lists = Lists::new(&guard);
    let e = edits.resolve(ctx, &mut lists).await?;
    let patch = RulePatch {
        description: a.description.clone(),
        enabled,
        add_providers: e.add_providers,
        remove_providers: e.remove_providers,
        add_consumers: e.add_consumers,
        remove_consumers: e.remove_consumers,
        add_services: e.add_services,
        remove_services: e.remove_services,
        unscoped_consumers: unscoped,
        resolve_labels_as: None,
    };
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_update_rule(&href, &patch)
        .await?;
    let impact = maybe_impact(ctx, a.impact, &plan).await?;
    run_rule_plan(ctx, &a.write, &guard, plan, impact).await
}

async fn rule_delete(ctx: &Context, a: &RuleDeleteArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let plan = ctx.pce.writer(&guard).plan_delete_rule(&a.rule)?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn deny_rule_add(ctx: &Context, a: &DenyRuleAddArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let sides = a.sides.parse()?;
    let ruleset = ruleset_href(ctx, &a.ruleset).await?;
    let mut lists = Lists::new(&guard);
    let draft = DenyRuleDraft {
        description: a.description.clone(),
        enabled: !a.disabled,
        providers: resolve_actors(ctx, &sides.providers, &mut lists).await?,
        consumers: resolve_actors(ctx, &sides.consumers, &mut lists).await?,
        services: resolve_services(ctx, &sides.services, &mut lists).await?,
        override_: a.override_,
    };
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_create_deny_rule(&ruleset, &draft)?;
    let impact = maybe_impact(ctx, a.impact, &plan).await?;
    run_rule_plan(ctx, &a.write, &guard, plan, impact).await
}

async fn deny_rule_update(ctx: &Context, a: &DenyRuleUpdateArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let href = checked_nested_href(&a.deny_rule, "rule_sets", "deny_rules", ctx.config.org_id)?;
    let enabled = on_off(a.enabled, a.disabled);
    let override_ = on_off(a.override_, a.no_override);
    if a.description.is_none() && enabled.is_none() && override_.is_none() && a.edits.is_empty() {
        return Err(CliError::Args(format!(
            "{NOTHING_FOR_RULE}, --override or --no-override"
        )));
    }
    let edits = a.edits.parse()?;
    let mut lists = Lists::new(&guard);
    let e = edits.resolve(ctx, &mut lists).await?;
    let patch = DenyRulePatch {
        description: a.description.clone(),
        enabled,
        add_providers: e.add_providers,
        remove_providers: e.remove_providers,
        add_consumers: e.add_consumers,
        remove_consumers: e.remove_consumers,
        add_services: e.add_services,
        remove_services: e.remove_services,
        override_,
    };
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_update_deny_rule(&href, &patch)
        .await?;
    let impact = maybe_impact(ctx, a.impact, &plan).await?;
    run_rule_plan(ctx, &a.write, &guard, plan, impact).await
}

async fn deny_rule_delete(ctx: &Context, a: &DenyRuleDeleteArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let plan = ctx.pce.writer(&guard).plan_delete_deny_rule(&a.deny_rule)?;
    run_plan(ctx, &a.write, &guard, plan).await
}

/// Hrefs as given (the planner checks them), paired with what the user
/// typed; names resolved as rulesets (pending deletes included, the
/// planner's pending check still guards) with one draft list GET, made only
/// when a name is given.
async fn provision_targets(
    ctx: &Context,
    targets: &[String],
) -> Result<Vec<(String, String)>, CliError> {
    let sets = if targets.iter().all(|t| is_href(t)) {
        Vec::new()
    } else {
        ctx.pce
            .rule_sets(PolicyScope::Draft, ctx.cache_mode)
            .await?
            .value
    };
    targets
        .iter()
        .map(|t| {
            let href = if is_href(t) {
                t.trim().to_string()
            } else {
                ruleset_named(&sets, t.trim(), true)?
            };
            Ok((t.trim().to_string(), href))
        })
        .collect()
}

/// `<href> has no pending changes` becomes `<typed> (<href>) ...` when the
/// target was given by name.
fn name_typed_target(msg: String, typed: &[(String, String)]) -> String {
    typed
        .iter()
        .find(|(t, h)| t != h && msg == format!("{h} has no pending changes"))
        .map(|(t, h)| format!("{t} ({h}) has no pending changes"))
        .unwrap_or(msg)
}

async fn provision(ctx: &Context, a: &ProvisionArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let mut typed = Vec::new();
    let subset = if a.all {
        ProvisionSubset::All
    } else {
        let pairs = provision_targets(ctx, &a.targets).await?;
        let hrefs = pairs.iter().map(|(_, h)| h.clone()).collect();
        typed = pairs;
        ProvisionSubset::Hrefs(hrefs)
    };
    let req = ProvisionRequest {
        description: a.description.clone(),
        subset,
    };
    let plan = match ctx.pce.writer(&guard).plan_provision(&req).await {
        Ok(p) => p,
        // Name what the user typed alongside the href.
        Err(WriteError::Invalid(errs)) => {
            return Err(WriteError::Invalid(
                errs.into_iter()
                    .map(|m| name_typed_target(m, &typed))
                    .collect(),
            )
            .into());
        }
        Err(e) => return Err(e.into()),
    };
    write::run_provision_plan(ctx, &a.write, &guard, plan).await
}
