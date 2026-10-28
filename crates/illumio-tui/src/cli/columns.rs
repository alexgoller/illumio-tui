//! The `--csv` columns each command can print, known before it runs, so
//! `--csv --fields` names are checked before any request. Where the
//! columns depend on flags or on the data (draft and mark columns, the
//! traffic views), the list is the union of every column the command can
//! print. `output::write_table_csv` checks (debug builds) that every
//! column printed is listed here, so a new column cannot be left out.
use super::Command;
use super::traffic::{
    CHANGE_DETAIL, COMPARE_COLUMNS, CONNS_BY_DECISION, DRAFT_COUNTS, SECTION_CSV_HEADERS,
};

const FLOWS: &[&str] = &[
    "last seen",
    "src",
    "src labels",
    "dst",
    "dst labels",
    "service",
    "conns",
    "decision",
    "draft",
    "process",
    "mark",
];
const GROUPS: &[&str] = &[
    "src",
    "dst",
    "service",
    "flows",
    "conns",
    "bytes",
    "allowed",
    "pot.blocked",
    "blocked",
];
const MESH: &[&str] = &[
    "user",
    "process",
    "src",
    "dst",
    "service",
    "dst process",
    "flows",
    "conns",
    "bytes",
    "decision",
];
const WORKLOADS: &[&str] = &[
    "group",
    "hostname",
    "name",
    "ip",
    "labels",
    "enforcement",
    "online",
    "agent",
    "sync",
    "heartbeat_age_secs",
    "health",
    "href",
    "missing",
    "change",
    "was",
    "enforcement_mode",
];
const FIELD_VALUE: &[&str] = &["field", "value"];

fn cat(parts: &[&[&'static str]]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for c in parts.iter().flat_map(|p| p.iter()) {
        if !out.contains(c) {
            out.push(c);
        }
    }
    out
}

/// The flow-group views (`--group`, `traffic groups` pairs and
/// `--services`) and the app sections, with their draft and mark columns.
fn traffic_views() -> Vec<&'static str> {
    cat(&[
        GROUPS,
        &CONNS_BY_DECISION,
        &DRAFT_COUNTS,
        &["mark"],
        &SECTION_CSV_HEADERS,
        &COMPARE_COLUMNS,
        &[CHANGE_DETAIL],
    ])
}

/// Every column `command` can print with `--csv`; empty when it prints no
/// CSV (`--csv` is refused before it runs, or it prints no envelope).
pub fn csv_columns(command: &Command) -> Vec<&'static str> {
    use super::app::SectionArg as A;
    use super::filters::FiltersCmd as F;
    use super::marks::MarksCmd as M;
    use super::policy::{PolicyCmd as P, RulesAction as RA};
    use super::profile::ProfileCmd as PR;
    use super::traffic::TrafficCmd as T;
    use super::workloads::WorkloadsAction as W;
    // `--save-filter` prints no table (`--csv` is refused).
    if super::filters::saves_filter(command) {
        return Vec::new();
    }
    match command {
        Command::Health => Vec::new(),
        Command::Labels(c) if c.action.is_none() => {
            vec!["key", "value", "workloads", "rules", "deleted", "href"]
        }
        Command::LabelGroups => vec!["name", "key", "labels", "sub_groups", "pending", "href"],
        Command::Iplists(c) if c.action.is_none() => {
            vec!["name", "size", "ranges", "pending", "href"]
        }
        Command::Services(c) if c.action.is_none() => vec!["name", "ports", "pending", "href"],
        Command::Labels(_) | Command::Iplists(_) | Command::Services(_) => Vec::new(),
        Command::Events(_) => cat(&[&super::events::HEADERS, &["diff"]]),
        Command::Workloads(c) => match &c.action {
            None => WORKLOADS.to_vec(),
            Some(W::VenSummary(_)) => vec!["section", "name", "count", "note"],
            Some(W::Add(_) | W::Update(_) | W::Delete(_) | W::Enforce(_) | W::Relabel(_)) => {
                Vec::new()
            }
        },
        Command::App(a) => match a.section {
            Some(A::Workloads) => vec![
                "href",
                "name",
                "role",
                "enforcement",
                "enforcement_mode",
                "ven",
            ],
            Some(A::Inbound | A::Outbound) => cat(&[
                &["peer", "service", "decision", "flows", "conns", "bytes"],
                &DRAFT_COUNTS,
                &["mark"],
            ]),
            Some(A::Policy) => vec!["ruleset", "enabled", "reason", "rules"],
            None => Vec::new(),
        },
        Command::Gen(_) => vec![
            "ruleset",
            "action",
            "consumers",
            "providers",
            "services",
            "unscoped",
            "existing_rule",
            "description",
        ],
        Command::Readiness(_) => vec![
            "app",
            "workloads",
            "managed",
            "unmanaged",
            "enforced",
            "selective",
            "visibility",
            "idle",
            "offline",
            "missing_tier",
            "window",
            "flows",
            "connections",
            "little_evidence",
            "conns_in",
            "conns_out",
            "allowed",
            "would_block",
            "accepted",
            "would_block_flows",
            "coverage",
            "pending",
            "verdict",
            "warnings",
            "trend",
        ],
        Command::Exposure(_) => super::exposure::CSV_COLUMNS.to_vec(),
        Command::Enforce(a) if a.watch => vec!["direction", "peer", "service", "flows", "conns"],
        Command::Enforce(_) => Vec::new(),
        Command::Policy(p) => match p {
            P::Rulesets(c) if c.action.is_none() => {
                vec!["name", "enabled", "scopes", "rules", "pending", "href"]
            }
            P::Rules(c) => match &c.action {
                None => vec![
                    "kind",
                    "enabled",
                    "consumers",
                    "providers",
                    "services",
                    "P/C",
                    "description",
                    "pending",
                    "href",
                ],
                Some(RA::Impact(i)) if i.list.is_some() => {
                    vec![
                        "hostname",
                        "name",
                        "labels",
                        "enforcement",
                        "online",
                        "href",
                    ]
                }
                Some(_) => Vec::new(),
            },
            P::Search(_) => vec![
                "ruleset",
                "kind",
                "enabled",
                "scope",
                "consumers",
                "providers",
                "services",
                "href",
            ],
            P::Pending(_) => vec!["object_type", "change_type", "name", "href", "diff"],
            P::Impact(_) => super::policy_impact::CSV_COLUMNS.to_vec(),
            P::Rulesets(_) | P::DenyRules(_) | P::Provision(_) | P::Revert(_) => Vec::new(),
        },
        Command::WhereUsed(_) => super::where_used::HEADERS.to_vec(),
        Command::Hygiene(_) => super::hygiene::HEADERS.to_vec(),
        Command::PairingProfiles(c) => match c.write_flags() {
            None => super::pairing_profiles::HEADERS.to_vec(),
            Some(_) => Vec::new(),
        },
        Command::Traffic(t) => match t {
            _ if t.filter().is_some_and(|f| f.dry_run) => Vec::new(),
            T::Query(_) => cat(&[FLOWS, &COMPARE_COLUMNS, &traffic_views()]),
            T::Groups(_) => traffic_views(),
            T::Mesh(_) => cat(&[MESH, &DRAFT_COUNTS, &["mark"], &COMPARE_COLUMNS]),
            T::Explain(_) => vec![
                "src",
                "dst",
                "service",
                "decision",
                "draft",
                "verdict",
                "kind",
                "rule href",
                "ruleset",
                "scopes",
                "consumers",
                "providers",
                "services",
                "enabled",
            ],
        },
        Command::Profile(PR::List) => vec!["name", "host", "org_id", "default"],
        Command::Profile(PR::Use { .. } | PR::Add(_)) => Vec::new(),
        Command::Marks(m) => match m {
            M::List(_) => vec![
                "id",
                "action",
                "src",
                "dst",
                "service",
                "decision",
                "draft_decision",
                "pce",
                "org",
                "expires",
                "active",
                "comment",
                "created",
                "created_on",
            ],
            M::Add(_) => vec!["id"],
            M::Rm(_) => vec!["removed"],
            M::Import(_) => vec!["added", "replaced", "skipped"],
            M::Export(_) => Vec::new(),
        },
        Command::Filters(f) => match f {
            F::List(_) => vec!["screen", "name", "default", "summary"],
            F::Show(_) => FIELD_VALUE.to_vec(),
            F::Rm(_) | F::Default(_) => Vec::new(),
        },
        Command::Completions(c) => match c.action {
            Some(_) => FIELD_VALUE.to_vec(),
            None => Vec::new(),
        },
        Command::Schema(a) if a.command.is_empty() => vec!["kind", "command", "schema"],
        Command::Schema(_) => Vec::new(),
        Command::Snapshot(s) => {
            use super::snapshot::{
                DIFF_HEADERS, LIST_HEADERS, RM_HEADERS, SAVE_HEADERS, SnapshotCmd as SN,
            };
            match s {
                SN::Save(_) => SAVE_HEADERS.to_vec(),
                SN::List => LIST_HEADERS.to_vec(),
                SN::Diff(_) => DIFF_HEADERS.to_vec(),
                SN::Rm(_) | SN::Prune(_) => RM_HEADERS.to_vec(),
            }
        }
    }
}
