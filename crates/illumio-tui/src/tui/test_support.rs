//! Fixture-backed models and off-screen rendering for tests and snapshots.
//! Compiled always (it is tiny) so integration tests can use it.
use crate::tui::model::{Connection, Model, ProfileInfo};
use crate::tui::ui::view;
use pce::NameIndex;
use pce::models::{IpList, Label, LabelGroup, Service};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::sync::Arc;

pub fn fixture_index() -> Arc<NameIndex> {
    let labels: Vec<Label> = serde_json::from_str(include_str!("../../../../fixtures/labels.json"))
        .expect("labels fixture");
    let groups: Vec<LabelGroup> =
        serde_json::from_str(include_str!("../../../../fixtures/label_groups.json"))
            .expect("groups fixture");
    let lists: Vec<IpList> =
        serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json"))
            .expect("ip lists fixture");
    let services: Vec<Service> =
        serde_json::from_str(include_str!("../../../../fixtures/services.json"))
            .expect("services fixture");
    Arc::new(NameIndex::new(&labels, &groups, &lists, &services))
}

/// A connected model with the fixture index loaded and deterministic counters.
pub fn model_fixture() -> Model {
    let profile = ProfileInfo {
        name: "demo".into(),
        host: "pce.example".into(),
        org_id: 5,
    };
    let mut m = Model::new_connecting(profile, "dark".into(), 60, 1000, "24h".into());
    m.connection = Connection::Connected {
        index: Some(fixture_index()),
        index_age_secs: 0,
    };
    m.stats = pce::StatsSnapshot {
        calls: 5,
        bytes: 21_344,
    };
    m.status = crate::tui::model::StatusLine::info("ready");
    m
}

/// Renders the model into a `w`×`h` test terminal and returns the text.
pub fn render_to_string(model: &Model, w: u16, h: u16) -> String {
    let mut term = Terminal::new(TestBackend::new(w.max(1), h.max(1))).expect("test terminal");
    term.draw(|f| view(model, f)).expect("draw");
    let buf = term.backend().buffer();
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Text plus, per line, the distinct (fg, bg) runs: makes theme differences visible in snapshots.
pub fn render_styled(model: &Model, w: u16, h: u16) -> String {
    let mut term = Terminal::new(TestBackend::new(w.max(1), h.max(1))).expect("test terminal");
    term.draw(|f| view(model, f)).expect("draw");
    let buf = term.backend().buffer();
    let mut out = String::new();
    for y in 0..buf.area.height {
        let text: String = (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect();
        out.push_str(text.trim_end());
        out.push('\n');
        let mut runs: Vec<String> = Vec::new();
        let mut last: Option<(ratatui::style::Color, ratatui::style::Color)> = None;
        for x in 0..buf.area.width {
            let c = &buf[(x, y)];
            let key = (c.fg, c.bg);
            if last != Some(key) {
                runs.push(format!("@{x}:{:?}/{:?}", c.fg, c.bg));
                last = Some(key);
            }
        }
        out.push_str("    ");
        out.push_str(&runs.join(" "));
        out.push('\n');
    }
    out
}
