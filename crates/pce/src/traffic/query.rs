//! Builds a `TrafficQuery` that the PCE will accept: full UTC timestamps,
//! `[[]]` for an empty include, validated window and caps.
use crate::error::{PceError, Result};
use crate::models::{
    ActorFilter, ActorMatch, PolicyDecision, ServiceFilter, ServiceMatch, TrafficQuery,
    Transmission,
};
use jiff::{Span, Timestamp};
use std::time::Duration;

pub const DEFAULT_WINDOW: Duration = Duration::from_secs(24 * 3600);
pub const DEFAULT_TRAFFIC_MAX_RESULTS: u32 = 10_000;
pub const CONFIRM_WINDOW: Duration = Duration::from_secs(7 * 24 * 3600);
/// Caps above this need a confirmation (`--yes`, or `y` in the TUI).
pub const CONFIRM_MAX_RESULTS: u32 = 10_000;
/// The largest cap any query may ask for (Explorer's limit).
pub const MAX_TRAFFIC_MAX_RESULTS: u32 = 200_000;

/// `2026-10-10T07:31:02Z`
pub fn format_timestamp(t: Timestamp) -> String {
    t.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// `24h`, `7d`, `90m`, `1w` via humantime.
pub fn parse_window(s: &str) -> Result<Duration> {
    humantime::parse_duration(s.trim())
        .map_err(|e| PceError::Invalid(format!("bad window {s:?}: {e}")))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct QueryCost {
    pub window: Duration,
    pub max_results: u32,
    /// No source, destination or service filter at all.
    pub unbounded: bool,
}

impl TrafficQuery {
    pub fn window(&self) -> Duration {
        let start: Option<Timestamp> = self.start_date.parse().ok();
        let end: Option<Timestamp> = self.end_date.parse().ok();
        match (start, end) {
            (Some(s), Some(e)) if e > s => e.duration_since(s).unsigned_abs(),
            _ => Duration::ZERO,
        }
    }

    pub fn cost(&self) -> QueryCost {
        // A transmission exclude narrows by kind, not by actor.
        let has_actor = |f: &ActorFilter| {
            f.include.iter().any(|g| !g.is_empty())
                || f.exclude
                    .iter()
                    .any(|m| !matches!(m, ActorMatch::Transmission { .. }))
        };
        let unbounded = !has_actor(&self.sources)
            && !has_actor(&self.destinations)
            && self.services.include.is_empty();
        QueryCost {
            window: self.window(),
            max_results: self.max_results,
            unbounded,
        }
    }

    /// A reason the caller should confirm before running, if any.
    pub fn needs_confirmation(&self) -> Option<String> {
        let c = self.cost();
        let mut reasons = Vec::new();
        if c.window > CONFIRM_WINDOW {
            reasons.push(format!(
                "window is {}d (> {}d)",
                c.window.as_secs() / 86400,
                CONFIRM_WINDOW.as_secs() / 86400
            ));
        }
        if c.max_results > CONFIRM_MAX_RESULTS {
            reasons.push(format!(
                "max_results is {} (> {})",
                c.max_results, CONFIRM_MAX_RESULTS
            ));
        }
        if reasons.is_empty() {
            None
        } else {
            Some(reasons.join("; "))
        }
    }
}

#[derive(Debug, Clone)]
pub struct TrafficQueryBuilder {
    start: Timestamp,
    end: Timestamp,
    sources: ActorFilter,
    destinations: ActorFilter,
    services: ServiceFilter,
    decisions: Vec<PolicyDecision>,
    transmission: Vec<Transmission>,
    max_results: u32,
    query_name: String,
}

impl TrafficQueryBuilder {
    pub fn between(start: Timestamp, end: Timestamp) -> Self {
        Self {
            start,
            end,
            sources: ActorFilter::default(),
            destinations: ActorFilter::default(),
            services: ServiceFilter::default(),
            decisions: PolicyDecision::ALL.to_vec(),
            transmission: Vec::new(),
            max_results: DEFAULT_TRAFFIC_MAX_RESULTS,
            query_name: "illumio-tui".into(),
        }
    }

    /// The last `window` ending now.
    pub fn last(window: Duration) -> Result<Self> {
        let end = Timestamp::now();
        let span = Span::try_from(window)
            .map_err(|e| PceError::Invalid(format!("window too large: {e}")))?;
        let start = end
            .checked_sub(span)
            .map_err(|e| PceError::Invalid(format!("window too large: {e}")))?;
        Ok(Self::between(start, end))
    }

    pub fn source(mut self, and_group: Vec<ActorMatch>) -> Self {
        self.sources.include.push(and_group);
        self
    }

    pub fn destination(mut self, and_group: Vec<ActorMatch>) -> Self {
        self.destinations.include.push(and_group);
        self
    }

    pub fn exclude_source(mut self, m: ActorMatch) -> Self {
        self.sources.exclude.push(m);
        self
    }

    pub fn exclude_destination(mut self, m: ActorMatch) -> Self {
        self.destinations.exclude.push(m);
        self
    }

    pub fn service(mut self, m: ServiceMatch) -> Self {
        self.services.include.push(m);
        self
    }

    pub fn exclude_service(mut self, m: ServiceMatch) -> Self {
        self.services.exclude.push(m);
        self
    }

    pub fn decisions(mut self, d: Vec<PolicyDecision>) -> Self {
        if !d.is_empty() {
            self.decisions = d;
        }
        self
    }

    /// Only flows sent as one of `kinds` (empty, or all three: every flow).
    /// Explorer can only exclude a transmission, on the destination side,
    /// so `build` excludes the other kinds there.
    pub fn transmission(mut self, kinds: &[Transmission]) -> Self {
        self.transmission = kinds.to_vec();
        self
    }

    pub fn max_results(mut self, n: u32) -> Self {
        self.max_results = n;
        self
    }

    pub fn query_name(mut self, name: impl Into<String>) -> Self {
        self.query_name = name.into();
        self
    }

    pub fn build(mut self) -> Result<TrafficQuery> {
        if self.end <= self.start {
            return Err(PceError::Invalid(format!(
                "start ({}) must be before end ({})",
                format_timestamp(self.start),
                format_timestamp(self.end)
            )));
        }
        if self.start > Timestamp::now() {
            return Err(PceError::Invalid(format!(
                "start ({}) is in the future",
                format_timestamp(self.start)
            )));
        }
        if self.max_results == 0 {
            return Err(PceError::Invalid("max_results must be at least 1".into()));
        }
        if self.max_results > MAX_TRAFFIC_MAX_RESULTS {
            return Err(PceError::Invalid(format!(
                "max_results {} is above the maximum of {MAX_TRAFFIC_MAX_RESULTS}",
                self.max_results
            )));
        }
        if !self.transmission.is_empty() {
            for t in Transmission::ALL {
                if !self.transmission.contains(&t) {
                    self.destinations
                        .exclude
                        .push(ActorMatch::Transmission { transmission: t });
                }
            }
        }
        // The PCE returns nothing for `include: []`; `[[]]` means "no filter".
        self.sources.include.retain(|g| !g.is_empty());
        self.destinations.include.retain(|g| !g.is_empty());
        if self.sources.include.is_empty() {
            self.sources.include.push(Vec::new());
        }
        if self.destinations.include.is_empty() {
            self.destinations.include.push(Vec::new());
        }
        Ok(TrafficQuery {
            start_date: format_timestamp(self.start),
            end_date: format_timestamp(self.end),
            sources: self.sources,
            destinations: self.destinations,
            services: self.services,
            policy_decisions: self.decisions,
            exclude_workloads_from_ip_list_query: true,
            sources_destinations_query_op: "and".into(),
            max_results: self.max_results,
            query_name: self.query_name,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ActorMatch, PolicyDecision, Ref, ServiceMatch};
    use serde_json::json;

    fn ts(s: &str) -> jiff::Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn transmission_kinds_become_destination_excludes_of_the_others() {
        use crate::models::Transmission::{Broadcast, Multicast, Unicast};
        let q = |kinds: &[crate::models::Transmission]| {
            let q = TrafficQueryBuilder::between(
                ts("2026-10-01T00:00:00Z"),
                ts("2026-10-02T00:00:00Z"),
            )
            .exclude_destination(ActorMatch::IpAddress {
                ip_address: "10.0.0.1".into(),
            })
            .transmission(kinds)
            .build()
            .unwrap();
            serde_json::to_value(&q).unwrap()
        };
        assert_eq!(
            q(&[Unicast])["destinations"]["exclude"],
            json!([
                {"ip_address": "10.0.0.1"},
                {"transmission": "broadcast"},
                {"transmission": "multicast"}
            ]),
            "Explorer only excludes a transmission, on the destination side"
        );
        assert_eq!(
            q(&[Broadcast, Multicast])["destinations"]["exclude"],
            json!([{"ip_address": "10.0.0.1"}, {"transmission": "unicast"}])
        );
        for all in [
            &[][..],
            &[Unicast, Broadcast, Multicast],
            &[Unicast, Unicast, Broadcast, Multicast],
        ] {
            assert_eq!(
                q(all)["destinations"]["exclude"],
                json!([{"ip_address": "10.0.0.1"}]),
                "{all:?}: every kind, no filter"
            );
        }
        assert_eq!(q(&[Unicast])["sources"]["exclude"], json!([]));
        let unicast: crate::models::Transmission = "Unicast".parse().unwrap();
        assert_eq!(unicast, Unicast);
        assert!("anycast".parse::<crate::models::Transmission>().is_err());
        assert_eq!(Multicast.as_str(), "multicast");
    }

    #[test]
    fn empty_include_serialises_as_list_of_empty_list() {
        let q =
            TrafficQueryBuilder::between(ts("2026-10-01T00:00:00Z"), ts("2026-10-02T00:00:00Z"))
                .build()
                .unwrap();
        let v = serde_json::to_value(&q).unwrap();
        assert_eq!(v["sources"]["include"], json!([[]]));
        assert_eq!(v["destinations"]["include"], json!([[]]));
        assert_eq!(v["sources"]["exclude"], json!([]));
        assert_eq!(v["services"]["include"], json!([]));
        assert_eq!(
            v["policy_decisions"],
            json!(["allowed", "potentially_blocked", "blocked"])
        );
        assert_eq!(v["start_date"], "2026-10-01T00:00:00Z");
        assert_eq!(v["end_date"], "2026-10-02T00:00:00Z");
        assert_eq!(v["max_results"], 10_000);
        assert_eq!(v["sources_destinations_query_op"], "and");
        assert_eq!(v["exclude_workloads_from_ip_list_query"], true);
        assert_eq!(v["query_name"], "illumio-tui");
    }

    #[test]
    fn filters_serialise_in_pce_shape() {
        let q =
            TrafficQueryBuilder::between(ts("2026-10-01T00:00:00Z"), ts("2026-10-02T00:00:00Z"))
                .source(vec![
                    ActorMatch::Label {
                        label: Ref::new("/orgs/5/labels/3"),
                    },
                    ActorMatch::Label {
                        label: Ref::new("/orgs/5/labels/4"),
                    },
                ])
                .source(vec![ActorMatch::IpAddress {
                    ip_address: "10.0.0.1".into(),
                }])
                .exclude_destination(ActorMatch::IpList {
                    ip_list: Ref::new("/orgs/5/sec_policy/draft/ip_lists/20"),
                })
                .service(ServiceMatch {
                    port: Some(443),
                    proto: Some(6),
                    process_name: None,
                })
                .decisions(vec![PolicyDecision::Blocked])
                .max_results(50)
                .build()
                .unwrap();
        let v = serde_json::to_value(&q).unwrap();
        assert_eq!(
            v["sources"]["include"],
            json!([[{"label": {"href": "/orgs/5/labels/3"}}, {"label": {"href": "/orgs/5/labels/4"}}], [{"ip_address": "10.0.0.1"}]])
        );
        assert_eq!(
            v["destinations"]["exclude"],
            json!([{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/20"}}])
        );
        assert_eq!(v["services"]["include"], json!([{"port": 443, "proto": 6}]));
        assert_eq!(v["policy_decisions"], json!(["blocked"]));
        assert_eq!(v["max_results"], 50);
    }

    #[test]
    fn backwards_window_and_zero_results_are_rejected() {
        let r =
            TrafficQueryBuilder::between(ts("2026-10-02T00:00:00Z"), ts("2026-10-01T00:00:00Z"))
                .build();
        assert!(matches!(r, Err(crate::PceError::Invalid(m)) if m.contains("before")));
        let r =
            TrafficQueryBuilder::between(ts("2026-10-01T00:00:00Z"), ts("2026-10-02T00:00:00Z"))
                .max_results(0)
                .build();
        assert!(matches!(r, Err(crate::PceError::Invalid(_))));
    }

    #[test]
    fn caps_above_the_hard_maximum_are_rejected() {
        let b =
            TrafficQueryBuilder::between(ts("2026-10-01T00:00:00Z"), ts("2026-10-02T00:00:00Z"));
        let q = b.clone().max_results(200_000).build().unwrap();
        assert_eq!(q.max_results, 200_000);
        let r = b.max_results(200_001).build();
        assert!(
            matches!(r, Err(crate::PceError::Invalid(ref m)) if m.contains("200001") && m.contains("maximum of 200000")),
            "{r:?}"
        );
    }

    #[test]
    fn future_start_is_rejected() {
        let start = Timestamp::now().checked_add(Span::new().hours(24)).unwrap();
        let end = start.checked_add(Span::new().hours(1)).unwrap();
        let r = TrafficQueryBuilder::between(start, end).build();
        assert!(
            matches!(r, Err(crate::PceError::Invalid(ref m)) if m.contains("in the future")),
            "{r:?}"
        );
    }

    #[test]
    fn cost_and_confirmation() {
        let q =
            TrafficQueryBuilder::between(ts("2026-10-01T00:00:00Z"), ts("2026-10-09T00:00:00Z"))
                .build()
                .unwrap();
        let c = q.cost();
        assert_eq!(c.window, std::time::Duration::from_secs(8 * 86400));
        assert!(c.unbounded);
        assert!(q.needs_confirmation().unwrap().contains("8d"));
        let small =
            TrafficQueryBuilder::between(ts("2026-10-01T00:00:00Z"), ts("2026-10-02T00:00:00Z"))
                .source(vec![ActorMatch::IpAddress {
                    ip_address: "1.1.1.1".into(),
                }])
                .build()
                .unwrap();
        assert!(!small.cost().unbounded);
        assert!(small.needs_confirmation().is_none());
        let big =
            TrafficQueryBuilder::between(ts("2026-10-01T00:00:00Z"), ts("2026-10-02T00:00:00Z"))
                .max_results(20_000)
                .build()
                .unwrap();
        assert!(big.needs_confirmation().unwrap().contains("20000"));
    }

    #[test]
    fn windows_parse_and_last_uses_now() {
        assert_eq!(
            parse_window("24h").unwrap(),
            std::time::Duration::from_secs(86400)
        );
        assert_eq!(
            parse_window("7d").unwrap(),
            std::time::Duration::from_secs(7 * 86400)
        );
        assert_eq!(
            parse_window("90m").unwrap(),
            std::time::Duration::from_secs(5400)
        );
        assert!(parse_window("soon").is_err());
        let q = TrafficQueryBuilder::last(std::time::Duration::from_secs(3600))
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(q.cost().window, std::time::Duration::from_secs(3600));
        assert!(q.end_date.ends_with('Z'));
    }
}
