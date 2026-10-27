//! `illumio-tui snapshot`: local snapshots of the PCE's object lists
//! (`crate::snapshot`) and what changed between two of them. `save` and
//! `diff … now` read the PCE (the cached list reads every command uses;
//! `--no-cache` for fresh ones); `list`, `rm`, `prune` and a diff of two
//! files read nothing but the files.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{Meta, OutputFormat, TableRows, join_list};
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
    /// Delete one snapshot (local, no request)
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
    /// How many of the newest snapshots to keep
    #[arg(long, value_name = "N")]
    pub keep: usize,
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
        self.0.iter().map(|c| c.details.clone()).collect()
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

/// The profile's snapshot directory.
fn dir(ctx: &Context) -> Result<PathBuf, CliError> {
    let root = snapshot::default_root().ok_or_else(|| {
        CliError::Other("no data directory for snapshots: set HOME or XDG_DATA_HOME".into())
    })?;
    Ok(snapshot::profile_dir(&root, &ctx.config.profile_name))
}

fn identity(ctx: &Context) -> Identity {
    Identity {
        profile: ctx.config.profile_name.clone(),
        pce: ctx.config.host.clone(),
        org: ctx.config.org_id,
    }
}

fn metas(ctx: &Context) -> Result<Vec<SnapshotMeta>, CliError> {
    Ok(snapshot::list(&dir(ctx)?)?)
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

pub async fn run(ctx: &Context, cmd: &SnapshotCmd) -> Result<(), CliError> {
    match cmd {
        SnapshotCmd::Save(a) => save(ctx, a).await,
        SnapshotCmd::List => {
            let list = Listed(metas(ctx)?);
            let title = format!("snapshots · {} · {}", ctx.config.profile_name, list.0.len());
            ctx.emit_report(&list, &[(title, &list)], &ctx.meta("none", false, None))
        }
        SnapshotCmd::Diff(a) => diff_cmd(ctx, a).await,
        SnapshotCmd::Rm(a) => {
            let all = metas(ctx)?;
            let m = snapshot::find(&all, &a.timestamp).map_err(CliError::Args)?;
            snapshot::remove(m)?;
            removed(ctx, vec![m.clone()], all.len() - 1)
        }
        SnapshotCmd::Prune(a) => {
            let all = metas(ctx)?;
            let gone = snapshot::prune(&all, a.keep)?;
            let kept = all.len() - gone.len();
            removed(ctx, gone, kept)
        }
    }
}

fn removed(ctx: &Context, gone: Vec<SnapshotMeta>, kept: usize) -> Result<(), CliError> {
    let data = Removed {
        removed: gone
            .into_iter()
            .map(|m| RemovedRow {
                id: m.id,
                path: m.path,
            })
            .collect(),
        kept,
    };
    let title = format!("removed {} · kept {kept}", data.removed.len());
    ctx.emit_report(&data, &[(title, &data)], &ctx.meta("none", false, None))
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

async fn save(ctx: &Context, args: &SaveArgs) -> Result<(), CliError> {
    let readiness_args = ReadinessArgs {
        yes: args.yes,
        ..ReadinessArgs::default()
    };
    // Before any request: where the file goes, and the readiness query's cost.
    let dir = dir(ctx)?;
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
    let mut warnings = Vec::new();
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
    let meta = with_warnings(
        ctx.meta(cache_label(all_hit, ctx.cache_mode), false, None),
        warnings,
    );
    ctx.emit_report(&data, &[(title, &data)], &meta)
}

/// One side of a diff: a file, or a live read (`now`) of `what`.
enum Side {
    File(Snapshot, End),
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
    Ok(Side::File(s, end))
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

async fn diff_cmd(ctx: &Context, args: &DiffArgs) -> Result<(), CliError> {
    let a_spec = args.from.as_deref().unwrap_or("previous");
    let b_spec = args.to.as_deref().unwrap_or("latest");
    if a_spec == "now" && b_spec == "now" {
        return Err(CliError::Args(
            "now against now: one side must be a snapshot".into(),
        ));
    }
    let all = metas(ctx)?;
    let (a, b) = (side(&all, a_spec)?, side(&all, b_spec)?);
    // The live side, read only after both files loaded and matched.
    let mut cache = "none";
    let live = |other: &Snapshot| -> Result<(), CliError> {
        check_same(
            other,
            "the snapshot",
            &Snapshot {
                profile: ctx.config.profile_name.clone(),
                pce: ctx.config.host.clone(),
                org: ctx.config.org_id,
                ..other.clone()
            },
            "now",
        )
        .map_err(CliError::Args)
    };
    let now_end = |s: &Snapshot| End {
        id: "now".into(),
        taken_at: s.taken_at,
        label: None,
    };
    let ((sa, ea), (sb, eb)) = match (a, b) {
        (Side::File(sa, ea), Side::File(sb, eb)) => {
            check_same(&sa, &ea.id, &sb, &eb.id).map_err(CliError::Args)?;
            ((sa, ea), (sb, eb))
        }
        (Side::File(sa, ea), Side::Now) => {
            live(&sa)?;
            let t = take(
                &ctx.pce,
                ctx.cache_mode,
                live_what(&sa, &args.section),
                &identity(ctx),
                None,
            )
            .await?;
            cache = cache_label(t.all_hit, ctx.cache_mode);
            let e = now_end(&t.snapshot);
            ((sa, ea), (t.snapshot, e))
        }
        (Side::Now, Side::File(sb, eb)) => {
            live(&sb)?;
            let t = take(
                &ctx.pce,
                ctx.cache_mode,
                live_what(&sb, &args.section),
                &identity(ctx),
                None,
            )
            .await?;
            cache = cache_label(t.all_hit, ctx.cache_mode);
            let e = now_end(&t.snapshot);
            ((t.snapshot, e), (sb, eb))
        }
        (Side::Now, Side::Now) => unreachable!("refused above"),
    };
    let mut warnings = Vec::new();
    truncation_warnings(&ea.id, &sa, &mut warnings);
    truncation_warnings(&eb.id, &sb, &mut warnings);
    let report: SnapshotDiff = diff((&sa, ea), (&sb, eb), &args.section, &ctx.config.key_order());
    warnings.extend(
        report
            .not_compared
            .iter()
            .map(|n| format!("{}: not compared ({})", n.section, n.reason)),
    );
    let meta = with_warnings(ctx.meta(cache, false, None), warnings);
    let counts = Counts(&report.sections);
    let changes = Changes(&report.changes);
    let head = format!(
        "snapshot diff · {} → {} · {} change{}",
        report.from.id,
        report.to.id,
        report.changes.len(),
        if report.changes.len() == 1 { "" } else { "s" }
    );
    let tables: Vec<(String, &dyn TableRows)> = if ctx.format == OutputFormat::Csv {
        vec![(String::new(), &changes)]
    } else {
        vec![(head, &counts), ("changes".into(), &changes)]
    };
    ctx.emit_report(&report, &tables, &meta)?;
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
