//! The traffic filter as the user edits it (strings), its validation and
//! cost hints (no network), and its conversion to a `TrafficQuery` (needs
//! the name index, so it is async and runs in the runtime, not in `update`).
pub use crate::filters::Terms;
use crate::filters::{or_segments, parse_terms};
use crate::tui::form::{Completing, Completion, FieldKind};
use pce::models::{PolicyDecision, TrafficQuery, Transmission};
use pce::traffic::{
    CONFIRM_MAX_RESULTS, CONFIRM_WINDOW, FilterExpr, MAX_TRAFFIC_MAX_RESULTS, ServiceExpr,
    TrafficQueryBuilder, parse_window, resolve_service,
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

fn parse_actor(e: &str) -> Result<FilterExpr, String> {
    FilterExpr::parse(e).map_err(|err| err.to_string())
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

    /// The source terms. Explorer takes no FQDN on the source side, to
    /// include or to exclude.
    pub fn src_terms(&self) -> Result<Terms<FilterExpr>, String> {
        let src = |e: &str| {
            let x = parse_actor(e)?;
            x.check_source().map_err(|err| err.to_string())?;
            Ok(x)
        };
        parse_terms(&self.src, src).map_err(|e| format!("source: {e}"))
    }

    pub fn dst_terms(&self) -> Result<Terms<FilterExpr>, String> {
        parse_terms(&self.dst, parse_actor).map_err(|e| format!("destination: {e}"))
    }

    /// `443/tcp`, `443`, `tcp` or `svc:NAME`, any of them `|` another, each
    /// with `!` to leave it out instead; empty means no service filter. A
    /// flow has one service, so a `,` group holds one service to include at
    /// most. `svc:NAME` resolves when the query is built (`build`).
    pub fn service_terms(&self) -> Result<Terms<ServiceExpr>, String> {
        let terms =
            parse_terms(&self.service, ServiceExpr::parse).map_err(|e| format!("service: {e}"))?;
        if terms.include.iter().any(|g| g.len() > 1) {
            return Err(
                "service: a flow has one service; use | between services (443/tcp | 80/tcp)".into(),
            );
        }
        Ok(terms)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.window_duration()?;
        self.src_terms()?;
        self.dst_terms()?;
        self.service_terms()?;
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

    /// No source, destination or service to include, judged from the parsed
    /// terms so `,` or ` | ` counts as empty. Only `!` terms still ask for
    /// every flow but a few: unbounded. Unparseable text is not unbounded
    /// (it fails validation instead).
    pub fn is_unbounded(&self) -> bool {
        self.src_terms().is_ok_and(|t| t.include.is_empty())
            && self.dst_terms().is_ok_and(|t| t.include.is_empty())
            && self.service_terms().is_ok_and(|t| t.include.is_empty())
    }

    /// The editor's note on `!` terms, `None` without any: Explorer's
    /// exclude list is flat, so a `!` in one `|` group leaves its match out
    /// of the whole field, and a field of `!` terms only keeps every flow
    /// but those.
    pub fn exclude_note(&self) -> Option<&'static str> {
        // (`|` segments with a term, include groups) per field that parses
        // and has a `!` term.
        let shape = |text: &String, include: usize, exclude: usize| {
            (exclude > 0).then(|| (or_segments(text), include))
        };
        let excluding: Vec<(usize, usize)> = [
            self.src_terms()
                .ok()
                .and_then(|t| shape(&self.src, t.include.len(), t.exclude.len())),
            self.dst_terms()
                .ok()
                .and_then(|t| shape(&self.dst, t.include.len(), t.exclude.len())),
            self.service_terms()
                .ok()
                .and_then(|t| shape(&self.service, t.include.len(), t.exclude.len())),
        ]
        .into_iter()
        .flatten()
        .collect();
        if excluding.iter().any(|(segments, _)| *segments > 1) {
            Some("a ! term leaves its match out of the whole field, not one | group")
        } else if excluding.iter().any(|(_, inc)| *inc == 0) {
            Some("a field of ! terms only: every flow except those")
        } else if !excluding.is_empty() {
            Some("! leaves its match out of the whole field")
        } else {
            None
        }
    }

    /// What narrows the flows besides the window and the cap (`svc 443,
    /// decision blocked`); `None` when every flow of the window is asked.
    pub fn narrowing(&self) -> Option<String> {
        let mut parts = Vec::new();
        for (name, text) in [
            ("src", &self.src),
            ("dst", &self.dst),
            ("svc", &self.service),
        ] {
            if !text.trim().is_empty() {
                parts.push(format!("{name} {}", text.trim()));
            }
        }
        if self.decisions_text() != "all" {
            parts.push(format!("decision {}", self.decisions_text()));
        }
        if self.has_transmission() {
            parts.push("transmission".to_string());
        }
        (!parts.is_empty()).then(|| parts.join(", "))
    }

    /// [`Self::narrowing`] as readiness sees it: readiness counts unicast
    /// flows only, so a transmission filter that keeps unicast (`:unicast`)
    /// narrows nothing it reads.
    pub fn readiness_narrowing(&self) -> Option<String> {
        if self.transmission.contains(&Transmission::Unicast) {
            TrafficFilter {
                transmission: Vec::new(),
                ..self.clone()
            }
            .narrowing()
        } else {
            self.narrowing()
        }
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
        // A narrowed transmission (the default is every kind), where the
        // filter bar's title shows it whole even at 80 columns.
        if self.has_transmission() {
            s.push_str(&format!(" · {}", self.transmission_text()));
        }
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

    /// `to_query` over exactly `[start, end)` instead of the window ending
    /// now: compare mode's baseline.
    pub async fn to_query_between(
        &self,
        start: jiff::Timestamp,
        end: jiff::Timestamp,
        pce: &Pce,
        mode: CacheMode,
    ) -> pce::Result<TrafficQuery> {
        self.build(TrafficQueryBuilder::between(start, end), pce, mode)
            .await
    }

    async fn build(
        &self,
        mut b: TrafficQueryBuilder,
        pce: &Pce,
        mode: CacheMode,
    ) -> pce::Result<TrafficQuery> {
        let src = self.src_terms().map_err(pce::PceError::Invalid)?;
        let dst = self.dst_terms().map_err(pce::PceError::Invalid)?;
        let svc = self.service_terms().map_err(pce::PceError::Invalid)?;
        // One resolution for every group: the include groups of both sides,
        // then each side's excludes as one more group.
        let groups: Vec<Vec<FilterExpr>> = src
            .include
            .iter()
            .chain(&dst.include)
            .cloned()
            .chain([src.exclude, dst.exclude])
            .collect();
        let mut resolved = pce.resolve_filter_groups(&groups, mode).await?.into_iter();
        for g in resolved.by_ref().take(src.include.len()) {
            b = b.source(g);
        }
        for g in resolved.by_ref().take(dst.include.len()) {
            b = b.destination(g);
        }
        for m in resolved.next().unwrap_or_default() {
            b = b.exclude_source(m);
        }
        for m in resolved.next().unwrap_or_default() {
            b = b.exclude_destination(m);
        }
        // `svc:NAME` reads the cached index (the one the actors used); each
        // of a service's ports is one more alternative, or one more exclude.
        let named = svc
            .include
            .iter()
            .flatten()
            .chain(&svc.exclude)
            .any(ServiceExpr::is_named);
        let idx = if named {
            Some(pce.name_index(mode).await?)
        } else {
            None
        };
        let services = idx.as_ref().map(|i| i.services()).unwrap_or_default();
        let resolve = |e: &ServiceExpr| {
            resolve_service(e, &services)
                .map_err(|err| pce::PceError::Invalid(format!("service: {err}")))
        };
        for e in svc.include.iter().flatten() {
            for m in resolve(e)? {
                b = b.service(m);
            }
        }
        for e in &svc.exclude {
            for m in resolve(e)? {
                b = b.exclude_service(m);
            }
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
    FieldKind::TrafficSource,
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
    /// Ctrl-U: empties the active field.
    pub fn clear(&mut self) {
        self.values[self.active].clear();
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
    use pce::models::{PolicyDecision, ServiceMatch};
    use pce::traffic::FilterExpr;

    #[test]
    fn narrowing_names_what_is_not_window_or_cap() {
        let mut f = TrafficFilter::defaults("24h", 1000);
        assert_eq!(f.narrowing(), None);
        f.service = "443".into();
        f.decisions = vec![PolicyDecision::Blocked];
        assert_eq!(
            f.narrowing().unwrap(),
            format!("svc 443, decision {}", f.decisions_text())
        );
    }

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
        let g = f.src_terms().unwrap().include;
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
            f.dst_terms().unwrap().include,
            vec![vec![FilterExpr::Ip("10.0.0.5".into())]]
        );
        assert!(!f.is_unbounded());
        f.src = "justaword".into();
        assert!(f.src_terms().unwrap_err().contains("cannot parse"));
    }

    #[test]
    fn separator_only_text_is_unbounded() {
        let mut f = TrafficFilter::defaults("24h", 1000);
        for text in [",", " | ", ", | ,", "|"] {
            f.src = text.into();
            f.dst = text.into();
            assert!(f.src_terms().unwrap().is_empty());
            assert!(f.is_unbounded(), "{text:?}");
            assert!(f.cost_hint().ends_with("unbounded"), "{text:?}");
        }
        f.src = "app=web | ,".into();
        assert_eq!(
            f.src_terms().unwrap().include.len(),
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
        assert_eq!(
            back.cost_hint(),
            "window 24h · cap 1000 · unicast · unbounded"
        );
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
        let m = |port, proto| {
            ServiceExpr::Port(ServiceMatch {
                port,
                to_port: None,
                proto,
                process_name: None,
            })
        };
        let mut f = TrafficFilter::defaults("1h", 50);
        assert!(f.service_terms().unwrap().is_empty());
        f.service = "443/tcp".into();
        assert_eq!(
            f.service_terms().unwrap().include,
            vec![vec![m(Some(443), Some(6))]]
        );
        f.service = "53".into();
        assert_eq!(
            f.service_terms().unwrap().include,
            vec![vec![m(Some(53), None)]]
        );
        f.service = "udp".into();
        assert_eq!(
            f.service_terms().unwrap().include,
            vec![vec![m(None, Some(17))]]
        );
        f.service = "443/tcp | 80/tcp".into();
        assert_eq!(f.service_terms().unwrap().include.len(), 2, "| is any of");
        f.service = "443/tcp, 80/tcp".into();
        assert!(
            f.service_terms()
                .unwrap_err()
                .contains("a flow has one service")
        );
        f.service = "8000-8100/tcp | !8443-8444/udp".into();
        let t = f.service_terms().unwrap();
        let range = |from, to, proto| {
            ServiceExpr::Port(ServiceMatch {
                port: Some(from),
                to_port: Some(to),
                proto: Some(proto),
                process_name: None,
            })
        };
        assert_eq!(t.include, vec![vec![range(8000, 8100, 6)]]);
        assert_eq!(t.exclude, vec![range(8443, 8444, 17)]);
        f.service = "8000-8100".into();
        assert!(f.service_terms().unwrap_err().contains("needs tcp or udp"));
        f.service = "udp/53".into();
        assert!(f.service_terms().unwrap_err().starts_with("service: "));
        f.service = "99999".into();
        assert!(f.service_terms().is_err());
    }

    #[test]
    fn bang_terms_exclude_in_each_field() {
        let label = |k: &str, v: &str| FilterExpr::Label {
            key: k.into(),
            value: v.into(),
        };
        let mut f = TrafficFilter::defaults("1h", 50);
        f.src = "env=prod, !app=monitoring".into();
        f.dst = "! ipl:Corp, 10.0.0.0/8".into();
        f.service = "tcp, !22/tcp".into();
        let src = f.src_terms().unwrap();
        assert_eq!(src.include, vec![vec![label("env", "prod")]]);
        assert_eq!(src.exclude, vec![label("app", "monitoring")]);
        let dst = f.dst_terms().unwrap();
        assert_eq!(dst.include, vec![vec![FilterExpr::Ip("10.0.0.0/8".into())]]);
        assert_eq!(dst.exclude, vec![FilterExpr::IpList("Corp".into())]);
        let svc = f.service_terms().unwrap();
        assert_eq!(
            svc.exclude,
            vec![ServiceExpr::Port(ServiceMatch {
                port: Some(22),
                to_port: None,
                proto: Some(6),
                process_name: None
            })]
        );
        assert_eq!(svc.include.len(), 1, "one service plus an exclusion");
        assert!(f.validate().is_ok());
        assert!(f.summary().contains("src env=prod, !app=monitoring"));
        assert!(f.narrowing().unwrap().contains("svc tcp, !22/tcp"));
        assert_eq!(
            f.exclude_note(),
            Some("! leaves its match out of the whole field")
        );
    }

    #[test]
    fn a_field_of_bang_terms_only_means_everything_except() {
        let mut f = TrafficFilter::defaults("1h", 50);
        f.dst = "!app=monitoring, !wl:host-1.example".into();
        let dst = f.dst_terms().unwrap();
        assert!(dst.include.is_empty(), "no include group: every flow");
        assert_eq!(dst.exclude.len(), 2);
        assert!(f.is_unbounded(), "only exclusions: every flow but a few");
        assert!(f.cost_hint().ends_with(" · unbounded"));
        assert_eq!(
            f.exclude_note(),
            Some("a field of ! terms only: every flow except those")
        );
        f.dst.clear();
        f.service = "!udp | !svc:SSH".into();
        assert!(f.is_unbounded());
        assert!(f.service_terms().unwrap().include.is_empty());
        f.service = "tcp, !22/tcp".into();
        assert!(!f.is_unbounded(), "an include narrows");
        f.service.clear();
        f.src = "app=web, !env=dev".into();
        assert!(!f.is_unbounded());
    }

    #[test]
    fn a_bang_inside_a_term_asks_for_a_comma() {
        let mut f = TrafficFilter::defaults("1h", 50);
        for text in ["env=prod !app=monitoring", "!env=prod !app=x", "app=a!b"] {
            f.src = text.into();
            let err = f.src_terms().unwrap_err();
            assert!(err.contains("use , before !"), "{text:?}: {err}");
        }
        f.service = "tcp !22/tcp".into();
        assert!(f.service_terms().unwrap_err().contains("use , before !"));
        f.src = "env=prod, !app=monitoring".into();
        assert!(f.src_terms().is_ok());
    }

    #[test]
    fn services_by_name_parse_offline() {
        let mut f = TrafficFilter::defaults("1h", 50);
        f.service = "svc:PostgreSQL | 443/tcp, !svc:SSH".into();
        let t = f.service_terms().unwrap();
        assert_eq!(
            t.include[0],
            vec![ServiceExpr::Named("PostgreSQL".into())],
            "resolved when the query is built"
        );
        assert_eq!(t.exclude, vec![ServiceExpr::Named("SSH".into())]);
        f.service = "svc:".into();
        assert!(f.validate().unwrap_err().contains("needs a service name"));
    }

    #[test]
    fn a_bang_in_an_or_group_excludes_from_the_whole_field() {
        let mut f = TrafficFilter::defaults("1h", 50);
        f.src = "app=web | app=db, !env=dev".into();
        let src = f.src_terms().unwrap();
        assert_eq!(src.include.len(), 2);
        assert_eq!(
            src.exclude,
            vec![FilterExpr::Label {
                key: "env".into(),
                value: "dev".into()
            }],
            "flat: not part of the app=db group"
        );
        assert_eq!(
            f.exclude_note(),
            Some("a ! term leaves its match out of the whole field, not one | group")
        );
        f.src = "app=web | !env=dev".into();
        assert_eq!(f.src_terms().unwrap().include.len(), 1);
        assert!(f.exclude_note().unwrap().contains("not one | group"));
        f.src = "app=web, !env=dev | ,".into();
        assert_eq!(
            f.exclude_note(),
            Some("! leaves its match out of the whole field"),
            "an empty | segment is no OR group"
        );
        f.src = "app=web".into();
        assert_eq!(f.exclude_note(), None);
    }

    #[test]
    fn bad_bang_terms_fail_validation() {
        let mut f = TrafficFilter::defaults("1h", 50);
        for (field, text, want) in [
            ("src", "!", "source: ! needs a term after it"),
            ("src", "app=web, ! ", "source: ! needs a term after it"),
            ("dst", "!!app=web", "one ! per term"),
            ("dst", "!justaword", "cannot parse"),
            ("src", "!fqdn:x.example", "destination filter only"),
            ("svc", "!", "service: ! needs a term after it"),
            (
                "svc",
                "!ssh",
                "service: use PORT, PROTO, PORT/PROTO or FROM-TO/PROTO",
            ),
            ("svc", "!22/xyz", "unknown protocol"),
        ] {
            let mut g = f.clone();
            match field {
                "src" => g.src = text.into(),
                "dst" => g.dst = text.into(),
                _ => g.service = text.into(),
            }
            let err = g.validate().unwrap_err();
            assert!(err.contains(want), "{field} {text:?}: {err}");
            let mut e = FilterEditor::from_filter(&f);
            e.values[match field {
                "src" => 1,
                "dst" => 2,
                _ => 3,
            }] = text.into();
            assert_eq!(e.to_filter().unwrap_err(), err, "the form says the same");
        }
        f.dst = "!fqdn:x.example".into();
        assert!(f.validate().is_ok(), "an FQDN is a destination exclude");
        f.src = "fqdn:x.example".into();
        assert!(
            f.validate()
                .unwrap_err()
                .contains("destination filter only"),
            "nor a source include"
        );
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
