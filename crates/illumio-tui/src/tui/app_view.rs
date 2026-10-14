//! The app overlay's state (`A` / `:app`): one app's peers, workloads and
//! policy. Built from what is loaded plus at most two requests, each counted
//! here. Pure: `update` fills it, `ui::widgets::app_overlay` draws it.
use pce::app::{AppPeer, AppPolicyRow, AppWorkload, WorkloadSummary, app_peers};
use pce::models::PolicyScope;
use pce::traffic::{Direction, Focus, FocusDirection, ResolvedFlow};

/// The panels `Tab` moves between, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppPanel {
    Inbound,
    Outbound,
    Policy,
    Workloads,
}

impl AppPanel {
    pub const ALL: [AppPanel; 4] = [
        AppPanel::Inbound,
        AppPanel::Outbound,
        AppPanel::Policy,
        AppPanel::Workloads,
    ];

    pub fn index(self) -> usize {
        AppPanel::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }

    /// `Tab`.
    pub fn next(self) -> Self {
        AppPanel::ALL[(self.index() + 1) % AppPanel::ALL.len()]
    }

    /// `BackTab`.
    pub fn prev(self) -> Self {
        AppPanel::ALL[(self.index() + AppPanel::ALL.len() - 1) % AppPanel::ALL.len()]
    }

    pub fn title(self) -> &'static str {
        match self {
            AppPanel::Inbound => "Inbound",
            AppPanel::Outbound => "Outbound",
            AppPanel::Policy => "Policy",
            AppPanel::Workloads => "Workloads",
        }
    }
}

/// A panel fed by a request.
#[derive(Debug, Clone, PartialEq)]
pub enum Loadable<T> {
    Loading,
    Loaded(T),
    /// The request failed; `r` retries.
    Failed(String),
    /// Nothing can be loaded (why).
    Unavailable(String),
}

#[derive(Debug, Clone)]
pub struct AppView {
    pub focus: Focus,
    /// The traffic window, as the filter names it (`24h`).
    pub window: String,
    /// Answers carrying another `seq` belong to an earlier overlay.
    pub seq: u64,
    pub panel: AppPanel,
    /// One cursor per panel, by `AppPanel::index`.
    pub cursor: [usize; 4],
    pub inbound: Vec<AppPeer>,
    pub outbound: Vec<AppPeer>,
    /// The traffic these panels come from was truncated and is not a
    /// focused result for this app.
    pub partial: bool,
    /// The app's labels; `None` for a group that is not a label group.
    pub label_hrefs: Option<Vec<String>>,
    pub workloads: Loadable<(Vec<AppWorkload>, WorkloadSummary)>,
    pub workloads_truncated: bool,
    pub policy: Loadable<Vec<AppPolicyRow>>,
    /// The rulesets were requested by this overlay (an answer counts).
    pub policy_sent: bool,
    /// Draft or active: the Policy screen's setting the panel was filled
    /// (or is loading) for. Another one makes the panel stale.
    pub policy_scope: PolicyScope,
    /// Requests this overlay sent.
    pub requests: u32,
    /// Of those, the ones answered from the cache.
    pub cached: u32,
}

impl AppView {
    pub fn new(focus: Focus, window: String, seq: u64) -> Self {
        Self {
            focus,
            window,
            seq,
            panel: AppPanel::Inbound,
            cursor: [0; 4],
            inbound: Vec::new(),
            outbound: Vec::new(),
            partial: false,
            label_hrefs: None,
            workloads: Loadable::Loading,
            workloads_truncated: false,
            policy: Loadable::Loading,
            policy_sent: false,
            policy_scope: PolicyScope::Draft,
            requests: 0,
            cached: 0,
        }
    }

    /// Peers from `flows` for the direction in force; a side the direction
    /// leaves out has no rows. No request.
    pub fn rebuild_traffic(&mut self, flows: &[ResolvedFlow], partial: bool) {
        let d = self.focus.direction;
        self.inbound = if d == FocusDirection::Out {
            Vec::new()
        } else {
            app_peers(flows, &self.focus, Direction::In)
        };
        self.outbound = if d == FocusDirection::In {
            Vec::new()
        } else {
            app_peers(flows, &self.focus, Direction::Out)
        };
        self.partial = partial;
        for p in AppPanel::ALL {
            let n = self.panel_len(p);
            self.cursor[p.index()] = self.cursor[p.index()].min(n.saturating_sub(1));
        }
    }

    pub fn panel_len(&self, panel: AppPanel) -> usize {
        match panel {
            AppPanel::Inbound => self.inbound.len(),
            AppPanel::Outbound => self.outbound.len(),
            AppPanel::Policy => match &self.policy {
                Loadable::Loaded(rows) => rows.len(),
                _ => 0,
            },
            AppPanel::Workloads => match &self.workloads {
                Loadable::Loaded((rows, _)) => rows.len(),
                _ => 0,
            },
        }
    }

    /// `j` / `k`: within the current panel, clamped.
    pub fn move_cursor(&mut self, delta: isize) {
        let n = self.panel_len(self.panel);
        let c = &mut self.cursor[self.panel.index()];
        *c = c.saturating_add_signed(delta).min(n.saturating_sub(1));
    }

    pub fn selected_peer(&self) -> Option<(Direction, &AppPeer)> {
        let i = self.cursor[self.panel.index()];
        match self.panel {
            AppPanel::Inbound => self.inbound.get(i).map(|p| (Direction::In, p)),
            AppPanel::Outbound => self.outbound.get(i).map(|p| (Direction::Out, p)),
            _ => None,
        }
    }

    pub fn selected_workload(&self) -> Option<&AppWorkload> {
        match (&self.workloads, self.panel) {
            (Loadable::Loaded((rows, _)), AppPanel::Workloads) => {
                rows.get(self.cursor[AppPanel::Workloads.index()])
            }
            _ => None,
        }
    }

    pub fn selected_policy(&self) -> Option<&AppPolicyRow> {
        match (&self.policy, self.panel) {
            (Loadable::Loaded(rows), AppPanel::Policy) => {
                rows.get(self.cursor[AppPanel::Policy.index()])
            }
            _ => None,
        }
    }

    /// `shop/prod · in+out · last 24h`.
    pub fn header_title(&self) -> String {
        format!(
            "{} · {} · last {}",
            self.focus.app_text(),
            self.focus.direction.header_text(),
            self.window
        )
    }

    /// `2 req`, `· cached` once every request it sent came from the cache
    /// (or it sent none).
    pub fn header_meta(&self) -> String {
        let mut s = format!("{} req", self.requests);
        if self.requests == 0 || self.cached >= self.requests {
            s.push_str(" · cached");
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::model_with_flows;
    use pce::traffic::{FocusDirection, parse_app};

    fn shop(d: FocusDirection) -> Focus {
        Focus::new(parse_app("shop/prod"), vec!["app".into(), "env".into()]).with_direction(d)
    }

    #[test]
    fn the_traffic_panels_follow_the_direction() {
        let m = model_with_flows();
        let mut v = AppView::new(shop(FocusDirection::Both), "24h".into(), 1);
        v.rebuild_traffic(m.traffic.loaded_flows(), false);
        assert_eq!((v.inbound.len(), v.outbound.len()), (3, 2));
        v.focus.direction = FocusDirection::In;
        v.rebuild_traffic(m.traffic.loaded_flows(), false);
        assert_eq!((v.inbound.len(), v.outbound.len()), (3, 0));
        assert_eq!(v.header_title(), "shop/prod · in · last 24h");
        assert_eq!(v.header_meta(), "0 req · cached");
    }

    #[test]
    fn tab_cycles_the_panels_and_the_cursor_stays_in_its_panel() {
        let m = model_with_flows();
        let mut v = AppView::new(shop(FocusDirection::Both), "24h".into(), 1);
        v.rebuild_traffic(m.traffic.loaded_flows(), false);
        assert_eq!(v.panel, AppPanel::Inbound);
        v.move_cursor(5);
        assert_eq!(v.cursor[0], 2, "clamped to the 3 inbound rows");
        assert_eq!(
            v.selected_peer().map(|(d, p)| (d, p.peer_text())),
            Some((Direction::In, "unmanaged-nas/-".into()))
        );
        v.move_cursor(-1);
        assert_eq!(v.cursor[0], 1);
        let mut seen = Vec::new();
        for _ in 0..4 {
            v.panel = v.panel.next();
            seen.push(v.panel);
        }
        assert_eq!(
            seen,
            vec![
                AppPanel::Outbound,
                AppPanel::Policy,
                AppPanel::Workloads,
                AppPanel::Inbound
            ]
        );
        assert_eq!(AppPanel::Inbound.prev(), AppPanel::Workloads);
        v.panel = AppPanel::Policy;
        v.move_cursor(1);
        assert_eq!(v.cursor[2], 0, "nothing loaded: the cursor stays");
        assert!(v.selected_policy().is_none());
    }

    #[test]
    fn the_view_is_partial_when_the_traffic_is_truncated_and_not_focused() {
        let mut m = model_with_flows();
        m.traffic.data.as_mut().unwrap().truncated = true;
        let f = shop(FocusDirection::Both);
        assert!(m.traffic.partial_for(&f));
        let data = crate::tui::model::TrafficData {
            flows: m.traffic.loaded_flows()[..3].to_vec(),
            truncated: false,
            incomplete: false,
            elapsed: std::time::Duration::ZERO,
            cache_hit: false,
            fetched_at: std::time::Instant::now(),
            filter: m.traffic.filter.clone(),
            range: None,
            hrefs: Vec::new(),
            draft: None,
        };
        m.traffic.apply_focused(f.clone(), 2, data);
        assert!(!m.traffic.partial_for(&f), "a focused result covers it");
        assert_eq!(m.traffic.flows_for(&f).len(), 3);
        let other = Focus::new(
            parse_app("unmanaged-nas/-"),
            vec!["app".into(), "env".into()],
        );
        assert_eq!(
            m.traffic.flows_for(&other).len(),
            4,
            "another app reads the unfocused result"
        );
        assert!(m.traffic.partial_for(&other));
    }

    #[test]
    fn the_header_counts_requests_and_says_cached_when_every_answer_was() {
        let mut v = AppView::new(shop(FocusDirection::Both), "1h".into(), 1);
        v.requests = 2;
        assert_eq!(v.header_meta(), "2 req");
        v.cached = 2;
        assert_eq!(v.header_meta(), "2 req · cached");
    }
}
