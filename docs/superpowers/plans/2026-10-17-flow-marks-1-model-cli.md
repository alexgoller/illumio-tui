# Flow marks (1 of 3): the model, the file and the CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Flow marks without any UI: `pce::marks` (the types, matching and precedence, the partition of a result and the per-row annotations, the side grammar), `illumio-tui::marks` (`marks.toml`, its validation, `MarksStore`: lock, reload, atomic write, `.bak`), `illumio-tui marks list|add|rm|export|import`, and the traffic commands and `app` applying marks (`meta.squelched`, `mark` fields, `--show-squelched`, `--no-marks`) while `gen` only counts them.

**Architecture:** `pce::marks` is pure (no I/O, no clock: callers pass `now`); `partition` classifies every flow once and copies only what matches, and `annotate_*` run the view's own aggregator on each mark's subset and join the rows by their key, so the aggregators keep their signatures. `illumio-tui::marks` holds the serde mirror of the file (strings and TOML values, unknown fields in `extra`) and converts it with the `pce::marks` parsers; every write is one operation under `marks.toml.lock`, from a fresh read, through a temp file and a rename. The CLI reads the file once per command (`cli::marked::CliMarks`), never sends a request for it, and puts the counts in the envelope.

**Tech Stack:** Rust 2024 (rust-version 1.88), serde, toml 1.x, jiff, clap, assert_cmd, wiremock, tempfile. No new dependency.

**Spec:** `docs/superpowers/specs/2026-10-17-flow-marks-design.md` (binding), including "Decisions (Alex, 2026-10-17)"; delivery item 1. Plans 2 (`2026-10-17-flow-marks-2-display.md`) and 3 (`2026-10-17-flow-marks-3-create-manage.md`) build on this one.

**Tasks:** 6, in this order: 1 `pce::marks` (types, matching, precedence, ids, the side grammar); 2 the partition and the row annotations; 3 `marks.toml` and `MarksStore`; 4 `illumio-tui marks …`; 5 traffic, `app` and `gen` apply marks; 6 tracking, read-only smoke, final gate.

## Global Constraints

- The gate is `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`. Every task ends with it green. The code in this plan is already formatted: if `cargo fmt --all --check` complains, the transcription is wrong, not the formatter.
- No new dependencies. `git diff main -- '*Cargo.toml' Cargo.lock` stays empty: no `toml_edit`; `toml` 1.x and `serde` (already in the workspace) only. The workspace's `rust-version` is 1.88: no `File::lock` (1.89); the lock is `create_new`.
- Marks never cause a PCE request. They are matched against loaded flows only; the mark form completes from the loaded name index and workloads only. No marks path returns a `Cmd` (TUI) or calls the client (CLI).
- `marks.toml` lives next to the config file in use: `$XDG_CONFIG_HOME/illumio-tui/marks.toml`, else `~/.config/illumio-tui/marks.toml`, or the directory of `--config`. Tests point it at a temp dir (`XDG_CONFIG_HOME` for the CLI, `MarksStore::new(<tempdir>/marks.toml)` in unit tests); no test may read or write the real home. A TUI model has `MarksStore::none()` (no file) until `app::run` gives it the path.
- A broken marks file never crashes the TUI or the CLI: it is a warning and marks are off. An invalid file is never written.
- Marks are not policy: the generator's rules ignore them.
- Tests use fixtures only (`fixtures/`, `fixtures/recorded/`) and wiremock; never contact a PCE, never read `.env` files. The one live smoke (Plan 1, last task) is read-only and run by the integrator.
- Release builds include Windows: Unix permission code (0600, 0700) sits under `#[cfg(unix)]`.
- Edition 2024: clippy wants let-chains for nested `if let`; take its suggestion and list it as a deviation in the task report.
- Every commit message ends with these two lines:
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx
- Plan 1 has no UI: nothing under `crates/illumio-tui/src/tui/` changes.
- CLI tests run the binary with `env_clear()` and a temp `XDG_CONFIG_HOME`; the traffic tests answer from wiremock with `fixtures/traffic_flows.json`.

## Spec deviations and resolutions (read before Task 1)

- **Precedence** is the spec's: the strongest action wins (critical > important > revisit > accept-blocked > reviewed > squelch); specificity (sides, the most specific service, 1 each for `decision`, `draft_decision`, `pce`), then the earlier `created` (none last), then the lower id, only pick the reported mark within one action.
- **`draft_decision`** (Alex's decision 2) is a field of the file, the CLI (`--draft-decision`) and the form (`Draft`). `MarkSet::active(pce, now, draft)` takes whether draft decisions are loaded; without them such marks are inactive and counted as waiting (`N marks wait for the draft decision (D)`). It is matched against the draft decision's class (`allowed_across_boundary` is allowed).
- **`Partition`** keeps `kept` as a `Cow` (borrowed unless a flow is left out) and its subsets in an owned `Marked`, one subset per winning *mark* (not per action): that is what gives every aggregated row its strongest mark id (`mark.id`) without copying key logic. The annotations take `&Marked`, so the TUI keeps it between rebuilds.
- **`RowMarks`** carries `flows`, `by_action`, `top` (the strongest mark per action) and `hidden` (squelched records of the row left out). `RowMarks::marker()` replaces the spec's free `marker(row_flows, rm, squelched_hidden)`. It is flattened into the row's JSON as `mark` (the marker's action, its strongest mark, its records), `marks` (records per action) and `squelched` (hidden records).
- **The path** is the directory of the config file in use (`--config` too), else the XDG rule (`default_marks_path`).
- **The file**: every known field of `MarkEntry` is a `toml::Value`, so a wrong type is a positioned problem (`mark 3 (m-…): org: expected an org id`), not a whole-file parse error. Writes put `id` first, then the spec table's order, then unknown fields; values are inline (`src = { labels = { … } }`), never sub-tables.
- **`MarkOp`** adds `DeleteExpired` and `Extend { id, days, comment }` to the spec's list; both are judged on the fresh read under the lock.
- **`marks add`** without `--expires` takes the action's default (the form's defaults); `marks list`, `export` and `rm` on an invalid file exit 2 with every problem in the message; `rm` of an unknown id exits 2 (`unknown mark id …: nothing removed`).
- **`meta.squelched`** is `{rows, flows}` for traffic and `app`, `{flows}` for `gen`; with `meta.marks` it is present only while a valid file has an active mark. The `mark` column of tables and CSV appears when a row carries marks.

## Review Focus

- **A broad squelch and a narrow critical on the same flow**: the critical wins and the flow stays in every view (the spec's 'no squelch ever hides a highlight'). Pinned in Task 1 (`the_strongest_action_wins_whatever_the_specificity`) and Task 2 (`a_row_is_hidden_iff_all_its_flows_are_squelched_and_partial_rows_keep_visible_counts`, Corp → shop/prod).
- **Two writers at once** (a second TUI, the CLI, a hand edit): both added marks survive, an edit of a mark deleted elsewhere says so. Pinned in Task 3 (`two_stores_adding_to_one_file_keep_both_marks`, `an_edit_of_a_mark_deleted_elsewhere_is_gone`, `a_stale_lock_is_removed_and_a_live_one_refuses`).
- **A hand-edited file with comments, a typo or a truncated save**: the comments are kept once in `marks.toml.bak`, the typo names the mark and field, garbage never panics, an invalid file is never overwritten. Pinned in Task 3 (`comments_are_kept_once_in_the_backup_then_the_header_says_so`, `every_validation_rule_names_the_mark`, `truncated_and_garbage_files_never_panic`, `an_invalid_file_is_never_written`) and Task 4 (`an_invalid_file_refuses_every_write_and_a_held_lock_exits_3`).
- **`--app` with `--by both`**: a squelched record in two sections (out of its source app, into its destination app) is counted once in `meta.squelched.flows`. Pinned in Task 2 (`visible_plus_squelched_is_the_input_in_every_view`, the `both` branch).
- **`gen` with marks**: the proposal is byte-identical to `--no-marks`, only `meta` differs. Pinned in Task 5 (`gen_ignores_marks_for_its_rules_and_only_counts_them`).

---

### Task 1: `pce::marks`: actions, sides, services, matching, precedence, ids

**Files:**
- Modify: `crates/pce/src/lib.rs`
- Create: `crates/pce/src/marks/mod.rs`
- Modify: `crates/pce/src/write/specs.rs`

**Interfaces:**
- Consumes: `pce::traffic::{ResolvedFlow, ResolvedEndpoint}`, `pce::models::{PolicyDecision, DraftDecision::class, proto_name, proto_number}`, `pce::write::specs::parse_cidr` (made `pub(crate)` here).
- Produces (`pce::marks`):

```rust
pub enum Action { Critical, Important, Revisit, AcceptBlocked, Reviewed, Squelch } // Ord: strongest first; serde kebab-case
impl Action { pub const ALL: [Action; 6]; fn as_str; fn parse(&str) -> Option<Self>; fn index; fn is_highlight; fn is_quiet;
              fn names() -> String /* "critical important … squelch" */; fn default_expiry(self) -> &'static str /* "90d", "30d", "7d", "never" */ }
pub fn parse_expiry(s: &str, now: Timestamp) -> Result<Option<Timestamp>, String>; // 1d 7d 30d 90d (any Nd), YYYY-MM-DD (23:59:59Z), RFC 3339, never
pub fn expiry_text(e: Option<Timestamp>) -> String;  // "expires 2027-01-15 (UTC)" | "never expires"
pub fn expiry_date(t: Timestamp) -> String;          // "2027-01-15"
pub enum Side { Any, Labels(BTreeMap<String, Vec<String>>), Workload(String), Ip { addr: IpAddr, bits: u8 }, IpList(String) }
impl Side { fn matches(&self, ep: &ResolvedEndpoint) -> bool; fn specificity(&self) -> u32 }
pub fn side_text(s: &Side) -> String;  pub fn parse_side(s: &str) -> Result<Side, String>;
pub enum ServiceSpec { Proto(u8), Ports { proto: u8, from: u16, to: u16 } }
impl ServiceSpec { fn parse(&str) -> Result<Self, String>; fn text; fn matches(proto: Option<u8>, port: Option<u16>); fn specificity }
pub fn parse_decision(s: &str) -> Result<PolicyDecision, String>;
pub struct PceScope { pub host: String, pub org: Option<u64> }   // fn covers(&self, &PceId) -> bool
pub struct PceId { pub host: String, pub org: u64 }
pub struct Mark { pub id, pub action, pub src: Side, pub dst: Side, pub services: Vec<ServiceSpec>, pub decisions: Vec<PolicyDecision>,
                  pub draft_decisions: Vec<PolicyDecision>, pub scope: Option<PceScope>, pub expires: Option<Timestamp>,
                  pub comment: String, pub created: Option<Timestamp>, pub created_on: Option<String> }
impl Mark { fn matches(&self, &ResolvedFlow) -> bool; fn specificity; fn is_expired(now); fn in_scope(Option<&PceId>);
            fn service_text(&self) -> Option<String>; fn match_text(&self) -> String; fn canonical(&self) -> String; fn problems(&self) -> Vec<String> }
pub struct MarkSet { pub marks: Vec<Mark> }  // fn active(&self, pce: Option<&PceId>, now: Timestamp, draft: bool) -> ActiveMarks; fn find(&self, id) -> Option<usize>
pub struct Hit { pub action: Action, pub mark: usize }
impl ActiveMarks<'_> { fn is_empty; fn len; fn waiting_for_draft; fn set; fn mark(i); fn is_active(i); fn has(Action); fn rank(i) -> usize;
                       fn classify(&self, &ResolvedFlow) -> Option<Hit>; fn matching(&self, &ResolvedFlow) -> Vec<usize> /* winner first */ }
pub fn fnv1a(bytes: &[u8]) -> u64;
pub fn new_id(now_ns: i128, pid: u32, canonical: &str, taken: impl Fn(&str) -> bool) -> String;  // "m-" + 6 Crockford
pub fn derived_id(canonical: &str) -> String;                                                     // "h-" + 6 Crockford
```

`#[cfg(test)] pub(crate) mod tests` exports `mark(id, action, src, dst)` for Task 2's tests.

- [ ] **Step 1: Write the failing tests**

In `crates/pce/src/lib.rs`, replace:

```rust
pub mod key_order;
pub mod limiter;
pub mod models;
pub mod resolve;
```

with:

```rust
pub mod key_order;
pub mod limiter;
pub mod marks;
pub mod models;
pub mod resolve;
```

Create `crates/pce/src/marks/mod.rs` with its tests only (Step 3 adds the code above them):

```rust
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::models::DraftDecision;
    use crate::resolve::ResolvedLabel;

    pub(crate) fn wl(host: &str, ip: &str, labels: &[(&str, &str)]) -> ResolvedEndpoint {
        ResolvedEndpoint {
            ip: Some(ip.into()),
            hostname: Some(host.into()),
            workload_href: Some(format!("/orgs/1/workloads/{host}")),
            labels: labels
                .iter()
                .map(|(k, v)| ResolvedLabel {
                    href: format!("/orgs/1/labels/{k}-{v}"),
                    key: (*k).into(),
                    value: (*v).into(),
                })
                .collect(),
            ..Default::default()
        }
    }

    pub(crate) fn addr(ip: &str, lists: &[&str]) -> ResolvedEndpoint {
        ResolvedEndpoint {
            ip: Some(ip.into()),
            ip_lists: lists.iter().map(|l| (*l).into()).collect(),
            ..Default::default()
        }
    }

    pub(crate) fn flow(
        src: ResolvedEndpoint,
        dst: ResolvedEndpoint,
        port: Option<u16>,
        proto: &str,
        d: PolicyDecision,
    ) -> ResolvedFlow {
        ResolvedFlow {
            src,
            dst,
            port,
            proto: proto.into(),
            process_name: None,
            user_name: None,
            num_connections: 1,
            policy_decision: d,
            draft_policy_decision: None,
            flow_direction: None,
            first_detected: None,
            last_detected: None,
            rules: Vec::new(),
            bytes_in: None,
            bytes_out: None,
        }
    }

    /// A portable mark `src → dst` with `action`, no service, no expiry.
    pub(crate) fn mark(id: &str, action: Action, src: &str, dst: &str) -> Mark {
        Mark {
            id: id.into(),
            action,
            src: parse_side(src).unwrap(),
            dst: parse_side(dst).unwrap(),
            services: Vec::new(),
            decisions: Vec::new(),
            draft_decisions: Vec::new(),
            scope: None,
            expires: None,
            comment: "test".into(),
            created: None,
            created_on: None,
        }
    }

    fn web() -> ResolvedEndpoint {
        wl(
            "web-01",
            "10.0.1.10",
            &[("app", "shop"), ("env", "prod"), ("role", "web")],
        )
    }

    fn db() -> ResolvedEndpoint {
        wl(
            "db-01",
            "10.0.2.20",
            &[("app", "shop"), ("env", "prod"), ("role", "db")],
        )
    }

    fn now() -> Timestamp {
        "2026-10-17T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn every_side_kind_matches_in_one_direction_only() {
        let f = flow(web(), db(), Some(5432), "tcp", PolicyDecision::Allowed);
        for (src, dst) in [
            ("role=web", "role=db"),
            ("wl:WEB-01", "wl:db-01"),
            ("ip:10.0.1.10", "ip:10.0.0.0/16"),
            ("any", "role=db env=prod|dr"),
        ] {
            let m = mark("m-1", Action::Reviewed, src, dst);
            assert!(m.matches(&f), "{src} → {dst}");
            let back = mark("m-2", Action::Reviewed, dst, src);
            assert!(!back.matches(&f), "{dst} → {src} must not match a → b");
        }
        let corp = flow(
            addr("10.1.4.9", &["Corp"]),
            web(),
            Some(8443),
            "tcp",
            PolicyDecision::Blocked,
        );
        assert!(mark("m-3", Action::Squelch, "ipl:corp", "any").matches(&corp));
        assert!(!mark("m-4", Action::Squelch, "ipl:other", "any").matches(&corp));
    }

    #[test]
    fn labels_need_every_key_on_a_workload_and_never_match_unresolved_ones() {
        let f = flow(web(), db(), Some(5432), "tcp", PolicyDecision::Allowed);
        let m = |src: &str| mark("m-1", Action::Reviewed, src, "any");
        assert!(m("app=shop env=prod").matches(&f));
        assert!(m("app=crm|shop").matches(&f), "a list is OR");
        assert!(!m("app=shop loc=eu").matches(&f), "a missing key");
        assert!(!m("app=Shop").matches(&f), "values are case-sensitive");
        let mut unresolved = web();
        unresolved.labels[0].key = "?".into();
        let g = flow(unresolved, db(), None, "icmp", PolicyDecision::Allowed);
        assert!(!m("?=shop").matches(&g));
        // Labels on a non-workload endpoint never match.
        let mut ep = addr("10.9.9.9", &[]);
        ep.labels = web().labels;
        let h = flow(ep, db(), None, "icmp", PolicyDecision::Allowed);
        assert!(!m("app=shop").matches(&h));
    }

    #[test]
    fn cidrs_contain_their_own_family_only() {
        let v6 = flow(
            addr("2001:db8::5", &[]),
            db(),
            Some(443),
            "tcp",
            PolicyDecision::Allowed,
        );
        let m = |src: &str| mark("m-1", Action::Reviewed, src, "any");
        assert!(m("ip:2001:db8::/32").matches(&v6));
        assert!(m("ip:2001:db8::5").matches(&v6), "a host address");
        assert!(!m("ip:2001:db8::6").matches(&v6));
        assert!(
            !m("ip:0.0.0.0/0").matches(&v6),
            "a v4 net never holds a v6 address"
        );
        assert!(m("ip:::/0").matches(&v6));
    }

    #[test]
    fn services_match_ports_ranges_protocols_and_lists() {
        let at = |port: Option<u16>, proto: &str| {
            flow(web(), db(), port, proto, PolicyDecision::Allowed)
        };
        let with = |svc: &[&str]| {
            let mut m = mark("m-1", Action::Reviewed, "any", "any");
            m.services = svc.iter().map(|s| ServiceSpec::parse(s).unwrap()).collect();
            m
        };
        assert!(with(&["5432/tcp"]).matches(&at(Some(5432), "tcp")));
        assert!(!with(&["5432/udp"]).matches(&at(Some(5432), "tcp")));
        let range = with(&["8000-8010/tcp"]);
        assert!(range.matches(&at(Some(8000), "tcp")) && range.matches(&at(Some(8010), "tcp")));
        assert!(!range.matches(&at(Some(7999), "tcp")) && !range.matches(&at(Some(8011), "tcp")));
        assert!(with(&["udp"]).matches(&at(Some(53), "udp")));
        assert!(with(&["icmp"]).matches(&at(None, "icmp")));
        assert!(with(&["47"]).matches(&at(None, "47")));
        let list = with(&["443/tcp", "icmp"]);
        assert!(list.matches(&at(None, "icmp")) && list.matches(&at(Some(443), "tcp")));
        assert!(!list.matches(&at(Some(80), "tcp")));
    }

    #[test]
    fn service_specs_parse_and_print_back() {
        for s in ["5666/tcp", "8000-8010/tcp", "udp", "icmp", "47"] {
            assert_eq!(ServiceSpec::parse(s).unwrap().text(), s);
        }
        assert_eq!(ServiceSpec::parse("80-80/TCP").unwrap().text(), "80/tcp");
        for (bad, why) in [
            ("70000/tcp", "not a port"),
            ("90-80/tcp", "ends before"),
            ("80/icmp", "has no ports"),
            ("80/bogus", "unknown protocol"),
            ("bogus", "unknown protocol"),
            ("x/tcp", "not a port"),
        ] {
            let e = ServiceSpec::parse(bad).unwrap_err();
            assert!(e.contains(why), "{bad}: {e}");
        }
    }

    #[test]
    fn decisions_match_the_reported_one_and_draft_decisions_the_draft_class() {
        let mut f = flow(web(), db(), None, "icmp", PolicyDecision::Blocked);
        let mut m = mark("m-1", Action::Reviewed, "any", "any");
        m.decisions = vec![PolicyDecision::Blocked, PolicyDecision::PotentiallyBlocked];
        assert!(m.matches(&f));
        m.decisions = vec![PolicyDecision::Allowed];
        assert!(!m.matches(&f));
        m.decisions = vec![PolicyDecision::Blocked];
        m.draft_decisions = vec![PolicyDecision::Allowed];
        assert!(!m.matches(&f), "no draft decision loaded");
        f.draft_policy_decision = Some(DraftDecision::Other("allowed_across_boundary".into()));
        assert!(m.matches(&f), "the draft class is allowed");
        f.draft_policy_decision = Some(DraftDecision::Blocked);
        assert!(!m.matches(&f));
    }

    #[test]
    fn the_strongest_action_wins_whatever_the_specificity() {
        let f = flow(web(), db(), Some(5432), "tcp", PolicyDecision::Blocked);
        let mut broad_squelch = mark("m-s", Action::Squelch, "any", "any");
        broad_squelch.services = vec![ServiceSpec::parse("5432/tcp").unwrap()];
        let set = MarkSet {
            marks: vec![
                broad_squelch,
                mark("m-r", Action::Reviewed, "wl:web-01", "wl:db-01"),
                mark("m-c", Action::Critical, "any", "any"),
            ],
        };
        let mut set = set;
        for m in &mut set.marks {
            m.scope = Some(PceScope {
                host: "pce.example".into(),
                org: None,
            });
        }
        let pce = PceId {
            host: "PCE.example".into(),
            org: 1,
        };
        let a = set.active(Some(&pce), now(), false);
        assert_eq!(
            a.classify(&f),
            Some(Hit {
                action: Action::Critical,
                mark: 2
            })
        );
        assert_eq!(a.matching(&f), vec![2, 1, 0], "the winner first");
        // Every pair of actions: the stronger one wins.
        for (i, strong) in Action::ALL.into_iter().enumerate() {
            for weak in &Action::ALL[i + 1..] {
                let set = MarkSet {
                    marks: vec![
                        mark("m-w", *weak, "wl:web-01", "role=db"),
                        mark("m-s", strong, "any", "any"),
                    ],
                };
                let mut set = set;
                set.marks[0].scope = Some(PceScope {
                    host: "pce.example".into(),
                    org: None,
                });
                let hit = set.active(Some(&pce), now(), false).classify(&f).unwrap();
                assert_eq!(hit.action, strong, "{strong:?} over {weak:?}");
            }
        }
    }

    #[test]
    fn within_one_action_the_most_specific_then_the_oldest_then_the_lowest_id_is_reported() {
        let f = flow(web(), db(), Some(5432), "tcp", PolicyDecision::Allowed);
        let t = |s: &str| Some(s.parse::<Timestamp>().unwrap());
        let mut a = mark("m-b", Action::Reviewed, "any", "any");
        let mut b = mark("m-a", Action::Reviewed, "any", "any");
        let c = mark("m-c", Action::Reviewed, "app=shop", "any");
        a.created = t("2026-01-01T00:00:00Z");
        b.created = t("2026-02-01T00:00:00Z");
        let set = MarkSet {
            marks: vec![a.clone(), b.clone(), c],
        };
        let hit = |s: &MarkSet| s.active(None, now(), false).classify(&f).unwrap().mark;
        assert_eq!(hit(&set), 2, "labels beat any");
        let set = MarkSet {
            marks: vec![b.clone(), a.clone()],
        };
        assert_eq!(hit(&set), 1, "the earlier created");
        b.created = a.created;
        let set = MarkSet {
            marks: vec![a.clone(), b.clone()],
        };
        assert_eq!(hit(&set), 1, "then the lower id (m-a)");
        b.created = None;
        let set = MarkSet { marks: vec![b, a] };
        assert_eq!(hit(&set), 1, "a mark without created comes last");
    }

    #[test]
    fn specificity_scores_sides_services_and_fields() {
        let mut m = mark("m-1", Action::Reviewed, "wl:web-01", "ip:10.0.0.5");
        assert_eq!(m.specificity(), 9);
        m.dst = parse_side("ip:10.0.0.0/8").unwrap();
        m.services = vec![
            ServiceSpec::parse("udp").unwrap(),
            ServiceSpec::parse("80/tcp").unwrap(),
        ];
        m.decisions = vec![PolicyDecision::Blocked];
        m.scope = Some(PceScope {
            host: "h".into(),
            org: None,
        });
        assert_eq!(m.specificity(), 5 + 1 + 3 + 1 + 1);
        assert_eq!(parse_side("app=a env=b").unwrap().specificity(), 4);
        assert_eq!(parse_side("ipl:x").unwrap().specificity(), 2);
        assert_eq!(Side::Any.specificity(), 0);
    }

    #[test]
    fn expiry_is_exclusive_and_scope_follows_host_and_org() {
        let mut m = mark("m-1", Action::Squelch, "any", "any");
        m.expires = Some(now());
        assert!(m.is_expired(now()), "now == expires is expired");
        assert!(!m.is_expired(now() - jiff::SignedDuration::from_secs(1)));
        let pce = |host: &str, org: u64| PceId {
            host: host.into(),
            org,
        };
        assert!(m.in_scope(Some(&pce("any", 1))), "portable");
        m.scope = Some(PceScope {
            host: "pce.example.com".into(),
            org: None,
        });
        assert!(m.in_scope(Some(&pce("PCE.Example.com", 7))));
        assert!(!m.in_scope(Some(&pce("other", 7))));
        m.scope = Some(PceScope {
            host: "pce.example.com".into(),
            org: Some(1),
        });
        assert!(m.in_scope(Some(&pce("pce.example.com", 1))));
        assert!(!m.in_scope(Some(&pce("pce.example.com", 2))));
        let set = MarkSet { marks: vec![m] };
        assert!(set.active(Some(&pce("other", 1)), now(), false).is_empty());
        assert_eq!(
            set.active(Some(&pce("pce.example.com", 1)), now(), false)
                .len(),
            0,
            "expired"
        );
    }

    #[test]
    fn draft_decision_marks_wait_for_draft_decisions() {
        let mut m = mark("m-1", Action::Revisit, "any", "any");
        m.decisions = vec![PolicyDecision::Blocked];
        m.draft_decisions = vec![PolicyDecision::Allowed];
        let set = MarkSet { marks: vec![m] };
        let off = set.active(None, now(), false);
        assert!(off.is_empty());
        assert_eq!(off.waiting_for_draft(), 1);
        let on = set.active(None, now(), true);
        assert_eq!((on.len(), on.waiting_for_draft()), (1, 0));
    }

    #[test]
    fn the_side_grammar_round_trips() {
        for s in [
            "any",
            "app=shop env=dr|prod",
            "wl:web-01",
            "ip:10.0.0.0/8",
            "ip:10.0.0.5",
            "ip:2001:db8::/32",
            "ipl:rfc1918",
            "app=\"my app\" env=prod",
        ] {
            assert_eq!(side_text(&parse_side(s).unwrap()), s, "{s}");
        }
        assert_eq!(
            parse_side("app=shop,env=prod").unwrap(),
            parse_side("env=prod  app=shop").unwrap()
        );
        assert_eq!(
            parse_side("ip:10.0.0.5").unwrap(),
            Side::Ip {
                addr: "10.0.0.5".parse().unwrap(),
                bits: 32
            }
        );
        let quoted = parse_side("app=\"a|b\"").unwrap();
        assert_eq!(
            quoted,
            Side::Labels(BTreeMap::from([("app".into(), vec!["a|b".into()])]))
        );
        for (bad, why) in [
            ("", "empty side"),
            ("app", "key=value"),
            ("=shop", "missing label key"),
            ("app=", "empty label value"),
            ("app=a|", "empty label value"),
            ("app=a app=b", "twice"),
            ("ip:10.0.0.0/33", "at most 32"),
            ("ip:nope", "not an IP"),
            ("wl:", "needs a workload"),
            ("app=\"open", "unclosed quote"),
        ] {
            let e = parse_side(bad).unwrap_err();
            assert!(e.contains(why), "{bad:?}: {e}");
        }
    }

    #[test]
    fn mark_problems_name_the_rules() {
        let mut m = mark("m-1", Action::Critical, "any", "wl:db-01");
        assert!(m.problems()[0].contains("workload needs pce"));
        m.scope = Some(PceScope {
            host: "h".into(),
            org: None,
        });
        assert!(m.problems().is_empty());
        let mut a = mark("m-2", Action::AcceptBlocked, "any", "any");
        assert!(a.problems()[0].contains("needs decision"));
        a.decisions = vec![PolicyDecision::Blocked, PolicyDecision::Allowed];
        assert!(a.problems()[0].contains("cannot include allowed"));
        a.decisions = vec![PolicyDecision::Blocked];
        assert!(a.problems().is_empty());
    }

    #[test]
    fn ids_are_stable_hashes_in_crockford_base32() {
        let m = mark("", Action::Squelch, "app=monitoring", "any");
        let c = m.canonical();
        let a = new_id(1, 2, &c, |_| false);
        assert_eq!(
            a,
            new_id(1, 2, &c, |_| false),
            "the same input, the same id"
        );
        assert!(a.starts_with("m-") && a.len() == 8, "{a}");
        assert!(a[2..].bytes().all(|b| CROCKFORD.contains(&b)), "{a}");
        let b = new_id(1, 2, &c, |id| id == a);
        assert_ne!(a, b, "a taken id is hashed again");
        let h = derived_id(&c);
        assert_eq!(h, derived_id(&m.clone().canonical()), "stable across loads");
        assert!(h.starts_with("h-") && h.len() == 8);
        let mut other = m.clone();
        other.comment = "another reason".into();
        assert_ne!(derived_id(&other.canonical()), h);
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn expiries_parse_in_utc_with_defaults_per_action() {
        let day = jiff::SignedDuration::from_hours(24);
        assert_eq!(parse_expiry("7d", now()).unwrap(), Some(now() + day * 7));
        assert_eq!(parse_expiry("never", now()).unwrap(), None);
        assert_eq!(
            parse_expiry("2027-01-15", now()).unwrap(),
            Some("2027-01-15T23:59:59Z".parse().unwrap())
        );
        assert_eq!(
            parse_expiry("2027-01-15T10:00:00+02:00", now()).unwrap(),
            Some("2027-01-15T08:00:00Z".parse().unwrap())
        );
        assert!(parse_expiry("0d", now()).is_err());
        assert!(
            parse_expiry("soon", now())
                .unwrap_err()
                .contains("YYYY-MM-DD")
        );
        assert_eq!(
            expiry_text(Some("2027-01-15T23:59:59Z".parse().unwrap())),
            "expires 2027-01-15 (UTC)"
        );
        assert_eq!(expiry_text(None), "never expires");
        let defaults: Vec<&str> = Action::ALL.map(Action::default_expiry).to_vec();
        assert_eq!(defaults, ["never", "never", "7d", "90d", "30d", "90d"]);
    }

    #[test]
    fn actions_parse_and_order_strongest_first() {
        assert_eq!(Action::parse("accept-blocked"), Some(Action::AcceptBlocked));
        assert_eq!(Action::parse("nope"), None);
        assert!(Action::Critical < Action::Squelch);
        assert_eq!(
            Action::names(),
            "critical important revisit accept-blocked reviewed squelch"
        );
        assert_eq!(
            serde_json::to_string(&Action::AcceptBlocked).unwrap(),
            "\"accept-blocked\""
        );
        assert!(
            parse_decision("bogus")
                .unwrap_err()
                .contains("allowed potentially_blocked blocked")
        );
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p pce --lib marks`
Expected: FAIL: it does not compile (`cannot find type Action`, `cannot find function parse_side`, …).

- [ ] **Step 3: Implement**

In `crates/pce/src/marks/mod.rs`, insert above the tests:

```rust
//! Flow marks: the admin's judgement of traffic (critical, important,
//! revisit, accept-blocked, reviewed, squelch), matched against resolved
//! flows. Pure: no I/O and no clock (callers pass `now`); the file
//! (`marks.toml`) is read and written by `illumio-tui::marks`. Marks are
//! never sent to the PCE and never cause a request.
use crate::models::{PolicyDecision, proto_name, proto_number};
use crate::traffic::{ResolvedEndpoint, ResolvedFlow};
use jiff::Timestamp;
use serde::Serialize;
use std::collections::BTreeMap;
use std::net::IpAddr;

/// What a mark says about its flows. `Ord` is precedence: the strongest
/// first, so a highlight (and a quiet action) always beats squelch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    Critical,
    Important,
    Revisit,
    AcceptBlocked,
    Reviewed,
    Squelch,
}

impl Action {
    /// Every action, strongest first.
    pub const ALL: [Action; 6] = [
        Action::Critical,
        Action::Important,
        Action::Revisit,
        Action::AcceptBlocked,
        Action::Reviewed,
        Action::Squelch,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Action::Critical => "critical",
            Action::Important => "important",
            Action::Revisit => "revisit",
            Action::AcceptBlocked => "accept-blocked",
            Action::Reviewed => "reviewed",
            Action::Squelch => "squelch",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        Action::ALL.into_iter().find(|a| a.as_str() == s)
    }

    /// The position in [`Action::ALL`]: an index into per-action arrays.
    pub fn index(self) -> usize {
        self as usize
    }

    /// `critical`, `important`, `revisit`: shown wherever one flow has it.
    pub fn is_highlight(self) -> bool {
        matches!(self, Action::Critical | Action::Important | Action::Revisit)
    }

    /// `accept-blocked`, `reviewed`: a row is dimmed only when all its
    /// flows have one.
    pub fn is_quiet(self) -> bool {
        matches!(self, Action::AcceptBlocked | Action::Reviewed)
    }

    /// `critical important revisit accept-blocked reviewed squelch`.
    pub fn names() -> String {
        Action::ALL.map(Action::as_str).join(" ")
    }

    /// The expiry a new mark gets unless one is given: a hide or a
    /// forgiveness lapses on its own, a flag does not.
    pub fn default_expiry(self) -> &'static str {
        match self {
            Action::Squelch | Action::AcceptBlocked => "90d",
            Action::Reviewed => "30d",
            Action::Revisit => "7d",
            Action::Critical | Action::Important => "never",
        }
    }
}

/// `expires 2027-01-15 (UTC)`, or `never expires`: how the TUI shows an
/// expiry (in UTC, as stored).
pub fn expiry_text(expires: Option<Timestamp>) -> String {
    match expires {
        Some(t) => format!("expires {} (UTC)", expiry_date(t)),
        None => "never expires".into(),
    }
}

/// `2027-01-15`: the UTC date of an expiry.
pub fn expiry_date(t: Timestamp) -> String {
    t.strftime("%Y-%m-%d").to_string()
}

/// An expiry as typed: `1d`, `7d`, `30d`, `90d` (any `<N>d`: now + N x 24
/// h), a date `YYYY-MM-DD` (23:59:59 UTC that day), an RFC 3339 instant,
/// or `never` (`None`). UTC, as the CLI's date-only `--to`.
pub fn parse_expiry(s: &str, now: Timestamp) -> Result<Option<Timestamp>, String> {
    let t = s.trim();
    if t.eq_ignore_ascii_case("never") {
        return Ok(None);
    }
    if let Some(days) = t.strip_suffix('d').and_then(|n| n.parse::<i64>().ok()) {
        if days <= 0 {
            return Err(format!("{t:?}: at least 1d"));
        }
        return Ok(Some(now + jiff::SignedDuration::from_hours(24 * days)));
    }
    if let Ok(ts) = t.parse::<Timestamp>() {
        return Ok(Some(ts));
    }
    if let Ok(d) = t.parse::<jiff::civil::Date>() {
        return d
            .at(23, 59, 59, 0)
            .to_zoned(jiff::tz::TimeZone::UTC)
            .map(|z| Some(z.timestamp()))
            .map_err(|e| format!("{t:?}: {e}"));
    }
    Err(format!(
        "{t:?}: write 1d, 7d, 30d, 90d, YYYY-MM-DD, an RFC 3339 time or never"
    ))
}

/// One side of a mark: which endpoints it matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Side {
    Any,
    /// Every key required, one of its values (a list is OR); workloads only.
    Labels(BTreeMap<String, Vec<String>>),
    /// Hostname, else workload name, case-insensitive. Always PCE-scoped.
    Workload(String),
    /// An address or CIDR containing the endpoint's IP.
    Ip {
        addr: IpAddr,
        bits: u8,
    },
    /// One of the IP lists Explorer reported for the endpoint, by name,
    /// case-insensitive. No range evaluation.
    IpList(String),
}

/// Whether `ip` is inside `net/bits` (the same family only).
fn ip_contains(net: IpAddr, bits: u8, ip: IpAddr) -> bool {
    match (net, ip) {
        (IpAddr::V4(n), IpAddr::V4(a)) => {
            let m = if bits == 0 {
                0
            } else {
                u32::MAX << (32 - bits)
            };
            u32::from(n) & m == u32::from(a) & m
        }
        (IpAddr::V6(n), IpAddr::V6(a)) => {
            let m = if bits == 0 {
                0
            } else {
                u128::MAX << (128 - bits)
            };
            u128::from(n) & m == u128::from(a) & m
        }
        _ => false,
    }
}

fn max_bits(addr: IpAddr) -> u8 {
    if addr.is_ipv4() { 32 } else { 128 }
}

impl Side {
    pub fn matches(&self, ep: &ResolvedEndpoint) -> bool {
        self.matches_at(ep, endpoint_ip(ep))
    }

    /// `matches` with the endpoint's address parsed once per flow.
    fn matches_at(&self, ep: &ResolvedEndpoint, ip: Option<IpAddr>) -> bool {
        match self {
            Side::Any => true,
            Side::Labels(want) => {
                ep.is_workload()
                    && want.iter().all(|(k, values)| {
                        ep.labels
                            .iter()
                            .any(|l| l.key != "?" && &l.key == k && values.contains(&l.value))
                    })
            }
            Side::Workload(name) => ep
                .hostname
                .as_deref()
                .or(ep.name.as_deref())
                .is_some_and(|h| h.eq_ignore_ascii_case(name)),
            Side::Ip { addr, bits } => ip.is_some_and(|ip| ip_contains(*addr, *bits, ip)),
            Side::IpList(name) => ep.ip_lists.iter().any(|l| l.eq_ignore_ascii_case(name)),
        }
    }

    /// workload 5, a host address 4, labels 2 + one per key, ip_list 2, a
    /// wider CIDR 1, any 0.
    pub fn specificity(&self) -> u32 {
        match self {
            Side::Any => 0,
            Side::Labels(l) => 2 + l.len() as u32,
            Side::Workload(_) => 5,
            Side::Ip { addr, bits } if *bits == max_bits(*addr) => 4,
            Side::Ip { .. } => 1,
            Side::IpList(_) => 2,
        }
    }
}

/// `app=shop env=prod|dr`, `wl:web-01`, `ip:10.0.0.0/8`, `ipl:rfc1918` or
/// `any`: the one text form of a side (the TUI form, the CLI, `:marks`).
pub fn side_text(s: &Side) -> String {
    match s {
        Side::Any => "any".into(),
        Side::Labels(l) => l
            .iter()
            .map(|(k, vs)| {
                let vs: Vec<String> = vs.iter().map(|v| quote(v)).collect();
                format!("{}={}", quote(k), vs.join("|"))
            })
            .collect::<Vec<_>>()
            .join(" "),
        Side::Workload(w) => format!("wl:{w}"),
        Side::Ip { addr, bits } if *bits == max_bits(*addr) => format!("ip:{addr}"),
        Side::Ip { addr, bits } => format!("ip:{addr}/{bits}"),
        Side::IpList(l) => format!("ipl:{l}"),
    }
}

/// A label key or value as the side grammar writes it: quoted when it
/// holds a separator.
fn quote(s: &str) -> String {
    if s.is_empty() || s.contains([' ', ',', '|', '=', '"', '\\']) {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

/// Splits `s` at spaces and commas outside double quotes; quotes are
/// removed and `\"` inside them is a quote.
fn tokens(s: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            '\\' if quoted => match chars.next() {
                Some(n) => cur.push(n),
                None => return Err("a `\\` ends the text".into()),
            },
            ' ' | ',' if !quoted => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            // `|` inside quotes is part of the value: mark it so the value
            // split below leaves it alone.
            '|' if quoted => cur.push('\u{1}'),
            c => cur.push(c),
        }
    }
    if quoted {
        return Err("an unclosed quote".into());
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    Ok(out)
}

/// `addr` or `addr/N`, N checked against the family.
fn parse_net(s: &str) -> Result<(IpAddr, u8), String> {
    if s.contains('/') {
        return crate::write::specs::parse_cidr(s);
    }
    let addr: IpAddr = s
        .parse()
        .map_err(|_| format!("{s:?} is not an IP address"))?;
    Ok((addr, max_bits(addr)))
}

/// The side grammar of [`side_text`]. Tokens are separated by spaces or
/// commas; `|` within a value is OR; quote values with spaces.
pub fn parse_side(s: &str) -> Result<Side, String> {
    let t = s.trim();
    if t.is_empty() {
        return Err("empty side: write any, key=value, wl:NAME, ip:CIDR or ipl:NAME".into());
    }
    if t.eq_ignore_ascii_case("any") {
        return Ok(Side::Any);
    }
    let prefixed = |p: &str| {
        t.get(..p.len())
            .filter(|h| h.eq_ignore_ascii_case(p))
            .map(|_| t[p.len()..].trim())
    };
    if let Some(name) = prefixed("wl:") {
        if name.is_empty() {
            return Err("wl: needs a workload name".into());
        }
        return Ok(Side::Workload(name.to_string()));
    }
    if let Some(net) = prefixed("ipl:") {
        if net.is_empty() {
            return Err("ipl: needs an IP list name".into());
        }
        return Ok(Side::IpList(net.to_string()));
    }
    if let Some(net) = prefixed("ip:") {
        let (addr, bits) = parse_net(net)?;
        return Ok(Side::Ip { addr, bits });
    }
    let mut labels: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for tok in tokens(t)? {
        let (k, v) = tok
            .split_once('=')
            .ok_or_else(|| format!("{tok:?}: write key=value, wl:, ip:, ipl: or any"))?;
        let k = k.trim().replace('\u{1}', "|");
        if k.is_empty() {
            return Err(format!("{tok:?}: missing label key"));
        }
        let values: Vec<String> = v.split('|').map(|x| x.replace('\u{1}', "|")).collect();
        if values.iter().any(|x| x.is_empty()) {
            return Err(format!("{k}: empty label value"));
        }
        if labels.contains_key(&k) {
            return Err(format!("{k}: key given twice (write {k}=a|b for either)"));
        }
        labels.insert(k, values);
    }
    if labels.is_empty() {
        return Err("empty side".into());
    }
    Ok(Side::Labels(labels))
}

fn endpoint_ip(ep: &ResolvedEndpoint) -> Option<IpAddr> {
    ep.ip.as_deref().and_then(|s| s.parse().ok())
}

/// What matching needs of one flow, computed once for all marks: the
/// protocol number (without allocating, as `resolve_flows` writes it) and
/// the parsed addresses.
struct Probe<'f> {
    f: &'f ResolvedFlow,
    proto: Option<u8>,
    src_ip: Option<IpAddr>,
    dst_ip: Option<IpAddr>,
}

impl<'f> Probe<'f> {
    fn new(f: &'f ResolvedFlow) -> Self {
        let proto = match f.proto.as_str() {
            "tcp" => Some(6),
            "udp" => Some(17),
            "icmp" => Some(1),
            "icmpv6" => Some(58),
            n => n.parse().ok(),
        };
        Probe {
            f,
            proto,
            src_ip: endpoint_ip(&f.src),
            dst_ip: endpoint_ip(&f.dst),
        }
    }
}

/// A mark's service: a protocol (any port), or a port or port range of
/// TCP or UDP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceSpec {
    Proto(u8),
    Ports { proto: u8, from: u16, to: u16 },
}

impl ServiceSpec {
    /// `5666/tcp`, `8000-8010/tcp`, `udp` (any port), or a protocol name
    /// or number `proto_number` knows (`icmp`, `47`).
    pub fn parse(s: &str) -> Result<Self, String> {
        let t = s.trim();
        let Some((ports, proto_s)) = t.split_once('/') else {
            let proto = proto_number(t).ok_or_else(|| format!("{t:?}: unknown protocol"))?;
            return Ok(ServiceSpec::Proto(proto));
        };
        let proto = proto_number(proto_s)
            .ok_or_else(|| format!("{t:?}: unknown protocol {:?}", proto_s.trim()))?;
        if !matches!(proto, 6 | 17) {
            return Err(format!(
                "{t:?}: {} has no ports",
                proto_name(i64::from(proto))
            ));
        }
        let port = |p: &str| {
            p.trim()
                .parse::<u16>()
                .map_err(|_| format!("{t:?}: {:?} is not a port (0-65535)", p.trim()))
        };
        let (from, to) = match ports.split_once('-') {
            Some((a, b)) => (port(a)?, port(b)?),
            None => {
                let p = port(ports)?;
                (p, p)
            }
        };
        if from > to {
            return Err(format!("{t:?}: the range ends before it starts"));
        }
        Ok(ServiceSpec::Ports { proto, from, to })
    }

    /// The text `parse` reads back.
    pub fn text(&self) -> String {
        match *self {
            ServiceSpec::Proto(p) => proto_name(i64::from(p)),
            ServiceSpec::Ports { proto, from, to } if from == to => {
                format!("{from}/{}", proto_name(i64::from(proto)))
            }
            ServiceSpec::Ports { proto, from, to } => {
                format!("{from}-{to}/{}", proto_name(i64::from(proto)))
            }
        }
    }

    pub fn matches(&self, proto: Option<u8>, port: Option<u16>) -> bool {
        match *self {
            ServiceSpec::Proto(p) => proto == Some(p),
            ServiceSpec::Ports { proto: p, from, to } => {
                proto == Some(p) && port.is_some_and(|x| (from..=to).contains(&x))
            }
        }
    }

    /// One port 3, a range 2, a protocol 1.
    pub fn specificity(&self) -> u32 {
        match *self {
            ServiceSpec::Proto(_) => 1,
            ServiceSpec::Ports { from, to, .. } if from == to => 3,
            ServiceSpec::Ports { .. } => 2,
        }
    }
}

/// `allowed`, `potentially_blocked` or `blocked`.
pub fn parse_decision(s: &str) -> Result<PolicyDecision, String> {
    PolicyDecision::parse(s).ok_or_else(|| {
        format!(
            "unknown decision {:?}: allowed potentially_blocked blocked",
            s.trim()
        )
    })
}

/// The PCE a mark is limited to; `org` only with a host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PceScope {
    pub host: String,
    pub org: Option<u64>,
}

/// The PCE of the profile in use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PceId {
    pub host: String,
    pub org: u64,
}

impl PceScope {
    pub fn covers(&self, pce: &PceId) -> bool {
        self.host.eq_ignore_ascii_case(&pce.host) && self.org.is_none_or(|o| o == pce.org)
    }
}

/// One `[[mark]]`, validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    pub id: String,
    pub action: Action,
    pub src: Side,
    pub dst: Side,
    /// Empty: any service.
    pub services: Vec<ServiceSpec>,
    /// Matched against the reported decision; empty: any.
    pub decisions: Vec<PolicyDecision>,
    /// Matched against the draft decision (its class); empty: any. A mark
    /// with one is active only while draft decisions are loaded.
    pub draft_decisions: Vec<PolicyDecision>,
    pub scope: Option<PceScope>,
    pub expires: Option<Timestamp>,
    /// Why the flows were marked; empty only for a hand-written mark
    /// without one (it loads with a warning).
    pub comment: String,
    pub created: Option<Timestamp>,
    pub created_on: Option<String>,
}

impl Mark {
    /// Directional: `src` against the flow's source, `dst` against its
    /// destination. Every field given must match.
    pub fn matches(&self, f: &ResolvedFlow) -> bool {
        self.matches_probe(&Probe::new(f))
    }

    /// `matches` with the cheap fields first and the flow's protocol and
    /// addresses computed once.
    fn matches_probe(&self, p: &Probe) -> bool {
        let f = p.f;
        (self.decisions.is_empty() || self.decisions.contains(&f.policy_decision))
            && (self.draft_decisions.is_empty()
                || f.draft_policy_decision
                    .as_ref()
                    .is_some_and(|d| self.draft_decisions.contains(&d.class())))
            && (self.services.is_empty()
                || self.services.iter().any(|s| s.matches(p.proto, f.port)))
            && self.src.matches_at(&f.src, p.src_ip)
            && self.dst.matches_at(&f.dst, p.dst_ip)
    }

    /// Sides, the most specific service, 1 each for `decision`,
    /// `draft_decision` and `pce`: which of several marks of one action is
    /// reported for a flow.
    pub fn specificity(&self) -> u32 {
        self.src.specificity()
            + self.dst.specificity()
            + self
                .services
                .iter()
                .map(ServiceSpec::specificity)
                .max()
                .unwrap_or(0)
            + u32::from(!self.decisions.is_empty())
            + u32::from(!self.draft_decisions.is_empty())
            + u32::from(self.scope.is_some())
    }

    /// `now == expires` is expired.
    pub fn is_expired(&self, now: Timestamp) -> bool {
        self.expires.is_some_and(|e| now >= e)
    }

    /// Portable marks are in scope everywhere; a scoped one on its PCE.
    /// Without a PCE (`marks list` with no profile) scope is not judged.
    pub fn in_scope(&self, pce: Option<&PceId>) -> bool {
        match (&self.scope, pce) {
            (None, _) | (_, None) => true,
            (Some(s), Some(p)) => s.covers(p),
        }
    }

    /// `5666/tcp`, `443/tcp, udp`; `None` for any service.
    pub fn service_text(&self) -> Option<String> {
        (!self.services.is_empty()).then(|| {
            self.services
                .iter()
                .map(ServiceSpec::text)
                .collect::<Vec<_>>()
                .join(", ")
        })
    }

    /// `app=crm → app=shop env=prod 443/tcp`.
    pub fn match_text(&self) -> String {
        let mut s = format!("{} → {}", side_text(&self.src), side_text(&self.dst));
        if let Some(svc) = self.service_text() {
            s.push(' ');
            s.push_str(&svc);
        }
        let ds = |d: &[PolicyDecision]| d.iter().map(|d| d.as_str()).collect::<Vec<_>>().join("|");
        if !self.decisions.is_empty() {
            s.push_str(&format!(" {}", ds(&self.decisions)));
        }
        if !self.draft_decisions.is_empty() {
            s.push_str(&format!(" draft {}", ds(&self.draft_decisions)));
        }
        s
    }

    /// Everything that identifies the mark, without its id and creation:
    /// what new and derived ids hash.
    pub fn canonical(&self) -> String {
        let ds = |d: &[PolicyDecision]| d.iter().map(|d| d.as_str()).collect::<Vec<_>>().join("|");
        format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
            self.action.as_str(),
            side_text(&self.src),
            side_text(&self.dst),
            self.service_text().unwrap_or_default(),
            ds(&self.decisions),
            ds(&self.draft_decisions),
            self.scope
                .as_ref()
                .map(|s| format!("{}/{}", s.host.to_ascii_lowercase(), s.org.unwrap_or(0)))
                .unwrap_or_default(),
            self.expires.map(|e| e.to_string()).unwrap_or_default(),
            self.comment
        )
    }

    /// The rules a mark must keep beyond its fields' own syntax.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let workload = |s: &Side| matches!(s, Side::Workload(_));
        if self.scope.is_none() && (workload(&self.src) || workload(&self.dst)) {
            out.push("workload needs pce (workload marks are PCE-scoped)".into());
        }
        if self.action == Action::AcceptBlocked {
            if self.decisions.is_empty() {
                out.push("accept-blocked needs decision: blocked or potentially_blocked".into());
            } else if self.decisions.contains(&PolicyDecision::Allowed) {
                out.push("accept-blocked cannot include allowed".into());
            }
        }
        out
    }
}

/// The marks of one file, validated, in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarkSet {
    pub marks: Vec<Mark>,
}

/// The winning active mark of one flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub action: Action,
    /// Index into `MarkSet::marks`.
    pub mark: usize,
}

/// The marks in force for one result: in scope, unexpired, and (for a
/// `draft_decision` mark) with draft decisions loaded. Grouped by action,
/// strongest first, each group in report order (specificity, then the
/// earlier `created`, then the lower id).
#[derive(Debug, Clone)]
pub struct ActiveMarks<'a> {
    set: &'a MarkSet,
    groups: [Vec<usize>; 6],
    waiting: usize,
}

impl MarkSet {
    /// The marks active on `pce` at `now`; `draft`: draft decisions are
    /// loaded for the result they are matched against.
    pub fn active(&self, pce: Option<&PceId>, now: Timestamp, draft: bool) -> ActiveMarks<'_> {
        let mut groups: [Vec<usize>; 6] = Default::default();
        let mut waiting = 0;
        for (i, m) in self.marks.iter().enumerate() {
            if m.is_expired(now) || !m.in_scope(pce) {
                continue;
            }
            if !draft && !m.draft_decisions.is_empty() {
                waiting += 1;
                continue;
            }
            groups[m.action.index()].push(i);
        }
        for g in &mut groups {
            g.sort_by(|&a, &b| {
                let (a, b) = (&self.marks[a], &self.marks[b]);
                b.specificity()
                    .cmp(&a.specificity())
                    .then_with(|| a.created.is_none().cmp(&b.created.is_none()))
                    .then_with(|| a.created.cmp(&b.created))
                    .then_with(|| a.id.cmp(&b.id))
            });
        }
        ActiveMarks {
            set: self,
            groups,
            waiting,
        }
    }

    pub fn find(&self, id: &str) -> Option<usize> {
        self.marks.iter().position(|m| m.id == id)
    }
}

impl ActiveMarks<'_> {
    pub fn is_empty(&self) -> bool {
        self.groups.iter().all(Vec::is_empty)
    }

    /// How many marks are active.
    pub fn len(&self) -> usize {
        self.groups.iter().map(Vec::len).sum()
    }

    /// Marks with `draft_decision` left out because no draft decisions
    /// are loaded: `N marks wait for the draft decision (D)`.
    pub fn waiting_for_draft(&self) -> usize {
        self.waiting
    }

    pub fn set(&self) -> &MarkSet {
        self.set
    }

    pub fn mark(&self, i: usize) -> &Mark {
        &self.set.marks[i]
    }

    pub fn is_active(&self, i: usize) -> bool {
        self.set
            .marks
            .get(i)
            .is_some_and(|m| self.groups[m.action.index()].contains(&i))
    }

    /// Whether some active mark has `action`.
    pub fn has(&self, action: Action) -> bool {
        !self.groups[action.index()].is_empty()
    }

    /// The position of mark `i` in its action's report order (lower wins).
    pub fn rank(&self, i: usize) -> usize {
        let a = self.set.marks[i].action.index();
        self.groups[a]
            .iter()
            .position(|&x| x == i)
            .unwrap_or(usize::MAX)
    }

    /// The strongest action among the active marks that match, and its
    /// most specific mark. Stops at the first action with a match.
    pub fn classify(&self, f: &ResolvedFlow) -> Option<Hit> {
        let p = Probe::new(f);
        for (a, group) in Action::ALL.into_iter().zip(&self.groups) {
            if let Some(&i) = group.iter().find(|&&i| self.set.marks[i].matches_probe(&p)) {
                return Some(Hit { action: a, mark: i });
            }
        }
        None
    }

    /// Every active mark that matches, the winner first.
    pub fn matching(&self, f: &ResolvedFlow) -> Vec<usize> {
        let p = Probe::new(f);
        self.groups
            .iter()
            .flatten()
            .copied()
            .filter(|&i| self.set.marks[i].matches_probe(&p))
            .collect()
    }
}

/// FNV-1a, 64 bits.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Crockford's base32, lower case.
const CROCKFORD: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// The low 30 bits of `h` as 6 base32 characters.
fn base32_6(h: u64) -> String {
    (0..6)
        .rev()
        .map(|i| CROCKFORD[((h >> (i * 5)) & 31) as usize] as char)
        .collect()
}

/// A new id, `m-` and 6 characters, from the creation time, the process
/// and the mark's canonical text; hashed again with a counter while
/// `taken` says the id is in the file.
pub fn new_id(now_ns: i128, pid: u32, canonical: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut counter: u32 = 0;
    loop {
        let mut bytes = Vec::with_capacity(canonical.len() + 24);
        bytes.extend_from_slice(&now_ns.to_le_bytes());
        bytes.extend_from_slice(&pid.to_le_bytes());
        bytes.extend_from_slice(canonical.as_bytes());
        if counter > 0 {
            bytes.extend_from_slice(&counter.to_le_bytes());
        }
        let id = format!("m-{}", base32_6(fnv1a(&bytes)));
        if !taken(&id) {
            return id;
        }
        counter += 1;
    }
}

/// The id of a hand-written mark without one: `h-` and 6 characters of
/// the hash of its canonical text, the same on every load and machine.
pub fn derived_id(canonical: &str) -> String {
    format!("h-{}", base32_6(fnv1a(canonical.as_bytes())))
}

```

In `crates/pce/src/write/specs.rs`, replace:

```rust
}

/// `addr/N` → (addr, N) with N checked against the address family.
fn parse_cidr(s: &str) -> Result<(IpAddr, u8), String> {
    let (a, n) = s
        .split_once('/')
```

with:

```rust
}

/// `addr/N` → (addr, N) with N checked against the address family. Shared
/// with `marks` (its `ip:` sides).
pub(crate) fn parse_cidr(s: &str) -> Result<(IpAddr, u8), String> {
    let (a, n) = s
        .split_once('/')
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p pce --lib marks`
Expected: PASS (16 tests).

- [ ] **Step 5: Run the gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: no formatting change, no warning, every test passes.

- [ ] **Step 6: Commit**

```bash
git add crates/pce/src/lib.rs crates/pce/src/marks/mod.rs crates/pce/src/write/specs.rs
git commit -m "feat(marks): pce::marks: actions, sides, services, matching, precedence, ids

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

### Task 2: The partition and the row annotations; the speed budget

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `crates/illumio-tui/src/cli/traffic.rs`
- Modify: `crates/pce/src/app.rs`
- Modify: `crates/pce/src/marks/mod.rs`
- Create: `crates/pce/src/marks/partition.rs`
- Modify: `crates/pce/src/traffic/aggregate.rs`
- Modify: `crates/pce/src/traffic/mesh.rs`
- Modify: `crates/pce/src/traffic/sections.rs`
- Modify: `crates/pce/src/traffic/treemap.rs`
- Test: `crates/pce/tests/marks_perf.rs`

**Interfaces:**
- Consumes: Task 1; the aggregators unchanged (`group_by_labels`, `group_by_service`, `mesh`, `app_sections`, `app_sections_both`, `app::app_peers`).
- Produces (`pce::marks`, re-exported from `marks::partition`):

```rust
pub struct MarkedFlows { pub mark: usize, pub action: Action, pub flows: Vec<ResolvedFlow> }
pub struct Marked { pub subsets: Vec<MarkedFlows>, pub keep_squelched: bool }
impl Marked { fn squelched(&self) -> impl Iterator<Item = &ResolvedFlow>; fn squelched_where(&self, pred) -> u64; fn count(&self, Action) -> u64 }
pub struct Partition<'a> { pub kept: Cow<'a, [ResolvedFlow]>, pub kept_hits: Vec<Option<Hit>>, pub hits: usize, pub marked: Marked }
pub fn partition<'a>(flows: &'a [ResolvedFlow], m: &ActiveMarks, keep_squelched: bool) -> Partition<'a>;
pub struct TopMark { pub mark: usize, pub id: String }
pub struct RowMarks { pub flows: u64, pub by_action: [u64; 6], pub top: [Option<TopMark>; 6], pub hidden: u64 }
impl RowMarks { fn count(Action) -> u64; fn top_id(Action) -> Option<&str>; fn marker(&self) -> Option<Marker> }  // + flattening Serialize
pub enum Marker { Critical, Important, Revisit, Accepted, Reviewed, Squelched, PartlySquelched(u64) }
impl Marker { fn action(self) -> Option<Action>; fn glyph(self) -> &'static str /* ● ● ◆ ≈ ✓ ~ ~ */; fn of(Action) -> Marker }
pub fn annotate_groups(rows: &mut [FlowGroup], p: &Marked, m: &ActiveMarks, agg: impl Fn(&[ResolvedFlow]) -> Vec<FlowGroup>) -> u64;
pub fn annotate_mesh(rows: &mut [MeshRow], p: &Marked, m: &ActiveMarks, agg: impl Fn(&[ResolvedFlow]) -> Vec<MeshRow>) -> u64;
pub fn annotate_sections(s: &mut [AppSection], p: &Marked, m: &ActiveMarks, agg: impl Fn(&[ResolvedFlow]) -> Vec<AppSection>) -> u64;
pub fn annotate_peers(rows: &mut [AppPeer], p: &Marked, m: &ActiveMarks, agg: impl Fn(&[ResolvedFlow]) -> Vec<AppPeer>) -> u64;
// each returns the rows squelch removed (with squelched rows shown: the rows all of whose records are squelched)
```

`FlowGroup`, `MeshRow`, `PeerRow` and `AppPeer` gain `#[serde(flatten)] pub marks: Option<RowMarks>` (every constructor sets `None`).

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
            conns: Default::default(),
            draft: None,
        }
    }
```

with:

```rust
            conns: Default::default(),
            draft: None,
            marks: None,
        }
    }
```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
            blocked: 0,
            draft: None,
        };
        assert_eq!(
```

with:

```rust
            blocked: 0,
            draft: None,
            marks: None,
        };
        assert_eq!(
```

In `crates/pce/src/marks/mod.rs`, replace:

```rust
use std::collections::BTreeMap;
use std::net::IpAddr;

/// What a mark says about its flows. `Ord` is precedence: the strongest
```

with:

```rust
use std::collections::BTreeMap;
use std::net::IpAddr;
mod partition;

/// What a mark says about its flows. `Ord` is precedence: the strongest
```

Create `crates/pce/src/marks/partition.rs` with its tests only (Step 3 adds the code above them):

```rust
#[cfg(test)]
mod tests {
    use super::super::tests::mark;
    use super::super::{MarkSet, ServiceSpec, parse_decision};
    use super::*;
    use crate::app::app_peers;
    use crate::models::PolicyDecision;
    use crate::traffic::resolved::tests::rich_flows;
    use crate::traffic::{
        Decisions, Direction, Focus, MeshColumns, app_sections, app_sections_both, group_by_labels,
        group_by_service, mesh,
    };
    use jiff::Timestamp;

    fn now() -> Timestamp {
        "2026-10-17T12:00:00Z".parse().unwrap()
    }

    fn keys() -> Vec<String> {
        vec!["app".into(), "env".into()]
    }

    /// Two squelches (the Corp list; shop's eu web tier to its databases
    /// on 5432), a critical on blocked flows into shop/prod's databases
    /// (it takes two flows from the squelches), erp/prod reviewed inside
    /// itself, erp/dev accepted as blocked, hr to revisit.
    fn set() -> MarkSet {
        let mut sq = mark("m-sq2", Action::Squelch, "role=web loc=eu", "role=db");
        sq.services = vec![ServiceSpec::parse("5432/tcp").unwrap()];
        let mut crit = mark(
            "m-crit",
            Action::Critical,
            "any",
            "app=shop env=prod role=db",
        );
        crit.decisions = vec![PolicyDecision::Blocked];
        let mut acc = mark("m-acc", Action::AcceptBlocked, "app=erp env=dev", "any");
        acc.decisions = vec![
            parse_decision("blocked").unwrap(),
            parse_decision("potentially_blocked").unwrap(),
        ];
        MarkSet {
            marks: vec![
                mark("m-sq1", Action::Squelch, "ipl:Corp", "any"),
                sq,
                crit,
                mark(
                    "m-rev",
                    Action::Reviewed,
                    "app=erp env=prod",
                    "app=erp env=prod",
                ),
                acc,
                mark("m-hr", Action::Revisit, "app=hr", "any"),
            ],
        }
    }

    fn find<'a>(rows: &'a [FlowGroup], src: &str, dst: &str) -> Option<&'a FlowGroup> {
        rows.iter()
            .find(|g| g.src.join("/") == src && g.dst.join("/") == dst)
    }

    #[test]
    fn partition_copies_only_what_matches() {
        let flows = rich_flows();
        let empty = MarkSet::default();
        let p = partition(&flows, &empty.active(None, now(), false), false);
        assert!(matches!(p.kept, Cow::Borrowed(_)));
        assert!(p.marked.subsets.is_empty() && p.hits == 0);
        let s = set();
        let a = s.active(None, now(), false);
        let hidden = partition(&flows, &a, false);
        assert!(matches!(hidden.kept, Cow::Owned(_)));
        assert_eq!(hidden.marked.count(Action::Squelch), 9);
        assert_eq!(hidden.kept.len(), 30 - 9);
        assert_eq!(hidden.kept_hits.len(), hidden.kept.len());
        assert_eq!(hidden.hits, 9 + 2 + 2 + 2 + 4);
        let shown = partition(&flows, &a, true);
        assert!(matches!(shown.kept, Cow::Borrowed(_)));
        assert_eq!(shown.kept.len(), 30);
        assert_eq!(shown.marked.squelched().count(), 9);
        for (f, h) in shown.kept.iter().zip(&shown.kept_hits) {
            assert_eq!(*h, a.classify(f));
        }
    }

    #[test]
    fn a_row_is_hidden_iff_all_its_flows_are_squelched_and_partial_rows_keep_visible_counts() {
        let flows = rich_flows();
        let s = set();
        let a = s.active(None, now(), false);
        let p = partition(&flows, &a, false);
        let agg = |f: &[ResolvedFlow]| group_by_labels(f, &keys());
        let mut rows = agg(&p.kept);
        let gone = annotate_groups(&mut rows, &p.marked, &a, agg);
        // Corp to crm, hr and erp, and shop/prod inside itself.
        assert_eq!(gone, 4);
        for (src, dst) in [
            ("ipl:Corp/-", "crm/prod"),
            ("ipl:Corp/-", "hr/prod"),
            ("ipl:Corp/-", "erp/prod"),
            ("shop/prod", "shop/prod"),
        ] {
            assert!(find(&rows, src, dst).is_none(), "{src} → {dst}");
        }
        // Corp to shop/prod keeps its critical flow (21) and hides 17.
        let corp = find(&rows, "ipl:Corp/-", "shop/prod").unwrap();
        assert_eq!(corp.flows, 1);
        let rm = corp.marks.as_ref().unwrap();
        assert_eq!((rm.flows, rm.hidden), (1, 1));
        assert_eq!(rm.marker(), Some(Marker::Critical));
        assert_eq!(rm.top_id(Action::Critical), Some("m-crit"));
        // shop/dev inside itself: 5432 squelched and hidden, 22 shown.
        let dev = find(&rows, "shop/dev", "shop/dev").unwrap();
        assert_eq!(dev.flows, 1);
        assert_eq!(
            dev.marks.as_ref().unwrap().marker(),
            Some(Marker::PartlySquelched(1))
        );
        // With squelched rows shown, nothing disappears and the counts are
        // the full ones; the squelched-only rows say so.
        let shown = partition(&flows, &a, true);
        let mut all = agg(&shown.kept);
        let gone = annotate_groups(&mut all, &shown.marked, &a, agg);
        assert_eq!(gone, 4);
        assert_eq!(all.len(), rows.len() + 4);
        let full = find(&all, "shop/prod", "shop/prod").unwrap();
        assert_eq!(
            full.marks.as_ref().unwrap().marker(),
            Some(Marker::Squelched)
        );
        assert_eq!(full.marks.as_ref().unwrap().count(Action::Squelch), 4);
    }

    #[test]
    fn highlights_show_if_any_flow_has_one_and_quiet_rows_need_every_flow() {
        let flows = rich_flows();
        let s = set();
        let a = s.active(None, now(), false);
        let p = partition(&flows, &a, false);
        let agg = |f: &[ResolvedFlow]| group_by_labels(f, &keys());
        let mut rows = agg(&p.kept);
        annotate_groups(&mut rows, &p.marked, &a, agg);
        let marker = |src: &str, dst: &str| {
            find(&rows, src, dst)
                .and_then(|g| g.marks.as_ref())
                .and_then(RowMarks::marker)
        };
        assert_eq!(marker("erp/prod", "erp/prod"), Some(Marker::Reviewed));
        assert_eq!(marker("erp/dev", "erp/prod"), Some(Marker::Accepted));
        assert_eq!(marker("hr/prod", "shop/prod"), Some(Marker::Revisit));
        assert_eq!(marker("shop/dev", "shop/prod"), Some(Marker::Critical));
        assert_eq!(marker("crm/prod", "crm/prod"), None);
        // Quiet needs all: erp/prod's flows by service, plus one unmarked.
        let rm = |flows: u64, by: &[(Action, u64)]| {
            let mut r = RowMarks {
                flows,
                ..RowMarks::default()
            };
            for (act, n) in by {
                r.by_action[act.index()] = *n;
            }
            r
        };
        assert_eq!(rm(3, &[(Action::Reviewed, 2)]).marker(), None);
        assert_eq!(
            rm(3, &[(Action::Reviewed, 2), (Action::AcceptBlocked, 1)]).marker(),
            Some(Marker::Reviewed)
        );
        assert_eq!(
            rm(2, &[(Action::AcceptBlocked, 1), (Action::Squelch, 1)]).marker(),
            Some(Marker::Accepted)
        );
        assert_eq!(
            rm(5, &[(Action::Important, 1), (Action::Revisit, 1)]).marker(),
            Some(Marker::Important)
        );
        let mut partly = rm(4, &[]);
        partly.hidden = 12;
        assert_eq!(partly.marker(), Some(Marker::PartlySquelched(12)));
    }

    /// Every view: the shown rows' records plus the squelched records of
    /// the view's input are its input, and the rows reported gone are
    /// exactly the rows the unmarked aggregation has and the marked one
    /// lacks.
    #[test]
    fn visible_plus_squelched_is_the_input_in_every_view() {
        let flows = rich_flows();
        let s = set();
        let a = s.active(None, now(), false);
        let p = partition(&flows, &a, false);
        let k = keys();
        let groups = |rows: &[FlowGroup]| -> u64 { rows.iter().map(|g| g.flows).sum() };
        for decisions in [Decisions::All, Decisions::Blocked] {
            let input = flows.iter().filter(|f| decisions.keeps(f)).count() as u64;
            let sq = p.marked.squelched_where(|f| decisions.keeps(f));
            type Agg<'k> = Box<dyn Fn(&[ResolvedFlow]) -> Vec<FlowGroup> + 'k>;
            let aggs: [Agg; 2] = [
                Box::new(|f: &[ResolvedFlow]| group_by_labels(&decisions.kept(f), &k)),
                Box::new(|f: &[ResolvedFlow]| group_by_service(&decisions.kept(f), &k)),
            ];
            for agg in aggs {
                let mut rows = agg(&p.kept);
                let gone = annotate_groups(&mut rows, &p.marked, &a, &agg);
                assert_eq!(groups(&rows) + sq, input);
                assert_eq!(gone as usize, agg(&flows).len() - rows.len());
            }
            for d in [Direction::In, Direction::Out] {
                let agg = |f: &[ResolvedFlow]| app_sections(f, &k, d, decisions);
                let mut secs = agg(&p.kept);
                let gone = annotate_sections(&mut secs, &p.marked, &a, agg);
                let shown: u64 = secs.iter().map(|s| s.totals.flows).sum();
                assert_eq!(shown + sq, input, "{d:?} {decisions:?}");
                let rows = |s: &[AppSection]| s.iter().map(|s| s.rows.len()).sum::<usize>();
                assert_eq!(gone as usize, rows(&agg(&flows)) - rows(&secs));
            }
            // Both: each direction holds every record once; the squelched
            // records are counted once, not per section.
            let agg = |f: &[ResolvedFlow]| app_sections_both(f, &k, decisions);
            let mut secs = agg(&p.kept);
            annotate_sections(&mut secs, &p.marked, &a, agg);
            for d in [Direction::In, Direction::Out] {
                let shown: u64 = secs
                    .iter()
                    .filter(|s| s.direction == d)
                    .map(|s| s.totals.flows)
                    .sum();
                assert_eq!(shown + sq, input);
            }
        }
        let cols = MeshColumns {
            user: true,
            process: true,
        };
        let agg = |f: &[ResolvedFlow]| mesh(f, &k, cols);
        let mut rows = agg(&p.kept);
        let gone = annotate_mesh(&mut rows, &p.marked, &a, agg);
        assert_eq!(rows.iter().map(|r| r.flows).sum::<u64>() + 9, 30);
        assert_eq!(gone as usize, agg(&flows).len() - rows.len());
        let focus = Focus::new(vec!["shop".into(), "prod".into()], k.clone());
        for side in [Direction::In, Direction::Out] {
            let agg = |f: &[ResolvedFlow]| app_peers(f, &focus, side);
            let mut rows = agg(&p.kept);
            let gone = annotate_peers(&mut rows, &p.marked, &a, agg);
            let input: u64 = agg(&flows).iter().map(|r| r.flows).sum();
            let on_side = |f: &ResolvedFlow| {
                let ep = if side == Direction::In {
                    &f.dst
                } else {
                    &f.src
                };
                crate::traffic::group_key(ep, &k) == focus.app
            };
            assert_eq!(
                rows.iter().map(|r| r.flows).sum::<u64>() + p.marked.squelched_where(on_side),
                input,
                "{side:?}"
            );
            assert_eq!(gone as usize, agg(&flows).len() - rows.len());
        }
    }

    #[test]
    fn blocked_sections_count_accepted_records_per_row() {
        let flows = rich_flows();
        let s = set();
        let a = s.active(None, now(), false);
        let p = partition(&flows, &a, false);
        let agg = |f: &[ResolvedFlow]| app_sections(f, &keys(), Direction::In, Decisions::Blocked);
        let mut secs = agg(&p.kept);
        annotate_sections(&mut secs, &p.marked, &a, agg);
        let erp = secs.iter().find(|s| s.app_text() == "erp/prod").unwrap();
        let accepted: u64 = erp
            .rows
            .iter()
            .filter_map(|r| r.marks.as_ref())
            .map(|m| m.count(Action::AcceptBlocked))
            .sum();
        assert_eq!(accepted, 2, "erp/dev's 8443 and 1521");
        assert!(
            erp.rows
                .iter()
                .filter(|r| r.peer_text() == "erp/dev")
                .all(|r| r.marks.as_ref().and_then(RowMarks::marker) == Some(Marker::Accepted))
        );
    }

    #[test]
    fn row_marks_flatten_into_the_row_json() {
        let flows = rich_flows();
        let s = set();
        let a = s.active(None, now(), false);
        let p = partition(&flows, &a, false);
        let agg = |f: &[ResolvedFlow]| group_by_labels(f, &keys());
        let mut rows = agg(&p.kept);
        annotate_groups(&mut rows, &p.marked, &a, agg);
        let corp = serde_json::to_value(find(&rows, "ipl:Corp/-", "shop/prod").unwrap()).unwrap();
        assert_eq!(
            corp["mark"],
            serde_json::json!({"action": "critical", "id": "m-crit", "flows": 1})
        );
        assert_eq!(corp["marks"], serde_json::json!({"critical": 1}));
        assert_eq!(corp["squelched"], 1);
        let plain = serde_json::to_value(find(&rows, "crm/prod", "crm/prod").unwrap()).unwrap();
        assert!(plain.get("mark").is_none() && plain.get("marks").is_none());
        let dev = serde_json::to_value(find(&rows, "shop/dev", "shop/dev").unwrap()).unwrap();
        assert!(dev.get("mark").is_none(), "partly squelched: no action");
        assert_eq!(dev["squelched"], 1);
    }
}
```

In `crates/pce/src/traffic/treemap.rs`, replace:

```rust
            conns: Default::default(),
            draft: None,
        }
    }
```

with:

```rust
            conns: Default::default(),
            draft: None,
            marks: None,
        }
    }
```

Create `crates/pce/tests/marks_perf.rs`:

```rust
//! The marks' speed budget: 200 000 flows against 200 marks, classified
//! and partitioned, in under 300 ms. Release only (`#[ignore]`); CI runs
//! it with `cargo test --release -p pce --test marks_perf -- --ignored`.
use pce::marks::{Action, Mark, MarkSet, ServiceSpec, parse_side, partition};
use pce::models::PolicyDecision;
use pce::resolve::ResolvedLabel;
use pce::traffic::{ResolvedEndpoint, ResolvedFlow};
use std::time::Instant;

fn endpoint(i: usize) -> ResolvedEndpoint {
    let label = |k: &str, v: String| ResolvedLabel {
        href: format!("/orgs/1/labels/{k}{v}"),
        key: k.into(),
        value: v,
    };
    ResolvedEndpoint {
        ip: Some(format!("10.{}.{}.{}", i % 200, (i / 200) % 250, i % 250)),
        hostname: Some(format!("host-{}", i % 5000)),
        workload_href: Some(format!("/orgs/1/workloads/{}", i % 5000)),
        labels: vec![
            label("app", format!("app{}", i % 50)),
            label("env", ["prod", "dev", "test"][i % 3].into()),
            label("role", format!("role{}", i % 7)),
        ],
        ..Default::default()
    }
}

fn flows(n: usize) -> Vec<ResolvedFlow> {
    (0..n)
        .map(|i| ResolvedFlow {
            src: endpoint(i),
            dst: endpoint(i * 7 + 3),
            port: Some(1000 + (i % 3000) as u16),
            proto: if i % 5 == 0 { "udp" } else { "tcp" }.into(),
            process_name: None,
            user_name: None,
            num_connections: 1,
            policy_decision: PolicyDecision::ALL[i % 3],
            draft_policy_decision: None,
            flow_direction: None,
            first_detected: None,
            last_detected: None,
            rules: Vec::new(),
            bytes_in: None,
            bytes_out: None,
        })
        .collect()
}

/// 200 marks of every side kind, most with a service, a few broad ones.
fn marks() -> MarkSet {
    let marks = (0..200)
        .map(|i| {
            let (src, dst) = match i % 5 {
                0 => (format!("app=app{} env=prod", i % 50), "any".to_string()),
                1 => (
                    "any".to_string(),
                    format!("app=app{} role=role{}", i % 50, i % 7),
                ),
                2 => (format!("ip:10.{}.0.0/16", i % 200), "any".to_string()),
                3 => ("ipl:corp".to_string(), format!("app=app{}", i % 50)),
                _ => (
                    format!("app=app{}", i % 50),
                    format!("app=app{}", (i + 1) % 50),
                ),
            };
            Mark {
                id: format!("m-{i:06}"),
                action: Action::ALL[i % 6],
                src: parse_side(&src).unwrap(),
                dst: parse_side(&dst).unwrap(),
                services: if i % 4 == 0 {
                    Vec::new()
                } else {
                    vec![ServiceSpec::parse(&format!("{}/tcp", 1000 + i * 13)).unwrap()]
                },
                decisions: if i % 6 == 3 {
                    vec![PolicyDecision::Blocked]
                } else {
                    Vec::new()
                },
                draft_decisions: Vec::new(),
                scope: None,
                expires: None,
                comment: "perf".into(),
                created: None,
                created_on: None,
            }
        })
        .collect();
    MarkSet { marks }
}

#[test]
#[ignore = "release only: cargo test --release -p pce --test marks_perf -- --ignored"]
fn two_hundred_thousand_flows_against_two_hundred_marks_in_under_300_ms() {
    let flows = flows(200_000);
    let set = marks();
    let now = "2026-10-17T12:00:00Z".parse().unwrap();
    let active = set.active(None, now, false);
    assert_eq!(active.len(), 200);
    let start = Instant::now();
    let p = partition(&flows, &active, false);
    let took = start.elapsed();
    assert!(p.hits > 0, "the marks match something");
    assert!(
        took.as_millis() < 300,
        "200k flows x 200 marks took {took:?}"
    );
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p pce --lib marks::partition`
Expected: FAIL: it does not compile (`cannot find function partition`, `no field marks on FlowGroup`).

- [ ] **Step 3: Implement**

In `.github/workflows/ci.yml`, replace:

```yaml
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace
  deny:
    runs-on: ubuntu-latest
```

with:

```yaml
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace
      # The marks speed budget (200k flows x 200 marks under 300 ms), release only.
      - run: cargo test --release -p pce --test marks_perf -- --ignored
  deny:
    runs-on: ubuntu-latest
```

In `crates/pce/src/app.rs`, replace:

```rust
    #[serde(flatten)]
    pub draft: Option<crate::traffic::DraftCounts>,
}

```

with:

```rust
    #[serde(flatten)]
    pub draft: Option<crate::traffic::DraftCounts>,
    /// What the marks say about the row (`pce::marks::annotate_*`);
    /// flattened into JSON as `mark`, `marks` and `squelched`.
    #[serde(flatten)]
    pub marks: Option<crate::marks::RowMarks>,
}

```

In `crates/pce/src/app.rs`, replace:

```rust
                bytes: g.bytes,
                draft: g.draft,
            });
        }
```

with:

```rust
                bytes: g.bytes,
                draft: g.draft,
                marks: None,
            });
        }
```

In `crates/pce/src/marks/mod.rs`, replace:

```rust
use std::collections::BTreeMap;
use std::net::IpAddr;
mod partition;

/// What a mark says about its flows. `Ord` is precedence: the strongest
```

with:

```rust
use std::collections::BTreeMap;
use std::net::IpAddr;

mod partition;
pub use partition::{
    Marked, MarkedFlows, Marker, Partition, RowMarks, TopMark, annotate_groups, annotate_mesh,
    annotate_peers, annotate_sections, partition,
};

/// What a mark says about its flows. `Ord` is precedence: the strongest
```

In `crates/pce/src/marks/partition.rs`, insert above the tests:

```rust
//! Marks applied to a result: classify every flow, keep the unsquelched
//! ones for the views' own aggregators, and join each mark's subset,
//! aggregated by the same aggregator, to the view's rows by the row's key.
//! No key logic is copied: the aggregators keep their signatures.
use super::{Action, ActiveMarks, Hit};
use crate::app::AppPeer;
use crate::traffic::{AppSection, FlowGroup, MeshRow, ResolvedFlow};
use serde::Serialize;
use serde::ser::SerializeMap;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

/// The flows one mark wins, in input order.
#[derive(Debug, Clone, PartialEq)]
pub struct MarkedFlows {
    /// Index into `MarkSet::marks`.
    pub mark: usize,
    pub action: Action,
    pub flows: Vec<ResolvedFlow>,
}

/// The marked flows of a result, by winning mark: what the annotations
/// join to a view's rows. Owned, so the TUI keeps it between rebuilds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Marked {
    /// One entry per mark that wins a flow, by mark index; together they
    /// partition the marked flows.
    pub subsets: Vec<MarkedFlows>,
    /// Squelched flows are in the views' input (`M`, `--show-squelched`).
    pub keep_squelched: bool,
}

/// A result split by the marks.
#[derive(Debug, Clone)]
pub struct Partition<'a> {
    /// What the views aggregate: every flow with squelched ones shown,
    /// else the unsquelched ones. Borrowed unless a flow is left out.
    pub kept: Cow<'a, [ResolvedFlow]>,
    /// The hit of each flow of `kept`, in its order.
    pub kept_hits: Vec<Option<Hit>>,
    /// Flows with a hit.
    pub hits: usize,
    pub marked: Marked,
}

/// Classifies `flows` against `m`. Copies only what matches: the marked
/// flows into their subsets, and `kept` only when a flow is left out.
pub fn partition<'a>(
    flows: &'a [ResolvedFlow],
    m: &ActiveMarks,
    keep_squelched: bool,
) -> Partition<'a> {
    if m.is_empty() {
        return Partition {
            kept: Cow::Borrowed(flows),
            kept_hits: vec![None; flows.len()],
            hits: 0,
            marked: Marked {
                subsets: Vec::new(),
                keep_squelched,
            },
        };
    }
    let all: Vec<Option<Hit>> = flows.iter().map(|f| m.classify(f)).collect();
    let mut by_mark: BTreeMap<usize, MarkedFlows> = BTreeMap::new();
    for (f, h) in flows.iter().zip(&all) {
        if let Some(h) = h {
            by_mark
                .entry(h.mark)
                .or_insert_with(|| MarkedFlows {
                    mark: h.mark,
                    action: h.action,
                    flows: Vec::new(),
                })
                .flows
                .push(f.clone());
        }
    }
    let squelched = |h: &Option<Hit>| h.is_some_and(|h| h.action == Action::Squelch);
    let hide = !keep_squelched && all.iter().any(squelched);
    let (kept, kept_hits) = if hide {
        let (kept, hits): (Vec<ResolvedFlow>, Vec<Option<Hit>>) = flows
            .iter()
            .zip(&all)
            .filter(|(_, h)| !squelched(h))
            .map(|(f, h)| (f.clone(), *h))
            .unzip();
        (Cow::Owned(kept), hits)
    } else {
        (Cow::Borrowed(flows), all.clone())
    };
    Partition {
        kept,
        kept_hits,
        hits: all.iter().filter(|h| h.is_some()).count(),
        marked: Marked {
            subsets: by_mark.into_values().collect(),
            keep_squelched,
        },
    }
}

impl Marked {
    /// The squelched flows, shown or not.
    pub fn squelched(&self) -> impl Iterator<Item = &ResolvedFlow> {
        self.subsets
            .iter()
            .filter(|s| s.action == Action::Squelch)
            .flat_map(|s| &s.flows)
    }

    /// The squelched flows `pred` keeps (a view's decision filter, a
    /// drill): a view's `squelched.flows`, each record once.
    pub fn squelched_where(&self, pred: impl Fn(&ResolvedFlow) -> bool) -> u64 {
        self.squelched().filter(|f| pred(f)).count() as u64
    }

    /// Flow records whose hit is `action`.
    pub fn count(&self, action: Action) -> u64 {
        self.subsets
            .iter()
            .filter(|s| s.action == action)
            .map(|s| s.flows.len() as u64)
            .sum()
    }
}

/// The strongest mark of one action among a row's flows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TopMark {
    pub mark: usize,
    pub id: String,
}

/// What the marks say about one aggregated row. Flattened into the row's
/// JSON as `mark` (the marker's action, its strongest mark and its flow
/// records), `marks` (records per action) and `squelched` (hidden records).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowMarks {
    /// The row's flow records (its `flows`).
    pub flows: u64,
    /// The row's records per winning action, by `Action::index`.
    pub by_action: [u64; 6],
    /// Per action, the strongest mark among the row's records.
    pub top: [Option<TopMark>; 6],
    /// Squelched records under the row's key left out of its counts.
    pub hidden: u64,
}

/// What a row's gutter shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    Critical,
    Important,
    Revisit,
    /// Every flow accepted as blocked (or squelched, shown).
    Accepted,
    /// Every flow reviewed or accepted (or squelched, shown).
    Reviewed,
    /// Every flow squelched (shown).
    Squelched,
    /// Some records of the row are squelched and hidden: how many.
    PartlySquelched(u64),
}

impl Marker {
    /// The action `mark.action` reports; none for a partly squelched row.
    pub fn action(self) -> Option<Action> {
        match self {
            Marker::Critical => Some(Action::Critical),
            Marker::Important => Some(Action::Important),
            Marker::Revisit => Some(Action::Revisit),
            Marker::Accepted => Some(Action::AcceptBlocked),
            Marker::Reviewed => Some(Action::Reviewed),
            Marker::Squelched => Some(Action::Squelch),
            Marker::PartlySquelched(_) => None,
        }
    }

    /// The gutter glyph: `●` critical and important, `◆` revisit, `≈`
    /// accepted, `✓` reviewed, `~` squelched.
    pub fn glyph(self) -> &'static str {
        match self {
            Marker::Critical | Marker::Important => "●",
            Marker::Revisit => "◆",
            Marker::Accepted => "≈",
            Marker::Reviewed => "✓",
            Marker::Squelched | Marker::PartlySquelched(_) => "~",
        }
    }

    /// A flow's marker from its hit.
    pub fn of(action: Action) -> Marker {
        match action {
            Action::Critical => Marker::Critical,
            Action::Important => Marker::Important,
            Action::Revisit => Marker::Revisit,
            Action::AcceptBlocked => Marker::Accepted,
            Action::Reviewed => Marker::Reviewed,
            Action::Squelch => Marker::Squelched,
        }
    }
}

impl RowMarks {
    pub fn count(&self, a: Action) -> u64 {
        self.by_action[a.index()]
    }

    pub fn top_id(&self, a: Action) -> Option<&str> {
        self.top[a.index()].as_ref().map(|t| t.id.as_str())
    }

    /// Highlights: any (the strongest a flow has). Quiet: all, and no
    /// highlight. A partly squelched row says how many records it hides.
    pub fn marker(&self) -> Option<Marker> {
        for (a, m) in [
            (Action::Critical, Marker::Critical),
            (Action::Important, Marker::Important),
            (Action::Revisit, Marker::Revisit),
        ] {
            if self.count(a) > 0 {
                return Some(m);
            }
        }
        let (acc, rev, sq) = (
            self.count(Action::AcceptBlocked),
            self.count(Action::Reviewed),
            self.count(Action::Squelch),
        );
        if self.flows > 0 && acc + rev + sq == self.flows {
            return Some(if sq == self.flows {
                Marker::Squelched
            } else if rev == 0 {
                Marker::Accepted
            } else {
                Marker::Reviewed
            });
        }
        (self.hidden > 0).then_some(Marker::PartlySquelched(self.hidden))
    }
}

/// `{"critical": 3, "reviewed": 38}`: the non-zero counts, strongest first.
struct ActionCounts<'a>(&'a [u64; 6]);

impl Serialize for ActionCounts<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(None)?;
        for a in Action::ALL {
            let n = self.0[a.index()];
            if n > 0 {
                map.serialize_entry(a.as_str(), &n)?;
            }
        }
        map.end()
    }
}

#[derive(Serialize)]
struct RowMark<'a> {
    action: Action,
    id: Option<&'a str>,
    flows: u64,
}

impl Serialize for RowMarks {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(None)?;
        if let Some(a) = self.marker().and_then(Marker::action) {
            map.serialize_entry(
                "mark",
                &RowMark {
                    action: a,
                    id: self.top_id(a),
                    flows: self.count(a),
                },
            )?;
        }
        if self.by_action.iter().any(|n| *n > 0) {
            map.serialize_entry("marks", &ActionCounts(&self.by_action))?;
        }
        if self.hidden > 0 {
            map.serialize_entry("squelched", &self.hidden)?;
        }
        map.end()
    }
}

/// Joins each subset's aggregation (`subset_rows`: its row keys and flow
/// records) to the view's rows by key. Returns the rows squelch removed:
/// keys of squelched records the view no longer has, or, with squelched
/// flows shown, rows all of whose records are squelched.
fn join<K: Ord>(
    rows: Vec<(K, u64, &mut Option<RowMarks>)>,
    p: &Marked,
    m: &ActiveMarks,
    subset_rows: impl Fn(&[ResolvedFlow]) -> Vec<(K, u64)>,
) -> u64 {
    let mut index: BTreeMap<K, (u64, &mut Option<RowMarks>)> = rows
        .into_iter()
        .map(|(k, n, slot)| (k, (n, slot)))
        .collect();
    let mut gone: BTreeSet<K> = BTreeSet::new();
    for s in &p.subsets {
        let a = s.action.index();
        let hide = s.action == Action::Squelch && !p.keep_squelched;
        for (k, n) in subset_rows(&s.flows) {
            let Some((flows, slot)) = index.get_mut(&k) else {
                if hide {
                    gone.insert(k);
                }
                continue;
            };
            let rm = slot.get_or_insert_with(|| RowMarks {
                flows: *flows,
                ..RowMarks::default()
            });
            if hide {
                rm.hidden += n;
                continue;
            }
            rm.by_action[a] += n;
            if rm.top[a]
                .as_ref()
                .is_none_or(|t| m.rank(s.mark) < m.rank(t.mark))
            {
                rm.top[a] = Some(TopMark {
                    mark: s.mark,
                    id: m.mark(s.mark).id.clone(),
                });
            }
        }
    }
    if p.keep_squelched {
        index
            .values()
            .filter(|(_, slot)| {
                slot.as_ref()
                    .is_some_and(|r| r.flows > 0 && r.count(Action::Squelch) == r.flows)
            })
            .count() as u64
    } else {
        gone.len() as u64
    }
}

type GroupKey = (Vec<String>, Vec<String>, Option<String>);

fn group_key(g: &FlowGroup) -> GroupKey {
    (g.src.clone(), g.dst.clone(), g.service.clone())
}

/// Pairs (plain or by service; the treemap's groups): `agg` is the view's
/// own aggregator, applied to the flows `rows` came from.
pub fn annotate_groups(
    rows: &mut [FlowGroup],
    p: &Marked,
    m: &ActiveMarks,
    agg: impl Fn(&[ResolvedFlow]) -> Vec<FlowGroup>,
) -> u64 {
    let slots = rows
        .iter_mut()
        .map(|g| (group_key(g), g.flows, &mut g.marks))
        .collect();
    join(slots, p, m, |f| {
        agg(f).iter().map(|g| (group_key(g), g.flows)).collect()
    })
}

/// Mesh rows, joined by `MeshRow::key`.
pub fn annotate_mesh(
    rows: &mut [MeshRow],
    p: &Marked,
    m: &ActiveMarks,
    agg: impl Fn(&[ResolvedFlow]) -> Vec<MeshRow>,
) -> u64 {
    let slots = rows
        .iter_mut()
        .map(|r| (r.key(), r.flows, &mut r.marks))
        .collect();
    join(slots, p, m, |f| {
        agg(f).iter().map(|r| (r.key(), r.flows)).collect()
    })
}

type PeerKey = (Vec<String>, crate::traffic::Direction, Vec<String>, String);

fn peer_keys(sections: &[AppSection]) -> Vec<(PeerKey, u64)> {
    sections
        .iter()
        .flat_map(|s| {
            s.rows.iter().map(move |r| {
                (
                    (
                        s.app.clone(),
                        s.direction,
                        r.peer.clone(),
                        r.service.clone(),
                    ),
                    r.flows,
                )
            })
        })
        .collect()
}

/// App sections' peer rows, joined by (section app, direction, peer,
/// service). A section header is not a row: one that loses every row is
/// gone and not counted.
pub fn annotate_sections(
    sections: &mut [AppSection],
    p: &Marked,
    m: &ActiveMarks,
    agg: impl Fn(&[ResolvedFlow]) -> Vec<AppSection>,
) -> u64 {
    let slots = sections
        .iter_mut()
        .flat_map(|s| {
            let (app, dir) = (s.app.clone(), s.direction);
            s.rows.iter_mut().map(move |r| {
                (
                    (app.clone(), dir, r.peer.clone(), r.service.clone()),
                    r.flows,
                    &mut r.marks,
                )
            })
        })
        .collect();
    join(slots, p, m, |f| peer_keys(&agg(f)))
}

/// The app overlay's peers, joined by (peer, service, decision).
pub fn annotate_peers(
    rows: &mut [AppPeer],
    p: &Marked,
    m: &ActiveMarks,
    agg: impl Fn(&[ResolvedFlow]) -> Vec<AppPeer>,
) -> u64 {
    let key = |r: &AppPeer| (r.peer.clone(), r.service.clone(), r.decision.as_str());
    let slots = rows
        .iter_mut()
        .map(|r| (key(r), r.flows, &mut r.marks))
        .collect();
    join(slots, p, m, |f| {
        agg(f).iter().map(|r| (key(r), r.flows)).collect()
    })
}

```

In `crates/pce/src/traffic/aggregate.rs`, replace:

```rust
    #[serde(flatten)]
    pub draft: Option<DraftCounts>,
}

```

with:

```rust
    #[serde(flatten)]
    pub draft: Option<DraftCounts>,
    /// What the marks say about the row (`pce::marks::annotate_*`);
    /// flattened into JSON as `mark`, `marks` and `squelched`.
    #[serde(flatten)]
    pub marks: Option<crate::marks::RowMarks>,
}

```

In `crates/pce/src/traffic/aggregate.rs`, replace:

```rust
            conns: ConnsByDecision::default(),
            draft: None,
        });
        g.flows += 1;
```

with:

```rust
            conns: ConnsByDecision::default(),
            draft: None,
            marks: None,
        });
        g.flows += 1;
```

In `crates/pce/src/traffic/mesh.rs`, replace:

```rust
    #[serde(flatten)]
    pub draft: Option<DraftCounts>,
}

```

with:

```rust
    #[serde(flatten)]
    pub draft: Option<DraftCounts>,
    /// What the marks say about the row (`pce::marks::annotate_*`);
    /// flattened into JSON as `mark`, `marks` and `squelched`.
    #[serde(flatten)]
    pub marks: Option<crate::marks::RowMarks>,
}

```

In `crates/pce/src/traffic/mesh.rs`, replace:

```rust
            blocked: 0,
            draft: None,
        });
        row.flows += 1;
```

with:

```rust
            blocked: 0,
            draft: None,
            marks: None,
        });
        row.flows += 1;
```

In `crates/pce/src/traffic/sections.rs`, replace:

```rust
    /// The newest flow's last detection.
    pub last_detected: Option<String>,
}

```

with:

```rust
    /// The newest flow's last detection.
    pub last_detected: Option<String>,
    /// What the marks say about the row (`pce::marks::annotate_*`);
    /// flattened into JSON as `mark`, `marks` and `squelched`.
    #[serde(flatten)]
    pub marks: Option<crate::marks::RowMarks>,
}

```

In `crates/pce/src/traffic/sections.rs`, replace:

```rust
                        app_hosts: 0,
                        last_detected: None,
                    },
                    Seen::default(),
```

with:

```rust
                        app_hosts: 0,
                        last_detected: None,
                        marks: None,
                    },
                    Seen::default(),
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p pce --lib marks && cargo test --release -p pce --test marks_perf -- --ignored`
Expected: PASS (22 unit tests); the speed test reports well under 300 ms (about 170 ms on an Apple M-series machine).

- [ ] **Step 5: Run the gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: no formatting change, no warning, every test passes.

- [ ] **Step 6: Commit**

```bash
git add .github/workflows/ci.yml crates/illumio-tui/src/cli/traffic.rs crates/pce/src/app.rs crates/pce/src/marks/mod.rs crates/pce/src/marks/partition.rs crates/pce/src/traffic/aggregate.rs crates/pce/src/traffic/mesh.rs crates/pce/src/traffic/sections.rs crates/pce/src/traffic/treemap.rs crates/pce/tests/marks_perf.rs
git commit -m "feat(marks): partition flows by mark and join them to every view's rows

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

### Task 3: `marks.toml`: the file, validation and `MarksStore`

**Files:**
- Modify: `crates/illumio-tui/src/lib.rs`
- Create: `crates/illumio-tui/src/marks.rs`

**Interfaces:**
- Consumes: Tasks 1-2 (`Mark`, `MarkSet`, `parse_side`, `ServiceSpec::parse`, `parse_decision`, `new_id`, `derived_id`); `config::default_config_path`.
- Produces (`illumio_tui::marks`):

```rust
pub const HEADER: &str;  // "# illumio-tui marks. The tool rewrites this file: comments are not kept, put text in `comment`."
pub struct MarkFile { pub mark: Vec<MarkEntry>, pub extra: toml::Table }
pub struct MarkEntry { pub id, action, src, dst, service, decision, draft_decision, pce, org, expires, comment, created, created_on: Option<toml::Value>,
                       pub extra: toml::Table }  // fn id_text(&self) -> Option<&str>
pub struct MarkProblem { pub index: usize, pub id: Option<String>, pub message: String }  // Display "mark 3 (m-c41fz0): workload needs pce …"
pub enum MarksError { Parse(Vec<MarkProblem>), Io(String), Locked, Gone(String) }        // fn lines(&self) -> Vec<String>
pub struct Loaded { pub file: MarkFile, pub set: MarkSet, pub warnings: Vec<String>, pub comments: bool }
pub fn default_marks_path() -> PathBuf;  pub fn marks_path(config: Option<&Path>) -> PathBuf;
pub fn to_mark(e: &MarkEntry) -> Result<Mark, Vec<String>>;  pub fn entry_of(m: &Mark, extra: toml::Table) -> MarkEntry;
pub fn parse(text: &str) -> Result<Loaded, Vec<MarkProblem>>;  pub fn render(file: &MarkFile) -> String;
pub enum MarkOp { Add(Mark), Replace(Mark), Delete(Vec<String>), DeleteExpired,
                  Extend { id: String, days: i64, comment: Option<String> }, Import { marks: Vec<(Mark, toml::Table)>, replace: bool } }
pub struct Applied { pub ids: Vec<String>, pub added: usize, pub replaced: usize, pub removed: Vec<String>, pub skipped: Vec<String>, pub backup: Option<PathBuf> }
impl MarksStore { fn new(PathBuf); fn none(); fn open(PathBuf); fn path; fn set(&self) -> Option<&MarkSet>; fn loaded; fn error; fn warnings;
                  fn parses(&self) -> u64; fn refresh(&mut self) -> bool /* one stat */; fn apply(&mut self, MarkOp, now: Timestamp) -> Result<Applied, MarksError> }
```

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/src/lib.rs`, replace:

```rust
pub mod cli;
pub mod config;
pub mod tokens;
pub mod tui;
```

with:

```rust
pub mod cli;
pub mod config;
pub mod marks;
pub mod tokens;
pub mod tui;
```

Create `crates/illumio-tui/src/marks.rs` with its tests only (Step 3 adds the code above them):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: &str = r#"# illumio-tui marks. The tool rewrites this file: comments are not kept, put text in `comment`.

[[mark]]
id = "m-7k2q9x"
action = "squelch"
src = { labels = { app = "monitoring" } }
dst = "any"
service = "5666/tcp"
expires = "2027-01-15T23:59:59Z"
comment = "Nagios NRPE, known"
created = "2026-10-17T09:12:00Z"
created_on = "pce.example.com"

[[mark]]
id = "m-c41fz0"
action = "accept-blocked"
src = { ip_list = "rfc1918" }
dst = { labels = { app = "shop", env = ["prod", "dr"] } }
service = "8000-8010/tcp"
decision = "blocked"
pce = "pce.example.com"
comment = "legacy batch, blocked on purpose (CHG-1182)"
created = "2026-10-17T09:20:00Z"
created_on = "pce.example.com"

[[mark]]
id = "m-q9e2aa"
action = "critical"
src = "any"
dst = { workload = "db-01" }
pce = "pce.example.com"
org = 1
created = "2026-10-17T09:31:00Z"
created_on = "pce.example.com"
"#;

    fn now() -> Timestamp {
        "2026-10-17T12:00:00Z".parse().unwrap()
    }

    fn store(dir: &tempfile::TempDir) -> MarksStore {
        MarksStore::open(dir.path().join("illumio-tui").join("marks.toml"))
    }

    fn write(s: &MarksStore, text: &str) {
        fs::create_dir_all(s.path().parent().unwrap()).unwrap();
        fs::write(s.path(), text).unwrap();
    }

    fn new_mark(action: Action, src: &str, comment: &str) -> Mark {
        Mark {
            id: String::new(),
            action,
            src: parse_side(src).unwrap(),
            dst: Side::Any,
            services: Vec::new(),
            decisions: Vec::new(),
            draft_decisions: Vec::new(),
            scope: None,
            expires: None,
            comment: comment.into(),
            created: None,
            created_on: None,
        }
    }

    #[test]
    fn the_spec_example_parses_and_renders_back_unchanged() {
        let l = parse(SPEC).unwrap();
        assert_eq!(l.set.marks.len(), 3);
        assert_eq!(l.set.marks[1].action, Action::AcceptBlocked);
        assert_eq!(
            l.set.marks[2].scope,
            Some(PceScope {
                host: "pce.example.com".into(),
                org: Some(1)
            })
        );
        assert_eq!(l.warnings, vec!["m-q9e2aa has no comment"]);
        assert!(!l.comments, "the header is not a comment");
        assert_eq!(render(&l.file), SPEC);
    }

    #[test]
    fn a_round_trip_keeps_unknown_fields_and_order() {
        let text = format!(
            "{HEADER}\n\nversion = 3\n\n[[mark]]\nid = \"m-b\"\naction = \"reviewed\"\nsrc = \"any\"\ndst = \"any\"\ncomment = \"b\"\nowner = {{ team = \"net\" }}\n\n[[mark]]\nid = \"m-a\"\naction = \"revisit\"\nsrc = \"any\"\ndst = \"any\"\ncomment = \"a\"\n"
        );
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(&dir);
        write(&s, &text);
        s.refresh();
        let done = s
            .apply(
                MarkOp::Add(new_mark(Action::Squelch, "ipl:Corp", "c")),
                now(),
            )
            .unwrap();
        let back = fs::read_to_string(s.path()).unwrap();
        assert!(back.contains("version = 3"), "{back}");
        assert!(back.contains("owner = { team = \"net\" }"), "{back}");
        let l = parse(&back).unwrap();
        let ids: Vec<&str> = l.set.marks.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["m-b", "m-a", done.ids[0].as_str()],
            "file order, new last"
        );
        assert_eq!(l.set.marks[2].created, Some(now()));
    }

    #[test]
    fn comments_are_kept_once_in_the_backup_then_the_header_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(&dir);
        write(
            &s,
            "# my notes\n[[mark]]\naction = \"reviewed\"\nsrc = \"any\"\ndst = \"any\"\ncomment = \"x\"\n",
        );
        s.refresh();
        assert!(s.loaded().unwrap().comments);
        let done = s
            .apply(MarkOp::Add(new_mark(Action::Revisit, "app=a", "y")), now())
            .unwrap();
        let bak = sibling(s.path(), ".bak");
        assert_eq!(done.backup.as_deref(), Some(bak.as_path()));
        assert!(fs::read_to_string(&bak).unwrap().starts_with("# my notes"));
        let text = fs::read_to_string(s.path()).unwrap();
        assert!(text.starts_with(HEADER) && !text.contains("my notes"));
        // The id-less hand-written mark got its derived id written.
        assert!(text.contains("id = \"h-"), "{text}");
        fs::write(&bak, "old").unwrap();
        let again = s
            .apply(MarkOp::Add(new_mark(Action::Revisit, "app=b", "z")), now())
            .unwrap();
        assert_eq!(again.backup, None, "no comment left: no second backup");
        assert_eq!(fs::read_to_string(&bak).unwrap(), "old");
    }

    #[cfg(unix)]
    #[test]
    fn writes_go_through_a_temp_file_and_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(&dir);
        s.apply(MarkOp::Add(new_mark(Action::Squelch, "any", "x")), now())
            .unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(s.path()), 0o600);
        assert_eq!(mode(s.path().parent().unwrap()), 0o700);
        let left: Vec<String> = fs::read_dir(s.path().parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(left, vec!["marks.toml"], "no temp or lock file left");
        // A wider existing file keeps working; the next write sets 0600.
        fs::set_permissions(s.path(), fs::Permissions::from_mode(0o644)).unwrap();
        assert!(s.refresh() || s.set().is_some());
        s.apply(MarkOp::Add(new_mark(Action::Squelch, "app=x", "y")), now())
            .unwrap();
        assert_eq!(mode(s.path()), 0o600);
    }

    #[test]
    fn an_invalid_file_is_never_written() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(&dir);
        let bad = "[[mark]]\naction = \"squelch\"\nsrc = \"any\"\n";
        write(&s, bad);
        assert!(s.refresh());
        assert!(s.set().is_none());
        let e = s
            .apply(MarkOp::Add(new_mark(Action::Squelch, "any", "x")), now())
            .unwrap_err();
        assert!(matches!(e, MarksError::Parse(_)), "{e}");
        assert_eq!(
            e.to_string(),
            "marks.toml invalid (1 problem): mark 1: dst is required (\"any\" for any destination)"
        );
        assert_eq!(fs::read_to_string(s.path()).unwrap(), bad);
    }

    #[test]
    fn refresh_parses_only_when_the_file_changed_and_never_after_its_own_write() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(&dir);
        assert!(s.set().unwrap().marks.is_empty(), "missing: no marks");
        assert!(!s.refresh());
        s.apply(MarkOp::Add(new_mark(Action::Squelch, "any", "x")), now())
            .unwrap();
        let parses = s.parses();
        assert!(!s.refresh(), "its own write is not read again");
        assert_eq!(s.parses(), parses);
        let mut text = fs::read_to_string(s.path()).unwrap();
        text.push_str("\n[[mark]]\naction = \"revisit\"\nsrc = \"any\"\ndst = \"any\"\ncomment = \"by hand\"\n");
        fs::write(s.path(), text).unwrap();
        assert!(s.refresh(), "an edit elsewhere is picked up");
        assert_eq!(s.set().unwrap().marks.len(), 2);
        fs::remove_file(s.path()).unwrap();
        assert!(s.refresh());
        assert!(s.set().unwrap().marks.is_empty());
    }

    #[test]
    fn two_stores_adding_to_one_file_keep_both_marks() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = store(&dir);
        let mut b = store(&dir);
        a.apply(MarkOp::Add(new_mark(Action::Squelch, "app=a", "a")), now())
            .unwrap();
        // b has not seen a's write: the operation reloads first.
        b.apply(MarkOp::Add(new_mark(Action::Revisit, "app=b", "b")), now())
            .unwrap();
        assert_eq!(b.set().unwrap().marks.len(), 2);
        a.refresh();
        assert_eq!(a.set().unwrap().marks.len(), 2);
    }

    #[test]
    fn a_stale_lock_is_removed_and_a_live_one_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(&dir);
        fs::create_dir_all(s.path().parent().unwrap()).unwrap();
        let lp = sibling(s.path(), ".lock");
        let f = fs::File::create(&lp).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(60))
            .unwrap();
        s.apply(MarkOp::Add(new_mark(Action::Squelch, "any", "x")), now())
            .unwrap();
        assert!(!lp.exists());
        fs::File::create(&lp).unwrap();
        let e = s
            .apply(MarkOp::Add(new_mark(Action::Squelch, "app=y", "y")), now())
            .unwrap_err();
        assert_eq!(e, MarksError::Locked);
        assert_eq!(
            e.to_string(),
            "marks.toml is locked by another illumio-tui (marks.toml.lock): try again"
        );
        assert!(lp.exists(), "another writer's lock is left alone");
    }

    #[test]
    fn an_edit_of_a_mark_deleted_elsewhere_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = store(&dir);
        let id = a
            .apply(MarkOp::Add(new_mark(Action::Squelch, "any", "x")), now())
            .unwrap()
            .ids
            .remove(0);
        let mut b = store(&dir);
        b.apply(MarkOp::Delete(vec![id.clone()]), now()).unwrap();
        let mut m = a.set().unwrap().marks[0].clone();
        m.comment = "changed".into();
        assert_eq!(
            a.apply(MarkOp::Replace(m), now()).unwrap_err(),
            MarksError::Gone(id.clone())
        );
        let ext = MarkOp::Extend {
            id: id.clone(),
            days: 30,
            comment: None,
        };
        assert_eq!(a.apply(ext, now()).unwrap_err(), MarksError::Gone(id));
    }

    #[test]
    fn delete_takes_all_or_none_and_extend_counts_from_the_later_of_now_and_the_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(&dir);
        let mut soon = new_mark(Action::Squelch, "app=a", "a");
        soon.expires = Some(now() + SignedDuration::from_hours(48));
        let mut old = new_mark(Action::Reviewed, "app=b", "b");
        old.expires = Some(now() - SignedDuration::from_hours(24));
        let a = s.apply(MarkOp::Add(soon), now()).unwrap().ids.remove(0);
        let b = s.apply(MarkOp::Add(old), now()).unwrap().ids.remove(0);
        let never = s
            .apply(MarkOp::Add(new_mark(Action::Critical, "app=c", "c")), now())
            .unwrap()
            .ids
            .remove(0);
        assert_eq!(
            s.apply(MarkOp::Delete(vec![a.clone(), "m-nope00".into()]), now())
                .unwrap_err(),
            MarksError::Gone("m-nope00".into())
        );
        assert_eq!(s.set().unwrap().marks.len(), 3, "nothing removed");
        let ext = |id: &str| MarkOp::Extend {
            id: id.into(),
            days: 30,
            comment: Some("still needed".into()),
        };
        s.apply(ext(&a), now()).unwrap();
        s.apply(ext(&b), now()).unwrap();
        let unchanged = s.apply(ext(&never), now()).unwrap();
        assert!(unchanged.ids.is_empty(), "a never mark stays never");
        let set = s.set().unwrap();
        let get = |id: &str| &set.marks[set.find(id).unwrap()];
        assert_eq!(
            get(&a).expires,
            Some(now() + SignedDuration::from_hours(24 * 32))
        );
        assert_eq!(
            get(&b).expires,
            Some(now() + SignedDuration::from_hours(24 * 30)),
            "revived"
        );
        assert_eq!(get(&a).comment, "still needed");
        assert_eq!(get(&never).expires, None);
        let later = now() + SignedDuration::from_hours(24 * 29);
        let done = s.apply(MarkOp::DeleteExpired, later).unwrap();
        assert!(done.removed.is_empty());
        let done = s
            .apply(
                MarkOp::DeleteExpired,
                now() + SignedDuration::from_hours(24 * 40),
            )
            .unwrap();
        assert_eq!(done.removed.len(), 2);
    }

    #[test]
    fn every_validation_rule_names_the_mark() {
        let cases: [(&str, &str); 18] = [
            ("src = { }", "src: no kind"),
            (
                "src = { labels = { app = \"a\" }, ip = \"10.0.0.1\" }",
                "several kinds (ip, labels)",
            ),
            ("src = { fqdn = \"x\" }", "unknown kind \"fqdn\""),
            ("src = { labels = { } }", "empty labels"),
            ("src = { labels = { app = [] } }", "empty list"),
            ("src = { ip = \"10.0.0.0/40\" }", "at most 32"),
            ("service = \"70000/tcp\"", "is not a port"),
            ("service = \"90-80/tcp\"", "ends before"),
            ("service = \"80/bogus\"", "unknown protocol"),
            ("service = \"22/icmp\"", "has no ports"),
            ("src = { workload = \"db-01\" }", "workload needs pce"),
            ("org = 1", "org needs pce"),
            (
                "action = \"accept-blocked\"",
                "accept-blocked needs decision",
            ),
            (
                "action = \"accept-blocked\"\ndecision = [\"blocked\", \"allowed\"]",
                "cannot include allowed",
            ),
            ("action = \"nope\"", "unknown action \"nope\""),
            ("decision = \"maybe\"", "unknown decision \"maybe\""),
            ("expires = \"tomorrow\"", "bad timestamp"),
            ("src = \"everything\"", "write \"any\""),
        ];
        for (field, want) in cases {
            let key = field.split(' ').next().unwrap();
            let mut body = format!("[[mark]]\nid = \"m-1\"\n{field}\n");
            for (k, v) in [
                ("action", "\"reviewed\""),
                ("src", "\"any\""),
                ("dst", "\"any\""),
                ("comment", "\"c\""),
            ] {
                if k != key {
                    body.push_str(&format!("{k} = {v}\n"));
                }
            }
            let e = parse(&body).unwrap_err();
            let text = e
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join("; ");
            assert!(text.contains("mark 1 (m-1): "), "{field}: {text}");
            assert!(text.contains(want), "{field}: {text}");
        }
        let dup = "[[mark]]\nid = \"m-1\"\naction = \"reviewed\"\nsrc = \"any\"\ndst = \"any\"\n[[mark]]\nid = \"m-1\"\naction = \"revisit\"\nsrc = \"any\"\ndst = \"any\"\n";
        assert_eq!(
            parse(dup).unwrap_err()[0].to_string(),
            "mark 2 (m-1): id m-1 is used by mark 1 too"
        );
        let unknown = "future = true\n[[mark]]\naction = \"reviewed\"\nsrc = \"any\"\ndst = \"any\"\nwhy = \"kept\"\n";
        assert!(parse(unknown).is_ok(), "unknown fields are not errors");
    }

    #[test]
    fn hand_written_marks_get_a_stable_derived_id() {
        let text = "[[mark]]\naction = \"squelch\"\nsrc = { ip_list = \"Corp\" }\ndst = \"any\"\ncomment = \"x\"\n";
        let a = parse(text).unwrap().set.marks[0].id.clone();
        assert!(a.starts_with("h-"), "{a}");
        assert_eq!(parse(text).unwrap().set.marks[0].id, a);
    }

    #[test]
    fn truncated_and_garbage_files_never_panic() {
        let mut inputs: Vec<String> = (0..SPEC.len())
            .filter(|i| SPEC.is_char_boundary(*i))
            .map(|i| SPEC[..i].to_string())
            .collect();
        inputs.extend(
            [
                "\u{0}\u{1}\u{2}",
                "[[mark]",
                "[[mark]]\nsrc = { labels = { app = 1 } }",
                "mark = 5",
                "[[mark]]\norg = -1\npce = \"h\"",
                "[[mark]]\nexpires = 1979-05-27",
            ]
            .map(String::from),
        );
        for text in inputs {
            let _ = parse(&text);
        }
        let e = parse("[[mark]\n").unwrap_err();
        assert_eq!(e[0].index, 0);
        assert!(e[0].to_string().starts_with("marks.toml: "), "{}", e[0]);
    }

    #[test]
    fn an_unreadable_path_is_an_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("marks.toml");
        fs::create_dir_all(&path).unwrap();
        let s = MarksStore::open(path);
        assert!(
            matches!(s.error(), Some(MarksError::Io(_))),
            "{:?}",
            s.error()
        );
    }

    #[test]
    fn a_store_without_a_file_reads_and_writes_nothing() {
        let mut s = MarksStore::none();
        assert!(!s.refresh());
        assert!(s.set().unwrap().marks.is_empty());
        let e = s
            .apply(MarkOp::Add(new_mark(Action::Squelch, "any", "x")), now())
            .unwrap_err();
        assert!(matches!(e, MarksError::Io(_)), "{e}");
    }

    #[test]
    fn the_path_is_next_to_the_config_file() {
        assert_eq!(
            marks_path(Some(Path::new("/etc/x/config.toml"))),
            PathBuf::from("/etc/x/marks.toml")
        );
        assert!(default_marks_path().ends_with("illumio-tui/marks.toml"));
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p illumio-tui --lib marks::`
Expected: FAIL: it does not compile (`cannot find type MarksStore`, …).

- [ ] **Step 3: Implement**

In `crates/illumio-tui/src/marks.rs`, insert above the tests:

```rust
//! `marks.toml`: the serde mirror of the file (`MarkFile`, `MarkEntry`,
//! unknown fields kept in `extra`), its conversion to and from
//! `pce::marks::Mark`, and `MarksStore`: reloaded when the file changes
//! (one `stat`), written as operations (`MarkOp`) under a lock, from a
//! fresh read, atomically, mode 0600. A local file only: marks never cause
//! a PCE request.
use jiff::{SignedDuration, Timestamp};
use pce::marks::{
    Action, Mark, MarkSet, PceScope, ServiceSpec, Side, derived_id, new_id, parse_decision,
    parse_side,
};
use pce::models::PolicyDecision;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// The first line of every write.
pub const HEADER: &str = "# illumio-tui marks. The tool rewrites this file: comments are not kept, put text in `comment`.";

/// The whole file: its marks in file order, and the top-level keys this
/// version does not know.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MarkFile {
    #[serde(default)]
    pub mark: Vec<MarkEntry>,
    #[serde(default, flatten)]
    pub extra: toml::Table,
}

/// One `[[mark]]` as written: every known field as a TOML value, checked
/// by [`to_mark`] with the mark's position, and unknown fields kept.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MarkEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dst: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_decision: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pce: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_on: Option<toml::Value>,
    #[serde(default, flatten)]
    pub extra: toml::Table,
}

impl MarkEntry {
    /// The known fields in the order they are written.
    fn fields(&self) -> [(&'static str, Option<&toml::Value>); 13] {
        [
            ("id", self.id.as_ref()),
            ("action", self.action.as_ref()),
            ("src", self.src.as_ref()),
            ("dst", self.dst.as_ref()),
            ("service", self.service.as_ref()),
            ("decision", self.decision.as_ref()),
            ("draft_decision", self.draft_decision.as_ref()),
            ("pce", self.pce.as_ref()),
            ("org", self.org.as_ref()),
            ("expires", self.expires.as_ref()),
            ("comment", self.comment.as_ref()),
            ("created", self.created.as_ref()),
            ("created_on", self.created_on.as_ref()),
        ]
    }

    /// The entry's own `id`, when it is a string.
    pub fn id_text(&self) -> Option<&str> {
        self.id.as_ref().and_then(toml::Value::as_str)
    }
}

/// One validation problem, with its mark's position (from 1; 0 is the
/// file itself).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkProblem {
    pub index: usize,
    pub id: Option<String>,
    pub message: String,
}

impl std::fmt::Display for MarkProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.index, &self.id) {
            (0, _) => write!(f, "marks.toml: {}", self.message),
            (i, Some(id)) => write!(f, "mark {i} ({id}): {}", self.message),
            (i, None) => write!(f, "mark {i}: {}", self.message),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MarksError {
    /// The file does not validate: marks are off and nothing writes it.
    #[error("{}", problems_text(.0))]
    Parse(Vec<MarkProblem>),
    /// The file cannot be read or written.
    #[error("{0}")]
    Io(String),
    /// Another writer holds `marks.toml.lock`.
    #[error("marks.toml is locked by another illumio-tui (marks.toml.lock): try again")]
    Locked,
    /// The mark an operation names is not in the file any more.
    #[error("mark {0} was deleted elsewhere")]
    Gone(String),
}

/// `marks.toml invalid (2 problems): mark 3 (m-c41fz0): workload needs pce`.
fn problems_text(p: &[MarkProblem]) -> String {
    let n = p.len();
    let first = p.first().map(|x| x.to_string()).unwrap_or_default();
    format!(
        "marks.toml invalid ({n} problem{}): {first}",
        if n == 1 { "" } else { "s" }
    )
}

impl MarksError {
    /// The problems of an invalid file, one per line; the error otherwise.
    pub fn lines(&self) -> Vec<String> {
        match self {
            MarksError::Parse(p) => p.iter().map(|x| x.to_string()).collect(),
            e => vec![e.to_string()],
        }
    }
}

/// A parsed, valid file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Loaded {
    pub file: MarkFile,
    /// The marks, aligned with `file.mark`; hand-written marks without an
    /// id carry their derived `h-` id.
    pub set: MarkSet,
    /// `m-… has no comment`: loads, shows `(no comment)`.
    pub warnings: Vec<String>,
    /// A comment line other than the header: the next write keeps the
    /// file in `marks.toml.bak` first.
    pub comments: bool,
}

/// `$XDG_CONFIG_HOME/illumio-tui/marks.toml`, else
/// `~/.config/illumio-tui/marks.toml`: next to the default config file.
pub fn default_marks_path() -> PathBuf {
    marks_path(None)
}

/// `marks.toml` in the directory of the config file in use (`--config`),
/// else [`default_marks_path`].
pub fn marks_path(config: Option<&Path>) -> PathBuf {
    let config = config
        .map(Path::to_path_buf)
        .unwrap_or_else(crate::config::default_config_path);
    config
        .parent()
        .map(|d| d.join("marks.toml"))
        .unwrap_or_else(|| PathBuf::from("marks.toml"))
}

fn text_of<'v>(v: &'v toml::Value, what: &str) -> Result<&'v str, String> {
    v.as_str()
        .ok_or_else(|| format!("{what}: expected a string, found {}", v.type_str()))
}

/// A string or a list of strings, as a list.
fn strings(v: &toml::Value, what: &str) -> Result<Vec<String>, String> {
    let list: Vec<String> = match v {
        toml::Value::Array(a) => a
            .iter()
            .map(|x| text_of(x, what).map(str::to_string))
            .collect::<Result<_, _>>()?,
        other => vec![text_of(other, what)?.to_string()],
    };
    if list.is_empty() {
        return Err(format!("{what}: empty list"));
    }
    Ok(list)
}

/// `"any"`, or a table with exactly one of `labels`, `workload`, `ip`,
/// `ip_list`.
fn side_of(v: &toml::Value, what: &str) -> Result<Side, String> {
    const KINDS: [&str; 4] = ["labels", "workload", "ip", "ip_list"];
    let t = match v {
        toml::Value::String(s) if s == "any" => return Ok(Side::Any),
        toml::Value::Table(t) => t,
        _ => {
            return Err(format!(
                "{what}: write \"any\" or one of {{ labels = … }}, {{ workload = … }}, {{ ip = … }}, {{ ip_list = … }}"
            ));
        }
    };
    if let Some(k) = t.keys().find(|k| !KINDS.contains(&k.as_str())) {
        return Err(format!("{what}: unknown kind {k:?}"));
    }
    let kinds: Vec<&String> = t.keys().collect();
    let (kind, value) = match kinds.as_slice() {
        [k] => (k.as_str(), &t[k.as_str()]),
        [] => return Err(format!("{what}: no kind (labels, workload, ip or ip_list)")),
        many => {
            let names: Vec<&str> = many.iter().map(|k| k.as_str()).collect();
            return Err(format!("{what}: several kinds ({})", names.join(", ")));
        }
    };
    let prefixed = |p: &str| -> Result<Side, String> {
        parse_side(&format!("{p}{}", text_of(value, what)?)).map_err(|e| format!("{what}: {e}"))
    };
    match kind {
        "workload" => prefixed("wl:"),
        "ip" => prefixed("ip:"),
        "ip_list" => prefixed("ipl:"),
        _ => {
            let labels = value
                .as_table()
                .ok_or_else(|| format!("{what}: labels must be a table of key = value"))?;
            if labels.is_empty() {
                return Err(format!("{what}: empty labels"));
            }
            let mut out = std::collections::BTreeMap::new();
            for (k, v) in labels {
                let values = strings(v, &format!("{what}: label {k}"))?;
                if values.iter().any(String::is_empty) {
                    return Err(format!("{what}: label {k}: empty value"));
                }
                out.insert(k.clone(), values);
            }
            Ok(Side::Labels(out))
        }
    }
}

fn timestamp_of(v: &toml::Value, what: &str) -> Result<Timestamp, String> {
    let text = match v {
        toml::Value::Datetime(d) => d.to_string(),
        other => text_of(other, what)?.to_string(),
    };
    text.parse::<Timestamp>().map_err(|_| {
        format!("{what}: bad timestamp {text:?} (RFC 3339 UTC, e.g. 2027-01-15T23:59:59Z)")
    })
}

fn decisions_of(v: Option<&toml::Value>, what: &str) -> Result<Vec<PolicyDecision>, String> {
    match v {
        None => Ok(Vec::new()),
        Some(v) => strings(v, what)?
            .iter()
            .map(|d| parse_decision(d).map_err(|e| format!("{what}: {e}")))
            .collect(),
    }
}

/// `r`'s value, its error kept in `problems`.
fn take<T>(problems: &mut Vec<String>, r: Result<T, String>) -> Option<T> {
    r.map_err(|e| problems.push(e)).ok()
}

/// One entry as a mark: every problem of its fields at once. A missing
/// id is left empty (the caller derives one).
pub fn to_mark(e: &MarkEntry) -> Result<Mark, Vec<String>> {
    let mut problems = Vec::new();
    let id = take(
        &mut problems,
        match &e.id {
            None => Ok(String::new()),
            Some(v) => text_of(v, "id").map(str::to_string),
        },
    );
    let action = take(
        &mut problems,
        match &e.action {
            None => Err("action is required".into()),
            Some(v) => text_of(v, "action").and_then(|a| {
                Action::parse(a).ok_or_else(|| format!("unknown action {a:?}: {}", Action::names()))
            }),
        },
    );
    let src = take(
        &mut problems,
        match &e.src {
            None => Err("src is required (\"any\" for any source)".into()),
            Some(v) => side_of(v, "src"),
        },
    );
    let dst = take(
        &mut problems,
        match &e.dst {
            None => Err("dst is required (\"any\" for any destination)".into()),
            Some(v) => side_of(v, "dst"),
        },
    );
    let services = take(
        &mut problems,
        match &e.service {
            None => Ok(Vec::new()),
            Some(v) => strings(v, "service").and_then(|l| {
                l.iter()
                    .map(|s| ServiceSpec::parse(s).map_err(|e| format!("service: {e}")))
                    .collect()
            }),
        },
    );
    let decisions = take(&mut problems, decisions_of(e.decision.as_ref(), "decision"));
    let draft_decisions = take(
        &mut problems,
        decisions_of(e.draft_decision.as_ref(), "draft_decision"),
    );
    let host = take(
        &mut problems,
        match &e.pce {
            None => Ok(None),
            Some(v) => text_of(v, "pce").and_then(|h| {
                if h.trim().is_empty() {
                    Err("pce: empty host".into())
                } else {
                    Ok(Some(h.trim().to_string()))
                }
            }),
        },
    );
    let org = take(
        &mut problems,
        match &e.org {
            None => Ok(None),
            Some(v) => v
                .as_integer()
                .and_then(|n| u64::try_from(n).ok())
                .map(Some)
                .ok_or_else(|| "org: expected an org id (a number)".to_string()),
        },
    );
    let expires = take(
        &mut problems,
        e.expires
            .as_ref()
            .map(|v| timestamp_of(v, "expires"))
            .transpose(),
    );
    let created = take(
        &mut problems,
        e.created
            .as_ref()
            .map(|v| timestamp_of(v, "created"))
            .transpose(),
    );
    let comment = take(
        &mut problems,
        match &e.comment {
            None => Ok(String::new()),
            Some(v) => text_of(v, "comment").map(str::to_string),
        },
    );
    let created_on = take(
        &mut problems,
        match &e.created_on {
            None => Ok(None),
            Some(v) => text_of(v, "created_on").map(|s| Some(s.to_string())),
        },
    );
    let scope = match (host, org) {
        (Some(Some(host)), Some(org)) => Some(PceScope { host, org }),
        (Some(None), Some(Some(_))) => {
            problems.push("org needs pce".into());
            None
        }
        _ => None,
    };
    let (
        Some(id),
        Some(action),
        Some(src),
        Some(dst),
        Some(services),
        Some(decisions),
        Some(draft_decisions),
        Some(expires),
        Some(created),
        Some(comment),
        Some(created_on),
    ) = (
        id,
        action,
        src,
        dst,
        services,
        decisions,
        draft_decisions,
        expires,
        created,
        comment,
        created_on,
    )
    else {
        return Err(problems);
    };
    if !problems.is_empty() {
        return Err(problems);
    }
    let m = Mark {
        id,
        action,
        src,
        dst,
        services,
        decisions,
        draft_decisions,
        scope,
        expires,
        comment,
        created,
        created_on,
    };
    let rules = m.problems();
    if rules.is_empty() { Ok(m) } else { Err(rules) }
}

fn side_value(s: &Side) -> toml::Value {
    use toml::Value as V;
    let one = |k: &str, v: String| {
        let mut t = toml::Table::new();
        t.insert(k.into(), V::String(v));
        V::Table(t)
    };
    match s {
        Side::Any => V::String("any".into()),
        Side::Labels(l) => {
            let mut labels = toml::Table::new();
            for (k, vs) in l {
                let v = match vs.as_slice() {
                    [one] => V::String(one.clone()),
                    many => V::Array(many.iter().cloned().map(V::String).collect()),
                };
                labels.insert(k.clone(), v);
            }
            let mut t = toml::Table::new();
            t.insert("labels".into(), V::Table(labels));
            V::Table(t)
        }
        Side::Workload(w) => one("workload", w.clone()),
        Side::Ip { .. } => one(
            "ip",
            pce::marks::side_text(s)
                .trim_start_matches("ip:")
                .to_string(),
        ),
        Side::IpList(l) => one("ip_list", l.clone()),
    }
}

/// One string, or a list of them.
fn list_value(items: Vec<String>) -> Option<toml::Value> {
    match items.len() {
        0 => None,
        1 => items.into_iter().next().map(toml::Value::String),
        _ => Some(toml::Value::Array(
            items.into_iter().map(toml::Value::String).collect(),
        )),
    }
}

/// The entry that writes `m`, with `extra` (the old entry's unknown
/// fields on a replace).
pub fn entry_of(m: &Mark, extra: toml::Table) -> MarkEntry {
    let s = |x: &str| Some(toml::Value::String(x.to_string()));
    let decisions =
        |d: &[PolicyDecision]| list_value(d.iter().map(|d| d.as_str().into()).collect());
    MarkEntry {
        id: (!m.id.is_empty()).then(|| toml::Value::String(m.id.clone())),
        action: s(m.action.as_str()),
        src: Some(side_value(&m.src)),
        dst: Some(side_value(&m.dst)),
        service: list_value(m.services.iter().map(ServiceSpec::text).collect()),
        decision: decisions(&m.decisions),
        draft_decision: decisions(&m.draft_decisions),
        pce: m.scope.as_ref().and_then(|p| s(&p.host)),
        org: m
            .scope
            .as_ref()
            .and_then(|p| p.org)
            .and_then(|o| i64::try_from(o).ok())
            .map(toml::Value::Integer),
        expires: m.expires.and_then(|t| s(&t.to_string())),
        comment: (!m.comment.is_empty()).then(|| toml::Value::String(m.comment.clone())),
        created: m.created.and_then(|t| s(&t.to_string())),
        created_on: m.created_on.as_deref().and_then(s),
        extra,
    }
}

/// Every entry as a mark, with positions, ids (derived for hand-written
/// marks without one), duplicate ids and missing comments.
fn convert(file: &MarkFile) -> Result<(MarkSet, Vec<String>), Vec<MarkProblem>> {
    let mut problems = Vec::new();
    let mut marks: Vec<Mark> = Vec::new();
    let mut warnings = Vec::new();
    for (i, e) in file.mark.iter().enumerate() {
        let index = i + 1;
        match to_mark(e) {
            Ok(mut m) => {
                if m.id.is_empty() {
                    m.id = derived_id(&m.canonical());
                }
                if let Some(j) = marks.iter().position(|x| x.id == m.id) {
                    problems.push(MarkProblem {
                        index,
                        id: Some(m.id.clone()),
                        message: format!("id {} is used by mark {} too", m.id, j + 1),
                    });
                }
                if m.comment.trim().is_empty() {
                    warnings.push(format!("{} has no comment", m.id));
                }
                marks.push(m);
            }
            Err(msgs) => {
                let id = e.id_text().map(str::to_string);
                problems.extend(msgs.into_iter().map(|message| MarkProblem {
                    index,
                    id: id.clone(),
                    message,
                }));
                // A placeholder keeps the positions of later duplicates.
                marks.push(Mark {
                    id: id.unwrap_or_default(),
                    ..placeholder()
                });
            }
        }
    }
    if problems.is_empty() {
        Ok((MarkSet { marks }, warnings))
    } else {
        Err(problems)
    }
}

fn placeholder() -> Mark {
    Mark {
        id: String::new(),
        action: Action::Reviewed,
        src: Side::Any,
        dst: Side::Any,
        services: Vec::new(),
        decisions: Vec::new(),
        draft_decisions: Vec::new(),
        scope: None,
        expires: None,
        comment: String::new(),
        created: None,
        created_on: None,
    }
}

/// A comment line other than the header.
fn has_comments(text: &str) -> bool {
    text.lines().any(|l| {
        let t = l.trim();
        t.starts_with('#') && t != HEADER
    })
}

/// The file's text, every problem at once. The parser's own message is
/// reported with its byte offset only, never the offending line.
pub fn parse(text: &str) -> Result<Loaded, Vec<MarkProblem>> {
    let file: MarkFile = toml::from_str(text).map_err(|e| {
        let message = match e.span() {
            Some(span) => format!("{} at byte {}", e.message().trim(), span.start),
            None => e.message().trim().to_string(),
        };
        vec![MarkProblem {
            index: 0,
            id: None,
            message,
        }]
    })?;
    let (set, warnings) = convert(&file)?;
    Ok(Loaded {
        file,
        set,
        warnings,
        comments: has_comments(text),
    })
}

/// `key`, or `"key"` when it is not a bare TOML key.
fn key_text(k: &str) -> String {
    if !k.is_empty()
        && k.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        k.to_string()
    } else {
        toml::Value::String(k.to_string()).to_string()
    }
}

/// The file's text: the header, the unknown top-level keys, then each
/// mark in order, known fields first (inline values), unknown ones after.
pub fn render(file: &MarkFile) -> String {
    let mut out = format!("{HEADER}\n");
    if !file.extra.is_empty() {
        out.push('\n');
        for (k, v) in &file.extra {
            out.push_str(&format!("{} = {v}\n", key_text(k)));
        }
    }
    for e in &file.mark {
        out.push_str("\n[[mark]]\n");
        for (k, v) in e.fields() {
            if let Some(v) = v {
                out.push_str(&format!("{k} = {v}\n"));
            }
        }
        for (k, v) in &e.extra {
            out.push_str(&format!("{} = {v}\n", key_text(k)));
        }
    }
    out
}

/// One write: what changes, never the whole state, so writers in other
/// instances keep their changes.
#[derive(Debug, Clone, PartialEq)]
pub enum MarkOp {
    /// A new mark; an empty id gets a new one, an empty `created` now.
    Add(Mark),
    /// The mark with this id, new fields; its unknown fields are kept.
    Replace(Mark),
    /// These marks, all or none.
    Delete(Vec<String>),
    /// Every mark expired at `now`.
    DeleteExpired,
    /// `days` more from the later of now and the mark's expiry (a `never`
    /// mark is left as it is), with a new comment when given.
    Extend {
        id: String,
        days: i64,
        comment: Option<String>,
    },
    /// Validated marks from `marks import`: an existing id is skipped, or
    /// replaced with `replace`; an empty id gets a new one.
    Import {
        marks: Vec<(Mark, toml::Table)>,
        replace: bool,
    },
}

/// What a write did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Applied {
    /// The marks added or changed, by id.
    pub ids: Vec<String>,
    pub added: usize,
    pub replaced: usize,
    pub removed: Vec<String>,
    pub skipped: Vec<String>,
    /// The file had comments and was kept here first.
    pub backup: Option<PathBuf>,
}

/// `op` on `loaded` (a fresh read).
fn apply_op(loaded: &mut Loaded, op: MarkOp, now: Timestamp) -> Result<Applied, MarksError> {
    let pid = std::process::id();
    let mut done = Applied::default();
    let entries = &mut loaded.file.mark;
    let marks = &mut loaded.set.marks;
    // Every entry gets its id written (hand-written ones their derived id).
    for (e, m) in entries.iter_mut().zip(marks.iter()) {
        if e.id_text().is_none() {
            e.id = Some(toml::Value::String(m.id.clone()));
        }
    }
    let find = |marks: &[Mark], id: &str| {
        marks
            .iter()
            .position(|m| m.id == id)
            .ok_or_else(|| MarksError::Gone(id.to_string()))
    };
    let fresh_id = |marks: &[Mark], m: &Mark| {
        new_id(now.as_nanosecond(), pid, &m.canonical(), |id| {
            marks.iter().any(|x| x.id == id)
        })
    };
    match op {
        MarkOp::Add(mut m) => {
            if m.id.is_empty() || marks.iter().any(|x| x.id == m.id) {
                m.id = fresh_id(marks, &m);
            }
            m.created.get_or_insert(now);
            entries.push(entry_of(&m, toml::Table::new()));
            done.ids.push(m.id.clone());
            done.added = 1;
            marks.push(m);
        }
        MarkOp::Replace(mut m) => {
            let i = find(marks, &m.id)?;
            m.created = m.created.or(marks[i].created);
            m.created_on = m.created_on.or_else(|| marks[i].created_on.clone());
            let extra = std::mem::take(&mut entries[i].extra);
            entries[i] = entry_of(&m, extra);
            done.ids.push(m.id.clone());
            done.replaced = 1;
            marks[i] = m;
        }
        MarkOp::Delete(ids) => {
            for id in &ids {
                find(marks, id)?;
            }
            let keep: Vec<bool> = marks.iter().map(|m| !ids.contains(&m.id)).collect();
            retain(entries, marks, &keep);
            done.removed = ids;
        }
        MarkOp::DeleteExpired => {
            let keep: Vec<bool> = marks.iter().map(|m| !m.is_expired(now)).collect();
            done.removed = marks
                .iter()
                .filter(|m| m.is_expired(now))
                .map(|m| m.id.clone())
                .collect();
            retain(entries, marks, &keep);
        }
        MarkOp::Extend { id, days, comment } => {
            let i = find(marks, &id)?;
            let m = &mut marks[i];
            if let Some(e) = m.expires {
                m.expires = Some(e.max(now) + SignedDuration::from_hours(24 * days));
                if let Some(c) = comment {
                    m.comment = c;
                }
                let extra = std::mem::take(&mut entries[i].extra);
                entries[i] = entry_of(m, extra);
                done.ids.push(id);
                done.replaced = 1;
            }
        }
        MarkOp::Import {
            marks: incoming,
            replace,
        } => {
            for (mut m, extra) in incoming {
                match marks.iter().position(|x| !m.id.is_empty() && x.id == m.id) {
                    Some(i) if replace => {
                        entries[i] = entry_of(&m, extra);
                        done.ids.push(m.id.clone());
                        done.replaced += 1;
                        marks[i] = m;
                    }
                    Some(_) => done.skipped.push(m.id.clone()),
                    None => {
                        if m.id.is_empty() {
                            m.id = fresh_id(marks, &m);
                        }
                        m.created.get_or_insert(now);
                        entries.push(entry_of(&m, extra));
                        done.ids.push(m.id.clone());
                        done.added += 1;
                        marks.push(m);
                    }
                }
            }
        }
    }
    Ok(done)
}

fn retain(entries: &mut Vec<MarkEntry>, marks: &mut Vec<Mark>, keep: &[bool]) {
    let mut k = keep.iter();
    entries.retain(|_| *k.next().unwrap_or(&true));
    let mut k = keep.iter();
    marks.retain(|_| *k.next().unwrap_or(&true));
}

/// What `refresh` compares: the file's modification time and length, or
/// its absence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stamp {
    Missing,
    File {
        modified: Option<SystemTime>,
        len: u64,
    },
}

/// `marks.toml.lock`, `marks.toml.tmp-<pid>`, `marks.toml.bak`.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| "marks.toml".into());
    name.push(suffix);
    path.with_file_name(name)
}

/// The lock retries every 25 ms for this long.
const LOCK_WAIT: Duration = Duration::from_secs(1);
/// A lock file older than this is a crashed writer's.
const LOCK_STALE: Duration = Duration::from_secs(10);

/// Held while a write runs; the lock file goes when it drops.
struct Lock(PathBuf);

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn io(path: &Path, what: &str, e: std::io::Error) -> MarksError {
    MarksError::Io(format!("cannot {what} {}: {e}", path.display()))
}

/// `create_new` (O_EXCL) on the lock file, std only: `File::lock` needs
/// Rust 1.89 and the workspace is 1.88.
fn lock(path: &Path) -> Result<Lock, MarksError> {
    let lp = sibling(path, ".lock");
    let start = Instant::now();
    let mut stale_removed = false;
    loop {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lp)
        {
            Ok(mut f) => {
                let _ = writeln!(f, "{}", std::process::id());
                return Ok(Lock(lp));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let stale = fs::metadata(&lp)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age > LOCK_STALE);
                if stale && !stale_removed {
                    stale_removed = true;
                    let _ = fs::remove_file(&lp);
                    continue;
                }
                if start.elapsed() >= LOCK_WAIT {
                    return Err(MarksError::Locked);
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => return Err(io(&lp, "create", e)),
        }
    }
}

/// The directory, created 0700 on Unix when missing.
fn ensure_dir(dir: &Path) -> Result<(), MarksError> {
    if dir.as_os_str().is_empty() || dir.is_dir() {
        return Ok(());
    }
    let mut b = fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(dir).map_err(|e| io(dir, "create", e))
}

/// Writes `text` to `path` mode 0600: a temp file in the same directory,
/// synced, renamed over it.
fn write_atomic(path: &Path, text: &str) -> Result<(), MarksError> {
    let tmp = sibling(path, &format!(".tmp-{}", std::process::id()));
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let result = (|| {
        let mut f = opts.open(&tmp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    result.map_err(|e| {
        let _ = fs::remove_file(&tmp);
        io(path, "write", e)
    })
}

/// The marks file: its path, its last stamp and the parsed result.
#[derive(Debug, Clone)]
pub struct MarksStore {
    path: PathBuf,
    stamp: Option<Stamp>,
    state: Result<Loaded, MarksError>,
    parses: u64,
}

impl MarksStore {
    /// A store that has not looked at its file yet: no marks.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            stamp: None,
            state: Ok(Loaded::default()),
            parses: 0,
        }
    }

    /// A store without a file: no marks, nothing read or written (a model
    /// before `app::run` gives it the path, and every test that does not).
    pub fn none() -> Self {
        Self::new(PathBuf::new())
    }

    /// `new`, then one `refresh`.
    pub fn open(path: PathBuf) -> Self {
        let mut s = Self::new(path);
        s.refresh();
        s
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The valid marks; `None` while the file is invalid or unreadable.
    pub fn set(&self) -> Option<&MarkSet> {
        self.state.as_ref().ok().map(|l| &l.set)
    }

    pub fn loaded(&self) -> Option<&Loaded> {
        self.state.as_ref().ok()
    }

    pub fn error(&self) -> Option<&MarksError> {
        self.state.as_ref().err()
    }

    pub fn warnings(&self) -> &[String] {
        self.loaded().map_or(&[], |l| l.warnings.as_slice())
    }

    /// How many times the file's text was parsed (tests: no reparse after
    /// this store's own write).
    pub fn parses(&self) -> u64 {
        self.parses
    }

    fn stamp_now(&self) -> Result<Stamp, MarksError> {
        match fs::metadata(&self.path) {
            Ok(m) => Ok(Stamp::File {
                modified: m.modified().ok(),
                len: m.len(),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Stamp::Missing),
            Err(e) => Err(io(&self.path, "read", e)),
        }
    }

    /// The file's text and its parse, `Loaded::default()` when missing.
    fn read(&mut self) -> Result<Loaded, MarksError> {
        match fs::read_to_string(&self.path) {
            Ok(text) => {
                self.parses += 1;
                parse(&text).map_err(MarksError::Parse)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Loaded::default()),
            Err(e) => Err(io(&self.path, "read", e)),
        }
    }

    /// One `stat`; the file is read and parsed again only when its stamp
    /// changed. Whether it did. A local file, never a request.
    pub fn refresh(&mut self) -> bool {
        if self.path.as_os_str().is_empty() {
            return false;
        }
        let stamp = match self.stamp_now() {
            Ok(s) => s,
            Err(e) => {
                let changed = self.error() != Some(&e);
                self.stamp = None;
                self.state = Err(e);
                return changed;
            }
        };
        if self.stamp == Some(stamp) {
            return false;
        }
        self.stamp = Some(stamp);
        self.state = self.read();
        true
    }

    /// One operation under the lock: read and parse the file again
    /// (whatever its stamp says), apply `op`, keep a file with comments in
    /// `marks.toml.bak` first, write atomically (0600), unlock. An invalid
    /// file is never written: its marks would be lost.
    pub fn apply(&mut self, op: MarkOp, now: Timestamp) -> Result<Applied, MarksError> {
        if self.path.as_os_str().is_empty() {
            return Err(MarksError::Io("no marks file in this session".into()));
        }
        if let Some(dir) = self.path.parent() {
            ensure_dir(dir)?;
        }
        let _lock = lock(&self.path)?;
        let text = match fs::read_to_string(&self.path) {
            Ok(t) => Some(t),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(io(&self.path, "read", e)),
        };
        self.parses += 1;
        let mut loaded = match text.as_deref().map(parse) {
            None => Loaded::default(),
            Some(Ok(l)) => l,
            Some(Err(p)) => {
                let e = MarksError::Parse(p);
                self.stamp = self.stamp_now().ok();
                self.state = Err(e.clone());
                return Err(e);
            }
        };
        let mut done = apply_op(&mut loaded, op, now)?;
        if done.ids.is_empty() && done.removed.is_empty() {
            return Ok(done);
        }
        let (set, warnings) = convert(&loaded.file).map_err(MarksError::Parse)?;
        if loaded.comments
            && let Some(old) = &text
        {
            let bak = sibling(&self.path, ".bak");
            write_atomic(&bak, old)?;
            done.backup = Some(bak);
        }
        write_atomic(&self.path, &render(&loaded.file))?;
        self.stamp = Some(self.stamp_now()?);
        self.state = Ok(Loaded {
            file: loaded.file,
            set,
            warnings,
            comments: false,
        });
        Ok(done)
    }
}

```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p illumio-tui --lib marks::`
Expected: PASS (16 tests). The lock test waits the 1 s lock timeout once.

- [ ] **Step 5: Run the gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: no formatting change, no warning, every test passes.

- [ ] **Step 6: Commit**

```bash
git add crates/illumio-tui/src/lib.rs crates/illumio-tui/src/marks.rs
git commit -m "feat(marks): marks.toml: the file, validation, MarksStore (lock, atomic write, backup)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

### Task 4: `illumio-tui marks list|add|rm|export|import`

**Files:**
- Modify: `crates/illumio-tui/src/cli/context.rs`
- Modify: `crates/illumio-tui/src/cli/error.rs`
- Create: `crates/illumio-tui/src/cli/marks.rs`
- Modify: `crates/illumio-tui/src/cli/mod.rs`
- Modify: `crates/illumio-tui/src/cli/output.rs`
- Modify: `crates/illumio-tui/src/cli/profile.rs`
- Modify: `crates/illumio-tui/src/config.rs`
- Test (modify): `crates/illumio-tui/tests/cli.rs`
- Test: `crates/illumio-tui/tests/cli_marks.rs`

**Interfaces:**
- Consumes: Task 3 (`MarksStore`, `MarkOp`, `marks_path`, `entry_of`, `to_mark`, `render`, `MarksError`); Task 1 (`parse_side`, `parse_expiry`, `Action::default_expiry`, …).
- Produces:

```rust
// config.rs: the profile and its PCE without credentials (resolve uses it)
pub struct Identity { pub profile_name: String, pub profile: Profile, pub host: String, pub org_id: u64 }
pub fn identity(file: &ConfigFile, overrides: &Overrides, env: &HashMap<String, String>) -> Result<Identity, ConfigError>;
// cli/context.rs
pub fn config_inputs(global: &GlobalArgs) -> Result<(ConfigFile, Overrides, HashMap<String, String>), CliError>;
// cli/output.rs: Meta derives Default; meta.marks
pub struct MarksMeta { pub active: usize, pub file: String }
// cli/error.rs
CliError::Marks(MarksError)  // Parse and Gone exit 2, Locked 3, Io 1; kinds marks_invalid / marks_locked / marks_gone / io
// cli/marks.rs
pub enum MarksCmd { List(ListArgs), Add(AddArgs), Rm(RmArgs), Export(ExportArgs), Import(ImportArgs) }
pub fn run(global: &GlobalArgs, cmd: &MarksCmd) -> Result<(), CliError>;   // before Context::new: no profile needed
pub fn mark_from_args(args: &AddArgs, profile: Option<&PceId>, now: Timestamp) -> Result<Mark, CliError>;
pub fn read_import(path: &Path) -> Result<Vec<(Mark, toml::Table)>, CliError>;
pub struct MarksJson { pub marks: Vec<MarkEntry> }   // export's and import's JSON
```

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/src/cli/mod.rs`, replace:

```rust
pub mod generate;
pub mod health;
pub mod objects;
pub mod output;
```

with:

```rust
pub mod generate;
pub mod health;
pub mod marks;
pub mod objects;
pub mod output;
```

In `crates/illumio-tui/src/cli/output.rs`, replace:

```rust
            local_filter: None,
            warnings: None,
        };
        assert!(meta_line(&meta).contains("POST /orgs/5/labels → 201"));
```

with:

```rust
            local_filter: None,
            warnings: None,
            marks: None,
        };
        assert!(meta_line(&meta).contains("POST /orgs/5/labels → 201"));
```

In `crates/illumio-tui/tests/cli.rs`, replace:

```rust
        vec!["workloads", "update", "--help"],
        vec!["workloads", "delete", "--help"],
    ] {
        let out = Command::cargo_bin("illumio-tui")
```

with:

```rust
        vec!["workloads", "update", "--help"],
        vec!["workloads", "delete", "--help"],
        vec!["marks", "--help"],
        vec!["marks", "list", "--help"],
        vec!["marks", "add", "--help"],
        vec!["marks", "rm", "--help"],
        vec!["marks", "export", "--help"],
        vec!["marks", "import", "--help"],
    ] {
        let out = Command::cargo_bin("illumio-tui")
```

Create `crates/illumio-tui/tests/cli_marks.rs`:

```rust
//! `illumio-tui marks`: the local marks file through the CLI, in a temp
//! `XDG_CONFIG_HOME`. No PCE: none of these commands sends a request.
use assert_cmd::Command;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn marks(home: &Path) -> Command {
    let mut c = Command::cargo_bin("illumio-tui").unwrap();
    c.env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("XDG_CONFIG_HOME", home)
        .arg("marks");
    c
}

fn file(home: &Path) -> PathBuf {
    home.join("illumio-tui").join("marks.toml")
}

fn json_out(a: assert_cmd::assert::Assert) -> Value {
    serde_json::from_slice(&a.get_output().stdout).expect("stdout is JSON")
}

fn stderr(a: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&a.get_output().stderr).to_string()
}

fn add(home: &Path, args: &[&str]) -> String {
    let out = json_out(marks(home).arg("add").args(args).assert().success());
    out["data"]["id"].as_str().unwrap().to_string()
}

#[test]
fn add_list_rm_round_trip_without_a_profile() {
    let home = tempfile::tempdir().unwrap();
    let a = add(
        home.path(),
        &[
            "--action",
            "squelch",
            "--src",
            "app=monitoring",
            "--dst",
            "any",
            "--service",
            "5666/tcp",
            "--comment",
            "Nagios NRPE, known",
        ],
    );
    assert!(a.starts_with("m-") && a.len() == 8, "{a}");
    let b = add(
        home.path(),
        &[
            "--action",
            "critical",
            "--src",
            "any",
            "--dst",
            "wl:db-01",
            "--pce",
            "pce.example.com",
            "--org",
            "1",
            "--comment",
            "crown jewels",
        ],
    );
    let out = json_out(marks(home.path()).arg("list").assert().success());
    let rows = out["data"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["id"], a.as_str());
    assert_eq!(rows[0]["src"], "app=monitoring");
    assert_eq!(rows[0]["service"], json!(["5666/tcp"]));
    assert_eq!(rows[0]["active"], true);
    assert!(
        rows[0]["expires"].as_str().is_some(),
        "squelch defaults to 90d"
    );
    assert_eq!(rows[1]["dst"], "wl:db-01");
    assert_eq!(rows[1]["pce"], "pce.example.com");
    assert_eq!(rows[1]["org"], 1);
    assert_eq!(
        rows[1]["expires"],
        Value::Null,
        "critical defaults to never"
    );
    assert_eq!(out["meta"]["api_calls"], 0);
    assert_eq!(
        out["meta"]["marks"]["active"], 2,
        "no profile: scope not judged"
    );
    let only = json_out(
        marks(home.path())
            .args(["list", "--action", "critical"])
            .assert()
            .success(),
    );
    assert_eq!(only["data"].as_array().unwrap().len(), 1);
    let text = std::fs::read_to_string(file(home.path())).unwrap();
    assert!(text.starts_with("# illumio-tui marks."), "{text}");
    assert!(
        text.contains("src = { labels = { app = \"monitoring\" } }"),
        "{text}"
    );
    marks(home.path())
        .args(["rm", &b, "m-nope00"])
        .assert()
        .code(2);
    let out = json_out(marks(home.path()).args(["rm", &b]).assert().success());
    assert_eq!(out["data"]["removed"], json!([b]));
    let out = json_out(marks(home.path()).arg("list").assert().success());
    assert_eq!(out["data"].as_array().unwrap().len(), 1);
}

#[test]
fn add_refuses_without_a_comment_and_names_the_bad_field() {
    let home = tempfile::tempdir().unwrap();
    marks(home.path())
        .args(["add", "--action", "squelch", "--src", "any", "--dst", "any"])
        .assert()
        .code(2);
    let a = marks(home.path())
        .args([
            "add",
            "--action",
            "squelch",
            "--src",
            "app=",
            "--dst",
            "any",
            "--comment",
            "x",
        ])
        .assert()
        .code(2);
    assert!(
        stderr(&a).contains("--src: app: empty label value"),
        "{}",
        stderr(&a)
    );
    let a = marks(home.path())
        .args([
            "add",
            "--action",
            "critical",
            "--src",
            "any",
            "--dst",
            "wl:db-01",
            "--comment",
            "x",
        ])
        .assert()
        .code(2);
    assert!(stderr(&a).contains("workload needs pce"), "{}", stderr(&a));
    let a = marks(home.path())
        .args([
            "add",
            "--action",
            "squelch",
            "--src",
            "any",
            "--dst",
            "any",
            "--pce",
            "--comment",
            "x",
        ])
        .assert()
        .code(2);
    assert!(stderr(&a).contains("no profile"), "{}", stderr(&a));
    assert!(!file(home.path()).exists(), "nothing was written");
}

#[test]
fn pce_without_a_host_takes_the_profiles_host() {
    let home = tempfile::tempdir().unwrap();
    let id = json_out(
        marks(home.path())
            .env("PCE_HOST", "pce.lab.example")
            .env("PCE_ORG_ID", "3")
            .args([
                "add",
                "--action",
                "revisit",
                "--src",
                "ip:10.0.0.0/8",
                "--dst",
                "any",
                "--pce",
                "--comment",
                "check",
            ])
            .assert()
            .success(),
    )["data"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let out = json_out(
        marks(home.path())
            .env("PCE_HOST", "other.example")
            .env("PCE_ORG_ID", "3")
            .arg("list")
            .assert()
            .success(),
    );
    assert_eq!(out["data"][0]["id"], id.as_str());
    assert_eq!(out["data"][0]["pce"], "pce.lab.example");
    assert_eq!(out["data"][0]["created_on"], "pce.lab.example");
    assert_eq!(out["data"][0]["active"], false, "scoped to another PCE");
}

#[test]
fn export_then_import_round_trips_and_import_is_refused_atomically() {
    let home = tempfile::tempdir().unwrap();
    add(
        home.path(),
        &[
            "--action",
            "accept-blocked",
            "--src",
            "ipl:rfc1918",
            "--dst",
            "app=shop env=prod|dr",
            "--decision",
            "blocked",
            "--comment",
            "CHG-1182",
        ],
    );
    let out = marks(home.path()).arg("export").assert().success();
    let exported: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    let entry = &exported["marks"][0];
    assert_eq!(entry["action"], "accept-blocked");
    assert_eq!(entry["src"], json!({"ip_list": "rfc1918"}));
    assert_eq!(
        entry["dst"],
        json!({"labels": {"app": "shop", "env": ["prod", "dr"]}})
    );
    let toml_out = marks(home.path())
        .args(["export", "--format", "toml"])
        .assert()
        .success();
    let toml_text = String::from_utf8_lossy(&toml_out.get_output().stdout).to_string();
    assert_eq!(
        toml_text,
        std::fs::read_to_string(file(home.path())).unwrap()
    );

    let other = tempfile::tempdir().unwrap();
    let path = other.path().join("marks.json");
    std::fs::write(&path, serde_json::to_string(&exported).unwrap()).unwrap();
    let res = json_out(
        marks(other.path())
            .arg("import")
            .arg(&path)
            .assert()
            .success(),
    );
    assert_eq!(
        res["data"],
        json!({"added": 1, "replaced": 0, "skipped": []})
    );
    let again = json_out(
        marks(other.path())
            .arg("import")
            .arg(&path)
            .assert()
            .success(),
    );
    assert_eq!(again["data"]["skipped"], json!([entry["id"]]));
    let replaced = json_out(
        marks(other.path())
            .arg("import")
            .arg(&path)
            .arg("--replace")
            .assert()
            .success(),
    );
    assert_eq!(replaced["data"]["replaced"], 1);

    // One bad mark refuses the whole import; nothing is written.
    let before = std::fs::read_to_string(file(other.path())).unwrap();
    let bad = other.path().join("bad.json");
    std::fs::write(
        &bad,
        json!({"marks": [
            {"action": "reviewed", "src": "any", "dst": "any", "comment": "fine"},
            {"action": "reviewed", "src": {"workload": "x"}, "dst": "any", "comment": "no pce"}
        ]})
        .to_string(),
    )
    .unwrap();
    let a = marks(other.path()).arg("import").arg(&bad).assert().code(2);
    assert!(
        stderr(&a).contains("mark 2: workload needs pce"),
        "{}",
        stderr(&a)
    );
    assert_eq!(std::fs::read_to_string(file(other.path())).unwrap(), before);
}

#[test]
fn an_invalid_file_refuses_every_write_and_a_held_lock_exits_3() {
    let home = tempfile::tempdir().unwrap();
    let path = file(home.path());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let bad = "[[mark]]\naction = \"squelch\"\nsrc = \"any\"\ndst = { workload = \"db\" }\ncomment = \"x\"\n";
    std::fs::write(&path, bad).unwrap();
    let args = [
        "add",
        "--action",
        "squelch",
        "--src",
        "any",
        "--dst",
        "any",
        "--comment",
        "x",
    ];
    let a = marks(home.path()).args(args).assert().code(2);
    assert!(
        stderr(&a).contains("mark 1: workload needs pce"),
        "{}",
        stderr(&a)
    );
    marks(home.path()).arg("list").assert().code(2);
    marks(home.path())
        .args(["rm", "--expired"])
        .assert()
        .code(2);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), bad);
    std::fs::remove_file(&path).unwrap();
    std::fs::write(path.with_file_name("marks.toml.lock"), "1").unwrap();
    let a = marks(home.path()).args(args).assert().code(3);
    assert!(
        stderr(&a).contains("locked by another illumio-tui"),
        "{}",
        stderr(&a)
    );
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p illumio-tui --test cli_marks`
Expected: FAIL: it does not compile (`file not found for module marks`: Step 3 creates `cli/marks.rs`).

- [ ] **Step 3: Implement**

In `crates/illumio-tui/src/cli/context.rs`, replace:

```rust
}

impl Context {
    pub fn new(global: &GlobalArgs) -> Result<Self, CliError> {
        let path = global
            .config
            .clone()
            .unwrap_or_else(config::default_config_path);
        let file = config::load_config_file(&path)?;
        let real: HashMap<String, String> = std::env::vars().collect();
        let env = match &global.env_file {
            Some(p) => config::merge_env(config::load_env_file(p)?, real),
            None => real,
        };
        let overrides = Overrides {
            profile: global.profile.clone(),
            host: global.host.clone(),
            port: global.pce_port,
            org_id: global.org_id,
            tls_verify: if global.insecure { Some(false) } else { None },
        };
        let resolved = config::resolve(&file, &overrides, &env)?;
        let pce = Pce::new(PceClient::new(resolved.client_config())?)
```

with:

```rust
}

/// What a profile is resolved from: the config file, the flags and the
/// environment (with `--env-file`).
pub fn config_inputs(
    global: &GlobalArgs,
) -> Result<(config::ConfigFile, Overrides, HashMap<String, String>), CliError> {
    let path = global
        .config
        .clone()
        .unwrap_or_else(config::default_config_path);
    let file = config::load_config_file(&path)?;
    let real: HashMap<String, String> = std::env::vars().collect();
    let env = match &global.env_file {
        Some(p) => config::merge_env(config::load_env_file(p)?, real),
        None => real,
    };
    let overrides = Overrides {
        profile: global.profile.clone(),
        host: global.host.clone(),
        port: global.pce_port,
        org_id: global.org_id,
        tls_verify: if global.insecure { Some(false) } else { None },
    };
    Ok((file, overrides, env))
}

impl Context {
    pub fn new(global: &GlobalArgs) -> Result<Self, CliError> {
        let (file, overrides, env) = config_inputs(global)?;
        let resolved = config::resolve(&file, &overrides, &env)?;
        let pce = Pce::new(PceClient::new(resolved.client_config())?)
```

In `crates/illumio-tui/src/cli/context.rs`, replace:

```rust
            local_filter: None,
            warnings: None,
        }
    }
```

with:

```rust
            local_filter: None,
            warnings: None,
            marks: None,
        }
    }
```

In `crates/illumio-tui/src/cli/error.rs`, replace:

```rust
//! 4 PCE error, 5 rate limited, 6 write refused.
use crate::config::ConfigError;
use pce::PceError;
use pce::write::WriteError;
```

with:

```rust
//! 4 PCE error, 5 rate limited, 6 write refused.
use crate::config::ConfigError;
use crate::marks::MarksError;
use pce::PceError;
use pce::write::WriteError;
```

In `crates/illumio-tui/src/cli/error.rs`, replace:

```rust
    #[error("{0}")]
    Incomplete(String),
}

```

with:

```rust
    #[error("{0}")]
    Incomplete(String),
    /// `marks.toml`: invalid (exit 2, every problem), locked by another
    /// writer (exit 3), a mark gone (exit 2), unreadable (exit 1).
    #[error("{}", .0.lines().join("; "))]
    Marks(crate::marks::MarksError),
}

```

In `crates/illumio-tui/src/cli/error.rs`, replace:

```rust
            CliError::Write(e) => write_exit_code(e),
            CliError::Io(_) | CliError::Other(_) => 1,
        }
    }
```

with:

```rust
            CliError::Write(e) => write_exit_code(e),
            CliError::Io(_) | CliError::Other(_) => 1,
            CliError::Marks(e) => match e {
                MarksError::Parse(_) | MarksError::Gone(_) => 2,
                MarksError::Locked => 3,
                MarksError::Io(_) => 1,
            },
        }
    }
```

In `crates/illumio-tui/src/cli/error.rs`, replace:

```rust
            CliError::Other(_) => "error".into(),
            CliError::Incomplete(_) => "incomplete".into(),
        }
    }
```

with:

```rust
            CliError::Other(_) => "error".into(),
            CliError::Incomplete(_) => "incomplete".into(),
            CliError::Marks(e) => match e {
                MarksError::Parse(_) => "marks_invalid".into(),
                MarksError::Locked => "marks_locked".into(),
                MarksError::Gone(_) => "marks_gone".into(),
                MarksError::Io(_) => "io".into(),
            },
        }
    }
```

Create `crates/illumio-tui/src/cli/marks.rs`:

```rust
//! `illumio-tui marks list|add|rm|export|import`: the local marks file.
//! No request: `list`, `export` and `rm` need no profile; `add --pce`
//! without a host and the `active` column use the profile's host when one
//! resolves (no secret needed).
use super::GlobalArgs;
use super::context::config_inputs;
use super::error::CliError;
use super::output::{
    MarksMeta, Meta, OutputFormat, TableRows, choose_format, meta_line, write_json, write_table,
    write_table_csv,
};
use crate::config;
use crate::marks::{
    MarkEntry, MarkFile, MarkOp, MarksStore, entry_of, marks_path, render, to_mark,
};
use clap::{Args, Subcommand, ValueEnum};
use jiff::Timestamp;
use pce::marks::{
    Action, Mark, PceId, PceScope, ServiceSpec, parse_decision, parse_expiry, parse_side, side_text,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Subcommand, Debug)]
pub enum MarksCmd {
    /// List the marks in marks.toml, active or not (no profile needed)
    List(ListArgs),
    /// Add a mark; prints its id. Portable unless --pce is given
    Add(AddArgs),
    /// Remove marks by id (all or none), or every expired one
    Rm(RmArgs),
    /// Print every mark to stdout: JSON ({"marks": [...]}, what import reads) or TOML
    Export(ExportArgs),
    /// Add the marks of a JSON or TOML file (by extension); existing ids are skipped
    Import(ImportArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct ListArgs {
    /// Only this action: critical, important, revisit, accept-blocked, reviewed, squelch
    #[arg(long)]
    pub action: Option<String>,
    /// Only active marks (unexpired, in scope for the profile's PCE)
    #[arg(long, conflicts_with = "expired")]
    pub active: bool,
    /// Only expired marks
    #[arg(long)]
    pub expired: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct AddArgs {
    /// critical, important, revisit, accept-blocked, reviewed or squelch
    #[arg(long)]
    pub action: String,
    /// Source side: any, key=value (| = or), wl:HOST, ip:CIDR, ipl:NAME
    #[arg(long)]
    pub src: String,
    /// Destination side, same syntax as --src
    #[arg(long)]
    pub dst: String,
    /// 5666/tcp, 8000-8010/tcp, udp, icmp or a protocol number (repeatable or
    /// comma-separated: any of them); default any service
    #[arg(long, value_delimiter = ',')]
    pub service: Vec<String>,
    /// allowed, potentially_blocked, blocked (repeatable or comma-separated);
    /// matched against the reported decision
    #[arg(long, value_delimiter = ',')]
    pub decision: Vec<String>,
    /// Same values, matched against the draft decision (traffic --draft only)
    #[arg(long, value_delimiter = ',')]
    pub draft_decision: Vec<String>,
    /// 1d, 7d, 30d, 90d, YYYY-MM-DD (23:59:59 UTC), RFC 3339 or never;
    /// default by action: squelch and accept-blocked 90d, reviewed 30d,
    /// revisit 7d, critical and important never
    #[arg(long)]
    pub expires: Option<String>,
    /// Limit the mark to a PCE host; without a value, the profile's host
    #[arg(long, num_args = 0..=1)]
    pub pce: Option<Option<String>>,
    /// Limit the mark to an org id too (with --pce)
    #[arg(long, requires = "pce")]
    pub org: Option<u64>,
    /// Why the flows are marked (required): a ticket, the owner's answer
    #[arg(long)]
    pub comment: String,
}

#[derive(Args, Debug, Clone, Default)]
pub struct RmArgs {
    /// Mark ids (m-…, h-…); an unknown one removes nothing
    #[arg(required_unless_present = "expired")]
    pub ids: Vec<String>,
    /// Every expired mark
    #[arg(long, conflicts_with = "ids")]
    pub expired: bool,
}

#[derive(ValueEnum, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ExportFormat {
    #[default]
    Json,
    Toml,
}

#[derive(Args, Debug, Clone, Default)]
pub struct ExportArgs {
    /// json (default) or toml
    #[arg(long, value_enum, default_value_t)]
    pub format: ExportFormat,
}

#[derive(Args, Debug, Clone, Default)]
pub struct ImportArgs {
    /// A .json file ({"marks": [...]}, as export writes it) or a .toml file
    pub file: PathBuf,
    /// Replace marks whose id exists instead of skipping them
    #[arg(long)]
    pub replace: bool,
}

/// `export`'s and `import`'s JSON: the TOML field names.
#[derive(Debug, Serialize, Deserialize)]
pub struct MarksJson {
    pub marks: Vec<MarkEntry>,
}

/// The profile's PCE when one resolves (no secret needed), and its name.
fn profile_pce(global: &GlobalArgs) -> Option<(String, PceId)> {
    let (file, overrides, env) = config_inputs(global).ok()?;
    let id = config::identity(&file, &overrides, &env).ok()?;
    Some((
        id.profile_name,
        PceId {
            host: id.host,
            org: id.org_id,
        },
    ))
}

fn store(global: &GlobalArgs) -> MarksStore {
    MarksStore::open(marks_path(global.config.as_deref()))
}

fn emit<T: Serialize + TableRows>(
    global: &GlobalArgs,
    data: &T,
    meta: &Meta,
) -> Result<(), CliError> {
    let mut out = std::io::stdout().lock();
    match choose_format(global.json, global.table, global.csv) {
        OutputFormat::Json => write_json(&mut out, data, meta)?,
        OutputFormat::Table => {
            write_table(&mut out, data)?;
            eprintln!("{}", meta_line(meta));
        }
        OutputFormat::Csv => {
            write_table_csv(&mut out, data)?;
            eprintln!("{}", meta_line(meta));
        }
    }
    Ok(())
}

fn meta(profile: Option<&str>, marks: Option<MarksMeta>) -> Meta {
    Meta {
        profile: profile.unwrap_or_default().to_string(),
        cache: "none",
        marks,
        ..Meta::default()
    }
}

/// One `list` row: the TOML field names, sides in the side grammar.
#[derive(Debug, Serialize)]
pub struct ListRow {
    pub id: String,
    pub action: Action,
    pub src: String,
    pub dst: String,
    pub service: Vec<String>,
    pub decision: Vec<String>,
    pub draft_decision: Vec<String>,
    pub pce: Option<String>,
    pub org: Option<u64>,
    pub expires: Option<String>,
    pub active: bool,
    pub comment: String,
    pub created: Option<String>,
    pub created_on: Option<String>,
    /// The mark's unknown fields.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

impl ListRow {
    fn new(m: &Mark, extra: &toml::Table, active: bool) -> Self {
        let ds = |d: &[pce::models::PolicyDecision]| d.iter().map(|d| d.as_str().into()).collect();
        Self {
            id: m.id.clone(),
            action: m.action,
            src: side_text(&m.src),
            dst: side_text(&m.dst),
            service: m.services.iter().map(ServiceSpec::text).collect(),
            decision: ds(&m.decisions),
            draft_decision: ds(&m.draft_decisions),
            pce: m.scope.as_ref().map(|s| s.host.clone()),
            org: m.scope.as_ref().and_then(|s| s.org),
            expires: m.expires.map(|t| t.to_string()),
            active,
            comment: m.comment.clone(),
            created: m.created.map(|t| t.to_string()),
            created_on: m.created_on.clone(),
            extra: match serde_json::to_value(extra) {
                Ok(Value::Object(o)) => o,
                _ => serde_json::Map::new(),
            },
        }
    }
}

impl TableRows for Vec<ListRow> {
    fn headers(&self) -> Vec<&'static str> {
        vec![
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
        ]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let opt = |v: &Option<String>| v.clone().unwrap_or_default();
        self.iter()
            .map(|r| {
                vec![
                    r.id.clone(),
                    r.action.as_str().into(),
                    r.src.clone(),
                    r.dst.clone(),
                    r.service.join(", "),
                    r.decision.join("|"),
                    r.draft_decision.join("|"),
                    opt(&r.pce),
                    r.org.map(|o| o.to_string()).unwrap_or_default(),
                    r.expires.clone().unwrap_or_else(|| "never".into()),
                    r.active.to_string(),
                    r.comment.clone(),
                    opt(&r.created),
                    opt(&r.created_on),
                ]
            })
            .collect()
    }
}

/// `{"id": "m-…"}`: what `add` prints.
#[derive(Debug, Serialize)]
pub struct Added {
    pub id: String,
}

impl TableRows for Added {
    fn headers(&self) -> Vec<&'static str> {
        vec!["id"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        vec![vec![self.id.clone()]]
    }
}

#[derive(Debug, Serialize)]
pub struct Removed {
    pub removed: Vec<String>,
}

impl TableRows for Removed {
    fn headers(&self) -> Vec<&'static str> {
        vec!["removed"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.removed.iter().map(|id| vec![id.clone()]).collect()
    }
}

#[derive(Debug, Serialize)]
pub struct Imported {
    pub added: usize,
    pub replaced: usize,
    pub skipped: Vec<String>,
}

impl TableRows for Imported {
    fn headers(&self) -> Vec<&'static str> {
        vec!["added", "replaced", "skipped"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        vec![vec![
            self.added.to_string(),
            self.replaced.to_string(),
            self.skipped.join(", "),
        ]]
    }
}

pub fn run(global: &GlobalArgs, cmd: &MarksCmd) -> Result<(), CliError> {
    match cmd {
        MarksCmd::List(a) => list(global, a),
        MarksCmd::Add(a) => add(global, a),
        MarksCmd::Rm(a) => rm(global, a),
        MarksCmd::Export(a) => export(global, a),
        MarksCmd::Import(a) => import(global, a),
    }
}

fn list(global: &GlobalArgs, args: &ListArgs) -> Result<(), CliError> {
    let action =
        match &args.action {
            Some(a) => Some(Action::parse(a).ok_or_else(|| {
                CliError::Args(format!("unknown action {a:?}: {}", Action::names()))
            })?),
            None => None,
        };
    let s = store(global);
    let loaded = s.loaded().ok_or_else(|| marks_error(s.error()))?;
    let now = Timestamp::now();
    let profile = profile_pce(global);
    let pce = profile.as_ref().map(|(_, p)| p);
    let rows: Vec<ListRow> = loaded
        .set
        .marks
        .iter()
        .zip(&loaded.file.mark)
        .filter(|(m, _)| action.is_none_or(|a| m.action == a))
        .filter(|(m, _)| !args.expired || m.is_expired(now))
        .map(|(m, e)| ListRow::new(m, &e.extra, !m.is_expired(now) && m.in_scope(pce)))
        .filter(|r| !args.active || r.active)
        .collect();
    for w in s.warnings() {
        eprintln!("warning: {w}");
    }
    let active = loaded
        .set
        .marks
        .iter()
        .filter(|m| !m.is_expired(now) && m.in_scope(pce))
        .count();
    let meta = meta(
        profile.as_ref().map(|(n, _)| n.as_str()),
        Some(MarksMeta {
            active,
            file: s.path().display().to_string(),
        }),
    );
    emit(global, &rows, &meta)
}

/// The store's problems (an invalid or unreadable file) as an exit-2 or
/// I/O error.
fn marks_error(e: Option<&crate::marks::MarksError>) -> CliError {
    match e {
        Some(e) => CliError::Marks(e.clone()),
        None => CliError::Other("marks.toml: no marks loaded".into()),
    }
}

/// The mark `add` describes, checked as the file checks it.
pub fn mark_from_args(
    args: &AddArgs,
    profile: Option<&PceId>,
    now: Timestamp,
) -> Result<Mark, CliError> {
    let bad = |flag: &str, e: String| CliError::Args(format!("--{flag}: {e}"));
    let action = Action::parse(&args.action).ok_or_else(|| {
        bad(
            "action",
            format!("unknown action {:?}: {}", args.action, Action::names()),
        )
    })?;
    let src = parse_side(&args.src).map_err(|e| bad("src", e))?;
    let dst = parse_side(&args.dst).map_err(|e| bad("dst", e))?;
    let services = args
        .service
        .iter()
        .map(|s| ServiceSpec::parse(s).map_err(|e| bad("service", e)))
        .collect::<Result<_, _>>()?;
    let decisions = |flag: &str, list: &[String]| {
        list.iter()
            .map(|d| parse_decision(d).map_err(|e| bad(flag, e)))
            .collect::<Result<Vec<_>, _>>()
    };
    let expires = parse_expiry(
        args.expires.as_deref().unwrap_or(action.default_expiry()),
        now,
    )
    .map_err(|e| bad("expires", e))?;
    let host = match &args.pce {
        None => None,
        Some(Some(h)) if !h.trim().is_empty() => Some(h.trim().to_string()),
        Some(_) => Some(profile.map(|p| p.host.clone()).ok_or_else(|| {
            bad(
                "pce",
                "no profile resolves to take the host from: write --pce HOST".into(),
            )
        })?),
    };
    if args.comment.trim().is_empty() {
        return Err(bad(
            "comment",
            "required: why are these flows marked?".into(),
        ));
    }
    let m = Mark {
        id: String::new(),
        action,
        src,
        dst,
        services,
        decisions: decisions("decision", &args.decision)?,
        draft_decisions: decisions("draft-decision", &args.draft_decision)?,
        scope: host.map(|host| PceScope {
            host,
            org: args.org,
        }),
        expires,
        comment: args.comment.trim().to_string(),
        created: Some(now),
        created_on: profile.map(|p| p.host.clone()),
    };
    if let Some(p) = m.problems().into_iter().next() {
        return Err(CliError::Args(p));
    }
    Ok(m)
}

fn add(global: &GlobalArgs, args: &AddArgs) -> Result<(), CliError> {
    let now = Timestamp::now();
    let profile = profile_pce(global);
    let m = mark_from_args(args, profile.as_ref().map(|(_, p)| p), now)?;
    let mut s = store(global);
    let done = s.apply(MarkOp::Add(m), now).map_err(CliError::Marks)?;
    if let Some(b) = &done.backup {
        eprintln!("marks.toml had comments: kept in {}", b.display());
    }
    let id = done.ids.into_iter().next().unwrap_or_default();
    emit(
        global,
        &Added { id },
        &meta(profile.as_ref().map(|(n, _)| n.as_str()), None),
    )
}

fn rm(global: &GlobalArgs, args: &RmArgs) -> Result<(), CliError> {
    let mut s = store(global);
    let op = if args.expired {
        MarkOp::DeleteExpired
    } else {
        MarkOp::Delete(args.ids.clone())
    };
    let done = s.apply(op, Timestamp::now()).map_err(|e| match e {
        crate::marks::MarksError::Gone(id) => {
            CliError::Args(format!("unknown mark id {id}: nothing removed"))
        }
        e => CliError::Marks(e),
    })?;
    emit(
        global,
        &Removed {
            removed: done.removed,
        },
        &meta(None, None),
    )
}

/// Every mark as the entry that writes it (ids filled in), unknown
/// fields kept.
fn entries(s: &MarksStore) -> Result<MarkFile, CliError> {
    let loaded = s.loaded().ok_or_else(|| marks_error(s.error()))?;
    Ok(MarkFile {
        mark: loaded
            .set
            .marks
            .iter()
            .zip(&loaded.file.mark)
            .map(|(m, e)| entry_of(m, e.extra.clone()))
            .collect(),
        extra: loaded.file.extra.clone(),
    })
}

fn export(global: &GlobalArgs, args: &ExportArgs) -> Result<(), CliError> {
    let file = entries(&store(global))?;
    let text = match args.format {
        ExportFormat::Toml => render(&file),
        ExportFormat::Json => {
            let mut t = serde_json::to_string_pretty(&MarksJson { marks: file.mark })
                .map_err(|e| CliError::Other(e.to_string()))?;
            t.push('\n');
            t
        }
    };
    print!("{text}");
    Ok(())
}

/// The marks of an import file, every one checked before anything is
/// written; problems name the mark's position in that file.
pub fn read_import(path: &std::path::Path) -> Result<Vec<(Mark, toml::Table)>, CliError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| CliError::Args(format!("cannot read {}: {e}", path.display())))?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let entries: Vec<MarkEntry> = match ext {
        "json" => {
            serde_json::from_str::<MarksJson>(&text)
                .map_err(|e| CliError::Args(format!("{}: {e}", path.display())))?
                .marks
        }
        "toml" => {
            toml::from_str::<MarkFile>(&text)
                .map_err(|e| CliError::Args(format!("{}: {}", path.display(), e.message())))?
                .mark
        }
        _ => {
            return Err(CliError::Args(format!(
                "{}: import reads a .json or a .toml file",
                path.display()
            )));
        }
    };
    let mut problems = Vec::new();
    let mut out = Vec::new();
    for (i, e) in entries.into_iter().enumerate() {
        match to_mark(&e) {
            Ok(m) => out.push((m, e.extra)),
            Err(msgs) => problems.extend(msgs.into_iter().map(|m| {
                let id = e.id_text().map(|id| format!(" ({id})")).unwrap_or_default();
                format!("mark {}{id}: {m}", i + 1)
            })),
        }
    }
    if problems.is_empty() {
        Ok(out)
    } else {
        Err(CliError::Args(format!(
            "import refused, nothing written: {}",
            problems.join("; ")
        )))
    }
}

fn import(global: &GlobalArgs, args: &ImportArgs) -> Result<(), CliError> {
    let marks = read_import(&args.file)?;
    let mut s = store(global);
    let done = s
        .apply(
            MarkOp::Import {
                marks,
                replace: args.replace,
            },
            Timestamp::now(),
        )
        .map_err(CliError::Marks)?;
    emit(
        global,
        &Imported {
            added: done.added,
            replaced: done.replaced,
            skipped: done.skipped,
        },
        &meta(None, None),
    )
}
```

In `crates/illumio-tui/src/cli/mod.rs`, replace:

```rust
    #[command(subcommand)]
    Profile(profile::ProfileCmd),
}

```

with:

```rust
    #[command(subcommand)]
    Profile(profile::ProfileCmd),
    /// Flow marks in marks.toml: squelch, reviewed, critical … (local file, no request)
    ///
    /// Marks are applied to every traffic result (traffic, app; gen reports them): squelched
    /// flows are left out, the others carry their mark. --no-marks ignores the file.
    #[command(subcommand)]
    Marks(marks::MarksCmd),
}

```

In `crates/illumio-tui/src/cli/mod.rs`, replace:

```rust
        Command::Traffic(t) if t.filter().dry_run => Some("traffic --dry-run"),
        Command::Profile(profile::ProfileCmd::Use { .. }) => Some("profile use"),
        _ => None,
    }
```

with:

```rust
        Command::Traffic(t) if t.filter().dry_run => Some("traffic --dry-run"),
        Command::Profile(profile::ProfileCmd::Use { .. }) => Some("profile use"),
        Command::Marks(marks::MarksCmd::Export(_)) => Some("marks export"),
        _ => None,
    }
```

In `crates/illumio-tui/src/cli/mod.rs`, replace:

```rust
        return profile::run(&global, p);
    }
    let ctx = Context::new(&global)?;
    match command {
```

with:

```rust
        return profile::run(&global, p);
    }
    if let Command::Marks(m) = &command {
        return marks::run(&global, m);
    }
    let ctx = Context::new(&global)?;
    match command {
```

In `crates/illumio-tui/src/cli/mod.rs`, replace:

```rust
        Command::Policy(p) => policy::run(&ctx, &p).await,
        Command::Traffic(t) => traffic::run(&ctx, &t).await,
        Command::Profile(_) => unreachable!("handled above"),
    }
}
```

with:

```rust
        Command::Policy(p) => policy::run(&ctx, &p).await,
        Command::Traffic(t) => traffic::run(&ctx, &t).await,
        Command::Profile(_) | Command::Marks(_) => unreachable!("handled above"),
    }
}
```

In `crates/illumio-tui/src/cli/output.rs`, replace:

```rust
pub const USAGE_FROM_COUNTS: &str = "workloads, rules and scopes only";

#[derive(Debug, Clone, Serialize)]
pub struct Meta {
    pub profile: String,
```

with:

```rust
pub const USAGE_FROM_COUNTS: &str = "workloads, rules and scopes only";

#[derive(Debug, Clone, Default, Serialize)]
pub struct Meta {
    pub profile: String,
```

In `crates/illumio-tui/src/cli/output.rs`, replace:

```rust
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<String>>,
}

```

with:

```rust
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<String>>,
    /// The marks applied (`marks` commands, and traffic with a valid file
    /// holding an active mark): how many are active, and the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marks: Option<MarksMeta>,
}

/// `meta.marks`.
#[derive(Debug, Clone, Serialize)]
pub struct MarksMeta {
    pub active: usize,
    pub file: String,
}

```

In `crates/illumio-tui/src/cli/profile.rs`, replace:

```rust
        local_filter: None,
        warnings: None,
    };
    let mut out = std::io::stdout().lock();
```

with:

```rust
        local_filter: None,
        warnings: None,
        marks: None,
    };
    let mut out = std::io::stdout().lock();
```

In `crates/illumio-tui/src/config.rs`, replace:

```rust
}

pub fn resolve(
    file: &ConfigFile,
    overrides: &Overrides,
    env: &HashMap<String, String>,
) -> Result<ResolvedConfig, ConfigError> {
    let env_str = |k: &str| {
        env.get(k)
```

with:

```rust
}

/// The profile `resolve` uses, and its PCE: the host and org id from the
/// same flags, environment and file, without credentials (`marks` scopes
/// marks to it and needs no secret).
#[derive(Debug, Clone)]
pub struct Identity {
    pub profile_name: String,
    pub profile: Profile,
    pub host: String,
    pub org_id: u64,
}

pub fn identity(
    file: &ConfigFile,
    overrides: &Overrides,
    env: &HashMap<String, String>,
) -> Result<Identity, ConfigError> {
    let env_str = |k: &str| {
        env.get(k)
```

In `crates/illumio-tui/src/config.rs`, replace:

```rust
            )
        })?;
    let port = if let Some(p) = overrides.port {
        p
    } else if let Some(raw) = env_str("PCE_PORT") {
        raw.parse::<u16>()
            .map_err(|_| ConfigError::Message(format!("PCE_PORT is not a number: {raw:?}")))?
    } else {
        profile.port.unwrap_or(443)
    };
    let org_id = if let Some(o) = overrides.org_id {
        o
```

with:

```rust
            )
        })?;
    let org_id = if let Some(o) = overrides.org_id {
        o
```

In `crates/illumio-tui/src/config.rs`, replace:

```rust
            )
        })?
    };
    let scheme = env_str("PCE_SCHEME")
```

with:

```rust
            )
        })?
    };
    Ok(Identity {
        profile_name,
        profile,
        host,
        org_id,
    })
}

pub fn resolve(
    file: &ConfigFile,
    overrides: &Overrides,
    env: &HashMap<String, String>,
) -> Result<ResolvedConfig, ConfigError> {
    let env_str = |k: &str| {
        env.get(k)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };

    let Identity {
        profile_name,
        profile,
        host,
        org_id,
    } = identity(file, overrides, env)?;
    let port = if let Some(p) = overrides.port {
        p
    } else if let Some(raw) = env_str("PCE_PORT") {
        raw.parse::<u16>()
            .map_err(|_| ConfigError::Message(format!("PCE_PORT is not a number: {raw:?}")))?
    } else {
        profile.port.unwrap_or(443)
    };
    let scheme = env_str("PCE_SCHEME")
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p illumio-tui --test cli_marks && cargo test -p illumio-tui --test cli every_flag_has_help_text && cargo test -p illumio-tui --lib config`
Expected: PASS.

- [ ] **Step 5: Run the gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: no formatting change, no warning, every test passes.

- [ ] **Step 6: Commit**

```bash
git add crates/illumio-tui/src/cli/context.rs crates/illumio-tui/src/cli/error.rs crates/illumio-tui/src/cli/marks.rs crates/illumio-tui/src/cli/mod.rs crates/illumio-tui/src/cli/output.rs crates/illumio-tui/src/cli/profile.rs crates/illumio-tui/src/config.rs crates/illumio-tui/tests/cli.rs crates/illumio-tui/tests/cli_marks.rs
git commit -m "feat(cli): illumio-tui marks list|add|rm|export|import

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

### Task 5: Traffic commands and `app` apply marks; `gen` counts them

**Files:**
- Modify: `crates/illumio-tui/src/cli/app.rs`
- Modify: `crates/illumio-tui/src/cli/context.rs`
- Modify: `crates/illumio-tui/src/cli/generate.rs`
- Create: `crates/illumio-tui/src/cli/marked.rs`
- Modify: `crates/illumio-tui/src/cli/mod.rs`
- Modify: `crates/illumio-tui/src/cli/output.rs`
- Modify: `crates/illumio-tui/src/cli/profile.rs`
- Modify: `crates/illumio-tui/src/cli/traffic.rs`
- Test: `crates/illumio-tui/tests/cli_marks_traffic.rs`

**Interfaces:**
- Consumes: Tasks 1-4 (`partition`, `annotate_*`, `MarksStore`, `marks_path`, `Meta::default`, `MarksMeta`).
- Produces:

```rust
// cli/output.rs
pub struct SquelchedMeta { pub rows: Option<u64>, pub flows: u64 }   // Meta.squelched; meta_line " · 4 rows (9 flows) squelched" / " · 52 squelched (rules include them)"
// cli/context.rs
pub marks_path: PathBuf  // Context field
// cli/marked.rs
pub struct CliMarks { pub show: bool, .. }
impl CliMarks { fn load(ctx: &Context, no_marks: bool, show: bool) -> Self; fn active(&self, draft: bool) -> Option<ActiveMarks<'_>>;
                fn fill(&self, meta: &mut Meta, active: Option<&ActiveMarks>, sq: Option<SquelchedMeta>) }
pub fn squelched(part: Option<&Partition>, active: Option<&ActiveMarks>, annotate: impl FnOnce(&Marked, &ActiveMarks) -> u64,
                 pred: impl Fn(&ResolvedFlow) -> bool) -> Option<SquelchedMeta>;
pub struct FlowMark { pub action: Action, pub id: String }       // fn of(Option<&Hit>, &ActiveMarks) -> Option<FlowMark>
pub struct MarkedFlow<'a> { #[serde(flatten)] pub flow: &'a ResolvedFlow, pub mark: Option<&'a FlowMark> }
pub fn mark_cell(m: Option<&RowMarks>) -> String;                 // the `mark` column: action, "~12", or empty
```

`FilterArgs` (every `traffic` command) and `AppArgs` gain `--show-squelched` and `--no-marks`; `GenArgs` gains `--no-marks`.

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/src/cli/mod.rs`, replace:

```rust
pub mod generate;
pub mod health;
pub mod marks;
pub mod objects;
```

with:

```rust
pub mod generate;
pub mod health;
pub mod marked;
pub mod marks;
pub mod objects;
```

In `crates/illumio-tui/src/cli/output.rs`, replace:

```rust
            warnings: None,
            marks: None,
        };
        assert!(meta_line(&meta).contains("POST /orgs/5/labels → 201"));
    }
}
```

with:

```rust
            warnings: None,
            marks: None,
            squelched: None,
        };
        assert!(meta_line(&meta).contains("POST /orgs/5/labels → 201"));
        let sq = |rows| Meta {
            squelched: Some(SquelchedMeta { rows, flows: 52 }),
            ..Meta::default()
        };
        assert!(meta_line(&sq(Some(3))).ends_with(" · 3 rows (52 flows) squelched"));
        assert!(meta_line(&sq(None)).ends_with(" · 52 squelched (rules include them)"));
    }
}
```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
            flows: &flows,
            order: idx.key_order(),
        };
        let headers = table.headers();
```

with:

```rust
            flows: &flows,
            order: idx.key_order(),
            marks: None,
        };
        let headers = table.headers();
```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
            flows: &flows,
            order: &order,
        };
        let headers = table.headers();
```

with:

```rust
            flows: &flows,
            order: &order,
            marks: None,
        };
        let headers = table.headers();
```

Create `crates/illumio-tui/tests/cli_marks_traffic.rs`:

```rust
//! Traffic commands apply marks.toml: squelched records left out (or kept
//! with `--show-squelched`), `mark` on rows, `meta.squelched`, an invalid
//! file a warning; `gen` only counts them. The 4-flow fixture through a
//! mock PCE, marks in a temp `XDG_CONFIG_HOME`. The helpers repeat those of
//! cli.rs (test files share no code without a module).
use assert_cmd::Command;
use serde_json::{Value, json};
use std::path::Path;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const QHREF: &str = "/orgs/5/traffic_flows/async_queries/q-1";

/// The fixture's flows: web → db 5432 (allowed), Corp → web 8443
/// (potentially blocked), unmanaged-nas → db icmp (blocked), web → an
/// FQDN 443 (allowed). Corp is squelched, both flows into db are
/// critical, 443 is reviewed.
const MARKS: &str = r#"
[[mark]]
id = "m-sqcorp"
action = "squelch"
src = { ip_list = "Corp" }
dst = "any"
comment = "scanner"

[[mark]]
id = "m-crit01"
action = "critical"
src = "any"
dst = { ip = "10.0.2.20" }
comment = "the database"

[[mark]]
id = "m-rev443"
action = "reviewed"
src = "any"
dst = "any"
service = "443/tcp"
comment = "egress, known"
"#;

fn cmd(server: &MockServer, home: &Path) -> Command {
    let url = url::Url::parse(&server.uri()).unwrap();
    let mut c = Command::cargo_bin("illumio-tui").unwrap();
    c.env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("PCE_SCHEME", "http")
        .env("PCE_HOST", url.host_str().unwrap())
        .env("PCE_PORT", url.port().unwrap().to_string())
        .env("PCE_ORG_ID", "5")
        .env("API_KEY", "key")
        .env("API_SECRET", "s3cr3t-marks-test")
        .env("XDG_CONFIG_HOME", home);
    c
}

fn home_with(marks: &str) -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("illumio-tui");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("marks.toml"), marks).unwrap();
    home
}

fn json_out(a: assert_cmd::assert::Assert) -> Value {
    serde_json::from_slice(&a.get_output().stdout).expect("stdout is JSON")
}

/// The object lists, the traffic query (any POST), the draft rulesets.
async fn mount(server: &MockServer) {
    for (p, body) in [
        (
            "/api/v2/orgs/5/labels",
            include_str!("../../../fixtures/labels.json"),
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/label_groups",
            include_str!("../../../fixtures/label_groups.json"),
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/ip_lists",
            include_str!("../../../fixtures/ip_lists.json"),
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/services",
            include_str!("../../../fixtures/services.json"),
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/rule_sets",
            include_str!("../../../fixtures/rule_sets.json"),
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(body)
                    .insert_header("Content-Type", "application/json"),
            )
            .mount(server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .respond_with(
            ResponseTemplate::new(202).set_body_json(json!({"href": QHREF, "status": "queued"})),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "completed",
            "result": {"href": format!("{QHREF}/download")},
            "matches_count": 4
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}/download")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/traffic_flows.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .mount(server)
        .await;
}

async fn requests(server: &MockServer) -> usize {
    server.received_requests().await.unwrap().len()
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_query_leaves_out_squelched_flows_and_marks_the_rest() {
    let server = MockServer::start().await;
    mount(&server).await;
    let home = home_with(MARKS);
    let q = ["traffic", "query", "--window", "1h"];
    let out = json_out(cmd(&server, home.path()).args(q).assert().success());
    let marked_run = requests(&server).await;
    let flows = out["data"].as_array().unwrap();
    assert_eq!(flows.len(), 3, "Corp's flow is squelched");
    let marks: Vec<&Value> = flows.iter().map(|f| &f["mark"]).collect();
    assert_eq!(
        marks,
        vec![
            &json!({"action": "critical", "id": "m-crit01"}),
            &json!({"action": "critical", "id": "m-crit01"}),
            &json!({"action": "reviewed", "id": "m-rev443"}),
        ]
    );
    assert_eq!(
        flows[0]["src"]["hostname"], "host-1.example",
        "the flow itself, flattened"
    );
    assert_eq!(out["meta"]["squelched"], json!({"rows": 1, "flows": 1}));
    assert_eq!(out["meta"]["marks"]["active"], 3);
    assert!(
        out["meta"]["marks"]["file"]
            .as_str()
            .unwrap()
            .ends_with("marks.toml")
    );

    let shown = json_out(
        cmd(&server, home.path())
            .args(q)
            .arg("--show-squelched")
            .assert()
            .success(),
    );
    let flows = shown["data"].as_array().unwrap();
    assert_eq!(flows.len(), 4);
    assert_eq!(
        flows[1]["mark"],
        json!({"action": "squelch", "id": "m-sqcorp"})
    );
    assert_eq!(shown["meta"]["squelched"], json!({"rows": 1, "flows": 1}));

    let before = requests(&server).await;
    let off = json_out(
        cmd(&server, home.path())
            .args(q)
            .arg("--no-marks")
            .assert()
            .success(),
    );
    assert_eq!(
        requests(&server).await - before,
        marked_run,
        "marks add no request"
    );
    assert_eq!(off["data"].as_array().unwrap().len(), 4);
    assert!(off["data"][0].get("mark").is_none());
    assert!(off["meta"].get("squelched").is_none() && off["meta"].get("marks").is_none());

    let table = cmd(&server, home.path())
        .args(q)
        .arg("--table")
        .assert()
        .success();
    let text = String::from_utf8_lossy(&table.get_output().stdout).to_string();
    let header = text.lines().nth(1).unwrap();
    assert!(header.trim_end().ends_with("mark"), "{header}");
    assert!(text.contains("critical"), "{text}");
    let err = String::from_utf8_lossy(&table.get_output().stderr).to_string();
    assert!(err.contains("1 rows (1 flows) squelched"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_groups_by_in_omits_squelched_peer_rows_and_counts_them() {
    let server = MockServer::start().await;
    mount(&server).await;
    let home = home_with(MARKS);
    let out = json_out(
        cmd(&server, home.path())
            .args(["traffic", "groups", "--window", "1h", "--by", "in"])
            .assert()
            .success(),
    );
    let shop = &out["data"]["sections"][0];
    assert_eq!(shop["app"], json!(["shop", "prod"]));
    let rows = shop["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "Corp's 8443 row is gone");
    assert!(rows.iter().all(|r| r["peer"] != json!(["ipl:Corp", "-"])));
    for r in rows {
        assert_eq!(r["mark"]["action"], "critical", "{r}");
        assert_eq!(r["mark"]["id"], "m-crit01");
        assert_eq!(r["marks"], json!({"critical": 1}));
    }
    assert_eq!(out["meta"]["squelched"], json!({"rows": 1, "flows": 1}));
    // Pairs, by service: the same squelched record, its own row gone.
    let pairs = json_out(
        cmd(&server, home.path())
            .args(["traffic", "groups", "--window", "1h", "--services"])
            .assert()
            .success(),
    );
    assert_eq!(pairs["data"].as_array().unwrap().len(), 3);
    assert_eq!(pairs["meta"]["squelched"], json!({"rows": 1, "flows": 1}));
    let mesh = json_out(
        cmd(&server, home.path())
            .args(["traffic", "mesh", "--window", "1h"])
            .assert()
            .success(),
    );
    assert_eq!(mesh["data"].as_array().unwrap().len(), 3);
    assert_eq!(mesh["meta"]["squelched"]["flows"], 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_active_file_without_hits_reports_zeros_and_an_invalid_one_warns() {
    let server = MockServer::start().await;
    mount(&server).await;
    let none = home_with(
        "[[mark]]\naction = \"squelch\"\nsrc = { ip_list = \"Nowhere\" }\ndst = \"any\"\ncomment = \"x\"\n",
    );
    let out = json_out(
        cmd(&server, none.path())
            .args(["traffic", "query", "--window", "1h"])
            .assert()
            .success(),
    );
    assert_eq!(out["meta"]["squelched"], json!({"rows": 0, "flows": 0}));
    assert_eq!(out["data"].as_array().unwrap().len(), 4);

    let bad = home_with("[[mark]]\naction = \"squelch\"\nsrc = \"any\"\n");
    let a = cmd(&server, bad.path())
        .args(["traffic", "query", "--window", "1h"])
        .assert()
        .success();
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(
        err.contains(
            "marks.toml invalid: 1 problem, marks off (illumio-tui marks list shows them)"
        ),
        "{err}"
    );
    let out = json_out(a);
    assert_eq!(
        out["data"].as_array().unwrap().len(),
        4,
        "runs without marks"
    );
    assert!(out["meta"].get("squelched").is_none());
    assert_eq!(
        out["meta"]["warnings"],
        json!(["marks.toml invalid: 1 problem, marks off (illumio-tui marks list shows them)"])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn app_peers_leave_out_squelched_rows() {
    let server = MockServer::start().await;
    mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/workloads.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .mount(&server)
        .await;
    let home = home_with(MARKS);
    let out = json_out(
        cmd(&server, home.path())
            .args(["app", "shop/prod", "--window", "1h", "--direction", "in"])
            .assert()
            .success(),
    );
    let inbound = out["data"]["inbound"].as_array().unwrap();
    assert!(
        inbound
            .iter()
            .all(|p| p["peer"] != json!(["ipl:Corp", "-"])),
        "{inbound:?}"
    );
    assert!(inbound.iter().all(|p| p["mark"]["action"] == "critical"));
    assert_eq!(out["meta"]["squelched"], json!({"rows": 1, "flows": 1}));
}

#[tokio::test(flavor = "multi_thread")]
async fn gen_ignores_marks_for_its_rules_and_only_counts_them() {
    let server = MockServer::start().await;
    mount(&server).await;
    let home = home_with(MARKS);
    let args = ["gen", "shop/prod", "--window", "1h"];
    let with = json_out(cmd(&server, home.path()).args(args).assert().success());
    let without = json_out(
        cmd(&server, home.path())
            .args(args)
            .arg("--no-marks")
            .assert()
            .success(),
    );
    assert_eq!(
        serde_json::to_string(&with["data"]).unwrap(),
        serde_json::to_string(&without["data"]).unwrap(),
        "the proposal is byte-identical"
    );
    assert_eq!(with["meta"]["squelched"], json!({"flows": 1}));
    assert!(without["meta"].get("squelched").is_none());
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p illumio-tui --test cli_marks_traffic`
Expected: FAIL: it does not compile (`file not found for module marked`: Step 3 creates `cli/marked.rs`).

- [ ] **Step 3: Implement**

In `crates/illumio-tui/src/cli/app.rs`, replace:

```rust
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{OutputFormat, TableRows};
use super::traffic::{ByArg, DirectionArg, FilterArgs, TransmissionArg, cli_focus, fetch};
```

with:

```rust
use super::context::{Context, cache_label};
use super::error::CliError;
use super::marked::{CliMarks, mark_cell, squelched};
use super::output::{OutputFormat, TableRows};
use super::traffic::{ByArg, DirectionArg, FilterArgs, TransmissionArg, cli_focus, fetch};
```

In `crates/illumio-tui/src/cli/app.rs`, replace:

```rust
    app_workload_filter, app_workloads, enforcement_word, label_hrefs, workload_in_app,
};
use pce::models::{PolicyScope, Workload};
use pce::traffic::{Direction, DraftCounts, FocusDirection};
```

with:

```rust
    app_workload_filter, app_workloads, enforcement_word, label_hrefs, workload_in_app,
};
use pce::marks::{annotate_peers, partition};
use pce::models::{PolicyScope, Workload};
use pce::traffic::{Direction, DraftCounts, FocusDirection};
```

In `crates/illumio-tui/src/cli/app.rs`, replace:

```rust
    #[arg(long)]
    pub draft: bool,
}

```

with:

```rust
    #[arg(long)]
    pub draft: bool,
    /// Keep squelched flows (marks.toml) in the peers, marked `squelch`
    #[arg(long, conflicts_with = "no_marks")]
    pub show_squelched: bool,
    /// Ignore marks.toml: no mark fields, nothing left out
    #[arg(long)]
    pub no_marks: bool,
}

```

In `crates/illumio-tui/src/cli/app.rs`, replace:

```rust
            h.push("draft");
        }
        h
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let draft = self.draft();
        self.0
            .iter()
```

with:

```rust
            h.push("draft");
        }
        if self.marked() {
            h.push("mark");
        }
        h
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let draft = self.draft();
        let marked = self.marked();
        self.0
            .iter()
```

In `crates/illumio-tui/src/cli/app.rs`, replace:

```rust
                    row.push(DraftCounts::text(p.draft.as_ref()));
                }
                row
            })
            .collect()
    }
    /// The table's, with the draft counts as numbers.
    fn full_headers(&self) -> Vec<&'static str> {
        let mut h = self.headers();
        if self.draft() {
            h.pop();
            h.extend(super::traffic::DRAFT_COUNTS);
        }
        h
    }
    fn full_rows(&self) -> Vec<Vec<String>> {
        let draft = self.draft();
        self.0
            .iter()
            .zip(self.rows())
            .map(|(p, mut row)| {
                if draft {
                    row.pop();
                    row.extend(super::traffic::draft_cells(p.draft.as_ref()));
                }
                row
            })
```

with:

```rust
                    row.push(DraftCounts::text(p.draft.as_ref()));
                }
                if marked {
                    row.push(mark_cell(p.marks.as_ref()));
                }
                row
            })
            .collect()
    }
    /// The table's, with the draft counts as numbers, then `mark`.
    fn full_headers(&self) -> Vec<&'static str> {
        let mut h = self.headers();
        h.retain(|c| *c != "draft" && *c != "mark");
        if self.draft() {
            h.extend(super::traffic::DRAFT_COUNTS);
        }
        if self.marked() {
            h.push("mark");
        }
        h
    }
    fn full_rows(&self) -> Vec<Vec<String>> {
        let draft = self.draft();
        let marked = self.marked();
        self.0
            .iter()
            .zip(self.rows())
            .map(|(p, mut row)| {
                let mark = if marked { row.pop() } else { None };
                if draft {
                    row.pop();
                    row.extend(super::traffic::draft_cells(p.draft.as_ref()));
                }
                row.extend(mark);
                row
            })
```

In `crates/illumio-tui/src/cli/app.rs`, replace:

```rust
    fn draft(&self) -> bool {
        self.0.iter().any(|p| p.draft.is_some())
    }
}
```

with:

```rust
    fn draft(&self) -> bool {
        self.0.iter().any(|p| p.draft.is_some())
    }

    fn marked(&self) -> bool {
        self.0.iter().any(|p| p.marks.is_some())
    }
}
```

In `crates/illumio-tui/src/cli/app.rs`, replace:

```rust
    let (rows, summary) = app_workloads(&mine, &idx, &ctx.config.tier_key, Timestamp::now());
    let d = focus.direction;
    let report = AppReport {
        app: focus.app_text(),
```

with:

```rust
    let (rows, summary) = app_workloads(&mine, &idx, &ctx.config.tier_key, Timestamp::now());
    let d = focus.direction;
    // Marks: the peers aggregate the records kept; each side joins them.
    let cm = CliMarks::load(ctx, args.no_marks, args.show_squelched);
    let act = cm.active(args.draft);
    let part = act.as_ref().map(|a| partition(&fetched.flows, a, cm.show));
    let flows: &[pce::traffic::ResolvedFlow] = part.as_ref().map_or(&fetched.flows, |p| &p.kept);
    let side = |dir: Direction, skip: FocusDirection| -> (Vec<AppPeer>, u64) {
        if d == skip {
            return (Vec::new(), 0);
        }
        let agg = |x: &[pce::traffic::ResolvedFlow]| app_peers(x, &focus, dir);
        let mut rows = agg(flows);
        let gone = match (&part, &act) {
            (Some(p), Some(a)) => annotate_peers(&mut rows, &p.marked, a, agg),
            _ => 0,
        };
        (rows, gone)
    };
    let (inbound, gone_in) = side(Direction::In, FocusDirection::Out);
    let (outbound, gone_out) = side(Direction::Out, FocusDirection::In);
    let sq = squelched(
        part.as_ref(),
        act.as_ref(),
        |_, _| gone_in + gone_out,
        |x| focus.keeps(x),
    );
    let report = AppReport {
        app: focus.app_text(),
```

In `crates/illumio-tui/src/cli/app.rs`, replace:

```rust
            rows,
        },
        inbound: if d == FocusDirection::Out {
            Vec::new()
        } else {
            app_peers(&fetched.flows, &focus, Direction::In)
        },
        outbound: if d == FocusDirection::In {
            Vec::new()
        } else {
            app_peers(&fetched.flows, &focus, Direction::Out)
        },
        policy: PolicySection {
            scope,
```

with:

```rust
            rows,
        },
        inbound,
        outbound,
        policy: PolicySection {
            scope,
```

In `crates/illumio-tui/src/cli/app.rs`, replace:

```rust
    // say so in their sections.
    let mut meta = fetched.meta(ctx);
    let all_hit = fetched.cache == "hit" && wl.hit && rs.hit;
    meta.cache = cache_label(all_hit, ctx.cache_mode);
```

with:

```rust
    // say so in their sections.
    let mut meta = fetched.meta(ctx);
    cm.fill(&mut meta, act.as_ref(), sq);
    let all_hit = fetched.cache == "hit" && wl.hit && rs.hit;
    meta.cache = cache_label(all_hit, ctx.cache_mode);
```

In `crates/illumio-tui/src/cli/context.rs`, replace:

```rust
    pub format: OutputFormat,
    pub cache_mode: CacheMode,
    started: Instant,
}
```

with:

```rust
    pub format: OutputFormat,
    pub cache_mode: CacheMode,
    /// `marks.toml` next to the config file in use.
    pub marks_path: std::path::PathBuf,
    started: Instant,
}
```

In `crates/illumio-tui/src/cli/context.rs`, replace:

```rust
            format,
            cache_mode,
            started: Instant::now(),
        })
```

with:

```rust
            format,
            cache_mode,
            marks_path: crate::marks::marks_path(global.config.as_deref()),
            started: Instant::now(),
        })
```

In `crates/illumio-tui/src/cli/context.rs`, replace:

```rust
            warnings: None,
            marks: None,
        }
    }
```

with:

```rust
            warnings: None,
            marks: None,
            squelched: None,
        }
    }
```

In `crates/illumio-tui/src/cli/generate.rs`, replace:

```rust
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{TableRows, join_list};
use super::traffic::{ByArg, DirectionArg, FilterArgs, cli_focus, fetch};
use clap::{Args, ValueEnum};
```

with:

```rust
use super::context::{Context, cache_label};
use super::error::CliError;
use super::marked::CliMarks;
use super::output::{SquelchedMeta, TableRows, join_list};
use super::traffic::{ByArg, DirectionArg, FilterArgs, cli_focus, fetch};
use clap::{Args, ValueEnum};
```

In `crates/illumio-tui/src/cli/generate.rs`, replace:

```rust
    candidates, keys_refusal, propose, rows,
};
use pce::models::PolicyScope;
use pce::write::PortSpec;
```

with:

```rust
    candidates, keys_refusal, propose, rows,
};
use pce::marks::Action;
use pce::models::PolicyScope;
use pce::write::PortSpec;
```

In `crates/illumio-tui/src/cli/generate.rs`, replace:

```rust
    #[arg(long, value_name = "PORT/PROTO=any|all")]
    pub consolidate: Vec<String>,
}

```

with:

```rust
    #[arg(long, value_name = "PORT/PROTO=any|all")]
    pub consolidate: Vec<String>,
    /// Ignore marks.toml (the rules include squelched flows either way; only
    /// meta.squelched reports them)
    #[arg(long)]
    pub no_marks: bool,
}

```

In `crates/illumio-tui/src/cli/generate.rs`, replace:

```rust
    let mut meta = fetched.meta(ctx);
    meta.cache = cache_label(fetched.cache == "hit" && drafts.hit, ctx.cache_mode);
    ctx.emit_meta(&report, &meta)
}
```

with:

```rust
    let mut meta = fetched.meta(ctx);
    meta.cache = cache_label(fetched.cache == "hit" && drafts.hit, ctx.cache_mode);
    // Marks are not policy: the rules above saw every flow. Only the app's
    // squelched flows are counted.
    let cm = CliMarks::load(ctx, args.no_marks, false);
    let act = cm.active(false);
    let sq = act.as_ref().map(|a| SquelchedMeta {
        rows: None,
        flows: fetched
            .flows
            .iter()
            .filter(|f| focus.keeps(f))
            .filter(|f| a.classify(f).is_some_and(|h| h.action == Action::Squelch))
            .count() as u64,
    });
    cm.fill(&mut meta, act.as_ref(), sq);
    ctx.emit_meta(&report, &meta)
}
```

Create `crates/illumio-tui/src/cli/marked.rs`:

```rust
//! Marks applied to traffic output (`traffic query|groups|mesh`, `app`;
//! `gen` only counts them): the file read once (a local file, no
//! request), its active marks for the profile's PCE, and what the
//! envelope says: `meta.squelched`, `meta.marks`, a warning for an invalid
//! file (the command runs without marks).
use super::context::Context;
use super::output::{MarksMeta, Meta, SquelchedMeta};
use crate::marks::MarksStore;
use jiff::Timestamp;
use pce::marks::{Action, ActiveMarks, Hit, MarkSet, Marked, Marker, Partition, PceId, RowMarks};
use pce::traffic::ResolvedFlow;
use serde::Serialize;

/// The marks file as a traffic command sees it.
pub struct CliMarks {
    file: String,
    set: Option<MarkSet>,
    warning: Option<String>,
    /// `--show-squelched`: squelched flows stay in, marked `squelch`.
    pub show: bool,
    pce: PceId,
    now: Timestamp,
}

impl CliMarks {
    /// Reads `marks.toml` unless `no_marks`. An invalid or unreadable file
    /// is a warning on stderr (and in `meta.warnings`): marks off.
    pub fn load(ctx: &Context, no_marks: bool, show: bool) -> Self {
        let mut cm = CliMarks {
            file: ctx.marks_path.display().to_string(),
            set: None,
            warning: None,
            show,
            pce: PceId {
                host: ctx.config.host.clone(),
                org: ctx.config.org_id,
            },
            now: Timestamp::now(),
        };
        if no_marks {
            return cm;
        }
        let store = MarksStore::open(ctx.marks_path.clone());
        match (store.set(), store.error()) {
            (Some(set), _) => cm.set = Some(set.clone()),
            (None, Some(crate::marks::MarksError::Parse(p))) => {
                cm.warning = Some(format!(
                    "marks.toml invalid: {} problem{}, marks off (illumio-tui marks list shows them)",
                    p.len(),
                    if p.len() == 1 { "" } else { "s" }
                ));
            }
            (None, e) => {
                cm.warning = Some(format!(
                    "marks.toml unreadable, marks off: {}",
                    e.map(|e| e.to_string()).unwrap_or_default()
                ));
            }
        }
        if let Some(w) = &cm.warning {
            eprintln!("warning: {w}");
        }
        cm
    }

    /// The marks active for this result (`draft`: draft decisions were
    /// loaded, `--draft`); `None` without a valid file or an active mark.
    pub fn active(&self, draft: bool) -> Option<ActiveMarks<'_>> {
        self.set
            .as_ref()
            .map(|s| s.active(Some(&self.pce), self.now, draft))
            .filter(|a| !a.is_empty())
    }

    /// `meta.marks`, `meta.squelched` (with an active mark) and the
    /// warning of an invalid file.
    pub fn fill(&self, meta: &mut Meta, active: Option<&ActiveMarks>, sq: Option<SquelchedMeta>) {
        if let Some(w) = &self.warning {
            meta.warnings.get_or_insert_with(Vec::new).push(w.clone());
        }
        if let Some(a) = active {
            meta.marks = Some(MarksMeta {
                active: a.len(),
                file: self.file.clone(),
            });
            meta.squelched = sq;
        }
    }
}

/// `meta.squelched` of one view: `annotate` joins the marks to its rows
/// and returns the rows squelch removed; `pred` keeps the squelched
/// records of the view's own input (its decision filter).
pub fn squelched(
    part: Option<&Partition>,
    active: Option<&ActiveMarks>,
    annotate: impl FnOnce(&Marked, &ActiveMarks) -> u64,
    pred: impl Fn(&ResolvedFlow) -> bool,
) -> Option<SquelchedMeta> {
    let (p, a) = (part?, active?);
    Some(SquelchedMeta {
        rows: Some(annotate(&p.marked, a)),
        flows: p.marked.squelched_where(pred),
    })
}

/// A flow's `mark`: its winning action and mark.
#[derive(Debug, Clone, Serialize)]
pub struct FlowMark {
    pub action: Action,
    pub id: String,
}

impl FlowMark {
    pub fn of(h: Option<&Hit>, a: &ActiveMarks) -> Option<FlowMark> {
        h.map(|h| FlowMark {
            action: h.action,
            id: a.mark(h.mark).id.clone(),
        })
    }
}

/// A flow record with its mark (`traffic query`): `ResolvedFlow` itself
/// is unchanged.
#[derive(Debug, Serialize)]
pub struct MarkedFlow<'a> {
    #[serde(flatten)]
    pub flow: &'a ResolvedFlow,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mark: Option<&'a FlowMark>,
}

/// The `mark` cell of tables and CSV: the row marker's action, `~12` for
/// a partly squelched row, empty without one.
pub fn mark_cell(m: Option<&RowMarks>) -> String {
    match m.and_then(RowMarks::marker) {
        Some(Marker::PartlySquelched(n)) => format!("~{n}"),
        Some(k) => k
            .action()
            .map(Action::as_str)
            .unwrap_or_default()
            .to_string(),
        None => String::new(),
    }
}
```

In `crates/illumio-tui/src/cli/output.rs`, replace:

```rust
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marks: Option<MarksMeta>,
}

```

with:

```rust
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marks: Option<MarksMeta>,
    /// Traffic with an active mark: the rows squelch removed and the
    /// squelched flow records of the view (zeros included); `gen`: the
    /// flows only (its rules include them).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub squelched: Option<SquelchedMeta>,
}

/// `meta.squelched`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SquelchedMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<u64>,
    pub flows: u64,
}

```

In `crates/illumio-tui/src/cli/output.rs`, replace:

```rust
        s.push_str(" · WARNING: incomplete, a PCE region did not respond");
    }
    if let Some(w) = &meta.write {
        s.push_str(&format!(
```

with:

```rust
        s.push_str(" · WARNING: incomplete, a PCE region did not respond");
    }
    match meta.squelched {
        Some(SquelchedMeta {
            rows: Some(r),
            flows,
        }) if flows > 0 => s.push_str(&format!(" · {r} rows ({flows} flows) squelched")),
        Some(SquelchedMeta { rows: None, flows }) if flows > 0 => {
            s.push_str(&format!(" · {flows} squelched (rules include them)"))
        }
        _ => {}
    }
    if let Some(w) = &meta.write {
        s.push_str(&format!(
```

In `crates/illumio-tui/src/cli/profile.rs`, replace:

```rust
        warnings: None,
        marks: None,
    };
    let mut out = std::io::stdout().lock();
```

with:

```rust
        warnings: None,
        marks: None,
        squelched: None,
    };
    let mut out = std::io::stdout().lock();
```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{Meta, TableRows, Window, join_list};
use clap::{Args, Subcommand, ValueEnum};
use jiff::Timestamp;
use jiff::civil::Date;
use pce::models::{
    ActorMatch, PolicyDecision, ServiceMatch, TrafficQuery, Transmission, WorkloadFilter,
```

with:

```rust
use super::context::{Context, cache_label};
use super::error::CliError;
use super::marked::{CliMarks, FlowMark, MarkedFlow, mark_cell, squelched};
use super::output::{Meta, SquelchedMeta, TableRows, Window, join_list};
use clap::{Args, Subcommand, ValueEnum};
use jiff::Timestamp;
use jiff::civil::Date;
use pce::marks::{annotate_groups, annotate_mesh, annotate_sections, partition};
use pce::models::{
    ActorMatch, PolicyDecision, ServiceMatch, TrafficQuery, Transmission, WorkloadFilter,
```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
    FlowGroup, Focus, FocusDirection, MAX_TRAFFIC_MAX_RESULTS, MeshColumns, MeshRow, ResolvedFlow,
    TrafficQueryBuilder, app_header, app_sections, app_sections_both, backfill_cap,
    compact_duration, group_by_labels, group_by_service, mesh, parse_app, parse_window,
    resolve_actor, resolve_flows,
};
use pce::{KeyOrder, NameIndex};
```

with:

```rust
    FlowGroup, Focus, FocusDirection, MAX_TRAFFIC_MAX_RESULTS, MeshColumns, MeshRow, ResolvedFlow,
    TrafficQueryBuilder, app_header, app_sections, app_sections_both, backfill_cap,
    compact_duration, group_by_labels, group_by_service, in_app, keys_or_default, mesh, parse_app,
    parse_window, resolve_actor, resolve_flows,
};
use pce::{KeyOrder, NameIndex};
```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
    #[arg(long)]
    pub draft: bool,
}

```

with:

```rust
    #[arg(long)]
    pub draft: bool,
    /// Keep squelched flows (marks.toml) in the output, marked `squelch`, instead of
    /// leaving them out; meta.squelched still counts them
    #[arg(long, conflicts_with = "no_marks")]
    pub show_squelched: bool,
    /// Ignore marks.toml: no mark fields, nothing left out, no meta.squelched
    #[arg(long)]
    pub no_marks: bool,
}

```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
}

/// Flows as printed: JSON is the flows themselves (every label); the
/// table's label columns follow `order` and hide its ignored keys.
struct FlowRows<'a> {
    flows: &'a [ResolvedFlow],
    order: &'a KeyOrder,
}

impl Serialize for FlowRows<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.flows.serialize(s)
    }
}
```

with:

```rust
}

/// Flows as printed: JSON is the flows themselves (every label), with
/// their `mark` while marks are active; the table's label columns follow
/// `order` and hide its ignored keys, and a last `mark` column the marks.
struct FlowRows<'a> {
    flows: &'a [ResolvedFlow],
    order: &'a KeyOrder,
    /// Each flow's mark, while marks are active.
    marks: Option<Vec<Option<FlowMark>>>,
}

impl Serialize for FlowRows<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match &self.marks {
            None => self.flows.serialize(s),
            Some(marks) => {
                s.collect_seq(self.flows.iter().zip(marks).map(|(flow, mark)| MarkedFlow {
                    flow,
                    mark: mark.as_ref(),
                }))
            }
        }
    }
}
```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
            h.insert(8, "draft");
        }
        h
    }
```

with:

```rust
            h.insert(8, "draft");
        }
        if self.marks.is_some() {
            h.push("mark");
        }
        h
    }
```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
        self.flows
            .iter()
            .map(|f| {
                let mut row = vec![
                    f.last_detected.clone().unwrap_or_default(),
```

with:

```rust
        self.flows
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let mut row = vec![
                    f.last_detected.clone().unwrap_or_default(),
```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
                if draft {
                    row.insert(8, f.draft_text());
                }
                row
```

with:

```rust
                if draft {
                    row.insert(8, f.draft_text());
                }
                if let Some(marks) = &self.marks {
                    row.push(
                        marks[i]
                            .as_ref()
                            .map(|m| m.action.as_str().to_string())
                            .unwrap_or_default(),
                    );
                }
                row
```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
}

fn group_cells(groups: &[FlowGroup]) -> Vec<Vec<String>> {
    let any = groups.iter().any(|g| g.bytes > 0);
```

with:

```rust
}

/// Some row carries marks: the tables get a last `mark` column.
fn groups_marked(groups: &[FlowGroup]) -> bool {
    groups.iter().any(|g| g.marks.is_some())
}

fn group_cells(groups: &[FlowGroup]) -> Vec<Vec<String>> {
    let any = groups.iter().any(|g| g.bytes > 0);
```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
            h.push("draft");
        }
        h
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let draft = groups_draft(self);
        self.iter()
            .zip(group_cells(self))
```

with:

```rust
            h.push("draft");
        }
        if groups_marked(self) {
            h.push("mark");
        }
        h
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let draft = groups_draft(self);
        let marked = groups_marked(self);
        self.iter()
            .zip(group_cells(self))
```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
                    row.push(DraftCounts::text(g.draft.as_ref()));
                }
                row
            })
            .collect()
    }
    /// The table's counts, then the connections per decision, then with
    /// draft decisions `draft_allowed` and `draft_blocked`.
    fn full_headers(&self) -> Vec<&'static str> {
        let mut h = self.headers();
        h.retain(|c| *c != "draft");
        h.extend(CONNS_BY_DECISION);
        if groups_draft(self) {
            h.extend(DRAFT_COUNTS);
        }
        h
```

with:

```rust
                    row.push(DraftCounts::text(g.draft.as_ref()));
                }
                if marked {
                    row.push(mark_cell(g.marks.as_ref()));
                }
                row
            })
            .collect()
    }
    /// The table's counts, then the connections per decision, then with
    /// draft decisions `draft_allowed` and `draft_blocked`, then `mark`.
    fn full_headers(&self) -> Vec<&'static str> {
        let mut h = self.headers();
        h.retain(|c| *c != "draft" && *c != "mark");
        h.extend(CONNS_BY_DECISION);
        if groups_draft(self) {
            h.extend(DRAFT_COUNTS);
        }
        if groups_marked(self) {
            h.push("mark");
        }
        h
```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
                if draft {
                    row.extend(draft_cells(g.draft.as_ref()));
                }
                row
```

with:

```rust
                if draft {
                    row.extend(draft_cells(g.draft.as_ref()));
                }
                if groups_marked(self) {
                    row.push(mark_cell(g.marks.as_ref()));
                }
                row
```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
            h.push("draft");
        }
        h
    }
    /// The table's, with the draft counts as `draft_allowed` and
    /// `draft_blocked` numbers.
    fn full_headers(&self) -> Vec<&'static str> {
        let mut h = self.headers();
        if mesh_draft(self) {
            h.pop();
            h.extend(DRAFT_COUNTS);
        }
        h
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let draft = mesh_draft(self);
        let any = self.iter().any(|r| r.bytes > 0);
        let or_dash = |v: &Option<String>| v.clone().unwrap_or_else(|| "-".into());
```

with:

```rust
            h.push("draft");
        }
        if self.iter().any(|r| r.marks.is_some()) {
            h.push("mark");
        }
        h
    }
    /// The table's, with the draft counts as `draft_allowed` and
    /// `draft_blocked` numbers, then `mark`.
    fn full_headers(&self) -> Vec<&'static str> {
        let mut h = self.headers();
        let marked = h.last() == Some(&"mark");
        h.retain(|c| *c != "draft" && *c != "mark");
        if mesh_draft(self) {
            h.extend(DRAFT_COUNTS);
        }
        if marked {
            h.push("mark");
        }
        h
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let draft = mesh_draft(self);
        let marked = self.iter().any(|r| r.marks.is_some());
        let any = self.iter().any(|r| r.bytes > 0);
        let or_dash = |v: &Option<String>| v.clone().unwrap_or_else(|| "-".into());
```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
                    row.push(DraftCounts::text(r.draft.as_ref()));
                }
                row
            })
```

with:

```rust
                    row.push(DraftCounts::text(r.draft.as_ref()));
                }
                if marked {
                    row.push(mark_cell(r.marks.as_ref()));
                }
                row
            })
```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
    fn full_rows(&self) -> Vec<Vec<String>> {
        let draft = mesh_draft(self);
        let or_empty = |v: &Option<String>| v.clone().unwrap_or_default();
        self.iter()
```

with:

```rust
    fn full_rows(&self) -> Vec<Vec<String>> {
        let draft = mesh_draft(self);
        let marked = self.iter().any(|r| r.marks.is_some());
        let or_empty = |v: &Option<String>| v.clone().unwrap_or_default();
        self.iter()
```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
                row[5] = or_empty(&r.dst_process);
                row[8] = r.bytes.to_string();
                if draft {
                    row.pop();
                    row.extend(draft_cells(r.draft.as_ref()));
                }
                row
            })
```

with:

```rust
                row[5] = or_empty(&r.dst_process);
                row[8] = r.bytes.to_string();
                let mark = if marked { row.pop() } else { None };
                if draft {
                    row.pop();
                    row.extend(draft_cells(r.draft.as_ref()));
                }
                row.extend(mark);
                row
            })
```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
            h.push("draft");
        }
        h
    }
```

with:

```rust
            h.push("draft");
        }
        if self.marked() {
            h.push("mark");
        }
        h
    }
```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
    fn rows(&self) -> Vec<Vec<String>> {
        let draft = self.draft();
        let any = self
            .sections
```

with:

```rust
    fn rows(&self) -> Vec<Vec<String>> {
        let draft = self.draft();
        let marked = self.marked();
        let any = self
            .sections
```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
                        row.push(DraftCounts::text(r.draft.as_ref()));
                    }
                }
            }
```

with:

```rust
                        row.push(DraftCounts::text(r.draft.as_ref()));
                    }
                    if marked && let Some(row) = out.last_mut() {
                        row.push(mark_cell(r.marks.as_ref()));
                    }
                }
            }
```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
    /// columns, the host names lists beside their counts.
    fn full_headers(&self) -> Vec<&'static str> {
        section_csv_headers(self.draft())
    }
    fn full_rows(&self) -> Vec<Vec<String>> {
        let draft = self.draft();
        self.sections
            .iter()
            .flat_map(|s| {
                s.rows
                    .iter()
                    .map(move |r| section_csv_row(&s.app, s.direction, r, draft))
            })
            .collect()
```

with:

```rust
    /// columns, the host names lists beside their counts.
    fn full_headers(&self) -> Vec<&'static str> {
        let mut h = section_csv_headers(self.draft());
        if self.marked() {
            h.push("mark");
        }
        h
    }
    fn full_rows(&self) -> Vec<Vec<String>> {
        let (draft, marked) = (self.draft(), self.marked());
        self.sections
            .iter()
            .flat_map(|s| {
                s.rows.iter().map(move |r| {
                    let mut row = section_csv_row(&s.app, s.direction, r, draft);
                    if marked {
                        row.push(mark_cell(r.marks.as_ref()));
                    }
                    row
                })
            })
            .collect()
```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
            .flat_map(|s| &s.rows)
            .any(|r| r.draft.is_some())
    }
}
```

with:

```rust
            .flat_map(|s| &s.rows)
            .any(|r| r.draft.is_some())
    }

    /// Some row carries marks: a last `mark` column.
    fn marked(&self) -> bool {
        self.sections
            .iter()
            .flat_map(|s| &s.rows)
            .any(|r| r.marks.is_some())
    }
}
```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
        return Ok(());
    };
    if args.blocked {
        let keys = keys_or_profile(ctx, &[]);
        let sections = Sections {
            sections: app_sections(&f.flows, &keys, Direction::In, Decisions::Blocked),
            both: false,
        };
        ctx.emit_meta(&sections, &f.meta(ctx))
    } else if !args.group.is_empty() {
        ctx.emit_meta(&group_by_labels(&f.flows, &args.group), &f.meta(ctx))
    } else {
        let order = ctx.config.key_order();
        let rows = FlowRows {
            flows: &f.flows,
            order: &order,
        };
        ctx.emit_meta(&rows, &f.meta(ctx))
    }
}
```

with:

```rust
        return Ok(());
    };
    // Marks: classify, then aggregate the records kept (all of them with
    // --show-squelched); the marks join the rows by their keys.
    let cm = CliMarks::load(ctx, args.filter.no_marks, args.filter.show_squelched);
    let act = cm.active(args.filter.draft);
    let part = act.as_ref().map(|a| partition(&f.flows, a, cm.show));
    let flows: &[ResolvedFlow] = part.as_ref().map_or(&f.flows, |p| &p.kept);
    let mut meta = f.meta(ctx);
    if args.blocked {
        let keys = keys_or_profile(ctx, &[]);
        let agg = |x: &[ResolvedFlow]| app_sections(x, &keys, Direction::In, Decisions::Blocked);
        let mut sections = Sections {
            sections: agg(flows),
            both: false,
        };
        let sq = squelched(
            part.as_ref(),
            act.as_ref(),
            |p, a| annotate_sections(&mut sections.sections, p, a, agg),
            |x| Decisions::Blocked.keeps(x),
        );
        cm.fill(&mut meta, act.as_ref(), sq);
        ctx.emit_meta(&sections, &meta)
    } else if !args.group.is_empty() {
        let agg = |x: &[ResolvedFlow]| group_by_labels(x, &args.group);
        let mut groups = agg(flows);
        let sq = squelched(
            part.as_ref(),
            act.as_ref(),
            |p, a| annotate_groups(&mut groups, p, a, agg),
            |_| true,
        );
        cm.fill(&mut meta, act.as_ref(), sq);
        ctx.emit_meta(&groups, &meta)
    } else {
        let order = ctx.config.key_order();
        // Flows: rows are flows.
        let (marks, sq) = match (&part, &act) {
            (Some(p), Some(a)) => {
                let n = p.marked.squelched_where(|_| true);
                (
                    Some(
                        p.kept_hits
                            .iter()
                            .map(|h| FlowMark::of(h.as_ref(), a))
                            .collect(),
                    ),
                    Some(SquelchedMeta {
                        rows: Some(n),
                        flows: n,
                    }),
                )
            }
            _ => (None, None),
        };
        cm.fill(&mut meta, act.as_ref(), sq);
        let rows = FlowRows {
            flows,
            order: &order,
            marks,
        };
        ctx.emit_meta(&rows, &meta)
    }
}
```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
        return Ok(());
    };
    match sections_for(&f.flows, &keys, args) {
        Some(sections) => {
            if let Some(miss) = app_miss(args, &sections) {
                eprintln!("{miss}");
            }
            ctx.emit_meta(&sections, &f.meta(ctx))
        }
        None if args.services => ctx.emit_meta(
            &ServiceGroups(pairs_for(&f.flows, &keys, args)),
            &f.meta(ctx),
        ),
        None => ctx.emit_meta(&pairs_for(&f.flows, &keys, args), &f.meta(ctx)),
    }
}
```

with:

```rust
        return Ok(());
    };
    let cm = CliMarks::load(ctx, args.filter.no_marks, args.filter.show_squelched);
    let act = cm.active(args.filter.draft);
    let part = act.as_ref().map(|a| partition(&f.flows, a, cm.show));
    let flows: &[ResolvedFlow] = part.as_ref().map_or(&f.flows, |p| &p.kept);
    let decisions = Decisions::from(args.show);
    let mut meta = f.meta(ctx);
    match sections_for(flows, &keys, args) {
        Some(mut sections) => {
            // The squelched records of the sections shown: `--app`'s only.
            let resolved = keys_or_default(&keys);
            let app = args.filter.app.as_deref().map(parse_app);
            let dir = match args.by {
                ByArg::In => Some(Direction::In),
                ByArg::Out => Some(Direction::Out),
                _ => None,
            };
            let agg = |x: &[ResolvedFlow]| {
                sections_for(x, &keys, args)
                    .map(|s| s.sections)
                    .unwrap_or_default()
            };
            let sq = squelched(
                part.as_ref(),
                act.as_ref(),
                |p, a| annotate_sections(&mut sections.sections, p, a, agg),
                |x| match &app {
                    Some(app) => in_app(x, &resolved, app, dir, decisions),
                    None => decisions.keeps(x),
                },
            );
            cm.fill(&mut meta, act.as_ref(), sq);
            if let Some(miss) = app_miss(args, &sections) {
                eprintln!("{miss}");
            }
            ctx.emit_meta(&sections, &meta)
        }
        None => {
            let agg = |x: &[ResolvedFlow]| pairs_for(x, &keys, args);
            let mut pairs = agg(flows);
            let sq = squelched(
                part.as_ref(),
                act.as_ref(),
                |p, a| annotate_groups(&mut pairs, p, a, agg),
                |x| decisions.keeps(x),
            );
            cm.fill(&mut meta, act.as_ref(), sq);
            if args.services {
                ctx.emit_meta(&ServiceGroups(pairs), &meta)
            } else {
                ctx.emit_meta(&pairs, &meta)
            }
        }
    }
}
```

In `crates/illumio-tui/src/cli/traffic.rs`, replace:

```rust
        process: !args.no_process,
    };
    ctx.emit_meta(&mesh(&f.flows, &keys, cols), &f.meta(ctx))
}

```

with:

```rust
        process: !args.no_process,
    };
    let cm = CliMarks::load(ctx, args.filter.no_marks, args.filter.show_squelched);
    let act = cm.active(args.filter.draft);
    let part = act.as_ref().map(|a| partition(&f.flows, a, cm.show));
    let flows: &[ResolvedFlow] = part.as_ref().map_or(&f.flows, |p| &p.kept);
    let agg = |x: &[ResolvedFlow]| mesh(x, &keys, cols);
    let mut rows = agg(flows);
    let sq = squelched(
        part.as_ref(),
        act.as_ref(),
        |p, a| annotate_mesh(&mut rows, p, a, agg),
        |_| true,
    );
    let mut meta = f.meta(ctx);
    cm.fill(&mut meta, act.as_ref(), sq);
    ctx.emit_meta(&rows, &meta)
}

```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p illumio-tui --test cli_marks_traffic && cargo test -p illumio-tui --lib cli::output`
Expected: PASS.

- [ ] **Step 5: Run the gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: no formatting change, no warning, every test passes.

- [ ] **Step 6: Commit**

```bash
git add crates/illumio-tui/src/cli/app.rs crates/illumio-tui/src/cli/context.rs crates/illumio-tui/src/cli/generate.rs crates/illumio-tui/src/cli/marked.rs crates/illumio-tui/src/cli/mod.rs crates/illumio-tui/src/cli/output.rs crates/illumio-tui/src/cli/profile.rs crates/illumio-tui/src/cli/traffic.rs crates/illumio-tui/tests/cli_marks_traffic.rs
git commit -m "feat(cli): traffic, app and gen apply marks

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

### Task 6: Tracking, read-only smoke, final gate

**Files:**
- Modify: `tasks/todo.md`

**Interfaces:**
- Consumes: Tasks 1-5.
- Produces: the tracking section.

- [ ] **Step 1: Track the plan**

Append to `tasks/todo.md`:

```markdown
## Flow marks 1: model, file and CLI (plan docs/superpowers/plans/2026-10-17-flow-marks-1-model-cli.md)
- [x] Task 1 pce::marks: actions, sides, services, matching, precedence, ids
- [x] Task 2 partition, row annotations, speed budget
- [x] Task 3 marks.toml and MarksStore
- [x] Task 4 illumio-tui marks list|add|rm|export|import
- [x] Task 5 traffic, app and gen apply marks
- [x] Task 6 tracking, smoke, gate
```

- [ ] **Step 2: Run the final gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo test --release -p pce --test marks_perf -- --ignored && git diff --exit-code main -- '*Cargo.toml' Cargo.lock`
Expected: every test passes, no warning, the speed test under 300 ms, the diff empty.

- [ ] **Step 3: Read-only smoke on demo100 (the integrator; skip when told not to contact a PCE)**

Marks in a temp config directory; only the traffic query is sent, the marks add no request. Never read or print the env file.

```bash
export XDG_CONFIG_HOME=$(mktemp -d)
illumio-tui marks add --action squelch --src any --dst any --service 53/udp --comment "smoke: DNS is known"
illumio-tui marks add --action critical --src any --dst any --decision blocked --comment "smoke: every blocked flow"
illumio-tui marks list --table
illumio-tui --env-file ../illumio-mcp-server/.env traffic groups --window 1h --by in --json > /tmp/marks-smoke.json
jq '.meta | {api_calls, squelched, marks}' /tmp/marks-smoke.json
illumio-tui --env-file ../illumio-mcp-server/.env traffic groups --window 1h --by in --no-marks --json | jq '.meta.api_calls'
```

Expected: `meta.squelched` with `rows` and `flows`, `meta.marks.active` 2, and the same `api_calls` with and without marks (the second run may be a cache hit, never more calls).

- [ ] **Step 4: Write the review section**

Append `## Review (Flow marks 1)` to `tasks/todo.md`: what shipped, the test count, the speed test's time, the smoke's `meta.squelched` (or that it was not run), "nothing was written to any PCE".

- [ ] **Step 5: Commit**

```bash
git add tasks/todo.md
git commit -m "docs: flow marks part 1 tracking

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

## Self-review notes

- Spec coverage: definitions, the file (fields, sides, validation, unknown fields kept), matching and precedence, aggregated rows (filter then aggregate, highlights any, quiet all, per-row counts by key), counts per view and the invariant, scope and expiry (UTC inputs, defaults, expired marks kept), file I/O (stat refresh, rewrite through serde, `.bak`, atomic 0600 writes, the lock, reload, operations, `Gone`), ids (`m-` FNV-1a, `h-` derived), the data model, the side grammar, the CLI (`marks …`, traffic and `app` applying marks, `meta.squelched`, `meta.marks`, `mark` fields and columns, `--show-squelched`, `--no-marks`, an invalid file a warning), the generator reporting only, the speed budget: Tasks 1-5. The TUI parts are Plans 2 and 3.
- Verification: every step of this plan was applied in order to a clean checkout of main (after the plans before it); each task built clean under `cargo clippy --workspace --all-targets -- -D warnings` with its own tests passing, and the gate passed at the end.
