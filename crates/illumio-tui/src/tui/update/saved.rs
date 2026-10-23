//! Saved filters on the current screen: `:save NAME`, `:filters` (`:fl`),
//! `:load NAME` (and `:filter NAME`). The config file is read and written
//! here (a local file, as `marks.toml` is); applying a filter goes through
//! the screen's `f` path, so Traffic runs its one query through the cost
//! guard and the others filter locally.
use super::*;
use crate::filters::Saved;
use crate::tui::saved::{self, FilterPicker, PickerOutcome};

/// The current screen's saved filters, or why it has none.
fn screen_of(model: &Model) -> Result<crate::filters::Screen, String> {
    saved::filter_screen(model.screen)
        .ok_or_else(|| "saved filters: Traffic, Policy, Workloads and Events".to_string())
}

/// `:save NAME`: the filter in force on the screen, under `NAME`; an
/// existing name asks first.
pub(super) fn save_filter(model: &mut Model, name: &str) -> Vec<Cmd> {
    let screen = match screen_of(model).and_then(|s| {
        crate::filters::check_name(name)?;
        Ok(s)
    }) {
        Ok(s) => s,
        Err(e) => {
            model.status = StatusLine::error(e);
            return Vec::new();
        }
    };
    let Some(current) = saved::current(model, model.screen) else {
        model.status = StatusLine::info("no rule search in force: f searches");
        return Vec::new();
    };
    match model.saved.load(&model.profile.name) {
        Ok(f) if f.of(screen).contains_key(name) => {
            model.overlay = Overlay::Confirm {
                reason: format!("{screen} filter {name} exists. Replace it? (y/n)"),
                action: PendingAction::SaveFilter(name.to_string(), current),
            };
        }
        Ok(_) => write_filter(model, name, &current),
        Err(e) => model.status = StatusLine::error(e),
    }
    Vec::new()
}

/// Writes `saved` as `name` (replacing one), which is then the screen's
/// applied filter.
pub(super) fn write_filter(model: &mut Model, name: &str, saved: &Saved) {
    let written = model
        .saved
        .store(&model.profile.name)
        .and_then(|s| s.save(name, saved, true).map_err(|e| e.to_string()));
    model.status = match written {
        Ok(_) => {
            saved::mark_applied(model, model.screen, name);
            StatusLine::info(format!("saved {} filter {name}", saved.screen()))
        }
        Err(e) => StatusLine::error(e),
    };
}

/// `:filters`: the picker of the screen's saved filters.
pub(super) fn open_filters(model: &mut Model) {
    let loaded = screen_of(model).and_then(|s| Ok((s, model.saved.load(&model.profile.name)?)));
    let profile = model.profile.name.clone();
    match loaded {
        Ok((_, f)) if let Some(e) = saved_shape(&f, &model.screen) => {
            model.status = StatusLine::error(format!("profile {profile}: {e}"));
        }
        Ok((screen, f)) if f.of(screen).is_empty() => {
            model.status = StatusLine::info(format!(
                "no saved {screen} filters · :save NAME saves the one in force"
            ));
        }
        Ok((screen, f)) => model.overlay = Overlay::FilterPicker(FilterPicker::new(screen, &f)),
        Err(e) => model.status = StatusLine::error(e),
    }
}

/// Why the screen's saved filters cannot be read (a wrong-shaped key).
fn saved_shape(f: &crate::filters::SavedFilters, screen: &Screen) -> Option<String> {
    saved::filter_screen(*screen).and_then(|s| f.shape_error(s))
}

/// Whether the screen has a saved filter `name` (`:filter NAME` loads it).
pub(super) fn has_filter(model: &Model, name: &str) -> bool {
    match (screen_of(model), model.saved.load(&model.profile.name)) {
        (Ok(screen), Ok(f)) => f.of(screen).contains_key(name),
        _ => false,
    }
}

/// The picker's keys: `Enter` applies, `D` asks to delete.
pub(super) fn picker_key(model: &mut Model, p: FilterPicker, k: KeyEvent) -> Vec<Cmd> {
    match p.key(k) {
        PickerOutcome::Open(p) => model.overlay = Overlay::FilterPicker(p),
        PickerOutcome::Close => model.overlay = Overlay::None,
        PickerOutcome::Apply(name) => {
            model.overlay = Overlay::None;
            return load_filter(model, &name);
        }
        PickerOutcome::Delete(p, name) => {
            model.overlay = Overlay::Confirm {
                reason: format!("Delete {} filter {name}? (y/n)", p.screen),
                action: PendingAction::DeleteFilter(p, name),
            };
        }
    }
    Vec::new()
}

/// `y` on a delete: the filter leaves the file; the picker comes back
/// while filters are left.
pub(super) fn delete_filter(model: &mut Model, p: FilterPicker, name: &str) {
    let removed = model
        .saved
        .store(&model.profile.name)
        .and_then(|s| s.remove(p.screen, name).map_err(|e| e.to_string()));
    if let Err(e) = removed {
        model.status = StatusLine::error(e);
        model.overlay = Overlay::FilterPicker(p);
        return;
    }
    let a = &mut model.saved.applied;
    let unname = |n: Option<&String>| n.is_some_and(|n| n == name);
    match p.screen {
        crate::filters::Screen::Traffic if unname(a.traffic.as_ref().map(|t| &t.0)) => {
            a.traffic = None
        }
        crate::filters::Screen::Policy if unname(a.policy.as_ref().map(|t| &t.0)) => {
            a.policy = None
        }
        crate::filters::Screen::Events if unname(a.events.as_ref().map(|t| &t.0)) => {
            a.events = None
        }
        crate::filters::Screen::Workloads if unname(a.workloads.as_ref().map(|t| &t.0)) => {
            a.workloads = None
        }
        _ => {}
    }
    model.status = StatusLine::info(format!("deleted {} filter {name}", p.screen));
    model.overlay = match model.saved.load(&model.profile.name) {
        Ok(f) if !f.of(p.screen).is_empty() => {
            Overlay::FilterPicker(FilterPicker::new(p.screen, &f).with_input(&p.input))
        }
        _ => Overlay::None,
    };
}

/// `:load NAME`: the saved filter, applied as its screen's `f` would
/// apply it. A filter that does not parse is said, with its profile.
pub(super) fn load_filter(model: &mut Model, name: &str) -> Vec<Cmd> {
    let got = screen_of(model).and_then(|screen| {
        model
            .saved
            .store(&model.profile.name)?
            .get(screen, name)
            .map_err(|e| e.to_string())
    });
    let saved = match got {
        Ok(s) => s,
        Err(e) => {
            model.status = StatusLine::error(e);
            return Vec::new();
        }
    };
    let what = format!(
        "profile {}, {} filter {name}",
        model.profile.name,
        saved.screen()
    );
    match apply(model, name, saved) {
        Ok(cmds) => cmds,
        Err(e) => {
            model.status = StatusLine::error(format!("{what}: {e}"));
            Vec::new()
        }
    }
}

fn apply(model: &mut Model, name: &str, saved: Saved) -> Result<Vec<Cmd>, String> {
    match saved {
        Saved::Traffic(s) => {
            let f = saved::traffic_filter(&s, &model.traffic.filter)?;
            model.saved.applied.traffic = Some((name.to_string(), f.clone()));
            Ok(request_query(model, f))
        }
        Saved::Policy(q) => {
            let f = pce::RuleFilter::parse(&q)?;
            apply_rule_search(model, f);
            saved::mark_applied(model, Screen::Policy, name);
            Ok(Vec::new())
        }
        Saved::Events(s) => {
            let edit = saved::events_edit(&s, &model.limits.event_type_filter());
            let cmds = apply_events_edit(model, edit)?;
            saved::mark_applied(model, Screen::Events, name);
            Ok(cmds)
        }
        Saved::Workloads(s) => {
            let (filters, labels) = saved::workload_filters(&s);
            pce::LabelFilter::parse(&labels).map_err(|e| format!("labels: {e}"))?;
            model.workloads.filters = filters;
            apply_workload_labels(model, &labels)?;
            saved::mark_applied(model, Screen::Workloads, name);
            let w = &model.workloads;
            if w.label_warnings.is_empty() {
                model.status = StatusLine::info(format!(
                    "workloads filter {name} · {} workloads",
                    w.table.len()
                ));
            }
            Ok(Vec::new())
        }
    }
}

/// `Tab` in the `:` prompt after `load ` or `filter `: the screen's saved
/// filter names starting with the typed one, to their common start.
pub(super) fn complete_prompt(model: &mut Model, input: &str) -> String {
    let Some((cmd, typed)) = input.split_once(' ') else {
        return input.to_string();
    };
    if !matches!(cmd, "load" | "filter" | "f") {
        return input.to_string();
    }
    let names = match (screen_of(model), model.saved.load(&model.profile.name)) {
        (Ok(screen), Ok(f)) => f.names(screen),
        _ => Vec::new(),
    };
    let typed = typed.trim_start();
    let hits: Vec<&String> = names.iter().filter(|n| n.starts_with(typed)).collect();
    if hits.len() > 1 {
        model.status = StatusLine::info(
            hits.iter()
                .map(|h| h.as_str())
                .collect::<Vec<_>>()
                .join("  "),
        );
    }
    match saved::complete_name(&names, typed) {
        Some(done) => format!("{cmd} {done}"),
        None => input.to_string(),
    }
}
