use super::*;
use crate::tui::model::StatusKind;
use crate::tui::test_support::model_fixture;

fn snapshot(m: &mut Model, label: Option<&str>) -> Vec<Cmd> {
    update(
        m,
        Msg::Command(Command::Snapshot(label.map(str::to_string))),
    )
}

#[test]
fn snapshot_asks_the_runtime_once_for_the_profiles_directory() {
    let mut m = model_fixture();
    m.snapshots_root = Some("/data/illumio-tui/snapshots".into());
    let cmds = snapshot(&mut m, Some("before Q4"));
    match cmds.as_slice() {
        [
            Cmd::SaveSnapshot {
                dir,
                profile,
                label,
            },
        ] => {
            assert_eq!(
                dir,
                &std::path::PathBuf::from("/data/illumio-tui/snapshots")
                    .join(crate::names::dir_name(&m.profile.name))
            );
            assert_eq!(profile, &m.profile.name);
            assert_eq!(label.as_deref(), Some("before Q4"));
        }
        other => panic!("{other:?}"),
    }
    assert!(m.snapshot_saving);
    assert!(m.status.text.contains("reading"), "{}", m.status.text);
    // A second one waits for the first: no second set of reads.
    assert!(snapshot(&mut m, None).is_empty());
    assert!(m.status.text.contains("being saved"), "{}", m.status.text);
    // The answer is shown and frees the next one.
    update(
        &mut m,
        Msg::SnapshotSaved(Ok("snapshot saved: /data/x.json (3K)".into())),
    );
    assert!(!m.snapshot_saving);
    assert_eq!(m.status.text, "snapshot saved: /data/x.json (3K)");
    update(&mut m, Msg::SnapshotSaved(Err("disk full".into())));
    assert_eq!(m.status.kind, StatusKind::Error);
    assert_eq!(m.status.text, "snapshot: disk full");
    assert_eq!(snapshot(&mut m, None).len(), 1);
}

#[test]
fn snapshot_without_a_data_directory_sends_nothing() {
    let mut m = model_fixture();
    m.snapshots_root = None;
    assert!(snapshot(&mut m, None).is_empty());
    assert_eq!(m.status.kind, StatusKind::Error);
    assert!(m.status.text.contains("XDG_DATA_HOME"));
    assert!(!m.snapshot_saving);
}
