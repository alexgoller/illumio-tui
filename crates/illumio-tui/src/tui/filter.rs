//! The traffic filter as the user edits it (strings), its validation and
//! cost hints (no network), and its conversion to a `TrafficQuery` (needs
//! the name index, so it is async and runs in the runtime, not in `update`).
use pce::models::{PolicyDecision, ServiceMatch, TrafficQuery, proto_number};
use pce::traffic::{
    CONFIRM_MAX_RESULTS, CONFIRM_WINDOW, FilterExpr, TrafficQueryBuilder, parse_window,
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
    pub max_results: u32,
}

fn parse_groups(text: &str) -> Result<Vec<Vec<FilterExpr>>, String> {
    text.split('|')
        .map(str::trim)
        .filter(|g| !g.is_empty())
        .map(|g| {
            g.split(',')
                .map(str::trim)
                .filter(|e| !e.is_empty())
                .map(|e| FilterExpr::parse(e).map_err(|err| err.to_string()))
                .collect()
        })
        .collect()
}

impl TrafficFilter {
    pub fn defaults(window: &str, max_results: u32) -> Self {
        Self {
            window: window.to_string(),
            src: String::new(),
            dst: String::new(),
            service: String::new(),
            decisions: PolicyDecision::ALL.to_vec(),
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
        Ok(())
    }

    pub fn is_unbounded(&self) -> bool {
        self.src.trim().is_empty() && self.dst.trim().is_empty() && self.service.trim().is_empty()
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
        let decision =
            if self.decisions.len() == PolicyDecision::ALL.len() || self.decisions.is_empty() {
                "all".to_string()
            } else {
                self.decisions
                    .iter()
                    .map(|d| d.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            };
        format!(
            "window {}  src {}  dst {}  svc {}  decision {}",
            self.window,
            or(&self.src),
            or(&self.dst),
            or(&self.service),
            decision
        )
    }

    /// Resolves names through the facade's cached index and builds the query.
    pub async fn to_query(&self, pce: &Pce, mode: CacheMode) -> pce::Result<TrafficQuery> {
        let window = self.window_duration().map_err(pce::PceError::Invalid)?;
        let mut b = TrafficQueryBuilder::last(window)?;
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
            .max_results(self.max_results)
            .build()
    }
}

pub const FIELD_NAMES: [&str; 6] = [
    "window",
    "source",
    "destination",
    "service",
    "decisions",
    "max results",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterEditor {
    pub values: [String; 6],
    pub active: usize,
    pub error: Option<String>,
}

impl FilterEditor {
    pub fn from_filter(f: &TrafficFilter) -> Self {
        let decisions = if f.decisions.len() == PolicyDecision::ALL.len() {
            "all".to_string()
        } else {
            f.decisions
                .iter()
                .map(|d| d.as_str())
                .collect::<Vec<_>>()
                .join(",")
        };
        Self {
            values: [
                f.window.clone(),
                f.src.clone(),
                f.dst.clone(),
                f.service.clone(),
                decisions,
                f.max_results.to_string(),
            ],
            active: 0,
            error: None,
        }
    }

    pub fn to_filter(&self) -> Result<TrafficFilter, String> {
        let decisions_text = self.values[4].trim();
        let decisions = if decisions_text.is_empty() || decisions_text == "all" {
            PolicyDecision::ALL.to_vec()
        } else {
            decisions_text
                .split(',')
                .map(str::trim)
                .map(|d| {
                    PolicyDecision::parse(d).ok_or_else(|| {
                        format!("decision: unknown {d:?} (allowed, potentially_blocked, blocked)")
                    })
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        let max_results = self.values[5]
            .trim()
            .parse::<u32>()
            .map_err(|_| format!("max results: not a number {:?}", self.values[5]))?;
        let f = TrafficFilter {
            window: self.values[0].trim().to_string(),
            src: self.values[1].trim().to_string(),
            dst: self.values[2].trim().to_string(),
            service: self.values[3].trim().to_string(),
            decisions,
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
        e.active = 5;
        e.values[5] = "x".into();
        assert!(e.to_filter().unwrap_err().contains("max results"));
        e.values[5] = "500".into();
        e.values[4] = "allowed, maybe".into();
        assert!(e.to_filter().unwrap_err().contains("decision"));
        e.prev();
        e.prev();
        e.prev();
        e.prev();
        e.prev();
        e.prev();
        assert_eq!(e.active, 0, "prev clamps at the first field");
    }
}
