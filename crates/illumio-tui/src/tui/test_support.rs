//! Fixture-backed models and off-screen rendering for tests and snapshots.
//! Compiled always (it is tiny) so integration tests can use it.
use crate::tui::model::{Connection, Model, ProfileInfo, QueryState, TrafficData};
use crate::tui::ui::view;
use pce::NameIndex;
use pce::models::{Flow, IpList, Label, LabelGroup, Service};
use pce::traffic::{ResolvedFlow, resolve_flows};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::sync::Arc;
use std::time::{Duration, Instant};

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

pub fn fixture_flows() -> Vec<ResolvedFlow> {
    let raw: Vec<Flow> =
        serde_json::from_str(include_str!("../../../../fixtures/traffic_flows.json"))
            .expect("flows fixture");
    resolve_flows(&raw, &fixture_index())
}

/// A connected model with the fixture flows loaded as a finished query.
pub fn model_with_flows() -> Model {
    let mut m = model_fixture();
    let flows = fixture_flows();
    m.traffic.data = Some(TrafficData {
        flows,
        truncated: false,
        matches_count: Some(4),
        elapsed: Duration::from_millis(1840),
        cache_hit: false,
        fetched_at: Instant::now() - Duration::from_secs(3),
        filter: m.traffic.filter.clone(),
    });
    m.traffic.query = QueryState::Idle;
    m.traffic.auto.last_run = Some(Instant::now() - Duration::from_secs(3));
    m.traffic.rebuild_views();
    m
}

/// A connected model with the fixture rulesets loaded (draft scope, loaded 5 s ago).
pub fn model_with_policy() -> Model {
    let mut m = model_fixture();
    let raw: Vec<pce::models::RuleSet> =
        serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json"))
            .expect("rule sets fixture");
    let idx = fixture_index();
    m.policy.set_rule_sets(raw, &idx);
    // The first ruleset has inline deny rules; the second is left to the fallback.
    m.policy.loaded_at = Some(Instant::now() - Duration::from_secs(5));
    m
}

/// A connected model with the four fixture object lists loaded (ages of 3 s).
pub fn model_with_objects() -> Model {
    let mut m = model_fixture();
    let o = &mut m.objects;
    o.labels.set_rows(
        serde_json::from_str(include_str!("../../../../fixtures/labels.json"))
            .expect("labels fixture"),
    );
    o.groups.set_rows(
        serde_json::from_str(include_str!("../../../../fixtures/label_groups.json"))
            .expect("groups fixture"),
    );
    o.ip_lists.set_rows(
        serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json"))
            .expect("ip lists fixture"),
    );
    o.services.set_rows(
        serde_json::from_str(include_str!("../../../../fixtures/services.json"))
            .expect("services fixture"),
    );
    o.ages = [Some(3); 4];
    m
}

/// A connected model with the three fixture workloads loaded (age 5 s).
pub fn model_with_workloads() -> Model {
    let mut m = model_fixture();
    m.workloads.table.set_rows(
        serde_json::from_str(include_str!("../../../../fixtures/workloads.json"))
            .expect("workloads fixture"),
    );
    m.workloads.loaded_at = Some(Instant::now() - Duration::from_secs(5));
    m
}

/// Renders the model into a `w`×`h` test terminal and returns the text.
pub fn render_to_string(model: &Model, w: u16, h: u16) -> String {
    let mut term = Terminal::new(TestBackend::new(w, h)).expect("test terminal");
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
    let mut term = Terminal::new(TestBackend::new(w, h)).expect("test terminal");
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
