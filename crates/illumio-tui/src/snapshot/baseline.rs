//! A snapshot as a baseline (#85): the workloads loaded now against a
//! snapshot's (the Workloads screen's compare mode, `workloads
//! --baseline-snapshot`), and the readiness sections of the profile's
//! snapshots as a trend per app (`:readiness`, `readiness --trend`).
//! Pure but for reading the files: never a request.
use super::diff::{check_same, workload_transitions};
use super::{Identity, Sections, Snapshot, SnapshotMeta, find, list, load, usable};
use pce::NameIndex;
use pce::models::Workload;
use pce::readiness::Verdict;
use pce::traffic::{Change, CompareCounts};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

/// A workload's change against the baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WlChange {
    pub change: Change,
    /// A changed workload's transitions (`app: shop→cart`, `health:
    /// healthy→offline`); empty on any other.
    pub was: Vec<String>,
}

impl WlChange {
    pub fn of(change: Change) -> Self {
        Self {
            change,
            was: Vec::new(),
        }
    }

    /// The `was→now` cell: the transitions, `·`-joined.
    pub fn was_text(&self) -> String {
        self.was.join(" · ")
    }
}

/// The workloads loaded now against a snapshot's.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorkloadCompare {
    /// Each loaded workload's change, by href: new, changed or same.
    pub live: HashMap<String, WlChange>,
    /// The snapshot's workloads not loaded now, in its order.
    pub gone: Vec<Workload>,
    /// Workloads per change, every one (before any filter).
    pub counts: CompareCounts,
}

impl WorkloadCompare {
    /// A loaded workload's change (`None`: not compared).
    pub fn of(&self, href: &str) -> Option<&WlChange> {
        self.live.get(href)
    }
}

/// `live` (read at `now`) against the snapshot's `base` (taken at
/// `base_at`), by href: a workload only loaded now is new, one only in the
/// snapshot gone (its values then), one in both changed when a field
/// `snapshot diff` compares differs (labels, enforcement mode, managed,
/// the VEN's status and version, name, hostname) or its VEN health does
/// (each side's at its own time: `health: healthy→offline`), else same.
pub fn compare_workloads(
    live: &[Workload],
    now: jiff::Timestamp,
    base: &[Workload],
    base_at: jiff::Timestamp,
) -> WorkloadCompare {
    // Both lists carry each label's key and value: no names needed.
    let idx = NameIndex::new(&[], &[], &[], &[]);
    let base_by: HashMap<&str, &Workload> = base.iter().map(|w| (w.href.as_str(), w)).collect();
    let mut out = WorkloadCompare::default();
    for w in live {
        let c = match base_by.get(w.href.as_str()) {
            None => WlChange::of(Change::New),
            Some(b) => {
                let mut was = workload_transitions(b, w, &idx);
                let (hb, hw) = (b.ven_health(base_at), w.ven_health(now));
                if hb != hw {
                    was.push(format!("health: {}→{}", hb.as_str(), hw.as_str()));
                }
                let change = if was.is_empty() {
                    Change::Same
                } else {
                    Change::Changed
                };
                WlChange { change, was }
            }
        };
        out.counts.add(c.change);
        out.live.insert(w.href.clone(), c);
    }
    let hrefs: HashSet<&str> = live.iter().map(|w| w.href.as_str()).collect();
    out.gone = base
        .iter()
        .filter(|b| !hrefs.contains(b.href.as_str()))
        .cloned()
        .collect();
    out.counts.gone = out.gone.len() as u64;
    out
}

/// Refuses a snapshot of another profile, PCE or org than `me` (as
/// `snapshot diff` refuses two), naming both.
pub fn check_baseline(s: &Snapshot, id: &str, me: &Identity) -> Result<(), String> {
    let live = Snapshot {
        version: s.version,
        profile: me.profile.clone(),
        pce: me.pce.clone(),
        org: me.org,
        taken_at: s.taken_at,
        label: None,
        truncated: Vec::new(),
        sections: Sections::default(),
    };
    check_same(s, id, &live, "now")
}

/// The snapshots of `metas` that hold workloads, oldest first.
pub fn with_workloads(metas: &[SnapshotMeta]) -> Vec<SnapshotMeta> {
    usable(metas)
        .into_iter()
        .filter(|m| m.sections.contains(&"workloads"))
        .cloned()
        .collect()
}

/// Why a baseline cannot be used: the user's (an unknown or ambiguous
/// name, a snapshot without workloads or of another PCE: exit 2) or the
/// file's (unreadable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaselineError {
    Usage(String),
    File(String),
}

impl std::fmt::Display for BaselineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BaselineError::Usage(s) | BaselineError::File(s) => f.write_str(s),
        }
    }
}

/// The snapshot `spec` names among the profile's in `dir` that hold
/// workloads (`latest`, `previous`, a time or a prefix of it), loaded and
/// checked to be of `me`'s PCE: the baseline and its meta.
pub fn load_baseline(
    dir: &Path,
    spec: &str,
    me: &Identity,
) -> Result<(SnapshotMeta, Snapshot), BaselineError> {
    let all = list(dir).map_err(|e| BaselineError::File(format!("{}: {e}", dir.display())))?;
    let with = with_workloads(&all);
    let m = match find(&with, spec) {
        Ok(m) => m.clone(),
        // Named, but without workloads (`--what policy`): say so.
        Err(e) => match find(&all, spec) {
            Ok(m) if spec != "latest" && spec != "previous" => {
                return Err(BaselineError::Usage(format!(
                    "snapshot {} has no workloads section: nothing to compare",
                    m.id
                )));
            }
            _ => return Err(BaselineError::Usage(e)),
        },
    };
    let s = load(&m.path).map_err(BaselineError::File)?;
    check_baseline(&s, &m.id, me).map_err(BaselineError::Usage)?;
    if s.sections.workloads.is_none() {
        return Err(BaselineError::Usage(format!(
            "snapshot {} has no workloads section: nothing to compare",
            m.id
        )));
    }
    Ok((m, s))
}

/// How many snapshots a trend reads at most: the newest that hold
/// readiness.
pub const TREND_MAX: usize = 14;

/// One app's readiness in one snapshot (or the live result).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrendPoint {
    pub taken_at: jiff::Timestamp,
    pub verdict: Verdict,
    /// Its gap tuples (`gap_count`).
    pub gaps: usize,
}

/// The readiness of every app over the snapshots read, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Trend {
    /// The snapshots read (each holds readiness).
    pub snapshots: usize,
    pub apps: BTreeMap<String, Vec<TrendPoint>>,
    /// The snapshots left out, and why (unreadable, another PCE).
    pub skipped: Vec<String>,
}

impl Trend {
    /// An app's points, when the trend has at least two snapshots (fewer
    /// show nothing).
    pub fn of(&self, app: &str) -> Option<&[TrendPoint]> {
        if self.snapshots < 2 {
            return None;
        }
        self.apps.get(app).map(Vec::as_slice)
    }
}

const VERDICTS: [Verdict; 5] = [
    Verdict::NotEvaluated,
    Verdict::NeedsRules,
    Verdict::NeedsProvision,
    Verdict::Ready,
    Verdict::Enforced,
];

/// The points `s`'s readiness section holds, per app: `apps[]`'s `app`,
/// `verdict` and `gap_count` (as `readiness` prints them); an entry
/// without them is skipped.
fn points_of(s: &Snapshot) -> Vec<(String, TrendPoint)> {
    let apps = s
        .sections
        .readiness
        .as_ref()
        .and_then(|r| r.get("apps"))
        .and_then(serde_json::Value::as_array);
    apps.into_iter()
        .flatten()
        .filter_map(|a| {
            let app = a.get("app")?.as_str()?.to_string();
            let word = a.get("verdict")?.as_str()?;
            let verdict = VERDICTS.into_iter().find(|v| v.as_str() == word)?;
            let gaps = a.get("gap_count")?.as_u64()? as usize;
            Some((
                app,
                TrendPoint {
                    taken_at: s.taken_at,
                    verdict,
                    gaps,
                },
            ))
        })
        .collect()
}

/// The trend of `snaps` (oldest first; those without readiness skipped).
pub fn trend_of(snaps: &[Snapshot]) -> Trend {
    let mut t = Trend::default();
    for s in snaps.iter().filter(|s| s.sections.readiness.is_some()) {
        t.snapshots += 1;
        for (app, p) in points_of(s) {
            t.apps.entry(app).or_default().push(p);
        }
    }
    t
}

/// The trend of the newest [`TREND_MAX`] snapshots in `dir` that hold
/// readiness; one of another profile, PCE or org than `me`, or one that
/// cannot be read, is skipped and named.
pub fn load_trend(dir: &Path, me: &Identity) -> Result<Trend, String> {
    let all = list(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let with: Vec<&SnapshotMeta> = usable(&all)
        .into_iter()
        .filter(|m| m.sections.contains(&"readiness"))
        .collect();
    let mut snaps = Vec::new();
    let mut skipped = Vec::new();
    for m in &with[with.len().saturating_sub(TREND_MAX)..] {
        match load(&m.path).and_then(|s| check_baseline(&s, &m.id, me).map(|()| s)) {
            Ok(s) => snaps.push(s),
            Err(e) => skipped.push(e),
        }
    }
    Ok(Trend {
        skipped,
        ..trend_of(&snaps)
    })
}

/// The blocks a sparkline is drawn with, lowest first.
pub const SPARK: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// A block per value, scaled to the largest (all `▁` when every value is
/// 0).
pub fn sparkline(values: &[usize]) -> Vec<char> {
    let max = values.iter().copied().max().unwrap_or(0);
    values
        .iter()
        .map(|&v| {
            if max == 0 {
                SPARK[0]
            } else {
                SPARK[(v * (SPARK.len() - 1) + max / 2) / max]
            }
        })
        .collect()
}

/// `▁▃█` for an app's points: their gaps.
pub fn spark_text(points: &[TrendPoint]) -> String {
    sparkline(&points.iter().map(|p| p.gaps).collect::<Vec<_>>())
        .into_iter()
        .collect()
}
