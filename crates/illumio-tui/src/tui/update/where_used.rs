//! `W` / `:used` on the Objects screen: the where-used overlay. It reads
//! what is loaded (the draft rulesets as the Policy screen loads them, the
//! label groups, the workload list) and loads what is missing once, on the
//! key press, through the cache: the draft rulesets, and for a label the
//! pairing profiles and, with no workload list, a one-row probe. Its rows
//! follow every answer; nothing runs on a timer, and reopening it loads
//! nothing again.
use super::*;
use crate::tui::where_used::{WAIT_COUNT, WAIT_PROFILES, WAIT_RULESETS, WhereUsedView};
use pce::where_used::{Inputs, ObjectType, RefKind, Reference, Target, WorkloadCount};

/// The selected object of the Objects tab on screen, as a target and its
/// display name (`env=prod` for a label).
fn selected_target(model: &Model) -> Option<(Target, String)> {
    let o = &model.objects;
    match o.kind {
        ObjectKind::Labels => o.labels.selected_item().map(|r| {
            (
                Target::new(ObjectType::Label, r.href.clone()),
                format!("{}={}", r.key, r.value),
            )
        }),
        ObjectKind::LabelGroups => o.groups.selected_item().map(|g| {
            (
                Target::new(ObjectType::LabelGroup, g.href.clone()),
                g.name.clone(),
            )
        }),
        ObjectKind::Services => o.services.selected_item().map(|s| {
            (
                Target::new(ObjectType::Service, s.href.clone()),
                s.name.clone(),
            )
        }),
        ObjectKind::IpLists => o.ip_lists.table.selected_item().map(|r| {
            (
                Target::new(ObjectType::IpList, r.list.href.clone()),
                r.list.name.clone(),
            )
        }),
    }
}

/// `W` / `:used`: the overlay for the selected row. The draft rulesets are
/// loaded when neither held nor on their way (one cached request, as a `w`
/// form loads them); a label gets the pairing profiles once (one cached
/// request) and, without the workload list, one cached probe, unless one
/// answered already. `u` is left as it is.
pub(super) fn open_where_used(model: &mut Model) -> Vec<Cmd> {
    if model.screen != Screen::Objects {
        model.status = StatusLine::info("where-used: W on a row of the Objects screen");
        return Vec::new();
    }
    if model.index().is_none() {
        model.status = StatusLine::info("objects are still loading");
        return Vec::new();
    }
    let Some((target, name)) = selected_target(model) else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
    open_where_used_on(model, target, name)
}

/// The overlay for `target` (`name` as its title says it), wherever it was
/// picked: the Objects screen's row, or a `:hygiene` row (`W`).
pub(super) fn open_where_used_on(model: &mut Model, target: Target, name: String) -> Vec<Cmd> {
    let label = (target.kind == ObjectType::Label).then(|| target.href.clone());
    let label_target = label.is_some();
    model.objects.where_used = Some(WhereUsedView::new(target, name));
    model.overlay = Overlay::WhereUsed;
    let mut cmds = Vec::new();
    if model.draft_rulesets().is_none() {
        // As a `w` form loads them (its status line is replaced below).
        cmds.extend(load_form_drafts(model));
    }
    if label.is_some()
        && model.objects.pairing_profiles.is_none()
        && !model.objects.pairing_loading
        && model.is_connected()
    {
        model.objects.pairing_loading = true;
        model.objects.pairing_error = None;
        cmds.push(Cmd::LoadPairingProfiles(CacheMode::Use));
    }
    if let Some(href) = label {
        let o = &model.objects;
        let wants_probe = model.workloads.loaded_at.is_none()
            && !model.workloads.loading
            && !o.workload_probes.contains_key(&href)
            && !o.probes_loading.contains(&href);
        if wants_probe && model.is_connected() {
            model.objects.probes_loading.insert(href.clone());
            cmds.push(Cmd::CountLabelWorkloads {
                href,
                mode: CacheMode::Use,
            });
        }
    }
    // The requests it waits for: those just sent, and those a W before
    // (or the Policy screen) sent that are still on their way.
    let rulesets_coming = if model.policy.scope == PolicyScope::Draft {
        model.policy.loading
    } else {
        model.policy.form_drafts_loading
    };
    let probe_coming = model
        .objects
        .where_used
        .as_ref()
        .is_some_and(|v| model.objects.probes_loading.contains(&v.target.href));
    let profiles_coming = label_target && model.objects.pairing_loading;
    if let Some(v) = model.objects.where_used.as_mut() {
        v.requests =
            usize::from(rulesets_coming) + usize::from(profiles_coming) + usize::from(probe_coming);
    }
    refresh_where_used(model);
    model.status = where_used_status(model);
    cmds
}

/// What the status line says while the overlay is open.
fn where_used_status(model: &Model) -> StatusLine {
    let Some(v) = model.objects.where_used.as_ref() else {
        return model.status.clone();
    };
    let what = format!("where used: {}", v.name);
    if let Some(e) = &v.failed {
        return StatusLine::warn(format!("{what} · {e}"));
    }
    if !v.waiting.is_empty() {
        return StatusLine::info(format!("{what} · loading {}…", v.waiting.join(", ")));
    }
    let n = v.total();
    StatusLine::info(format!(
        "{what} · {n} {}",
        if n == 1 { "reference" } else { "references" }
    ))
}

/// The overlay's rows again from what is loaded now; no request. Called
/// whenever something it reads lands.
pub(super) fn refresh_where_used(model: &mut Model) {
    let Some(target) = model.objects.where_used.as_ref().map(|v| v.target.clone()) else {
        return;
    };
    let mut waiting = Vec::new();
    let draft_scope = model.policy.scope == PolicyScope::Draft;
    let sets: &[pce::models::RuleSet] = match model.draft_rulesets() {
        Some(s) => s,
        None => {
            let coming = if draft_scope {
                model.policy.loading
            } else {
                model.policy.form_drafts_loading
            };
            if coming {
                waiting.push(WAIT_RULESETS);
            }
            &[]
        }
    };
    let rulesets_missing = model.draft_rulesets().is_none() && waiting.is_empty();
    // Deny rules read apart belong to the Policy screen's (draft) list.
    let empty = std::collections::HashMap::new();
    let fallback = if draft_scope {
        &model.policy.fallback_deny
    } else {
        &empty
    };
    let deny_apart = !model.policy.deny_endpoint_missing
        && !sets.is_empty()
        && sets.iter().all(|rs| rs.deny_rules.is_empty());
    // The Label groups tab's list once loaded, else the name index's (the
    // same draft list, read at startup): never a request.
    let index_groups: Vec<pce::models::LabelGroup>;
    let groups: &[pce::models::LabelGroup] =
        if model.objects.ages[ObjectKind::LabelGroups.index()].is_some() {
            model.objects.groups.rows()
        } else {
            index_groups = model
                .index()
                .map(|i| i.label_groups().into_iter().cloned().collect())
                .unwrap_or_default();
            &index_groups
        };
    let found = pce::where_used::where_used(
        &target,
        &Inputs {
            rule_sets: sets,
            fallback_deny: fallback,
            groups,
            deny_apart,
        },
    );
    let mut rows = found.refs;
    let mut profiles_failed = None;
    let mut count_failed = None;
    if target.kind == ObjectType::Label {
        match &model.objects.pairing_profiles {
            Some(list) => {
                // The key says which lock applies; the index has it.
                let key = model
                    .index()
                    .and_then(|i| i.label(&target.href))
                    .map(|l| l.key.clone())
                    .unwrap_or_default();
                rows.extend(pce::where_used::pairing_profiles(&target.href, &key, list));
            }
            None if model.objects.pairing_loading => waiting.push(WAIT_PROFILES),
            None => {
                profiles_failed = Some(match &model.objects.pairing_error {
                    Some(e) => format!("pairing profiles: {e} (W again asks)"),
                    None => "pairing profiles not loaded (W again asks)".to_string(),
                })
            }
        }
        let w = &model.workloads;
        let count = if w.loaded_at.is_some() {
            Some(WorkloadCount::from_loaded(
                &w.raw,
                w.truncated,
                &target.href,
            ))
        } else {
            model.objects.workload_probes.get(&target.href).copied()
        };
        match count {
            Some(n) => rows.push(Reference::workloads(&target.href, n)),
            None if w.loading || model.objects.probes_loading.contains(&target.href) => {
                waiting.push(WAIT_COUNT)
            }
            None => count_failed = Some("workload count not loaded (W again asks)"),
        }
    }
    let Some(v) = model.objects.where_used.as_mut() else {
        return;
    };
    v.set_rows(rows);
    v.deny_unread = found.deny_unread;
    v.waiting = waiting;
    // Every load that failed, each said.
    let failed: Vec<String> = rulesets_missing
        .then(|| "the draft rulesets are not loaded (W again asks)".to_string())
        .into_iter()
        .chain(profiles_failed)
        .chain(count_failed.map(str::to_string))
        .collect();
    v.failed = (!failed.is_empty()).then(|| failed.join(" · "));
}

/// The pairing profiles answered: kept for every later `W` on a label, the
/// overlay rebuilt.
pub(super) fn pairing_answered(
    model: &mut Model,
    result: Result<Vec<pce::models::PairingProfile>, String>,
) {
    model.objects.pairing_loading = false;
    match result {
        Ok(list) => model.objects.pairing_profiles = Some(list),
        Err(e) => {
            model.status = StatusLine::warn(format!("pairing profiles: {e}"));
            model.objects.pairing_error = Some(e);
        }
    }
    let open = model.overlay == Overlay::WhereUsed;
    refresh_where_used(model);
    if open {
        model.status = where_used_status(model);
    }
}

/// A label's probe answered: kept for the next `W` on it, the overlay
/// rebuilt.
pub(super) fn probe_answered(
    model: &mut Model,
    href: String,
    result: Result<WorkloadCount, String>,
) {
    model.objects.probes_loading.remove(&href);
    match result {
        Ok(n) => {
            model.objects.workload_probes.insert(href, n);
        }
        Err(e) => model.status = StatusLine::warn(format!("workload count: {e}")),
    }
    let open = model.overlay == Overlay::WhereUsed;
    refresh_where_used(model);
    if open {
        model.status = where_used_status(model);
    }
}

/// Something the overlay reads landed: its rows follow, and while it is
/// open the status line says where it stands.
pub(super) fn where_used_answer(model: &mut Model) {
    if model.objects.where_used.is_none() {
        return;
    }
    refresh_where_used(model);
    if model.overlay == Overlay::WhereUsed {
        model.status = where_used_status(model);
    }
}

fn where_used_page(model: &Model) -> usize {
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    crate::tui::ui::screens::objects::where_used_layout(area)
        .table
        .height
        .saturating_sub(1)
        .max(1) as usize
}

/// The overlay's keys: `Esc`/`q` close, `Enter` opens the selected
/// reference, `/` searches, the table keys move. Never a request but
/// what `Enter` opens.
pub(super) fn where_used_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let page = where_used_page(model);
    let Some(v) = model.objects.where_used.as_mut() else {
        model.overlay = Overlay::None;
        return Vec::new();
    };
    if v.table.is_searching() {
        // A search filters; it is not a move: rows landing still put the
        // cursor on the first match.
        drive_table(&mut v.table, k, page);
        return Vec::new();
    }
    match k.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            let back = v.from_hygiene && model.objects.hygiene.is_some();
            model.objects.where_used = None;
            model.overlay = Overlay::None;
            if back {
                // `W` on a `:hygiene` row: back to the report.
                reopen_hygiene(model);
            }
            Vec::new()
        }
        KeyCode::Char('?') => {
            open_help(model);
            Vec::new()
        }
        KeyCode::Enter => open_reference(model),
        _ => {
            // Only a cursor key that moves the cursor is a move (`s` sorts,
            // `/` starts a search).
            let cursor = matches!(
                k.code,
                KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::Char('j' | 'k')
                    | KeyCode::PageUp
                    | KeyCode::PageDown
                    | KeyCode::Home
                    | KeyCode::End
            );
            let before = (v.table.selected_pos(), v.selected_id());
            drive_table(&mut v.table, k, page);
            v.moved |= cursor && before != (v.table.selected_pos(), v.selected_id());
            Vec::new()
        }
    }
}

/// `Enter`: a rule or a ruleset on the Policy screen, selected; a label
/// group on the Label groups tab; the Workloads screen filtered by the
/// label. A pairing profile has no screen: the status names the command
/// that lists it, and the overlay stays.
fn open_reference(model: &mut Model) -> Vec<Cmd> {
    let Some(v) = model.objects.where_used.as_ref() else {
        return Vec::new();
    };
    let Some(r) = v.table.selected_item().cloned() else {
        model.status = StatusLine::info("nothing to open");
        return Vec::new();
    };
    let label = v.target.href.clone();
    if r.kind == RefKind::PairingProfile {
        model.status = StatusLine::info(format!(
            "pairing profile {}: illumio-tui pairing-profiles list",
            r.name.unwrap_or_default()
        ));
        return Vec::new();
    }
    // The active policy on its way: the draft cannot be shown yet. The
    // overlay stays, so Enter works once it is in.
    let to_policy = matches!(r.kind, RefKind::Rule | RefKind::Scope);
    if to_policy && model.policy.scope != PolicyScope::Draft && model.policy.loading {
        model.status = StatusLine::info("rulesets loading: Enter again once they are in");
        return Vec::new();
    }
    model.objects.where_used = None;
    model.overlay = Overlay::None;
    match r.kind {
        RefKind::Rule => {
            let rs = r.ruleset_href.unwrap_or_default();
            open_policy_rule(model, rs, r.href)
        }
        RefKind::Scope => open_policy_rule(model, r.href, String::new()),
        RefKind::LabelGroup => {
            let o = &mut model.objects;
            o.kind = ObjectKind::LabelGroups;
            o.show_detail = false;
            o.ip_lists.show_detail = false;
            o.groups.select_where(|g| g.href == r.href);
            let name = r.name.unwrap_or_default();
            model.status = if o.groups.selected_item().is_some_and(|g| g.href == r.href) {
                let page = objects_page(model);
                model.objects.groups.ensure_visible(page);
                StatusLine::info(name)
            } else {
                StatusLine::warn(format!(
                    "{name} is hidden by the Label groups search (Esc clears it)"
                ))
            };
            Vec::new()
        }
        // Answered above, the overlay kept.
        RefKind::PairingProfile => Vec::new(),
        RefKind::Workloads => {
            let Some((key, value)) = model
                .index()
                .and_then(|i| i.label(&label))
                .map(|l| (l.key.clone(), l.value.clone()))
            else {
                model.status = StatusLine::warn("the label is not in the name index");
                return Vec::new();
            };
            let filter = pce::LabelFilter {
                terms: vec![pce::LabelTerm {
                    key,
                    values: vec![value],
                    none: false,
                    negate: false,
                }],
            };
            let cmds = switch(model, Screen::Workloads);
            let expr = filter.to_string();
            if let Err(e) = apply_workload_labels(model, &expr) {
                model.status = StatusLine::error(format!("label filter: {e}"));
            } else if model.workloads.loaded_at.is_none() {
                // No count from an empty table: the list is on its way.
                model.status = StatusLine::info(format!("labels {expr} · loading workloads…"));
            }
            cmds
        }
    }
}
