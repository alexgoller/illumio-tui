//! `:ven`: the VEN fleet overlay's lines, from `WorkloadsState::ven`
//! (counts per health state, version, enforcement and health message), and
//! the filter `Enter` on a bucket sets. Never a request.
use crate::tui::model::WorkloadFilters;
use pce::models::VenHealth;
use pce::ven::VenSummary;

/// What a row counts; `Enter` filters the list to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VenBucket {
    Health(VenHealth),
    Version(String),
    /// The effective enforcement (`unmanaged` without a VEN).
    Enforcement(String),
    /// A health message with its severity (`error`, `warning`).
    Message(&'static str, String),
}

impl VenBucket {
    /// Sets this bucket's filter on `f`, the others kept; an enforcement
    /// the `E` cycle does not know is refused.
    pub fn apply(&self, f: &mut WorkloadFilters) -> Result<(), String> {
        match self {
            VenBucket::Health(h) => f.health = pce::ven::HealthSet::one(*h),
            VenBucket::Version(v) => f.ven_version = Some(v.clone()),
            VenBucket::Enforcement(e) => {
                f.enforcement = Some(
                    crate::filters::ENFORCEMENTS
                        .into_iter()
                        .find(|k| *k == e.as_str())
                        .ok_or_else(|| format!("enforcement {e} cannot be filtered on"))?,
                );
            }
            VenBucket::Message(sev, m) => f.message = Some((sev, m.clone())),
        }
        Ok(())
    }
}

/// One overlay line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VenLine {
    /// A section title and what it adds (`newest 23.2.31 · 4 behind`).
    Section(&'static str, String),
    Bucket {
        bucket: VenBucket,
        /// `newest`, `behind`, `error`, `warning`.
        tag: &'static str,
        count: usize,
    },
    /// Not selectable: `no version 2`, `none`.
    Note(String),
}

/// The overlay's lines: health, versions (newest first), enforcement, and
/// the health messages (most common first).
pub fn lines(s: &VenSummary) -> Vec<VenLine> {
    let mut out = vec![VenLine::Section("health", String::new())];
    out.extend(s.health.iter().map(|c| VenLine::Bucket {
        bucket: VenBucket::Health(c.health),
        tag: "",
        count: c.count,
    }));
    let note = match &s.newest {
        Some(v) => format!("newest {v} · {} behind", s.behind),
        None => String::new(),
    };
    out.push(VenLine::Section("versions", note));
    out.extend(s.versions.iter().map(|v| VenLine::Bucket {
        bucket: VenBucket::Version(v.version.clone()),
        tag: if v.newest { "newest" } else { "behind" },
        count: v.count,
    }));
    if s.no_version > 0 {
        out.push(VenLine::Note(format!(
            "no version reported: {}",
            s.no_version
        )));
    } else if s.versions.is_empty() {
        out.push(VenLine::Note("none".into()));
    }
    out.push(VenLine::Section("enforcement", String::new()));
    out.extend(s.enforcement.iter().map(|e| VenLine::Bucket {
        bucket: VenBucket::Enforcement(e.enforcement.clone()),
        tag: "",
        count: e.count,
    }));
    if s.enforcement.is_empty() {
        out.push(VenLine::Note("none".into()));
    }
    out.push(VenLine::Section("messages", String::new()));
    out.extend(s.messages.iter().map(|m| VenLine::Bucket {
        bucket: VenBucket::Message(m.severity, m.message.clone()),
        tag: m.severity,
        count: m.count,
    }));
    if s.messages.is_empty() {
        out.push(VenLine::Note("none".into()));
    }
    out
}

/// The line index of each bucket, in order: `selected` indexes this.
pub fn bucket_lines(lines: &[VenLine]) -> Vec<usize> {
    lines
        .iter()
        .enumerate()
        .filter(|(_, l)| matches!(l, VenLine::Bucket { .. }))
        .map(|(i, _)| i)
        .collect()
}

/// The `selected`th bucket (clamped), if any.
pub fn bucket_at(lines: &[VenLine], selected: usize) -> Option<&VenBucket> {
    let at = bucket_lines(lines);
    let i = *at.get(selected.min(at.len().saturating_sub(1)))?;
    match &lines[i] {
        VenLine::Bucket { bucket, .. } => Some(bucket),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary() -> VenSummary {
        let w: Vec<pce::models::Workload> =
            serde_json::from_str(include_str!("../../../../fixtures/workloads.json")).unwrap();
        VenSummary::of(&w, "2026-10-10T08:00:00Z".parse().unwrap(), false)
    }

    #[test]
    fn lines_hold_every_section_and_buckets_are_selectable() {
        let l = lines(&summary());
        let sections: Vec<&str> = l
            .iter()
            .filter_map(|l| match l {
                VenLine::Section(s, _) => Some(*s),
                _ => None,
            })
            .collect();
        assert_eq!(sections, ["health", "versions", "enforcement", "messages"]);
        // 5 health, 2 versions, 4 modes, 1 message.
        assert_eq!(bucket_lines(&l).len(), 12);
        assert_eq!(
            bucket_at(&l, 5),
            Some(&VenBucket::Version("23.2.31-3113".into()))
        );
        assert_eq!(
            bucket_at(&l, 99),
            Some(&VenBucket::Message(
                "warning",
                "policy_sync: policy out of date".into()
            )),
            "clamped"
        );
    }

    #[test]
    fn a_bucket_sets_its_filter_and_keeps_the_others() {
        let mut f = WorkloadFilters {
            offline: true,
            ..WorkloadFilters::default()
        };
        VenBucket::Version("23.2.31-3113".into())
            .apply(&mut f)
            .unwrap();
        VenBucket::Enforcement("unmanaged".into())
            .apply(&mut f)
            .unwrap();
        assert!(f.offline);
        assert_eq!(f.ven_version.as_deref(), Some("23.2.31-3113"));
        assert_eq!(f.enforcement, Some("unmanaged"));
        assert!(
            VenBucket::Enforcement("weird".into())
                .apply(&mut f)
                .is_err()
        );
    }
}
