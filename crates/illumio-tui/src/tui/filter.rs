//! The traffic filter as the user edits it (strings), its validation and
//! cost hints (no network), and its conversion to a `TrafficQuery` (needs
//! the name index, so it is async and runs in the runtime, not in `update`).
use crate::tui::form::{Completing, Completion, FieldKind};
use pce::models::{PolicyDecision, ServiceMatch, TrafficQuery, Transmission, proto_number};
use pce::traffic::{
    CONFIRM_MAX_RESULTS, CONFIRM_WINDOW, FilterExpr, MAX_TRAFFIC_MAX_RESULTS, TrafficQueryBuilder,
    parse_window,
};
use pce::{CacheMode, Pce};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrafficFilter {
    pub window: String,
    pub src: String,
    pub dst: String,
    pub service: String,
    pub decisions: Vec<PolicyDecision>,
    /// The transmission kinds kept; empty: every kind (the default).
    pub transmission: Vec<Transmission>,
    pub max_results: u32,
}

/// `a,b | c` is (a AND b) OR c. Empty expressions and groups that end up empty
/// (`,` or ` | `) are dropped, so blank-ish text parses to no groups at all.
fn parse_groups(text: &str) -> Result<Vec<Vec<FilterExpr>>, String> {
    let mut groups = Vec::new();
    for g in text.split('|') {
        let group = g
            .split(',')
            .map(str::trim)
            .filter(|e| !e.is_empty())
            .map(|e| FilterExpr::parse(e).map_err(|err| err.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        if !group.is_empty() {
            groups.push(group);
        }
    }
    Ok(groups)
}

/// `all` when every decision is selected (or none, which the PCE treats the same),
/// else the list in order.
fn decisions_text(decisions: &[PolicyDecision]) -> String {
    if decisions.is_empty() || PolicyDecision::ALL.iter().all(|d| decisions.contains(d)) {
        "all".to_string()
    } else {
        decisions
            .iter()
            .map(|d| d.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }
}

impl TrafficFilter {
    pub fn defaults(window: &str, max_results: u32) -> Self {
        Self {
            window: window.to_string(),
            src: String::new(),
            dst: String::new(),
            service: String::new(),
            decisions: PolicyDecision::ALL.to_vec(),
            transmission: Vec::new(),
            max_results,
        }
    }

    pub fn window_duration(&self) -> Result<Duration, String> {
        parse_window(&self.window).map_err(|e| format!("window: {e}"))
    }

    pub fn src_groups(&self) -> Result<Vec<Vec<FilterExpr>>, String> {
        parse_groups(&self.src).map_err(|e| format!("source: {e}"))
    }

    pub fn dst_groups(&self) -> Result<Vec<Vec<FilterExpr>>, String> {
        parse_groups(&self.dst).map_err(|e| format!("destination: {e}"))
    }

    /// `443/tcp`, `443`, `tcp`; empty means no service filter.
    pub fn service_match(&self) -> Result<Option<ServiceMatch>, String> {
        let s = self.service.trim();
        if s.is_empty() {
            return Ok(None);
        }
        let (port_s, proto_s) = match s.split_once('/') {
            Some((p, pr)) => (p.trim(), Some(pr.trim())),
            None => (s, None),
        };
        let mut m = ServiceMatch::default();
        if let Some(pr) = proto_s {
            m.proto =
                Some(proto_number(pr).ok_or_else(|| format!("service: unknown protocol {pr:?}"))?);
            m.port = Some(
                port_s
                    .parse::<u16>()
                    .map_err(|_| format!("service: bad port {port_s:?}"))?,
            );
        } else if let Ok(p) = port_s.parse::<u16>() {
            m.port = Some(p);
        } else if let Some(n) = proto_number(port_s) {
            m.proto = Some(n);
        } else {
            return Err(format!("service: use PORT, PROTO or PORT/PROTO, got {s:?}"));
        }
        Ok(Some(m))
    }

    pub fn validate(&self) -> Result<(), String> {
        self.window_duration()?;
        self.src_groups()?;
        self.dst_groups()?;
        self.service_match()?;
        if self.max_results == 0 {
            return Err("max results must be at least 1".into());
        }
        if self.max_results > MAX_TRAFFIC_MAX_RESULTS {
            return Err(format!(
                "max results {} is above the maximum of {MAX_TRAFFIC_MAX_RESULTS}",
                self.max_results
            ));
        }
        Ok(())
    }

    /// No source, destination or service restriction, judged from the parsed groups
    /// so `,` or ` | ` counts as empty. Unparseable text is not unbounded (it fails
    /// validation instead).
    pub fn is_unbounded(&self) -> bool {
        let empty = |g: Result<Vec<Vec<FilterExpr>>, String>| g.is_ok_and(|g| g.is_empty());
        empty(self.src_groups()) && empty(self.dst_groups()) && self.service.trim().is_empty()
    }

    /// The decisions as shown in the summary, the filter bar and the editor.
    pub fn decisions_text(&self) -> String {
        decisions_text(&self.decisions)
    }

    /// Whether the transmission is narrowed (not every kind).
    pub fn has_transmission(&self) -> bool {
        !self.transmission.is_empty()
            && !Transmission::ALL
                .iter()
                .all(|t| self.transmission.contains(t))
    }

    /// `all`, or the kinds kept, as the editor and the filter bar show them.
    pub fn transmission_text(&self) -> String {
        if self.has_transmission() {
            self.transmission
                .iter()
                .map(|t| t.as_str())
                .collect::<Vec<_>>()
                .join(",")
        } else {
            "all".into()
        }
    }

    /// `:unicast`: unicast only, or back to every kind when it already is.
    pub fn toggle_unicast(&mut self) {
        self.transmission = if self.transmission == [Transmission::Unicast] {
            Vec::new()
        } else {
            vec![Transmission::Unicast]
        };
    }

    pub fn cost_hint(&self) -> String {
        let mut s = format!("window {} · cap {}", self.window, self.max_results);
        if self.is_unbounded() {
            s.push_str(" · unbounded");
        }
        s
    }

    pub fn needs_confirmation(&self) -> Option<String> {
        let mut reasons = Vec::new();
        if let Ok(w) = self.window_duration()
            && w > CONFIRM_WINDOW
        {
            reasons.push(format!(
                "window {} exceeds {}d",
                self.window,
                CONFIRM_WINDOW.as_secs() / 86400
            ));
        }
        if self.max_results > CONFIRM_MAX_RESULTS {
            reasons.push(format!(
                "cap {} exceeds {}",
                self.max_results, CONFIRM_MAX_RESULTS
            ));
        }
        if reasons.is_empty() {
            None
        } else {
            Some(reasons.join("; "))
        }
    }

    pub fn summary(&self) -> String {
        let or = |s: &str| {
            if s.trim().is_empty() {
                "*".to_string()
            } else {
                s.trim().to_string()
            }
        };
        let decision = self.decisions_text();
        let mut s = format!(
            "window {}  src {}  dst {}  svc {}  decision {}",
            self.window,
            or(&self.src),
            or(&self.dst),
            or(&self.service),
            decision
        );
        if self.has_transmission() {
            s.push_str(&format!("  transmission {}", self.transmission_text()));
        }
        s
    }

    /// Resolves names through the facade's cached index and builds the query
    /// for the last `window`.
    pub async fn to_query(&self, pce: &Pce, mode: CacheMode) -> pce::Result<TrafficQuery> {
        let window = self.window_duration().map_err(pce::PceError::Invalid)?;
        self.build(TrafficQueryBuilder::last(window)?, pce, mode)
            .await
    }

    async fn build(
        &self,
        mut b: TrafficQueryBuilder,
        pce: &Pce,
        mode: CacheMode,
    ) -> pce::Result<TrafficQuery> {
        let src = self.src_groups().map_err(pce::PceError::Invalid)?;
        let dst = self.dst_groups().map_err(pce::PceError::Invalid)?;
        for g in pce.resolve_filter_groups(&src, mode).await? {
            b = b.source(g);
        }
        for g in pce.resolve_filter_groups(&dst, mode).await? {
            b = b.destination(g);
        }
        if let Some(m) = self.service_match().map_err(pce::PceError::Invalid)? {
            b = b.service(m);
        }
        b.decisions(self.decisions.clone())
            .transmission(&self.transmission)
            .max_results(self.max_results)
            .build()
    }
}

pub const FIELD_NAMES: [&str; 7] = [
    "window",
    "source",
    "destination",
    "service",
    "decisions",
    "transmission",
    "max results",
];

/// What `Ctrl-N` completes in each field, in the order of [`FIELD_NAMES`].
pub const FIELD_KINDS: [FieldKind; 7] = [
    FieldKind::Window,
    FieldKind::TrafficActors,
    FieldKind::TrafficActors,
    FieldKind::TrafficService,
    FieldKind::Decisions,
    FieldKind::Transmission,
    FieldKind::Text,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterEditor {
    pub values: [String; 7],
    pub active: usize,
    pub error: Option<String>,
    /// The completion being cycled; any key but `Ctrl-N`/`Ctrl-P` ends it.
    pub completion: Option<Box<Completion>>,
}

impl FilterEditor {
    pub fn from_filter(f: &TrafficFilter) -> Self {
        let decisions = f.decisions_text();
        Self {
            values: [
                f.window.clone(),
                f.src.clone(),
                f.dst.clone(),
                f.service.clone(),
                decisions,
                f.transmission_text(),
                f.max_results.to_string(),
            ],
            active: 0,
            error: None,
            completion: None,
        }
    }

    pub fn to_filter(&self) -> Result<TrafficFilter, String> {
        let decisions_text = self.values[4].trim();
        let decisions = if decisions_text.is_empty() || decisions_text == "all" {
            PolicyDecision::ALL.to_vec()
        } else {
            let mut out: Vec<PolicyDecision> = Vec::new();
            for d in decisions_text.split(',').map(str::trim) {
                let d = PolicyDecision::parse(d).ok_or_else(|| {
                    format!("decision: unknown {d:?} (allowed, potentially_blocked, blocked)")
                })?;
                if !out.contains(&d) {
                    out.push(d);
                }
            }
            out
        };
        let transmission_text = self.values[5].trim();
        let mut transmission: Vec<Transmission> = Vec::new();
        if !transmission_text.is_empty() && transmission_text != "all" {
            for t in transmission_text.split(',').map(str::trim) {
                let t: Transmission = t.parse().map_err(|e| format!("transmission: {e}"))?;
                if !transmission.contains(&t) {
                    transmission.push(t);
                }
            }
        }
        if Transmission::ALL.iter().all(|t| transmission.contains(t)) {
            transmission.clear();
        }
        let max_results = self.values[6]
            .trim()
            .parse::<u32>()
            .map_err(|_| format!("max results: not a number {:?}", self.values[6]))?;
        let f = TrafficFilter {
            window: self.values[0].trim().to_string(),
            src: self.values[1].trim().to_string(),
            dst: self.values[2].trim().to_string(),
            service: self.values[3].trim().to_string(),
            decisions,
            transmission,
            max_results,
        };
        f.validate()?;
        Ok(f)
    }

    pub fn next(&mut self) {
        self.active = (self.active + 1).min(FIELD_NAMES.len() - 1);
    }
    pub fn prev(&mut self) {
        self.active = self.active.saturating_sub(1);
    }
    pub fn push(&mut self, c: char) {
        self.values[self.active].push(c);
        self.error = None;
    }
    pub fn pop(&mut self) {
        self.values[self.active].pop();
        self.error = None;
    }

    /// Up to five candidates around the shown one, then `+N more`.
    pub fn completion_strip(&self) -> Option<Vec<(String, bool)>> {
        self.completion.as_ref().map(|c| c.strip())
    }

    /// Ends an open completion (`true`); `restore` puts the typed token back.
    pub fn end_completion(&mut self, restore: bool) -> bool {
        let Some(c) = self.completion.take() else {
            return false;
        };
        if restore && let Some(v) = self.values.get_mut(c.field) {
            *v = c.typed();
        }
        true
    }
}

impl Completing for FilterEditor {
    fn completion_request(&self) -> Option<(FieldKind, String, Vec<String>)> {
        let kind = *FIELD_KINDS.get(self.active)?;
        let (token, earlier) = kind.request(&self.values[self.active])?;
        Some((kind, token, earlier))
    }

    fn start_completion(&mut self, candidates: Vec<String>) -> bool {
        let active = self.active;
        let (Some(&kind), Some(value)) = (FIELD_KINDS.get(active), self.values.get_mut(active))
        else {
            return false;
        };
        match Completion::open(self.completion.as_deref(), active, kind, value, candidates) {
            Some(c) => {
                self.completion = Some(Box::new(c));
                self.error = None;
                true
            }
            None => false,
        }
    }

    fn cycle(&mut self, forward: bool) {
        let Some(c) = self.completion.as_mut() else {
            return;
        };
        c.step(forward);
        if let Some(v) = self.values.get_mut(c.field) {
            *v = c.shown();
        }
    }

    fn completing(&self) -> bool {
        self.completion.is_some()
    }

    fn completion_dives(&self) -> bool {
        self.completion
            .as_ref()
            .is_some_and(|c| FIELD_KINDS.get(c.field).is_some_and(|&k| c.dives(k)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pce::models::PolicyDecision;
    use pce::traffic::FilterExpr;

    #[test]
    fn defaults_and_cost_hint() {
        let f = TrafficFilter::defaults("24h", 1000);
        assert_eq!(f.window, "24h");
        assert!(f.is_unbounded());
        assert_eq!(f.cost_hint(), "window 24h · cap 1000 · unbounded");
        assert!(f.needs_confirmation().is_none());
        assert_eq!(f.summary(), "window 24h  src *  dst *  svc *  decision all");
    }

    #[test]
    fn groups_parse_or_and() {
        let mut f = TrafficFilter::defaults("1h", 50);
        f.src = "app=web,env=prod | ipl:Corp".into();
        let g = f.src_groups().unwrap();
        assert_eq!(
            g,
            vec![
                vec![
                    FilterExpr::Label {
                        key: "app".into(),
                        value: "web".into()
                    },
                    FilterExpr::Label {
                        key: "env".into(),
                        value: "prod".into()
                    }
                ],
                vec![FilterExpr::IpList("Corp".into())]
            ]
        );
        f.dst = "10.0.0.5".into();
        assert_eq!(
            f.dst_groups().unwrap(),
            vec![vec![FilterExpr::Ip("10.0.0.5".into())]]
        );
        assert!(!f.is_unbounded());
        f.src = "justaword".into();
        assert!(f.src_groups().unwrap_err().contains("cannot parse"));
    }

    #[test]
    fn separator_only_text_is_unbounded() {
        let mut f = TrafficFilter::defaults("24h", 1000);
        for text in [",", " | ", ", | ,", "|"] {
            f.src = text.into();
            f.dst = text.into();
            assert_eq!(f.src_groups().unwrap(), Vec::<Vec<FilterExpr>>::new());
            assert!(f.is_unbounded(), "{text:?}");
            assert!(f.cost_hint().ends_with("unbounded"), "{text:?}");
        }
        f.src = "app=web | ,".into();
        assert_eq!(
            f.src_groups().unwrap().len(),
            1,
            "the empty group is dropped"
        );
        assert!(!f.is_unbounded());
        f.src = "justaword".into();
        assert!(!f.is_unbounded(), "unparseable text is not unbounded");
    }

    #[test]
    fn duplicate_decisions_are_deduplicated_and_not_all() {
        let mut e = FilterEditor::from_filter(&TrafficFilter::defaults("24h", 1000));
        assert_eq!(e.values[4], "all");
        e.values[4] = "allowed,blocked,allowed".into();
        let f = e.to_filter().unwrap();
        assert_eq!(
            f.decisions,
            vec![PolicyDecision::Allowed, PolicyDecision::Blocked]
        );
        assert_eq!(f.decisions_text(), "allowed,blocked");
        assert!(f.summary().ends_with("decision allowed,blocked"));
        assert_eq!(FilterEditor::from_filter(&f).values[4], "allowed,blocked");
        // A hand-built filter with duplicates still is not `all`.
        let mut dup = f.clone();
        dup.decisions.push(PolicyDecision::Allowed);
        assert_eq!(dup.decisions_text(), "allowed,blocked,allowed");
        e.values[4] = "blocked, potentially_blocked, allowed".into();
        assert_eq!(e.to_filter().unwrap().decisions_text(), "all");
    }

    #[test]
    fn transmission_round_trips_through_the_editor_and_shows_only_when_set() {
        use pce::models::Transmission::{Broadcast, Multicast, Unicast};
        let mut f = TrafficFilter::defaults("24h", 1000);
        assert!(f.transmission.is_empty(), "the default is every kind");
        assert_eq!(f.transmission_text(), "all");
        assert!(!f.summary().contains("transmission"));
        let mut e = FilterEditor::from_filter(&f);
        assert_eq!(e.values[5], "all");
        e.values[5] = "unicast".into();
        let back = e.to_filter().unwrap();
        assert_eq!(back.transmission, vec![Unicast]);
        assert!(back.summary().ends_with("  transmission unicast"));
        assert_eq!(FilterEditor::from_filter(&back).values[5], "unicast");
        e.values[5] = "Broadcast, multicast, broadcast".into();
        assert_eq!(
            e.to_filter().unwrap().transmission,
            vec![Broadcast, Multicast]
        );
        for all in ["", "all", "unicast,broadcast,multicast"] {
            e.values[5] = all.into();
            assert!(e.to_filter().unwrap().transmission.is_empty(), "{all:?}");
        }
        e.values[5] = "anycast".into();
        assert!(e.to_filter().unwrap_err().contains("transmission"));
        f.toggle_unicast();
        assert_eq!(f.transmission, vec![Unicast]);
        f.toggle_unicast();
        assert!(f.transmission.is_empty());
        f.transmission = vec![Broadcast];
        f.toggle_unicast();
        assert_eq!(f.transmission, vec![Unicast], "any other choice: unicast");
    }

    #[test]
    fn service_forms() {
        let mut f = TrafficFilter::defaults("1h", 50);
        assert_eq!(f.service_match().unwrap(), None);
        f.service = "443/tcp".into();
        assert_eq!(
            f.service_match().unwrap(),
            Some(ServiceMatch {
                port: Some(443),
                proto: Some(6),
                process_name: None
            })
        );
        f.service = "53".into();
        assert_eq!(
            f.service_match().unwrap(),
            Some(ServiceMatch {
                port: Some(53),
                proto: None,
                process_name: None
            })
        );
        f.service = "udp".into();
        assert_eq!(
            f.service_match().unwrap(),
            Some(ServiceMatch {
                port: None,
                proto: Some(17),
                process_name: None
            })
        );
        f.service = "udp/53".into();
        assert!(f.service_match().is_err());
        f.service = "99999".into();
        assert!(f.service_match().is_err());
    }

    #[test]
    fn confirmation_thresholds() {
        let mut f = TrafficFilter::defaults("8d", 1000);
        assert!(f.needs_confirmation().unwrap().contains("8d"));
        f.window = "1h".into();
        f.max_results = 20_000;
        assert!(f.needs_confirmation().unwrap().contains("20000"));
        f.max_results = 200_000;
        assert!(f.validate().is_ok());
        f.max_results = 200_001;
        assert!(f.validate().unwrap_err().contains("maximum of 200000"));
        f.max_results = 20_000;
        f.window = "soon".into();
        assert!(f.validate().unwrap_err().contains("window"));
    }

    #[test]
    fn editor_round_trips_and_reports_errors() {
        let mut f = TrafficFilter::defaults("24h", 1000);
        f.decisions = vec![PolicyDecision::Blocked];
        let mut e = FilterEditor::from_filter(&f);
        assert_eq!(e.values[4], "blocked");
        assert_eq!(e.active, 0);
        e.next();
        e.next();
        assert_eq!(e.active, 2);
        for c in "app=db".chars() {
            e.push(c);
        }
        e.pop();
        e.push('b');
        let back = e.to_filter().unwrap();
        assert_eq!(back.dst, "app=db");
        assert_eq!(back.decisions, vec![PolicyDecision::Blocked]);
        e.active = 6;
        e.values[6] = "x".into();
        assert!(e.to_filter().unwrap_err().contains("max results"));
        e.values[6] = "500".into();
        e.values[4] = "allowed, maybe".into();
        assert!(e.to_filter().unwrap_err().contains("decision"));
        for _ in 0..7 {
            e.prev();
        }
        assert_eq!(e.active, 0, "prev clamps at the first field");
    }
}
