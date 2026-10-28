//! Write plumbing: the writes toggle and its refusals, the edit forms and
//! their draft rulesets, `w` on a flow or a service pair, batches and
//! their reports, and the reloads after a write.
use super::*;

/// Why `Ctrl-R` must not create `name`: a loaded draft ruleset (any case)
/// has it already, or several do. `None` when none does, or while the
/// drafts are not loaded (the plan then reads them fresh).
pub(super) fn existing_ruleset(model: &Model, name: &str) -> Option<String> {
    let hits = edit::rulesets_named(model.draft_rulesets()?, name);
    let name = name.trim();
    match hits.as_slice() {
        [] => None,
        [_] => Some(format!(
            "ruleset {name:?} exists: Enter adds the rule to it"
        )),
        many => Some(format!(
            "{name:?} names several rulesets: {}; use the href",
            many.iter()
                .map(|rs| rs.href.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// The plan or write a form led to has ended: back where the form came
/// from when nothing else is open (`close_form`); the way back is
/// forgotten either way.
pub(super) fn finish_form(model: &mut Model) {
    if model.overlay == Overlay::None {
        close_form(model);
    } else {
        model.form_from_app = false;
    }
}

/// A form is done, or the plan and confirm it led to: back to the app
/// overlay it was opened from (its view is kept: no request), else to the
/// screen behind it.
pub(super) fn close_form(model: &mut Model) {
    let back = std::mem::take(&mut model.form_from_app) && model.app_view.is_some();
    model.overlay = if back { Overlay::App } else { Overlay::None };
}

/// Sent but not answered, whether or not its audit entry was written.
pub(super) fn is_unanswered(e: &WriteError) -> bool {
    match e {
        WriteError::Unanswered(_) => true,
        WriteError::AuditFailed { write, .. } => is_unanswered(write),
        _ => false,
    }
}

/// Whether a planned batch was planned on this session's profile and PCE
/// (and no switch is on its way).
pub(super) fn batch_for_session(model: &Model, batch: &pce::write::Batch) -> bool {
    let p = &model.profile;
    !model.switching
        && batch
            .origin
            .as_ref()
            .is_some_and(|o| o.profile == p.name && o.host == p.host && o.org_id == p.org_id)
}

/// Whether a planned batch may open its confirm: only over the generator
/// that planned it (still open, still planning).
pub(super) fn batch_lands(model: &Model) -> bool {
    model.overlay == Overlay::Generator && model.gen_view.as_ref().is_some_and(|v| v.planning)
}

/// A batch could not be planned: nothing was sent; the generator that
/// planned it (still open) keeps its state and shows why.
pub(super) fn batch_plan_failed(model: &mut Model, e: &str, planned_here: bool) {
    match model.gen_view.as_mut() {
        Some(v) if planned_here => {
            v.error = Some(e.to_string());
            model.status = StatusLine::error(format!("plan: {e}"));
        }
        _ => fail(model, "plan", e, false),
    }
}

/// The batch confirm or its report is done: back to the generator (the
/// drafts it reads load again after a write: one request, counted), else
/// to the screen.
pub(super) fn batch_closed(model: &mut Model) -> Vec<Cmd> {
    if !std::mem::take(&mut model.batch_from_gen) || model.gen_view.is_none() {
        model.overlay = Overlay::None;
        return Vec::new();
    }
    model.overlay = Overlay::Generator;
    // The batch's outcome stays on the status line, whatever loads.
    let status = model.status.clone();
    let cmds = generator::ensure_drafts(model);
    // Rows, gap and keys again: traffic may have landed meanwhile.
    generator::rebuild_gen(model);
    if model.overlay == Overlay::Generator {
        model.status = status;
    }
    cmds
}

/// A written step whose audit entry could not be written (why).
pub(super) fn not_audited(r: &pce::write::StepResult) -> Option<&str> {
    match r {
        pce::write::StepResult::Written(w) => w.audit_failed.as_deref(),
        _ => None,
    }
}

/// `wrote 5 of 7 (pending); step 6 failed: …; 1 not sent`, then what the
/// PCE dropped or warned about per written step, and whether the batch
/// stopped (`Some(true)`), only warned (`Some(false)`) or neither.
pub(super) fn batch_status(results: &[pce::write::StepResult]) -> (String, Option<bool>) {
    use pce::write::StepResult as R;
    let count = |f: fn(&R) -> bool| results.iter().filter(|r| f(r)).count();
    let written = count(|r| matches!(r, R::Written(_)));
    let reused = count(|r| matches!(r, R::Skipped { .. }));
    let not_sent = count(|r| matches!(r, R::NotSent));
    let mut s = format!("wrote {written} of {} (pending)", results.len());
    if reused > 0 {
        s.push_str(&format!("; reused {reused}"));
    }
    let failed = results.iter().enumerate().find_map(|(i, r)| match r {
        R::Failed(e) => Some((i, e)),
        _ => None,
    });
    if let Some((i, e)) = failed {
        if is_unanswered(e) {
            s.push_str(&format!(
                "; step {} not answered: it may have applied; reloading",
                i + 1
            ));
        } else {
            s.push_str(&format!("; step {} failed: {e}", i + 1));
        }
    }
    let unaudited = results
        .iter()
        .enumerate()
        .find_map(|(i, r)| not_audited(r).map(|why| (i, why)));
    if let Some((i, why)) = unaudited {
        s.push_str(&format!(
            "; step {} written but not audited ({why}): batch stopped",
            i + 1
        ));
    }
    if not_sent > 0 {
        s.push_str(&format!("; {not_sent} not sent"));
    }
    let mut noted = false;
    for (i, r) in results.iter().enumerate() {
        let R::Written(w) = r else { continue };
        if !w.dropped_fields.is_empty() {
            noted = true;
            s.push_str(&format!(
                "; step {}: the PCE dropped {}",
                i + 1,
                w.dropped_fields.join(", ")
            ));
        }
        for warning in &w.warnings {
            if w.audit_failed.as_ref() == Some(warning) {
                continue;
            }
            noted = true;
            s.push_str(&format!("; step {}: {warning}", i + 1));
        }
    }
    let kind = match (failed.is_some() || unaudited.is_some(), noted) {
        (true, _) => Some(true),
        (false, true) => Some(false),
        (false, false) => None,
    };
    (s, kind)
}

/// The batch report: each step's summary and what became of it (the href
/// written or reused, the failure, not sent), then how to undo and resume.
pub(super) fn batch_report_lines(
    target: &str,
    summaries: &[String],
    results: &[pce::write::StepResult],
) -> Vec<String> {
    use pce::write::StepResult as R;
    let n = results.len();
    let mut out = vec![format!("sent to {target}")];
    for (i, r) in results.iter().enumerate() {
        let what = summaries.get(i).map_or("?", String::as_str);
        out.push(format!("{}/{n} {what}", i + 1));
        out.push(match r {
            R::Written(w) => match &w.audit_failed {
                Some(why) => format!("    written {}, NOT AUDITED: {why}", w.href),
                None => format!("    written {}", w.href),
            },
            R::Skipped { existing_href } => format!("    reused {existing_href}"),
            R::Failed(e) if is_unanswered(e) => {
                format!("    NOT ANSWERED, it may have applied: {e}")
            }
            R::Failed(e) => format!("    FAILED: {e}"),
            R::NotSent => "    not sent".into(),
        });
    }
    let touched = results.iter().any(|r| match r {
        R::Written(_) => true,
        R::Failed(e) => is_unanswered(e),
        _ => false,
    });
    let stopped = results
        .iter()
        .any(|r| matches!(r, R::Failed(_) | R::NotSent) || not_audited(r).is_some());
    out.push(String::new());
    if touched {
        out.push(
            "These are draft changes, not provisioned: p on Policy lists them, R reverts the marked ones."
                .into(),
        );
    }
    if stopped {
        out.push("Planning the batch again resumes it: what was written then exists.".into());
    }
    out
}

/// A sent batch's answer: the status line and the report (opened when the
/// batch stopped, else `:batch` shows it), then one reload of the
/// rulesets (and of the IP lists when one was written, so the index
/// follows) and one draft re-evaluation, whatever the step count. An
/// overlay opened while it was sent stays.
pub(super) fn batch_done(
    model: &mut Model,
    kinds: &[pce::write::ObjectKind],
    summaries: &[String],
    target: &str,
    results: &[pce::write::StepResult],
) -> Vec<Cmd> {
    use pce::write::StepResult as R;
    model.writes.in_flight = false;
    model.batch_sending = false;
    // The runtime dropped `policy:pending`; the next `p` fetches again.
    model.policy.pending = None;
    model.policy.pending_marked.clear();
    // The draft moved: the impact summary is stale (as after one write).
    model.policy.impact.forget();
    let (text, kind) = batch_status(results);
    let stopped = kind == Some(true) || results.iter().any(|r| matches!(r, R::NotSent));
    model.batch_report = Some(batch_report_lines(target, summaries, results));
    model.status = match kind {
        Some(true) => StatusLine::error(text),
        Some(false) => StatusLine::warn(text),
        None => StatusLine::info(text),
    };
    // Written, or sent without an answer (it may have applied).
    let touched = |r: &R| match r {
        R::Written(_) => true,
        R::Failed(e) => is_unanswered(e),
        R::Skipped { .. } | R::NotSent => false,
    };
    let mut cmds = Vec::new();
    if results.iter().any(touched) {
        // Once per batch, not per step.
        let redraft = redraft_after_write(model, true, false);
        let lists = kinds
            .iter()
            .zip(results)
            .any(|(k, r)| *k == pce::write::ObjectKind::IpList && touched(r));
        if lists {
            cmds.extend(reload_kind_after_write(
                model,
                pce::write::ObjectKind::IpList,
            ));
        }
        cmds.extend(reload_after_write(model, pce::write::ObjectKind::Rule));
        cmds.extend(redraft);
    }
    if model.overlay != Overlay::None {
        // Whatever opened meanwhile closes onto the screen.
        model.batch_from_gen = false;
        model.status.text.push_str(" · :batch shows each step");
    } else if stopped {
        model.overlay = Overlay::BatchReport { scroll: 0 };
    } else {
        model.status.text.push_str(" · :batch shows each step");
        cmds.extend(batch_closed(model));
    }
    cmds
}

/// One reload of the written kind's table, past the cache (the writer
/// already invalidated it).
pub(super) fn reload_after_write(model: &mut Model, kind: pce::write::ObjectKind) -> Vec<Cmd> {
    let mut cmds = reload_kind_after_write(model, kind);
    cmds.extend(usage_after_write(model));
    cmds
}

/// Any write may change what holds a label (a workload's labels, a rule's
/// actors, a group's members): the PCE's usage flags are forgotten, and
/// with `u` on asked for again, past the cache. A PCE without flags stays so.
pub(super) fn usage_after_write(model: &mut Model) -> Vec<Cmd> {
    let o = &mut model.objects;
    if o.pce_usage == PceUsage::Unsupported {
        return Vec::new();
    }
    o.pce_usage = PceUsage::Unknown;
    o.rebuild_labels(&model.limits.key_order());
    if !o.unused_only {
        return Vec::new();
    }
    o.pce_usage_loading = true;
    vec![Cmd::LoadLabelUsage(CacheMode::Bypass)]
}

/// `r` on the Labels tab while the PCE's flags are held: they are read
/// again with the labels, past the cache.
pub(super) fn reload_usage_with_labels(model: &mut Model) -> Option<Cmd> {
    let o = &mut model.objects;
    if !matches!(o.pce_usage, PceUsage::Flags(_)) || o.pce_usage_loading {
        return None;
    }
    o.pce_usage_loading = true;
    Some(Cmd::LoadLabelUsage(CacheMode::Bypass))
}

/// A policy write changed the drafts: the held ones go, and the answer to
/// a form's load still on its way (read before the write) is dropped.
pub(super) fn forget_form_drafts(model: &mut Model) {
    model.policy.form_drafts = None;
    model.policy.form_drafts_stale = model.policy.form_drafts_loading;
}

/// The reload of the written kind's own list.
pub(super) fn reload_kind_after_write(model: &mut Model, kind: pce::write::ObjectKind) -> Vec<Cmd> {
    match kind {
        pce::write::ObjectKind::Label => load_objects(model, ObjectKind::Labels, CacheMode::Bypass),
        // Nothing writes label groups yet; the tab has no reload of its own.
        pce::write::ObjectKind::LabelGroup => Vec::new(),
        // Only the CLI writes pairing profiles; the TUI shows none.
        pce::write::ObjectKind::PairingProfile => Vec::new(),
        pce::write::ObjectKind::IpList => load_ip_lists(model, CacheMode::Bypass),
        pce::write::ObjectKind::Service => {
            load_objects(model, ObjectKind::Services, CacheMode::Bypass)
        }
        pce::write::ObjectKind::Workload | pce::write::ObjectKind::WorkloadBulk => {
            load_workloads(model, CacheMode::Bypass)
        }
        // A draft write changes the draft view only; the reload keeps the
        // write's report in the status line.
        pce::write::ObjectKind::RuleSet
        | pce::write::ObjectKind::Rule
        | pce::write::ObjectKind::DenyRule => {
            // The next `w` form loads the changed drafts (one GET).
            forget_form_drafts(model);
            if model.policy.scope != PolicyScope::Draft {
                return Vec::new();
            }
            model.policy.quiet_reload = true;
            load_rule_sets(model, CacheMode::Bypass)
        }
        // Provisioning changes both scopes and the pending list; a revert
        // changes the draft and the pending list.
        pce::write::ObjectKind::Provision | pce::write::ObjectKind::Revert => {
            forget_form_drafts(model);
            model.policy.quiet_reload = true;

            let mut cmds = load_rule_sets(model, CacheMode::Bypass);
            model.policy.pending_loading = true;
            cmds.push(Cmd::LoadPending(CacheMode::Bypass));
            cmds
        }
    }
}

/// `writes on`, or `writes off: <reason>` (just the reason when it already
/// says so, as `WRITES_OFF` does).
pub(super) fn writes_status(writes: &WritesState) -> String {
    if writes.enabled {
        return "writes on".into();
    }
    let reason = writes.refusal();
    if reason.starts_with("writes off") {
        reason
    } else {
        format!("writes off: {reason}")
    }
}

/// Writes off, or a write already on its way: the reason goes to the status
/// line and the key does nothing else.
pub(super) fn writes_refused(model: &mut Model) -> bool {
    if !model.writes.enabled {
        model.status = StatusLine::warn(model.writes.refusal());
        return true;
    }
    if model.writes.in_flight {
        model.status = StatusLine::warn("a write is already in progress");
        return true;
    }
    false
}

/// An open `w` form again from the draft rulesets that just arrived: its
/// ruleset hint, and the pick while the field is empty. Nothing for another
/// overlay (a form closed meanwhile is gone). No request.
pub(super) fn refill_rule_form(model: &mut Model) {
    let Some(n) = model.draft_rulesets().map(<[_]>::len) else {
        return;
    };
    if model.status.text == FORM_DRAFTS_LOADING {
        model.status = StatusLine::info(format!("{n} draft rulesets for the form"));
    }
    let mut overlay = std::mem::replace(&mut model.overlay, Overlay::None);
    if let Overlay::Form(form) = &mut overlay {
        let empty = pce::NameIndex::default();
        let idx = model.index().cloned();
        edit::refill_ruleset(form, model.flow_prefill(), idx.as_deref().unwrap_or(&empty));
    }
    model.overlay = overlay;
}

/// A `w` form needs the draft rulesets: one load (cached) when they are
/// neither held nor on their way. The Policy screen on the draft loads its
/// own list (it shows it); on the active one the drafts are kept apart.
pub(super) fn load_form_drafts(model: &mut Model) -> Vec<Cmd> {
    if !model.is_connected() || model.draft_rulesets().is_some() {
        return Vec::new();
    }
    let cmds = if model.policy.scope == PolicyScope::Draft {
        if model.policy.loading {
            return Vec::new();
        }
        // Its answer must not replace the status line's form hint.
        model.policy.quiet_reload = true;
        load_rule_sets(model, CacheMode::Use)
    } else {
        if model.policy.form_drafts_loading {
            return Vec::new();
        }
        model.policy.form_drafts_loading = true;
        vec![Cmd::LoadRuleSets {
            scope: PolicyScope::Draft,
            mode: CacheMode::Use,
        }]
    };
    model.status = StatusLine::info(FORM_DRAFTS_LOADING);
    cmds
}

/// The status line while a `w` form's draft rulesets load.
pub(super) const FORM_DRAFTS_LOADING: &str = "loading the draft rulesets for the form · 1 req";

/// A create or update opens its form; a delete goes straight to planning.
pub(super) fn begin_edit(model: &mut Model, intent: EditIntent) -> Vec<Cmd> {
    let start = edit::start(
        intent,
        model.index().map(|i| i.as_ref()),
        model.flow_prefill(),
    );
    begin_start(model, start)
}

pub(super) fn begin_start(model: &mut Model, start: Start) -> Vec<Cmd> {
    // `app_rule_key` sets it again for a form the app overlay opened.
    model.form_from_app = false;
    match start {
        Start::Form(f) => {
            let flow = matches!(f.intent, EditIntent::RuleFromFlow(_));
            model.overlay = Overlay::Form(f);
            if flow {
                load_form_drafts(model)
            } else {
                Vec::new()
            }
        }
        Start::Plan(w) => {
            model.status = StatusLine::info("planning…");
            model.writes.in_flight = true;
            vec![Cmd::PlanWrite(w)]
        }
    }
}

/// `w` on Traffic: a draft allow rule from the selected blocked or
/// potentially blocked flow (Flows view or a drilled pair), or from the
/// first such flow behind a section's peer row (App-to-app `in` / `out` /
/// `both`, Blocked) or a Mesh row. App-to-app `pairs` refuses: a pair holds
/// flows of every decision; so does a section header.
pub(super) fn flow_rule_key(model: &mut Model) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    let t = &model.traffic;
    let flow = match t.view {
        TrafficView::Flows
            if t.flows_grouped
                .selected_item()
                .is_some_and(SectionRow::is_header) =>
        {
            model.status = StatusLine::warn("select a flow row (a header holds a whole app)");
            return Vec::new();
        }
        TrafficView::Flows => t.selected_flow().map(|r| (*r.flow).clone()),
        TrafficView::Blocked | TrafficView::Aggregate if t.section_table().is_some() => {
            match t.section_drill() {
                Some(d @ Drill::Peer { .. }) => {
                    let keys = t.effective_keys();
                    t.first_flow(|f| d.matches(f, &keys))
                }
                Some(_) => {
                    model.status =
                        StatusLine::warn("select a peer row (a header holds a whole app)");
                    return Vec::new();
                }
                None => None,
            }
        }
        TrafficView::Mesh => t.mesh.selected_item().and_then(|r| {
            let keys = t.effective_keys();
            t.first_flow(|f| r.contains(f, &keys, t.mesh_cols))
        }),
        TrafficView::Aggregate if t.apps_state.by_service => return service_pair_rule_key(model),
        TrafficView::Blocked
        | TrafficView::Aggregate
        | TrafficView::Treemap
        | TrafficView::Exposure => {
            model.status = StatusLine::warn("select a flow in the Flows view (Enter on a pair)");
            return Vec::new();
        }
    };
    let Some(flow) = flow else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
    match flow.policy_decision {
        PolicyDecision::Blocked | PolicyDecision::PotentiallyBlocked => {
            begin_edit(model, EditIntent::RuleFromFlow(Box::new(flow)))
        }
        PolicyDecision::Allowed => {
            model.status = StatusLine::warn("this flow is allowed; nothing to add");
            Vec::new()
        }
        PolicyDecision::Unknown => {
            model.status = StatusLine::warn("this flow has no policy decision; nothing to add");
            Vec::new()
        }
    }
}

/// `w` on an App-to-app pair by service (`o`): the allow rule for the
/// row's tuple, drafted from one of its blocked flows (one the decision
/// filter keeps) with the row's groups (under the `g` keys) as providers
/// and consumers and its service.
pub(super) fn service_pair_rule_key(model: &mut Model) -> Vec<Cmd> {
    let t = &model.traffic;
    let Some(g) = t.pairs_table().selected_item() else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
    if g.blocked + g.potentially_blocked == 0 {
        model.status = StatusLine::warn("every flow of this row is allowed; nothing to add");
        return Vec::new();
    }
    let keys = t.effective_keys();
    let decisions = t.apps_state.decisions;
    let Some(flow) = t.first_flow(|f| decisions.keeps(f) && g.contains(f, &keys)) else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
    let prefill = edit::FlowPrefill {
        group_keys: Some(&keys),
        ..model.flow_prefill()
    };
    let start = edit::start(
        EditIntent::RuleFromFlow(Box::new(flow)),
        model.index().map(|i| i.as_ref()),
        prefill,
    );
    begin_start(model, start)
}
