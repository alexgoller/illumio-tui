//! `:hygiene`: the report overlay. It reads what is loaded (the object
//! lists from the name index or the Objects tabs, the draft rulesets as the
//! Policy screen loads them, the label groups, the pairing profiles, the
//! workload list) and loads what is missing once, on the command, through
//! the cache: the draft rulesets, the pairing profiles and the workloads.
//! Its rows follow every answer; nothing runs on a timer, and reopening it
//! loads nothing again.
use super::*;
use crate::tui::hygiene::{
    HygieneView, OBJECT_KINDS, WAIT_PROFILES, WAIT_RULESETS, WAIT_USAGE, WAIT_WORKLOADS,
};
use pce::hygiene::{Gaps, Kind, Objects, Unused, Used};
use pce::where_used::{Inputs, ObjectType, Target};

/// `:hygiene`: the report. The draft rulesets (as a `w` form loads them),
/// the pairing profiles, the workload list and the labels with the PCE's
/// usage flags (as `u` asks) are each loaded when neither held nor on
/// their way: one cached request each.
pub(super) fn open_hygiene(model: &mut Model) -> Vec<Cmd> {
    if model.index().is_none() {
        model.status = StatusLine::info("objects are still loading");
        return Vec::new();
    }
    // `loc` is expected only when the PCE has that key.
    let has_loc = model
        .index()
        .is_some_and(|i| i.labels().iter().any(|l| !l.is_deleted() && l.key == "loc"));
    let keys =
        crate::config::hygiene_keys(&model.limits.scope_keys, &model.limits.tier_key, has_loc);
    model.objects.hygiene = Some(HygieneView::new(keys));
    model.objects.where_used = None;
    model.overlay = Overlay::Hygiene;
    let mut cmds = Vec::new();
    if model.draft_rulesets().is_none() {
        // As a `w` form loads them (its status line is replaced below).
        cmds.extend(load_form_drafts(model));
    }
    if model.objects.pairing_profiles.is_none()
        && !model.objects.pairing_loading
        && model.is_connected()
    {
        model.objects.pairing_loading = true;
        model.objects.pairing_error = None;
        cmds.push(Cmd::LoadPairingProfiles(CacheMode::Use));
    }
    if model.workloads.loaded_at.is_none() && !model.workloads.loading && model.is_connected() {
        cmds.extend(load_workloads(model, CacheMode::Use));
    }
    // The PCE's own word on what holds each label: holders this tool
    // cannot read (container workloads, permissions, mapping rules ...).
    if model.objects.pce_usage == PceUsage::Unknown
        && !model.objects.pce_usage_loading
        && model.is_connected()
    {
        model.objects.pce_usage_loading = true;
        cmds.push(Cmd::LoadLabelUsage(CacheMode::Use));
    }
    // The requests it waits for: those just sent, and those sent before
    // (by the Policy or Workloads screen, a `W`) still on their way.
    let rulesets_coming = if model.policy.scope == PolicyScope::Draft {
        model.policy.loading
    } else {
        model.policy.form_drafts_loading
    };
    let coming = usize::from(rulesets_coming)
        + usize::from(model.objects.pairing_loading)
        + usize::from(model.workloads.loading)
        + usize::from(model.objects.pce_usage_loading);
    if let Some(v) = model.objects.hygiene.as_mut() {
        v.requests = coming;
    }
    refresh_hygiene(model);
    model.status = hygiene_status(model);
    cmds
}

/// Back to the report after `W` checked one of its rows: rebuilt from
/// what is loaded now, no request.
pub(super) fn reopen_hygiene(model: &mut Model) {
    model.overlay = Overlay::Hygiene;
    refresh_hygiene(model);
    model.status = hygiene_status(model);
}

/// What the status line says while the overlay is open.
fn hygiene_status(model: &Model) -> StatusLine {
    let Some(v) = model.objects.hygiene.as_ref() else {
        return model.status.clone();
    };
    if let Some(e) = &v.failed {
        return StatusLine::warn(format!("hygiene · {e}"));
    }
    if !v.waiting.is_empty() {
        return StatusLine::info(format!("hygiene · loading {}…", v.waiting.join(", ")));
    }
    let (u, m) = (v.unused_total(), v.missing_total());
    StatusLine::info(format!(
        "hygiene · {u} unused {} · {m} workload{} missing labels",
        if u == 1 { "object" } else { "objects" },
        if m == 1 { "" } else { "s" }
    ))
}

/// The overlay's rows again from what is loaded now; no request. Called
/// whenever something it reads lands.
pub(super) fn refresh_hygiene(model: &mut Model) {
    if model.objects.hygiene.is_none() {
        return;
    }
    let Some(idx) = model.index().cloned() else {
        return;
    };
    let mut waiting = Vec::new();
    let mut failed = Vec::new();
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
            } else {
                failed.push("the draft rulesets are not loaded (:hygiene again asks)".to_string());
            }
            &[]
        }
    };
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
    // Each list as its Objects tab holds it once loaded (a write reloads
    // it), else the name index's (the same draft list, read at startup).
    let o = &model.objects;
    let tab = |k: ObjectKind| o.ages[k.index()].is_some();
    let mut labels: Vec<pce::models::Label> = if tab(ObjectKind::Labels) {
        o.all_labels.clone()
    } else {
        idx.labels().into_iter().cloned().collect()
    };
    // The PCE's usage flags, once `u` or `:hygiene` asked: they settle a
    // label whatever this tool could read. A failed or flagless answer
    // leaves the scan to decide.
    match &o.pce_usage {
        PceUsage::Flags(m) => {
            for l in &mut labels {
                l.usage = m.get(&l.href).cloned();
            }
        }
        _ if o.pce_usage_loading => waiting.push(WAIT_USAGE),
        _ => {}
    }
    let groups: Vec<pce::models::LabelGroup> = if tab(ObjectKind::LabelGroups) {
        o.groups.rows().to_vec()
    } else {
        idx.label_groups().into_iter().cloned().collect()
    };
    let ip_lists: Vec<pce::models::IpList> = if o.ip_lists.loaded_at.is_some() {
        o.ip_lists
            .table
            .rows()
            .iter()
            .map(|r| r.list.clone())
            .collect()
    } else {
        idx.ip_lists().into_iter().cloned().collect()
    };
    let services: Vec<pce::models::Service> = if tab(ObjectKind::Services) {
        o.services.rows().to_vec()
    } else {
        idx.services().into_iter().cloned().collect()
    };
    let profiles: &[pce::models::PairingProfile] = match &o.pairing_profiles {
        Some(list) => list,
        None => {
            if o.pairing_loading {
                waiting.push(WAIT_PROFILES);
            } else {
                failed.push(match &o.pairing_error {
                    Some(e) => format!("pairing profiles: {e} (:hygiene again asks)"),
                    None => "pairing profiles not loaded (:hygiene again asks)".to_string(),
                });
            }
            &[]
        }
    };
    let w = &model.workloads;
    let workloads: &[pce::models::Workload] = if w.loaded_at.is_some() {
        &w.raw
    } else {
        if w.loading {
            waiting.push(WAIT_WORKLOADS);
        } else {
            failed.push("workloads not loaded (:hygiene again asks)".to_string());
        }
        &[]
    };
    let used = Used::new(
        &Inputs {
            rule_sets: sets,
            fallback_deny: fallback,
            groups: &groups,
            deny_apart,
        },
        profiles,
        workloads,
    );
    // Unused rows wait for every list that may name an object; a list
    // that failed or came back cut short withholds the kinds it may hold.
    let complete = waiting.is_empty();
    let gaps = Gaps {
        rule_sets: model.draft_rulesets().is_none(),
        groups: false,
        profiles: o.pairing_profiles.is_none(),
        workloads: w.loaded_at.is_none() || w.truncated,
    };
    let found = if complete {
        pce::hygiene::unused(
            &Objects {
                labels: Some(&labels),
                groups: Some(&groups),
                ip_lists: Some(&ip_lists),
                services: Some(&services),
            },
            &used,
            gaps,
        )
    } else {
        Unused::default()
    };
    let mut rows = found.rows;
    let keys = model
        .objects
        .hygiene
        .as_ref()
        .map(|v| v.keys.clone())
        .unwrap_or_default();
    let missing = pce::hygiene::missing_labels(workloads, &keys);
    let listed: Vec<Kind> = OBJECT_KINDS
        .into_iter()
        .filter(|k| !found.withheld.contains(k))
        .collect();
    let unused = if complete {
        pce::hygiene::counts_by_kind(&rows, &listed)
    } else {
        Vec::new()
    };
    let per_key = pce::hygiene::counts_by_key(&missing, &keys);
    rows.extend(missing);
    let loaded = w.loaded_at.is_some().then_some(w.raw.len());
    let truncated = w.truncated;
    let Some(v) = model.objects.hygiene.as_mut() else {
        return;
    };
    v.set_rows(rows);
    v.unused = unused;
    v.withheld = found.withheld;
    v.uncertain = found.uncertain;
    v.gaps = gaps.text();
    v.missing = per_key;
    v.workloads = loaded;
    v.workloads_truncated = truncated && loaded.is_some();
    v.deny_unread = used.deny_unread;
    v.waiting = waiting;
    v.failed = (!failed.is_empty()).then(|| failed.join(" · "));
}

/// Something the overlay reads landed: its rows follow, and while it is
/// open the status line says where it stands. A tab `Enter` waited for
/// gets its row selected.
pub(super) fn hygiene_answer(model: &mut Model) {
    select_waiting_row(model);
    if model.objects.hygiene.is_none() {
        return;
    }
    refresh_hygiene(model);
    if model.overlay == Overlay::Hygiene {
        model.status = hygiene_status(model);
    }
}

fn hygiene_page(model: &Model) -> usize {
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    let Some(v) = model.objects.hygiene.as_ref() else {
        return 1;
    };
    crate::tui::ui::screens::objects::hygiene_layout(area, v)
        .table
        .height
        .saturating_sub(1)
        .max(1) as usize
}

/// The overlay's keys: `Esc`/`q` close, `Enter` opens the row on its
/// screen, `W` runs where-used for an unused object, `/` searches, the
/// table keys move. Never a request but what `Enter` or `W` opens.
pub(super) fn hygiene_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let page = hygiene_page(model);
    let Some(v) = model.objects.hygiene.as_mut() else {
        model.overlay = Overlay::None;
        return Vec::new();
    };
    if v.table.is_searching() {
        drive_table(&mut v.table, k, page);
        return Vec::new();
    }
    match k.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            model.objects.hygiene = None;
            model.overlay = Overlay::None;
            Vec::new()
        }
        KeyCode::Char('?') => {
            open_help(model);
            Vec::new()
        }
        KeyCode::Enter => open_row(model),
        KeyCode::Char('W') => check_row(model),
        _ => {
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

/// The Objects tab of an object kind.
fn tab_of(kind: Kind) -> Option<ObjectKind> {
    match kind {
        Kind::Label => Some(ObjectKind::Labels),
        Kind::LabelGroup => Some(ObjectKind::LabelGroups),
        Kind::IpList => Some(ObjectKind::IpLists),
        Kind::Service => Some(ObjectKind::Services),
        Kind::Workload => None,
    }
}

/// `W`: where-used for the unused object under the cursor, as a check;
/// `Esc` there comes back here.
fn check_row(model: &mut Model) -> Vec<Cmd> {
    let Some(r) = model
        .objects
        .hygiene
        .as_ref()
        .and_then(|v| v.table.selected_item().cloned())
    else {
        model.status = StatusLine::info("nothing to check");
        return Vec::new();
    };
    let kind = match r.kind {
        Kind::Label => ObjectType::Label,
        Kind::LabelGroup => ObjectType::LabelGroup,
        Kind::IpList => ObjectType::IpList,
        Kind::Service => ObjectType::Service,
        Kind::Workload => {
            model.status = StatusLine::info("W checks an unused object; Enter opens the workload");
            return Vec::new();
        }
    };
    let cmds = open_where_used_on(model, Target::new(kind, r.href), r.name);
    if let Some(v) = model.objects.where_used.as_mut() {
        v.from_hygiene = true;
    }
    cmds
}

/// `Enter`: an object on its Objects tab, selected (where `x` deletes it
/// with the usual confirm); a workload on the Workloads screen, its detail
/// open. A tab or list still loading selects the row when it arrives.
fn open_row(model: &mut Model) -> Vec<Cmd> {
    let Some(r) = model
        .objects
        .hygiene
        .as_ref()
        .and_then(|v| v.table.selected_item().cloned())
    else {
        model.status = StatusLine::info("nothing to open");
        return Vec::new();
    };
    model.objects.hygiene = None;
    model.overlay = Overlay::None;
    let Some(tab) = tab_of(r.kind) else {
        let cmds = switch(model, Screen::Workloads);
        let w = &mut model.workloads;
        w.show_detail = w.select_href(&r.href);
        w.open_on_load = None;
        if !w.show_detail {
            if w.loaded_at.is_some() {
                model.status = StatusLine::warn(format!(
                    "{} is hidden by the Workloads filters (c clears them)",
                    r.name
                ));
            } else {
                // Not in the list yet: opened when the list arrives.
                w.open_on_load = Some(r.href);
                model.status = StatusLine::info("loading the workloads: its detail opens then");
            }
        }
        return cmds;
    };
    // The tab first: `switch` loads the IP lists only for their own tab.
    let o = &mut model.objects;
    o.kind = tab;
    o.show_detail = false;
    o.ip_lists.show_detail = false;
    o.select_on_load = Some((tab, r.href));
    let cmds = switch(model, Screen::Objects);
    if !select_waiting_row(model) {
        model.status = StatusLine::info(format!(
            "loading {}: {} is selected then",
            tab.title(),
            r.name
        ));
    }
    cmds
}

/// The row `Enter` waits for, selected once its tab is loaded; whether
/// the tab was there (the wait is over either way).
fn select_waiting_row(model: &mut Model) -> bool {
    let Some((tab, href)) = model.objects.select_on_load.clone() else {
        return false;
    };
    let o = &model.objects;
    let loaded = match tab {
        ObjectKind::IpLists => o.ip_lists.loaded_at.is_some(),
        k => o.ages[k.index()].is_some(),
    };
    if !loaded {
        return false;
    }
    model.objects.select_on_load = None;
    let page = objects_page(model);
    let ip_page = ip_lists_page(model);
    let o = &mut model.objects;
    let name = match tab {
        ObjectKind::Labels => {
            o.labels.select_where(|r| r.href == href);
            o.labels.ensure_visible(page);
            o.labels
                .selected_item()
                .filter(|r| r.href == href)
                .map(|r| format!("{}={}", r.key, r.value))
        }
        ObjectKind::LabelGroups => {
            o.groups.select_where(|g| g.href == href);
            o.groups.ensure_visible(page);
            o.groups
                .selected_item()
                .filter(|g| g.href == href)
                .map(|g| g.name.clone())
        }
        ObjectKind::Services => {
            o.services.select_where(|s| s.href == href);
            o.services.ensure_visible(page);
            o.services
                .selected_item()
                .filter(|s| s.href == href)
                .map(|s| s.name.clone())
        }
        ObjectKind::IpLists => {
            let t = &mut o.ip_lists.table;
            t.select_where(|r| r.list.href == href);
            t.ensure_visible(ip_page);
            t.selected_item()
                .filter(|r| r.list.href == href)
                .map(|r| r.list.name.clone())
        }
    };
    model.status = match name {
        Some(n) => StatusLine::info(format!("{n} · x deletes it (with the usual confirm)")),
        None => StatusLine::warn(format!(
            "{href} is not on the {} tab: hidden by its search or filters, or gone",
            tab.title()
        )),
    };
    true
}
