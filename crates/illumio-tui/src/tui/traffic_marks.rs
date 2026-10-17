//! The marks on the Traffic screen: `marks.toml` refreshed at every
//! rebuild (one `stat` of a local file, never a request), the result on
//! screen split by them (squelched flows left out of every view unless
//! `M`), and what each view's meta says about them.
use crate::tui::model::{FlowRow, RowKind, SectionRow, StatusLine, TrafficState, TrafficView};
use pce::marks::{Action, ActiveMarks, Hit, Marked, Marker, RowMarks, partition};
use pce::traffic::{FlowGroup, MeshRow, ResolvedFlow};
use std::borrow::Cow;

/// A view's `squelched {rows, flows}`: the rows squelch removed (or, with
/// `M`, whose every record is squelched) and the squelched records of
/// the view's input.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Squelch {
    pub rows: u64,
    pub flows: u64,
}

impl Squelch {
    /// `37 rows (412 flows) squelched · M shows them`, or with `M` on
    /// `37 rows (412 flows) squelched, shown ~ · M hides them`; flows only
    /// for the treemap (`412 flows squelched`). `None` without a squelch hit.
    pub fn meta(self, shown: bool, rows: bool) -> Option<String> {
        if self.flows == 0 {
            return None;
        }
        let what = if rows {
            format!("{} rows ({} flows) squelched", self.rows, self.flows)
        } else {
            format!("{} flows squelched", self.flows)
        };
        Some(if shown {
            format!("{what}, shown ~ · M hides them")
        } else {
            format!("{what} · M shows them")
        })
    }
}

/// What the marks did to the result on screen.
#[derive(Debug, Clone, Default)]
pub struct MarkedView {
    /// The marked flows of `focus_flows()`, by winning mark.
    pub marked: Marked,
    /// The hit of each flow of `view_flows()`, in its order.
    pub hits: Vec<Option<Hit>>,
    /// Active marks, and marks waiting for draft decisions (`D`).
    pub active: usize,
    pub waiting: usize,
    pub flows: Squelch,
    pub pairs: Squelch,
    pub services: Squelch,
    pub sections: Squelch,
    pub blocked: Squelch,
    pub mesh: Squelch,
    pub treemap: Squelch,
    /// Blocked: flow records still blocked or potentially blocked, and
    /// those accepted as blocked (not problems).
    pub blocked_open: u64,
    pub accepted: u64,
}

/// The Flows view's gutter: the flow's own hit.
pub fn flow_marker(r: &FlowRow) -> Option<Marker> {
    r.mark.map(|h| Marker::of(h.action))
}

/// App-to-app pairs' (and by service) gutter.
pub fn group_marker(g: &FlowGroup) -> Option<Marker> {
    g.marks.as_ref().and_then(RowMarks::marker)
}

/// The Mesh view's gutter.
pub fn mesh_marker(r: &MeshRow) -> Option<Marker> {
    r.marks.as_ref().and_then(RowMarks::marker)
}

/// A sectioned table's gutter: a header shows its rows' marks summed.
pub fn section_marker(r: &SectionRow) -> Option<Marker> {
    match &r.kind {
        RowKind::Header { marks, .. } => marks.as_deref().and_then(RowMarks::marker),
        RowKind::Peer { row, .. } => row.marks.as_ref().and_then(RowMarks::marker),
    }
}

/// A section header's marks: its rows' records per action summed, so a
/// header shows any highlight of its rows and is dimmed only when all are.
pub fn sum_marks<'a>(
    rows: impl IntoIterator<Item = (u64, Option<&'a RowMarks>)>,
) -> Option<RowMarks> {
    let mut out = RowMarks::default();
    let mut any = false;
    for (flows, m) in rows {
        out.flows += flows;
        if let Some(m) = m {
            any = true;
            out.hidden += m.hidden;
            for (i, n) in m.by_action.iter().enumerate() {
                out.by_action[i] += n;
                if out.top[i].is_none() {
                    out.top[i].clone_from(&m.top[i]);
                }
            }
        }
    }
    any.then_some(out)
}

impl TrafficState {
    /// The marks in force for the result on screen: valid file, in scope
    /// for the profile's PCE, unexpired at the last refresh, `draft_decision`
    /// marks only with draft decisions loaded. `None` without a valid file.
    pub fn active_marks(&self) -> Option<ActiveMarks<'_>> {
        self.marks
            .set()
            .map(|s| s.active(Some(&self.marks_pce), self.marks_now, self.draft_loaded()))
    }

    /// The active marks and what they did, while some are active.
    pub fn marks_ctx(&self) -> Option<(ActiveMarks<'_>, &MarkedView)> {
        let mv = self.marked.as_ref()?;
        let a = self.active_marks().filter(|a| !a.is_empty())?;
        Some((a, mv))
    }

    /// The hit of the selected flow (Flows) and the weaker marks it also
    /// matches.
    pub fn flow_marks(&self, f: &ResolvedFlow) -> Vec<usize> {
        self.active_marks()
            .map(|a| a.matching(f))
            .unwrap_or_default()
    }

    /// One `stat` of `marks.toml` (parsed again only when it changed),
    /// then the result split by the active marks. Pure apart from that
    /// `stat`: a local file, never a request.
    /// The app overlay and the generator open (and rebuild) with the marks
    /// file as it is now: when a `stat` says it changed since the last
    /// rebuild, every view is rebuilt with it first. Never a request.
    pub fn sync_marks(&mut self) {
        if self.marks.is_stale() {
            self.rebuild_views();
        }
    }

    pub(crate) fn refresh_marks(&mut self) {
        let was_error = self.marks.error().cloned();
        let changed = self.marks.refresh();
        if changed {
            match self.marks.error() {
                Some(crate::marks::MarksError::Parse(p)) => {
                    self.marks_notice = Some(StatusLine::warn(format!(
                        "marks.toml invalid ({} problem{}): marks off · :marks shows them",
                        p.len(),
                        if p.len() == 1 { "" } else { "s" }
                    )));
                }
                Some(e) => {
                    self.marks_notice = Some(StatusLine::warn(format!("{e}: marks off")));
                }
                None if was_error.is_some() => {
                    self.marks_notice = Some(StatusLine::info("marks.toml valid again: marks on"));
                }
                None => {}
            }
        }
        self.marks_now = jiff::Timestamp::now();
        // The first load of the session announces marks expiring soon.
        if changed
            && !self.marks_announced
            && let Some(set) = self.marks.set()
        {
            self.marks_announced = true;
            let soon = self.marks_now + jiff::SignedDuration::from_hours(72);
            let n = set
                .marks
                .iter()
                .filter(|m| m.expires.is_some_and(|e| e > self.marks_now && e <= soon))
                .count();
            if n > 0 && self.marks_notice.is_none() {
                self.marks_notice = Some(StatusLine::info(format!(
                    "{n} mark{} expire{} within 3 days · :marks",
                    if n == 1 { "" } else { "s" },
                    if n == 1 { "s" } else { "" }
                )));
            }
        }
        let keep = self.show_squelched;
        let computed = self.active_marks().map(|a| {
            let (active, waiting) = (a.len(), a.waiting_for_draft());
            if a.is_empty() {
                return (
                    None,
                    MarkedView {
                        active,
                        waiting,
                        ..MarkedView::default()
                    },
                );
            }
            let p = partition(self.focus_flows(), &a, keep);
            let unsquelched = match p.kept {
                Cow::Owned(v) => Some(v),
                Cow::Borrowed(_) => None,
            };
            (
                unsquelched,
                MarkedView {
                    marked: p.marked,
                    hits: p.kept_hits,
                    active,
                    waiting,
                    ..MarkedView::default()
                },
            )
        });
        // A valid file with no mark active or waiting changes nothing.
        let (unsquelched, marked) = match computed {
            Some((u, m)) if m.active > 0 || m.waiting > 0 => (u, Some(m)),
            _ => (None, None),
        };
        self.unsquelched = unsquelched;
        self.marked = marked;
        // The gutter is drawn while a mark is active, and only then.
        let on = self.marked.as_ref().is_some_and(|m| m.active > 0);
        self.flows.marker = on.then_some(flow_marker as fn(&FlowRow) -> Option<Marker>);
        let groups = on.then_some(group_marker as fn(&FlowGroup) -> Option<Marker>);
        self.groups.marker = groups;
        self.services.marker = groups;
        self.mesh.marker = on.then_some(mesh_marker as fn(&MeshRow) -> Option<Marker>);
        let sections = on.then_some(section_marker as fn(&SectionRow) -> Option<Marker>);
        self.apps.marker = sections;
        self.blocked.marker = sections;
    }

    /// A status the marks file asked for (invalid, valid again, marks
    /// expiring soon): shown once by `update`.
    pub fn take_marks_notice(&mut self) -> Option<StatusLine> {
        self.marks_notice.take()
    }

    /// `M`: squelched flows shown (dimmed `~`) or left out, in every view
    /// and the app overlay. Session only; no request.
    pub fn toggle_squelched(&mut self) -> StatusLine {
        self.show_squelched = !self.show_squelched;
        self.rebuild_views();
        StatusLine::info(if self.show_squelched {
            "squelched rows shown (~) · M hides them"
        } else {
            "squelched rows hidden · M shows them"
        })
    }

    /// The marks' part of a view's panel meta: the squelched counts, the
    /// marks waiting for draft decisions, or `marks off: marks.toml
    /// invalid`. `None` when there is nothing to say.
    pub fn marks_meta(&self, view: TrafficView) -> Option<String> {
        if self.marks.error().is_some() {
            return Some("marks off: marks.toml invalid".into());
        }
        let mv = self.marked.as_ref()?;
        let sq = match view {
            TrafficView::Flows => mv.flows,
            TrafficView::Aggregate if self.apps_state.by != crate::tui::model::By::Pairs => {
                mv.sections
            }
            TrafficView::Aggregate if self.apps_state.by_service => mv.services,
            TrafficView::Aggregate => mv.pairs,
            TrafficView::Mesh => mv.mesh,
            TrafficView::Blocked => mv.blocked,
            TrafficView::Treemap => mv.treemap,
        };
        let mut parts: Vec<String> = Vec::new();
        if view == TrafficView::Blocked && mv.accepted > 0 {
            parts.push(format!(
                "{} blocked flows · {} accepted",
                mv.blocked_open, mv.accepted
            ));
        }
        parts.extend(sq.meta(self.show_squelched, view != TrafficView::Treemap));
        if mv.waiting > 0 {
            parts.push(format!(
                "{} mark{} wait for the draft decision (D)",
                mv.waiting,
                if mv.waiting == 1 { "" } else { "s" }
            ));
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    /// The squelched records `pred` keeps, with the rows `gone` reported:
    /// one view's counts.
    pub(crate) fn squelch(&self, rows: u64, pred: impl Fn(&ResolvedFlow) -> bool) -> Squelch {
        Squelch {
            rows,
            flows: self
                .marked
                .as_ref()
                .map_or(0, |m| m.marked.squelched_where(pred)),
        }
    }

    /// The flows accepted as blocked (`accept-blocked` hits) of the result.
    pub(crate) fn accepted_flows(&self) -> Vec<ResolvedFlow> {
        self.marked
            .iter()
            .flat_map(|m| &m.marked.subsets)
            .filter(|s| s.action == Action::AcceptBlocked)
            .flat_map(|s| s.flows.iter().cloned())
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::tui::model::{By, Model, RowKind};
    use crate::tui::test_support::model_with_marks;
    use pce::marks::Marker;

    pub(crate) use crate::tui::test_support::RICH_MARKS;

    /// The rich model with `marks` in a temp `marks.toml`, rebuilt.
    pub(crate) fn marked_model(marks: &str) -> (Model, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let m = model_with_marks(dir.path(), marks);
        (m, dir)
    }

    #[test]
    fn squelched_flows_leave_every_view_and_each_view_counts_them() {
        let (m, _dir) = marked_model(RICH_MARKS);
        let t = &m.traffic;
        let mv = t.marked.as_ref().unwrap();
        assert_eq!((mv.active, mv.waiting), (6, 0));
        assert_eq!(t.flows.rows().len(), 21, "30 flows, 9 squelched");
        assert_eq!(mv.flows, Squelch { rows: 9, flows: 9 });
        assert_eq!(mv.pairs, Squelch { rows: 4, flows: 9 });
        assert_eq!(mv.treemap.flows, 9);
        assert_eq!(mv.blocked, Squelch { rows: 2, flows: 2 });
        assert_eq!((mv.blocked_open, mv.accepted), (10, 2));
        let shown: u64 = t.groups.rows().iter().map(|g| g.flows).sum();
        assert_eq!(shown + mv.pairs.flows, 30, "visible + squelched = input");
        assert!(
            t.groups
                .rows()
                .iter()
                .all(|g| g.src.join("/") != "ipl:Corp/-" || g.dst.join("/") == "shop/prod"),
            "Corp is left only where its critical flow is"
        );
        let corp = t
            .groups
            .rows()
            .iter()
            .find(|g| g.src.join("/") == "ipl:Corp/-")
            .unwrap();
        assert_eq!(
            corp.marks.as_ref().and_then(RowMarks::marker),
            Some(Marker::Critical)
        );
        assert_eq!(corp.marks.as_ref().unwrap().hidden, 1);
        let hit = t
            .flows
            .rows()
            .iter()
            .find(|r| r.src.label_value("app") == Some("hr"))
            .and_then(|r| r.mark)
            .unwrap();
        assert_eq!(hit.action, Action::Revisit);
        assert_eq!(
            t.marks_meta(TrafficView::Aggregate).as_deref(),
            Some("4 rows (9 flows) squelched · M shows them")
        );
        assert_eq!(
            t.marks_meta(TrafficView::Blocked).as_deref(),
            Some("10 blocked flows · 2 accepted · 2 rows (2 flows) squelched · M shows them")
        );
        assert_eq!(
            t.marks_meta(TrafficView::Treemap).as_deref(),
            Some("9 flows squelched · M shows them")
        );
    }

    #[test]
    fn blocked_headers_take_accepted_records_out_and_say_how_many() {
        let (m, _dir) = marked_model(RICH_MARKS);
        let erp = m
            .traffic
            .blocked
            .rows()
            .iter()
            .find(|r| r.is_header() && r.app.join("/") == "erp/prod")
            .unwrap();
        let line = erp.header_line().unwrap();
        assert!(line.ends_with("· 2 pot.blocked · 2 accepted"), "{line}");
        let RowKind::Header { marks, .. } = &erp.kind else {
            unreachable!()
        };
        assert_eq!(marks.as_ref().unwrap().count(Action::AcceptBlocked), 2);
    }

    #[test]
    fn m_shows_squelched_rows_and_the_counts_stay() {
        let (mut m, _dir) = marked_model(RICH_MARKS);
        let s = m.traffic.toggle_squelched();
        assert_eq!(s.text, "squelched rows shown (~) · M hides them");
        let t = &m.traffic;
        assert_eq!(t.flows.rows().len(), 30);
        let mv = t.marked.as_ref().unwrap();
        assert_eq!(mv.pairs, Squelch { rows: 4, flows: 9 });
        let corp = t
            .flows
            .rows()
            .iter()
            .find(|r| r.src.ip_lists.first().map(String::as_str) == Some("Corp"))
            .unwrap();
        assert_eq!(corp.mark.map(|h| h.action), Some(Action::Squelch));
        assert_eq!(
            t.marks_meta(TrafficView::Flows).as_deref(),
            Some("9 rows (9 flows) squelched, shown ~ · M hides them")
        );
        let shop = t
            .groups
            .rows()
            .iter()
            .find(|g| g.src.join("/") == "shop/prod" && g.dst.join("/") == "shop/prod")
            .unwrap();
        assert_eq!(
            shop.marks.as_ref().and_then(RowMarks::marker),
            Some(Marker::Squelched)
        );
        m.traffic.toggle_squelched();
        assert_eq!(m.traffic.flows.rows().len(), 21);
    }

    #[test]
    fn sections_count_their_own_squelched_rows() {
        let (mut m, _dir) = marked_model(RICH_MARKS);
        m.traffic.apps_state.by = By::In;
        m.traffic.rebuild_sections();
        let mv = m.traffic.marked.as_ref().unwrap();
        assert_eq!(mv.sections.flows, 9);
        let peers: u64 = m
            .traffic
            .apps
            .rows()
            .iter()
            .filter_map(|r| r.peer())
            .map(|(_, p)| p.flows)
            .sum();
        assert_eq!(peers + 9, 30);
        assert!(mv.sections.rows > 0);
    }

    #[test]
    fn an_invalid_file_turns_marks_off_with_one_status_line() {
        let (mut m, _dir) = marked_model("[[mark]]\naction = \"squelch\"\n");
        assert!(m.traffic.marked.is_none());
        assert_eq!(m.traffic.flows.rows().len(), 30, "nothing left out");
        assert_eq!(
            m.traffic.marks_meta(TrafficView::Flows).as_deref(),
            Some("marks off: marks.toml invalid")
        );
        let s = m.traffic.take_marks_notice().unwrap();
        assert_eq!(
            s.text,
            "marks.toml invalid (2 problems): marks off · :marks shows them"
        );
        assert!(m.traffic.take_marks_notice().is_none(), "once");
        m.traffic.rebuild_views();
        assert!(
            m.traffic.take_marks_notice().is_none(),
            "unchanged file: no reparse"
        );
    }

    #[test]
    fn marks_expiring_within_three_days_are_announced_once() {
        let soon = (jiff::Timestamp::now() + jiff::SignedDuration::from_hours(48)).to_string();
        let text = format!(
            "[[mark]]\naction = \"squelch\"\nsrc = \"any\"\ndst = \"any\"\ncomment = \"x\"\nexpires = \"{soon}\"\n"
        );
        let (mut m, _dir) = marked_model(&text);
        assert_eq!(
            m.traffic.take_marks_notice().map(|s| s.text).as_deref(),
            Some("1 mark expires within 3 days · :marks")
        );
        m.traffic.rebuild_views();
        assert!(m.traffic.take_marks_notice().is_none());
    }

    #[test]
    fn draft_decision_marks_wait_for_draft_decisions() {
        let (m, _dir) = marked_model(
            "[[mark]]\naction = \"revisit\"\nsrc = \"any\"\ndst = \"any\"\ndecision = \"blocked\"\ndraft_decision = \"allowed\"\ncomment = \"fixed by the pending policy\"\n",
        );
        let mv = m.traffic.marked.as_ref().unwrap();
        assert_eq!((mv.active, mv.waiting), (0, 1));
        assert_eq!(
            m.traffic.marks_meta(TrafficView::Flows).as_deref(),
            Some("1 mark wait for the draft decision (D)")
        );
    }

    #[test]
    fn partly_squelched_rows_show_the_hidden_records_beside_the_count() {
        let (m, _dir) = marked_model(RICH_MARKS);
        let t = &m.traffic;
        let dev = t
            .groups
            .rows()
            .iter()
            .find(|g| g.src.join("/") == "shop/dev" && g.dst.join("/") == "shop/dev")
            .unwrap();
        let col = t
            .groups
            .columns
            .iter()
            .position(|c| c.title == "flows")
            .unwrap();
        assert_eq!(t.groups.cell(dev, col), "1 ~1");
        assert_eq!(
            t.groups.full_cells(dev)[col],
            "1",
            ":export writes the count"
        );
        assert!(t.groups.marker.is_some() && t.flows.marker.is_some());
        let plain = crate::tui::test_support::model_with_rich_flows();
        assert!(plain.traffic.groups.marker.is_none(), "no marks, no gutter");
    }

    #[test]
    fn the_panels_say_what_is_squelched_and_the_detail_names_the_mark() {
        use crate::tui::test_support::render_to_string;
        let (mut m, _dir) = marked_model(RICH_MARKS);
        m.traffic.view = TrafficView::Aggregate;
        let out = render_to_string(&m, 160, 40);
        assert!(
            out.contains("4 rows (9 flows) squelched · M shows them"),
            "{out}"
        );
        m.traffic.view = TrafficView::Blocked;
        let out = render_to_string(&m, 160, 40);
        assert!(out.contains("10 blocked flows · 2 accepted"), "{out}");
        assert!(out.contains("· 2 pot.blocked · 2 accepted"), "{out}");
        m.traffic.view = TrafficView::Treemap;
        let out = render_to_string(&m, 160, 40);
        assert!(out.contains("9 flows squelched · M shows them"), "{out}");
        assert_eq!(
            m.traffic.treemap.highlight("shop/dev", Some("shop/prod")),
            Some(Marker::Critical)
        );
        assert_eq!(
            m.traffic.treemap.highlight("hr/prod", None),
            Some(Marker::Revisit)
        );
        assert_eq!(
            m.traffic.treemap.highlight("erp/prod", None),
            None,
            "reviewed is no highlight"
        );
        assert!(out.contains("◆ hr/"), "{out}");
        // The Flows detail: Corp's blocked flow into the shop database is
        // critical and squelched by the Corp mark too.
        m.traffic.view = TrafficView::Flows;
        m.traffic.show_detail = true;
        m.traffic
            .flows
            .select_where(|r| !r.src.ip_lists.is_empty() && r.port == Some(5432));
        let out = render_to_string(&m, 160, 40);
        assert!(
            out.contains("mark       critical m-crit · payments db · never expires · also matches: m-sq1 (squelch)"),
            "{out}"
        );
        assert!(
            out.contains("9 rows (9 flows) squelched · M shows them"),
            "{out}"
        );
    }

    #[test]
    fn an_invalid_file_says_marks_off_in_the_panel() {
        use crate::tui::test_support::render_to_string;
        let (m, _dir) = marked_model("[[mark]]\naction = \"squelch\"\n");
        let out = render_to_string(&m, 120, 34);
        assert!(out.contains("marks off: marks.toml invalid"), "{out}");
    }

    #[test]
    fn a_profile_switch_keeps_the_file_and_m_and_scopes_to_the_new_pce() {
        let (mut m, _dir) = marked_model(RICH_MARKS);
        m.traffic.toggle_squelched();
        m.profile.host = "other.example".into();
        m.reset_data();
        assert!(m.traffic.show_squelched);
        assert_eq!(m.traffic.marks_pce.host, "other.example");
        assert!(m.traffic.marks.set().is_some(), "the same file");
    }
}
