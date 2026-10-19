//! The app overlay's transitions (`A` / `:app`): opening it, its keys,
//! its reload, `Enter` and `w` on its rows, and its traffic and policy
//! following what lands.
use super::*;

/// The direction a new app overlay starts at: the focus's, else `both`.
pub(super) fn app_direction(model: &Model) -> FocusDirection {
    model
        .traffic
        .focus
        .as_ref()
        .map_or(FocusDirection::Both, |f| f.direction)
}

/// `A`: the app overlay for the row's app (the first of `row_apps`, the
/// order `F` takes), at the focus direction (or `both`); on a row without
/// an app, the picker.
pub(super) fn app_open_key(model: &mut Model) -> Vec<Cmd> {
    let Some(app) = model.traffic.row_apps().into_iter().next() else {
        open_app_picker(model, AppPickerPurpose::Overlay, "");
        return Vec::new();
    };
    let focus = Focus::new(app, model.traffic.keys.clone()).with_direction(app_direction(model));
    open_app(model, focus)
}

/// `:app <app>` and the picker: an app of the loaded result, or one whose
/// labels the name index knows (it may have no flows in the window); else
/// the closest match in the status line, nothing sent.
pub(super) fn open_app_by_name(model: &mut Model, name: &str) -> Vec<Cmd> {
    let keys = model.traffic.keys.clone();
    let apps = model.traffic.loaded_apps();
    let app = pce::traffic::parse_app(name);
    let labelled = model
        .index()
        .map(|idx| pce::traffic::app_labels(&app, &keys, idx));
    if !apps.iter().any(|a| a == name) && !matches!(labelled, Some(Ok(_))) {
        let near = pce::traffic::closest(name, &apps, 3);
        model.status = StatusLine::warn(match (near.is_empty(), labelled) {
            (false, _) => format!(
                "unknown app {name:?}; closest: {}; nothing sent",
                near.join(", ")
            ),
            (true, Some(Err(e))) => format!("{e}; nothing sent"),
            (true, _) => {
                format!("unknown app {name:?} in the loaded result or the labels; nothing sent")
            }
        });
        return Vec::new();
    }
    let focus = Focus::new(app, keys).with_direction(app_direction(model));
    open_app(model, focus)
}

/// `:gen <app>` and the picker: as `:app` finds an app (the loaded result
/// or the labels), then the generator for it.
pub(super) fn gen_by_name(model: &mut Model, name: &str) -> Vec<Cmd> {
    let keys = model.traffic.keys.clone();
    let app = pce::traffic::parse_app(name);
    let loaded = model.traffic.loaded_apps();
    let labelled = model
        .index()
        .map(|idx| pce::traffic::app_labels(&app, &keys, idx));
    if !loaded.iter().any(|a| a == name) && !matches!(labelled, Some(Ok(_))) {
        let near = pce::traffic::closest(name, &loaded, 3);
        model.status = StatusLine::warn(match (near.is_empty(), labelled) {
            (false, _) => format!(
                "unknown app {name:?}; closest: {}; nothing sent",
                near.join(", ")
            ),
            (true, Some(Err(e))) => format!("{e}; nothing sent"),
            (true, _) => format!("unknown app {name:?}; nothing sent"),
        });
        return Vec::new();
    }
    generator::open_gen(model, app, false)
}

/// Opens the overlay for `focus`. The traffic panels come from the loaded
/// result (no request). The workloads come from the Workloads screen when
/// it holds every workload; the rulesets from the Policy screen when it is
/// loaded (none is requested while its load is on the way). Reopening the
/// same app keeps its view (`seq`, panels, cursors): nothing is sent again.
/// Otherwise one request each, counted. A new app gets a new `seq`, so an
/// earlier overlay's late answer is dropped.
pub(super) fn open_app(model: &mut Model, focus: Focus) -> Vec<Cmd> {
    model.traffic.sync_marks();
    let window = model.traffic.filter.window.clone();
    let (mut v, reused) = match model.app_view.take() {
        Some(mut v) if v.focus.same_app(&focus) => {
            v.focus = focus.clone();
            v.window = window;
            (v, true)
        }
        _ => {
            model.app_seq += 1;
            (AppView::new(focus.clone(), window, model.app_seq), false)
        }
    };
    let t = &model.traffic;
    let a = t.active_marks();
    v.rebuild_traffic(
        t.flows_for(&focus),
        t.partial_for(&focus),
        a.as_ref().map(|a| (a, t.show_squelched)),
    );
    let idx = model.index().cloned();
    v.label_hrefs = idx
        .as_ref()
        .and_then(|idx| focus.label_matches(idx).ok())
        .map(|m| pce::app::label_hrefs(&m));
    let why = if idx.is_none() {
        "the name index is not loaded yet".to_string()
    } else {
        format!(
            "{} is not a label group: no workloads or rulesets to match",
            focus.app_text()
        )
    };
    let mut cmds = Vec::new();
    match (v.label_hrefs.clone(), &idx) {
        (Some(hrefs), Some(idx))
            if model.workloads.loaded_at.is_some() && !model.workloads.truncated =>
        {
            let mine: Vec<Workload> = model
                .workloads
                .raw
                .iter()
                .filter(|w| pce::app::workload_in_app(w, &hrefs))
                .cloned()
                .collect();
            let now = model.workloads.now.unwrap_or_else(jiff::Timestamp::now);
            v.workloads = Loadable::Loaded(pce::app::app_workloads(
                &mine,
                idx,
                &model.limits.tier_key,
                now,
            ));
            v.workloads_truncated = false;
        }
        (Some(_), Some(_)) if reused && !matches!(v.workloads, Loadable::Unavailable(_)) => {}
        (Some(hrefs), Some(_)) => {
            v.workloads = Loadable::Loading;
            cmds.push(Cmd::LoadAppWorkloads {
                seq: v.seq,
                filter: pce::app::app_workload_filter(&hrefs),
                mode: CacheMode::Use,
            });
        }
        _ => v.workloads = Loadable::Unavailable(why.clone()),
    }
    // A panel of the other scope (`:draft` / `:active` since) is stale.
    let same_scope = v.policy_scope == model.policy.scope;
    v.policy_scope = model.policy.scope;
    match (&v.label_hrefs, &idx) {
        (Some(hrefs), Some(idx)) if model.policy.loaded_at.is_some() => {
            v.policy = Loadable::Loaded(pce::app::app_policy(&model.policy.raw, hrefs, idx));
        }
        (Some(_), Some(_))
            if reused && same_scope && !matches!(v.policy, Loadable::Unavailable(_)) => {}
        (Some(_), Some(_)) if model.policy.loading => {
            v.policy = Loadable::Loading;
            v.policy_sent = false;
        }
        (Some(_), Some(_)) => {
            v.policy = Loadable::Loading;
            v.policy_sent = true;
            cmds.extend(load_rule_sets(model, CacheMode::Use));
        }
        _ => v.policy = Loadable::Unavailable(why),
    }
    let sent = u32::try_from(cmds.len()).unwrap_or(u32::MAX);
    v.requests += sent;
    // `req` as in the header: the hints beside it leave about 20 columns.
    model.status = StatusLine::info(format!("{} · {sent} req", focus.app_text()));
    model.app_view = Some(v);
    model.overlay = Overlay::App;
    cmds
}

/// The overlay's peers again from the loaded flows: after `X`, and when a
/// traffic answer lands while it is open. No request.
pub(super) fn rebuild_app_traffic(model: &mut Model) {
    model.traffic.sync_marks();
    let t = &model.traffic;
    if let Some(v) = model.app_view.as_mut() {
        let focus = v.focus.clone();
        let a = t.active_marks();
        v.rebuild_traffic(
            t.flows_for(&focus),
            t.partial_for(&focus),
            a.as_ref().map(|a| (a, t.show_squelched)),
        );
    }
}

/// The overlay's keys: `Esc` / `q` close, `Tab` / `BackTab` panels, `j` /
/// `k` rows, `X` direction, `Enter` opens, `w` drafts a rule, `r` asks to
/// reload, `?` help. Only `r` (after `y`) and `Enter` (on another screen) can send.
pub(super) fn app_overlay_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let Some(v) = model.app_view.as_mut() else {
        model.overlay = Overlay::None;
        return Vec::new();
    };
    match k.code {
        KeyCode::Esc | KeyCode::Char('q') => model.overlay = Overlay::None,
        KeyCode::Tab => v.panel = v.panel.next(),
        KeyCode::BackTab => v.panel = v.panel.prev(),
        KeyCode::Down | KeyCode::Char('j') => v.move_cursor(1),
        KeyCode::Up | KeyCode::Char('k') => v.move_cursor(-1),
        KeyCode::Char('X') => {
            v.focus.direction = v.focus.direction.next();
            rebuild_app_traffic(model);
        }
        // Squelched peers shown or left out here and on the Traffic screen.
        KeyCode::Char('M') => {
            model.status = model.traffic.toggle_squelched();
            rebuild_app_traffic(model);
        }
        KeyCode::Enter => return app_enter(model),
        KeyCode::Char('m') => return marks::mark_app_key(model),
        KeyCode::Char('w') => return app_rule_key(model),
        KeyCode::Char('E') => return explain_app_key(model),
        KeyCode::Char('e') => return enforce::app_enforce_key(model),
        KeyCode::Char('r') => app_reload_key(model),
        KeyCode::Char('G') => {
            let app = v.focus.app.clone();
            return generator::open_gen(model, app, true);
        }
        KeyCode::Char('?') => open_help(model),
        _ => {}
    }
    Vec::new()
}

/// `r` in the overlay: asks before reloading its two panels.
pub(super) fn app_reload_key(model: &mut Model) {
    let Some(v) = model.app_view.as_ref() else {
        return;
    };
    let app = v.focus.app_text();
    if v.label_hrefs.is_none() {
        model.status = StatusLine::info(format!("nothing to reload: {app} is not a label group"));
        return;
    }
    // The rulesets are not asked again while a load of them is on its way.
    let reason = if model.policy.loading {
        format!("Reload the workloads of {app} (the rulesets are on their way): 1 request? (y/n)")
    } else {
        format!("Reload the workloads and the rulesets of {app}: 2 requests? (y/n)")
    };
    model.overlay = Overlay::Confirm {
        reason,
        action: PendingAction::ReloadApp,
    };
}

/// `y` on the reload prompt: both panels past the cache, counted (the
/// rulesets not while a load of them is on the way); back to the overlay.
pub(super) fn app_reload(model: &mut Model) -> Vec<Cmd> {
    model.overlay = Overlay::App;
    let policy_loading = model.policy.loading;
    let Some(v) = model.app_view.as_mut() else {
        model.overlay = Overlay::None;
        return Vec::new();
    };
    let Some(hrefs) = v.label_hrefs.clone() else {
        return Vec::new();
    };
    v.workloads = Loadable::Loading;
    v.policy = Loadable::Loading;
    v.policy_scope = model.policy.scope;
    v.cached = 0;
    let mut cmds = vec![Cmd::LoadAppWorkloads {
        seq: v.seq,
        filter: pce::app::app_workload_filter(&hrefs),
        mode: CacheMode::Bypass,
    }];
    v.policy_sent = !policy_loading;
    if !policy_loading {
        cmds.extend(load_rule_sets(model, CacheMode::Bypass));
    }
    if let Some(v) = model.app_view.as_mut() {
        v.requests += u32::try_from(cmds.len()).unwrap_or(u32::MAX);
    }
    cmds
}

/// `Enter`: a peer closes the overlay and focuses that pair in Flows; a
/// workload opens its detail on the Workloads screen; a ruleset opens it on
/// the Policy screen (those screens load what they lack, as always).
pub(super) fn app_enter(model: &mut Model) -> Vec<Cmd> {
    let Some(v) = model.app_view.as_ref() else {
        return Vec::new();
    };
    match v.panel {
        AppPanel::Inbound | AppPanel::Outbound => {
            let Some((side, p)) = v.selected_peer() else {
                return Vec::new();
            };
            let (src, dst) = match side {
                Direction::In => (p.peer.clone(), v.focus.app.clone()),
                Direction::Out => (v.focus.app.clone(), p.peer.clone()),
            };
            let service = p.service.clone();
            let focus = v.focus.clone();
            model.overlay = Overlay::None;
            model.screen = Screen::Traffic;
            let t = &mut model.traffic;
            t.drill = Some(Drill::Service(src, dst, service, Decisions::All));
            t.view = TrafficView::Flows;
            t.show_detail = false;
            t.set_focus(focus); // rebuilds every view, the drill included
            model.status = focus_status(&model.traffic);
            Vec::new()
        }
        AppPanel::Workloads => {
            let Some(href) = v.selected_workload().map(|w| w.href.clone()) else {
                return Vec::new();
            };
            model.overlay = Overlay::None;
            let cmds = switch(model, Screen::Workloads);
            let w = &mut model.workloads;
            w.show_detail = w.select_href(&href);
            w.open_on_load = None;
            if !w.show_detail {
                // Not in the list yet: opened when the list arrives.
                w.open_on_load = Some(href);
                model.status = StatusLine::info("loading the workloads: its detail opens then");
            }
            cmds
        }
        AppPanel::Policy => {
            let Some(href) = v.selected_policy().map(|r| r.href.clone()) else {
                return Vec::new();
            };
            model.overlay = Overlay::None;
            // Select first, so the deny-rules fallback `switch` fires is
            // for this ruleset, not the row selected before.
            model.policy.pane = PolicyPane::RuleSets;
            model.policy.rulesets.select_where(|r| r.href == href);
            let mut cmds = switch(model, Screen::Policy);
            if let Some(idx) = model.index().cloned() {
                model
                    .policy
                    .rebuild_rules(&idx, model.workloads.impact_source());
                cmds.extend(deny_fallback(model, CacheMode::Use));
            }
            cmds
        }
    }
}

/// `w` on a blocked (or potentially blocked) peer row: the draft allow rule
/// from one of the flows behind it, as `w` on a traffic row (writes armed).
/// The form replaces the overlay; cancelling it, or the write done, comes
/// back to the overlay with nothing sent.
pub(super) fn app_rule_key(model: &mut Model) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    let Some(v) = model.app_view.as_ref() else {
        return Vec::new();
    };
    let Some((side, p)) = v.selected_peer() else {
        model.status = StatusLine::warn("select a peer row (Inbound or Outbound)");
        return Vec::new();
    };
    if !matches!(
        p.decision,
        PolicyDecision::Blocked | PolicyDecision::PotentiallyBlocked
    ) {
        model.status = StatusLine::warn("this row is allowed; nothing to add");
        return Vec::new();
    }
    let focus = &v.focus;
    let flow = model
        .traffic
        .flows_for(focus)
        .iter()
        .find(|f| {
            let (app_ep, peer_ep) = match side {
                Direction::In => (&f.dst, &f.src),
                Direction::Out => (&f.src, &f.dst),
            };
            f.policy_decision == p.decision
                && f.service_text() == p.service
                && pce::traffic::group_key(app_ep, &focus.keys) == focus.app
                && pce::traffic::group_key(peer_ep, &focus.keys) == p.peer
        })
        .cloned();
    let Some(flow) = flow else {
        model.status = StatusLine::warn("no loaded flow behind this row");
        return Vec::new();
    };
    let cmds = begin_edit(model, EditIntent::RuleFromFlow(Box::new(flow)));
    model.form_from_app = matches!(model.overlay, Overlay::Form(_));
    cmds
}

/// The overlay's policy panel from the rulesets that just arrived (`hit`:
/// from the cache), or their error, while it waits for them. Fills the
/// view only: never reopens a closed overlay.
pub(super) fn refresh_app_policy(
    model: &mut Model,
    scope: PolicyScope,
    outcome: Result<bool, &str>,
) {
    let idx = model.index().cloned();
    let Some(v) = model.app_view.as_mut() else {
        return;
    };
    if !matches!(v.policy, Loadable::Loading) || v.policy_scope != scope {
        return;
    }
    match (outcome, idx, v.label_hrefs.as_ref()) {
        (Ok(hit), Some(idx), Some(hrefs)) => {
            if v.policy_sent && hit {
                v.cached += 1;
            }
            v.policy = Loadable::Loaded(pce::app::app_policy(&model.policy.raw, hrefs, &idx));
        }
        (Err(e), _, _) => v.policy = Loadable::Failed(e.to_string()),
        _ => v.policy = Loadable::Unavailable("the name index is not loaded".into()),
    }
}
