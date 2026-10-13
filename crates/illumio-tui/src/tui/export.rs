//! `:export <path>`: the table on screen as CSV, through the CLI's
//! `--csv` writer. The rows as shown (search and filters applied, in sort
//! order, folded sections without their peers), every column whatever the
//! width, each cell whole. No request; the file is the only effect.
use crate::cli::output::write_csv;
use crate::tui::model::{Model, ObjectKind, PolicyPane, Screen, SectionRow, TrafficView};
use crate::tui::table::TableView;
use std::io::Write;
use std::path::PathBuf;

/// A table as CSV: header row, then records.
type Csv = (Vec<&'static str>, Vec<Vec<String>>);

/// The active table of the current screen, or why there is none.
pub fn current_table(model: &Model) -> Result<Csv, String> {
    Ok(match model.screen {
        Screen::Traffic => {
            let t = &model.traffic;
            match (t.view, t.section_table()) {
                (_, Some(s)) => sections(s),
                (TrafficView::Flows, _) => t.flows.export(),
                (TrafficView::Mesh, _) => t.mesh.export(),
                (TrafficView::Treemap, _) => {
                    return Err("the treemap has no table to export".into());
                }
                // App-to-app by pairs.
                (_, None) => t.groups.export(),
            }
        }
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

/// A sectioned table one row per peer, like `traffic groups --csv`: the
/// app and the direction lead each row; the header rows are left out.
fn sections(t: &TableView<SectionRow>) -> Csv {
    let mut headers = vec!["app", "direction"];
    headers.extend(t.columns.iter().map(|c| c.title));
    let rows = t
        .visible()
        .iter()
        .map(|&i| &t.rows()[i])
        .filter_map(|r| {
            let (direction, _) = r.peer()?;
            let mut row = vec![r.app.join("/"), direction.as_str().to_string()];
            row.extend(t.full_cells(r));
            Some(row)
        })
        .collect();
    (headers, rows)
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
pub fn write(path: &str, force: bool, csv: &Csv) -> Result<String, String> {
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
    write_csv(&mut out, &csv.0, &csv.1)
        .and_then(|()| out.flush())
        .map_err(|e| format!("cannot write {shown}: {e}"))?;
    let n = csv.1.len();
    Ok(format!(
        "wrote {n} row{} to {shown}",
        if n == 1 { "" } else { "s" }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_leading_tilde_is_the_home_directory() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(expand("~/out/a.csv"), home.join("out/a.csv"));
        assert_eq!(expand("a~/b.csv"), PathBuf::from("a~/b.csv"));
        assert_eq!(expand("/tmp/x.csv"), PathBuf::from("/tmp/x.csv"));
    }
}
