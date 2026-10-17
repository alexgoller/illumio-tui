//! `illumio-tui marks list|add|rm|export|import`: the local marks file.
//! No request: `list`, `export` and `rm` need no profile; `add --pce`
//! without a host and the `active` column use the profile's host when one
//! resolves (no secret needed).
use super::GlobalArgs;
use super::context::config_inputs;
use super::error::CliError;
use super::output::{
    MarksMeta, Meta, OutputFormat, TableRows, choose_format, meta_line, write_json, write_table,
    write_table_csv,
};
use crate::config;
use crate::marks::{
    MarkEntry, MarkFile, MarkOp, MarksStore, entry_of, marks_path, render, to_mark,
};
use clap::{Args, Subcommand, ValueEnum};
use jiff::Timestamp;
use pce::marks::{
    Action, Mark, PceId, PceScope, ServiceSpec, parse_decision, parse_expiry, parse_side, side_text,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Subcommand, Debug)]
pub enum MarksCmd {
    /// List the marks in marks.toml, active or not (no profile needed)
    List(ListArgs),
    /// Add a mark; prints its id. Portable unless --pce is given
    Add(AddArgs),
    /// Remove marks by id (all or none), or every expired one
    Rm(RmArgs),
    /// Print every mark to stdout: JSON ({"marks": [...]}, what import reads) or TOML
    Export(ExportArgs),
    /// Add the marks of a JSON or TOML file (by extension); existing ids are skipped
    Import(ImportArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct ListArgs {
    /// Only this action: critical, important, revisit, accept-blocked, reviewed, squelch
    #[arg(long)]
    pub action: Option<String>,
    /// Only active marks (unexpired, in scope for the profile's PCE)
    #[arg(long, conflicts_with = "expired")]
    pub active: bool,
    /// Only expired marks
    #[arg(long)]
    pub expired: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct AddArgs {
    /// critical, important, revisit, accept-blocked, reviewed or squelch
    #[arg(long)]
    pub action: String,
    /// Source side: any, key=value (| = or), wl:HOST, ip:CIDR, ipl:NAME
    #[arg(long)]
    pub src: String,
    /// Destination side, same syntax as --src
    #[arg(long)]
    pub dst: String,
    /// 5666/tcp, 8000-8010/tcp, udp, icmp or a protocol number (repeatable or
    /// comma-separated: any of them); default any service
    #[arg(long, value_delimiter = ',')]
    pub service: Vec<String>,
    /// allowed, potentially_blocked, blocked (repeatable or comma-separated);
    /// matched against the reported decision
    #[arg(long, value_delimiter = ',')]
    pub decision: Vec<String>,
    /// Same values, matched against the draft decision (traffic --draft only)
    #[arg(long, value_delimiter = ',')]
    pub draft_decision: Vec<String>,
    /// 1d, 7d, 30d, 90d, YYYY-MM-DD (23:59:59 UTC), RFC 3339 or never;
    /// default by action: squelch and accept-blocked 90d, reviewed 30d,
    /// revisit 7d, critical and important never
    #[arg(long)]
    pub expires: Option<String>,
    /// Limit the mark to a PCE host; without a value, the profile's host
    #[arg(long, num_args = 0..=1)]
    pub pce: Option<Option<String>>,
    /// Limit the mark to an org id too (with --pce)
    #[arg(long, requires = "pce")]
    pub org: Option<u64>,
    /// Why the flows are marked (required): a ticket, the owner's answer
    #[arg(long)]
    pub comment: String,
}

#[derive(Args, Debug, Clone, Default)]
pub struct RmArgs {
    /// Mark ids (m-…, h-…); an unknown one removes nothing
    #[arg(required_unless_present = "expired")]
    pub ids: Vec<String>,
    /// Every expired mark
    #[arg(long, conflicts_with = "ids")]
    pub expired: bool,
}

#[derive(ValueEnum, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ExportFormat {
    #[default]
    Json,
    Toml,
}

#[derive(Args, Debug, Clone, Default)]
pub struct ExportArgs {
    /// json (default) or toml
    #[arg(long, value_enum, default_value_t)]
    pub format: ExportFormat,
}

#[derive(Args, Debug, Clone, Default)]
pub struct ImportArgs {
    /// A .json file ({"marks": [...]}, as export writes it) or a .toml file
    pub file: PathBuf,
    /// Replace marks whose id exists instead of skipping them
    #[arg(long)]
    pub replace: bool,
}

/// `export`'s and `import`'s JSON: the TOML field names.
#[derive(Debug, Serialize, Deserialize)]
pub struct MarksJson {
    pub marks: Vec<MarkEntry>,
}

/// The profile's PCE when one resolves (no secret needed), and its name.
fn profile_pce(global: &GlobalArgs) -> Option<(String, PceId)> {
    let (file, overrides, env) = config_inputs(global).ok()?;
    let id = config::identity(&file, &overrides, &env).ok()?;
    Some((
        id.profile_name,
        PceId {
            host: id.host,
            org: id.org_id,
        },
    ))
}

fn store(global: &GlobalArgs) -> MarksStore {
    MarksStore::open(marks_path(global.config.as_deref()))
}

fn emit<T: Serialize + TableRows>(
    global: &GlobalArgs,
    data: &T,
    meta: &Meta,
) -> Result<(), CliError> {
    let mut out = std::io::stdout().lock();
    match choose_format(global.json, global.table, global.csv) {
        OutputFormat::Json => write_json(&mut out, data, meta)?,
        OutputFormat::Table => {
            write_table(&mut out, data)?;
            eprintln!("{}", meta_line(meta));
        }
        OutputFormat::Csv => {
            write_table_csv(&mut out, data)?;
            eprintln!("{}", meta_line(meta));
        }
    }
    Ok(())
}

fn meta(profile: Option<&str>, marks: Option<MarksMeta>) -> Meta {
    Meta {
        profile: profile.unwrap_or_default().to_string(),
        cache: "none",
        marks,
        ..Meta::default()
    }
}

/// One `list` row: the TOML field names, sides in the side grammar.
#[derive(Debug, Serialize)]
pub struct ListRow {
    pub id: String,
    pub action: Action,
    pub src: String,
    pub dst: String,
    pub service: Vec<String>,
    pub decision: Vec<String>,
    pub draft_decision: Vec<String>,
    pub pce: Option<String>,
    pub org: Option<u64>,
    pub expires: Option<String>,
    pub active: bool,
    pub comment: String,
    pub created: Option<String>,
    pub created_on: Option<String>,
    /// The mark's unknown fields.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

impl ListRow {
    fn new(m: &Mark, extra: &toml::Table, active: bool) -> Self {
        let ds = |d: &[pce::models::PolicyDecision]| d.iter().map(|d| d.as_str().into()).collect();
        Self {
            id: m.id.clone(),
            action: m.action,
            src: side_text(&m.src),
            dst: side_text(&m.dst),
            service: m.services.iter().map(ServiceSpec::text).collect(),
            decision: ds(&m.decisions),
            draft_decision: ds(&m.draft_decisions),
            pce: m.scope.as_ref().map(|s| s.host.clone()),
            org: m.scope.as_ref().and_then(|s| s.org),
            expires: m.expires.map(|t| t.to_string()),
            active,
            comment: m.comment.clone(),
            created: m.created.map(|t| t.to_string()),
            created_on: m.created_on.clone(),
            extra: match serde_json::to_value(extra) {
                Ok(Value::Object(o)) => o,
                _ => serde_json::Map::new(),
            },
        }
    }
}

impl TableRows for Vec<ListRow> {
    fn headers(&self) -> Vec<&'static str> {
        vec![
            "id",
            "action",
            "src",
            "dst",
            "service",
            "decision",
            "draft_decision",
            "pce",
            "org",
            "expires",
            "active",
            "comment",
            "created",
            "created_on",
        ]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let opt = |v: &Option<String>| v.clone().unwrap_or_default();
        self.iter()
            .map(|r| {
                vec![
                    r.id.clone(),
                    r.action.as_str().into(),
                    r.src.clone(),
                    r.dst.clone(),
                    r.service.join(", "),
                    r.decision.join("|"),
                    r.draft_decision.join("|"),
                    opt(&r.pce),
                    r.org.map(|o| o.to_string()).unwrap_or_default(),
                    r.expires.clone().unwrap_or_else(|| "never".into()),
                    r.active.to_string(),
                    if r.comment.trim().is_empty() {
                        "(no comment)".into()
                    } else {
                        r.comment.clone()
                    },
                    opt(&r.created),
                    opt(&r.created_on),
                ]
            })
            .collect()
    }
}

/// `{"id": "m-…"}`: what `add` prints.
#[derive(Debug, Serialize)]
pub struct Added {
    pub id: String,
}

impl TableRows for Added {
    fn headers(&self) -> Vec<&'static str> {
        vec!["id"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        vec![vec![self.id.clone()]]
    }
}

#[derive(Debug, Serialize)]
pub struct Removed {
    pub removed: Vec<String>,
}

impl TableRows for Removed {
    fn headers(&self) -> Vec<&'static str> {
        vec!["removed"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.removed.iter().map(|id| vec![id.clone()]).collect()
    }
}

#[derive(Debug, Serialize)]
pub struct Imported {
    pub added: usize,
    pub replaced: usize,
    pub skipped: Vec<String>,
}

impl TableRows for Imported {
    fn headers(&self) -> Vec<&'static str> {
        vec!["added", "replaced", "skipped"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        vec![vec![
            self.added.to_string(),
            self.replaced.to_string(),
            self.skipped.join(", "),
        ]]
    }
}

pub fn run(global: &GlobalArgs, cmd: &MarksCmd) -> Result<(), CliError> {
    match cmd {
        MarksCmd::List(a) => list(global, a),
        MarksCmd::Add(a) => add(global, a),
        MarksCmd::Rm(a) => rm(global, a),
        MarksCmd::Export(a) => export(global, a),
        MarksCmd::Import(a) => import(global, a),
    }
}

fn list(global: &GlobalArgs, args: &ListArgs) -> Result<(), CliError> {
    let action =
        match &args.action {
            Some(a) => Some(Action::parse(a).ok_or_else(|| {
                CliError::Args(format!("unknown action {a:?}: {}", Action::names()))
            })?),
            None => None,
        };
    let s = store(global);
    let loaded = s.loaded().ok_or_else(|| marks_error(s.error()))?;
    let now = Timestamp::now();
    let profile = profile_pce(global);
    let pce = profile.as_ref().map(|(_, p)| p);
    let rows: Vec<ListRow> = loaded
        .set
        .marks
        .iter()
        .zip(&loaded.file.mark)
        .filter(|(m, _)| action.is_none_or(|a| m.action == a))
        .filter(|(m, _)| !args.expired || m.is_expired(now))
        .map(|(m, e)| ListRow::new(m, &e.extra, !m.is_expired(now) && m.in_scope(pce)))
        .filter(|r| !args.active || r.active)
        .collect();
    for w in s.warnings() {
        eprintln!("warning: {w}");
    }
    let active = loaded
        .set
        .marks
        .iter()
        .filter(|m| !m.is_expired(now) && m.in_scope(pce))
        .count();
    let meta = meta(
        profile.as_ref().map(|(n, _)| n.as_str()),
        Some(MarksMeta {
            active,
            file: s.path().display().to_string(),
            waiting_draft: None,
        }),
    );
    emit(global, &rows, &meta)
}

/// The store's problems (an invalid or unreadable file) as an exit-2 or
/// I/O error.
fn marks_error(e: Option<&crate::marks::MarksError>) -> CliError {
    match e {
        Some(e) => CliError::Marks(e.clone()),
        None => CliError::Other("marks.toml: no marks loaded".into()),
    }
}

/// The mark `add` describes, checked as the file checks it.
pub fn mark_from_args(
    args: &AddArgs,
    profile: Option<&PceId>,
    now: Timestamp,
) -> Result<Mark, CliError> {
    let bad = |flag: &str, e: String| CliError::Args(format!("--{flag}: {e}"));
    let action = Action::parse(&args.action).ok_or_else(|| {
        bad(
            "action",
            format!("unknown action {:?}: {}", args.action, Action::names()),
        )
    })?;
    let src = parse_side(&args.src).map_err(|e| bad("src", e))?;
    let dst = parse_side(&args.dst).map_err(|e| bad("dst", e))?;
    let services = args
        .service
        .iter()
        .map(|s| ServiceSpec::parse(s).map_err(|e| bad("service", e)))
        .collect::<Result<_, _>>()?;
    let decisions = |flag: &str, list: &[String]| {
        list.iter()
            .map(|d| parse_decision(d).map_err(|e| bad(flag, e)))
            .collect::<Result<Vec<_>, _>>()
    };
    let expires = parse_expiry(
        args.expires.as_deref().unwrap_or(action.default_expiry()),
        now,
    )
    .map_err(|e| bad("expires", e))?;
    let host = match &args.pce {
        None => None,
        Some(Some(h)) if !h.trim().is_empty() => Some(h.trim().to_string()),
        Some(_) => Some(profile.map(|p| p.host.clone()).ok_or_else(|| {
            bad(
                "pce",
                "no profile resolves to take the host from: write --pce HOST".into(),
            )
        })?),
    };
    if args.comment.trim().is_empty() {
        return Err(bad(
            "comment",
            "required: why are these flows marked?".into(),
        ));
    }
    let m = Mark {
        id: String::new(),
        action,
        src,
        dst,
        services,
        decisions: decisions("decision", &args.decision)?,
        draft_decisions: decisions("draft-decision", &args.draft_decision)?,
        scope: host.map(|host| PceScope {
            host,
            org: args.org,
        }),
        expires,
        comment: args.comment.trim().to_string(),
        created: Some(now),
        created_on: profile.map(|p| p.host.clone()),
    };
    if let Some(p) = m.problems().into_iter().next() {
        // Name the flag that fixes it.
        let flag = if p.contains("workload") {
            "pce"
        } else {
            "decision"
        };
        return Err(bad(flag, p));
    }
    Ok(m)
}

fn add(global: &GlobalArgs, args: &AddArgs) -> Result<(), CliError> {
    let now = Timestamp::now();
    let profile = profile_pce(global);
    let m = mark_from_args(args, profile.as_ref().map(|(_, p)| p), now)?;
    let mut s = store(global);
    let done = s.apply(MarkOp::Add(m), now).map_err(CliError::Marks)?;
    if let Some(b) = &done.backup {
        eprintln!("marks.toml had comments: kept in {}", b.display());
    }
    let id = done.ids.into_iter().next().unwrap_or_default();
    emit(
        global,
        &Added { id },
        &meta(profile.as_ref().map(|(n, _)| n.as_str()), None),
    )
}

fn rm(global: &GlobalArgs, args: &RmArgs) -> Result<(), CliError> {
    let mut s = store(global);
    let op = if args.expired {
        MarkOp::DeleteExpired
    } else {
        MarkOp::Delete(args.ids.clone())
    };
    let done = s.apply(op, Timestamp::now()).map_err(|e| match e {
        crate::marks::MarksError::Gone(id) => {
            CliError::Args(format!("unknown mark id {id}: nothing removed"))
        }
        e => CliError::Marks(e),
    })?;
    emit(
        global,
        &Removed {
            removed: done.removed,
        },
        &meta(None, None),
    )
}

/// Every mark as the entry that writes it (ids filled in), unknown
/// fields kept.
fn entries(s: &MarksStore) -> Result<MarkFile, CliError> {
    let loaded = s.loaded().ok_or_else(|| marks_error(s.error()))?;
    Ok(MarkFile {
        mark: loaded
            .set
            .marks
            .iter()
            .zip(&loaded.file.mark)
            .map(|(m, e)| entry_of(m, e.extra.clone()))
            .collect(),
        extra: loaded.file.extra.clone(),
    })
}

fn export(global: &GlobalArgs, args: &ExportArgs) -> Result<(), CliError> {
    let file = entries(&store(global))?;
    let text = match args.format {
        ExportFormat::Toml => render(&file),
        ExportFormat::Json => {
            let mut t = serde_json::to_string_pretty(&MarksJson { marks: file.mark })
                .map_err(|e| CliError::Other(e.to_string()))?;
            t.push('\n');
            t
        }
    };
    print!("{text}");
    Ok(())
}

/// The marks of an import file, every one checked before anything is
/// written; problems name the mark's position in that file.
pub fn read_import(path: &std::path::Path) -> Result<Vec<(Mark, toml::Table)>, CliError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| CliError::Args(format!("cannot read {}: {e}", path.display())))?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let entries: Vec<MarkEntry> = match ext.as_str() {
        "json" => {
            serde_json::from_str::<MarksJson>(&text)
                .map_err(|e| CliError::Args(format!("{}: {e}", path.display())))?
                .marks
        }
        "toml" => {
            toml::from_str::<MarkFile>(&text)
                .map_err(|e| {
                    CliError::Args(format!(
                        "{}: {}",
                        path.display(),
                        crate::marks::toml_error_text(&text, &e)
                    ))
                })?
                .mark
        }
        _ => {
            return Err(CliError::Args(format!(
                "{}: import reads a .json or a .toml file",
                path.display()
            )));
        }
    };
    let mut problems = Vec::new();
    let mut out: Vec<(Mark, toml::Table)> = Vec::new();
    // (id, position in the file) of the marks read so far.
    let mut seen: Vec<(String, usize)> = Vec::new();
    for (i, e) in entries.into_iter().enumerate() {
        match to_mark(&e) {
            Ok(m) => {
                let at = || {
                    let id = if m.id.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", m.id)
                    };
                    format!("mark {}{id}", i + 1)
                };
                if let Some((_, j)) = seen.iter().find(|(id, _)| !m.id.is_empty() && *id == m.id) {
                    problems.push(format!("{}: id used by mark {j} too", at()));
                }
                seen.push((m.id.clone(), i + 1));
                if m.comment.trim().is_empty() {
                    eprintln!("warning: {}: no comment (shown as (no comment))", at());
                }
                out.push((m, e.extra));
            }
            Err(msgs) => problems.extend(msgs.into_iter().map(|m| {
                let id = e.id_text().map(|id| format!(" ({id})")).unwrap_or_default();
                format!("mark {}{id}: {m}", i + 1)
            })),
        }
    }
    if problems.is_empty() {
        Ok(out)
    } else {
        Err(CliError::Args(format!(
            "import refused, nothing written: {}",
            problems.join("; ")
        )))
    }
}

fn import(global: &GlobalArgs, args: &ImportArgs) -> Result<(), CliError> {
    let marks = read_import(&args.file)?;
    let mut s = store(global);
    let done = s
        .apply(
            MarkOp::Import {
                marks,
                replace: args.replace,
            },
            Timestamp::now(),
        )
        .map_err(CliError::Marks)?;
    emit(
        global,
        &Imported {
            added: done.added,
            replaced: done.replaced,
            skipped: done.skipped,
        },
        &meta(None, None),
    )
}
