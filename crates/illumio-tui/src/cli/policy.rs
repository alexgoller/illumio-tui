//! Policy: rulesets (list; add, update, delete), the rules of one ruleset
//! (list; add, update, delete allow rules), deny rules, and pending
//! provisioning changes (list, provision, revert).
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{MatchedMeta, TableRows, join_list};
use super::write::{
    self, ActorToken, Lists, ServiceToken, WriteFlags, is_href, parse_actors, parse_scopes,
    parse_services, resolve_actors, resolve_scopes, resolve_services, run_plan, run_rule_plan,
};
use crate::tui::diff::{Sign, diff_object_kind, load_diff};
use clap::{Args, Subcommand};
use pce::impact::{
    Impact, ImpactInputs, RuleActors, Side, is_rule_href, rule_impact, rule_impact_with,
    rule_matches, rule_set_href_of, workload_labels,
};
use pce::models::{
    Actor, DenyRule, PendingChange, PolicyScope, ResolveLabelsAs, RuleSet, Workload, WorkloadFilter,
};
use pce::write::{
    ActorRef, DenyRuleDraft, DenyRulePatch, IfMissing, ObjectKind, Plan, ProvisionRequest,
    ProvisionSubset, RevertRequest, RuleDraft, RulePatch, RuleSetDraft, RuleSetPatch, ServiceRef,
    WriteError, checked_nested_href,
};
use pce::{CacheMode, Cached, PceError, RuleKind, RuleSetView, RuleView};
use serde::Serialize;
use std::ffi::OsString;

#[derive(Subcommand, Debug)]
pub enum PolicyCmd {
    /// List rulesets with resolved scopes; add, update or delete one
    Rulesets(RulesetsCmd),
    /// List the rules of one ruleset (name or href); add, update or delete an allow rule
    Rules(RulesCmd),
    /// Add, update or delete a deny rule
    #[command(subcommand)]
    DenyRules(DenyRulesAction),
    /// Find rules across all rulesets by label, IP list, service, workload, href, state or kind
    Search(SearchArgs),
    /// List changes waiting to be provisioned
    Pending(PendingArgs),
    /// What provisioning the draft would change: pending objects, rules and their coverage,
    /// traffic that would become blocked or allowed (read-only)
    ///
    /// Three sections. Changes: the pending objects by type and change type. Rules: per
    /// pending ruleset its rules added, removed and changed (draft against active, by href),
    /// each with the workloads it matches active → draft (providers 12→40 · consumers 3→3;
    /// the active side through the active label groups and IP lists; - where not in force).
    /// Traffic: one Explorer query over --window re-evaluated against the draft
    /// (update_rules; it changes no policy): flow records reported → draft, and the app/env
    /// group pairs whose decision gets stricter (allowed → potentially blocked or blocked,
    /// potentially blocked → blocked; first), then those the draft allows (blocked or
    /// potentially blocked → allowed; blocked → potentially blocked is in neither). The PCE
    /// evaluates the whole draft, so this always covers everything pending. Requests: the
    /// pending list; nothing else when nothing is pending; else the object lists, the draft
    /// and active rulesets, active label groups and IP lists and one workloads read (all
    /// cached; the rulesets, groups, lists and workloads only when a ruleset is pending), then
    /// 1 POST, status polls, 1 PUT update_rules, polls, 1 download. --csv: the pair rows.
    /// --fail-on-blocked: exit 7 when a flow would become blocked or potentially blocked
    Impact(super::policy_impact::ImpactArgs),
    /// Provision pending changes (needs writes = true in the profile)
    Provision(ProvisionArgs),
    /// Revert pending changes to their active version; draft-only objects are deleted (needs writes = true in the profile)
    Revert(RevertArgs),
}

/// `policy search`: the TUI's rule search (`f` on Policy) over one ruleset
/// list read (cached).
#[derive(Args, Debug)]
pub struct SearchArgs {
    /// Search terms as in the TUI (all must hold): app=shop,crm '!env=prod' ipl:NAME svc:NAME svc:443/tcp svc:tcp wl:NAME href:TEXT disabled enabled deny allow intra extra (allow and deny rules); scope:, src: (consumer:) or dst: (provider:) before a label (src:/dst: also ipl:NAME, wl:NAME, all; scope:global); a leading - negates a term (-deny, -scope:env=dev). Examples: `src:role=web dst:role=db extra`, `scope:app=payment -svc:443/tcp`, `'src:!env=prod'` (quote a `!` for the shell), `src:all -deny`. A -term needs no `--`; after `--` every word is a term
    pub terms: Vec<String>,
    /// A label in a scope, consumer or provider (directly or in a label group; exclusions and All workloads do not count): key=value, a comma lists values any of which matches (repeatable; all must hold). For exclusions use the term !key=value
    #[arg(long = "label")]
    pub labels: Vec<String>,
    /// An IP list consumer or provider, by name: exact, else the start of exactly one name, any case (repeatable)
    #[arg(long = "ip-list")]
    pub ip_lists: Vec<String>,
    /// A service object by name (exact, else a unique prefix), a port that a rule's service holds: PORT/PROTO or PORT (tcp or udp), or a protocol: tcp, udp, icmp, icmpv6 (repeatable)
    #[arg(long = "service")]
    pub services: Vec<String>,
    /// A workload consumer or provider, by hostname or name (one workloads read, cached) or href id (repeatable)
    #[arg(long = "workload")]
    pub workloads: Vec<String>,
    /// The rule's href contains this text
    #[arg(long)]
    pub href: Option<String>,
    /// Only disabled rules (or rules of disabled rulesets)
    #[arg(long)]
    pub disabled: bool,
    /// Only deny and override deny rules
    #[arg(long)]
    pub deny: bool,
    /// Search active policy instead of draft
    #[arg(long, conflicts_with = "draft")]
    pub active: bool,
    /// Search draft policy (the default; explicit for scripts)
    #[arg(long)]
    pub draft: bool,
    #[command(flatten)]
    pub saved: super::filters::SavedFilterArgs,
}

impl SearchArgs {
    /// The saved query `q` as the base: the terms and flags given hold too
    /// (AND).
    pub fn with_saved(&mut self, q: &str) -> Result<(), String> {
        pce::RuleFilter::parse(q).map_err(|e| format!("query: {e}"))?;
        self.terms.insert(0, q.to_string());
        Ok(())
    }

    /// The search the terms and flags make, as one query (the TUI's).
    pub fn to_saved(&self) -> Result<String, String> {
        search_filter(self)
            .map(|f| f.to_string())
            .map_err(|e| e.to_string())
    }
}

#[derive(Args, Debug)]
pub struct PendingArgs {
    /// Under each change, show what it changes against the active version
    #[arg(long)]
    pub diff: bool,
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
    /// Create a ruleset (needs writes = true in the profile)
    Add(RulesetAddArgs),
    /// Change a ruleset's name, description, state or scopes
    Update(RulesetUpdateArgs),
    /// Delete a ruleset and its rules
    Delete(RulesetDeleteArgs),
    /// Enable a ruleset (a draft change; provision to apply it)
    Enable(RulesetToggleArgs),
    /// Disable a ruleset (a draft change; provision to apply it)
    Disable(RulesetToggleArgs),
}

#[derive(Args, Debug)]
pub struct RulesetToggleArgs {
    /// Ruleset name (case-insensitive) or href
    pub ruleset: String,
    #[command(flatten)]
    pub write: WriteFlags,
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
    /// Create an allow rule in a ruleset (needs writes = true in the profile)
    Add(RuleAddArgs),
    /// Change an allow rule, addressed by href
    Update(RuleUpdateArgs),
    /// Delete an allow rule, addressed by href
    Delete(RuleDeleteArgs),
    /// Report one rule's impact (workloads matched as providers and consumers), addressed by href
    Impact(RuleImpactArgs),
    /// Enable an allow or deny rule, addressed by href (a draft change; provision to apply it)
    Enable(RuleToggleArgs),
    /// Disable an allow or deny rule, addressed by href (a draft change; provision to apply it)
    Disable(RuleToggleArgs),
}

#[derive(Args, Debug)]
pub struct RuleToggleArgs {
    /// Rule href: /orgs/N/sec_policy/draft/rule_sets/ID/sec_rules/ID (or deny_rules/ID)
    pub rule: String,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Subcommand, Debug)]
pub enum DenyRulesAction {
    /// Create a deny rule in a ruleset (needs writes = true in the profile)
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
    /// Extra-scope: consumers may come from outside the ruleset's scopes (default: intra-scope, consumers inside them)
    #[arg(long, visible_alias = "extra-scope")]
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
    /// Extra-scope: consumers may come from outside the ruleset's scopes
    #[arg(
        long,
        visible_alias = "extra-scope",
        conflicts_with = "scoped_consumers"
    )]
    pub unscoped_consumers: bool,
    /// Intra-scope: consumers must come from inside the ruleset's scopes
    #[arg(long, visible_alias = "intra-scope")]
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
    /// List the workloads matched on one side (hostname, labels, enforcement, online) instead of the counts
    #[arg(long, value_parser = ["providers", "consumers"])]
    pub list: Option<String>,
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
    /// Also report what provisioning the draft would change (policy impact over the
    /// profile's default window), computed after the plan and before anything is sent; in a
    /// table it is printed in front of the request. It covers the whole draft, targets or not
    #[arg(long)]
    pub impact: bool,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct RevertArgs {
    /// Revert every pending change (each listed in the request)
    #[arg(long, conflicts_with = "targets", required_unless_present = "targets")]
    pub all: bool,
    /// Ruleset names, or hrefs of pending rulesets, rules, IP lists, services or label groups
    pub targets: Vec<String>,
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
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .zip(self.rows())
            .map(|(r, mut row)| {
                row[2] = join_list(&r.scopes);
                row
            })
            .collect()
    }
}

fn names<'a>(it: impl Iterator<Item = &'a str>) -> String {
    it.collect::<Vec<_>>().join(", ")
}

/// `allow`, `deny`, `override_deny`, as in JSON.
pub(crate) fn kind_text(kind: RuleKind) -> String {
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
    /// Plus each rule's `href`, which the terminal has no room for.
    fn full_headers(&self) -> Vec<&'static str> {
        let mut h = self.headers();
        h.push("href");
        h
    }
    /// Actors and services one list entry each; no `-` for a rule
    /// without impact; the rule's href last.
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.rules
            .iter()
            .zip(self.rows())
            .map(|(o, mut row)| {
                let r = &o.rule;
                row[2] = join_list(r.consumers.iter().map(|a| &a.name));
                row[3] = join_list(r.providers.iter().map(|a| &a.name));
                row[4] = join_list(r.services.iter().map(|s| &s.name));
                if self.with_impact {
                    row[5] = o.impact.as_ref().map(Impact::pc).unwrap_or_default();
                }
                row.push(r.href.clone());
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

pub async fn run(ctx: &Context, cmd: &PolicyCmd) -> Result<(), CliError> {
    match cmd {
        PolicyCmd::Rulesets(c) => match &c.action {
            None => rulesets(ctx, &c.list).await,
            Some(RulesetsAction::Add(a)) => ruleset_add(ctx, a).await,
            Some(RulesetsAction::Update(a)) => ruleset_update(ctx, a).await,
            Some(RulesetsAction::Delete(a)) => ruleset_delete(ctx, a).await,
            Some(RulesetsAction::Enable(a)) => ruleset_toggle(ctx, a, true).await,
            Some(RulesetsAction::Disable(a)) => ruleset_toggle(ctx, a, false).await,
        },
        PolicyCmd::Rules(c) => match &c.action {
            None => rules(ctx, &c.list).await,
            Some(RulesAction::Add(a)) => rule_add(ctx, a).await,
            Some(RulesAction::Update(a)) => rule_update(ctx, a).await,
            Some(RulesAction::Delete(a)) => rule_delete(ctx, a).await,
            Some(RulesAction::Impact(a)) => rule_impact_one(ctx, a).await,
            Some(RulesAction::Enable(a)) => rule_toggle(ctx, a, true).await,
            Some(RulesAction::Disable(a)) => rule_toggle(ctx, a, false).await,
        },
        PolicyCmd::DenyRules(action) => match action {
            DenyRulesAction::Add(a) => deny_rule_add(ctx, a).await,
            DenyRulesAction::Update(a) => deny_rule_update(ctx, a).await,
            DenyRulesAction::Delete(a) => deny_rule_delete(ctx, a).await,
        },
        PolicyCmd::Search(a) => search(ctx, a).await,
        PolicyCmd::Pending(a) => pending(ctx, a).await,
        PolicyCmd::Impact(a) => super::policy_impact::run(ctx, a).await,
        PolicyCmd::Provision(a) => provision(ctx, a).await,
        PolicyCmd::Revert(a) => revert(ctx, a).await,
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

/// One workload `policy rules impact --list` matched.
#[derive(Debug, Serialize)]
pub struct MatchedWorkload {
    pub href: String,
    pub hostname: Option<String>,
    pub name: Option<String>,
    /// `key=value` per label, keys in the profile's order.
    pub labels: Vec<String>,
    /// The effective enforcement: `unmanaged` without a VEN.
    pub enforcement: String,
    pub online: Option<bool>,
}

impl MatchedWorkload {
    fn new(w: &Workload, order: &pce::KeyOrder) -> Self {
        Self {
            href: w.href.clone(),
            hostname: w.hostname.clone(),
            name: w.name.clone(),
            labels: w.label_entries(order),
            enforcement: w.enforcement().unwrap_or_default().to_string(),
            online: w.online,
        }
    }
}

/// `policy rules impact <href> --list <side>`: that side's workloads, and
/// what the list cannot show (`note`: IP lists, virtual services).
#[derive(Debug, Serialize)]
pub struct MatchedOut {
    pub rule: String,
    pub side: &'static str,
    pub count: usize,
    pub note: Option<String>,
    /// The workload read was cut short: a lower bound.
    pub truncated: bool,
    pub workloads: Vec<MatchedWorkload>,
}

impl TableRows for MatchedOut {
    fn headers(&self) -> Vec<&'static str> {
        vec!["hostname", "name", "labels", "enforcement", "online"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.workloads
            .iter()
            .map(|w| {
                vec![
                    w.hostname.clone().unwrap_or_default(),
                    w.name.clone().unwrap_or_default(),
                    w.labels.join(" "),
                    w.enforcement.clone(),
                    w.online.map(|o| o.to_string()).unwrap_or_default(),
                ]
            })
            .collect()
    }
    fn full_headers(&self) -> Vec<&'static str> {
        let mut h = self.headers();
        h.push("href");
        h
    }
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.workloads
            .iter()
            .zip(self.rows())
            .map(|(w, mut row)| {
                row[2] = join_list(&w.labels);
                row.push(w.href.clone());
                row
            })
            .collect()
    }
}

/// One rule `policy search` found, with its ruleset.
#[derive(Debug, Serialize)]
pub struct SearchRow {
    pub ruleset: String,
    pub ruleset_href: String,
    pub ruleset_enabled: bool,
    /// The ruleset's scopes, one string each (`app=shop & env=prod`, `All`).
    pub scopes: Vec<String>,
    #[serde(flatten)]
    pub rule: RuleView,
}

impl TableRows for Vec<SearchRow> {
    fn headers(&self) -> Vec<&'static str> {
        vec![
            "ruleset",
            "kind",
            "enabled",
            "scope",
            "consumers",
            "providers",
            "services",
        ]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .map(|o| {
                let r = &o.rule;
                vec![
                    o.ruleset.clone(),
                    kind_text(r.kind),
                    crate::tui::model::enabled_text(r.enabled, o.ruleset_enabled, false),
                    o.scopes.join(" | "),
                    names(r.consumers.iter().map(|a| a.name.as_str())),
                    names(r.providers.iter().map(|a| a.name.as_str())),
                    names(r.services.iter().map(|s| s.name.as_str())),
                ]
            })
            .collect()
    }
    /// Plus the rule's href; lists one entry each.
    fn full_headers(&self) -> Vec<&'static str> {
        let mut h = self.headers();
        h.push("href");
        h
    }
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .zip(self.rows())
            .map(|(o, mut row)| {
                let r = &o.rule;
                row[3] = join_list(&o.scopes);
                row[4] = join_list(r.consumers.iter().map(|a| &a.name));
                row[5] = join_list(r.providers.iter().map(|a| &a.name));
                row[6] = join_list(r.services.iter().map(|s| &s.name));
                row.push(r.href.clone());
                row
            })
            .collect()
    }
}

/// `policy search -deny …`: clap reads a word that starts with `-` as a
/// flag, and a positional that takes such words swallows every flag after
/// its first one. So the words of `policy search` that start with one `-`
/// (not `-h`, not `-` alone, not the value of a flag written without `=`)
/// move behind a `--` (the one given, else a new one at the end), where
/// they are terms; terms AND, so their order does not matter. The words
/// are walked as clap would (global flags and their values may sit before
/// or between `policy` and `search`); every other command line is left as
/// it is.
pub fn negated_terms_last(mut args: Vec<OsString>) -> Vec<OsString> {
    use clap::CommandFactory;
    if !args.iter().any(|a| a == "search") {
        return args;
    }
    let mut cmd = super::Cli::command();
    cmd.build();
    let Some(start) = search_start(&cmd, &args) else {
        return args;
    };
    let search = cmd
        .find_subcommand("policy")
        .and_then(|p| p.find_subcommand("search"))
        .expect("policy search exists");
    let mut rest = args.split_off(start);
    // After `--`, the words are terms already.
    let tail = match rest.iter().position(|a| a == "--") {
        Some(i) => rest.split_off(i),
        None => Vec::new(),
    };
    let (mut terms, mut kept) = (Vec::new(), Vec::new());
    let mut value_next = false;
    for a in rest {
        let text = a.to_str().unwrap_or_default();
        let negated = !value_next
            && text.len() > 1
            && text.starts_with('-')
            && !text.starts_with("--")
            && text != "-h";
        value_next = !value_next && flag_takes_value(search, text);
        if negated {
            terms.push(a);
        } else {
            kept.push(a);
        }
    }
    args.extend(kept);
    if !terms.is_empty() && tail.is_empty() {
        args.push("--".into());
    }
    let mut tail = tail.into_iter();
    args.extend(tail.next());
    args.extend(terms);
    args.extend(tail);
    args
}

/// Whether `word` is a long flag of `cmd` (globals included, once built)
/// written without `=` whose value is the next word.
fn flag_takes_value(cmd: &clap::Command, word: &str) -> bool {
    let Some(long) = word
        .strip_prefix("--")
        .filter(|l| !l.is_empty() && !l.contains('='))
    else {
        return false;
    };
    cmd.get_arguments()
        .find(|a| a.get_long() == Some(long))
        .is_some_and(|a| a.get_action().takes_values())
}

/// Where the words of `policy search` start in `args` (after the program
/// name), walking subcommands as clap does and skipping flags and the
/// values of those that take one; `None` for any other command line.
fn search_start(root: &clap::Command, args: &[OsString]) -> Option<usize> {
    let mut cmd = root;
    let mut path = Vec::new();
    let mut words = args.iter().enumerate().skip(1);
    while let Some((i, a)) = words.next() {
        let text = a.to_str()?;
        if text == "--" {
            return None;
        }
        if text.starts_with('-') {
            if flag_takes_value(cmd, text) {
                words.next();
            }
            continue;
        }
        cmd = cmd.find_subcommand(text)?;
        path.push(text);
        if path == ["policy", "search"] {
            return Some(i + 1);
        }
    }
    None
}

/// The search the flags and terms make; exit 2 for a malformed one or none.
fn search_filter(a: &SearchArgs) -> Result<pce::RuleFilter, CliError> {
    use pce::RuleTerm;
    let mut terms = Vec::new();
    for t in &a.terms {
        terms.extend(pce::RuleFilter::parse(t).map_err(CliError::Args)?.terms);
    }
    for l in &a.labels {
        match RuleTerm::parse(l).map_err(CliError::Args)? {
            RuleTerm::Label(t) if t.negate => {
                return Err(CliError::Args(format!(
                    "--label {l:?}: expected key=value (the term {l} finds exclusions)"
                )));
            }
            t @ RuleTerm::Label(_) => terms.push(t),
            _ => return Err(CliError::Args(format!("--label {l:?}: expected key=value"))),
        }
    }
    terms.extend(a.ip_lists.iter().map(|n| RuleTerm::IpList(n.clone())));
    terms.extend(a.services.iter().map(|n| RuleTerm::service(n)));
    terms.extend(a.workloads.iter().map(|n| RuleTerm::Workload(n.clone())));
    terms.extend(a.href.iter().map(|h| RuleTerm::Href(h.clone())));
    if a.disabled {
        terms.push(RuleTerm::Disabled);
    }
    if a.deny {
        terms.push(RuleTerm::Deny);
    }
    if terms.is_empty() {
        return Err(CliError::Args(
            "policy search needs a term: --label, --ip-list, --service, --workload, --href, --disabled, --deny, or a TUI search term".into(),
        ));
    }
    Ok(pce::RuleFilter { terms })
}

/// `policy search`: the rules of every ruleset (one list read, cached;
/// deny rules as the list carries them) that hold every term. Names the
/// index does not know are exit 2. A `--workload` name costs one cached
/// workloads read.
async fn search(ctx: &Context, a: &SearchArgs) -> Result<(), CliError> {
    let filter = search_filter(a)?;
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
    // Names made the objects' own: exact, else a unique prefix (`--ip-list
    // Any`); none or several is exit 2, with the closest or the matches.
    let (filter, unknown) = filter.resolve(&idx);
    if !unknown.is_empty() {
        return Err(CliError::Args(unknown.join("; ")));
    }
    let c = ctx.pce.rule_sets(scope(a.active), ctx.cache_mode).await?;
    let (workloads, mut truncated) = if filter.names_workloads() {
        let w = impact_workloads(ctx).await?;
        (w.value, w.truncated)
    } else {
        (Vec::new(), false)
    };
    truncated |= c.truncated;
    let mut rows = Vec::new();
    let mut loaded = 0;
    for rs in &c.value {
        loaded += rs.rules.len() + rs.deny_rules.len();
        let view = RuleSetView::from_rule_set(rs, &idx);
        let hit = |r: pce::RuleRef| filter.matches(&r, &idx, &workloads);
        let mut mine: Vec<RuleView> = rs
            .rules
            .iter()
            .filter(|r| hit(pce::RuleRef::of_rule(r, rs)))
            .map(|r| RuleView::from_rule(r, &idx))
            .chain(
                rs.deny_rules
                    .iter()
                    .filter(|d| hit(pce::RuleRef::of_deny(d, rs)))
                    .map(|d| RuleView::from_deny(d, &idx)),
            )
            .collect();
        // Evaluation order: override deny, allow, deny.
        mine.sort_by_key(|r| match r.kind {
            RuleKind::OverrideDeny => 0,
            RuleKind::Allow => 1,
            RuleKind::Deny => 2,
        });
        rows.extend(mine.into_iter().map(|rule| SearchRow {
            ruleset: rs.name.clone(),
            ruleset_href: rs.href.clone(),
            ruleset_enabled: rs.enabled,
            scopes: view.scopes.clone(),
            rule,
        }));
    }
    let mut meta = ctx.meta(cache_label(c.hit, ctx.cache_mode), truncated, None);
    meta.matched = Some(MatchedMeta {
        matched: rows.len(),
        loaded,
    });
    let unlisted = deny_maybe_apart(&c.value);
    if unlisted > 0 {
        let w = format!(
            "{unlisted} ruleset{} may hold deny rules not loaded: this PCE lists none inline (policy rules <ruleset> reads them)",
            if unlisted == 1 { "" } else { "s" }
        );
        if ctx.format != super::output::OutputFormat::Json {
            eprintln!("warning: {w}");
        }
        meta.warnings = Some(vec![w]);
    }
    ctx.emit_meta(&rows, &meta)
}

/// The rulesets whose deny rules a search over the list cannot see: on a
/// PCE that lists none inline (older ones keep them at a per-ruleset
/// endpoint), every ruleset; where one ruleset carries them inline, an
/// empty list means none, so 0.
fn deny_maybe_apart(sets: &[RuleSet]) -> usize {
    if sets.iter().any(|rs| !rs.deny_rules.is_empty()) {
        0
    } else {
        sets.len()
    }
}

/// A ruleset's deny rules: inline, or from the per-ruleset endpoint when the
/// inline list is empty (older PCEs; others have no such endpoint, a 404
/// there means none).
pub(crate) async fn deny_rules_of(ctx: &Context, rs: &RuleSet) -> Result<Vec<DenyRule>, CliError> {
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
    if let Some(side) = a.list.as_deref() {
        let side = if side == "consumers" {
            Side::Consumer
        } else {
            Side::Provider
        };
        let (impact, matched) = rule_matches(&rs.scopes, &actors, &inputs);
        let (side_actors, as_workloads) = match side {
            Side::Provider => (&actors.providers, actors.providers_as_workloads),
            Side::Consumer => (&actors.consumers, actors.consumers_as_workloads),
        };
        let note = crate::tui::impact_list::side_note(side_actors, as_workloads, &idx, side);
        let order = ctx.config.key_order();
        let out = MatchedOut {
            rule: rule.href.clone(),
            side: crate::tui::impact_list::side_word(side),
            count: matched.side(side).len(),
            note,
            truncated: impact.truncated,
            workloads: matched
                .side(side)
                .iter()
                .map(|&i| MatchedWorkload::new(&w.value[i], &order))
                .collect(),
        };
        if ctx.format != super::output::OutputFormat::Json
            && let Some(n) = &out.note
        {
            eprintln!("note: {n}");
        }
        return ctx.emit(
            &out,
            cache_label(c.hit, ctx.cache_mode),
            c.truncated || w.truncated,
            None,
        );
    }
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

/// One pending change; with `--diff` also its diff lines (`diff` is left out
/// of the JSON otherwise, so the shape without the flag is unchanged).
#[derive(Serialize)]
struct PendingRow {
    #[serde(flatten)]
    change: PendingChange,
    #[serde(skip_serializing_if = "Option::is_none")]
    diff: Option<Vec<DiffRow>>,
}

#[derive(Serialize)]
struct DiffRow {
    sign: &'static str,
    text: String,
}

impl DiffRow {
    fn glyph(&self) -> &'static str {
        match self.sign {
            "added" => "+",
            "removed" => "-",
            _ => " ",
        }
    }
}

/// `policy pending` as printed: JSON is the rows themselves; `diff` says
/// whether `--diff` asked for each change's lines (the CSV `diff` column).
#[derive(Serialize)]
#[serde(transparent)]
struct PendingOut {
    rows: Vec<PendingRow>,
    #[serde(skip)]
    diff: bool,
}

impl TableRows for PendingOut {
    fn headers(&self) -> Vec<&'static str> {
        vec!["object_type", "change_type", "name", "href"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.rows
            .iter()
            .map(|p| {
                vec![
                    p.change.object_type.clone(),
                    p.change.change_type.clone(),
                    p.change.name.clone().unwrap_or_default(),
                    p.change.href.clone(),
                ]
            })
            .collect()
    }
    fn notes(&self) -> Vec<Vec<String>> {
        self.rows
            .iter()
            .map(|p| {
                p.diff
                    .iter()
                    .flatten()
                    .map(|d| format!("  {} {}", d.glyph(), d.text))
                    .collect()
            })
            .collect()
    }
    /// With `--diff`, a `diff` column in place of the lines under each row.
    fn full_headers(&self) -> Vec<&'static str> {
        let mut h = self.headers();
        if self.diff {
            h.push("diff");
        }
        h
    }
    /// The diff lines one per line of the cell (a quoted CSV field).
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.rows
            .iter()
            .zip(self.rows())
            .map(|(p, mut row)| {
                if self.diff {
                    let lines: Vec<String> = p
                        .diff
                        .iter()
                        .flatten()
                        .map(|d| format!("{} {}", d.glyph(), d.text))
                        .collect();
                    row.push(lines.join("\n"));
                }
                row
            })
            .collect()
    }
}

fn sign_name(s: Sign) -> &'static str {
    match s {
        Sign::Same => "same",
        Sign::Added => "added",
        Sign::Removed => "removed",
    }
}

async fn pending(ctx: &Context, args: &PendingArgs) -> Result<(), CliError> {
    let c = ctx.pce.pending_changes(ctx.cache_mode).await?;
    let cache = cache_label(c.hit, ctx.cache_mode);
    if !args.diff {
        let rows: Vec<PendingRow> = c
            .value
            .into_iter()
            .map(|change| PendingRow { change, diff: None })
            .collect();
        return ctx.emit(&PendingOut { rows, diff: false }, cache, false, None);
    }
    // The draft and active list of each kind present is read once, past the
    // cache when asked to; every change then compares from the cache.
    let mut kinds: Vec<ObjectKind> = Vec::new();
    for k in c
        .value
        .iter()
        .filter_map(|p| diff_object_kind(&p.object_type))
    {
        if !kinds.contains(&k) {
            kinds.push(k);
        }
    }
    let mut index = None;
    if kinds.contains(&ObjectKind::LabelGroup) || kinds.contains(&ObjectKind::RuleSet) {
        index = Some(ctx.pce.name_index(ctx.cache_mode).await?);
    }
    // This loop exists only so `--no-cache` refreshes each list once: the
    // index already read the draft label group, IP list and service lists, so
    // those are taken from the cache it just filled.
    let mode = ctx.cache_mode;
    let draft_mode = if index.is_some() {
        CacheMode::Use
    } else {
        mode
    };
    for kind in &kinds {
        match kind {
            ObjectKind::LabelGroup => {
                ctx.pce.label_groups(draft_mode).await?;
                ctx.pce.active_label_groups(mode).await?;
            }
            ObjectKind::IpList => {
                ctx.pce.ip_lists(draft_mode).await?;
                ctx.pce.active_ip_lists(mode).await?;
            }
            ObjectKind::Service => {
                ctx.pce.services(draft_mode).await?;
                ctx.pce.active_services(mode).await?;
            }
            ObjectKind::RuleSet => {
                ctx.pce.rule_sets(PolicyScope::Draft, mode).await?;
                ctx.pce.rule_sets(PolicyScope::Active, mode).await?;
            }
            _ => {}
        }
    }
    let mut rows = Vec::new();
    for change in c.value {
        let lines = match diff_object_kind(&change.object_type) {
            Some(kind) => load_diff(&ctx.pce, &change, kind, index.as_deref(), None)
                .await?
                .lines
                .into_iter()
                .map(|l| DiffRow {
                    sign: sign_name(l.sign),
                    text: l.text,
                })
                .collect(),
            None => vec![DiffRow {
                sign: "same",
                text: "(no diff)".into(),
            }],
        };
        rows.push(PendingRow {
            change,
            diff: Some(lines),
        });
    }
    ctx.emit(&PendingOut { rows, diff: true }, cache, false, None)
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

/// `policy rulesets enable|disable`: a `PUT` of `enabled` only; a ruleset
/// already in that state plans nothing.
async fn ruleset_toggle(ctx: &Context, a: &RulesetToggleArgs, on: bool) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let href = ruleset_href(ctx, &a.ruleset).await?;
    let patch = RuleSetPatch {
        enabled: Some(on),
        ..RuleSetPatch::default()
    };
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_update_rule_set(&href, &patch)
        .await?;
    run_plan(ctx, &a.write, &guard, plan).await
}

/// `policy rules enable|disable <href>`: an allow rule (`sec_rules`) or a
/// deny rule (`deny_rules`) by its href, a `PUT` of `enabled` only; one
/// already in that state plans nothing.
async fn rule_toggle(ctx: &Context, a: &RuleToggleArgs, on: bool) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let deny = a.rule.contains("/deny_rules/");
    let child = if deny { "deny_rules" } else { "sec_rules" };
    let href = checked_nested_href(&a.rule, "rule_sets", child, ctx.config.org_id)?;
    let w = ctx.pce.writer(&guard);
    let plan = if deny {
        let patch = DenyRulePatch {
            enabled: Some(on),
            ..DenyRulePatch::default()
        };
        w.plan_update_deny_rule(&href, &patch).await?
    } else {
        let patch = RulePatch {
            enabled: Some(on),
            ..RulePatch::default()
        };
        w.plan_update_rule(&href, &patch).await?
    };
    run_plan(ctx, &a.write, &guard, plan).await
}

/// A `rules add` ruleset: its href and, when it was found by name in the
/// draft rulesets read for that, its scopes (an href costs no read, so it
/// comes without them).
async fn rule_ruleset(
    ctx: &Context,
    needle: &str,
) -> Result<(String, Option<Vec<Vec<Actor>>>), CliError> {
    if is_href(needle) {
        return Ok((needle.trim().to_string(), None));
    }
    let sets = ctx
        .pce
        .rule_sets(PolicyScope::Draft, ctx.cache_mode)
        .await?
        .value;
    let href = ruleset_named(&sets, needle.trim(), false)?;
    let scopes = sets
        .iter()
        .find(|rs| rs.href == href)
        .map(|rs| rs.scopes.clone());
    Ok((href, scopes))
}

/// The Scope warnings of a new allow rule ([`crate::rule_scope::warnings`])
/// on stderr; never fatal. The scopes are named as the TUI names them
/// (`!` before an exclusion, `group:NAME`), from the labels and label
/// groups a token already read; what they did not read stays an href,
/// which compares as nothing, and a label group then pins no key (the
/// conservative reading). Without the scopes (a ruleset href) only the
/// extra-scope check runs. No request.
fn warn_rule_scope(ctx: &Context, a: &RuleAddArgs, scopes: Option<&[Vec<Actor>]>, lists: &Lists) {
    let groups = lists.loaded_label_groups().unwrap_or_default();
    let idx = pce::NameIndex::new(lists.loaded_labels().unwrap_or_default(), groups, &[], &[]);
    let scopes: Option<Vec<Vec<String>>> = scopes.map(|s| {
        s.iter()
            .map(|s| crate::tui::edit::actor_tokens(s, &idx))
            .collect()
    });
    let group_keys: Vec<(String, String)> = groups
        .iter()
        .filter(|g| !g.key.is_empty())
        .map(|g| (g.name.clone(), g.key.clone()))
        .collect();
    let m = crate::rule_scope::ScopeKeys {
        keys: &ctx.config.scope_keys,
        groups: &group_keys,
    };
    let scope = crate::rule_scope::RuleScope::from_unscoped(a.unscoped_consumers);
    for w in crate::rule_scope::warnings(&a.sides.consumers.join(", "), scope, m, scopes.as_deref())
    {
        eprintln!("warning: {w}");
    }
}

async fn rule_add(ctx: &Context, a: &RuleAddArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let sides = a.sides.parse()?;
    let (ruleset, scopes) = rule_ruleset(ctx, &a.ruleset).await?;
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
    warn_rule_scope(ctx, a, scopes.as_deref(), &lists);
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

/// What the user typed beside each href, and the hrefs, for a provision or
/// a revert of named targets.
async fn typed_targets(
    ctx: &Context,
    targets: &[String],
) -> Result<(Vec<(String, String)>, Vec<String>), CliError> {
    let pairs = provision_targets(ctx, targets).await?;
    let hrefs = pairs.iter().map(|(_, h)| h.clone()).collect();
    Ok((pairs, hrefs))
}

/// A planner's `Invalid` with each `<href> has no pending changes` naming
/// what the user typed.
fn with_typed(e: WriteError, typed: &[(String, String)]) -> CliError {
    match e {
        WriteError::Invalid(errs) => WriteError::Invalid(
            errs.into_iter()
                .map(|m| name_typed_target(m, typed))
                .collect(),
        )
        .into(),
        e => e.into(),
    }
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
    let (typed, subset) = if a.all {
        (Vec::new(), ProvisionSubset::All)
    } else {
        let (typed, hrefs) = typed_targets(ctx, &a.targets).await?;
        (typed, ProvisionSubset::Hrefs(hrefs))
    };
    let req = ProvisionRequest {
        description: a.description.clone(),
        subset,
    };
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_provision(&req)
        .await
        .map_err(|e| with_typed(e, &typed))?;
    let impact = match plan.request() {
        Some(req) if a.impact => Some(provision_impact(ctx, req, !a.all).await?),
        _ => None,
    };
    write::run_provision_plan(ctx, &a.write, &guard, plan, impact).await
}

/// `policy provision --impact`: the summary over the profile's default
/// window (the cost guard first), after the plan and before the send; with
/// targets the request's hrefs mark the changes, and a warning says it
/// covers the whole draft.
async fn provision_impact(
    ctx: &Context,
    req: &pce::write::WriteRequest,
    targets: bool,
) -> Result<write::ImpactOut, CliError> {
    use super::policy_impact::{fetch, impact_meta, impact_spec};
    let requested = targets.then(|| {
        crate::provision_impact::request_hrefs(
            req.body.as_ref().unwrap_or(&serde_json::Value::Null),
        )
    });
    let spec = impact_spec(ctx, None, false, requested)?;
    let loaded = fetch(ctx, &spec).await?;
    let meta = impact_meta(ctx, &loaded, &spec, targets);
    Ok(write::ImpactOut {
        report: loaded.report,
        meta,
    })
}

async fn revert(ctx: &Context, a: &RevertArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let (typed, targets) = if a.all {
        (Vec::new(), Vec::new())
    } else {
        typed_targets(ctx, &a.targets).await?
    };
    let req = RevertRequest {
        all: a.all,
        targets,
    };
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_revert(&req)
        .await
        .map_err(|e| with_typed(e, &typed))?;
    write::run_provision_plan(ctx, &a.write, &guard, plan, None).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn moved(line: &str) -> String {
        let args = line.split(' ').map(OsString::from).collect();
        let out: Vec<String> = negated_terms_last(args)
            .into_iter()
            .map(|a| a.into_string().unwrap())
            .collect();
        out.join(" ")
    }

    #[test]
    fn negated_terms_move_behind_the_escape() {
        assert_eq!(
            moved("x --json policy search -deny app=shop --active -scope:env=dev"),
            "x --json policy search app=shop --active -- -deny -scope:env=dev"
        );
        // A `--` given: the terms go right behind it.
        assert_eq!(
            moved("x policy search -deny --label app=x -- -svc:443 y"),
            "x policy search --label app=x -- -deny -svc:443 y"
        );
        // Global flags before or between `policy` and `search`.
        assert_eq!(
            moved("x policy --json search -deny"),
            "x policy --json search -- -deny"
        );
        assert_eq!(
            moved("x --profile p policy --profile p search src:all -deny"),
            "x --profile p policy --profile p search src:all -- -deny"
        );
        // A flag's value written apart stays for clap to judge.
        for line in [
            "x policy search --profile -x app=y",
            "x policy search --label -app=x app=y",
            "x policy search --href -abc app=y",
        ] {
            assert_eq!(moved(line), line);
        }
        assert_eq!(
            moved("x policy search --href=-abc -deny"),
            "x policy search --href=-abc -- -deny"
        );
        // A word named `policy` as a flag's value is not the command.
        assert_eq!(
            moved("x --profile policy search -deny"),
            "x --profile policy search -deny"
        );
        // Nothing to move, `-h`, `-` alone, other commands: as they are.
        for line in [
            "x policy search app=shop --json",
            "x policy search -h",
            "x policy search -",
            "x policy search app=x -- y",
            "x workloads -deny",
            "x policy rules -x",
        ] {
            assert_eq!(moved(line), line);
        }
    }

    /// The terms clap hands `policy search` for this command line.
    fn terms(line: &str) -> Result<(Vec<String>, bool), String> {
        let args = line.split(' ').map(OsString::from).collect();
        let cli = super::super::Cli::try_parse_from(negated_terms_last(args))
            .map_err(|e| e.to_string())?;
        match cli.command {
            Some(super::super::Command::Policy(PolicyCmd::Search(a))) => Ok((a.terms, a.active)),
            other => Err(format!("{other:?}")),
        }
    }

    #[test]
    fn dash_terms_parse_as_terms_and_flags_as_flags() {
        assert_eq!(
            terms("x policy search -deny --active app=shop").unwrap(),
            (vec!["app=shop".into(), "-deny".into()], true)
        );
        assert_eq!(
            terms("x --json policy search --active -- -deny -src:all").unwrap(),
            (vec!["-deny".into(), "-src:all".into()], true)
        );
        assert!(terms("x policy search app=x --nope").is_err());
        assert_eq!(
            terms("x policy --json search -deny").unwrap(),
            (vec!["-deny".into()], false)
        );
        assert_eq!(
            terms("x policy --profile p search src:all -deny").unwrap(),
            (vec!["src:all".into(), "-deny".into()], false)
        );
        // A value-taking flag before a `-word` is refused as before.
        for line in [
            "x policy search --profile -x app=y",
            "x policy search --label -app=x app=y",
            "x policy search --href -abc app=y",
        ] {
            assert!(terms(line).is_err(), "{line}");
        }
        let f = search_filter(&SearchArgs {
            terms: terms("x policy search -deny scope:global").unwrap().0,
            labels: Vec::new(),
            ip_lists: Vec::new(),
            services: Vec::new(),
            workloads: Vec::new(),
            href: None,
            disabled: false,
            deny: false,
            active: false,
            draft: false,
            saved: Default::default(),
        })
        .unwrap();
        assert_eq!(f.to_string(), "scope:global -deny");
    }
}
