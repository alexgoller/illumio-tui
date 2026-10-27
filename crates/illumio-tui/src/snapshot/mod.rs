//! Local snapshots of a PCE's object lists (#84): a baseline that costs the
//! PCE nothing to compare against later. One JSON file per snapshot under
//! `$XDG_DATA_HOME/illumio-tui/snapshots/<profile>/<UTC timestamp>.json`
//! (else `$HOME/.local/share/...`), written by `snapshot save` and the
//! TUI's `:snapshot`, read by `snapshot list|diff` (and #85's compare and
//! trend). Plain JSON: the crate has no gzip of its own. The files hold
//! what the reads returned (policy, labels, workloads, readiness), never
//! credentials.
//!
//! - [`take`]: the reads (cached, counted) that make a [`Snapshot`].
//! - [`list`], [`load`], [`save`], [`find`], [`remove`], [`prune`]: the
//!   files of one profile's directory.
//! - [`diff`]: what changed between two snapshots, per section.
pub mod diff;
mod take;

pub use take::{Identity, Taken, What, take};

use pce::models::{EnforcementBoundary, IpList, Label, LabelGroup, RuleSet, Service, Workload};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The file format this release writes and the newest it reads.
pub const VERSION: u32 = 1;
const EXT: &str = "json";

/// One snapshot file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub version: u32,
    pub profile: String,
    /// The PCE host the reads went to.
    pub pce: String,
    pub org: u64,
    pub taken_at: jiff::Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The reads that hit their row cap (`workloads`, `draft_rule_sets` …):
    /// that part of a section is partial.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub truncated: Vec<String>,
    pub sections: Sections,
}

/// What a snapshot holds; a section not asked for is absent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Sections {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<PolicySection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labels: Option<Vec<Label>>,
    /// Each workload as the list read it, without interfaces or
    /// description, its `agent` cut to `agent.status` (what VEN health,
    /// status and version read).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workloads: Option<Vec<Workload>>,
    /// `readiness`'s `data` (`--with-readiness`), as that command prints it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readiness: Option<serde_json::Value>,
}

/// The policy: draft and active rulesets (rules and inline deny rules) and
/// the draft objects they name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PolicySection {
    pub draft_rule_sets: Vec<RuleSet>,
    pub active_rule_sets: Vec<RuleSet>,
    pub label_groups: Vec<LabelGroup>,
    pub ip_lists: Vec<IpList>,
    pub services: Vec<Service>,
    pub enforcement_boundaries: Vec<EnforcementBoundary>,
}

/// The sections a snapshot can hold, as `--what` and the files name them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, clap::ValueEnum)]
pub enum Section {
    Policy,
    Labels,
    Workloads,
    Readiness,
}

impl Section {
    pub fn as_str(self) -> &'static str {
        match self {
            Section::Policy => "policy",
            Section::Labels => "labels",
            Section::Workloads => "workloads",
            Section::Readiness => "readiness",
        }
    }
}

impl Sections {
    /// The sections present, in file order.
    pub fn names(&self) -> Vec<Section> {
        let mut out = Vec::new();
        if self.policy.is_some() {
            out.push(Section::Policy);
        }
        if self.labels.is_some() {
            out.push(Section::Labels);
        }
        if self.workloads.is_some() {
            out.push(Section::Workloads);
        }
        if self.readiness.is_some() {
            out.push(Section::Readiness);
        }
        out
    }

    /// Each present section's item count and size in the file (bytes of
    /// its JSON).
    pub fn sizes(&self) -> Vec<SectionSize> {
        let mut out = Vec::new();
        if let Some(p) = &self.policy {
            out.push(SectionSize {
                section: "policy",
                items: p.draft_rule_sets.len()
                    + p.active_rule_sets.len()
                    + p.label_groups.len()
                    + p.ip_lists.len()
                    + p.services.len()
                    + p.enforcement_boundaries.len(),
                bytes: bytes(p),
            });
        }
        if let Some(l) = &self.labels {
            out.push(SectionSize {
                section: "labels",
                items: l.len(),
                bytes: bytes(l),
            });
        }
        if let Some(w) = &self.workloads {
            out.push(SectionSize {
                section: "workloads",
                items: w.len(),
                bytes: bytes(w),
            });
        }
        if let Some(r) = &self.readiness {
            out.push(SectionSize {
                section: "readiness",
                items: r
                    .get("apps")
                    .and_then(serde_json::Value::as_array)
                    .map_or(0, Vec::len),
                bytes: bytes(r),
            });
        }
        out
    }
}

/// The length of `v`'s JSON.
fn bytes<T: Serialize>(v: &T) -> u64 {
    serde_json::to_vec(v).map_or(0, |b| b.len() as u64)
}

/// One section's share of a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SectionSize {
    pub section: &'static str,
    /// Rulesets, label groups, IP lists, services and boundaries for
    /// `policy`; labels; workloads; apps for `readiness`.
    pub items: usize,
    pub bytes: u64,
}

/// A workload as a snapshot keeps it: interfaces and description dropped,
/// `agent` cut to `{"status": …}`.
pub fn trim_workload(mut w: Workload) -> Workload {
    w.interfaces.clear();
    w.description = None;
    w.agent = w.agent.and_then(|a| {
        a.get("status")
            .map(|s| serde_json::json!({ "status": s.clone() }))
    });
    w
}

/// `$XDG_DATA_HOME/illumio-tui/snapshots`, else
/// `$HOME/.local/share/illumio-tui/snapshots`; `None` when neither is set
/// (never a path relative to the working directory).
pub fn root_from(xdg_data_home: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    let base = match (xdg_data_home, home) {
        (Some(x), _) => x.to_path_buf(),
        (None, Some(h)) => h.join(".local").join("share"),
        (None, None) => return None,
    };
    Some(base.join("illumio-tui").join("snapshots"))
}

/// [`root_from`] with the environment; on Windows without `HOME`, the
/// local app data folder.
pub fn default_root() -> Option<PathBuf> {
    let var = |name| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let home = var("HOME");
    #[cfg(windows)]
    if var("XDG_DATA_HOME").is_none() && home.is_none() {
        return dirs::data_local_dir().map(|d| d.join("illumio-tui").join("snapshots"));
    }
    root_from(var("XDG_DATA_HOME").as_deref(), home.as_deref())
}

/// The profile's directory under `root`; the profile name made safe for a
/// directory name as the names cache does.
pub fn profile_dir(root: &Path, profile: &str) -> PathBuf {
    root.join(crate::names::dir_name(profile))
}

/// A snapshot file as `list` sees it: what its header says, or why it
/// cannot be read.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SnapshotMeta {
    /// The file name without `.json`: the UTC time it was taken
    /// (`2026-10-27T031500Z`), what `diff` and `rm` take.
    pub id: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub version: Option<u32>,
    pub taken_at: Option<jiff::Timestamp>,
    pub label: Option<String>,
    pub sections: Vec<&'static str>,
    /// Why the file cannot be read (not JSON, a newer format); `None` when
    /// it can.
    pub problem: Option<String>,
}

/// The fields `list` reads; the sections are skipped, not parsed.
#[derive(Deserialize)]
struct Header {
    version: u32,
    taken_at: Option<jiff::Timestamp>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    sections: std::collections::BTreeMap<String, serde::de::IgnoredAny>,
}

fn newer(version: u32) -> String {
    format!("format version {version}: newer than this illumio-tui reads ({VERSION}); upgrade")
}

fn meta_of(path: &Path) -> std::io::Result<SnapshotMeta> {
    let bytes = std::fs::read(path)?;
    let id = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut m = SnapshotMeta {
        id,
        path: path.to_path_buf(),
        bytes: bytes.len() as u64,
        version: None,
        taken_at: None,
        label: None,
        sections: Vec::new(),
        problem: None,
    };
    match serde_json::from_slice::<Header>(&bytes) {
        Err(e) => m.problem = Some(format!("not a snapshot file: {e}")),
        Ok(h) => {
            m.version = Some(h.version);
            m.taken_at = h.taken_at;
            m.label = h.label;
            m.sections = [
                Section::Policy,
                Section::Labels,
                Section::Workloads,
                Section::Readiness,
            ]
            .into_iter()
            .map(Section::as_str)
            .filter(|s| h.sections.contains_key(*s))
            .collect();
            if h.version > VERSION {
                m.problem = Some(newer(h.version));
            }
        }
    }
    Ok(m)
}

/// The snapshots in `dir`, oldest first (by id: the UTC time); a missing
/// directory has none.
pub fn list(dir: &Path) -> std::io::Result<Vec<SnapshotMeta>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for e in entries {
        let path = e?.path();
        if path.extension().and_then(|x| x.to_str()) == Some(EXT) && path.is_file() {
            out.push(meta_of(&path)?);
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

/// The snapshot at `path`, or why it cannot be read: not JSON, not a
/// snapshot, or a newer format than this release reads.
pub fn load(path: &Path) -> Result<Snapshot, String> {
    let name = path.display();
    let bytes = std::fs::read(path).map_err(|e| format!("{name}: {e}"))?;
    let v: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("{name}: not a snapshot file: {e}"))?;
    match v.get("version").and_then(serde_json::Value::as_u64) {
        Some(n) if n > u64::from(VERSION) => {
            return Err(format!("{name}: {}", newer(n as u32)));
        }
        Some(_) => {}
        None => return Err(format!("{name}: not a snapshot file: no version")),
    }
    serde_json::from_value(v).map_err(|e| format!("{name}: not a snapshot file: {e}"))
}

/// The id of a snapshot taken at `t`: its UTC time, without colons (a file
/// name on every system).
pub fn id_for(t: jiff::Timestamp) -> String {
    t.strftime("%Y-%m-%dT%H%M%SZ").to_string()
}

/// A file written by [`save`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedFile {
    pub id: String,
    pub path: PathBuf,
    pub bytes: u64,
}

/// Writes `snap` into `dir` (created when missing) as `<id>.json`, through
/// a temporary file renamed into place; a second snapshot in the same
/// second gets `-1`, `-2` … after the time.
pub fn save(dir: &Path, snap: &Snapshot) -> std::io::Result<SavedFile> {
    std::fs::create_dir_all(dir)?;
    let body = serde_json::to_vec(snap).map_err(std::io::Error::other)?;
    let base = id_for(snap.taken_at);
    let mut n = 0u32;
    let (id, path) = loop {
        let id = if n == 0 {
            base.clone()
        } else {
            format!("{base}-{n}")
        };
        let path = dir.join(format!("{id}.{EXT}"));
        if !path.exists() {
            break (id, path);
        }
        n += 1;
    };
    let tmp = dir.join(format!(".{id}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, &body)?;
    if let Err(e) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(SavedFile {
        id,
        path,
        bytes: body.len() as u64,
    })
}

/// The snapshot `spec` names among `metas` (oldest first): `latest`,
/// `previous` (the one before it), or a unique prefix of an id.
pub fn find<'a>(metas: &'a [SnapshotMeta], spec: &str) -> Result<&'a SnapshotMeta, String> {
    let n = metas.len();
    let have = |need: usize| {
        format!(
            "{spec}: {n} snapshot{} of this profile, {need} needed",
            if n == 1 { "" } else { "s" }
        )
    };
    match spec {
        "latest" => metas.last().ok_or_else(|| have(1)),
        "previous" => n.checked_sub(2).map(|i| &metas[i]).ok_or_else(|| have(2)),
        _ => {
            if let Some(m) = metas.iter().find(|m| m.id == spec) {
                return Ok(m);
            }
            let hits: Vec<&SnapshotMeta> =
                metas.iter().filter(|m| m.id.starts_with(spec)).collect();
            match hits.as_slice() {
                [one] => Ok(one),
                [] => Err(format!(
                    "no snapshot {spec:?} of this profile (snapshot list shows them)"
                )),
                many => Err(format!(
                    "{spec:?} names {} snapshots: {}",
                    many.len(),
                    many.iter()
                        .map(|m| m.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            }
        }
    }
}

/// Deletes one snapshot file.
pub fn remove(meta: &SnapshotMeta) -> std::io::Result<()> {
    std::fs::remove_file(&meta.path)
}

/// Deletes all but the newest `keep` of `metas` (oldest first); returns
/// the ones deleted.
pub fn prune(metas: &[SnapshotMeta], keep: usize) -> std::io::Result<Vec<SnapshotMeta>> {
    let cut = metas.len().saturating_sub(keep);
    let mut removed = Vec::new();
    for m in &metas[..cut] {
        remove(m)?;
        removed.push(m.clone());
    }
    Ok(removed)
}

#[cfg(test)]
mod tests;
