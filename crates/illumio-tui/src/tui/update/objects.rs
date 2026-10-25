//! The Objects screen's transitions: labels, label groups, services and
//! IP lists, the label keys picker, the unused-labels count and the edit
//! keys on objects.
use super::*;

/// Loads every fetched kind that has neither arrived nor is in flight, and
/// the IP lists when their tab is the current one. Needs the name index: it
/// is built from the same four lists, so these are cache hits and the screen
/// costs no request.
pub(super) fn load_missing_objects(model: &mut Model) -> Vec<Cmd> {
    if model.index().is_none() {
        return Vec::new();
    }
    let missing: Vec<ObjectKind> = ObjectKind::FETCHED
        .into_iter()
        .filter(|k| model.objects.ages[k.index()].is_none() && !model.objects.loading[k.index()])
        .collect();
    let mut cmds: Vec<Cmd> = missing
        .into_iter()
        .flat_map(|k| load_objects(model, k, CacheMode::Use))
        .collect();
    if model.objects.kind == ObjectKind::IpLists {
        cmds.extend(load_missing_ip_lists(model));
    }
    cmds
}

pub(super) fn load_objects(model: &mut Model, kind: ObjectKind, mode: CacheMode) -> Vec<Cmd> {
    if kind == ObjectKind::IpLists {
        return load_ip_lists(model, mode);
    }
    model.objects.loading[kind.index()] = true;
    model.objects.auto.last_run = Some(Instant::now());
    model.objects.auto.from_tick = false;
    vec![Cmd::LoadObjects { kind, mode }]
}

pub(super) fn ip_lists_page(model: &Model) -> usize {
    use crate::tui::ui::screens::{iplists, objects};
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    crate::tui::ui::layout::chrome(area)
        .map(|c| {
            iplists::table_rows(
                objects::strip_split(c.body).1,
                model.objects.ip_lists.show_detail,
            )
        })
        .unwrap_or(10)
        .max(1)
}

/// Loads the IP lists once (the Objects screen's IP lists tab). Waits for the
/// name index like the other kinds: the index reads the same list, so after
/// it the load is a cache hit.
pub(super) fn load_missing_ip_lists(model: &mut Model) -> Vec<Cmd> {
    if model.index().is_none()
        || model.objects.ip_lists.loaded_at.is_some()
        || model.objects.ip_lists.loading
    {
        return Vec::new();
    }
    load_ip_lists(model, CacheMode::Use)
}

pub(super) fn load_ip_lists(model: &mut Model, mode: CacheMode) -> Vec<Cmd> {
    model.objects.ip_lists.loading = true;
    model.objects.ip_lists.auto.last_run = Some(Instant::now());
    model.objects.ip_lists.auto.from_tick = false;
    vec![Cmd::LoadIpLists(mode)]
}

pub(super) fn ip_lists_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let page = ip_lists_page(model);
    if !model.objects.ip_lists.table.is_searching() {
        match k.code {
            KeyCode::Char('r') => {
                if model.index().is_none() {
                    if model.objects.index_failed {
                        model.objects.index_failed = false;
                        model.workloads.index_failed = false;
                        model.status = StatusLine::info("loading name index…");
                        return vec![Cmd::LoadIndex(CacheMode::Use)];
                    }
                    model.status = StatusLine::info("objects are still loading");
                    return Vec::new();
                }
                if model.objects.ip_lists.loading {
                    model.status = StatusLine::info("IP lists already loading");
                    return Vec::new();
                }
                model.objects.ip_lists.auto.rearm();
                return load_ip_lists(model, CacheMode::Bypass);
            }
            KeyCode::Enter => {
                let s = &mut model.objects.ip_lists;
                s.show_detail = !s.show_detail && s.table.selected_item().is_some();
                return Vec::new();
            }
            KeyCode::Char('a') => {
                model.status = model.objects.ip_lists.auto.toggle();
                return Vec::new();
            }
            KeyCode::Char('W') => return open_where_used(model),
            KeyCode::Char(c @ ('n' | 'e' | 'x')) => return ip_lists_edit_key(model, c),
            _ => {}
        }
    }
    drive_table(&mut model.objects.ip_lists.table, k, page);
    Vec::new()
}

/// `n` / `e` / `x` on IP lists, through the Plan 4 intents.
pub(super) fn ip_lists_edit_key(model: &mut Model, c: char) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    let selected = model
        .objects
        .ip_lists
        .table
        .selected_item()
        .map(|r| r.list.clone());
    let intent = match c {
        'n' => Some(EditIntent::CreateIpList),
        'e' => selected.map(EditIntent::UpdateIpList),
        _ => selected.map(EditIntent::DeleteIpList),
    };
    let Some(intent) = intent else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
    begin_edit(model, intent)
}

pub(super) fn objects_page(model: &Model) -> usize {
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    crate::tui::ui::layout::chrome(area)
        .map(|c| crate::tui::ui::screens::objects::table_rows(c.body, model.objects.show_detail))
        .unwrap_or(10)
        .max(1)
}

pub(super) fn objects_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let page = objects_page(model);
    if !model.objects.is_searching() {
        let kind = model.objects.kind;
        match k.code {
            KeyCode::Tab | KeyCode::BackTab => {
                let o = &mut model.objects;
                o.kind = if k.code == KeyCode::Tab {
                    kind.next()
                } else {
                    kind.prev()
                };
                o.show_detail = false;
                o.ip_lists.show_detail = false;
                // The first visit to the IP lists tab loads them (a cache hit).
                return if o.kind == ObjectKind::IpLists {
                    load_missing_ip_lists(model)
                } else {
                    Vec::new()
                };
            }
            _ if kind == ObjectKind::IpLists => return ip_lists_key(model, k),
            KeyCode::Char('r') => {
                if model.index().is_none() {
                    if model.objects.index_failed {
                        model.objects.index_failed = false;
                        model.workloads.index_failed = false;
                        model.status = StatusLine::info("loading name index…");
                        return vec![Cmd::LoadIndex(CacheMode::Use)];
                    }
                    model.status = StatusLine::info("objects are still loading");
                } else if model.objects.loading[kind.index()] {
                    model.status = StatusLine::info(format!("{} already loading", kind.title()));
                } else {
                    model.objects.auto.rearm();
                    let mut cmds = load_objects(model, kind, CacheMode::Bypass);
                    if kind == ObjectKind::Labels {
                        cmds.extend(reload_usage_with_labels(model));
                    }
                    return cmds;
                }
                return Vec::new();
            }
            KeyCode::Enter => {
                let o = &mut model.objects;
                o.show_detail = !o.show_detail && o.has_selection();
                return Vec::new();
            }
            KeyCode::Char('a') => {
                model.status = model.objects.auto.toggle();
                return Vec::new();
            }
            KeyCode::Char('W') => return open_where_used(model),
            KeyCode::Char(c @ ('n' | 'e' | 'x')) => return objects_edit_key(model, c),
            KeyCode::Char('f') if kind == ObjectKind::Labels => {
                model.overlay = Overlay::LabelKeys(open_label_key_picker(model));
                return Vec::new();
            }
            KeyCode::Char('i') if kind == ObjectKind::Labels => {
                toggle_ignored_labels(model);
                return Vec::new();
            }
            KeyCode::Char('u') if kind == ObjectKind::Labels => return unused_labels_key(model),
            _ => {}
        }
    }
    let o = &mut model.objects;
    match o.kind {
        ObjectKind::Labels => drive_table(&mut o.labels, k, page),
        ObjectKind::LabelGroups => drive_table(&mut o.groups, k, page),
        ObjectKind::Services => drive_table(&mut o.services, k, page),
        // A search on the IP lists tab: its own keys and page size.
        ObjectKind::IpLists => return ip_lists_key(model, k),
    }
    Vec::new()
}

/// `f` on the Labels tab: the profile's scope and tier keys, then every
/// key the index knows with its label count (ignored keys only while `i`
/// shows them); the current filter checked.
pub(super) fn open_label_key_picker(model: &Model) -> KeyPicker {
    let order = model.limits.key_order();
    let hide = model.objects.hide_ignored;
    let seen: Vec<(String, usize)> = model
        .index()
        .map(|idx| {
            idx.label_keys()
                .into_iter()
                // Keys `i` hides are not offered while hidden.
                .filter(|k| !hide || order.shows(k))
                .map(|k| {
                    let n = idx.values_for_key(&k).len();
                    (k, n)
                })
                .collect()
        })
        .unwrap_or_default();
    let keys = &model.objects.label_keys;
    KeyPicker::new(
        &model.limits.scope_keys,
        &model.limits.tier_key,
        &seen,
        keys,
        keys.is_empty(),
    )
}

/// Enter in the label key picker: the Labels tab shows these keys (every
/// key when empty). No request.
pub(super) fn apply_label_keys(model: &mut Model, keys: Vec<String>) {
    model.status = StatusLine::info(if keys.is_empty() {
        "labels: all keys".to_string()
    } else {
        format!("labels: key {}", keys.join(","))
    });
    model.objects.label_keys = keys;
    model.objects.rebuild_labels(&model.limits.key_order());
}

/// `i` on the Labels tab: show or hide the labels of the profile's `ignore_keys`.
pub(super) fn toggle_ignored_labels(model: &mut Model) {
    if model.limits.ignore_keys.is_empty() {
        model.status = StatusLine::info("the profile has no ignore_keys");
        return;
    }
    let o = &mut model.objects;
    o.hide_ignored = !o.hide_ignored;
    o.rebuild_labels(&model.limits.key_order());
    let keys = model.limits.ignore_keys.join(",");
    model.status = StatusLine::info(if model.objects.hide_ignored {
        format!("ignored keys hidden ({keys})")
    } else {
        format!("ignored keys shown ({keys})")
    });
}

/// `u` on the Labels tab: unused labels only, or all again. The PCE's own
/// usage flags decide: one `usage=true` labels request per `u` (cached like
/// the labels), the filter applying when it answers (at once with flags
/// already in hand). A PCE without the flags falls back to our counts.
pub(super) fn unused_labels_key(model: &mut Model) -> Vec<Cmd> {
    let o = &mut model.objects;
    if o.unused_only {
        o.unused_only = false;
        o.rebuild_labels(&model.limits.key_order());
        model.status = StatusLine::info("labels: all, used or not");
        return Vec::new();
    }
    if o.pce_usage == PceUsage::Unsupported {
        return counted_unused_key(model);
    }
    if !model.is_connected() {
        model.status = StatusLine::warn("unused labels: not connected");
        return Vec::new();
    }
    let o = &mut model.objects;
    o.unused_only = true;
    o.rebuild_labels(&model.limits.key_order());
    model.status = unused_status(&model.objects);
    if std::mem::replace(&mut model.objects.pce_usage_loading, true) {
        return Vec::new(); // already asked
    }
    vec![Cmd::LoadLabelUsage(CacheMode::Use)]
}

/// The PCE's usage answer: its flags decide `u`; without them (an older
/// PCE) `u` goes on with our own counts, asking for what they need.
pub(super) fn label_usage_loaded(
    model: &mut Model,
    result: Result<Vec<pce::models::Label>, String>,
) -> Vec<Cmd> {
    let o = &mut model.objects;
    o.pce_usage_loading = false;
    match result {
        Ok(labels) => {
            o.pce_usage = PceUsage::from_labels(labels);
            if o.pce_usage == PceUsage::Unsupported && o.unused_only {
                o.unused_only = false;
                o.rebuild_labels(&model.limits.key_order());
                return counted_unused_key(model);
            }
            o.rebuild_labels(&model.limits.key_order());
            if o.unused_only {
                model.status = unused_status(&model.objects);
            }
        }
        Err(e) => {
            if o.unused_only && !o.unused_decided() {
                o.unused_only = false;
                o.rebuild_labels(&model.limits.key_order());
            }
            fail(model, "label usage", &e, false);
        }
    }
    Vec::new()
}

/// What `u` shows in the status line once on.
pub(super) fn unused_status(o: &ObjectsState) -> StatusLine {
    use crate::cli::output::USAGE_FROM_COUNTS;
    let n = o.labels.len();
    match (&o.pce_usage, &o.label_usage, o.usage_truncated) {
        (PceUsage::Flags(_), ..) => StatusLine::info(format!("{n} unused labels")),
        (PceUsage::Unknown, ..) => {
            StatusLine::info("unused labels: asking the PCE what holds each label")
        }
        (PceUsage::Unsupported, None, _) => {
            StatusLine::info("unused labels: waiting for workloads and policy")
        }
        (PceUsage::Unsupported, Some(_), true) => StatusLine::warn(format!(
            "{n} unused labels; the workload list is truncated, some may be in use"
        )),
        (PceUsage::Unsupported, Some(_), false) => StatusLine::info(format!(
            "{n} unused labels (usage from {USAGE_FROM_COUNTS})"
        )),
    }
}

/// `u` from a PCE without usage flags: our counts need the workloads and
/// the rulesets; when one is neither cached nor on its way the reads wait
/// for a `y` (no request before it).
pub(super) fn counted_unused_key(model: &mut Model) -> Vec<Cmd> {
    let missing = |loaded: bool, loading: bool| !loaded && !loading;
    let workloads = missing(model.workloads.loaded_at.is_some(), model.workloads.loading);
    let policy = missing(model.policy.loaded_at.is_some(), model.policy.loading);
    if workloads || policy {
        let reason = match (workloads, policy) {
            (true, false) => "unused needs workloads loaded: press y to load workloads".to_string(),
            (false, _) => "unused needs policy loaded: press y to load policy \
                 (it may also fetch the selected ruleset's deny rules)"
                .to_string(),
            (true, true) => "unused needs workloads and policy loaded: press y to load both \
                 (policy may also fetch the selected ruleset's deny rules)"
                .to_string(),
        };
        model.status = StatusLine::info(reason.clone());
        model.overlay = Overlay::Confirm {
            reason,
            action: PendingAction::LoadLabelUsage,
        };
        return Vec::new();
    }
    apply_unused(model);
    Vec::new()
}

/// Turns `unused_only` on once usage can be counted or is on its way;
/// otherwise leaves it off and says what is missing.
pub(super) fn apply_unused(model: &mut Model) {
    if let Some(gap) = usage_gap(model) {
        model.status = StatusLine::warn(gap);
        return;
    }
    model.objects.unused_only = true;
    model.recount_labels();
    model.status = unused_status(&model.objects);
}

/// Why label usage cannot be counted: what is neither cached nor loading.
/// `None` when usage is ready or every missing part is in flight.
pub(super) fn usage_gap(model: &Model) -> Option<String> {
    if model.usage_ready() {
        return None;
    }
    let groups = ObjectKind::LabelGroups.index();
    let parts = [
        (
            "workloads",
            model.workloads.loaded_at.is_some(),
            model.workloads.loading,
        ),
        (
            "policy",
            model.policy.loaded_at.is_some(),
            model.policy.loading,
        ),
        (
            "label groups",
            model.objects.ages[groups].is_some(),
            model.objects.loading[groups],
        ),
    ];
    let missing: Vec<&str> = parts
        .iter()
        .filter(|(_, loaded, loading)| !loaded && !loading)
        .map(|(name, _, _)| *name)
        .collect();
    if missing.is_empty() {
        return None; // all on their way
    }
    let index = if model.index().is_none() {
        " (no name index yet)"
    } else {
        ""
    };
    Some(format!(
        "unused labels need {} loaded{index}",
        missing.join(" and ")
    ))
}

/// `y` on the `u` confirm: the workloads and the rulesets, each through its
/// screen's own load unless cached or on its way; the filter applies when
/// they have arrived. Nothing to load and nothing coming: the filter stays
/// off and the status line names the gap.
pub(super) fn load_label_usage(model: &mut Model) -> Vec<Cmd> {
    if !model.is_connected() {
        model.status = StatusLine::warn("unused labels: not connected");
        return Vec::new();
    }
    if model.index().is_none() {
        // The workloads wait for the index: load nothing rather than half.
        if let Some(gap) = usage_gap(model) {
            model.status = StatusLine::warn(gap);
        }
        return Vec::new();
    }
    let mut cmds = load_missing_workloads(model);
    if model.policy.loaded_at.is_none() && !model.policy.loading {
        cmds.extend(load_rule_sets(model, CacheMode::Use));
    }
    apply_unused(model);
    if !cmds.is_empty() && model.objects.unused_only {
        model.status = StatusLine::info("unused labels: loading workloads and policy");
    }
    cmds
}

/// `n` / `e` / `x` on Objects: labels and services; label groups are read-only.
pub(super) fn objects_edit_key(model: &mut Model, c: char) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    let o = &model.objects;
    let intent = match (o.kind, c) {
        (ObjectKind::IpLists, _) => return ip_lists_edit_key(model, c),
        (ObjectKind::LabelGroups, _) => {
            model.status = StatusLine::warn("label groups are read-only in this version");
            return Vec::new();
        }
        (ObjectKind::Labels, 'n') => Some(EditIntent::CreateLabel),
        (ObjectKind::Services, 'n') => Some(EditIntent::CreateService),
        (ObjectKind::Labels, 'e') => o
            .labels
            .selected_item()
            .map(|r| EditIntent::UpdateLabel(r.label.clone())),
        (ObjectKind::Labels, _) => o
            .labels
            .selected_item()
            .map(|r| EditIntent::DeleteLabel(r.label.clone())),
        (ObjectKind::Services, 'e') => o
            .services
            .selected_item()
            .cloned()
            .map(EditIntent::UpdateService),
        (ObjectKind::Services, _) => o
            .services
            .selected_item()
            .cloned()
            .map(EditIntent::DeleteService),
    };
    let Some(intent) = intent else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
    begin_edit(model, intent)
}
