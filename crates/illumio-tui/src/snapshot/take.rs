//! The reads a snapshot is made of: the same cached list reads as the rest
//! of the tool, one after another, each once. `policy`: the draft and
//! active rulesets, label groups, IP lists, services and enforcement
//! boundaries (six GETs); `labels`: one; `workloads`: one unfiltered list.
use super::{PolicySection, Sections, Snapshot, VERSION, trim_workload};
use pce::models::{PolicyScope, WorkloadFilter};
use pce::{CacheMode, Pce};

/// The sections to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct What {
    pub policy: bool,
    pub labels: bool,
    pub workloads: bool,
}

impl Default for What {
    /// Everything but readiness (`--with-readiness` adds it).
    fn default() -> Self {
        Self {
            policy: true,
            labels: true,
            workloads: true,
        }
    }
}

/// Whose snapshot it is: what `diff` refuses to mix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub profile: String,
    pub pce: String,
    pub org: u64,
}

/// A snapshot read, and whether every read was a cache hit.
#[derive(Debug, Clone)]
pub struct Taken {
    pub snapshot: Snapshot,
    pub all_hit: bool,
}

/// Reads `what` and makes a snapshot of it, taken now.
pub async fn take(
    pce: &Pce,
    mode: CacheMode,
    what: What,
    id: &Identity,
    label: Option<String>,
) -> pce::Result<Taken> {
    let mut sections = Sections::default();
    let mut truncated = Vec::new();
    let mut all_hit = true;
    let mut note = |name: &str, hit: bool, cut: bool| {
        all_hit &= hit;
        if cut {
            truncated.push(name.to_string());
        }
    };
    if what.policy {
        let draft = pce.rule_sets(PolicyScope::Draft, mode).await?;
        note("draft_rule_sets", draft.hit, draft.truncated);
        let active = pce.rule_sets(PolicyScope::Active, mode).await?;
        note("active_rule_sets", active.hit, active.truncated);
        let groups = pce.label_groups(mode).await?;
        note("label_groups", groups.hit, groups.truncated);
        let lists = pce.ip_lists(mode).await?;
        note("ip_lists", lists.hit, lists.truncated);
        let services = pce.services(mode).await?;
        note("services", services.hit, services.truncated);
        let boundaries = pce.enforcement_boundaries(mode).await?;
        note(
            "enforcement_boundaries",
            boundaries.hit,
            boundaries.truncated,
        );
        sections.policy = Some(PolicySection {
            draft_rule_sets: draft.value,
            active_rule_sets: active.value,
            label_groups: groups.value,
            ip_lists: lists.value,
            services: services.value,
            enforcement_boundaries: boundaries.value,
        });
    }
    if what.labels {
        let labels = pce.labels(mode).await?;
        note("labels", labels.hit, labels.truncated);
        sections.labels = Some(labels.value);
    }
    if what.workloads {
        let wl = pce.workloads(&WorkloadFilter::default(), mode).await?;
        note("workloads", wl.hit, wl.truncated);
        sections.workloads = Some(wl.value.into_iter().map(trim_workload).collect());
    }
    Ok(Taken {
        snapshot: Snapshot {
            version: VERSION,
            profile: id.profile.clone(),
            pce: id.pce.clone(),
            org: id.org,
            taken_at: jiff::Timestamp::now(),
            label,
            truncated,
            sections,
        },
        all_hit,
    })
}
