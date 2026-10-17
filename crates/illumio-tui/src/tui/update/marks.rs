//! The marks' keys: `m` opens the mark form prefilled from the row, its
//! keys and live preview, the save to `marks.toml`. No path here returns a
//! `Cmd`: marks never reach the PCE.
use super::complete_field;
use crate::marks::MarkOp;
use crate::tui::app_view::AppPanel;
use crate::tui::form::FormEvent;
use crate::tui::mark_form::{
    MarkForm, MarkOrigin, Prefill, RowsOf, SidePrefill, any_side, decisions_text, endpoint_side,
    group_side,
};
use crate::tui::model::{
    By, Drill, FlowRow, Model, Overlay, RowKind, SectionRow, StatusLine, TrafficState, TrafficView,
};
use crate::tui::msg::Cmd;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pce::marks::Action;
use pce::traffic::{
    Decisions, Direction, FlowGroup, MeshRow, ResolvedFlow, app_sections, app_sections_both,
    group_by_labels, group_by_service, group_key, mesh,
};
use std::collections::BTreeMap;

/// `m` on the Traffic screen: the form for the selected row, or why not.
/// Never a request.
pub(super) fn mark_key(model: &mut Model) -> Vec<Cmd> {
    let t = &model.traffic;
    // An invalid file: its problems instead of the form.
    if t.marks.error().is_some() {
        return open_marks(model, None);
    }
    if t.data.is_none() {
        model.status = StatusLine::info("no traffic loaded: r runs the query");
        return Vec::new();
    }
    let prefill = match t.view {
        TrafficView::Flows if t.flows_grouped() => {
            t.flows_grouped
                .selected_item()
                .and_then(|r| match r.flow() {
                    Some(f) => Some(flow_prefill(t, f)),
                    None => section_prefill(t, r, false),
                })
        }
        TrafficView::Flows => t.flows.selected_item().map(|r| flow_prefill(t, r)),
        TrafficView::Aggregate if t.apps_state.by == By::Pairs => {
            t.pairs_table().selected_item().map(|g| pair_prefill(t, g))
        }
        TrafficView::Aggregate => t
            .apps
            .selected_item()
            .and_then(|r| section_prefill(t, r, false)),
        TrafficView::Mesh => t.mesh.selected_item().map(|r| mesh_prefill(t, r)),
        TrafficView::Blocked => t
            .blocked
            .selected_item()
            .and_then(|r| section_prefill(t, r, true)),
        TrafficView::Treemap => {
            model.status = StatusLine::info("m: Enter drills to the flows, mark there");
            return Vec::new();
        }
    };
    let Some(p) = prefill else {
        model.status = StatusLine::info("no row to mark");
        return Vec::new();
    };
    let host = model.profile.host.clone();
    open_form(model, MarkForm::new(p, MarkOrigin::Traffic, &host));
    Vec::new()
}

/// `m` in the app overlay: the selected peer of the Inbound or Outbound
/// panel; the Policy and Workloads panels say where marks go.
pub(super) fn mark_app_key(model: &mut Model) -> Vec<Cmd> {
    let Some(v) = model.app_view.as_ref() else {
        return Vec::new();
    };
    if matches!(v.panel, AppPanel::Policy | AppPanel::Workloads) {
        model.status = StatusLine::info("m marks traffic rows (Inbound, Outbound)");
        return Vec::new();
    }
    if model.traffic.marks.error().is_some() {
        return open_marks(model, None);
    }
    let Some((dir, p)) = v.selected_peer() else {
        model.status = StatusLine::info("no row to mark");
        return Vec::new();
    };
    let focus = v.focus.clone();
    let drill = Drill::Peer {
        app: focus.app.clone(),
        direction: dir,
        peer: p.peer.clone(),
        service: p.service.clone(),
        decisions: Decisions::All,
    };
    let t = &model.traffic;
    let rep = t.first_flow(|f| drill.matches(f, &focus.keys) && f.policy_decision == p.decision);
    let (peer_ep, app_ep) = split(rep.as_ref(), dir);
    let peer = group_side(&p.peer, &focus.keys, peer_ep);
    let app = group_side(&focus.app, &focus.keys, app_ep);
    let (src, dst) = match dir {
        Direction::In => (peer, app),
        Direction::Out => (app, peer),
    };
    let decision = match p.decision {
        pce::models::PolicyDecision::Unknown => String::new(),
        d => d.as_str().to_string(),
    };
    let prefill = Prefill {
        what: format!(
            "{} {} · {} {}",
            focus.app_text(),
            dir.as_str(),
            p.peer_text(),
            p.service
        ),
        action: Action::Reviewed,
        src,
        dst,
        service: p.service.clone(),
        decision,
        draft: String::new(),
        rows: RowsOf::Peers(Box::new(focus), dir),
    };
    let host = model.profile.host.clone();
    open_form(model, MarkForm::new(prefill, MarkOrigin::App, &host));
    Vec::new()
}

/// The endpoints of `f` on the peer and the app side of `dir`.
fn split(
    f: Option<&ResolvedFlow>,
    dir: Direction,
) -> (
    Option<&pce::traffic::ResolvedEndpoint>,
    Option<&pce::traffic::ResolvedEndpoint>,
) {
    match (f, dir) {
        (Some(f), Direction::In) => (Some(&f.src), Some(&f.dst)),
        (Some(f), Direction::Out) => (Some(&f.dst), Some(&f.src)),
        (None, _) => (None, None),
    }
}

fn flow_prefill(t: &TrafficState, r: &FlowRow) -> Prefill {
    let service = r.service_text();
    Prefill {
        what: format!(
            "Flows · {} → {} {service}",
            r.src.display_name(),
            r.dst.display_name()
        ),
        action: Action::Reviewed,
        src: endpoint_side(&r.src, &t.keys),
        dst: endpoint_side(&r.dst, &t.keys),
        service,
        decision: String::new(),
        draft: String::new(),
        rows: RowsOf::Flows,
    }
}

fn pair_prefill(t: &TrafficState, g: &FlowGroup) -> Prefill {
    let d = t.apps_state.decisions;
    let drill = match &g.service {
        Some(s) => Drill::Service(g.src.clone(), g.dst.clone(), s.clone(), d),
        None => Drill::Pair(g.src.clone(), g.dst.clone(), d),
    };
    let rep = t.first_flow(|f| drill.matches(f, &t.keys));
    let (decision, draft) = decisions_text(d);
    let service = g.service.clone().unwrap_or_default();
    Prefill {
        what: format!(
            "App-to-app pairs · {} → {} {service}",
            g.src.join("/"),
            g.dst.join("/")
        )
        .trim_end()
        .to_string(),
        action: Action::Reviewed,
        src: group_side(&g.src, &t.keys, rep.as_ref().map(|f| &f.src)),
        dst: group_side(&g.dst, &t.keys, rep.as_ref().map(|f| &f.dst)),
        service,
        decision,
        draft,
        rows: if g.service.is_some() {
            RowsOf::Services(d)
        } else {
            RowsOf::Pairs(d)
        },
    }
}

fn mesh_prefill(t: &TrafficState, r: &MeshRow) -> Prefill {
    let drill = Drill::Mesh {
        key: r.key(),
        cols: t.mesh_cols,
    };
    let rep = t.first_flow(|f| drill.matches(f, &t.keys));
    let service = r.service_text();
    Prefill {
        what: format!(
            "Mesh · {} → {} {service}{}",
            r.src.join("/"),
            r.dst.join("/"),
            // A mark has no user or process: it covers every one.
            if t.mesh_cols.user || t.mesh_cols.process {
                " · user/process not part of a mark"
            } else {
                ""
            }
        ),
        action: Action::Reviewed,
        src: group_side(&r.src, &t.keys, rep.as_ref().map(|f| &f.src)),
        dst: group_side(&r.dst, &t.keys, rep.as_ref().map(|f| &f.dst)),
        service,
        decision: String::new(),
        draft: String::new(),
        rows: RowsOf::Mesh(t.mesh_cols),
    }
}

/// A section's peer row (direction-aware: in `in` the peer is the
/// source) or its header (the app side only; the peer side `any`).
fn section_prefill(t: &TrafficState, r: &SectionRow, blocked: bool) -> Option<Prefill> {
    let drill = t.section_drill()?;
    let decisions = t.section_decisions();
    let (decision, draft) = decisions_text(decisions);
    let rep = t.first_flow(|f| drill.matches(f, &t.keys));
    let dir_of_view = if blocked {
        Some(Direction::In)
    } else {
        match t.apps_state.by {
            By::In => Some(Direction::In),
            By::Out => Some(Direction::Out),
            _ => None,
        }
    };
    let (view, rows) = match t.view {
        TrafficView::Blocked => ("Blocked", RowsOf::Sections(dir_of_view, decisions)),
        // A Flows section's rows are its flows.
        TrafficView::Flows => ("Flows", RowsOf::Flows),
        _ => ("App-to-app", RowsOf::Sections(dir_of_view, decisions)),
    };
    let app_text = r.app.join("/");
    let (src, dst, service, what) = match &r.kind {
        RowKind::Header { totals, .. } => {
            // A `both` header: the app as the destination.
            let dir = match totals.as_slice() {
                [(d, _)] => *d,
                _ => Direction::In,
            };
            // The representative flow may run either way under a `both`
            // header: the app's endpoint is the one in the app's group.
            let app_ep = rep.as_ref().and_then(|f| {
                [&f.dst, &f.src]
                    .into_iter()
                    .find(|ep| group_key(ep, &t.keys) == r.app)
            });
            let app = group_side(&r.app, &t.keys, app_ep);
            let (src, dst) = match dir {
                Direction::In => (any_side(), app),
                Direction::Out => (app, any_side()),
            };
            (src, dst, String::new(), format!("{view} · {app_text}"))
        }
        RowKind::Peer { direction, row, .. } => {
            let (peer_ep, app_ep) = split(rep.as_ref(), *direction);
            let peer: SidePrefill = group_side(&row.peer, &t.keys, peer_ep);
            let app = group_side(&r.app, &t.keys, app_ep);
            let what = format!(
                "{view} {} · {app_text} {} {} {}",
                direction.as_str(),
                direction.arrow(),
                row.peer_text(),
                row.service
            );
            match direction {
                Direction::In => (peer, app, row.service.clone(), what),
                Direction::Out => (app, peer, row.service.clone(), what),
            }
        }
        // A flow is marked as on the flat Flows view (`flow_prefill`).
        RowKind::Flow { .. } => return None,
    };
    Some(Prefill {
        what,
        action: if blocked {
            Action::AcceptBlocked
        } else {
            Action::Reviewed
        },
        src,
        dst,
        service,
        decision,
        draft,
        rows,
    })
}

/// Of `flows`, those the view `rows` names can hold: its decision filter
/// applied (pairs, services, sections), the app overlay's side of the app.
fn in_view(rows: &RowsOf, flows: Vec<ResolvedFlow>) -> Vec<ResolvedFlow> {
    match rows {
        RowsOf::Flows | RowsOf::Mesh(_) => flows,
        RowsOf::Pairs(d) | RowsOf::Services(d) | RowsOf::Sections(_, d) => {
            flows.into_iter().filter(|f| d.keeps(f)).collect()
        }
        RowsOf::Peers(focus, dir) => flows
            .into_iter()
            .filter(|f| {
                let ep = match dir {
                    Direction::In => &f.dst,
                    Direction::Out => &f.src,
                };
                group_key(ep, &focus.keys) == focus.app
            })
            .collect(),
    }
}

/// The rows of the view `rows` names that hold one of `flows`: the view's
/// own aggregation of them.
fn rows_of(t: &TrafficState, rows: &RowsOf, flows: &[ResolvedFlow]) -> usize {
    let keys = &t.keys;
    match rows {
        RowsOf::Flows => flows.len(),
        RowsOf::Pairs(d) => group_by_labels(&d.kept(flows), keys).len(),
        RowsOf::Services(d) => group_by_service(&d.kept(flows), keys).len(),
        RowsOf::Sections(Some(dir), d) => app_sections(flows, keys, *dir, *d)
            .iter()
            .map(|s| s.rows.len())
            .sum(),
        RowsOf::Sections(None, d) => app_sections_both(flows, keys, *d)
            .iter()
            .map(|s| s.rows.len())
            .sum(),
        RowsOf::Mesh(cols) => mesh(flows, keys, *cols).len(),
        RowsOf::Peers(focus, dir) => pce::app::app_peers(flows, focus, *dir).len(),
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The live preview: the loaded result (all of it, not only the focus)
/// classified against the active marks plus this one. Local; no request.
pub(crate) fn preview(t: &TrafficState, mf: &MarkForm, now: jiff::Timestamp) -> Vec<String> {
    let Ok(draft) = mf.mark(now) else {
        return Vec::new();
    };
    let matched: Vec<ResolvedFlow> = t
        .loaded_flows()
        .iter()
        .filter(|f| draft.matches(f))
        .cloned()
        .collect();
    if matched.is_empty() {
        return vec!["matches nothing in this result".into()];
    }
    // Rows and flows of one set: the flows the view's rows can hold.
    let all = matched.len();
    let matched = in_view(&mf.ctx.rows, matched);
    let rows = rows_of(t, &mf.ctx.rows, &matched);
    let squelch = draft.action == Action::Squelch;
    let mut first = format!(
        "matches {} · {} in this result",
        plural(rows, "row", "rows"),
        plural(matched.len(), "flow", "flows")
    );
    if all > matched.len() {
        first.push_str(&format!(" (+{} outside this view)", all - matched.len()));
    }
    if squelch {
        first.push_str(" · hides them (M shows)");
    }
    let mut lines = vec![first];
    // The file's marks (the edited one replaced) plus this one.
    let mut set = t.marks.set().cloned().unwrap_or_default();
    let at = match mf.ctx.editing.as_ref().and_then(|m| set.find(&m.id)) {
        Some(i) => {
            set.marks[i] = draft.clone();
            i
        }
        None => {
            set.marks.push(draft.clone());
            set.marks.len() - 1
        }
    };
    let active = set.active(Some(&t.marks_pce), now, t.draft_loaded());
    if !active.is_active(at) {
        lines.push("not active here: another PCE, expired, or waiting for D".into());
        return lines;
    }
    let mut winners: BTreeMap<usize, u64> = BTreeMap::new();
    for f in &matched {
        // Only a stronger action takes a flow away; a more specific mark
        // of the same action leaves it the same.
        if let Some(h) = active.classify(f)
            && h.action < draft.action
        {
            *winners.entry(h.mark).or_default() += 1;
        }
    }
    let k: u64 = winners.values().sum();
    if let Some((&top, _)) = winners.iter().max_by_key(|(_, n)| **n) {
        let w = &set.marks[top];
        lines.push(format!(
            "{k} of them {}: {} {} wins",
            if squelch {
                "stay shown"
            } else {
                "go to a stronger mark"
            },
            w.action.as_str(),
            w.id
        ));
    }
    lines
}

/// The form's validation and preview after a key.
fn refresh(t: &TrafficState, mf: &mut MarkForm) {
    let now = jiff::Timestamp::now();
    mf.form.error = mf.mark(now).err();
    mf.preview = preview(t, mf, now);
}

fn open_form(model: &mut Model, mut mf: MarkForm) {
    refresh(&model.traffic, &mut mf);
    model.overlay = Overlay::MarkForm(Box::new(mf));
}

/// Back where the form came from: the `:marks` overlay rebuilt.
fn close(model: &mut Model, origin: MarkOrigin) {
    model.overlay = match origin {
        MarkOrigin::App if model.app_view.is_some() => Overlay::App,
        MarkOrigin::Marks if model.marks_view.is_some() => {
            rebuild_view(model);
            Overlay::Marks
        }
        _ => Overlay::None,
    };
}

/// The `:marks` rows again from the file as it is now: a write refused
/// (a mark deleted elsewhere, an invalid file) leaves the store behind the
/// file, so one `stat` first (the views rebuilt if it changed).
fn rebuild_view(model: &mut Model) {
    super::rebuild_app_traffic(model); // `sync_marks` first
    if let Some(v) = model.marks_view.as_mut() {
        v.rebuild(&model.traffic, jiff::Timestamp::now());
    }
}

/// `:marks [action]`: every mark (one action's), or the problems of an
/// invalid file. The file is looked at first (one `stat`). No request.
pub(super) fn open_marks(model: &mut Model, action: Option<&str>) -> Vec<Cmd> {
    let filter = match action {
        None => None,
        Some(a) => match Action::parse(a) {
            Some(a) => Some(a),
            None => {
                model.status = StatusLine::error(format!("unknown action: {}", Action::names()));
                return Vec::new();
            }
        },
    };
    // The file as it is now (one `stat`; the views rebuilt if it changed).
    model.traffic.sync_marks();
    let mut v = crate::tui::marks_view::MarksView::new(filter);
    v.from_app = matches!(model.overlay, Overlay::App);
    v.rebuild(&model.traffic, jiff::Timestamp::now());
    model.marks_view = Some(v);
    model.overlay = Overlay::Marks;
    Vec::new()
}

/// The overlay's keys: `j`/`k` (and the table's keys, `/`, `s`), `Enter`
/// edits, `x` asks to delete, `e` extends by 30 days (asking why again),
/// `Esc`/`q` back. Every change is one operation on `marks.toml`.
pub(super) fn overlay_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let page = super::traffic_page(model);
    let Some(v) = model.marks_view.as_mut() else {
        model.overlay = Overlay::None;
        return Vec::new();
    };
    if v.table.is_searching() {
        super::search_key(&mut v.table, k);
        v.table.ensure_visible(page);
        return Vec::new();
    }
    let pce = model.traffic.marks_pce.clone();
    match k.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            model.overlay = if v.from_app && model.app_view.is_some() {
                Overlay::App
            } else {
                Overlay::None
            };
        }
        KeyCode::Char('?') => super::open_help(model),
        KeyCode::Enter => {
            if let Some(r) = v.selected() {
                let mf = MarkForm::edit(&r.mark, &pce, false);
                open_form(model, mf);
            }
        }
        KeyCode::Char('x') => {
            if let Some(r) = v.selected() {
                model.overlay = Overlay::Confirm {
                    reason: format!(
                        "delete {} ({}, {})? y/n",
                        r.mark.id,
                        r.mark.action.as_str(),
                        // An inactive mark has no hits here, not zero.
                        if r.active {
                            plural(r.hit_count as usize, "flow", "flows")
                        } else {
                            "–".into()
                        }
                    ),
                    action: crate::tui::model::PendingAction::DeleteMark(r.mark.id.clone()),
                };
            }
        }
        KeyCode::Char('e') => match v.selected() {
            Some(r) if r.mark.expires.is_none() => {
                model.status =
                    StatusLine::info(format!("{} never expires: nothing to extend", r.mark.id));
            }
            Some(r) => {
                let mf = MarkForm::edit(&r.mark, &pce, true);
                open_form(model, mf);
            }
            None => {}
        },
        _ => {
            super::table_key(&mut v.table, k, page);
            v.table.ensure_visible(page);
        }
    }
    Vec::new()
}

/// `y` on `delete m-…?`: the mark leaves the file (lock, reload, write),
/// the views rebuild, the overlay comes back. A local file only.
pub(super) fn delete_mark(model: &mut Model, id: &str) -> Vec<Cmd> {
    let now = jiff::Timestamp::now();
    model.status = match model
        .traffic
        .marks
        .apply(MarkOp::Delete(vec![id.to_string()]), now)
    {
        Ok(_) => {
            model.traffic.rebuild_views();
            super::rebuild_app_traffic(model);
            StatusLine::info(format!("mark {id} deleted"))
        }
        Err(e) => StatusLine::error(e.to_string()),
    };
    rebuild_view(model);
    model.overlay = Overlay::Marks;
    Vec::new()
}

/// The form's keys: `Ctrl-N`/`Ctrl-P` complete from the loaded data (no
/// request), `Ctrl-T` cycles a side's kind, `Ctrl-A` swaps all labels in,
/// `Enter` saves, `Esc` returns.
pub(super) fn form_key(model: &mut Model, mut mf: MarkForm, k: KeyEvent) -> Vec<Cmd> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let before = mf.form.value("Expires").to_string();
    if ctrl && matches!(k.code, KeyCode::Char('n' | 'p')) {
        complete_field(model, &mut mf.form, k.code == KeyCode::Char('n'));
    } else if ctrl && k.code == KeyCode::Char('t') {
        mf.form.completion = None;
        mf.cycle_kind();
    } else if ctrl && k.code == KeyCode::Char('a') {
        mf.form.completion = None;
        mf.swap_labels();
    } else {
        match mf.form.on_key(k) {
            FormEvent::Continue | FormEvent::NewRuleset => {}
            FormEvent::Cancel => {
                close(model, mf.ctx.origin);
                model.status = StatusLine::info("cancelled, nothing written");
                return Vec::new();
            }
            FormEvent::Submit => return submit(model, mf),
        }
    }
    mf.sync_expiry(&before);
    refresh(&model.traffic, &mut mf);
    model.overlay = Overlay::MarkForm(Box::new(mf));
    Vec::new()
}

/// `Enter`: the comment is required; then one operation on `marks.toml`
/// (lock, reload, write), the views rebuilt, back where the form came
/// from. A failure keeps the form open with the error. Never a `Cmd`.
fn submit(model: &mut Model, mut mf: MarkForm) -> Vec<Cmd> {
    let now = jiff::Timestamp::now();
    if mf.comment().is_empty() {
        mf.form.active = if mf.ctx.extend {
            0
        } else {
            crate::tui::mark_form::COMMENT
        };
        mf.form.error = Some("comment required: why are these flows marked?".into());
        model.overlay = Overlay::MarkForm(Box::new(mf));
        return Vec::new();
    }
    let mark = match mf.mark(now) {
        Ok(m) => m,
        Err(e) => {
            mf.form.error = Some(e);
            model.overlay = Overlay::MarkForm(Box::new(mf));
            return Vec::new();
        }
    };
    let flows = model
        .traffic
        .loaded_flows()
        .iter()
        .filter(|f| mark.matches(f))
        .count();
    let op = match (&mf.ctx.editing, mf.ctx.extend) {
        (Some(m), true) => MarkOp::Extend {
            id: m.id.clone(),
            days: 30,
            comment: Some(mf.comment().to_string()),
        },
        (Some(_), false) => MarkOp::Replace(mark.clone()),
        (None, _) => MarkOp::Add(mark.clone()),
    };
    match model.traffic.marks.apply(op, now) {
        Ok(done) => {
            let id = done.ids.first().cloned().unwrap_or(mark.id.clone());
            model.traffic.rebuild_views();
            super::rebuild_app_traffic(model);
            close(model, mf.ctx.origin);
            let backup = if done.backup.is_some() {
                " · marks.toml had comments: kept in marks.toml.bak"
            } else {
                ""
            };
            model.status = StatusLine::info(if mf.ctx.extend && done.ids.is_empty() {
                // Made `never` elsewhere since `:marks` showed it.
                format!("{id} now never expires: nothing extended")
            } else if mf.ctx.extend {
                format!("mark {id} extended by 30 days{backup}")
            } else {
                format!(
                    "mark {id} saved: {}, {} · :marks lists marks{backup}",
                    mark.action.as_str(),
                    plural(flows, "flow", "flows")
                )
            });
        }
        Err(e) => {
            // The file may have changed under the refused write: one
            // `stat`, the views (and the app overlay) rebuilt if it did.
            super::rebuild_app_traffic(model);
            mf.form.error = Some(e.to_string());
            model.overlay = Overlay::MarkForm(Box::new(mf));
        }
    }
    Vec::new()
}
