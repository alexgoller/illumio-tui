//! Read-only policy views: rulesets, the rules of one ruleset (allow and
//! deny), and pending provisioning changes.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::TableRows;
use clap::{Args, Subcommand};
use pce::models::{PendingChange, PolicyScope, RuleSet};
use pce::{PceError, RuleSetView, RuleView};
use serde::Serialize;

#[derive(Subcommand, Debug)]
pub enum PolicyCmd {
    /// List rulesets with resolved scopes
    Rulesets(RulesetsArgs),
    /// List the rules of one ruleset (name or href), allow and deny
    Rules(RulesArgs),
    /// List changes waiting to be provisioned
    Pending,
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
    pub ruleset: String,
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
        PolicyCmd::Rulesets(a) => rulesets(ctx, a).await,
        PolicyCmd::Rules(a) => rules(ctx, a).await,
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
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
    let c = ctx
        .pce
        .rule_sets(scope(args.active), ctx.cache_mode)
        .await?;
    let rs = find_ruleset(&c.value, &args.ruleset).ok_or_else(|| {
        let names: Vec<&str> = c.value.iter().map(|r| r.name.as_str()).collect();
        CliError::Args(format!(
            "ruleset {:?} not found; known: {}",
            args.ruleset,
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
