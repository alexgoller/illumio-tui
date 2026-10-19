//! Bulk workload changes on the Workloads screen: marks (`Space`, `V`,
//! `Esc`), the labels form (`L`), the enforcement picker (`M`), the
//! confirm (typed count, draft check on `d`) and the answer. Planning is
//! local; the only requests are the draft check (on `d` only) and the send
//! (`y`: one re-read, the label creates, the chunks), then one reload.
use super::{
    Cmd, Model, Overlay, StatusLine, load_workloads, redraft_after_write, reload_kind_after_write,
    usage_after_write, workloads_page, writes_refused,
};
use crate::tui::bulk::{
    BulkConfirm, CheckResult, DraftCheck, EnforcePicker, PickerKey, labels_change, labels_form,
    loaded_counts, origin, refresh_hints, toggle_create,
};
use crate::tui::form::{Form, FormEvent};
use crate::tui::model::BulkSource;
use crate::tui::ui::widgets::bulk::confirm_last_scroll;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pce::CacheMode;
use pce::models::WorkloadFilter;
use pce::write::bulk::DRAFT_CHECK_MAX_TARGETS;
use pce::write::{BulkChange, BulkReport, BulkTarget, plan_bulk};
use tokio_util::sync::CancellationToken;

/// `Space` on a workload row: marks or unmarks it.
pub(super) fn toggle_row(model: &mut Model) {
    let Some(href) = model
        .workloads
        .selected_row()
        .map(|r| r.workload.href.clone())
    else {
        model.status = StatusLine::warn("no workload under the cursor");
        return;
    };
    let on = model.workloads.toggle_mark(&href);
    let n = model.workloads.marked.len();
    model.status = StatusLine::info(format!(
        "{} · {n} marked · L labels · M enforcement · Esc clears",
        if on { "marked" } else { "unmarked" }
    ));
}

/// `V`: marks the group under the cursor (grouped) or every workload shown
/// (flat); again: unmarks them.
pub(super) fn toggle_scope(model: &mut Model) {
    let scope = model.workloads.mark_scope();
    if scope.is_empty() {
        model.status = StatusLine::warn("no workloads to mark");
        return;
    }
    let what = if model.workloads.grouped {
        "the group"
    } else {
        "every workload shown"
    };
    let (n, on) = model.workloads.toggle_marks(&scope);
    let total = model.workloads.marked.len();
    model.status = StatusLine::info(format!(
        "{} {n} ({what}) · {total} marked",
        if on { "marked" } else { "unmarked" }
    ));
}

/// More marks than this: `Esc` asks once before clearing them.
const ESC_ASKS_ABOVE: usize = 20;

/// `Esc` on Workloads (no search open): closes the detail pane first; else
/// clears the marks (more than 20: on a second `Esc`). `false`: nothing to
/// do here.
pub(super) fn esc(model: &mut Model) -> bool {
    let w = &mut model.workloads;
    if w.show_detail {
        w.show_detail = false;
        return true;
    }
    let n = w.marked.len();
    if n == 0 {
        return false;
    }
    if n > ESC_ASKS_ABOVE && !w.esc_armed {
        w.esc_armed = true;
        model.status = StatusLine::warn(format!("Esc again clears {n} marks"));
        return true;
    }
    w.esc_armed = false;
    w.clear_marks();
    model.status = StatusLine::info(format!("{n} marks cleared"));
    true
}

/// The targets of `L` / `M` as the plan sees them, with what names them;
/// `None` (and why on the status line) when writes are off or nothing is
/// selected.
fn targets(model: &mut Model) -> Option<(Vec<BulkTarget>, String)> {
    if writes_refused(model) {
        return None;
    }
    let Some(idx) = model.index().cloned() else {
        model.status = StatusLine::info("the name index is not loaded yet");
        return None;
    };
    let (ws, what, source) = model.workloads.bulk_targets();
    if ws.is_empty() {
        model.status = StatusLine::warn(match source {
            BulkSource::Marked => "the marked workloads are no longer loaded",
            _ => "no workload under the cursor",
        });
        return None;
    }
    let targets = ws.iter().map(|w| BulkTarget::of(w, &idx)).collect();
    Some((targets, what))
}

/// `L`: the labels form for the targets.
pub(super) fn open_labels(model: &mut Model) {
    let Some((targets, what)) = targets(model) else {
        return;
    };
    let Some(idx) = model.index().cloned() else {
        return;
    };
    let order = model.limits.key_order();
    let n = targets.len();
    model.overlay = Overlay::Form(labels_form(targets, what, &order, &idx));
    model.status = StatusLine::info(format!(
        "labels for {n} workload{}: empty keeps, - removes; Enter reviews, nothing is sent yet",
        if n == 1 { "" } else { "s" }
    ));
}

/// `M`: the enforcement picker for the targets.
pub(super) fn open_enforcement(model: &mut Model) {
    let Some((targets, what)) = targets(model) else {
        return;
    };
    model.overlay = Overlay::EnforcePicker(Box::new(EnforcePicker::new(targets, what)));
    model.status =
        StatusLine::info("pick the mode (and level); Enter reviews, nothing is sent yet");
}

/// The confirm for `change` over `targets`, or why not (nothing to change
/// stays where it is: `Err` with the summary).
fn confirm(model: &mut Model, targets: &[BulkTarget], change: &BulkChange) -> Result<(), String> {
    let plan = plan_bulk(
        targets,
        change,
        WorkloadFilter::default(),
        Some(origin(&model.profile)),
    )
    .map_err(|e| e.to_string())?;
    if plan.items.is_empty() {
        return Err(format!("nothing to change: {}", plan.summary()));
    }
    let loaded = (change.enforcement.is_some() && model.traffic.data.is_some())
        .then(|| loaded_counts(model.traffic.loaded_flows(), &plan.hrefs()));
    let needs = plan.needs_count();
    let n = plan.items.len();
    model.status = if needs {
        StatusLine::warn(format!(
            "review the change: type {n} (the workload count), then y; Esc cancels"
        ))
    } else {
        StatusLine::info("review the change: y sends it, Esc cancels")
    };
    let org = model.profile.org_id;
    model.overlay = Overlay::ConfirmBulk(Box::new(BulkConfirm::new(plan, loaded, org)));
    Ok(())
}

pub(super) fn picker_key(model: &mut Model, mut p: Box<EnforcePicker>, k: KeyEvent) -> Vec<Cmd> {
    match p.key(k) {
        PickerKey::Continue => model.overlay = Overlay::EnforcePicker(p),
        PickerKey::Cancel => {
            model.overlay = Overlay::None;
            model.status = StatusLine::info("cancelled, nothing sent");
        }
        PickerKey::Review => {
            let change = p.change();
            if let Err(e) = confirm(model, &p.targets, &change) {
                model.status = StatusLine::warn(e);
                model.overlay = Overlay::EnforcePicker(p);
            }
        }
    }
    Vec::new()
}

/// A key in the labels form: `Ctrl-A` creates the typed label (or no
/// longer), `Enter` plans and opens the confirm, the rest edits.
pub(super) fn labels_key(model: &mut Model, mut form: Form, k: KeyEvent) -> Vec<Cmd> {
    let idx = model.index().cloned().unwrap_or_default();
    if k.code == KeyCode::Char('a') && k.modifiers.contains(KeyModifiers::CONTROL) {
        match toggle_create(&mut form, &idx) {
            Ok(s) => {
                form.error = None;
                model.status = StatusLine::info(s);
            }
            Err(e) => model.status = StatusLine::warn(e),
        }
        model.overlay = Overlay::Form(form);
        return Vec::new();
    }
    match form.on_key(k) {
        FormEvent::Continue | FormEvent::NewRuleset => {
            refresh_hints(&mut form, &idx);
            model.overlay = Overlay::Form(form);
        }
        FormEvent::Cancel => {
            model.overlay = Overlay::None;
            model.status = StatusLine::info("cancelled, nothing sent");
        }
        FormEvent::Submit => {
            let crate::tui::edit::EditIntent::BulkLabels(ctx) = &form.intent else {
                model.overlay = Overlay::None;
                return Vec::new();
            };
            let targets = ctx.targets.clone();
            let result = labels_change(&form, &idx).and_then(|c| {
                let c = c.ok_or_else(|| {
                    "nothing to change: type a value to set, or - to remove a key".to_string()
                })?;
                confirm(model, &targets, &c)
            });
            if let Err(e) = result {
                form.error = Some(e);
                model.overlay = Overlay::Form(form);
            }
        }
    }
    Vec::new()
}

pub(super) fn confirm_key(model: &mut Model, mut c: Box<BulkConfirm>, k: KeyEvent) -> Vec<Cmd> {
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    let last = u16::try_from(confirm_last_scroll(model, &c, area)).unwrap_or(u16::MAX);
    let n = c.plan.items.len();
    match k.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            if !c.count_ok() {
                model.status = StatusLine::warn(format!(
                    "type {n} (the number of workloads) before y: nothing sent"
                ));
                model.overlay = Overlay::ConfirmBulk(c);
                return Vec::new();
            }
            // A move the enforcement overlay planned: readiness again, as
            // it is now (data may have landed since the review).
            if let Some(why) = super::enforce::recheck_send(model) {
                if let DraftCheck::Running { cancel, .. } = &c.check {
                    cancel.cancel();
                }
                model.enforce_send = None;
                model.overlay = Overlay::None;
                model.status = StatusLine::warn(why);
                return Vec::new();
            }
            if let DraftCheck::Running { cancel, .. } = &c.check {
                cancel.cancel();
            }
            let mut plan = c.plan;
            if plan.needs_count() {
                plan.confirmed = c.typed.parse().ok();
            }
            let writes = plan.requests(model.profile.org_id).len();
            model.overlay = Overlay::None;
            model.writes.in_flight = true;
            model.batch_sending = true;
            model.batch_from_gen = false;
            model.status = StatusLine::info(format!(
                "sending: 1 re-read, then {writes} request{} …",
                if writes == 1 { "" } else { "s" }
            ));
            // Planned by the enforcement overlay: sent with its change record.
            if let Some(send) = model.enforce_send.take() {
                return vec![Cmd::EnforceApp {
                    plan: Box::new(plan),
                    send,
                }];
            }
            vec![Cmd::ExecuteBulk(Box::new(plan))]
        }
        KeyCode::Esc => {
            if let DraftCheck::Running { cancel, .. } = &c.check {
                cancel.cancel();
                c.check = DraftCheck::Idle;
                model.status = StatusLine::info("draft check cancelled");
                model.overlay = Overlay::ConfirmBulk(c);
            } else {
                model.overlay = Overlay::None;
                model.status = StatusLine::info("cancelled, nothing sent");
            }
            Vec::new()
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            if let DraftCheck::Running { cancel, .. } = &c.check {
                cancel.cancel();
            }
            model.overlay = Overlay::None;
            model.status = StatusLine::info("cancelled, nothing sent");
            Vec::new()
        }
        KeyCode::Char('d') => {
            let cmds = start_check(model, &mut c);
            model.overlay = Overlay::ConfirmBulk(c);
            cmds
        }
        KeyCode::Char(d) if d.is_ascii_digit() && c.plan.needs_count() => {
            if c.typed.len() < 9 {
                c.typed.push(d);
            }
            model.overlay = Overlay::ConfirmBulk(c);
            Vec::new()
        }
        KeyCode::Backspace => {
            c.typed.pop();
            model.overlay = Overlay::ConfirmBulk(c);
            Vec::new()
        }
        KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
            c.typed.clear();
            model.overlay = Overlay::ConfirmBulk(c);
            Vec::new()
        }
        KeyCode::Down | KeyCode::Char('j') => {
            c.scroll = (c.scroll + 1).min(last);
            model.overlay = Overlay::ConfirmBulk(c);
            Vec::new()
        }
        KeyCode::Up | KeyCode::Char('k') => {
            c.scroll = c.scroll.saturating_sub(1);
            model.overlay = Overlay::ConfirmBulk(c);
            Vec::new()
        }
        _ => {
            model.overlay = Overlay::ConfirmBulk(c);
            Vec::new()
        }
    }
}

/// `d`: the draft check, when it applies and is not running. One Explorer
/// query over the traffic filter's window, the targets as destinations.
fn start_check(model: &mut Model, c: &mut BulkConfirm) -> Vec<Cmd> {
    if !c.checkable() {
        model.status = StatusLine::info("the draft check is for an enforcement change");
        return Vec::new();
    }
    if matches!(c.check, DraftCheck::Running { .. }) {
        model.status = StatusLine::info("the draft check is running · Esc cancels it");
        return Vec::new();
    }
    let hrefs = c.plan.hrefs();
    if hrefs.len() > DRAFT_CHECK_MAX_TARGETS {
        model.status = StatusLine::warn(format!(
            "the draft check covers at most {DRAFT_CHECK_MAX_TARGETS} workloads ({} here): narrow the selection",
            hrefs.len()
        ));
        return Vec::new();
    }
    let filter = &model.traffic.filter;
    let window = match filter.window_duration() {
        Ok(w) => w,
        Err(e) => {
            model.status = StatusLine::warn(e);
            return Vec::new();
        }
    };
    model.bulk_check_seq += 1;
    let seq = model.bulk_check_seq;
    let cancel = CancellationToken::new();
    c.check = DraftCheck::Running {
        seq,
        cancel: cancel.clone(),
    };
    model.status = StatusLine::info(format!(
        "draft check: 1 Explorer query over {} (POST, polls, draft PUT, download) · Esc cancels",
        filter.window
    ));
    vec![Cmd::BulkDraftCheck {
        seq,
        hrefs,
        window,
        window_text: filter.window.clone(),
        max_results: filter.max_results,
        cancel,
    }]
}

/// The draft check's answer, into the confirm that asked (else dropped).
pub(super) fn check_done(model: &mut Model, seq: u64, result: Result<CheckResult, String>) {
    let Overlay::ConfirmBulk(c) = &mut model.overlay else {
        return;
    };
    if !matches!(c.check, DraftCheck::Running { seq: s, .. } if s == seq) {
        return;
    }
    match result {
        Ok(r) => {
            model.status = if r.blocked.is_empty() {
                StatusLine::info(format!("draft check: {}", r.text()))
            } else {
                StatusLine::warn(format!("draft check: {}", r.text()))
            };
            c.check = DraftCheck::Done(Box::new(r));
        }
        Err(e) => {
            model.status = StatusLine::error(format!("draft check failed: {e}"));
            c.check = DraftCheck::Failed(e);
        }
    }
}

/// A sent bulk change's answer: the status line and the report (opened
/// when it did not change everything, else `:batch` shows it); the marks
/// cleared after a full success; one reload of the workloads (the label
/// list too when a label was created); after a label change what holds a
/// label and the draft decisions follow, as after any relabel.
pub(super) fn bulk_done(model: &mut Model, report: &BulkReport) -> Vec<Cmd> {
    model.writes.in_flight = false;
    model.batch_sending = false;
    model.batch_from_gen = false;
    model.batch_report = Some(report.lines());
    let stopped = report.stopped();
    let text = report.status_text();
    model.status = if stopped {
        StatusLine::error(text)
    } else if report.dropped() > 0 {
        StatusLine::warn(text)
    } else {
        StatusLine::info(text)
    };
    let mut cmds = Vec::new();
    if report.touched() {
        if !stopped {
            model.workloads.clear_marks();
        }
        if report.labels_changed {
            cmds.extend(redraft_after_write(model, false, false));
            cmds.extend(usage_after_write(model));
        }
        if report.labels_created {
            cmds.extend(reload_kind_after_write(
                model,
                pce::write::ObjectKind::Label,
            ));
        }
        // Once, whatever the chunk count; the report stays on the status line.
        model.workloads.keep_status = true;
        cmds.extend(load_workloads(model, CacheMode::Bypass));
    }
    if model.overlay == Overlay::None && stopped {
        model.overlay = Overlay::BatchReport { scroll: 0 };
    } else {
        model.status.text.push_str(" · :batch shows the report");
    }
    let page = workloads_page(model);
    model.workloads.ensure_visible(page);
    cmds
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::edit::EditIntent;
    use crate::tui::model::{Screen, WritesState};
    use crate::tui::msg::Msg;
    use crate::tui::test_support::model_with_workloads;
    use crate::tui::update::update;
    use pce::write::bulk::{ItemResult, ItemStatus};
    use pce::write::{EnforcementMode, LabelSet};

    fn key(c: char) -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    fn code(c: KeyCode) -> Msg {
        Msg::Key(KeyEvent::new(c, KeyModifiers::NONE))
    }

    fn ctrl(c: char) -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
    }

    /// The key asks for nothing: no request.
    fn quiet(m: &mut Model, msg: Msg) {
        assert_eq!(update(m, msg), Vec::<Cmd>::new());
    }

    fn on_workloads(writes: bool) -> Model {
        let mut m = model_with_workloads();
        m.screen = Screen::Workloads;
        if writes {
            m.writes = WritesState::on();
        }
        m
    }

    fn marked(m: &Model) -> Vec<&str> {
        m.workloads.marked.iter().map(String::as_str).collect()
    }

    fn picker(m: &Model) -> &EnforcePicker {
        match &m.overlay {
            Overlay::EnforcePicker(p) => p,
            other => panic!("{other:?}"),
        }
    }

    fn confirm(m: &Model) -> &BulkConfirm {
        match &m.overlay {
            Overlay::ConfirmBulk(c) => c,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn space_v_and_esc_mark_and_clear_without_a_request() {
        let mut m = on_workloads(false);
        let first = m.workloads.selected_row().unwrap().workload.href.clone();
        quiet(&mut m, key(' '));
        assert_eq!(marked(&m), vec![first.as_str()]);
        assert!(m.workloads.selected_row().unwrap().marked.get(), "●");
        quiet(&mut m, key(' '));
        assert!(marked(&m).is_empty());
        // V: every workload shown; again: none.
        quiet(&mut m, key('V'));
        assert_eq!(m.workloads.marked.len(), 4);
        assert!(m.status.text.starts_with("marked 4 (every workload shown)"));
        quiet(&mut m, key('V'));
        assert!(m.workloads.marked.is_empty());
        quiet(&mut m, key(' '));
        quiet(&mut m, code(KeyCode::Esc));
        assert!(m.workloads.marked.is_empty());
        assert_eq!(m.status.text, "1 marks cleared");
    }

    #[test]
    fn marks_survive_regroup_filter_and_sort() {
        let mut m = on_workloads(false);
        quiet(&mut m, key(' ')); // host-1
        quiet(&mut m, key('j'));
        quiet(&mut m, key(' ')); // the next row
        let before: Vec<String> = m.workloads.marked.iter().cloned().collect();
        assert_eq!(before.len(), 2);
        quiet(&mut m, key('s'));
        quiet(&mut m, key('x'));
        assert_eq!(
            m.workloads.marked.iter().cloned().collect::<Vec<_>>(),
            before
        );
        // The grouped rows show the same marks (shared rows).
        let shown = m
            .workloads
            .sections
            .rows()
            .iter()
            .filter_map(|r| r.member())
            .filter(|r| r.marked.get())
            .count();
        assert_eq!(shown, 2);
        crate::tui::update::apply_workload_labels(&mut m, "app=nope").unwrap();
        assert_eq!(
            m.workloads.marked.len(),
            2,
            "hidden by a filter, still marked"
        );
        crate::tui::update::apply_workload_labels(&mut m, "").unwrap();
        quiet(&mut m, key('x'));
        assert!(
            m.workloads
                .table
                .rows()
                .iter()
                .filter(|r| r.marked.get())
                .count()
                == 2
        );
        // A rebuild (h) keeps them too.
        quiet(&mut m, key('h'));
        quiet(&mut m, key('h'));
        assert_eq!(m.workloads.marked.len(), 2);
    }

    #[test]
    fn grouped_v_marks_the_group_under_the_cursor() {
        let mut m = on_workloads(false);
        quiet(&mut m, key('x'));
        m.workloads.sections.home(); // shop | prod header
        quiet(&mut m, key('V'));
        let mut got = marked(&m);
        got.sort_unstable();
        assert_eq!(
            got,
            vec!["/orgs/5/workloads/aaaa-1", "/orgs/5/workloads/aaaa-2"]
        );
        // On a member of it: the same group, unmarked again.
        quiet(&mut m, key('j'));
        quiet(&mut m, key('V'));
        assert!(m.workloads.marked.is_empty());
    }

    #[test]
    fn targets_are_the_marked_else_the_header_group_else_the_row() {
        let mut m = on_workloads(true);
        // The row.
        let (ws, what, src) = m.workloads.bulk_targets();
        assert_eq!((ws.len(), src), (1, BulkSource::Row));
        assert_eq!(what, "host-1.example");
        // The header's group.
        quiet(&mut m, key('x'));
        m.workloads.sections.home();
        let (ws, what, src) = m.workloads.bulk_targets();
        assert_eq!((ws.len(), src), (2, BulkSource::Group));
        assert_eq!(what, "group shop | prod");
        // Marked win over the cursor.
        m.workloads.toggle_mark("/orgs/5/workloads/aaaa-4");
        let (ws, what, src) = m.workloads.bulk_targets();
        assert_eq!((ws.len(), src), (1, BulkSource::Marked));
        assert_eq!(what, "1 marked");
        assert_eq!(ws[0].href, "/orgs/5/workloads/aaaa-4");
    }

    #[test]
    fn writes_off_refuse_l_and_m() {
        let mut m = on_workloads(false);
        quiet(&mut m, key('L'));
        assert_eq!(m.overlay, Overlay::None);
        quiet(&mut m, key('M'));
        assert_eq!(m.overlay, Overlay::None);
        assert!(m.status.text.contains("writes"), "{}", m.status.text);
    }

    #[test]
    fn full_on_several_needs_the_typed_count_before_y() {
        let mut m = on_workloads(true);
        quiet(&mut m, key('V')); // all four
        quiet(&mut m, key('M'));
        assert_eq!(picker(&m).what, "4 marked");
        // To full: j until full.
        while EnforcementMode::ALL[picker(&m).mode] != EnforcementMode::Full {
            quiet(&mut m, key('j'));
        }
        assert_eq!(
            picker(&m).summary,
            "4 workloads: 1 selective → full · 1 visibility_only → full · 1 already full (skipped) · 1 unmanaged (skipped)"
        );
        quiet(&mut m, code(KeyCode::Enter));
        let c = confirm(&m);
        assert!(c.plan.needs_count());
        assert_eq!(c.plan.items.len(), 2);
        assert_eq!(c.loaded, None, "no traffic loaded");
        // y is refused until the count is typed; a wrong count too.
        quiet(&mut m, key('y'));
        assert!(matches!(m.overlay, Overlay::ConfirmBulk(_)));
        quiet(&mut m, key('3'));
        quiet(&mut m, key('y'));
        assert!(matches!(m.overlay, Overlay::ConfirmBulk(_)));
        quiet(&mut m, code(KeyCode::Backspace));
        quiet(&mut m, key('2'));
        let cmds = update(&mut m, key('y'));
        let [Cmd::ExecuteBulk(plan)] = &cmds[..] else {
            panic!("{cmds:?}");
        };
        assert_eq!(plan.confirmed, Some(2));
        assert_eq!(plan.origin.as_ref().unwrap().profile, "demo");
        assert!(m.batch_sending && m.writes.in_flight);
        // Quitting waits for the answer.
        quiet(&mut m, key('q'));
        assert!(!m.quit);
    }

    #[test]
    fn the_draft_check_runs_only_on_d_and_esc_cancels_it() {
        let mut m = on_workloads(true);
        quiet(&mut m, key('V'));
        quiet(&mut m, key('M'));
        while EnforcementMode::ALL[picker(&m).mode] != EnforcementMode::Full {
            quiet(&mut m, key('j'));
        }
        quiet(&mut m, code(KeyCode::Enter));
        // Scrolling and typing ask for nothing.
        for c in ['j', 'k', '1'] {
            quiet(&mut m, key(c));
        }
        let cmds = update(&mut m, key('d'));
        let [Cmd::BulkDraftCheck { seq, hrefs, .. }] = &cmds[..] else {
            panic!("{cmds:?}");
        };
        assert_eq!(hrefs.len(), 2);
        let seq = *seq;
        // A second d while it runs: nothing more.
        quiet(&mut m, key('d'));
        // Esc cancels the check, not the confirm.
        let cancel = match &confirm(&m).check {
            DraftCheck::Running { cancel, .. } => cancel.clone(),
            other => panic!("{other:?}"),
        };
        quiet(&mut m, code(KeyCode::Esc));
        assert!(cancel.is_cancelled());
        assert_eq!(confirm(&m).check, DraftCheck::Idle);
        // A late answer for it is dropped.
        quiet(
            &mut m,
            Msg::BulkCheckDone {
                seq,
                result: Err("cancelled".into()),
            },
        );
        assert_eq!(confirm(&m).check, DraftCheck::Idle);
        // A new check's answer lands.
        let cmds = update(&mut m, key('d'));
        let [Cmd::BulkDraftCheck { seq, .. }] = &cmds[..] else {
            panic!("{cmds:?}");
        };
        let r = CheckResult::new(&[], &[], 5, false, "24h".into());
        quiet(
            &mut m,
            Msg::BulkCheckDone {
                seq: *seq,
                result: Ok(r.clone()),
            },
        );
        assert_eq!(confirm(&m).check, DraftCheck::Done(Box::new(r)));
        assert!(m.status.text.contains(
            "0 of 0 flows (24h, in and out) would be blocked: 0 in · 0 out · ≈5 requests"
        ));
        // Esc now closes the confirm; nothing sent.
        quiet(&mut m, code(KeyCode::Esc));
        assert_eq!(m.overlay, Overlay::None);
    }

    #[test]
    fn a_label_change_needs_no_count_and_ctrl_a_creates_the_label() {
        let mut m = on_workloads(true);
        quiet(&mut m, key('V'));
        quiet(&mut m, key('L'));
        let Overlay::Form(f) = &m.overlay else {
            panic!("{:?}", m.overlay);
        };
        assert!(matches!(f.intent, EditIntent::BulkLabels(_)));
        assert_eq!(f.fields[0].label, "app");
        // app: a value no label has.
        for c in "brandnew".chars() {
            quiet(&mut m, key(c));
        }
        quiet(&mut m, code(KeyCode::Enter));
        let Overlay::Form(f) = &m.overlay else {
            panic!("{:?}", m.overlay);
        };
        assert!(f.error.as_deref().unwrap().contains("Ctrl-A creates"));
        quiet(&mut m, ctrl('a'));
        quiet(&mut m, code(KeyCode::Enter));
        let c = confirm(&m);
        assert!(!c.plan.needs_count());
        assert!(!c.checkable(), "no draft check for labels");
        assert_eq!(c.plan.creates.len(), 1);
        assert_eq!(
            c.plan.change.labels.set,
            vec![LabelSet::Create(pce::write::LabelDraft::new(
                "app", "brandnew"
            ))]
        );
        assert_eq!(c.plan.items.len(), 4, "managed and unmanaged");
        let cmds = update(&mut m, key('y'));
        assert!(matches!(&cmds[..], [Cmd::ExecuteBulk(_)]));
    }

    fn report(stopped: bool, labels: bool) -> BulkReport {
        BulkReport {
            target: "demo (pce.example, org 5)".into(),
            change: "x".into(),
            refused: None,
            reads: 1,
            steps: vec![pce::write::bulk::BulkStep {
                method: pce::WriteMethod::Put,
                path: "/orgs/5/workloads/bulk_update".into(),
                summary: "bulk update 2 workloads (1/1)".into(),
                status: if stopped {
                    pce::write::bulk::StepStatus::Partial
                } else {
                    pce::write::bulk::StepStatus::Written
                },
                href: None,
                http_status: Some(200),
                error: None,
                audit_failed: None,
            }],
            items: vec![
                ItemResult {
                    href: "/orgs/5/workloads/aaaa-1".into(),
                    name: "host-1.example".into(),
                    change: "a → b".into(),
                    status: ItemStatus::Updated,
                    error: None,
                },
                ItemResult {
                    href: "/orgs/5/workloads/aaaa-2".into(),
                    name: "host-2.example".into(),
                    change: "a → b".into(),
                    status: if stopped {
                        ItemStatus::Failed
                    } else {
                        ItemStatus::Updated
                    },
                    error: stopped.then(|| "bad".to_string()),
                },
            ],
            labels_changed: labels,
            labels_created: false,
            reread_skipped: None,
            failure: stopped.then_some(pce::write::bulk::BulkFailure::Partial),
        }
    }

    #[test]
    fn a_sent_change_clears_the_marks_and_reloads_the_workloads_once() {
        let mut m = on_workloads(true);
        quiet(&mut m, key('V'));
        m.batch_sending = true;
        m.writes.in_flight = true;
        let cmds = update(&mut m, Msg::BulkDone(Box::new(report(false, false))));
        assert_eq!(cmds, vec![Cmd::LoadWorkloads(CacheMode::Bypass)]);
        assert!(m.workloads.marked.is_empty());
        assert!(!m.batch_sending && !m.writes.in_flight);
        assert_eq!(m.overlay, Overlay::None);
        assert!(
            m.status.text.starts_with("updated 2 of 2"),
            "{}",
            m.status.text
        );
        assert!(m.batch_report.as_ref().unwrap()[0].starts_with("sent to demo"));
        // The reload keeps the report on the status line.
        let status = m.status.text.clone();
        let raw = m.workloads.raw.clone();
        quiet(&mut m, Msg::WorkloadsLoaded(Ok((raw, 0, false))));
        assert_eq!(m.status.text, status);
    }

    #[test]
    fn a_failed_workload_keeps_the_marks_and_opens_the_report() {
        let mut m = on_workloads(true);
        quiet(&mut m, key('V'));
        let cmds = update(&mut m, Msg::BulkDone(Box::new(report(true, true))));
        assert!(cmds.contains(&Cmd::LoadWorkloads(CacheMode::Bypass)));
        assert_eq!(
            cmds.iter()
                .filter(|c| matches!(c, Cmd::LoadWorkloads(_)))
                .count(),
            1
        );
        assert_eq!(m.workloads.marked.len(), 4, "kept to retry");
        assert_eq!(m.overlay, Overlay::BatchReport { scroll: 0 });
        assert!(
            m.batch_report
                .as_ref()
                .unwrap()
                .iter()
                .any(|l| l.contains("FAILED: bad"))
        );
    }

    #[test]
    fn the_confirm_counts_the_loaded_flows_to_the_targets() {
        use crate::tui::test_support::{model_with_flows, with_workloads};
        let mut m = with_workloads(model_with_flows());
        m.screen = Screen::Workloads;
        m.writes = WritesState::on();
        // Every flow destination is a target: the counts are the loaded ones.
        let hrefs: Vec<String> = m
            .traffic
            .loaded_flows()
            .iter()
            .filter_map(|f| f.dst.workload_href.clone())
            .collect();
        let expect = loaded_counts(m.traffic.loaded_flows(), &hrefs);
        let unique: std::collections::BTreeSet<&String> = hrefs.iter().collect();
        let targets: Vec<BulkTarget> = unique
            .into_iter()
            .map(|h| BulkTarget {
                href: h.clone(),
                name: h.clone(),
                managed: true,
                enforcement: Some("visibility_only".into()),
                visibility: None,
                labels: Vec::new(),
            })
            .collect();
        super::confirm(
            &mut m,
            &targets,
            &BulkChange::enforce(pce::write::EnforcementMode::Full, None),
        )
        .unwrap();
        let c = confirm(&m);
        assert_eq!(c.loaded, Some(expect));
        assert!(
            expect.in_blocked + expect.in_potentially > 0,
            "the fixture has blocked flows"
        );
    }

    #[test]
    fn esc_closes_the_detail_first_and_asks_once_above_twenty_marks() {
        let mut m = on_workloads(false);
        quiet(&mut m, key(' '));
        m.workloads.show_detail = true;
        quiet(&mut m, code(KeyCode::Esc));
        assert!(!m.workloads.show_detail);
        assert_eq!(m.workloads.marked.len(), 1, "the detail went first");
        let many: Vec<String> = (0..25).map(|i| format!("/orgs/5/workloads/x{i}")).collect();
        m.workloads.toggle_marks(&many);
        quiet(&mut m, code(KeyCode::Esc));
        assert_eq!(m.status.text, "Esc again clears 26 marks");
        assert_eq!(m.workloads.marked.len(), 26);
        // Another key in between: asked again.
        quiet(&mut m, key('j'));
        quiet(&mut m, code(KeyCode::Esc));
        assert_eq!(m.workloads.marked.len(), 26);
        // A screen switch and back in between: asked again too.
        let _ = update(&mut m, key('1'));
        let _ = update(&mut m, key('4'));
        assert!(!m.workloads.esc_armed);
        quiet(&mut m, code(KeyCode::Esc));
        assert_eq!(m.workloads.marked.len(), 26);
        assert!(m.workloads.esc_armed);
        quiet(&mut m, code(KeyCode::Esc));
        assert!(m.workloads.marked.is_empty());
        assert_eq!(m.status.text, "26 marks cleared");
    }

    #[test]
    fn the_summary_counts_marks_the_filter_hides() {
        let mut m = on_workloads(false);
        quiet(&mut m, key('V'));
        assert_eq!(m.workloads.marked_hidden(), 0);
        crate::tui::update::apply_workload_labels(&mut m, "app=shop").unwrap();
        assert_eq!(m.workloads.marked_hidden(), 2);
        let out = crate::tui::test_support::render_to_string(&m, 120, 30);
        assert!(out.contains("4 marked (2 hidden by the filter)"), "{out}");
    }

    #[test]
    fn an_unknown_outcome_says_it_may_have_applied_and_reloads() {
        let mut m = on_workloads(true);
        quiet(&mut m, key('V'));
        let mut r = report(true, true);
        r.steps[0].status = pce::write::bulk::StepStatus::Unanswered;
        r.steps[0].error = Some(
            "answered 204 without per-workload results: it may have applied, check the PCE".into(),
        );
        for i in &mut r.items {
            i.status = ItemStatus::Unanswered;
            i.error = None;
        }
        r.failure = Some(pce::write::bulk::BulkFailure::Unanswered);
        let cmds = update(&mut m, Msg::BulkDone(Box::new(r)));
        assert!(
            cmds.contains(&Cmd::LoadWorkloads(CacheMode::Bypass)),
            "{cmds:?}"
        );
        assert!(
            m.status
                .text
                .contains("2 not answered · it may have applied: check the PCE"),
            "{}",
            m.status.text
        );
        assert_eq!(m.overlay, Overlay::BatchReport { scroll: 0 });
        assert!(
            m.batch_report
                .as_ref()
                .unwrap()
                .iter()
                .any(|l| l.contains("NOT ANSWERED: answered 204 without per-workload results")),
            "{:?}",
            m.batch_report
        );
        assert_eq!(m.workloads.marked.len(), 4, "kept: the outcome is unknown");
    }

    /// Workloads with `n` copies of the first fixture VEN (visibility_only).
    fn with_n(n: usize) -> Model {
        let mut m = on_workloads(true);
        let base = m.workloads.raw[0].clone();
        m.workloads.raw = (0..n)
            .map(|i| {
                let mut w = base.clone();
                w.href = format!("/orgs/5/workloads/n{i}");
                w.hostname = Some(format!("n{i}.example"));
                w.enforcement_mode = Some("visibility_only".into());
                w
            })
            .collect();
        let order = m.limits.key_order();
        m.workloads.rebuild(&order);
        m
    }

    #[test]
    fn more_than_twenty_five_need_the_count_for_any_change() {
        // 26, idle: the count, worded by size.
        let mut m = with_n(26);
        quiet(&mut m, key('V'));
        quiet(&mut m, key('M'));
        while EnforcementMode::ALL[picker(&m).mode] != EnforcementMode::Idle {
            quiet(&mut m, key('k'));
        }
        quiet(&mut m, code(KeyCode::Enter));
        assert!(confirm(&m).plan.needs_count());
        assert_eq!(
            confirm(&m).plan.count_reason().unwrap(),
            "26 workloads (more than 25)"
        );
        quiet(&mut m, key('y'));
        assert!(
            matches!(m.overlay, Overlay::ConfirmBulk(_)),
            "y waits for the count"
        );
        let out = crate::tui::test_support::render_to_string(&m, 100, 30);
        assert!(
            out.contains("26 workloads (more than 25): type 26 to confirm"),
            "{out}"
        );
        quiet(&mut m, key('2'));
        quiet(&mut m, key('6'));
        let cmds = update(&mut m, key('y'));
        let [Cmd::ExecuteBulk(plan)] = &cmds[..] else {
            panic!("{cmds:?}");
        };
        assert_eq!(plan.confirmed, Some(26));
        // 26, a relabel: the count too.
        let mut m = with_n(26);
        quiet(&mut m, key('V'));
        quiet(&mut m, key('L'));
        quiet(&mut m, key('-')); // remove app
        quiet(&mut m, code(KeyCode::Enter));
        assert!(confirm(&m).plan.needs_count());
        // 25, idle: none.
        let mut m = with_n(25);
        quiet(&mut m, key('V'));
        quiet(&mut m, key('M'));
        while EnforcementMode::ALL[picker(&m).mode] != EnforcementMode::Idle {
            quiet(&mut m, key('k'));
        }
        quiet(&mut m, code(KeyCode::Enter));
        assert!(!confirm(&m).plan.needs_count());
        let cmds = update(&mut m, key('y'));
        assert!(matches!(&cmds[..], [Cmd::ExecuteBulk(_)]));
    }
}
