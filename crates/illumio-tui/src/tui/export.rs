//! `:export <path>`: the table on screen as CSV, through the CLI's
//! `--csv` writer. The rows as shown (search and filters applied, in sort
//! order; folded sections export every peer), every column whatever the
//! width, each cell whole; sectioned views as `traffic groups --csv`, the
//! Flows view by app as its flows with a `section` column. No
//! request: `update` turns the table into `Cmd::Export`, the runtime
//! writes the file with `write`.
use crate::cli::output::TableRows;
use crate::cli::output::write_csv;
use crate::cli::traffic::{ServiceGroups, section_csv_headers, section_csv_row};
use crate::tui::model::{Model, ObjectKind, PolicyPane, Screen, SectionRow, TrafficView};
use crate::tui::table::TableView;
use pce::traffic::FlowGroup;
use std::io::Write;
use std::path::PathBuf;

/// A table as CSV: header row, then records.
type Csv = (Vec<&'static str>, Vec<Vec<String>>);

/// The active table of the current screen, or why there is none.
pub fn current_table(model: &Model) -> Result<Csv, String> {
    Ok(match model.screen {
        Screen::Traffic => {
            let t = &model.traffic;
            match (t.view, t.unfolded_section_rows()) {
                (TrafficView::Flows, Some(rows)) => {
                    without_transition(flow_sections(&t.flows_grouped, &rows))
                }
                (_, Some(rows)) => sections(&rows),
                (TrafficView::Flows, _) => without_transition(t.flows.export()),
                (TrafficView::Mesh, _) => without_transition(t.mesh.export()),
                (TrafficView::Exposure, _) => t.exposure_rows.export(),
                // The tree as shown: sources, and the unfolded destinations.
                (TrafficView::Volume, _) => t.volume.export(),
                // App-to-app by pairs as `traffic groups --csv`; by service (`o`) as
                // `traffic groups --services --csv`.
                (_, None) if t.apps_state.by_service => service_pairs(&t.services),
                (_, None) => pairs(&t.groups),
            }
        }
        Screen::Policy if model.policy.search.is_some() => model.policy.found.export(),
        Screen::Policy => match model.policy.pane {
            PolicyPane::RuleSets => model.policy.rulesets.export(),
            PolicyPane::Rules => model.policy.rules.export(),
        },
        Screen::Objects => match model.objects.kind {
            ObjectKind::Labels => model.objects.labels.export(),
            ObjectKind::LabelGroups => model.objects.groups.export(),
            ObjectKind::Services => model.objects.services.export(),
            ObjectKind::IpLists => model.objects.ip_lists.table.export(),
        },
        Screen::Workloads => model.workloads.table.export(),
        Screen::Events => model.events.table.export(),
    })
}

/// `csv` without compare mode's `was→now` column: the `change` column
/// has the transition with the whole decision names.
fn without_transition((mut titles, mut rows): Csv) -> Csv {
    if let Some(at) = titles
        .iter()
        .position(|t| *t == crate::tui::model::TRANSITION_COLUMN)
    {
        titles.remove(at);
        for r in &mut rows {
            if at < r.len() {
                r.remove(at);
            }
        }
    }
    (titles, rows)
}

/// Sectioned rows one row per peer, as `traffic groups --by ... --csv`
/// prints them; the header rows are left out.
fn sections(rows: &[SectionRow]) -> Csv {
    let draft = rows
        .iter()
        .any(|r| r.peer().is_some_and(|(_, p)| p.draft.is_some()));
    let rows = rows
        .iter()
        .filter_map(|r| {
            let (direction, peer) = r.peer()?;
            Some(section_csv_row(&r.app, direction, peer, draft))
        })
        .collect();
    (section_csv_headers(draft), rows)
}

/// The Flows view by app (`x`): one row per flow, as the flat table
/// exports it, after a `section` column naming the flow's app; the header
/// rows are left out.
fn flow_sections(table: &TableView<SectionRow>, rows: &[SectionRow]) -> Csv {
    let mut titles = vec!["section"];
    titles.extend(
        table
            .columns
            .iter()
            .map(|c| c.full_title.unwrap_or(c.title)),
    );
    let rows = rows
        .iter()
        .filter(|r| r.flow().is_some())
        .map(|r| {
            let mut cells = vec![r.app.join("/")];
            cells.extend(table.full_cells(r));
            cells
        })
        .collect();
    (titles, rows)
}

/// The rows of `table` as shown (search and sort applied).
fn shown(table: &TableView<FlowGroup>) -> Vec<FlowGroup> {
    table
        .visible()
        .iter()
        .map(|&i| table.rows()[i].clone())
        .collect()
}

/// The pairs as shown, in the columns of `traffic groups --csv`: bytes
/// whole and the connections per decision, whatever the screen shows.
fn pairs(table: &TableView<FlowGroup>) -> Csv {
    let shown = shown(table);
    (shown.full_headers(), shown.full_rows())
}

/// The pairs by service as shown, in the columns of `traffic groups
/// --services --csv`: every decision count, where the screen shows the
/// dominant one.
fn service_pairs(table: &TableView<FlowGroup>) -> Csv {
    let shown = ServiceGroups(shown(table));
    (shown.full_headers(), shown.full_rows())
}

/// `~/x` under the home directory; anything else as typed.
fn expand(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(path),
    }
}

/// Writes `csv` to `path`; an existing file only when `force`. The status
/// line text either way.
pub fn write(
    path: &str,
    force: bool,
    headers: &[&str],
    rows: &[Vec<String>],
) -> Result<String, String> {
    let target = expand(path);
    let shown = target.display();
    let mut open = std::fs::OpenOptions::new();
    open.write(true);
    if force {
        open.create(true).truncate(true);
    } else {
        // Atomic: no window between a check and the create.
        open.create_new(true);
    }
    let file = open.open(&target).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            format!("{shown} exists; :export! {path} overwrites it")
        } else {
            format!("cannot write {shown}: {e}")
        }
    })?;
    let mut out = std::io::BufWriter::new(file);
    write_csv(&mut out, headers, rows)
        .and_then(|()| out.flush())
        .map_err(|e| format!("cannot write {shown}: {e}"))?;
    let n = rows.len();
    Ok(format!(
        "wrote {n} row{} to {shown}",
        if n == 1 { "" } else { "s" }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> (Vec<&'static str>, Vec<Vec<String>>) {
        (vec!["a", "b"], vec![vec!["1".into(), "x,y".into()]])
    }

    #[test]
    fn write_creates_refuses_an_existing_file_and_overwrites_with_force() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.csv");
        let p = path.display().to_string();
        let (h, r) = rows();
        assert_eq!(
            write(&p, false, &h, &r).unwrap(),
            format!("wrote 1 row to {p}")
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "a,b\r\n1,\"x,y\"\r\n"
        );
        let err = write(&p, false, &h, &[]).unwrap_err();
        assert_eq!(err, format!("{p} exists; :export! {p} overwrites it"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "a,b\r\n1,\"x,y\"\r\n"
        );
        assert_eq!(
            write(&p, true, &h, &[]).unwrap(),
            format!("wrote 0 rows to {p}")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a,b\r\n");
    }

    #[test]
    fn write_reports_an_unwritable_path() {
        let (h, r) = rows();
        let err = write("/nonexistent-dir/x.csv", true, &h, &r).unwrap_err();
        assert!(
            err.starts_with("cannot write /nonexistent-dir/x.csv"),
            "{err}"
        );
    }

    #[test]
    fn a_leading_tilde_is_the_home_directory() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(expand("~/out/a.csv"), home.join("out/a.csv"));
        assert_eq!(expand("a~/b.csv"), PathBuf::from("a~/b.csv"));
        assert_eq!(expand("/tmp/x.csv"), PathBuf::from("/tmp/x.csv"));
    }
}
