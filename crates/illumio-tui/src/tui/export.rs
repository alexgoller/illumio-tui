//! `:export <path>`: the table on screen as CSV, through the CLI's
//! `--csv` writer. The rows as shown (search and filters applied, in sort
//! order; folded sections export every peer), every column whatever the
//! width, each cell whole; sectioned views as `traffic groups --csv`. No
//! request: `update` turns the table into `Cmd::Export`, the runtime
//! writes the file with `write`.
use crate::cli::output::write_csv;
use crate::cli::traffic::{SECTION_CSV_HEADERS, peer_csv_row};
use crate::tui::model::{Model, ObjectKind, PolicyPane, Screen, SectionRow, TrafficView};
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
                (_, Some(rows)) => sections(&rows),
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

/// Sectioned rows one row per peer, as `traffic groups --by ... --csv`
/// prints them; the header rows are left out.
fn sections(rows: &[SectionRow]) -> Csv {
    let rows = rows
        .iter()
        .filter_map(|r| {
            let (direction, peer) = r.peer()?;
            Some(peer_csv_row(&r.app, direction, peer))
        })
        .collect();
    (SECTION_CSV_HEADERS.to_vec(), rows)
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
