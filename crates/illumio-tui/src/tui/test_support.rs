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
    let mut m = Model::new_connecting(
        profile,
        "dark".into(),
        60,
        1000,
        "24h".into(),
        crate::tui::model::WritesState::off("started without --allow-writes"),
    );
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
    m.policy.set_rule_sets(raw, &idx, None);
    // The first ruleset has inline deny rules; the second is left to the fallback.
    m.policy.loaded_at = Some(Instant::now() - Duration::from_secs(5));
    m
}

/// `m` with the four fixture workloads cached (loaded 5 s ago, fixed clock)
/// and the rules pane rebuilt, so the impact is warm.
pub fn with_workloads(mut m: Model) -> Model {
    m.workloads.raw = serde_json::from_str(include_str!("../../../../fixtures/workloads.json"))
        .expect("workloads fixture");
    m.workloads.now = Some("2026-10-10T08:00:00Z".parse().expect("fixed test clock"));
    m.workloads.rebuild();
    m.workloads.loaded_at = Some(Instant::now() - Duration::from_secs(5));
    m.rebuild_rules();
    m
}

/// A connected model with the three fixture object lists loaded (ages of 3 s).
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
    o.services.set_rows(
        serde_json::from_str(include_str!("../../../../fixtures/services.json"))
            .expect("services fixture"),
    );
    o.ages = [Some(3); 3];
    m
}

/// `model_with_policy` plus the two fixture IP lists (loaded 3 s ago): sizes
/// computed, `used_by` counted from the loaded draft rulesets.
pub fn model_with_ip_lists() -> Model {
    let mut m = model_with_policy();
    let lists: Vec<IpList> =
        serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json"))
            .expect("ip lists fixture");
    m.ip_lists.set_lists(lists, &m.policy);
    m.ip_lists.loaded_at = Some(Instant::now() - Duration::from_secs(3));
    m
}

/// A connected model with the three fixture workloads loaded (age 5 s).
pub fn model_with_workloads() -> Model {
    let mut m = model_fixture();
    m.workloads.raw = serde_json::from_str(include_str!("../../../../fixtures/workloads.json"))
        .expect("workloads fixture");
    m.workloads.now = Some("2026-10-10T08:00:00Z".parse().expect("fixed test clock"));
    m.workloads.rebuild();
    m.workloads.loaded_at = Some(Instant::now() - Duration::from_secs(5));
    m
}

/// A connected model with the four fixture events loaded (age 5 s, window 24h).
pub fn model_with_events() -> Model {
    let mut m = model_fixture();
    m.events.raw = serde_json::from_str(include_str!("../../../../fixtures/events.json"))
        .expect("events fixture");
    m.events.rebuild();
    m.events.loaded_at = Some(Instant::now() - Duration::from_secs(5));
    m.events.age_secs = 0;
    m.events.window = Some((
        "2026-10-09T08:00:00Z".to_string(),
        "2026-10-10T08:00:00Z".to_string(),
    ));
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

/// The plan the confirm overlay shows for a new label `app=web2`.
pub fn label_post_plan() -> pce::write::Plan {
    pce::write::Plan::Send {
        request: pce::write::WriteRequest {
            method: pce::WriteMethod::Post,
            path: "/orgs/5/labels".into(),
            body: Some(serde_json::json!({"key": "app", "value": "web2"})),
            summary: "create label app=web2".into(),
        },
        kind: pce::write::ObjectKind::Label,
    }
}

/// The plan for `x` on the Corp IP list, with the TUI's named summary.
pub fn ip_list_delete_plan() -> pce::write::Plan {
    pce::write::Plan::Send {
        request: pce::write::WriteRequest {
            method: pce::WriteMethod::Delete,
            path: "/orgs/5/sec_policy/draft/ip_lists/21".into(),
            body: None,
            summary: "delete ip_list Corp".into(),
        },
        kind: pce::write::ObjectKind::IpList,
    }
}

/// Shop-Prod as active, and a draft of it with a new description, rule 2
/// without its 8000-8099/tcp service and a new rule 4: the ruleset diff the
/// tests and the snapshot draw.
pub fn rule_set_diff_fixture() -> (pce::models::RuleSet, pce::models::RuleSet) {
    let sets: Vec<pce::models::RuleSet> =
        serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json"))
            .expect("rule sets fixture");
    let active = sets[0].clone();
    let mut draft = active.clone();
    draft.description = Some("Ringfence shop".into());
    draft.rules[1].ingress_services.truncate(1);
    draft.rules.push(
        serde_json::from_value(serde_json::json!({
            "href": "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/4", "enabled": true,
            "providers": [{"label": {"href": "/orgs/5/labels/1"}}],
            "consumers": [{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/20"}}],
            "ingress_services": [{"port": 22, "proto": 6}]
        }))
        .expect("rule 4"),
    );
    (active, draft)
}

/// The plan `P` shows for the two fixture pending entries (a ruleset and an
/// IP list), as `Writer::plan_provision` builds it.
pub fn provision_plan() -> pce::write::Plan {
    pce::write::Plan::Send {
        request: pce::write::WriteRequest {
            method: pce::WriteMethod::Post,
            path: "/orgs/5/sec_policy".into(),
            body: Some(serde_json::json!({
                "update_description": "illumio-tui provision",
                "change_subset": {
                    "ip_lists": [{"href": "/orgs/5/sec_policy/draft/ip_lists/21"}],
                    "rule_sets": [{"href": "/orgs/5/sec_policy/draft/rule_sets/101"}]
                }
            })),
            summary: "provision 1 ip_list, 1 rule_set".into(),
        },
        kind: pce::write::ObjectKind::Provision,
    }
}
