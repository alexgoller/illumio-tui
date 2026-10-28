//! Compare mode on the Workloads screen (#85): `b` and its snapshot
//! picker, the baseline loaded, `n`, `C`. The snapshot files only:
//! nothing here asks the PCE.
use super::*;
use crate::tui::model::{WlCompare, WorkloadsState, wl_gone};
use crate::tui::msg::WlBaseline;

/// `b`: the profile's snapshots that hold workloads, for the picker.
/// Needs the list loaded (it is what is compared).
pub(super) fn baseline_key(model: &mut Model) -> Vec<Cmd> {
    if model.workloads.loaded_at.is_none() {
        model.status = StatusLine::info(if model.workloads.loading {
            "workloads are still loading: b once they are in"
        } else {
            "no workloads loaded: r loads them"
        });
        return Vec::new();
    }
    let Some(root) = &model.snapshots_root else {
        model.status =
            StatusLine::error("no data directory for snapshots: set HOME or XDG_DATA_HOME");
        return Vec::new();
    };
    let dir = crate::snapshot::profile_dir(root, &model.profile.name);
    vec![Cmd::ListBaselines { dir }]
}

/// The snapshots listed: the picker, newest first, on `latest` (or on the
/// baseline in use).
pub(super) fn baselines_listed(
    model: &mut Model,
    result: Result<Vec<crate::snapshot::SnapshotMeta>, String>,
) {
    match result {
        Err(e) => model.status = StatusLine::error(format!("snapshots: {e}")),
        Ok(all) if all.is_empty() => {
            model.status = StatusLine::info(
                "no snapshot of this profile holds workloads: :snapshot saves one",
            );
        }
        Ok(mut items) => {
            items.reverse();
            let selected = model
                .workloads
                .compare
                .as_ref()
                .and_then(|c| items.iter().position(|m| m.id == c.id))
                .unwrap_or(0);
            model.overlay = Overlay::SnapshotPicker { items, selected };
        }
    }
}

/// Keys of the picker: `←`/`→` (or `h`/`l`, `j`/`k`) older and newer,
/// `Enter` compares with the one shown, `Esc` closes.
pub(super) fn picker_key(
    model: &mut Model,
    items: Vec<crate::snapshot::SnapshotMeta>,
    selected: usize,
    k: KeyEvent,
) -> Vec<Cmd> {
    let last = items.len().saturating_sub(1);
    let selected = match k.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            model.overlay = Overlay::None;
            return Vec::new();
        }
        KeyCode::Enter => {
            model.overlay = Overlay::None;
            let Some(m) = items.get(selected) else {
                return Vec::new();
            };
            return vec![Cmd::LoadWlBaseline {
                path: m.path.clone(),
                id: m.id.clone(),
            }];
        }
        KeyCode::Left | KeyCode::Down | KeyCode::Char('h' | 'j') => (selected + 1).min(last),
        KeyCode::Right | KeyCode::Up | KeyCode::Char('l' | 'k') => selected.saturating_sub(1),
        KeyCode::Home | KeyCode::Char('g') => 0,
        KeyCode::End | KeyCode::Char('G') => last,
        _ => selected,
    };
    model.overlay = Overlay::SnapshotPicker { items, selected };
    Vec::new()
}

/// The snapshot picked: compare mode on (or on another snapshot), the
/// rows compared; else why not, compare mode as it was.
pub(super) fn baseline_loaded(model: &mut Model, result: Result<WlBaseline, String>) {
    match result {
        Err(e) => model.status = StatusLine::error(format!("baseline: {e}")),
        Ok(b) => {
            refilter(model, |w| w.compare = Some(WlCompare::new(b)));
            model.status = compare_status(&model.workloads);
        }
    }
}

/// `vs snapshot 10-20 06:00 · 3 new · 5 changed · 1 gone`; a warning when
/// a side is capped.
pub(super) fn compare_status(w: &WorkloadsState) -> StatusLine {
    let Some(text) = w.compare_meta() else {
        return StatusLine::info("compare off");
    };
    match w.compare_warning() {
        Some(warn) => StatusLine::warn(format!("{text} · {warn}")),
        None => StatusLine::info(text),
    }
}

/// `n` in compare mode: all → new → changed → gone. No request.
pub(super) fn cycle_changes(model: &mut Model) {
    refilter(model, |w| {
        if let Some(c) = w.compare.as_mut() {
            c.show = c.show.next();
        }
    });
    let show = model
        .workloads
        .compare
        .as_ref()
        .map_or("all", |c| c.show.label());
    model.status = StatusLine::info(format!("compare: {show} rows"));
}

/// `C`: compare mode off. No request.
pub(super) fn leave_compare(model: &mut Model) {
    let mut was = false;
    refilter(model, |w| was = w.compare.take().is_some());
    model.status = StatusLine::info(if was {
        "compare off"
    } else {
        "not comparing: b compares with a snapshot"
    });
}

/// A key that changes or acts on the workload under the cursor, on a gone
/// row (compare mode): refused, said. `true` when refused.
pub(super) fn refuse_gone(model: &mut Model, k: KeyEvent) -> bool {
    let gone = model.workloads.selected_row().is_some_and(|r| wl_gone(r));
    let acts = match k.code {
        KeyCode::Char(' ' | 'e' | 'X' | 't' | 'T') => true,
        KeyCode::Char('L' | 'M') => model.workloads.marked.is_empty(),
        _ => false,
    };
    if gone && acts {
        model.status = StatusLine::info("gone since the snapshot: no workload to change or query");
    }
    gone && acts
}
