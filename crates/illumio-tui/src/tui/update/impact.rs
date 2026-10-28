//! `I` in the pending overlay and `:impact` (#86): the provision impact
//! overlay. Opening it loads the summary once (`Cmd::LoadImpact`, through
//! the cache); reopening shows the loaded one with its age and sends
//! nothing; `r` in the overlay loads it again past the cache. No timer.
use super::*;
use crate::provision_impact::{LoadSpec, impact_query};
use crate::tui::provision_view::ImpactView;

/// Opens the overlay; loads the summary unless one is loaded or on its
/// way. `from_pending`: `Esc` goes back to the pending list.
pub(super) fn open_provision_impact(model: &mut Model, from_pending: bool) -> Vec<Cmd> {
    let v = &mut model.policy.impact;
    v.from_pending = from_pending;
    v.scroll = 0;
    model.overlay = Overlay::Impact;
    if v.report.is_some() || v.loading {
        model.status = impact_status(&model.policy.impact);
        return Vec::new();
    }
    load_impact(model, CacheMode::Use)
}

/// One load of the summary over the profile's default window and cap,
/// unless one is on its way; a window that needs confirming (> 7d) is
/// refused before any request.
fn load_impact(model: &mut Model, mode: CacheMode) -> Vec<Cmd> {
    if model.policy.impact.loading {
        return Vec::new();
    }
    if !model.is_connected() {
        model.status = StatusLine::warn("impact · not connected");
        return Vec::new();
    }
    let l = &model.limits;
    let query = match impact_query(&l.traffic_default_window, l.traffic_max_results) {
        Ok(q) => q,
        Err(e) => {
            model.policy.impact.error = Some(e.clone());
            model.status = StatusLine::warn(format!("impact · {e}"));
            return Vec::new();
        }
    };
    let spec = LoadSpec {
        query,
        window: l.traffic_default_window.clone(),
        keys: l.scope_keys.clone(),
        mode,
        requested: None,
    };
    let v = &mut model.policy.impact;
    v.loading = true;
    v.error = None;
    model.status = StatusLine::info(
        "impact · loading the pending list, the rulesets and one draft traffic query…",
    );
    vec![Cmd::LoadImpact(Box::new(spec))]
}

/// What the status line says while the overlay is open.
fn impact_status(v: &ImpactView) -> StatusLine {
    if let Some(e) = &v.error {
        return StatusLine::warn(format!("impact · {e}"));
    }
    if v.loading {
        return StatusLine::info("impact · loading…");
    }
    match &v.report {
        Some(r) if r.pending == 0 => StatusLine::info("impact · nothing pending"),
        Some(r) => {
            let t = r.traffic.as_ref().map(|t| &t.transitions);
            let (b, a) = t.map_or((0, 0), |t| (t.newly_blocked, t.newly_allowed));
            let s = format!(
                "impact · {} pending · {b} flow{} newly blocked · {a} newly allowed",
                r.pending,
                if b == 1 { "" } else { "s" }
            );
            if b > 0 {
                StatusLine::warn(s)
            } else {
                StatusLine::info(s)
            }
        }
        None => StatusLine::info("impact"),
    }
}

/// The summary's answer: kept with its time, or the error kept beside the
/// summary loaded before (if any).
pub(super) fn provision_impact_loaded(
    model: &mut Model,
    result: Result<crate::provision_impact::ProvisionImpact, String>,
) {
    let v = &mut model.policy.impact;
    v.loading = false;
    match result {
        Ok(r) => {
            v.report = Some(r);
            v.loaded_at = Some(Instant::now());
            v.error = None;
            v.scroll = 0;
        }
        Err(e) => v.error = Some(e),
    }
    if model.overlay == Overlay::Impact {
        model.status = impact_status(&model.policy.impact);
    }
}

/// The overlay's keys: scroll, `r` reload (past the cache), `Esc` back
/// (to the pending list when opened from it), `q` close, `?` help.
pub(super) fn provision_impact_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let last =
        u16::try_from(model.policy.impact.lines().len().saturating_sub(1)).unwrap_or(u16::MAX);
    let v = &mut model.policy.impact;
    match k.code {
        KeyCode::Esc => {
            model.overlay = if v.from_pending {
                Overlay::Pending
            } else {
                Overlay::None
            };
        }
        KeyCode::Char('q') => model.overlay = Overlay::None,
        KeyCode::Down | KeyCode::Char('j') => v.scroll = (v.scroll + 1).min(last),
        KeyCode::Up | KeyCode::Char('k') => v.scroll = v.scroll.saturating_sub(1),
        KeyCode::PageDown => v.scroll = v.scroll.saturating_add(10).min(last),
        KeyCode::PageUp => v.scroll = v.scroll.saturating_sub(10),
        KeyCode::Home | KeyCode::Char('g') => v.scroll = 0,
        KeyCode::End | KeyCode::Char('G') => v.scroll = last,
        KeyCode::Char('r') => return load_impact(model, CacheMode::Bypass),
        KeyCode::Char('?') => open_help(model),
        _ => {}
    }
    Vec::new()
}
