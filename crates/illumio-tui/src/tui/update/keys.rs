//! Key and mouse routing: the overlays and prompts first, then the screen
//! in front; the help overlay, the shared table and search keys, field
//! completion, screen switches and quit.
use super::*;

/// `Ctrl-N` (`forward`) or `Ctrl-P` in a form or the filter editor: cycles
/// an open completion, or opens one from the name index, the loaded draft
/// rulesets and the workloads the Workloads screen holds (`Ctrl-P` opens on
/// the last candidate). A shown key (`app=`) or prefix (`group:`, `wl:`)
/// completes what follows it on `Ctrl-N`. Without an index, or for a text
/// field, nothing happens. Never a request.
/// The candidates for the active field's current token, best first;
/// none without an index or for a text field. Never a request.
pub(super) fn field_candidates(model: &Model, editor: &impl Completing) -> Vec<String> {
    let Some(idx) = model.index() else {
        return Vec::new();
    };
    let Some((kind, token, earlier)) = editor.completion_request() else {
        return Vec::new();
    };
    let rulesets = model.draft_rulesets().unwrap_or_default();
    let workloads = &model.workloads.raw;
    let mut candidates =
        crate::tui::complete::complete(kind, &token, &earlier, idx, rulesets, workloads);
    // A `w` form's rulesets that cover the destination first, best first.
    if kind == FieldKind::Ruleset {
        let preferred = editor.preferred();
        candidates.sort_by_key(|c| preferred.iter().position(|p| p == c).unwrap_or(usize::MAX));
    }
    candidates
}

/// `Tab` in a Ruleset field completes instead of moving on: the field holds
/// text that is no draft ruleset's exact name (any case) and some name
/// matches it. Empty or exact text, or an open completion (`Tab` then keeps
/// the shown name and moves on), leaves `Tab` to the form.
pub(super) fn tab_completes(model: &Model, form: &Form) -> bool {
    let Some(field) = form.fields.get(form.active) else {
        return false;
    };
    let text = field.value.trim();
    field.kind == FieldKind::Ruleset
        && form.completion.is_none()
        && !text.is_empty()
        && !model
            .draft_rulesets()
            .is_some_and(|sets| !edit::rulesets_named(sets, text).is_empty())
        && !field_candidates(model, form).is_empty()
}

pub(super) fn complete_field(model: &Model, editor: &mut impl Completing, forward: bool) {
    let dives = forward && editor.completion_dives();
    if editor.completing() && !dives {
        editor.cycle(forward);
        return;
    }
    let candidates = field_candidates(model, editor);
    if editor.start_completion(candidates) {
        if !forward {
            editor.cycle(false);
        }
    } else if dives {
        editor.cycle(true);
    }
}

pub(super) fn on_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    if k.kind == KeyEventKind::Release {
        return Vec::new();
    }
    if k.code == KeyCode::Char('c') && k.modifiers == KeyModifiers::CONTROL {
        return quit(model);
    }
    match model.overlay.clone() {
        Overlay::Help => {
            help_key(model, k);
            Vec::new()
        }
        Overlay::Prompt { mut input } => match k.code {
            KeyCode::Esc => {
                model.overlay = Overlay::None;
                Vec::new()
            }
            KeyCode::Enter => {
                model.overlay = Overlay::None;
                match commands::parse(&input) {
                    Ok(cmd) => update(model, Msg::Command(cmd)),
                    Err(e) => {
                        model.status = StatusLine::error(e);
                        Vec::new()
                    }
                }
            }
            KeyCode::Backspace => {
                input.pop();
                model.overlay = Overlay::Prompt { input };
                Vec::new()
            }
            KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                input.clear();
                model.overlay = Overlay::Prompt { input };
                Vec::new()
            }
            KeyCode::Char(c) => {
                input.push(c);
                model.overlay = Overlay::Prompt { input };
                Vec::new()
            }
            _ => Vec::new(),
        },
        Overlay::GroupKeys(picker) => {
            match picker.key(k) {
                PickerOutcome::Open(p) => model.overlay = Overlay::GroupKeys(p),
                PickerOutcome::Close => model.overlay = Overlay::None,
                PickerOutcome::Apply { keys, all } => {
                    model.overlay = Overlay::None;
                    apply_group_keys(model, keys, all);
                }
            }
            Vec::new()
        }
        Overlay::LabelKeys(picker) => {
            match picker.key(k) {
                PickerOutcome::Open(p) => model.overlay = Overlay::LabelKeys(p),
                PickerOutcome::Close => model.overlay = Overlay::None,
                PickerOutcome::Apply { keys, all } => {
                    model.overlay = Overlay::None;
                    apply_label_keys(model, if all { Vec::new() } else { keys });
                }
            }
            Vec::new()
        }
        Overlay::FilterEditor(mut e)
            if k.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(k.code, KeyCode::Char('n' | 'p')) =>
        {
            complete_field(model, &mut e, k.code == KeyCode::Char('n'));
            model.overlay = Overlay::FilterEditor(e);
            Vec::new()
        }
        Overlay::FilterEditor(mut e) => {
            // Esc puts the typed token back; any other key keeps the shown
            // candidate and then does what it always does.
            let esc = k.code == KeyCode::Esc;
            if e.end_completion(esc) && esc {
                model.overlay = Overlay::FilterEditor(e);
                return Vec::new();
            }
            match k.code {
                KeyCode::Esc => model.overlay = Overlay::None,
                KeyCode::Tab | KeyCode::Down => {
                    e.next();
                    model.overlay = Overlay::FilterEditor(e);
                }
                KeyCode::BackTab | KeyCode::Up => {
                    e.prev();
                    model.overlay = Overlay::FilterEditor(e);
                }
                KeyCode::Backspace => {
                    e.pop();
                    model.overlay = Overlay::FilterEditor(e);
                }
                KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    e.clear();
                    model.overlay = Overlay::FilterEditor(e);
                }
                KeyCode::Char(c) => {
                    e.push(c);
                    model.overlay = Overlay::FilterEditor(e);
                }
                KeyCode::Enter => match e.to_filter() {
                    Ok(f) => {
                        model.overlay = Overlay::None;
                        return request_query(model, f);
                    }
                    Err(err) => {
                        e.error = Some(err);
                        model.overlay = Overlay::FilterEditor(e);
                    }
                },
                _ => model.overlay = Overlay::FilterEditor(e),
            }
            Vec::new()
        }
        Overlay::LabelFilter(mut e)
            if k.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(k.code, KeyCode::Char('n' | 'p')) =>
        {
            complete_field(model, &mut e, k.code == KeyCode::Char('n'));
            label_editor_unknown(model, &mut e);
            model.overlay = Overlay::LabelFilter(e);
            Vec::new()
        }
        Overlay::LabelFilter(mut e) => {
            // Esc puts the typed token back first, then closes.
            let esc = k.code == KeyCode::Esc;
            if e.end_completion(esc) && esc {
                model.overlay = Overlay::LabelFilter(e);
                return Vec::new();
            }
            match k.code {
                KeyCode::Esc => model.overlay = Overlay::None,
                KeyCode::Backspace => {
                    e.pop();
                    label_editor_unknown(model, &mut e);
                    model.overlay = Overlay::LabelFilter(e);
                }
                KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    e.clear();
                    label_editor_unknown(model, &mut e);
                    model.overlay = Overlay::LabelFilter(e);
                }
                KeyCode::Char(c) => {
                    e.push(c);
                    label_editor_unknown(model, &mut e);
                    model.overlay = Overlay::LabelFilter(e);
                }
                // A malformed filter stays in the editor; the old one holds.
                KeyCode::Enter => match apply_workload_labels(model, &e.value) {
                    Ok(()) => model.overlay = Overlay::None,
                    Err(err) => {
                        e.error = Some(err);
                        model.overlay = Overlay::LabelFilter(e);
                    }
                },
                _ => model.overlay = Overlay::LabelFilter(e),
            }
            Vec::new()
        }
        Overlay::RuleFilter(e) => rule_filter_key(model, e, k),
        Overlay::Confirm { action, .. } => match k.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                model.overlay = Overlay::None;
                match action {
                    PendingAction::RunQuery(f) => update(model, Msg::ApplyFilter(f)),
                    PendingAction::RunNow(f) => run_now(model, f),
                    PendingAction::LoadLabelUsage => load_label_usage(model),
                    PendingAction::RunFocused(f, focus, mode) => {
                        start_focused(model, f, focus, mode)
                    }
                    PendingAction::RunDraft {
                        filter,
                        focus,
                        hrefs,
                    } => {
                        let cmds = start_draft(model, filter, focus, hrefs);
                        // Asked from Explain over the app overlay: back to it.
                        if model.explain_back.take().is_some_and(|v| v.from_app)
                            && model.app_view.is_some()
                        {
                            model.overlay = Overlay::App;
                        }
                        cmds
                    }
                    PendingAction::ReloadApp => app_reload(model),
                    PendingAction::GenRequery(f, focus) => generator::gen_requery(model, f, focus),
                    PendingAction::DeleteMark(id) => marks::delete_mark(model, &id),
                    PendingAction::EnforceWatch => enforce::start_watch(model),
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                model.overlay = Overlay::None;
                model.status = StatusLine::info("cancelled");
                if matches!(action, PendingAction::ReloadApp) {
                    model.overlay = Overlay::App;
                }
                if matches!(action, PendingAction::GenRequery(..)) {
                    model.overlay = Overlay::Generator;
                }
                if matches!(action, PendingAction::DeleteMark(_)) {
                    model.overlay = Overlay::Marks;
                }
                // Asked from Explain (`D`): back to it.
                if matches!(action, PendingAction::RunDraft { .. })
                    && let Some(v) = model.explain_back.take()
                {
                    model.overlay = Overlay::Explain(v);
                }
                Vec::new()
            }
            _ => Vec::new(),
        },
        Overlay::EventsEditor {
            mut values,
            mut active,
            ..
        } => {
            let editor =
                |values: [String; 3], active: usize, error: Option<String>| Overlay::EventsEditor {
                    values,
                    active,
                    error,
                };
            match k.code {
                KeyCode::Esc => model.overlay = Overlay::None,
                KeyCode::Tab | KeyCode::Down => {
                    active = (active + 1) % values.len();
                    model.overlay = editor(values, active, None);
                }
                KeyCode::BackTab | KeyCode::Up => {
                    active = (active + values.len() - 1) % values.len();
                    model.overlay = editor(values, active, None);
                }
                KeyCode::Backspace => {
                    values[active].pop();
                    model.overlay = editor(values, active, None);
                }
                KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    values[active].clear();
                    model.overlay = editor(values, active, None);
                }
                KeyCode::Char(c) => {
                    values[active].push(c);
                    model.overlay = editor(values, active, None);
                }
                KeyCode::Enter => {
                    let edit = EventsFilterEdit {
                        severity: values[0].trim().to_string(),
                        event_type: values[1].trim().to_string(),
                        window: values[2].trim().to_string(),
                    };
                    let applied = match edit.to_filter(jiff::Timestamp::now()) {
                        Ok(_) if model.events.loading => Err("events are still loading".into()),
                        other => other,
                    };
                    match applied {
                        Ok(filter) => {
                            model.overlay = Overlay::None;
                            model.events.filter = edit;
                            model.events.rebuild();
                            return request_events(model, filter, CacheMode::Use);
                        }
                        Err(err) => model.overlay = editor(values, active, Some(err)),
                    }
                }
                _ => model.overlay = editor(values, active, None),
            }
            Vec::new()
        }
        Overlay::EventDetail { event, scroll, raw } => {
            // The renderer clamps to the last page; here only the top is a hard stop.
            let last = event_detail_lines(model, &event, raw).saturating_sub(1);
            let next = match k.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                    model.overlay = Overlay::None;
                    return Vec::new();
                }
                KeyCode::Char('?') => {
                    open_help(model);
                    return Vec::new();
                }
                // Diff ↔ raw JSON, from the top: the line counts differ.
                KeyCode::Char('J') => {
                    model.overlay = Overlay::EventDetail {
                        event,
                        scroll: 0,
                        raw: !raw,
                    };
                    return Vec::new();
                }
                // ↑/↓ and j/k step through the events behind the detail, as
                // in the table; the detail opens at the top in the same view.
                KeyCode::Down | KeyCode::Up | KeyCode::Char('j') | KeyCode::Char('k') => {
                    let t = &mut model.events.table;
                    if matches!(k.code, KeyCode::Down | KeyCode::Char('j')) {
                        t.select_next();
                    } else {
                        t.select_prev();
                    }
                    let event = t.selected_item().cloned().unwrap_or(event);
                    model.overlay = Overlay::EventDetail {
                        event,
                        scroll: 0,
                        raw,
                    };
                    return Vec::new();
                }
                KeyCode::Char('e') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    (scroll + 1).min(last)
                }
                KeyCode::Char('y') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    scroll.saturating_sub(1)
                }
                KeyCode::Char('d') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    (scroll + 10).min(last)
                }
                KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    scroll.saturating_sub(10)
                }
                KeyCode::PageDown | KeyCode::Char(' ') => (scroll + 10).min(last),
                KeyCode::PageUp => scroll.saturating_sub(10),
                KeyCode::Home | KeyCode::Char('g') => 0,
                KeyCode::End | KeyCode::Char('G') => last,
                _ => scroll,
            };
            model.overlay = Overlay::EventDetail {
                event,
                scroll: next,
                raw,
            };
            Vec::new()
        }
        Overlay::MarkForm(mf) => marks::form_key(model, *mf, k),
        Overlay::Marks => marks::overlay_key(model, k),
        Overlay::Form(mut form)
            if k.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(k.code, KeyCode::Char('n' | 'p')) =>
        {
            complete_field(model, &mut form, k.code == KeyCode::Char('n'));
            model.overlay = Overlay::Form(form);
            Vec::new()
        }
        Overlay::Form(mut form)
            if k.code == KeyCode::Tab && k.modifiers.is_empty() && tab_completes(model, &form) =>
        {
            complete_field(model, &mut form, true);
            model.overlay = Overlay::Form(form);
            Vec::new()
        }
        Overlay::Form(form) if matches!(form.intent, EditIntent::BulkLabels(_)) => {
            bulk::labels_key(model, form, k)
        }
        Overlay::EnforcePicker(p) => bulk::picker_key(model, p, k),
        Overlay::ConfirmBulk(c) => bulk::confirm_key(model, c, k),
        Overlay::Form(mut form) => match form.on_key(k) {
            FormEvent::Continue => {
                model.overlay = Overlay::Form(form);
                Vec::new()
            }
            FormEvent::Cancel => {
                close_form(model);
                model.status = StatusLine::info("cancelled, nothing sent");
                Vec::new()
            }
            // The ruleset is planned now; the rule waits for its href. A
            // name a loaded draft ruleset has gets the rule instead.
            FormEvent::NewRuleset => match existing_ruleset(model, form.value("Ruleset"))
                .map_or_else(
                    || {
                        edit::new_ruleset_intent(
                            &form,
                            model.index().map(|i| i.as_ref()),
                            model.profile.org_id,
                            &model.limits.scope_keys,
                        )
                    },
                    Err,
                ) {
                Ok((intent, draft)) => {
                    model.policy.rule_after_ruleset = Some((draft, form));
                    model.overlay = Overlay::None;
                    model.status = StatusLine::info("planning…");
                    model.writes.in_flight = true;
                    vec![Cmd::PlanWrite(intent)]
                }
                Err(e) => {
                    form.error = Some(e);
                    model.overlay = Overlay::Form(form);
                    Vec::new()
                }
            },
            FormEvent::Submit => {
                match edit::build_intent(
                    &form,
                    model.index().map(|i| i.as_ref()),
                    model.profile.org_id,
                ) {
                    Ok(Some(intent)) => {
                        if matches!(form.intent, EditIntent::RuleFromFlow(_)) {
                            model.traffic.rule_form = Some(form);
                        }
                        model.overlay = Overlay::None;
                        model.status = StatusLine::info("planning…");
                        model.writes.in_flight = true;
                        vec![Cmd::PlanWrite(intent)]
                    }
                    Ok(None) => {
                        close_form(model);
                        model.status = StatusLine::info("no changes");
                        Vec::new()
                    }
                    Err(e) => {
                        form.error = Some(e);
                        model.overlay = Overlay::Form(form);
                        Vec::new()
                    }
                }
            }
        },
        Overlay::ConfirmWrite {
            plan,
            kind: pce::write::ObjectKind::Revert,
            scroll,
        } => revert_confirm_key(model, plan, scroll, k),
        Overlay::ConfirmWrite { plan, kind, scroll } => {
            let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
            let extra = crate::tui::impact::confirm_rows(&plan);
            let last = confirm_write_last_scroll(&plan, kind, area);
            let last = u16::try_from(last).unwrap_or(u16::MAX);
            match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    if kind == pce::write::ObjectKind::Provision {
                        // The answer is the new policy version; the counts
                        // come from what is sent.
                        model.policy.provision_counts =
                            Some(counts_text(plan.request().and_then(|r| r.body.as_ref())));
                    }
                    model.overlay = Overlay::None;
                    model.status = StatusLine::info("sending …");
                    model.writes.in_flight = true;
                    vec![Cmd::ExecuteWrite { plan, kind }]
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    close_form(model);
                    model.status = StatusLine::info("cancelled, nothing sent");
                    model.policy.rule_after_ruleset = None;
                    Vec::new()
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    model.overlay = Overlay::ConfirmWrite {
                        plan,
                        kind,
                        scroll: (scroll + 1).min(last),
                    };
                    Vec::new()
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    model.overlay = Overlay::ConfirmWrite {
                        plan,
                        kind,
                        scroll: scroll.saturating_sub(1),
                    };
                    Vec::new()
                }
                KeyCode::Char('i') if extra > 0 => {
                    let cmds = impact_key(model);
                    model.refresh_confirm_impact();
                    cmds
                }
                _ => Vec::new(),
            }
        }
        Overlay::ConfirmBatch { batch, scroll } => {
            let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
            let last = u16::try_from(confirm_batch_last_scroll(&batch, area)).unwrap_or(u16::MAX);
            match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    model.overlay = Overlay::None;
                    let n = batch.sends();
                    model.status = StatusLine::info(if n == 1 {
                        "sending 1 request …".to_string()
                    } else {
                        format!("sending {n} requests …")
                    });
                    model.writes.in_flight = true;
                    model.batch_sending = true;
                    vec![Cmd::ExecuteBatch(batch)]
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    let cmds = batch_closed(model);
                    model.status = StatusLine::info("cancelled, nothing sent");
                    cmds
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    model.overlay = Overlay::ConfirmBatch {
                        batch,
                        scroll: (scroll + 1).min(last),
                    };
                    Vec::new()
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    model.overlay = Overlay::ConfirmBatch {
                        batch,
                        scroll: scroll.saturating_sub(1),
                    };
                    Vec::new()
                }
                _ => Vec::new(),
            }
        }
        Overlay::BatchReport { scroll } => {
            let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
            let lines = model.batch_report.clone().unwrap_or_default();
            let last = u16::try_from(batch_report_last_scroll(&lines, area)).unwrap_or(u16::MAX);
            match k.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => batch_closed(model),
                KeyCode::Down | KeyCode::Char('j') => {
                    model.overlay = Overlay::BatchReport {
                        scroll: (scroll + 1).min(last),
                    };
                    Vec::new()
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    model.overlay = Overlay::BatchReport {
                        scroll: scroll.saturating_sub(1),
                    };
                    Vec::new()
                }
                _ => Vec::new(),
            }
        }
        Overlay::Pending => {
            let n = model.policy.pending.as_ref().map_or(0, Vec::len);
            match k.code {
                KeyCode::Esc | KeyCode::Char('q') => model.overlay = Overlay::None,
                KeyCode::Down | KeyCode::Char('j') => {
                    model.policy.pending_selected =
                        (model.policy.pending_selected + 1).min(n.saturating_sub(1));
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    model.policy.pending_selected = model.policy.pending_selected.saturating_sub(1);
                }
                KeyCode::Char(' ') => {
                    let p = &mut model.policy;
                    if let Some(href) = p
                        .pending
                        .as_ref()
                        .and_then(|l| l.get(p.pending_selected))
                        .map(|c| c.href.clone())
                        && !p.pending_marked.remove(&href)
                    {
                        p.pending_marked.insert(href);
                    }
                }
                KeyCode::Char('P') => return provision_key(model),
                KeyCode::Char('R') => return revert_key(model),
                KeyCode::Enter => return pending_diff(model),
                KeyCode::Char('?') => open_help(model),
                _ => {}
            }
            Vec::new()
        }
        Overlay::Diff { diff, scroll } => {
            let last = u16::try_from(diff.lines.len().saturating_sub(1)).unwrap_or(u16::MAX);
            match k.code {
                KeyCode::Esc => model.overlay = Overlay::Pending,
                KeyCode::Char('q') => model.overlay = Overlay::None,
                KeyCode::Down | KeyCode::Char('j') => {
                    model.overlay = Overlay::Diff {
                        diff,
                        scroll: (scroll + 1).min(last),
                    };
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    model.overlay = Overlay::Diff {
                        diff,
                        scroll: scroll.saturating_sub(1),
                    };
                }
                _ => {}
            }
            Vec::new()
        }
        Overlay::RuleDetail(_) => match k.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                model.overlay = Overlay::None;
                Vec::new()
            }
            KeyCode::Char('i') => impact_key(model),
            KeyCode::Char('I') => {
                let href = match &model.overlay {
                    Overlay::RuleDetail(r) => r.href.clone(),
                    _ => String::new(),
                };
                open_matched(model, &href)
            }
            _ => Vec::new(),
        },
        Overlay::Matched => matched_key(model, k),
        Overlay::Error(_) => {
            if matches!(k.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q')) {
                model.overlay = Overlay::None;
            }
            Vec::new()
        }
        Overlay::AppPicker(picker) => match picker.key(k) {
            AppPickerOutcome::Open(p) => {
                model.overlay = Overlay::AppPicker(p);
                Vec::new()
            }
            AppPickerOutcome::Close => {
                model.overlay = Overlay::None;
                Vec::new()
            }
            AppPickerOutcome::Pick { app, purpose } => {
                model.overlay = Overlay::None;
                match purpose {
                    AppPickerPurpose::Focus => {
                        focus_by_name(model, &app);
                        Vec::new()
                    }
                    AppPickerPurpose::Overlay => open_app_by_name(model, &app),
                    AppPickerPurpose::Generator => gen_by_name(model, &app),
                    AppPickerPurpose::Enforce => enforce::enforce_by_name(model, &app),
                }
            }
        },
        Overlay::App => app_overlay_key(model, k),
        Overlay::Explain(v) => explain_overlay_key(model, v, k),
        Overlay::Generator => generator::gen_key(model, k),
        Overlay::Readiness => enforce::readiness_key(model, k),
        Overlay::Enforce => enforce::enforce_key(model, k),
        Overlay::None => on_screen_key(model, k),
    }
}

pub(super) fn on_screen_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    // An active table search captures every key except Ctrl-C.
    let ctrl_c = k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL);
    if !ctrl_c {
        if model.screen == Screen::Traffic && model.traffic.is_searching() {
            return traffic_key(model, k);
        }
        if model.screen == Screen::Policy && model.policy.is_searching() {
            return policy_key(model, k);
        }
        if model.screen == Screen::Objects && model.objects.is_searching() {
            return objects_key(model, k);
        }
        if model.screen == Screen::Workloads && model.workloads.table.is_searching() {
            return workloads_key(model, k);
        }
        if model.screen == Screen::Events && model.events.table.is_searching() {
            return events_key(model, k);
        }
    }
    if k.code == KeyCode::Char('r')
        && k.modifiers == KeyModifiers::NONE
        && matches!(model.connection, Connection::Disconnected { .. })
    {
        return reconnect(model);
    }
    match (k.code, k.modifiers) {
        (KeyCode::Char('q'), KeyModifiers::NONE) => quit(model),
        (KeyCode::Char(':'), _) => {
            model.overlay = Overlay::Prompt {
                input: String::new(),
            };
            Vec::new()
        }
        (KeyCode::Char('?'), _) => {
            open_help(model);
            Vec::new()
        }
        (KeyCode::Char('1'), _) => switch(model, Screen::Traffic),
        (KeyCode::Char('2'), _) => switch(model, Screen::Policy),
        (KeyCode::Char('3'), _) => switch(model, Screen::Objects),
        (KeyCode::Char('4'), _) => switch(model, Screen::Workloads),
        (KeyCode::Char('5'), _) => switch(model, Screen::Events),
        _ if model.screen == Screen::Traffic => traffic_key(model, k),
        _ if model.screen == Screen::Policy => policy_key(model, k),
        _ if model.screen == Screen::Objects => objects_key(model, k),
        _ if model.screen == Screen::Workloads => workloads_key(model, k),
        _ if model.screen == Screen::Events => events_key(model, k),
        _ => Vec::new(),
    }
}

/// `r` while disconnected: one user-caused reconnect attempt. Moving to
/// `Connecting` is what keeps a second `r` from sending another.
pub(super) fn reconnect(model: &mut Model) -> Vec<Cmd> {
    model.connection = Connection::Connecting;
    model.objects.index_failed = false;
    model.workloads.index_failed = false;
    model.status = StatusLine::info("reconnecting…");
    vec![Cmd::Connect]
}

/// Search editing keys, shared by every view's table.
pub(super) fn search_key<T>(table: &mut crate::tui::table::TableView<T>, k: KeyEvent) {
    match k.code {
        KeyCode::Esc => table.search_clear(),
        KeyCode::Enter => table.search_commit(),
        KeyCode::Backspace => table.search_pop(),
        KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
            table.search_clear_draft()
        }
        KeyCode::Char(c) => table.search_push(c),
        _ => {}
    }
}

/// Navigation, sort and search-start keys, shared by every view's table.
pub(super) fn table_key<T>(table: &mut crate::tui::table::TableView<T>, k: KeyEvent, page: usize) {
    match k.code {
        KeyCode::Down | KeyCode::Char('j') => table.select_next(),
        KeyCode::Up | KeyCode::Char('k') => table.select_prev(),
        KeyCode::PageDown => table.page_down(page),
        KeyCode::PageUp => table.page_up(page),
        KeyCode::Home => table.home(),
        KeyCode::End => table.end(),
        KeyCode::Char('s') => table.sort_next(),
        KeyCode::Char('/') => table.start_search(),
        _ => {}
    }
}

/// `?` / `:help`: the help at its top, on the keys of where it was opened
/// (`help::HelpContext::of`). Opened over the app view, the generator, an
/// event's detail, the pending changes or the marks, closing it goes back there.
pub(super) fn open_help(model: &mut Model) {
    model.help_scroll = 0;
    model.help_page = HelpPage::Context;
    model.help_return = match &model.overlay {
        o @ (Overlay::App
        | Overlay::Generator
        | Overlay::EventDetail { .. }
        | Overlay::Pending
        | Overlay::Marks
        | Overlay::Readiness
        | Overlay::Enforce) => o.clone(),
        _ => Overlay::None,
    };
    model.overlay = Overlay::Help;
}

/// The help's keys: `Esc` / `?` / `q` close it (back to where it was
/// opened), `Tab` / `BackTab` one group at a time, `a` all groups (again:
/// the context page), the scroll keys within the page. Others do nothing.
pub(super) fn help_key(model: &mut Model, k: KeyEvent) {
    let ctx = HelpContext::of(model);
    let page = model.help_page;
    match k.code {
        KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => {
            model.overlay = std::mem::replace(&mut model.help_return, Overlay::None);
            return;
        }
        KeyCode::Tab => model.help_page = page.step(ctx, true),
        KeyCode::BackTab => model.help_page = page.step(ctx, false),
        KeyCode::Char('a') => model.help_page = page.toggle_all(),
        _ => {}
    }
    if model.help_page != page {
        model.help_scroll = 0;
        return;
    }
    let last = help_last_scroll(model, model.size);
    let rows = help_body_rows(model, model.size).max(1);
    let s = model.help_scroll.min(last);
    model.help_scroll = match k.code {
        KeyCode::Down | KeyCode::Char('j') => (s + 1).min(last),
        KeyCode::Up | KeyCode::Char('k') => s.saturating_sub(1),
        KeyCode::PageDown => (s + rows).min(last),
        KeyCode::PageUp => s.saturating_sub(rows),
        KeyCode::End | KeyCode::Char('G') => last,
        KeyCode::Home | KeyCode::Char('g') => 0,
        _ => s,
    };
}

/// Mouse: the wheel moves the focused table one row (also while a search is
/// active), a left click on the tabs row switches screen through the same path
/// as the number keys. Everything else is ignored; the wheel never requests anything.
pub(super) fn on_mouse(model: &mut Model, me: crossterm::event::MouseEvent) -> Vec<Cmd> {
    use crossterm::event::{MouseButton, MouseEventKind};
    if model.overlay != Overlay::None {
        return Vec::new();
    }
    match me.kind {
        MouseEventKind::ScrollDown => scroll_focused(model, true),
        MouseEventKind::ScrollUp => scroll_focused(model, false),
        MouseEventKind::Down(MouseButton::Left) if me.row == 1 => {
            return match crate::tui::ui::tab_at(me.column, model.size.0) {
                Some(s) => switch(model, s),
                None => Vec::new(),
            };
        }
        _ => {}
    }
    Vec::new()
}

/// One row up or down in the current screen's focused table.
pub(super) fn scroll_focused(model: &mut Model, down: bool) {
    fn step<T>(table: &mut crate::tui::table::TableView<T>, down: bool, page: usize) {
        if down {
            table.select_next();
        } else {
            table.select_prev();
        }
        table.ensure_visible(page);
    }
    match model.screen {
        Screen::Traffic => {
            let page = traffic_page(model);
            let t = &mut model.traffic;
            match t.view {
                TrafficView::Flows if t.flows_grouped() => step(&mut t.flows_grouped, down, page),
                TrafficView::Flows => step(&mut t.flows, down, page),
                TrafficView::Aggregate if t.apps_state.by == By::Pairs => {
                    step(t.pairs_table_mut(), down, page)
                }
                TrafficView::Aggregate => step(&mut t.apps, down, page),
                TrafficView::Mesh => step(&mut t.mesh, down, page),
                TrafficView::Blocked => step(&mut t.blocked, down, page),
                // The wheel moves table rows; the treemap has none.
                TrafficView::Treemap => {}
            }
        }
        Screen::Policy if model.policy.search.is_some() => {
            let page = policy_page(model);
            step(&mut model.policy.found, down, page);
        }
        Screen::Policy => {
            let page = policy_page(model);
            match model.policy.pane {
                PolicyPane::RuleSets => {
                    let before = model.policy.selected_href();
                    step(&mut model.policy.rulesets, down, page);
                    // The rules follow the ruleset; a missing deny list is not
                    // fetched from here (the wheel never requests).
                    if model.policy.selected_href() != before
                        && let Some(idx) = model.index().cloned()
                    {
                        model
                            .policy
                            .rebuild_rules(&idx, model.workloads.impact_source());
                    }
                }
                PolicyPane::Rules => step(&mut model.policy.rules, down, page),
            }
        }
        Screen::Objects => {
            let page = if model.objects.kind == ObjectKind::IpLists {
                ip_lists_page(model)
            } else {
                objects_page(model)
            };
            let o = &mut model.objects;
            match o.kind {
                ObjectKind::Labels => step(&mut o.labels, down, page),
                ObjectKind::LabelGroups => step(&mut o.groups, down, page),
                ObjectKind::Services => step(&mut o.services, down, page),
                ObjectKind::IpLists => step(&mut o.ip_lists.table, down, page),
            }
        }
        Screen::Workloads => {
            let page = workloads_page(model);
            let w = &mut model.workloads;
            if w.grouped {
                step(&mut w.sections, down, page);
            } else {
                step(&mut w.table, down, page);
            }
        }
        Screen::Events => {
            let page = events_page(model);
            step(&mut model.events.table, down, page);
        }
    }
}

pub(super) fn switch(model: &mut Model, s: Screen) -> Vec<Cmd> {
    model.screen = s;
    // Another screen: the enforcement overlay a step left stays closed.
    model.enforce_back = None;
    // Leaving the Policy screen drops Explain's pending `Enter`.
    if s != Screen::Policy {
        model.policy.jump = None;
    }
    match s {
        // Loaded data is kept: returning to the screen never requests again.
        Screen::Policy
            if model.policy.loaded_at.is_none()
                && !model.policy.loading
                && model.is_connected() =>
        {
            load_rule_sets(model, CacheMode::Use)
        }
        // Loaded off this screen: the selected ruleset's deny rules now.
        // Not during a reload: its answer asks for them, on fresh data.
        Screen::Policy if model.policy.loaded_at.is_some() && !model.policy.loading => {
            deny_fallback(model, CacheMode::Use)
        }
        Screen::Objects => load_missing_objects(model),
        Screen::Workloads => load_missing_workloads(model),
        Screen::Events => load_missing_events(model),
        _ => Vec::new(),
    }
}

/// `q`, `Ctrl-C`, `:q`: refused while a batch is being sent, so no
/// request on the wire is dropped before its answer is audited.
pub(super) fn quit(model: &mut Model) -> Vec<Cmd> {
    if model.batch_sending {
        model.status = StatusLine::warn("batch sending: wait for it to finish");
        return Vec::new();
    }
    model.quit = true;
    vec![Cmd::Quit]
}

/// Search editing, navigation and sort on one table, keeping the selection in view.
pub(super) fn drive_table<T>(
    table: &mut crate::tui::table::TableView<T>,
    k: KeyEvent,
    page: usize,
) {
    if table.is_searching() {
        search_key(table, k);
    } else {
        table_key(table, k, page);
    }
    table.ensure_visible(page);
}
