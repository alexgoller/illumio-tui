//! `pairing-profiles` (`pp`): list them (cached), create one, delete one.
//! The request bodies are built in `pce::write::pairing_profiles`; here the
//! flags are parsed, names resolved against the cached lists, and the
//! results printed.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{OutputFormat, TableRows, WriteMeta, join_list, meta_line, write_json};
use super::write::{self, WriteFlags, WriteOutput, label_in, pairing_profile_in, parse_kv};
use clap::{Args, Subcommand};
use pce::models::pairing_profiles::{lifespan_text, value_text};
use pce::models::{Label, PairingProfile};
use pce::write::pairing_profiles::{
    checked_pairing_href, parse_enforcement, parse_locks, parse_visibility,
};
use pce::write::{Limit, Outcome, PairingProfileDraft, Plan, VenType};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Subcommand, Debug)]
pub enum PairingProfilesCmd {
    /// List pairing profiles (one cached read, plus the labels list when a
    /// profile has labels)
    List,
    /// Create a pairing profile (needs writes = true in the profile)
    ///
    /// Sends the name plus only the settings given; the PCE applies its own
    /// defaults to the rest. The name must be new (checked against the
    /// pairing profile list); labels are resolved through the labels list.
    /// No pairing key or activation code is made.
    Create(Box<CreateArgs>),
    /// Delete a pairing profile by exact name or href
    Delete(DeleteArgs),
}

#[derive(Args, Debug)]
pub struct CreateArgs {
    /// The new profile's name
    pub name: String,
    /// Description
    #[arg(long)]
    pub description: Option<String>,
    /// key=value, repeatable; at most one per label key
    #[arg(long, value_name = "KEY=VALUE")]
    pub label: Vec<String>,
    /// Enforcement mode: idle, visibility_only (or visibility-only), selective, full
    #[arg(long, value_name = "MODE")]
    pub enforcement: Option<String>,
    /// Visibility level: flow_summary, flow_drops, flow_off, enhanced_data_collection
    /// (or blocked-allowed, blocked, off, enhanced)
    #[arg(long, value_name = "LEVEL")]
    pub visibility: Option<String>,
    /// VEN type: server, endpoint, specified-during-activation
    #[arg(long, value_name = "TYPE")]
    pub ven_type: Option<String>,
    /// Uses per pairing key: a number of at least 1, or unlimited
    #[arg(long, value_name = "N|unlimited")]
    pub uses: Option<String>,
    /// Pairing key lifespan: seconds (3600), a duration (1h, 7d), or unlimited
    #[arg(long, value_name = "DURATION|unlimited")]
    pub lifespan: Option<String>,
    /// Create it disabled (the default is enabled)
    #[arg(long)]
    pub disabled: bool,
    /// Lock settings against change at pairing, comma-separated or repeatable:
    /// role, app, env, loc, enforcement, visibility, log-traffic. The PCE locks
    /// every one not given here or in --unlock
    #[arg(long, value_name = "LOCKS")]
    pub lock: Vec<String>,
    /// Leave settings unlocked (sent as false), same names as --lock; the PCE
    /// locks every one not given
    #[arg(long, value_name = "LOCKS")]
    pub unlock: Vec<String>,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct DeleteArgs {
    /// Exact name or href
    pub target: String,
    #[command(flatten)]
    pub write: WriteFlags,
}

impl PairingProfilesCmd {
    /// The write flags of a write, `None` for the list.
    pub fn write_flags(&self) -> Option<&WriteFlags> {
        match self {
            PairingProfilesCmd::List => None,
            PairingProfilesCmd::Create(a) => Some(&a.write),
            PairingProfilesCmd::Delete(a) => Some(&a.write),
        }
    }
}

pub async fn run(ctx: &Context, cmd: &PairingProfilesCmd) -> Result<(), CliError> {
    match cmd {
        PairingProfilesCmd::List => list(ctx).await,
        PairingProfilesCmd::Create(a) => create(ctx, a).await,
        PairingProfilesCmd::Delete(a) => delete(ctx, a).await,
    }
}

/// The CSV columns, in order (`columns::csv_columns`).
pub const HEADERS: [&str; 10] = [
    "name",
    "enabled",
    "labels",
    "enforcement",
    "visibility",
    "ven_type",
    "uses_per_key",
    "key_lifespan",
    "locks",
    "href",
];

/// The profiles as the PCE returned them, their labels named
/// ([`name_labels`]).
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct ProfileList {
    pub profiles: Vec<PairingProfile>,
}

/// Gives every label the PCE sent as a bare href its key and value from
/// `labels`; one not in it stays an href.
pub fn name_labels(profiles: &mut [PairingProfile], labels: &[Label]) {
    let by_href: HashMap<&str, &Label> = labels.iter().map(|l| (l.href.as_str(), l)).collect();
    for l in profiles.iter_mut().flat_map(|p| p.labels.iter_mut()) {
        if let (None, None, Some(known)) = (&l.key, &l.value, by_href.get(l.href.as_str())) {
            l.key = Some(known.key.clone());
            l.value = Some(known.value.clone());
        }
    }
}

impl ProfileList {
    fn labels_of(&self, p: &PairingProfile) -> Vec<String> {
        p.labels
            .iter()
            .map(|l| match (&l.key, &l.value) {
                (Some(k), Some(v)) => format!("{k}={v}"),
                _ => l.href.clone(),
            })
            .collect()
    }

    fn row(&self, p: &PairingProfile, full: bool) -> Vec<String> {
        let enabled = match p.enabled {
            Some(b) if full => b.to_string(),
            Some(true) => "yes".into(),
            Some(false) => "no".into(),
            None => String::new(),
        };
        let labels = self.labels_of(p);
        let locks = p.locks();
        vec![
            p.name.clone(),
            enabled,
            if full {
                join_list(&labels)
            } else {
                labels.join(", ")
            },
            p.enforcement_mode.clone().unwrap_or_default(),
            p.visibility_level.clone().unwrap_or_default(),
            p.ven_type.clone().unwrap_or_default(),
            value_text(p.allowed_uses_per_key.as_ref()),
            if full {
                value_text(p.key_lifespan.as_ref())
            } else {
                lifespan_text(p.key_lifespan.as_ref())
            },
            if full {
                join_list(&locks)
            } else {
                locks.join(",")
            },
            p.href.clone(),
        ]
    }
}

impl TableRows for ProfileList {
    fn headers(&self) -> Vec<&'static str> {
        HEADERS.to_vec()
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.profiles.iter().map(|p| self.row(p, false)).collect()
    }
    /// `enabled` as `true`/`false`, the lifespan in seconds, lists one entry each.
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.profiles.iter().map(|p| self.row(p, true)).collect()
    }
}

async fn list(ctx: &Context) -> Result<(), CliError> {
    let c = ctx.pce.pairing_profiles(ctx.cache_mode).await?;
    let mut truncated = c.truncated;
    // Labels come as hrefs: their names from the (cached) labels list,
    // read only when a profile has a label without them.
    let needs_names = c
        .value
        .iter()
        .flat_map(|p| &p.labels)
        .any(|l| l.key.is_none() || l.value.is_none());
    let mut profiles = c.value;
    if needs_names {
        let labels = ctx.pce.labels(ctx.cache_mode).await?;
        truncated |= labels.truncated;
        name_labels(&mut profiles, &labels.value);
    }
    let data = ProfileList { profiles };
    ctx.emit(&data, cache_label(c.hit, ctx.cache_mode), truncated, None)
}

/// The draft the flags describe, labels unresolved; checked before any
/// request. Returns it with the `key=value` label specs.
fn draft_of(a: &CreateArgs) -> Result<(PairingProfileDraft, Vec<String>), CliError> {
    let name = a.name.trim();
    if name.is_empty() {
        return Err(CliError::Args("the pairing profile name is empty".into()));
    }
    let mut keys: Vec<String> = Vec::new();
    for spec in &a.label {
        let (k, _) = parse_kv(spec)?;
        if keys.contains(&k) {
            return Err(CliError::Args(format!(
                "--label: at most one label per key; {k} is given twice"
            )));
        }
        keys.push(k);
    }
    let bad =
        |flag: &str, v: &str, valid: &str| CliError::Args(format!("{flag} {v:?}: one of {valid}"));
    let enforcement = a
        .enforcement
        .as_deref()
        .map(|v| {
            parse_enforcement(v)
                .ok_or_else(|| bad("--enforcement", v, "idle, visibility_only, selective, full"))
        })
        .transpose()?;
    let visibility = a
        .visibility
        .as_deref()
        .map(|v| {
            parse_visibility(v).ok_or_else(|| {
                bad(
                    "--visibility",
                    v,
                    "flow_summary, flow_drops, flow_off, enhanced_data_collection (or blocked-allowed, blocked, off, enhanced)",
                )
            })
        })
        .transpose()?;
    let ven_type = a
        .ven_type
        .as_deref()
        .map(|v| {
            VenType::parse(v).ok_or_else(|| {
                bad(
                    "--ven-type",
                    v,
                    "server, endpoint, specified-during-activation",
                )
            })
        })
        .transpose()?;
    let uses = a
        .uses
        .as_deref()
        .map(Limit::parse_uses)
        .transpose()
        .map_err(CliError::Args)?;
    let lifespan = a
        .lifespan
        .as_deref()
        .map(Limit::parse_lifespan)
        .transpose()
        .map_err(CliError::Args)?;
    let locks = parse_locks("--lock", &a.lock).map_err(CliError::Args)?;
    let unlocks = parse_locks("--unlock", &a.unlock).map_err(CliError::Args)?;
    if let Some((name, _)) = pce::models::pairing_profiles::LOCKS
        .iter()
        .find(|(_, f)| locks.contains(f) && unlocks.contains(f))
    {
        return Err(CliError::Args(format!(
            "{name} is in both --lock and --unlock"
        )));
    }
    let draft = PairingProfileDraft {
        name: name.to_string(),
        description: a.description.clone(),
        labels: Vec::new(),
        enforcement,
        visibility,
        ven_type,
        uses,
        lifespan,
        disabled: a.disabled,
        locks,
        unlocks,
    };
    Ok((draft, a.label.clone()))
}

/// `pairing-profiles create`'s table: the created profile's name and href.
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct Created(pub Value);

impl TableRows for Created {
    fn headers(&self) -> Vec<&'static str> {
        vec!["name", "href"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let text = |k: &str| {
            self.0
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        vec![vec![text("name"), text("href")]]
    }
}

/// `pairing-profiles delete`'s data.
#[derive(Debug, Serialize)]
pub struct Deleted {
    pub deleted: String,
    pub href: String,
}

async fn create(ctx: &Context, a: &CreateArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let (mut draft, specs) = draft_of(a)?;
    let existing = complete_list(ctx, "the name cannot be checked").await?;
    if !specs.is_empty() {
        let labels = ctx.pce.labels(ctx.cache_mode).await?.value;
        draft.labels = specs
            .iter()
            .map(|s| label_in(&labels, s))
            .collect::<Result<_, _>>()?;
    }
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_create_pairing_profile(&draft, &existing)?;
    send(ctx, &a.write, &guard, plan, Sent::Created(draft.name)).await
}

/// The whole pairing profile list; a truncated one is an argument error
/// (`why` says what it stops), never a check against part of it.
async fn complete_list(ctx: &Context, why: &str) -> Result<Vec<PairingProfile>, CliError> {
    let c = ctx.pce.pairing_profiles(ctx.cache_mode).await?;
    if c.truncated {
        return Err(CliError::Args(format!(
            "the pairing profile list is truncated: {why}"
        )));
    }
    Ok(c.value)
}

/// What a sent write prints: the created profile (its name, should the PCE
/// answer without a body), or the deleted one's name.
enum Sent {
    Created(String),
    Deleted(String),
}

async fn delete(ctx: &Context, a: &DeleteArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let (href, name) = if write::is_href(&a.target) {
        let href = checked_pairing_href(&a.target, ctx.config.org_id)?;
        (href.clone(), href)
    } else {
        let profiles = complete_list(ctx, "use the href").await?;
        let href = pairing_profile_in(&profiles, &a.target)?;
        (href, a.target.trim().to_string())
    };
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_delete_pairing_profile(&href, &name)?;
    send(ctx, &a.write, &guard, plan, Sent::Deleted(name)).await
}

/// Dry run: the request, as every write prints it. Sent: the created
/// profile as the PCE returned it, or `deleted NAME`.
async fn send(
    ctx: &Context,
    flags: &WriteFlags,
    guard: &pce::write::WriteGuard,
    plan: Plan,
    sent: Sent,
) -> Result<(), CliError> {
    let Plan::Send { request, kind } = plan else {
        unreachable!("pairing profile plans always send")
    };
    if flags.dry_run {
        return ctx.emit_write(&WriteOutput::DryRun(request), None);
    }
    let (method, path) = (request.method, request.path.clone());
    let Outcome::Written(w) = ctx
        .pce
        .writer(guard)
        .execute(Plan::Send { request, kind })
        .await?
    else {
        unreachable!("only a Noop plan is skipped")
    };
    let mut meta = ctx.meta("none", false, None);
    meta.write = Some(WriteMeta {
        method,
        path,
        status: w.status,
        audit: guard.audit_location(),
    });
    let mut warnings = w.warnings.clone();
    if !w.dropped_fields.is_empty() {
        warnings.push(format!(
            "sent but not in the PCE's answer: {}",
            w.dropped_fields.join(", ")
        ));
    }
    if !warnings.is_empty() {
        if ctx.format != OutputFormat::Json {
            for warning in &warnings {
                eprintln!("warning: {warning}");
            }
        }
        meta.warnings = Some(warnings);
    }
    match sent {
        // A POST answered without a body: what is known of the profile.
        Sent::Created(name) if !w.object.is_object() => ctx.emit_meta(
            &Created(serde_json::json!({"href": w.href, "name": name})),
            &meta,
        ),
        Sent::Created(_) => ctx.emit_meta(&Created(w.object), &meta),
        Sent::Deleted(name) => {
            let data = Deleted {
                deleted: name,
                href: w.href,
            };
            let mut out = std::io::stdout().lock();
            match ctx.format {
                OutputFormat::Json => write_json(&mut out, &data, &meta)?,
                // `run` refuses `--csv` on writes before any request.
                OutputFormat::Table | OutputFormat::Csv => {
                    use std::io::Write;
                    writeln!(out, "deleted {}", data.deleted)?;
                    eprintln!("{}", meta_line(&meta));
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn create_args(argv: &[&str]) -> CreateArgs {
        let mut full = vec!["illumio-tui", "pairing-profiles", "create"];
        full.extend_from_slice(argv);
        let cli = super::super::Cli::try_parse_from(full).unwrap();
        match cli.command {
            Some(super::super::Command::PairingProfiles(PairingProfilesCmd::Create(a))) => *a,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn flags_make_the_draft_before_any_request() {
        let (d, specs) = draft_of(&create_args(&[
            "shop",
            "--label",
            "app=shop",
            "--label",
            "env=prod",
            "--enforcement",
            "visibility-only",
            "--visibility",
            "blocked",
            "--ven-type",
            "server",
            "--uses",
            "unlimited",
            "--lifespan",
            "7d",
            "--disabled",
            "--lock",
            "app,enforcement",
            "--unlock",
            "role,log_traffic",
        ]))
        .unwrap();
        assert_eq!(specs, ["app=shop", "env=prod"]);
        assert_eq!(
            d.to_body(),
            serde_json::json!({
                "name": "shop",
                "enforcement_mode": "visibility_only",
                "visibility_level": "flow_drops",
                "ven_type": "server",
                "allowed_uses_per_key": "unlimited",
                "key_lifespan": 604_800,
                "enabled": false,
                "app_label_lock": true,
                "enforcement_mode_lock": true,
                "role_label_lock": false,
                "log_traffic_lock": false,
            })
        );
    }

    #[test]
    fn bad_flags_are_argument_errors() {
        for argv in [
            &["p", "--label", "app=a", "--label", "app=b"][..],
            &["p", "--label", "app"],
            &["p", "--enforcement", "enforced"],
            &["p", "--visibility", "all"],
            &["p", "--ven-type", "laptop"],
            &["p", "--uses", "0"],
            &["p", "--lifespan", "soon"],
            &["p", "--lock", "colour"],
            &["p", "--unlock", "colour"],
            &["p", "--lock", "app,env", "--unlock", "env"],
            &[" "],
        ] {
            let e = draft_of(&create_args(argv)).unwrap_err();
            assert_eq!(e.exit_code(), 2, "{argv:?}: {e}");
        }
    }

    #[test]
    fn the_table_names_labels_and_locks() {
        let mut profiles: Vec<PairingProfile> =
            serde_json::from_str(include_str!("../../../../fixtures/pairing_profiles.json"))
                .unwrap();
        let labels: Vec<Label> = serde_json::from_value(serde_json::json!([
            {"href": "/orgs/5/labels/1", "key": "role", "value": "web"}
        ]))
        .unwrap();
        name_labels(&mut profiles, &labels);
        assert_eq!(profiles[1].labels[0].key.as_deref(), Some("role"));
        assert_eq!(profiles[1].labels[1].key, None, "not in the list: an href");
        let list = ProfileList { profiles };
        let rows = list.rows();
        assert_eq!(
            rows[1],
            [
                "shop-prod",
                "no",
                "role=web, /orgs/5/labels/3",
                "selective",
                "flow_drops",
                "specified_during_activation",
                "1",
                "1d",
                "role,app,env,enforcement",
                "/orgs/5/pairing_profiles/7"
            ]
        );
        let full = list.full_rows();
        assert_eq!(full[1][1], "false");
        assert_eq!(full[1][7], "86400");
        assert_eq!(full[1][8], "role; app; env; enforcement");
        assert_eq!(full[0][6], "unlimited");
    }
}
