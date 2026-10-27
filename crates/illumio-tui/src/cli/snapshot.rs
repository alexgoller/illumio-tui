//! `illumio-tui snapshot`: local snapshots of the PCE's object lists
//! (`crate::snapshot`) and what changed between two of them. `save` and
//! `diff … now` read the PCE (the cached list reads every command uses;
//! `--no-cache` for fresh ones); `list`, `rm`, `prune` and a diff of two
//! files read nothing but the files.
use super::GlobalArgs;
use super::context::{Context, cache_label, config_inputs};
use super::error::CliError;
use super::output::{Meta, OutputFormat, TableRows, choose_format, emit_report, join_list};
use super::readiness::{ReadinessArgs, evaluate, guard};
use crate::snapshot::diff::{Change, Count, DiffSection, End, SnapshotDiff, check_same, diff};
use crate::snapshot::{self, Identity, Section, SectionSize, Snapshot, SnapshotMeta, What, take};
use clap::{Args, Subcommand};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Subcommand, Debug)]
pub enum SnapshotCmd {
    /// Read the policy, labels and workloads and keep them in a local file (one GET per list,
    /// cached as every read; --no-cache for fresh ones)
    ///
    /// Policy: the draft and active rulesets with their rules, the label groups, IP lists,
    /// services and enforcement boundaries (six GETs); labels: one; workloads: one unfiltered
    /// list (href, names, labels, enforcement mode, managed, the VEN's status). Written to
    /// $XDG_DATA_HOME/illumio-tui/snapshots/<profile>/<UTC time>.json (else
    /// ~/.local/share/illumio-tui/snapshots/…). Prints the path and each section's size.
    Save(SaveArgs),
    /// The profile's snapshots, oldest first: time, label, sections, size (local, no request)
    List,
    /// What changed between two snapshots: rulesets and rules, labels, label groups, IP lists,
    /// services, boundaries, workloads (added, removed, changed with the fields)
    ///
    /// A and B: a snapshot's time (a prefix is enough), latest, previous, or now (a live read
    /// of the sections the other side has, as save reads them). Default: previous latest.
    /// Snapshots of another profile or PCE are refused (exit 2). Exit 0; with
    /// --fail-on-change, exit 9 when anything differs (after the report).
    Diff(DiffArgs),
    /// Delete one snapshot, also an unreadable one named by its time (local, no request)
    Rm(RmArgs),
    /// Delete all but the newest N snapshots of the profile (local, no request)
    Prune(PruneArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct SaveArgs {
    /// The sections to read (default all: policy, labels, workloads); comma-separated
    #[arg(long, value_enum, value_delimiter = ',', value_name = "SECTIONS")]
    pub what: Vec<WhatArg>,
    /// Also judge every app's readiness and keep it: readiness's one Explorer query (7d,
    /// re-evaluated against the draft) and its reads, through its cost guard
    #[arg(long)]
    pub with_readiness: bool,
    /// With --with-readiness: confirm a query the cost guard calls expensive
    #[arg(long, requires = "with_readiness")]
    pub yes: bool,
    /// A note kept with the snapshot (`before the Q4 change`)
    #[arg(long, value_name = "TEXT")]
    pub label: Option<String>,
}

/// The sections `--what` takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum WhatArg {
    Policy,
    Labels,
    Workloads,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DiffArgs {
    /// The older side: a snapshot's time or a prefix of it, latest, previous, now (default previous)
    #[arg(value_name = "A")]
    pub from: Option<String>,
    /// The newer side, as A (default latest)
    #[arg(value_name = "B")]
    pub to: Option<String>,
    /// Only these sections (comma-separated or repeatable)
    #[arg(long, value_enum, value_delimiter = ',', value_name = "SECTIONS")]
    pub section: Vec<DiffSection>,
    /// Exit 9 (after the report) when anything differs
    #[arg(long)]
    pub fail_on_change: bool,
}

#[derive(Args, Debug, Clone)]
pub struct RmArgs {
    /// The snapshot's time (a unique prefix is enough), latest or previous
    pub timestamp: String,
}

#[derive(Args, Debug, Clone)]
pub struct PruneArgs {
    /// How many of the newest readable snapshots to keep, at least 1 (`rm` deletes the last);
    /// unreadable files (corrupt, a newer format) are never deleted
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u64).range(1..))]
    pub keep: u64,
}

/// The CSV columns of each table, in order (`columns::csv_columns`).
pub const SAVE_HEADERS: [&str; 3] = ["section", "items", "bytes"];
pub const LIST_HEADERS: [&str; 7] = [
    "id", "taken_at", "label", "sections", "bytes", "problem", "path",
];
pub const DIFF_HEADERS: [&str; 7] = [
    "section", "change", "scope", "name", "fields", "details", "href",
];
pub const RM_HEADERS: [&str; 2] = ["id", "path"];

/// `save`'s `data`.
#[derive(Debug, Serialize)]
pub struct Saved {
    pub id: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub profile: String,
    pub pce: String,
    pub org: u64,
    pub taken_at: jiff::Timestamp,
    pub label: Option<String>,
    pub sections: Vec<SectionSize>,
    /// The reads that hit their row cap: those sections are partial.
    pub truncated: Vec<String>,
}

impl TableRows for Saved {
    fn headers(&self) -> Vec<&'static str> {
        SAVE_HEADERS.to_vec()
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.sections
            .iter()
            .map(|s| {
                vec![
                    s.section.to_string(),
                    s.items.to_string(),
                    s.bytes.to_string(),
                ]
            })
            .collect()
    }
}

#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct Listed(pub Vec<SnapshotMeta>);

fn opt<T: ToString>(v: &Option<T>) -> String {
    v.as_ref().map(ToString::to_string).unwrap_or_default()
}

impl TableRows for Listed {
    fn headers(&self) -> Vec<&'static str> {
        LIST_HEADERS[..6].to_vec()
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .map(|m| {
                vec![
                    m.id.clone(),
                    opt(&m.taken_at),
                    opt(&m.label),
                    m.sections.join(","),
                    m.bytes.to_string(),
                    opt(&m.problem),
                ]
            })
            .collect()
    }
    fn full_headers(&self) -> Vec<&'static str> {
        LIST_HEADERS.to_vec()
    }
    fn full_rows(&self) -> Vec<Vec<String>> {
        let rows = self.rows();
        rows.into_iter()
            .zip(&self.0)
            .map(|(mut r, m)| {
                r[3] = join_list(&m.sections);
                r.push(m.path.display().to_string());
                r
            })
            .collect()
    }
}

struct Counts<'a>(&'a [Count]);

impl TableRows for Counts<'_> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["section", "added", "removed", "changed"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .map(|c| {
                vec![
                    c.section.to_string(),
                    c.added.to_string(),
                    c.removed.to_string(),
                    c.changed.to_string(),
                ]
            })
            .collect()
    }
}

struct Changes<'a>(&'a [Change]);

impl TableRows for Changes<'_> {
    fn headers(&self) -> Vec<&'static str> {
        DIFF_HEADERS[..5].to_vec()
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .map(|c| {
                vec![
                    c.section.to_string(),
                    c.change.to_string(),
                    c.scope.unwrap_or("").to_string(),
                    c.name.clone(),
                    c.fields.join(","),
                ]
            })
            .collect()
    }
    /// The changed lines under each changed object.
    fn notes(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .map(|c| c.details.iter().map(|d| format!("  {d}")).collect())
            .collect()
    }
    fn full_headers(&self) -> Vec<&'static str> {
        DIFF_HEADERS.to_vec()
    }
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .map(|c| {
                vec![
                    c.section.to_string(),
                    c.change.to_string(),
                    c.scope.unwrap_or("").to_string(),
                    c.name.clone(),
                    join_list(&c.fields),
                    join_list(&c.details),
                    c.href.clone(),
                ]
            })
            .collect()
    }
}

#[derive(Debug, Serialize)]
pub struct RemovedRow {
    pub id: String,
    pub path: PathBuf,
}

/// `rm`'s and `prune`'s `data`.
#[derive(Debug, Serialize)]
pub struct Removed {
    pub removed: Vec<RemovedRow>,
    /// Snapshots of the profile left.
    pub kept: usize,
}

impl TableRows for Removed {
    fn headers(&self) -> Vec<&'static str> {
        RM_HEADERS.to_vec()
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.removed
            .iter()
            .map(|r| vec![r.id.clone(), r.path.display().to_string()])
            .collect()
    }
}

/// Where a snapshot command prints, and for whom.
struct Out {
    format: OutputFormat,
    profile: String,
    started: std::time::Instant,
}

impl Out {
    /// The meta of a command that sent nothing.
    fn local_meta(&self) -> Meta {
        Meta {
            profile: self.profile.clone(),
            elapsed_ms: self.started.elapsed().as_millis(),
            cache: "none",
            ..Meta::default()
        }
    }
}

/// The profile's snapshot directory.
fn dir_for(profile: &str) -> Result<PathBuf, CliError> {
    let root = snapshot::default_root().ok_or_else(|| {
        CliError::Other("no data directory for snapshots: set HOME or XDG_DATA_HOME".into())
    })?;
    Ok(snapshot::profile_dir(&root, profile))
}

/// The profile the command is for, by name only: the config file read,
/// no credentials resolved (no `api_secret_cmd` run), as `Context` would
/// name it.
fn profile_only(global: &GlobalArgs) -> Result<String, CliError> {
    let (file, overrides, env) = config_inputs(global)?;
    let named = overrides.profile.clone().or_else(|| {
        env.get("ILLUMIO_PROFILE")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    });
    Ok(crate::config::profile_name(&file, named).0)
}

/// Whether `cmd` reads only files: everything but `save` and a diff with
/// a `now` side.
pub fn is_local(cmd: &SnapshotCmd) -> bool {
    match cmd {
        SnapshotCmd::Save(_) => false,
        SnapshotCmd::Diff(a) => !a.sides().contains(&"now"),
        SnapshotCmd::List | SnapshotCmd::Rm(_) | SnapshotCmd::Prune(_) => true,
    }
}

impl DiffArgs {
    /// A and B with their defaults.
    fn sides(&self) -> [&str; 2] {
        [
            self.from.as_deref().unwrap_or("previous"),
            self.to.as_deref().unwrap_or("latest"),
        ]
    }
}

/// The commands that read only files: the profile resolved by name, no
/// client built.
pub fn run_local(global: &GlobalArgs, cmd: &SnapshotCmd) -> Result<(), CliError> {
    let out = Out {
        format: choose_format(global.json, global.table, global.csv),
        profile: profile_only(global)?,
        started: std::time::Instant::now(),
    };
    let all = snapshot::list(&dir_for(&out.profile)?)?;
    match cmd {
        SnapshotCmd::List => {
            let list = Listed(all);
            let title = format!("snapshots · {} · {}", out.profile, list.0.len());
            emit_report(out.format, &list, &[(title, &list)], &out.local_meta())
        }
        SnapshotCmd::Rm(a) => {
            let m = snapshot::find(&all, &a.timestamp).map_err(CliError::Args)?;
            snapshot::remove(m)?;
            removed(&out, vec![m.clone()], &all, 1)
        }
        SnapshotCmd::Prune(a) => {
            let gone = snapshot::prune(&all, a.keep as usize)?;
            let n = gone.len();
            removed(&out, gone, &all, n)
        }
        SnapshotCmd::Diff(a) => {
            let [sa, sb] = a.sides();
            let (Side::File(fa, ea), Side::File(fb, eb)) = (side(&all, sa)?, side(&all, sb)?)
            else {
                unreachable!("a local diff has no now side")
            };
            check_same(&fa, &ea.id, &fb, &eb.id).map_err(CliError::Args)?;
            print_diff(&out, a, (*fa, ea), (*fb, eb), out.local_meta())
        }
        SnapshotCmd::Save(_) => unreachable!("save reads the PCE"),
    }
}

/// `rm`'s and `prune`'s report: what went, how many readable snapshots
/// are left, and the unreadable files left alone.
fn removed(
    out: &Out,
    gone: Vec<SnapshotMeta>,
    all: &[SnapshotMeta],
    n_gone: usize,
) -> Result<(), CliError> {
    let data = Removed {
        removed: gone
            .into_iter()
            .map(|m| RemovedRow {
                id: m.id,
                path: m.path,
            })
            .collect(),
        kept: all.len() - n_gone,
    };
    let unreadable = all
        .iter()
        .filter(|m| m.problem.is_some() && !data.removed.iter().any(|r| r.id == m.id))
        .count();
    let warnings = if unreadable > 0 {
        vec![format!(
            "{unreadable} unreadable snapshot file{} left alone (snapshot list shows why)",
            if unreadable == 1 { "" } else { "s" }
        )]
    } else {
        Vec::new()
    };
    let title = format!("removed {} · kept {}", data.removed.len(), data.kept);
    emit_report(
        out.format,
        &data,
        &[(title, &data)],
        &with_warnings(out.local_meta(), warnings),
    )
}

/// A warning per read a snapshot holds cut.
fn truncation_warnings(id: &str, s: &Snapshot, out: &mut Vec<String>) {
    for t in &s.truncated {
        out.push(format!(
            "{id}: the {t} read hit its row cap: that part is partial"
        ));
    }
}

fn with_warnings(mut meta: Meta, warnings: Vec<String>) -> Meta {
    if !warnings.is_empty() {
        meta.warnings.get_or_insert_with(Vec::new).extend(warnings);
    }
    meta
}

/// `save` and a diff with a `now` side: they read the PCE.
pub async fn run(ctx: &Context, cmd: &SnapshotCmd) -> Result<(), CliError> {
    match cmd {
        SnapshotCmd::Save(a) => save(ctx, a).await,
        SnapshotCmd::Diff(a) => diff_now(ctx, a).await,
        SnapshotCmd::List | SnapshotCmd::Rm(_) | SnapshotCmd::Prune(_) => {
            unreachable!("run_local")
        }
    }
}

fn what_of(args: &SaveArgs) -> What {
    if args.what.is_empty() {
        return What::default();
    }
    What {
        policy: args.what.contains(&WhatArg::Policy),
        labels: args.what.contains(&WhatArg::Labels),
        workloads: args.what.contains(&WhatArg::Workloads),
    }
}

fn identity(ctx: &Context) -> Identity {
    Identity::of(&ctx.config.profile_name, ctx.pce.client())
}

async fn save(ctx: &Context, args: &SaveArgs) -> Result<(), CliError> {
    let readiness_args = ReadinessArgs {
        yes: args.yes,
        ..ReadinessArgs::default()
    };
    // Before any request: where the file goes, and the readiness query's cost.
    let dir = dir_for(&ctx.config.profile_name)?;
    if args.with_readiness {
        guard(ctx, &readiness_args)?;
    }
    let taken = take(
        &ctx.pce,
        ctx.cache_mode,
        what_of(args),
        &identity(ctx),
        args.label.clone(),
    )
    .await?;
    let mut snap = taken.snapshot;
    let mut all_hit = taken.all_hit;
    let mut warnings = taken.warnings;
    if args.with_readiness {
        let ev = evaluate(ctx, &readiness_args).await?;
        all_hit &= ev.meta.cache == "hit";
        warnings.extend(ev.meta.warnings.clone().unwrap_or_default());
        snap.sections.readiness =
            Some(serde_json::to_value(&ev.report).map_err(|e| CliError::Other(e.to_string()))?);
    }
    truncation_warnings("this snapshot", &snap, &mut warnings);
    let file = snapshot::save(&dir, &snap)?;
    let data = Saved {
        id: file.id,
        path: file.path,
        bytes: file.bytes,
        profile: snap.profile.clone(),
        pce: snap.pce.clone(),
        org: snap.org,
        taken_at: snap.taken_at,
        label: snap.label.clone(),
        sections: snap.sections.sizes(),
        truncated: snap.truncated.clone(),
    };
    let title = format!(
        "snapshot {} · {} · {} bytes",
        data.id,
        data.path.display(),
        data.bytes
    );
    // With --table and --csv meta.warnings reach stderr on the meta line;
    // with --json they are in the envelope.
    let meta = with_warnings(
        ctx.meta(cache_label(all_hit, ctx.cache_mode), false, None),
        warnings,
    );
    ctx.emit_report(&data, &[(title, &data)], &meta)
}

/// One side of a diff: a file, or a live read (`now`).
enum Side {
    File(Box<Snapshot>, End),
    Now,
}

fn side(all: &[SnapshotMeta], spec: &str) -> Result<Side, CliError> {
    if spec == "now" {
        return Ok(Side::Now);
    }
    let m = snapshot::find(all, spec).map_err(CliError::Args)?;
    let s = snapshot::load(&m.path).map_err(CliError::Other)?;
    let end = End {
        id: m.id.clone(),
        taken_at: s.taken_at,
        label: s.label.clone(),
    };
    Ok(Side::File(Box::new(s), end))
}

/// What a live side reads: the sections the file side has that the
/// wanted diff sections need.
fn live_what(other: &Snapshot, wanted: &[DiffSection]) -> What {
    let has = other.sections.names();
    let need = |src: Section| {
        has.contains(&src) && (wanted.is_empty() || wanted.iter().any(|w| w.source() == src))
    };
    What {
        policy: need(Section::Policy),
        labels: need(Section::Labels),
        workloads: need(Section::Workloads),
    }
}

/// A diff with one `now` side: the file loaded and matched against this
/// profile's PCE first, then only the sections it holds read live.
async fn diff_now(ctx: &Context, args: &DiffArgs) -> Result<(), CliError> {
    let [a_spec, b_spec] = args.sides();
    if a_spec == "now" && b_spec == "now" {
        return Err(CliError::Args(
            "now against now: one side must be a snapshot".into(),
        ));
    }
    let all = snapshot::list(&dir_for(&ctx.config.profile_name)?)?;
    let (file, now_first) = match (side(&all, a_spec)?, side(&all, b_spec)?) {
        (Side::File(f, e), Side::Now) => ((f, e), false),
        (Side::Now, Side::File(f, e)) => ((f, e), true),
        _ => unreachable!("one side is now"),
    };
    let me = identity(ctx);
    let live_id = Snapshot {
        profile: me.profile.clone(),
        pce: me.pce.clone(),
        org: me.org,
        ..(*file.0).clone()
    };
    check_same(&file.0, &file.1.id, &live_id, "now").map_err(CliError::Args)?;
    let t = take(
        &ctx.pce,
        ctx.cache_mode,
        live_what(&file.0, &args.section),
        &me,
        None,
    )
    .await?;
    let now_end = End {
        id: "now".into(),
        taken_at: t.snapshot.taken_at,
        label: None,
    };
    let meta = with_warnings(
        ctx.meta(cache_label(t.all_hit, ctx.cache_mode), false, None),
        t.warnings,
    );
    let out = Out {
        format: ctx.format,
        profile: ctx.config.profile_name.clone(),
        started: std::time::Instant::now(),
    };
    let (f, live) = ((*file.0, file.1), (t.snapshot, now_end));
    let (a, b) = if now_first { (live, f) } else { (f, live) };
    print_diff(&out, args, a, b, meta)
}

/// Compares and prints; exit 9 after the report with `--fail-on-change`.
/// Labels in the order the PCE lists them (every key, none hidden).
fn print_diff(
    out: &Out,
    args: &DiffArgs,
    (sa, ea): (Snapshot, End),
    (sb, eb): (Snapshot, End),
    meta: Meta,
) -> Result<(), CliError> {
    let mut warnings = Vec::new();
    truncation_warnings(&ea.id, &sa, &mut warnings);
    truncation_warnings(&eb.id, &sb, &mut warnings);
    let report: SnapshotDiff = diff(
        (&sa, ea),
        (&sb, eb),
        &args.section,
        &pce::KeyOrder::default(),
    );
    warnings.extend(
        report
            .not_compared
            .iter()
            .map(|n| format!("{}: not compared ({})", n.section, n.reason)),
    );
    let meta = with_warnings(meta, warnings);
    let counts = Counts(&report.sections);
    let changes = Changes(&report.changes);
    let head = format!(
        "snapshot diff · {} → {} · {} change{}",
        report.from.id,
        report.to.id,
        report.changes.len(),
        if report.changes.len() == 1 { "" } else { "s" }
    );
    let tables: Vec<(String, &dyn TableRows)> = if out.format == OutputFormat::Csv {
        vec![(String::new(), &changes)]
    } else {
        vec![(head, &counts), ("changes".into(), &changes)]
    };
    emit_report(out.format, &report, &tables, &meta)?;
    if args.fail_on_change && report.changed {
        return Err(CliError::Changed(format!(
            "{} change{} between {} and {}",
            report.changes.len(),
            if report.changes.len() == 1 { "" } else { "s" },
            report.from.id,
            report.to.id
        )));
    }
    Ok(())
}
