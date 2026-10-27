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
    /// `None`: not read, the PCE has no such endpoint for this user (404
    /// or 403); `diff` then does not compare boundaries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enforcement_boundaries: Option<Vec<EnforcementBoundary>>,
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
                    + p.enforcement_boundaries.as_ref().map_or(0, Vec::len),
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
        let named = path
            .file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|s| parse_id(s).is_some());
        if named && path.extension().and_then(|x| x.to_str()) == Some(EXT) && path.is_file() {
            out.push(meta_of(&path)?);
        }
    }
    out.sort_by_key(|m| parse_id(&m.id).map(|(t, n)| (t.to_string(), n)));
    Ok(out)
}

/// A snapshot id's time part and same-second number (`-N`, else 0);
/// `None` for a name that is no snapshot id
/// (`YYYY-MM-DDTHHMMSSZ[-N]`).
pub fn parse_id(id: &str) -> Option<(&str, u32)> {
    let (time, n) = match id.get(18..) {
        Some("") => (id, 0),
        Some(rest) => {
            let digits = rest.strip_prefix('-')?;
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            (&id[..18], digits.parse().ok()?)
        }
        None => return None,
    };
    let ok = time.bytes().enumerate().all(|(i, b)| match i {
        4 | 7 => b == b'-',
        10 => b == b'T',
        17 => b == b'Z',
        _ => b.is_ascii_digit(),
    });
    ok.then_some((time, n))
}

/// The snapshots that can be read, oldest first: what `latest`,
/// `previous` and `prune` consider.
pub fn usable(metas: &[SnapshotMeta]) -> Vec<&SnapshotMeta> {
    metas.iter().filter(|m| m.problem.is_none()).collect()
}

/// The snapshot at `path`, or why it cannot be read: not JSON, not a
/// snapshot, or a newer format than this release reads.
pub fn load(path: &Path) -> Result<Snapshot, String> {
    let name = path.display();
    let bytes = std::fs::read(path).map_err(|e| format!("{name}: {e}"))?;
    // The version first (the rest skipped), so a newer file says so.
    #[derive(Deserialize)]
    struct Version {
        version: u64,
    }
    let v = serde_json::from_slice::<Version>(&bytes)
        .map_err(|e| format!("{name}: not a snapshot file: {e}"))?;
    if v.version > u64::from(VERSION) {
        return Err(format!("{name}: {}", newer(v.version as u32)));
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("{name}: not a snapshot file: {e}"))
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

/// A temporary file older than this is a crashed save's: removed.
const STALE_TMP: std::time::Duration = std::time::Duration::from_secs(3600);

/// Writes `snap` into `dir` (created when missing) as `<id>.json`: a
/// temporary file written and synced, then hard-linked to the first free
/// name (the link fails on a taken one, so two writers in the same second
/// never overwrite each other: the second gets `-1`, `-2` …). Temporary
/// files a crashed save left over an hour ago are removed.
pub fn save(dir: &Path, snap: &Snapshot) -> std::io::Result<SavedFile> {
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    clean_stale(dir);
    let body = serde_json::to_vec(snap).map_err(std::io::Error::other)?;
    let base = id_for(snap.taken_at);
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = dir.join(format!(".{base}.{}-{seq}.tmp", std::process::id()));
    let written = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&body)?;
        f.sync_all()
    })();
    // Where hard links are not supported (FAT, exFAT, some network
    // shares): the name reserved with `create_new`, then renamed over.
    let fallback = |e: &std::io::Error| {
        matches!(
            e.kind(),
            std::io::ErrorKind::Unsupported | std::io::ErrorKind::PermissionDenied
        ) || e.raw_os_error().is_some_and(is_no_link_errno)
    };
    let linked = written.and_then(|()| {
        let mut n = 0u32;
        loop {
            let id = if n == 0 {
                base.clone()
            } else {
                format!("{base}-{n}")
            };
            let path = dir.join(format!("{id}.{EXT}"));
            let linked = match std::fs::hard_link(&tmp, &path) {
                Err(e) if fallback(&e) => reserve_and_rename(&tmp, &path),
                other => other,
            };
            match linked {
                Ok(()) => return Ok((id, path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
                Err(e) => return Err(e),
            }
        }
    });
    let _ = std::fs::remove_file(&tmp);
    let (id, path) = linked?;
    Ok(SavedFile {
        id,
        path,
        bytes: body.len() as u64,
    })
}

/// `EPERM`/`EXDEV`-like errors a filesystem without hard links gives
/// (`EPERM` 1, `EMLINK` 31, `ENOTSUP`/`EOPNOTSUPP` 45/95, `ENOSYS` 38/78).
fn is_no_link_errno(code: i32) -> bool {
    matches!(code, 1 | 31 | 38 | 45 | 78 | 95)
}

/// The name `path` reserved (`create_new`: fails when taken), then `tmp`
/// renamed over the empty reservation.
fn reserve_and_rename(tmp: &Path, path: &Path) -> std::io::Result<()> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    std::fs::rename(tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(path);
    })
}

/// Whether `name` is a temporary file [`save`] writes:
/// `.<snapshot id>.<pid>-<seq>.tmp`.
fn is_own_tmp(name: &str) -> bool {
    let Some(inner) = name.strip_prefix('.').and_then(|n| n.strip_suffix(".tmp")) else {
        return false;
    };
    let Some((id, tag)) = inner.split_once('.') else {
        return false;
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    parse_id(id).is_some_and(|(_, n)| n == 0)
        && tag
            .split_once('-')
            .is_some_and(|(pid, seq)| digits(pid) && digits(seq))
}

/// Removes the temporary files [`save`] left in `dir` (a crash) that are
/// older than [`STALE_TMP`]; nothing else. Best effort.
fn clean_stale(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > STALE_TMP);
        if is_own_tmp(&name) && old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// The snapshot `spec` names among `metas` (oldest first): `latest`,
/// `previous` (the one before it) among the readable ones, or a unique
/// prefix of an id (any file, so `rm` can remove an unreadable one).
pub fn find<'a>(metas: &'a [SnapshotMeta], spec: &str) -> Result<&'a SnapshotMeta, String> {
    let ok = usable(metas);
    let n = ok.len();
    let have = |need: usize| {
        format!(
            "{spec}: {n} readable snapshot{} of this profile, {need} needed",
            if n == 1 { "" } else { "s" }
        )
    };
    match spec {
        "latest" => ok.last().copied().ok_or_else(|| have(1)),
        "previous" => n.checked_sub(2).map(|i| ok[i]).ok_or_else(|| have(2)),
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

/// Deletes all but the newest `keep` readable snapshots of `metas`
/// (oldest first); an unreadable file (corrupt, newer format) is never
/// deleted. Returns the ones deleted.
pub fn prune(metas: &[SnapshotMeta], keep: usize) -> std::io::Result<Vec<SnapshotMeta>> {
    let ok = usable(metas);
    let cut = ok.len().saturating_sub(keep);
    let mut removed = Vec::new();
    for m in &ok[..cut] {
        remove(m)?;
        removed.push((*m).clone());
    }
    Ok(removed)
}

#[cfg(test)]
mod tests;
