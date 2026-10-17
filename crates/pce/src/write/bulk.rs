//! One change to many workloads: their labels, or their enforcement mode
//! (and visibility level), sent as `PUT /orgs/{org}/workloads/bulk_update`
//! in chunks of [`BULK_CHUNK`].
//!
//! [`plan_bulk`] plans from the workloads the caller already holds and
//! sends nothing. [`Writer::execute_bulk`] then, in this order: refuses a
//! plan made on another profile, or one that enforces several workloads
//! without the typed count; re-reads the workload list once with the cache
//! bypassed and drops every target whose labels, enforcement, visibility or
//! VEN changed since the plan (reported, never overwritten); creates the
//! labels the change names that do not exist yet (one POST each, their
//! hrefs put where the placeholders `{step:N}` stand); sends the chunks one
//! at a time and stops after a chunk with any failure. Every request goes
//! through [`Writer::execute`]: arming, throttling and one audit entry per
//! request (a chunk's entry lists each workload's outcome).
use super::batch::{BatchOrigin, parse_placeholder, step_placeholder};
use super::labels::LabelDraft;
use super::workloads::LabelRemoval;
use super::{ObjectKind, Outcome, Plan, WriteError, WriteRequest, Writer, checked_href};
use crate::client::WriteMethod;
use crate::models::{ActorMatch, PolicyDecision, Ref, TrafficQuery, Workload, WorkloadFilter};
use crate::resolve::NameIndex;
use crate::traffic::{ResolvedFlow, TrafficQueryBuilder};
use crate::{CacheMode, PceError};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

/// Workloads per `bulk_update` request.
pub const BULK_CHUNK: usize = 100;

/// The draft check names each target as a destination; above this many
/// the query would be unwieldy, so it is refused (narrow the selection).
pub const DRAFT_CHECK_MAX_TARGETS: usize = 200;

/// The PCE's enforcement modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnforcementMode {
    Idle,
    VisibilityOnly,
    Selective,
    Full,
}

impl EnforcementMode {
    pub const ALL: [Self; 4] = [
        Self::Idle,
        Self::VisibilityOnly,
        Self::Selective,
        Self::Full,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::VisibilityOnly => "visibility_only",
            Self::Selective => "selective",
            Self::Full => "full",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.as_str() == s.trim())
    }

    /// Selective and full block traffic the policy does not allow: on more
    /// than one workload the person types the count first.
    pub fn enforces(self) -> bool {
        matches!(self, Self::Selective | Self::Full)
    }
}

/// The PCE's visibility levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityLevel {
    FlowSummary,
    FlowDrops,
    FlowOff,
    EnhancedDataCollection,
}

impl VisibilityLevel {
    pub const ALL: [Self; 4] = [
        Self::FlowSummary,
        Self::FlowDrops,
        Self::FlowOff,
        Self::EnhancedDataCollection,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::FlowSummary => "flow_summary",
            Self::FlowDrops => "flow_drops",
            Self::FlowOff => "flow_off",
            Self::EnhancedDataCollection => "enhanced_data_collection",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.as_str() == s.trim())
    }
}

/// A label of a workload, with its key and value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BulkLabel {
    pub href: String,
    /// Empty when neither the workload nor the name index knows it: such a
    /// label is kept as it is, whatever the change.
    pub key: String,
    pub value: String,
}

impl BulkLabel {
    pub fn text(&self) -> String {
        format!("{}={}", self.key, self.value)
    }
}

/// A workload as the plan saw it: what a change is computed from, and
/// what the re-read must still find before anything is sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BulkTarget {
    pub href: String,
    pub name: String,
    /// A VEN manages it, so an enforcement mode applies. Fails closed:
    /// `managed: true`, or (without the flag) a VEN or an agent status.
    pub managed: bool,
    pub enforcement: Option<String>,
    pub visibility: Option<String>,
    pub labels: Vec<BulkLabel>,
}

impl BulkTarget {
    /// `w` with its labels' keys from the workload, else from `idx`.
    pub fn of(w: &Workload, idx: &NameIndex) -> Self {
        let labels = w
            .labels
            .iter()
            .map(|l| {
                let known = idx.label(&l.href);
                BulkLabel {
                    href: l.href.clone(),
                    key: l
                        .key
                        .clone()
                        .or_else(|| known.map(|x| x.key.clone()))
                        .unwrap_or_default(),
                    value: l
                        .value
                        .clone()
                        .or_else(|| known.map(|x| x.value.clone()))
                        .unwrap_or_default(),
                }
            })
            .collect();
        Self {
            href: w.href.clone(),
            name: w.display_name(),
            managed: managed(w),
            enforcement: w.enforcement_mode.clone(),
            visibility: w.visibility_level.clone(),
            labels,
        }
    }

    /// The same labels (as a set of hrefs), mode, level and VEN state.
    fn same_state(&self, now: &BulkTarget) -> bool {
        fn hrefs(t: &BulkTarget) -> Vec<&str> {
            let mut v: Vec<&str> = t.labels.iter().map(|l| l.href.as_str()).collect();
            v.sort_unstable();
            v
        }
        self.managed == now.managed
            && self.enforcement == now.enforcement
            && self.visibility == now.visibility
            && hrefs(self) == hrefs(now)
    }

    fn label_of(&self, key: &str) -> Vec<&BulkLabel> {
        self.labels.iter().filter(|l| l.key == key).collect()
    }
}

fn managed(w: &Workload) -> bool {
    match w.managed {
        Some(m) => m,
        None => w.ven.is_some() || w.agent_status_view().is_some(),
    }
}

/// What a label key is set to: an existing label, or one created first
/// (a step of the send).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelSet {
    Existing(BulkLabel),
    Create(LabelDraft),
}

impl LabelSet {
    pub fn key(&self) -> &str {
        match self {
            LabelSet::Existing(l) => &l.key,
            LabelSet::Create(d) => &d.key,
        }
    }

    pub fn value(&self) -> &str {
        match self {
            LabelSet::Existing(l) => &l.value,
            LabelSet::Create(d) => &d.value,
        }
    }
}

/// Per key: set it (replacing whatever label of that key the workload
/// has), or remove it (any value, or only the one named). Keys not named
/// are kept.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LabelEdit {
    pub set: Vec<LabelSet>,
    pub remove: Vec<LabelRemoval>,
}

impl LabelRemoval {
    fn matches(&self, l: &BulkLabel) -> bool {
        !l.key.is_empty() && l.key == self.key && self.value.as_ref().is_none_or(|v| *v == l.value)
    }

    fn text(&self) -> String {
        match &self.value {
            Some(v) => format!("-{}={v}", self.key),
            None => format!("-{}", self.key),
        }
    }
}

impl LabelEdit {
    pub fn is_empty(&self) -> bool {
        self.set.is_empty() && self.remove.is_empty()
    }

    /// `app=shop (new) · -env`.
    pub fn text(&self) -> String {
        let mut parts: Vec<String> = self
            .set
            .iter()
            .map(|s| match s {
                LabelSet::Existing(l) => l.text(),
                LabelSet::Create(d) => format!("{} (new)", d.display()),
            })
            .collect();
        parts.extend(self.remove.iter().map(LabelRemoval::text));
        parts.join(" · ")
    }
}

/// The change a bulk write makes: an enforcement mode (with or without a
/// visibility level; without, each workload keeps its own), or labels.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BulkChange {
    pub enforcement: Option<EnforcementMode>,
    pub visibility: Option<VisibilityLevel>,
    pub labels: LabelEdit,
}

impl BulkChange {
    pub fn enforce(mode: EnforcementMode, visibility: Option<VisibilityLevel>) -> Self {
        Self {
            enforcement: Some(mode),
            visibility,
            labels: LabelEdit::default(),
        }
    }

    pub fn relabel(labels: LabelEdit) -> Self {
        Self {
            labels,
            ..Self::default()
        }
    }

    pub fn changes_labels(&self) -> bool {
        !self.labels.is_empty()
    }

    /// Full or selective: the typed count guards it on several workloads.
    pub fn enforces(&self) -> bool {
        self.enforcement.is_some_and(EnforcementMode::enforces)
    }

    /// `enforcement full (visibility kept)` or `labels app=shop · -env`.
    pub fn text(&self) -> String {
        match self.enforcement {
            Some(m) => format!(
                "enforcement {} (visibility {})",
                m.as_str(),
                self.visibility.map_or("kept", VisibilityLevel::as_str)
            ),
            None => format!("labels {}", self.labels.text()),
        }
    }

    pub fn validate(&self) -> Result<(), WriteError> {
        let mut errs = Vec::new();
        if self.enforcement.is_none() && self.visibility.is_some() {
            errs.push("a visibility level goes with an enforcement mode".to_string());
        }
        if self.enforcement.is_some() && self.changes_labels() {
            errs.push("change the labels or the enforcement, not both at once".into());
        }
        if self.enforcement.is_none() && !self.changes_labels() {
            errs.push("nothing to change".into());
        }
        let mut keys: Vec<&str> = Vec::new();
        for s in &self.labels.set {
            let k = s.key();
            if k.trim().is_empty() || s.value().trim().is_empty() {
                errs.push("a label to set needs a key and a value".into());
            } else if keys.contains(&k) {
                errs.push(format!("{k} is set twice: one label per key"));
            }
            keys.push(k);
        }
        for r in &self.labels.remove {
            let k = r.key.as_str();
            if k.trim().is_empty() {
                errs.push("a label to remove needs a key".into());
            } else if keys.contains(&k) {
                errs.push(format!("{k} is both set and removed"));
            }
        }
        if errs.is_empty() {
            Ok(())
        } else {
            Err(WriteError::Invalid(errs))
        }
    }
}

/// One workload the plan changes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BulkItem {
    pub href: String,
    pub name: String,
    /// `visibility_only → full`, `app=crm → app=shop · -env=prod`.
    pub change: String,
    /// What the chunk carries for it; a label to be created stands as its
    /// step's placeholder.
    pub body: Value,
    /// The workload as planned; the re-read must still find it so.
    #[serde(skip)]
    pub base: BulkTarget,
}

/// A workload the plan leaves alone, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BulkSkip {
    pub href: String,
    pub name: String,
    pub reason: String,
}

/// Everything a bulk write will do. Planned locally; sent by
/// [`Writer::execute_bulk`].
#[derive(Debug, Clone)]
pub struct BulkPlan {
    pub change: BulkChange,
    pub items: Vec<BulkItem>,
    pub skipped: Vec<BulkSkip>,
    /// Labels created before the chunks, step `i` standing as `{step:i}`.
    pub creates: Vec<LabelDraft>,
    /// The profile and PCE it was planned on; it is sent only there.
    pub origin: Option<BatchOrigin>,
    /// The list read that finds the targets again before sending.
    pub reread: WorkloadFilter,
    /// The target count the person typed. Required when the plan enforces
    /// (full or selective) more than one workload; `execute_bulk` refuses
    /// without it.
    pub confirmed: Option<usize>,
}

/// What a change does to one workload: the body it sends and the change
/// text, or why it is skipped. `created[i]` stands for create step `i`
/// (a placeholder at plan time, the created href at send time).
fn plan_one(
    t: &BulkTarget,
    change: &BulkChange,
    created: &[String],
) -> Result<(Value, String), String> {
    if let Some(mode) = change.enforcement {
        if !t.managed {
            return Err("unmanaged".into());
        }
        let mode_same = t.enforcement.as_deref() == Some(mode.as_str());
        let vis_same = change
            .visibility
            .is_none_or(|v| t.visibility.as_deref() == Some(v.as_str()));
        if mode_same && vis_same {
            return Err(format!("already {}", mode.as_str()));
        }
        let mut body = Map::new();
        body.insert("href".into(), json!(t.href));
        body.insert("enforcement_mode".into(), json!(mode.as_str()));
        let mut text = format!(
            "{} → {}",
            t.enforcement.as_deref().unwrap_or("?"),
            mode.as_str()
        );
        if let Some(v) = change.visibility {
            body.insert("visibility_level".into(), json!(v.as_str()));
            if t.visibility.as_deref() != Some(v.as_str()) {
                text.push_str(&format!(
                    " · {} → {}",
                    t.visibility.as_deref().unwrap_or("?"),
                    v.as_str()
                ));
            }
        }
        return Ok((Value::Object(body), text));
    }
    let edit = &change.labels;
    let replaced = |l: &BulkLabel| !l.key.is_empty() && edit.set.iter().any(|s| s.key() == l.key);
    let removed = |l: &BulkLabel| edit.remove.iter().any(|r| r.matches(l));
    // Kept labels in their order, then the set ones; a label with no
    // known key is always kept.
    let mut labels: Vec<String> = t
        .labels
        .iter()
        .filter(|l| !replaced(l) && !removed(l))
        .map(|l| l.href.clone())
        .collect();
    let mut parts = Vec::new();
    let mut create = 0;
    for s in &edit.set {
        let href = match s {
            LabelSet::Existing(l) => l.href.clone(),
            LabelSet::Create(_) => {
                let h = created.get(create).cloned().unwrap_or_default();
                create += 1;
                h
            }
        };
        let old = t.label_of(s.key());
        let now = format!("{}={}", s.key(), s.value());
        match old.as_slice() {
            [] => parts.push(format!("+{now}")),
            [one] if one.href == href => {}
            _ => parts.push(format!(
                "{} → {now}",
                old.iter().map(|l| l.text()).collect::<Vec<_>>().join(",")
            )),
        }
        labels.push(href);
    }
    parts.extend(
        t.labels
            .iter()
            .filter(|l| removed(l))
            .map(|l| format!("-{}", l.text())),
    );
    if parts.is_empty() {
        return Err("labels already so".into());
    }
    let body = json!({
        "href": t.href,
        "labels": labels.iter().map(|h| json!({"href": h})).collect::<Vec<_>>(),
    });
    Ok((body, parts.join(" · ")))
}

/// Plans `change` over `targets` (no request): each workload's body or
/// why it is skipped (unmanaged for an enforcement mode, nothing to
/// change), the labels to create first, `reread` as the list read that
/// finds them again before sending.
pub fn plan_bulk(
    targets: &[BulkTarget],
    change: &BulkChange,
    reread: WorkloadFilter,
    origin: Option<BatchOrigin>,
) -> Result<BulkPlan, WriteError> {
    change.validate()?;
    if targets.is_empty() {
        return Err(WriteError::invalid("no workloads to change"));
    }
    let creates: Vec<LabelDraft> = change
        .labels
        .set
        .iter()
        .filter_map(|s| match s {
            LabelSet::Create(d) => Some(d.clone()),
            LabelSet::Existing(_) => None,
        })
        .collect();
    for d in &creates {
        d.validate().map_err(WriteError::Invalid)?;
    }
    let placeholders: Vec<String> = (0..creates.len()).map(step_placeholder).collect();
    let mut items = Vec::new();
    let mut skipped = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for t in targets {
        if !seen.insert(t.href.as_str()) {
            continue;
        }
        match plan_one(t, change, &placeholders) {
            Ok((body, text)) => items.push(BulkItem {
                href: t.href.clone(),
                name: t.name.clone(),
                change: text,
                body,
                base: t.clone(),
            }),
            Err(reason) => skipped.push(BulkSkip {
                href: t.href.clone(),
                name: t.name.clone(),
                reason,
            }),
        }
    }
    Ok(BulkPlan {
        change: change.clone(),
        items,
        skipped,
        creates,
        origin,
        reread,
        confirmed: None,
    })
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// `n` occurrences of each string, most frequent first (then by text).
fn counted<'a>(texts: impl Iterator<Item = &'a str>) -> Vec<(usize, &'a str)> {
    let mut m: BTreeMap<&str, usize> = BTreeMap::new();
    for t in texts {
        *m.entry(t).or_default() += 1;
    }
    let mut v: Vec<(usize, &str)> = m.into_iter().map(|(t, n)| (n, t)).collect();
    v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
    v
}

impl BulkPlan {
    /// Full or selective on more than one workload: the count must be
    /// typed before it is sent.
    pub fn needs_count(&self) -> bool {
        self.change.enforces() && self.items.len() > 1
    }

    /// The hrefs of the workloads it changes.
    pub fn hrefs(&self) -> Vec<String> {
        self.items.iter().map(|i| i.href.clone()).collect()
    }

    /// `14 workloads: 9 visibility_only → full · 3 already full (skipped)
    /// · 2 unmanaged (skipped)`; a relabel counts `12 to relabel (app=shop)`.
    pub fn summary(&self) -> String {
        let total = self.items.len() + self.skipped.len();
        let mut parts: Vec<String> = Vec::new();
        if self.change.enforcement.is_some() {
            // The mode transition only (`visibility_only → full`).
            let modes = self
                .items
                .iter()
                .map(|i| i.change.split(" · ").next().unwrap_or(""));
            parts.extend(counted(modes).into_iter().map(|(n, t)| format!("{n} {t}")));
        } else if !self.items.is_empty() {
            parts.push(format!(
                "{} to relabel ({})",
                self.items.len(),
                self.change.labels.text()
            ));
        }
        parts.extend(
            counted(self.skipped.iter().map(|s| s.reason.as_str()))
                .into_iter()
                .map(|(n, t)| format!("{n} {t} (skipped)")),
        );
        if parts.is_empty() {
            parts.push("nothing to change".into());
        }
        format!(
            "{}: {}",
            plural(total, "workload", "workloads"),
            parts.join(" · ")
        )
    }

    /// The requests it sends after the re-read, in order: one POST per
    /// label to create, one `bulk_update` PUT per [`BULK_CHUNK`] workloads
    /// (placeholders where created labels go).
    pub fn requests(&self, org_id: u64) -> Vec<WriteRequest> {
        let mut out: Vec<WriteRequest> = self
            .creates
            .iter()
            .map(|d| WriteRequest {
                method: WriteMethod::Post,
                path: format!("/orgs/{org_id}/labels"),
                body: Some(d.to_body()),
                summary: format!("create label {}", d.display()),
                objects: Vec::new(),
            })
            .collect();
        let chunks = self.items.len().div_ceil(BULK_CHUNK);
        for (i, chunk) in self.items.chunks(BULK_CHUNK).enumerate() {
            out.push(chunk_request(org_id, chunk, i, chunks));
        }
        out
    }

    /// Reads before the first write (the re-read): always one.
    pub fn reads(&self) -> u32 {
        u32::from(!self.items.is_empty())
    }

    /// `1 read + 3 requests: 1 label, 2 bulk_update`.
    pub fn cost_text(&self) -> String {
        let chunks = self.items.len().div_ceil(BULK_CHUNK);
        let mut parts = Vec::new();
        if !self.creates.is_empty() {
            parts.push(plural(self.creates.len(), "label", "labels"));
        }
        parts.push(format!("{chunks} bulk_update"));
        format!(
            "1 re-read + {}: {}",
            plural(self.creates.len() + chunks, "write", "writes"),
            parts.join(", ")
        )
    }

    /// The confirm's text: each workload and its change, the skipped ones,
    /// then each request (the first item of a chunk as an example body).
    pub fn lines(&self, org_id: u64) -> Vec<String> {
        let mut out = Vec::new();
        let w = self
            .items
            .iter()
            .map(|i| i.name.chars().count())
            .chain(self.skipped.iter().map(|s| s.name.chars().count()))
            .max()
            .unwrap_or(0)
            .min(32);
        for i in &self.items {
            out.push(format!("{:<w$}  {}", i.name, i.change));
        }
        if !self.skipped.is_empty() {
            out.push(String::new());
            out.push(format!("skipped ({}):", self.skipped.len()));
            for s in &self.skipped {
                out.push(format!("{:<w$}  {}", s.name, s.reason));
            }
        }
        out.push(String::new());
        out.push(
            "before sending: 1 list read (cache bypassed); workloads changed since are skipped"
                .to_string(),
        );
        let reqs = self.requests(org_id);
        let n = reqs.len();
        for (i, r) in reqs.iter().enumerate() {
            out.push(format!("{}/{n} {} {}", i + 1, r.method.as_str(), r.path));
            out.push(format!("  {}", r.summary));
            let example = match &r.body {
                Some(Value::Array(items)) => items.first().cloned(),
                other => other.clone(),
            };
            if let Some(b) = example {
                let mut shown = serde_json::to_string(&b).unwrap_or_default();
                for (k, d) in self.creates.iter().enumerate() {
                    shown = shown.replace(
                        &step_placeholder(k),
                        &format!("<href of step {}: label {}>", k + 1, d.display()),
                    );
                }
                let lead = if matches!(&r.body, Some(Value::Array(_))) {
                    "  e.g. "
                } else {
                    "  "
                };
                out.push(format!("{lead}{shown}"));
            }
        }
        out
    }
}

fn chunk_request(org_id: u64, chunk: &[BulkItem], i: usize, of: usize) -> WriteRequest {
    WriteRequest {
        method: WriteMethod::Put,
        path: format!("/orgs/{org_id}/workloads/bulk_update"),
        body: Some(Value::Array(
            chunk.iter().map(|it| it.body.clone()).collect(),
        )),
        summary: format!(
            "bulk update {} ({}/{of})",
            plural(chunk.len(), "workload", "workloads"),
            i + 1
        ),
        objects: chunk
            .iter()
            .map(|it| format!("{}: {}", it.name, it.change))
            .collect(),
    }
}

/// What became of one workload of a bulk write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    Updated,
    /// The PCE refused it (its own status), or the chunk failed.
    Failed,
    /// Its chunk was sent but not answered: it may have applied.
    Unanswered,
    /// An earlier request failed, or the send was refused.
    NotSent,
    /// The re-read found it changed since the plan: skipped.
    Changed,
    /// The re-read did not find it: skipped.
    Missing,
    /// The plan skipped it (unmanaged, nothing to change).
    Skipped,
}

impl ItemStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Updated => "updated",
            Self::Failed => "failed",
            Self::Unanswered => "not answered",
            Self::NotSent => "not sent",
            Self::Changed => "changed since planned, skipped",
            Self::Missing => "not found by the re-read, skipped",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ItemResult {
    pub href: String,
    pub name: String,
    pub change: String,
    pub status: ItemStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What became of one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Written,
    /// Answered, some workloads refused.
    Partial,
    Failed,
    Unanswered,
    NotSent,
}

#[derive(Debug, Clone, Serialize)]
pub struct BulkStep {
    pub method: WriteMethod,
    pub path: String,
    pub summary: String,
    pub status: StepStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Written, but its audit entry could not be written (why); the send
    /// stopped after it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit_failed: Option<String>,
}

/// A sent (or refused) bulk write, request by request and workload by
/// workload.
#[derive(Debug, Clone, Serialize)]
pub struct BulkReport {
    /// `lab (pce.example, org 5)`.
    pub target: String,
    pub change: String,
    /// Nothing was sent (and why): a refused plan or a failed re-read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refused: Option<String>,
    /// List reads made (the re-read: 0 or 1).
    pub reads: u32,
    pub steps: Vec<BulkStep>,
    pub items: Vec<ItemResult>,
    /// A label write may have applied: labels and the label-derived
    /// caches are stale.
    pub labels_changed: bool,
    /// A label was created.
    pub labels_created: bool,
}

impl BulkReport {
    fn count(&self, s: ItemStatus) -> usize {
        self.items.iter().filter(|i| i.status == s).count()
    }

    pub fn updated(&self) -> usize {
        self.count(ItemStatus::Updated)
    }

    /// Some request may have applied (written or unanswered).
    pub fn touched(&self) -> bool {
        self.steps.iter().any(|s| {
            matches!(
                s.status,
                StepStatus::Written | StepStatus::Partial | StepStatus::Unanswered
            )
        })
    }

    /// It did not do everything it planned: a refusal, a failure, a
    /// workload not sent, or one written without an audit entry.
    pub fn stopped(&self) -> bool {
        self.refused.is_some()
            || self
                .steps
                .iter()
                .any(|s| !matches!(s.status, StepStatus::Written) || s.audit_failed.is_some())
            || self.items.iter().any(|i| {
                matches!(
                    i.status,
                    ItemStatus::Failed | ItemStatus::Unanswered | ItemStatus::NotSent
                )
            })
    }

    /// Workloads skipped by the re-read (changed or gone).
    pub fn dropped(&self) -> usize {
        self.count(ItemStatus::Changed) + self.count(ItemStatus::Missing)
    }

    /// `updated 12 of 14 · 1 failed · 1 changed since planned, skipped`.
    pub fn status_text(&self) -> String {
        if let Some(r) = &self.refused {
            return format!("nothing sent: {r}");
        }
        let planned = self
            .items
            .iter()
            .filter(|i| i.status != ItemStatus::Skipped)
            .count();
        let mut s = format!("updated {} of {planned}", self.updated());
        for st in [
            ItemStatus::Failed,
            ItemStatus::Unanswered,
            ItemStatus::NotSent,
            ItemStatus::Changed,
            ItemStatus::Missing,
        ] {
            let n = self.count(st);
            if n > 0 {
                s.push_str(&format!(" · {n} {}", st.as_str()));
            }
        }
        if let Some(st) = self.steps.iter().find(|s| s.audit_failed.is_some()) {
            s.push_str(&format!(
                " · {} written but NOT AUDITED: {}",
                st.summary,
                st.audit_failed.as_deref().unwrap_or("")
            ));
        }
        if self.count(ItemStatus::Unanswered) > 0 {
            s.push_str(" · it may have applied: check the PCE");
        }
        s
    }

    /// The report's lines (the TUI's `:batch`, the CLI table): where it
    /// went, the re-read, each request, then each workload.
    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![format!("sent to {}", self.target), self.change.clone()];
        if let Some(r) = &self.refused {
            out.push(format!("nothing sent: {r}"));
        }
        if self.reads > 0 {
            let dropped = self.dropped();
            out.push(if dropped > 0 {
                format!(
                    "re-read: 1 list read · {} skipped (changed since planned or gone)",
                    dropped
                )
            } else {
                "re-read: 1 list read · every workload as planned".to_string()
            });
        }
        let n = self.steps.len();
        for (i, s) in self.steps.iter().enumerate() {
            out.push(format!("{}/{n} {}", i + 1, s.summary));
            out.push(match (s.status, &s.error, &s.audit_failed) {
                (_, _, Some(why)) => format!("    written, NOT AUDITED: {why}"),
                (StepStatus::Written, _, _) => match &s.href {
                    Some(h) if s.method == WriteMethod::Post => format!("    written {h}"),
                    _ => "    written".to_string(),
                },
                (StepStatus::Partial, _, _) => "    answered: some workloads refused".to_string(),
                (StepStatus::Unanswered, e, _) => format!(
                    "    NOT ANSWERED, it may have applied: {}",
                    e.as_deref().unwrap_or("")
                ),
                (StepStatus::Failed, e, _) => format!("    FAILED: {}", e.as_deref().unwrap_or("")),
                (StepStatus::NotSent, _, _) => "    not sent".to_string(),
            });
        }
        if !self.items.is_empty() {
            out.push(String::new());
            out.push("workloads:".into());
            let w = self
                .items
                .iter()
                .map(|i| i.name.chars().count())
                .max()
                .unwrap_or(0)
                .min(32);
            for i in &self.items {
                let what = match (&i.error, i.status) {
                    (Some(e), ItemStatus::Failed) => format!("FAILED: {e}"),
                    (Some(e), ItemStatus::Skipped) => format!("skipped: {e}"),
                    (_, st) => st.as_str().to_string(),
                };
                let line = if i.change.is_empty() {
                    format!("  {:<w$}  {what}", i.name)
                } else {
                    format!("  {:<w$}  {}  {what}", i.name, i.change)
                };
                out.push(line);
            }
        }
        if self.stopped() && self.refused.is_none() {
            out.push(String::new());
            out.push("Plan it again to resume: workloads already changed are skipped.".into());
        }
        out
    }
}

/// Each workload of a sent chunk and its outcome from the PCE's answer:
/// a list of `{href, status, errors}`; `updated` (or `success`) is the
/// only success, an entry missing for a workload is a failure.
pub(crate) fn chunk_outcomes(
    sent: Option<&Value>,
    answer: &Value,
) -> Vec<(String, Result<(), String>)> {
    let hrefs: Vec<String> = match sent {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|i| i.get("href").and_then(Value::as_str).map(str::to_string))
            .collect(),
        _ => Vec::new(),
    };
    let Value::Array(entries) = answer else {
        return hrefs
            .into_iter()
            .map(|h| (h, Err("the answer was not a per-workload list".to_string())))
            .collect();
    };
    let mut by_href: HashMap<&str, &Value> = HashMap::new();
    for e in entries {
        if let Some(h) = e.get("href").and_then(Value::as_str) {
            by_href.insert(h, e);
        }
    }
    hrefs
        .into_iter()
        .map(|h| {
            let r = match by_href.get(h.as_str()) {
                None => Err("no result for this workload in the answer".to_string()),
                Some(e) => {
                    let status = e.get("status").and_then(Value::as_str).unwrap_or("");
                    if matches!(status, "updated" | "success") {
                        Ok(())
                    } else {
                        Err(entry_error(status, e))
                    }
                }
            };
            (h, r)
        })
        .collect()
}

/// `validation_failure: enforcement_mode is invalid`.
fn entry_error(status: &str, e: &Value) -> String {
    let mut msgs: Vec<String> = Vec::new();
    if let Some(Value::Array(errs)) = e.get("errors") {
        for x in errs {
            let m = x
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| x.get("token").and_then(Value::as_str))
                .map(str::to_string)
                .or_else(|| x.as_str().map(str::to_string))
                .unwrap_or_else(|| x.to_string());
            msgs.push(m);
        }
    }
    for k in ["message", "token"] {
        if msgs.is_empty()
            && let Some(m) = e.get(k).and_then(Value::as_str)
        {
            msgs.push(m.to_string());
        }
    }
    let status = if status.is_empty() {
        "no status"
    } else {
        status
    };
    if msgs.is_empty() {
        status.to_string()
    } else {
        format!("{status}: {}", msgs.join("; "))
    }
}

/// The audit entry's `items` for a chunk: each workload and its outcome.
pub(crate) fn audit_items(sent: Option<&Value>, answer: &Value) -> Value {
    Value::Array(
        chunk_outcomes(sent, answer)
            .into_iter()
            .map(|(href, r)| match r {
                Ok(()) => json!({"href": href, "status": "updated"}),
                Err(e) => json!({"href": href, "status": "failed", "error": e}),
            })
            .collect(),
    )
}

/// `body` with every `{step:i}` href replaced by `created[i]`.
fn fill_labels(body: &mut Value, created: &[String]) -> Result<(), WriteError> {
    if let Some(Value::Array(labels)) = body.get_mut("labels") {
        for l in labels {
            if let Some(Value::String(h)) = l.get_mut("href")
                && let Some(n) = parse_placeholder(h)
            {
                *h = created.get(n).cloned().ok_or_else(|| {
                    WriteError::invalid(format!(
                        "label step {} has no href: nothing more sent",
                        n + 1
                    ))
                })?;
            }
        }
    }
    if body.to_string().contains("{step:") {
        return Err(WriteError::invalid(
            "a placeholder is left in a chunk: nothing more sent",
        ));
    }
    Ok(())
}

fn error_status(e: &WriteError) -> (StepStatus, Option<u16>) {
    fn status_of(e: &PceError) -> Option<u16> {
        match e {
            PceError::Server { status, .. }
            | PceError::Rejected { status, .. }
            | PceError::Auth { status } => Some(*status),
            PceError::NotFound { .. } => Some(404),
            PceError::RateLimited { .. } => Some(429),
            _ => None,
        }
    }
    match e {
        WriteError::Unanswered(p) => (StepStatus::Unanswered, status_of(p)),
        WriteError::Pce(p) => (StepStatus::Failed, status_of(p)),
        WriteError::AuditFailed { write, .. } => error_status(write),
        _ => (StepStatus::Failed, None),
    }
}

/// The draft check: the targets' recent traffic as destinations, every
/// decision, evaluated against the draft policy (what full enforcement
/// would block). One Explorer query; refused above
/// [`DRAFT_CHECK_MAX_TARGETS`] targets.
pub fn draft_check_query(
    hrefs: &[String],
    window: Duration,
    max_results: u32,
) -> Result<TrafficQuery, PceError> {
    if hrefs.is_empty() {
        return Err(PceError::Invalid("no workloads to check".into()));
    }
    if hrefs.len() > DRAFT_CHECK_MAX_TARGETS {
        return Err(PceError::Invalid(format!(
            "the draft check covers at most {DRAFT_CHECK_MAX_TARGETS} workloads ({} here): narrow the selection",
            hrefs.len()
        )));
    }
    let mut b = TrafficQueryBuilder::last(window)?
        .max_results(max_results)
        .query_name("illumio-tui draft check");
    for h in hrefs {
        b = b.destination(vec![ActorMatch::Workload {
            workload: Ref::new(h.clone()),
        }]);
    }
    b.build()
}

/// A flow the draft policy does not allow: blocked once its destination
/// enforces.
pub fn would_block(f: &ResolvedFlow) -> bool {
    f.draft_policy_decision.as_ref().is_some_and(|d| {
        matches!(
            d.class(),
            PolicyDecision::Blocked | PolicyDecision::PotentiallyBlocked
        )
    })
}

impl Writer<'_> {
    /// Sends `plan` (see the module doc), reporting `progress(done, of)`
    /// after each request. Never retries; stops after the first chunk with
    /// a failure. Every outcome is in the report, nothing is an `Err`.
    pub async fn execute_bulk(
        &self,
        plan: BulkPlan,
        progress: &mut (dyn FnMut(usize, usize) + Send),
    ) -> BulkReport {
        let here = BatchOrigin::of(self.guard().context());
        let mut report = BulkReport {
            target: here.text(),
            change: plan.change.text(),
            refused: None,
            reads: 0,
            steps: Vec::new(),
            items: plan
                .skipped
                .iter()
                .map(|s| ItemResult {
                    href: s.href.clone(),
                    name: s.name.clone(),
                    change: String::new(),
                    status: ItemStatus::Skipped,
                    error: Some(s.reason.clone()),
                })
                .collect(),
            labels_changed: false,
            labels_created: false,
        };
        let org = self.pce().client().org_id();
        let not_sent = |report: &mut BulkReport, items: &[BulkItem], why: String| {
            report.refused = Some(why);
            report.items.extend(items.iter().map(|i| ItemResult {
                href: i.href.clone(),
                name: i.name.clone(),
                change: i.change.clone(),
                status: ItemStatus::NotSent,
                error: None,
            }));
        };
        // Refusals: nothing read, nothing sent.
        let refusal = if plan.items.is_empty() {
            Some("nothing to change".to_string())
        } else if plan.origin.as_ref() != Some(&here) {
            Some(match &plan.origin {
                None => "the plan names no profile it was planned on".to_string(),
                Some(o) => format!(
                    "planned on {}, this session writes to {}; plan again",
                    o.text(),
                    here.text()
                ),
            })
        } else if plan.needs_count() && plan.confirmed != Some(plan.items.len()) {
            Some(format!(
                "{} on {} workloads needs that count typed first",
                plan.change
                    .enforcement
                    .map_or("enforcement", EnforcementMode::as_str),
                plan.items.len()
            ))
        } else {
            plan.items
                .iter()
                .find(|i| checked_href(&i.href, "workloads", org).is_err())
                .map(|bad| format!("{} is not a workload href of this org", bad.href))
        };
        if let Some(why) = refusal {
            not_sent(&mut report, &plan.items, why);
            return report;
        }

        // The re-read: one list read past the cache.
        report.reads = 1;
        let now = match self.pce().workloads(&plan.reread, CacheMode::Bypass).await {
            Ok(c) => c.value,
            Err(e) => {
                not_sent(&mut report, &plan.items, format!("the re-read failed: {e}"));
                return report;
            }
        };
        let now: HashMap<&str, &Workload> = now.iter().map(|w| (w.href.as_str(), w)).collect();
        let placeholders: Vec<String> = (0..plan.creates.len()).map(step_placeholder).collect();
        let mut sendable: Vec<BulkItem> = Vec::new();
        for item in &plan.items {
            let Some(w) = now.get(item.href.as_str()) else {
                report.items.push(ItemResult {
                    href: item.href.clone(),
                    name: item.name.clone(),
                    change: item.change.clone(),
                    status: ItemStatus::Missing,
                    error: None,
                });
                continue;
            };
            let current = current_target(w, &item.base);
            let fresh = item
                .base
                .same_state(&current)
                .then(|| plan_one(&current, &plan.change, &placeholders));
            match fresh {
                Some(Ok((body, change))) => sendable.push(BulkItem {
                    body,
                    change,
                    base: current,
                    ..item.clone()
                }),
                _ => report.items.push(ItemResult {
                    href: item.href.clone(),
                    name: item.name.clone(),
                    change: item.change.clone(),
                    status: ItemStatus::Changed,
                    error: None,
                }),
            }
        }
        if sendable.is_empty() {
            return report;
        }
        let chunks = sendable.len().div_ceil(BULK_CHUNK);
        let total = plan.creates.len() + chunks;
        let mut done = 0;

        // The labels to create, one POST each.
        let mut created: Vec<String> = Vec::new();
        let mut stopped = false;
        for d in &plan.creates {
            let request = WriteRequest {
                method: WriteMethod::Post,
                path: format!("/orgs/{org}/labels"),
                body: Some(d.to_body()),
                summary: format!("create label {}", d.display()),
                objects: Vec::new(),
            };
            let mut step = BulkStep {
                method: request.method,
                path: request.path.clone(),
                summary: request.summary.clone(),
                status: StepStatus::NotSent,
                href: None,
                http_status: None,
                error: None,
                audit_failed: None,
            };
            if !stopped {
                let result = self
                    .execute(Plan::Send {
                        request,
                        kind: ObjectKind::Label,
                    })
                    .await;
                done += 1;
                progress(done, total);
                match result {
                    Ok(Outcome::Written(w)) => {
                        step.http_status = Some(w.status);
                        step.audit_failed = w.audit_failed.clone();
                        match checked_href(&w.href, "labels", org) {
                            Ok(h) => {
                                step.status = StepStatus::Written;
                                step.href = Some(h.clone());
                                report.labels_created = true;
                                created.push(h);
                            }
                            Err(e) => {
                                step.status = StepStatus::Failed;
                                step.error =
                                    Some(format!("created, but its href is unusable: {e}"));
                                report.labels_created = true;
                            }
                        }
                        stopped |=
                            step.status != StepStatus::Written || step.audit_failed.is_some();
                    }
                    Ok(Outcome::Skipped { existing_href, .. }) => {
                        step.status = StepStatus::Written;
                        step.href = Some(existing_href.clone());
                        created.push(existing_href);
                    }
                    Err(e) => {
                        let (st, code) = error_status(&e);
                        step.status = st;
                        step.http_status = code;
                        step.error = Some(e.to_string());
                        report.labels_created |= st == StepStatus::Unanswered;
                        stopped = true;
                    }
                }
            }
            report.steps.push(step);
        }

        // The chunks, one at a time.
        for (ci, chunk) in sendable.chunks(BULK_CHUNK).enumerate() {
            let mut request = chunk_request(org, chunk, ci, chunks);
            let mut step = BulkStep {
                method: request.method,
                path: request.path.clone(),
                summary: request.summary.clone(),
                status: StepStatus::NotSent,
                href: None,
                http_status: None,
                error: None,
                audit_failed: None,
            };
            let item = |i: &BulkItem, status: ItemStatus, error: Option<String>| ItemResult {
                href: i.href.clone(),
                name: i.name.clone(),
                change: i.change.clone(),
                status,
                error,
            };
            if stopped {
                report
                    .items
                    .extend(chunk.iter().map(|i| item(i, ItemStatus::NotSent, None)));
                report.steps.push(step);
                continue;
            }
            if let Some(Value::Array(bodies)) = request.body.as_mut() {
                let filled: Result<(), WriteError> =
                    bodies.iter_mut().try_for_each(|b| fill_labels(b, &created));
                if let Err(e) = filled {
                    step.status = StepStatus::Failed;
                    step.error = Some(e.to_string());
                    report
                        .items
                        .extend(chunk.iter().map(|i| item(i, ItemStatus::NotSent, None)));
                    report.steps.push(step);
                    stopped = true;
                    continue;
                }
            }
            let sent = request.body.clone();
            let result = self
                .execute(Plan::Send {
                    request,
                    kind: ObjectKind::WorkloadBulk,
                })
                .await;
            done += 1;
            progress(done, total);
            match result {
                Ok(Outcome::Written(w)) => {
                    step.http_status = Some(w.status);
                    step.audit_failed = w.audit_failed.clone();
                    let outcomes = chunk_outcomes(sent.as_ref(), &w.object);
                    let by: HashMap<&str, &Result<(), String>> =
                        outcomes.iter().map(|(h, r)| (h.as_str(), r)).collect();
                    let mut failed = false;
                    for i in chunk {
                        match by.get(i.href.as_str()) {
                            Some(Ok(())) => report.items.push(item(i, ItemStatus::Updated, None)),
                            Some(Err(e)) => {
                                failed = true;
                                report
                                    .items
                                    .push(item(i, ItemStatus::Failed, Some(e.clone())));
                            }
                            None => {
                                failed = true;
                                report.items.push(item(
                                    i,
                                    ItemStatus::Failed,
                                    Some("no result for this workload".into()),
                                ));
                            }
                        }
                    }
                    let any_ok = outcomes.iter().any(|(_, r)| r.is_ok());
                    step.status = if failed {
                        StepStatus::Partial
                    } else {
                        StepStatus::Written
                    };
                    report.labels_changed |= plan.change.changes_labels() && any_ok;
                    stopped = failed || step.audit_failed.is_some();
                }
                Ok(Outcome::Skipped { .. }) => {
                    step.status = StepStatus::Failed;
                    step.error = Some("the writer skipped the chunk".into());
                    report
                        .items
                        .extend(chunk.iter().map(|i| item(i, ItemStatus::NotSent, None)));
                    stopped = true;
                }
                Err(e) => {
                    let (st, code) = error_status(&e);
                    step.status = st;
                    step.http_status = code;
                    step.error = Some(e.to_string());
                    let (each, why) = if st == StepStatus::Unanswered {
                        report.labels_changed |= plan.change.changes_labels();
                        (ItemStatus::Unanswered, None)
                    } else {
                        (ItemStatus::Failed, Some(e.to_string()))
                    };
                    report
                        .items
                        .extend(chunk.iter().map(|i| item(i, each, why.clone())));
                    stopped = true;
                }
            }
            report.steps.push(step);
        }
        // A label change moves workloads between scopes: what holds a
        // label and the draft decisions are stale, as after any relabel.
        if report.labels_changed {
            for p in ObjectKind::Workload.cache_prefixes() {
                self.pce().invalidate(p);
            }
        }
        report
    }
}

/// The re-read workload as a target; a label the list names without a key
/// takes it from the planned one of the same href.
fn current_target(w: &Workload, base: &BulkTarget) -> BulkTarget {
    let labels = w
        .labels
        .iter()
        .map(|l| {
            let planned = base.labels.iter().find(|b| b.href == l.href);
            BulkLabel {
                href: l.href.clone(),
                key: l
                    .key
                    .clone()
                    .or_else(|| planned.map(|p| p.key.clone()))
                    .unwrap_or_default(),
                value: l
                    .value
                    .clone()
                    .or_else(|| planned.map(|p| p.value.clone()))
                    .unwrap_or_default(),
            }
        })
        .collect();
    BulkTarget {
        href: w.href.clone(),
        name: w.display_name(),
        managed: managed(w),
        enforcement: w.enforcement_mode.clone(),
        visibility: w.visibility_level.clone(),
        labels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(href: &str, key: &str, value: &str) -> BulkLabel {
        BulkLabel {
            href: href.into(),
            key: key.into(),
            value: value.into(),
        }
    }

    fn target(n: usize, mode: &str, managed: bool, labels: Vec<BulkLabel>) -> BulkTarget {
        BulkTarget {
            href: format!("/orgs/5/workloads/w{n}"),
            name: format!("host-{n}"),
            managed,
            enforcement: Some(mode.into()),
            visibility: Some("flow_summary".into()),
            labels,
        }
    }

    fn plan(targets: &[BulkTarget], change: &BulkChange) -> BulkPlan {
        plan_bulk(targets, change, WorkloadFilter::default(), None).unwrap()
    }

    #[test]
    fn enforcement_skips_unmanaged_and_already_set() {
        let ts = vec![
            target(1, "visibility_only", true, vec![]),
            target(2, "full", true, vec![]),
            target(3, "visibility_only", false, vec![]),
            target(4, "idle", true, vec![]),
        ];
        let p = plan(&ts, &BulkChange::enforce(EnforcementMode::Full, None));
        assert_eq!(p.items.len(), 2);
        assert_eq!(
            p.items[0].body,
            json!({"href": "/orgs/5/workloads/w1", "enforcement_mode": "full"})
        );
        assert_eq!(p.items[1].change, "idle → full");
        assert_eq!(
            p.summary(),
            "4 workloads: 1 idle → full · 1 visibility_only → full · 1 already full (skipped) · 1 unmanaged (skipped)"
        );
        assert!(p.needs_count());
        let one = plan(&ts[..1], &BulkChange::enforce(EnforcementMode::Full, None));
        assert!(!one.needs_count(), "one workload: no typed count");
        let vis = plan(
            &ts,
            &BulkChange::enforce(EnforcementMode::VisibilityOnly, None),
        );
        assert!(!vis.needs_count(), "visibility_only blocks nothing");
    }

    #[test]
    fn a_visibility_level_is_sent_and_counts_as_a_change() {
        let ts = vec![target(2, "full", true, vec![])];
        let p = plan(
            &ts,
            &BulkChange::enforce(EnforcementMode::Full, Some(VisibilityLevel::FlowDrops)),
        );
        assert_eq!(
            p.items[0].body,
            json!({"href": "/orgs/5/workloads/w2", "enforcement_mode": "full", "visibility_level": "flow_drops"})
        );
        assert_eq!(p.items[0].change, "full → full · flow_summary → flow_drops");
    }

    #[test]
    fn labels_set_remove_keep_and_mixed() {
        let shop = label("/orgs/5/labels/1", "app", "shop");
        let crm = label("/orgs/5/labels/2", "app", "crm");
        let prod = label("/orgs/5/labels/3", "env", "prod");
        let web = label("/orgs/5/labels/4", "role", "web");
        let ts = vec![
            target(
                1,
                "full",
                true,
                vec![crm.clone(), prod.clone(), web.clone()],
            ),
            target(2, "full", false, vec![shop.clone()]),
            target(3, "full", true, vec![prod.clone()]),
        ];
        let edit = LabelEdit {
            set: vec![LabelSet::Existing(shop.clone())],
            remove: vec![LabelRemoval {
                key: "env".into(),
                value: None,
            }],
        };
        let p = plan(&ts, &BulkChange::relabel(edit));
        // Unmanaged workloads take labels too; host-2 already so.
        assert_eq!(p.items.len(), 2);
        assert_eq!(
            p.items[0].body,
            json!({"href": "/orgs/5/workloads/w1", "labels": [{"href": "/orgs/5/labels/4"}, {"href": "/orgs/5/labels/1"}]})
        );
        assert_eq!(p.items[0].change, "app=crm → app=shop · -env=prod");
        assert_eq!(p.items[1].change, "+app=shop · -env=prod");
        assert_eq!(p.skipped[0].reason, "labels already so");
        assert_eq!(
            p.summary(),
            "3 workloads: 2 to relabel (app=shop · -env) · 1 labels already so (skipped)"
        );
        assert!(!p.needs_count());
    }

    #[test]
    fn a_removal_with_a_value_takes_only_that_value() {
        let prod = label("/orgs/5/labels/3", "env", "prod");
        let dev = label("/orgs/5/labels/5", "env", "dev");
        let ts = vec![
            target(1, "full", true, vec![prod.clone()]),
            target(2, "full", true, vec![dev.clone()]),
        ];
        let edit = LabelEdit {
            set: vec![],
            remove: vec![LabelRemoval {
                key: "env".into(),
                value: Some("prod".into()),
            }],
        };
        let p = plan(&ts, &BulkChange::relabel(edit));
        assert_eq!(p.items.len(), 1);
        assert_eq!(p.items[0].change, "-env=prod");
        assert_eq!(p.items[0].body["labels"], json!([]));
        assert_eq!(p.skipped[0].name, "host-2");
    }

    #[test]
    fn a_label_to_create_is_a_placeholder_until_sent() {
        let ts = vec![target(1, "full", true, vec![])];
        let edit = LabelEdit {
            set: vec![LabelSet::Create(LabelDraft::new("app", "new"))],
            remove: vec![],
        };
        let p = plan(&ts, &BulkChange::relabel(edit));
        assert_eq!(p.creates, vec![LabelDraft::new("app", "new")]);
        assert_eq!(p.items[0].body["labels"][0]["href"], "{step:0}");
        let reqs = p.requests(5);
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].path, "/orgs/5/labels");
        assert_eq!(reqs[1].path, "/orgs/5/workloads/bulk_update");
        let mut body = p.items[0].body.clone();
        fill_labels(&mut body, &["/orgs/5/labels/99".into()]).unwrap();
        assert_eq!(body["labels"][0]["href"], "/orgs/5/labels/99");
        let mut bad = p.items[0].body.clone();
        assert!(fill_labels(&mut bad, &[]).is_err(), "no href: refused");
        assert!(
            p.lines(5)
                .iter()
                .any(|l| l.contains("<href of step 1: label app=new>")),
            "{:?}",
            p.lines(5)
        );
    }

    #[test]
    fn chunks_of_a_hundred() {
        let ts: Vec<BulkTarget> = (0..250)
            .map(|n| target(n, "visibility_only", true, vec![]))
            .collect();
        let p = plan(&ts, &BulkChange::enforce(EnforcementMode::Idle, None));
        let reqs = p.requests(5);
        let sizes: Vec<usize> = reqs
            .iter()
            .map(|r| r.body.as_ref().unwrap().as_array().unwrap().len())
            .collect();
        assert_eq!(sizes, vec![100, 100, 50]);
        assert_eq!(reqs[2].summary, "bulk update 50 workloads (3/3)");
        assert_eq!(p.cost_text(), "1 re-read + 3 writes: 3 bulk_update");
    }

    #[test]
    fn invalid_changes_are_refused() {
        let ts = vec![target(1, "full", true, vec![])];
        let shop = label("/orgs/5/labels/1", "app", "shop");
        for change in [
            BulkChange::default(),
            BulkChange {
                visibility: Some(VisibilityLevel::FlowOff),
                ..BulkChange::default()
            },
            BulkChange::relabel(LabelEdit {
                set: vec![
                    LabelSet::Existing(shop.clone()),
                    LabelSet::Existing(shop.clone()),
                ],
                remove: vec![],
            }),
            BulkChange::relabel(LabelEdit {
                set: vec![LabelSet::Existing(shop.clone())],
                remove: vec![LabelRemoval {
                    key: "app".into(),
                    value: None,
                }],
            }),
            BulkChange {
                enforcement: Some(EnforcementMode::Full),
                visibility: None,
                labels: LabelEdit {
                    set: vec![],
                    remove: vec![LabelRemoval {
                        key: "app".into(),
                        value: None,
                    }],
                },
            },
        ] {
            assert!(
                plan_bulk(&ts, &change, WorkloadFilter::default(), None).is_err(),
                "{change:?}"
            );
        }
        assert!(
            plan_bulk(
                &[],
                &BulkChange::enforce(EnforcementMode::Full, None),
                WorkloadFilter::default(),
                None
            )
            .is_err()
        );
    }

    #[test]
    fn outcomes_from_the_answer() {
        let sent = json!([{"href": "/w/1"}, {"href": "/w/2"}, {"href": "/w/3"}]);
        let answer = json!([
            {"href": "/w/1", "status": "updated"},
            {"href": "/w/2", "status": "validation_failure", "errors": [{"token": "x", "message": "bad mode"}]},
        ]);
        let o = chunk_outcomes(Some(&sent), &answer);
        assert_eq!(o[0], ("/w/1".into(), Ok(())));
        assert_eq!(
            o[1],
            ("/w/2".into(), Err("validation_failure: bad mode".into()))
        );
        assert!(o[2].1.is_err(), "missing entry is a failure");
        let none = chunk_outcomes(Some(&sent), &Value::Null);
        assert!(none.iter().all(|(_, r)| r.is_err()));
        assert_eq!(
            audit_items(Some(&sent), &answer)[1],
            json!({"href": "/w/2", "status": "failed", "error": "validation_failure: bad mode"})
        );
    }

    #[test]
    fn state_comparison_ignores_label_order() {
        let a = label("/orgs/5/labels/1", "app", "shop");
        let b = label("/orgs/5/labels/3", "env", "prod");
        let t1 = target(1, "full", true, vec![a.clone(), b.clone()]);
        let t2 = target(1, "full", true, vec![b.clone(), a.clone()]);
        assert!(t1.same_state(&t2));
        let mut t3 = t2.clone();
        t3.enforcement = Some("idle".into());
        assert!(!t1.same_state(&t3));
        let mut t4 = t2.clone();
        t4.labels.pop();
        assert!(!t1.same_state(&t4));
    }

    #[test]
    fn the_draft_check_names_each_target_as_destination() {
        let q = draft_check_query(
            &["/orgs/5/workloads/a".into(), "/orgs/5/workloads/b".into()],
            Duration::from_secs(3600),
            1000,
        )
        .unwrap();
        assert_eq!(q.destinations.include.len(), 2);
        assert_eq!(q.max_results, 1000);
        let many: Vec<String> = (0..=DRAFT_CHECK_MAX_TARGETS)
            .map(|i| format!("/orgs/5/workloads/{i}"))
            .collect();
        assert!(draft_check_query(&many, Duration::from_secs(60), 10).is_err());
        assert!(draft_check_query(&[], Duration::from_secs(60), 10).is_err());
    }
}
