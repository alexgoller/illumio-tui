//! The `:` command line's commands.
use super::*;

pub fn on_command(model: &mut Model, cmd: Command) -> Vec<Cmd> {
    // A command goes somewhere else: the enforcement overlay a step left
    // does not come back over it.
    model.enforce_back = None;
    match cmd {
        Command::Screen(s) => switch(model, s),
        Command::IpLists => {
            model.objects.kind = ObjectKind::IpLists;
            model.objects.show_detail = false;
            model.objects.ip_lists.show_detail = false;
            switch(model, Screen::Objects)
        }
        Command::Quit => quit(model),
        Command::Batch => {
            if model.batch_report.is_some() {
                model.overlay = Overlay::BatchReport { scroll: 0 };
            } else {
                model.status = StatusLine::info("no batch sent in this session");
            }
            Vec::new()
        }
        Command::Help => {
            open_help(model);
            Vec::new()
        }
        Command::Marks(action) => marks::open_marks(model, action.as_deref()),
        Command::Ven => {
            open_ven(model);
            Vec::new()
        }
        Command::Used => open_where_used(model),
        Command::Save(name) => saved::save_filter(model, &name),
        Command::Filters => {
            saved::open_filters(model);
            Vec::new()
        }
        Command::Load(name) => saved::load_filter(model, &name),
        Command::Default(name) => {
            saved::default_command(model, &name);
            Vec::new()
        }
        // A saved filter of the screen's by that name wins over the expression.
        Command::Filter(name) if saved::has_filter(model, &name) => {
            saved::load_filter(model, &name)
        }
        // On Workloads `:filter` sets the label filter (local, no request).
        Command::Filter(expr) if model.screen == Screen::Workloads => {
            if let Err(e) = apply_workload_labels(model, &expr) {
                model.status = StatusLine::error(format!("label filter: {e}"));
            }
            Vec::new()
        }
        Command::Filter(expr) => {
            let mut f = model.traffic.filter.clone();
            f.src = expr;
            match f.validate() {
                Ok(()) => request_query(model, f),
                Err(e) => {
                    model.status = StatusLine::error(e);
                    Vec::new()
                }
            }
        }
        Command::Unicast => {
            // Through the cost guard like any filter change: one query.
            let mut f = model.traffic.filter.clone();
            f.toggle_unicast();
            request_query(model, f)
        }
        Command::Window(w) => {
            let mut f = model.traffic.filter.clone();
            f.window = w;
            match f.validate() {
                Ok(()) => request_query(model, f),
                Err(e) => {
                    model.status = StatusLine::error(e);
                    Vec::new()
                }
            }
        }
        Command::Focus(Some(name)) => {
            focus_by_name(model, &name);
            Vec::new()
        }
        Command::Focus(None) => {
            if model.traffic.data.is_none() {
                model.status = StatusLine::info("no result to focus yet: r runs the query");
            } else {
                model.screen = Screen::Traffic;
                open_app_picker(model, AppPickerPurpose::Focus, "");
            }
            Vec::new()
        }
        Command::App(Some(name)) => open_app_by_name(model, &name),
        Command::App(None) => {
            open_app_picker(model, AppPickerPurpose::Overlay, "");
            Vec::new()
        }
        Command::Gen(Some(name)) => gen_by_name(model, &name),
        Command::Gen(None) => {
            open_app_picker(model, AppPickerPurpose::Generator, "");
            Vec::new()
        }
        Command::Readiness => enforce::open_readiness(model),
        Command::Enforce(Some(name)) => enforce::enforce_by_name(model, &name),
        Command::Enforce(None) => {
            enforce::enforce_picker(model);
            Vec::new()
        }
        Command::Profile(name) => {
            if model.batch_sending {
                model.status = StatusLine::warn("batch sending: wait for it to finish");
                return Vec::new();
            }
            if model.writes.in_flight {
                model.status = StatusLine::warn("finish or cancel the write first");
                return Vec::new();
            }
            if model.switching {
                model.status = StatusLine::info("profile switch in progress");
                return Vec::new();
            }
            model.switching = true;
            vec![Cmd::SwitchProfile(name)]
        }
        Command::Writes(None) => {
            model.status = StatusLine::info(writes_status(&model.writes));
            Vec::new()
        }
        Command::Writes(Some(on)) => {
            if model.batch_sending {
                model.status = StatusLine::warn("batch sending: wait for it to finish");
                return Vec::new();
            }
            if model.writes.in_flight {
                model.status = StatusLine::warn("finish or cancel the write first");
                return Vec::new();
            }
            if model.switching {
                model.status = StatusLine::info("profile switch in progress");
                return Vec::new();
            }
            vec![Cmd::SetWrites(on)]
        }
        Command::Export { path, force } => match crate::tui::export::current_table(model) {
            Ok((headers, rows)) => vec![Cmd::Export {
                path,
                force,
                headers,
                rows,
            }],
            Err(e) => {
                model.status = StatusLine::error(e);
                Vec::new()
            }
        },
        Command::Snapshot(label) => save_snapshot(model, label),
        Command::Draft => show_policy(model, PolicyScope::Draft),
        Command::Active => show_policy(model, PolicyScope::Active),
    }
}

/// `:snapshot`: the reads `snapshot save` makes, then the file under the
/// profile's snapshot directory; one at a time.
fn save_snapshot(model: &mut Model, label: Option<String>) -> Vec<Cmd> {
    if model.snapshot_saving {
        model.status = StatusLine::info("a snapshot is being saved: wait for it");
        return Vec::new();
    }
    let Some(root) = &model.snapshots_root else {
        model.status =
            StatusLine::error("no data directory for snapshots: set HOME or XDG_DATA_HOME");
        return Vec::new();
    };
    let profile = model.profile.name.clone();
    let dir = crate::snapshot::profile_dir(root, &profile);
    model.snapshot_saving = true;
    model.status = StatusLine::info("snapshot: reading policy, labels and workloads…");
    vec![Cmd::SaveSnapshot {
        dir,
        profile,
        label,
    }]
}
