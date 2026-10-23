//! The VEN fleet of a loaded workload list: counts per health state, VEN
//! version, enforcement and health message, and the health and version
//! filters. Built from the list already read: never a request.
use crate::models::event_select::glob;
use crate::models::{VenHealth, Workload};
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::HashMap;

/// A set of health states: `--health warning,error`, the app's `v`.
/// Empty keeps every workload.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct HealthSet(u8);

impl HealthSet {
    fn bit(h: VenHealth) -> u8 {
        1 << VenHealth::ALL.iter().position(|x| *x == h).unwrap_or(0)
    }

    pub fn one(h: VenHealth) -> Self {
        Self(Self::bit(h))
    }

    pub fn insert(&mut self, h: VenHealth) {
        self.0 |= Self::bit(h);
    }

    pub fn contains(self, h: VenHealth) -> bool {
        self.0 & Self::bit(h) != 0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The one state of a set of one.
    pub fn single(self) -> Option<VenHealth> {
        let mut it = self.iter();
        match (it.next(), it.next()) {
            (Some(h), None) => Some(h),
            _ => None,
        }
    }

    /// The states in [`VenHealth::ALL`] order.
    pub fn iter(self) -> impl Iterator<Item = VenHealth> {
        VenHealth::ALL
            .into_iter()
            .filter(move |h| self.contains(*h))
    }

    /// Empty, or one of the states.
    pub fn keeps(self, h: VenHealth) -> bool {
        self.is_empty() || self.contains(h)
    }

    /// States by name (blanks skipped); an unknown name is an error that
    /// lists the known ones.
    pub fn from_names<S: AsRef<str>>(names: &[S]) -> Result<Self, String> {
        let mut set = Self::default();
        for n in names
            .iter()
            .map(|n| n.as_ref().trim())
            .filter(|n| !n.is_empty())
        {
            let h = VenHealth::parse(n).ok_or_else(|| {
                let known: Vec<&str> = VenHealth::ALL.iter().map(|h| h.as_str()).collect();
                format!("unknown health {n:?} (one of {})", known.join(", "))
            })?;
            set.insert(h);
        }
        Ok(set)
    }

    /// `warning,error`: comma-separated names.
    pub fn parse(text: &str) -> Result<Self, String> {
        Self::from_names(&text.split(',').collect::<Vec<_>>())
    }

    pub fn names(self) -> Vec<String> {
        self.iter().map(|h| h.as_str().to_string()).collect()
    }
}

impl std::fmt::Display for HealthSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.names().join(","))
    }
}

/// One part of a version: its leading number (if any) and the rest.
fn version_part(p: &str) -> (Option<u64>, &str) {
    let digits = p.bytes().take_while(u8::is_ascii_digit).count();
    (p[..digits].parse().ok(), &p[digits..])
}

/// VEN versions in release order: numeric per part (`23.10` after `23.2`),
/// parts split at `.` `-` `_` `+`; a part without a number (`rc1`) comes
/// before one with. A further numeric part is later (`23.2.31-3113` after
/// `23.2.31`), a further text part is a pre-release, earlier
/// (`23.2.31-rc1` before `23.2.31`). Equal parts fall back to the text, so
/// the order is total.
pub fn cmp_versions(a: &str, b: &str) -> Ordering {
    let split = |s: &str| -> Vec<(Option<u64>, String)> {
        s.trim()
            .split(['.', '-', '_', '+'])
            .map(|p| {
                let (n, rest) = version_part(p);
                (n, rest.to_ascii_lowercase())
            })
            .collect()
    };
    let (x, y) = (split(a), split(b));
    // What one version has past the other's end: numeric is later, text earlier.
    let tail = |extra: &(Option<u64>, String)| {
        if extra.0.is_some() {
            Ordering::Greater
        } else {
            Ordering::Less
        }
    };
    let parts = x
        .iter()
        .zip(&y)
        .map(|(p, q)| p.cmp(q))
        .find(|o| o.is_ne())
        .unwrap_or_else(|| match (x.get(y.len()), y.get(x.len())) {
            (Some(extra), _) => tail(extra),
            (_, Some(extra)) => tail(extra).reverse(),
            _ => Ordering::Equal,
        });
    parts.then_with(|| a.trim().cmp(b.trim()))
}

/// `--ven-version`: the exact version, or a `*` glob (`23.2.*`); any case.
pub fn version_matches(pattern: &str, version: &str) -> bool {
    let p = pattern.trim().to_ascii_lowercase();
    let v = version.trim().to_ascii_lowercase();
    if p.contains('*') {
        glob(&p, &v)
    } else {
        p == v
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HealthCount {
    pub health: VenHealth,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersionCount {
    pub version: String,
    pub count: usize,
    /// The newest version loaded.
    pub newest: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnforcementCount {
    /// The effective enforcement: `unmanaged` without a VEN.
    pub enforcement: String,
    pub count: usize,
}

/// A health message (`type: text`) and the VENs that report it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MessageCount {
    /// `error` or `warning`.
    pub severity: &'static str,
    pub message: String,
    pub count: usize,
}

/// The fleet: every count from the loaded workloads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct VenSummary {
    /// Workloads loaded.
    pub workloads: usize,
    /// Of them with a VEN (not unmanaged).
    pub vens: usize,
    /// The list was capped: every count is a lower bound.
    pub truncated: bool,
    /// Every state, in [`VenHealth::ALL`] order.
    pub health: Vec<HealthCount>,
    /// Newest first.
    pub versions: Vec<VersionCount>,
    pub newest: Option<String>,
    /// VENs on a version older than `newest`.
    pub behind: usize,
    /// VENs that report no version.
    pub no_version: usize,
    /// Most first, then by name.
    pub enforcement: Vec<EnforcementCount>,
    /// Most first, errors before warnings, then by text.
    pub messages: Vec<MessageCount>,
}

impl VenSummary {
    /// The summary of `workloads`, health at `now`.
    pub fn of(workloads: &[Workload], now: jiff::Timestamp, truncated: bool) -> Self {
        let mut t = VenTally::default();
        for w in workloads {
            t.add(w, w.ven_health(now));
        }
        t.finish(truncated)
    }

    pub fn count(&self, h: VenHealth) -> usize {
        self.health
            .iter()
            .find(|c| c.health == h)
            .map_or(0, |c| c.count)
    }
}

/// Counts as workloads are added, for a caller that has their health
/// already (the app's rebuild).
#[derive(Debug, Default)]
pub struct VenTally {
    workloads: usize,
    health: [usize; 5],
    versions: HashMap<String, usize>,
    no_version: usize,
    enforcement: HashMap<String, usize>,
    messages: HashMap<(&'static str, String), usize>,
}

impl VenTally {
    pub fn add(&mut self, w: &Workload, health: VenHealth) {
        self.workloads += 1;
        if let Some(i) = VenHealth::ALL.iter().position(|h| *h == health) {
            self.health[i] += 1;
        }
        if let Some(e) = w.enforcement() {
            *self.enforcement.entry(e.to_string()).or_default() += 1;
        }
        if health == VenHealth::Unmanaged {
            return;
        }
        match w.agent_version().map(str::trim).filter(|v| !v.is_empty()) {
            Some(v) => *self.versions.entry(v.to_string()).or_default() += 1,
            None => self.no_version += 1,
        }
        // A healthy VEN reports no message; the others may.
        if health != VenHealth::Healthy
            && let Some(a) = w.agent_status_view()
        {
            let mut seen: Vec<(&'static str, &String)> = Vec::new();
            let all = a
                .errors
                .iter()
                .map(|m| ("error", m))
                .chain(a.warnings.iter().map(|m| ("warning", m)));
            for (sev, m) in all {
                if !seen.contains(&(sev, m)) {
                    seen.push((sev, m));
                    *self.messages.entry((sev, m.clone())).or_default() += 1;
                }
            }
        }
    }

    pub fn finish(self, truncated: bool) -> VenSummary {
        let mut versions: Vec<(String, usize)> = self.versions.into_iter().collect();
        versions.sort_by(|a, b| cmp_versions(&b.0, &a.0));
        let newest = versions.first().map(|(v, _)| v.clone());
        let behind = versions.iter().skip(1).map(|(_, n)| n).sum();
        let mut enforcement: Vec<EnforcementCount> = self
            .enforcement
            .into_iter()
            .map(|(enforcement, count)| EnforcementCount { enforcement, count })
            .collect();
        enforcement.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then(a.enforcement.cmp(&b.enforcement))
        });
        let mut messages: Vec<MessageCount> = self
            .messages
            .into_iter()
            .map(|((severity, message), count)| MessageCount {
                severity,
                message,
                count,
            })
            .collect();
        messages.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then(a.severity.cmp(b.severity))
                .then(a.message.cmp(&b.message))
        });
        VenSummary {
            workloads: self.workloads,
            vens: self.workloads - self.health[4],
            truncated,
            health: VenHealth::ALL
                .into_iter()
                .zip(self.health)
                .map(|(health, count)| HealthCount { health, count })
                .collect(),
            versions: versions
                .into_iter()
                .enumerate()
                .map(|(i, (version, count))| VersionCount {
                    version,
                    count,
                    newest: i == 0,
                })
                .collect(),
            newest,
            behind,
            no_version: self.no_version,
            enforcement,
            messages,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<Workload> {
        serde_json::from_str(include_str!("../../../fixtures/workloads.json")).unwrap()
    }

    fn now() -> jiff::Timestamp {
        "2026-10-10T08:00:00Z".parse().unwrap()
    }

    #[test]
    fn versions_order_numerically_per_part_and_tolerate_suffixes() {
        let mut v = vec![
            "23.2.31-3113",
            "23.10.0",
            "22.5.10-1",
            "23.2.31",
            "23.2.4",
            "23.2.31-rc1",
            "9.9",
        ];
        v.sort_by(|a, b| cmp_versions(b, a));
        assert_eq!(
            v,
            [
                "23.10.0",
                "23.2.31-3113",
                "23.2.31",
                "23.2.31-rc1",
                "23.2.4",
                "22.5.10-1",
                "9.9"
            ]
        );
        assert_eq!(cmp_versions("23.10", "23.2"), Ordering::Greater);
        assert_eq!(cmp_versions("23.2", "23.2"), Ordering::Equal);
        assert_eq!(cmp_versions("23.2.31-rc1", "23.2.31"), Ordering::Less);
        assert_eq!(cmp_versions("23.2.31", "23.2.31-rc1"), Ordering::Greater);
        // Equal numbers, different text: still a total order.
        assert_ne!(cmp_versions("23.02", "23.2"), Ordering::Equal);
    }

    #[test]
    fn version_patterns_are_exact_or_globs() {
        assert!(version_matches("23.2.31-3113", "23.2.31-3113"));
        assert!(
            !version_matches("23.2", "23.2.31-3113"),
            "exact, not a prefix"
        );
        assert!(version_matches("23.2.*", "23.2.31-3113"));
        assert!(version_matches("*-3113", "23.2.31-3113"));
        assert!(!version_matches("22.*", "23.2.31-3113"));
        assert!(version_matches(" 23.2.31-RC1 ", "23.2.31-rc1"));
        // A padded version as the PCE may report it: the bucket is trimmed.
        assert!(version_matches("23.2.31", " 23.2.31 "));
        assert!(version_matches("23.2.*", "23.2.31 "));
    }

    #[test]
    fn health_sets_parse_names_and_say_what_is_known() {
        let s = HealthSet::parse("warning, error,").unwrap();
        assert!(s.contains(VenHealth::Warning) && s.contains(VenHealth::Error));
        assert!(!s.keeps(VenHealth::Healthy));
        assert_eq!(s.to_string(), "warning,error");
        assert_eq!(s.single(), None);
        assert_eq!(
            HealthSet::one(VenHealth::Offline).single(),
            Some(VenHealth::Offline)
        );
        assert!(HealthSet::default().keeps(VenHealth::Healthy), "empty: all");
        let e = HealthSet::parse("warn").unwrap_err();
        assert!(e.contains("\"warn\"") && e.contains("unmanaged"), "{e}");
    }

    #[test]
    fn the_summary_counts_health_versions_enforcement_and_messages() {
        let s = VenSummary::of(&fixture(), now(), false);
        assert_eq!((s.workloads, s.vens), (4, 3));
        let health: Vec<(&str, usize)> = s
            .health
            .iter()
            .map(|c| (c.health.as_str(), c.count))
            .collect();
        assert_eq!(
            health,
            [
                ("healthy", 1),
                ("warning", 1),
                ("error", 0),
                ("offline", 1),
                ("unmanaged", 1)
            ]
        );
        let versions: Vec<(&str, usize, bool)> = s
            .versions
            .iter()
            .map(|v| (v.version.as_str(), v.count, v.newest))
            .collect();
        assert_eq!(
            versions,
            [("23.2.31-3113", 2, true), ("22.5.10-1", 1, false)]
        );
        assert_eq!((s.newest.as_deref(), s.behind), (Some("23.2.31-3113"), 1));
        assert_eq!(s.enforcement.len(), 4, "{:?}", s.enforcement);
        assert!(s.enforcement.iter().any(|e| e.enforcement == "unmanaged"));
        // The offline VEN's warning, once (legacy and agent_health agree).
        assert_eq!(
            s.messages,
            [MessageCount {
                severity: "warning",
                message: "policy_sync: policy out of date".into(),
                count: 1
            }]
        );
    }

    #[test]
    fn an_empty_list_summarises_to_zeros() {
        let s = VenSummary::of(&[], now(), true);
        assert_eq!((s.workloads, s.vens, s.behind), (0, 0, 0));
        assert!(s.truncated && s.newest.is_none() && s.versions.is_empty());
        assert_eq!(s.health.len(), 5);
    }
}
