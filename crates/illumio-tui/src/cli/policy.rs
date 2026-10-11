//! Policy: rulesets (list; add, update, delete), the rules of one ruleset
//! (list; add, update, delete allow rules), deny rules, and pending
//! provisioning changes.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::TableRows;
use super::write::{
    self, ActorToken, Lists, ServiceToken, WriteFlags, is_href, parse_actors, parse_scopes,
    parse_services, resolve_actors, resolve_scopes, resolve_services, run_plan,
};
use clap::{Args, Subcommand};
use pce::models::{PendingChange, PolicyScope, ResolveLabelsAs, RuleSet};
use pce::write::{
    ActorRef, DenyRuleDraft, DenyRulePatch, IfMissing, RuleDraft, RulePatch, RuleSetDraft,
    RuleSetPatch, ServiceRef, WriteError, checked_nested_href,
};
use pce::{PceError, RuleSetView, RuleView};
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

/// `policy rules <ruleset>` lists; `add`, `update` and `delete` write.
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
}

#[derive(Debug, Serialize)]
pub struct RulesOutput {
    pub ruleset: RuleSetView,
    pub rules: Vec<RuleView>,
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

impl TableRows for RulesOutput {
    fn headers(&self) -> Vec<&'static str> {
        vec![
            "kind",
            "enabled",
            "consumers",
            "providers",
            "services",
            "description",
            "pending",
        ]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.rules
            .iter()
            .map(|r| {
                vec![
                    serde_json::to_value(r.kind)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_string))
                        .unwrap_or_default(),
                    r.enabled.to_string(),
                    names(r.consumers.iter().map(|a| a.name.as_str())),
                    names(r.providers.iter().map(|a| a.name.as_str())),
                    names(r.services.iter().map(|s| s.name.as_str())),
                    r.description.clone().unwrap_or_default(),
                    r.update_type.clone().unwrap_or_default(),
                ]
            })
            .collect()
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
        },
        PolicyCmd::DenyRules(action) => match action {
            DenyRulesAction::Add(a) => deny_rule_add(ctx, a).await,
            DenyRulesAction::Update(a) => deny_rule_update(ctx, a).await,
            DenyRulesAction::Delete(a) => deny_rule_delete(ctx, a).await,
        },
        PolicyCmd::Pending => pending(ctx).await,
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
            "policy rules needs a ruleset name or href (or one of add, update, delete)".into(),
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
    let mut views: Vec<RuleView> = rs
        .rules
        .iter()
        .map(|r| RuleView::from_rule(r, &idx))
        .collect();
    if rs.deny_rules.is_empty() {
        // Older PCEs leave the inline list empty and serve deny rules from
        // a per-ruleset endpoint; others have no such endpoint at all.
        match ctx.pce.deny_rules(&rs.href, ctx.cache_mode).await {
            Ok(d) => views.extend(d.value.iter().map(|r| RuleView::from_deny(r, &idx))),
            Err(PceError::NotFound { .. }) => {}
            Err(e) => return Err(e.into()),
        }
    } else {
        views.extend(rs.deny_rules.iter().map(|r| RuleView::from_deny(r, &idx)));
    }
    let out = RulesOutput {
        ruleset: RuleSetView::from_rule_set(rs, &idx),
        rules: views,
    };
    ctx.emit(&out, cache_label(c.hit, ctx.cache_mode), c.truncated, None)
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

/// Exactly one live draft ruleset named `name` (case-insensitive); a
/// pending delete does not count.
fn ruleset_named(sets: &[RuleSet], name: &str) -> Result<String, CliError> {
    let live = || {
        sets.iter()
            .filter(|rs| rs.update_type.as_deref() != Some("delete"))
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
    ruleset_named(&sets, needle.trim())
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
    run_plan(ctx, &a.write, &guard, plan).await
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
    run_plan(ctx, &a.write, &guard, plan).await
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
    run_plan(ctx, &a.write, &guard, plan).await
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
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn deny_rule_delete(ctx: &Context, a: &DenyRuleDeleteArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let plan = ctx.pce.writer(&guard).plan_delete_deny_rule(&a.deny_rule)?;
    run_plan(ctx, &a.write, &guard, plan).await
}
