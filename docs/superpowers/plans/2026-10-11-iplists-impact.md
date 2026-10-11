# IP Lists and Rule Impact Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** IP lists leave the Objects tabs and become a sixth screen (`6`, `:iplists`) with a size column, a "used by" count and detail, and the Plan 4 editing keys; every rule (an existing one, one being written, a `w` form) gets an impact report, the number of workloads it matches as providers and as consumers over the cached workload list, in the TUI rule detail, the rules pane (`P/C`), the rule confirm overlay and the CLI (`policy rules --impact`, `policy rules impact <href>`, `--impact` on rule writes, a `size` column on `iplists`). The workload list is read only on `i` in the TUI or `--impact` in the CLI, and only when it is not cached.

**Architecture:** Two pure `pce` modules. `iplist_size.rs` counts addresses per family with std integers (`Ipv4Addr` to `u32`, `Ipv6Addr` to `u128`): included ranges are merged, exclusions subtracted, `0.0.0.0/0`/`::/0` or at least 2^24 addresses is "any". `impact.rs` evaluates a rule's providers and consumers per ruleset scope over `&[Workload]`, looking label groups (recursively, cycle-safe) and IP lists up through the `NameIndex`, which gains `label_group(href)` and `ip_list(href)`. The TUI adds `Screen::IpLists` (`IpListsState`, `Cmd::LoadIpLists`, `Msg::IpListsLoaded`, `ui/screens/iplists.rs`), drops the Objects IP-list tab, and adds `tui/impact.rs`, which turns the loaded rulesets, the index and the cached workloads into the `P/C` cells, the rule detail's impact rows and the confirm overlay's impact line; `i` issues the one `Cmd::LoadWorkloads(Use)`. The CLI computes the same `Impact` from the same inputs (one cached workloads read) and prints it in JSON next to the rule, the dry-run request or the write result.

**Tech Stack:** Rust 2024 (rust-version 1.88), tokio, ratatui, crossterm, serde/serde_json, clap, insta (snapshots), wiremock 0.6 (runtime and CLI tests), assert_cmd; `std::net` for the IP arithmetic. No new dependency.

**Spec:** `docs/superpowers/specs/2026-10-11-iplists-impact-design.md` (binding).

**Tasks:** 9. Task 3 removes `ObjectKind::IpLists`, so the Objects tab's IP-list keys, its reload after a write and the tests that used them (unit, snapshot and runtime) have to move to the new screen in the same task, or the tree would not build or the existing runtime test would fail; Task 3 therefore binds `n`/`e`/`x` on the IP lists screen and points `reload_after_write` at it. Task 4 adds the refusal and selection coverage of those keys, the create runtime test, the used-by recompute and the CLI `size` column. Every task boundary is green.

## Global Constraints

- edition 2024, rust-version 1.88; no new dependencies (IP arithmetic in std: `Ipv4Addr`/`Ipv6Addr` → `u32`/`u128`).
- Every PCE request is user-caused: the workload list is read only on `i` in the TUI or `--impact` in the CLI and only when not cached; `6` loads IP lists once per visit like other screens; `used_by` and sizes cost no request.
- POST/PUT/DELETE unchanged and sent once.
- The TUI never bypasses `WriteGuard`.
- The CLI envelope and exit codes unchanged except the new `impact`/`size` fields.
- Existing tests keep passing (number keys 1–5 keep their screens; `6` is new).
- Both themes snapshot every new or changed screen/overlay.
- `PCE_LIVE_WRITES=1` never run.
- The smoke is reads only.
- The TUI's `ObjectKind` (Objects tabs, `tui/model.rs`) and `pce::write::ObjectKind` collide: the latter is always written fully qualified, never imported or aliased.
- Commit trailer on every commit, exactly (after a blank line): `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.
- Gate before every commit: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p <crate>` (`pce` for Tasks 1-2, `illumio-tui` for Tasks 3-8, `--workspace` for Task 9). Run `cargo fmt --all` first and accept its import ordering.
- Edition 2024 clippy may rewrite nested `if`/`if let` into let-chains: accept and list it as a deviation in the task report. If clippy reports `large_enum_variant` on `WriteOutput`, `Msg` or `Cmd`, box the new payload (`Box<Impact>`) and list it as a deviation.
- Tests: `pce` unit tests in the new modules (`#[cfg(test)] mod tests`); TUI unit tests in `#[cfg(test)]`; snapshots in `crates/illumio-tui/tests/tui_snapshots.rs`; runtime tests in `crates/illumio-tui/tests/tui_runtime.rs` (wiremock; reuse `pce_for`, `mount_objects`, `mount_lists`, `mount_rule_sets`, `mount_workloads`, `writes_model`, `tui_guard`, `wait`, `key_session`, `objects_session`, `press`, `typed`, `ch`, `writes_seen`); CLI tests in `crates/illumio-tui/tests/cli.rs` and `cli_writes.rs` (assert_cmd + wiremock; request counts).
- Snapshots: new ones are created with `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots`, each new or changed `.snap` is read before committing, and `git status --short crates/illumio-tui/tests/snapshots` must list exactly the files the task names (Tasks 3 and 5 change many existing snapshots for one reason each; they name the set and give a per-file check).
- Lessons that apply (`tasks/lessons.md`): check the recorded shapes (`fixtures/recorded/ip_lists.json` has `0.0.0.0/0` with exclusions and CIDRs with host bits set, e.g. `10.0.0.1/8`); a late result checks the open overlay before touching it; writes-off tests count at the server; doc claims match the code.
- Spec refinements this plan makes (reviewers: these are intended):
  - `ImpactInputs` carries `index: &NameIndex` instead of the `label_groups`/`ip_lists` slices: the index is built from exactly those lists, the TUI holds only the index, and the spec's file map already asks for `label_group(href)`/`ip_list(href)` accessors. No `ImpactInputs::from_index` (the index cannot hand out slices and does not need to).
  - `IpListRef.size` is `Option<IpListSize>` (`None`, shown as `?`, when the index does not know the href; spec 4 "size `?`, name = href").
  - `pce::impact` also holds what CLI and TUI share: `RuleActors` (a rule's providers, consumers and `unscoped_consumers`, from a `Rule`, a `DenyRule` or a planned request body merged over the loaded rule), `rule_set_href_of(path)`, `is_rule_href(path)`, `Impact::{pc, summary}`, `IpListRef::line`. `IpListSize` gains `total()` and `sort_key()`. Drafts reach the evaluator as their planned body (the JSON `to_actor` produced), so CLI and TUI evaluate exactly what would be sent.
  - `resolve::canon` becomes `pub(crate)` (the evaluator folds active hrefs like the index does).
  - A scope member with `exclusion: true` must not match; a scope member `all` matches; a scope member of any other kind (IP list, workload) matches nothing.
  - Counts are pluralised (`1 provider · 0 consumers`); a truncated list reads `≥4 providers · ≥4 consumers (workload list truncated)` and `≥4/≥4` in `P/C`.
  - The rule detail's impact rows use the overlay's label column like its other rows: `impact      12 providers · 3 consumers`, `ip lists    Corp: 65.5K addresses` (spec: `impact: …`); the confirm overlay's line is `impact: …` as specified.
  - The spec's "load Policy (3)" names the wrong key: Policy is `2`. The detail says `used by: load Policy (2) to see rules`; the confirm line for a ruleset the Policy screen has not loaded says `impact: ruleset not loaded; open Policy (2) first`. The smoke presses `2` (not `3`) for Policy.
  - The IP lists table's `size` cell is `any` for an unbounded list (the 10-cell column cannot hold `any (0.0.0.0/0)`); the detail, the rule detail and the CLI show the full `any (<reason>)`.
  - `i` status texts: `loading workloads for impact (1 request)`, `workloads already loading` (spec 4), `N workloads already loaded; r on Workloads (4) refreshes them`.
  - `Msg::IpListsLoaded` leaves the status line alone (like `Msg::ObjectsLoaded`), so a write's report survives its reload.
  - The rules pane's rows become `RuleRow { view: RuleView, pc: String }` with `Deref<Target = RuleView>`: a `Column`'s `fn` pointer cannot see the workload cache, so the cell is computed when the rows are built. `PolicyState::{set_rule_sets, rebuild_rules}` take the cached workloads; `Model::rebuild_rules` wraps it. The `P/C` column sits before `pending`.
  - `WorkloadsState` gains `error: Option<String>` (the last failed read, for `impact: unavailable (<err>)`) besides `truncated`.
  - CLI: `WriteOutput::{DryRunImpact, WrittenImpact}` (the `ProvisionDryRun` precedent), `write::run_rule_plan`, `Context::emit_write_with(.., truncated)` (`emit_write` keeps its signature). `policy rules impact <href>` reads draft (`checked_nested_href` folds an active href) and prints the rule as one entry of `policy rules --impact` prints it.
  - Tab row: full titles when the whole row fits the width, short ones otherwise; `tab_x(screen, width)` gains the width. At 80 columns the full row (84 cells) does not fit, so every 80-column snapshot's tab row changes.
  - Help: the number line reads `Traffic · Policy · Objects · Workloads · Events · IPs` (53 cells, the widest that fits at 80 columns); the n/e/x line reads `new/edit/delete: Objects, IP lists, Workloads, Policy`. `:iplists` also answers to `:ip`.
  - The Objects screen's not-loaded panel is titled `Objects` (spec 2.1).

## Review Focus

1. Interval arithmetic: two overlapping includes count once, an exclusion takes only its overlap (edge, middle split, other family, clamp at zero), `0.0.0.0/0` stays `any` with exclusions, `10.0.0.1/8` counts as a /8. Test: Task 1 `exclusions_subtract_only_their_overlap_after_merging` (and `everything_and_huge_lists_are_unbounded`).
2. Evaluation semantics: labels AND across keys and OR within one, a label group counts under its own `key` with sub-groups expanded once even in a cycle, exclusions remove last, `all` and workload hrefs, IP lists and virtual services never match, unscoped consumers ignore the scopes, totals are unions across scopes. Tests: Task 2 `labels_and_across_keys_or_within_and_exclusions_apply_last`, `label_groups_expand_sub_groups_once_even_in_a_cycle`, `per_scope_counts_each_scope_and_the_totals_are_unions`.
3. Request discipline in the TUI: `6` after startup costs nothing and `r` exactly one GET; `i` in a rule detail costs exactly one workloads GET and a second `i` none, while the detail stays open and shows `1 provider · 2 consumers`. Tests: Task 3 `opening_ip_lists_costs_nothing_after_startup_and_r_bypasses_once`, Task 5 `i_on_a_rule_detail_loads_workloads_once_and_a_second_i_is_free`.
4. The confirm overlay before `y`: `w` on the potentially blocked flow, `Enter`, the confirm says `impact: press i to load workloads (1 request)`, `i`, it says `impact: 1 provider · 0 consumers`; one workloads GET, then `y` still sends exactly one POST. Test: Task 6 `w_confirm_shows_the_impact_after_i_and_y_still_posts_once`.
5. CLI writes: `policy rules add Shop-Prod … --dry-run --impact` sends no write, reads the workloads once (after the plan) and prints `data.impact` next to the request; every rule write with `--impact` and profile writes off is exit 6 before any request. Tests: Task 8 `rules_add_dry_run_impact_reads_workloads_once_and_sends_nothing`, `every_write_command_is_guarded_and_needs_yes` (`ALL_WRITES` grows to 27).

---

## File structure

| File | Responsibility |
| --- | --- |
| `crates/pce/src/iplist_size.rs` (new) | `IpListSize`, `ip_list_size`, `display`, `total`, `sort_key`; interval merge/subtract per family |
| `crates/pce/src/impact.rs` (new) | `ImpactInputs`, `Impact`, `ScopeImpact`, `IpListRef`, `Side`, `RuleActors`, `rule_impact`, `rule_set_href_of`, `is_rule_href` |
| `crates/pce/src/resolve.rs` | `NameIndex::{label_group, ip_list}`; `canon` becomes `pub(crate)` |
| `crates/pce/src/lib.rs` | `pub mod iplist_size; pub mod impact;` and re-exports |
| `crates/illumio-tui/src/tui/model.rs` | `Screen::IpLists` + `short_title`; `ObjectKind` shrinks to three; `IpListRow`, `IpListsState`, `ip_list_columns`; `IpListUse`, `PolicyState::ip_list_uses`; `RuleRow`, `P/C` column; `WorkloadsState.{truncated, error}`, `impact_source`; `Model.ip_lists`, `Model::rebuild_rules` |
| `crates/illumio-tui/src/tui/msg.rs` | `Cmd::LoadIpLists`, `Msg::IpListsLoaded`; `ObjectsPayload::IpLists` removed; `WorkloadsLoaded` carries `truncated` |
| `crates/illumio-tui/src/tui/update.rs` | key `6`, IP lists keys and loads, reload after an IP list write, used-by recompute, `i`, rule detail and confirm keys, workloads result |
| `crates/illumio-tui/src/tui/app.rs` | `Cmd::LoadIpLists` task, `task_result`, workloads `truncated` |
| `crates/illumio-tui/src/tui/impact.rs` (new) | TUI glue: `Source`, `compute`, `loaded_rule`, `pc_cell`, `rule_detail_rows`, `ip_list_lines`, `confirm_rows`, `confirm_line` |
| `crates/illumio-tui/src/tui/mod.rs` | `pub mod impact;` |
| `crates/illumio-tui/src/tui/ui/screens/iplists.rs` (new), `screens/mod.rs` | the IP lists table and detail |
| `crates/illumio-tui/src/tui/ui/screens/objects.rs` | three kinds; the IP list detail moves out |
| `crates/illumio-tui/src/tui/ui/mod.rs` | `tab_labels(width)`, `row_width`, `tab_at`/`tab_x` with the width, `render_body` arm |
| `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` | `HELP_LINES`, `screen_hint`, rule detail impact rows, confirm impact line and layout |
| `crates/illumio-tui/src/tui/commands.rs` | `:iplists` |
| `crates/illumio-tui/src/tui/test_support.rs` | `model_with_ip_lists`, `with_workloads`, `rule_post_plan`; `model_with_objects` loses the IP lists |
| `crates/illumio-tui/src/cli/policy.rs` | `--impact` on `policy rules`, `policy rules impact`, `--impact` on rule and deny-rule add/update, `RuleOut`, `deny_rules_of`, `planned_impact` |
| `crates/illumio-tui/src/cli/write.rs` | `WriteOutput::{DryRunImpact, WrittenImpact}`, `run_rule_plan`, `Extra` |
| `crates/illumio-tui/src/cli/context.rs` | `emit_write_with` |
| `crates/illumio-tui/src/cli/objects.rs` | `IpListOut` (`size`) for the `iplists` listing |
| `crates/illumio-tui/tests/{tui_snapshots,tui_runtime,cli,cli_writes}.rs` | tests |
| `README.md`, `docs/architecture.md`, `docs/index.html`, `tasks/todo.md` | docs and tracking |

---

### Task 1: `pce::iplist_size`

**Files:**
- Create: `crates/pce/src/iplist_size.rs`
- Modify: `crates/pce/src/lib.rs` (`pub mod iplist_size;` after `pub mod error;` ~line 16; re-export after the `pub use resolve::{…}` block ~lines 26-29)

**Interfaces:**
- Consumes: `IpList { ip_ranges: Vec<IpRange>, fqdns: Vec<Fqdn>, .. }`, `IpRange { from_ip, to_ip: Option<String>, exclusion: Option<bool>, .. }` (`crates/pce/src/models/labels.rs` ~lines 43-89); `fixtures/ip_lists.json` (`Any (0.0.0.0/0 and ::/0)` with `0.0.0.0/0` and `::/0`; `Corp` with `10.1.0.0`-`10.1.255.255` and one FQDN).
- Produces:

```rust
// crates/pce/src/iplist_size.rs
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct IpListSize { pub v4: u128, pub v6: u128, pub unbounded: Option<String>, pub ranges: usize, pub fqdns: usize }
impl IpListSize {
    pub fn total(&self) -> u128;
    pub fn sort_key(&self) -> u64;
    pub fn display(&self) -> String; // "254", "65.5K", "1.0M", "any (0.0.0.0/0)", "3 FQDNs"
}
pub fn ip_list_size(list: &IpList) -> IpListSize;
// crates/pce/src/lib.rs
pub use iplist_size::{IpListSize, ip_list_size};
```

- [ ] **Step 1: Write the failing tests** (create `crates/pce/src/iplist_size.rs` with only this module, and add `pub mod iplist_size;` to `crates/pce/src/lib.rs` after `pub mod error;`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::IpList;
    use serde_json::{Value, json};

    fn list(ranges: Value, fqdns: Value) -> IpList {
        serde_json::from_value(json!({
            "href": "/orgs/5/sec_policy/draft/ip_lists/9", "name": "T",
            "ip_ranges": ranges, "fqdns": fqdns
        }))
        .unwrap()
    }

    fn size(ranges: Value) -> IpListSize {
        ip_list_size(&list(ranges, json!([])))
    }

    fn fixture() -> Vec<IpList> {
        serde_json::from_str(include_str!("../../../fixtures/ip_lists.json")).unwrap()
    }

    #[test]
    fn cidrs_count_per_family_and_ignore_host_bits() {
        let s = size(json!([{"from_ip": "10.0.0.0/24"}, {"from_ip": "2001:db8::/120"}]));
        assert_eq!((s.v4, s.v6), (256, 256));
        assert_eq!(s.display(), "512");
        assert_eq!(
            size(json!([{"from_ip": "10.0.0.1/8"}])).v4,
            1 << 24,
            "host bits are ignored, as the PCE does"
        );
        assert_eq!(size(json!([{"from_ip": "10.0.0.7/32"}])).v4, 1);
    }

    #[test]
    fn ranges_are_inclusive_and_single_addresses_count_one() {
        let s = size(json!([{"from_ip": "10.1.0.0", "to_ip": "10.1.255.255"}]));
        assert_eq!(s.v4, 65_536);
        assert_eq!(s.display(), "65.5K");
        let s = size(json!([
            {"from_ip": "10.0.0.1", "to_ip": "10.0.0.254"},
            {"from_ip": "fe80::1"}
        ]));
        assert_eq!((s.v4, s.v6), (254, 1));
        assert_eq!(size(json!([{"from_ip": "10.0.0.9"}])).display(), "1");
    }

    #[test]
    fn exclusions_subtract_only_their_overlap_after_merging() {
        // Two overlapping includes count once; the exclusion takes its half.
        let s = size(json!([
            {"from_ip": "10.0.0.0/24"},
            {"from_ip": "10.0.0.128/25"},
            {"from_ip": "10.0.0.0/25", "exclusion": true}
        ]));
        assert_eq!(s.v4, 128);
        // An exclusion elsewhere, or in the other family, takes nothing.
        let s = size(json!([
            {"from_ip": "10.0.0.0/24"},
            {"from_ip": "10.1.0.0/24", "exclusion": true},
            {"from_ip": "::/0", "exclusion": true}
        ]));
        assert_eq!((s.v4, s.v6), (256, 0));
        // Across the edge of a range, and inside one (it splits in two).
        let s = size(json!([
            {"from_ip": "10.0.0.10", "to_ip": "10.0.0.19"},
            {"from_ip": "10.0.0.15", "to_ip": "10.0.0.30", "exclusion": true}
        ]));
        assert_eq!(s.v4, 5);
        let s = size(json!([
            {"from_ip": "10.0.0.0/24"},
            {"from_ip": "10.0.0.100", "to_ip": "10.0.0.109", "exclusion": true}
        ]));
        assert_eq!(s.v4, 246);
        // More excluded than included clamps at zero.
        let s = size(json!([
            {"from_ip": "10.0.0.5"},
            {"from_ip": "10.0.0.0/8", "exclusion": true}
        ]));
        assert_eq!(s.v4, 0);
        assert_eq!(s.display(), "0");
    }

    #[test]
    fn everything_and_huge_lists_are_unbounded() {
        let any4 = size(json!([{"from_ip": "0.0.0.0/0"}]));
        assert_eq!(any4.unbounded.as_deref(), Some("0.0.0.0/0"));
        assert_eq!(any4.v4, 1 << 32);
        assert_eq!(any4.display(), "any (0.0.0.0/0)");
        assert_eq!(any4.sort_key(), u64::MAX);
        let any6 = size(json!([{"from_ip": "::/0"}]));
        assert_eq!(any6.unbounded.as_deref(), Some("::/0"));
        assert_eq!(any6.v6, u128::MAX, "2^128 saturates");
        // The recorded "internet" shape: everything minus a private range is still any.
        let internet = size(json!([
            {"from_ip": "0.0.0.0/0", "exclusion": false},
            {"from_ip": "10.0.0.1/8", "exclusion": true}
        ]));
        assert_eq!(internet.unbounded.as_deref(), Some("0.0.0.0/0"));
        assert_eq!(internet.v4, (1 << 32) - (1 << 24));
        let a8 = size(json!([{"from_ip": "10.0.0.0/8"}]));
        assert_eq!(a8.unbounded.as_deref(), Some("≥16M addresses"));
        assert_eq!(a8.display(), "any (≥16M addresses)");
        let a9 = size(json!([{"from_ip": "10.0.0.0/9"}]));
        assert_eq!(a9.unbounded, None);
        assert_eq!(a9.display(), "8.4M");
        assert!(a8.sort_key() > a9.sort_key());
        assert_eq!(a9.sort_key(), 1 << 23);
    }

    #[test]
    fn fqdn_only_lists_and_parse_errors() {
        let three = ip_list_size(&list(
            json!([]),
            json!([{"fqdn": "a.example"}, {"fqdn": "b.example"}, {"fqdn": "c.example"}]),
        ));
        assert_eq!((three.v4, three.v6, three.ranges, three.fqdns), (0, 0, 0, 3));
        assert_eq!(three.display(), "3 FQDNs");
        let one = ip_list_size(&list(json!([]), json!([{"fqdn": "a.example"}])));
        assert_eq!(one.display(), "1 FQDN");
        let bad = size(json!([
            {"from_ip": "not-an-ip"},
            {"from_ip": "10.0.0.0/33"},
            {"from_ip": "10.0.0.9", "to_ip": "10.0.0.1"},
            {"from_ip": "10.0.0.1", "to_ip": "::1"},
            {"from_ip": "10.0.0.1"}
        ]));
        assert_eq!(
            (bad.v4, bad.v6, bad.ranges),
            (1, 0, 5),
            "unparsable entries count 0 but are ranges"
        );
        assert_eq!(size(json!([])).display(), "0");
    }

    #[test]
    fn the_fixture_lists_and_human_units() {
        let lists = fixture();
        assert_eq!(ip_list_size(&lists[0]).display(), "any (0.0.0.0/0)");
        let corp = ip_list_size(&lists[1]);
        assert_eq!((corp.v4, corp.v6, corp.ranges, corp.fqdns), (65_536, 0, 1, 1));
        assert_eq!(corp.total(), 65_536);
        assert_eq!(corp.display(), "65.5K");
        assert_eq!(human(999), "999");
        assert_eq!(human(1_000), "1.0K");
        assert_eq!(human(1_500_000), "1.5M");
        assert_eq!(human(2_000_000_000), "2.0G");
    }

    #[test]
    fn json_carries_the_counts_as_strings() {
        let lists = fixture();
        assert_eq!(
            serde_json::to_value(ip_list_size(&lists[1])).unwrap(),
            json!({"v4": "65536", "v6": "0", "unbounded": null, "ranges": 1, "fqdns": 1})
        );
        let any = serde_json::to_value(ip_list_size(&lists[0])).unwrap();
        assert_eq!(any["v6"], json!(u128::MAX.to_string()));
        assert_eq!(any["unbounded"], "0.0.0.0/0");
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p pce --lib iplist_size::`
Expected: FAIL to compile (`cannot find type IpListSize`, `cannot find function ip_list_size`, `human`).

- [ ] **Step 3: Implement** (put this above the test module in `crates/pce/src/iplist_size.rs`)

```rust
//! How many addresses an IP list covers, per family, with its exclusions
//! taken out. Pure std arithmetic (IPv4 as `u32`, IPv6 as `u128`); never a
//! request.
use crate::models::IpList;
use serde::{Serialize, Serializer};
use std::net::IpAddr;

/// The size of one IP list. `v4` and `v6` count addresses after exclusions;
/// `unbounded` says why the list counts as "any": an included `0.0.0.0/0` or
/// `::/0`, or at least 2^24 addresses. JSON carries `v4` and `v6` as strings
/// (an IPv6 count does not fit a JSON number).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct IpListSize {
    #[serde(serialize_with = "as_string")]
    pub v4: u128,
    #[serde(serialize_with = "as_string")]
    pub v6: u128,
    pub unbounded: Option<String>,
    /// Range entries, parsable or not.
    pub ranges: usize,
    pub fqdns: usize,
}

fn as_string<S: Serializer>(n: &u128, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&n.to_string())
}

/// From here on a list counts as unbounded: 2^24 addresses (a /8).
const UNBOUNDED_AT: u128 = 1 << 24;
/// The last IPv4 address as an integer.
const V4_LAST: u128 = u32::MAX as u128;

/// `[first, last]`, inclusive, in one family's integer space.
type Span = (u128, u128);

/// An address as `(is IPv4, integer)`.
fn int(ip: IpAddr) -> (bool, u128) {
    match ip {
        IpAddr::V4(a) => (true, u128::from(u32::from(a))),
        IpAddr::V6(a) => (false, u128::from(a)),
    }
}

/// One range entry (`A`, `A/N`, or `A` to `B`) as a span of its family;
/// `None` when it does not parse. A CIDR's host bits are ignored, as the PCE
/// does (`10.0.0.1/8` is `10.0.0.0/8`).
fn span(from: &str, to: Option<&str>) -> Option<(bool, Span)> {
    let from = from.trim();
    if let Some((addr, bits)) = from.split_once('/') {
        let (v4, start) = int(addr.trim().parse().ok()?);
        let width: u32 = if v4 { 32 } else { 128 };
        let bits: u32 = bits.trim().parse().ok()?;
        if bits > width {
            return None;
        }
        let host = width - bits;
        let mask = if host == 128 {
            u128::MAX
        } else {
            (1u128 << host) - 1
        };
        let first = start & !mask;
        return Some((v4, (first, first | mask)));
    }
    let (v4, first) = int(from.parse().ok()?);
    let last = match to {
        Some(t) => {
            let (to_v4, last) = int(t.trim().parse().ok()?);
            if to_v4 != v4 || last < first {
                return None;
            }
            last
        }
        None => first,
    };
    Some((v4, (first, last)))
}

/// Sorted, with overlapping and adjacent spans joined.
fn merge(mut spans: Vec<Span>) -> Vec<Span> {
    spans.sort_unstable();
    let mut out: Vec<Span> = Vec::with_capacity(spans.len());
    for (first, last) in spans {
        match out.last_mut() {
            Some(prev) if first <= prev.1.saturating_add(1) => prev.1 = prev.1.max(last),
            _ => out.push((first, last)),
        }
    }
    out
}

/// `included` minus every excluded span; only overlaps are taken out.
fn subtract(included: Vec<Span>, excluded: &[Span]) -> Vec<Span> {
    let mut left = included;
    for &(ex_first, ex_last) in excluded {
        let mut next = Vec::with_capacity(left.len() + 1);
        for (first, last) in left {
            if ex_last < first || ex_first > last {
                next.push((first, last));
                continue;
            }
            if ex_first > first {
                next.push((first, ex_first - 1));
            }
            if ex_last < last {
                next.push((ex_last + 1, last));
            }
        }
        left = next;
    }
    left
}

/// Addresses in `spans`, saturating at `u128::MAX` (all of IPv6).
fn count(spans: &[Span]) -> u128 {
    spans.iter().fold(0u128, |n, (first, last)| {
        n.saturating_add((last - first).saturating_add(1))
    })
}

/// `999`, `65.5K`, `1.5M`, `2.0G`.
fn human(n: u128) -> String {
    let f = n as f64;
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}K", f / 1e3),
        1_000_000..1_000_000_000 => format!("{:.1}M", f / 1e6),
        _ => format!("{:.1}G", f / 1e9),
    }
}

pub fn ip_list_size(list: &IpList) -> IpListSize {
    // Index 0 is IPv4, 1 is IPv6.
    let mut include: [Vec<Span>; 2] = [Vec::new(), Vec::new()];
    let mut exclude: [Vec<Span>; 2] = [Vec::new(), Vec::new()];
    let mut unbounded = None;
    for r in &list.ip_ranges {
        let Some((v4, s)) = span(&r.from_ip, r.to_ip.as_deref()) else {
            continue;
        };
        let family = usize::from(!v4);
        if r.exclusion.unwrap_or(false) {
            exclude[family].push(s);
            continue;
        }
        if unbounded.is_none() && s == (0, if v4 { V4_LAST } else { u128::MAX }) {
            unbounded = Some(if v4 { "0.0.0.0/0" } else { "::/0" }.to_string());
        }
        include[family].push(s);
    }
    let [inc_v4, inc_v6] = include;
    let v4 = count(&subtract(merge(inc_v4), &exclude[0]));
    let v6 = count(&subtract(merge(inc_v6), &exclude[1]));
    if unbounded.is_none() && v4.saturating_add(v6) >= UNBOUNDED_AT {
        unbounded = Some("≥16M addresses".to_string());
    }
    IpListSize {
        v4,
        v6,
        unbounded,
        ranges: list.ip_ranges.len(),
        fqdns: list.fqdns.len(),
    }
}

impl IpListSize {
    /// IPv4 plus IPv6 addresses, saturating.
    pub fn total(&self) -> u128 {
        self.v4.saturating_add(self.v6)
    }

    /// A size column's sort key: the address count, unbounded lists last.
    pub fn sort_key(&self) -> u64 {
        if self.unbounded.is_some() {
            return u64::MAX;
        }
        u64::try_from(self.total()).map_or(u64::MAX - 1, |n| n.min(u64::MAX - 1))
    }

    /// `254`, `65.5K`, `1.0M`, `any (0.0.0.0/0)`, `3 FQDNs` when it has no ranges.
    pub fn display(&self) -> String {
        if let Some(why) = &self.unbounded {
            return format!("any ({why})");
        }
        if self.ranges == 0 && self.fqdns > 0 {
            return if self.fqdns == 1 {
                "1 FQDN".to_string()
            } else {
                format!("{} FQDNs", self.fqdns)
            };
        }
        human(self.total())
    }
}
```

In `crates/pce/src/lib.rs`, after the `pub use resolve::{…};` block, add:

```rust
pub use iplist_size::{IpListSize, ip_list_size};
```

- [ ] **Step 4: Run the tests**

Run: `cargo fmt --all && cargo test -p pce --lib iplist_size::`
Expected: PASS (7 tests).

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p pce
git add crates/pce/src/iplist_size.rs crates/pce/src/lib.rs
git commit -m "feat(pce): IP list sizes per address family, exclusions subtracted

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: `pce::impact`

**Files:**
- Create: `crates/pce/src/impact.rs`
- Modify: `crates/pce/src/resolve.rs` (`canon` ~line 11 becomes `pub(crate)`; `label_group` and `ip_list` after `ip_list_href` ~line 148; one test at the end of `mod tests` ~line 710)
- Modify: `crates/pce/src/lib.rs` (`pub mod impact;` after `pub mod error;`; re-export next to the Task 1 one)

**Interfaces:**
- Consumes: `IpListSize`, `ip_list_size` (Task 1); `NameIndex::{new, label, label_groups (map), ip_lists (map)}`, `canon`, `describe_actor` (`crates/pce/src/resolve.rs` ~lines 11, 28-63, 96, 287); `Actor { actors, label, label_group, workload, virtual_service, virtual_server, ip_list, exclusion, .. }`, `Rule { providers, consumers, unscoped_consumers, .. }`, `DenyRule { providers, consumers, .. }` (`models/policy.rs` ~lines 33-138); `Workload { href, labels: Vec<WorkloadLabel { href, .. }>, .. }` (`models/workloads.rs` ~lines 23-58); `LabelGroup { key, labels: Vec<Ref>, sub_groups: Vec<Ref>, .. }` (`models/labels.rs` ~line 30); fixtures `workloads.json` (aaaa-1 role=web/app=shop/env=prod: labels 1,3,4; aaaa-2 role=db/app=shop/env=prod: 2,3,4; aaaa-3 unmanaged, no labels; aaaa-4 managed, no labels), `labels.json`, `label_groups.json` (group 10 `All prod envs`, key env, label 4), `ip_lists.json`, `rule_sets.json` (100 Shop-Prod scope app=shop & env=prod; 101 Global-Infra `[[]]`, `ams` provider, group 10 consumer).
- Produces:

```rust
// crates/pce/src/resolve.rs
pub(crate) fn canon(href: &str) -> Cow<'_, str>;
impl NameIndex {
    pub fn label_group(&self, href: &str) -> Option<&LabelGroup>;
    pub fn ip_list(&self, href: &str) -> Option<&IpList>;
}
// crates/pce/src/impact.rs
#[derive(Debug, Clone, Copy)]
pub struct ImpactInputs<'a> { pub workloads: &'a [Workload], pub index: &'a NameIndex, pub truncated: bool }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)] #[serde(rename_all = "lowercase")]
pub enum Side { Provider, Consumer }
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IpListRef { pub name: String, pub href: String, pub size: Option<IpListSize>, pub side: Side }
impl IpListRef { pub fn line(&self) -> String; }
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScopeImpact { pub scope: Vec<String>, pub providers: usize, pub consumers: usize }
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Impact { pub providers: usize, pub consumers: usize, pub per_scope: Vec<ScopeImpact>,
                    pub ip_lists: Vec<IpListRef>, pub unsupported: Vec<String>, pub truncated: bool }
impl Impact { pub fn pc(&self) -> String; pub fn summary(&self) -> String; }
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuleActors { pub providers: Vec<Actor>, pub consumers: Vec<Actor>, pub unscoped_consumers: bool }
impl RuleActors {
    pub fn of_rule(r: &Rule) -> Self;
    pub fn of_deny(d: &DenyRule) -> Self;
    pub fn planned(body: Option<&serde_json::Value>, base: Option<&RuleActors>) -> Self;
}
pub fn rule_impact(scopes: &[Vec<Actor>], providers: &[Actor], consumers: &[Actor], unscoped_consumers: bool, inputs: &ImpactInputs) -> Impact;
pub fn rule_set_href_of(path: &str) -> Option<&str>;
pub fn is_rule_href(path: &str) -> bool;
// crates/pce/src/lib.rs
pub use impact::{Impact, ImpactInputs, IpListRef, RuleActors, ScopeImpact, Side, rule_impact};
```

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `crates/pce/src/resolve.rs`:

```rust
    #[test]
    fn label_groups_and_ip_lists_are_found_by_href_in_either_scope() {
        let idx = index();
        assert_eq!(
            idx.label_group("/orgs/5/sec_policy/active/label_groups/10")
                .map(|g| g.key.as_str()),
            Some("env")
        );
        assert_eq!(
            idx.ip_list("/orgs/5/sec_policy/draft/ip_lists/21")
                .map(|l| l.name.as_str()),
            Some("Corp")
        );
        assert_eq!(
            idx.ip_list("/orgs/5/sec_policy/active/ip_lists/20")
                .map(|l| l.ip_ranges.len()),
            Some(2)
        );
        assert!(
            idx.label_group("/orgs/5/sec_policy/draft/label_groups/99")
                .is_none()
        );
        assert!(idx.ip_list("/orgs/5/labels/1").is_none());
    }
```

Create `crates/pce/src/impact.rs` with only this module, and add `pub mod impact;` to `crates/pce/src/lib.rs` after `pub mod error;`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{IpList, Label, LabelGroup, RuleSet};
    use serde_json::{Value, json};

    /// The fixture index plus two role groups that contain each other: a
    /// sub-group, a cycle, and an active href among the sub-groups.
    fn index() -> NameIndex {
        let labels: Vec<Label> =
            serde_json::from_str(include_str!("../../../fixtures/labels.json")).unwrap();
        let mut groups: Vec<LabelGroup> =
            serde_json::from_str(include_str!("../../../fixtures/label_groups.json")).unwrap();
        groups.push(
            serde_json::from_value(json!({
                "href": "/orgs/5/sec_policy/draft/label_groups/12", "name": "Web tier", "key": "role",
                "labels": [{"href": "/orgs/5/labels/1"}],
                "sub_groups": [{"href": "/orgs/5/sec_policy/draft/label_groups/13"}]
            }))
            .unwrap(),
        );
        groups.push(
            serde_json::from_value(json!({
                "href": "/orgs/5/sec_policy/draft/label_groups/13", "name": "Db tier", "key": "role",
                "labels": [{"href": "/orgs/5/labels/2"}],
                "sub_groups": [{"href": "/orgs/5/sec_policy/active/label_groups/12"}]
            }))
            .unwrap(),
        );
        let lists: Vec<IpList> =
            serde_json::from_str(include_str!("../../../fixtures/ip_lists.json")).unwrap();
        NameIndex::new(&labels, &groups, &lists, &[])
    }

    fn workloads() -> Vec<Workload> {
        serde_json::from_str(include_str!("../../../fixtures/workloads.json")).unwrap()
    }

    fn rule_sets() -> Vec<RuleSet> {
        serde_json::from_str(include_str!("../../../fixtures/rule_sets.json")).unwrap()
    }

    fn actors(v: Value) -> Vec<Actor> {
        serde_json::from_value(v).unwrap()
    }

    fn label(n: u32) -> Value {
        json!({"label": {"href": format!("/orgs/5/labels/{n}")}})
    }

    fn all() -> Value {
        json!([{"actors": "ams"}])
    }

    /// Shop-Prod's one scope: app=shop & env=prod.
    fn shop() -> Vec<Vec<Actor>> {
        vec![actors(json!([label(3), label(4)]))]
    }

    fn run(scopes: &[Vec<Actor>], providers: Value, consumers: Value, unscoped: bool) -> Impact {
        let idx = index();
        let w = workloads();
        let inputs = ImpactInputs {
            workloads: &w,
            index: &idx,
            truncated: false,
        };
        rule_impact(scopes, &actors(providers), &actors(consumers), unscoped, &inputs)
    }

    const EVERYWHERE: &[Vec<Actor>] = &[];

    #[test]
    fn the_fixture_rules_count_their_scoped_sides() {
        let sets = rule_sets();
        let idx = index();
        let w = workloads();
        let inputs = ImpactInputs {
            workloads: &w,
            index: &idx,
            truncated: false,
        };
        let at = |a: RuleActors, scopes: &[Vec<Actor>]| {
            let i = rule_impact(scopes, &a.providers, &a.consumers, a.unscoped_consumers, &inputs);
            (i.providers, i.consumers)
        };
        let shop = &sets[0];
        assert_eq!(at(RuleActors::of_rule(&shop.rules[0]), &shop.scopes), (1, 1), "db ← web");
        assert_eq!(
            at(RuleActors::of_rule(&shop.rules[1]), &shop.scopes),
            (1, 0),
            "an IP list consumer matches no workload"
        );
        assert_eq!(at(RuleActors::of_deny(&shop.deny_rules[0]), &shop.scopes), (2, 0));
        assert_eq!(
            at(RuleActors::of_deny(&shop.deny_rules[1]), &shop.scopes),
            (1, 2),
            "a deny rule's consumers stay inside the scope"
        );
        let infra = &sets[1];
        assert_eq!(
            at(RuleActors::of_rule(&infra.rules[0]), &infra.scopes),
            (4, 2),
            "an empty scope is everywhere; the group stands for env=prod"
        );
        assert_eq!(
            at(RuleActors::of_rule(&infra.rules[0]), EVERYWHERE),
            (4, 2),
            "no scopes at all is one empty scope"
        );
    }

    #[test]
    fn per_scope_counts_each_scope_and_the_totals_are_unions() {
        let scopes = vec![shop()[0].clone(), actors(json!([label(2)]))];
        let i = run(&scopes, all(), json!([label(1)]), false);
        assert_eq!((i.providers, i.consumers), (2, 1));
        assert_eq!(
            i.per_scope,
            vec![
                ScopeImpact {
                    scope: vec!["app=shop".into(), "env=prod".into()],
                    providers: 2,
                    consumers: 1
                },
                ScopeImpact {
                    scope: vec!["role=db".into()],
                    providers: 1,
                    consumers: 0
                },
            ]
        );
        let unscoped = run(&shop(), json!([label(1)]), all(), true);
        assert_eq!((unscoped.providers, unscoped.consumers), (1, 4));
        let scoped = run(&shop(), json!([label(1)]), all(), false);
        assert_eq!(scoped.consumers, 2);
        assert_eq!(run(EVERYWHERE, all(), all(), false).per_scope[0].scope, Vec::<String>::new());
    }

    #[test]
    fn labels_and_across_keys_or_within_and_exclusions_apply_last() {
        // (role=web OR role=db) AND app=shop
        assert_eq!(run(EVERYWHERE, json!([label(1), label(2), label(3)]), all(), false).providers, 2);
        // role=web AND env=prod
        assert_eq!(run(EVERYWHERE, json!([label(1), label(4)]), all(), false).providers, 1);
        // all minus env=prod, as a label and as a group
        let minus_label = json!([{"actors": "ams"}, {"label": {"href": "/orgs/5/labels/4"}, "exclusion": true}]);
        assert_eq!(run(EVERYWHERE, minus_label, all(), false).providers, 2);
        let minus_group = json!([
            {"actors": "ams"},
            {"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/10"}, "exclusion": true}
        ]);
        assert_eq!(run(EVERYWHERE, minus_group, all(), false).providers, 2);
        // Only an exclusion: nothing positive, nothing matches.
        let only = json!([{"label": {"href": "/orgs/5/labels/4"}, "exclusion": true}]);
        assert_eq!(run(EVERYWHERE, only, all(), false).providers, 0);
        // A label the index does not know is a key of its own: nobody carries it.
        assert_eq!(run(EVERYWHERE, json!([label(99)]), all(), false).providers, 0);
        // A scope exclusion keeps out what carries it.
        let scope = vec![actors(json!([label(3), {"label": {"href": "/orgs/5/labels/1"}, "exclusion": true}]))];
        assert_eq!(run(&scope, all(), all(), false).providers, 1, "app=shop but not role=web");
    }

    #[test]
    fn label_groups_expand_sub_groups_once_even_in_a_cycle() {
        let web_tier = json!([{"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/12"}}]);
        assert_eq!(
            run(EVERYWHERE, web_tier, all(), false).providers,
            2,
            "web, plus db through the sub-group"
        );
        // An active href folds to draft; the group's key is role, ANDed with app=shop.
        let mixed = json!([{"label_group": {"href": "/orgs/5/sec_policy/active/label_groups/13"}}, label(3)]);
        assert_eq!(run(EVERYWHERE, mixed, all(), false).providers, 2);
        // A scope member may be a group: env=prod through "All prod envs".
        let scope = vec![actors(json!([{"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/10"}}]))];
        assert_eq!(run(&scope, all(), all(), false).providers, 2);
        // A group the index does not know matches nothing.
        let unknown = json!([{"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/77"}}]);
        assert_eq!(run(EVERYWHERE, unknown, all(), false).providers, 0);
    }

    #[test]
    fn workload_actors_ip_lists_and_virtual_services() {
        let i = run(
            EVERYWHERE,
            json!([{"workload": {"href": "/orgs/5/workloads/aaaa-3"}}]),
            json!([
                {"ip_list": {"href": "/orgs/5/sec_policy/active/ip_lists/21"}},
                {"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/99"}},
                {"virtual_service": {"href": "/orgs/5/sec_policy/draft/virtual_services/v1"}}
            ]),
            false,
        );
        assert_eq!(
            (i.providers, i.consumers),
            (1, 0),
            "the unmanaged workload by href; lists and services match nobody"
        );
        assert_eq!(i.ip_lists[0].name, "Corp");
        assert_eq!(i.ip_lists[0].side, Side::Consumer);
        assert_eq!(
            i.ip_lists[0].size.as_ref().map(IpListSize::display).as_deref(),
            Some("65.5K")
        );
        assert_eq!(i.ip_lists[0].line(), "Corp: 65.5K addresses");
        assert_eq!(i.ip_lists[1].name, "/orgs/5/sec_policy/draft/ip_lists/99");
        assert_eq!(i.ip_lists[1].size, None);
        assert_eq!(i.ip_lists[1].line(), "/orgs/5/sec_policy/draft/ip_lists/99: ? addresses");
        assert_eq!(
            i.unsupported,
            vec!["/orgs/5/sec_policy/draft/virtual_services/v1".to_string()]
        );
        let any = run(
            EVERYWHERE,
            json!([{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/20"}}]),
            all(),
            false,
        );
        assert_eq!(any.ip_lists[0].side, Side::Provider);
        assert_eq!(any.ip_lists[0].line(), "Any (0.0.0.0/0 and ::/0): any (0.0.0.0/0)");
    }

    #[test]
    fn truncated_is_copied_and_counts_never_exceed_the_list() {
        let idx = index();
        let w = workloads();
        let inputs = ImpactInputs {
            workloads: &w,
            index: &idx,
            truncated: true,
        };
        let i = rule_impact(&[], &actors(all()), &actors(all()), false, &inputs);
        assert!(i.truncated);
        assert_eq!(i.pc(), "≥4/≥4");
        assert_eq!(i.summary(), "≥4 providers · ≥4 consumers (workload list truncated)");
        // Whatever the rule, no side counts more workloads than there are,
        // and no scope more than the total.
        for rs in rule_sets() {
            let sides = rs
                .rules
                .iter()
                .map(RuleActors::of_rule)
                .chain(rs.deny_rules.iter().map(RuleActors::of_deny));
            for a in sides {
                let i = rule_impact(&rs.scopes, &a.providers, &a.consumers, a.unscoped_consumers, &inputs);
                assert!(i.providers <= w.len() && i.consumers <= w.len());
                for s in &i.per_scope {
                    assert!(s.providers <= i.providers && s.consumers <= i.consumers);
                }
            }
        }
    }

    #[test]
    fn summary_and_pc_read_naturally() {
        let i = run(&shop(), json!([label(2)]), json!([label(1)]), false);
        assert_eq!(i.summary(), "1 provider · 1 consumer");
        assert_eq!(i.pc(), "1/1");
        let none = run(&shop(), json!([label(5)]), all(), false);
        assert_eq!(none.summary(), "0 providers · 2 consumers");
        assert_eq!(none.pc(), "0/2");
    }

    #[test]
    fn planned_actors_take_the_body_over_the_loaded_rule() {
        let sets = rule_sets();
        let base = RuleActors::of_rule(&sets[0].rules[1]); // web ← Corp, unscoped
        assert!(base.unscoped_consumers);
        let body = json!({"consumers": [
            {"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}},
            {"actors": "ams"}
        ]});
        let p = RuleActors::planned(Some(&body), Some(&base));
        assert_eq!(p.providers, base.providers, "an update without providers keeps them");
        assert_eq!(p.consumers.len(), 2);
        assert!(p.unscoped_consumers);
        let scoped = RuleActors::planned(Some(&json!({"unscoped_consumers": false})), Some(&base));
        assert!(!scoped.unscoped_consumers);
        let created = RuleActors::planned(
            Some(&json!({"providers": [label(1)], "consumers": [label(2)], "unscoped_consumers": true})),
            None,
        );
        assert_eq!(
            (created.providers.len(), created.consumers.len(), created.unscoped_consumers),
            (1, 1, true)
        );
        assert_eq!(RuleActors::planned(None, None), RuleActors::default());
        assert!(!RuleActors::of_deny(&sets[0].deny_rules[0]).unscoped_consumers);
    }

    #[test]
    fn rule_write_paths_name_their_ruleset() {
        let rs = "/orgs/5/sec_policy/draft/rule_sets/100";
        assert_eq!(rule_set_href_of(&format!("{rs}/sec_rules")), Some(rs));
        assert_eq!(rule_set_href_of(&format!("{rs}/deny_rules/8")), Some(rs));
        assert_eq!(rule_set_href_of(rs), Some(rs));
        assert_eq!(rule_set_href_of("/orgs/5/sec_policy/draft/rule_sets/"), None);
        assert_eq!(rule_set_href_of("/orgs/5/labels/1"), None);
        assert!(!is_rule_href(&format!("{rs}/sec_rules")));
        assert!(is_rule_href(&format!("{rs}/sec_rules/1")));
        assert!(is_rule_href(&format!("{rs}/deny_rules/8")));
        assert!(!is_rule_href(rs));
    }

    #[test]
    fn json_has_the_documented_shape() {
        let i = run(
            &shop(),
            json!([label(1)]),
            json!([{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}]),
            true,
        );
        assert_eq!(
            serde_json::to_value(&i).unwrap(),
            json!({
                "providers": 1, "consumers": 0,
                "per_scope": [{"scope": ["app=shop", "env=prod"], "providers": 1, "consumers": 0}],
                "ip_lists": [{
                    "name": "Corp", "href": "/orgs/5/sec_policy/draft/ip_lists/21",
                    "size": {"v4": "65536", "v6": "0", "unbounded": null, "ranges": 1, "fqdns": 1},
                    "side": "consumer"
                }],
                "unsupported": [], "truncated": false
            })
        );
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p pce --lib impact:: resolve::tests::label_groups_and_ip_lists`
Expected: FAIL to compile (`no method named label_group`, `cannot find function rule_impact`, `cannot find type ImpactInputs`, …).

- [ ] **Step 3: Implement**

In `crates/pce/src/resolve.rs` change `fn canon(` to `pub(crate) fn canon(` and add after `ip_list_href`:

```rust
    /// The label group with this href; an active href finds its draft.
    pub fn label_group(&self, href: &str) -> Option<&LabelGroup> {
        self.label_groups.get(canon(href).as_ref())
    }

    /// The IP list with this href; an active href finds its draft.
    pub fn ip_list(&self, href: &str) -> Option<&IpList> {
        self.ip_lists.get(canon(href).as_ref())
    }
```

Above the test module in `crates/pce/src/impact.rs`:

```rust
//! Rule impact: how many workloads a rule's providers and consumers match,
//! evaluated with its ruleset's scopes over a workload list. Pure: the
//! caller brings the workloads (one cached read) and the name index; IP
//! lists are reported with their size, never expanded into peers.
use crate::iplist_size::{IpListSize, ip_list_size};
use crate::models::{Actor, DenyRule, Rule, Workload};
use crate::resolve::{NameIndex, canon, describe_actor};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

/// What a rule is evaluated over. Label groups and IP lists are looked up
/// in `index`, which is built from the same lists.
#[derive(Debug, Clone, Copy)]
pub struct ImpactInputs<'a> {
    pub workloads: &'a [Workload],
    pub index: &'a NameIndex,
    /// The workload read was cut short (`Cached::truncated`).
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Provider,
    Consumer,
}

/// An IP-list actor of the rule, with its size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IpListRef {
    /// The list's name, or its href when the index does not know it.
    pub name: String,
    pub href: String,
    /// `None` when the index does not know the list.
    pub size: Option<IpListSize>,
    pub side: Side,
}

impl IpListRef {
    /// `Corp: 65.5K addresses`, `Any: any (0.0.0.0/0)`, `DNS: 3 FQDNs`,
    /// `<href>: ? addresses`.
    pub fn line(&self) -> String {
        match &self.size {
            None => format!("{}: ? addresses", self.name),
            Some(s) if s.unbounded.is_some() || (s.ranges == 0 && s.fqdns > 0) => {
                format!("{}: {}", self.name, s.display())
            }
            Some(s) => format!("{}: {} addresses", self.name, s.display()),
        }
    }
}

/// One scope's share of the impact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScopeImpact {
    /// The scope's members by name (`app=shop`, `group:Prod`); empty: everywhere.
    pub scope: Vec<String>,
    pub providers: usize,
    pub consumers: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Impact {
    /// Workloads matched as providers in any scope.
    pub providers: usize,
    /// Workloads matched as consumers in any scope (anywhere when unscoped).
    pub consumers: usize,
    pub per_scope: Vec<ScopeImpact>,
    pub ip_lists: Vec<IpListRef>,
    /// Virtual service and virtual server hrefs: not evaluated, not counted.
    pub unsupported: Vec<String>,
    /// The workload list was truncated: the counts are lower bounds.
    pub truncated: bool,
}

fn counted(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

impl Impact {
    /// `12/3`; `≥12/≥3` when the workload list was truncated.
    pub fn pc(&self) -> String {
        let ge = if self.truncated { "≥" } else { "" };
        format!("{ge}{}/{ge}{}", self.providers, self.consumers)
    }

    /// `12 providers · 3 consumers`; with a truncated list
    /// `≥12 providers · ≥3 consumers (workload list truncated)`.
    pub fn summary(&self) -> String {
        let ge = if self.truncated { "≥" } else { "" };
        let mut s = format!(
            "{ge}{} · {ge}{}",
            counted(self.providers, "provider"),
            counted(self.consumers, "consumer")
        );
        if self.truncated {
            s.push_str(" (workload list truncated)");
        }
        s
    }
}

/// The sides of a rule the evaluator needs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuleActors {
    pub providers: Vec<Actor>,
    pub consumers: Vec<Actor>,
    pub unscoped_consumers: bool,
}

impl RuleActors {
    pub fn of_rule(r: &Rule) -> Self {
        Self {
            providers: r.providers.clone(),
            consumers: r.consumers.clone(),
            unscoped_consumers: r.unscoped_consumers.unwrap_or(false),
        }
    }

    /// A deny rule's consumers are always scoped.
    pub fn of_deny(d: &DenyRule) -> Self {
        Self {
            providers: d.providers.clone(),
            consumers: d.consumers.clone(),
            unscoped_consumers: false,
        }
    }

    /// What a planned rule write leaves: the body's `providers`,
    /// `consumers` and `unscoped_consumers` where it has them, `base`'s
    /// (the loaded rule, for an update) otherwise.
    pub fn planned(body: Option<&Value>, base: Option<&RuleActors>) -> Self {
        let base = base.cloned().unwrap_or_default();
        let side = |key: &str, old: Vec<Actor>| {
            body.and_then(|b| b.get(key))
                .and_then(|v| serde_json::from_value::<Vec<Actor>>(v.clone()).ok())
                .unwrap_or(old)
        };
        Self {
            providers: side("providers", base.providers),
            consumers: side("consumers", base.consumers),
            unscoped_consumers: body
                .and_then(|b| b.get("unscoped_consumers"))
                .and_then(Value::as_bool)
                .unwrap_or(base.unscoped_consumers),
        }
    }
}

/// `/orgs/5/sec_policy/draft/rule_sets/100` for a rule write's path
/// (`…/rule_sets/100/sec_rules` to create, `…/sec_rules/1` to update).
pub fn rule_set_href_of(path: &str) -> Option<&str> {
    const SEGMENT: &str = "/rule_sets/";
    let start = path.find(SEGMENT)? + SEGMENT.len();
    let id = path[start..].find('/').unwrap_or(path.len() - start);
    (id > 0).then(|| &path[..start + id])
}

/// Whether a rule write path names one rule (an update) rather than a
/// ruleset's rule collection (a create).
pub fn is_rule_href(path: &str) -> bool {
    let mut parts = path.trim_end_matches('/').rsplit('/');
    let last = parts.next().unwrap_or("");
    !last.is_empty() && matches!(parts.next(), Some("sec_rules" | "deny_rules"))
}

/// The label hrefs a group stands for: its labels and its sub-groups',
/// recursively. Each group is visited once, so a cycle ends; an unknown
/// group adds nothing.
fn group_members(href: &str, idx: &NameIndex) -> HashSet<String> {
    let mut members = HashSet::new();
    let mut seen = HashSet::new();
    let mut todo = vec![canon(href).into_owned()];
    while let Some(h) = todo.pop() {
        if !seen.insert(h.clone()) {
            continue;
        }
        if let Some(g) = idx.label_group(&h) {
            members.extend(g.labels.iter().map(|r| r.href.clone()));
            todo.extend(g.sub_groups.iter().map(|r| canon(&r.href).into_owned()));
        }
    }
    members
}

/// One side's actors, ready to test workloads against.
#[derive(Default)]
struct Matcher {
    all: bool,
    workloads: HashSet<String>,
    /// Label key to the label hrefs that satisfy it: AND across keys, OR
    /// within one. A group counts under its own `key`.
    by_key: BTreeMap<String, HashSet<String>>,
    /// Labels whose carriers are removed at the end.
    excluded: HashSet<String>,
}

impl Matcher {
    fn new(actors: &[Actor], idx: &NameIndex) -> Self {
        let mut m = Self::default();
        for a in actors {
            let excluded = a.exclusion == Some(true);
            if a.actors.as_deref() == Some("ams") {
                m.all |= !excluded;
            } else if let Some(l) = &a.label {
                if excluded {
                    m.excluded.insert(l.href.clone());
                } else {
                    let key = idx
                        .label(&l.href)
                        .map_or_else(|| format!("?{}", l.href), |x| x.key.clone());
                    m.by_key.entry(key).or_default().insert(l.href.clone());
                }
            } else if let Some(g) = &a.label_group {
                let members = group_members(&g.href, idx);
                if excluded {
                    m.excluded.extend(members);
                } else {
                    let key = idx
                        .label_group(&g.href)
                        .map(|x| x.key.clone())
                        .filter(|k| !k.is_empty())
                        .unwrap_or_else(|| format!("?{}", g.href));
                    m.by_key.entry(key).or_default().extend(members);
                }
            } else if let Some(w) = &a.workload
                && !excluded
            {
                m.workloads.insert(canon(&w.href).into_owned());
            }
            // IP lists, virtual services and virtual servers match no workload.
        }
        m
    }

    fn matches(&self, href: &str, labels: &HashSet<&str>) -> bool {
        if self.excluded.iter().any(|h| labels.contains(h.as_str())) {
            return false;
        }
        self.all
            || self.workloads.contains(canon(href).as_ref())
            || (!self.by_key.is_empty()
                && self
                    .by_key
                    .values()
                    .all(|hrefs| hrefs.iter().any(|h| labels.contains(h.as_str()))))
    }
}

/// One scope: per member the label hrefs that satisfy it (`None` for
/// `all`) and whether it is an exclusion. Every member must hold.
struct Scope(Vec<(Option<HashSet<String>>, bool)>);

impl Scope {
    fn new(members: &[Actor], idx: &NameIndex) -> Self {
        Self(
            members
                .iter()
                .map(|a| {
                    let hrefs = if a.actors.as_deref() == Some("ams") {
                        None
                    } else if let Some(l) = &a.label {
                        Some(HashSet::from([l.href.clone()]))
                    } else if let Some(g) = &a.label_group {
                        Some(group_members(&g.href, idx))
                    } else {
                        // Any other kind in a scope matches nothing.
                        Some(HashSet::new())
                    };
                    (hrefs, a.exclusion == Some(true))
                })
                .collect(),
        )
    }

    fn contains(&self, labels: &HashSet<&str>) -> bool {
        self.0.iter().all(|(hrefs, excluded)| {
            let hit = hrefs
                .as_ref()
                .is_none_or(|s| s.iter().any(|h| labels.contains(h.as_str())));
            hit != *excluded
        })
    }
}

/// Per scope `Si`: providers `{w in Si matching providers}`, consumers
/// `{w matching consumers}` kept to `Si` unless `unscoped_consumers`; the
/// totals are the unions over the scopes. A ruleset without scopes is one
/// empty scope (everywhere).
pub fn rule_impact(
    scopes: &[Vec<Actor>],
    providers: &[Actor],
    consumers: &[Actor],
    unscoped_consumers: bool,
    inputs: &ImpactInputs,
) -> Impact {
    let idx = inputs.index;
    let everywhere = [Vec::new()];
    let scopes = if scopes.is_empty() {
        &everywhere[..]
    } else {
        scopes
    };
    let p = Matcher::new(providers, idx);
    let c = Matcher::new(consumers, idx);
    let labels: Vec<HashSet<&str>> = inputs
        .workloads
        .iter()
        .map(|w| w.labels.iter().map(|l| l.href.as_str()).collect())
        .collect();
    let is_p: Vec<bool> = inputs
        .workloads
        .iter()
        .zip(&labels)
        .map(|(w, l)| p.matches(&w.href, l))
        .collect();
    let is_c: Vec<bool> = inputs
        .workloads
        .iter()
        .zip(&labels)
        .map(|(w, l)| c.matches(&w.href, l))
        .collect();
    let mut all_p = HashSet::new();
    let mut all_c = HashSet::new();
    let mut per_scope = Vec::with_capacity(scopes.len());
    for members in scopes {
        let scope = Scope::new(members, idx);
        let (mut sp, mut sc) = (0, 0);
        for (i, l) in labels.iter().enumerate() {
            let inside = scope.contains(l);
            if inside && is_p[i] {
                all_p.insert(i);
                sp += 1;
            }
            if is_c[i] && (unscoped_consumers || inside) {
                all_c.insert(i);
                sc += 1;
            }
        }
        per_scope.push(ScopeImpact {
            scope: members.iter().map(|a| describe_actor(a, idx)).collect(),
            providers: sp,
            consumers: sc,
        });
    }
    let mut ip_lists = Vec::new();
    let mut unsupported = Vec::new();
    for (side, actors) in [(Side::Provider, providers), (Side::Consumer, consumers)] {
        for a in actors {
            if let Some(r) = &a.ip_list {
                let list = idx.ip_list(&r.href);
                ip_lists.push(IpListRef {
                    name: list.map_or_else(|| r.href.clone(), |l| l.name.clone()),
                    href: r.href.clone(),
                    size: list.map(ip_list_size),
                    side,
                });
            }
            for v in [&a.virtual_service, &a.virtual_server].into_iter().flatten() {
                unsupported.push(v.href.clone());
            }
        }
    }
    Impact {
        providers: all_p.len(),
        consumers: all_c.len(),
        per_scope,
        ip_lists,
        unsupported,
        truncated: inputs.truncated,
    }
}
```

In `crates/pce/src/lib.rs`, next to the Task 1 re-export:

```rust
pub use impact::{Impact, ImpactInputs, IpListRef, RuleActors, ScopeImpact, Side, rule_impact};
```

- [ ] **Step 4: Run the tests**

Run: `cargo fmt --all && cargo test -p pce --lib impact:: && cargo test -p pce --lib resolve::`
Expected: PASS (11 `impact` tests, `label_groups_and_ip_lists_are_found_by_href_in_either_scope` and every existing `resolve` test).

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p pce
git add crates/pce/src/impact.rs crates/pce/src/resolve.rs crates/pce/src/lib.rs
git commit -m "feat(pce): rule impact over a workload list with scopes, groups and exclusions

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 3: IP lists screen (read side)

**Files:**
- Create: `crates/illumio-tui/src/tui/ui/screens/iplists.rs`
- Modify: `crates/illumio-tui/src/tui/model.rs` (imports lines 7-13; `Screen` lines 18-49; `ObjectKind` ~1136-1172; old `ip_list_columns` ~1280-1313 replaced; `ObjectsState` ~1350-1414; `IpListUse` and `PolicyState::ip_list_uses` after `find_deny_rule` ~line 1107; `IpListRow`, `IpListsState` after `ObjectsState`; `Model` field ~1849, `new_connecting` ~1871, `any_auto_enabled` ~1949, `reset_data` ~1964; one test)
- Modify: `crates/illumio-tui/src/tui/msg.rs` (`ObjectsPayload` ~line 30; `Msg` ~line 41; `Cmd` ~line 105; `PartialEq for Cmd` ~line 156)
- Modify: `crates/illumio-tui/src/tui/app.rs` (`Cmd::LoadObjects` arm ~line 312; new `Cmd::LoadIpLists` arm after it; `task_result` ~line 410)
- Modify: `crates/illumio-tui/src/tui/update.rs` (`Msg::Tick` ~lines 27-96; `Msg::IndexLoaded` ~line 121; `Msg::ObjectsLoaded` ~line 371 and a new `Msg::IpListsLoaded` arm after it; `on_screen_key` ~lines 1034-1088; `scroll_focused` ~line 1241; `switch` ~line 1298; `reload_after_write` ~line 1487; `objects_key` ~line 1806; `objects_edit_key` ~line 1895; new `ip_lists_page`, `ip_lists_key`, `ip_lists_edit_key`, `load_missing_ip_lists`, `load_ip_lists` after `load_workloads` ~line 1474; tests)
- Modify: `crates/illumio-tui/src/tui/commands.rs` (`parse` ~line 44; tests)
- Modify: `crates/illumio-tui/src/tui/ui/mod.rs` (`tab_labels`, `tab_at`, `tab_x`, `render_tabs` ~lines 104-168; `render_body` ~line 204; tests)
- Modify: `crates/illumio-tui/src/tui/ui/screens/mod.rs` (`pub mod iplists;`)
- Modify: `crates/illumio-tui/src/tui/ui/screens/objects.rs` (module doc, `render_screen` panel title ~line 49, `render_table` ~lines 84-149, `render_detail` ~line 157, `ip_list_lines` ~lines 250-286 removed, imports line 7)
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` (`HELP_LINES` ~lines 17-80, `screen_hint` ~line 660)
- Modify: `crates/illumio-tui/src/tui/test_support.rs` (`model_with_objects` ~line 95; new `model_with_ip_lists`)
- Test: `crates/illumio-tui/tests/tui_snapshots.rs` (`objects_screen_tabs_80x24` ~line 281, `status_hint_lists_the_edit_keys_only_with_writes` ~line 141; append), `crates/illumio-tui/tests/tui_runtime.rs` (`opening_objects_after_startup_costs_zero_requests` ~line 414, `update_ip_list_puts_only_the_changed_ranges` ~line 1099; append)
- Delete: `crates/illumio-tui/tests/snapshots/tui_snapshots__objects_iplists_detail_80x24.snap`

**Interfaces:**
- Consumes: `IpListSize`, `ip_list_size` (Task 1); `IpList`, `Actor`, `DenyRule`, `RuleSet` (`pce::models`); `TableView`, `Column`, `SortKey` (`tui/table.rs`); `AutoRefresh` (`model.rs` ~line 591); `PolicyState::{raw, fallback_deny, scope, loaded_at}` (~line 943); `RuleKind`, `rule_kind_name` (`model.rs` ~line 794), `describe_actor` (`pce::describe_actor`); `EditIntent::{CreateIpList, UpdateIpList(IpList), DeleteIpList(IpList)}` (`tui/edit.rs` ~lines 50-52); `writes_refused`, `begin_edit`, `drive_table`, `search_key`, `fail`, `load_workloads` (`update.rs` ~lines 1862, 1875, 1797, 1109, 619, 1469); `objects::{DetailLine, clip, items}` (`ui/screens/objects.rs` ~lines 185-207, `pub(super)`); `panel`, `header_meta`, `render`, `age_text`; `Pce::ip_lists(CacheMode)` (`pce/src/lib.rs` ~line 148); test helpers `model_fixture`, `model_with_policy`, `writes_on`, `key`, `kc`, `enter`, `objects_session`, `mount_lists`, `key_session`, `writes_model`, `tui_guard`.
- Produces:

```rust
// tui/model.rs
pub enum Screen { Traffic, Policy, Objects, Workloads, Events, IpLists }
impl Screen { pub const ALL: [Screen; 6]; pub fn short_title(self) -> &'static str; }
pub enum ObjectKind { Labels, LabelGroups, Services }            // ALL: [ObjectKind; 3]
pub struct ObjectsState { /* ip_lists removed */ pub ages: [Option<u64>; 3], pub loaded_at: [Option<Instant>; 3], pub loading: [bool; 3], .. }
#[derive(Debug, Clone)] pub struct IpListRow { pub list: IpList, pub size: IpListSize, pub used_by: Option<usize> }
pub fn ip_list_columns() -> Vec<Column<IpListRow>>;
#[derive(Debug)] pub struct IpListsState { pub table: TableView<IpListRow>, pub age_secs: u64, pub loaded_at: Option<Instant>,
                                           pub loading: bool, pub show_detail: bool, pub auto: AutoRefresh }
impl IpListsState {
    pub fn new(min_refresh: u64) -> Self;
    pub fn set_lists(&mut self, lists: Vec<IpList>, policy: &PolicyState);
    pub fn recount(&mut self, policy: &PolicyState);
}
#[derive(Debug, Clone)] pub struct IpListUse { pub ruleset: String, pub kind: RuleKind, pub providers: Vec<Actor>, pub consumers: Vec<Actor> }
impl PolicyState { pub fn ip_list_uses(&self, href: &str) -> Option<Vec<IpListUse>>; }
pub struct Model { /* … */ pub ip_lists: IpListsState }
// tui/msg.rs
Msg::IpListsLoaded(Result<(Vec<IpList>, u64), String>)
Cmd::LoadIpLists(CacheMode)
// tui/ui/mod.rs
pub fn tab_at(x: u16, width: u16) -> Option<Screen>;
pub fn tab_x(screen: Screen, width: u16) -> u16;
// tui/ui/screens/iplists.rs
pub fn layout(body: Rect, show_detail: bool) -> (Rect, Rect);
pub fn table_rows(body: Rect, show_detail: bool) -> usize;
pub fn render_screen(model: &Model, f: &mut Frame, body: Rect);
// tui/test_support.rs
pub fn model_with_ip_lists() -> Model; // model_with_policy() plus the two fixture lists, loaded 3 s ago
// tui/update.rs (private)
fn ip_lists_page(model: &Model) -> usize;
fn ip_lists_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd>;
fn ip_lists_edit_key(model: &mut Model, c: char) -> Vec<Cmd>;
fn load_missing_ip_lists(model: &mut Model) -> Vec<Cmd>;
fn load_ip_lists(model: &mut Model, mode: CacheMode) -> Vec<Cmd>;
```

- [ ] **Step 1: Write the failing tests**

Create `crates/illumio-tui/src/tui/ui/screens/iplists.rs` with only this module, and add `pub mod iplists;` to `crates/illumio-tui/src/tui/ui/screens/mod.rs` (after `pub mod events;`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::{model_fixture, model_with_ip_lists};

    fn text(lines: &[DetailLine]) -> Vec<String> {
        lines
            .iter()
            .map(|(h, b)| format!("{:<8}{b}", h.unwrap_or("")))
            .collect()
    }

    #[test]
    fn the_detail_names_size_ranges_fqdns_and_the_rules_using_the_list() {
        let m = model_with_ip_lists();
        let corp = &m.ip_lists.table.rows()[1];
        assert_eq!(
            text(&detail_lines(&m, corp)),
            vec![
                "list    Corp  Office ranges",
                "size    65.5K addresses (65536 IPv4, 0 IPv6)",
                "ranges  10.1.0.0-10.1.255.255",
                "fqdns   *.corp.example",
                "used by Shop-Prod · allow · role=web ← ipl:Corp",
            ]
        );
        let any = text(&detail_lines(&m, &m.ip_lists.table.rows()[0]));
        assert_eq!(any[1], "size    any (0.0.0.0/0)");
        assert_eq!(any[2], "ranges  0.0.0.0/0");
        assert_eq!(any[3], "        ::/0");
        assert_eq!(
            any[4],
            "used by Shop-Prod · deny · All workloads ← ipl:Any (0.0.0.0/0 and ::/0)"
        );
    }

    #[test]
    fn without_loaded_rulesets_the_detail_says_where_to_load_them() {
        let mut m = model_fixture();
        let lists = serde_json::from_str(include_str!(
            "../../../../../../fixtures/ip_lists.json"
        ))
        .unwrap();
        m.ip_lists.set_lists(lists, &m.policy);
        let lines = text(&detail_lines(&m, &m.ip_lists.table.rows()[1]));
        assert_eq!(lines.last().unwrap(), "used by load Policy (2) to see rules");
        let excluded: pce::models::IpList = serde_json::from_value(serde_json::json!({
            "href": "/orgs/5/sec_policy/draft/ip_lists/30", "name": "Lab",
            "ip_ranges": [
                {"from_ip": "10.0.0.0/24", "description": "lab net"},
                {"from_ip": "10.0.0.128/25", "exclusion": true}
            ]
        }))
        .unwrap();
        m.ip_lists.set_lists(vec![excluded], &m.policy);
        let lines = text(&detail_lines(&m, &m.ip_lists.table.rows()[0]));
        assert_eq!(lines[1], "size    128 addresses (128 IPv4, 0 IPv6)");
        assert_eq!(lines[2], "ranges  10.0.0.0/24  lab net");
        assert_eq!(lines[3], "        !10.0.0.128/25");
    }
}
```

Append to `mod tests` in `crates/illumio-tui/src/tui/model.rs`:

```rust
    #[test]
    fn ip_list_rows_carry_their_size_and_how_many_loaded_rules_use_them() {
        let lists: Vec<IpList> =
            serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json")).unwrap();
        let mut s = IpListsState::new(60);
        let mut p = PolicyState::new(60);
        s.set_lists(lists, &p);
        assert_eq!(s.table.rows()[1].size.display(), "65.5K");
        assert_eq!(s.table.rows()[1].used_by, None, "no rulesets loaded");
        p.raw = serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
        p.loaded_at = Some(Instant::now());
        s.recount(&p);
        let used: Vec<Option<usize>> = s.table.rows().iter().map(|r| r.used_by).collect();
        assert_eq!(used, vec![Some(1), Some(1)], "deny rule 7 and allow rule 2");
        let uses = p.ip_list_uses("/orgs/5/sec_policy/draft/ip_lists/21").unwrap();
        assert_eq!(uses[0].ruleset, "Shop-Prod");
        assert_eq!(uses[0].kind, RuleKind::Allow);
        p.scope = PolicyScope::Active;
        s.recount(&p);
        assert_eq!(s.table.rows()[0].used_by, None, "only draft rulesets count");
        let cols = ip_list_columns();
        let titles: Vec<&str> = cols.iter().map(|c| c.title).collect();
        assert_eq!(titles, vec!["name", "size", "ranges", "fqdns", "used by", "pending"]);
        let rows = s.table.rows();
        assert_eq!((cols[1].cell)(&rows[0]), "any");
        assert_eq!((cols[1].cell)(&rows[1]), "65.5K");
        assert_eq!((cols[2].cell)(&rows[0]), "2");
        assert_eq!((cols[3].cell)(&rows[1]), "1");
        assert_eq!((cols[4].cell)(&rows[0]), "-");
        assert!((cols[1].key)(&rows[0]) > (cols[1].key)(&rows[1]), "any sorts last");
        assert_eq!(Screen::ALL.len(), 6);
        assert_eq!(Screen::IpLists.title(), "IP lists");
        assert_eq!(Screen::IpLists.short_title(), "IPs");
        assert_eq!(Screen::Objects.title(), "Objects");
        assert_eq!(ObjectKind::ALL.len(), 3);
        assert_eq!(ObjectKind::Labels.prev(), ObjectKind::Services);
        assert_eq!(ObjectKind::Services.next(), ObjectKind::Labels);
    }
```

Append to `mod tests` in `crates/illumio-tui/src/tui/commands.rs`, inside `parses_every_documented_command` after the `events` lines:

```rust
        assert_eq!(parse("iplists").unwrap(), Command::Screen(Screen::IpLists));
        assert_eq!(parse("ip").unwrap(), Command::Screen(Screen::IpLists));
```

In `crates/illumio-tui/src/tui/ui/mod.rs`, replace `tab_at_matches_the_rendered_row` and add one test:

```rust
    #[test]
    fn tab_at_matches_the_rendered_row() {
        for s in Screen::ALL {
            assert_eq!(tab_at(tab_x(s, 80), 80), Some(s));
        }
        let first_len = tab_width(&tab_labels(80)[0]);
        assert_eq!(tab_at(first_len, 80), None, "divider");
        assert_eq!(tab_at(79, 80), None, "past the last tab");
        let x = tab_x(Screen::Workloads, 80);
        assert_eq!(tab_at(x, x), None, "clipped by a narrow terminal");
        // The rendered row puts each label exactly where tab_at expects it,
        // with the short titles at 80 columns and the full ones at 160.
        let m = crate::tui::test_support::model_fixture();
        for (w, h) in [(80u16, 24u16), (160, 48)] {
            let out = crate::tui::test_support::render_to_string(&m, w, h);
            let row: Vec<char> = out.lines().nth(1).unwrap().chars().collect();
            for s in Screen::ALL {
                let x = tab_x(s, w) as usize;
                assert!(row[x + 1].is_ascii_digit(), "the label's number follows x");
                assert_eq!(row[x - 1], ' ', "padding cell before the label");
                assert_eq!(tab_at(x as u16 + 1, w), Some(s));
            }
        }
    }

    #[test]
    fn the_tab_row_shortens_only_when_the_full_row_does_not_fit() {
        let full = tab_labels(160);
        assert_eq!(row_width(&full), 84, "six full titles and five dividers");
        assert_eq!(full[2], " 3 Objects ");
        assert_eq!(full[5], " 6 IP lists ");
        assert_eq!(tab_labels(84)[5], " 6 IP lists ");
        assert_eq!(tab_labels(83)[5], " 6 IPs ");
        assert_eq!(tab_labels(83)[0], " 1 Traf ");
        assert_eq!(row_width(&tab_labels(80)), 58);
    }
```

In `crates/illumio-tui/src/tui/update.rs` tests:

Replace `objects_fixture_msgs` with:

```rust
    fn objects_fixture_msgs() -> Vec<Msg> {
        use crate::tui::model::ObjectKind;
        use crate::tui::msg::ObjectsPayload;
        let labels =
            serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap();
        let groups =
            serde_json::from_str(include_str!("../../../../fixtures/label_groups.json")).unwrap();
        let services =
            serde_json::from_str(include_str!("../../../../fixtures/services.json")).unwrap();
        vec![
            Msg::ObjectsLoaded {
                kind: ObjectKind::Labels,
                result: Ok(ObjectsPayload::Labels(labels, 120)),
            },
            Msg::ObjectsLoaded {
                kind: ObjectKind::LabelGroups,
                result: Ok(ObjectsPayload::LabelGroups(groups, 120)),
            },
            Msg::ObjectsLoaded {
                kind: ObjectKind::Services,
                result: Ok(ObjectsPayload::Services(services, 120)),
            },
        ]
    }
```

Rename `opening_objects_loads_all_four_once_and_tab_cycles` to `opening_objects_loads_all_three_once_and_tab_cycles` and change its `assert_eq!(cmds.len(), 4);` to `assert_eq!(cmds.len(), 3);`.

In `objects_load_only_what_is_missing_and_never_before_the_index` change `objects_fixture_msgs().into_iter().take(3)` to `.take(2)` and `assert!(m.objects.ages[3].is_none());` to `assert!(m.objects.ages[2].is_none());`.

In `a_late_result_for_another_kind_still_fills_its_table` replace the lines from `update(&mut m, msgs.into_iter().nth(2).unwrap()); // IpLists arrives` through `assert!(!m.objects.loading[2]);` with:

```rust
        update(&mut m, msgs.into_iter().nth(2).unwrap()); // Services arrives
        assert_eq!(m.objects.kind, ObjectKind::LabelGroups);
        assert_eq!(m.objects.services.len(), 3);
        assert_eq!(m.objects.ages[2], Some(120));
        assert!(!m.objects.loading[2]);
```

In `number_keys_switch_screens_policy_objects_and_workloads_load` replace the Objects assertion and add `6` before the final `key('1')`:

```rust
        // entering Objects for the first time loads all three kinds (cache hits)
        assert_eq!(update(&mut m, key('3')).len(), 3);
```

```rust
        // entering IP lists for the first time loads them once (a cache hit)
        assert_eq!(
            update(&mut m, key('6')),
            vec![Cmd::LoadIpLists(pce::CacheMode::Use)]
        );
        assert_eq!(m.screen, Screen::IpLists);
```

In `mouse_wheel_scrolls_and_tab_click_switches` and `mouse_ignores_everything_else_and_overlays` change every `crate::tui::ui::tab_x(Screen::Policy)` to `crate::tui::ui::tab_x(Screen::Policy, 80)` (three places).

In `e_opens_a_prefilled_form_and_enter_plans_only_the_change` replace the first six lines of the body:

```rust
        use crate::tui::edit::WriteIntent;
        use crate::tui::model::ObjectKind;
        use crate::tui::test_support::model_with_objects;
        let mut m = writes_on(model_with_objects());
        m.screen = Screen::Objects;
        m.objects.kind = ObjectKind::IpLists;
```

with:

```rust
        use crate::tui::edit::WriteIntent;
        use crate::tui::test_support::model_with_ip_lists;
        let mut m = writes_on(model_with_ip_lists());
        m.screen = Screen::IpLists;
```

In `write_done_reports_reloads_the_kind_once_and_forgets_pending` and `an_unanswered_write_whose_audit_failed_still_reloads_and_names_the_audit_log`, replace

```rust
            vec![Cmd::LoadObjects {
                kind: ObjectKind::IpLists,
                mode: pce::CacheMode::Bypass
            }]
```

with `vec![Cmd::LoadIpLists(pce::CacheMode::Bypass)]`, and delete the now unused `use crate::tui::model::ObjectKind;` from the second test (the first still uses `ObjectKind::Labels`).

In `help_scrolls_to_its_last_line_and_reopens_at_the_top` replace the two count assertions:

```rust
        assert_eq!(HELP_LINES.len(), 30);
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "IP lists Enter"));
        update(&mut m, key('?'));
        let last = help_last_scroll(24);
        assert_eq!(last, 8, "30 lines, 22 rows inside the box");
```

Append to `mod tests` in `update.rs`:

```rust
    #[test]
    fn six_opens_ip_lists_once_and_r_bypasses_the_cache_once() {
        let mut m = model_fixture();
        assert_eq!(
            update(&mut m, key('6')),
            vec![Cmd::LoadIpLists(pce::CacheMode::Use)]
        );
        assert_eq!(m.screen, Screen::IpLists);
        assert!(update(&mut m, key('6')).is_empty(), "already loading");
        let lists =
            serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json")).unwrap();
        assert!(update(&mut m, Msg::IpListsLoaded(Ok((lists, 120)))).is_empty());
        assert_eq!(m.ip_lists.table.len(), 2);
        assert_eq!(m.ip_lists.table.rows()[1].size.display(), "65.5K");
        assert_eq!(m.ip_lists.age_secs, 120);
        assert_eq!(m.status.text, "ready", "the status line is left alone");
        update(&mut m, key('1'));
        assert!(update(&mut m, key('6')).is_empty(), "coming back costs nothing");
        assert_eq!(
            update(&mut m, key('r')),
            vec![Cmd::LoadIpLists(pce::CacheMode::Bypass)]
        );
        assert!(update(&mut m, key('r')).is_empty());
        assert_eq!(m.status.text, "IP lists already loading");
        update(&mut m, Msg::IpListsLoaded(Err("boom".into())));
        assert_eq!(m.status.text, "IP lists: boom");
        assert!(!m.ip_lists.loading);
        assert_eq!(m.ip_lists.table.len(), 2, "a failed reload keeps the rows");
    }

    #[test]
    fn ip_lists_wait_for_the_name_index_and_the_command_opens_them() {
        let mut m = model_fixture();
        let Connection::Connected { index, .. } = &mut m.connection else {
            unreachable!()
        };
        let idx = index.take();
        assert!(
            update(&mut m, key('6')).is_empty(),
            "the startup load reads the same list"
        );
        let cmds = update(
            &mut m,
            Msg::IndexLoaded {
                index: idx.unwrap(),
                age_secs: 0,
            },
        );
        assert_eq!(cmds, vec![Cmd::LoadIpLists(pce::CacheMode::Use)]);
        let mut m = model_fixture();
        assert_eq!(
            on_command(&mut m, Command::Screen(Screen::IpLists)),
            vec![Cmd::LoadIpLists(pce::CacheMode::Use)]
        );
    }

    #[test]
    fn enter_toggles_the_ip_list_detail_and_search_captures_keys() {
        use crate::tui::test_support::model_with_ip_lists;
        let mut m = model_with_ip_lists();
        m.screen = Screen::IpLists;
        update(&mut m, enter());
        assert!(m.ip_lists.show_detail);
        update(&mut m, enter());
        assert!(!m.ip_lists.show_detail);
        update(&mut m, key('/'));
        for c in "corp6".chars() {
            assert!(update(&mut m, key(c)).is_empty());
        }
        assert_eq!(m.screen, Screen::IpLists, "6 is typed into the search");
        assert!(m.ip_lists.table.is_empty());
        update(&mut m, kc(KeyCode::Esc));
        assert_eq!(m.ip_lists.table.len(), 2);
        update(&mut m, key('j'));
        assert_eq!(
            m.ip_lists.table.selected_item().map(|r| r.list.name.as_str()),
            Some("Corp")
        );
    }

    #[test]
    fn ip_lists_auto_refresh_is_opt_in_and_bypasses_when_due() {
        use crate::tui::test_support::model_with_ip_lists;
        let mut m = model_with_ip_lists();
        m.screen = Screen::IpLists;
        let later = std::time::Instant::now() + std::time::Duration::from_secs(61);
        assert!(update(&mut m, Msg::Tick(later)).is_empty(), "off by default");
        update(&mut m, key('a'));
        assert!(m.ip_lists.auto.enabled);
        assert!(m.any_auto_enabled());
        assert_eq!(
            update(&mut m, Msg::Tick(later)),
            vec![Cmd::LoadIpLists(pce::CacheMode::Bypass)]
        );
        assert!(m.ip_lists.auto.from_tick);
    }
```

In `crates/illumio-tui/tests/tui_snapshots.rs`, in `objects_screen_tabs_80x24` replace the lines from `m.objects.kind = ObjectKind::IpLists;` through `insta::assert_snapshot!("objects_iplists_detail_80x24", out);` with:

```rust
    m.objects.show_detail = true;
```

(the Services snapshot below keeps its detail pane, as before). In `status_hint_lists_the_edit_keys_only_with_writes`, before `m.screen = Screen::Traffic;` add:

```rust
    m.screen = Screen::IpLists;
    assert!(last(&m).contains("n:new e:edit x:del"), "{}", last(&m));
```

Append:

```rust
#[test]
fn ip_lists_screen_80x24_and_160x48_with_detail() {
    use illumio_tui::tui::test_support::model_with_ip_lists;
    let mut m = model_with_ip_lists();
    m.screen = Screen::IpLists;
    m.ip_lists.table.select_next(); // Corp
    m.ip_lists.show_detail = true;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("6 IPs"), "{out}");
    assert!(out.contains("65.5K"), "{out}");
    assert!(out.contains("used by Shop-Prod · allow"), "{out}");
    insta::assert_snapshot!("ip_lists_80x24", out);
    let out = render_to_string(&m, 160, 48);
    assert!(out.contains("6 IP lists"), "{out}");
    assert!(out.contains("65536 IPv4"), "{out}");
    insta::assert_snapshot!("ip_lists_160x48", out);
    m.set_theme("light");
    insta::assert_snapshot!("ip_lists_light_styled_80x24", render_styled(&m, 80, 24));
}

#[test]
fn the_tab_row_uses_short_titles_only_when_the_full_row_does_not_fit() {
    let m = model_fixture();
    let narrow = render_to_string(&m, 80, 24);
    let row = narrow.lines().nth(1).unwrap();
    assert!(
        row.contains("1 Traf") && row.contains("3 Obj") && row.contains("6 IPs"),
        "{row}"
    );
    let wide = render_to_string(&m, 160, 48);
    let row = wide.lines().nth(1).unwrap();
    assert!(
        row.contains("3 Objects") && row.contains("6 IP lists"),
        "{row}"
    );
}
```

In `crates/illumio-tui/tests/tui_runtime.rs`, in `opening_objects_after_startup_costs_zero_requests` delete the line `assert_eq!(model.objects.ip_lists.len(), 2);`. In `update_ip_list_puts_only_the_changed_ranges` replace the first four key entries:

```rust
        (500, ch('3')),
        (50, KeyCode::Tab),
        (50, KeyCode::Tab),
        (50, ch('j')),
```

with:

```rust
        (500, ch('6')),
        (200, ch('j')),
```

(the IP lists arrive from the cache after `6`; the assertions stay: plan GET, PUT and one IP lists reload, 8 calls). Append:

```rust
#[tokio::test]
async fn opening_ip_lists_costs_nothing_after_startup_and_r_bypasses_once() {
    let server = MockServer::start().await;
    mount_lists(&server, 1, 2, 1).await; // IP lists: the startup load and the r
    let model = objects_session(
        &server,
        &[(500, '6'), (300, '1'), (100, '6'), (300, 'r'), (400, 'q')],
    )
    .await;
    assert_eq!(model.stats.calls, 6, "health + four lists + the r refresh");
    assert_eq!(model.ip_lists.table.len(), 2);
    assert!(!model.ip_lists.loading);
    assert_eq!(model.busy, 0);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --lib`
Expected: FAIL to compile (`no variant IpLists` on `Screen`, `no field ip_lists on Model`, `cannot find function detail_lines`, `tab_x` takes 2 arguments, `no variant LoadIpLists`, …).

- [ ] **Step 3: Implement**

`crates/illumio-tui/src/tui/model.rs`. Imports become:

```rust
use pce::models::{
    Actor, DenyRule, Event, EventFilter, IpList, Label, LabelGroup, PendingChange, PolicyScope,
    Rule, RuleSet, Service, VenHealth, Workload,
};
use pce::traffic::{BlockedGroup, FlowGroup, ResolvedFlow, blocked_summary, group_by_labels};
use pce::write::Plan;
use pce::{IpListSize, NameIndex, RuleKind, RuleSetView, RuleView, StatsSnapshot, ip_list_size};
```

`Screen` becomes:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    Traffic,
    Policy,
    Objects,
    Workloads,
    Events,
    IpLists,
}

impl Screen {
    /// In tab order; a screen's key is its position plus one.
    pub const ALL: [Screen; 6] = [
        Screen::Traffic,
        Screen::Policy,
        Screen::Objects,
        Screen::Workloads,
        Screen::Events,
        Screen::IpLists,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Screen::Traffic => "Traffic",
            Screen::Policy => "Policy",
            Screen::Objects => "Objects",
            Screen::Workloads => "Workloads",
            Screen::Events => "Events",
            Screen::IpLists => "IP lists",
        }
    }

    /// The tab row's label when the full titles do not fit.
    pub fn short_title(self) -> &'static str {
        match self {
            Screen::Traffic => "Traf",
            Screen::Policy => "Pol",
            Screen::Objects => "Obj",
            Screen::Workloads => "Wl",
            Screen::Events => "Ev",
            Screen::IpLists => "IPs",
        }
    }

    pub fn index(self) -> usize {
        Screen::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }
}
```

`ObjectKind` becomes:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Labels,
    LabelGroups,
    Services,
}

impl ObjectKind {
    pub const ALL: [ObjectKind; 3] = [
        ObjectKind::Labels,
        ObjectKind::LabelGroups,
        ObjectKind::Services,
    ];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|k| *k == self).unwrap_or(0)
    }

    pub fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    pub fn title(self) -> &'static str {
        match self {
            ObjectKind::Labels => "Labels",
            ObjectKind::LabelGroups => "Label groups",
            ObjectKind::Services => "Services",
        }
    }
}
```

Delete the old `ip_list_columns()` (the `Vec<Column<IpList>>` one). In `ObjectsState` delete the `ip_lists` field and its initialiser and the two `ObjectKind::IpLists` arms (`is_searching`, `has_selection`); the arrays become three long:

```rust
    /// Cache age in seconds when each kind (indexed by `ObjectKind::index`) was loaded.
    pub ages: [Option<u64>; 3],
    /// When each kind arrived, so the shown age keeps growing.
    pub loaded_at: [Option<Instant>; 3],
    pub loading: [bool; 3],
```

```rust
            ages: [None; 3],
            loaded_at: [None; 3],
            loading: [false; 3],
```

After `PolicyState::find_deny_rule` add:

```rust
    /// The rules of the loaded draft rulesets (allow, deny, fetched deny)
    /// that name the IP list `href` on either side; `None` until draft
    /// rulesets are loaded. Costs no request.
    pub fn ip_list_uses(&self, href: &str) -> Option<Vec<IpListUse>> {
        if self.scope != PolicyScope::Draft || self.loaded_at.is_none() {
            return None;
        }
        let names = |v: &[Actor]| {
            v.iter()
                .any(|a| a.ip_list.as_ref().is_some_and(|r| r.href == href))
        };
        let mut out = Vec::new();
        for rs in &self.raw {
            for r in &rs.rules {
                if names(&r.providers) || names(&r.consumers) {
                    out.push(IpListUse {
                        ruleset: rs.name.clone(),
                        kind: RuleKind::Allow,
                        providers: r.providers.clone(),
                        consumers: r.consumers.clone(),
                    });
                }
            }
            let deny: &[DenyRule] = if rs.deny_rules.is_empty() {
                self.fallback_deny.get(&rs.href).map_or(&[], Vec::as_slice)
            } else {
                &rs.deny_rules
            };
            for d in deny {
                if names(&d.providers) || names(&d.consumers) {
                    out.push(IpListUse {
                        ruleset: rs.name.clone(),
                        kind: if d.override_ {
                            RuleKind::OverrideDeny
                        } else {
                            RuleKind::Deny
                        },
                        providers: d.providers.clone(),
                        consumers: d.consumers.clone(),
                    });
                }
            }
        }
        Some(out)
    }
```

and, before `impl PolicyState`:

```rust
/// One loaded rule that names an IP list, for the IP lists screen.
#[derive(Debug, Clone)]
pub struct IpListUse {
    pub ruleset: String,
    pub kind: RuleKind,
    pub providers: Vec<Actor>,
    pub consumers: Vec<Actor>,
}
```

After `impl ObjectsState { … }` add:

```rust
/// An IP list with what its screen derives from it once per load.
#[derive(Debug, Clone)]
pub struct IpListRow {
    pub list: IpList,
    pub size: IpListSize,
    /// Rules of the loaded draft rulesets that name the list; `None` until
    /// the Policy screen loaded them.
    pub used_by: Option<usize>,
}

pub fn ip_list_columns() -> Vec<Column<IpListRow>> {
    vec![
        Column {
            title: "name",
            width: 28,
            cell: |r| r.list.name.clone(),
            key: |r| SortKey::Text(r.list.name.clone()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 24,
        },
        Column {
            title: "size",
            width: 10,
            // `any (0.0.0.0/0)` does not fit; the detail shows the reason.
            cell: |r| {
                if r.size.unbounded.is_some() {
                    "any".to_string()
                } else {
                    r.size.display()
                }
            },
            key: |r| SortKey::Num(r.size.sort_key()),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "ranges",
            width: 6,
            cell: |r| r.list.ip_ranges.len().to_string(),
            key: |r| SortKey::Num(r.list.ip_ranges.len() as u64),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "fqdns",
            width: 5,
            cell: |r| r.list.fqdns.len().to_string(),
            key: |r| SortKey::Num(r.list.fqdns.len() as u64),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "used by",
            width: 7,
            cell: |r| r.used_by.map_or_else(|| "-".to_string(), |n| n.to_string()),
            key: |r| SortKey::Num(r.used_by.map_or(0, |n| n as u64 + 1)),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "pending",
            width: 7,
            cell: |r| r.list.update_type.clone().unwrap_or_default(),
            key: |r| SortKey::Text(r.list.update_type.clone().unwrap_or_default()),
            right: false,
            hide_below: 70,
            narrow: None,
            narrow_width: 0,
        },
    ]
}

#[derive(Debug)]
pub struct IpListsState {
    pub table: TableView<IpListRow>,
    /// Cache age in seconds when the lists arrived.
    pub age_secs: u64,
    pub loaded_at: Option<Instant>,
    pub loading: bool,
    pub show_detail: bool,
    pub auto: AutoRefresh,
}

impl IpListsState {
    pub fn new(min_refresh: u64) -> Self {
        Self {
            table: TableView::new(ip_list_columns()),
            age_secs: 0,
            loaded_at: None,
            loading: false,
            show_detail: false,
            auto: AutoRefresh::new(min_refresh),
        }
    }

    /// The rows for `lists`: sizes computed, `used_by` from the loaded rulesets.
    pub fn set_lists(&mut self, lists: Vec<IpList>, policy: &PolicyState) {
        self.table.set_rows(
            lists
                .into_iter()
                .map(|list| IpListRow {
                    size: ip_list_size(&list),
                    used_by: policy.ip_list_uses(&list.href).map(|u| u.len()),
                    list,
                })
                .collect(),
        );
    }

    /// `used_by` again from `policy` (the rulesets changed); no request.
    pub fn recount(&mut self, policy: &PolicyState) {
        let rows = self
            .table
            .rows()
            .iter()
            .cloned()
            .map(|mut r| {
                r.used_by = policy.ip_list_uses(&r.list.href).map(|u| u.len());
                r
            })
            .collect();
        self.table.set_rows(rows);
    }
}
```

In `Model` add the field after `pub objects: ObjectsState,`:

```rust
    pub ip_lists: IpListsState,
```

In `new_connecting` add `let ip_lists = IpListsState::new(auto_refresh_min_seconds);` next to the other states and `ip_lists,` after `objects,` in the struct literal; in `any_auto_enabled` add `|| self.ip_lists.auto.enabled`; in `reset_data` add `self.ip_lists = IpListsState::new(min);` after the objects line.

`crates/illumio-tui/src/tui/msg.rs`: delete `IpLists(Vec<IpList>, u64),` from `ObjectsPayload`; in `Msg` after `ObjectsLoaded { … },` add:

```rust
    /// The draft IP lists with the cache age in seconds.
    IpListsLoaded(Result<(Vec<IpList>, u64), String>),
```

in `Cmd` after `LoadObjects { … },` add:

```rust
    /// The draft IP lists for the IP lists screen (the index's own list, cached).
    LoadIpLists(CacheMode),
```

and in `impl PartialEq for Cmd` after the `LoadObjects` arm:

```rust
            (Cmd::LoadIpLists(a), Cmd::LoadIpLists(b)) => a == b,
```

`crates/illumio-tui/src/tui/app.rs`: delete the `ObjectKind::IpLists => pce.ip_lists(mode)…` arm of `Cmd::LoadObjects`; after that arm add:

```rust
            Cmd::LoadIpLists(mode) => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                tokio::spawn(async move {
                    let result = pce
                        .ip_lists(mode)
                        .await
                        .map(|c| (c.value, c.age.as_secs()))
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Msg::IpListsLoaded(result));
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
```

and add `| Msg::IpListsLoaded(_)` after `| Msg::ObjectsLoaded { .. }` in `task_result`.

`crates/illumio-tui/src/tui/update.rs`:

In `Msg::Tick`, before the Workloads block:

```rust
            if model.screen == Screen::IpLists
                && model.focused
                && model.overlay == Overlay::None
                && model.ip_lists.loaded_at.is_some()
                && !model.ip_lists.loading
                && model.ip_lists.auto.due(now)
            {
                let cmds = load_ip_lists(model, CacheMode::Bypass);
                model.ip_lists.auto.from_tick = true;
                return cmds;
            }
```

In `Msg::IndexLoaded`, after the Workloads line:

```rust
            if model.screen == Screen::IpLists {
                cmds.extend(load_missing_ip_lists(model));
            }
```

In `Msg::ObjectsLoaded` delete the `ObjectsPayload::IpLists(v, a) => { … }` arm. After the `Msg::ObjectsLoaded` arm add:

```rust
        Msg::IpListsLoaded(result) => {
            model.ip_lists.loading = false;
            match result {
                // No status text: a write's report survives the reload it caused.
                Ok((lists, age)) => {
                    model.ip_lists.set_lists(lists, &model.policy);
                    let s = &mut model.ip_lists;
                    s.loaded_at = Some(Instant::now());
                    s.auto.from_tick = false;
                    s.age_secs = age;
                    s.show_detail &= s.table.selected_item().is_some();
                }
                Err(e) => {
                    let tripped = model.ip_lists.auto.take_tick_failure();
                    fail(model, "IP lists", &e, tripped);
                }
            }
            Vec::new()
        }
```

In `on_screen_key`, after the Events search capture:

```rust
        if model.screen == Screen::IpLists && model.ip_lists.table.is_searching() {
            return ip_lists_key(model, k);
        }
```

after `(KeyCode::Char('5'), _) => switch(model, Screen::Events),`:

```rust
        (KeyCode::Char('6'), _) => switch(model, Screen::IpLists),
```

and after `_ if model.screen == Screen::Events => events_key(model, k),`:

```rust
        _ if model.screen == Screen::IpLists => ip_lists_key(model, k),
```

In `scroll_focused` delete `ObjectKind::IpLists => step(&mut o.ip_lists, down, page),` and add after the `Screen::Events` arm:

```rust
        Screen::IpLists => {
            let page = ip_lists_page(model);
            step(&mut model.ip_lists.table, down, page);
        }
```

In `switch` add `Screen::IpLists => load_missing_ip_lists(model),` after the Events arm. In `reload_after_write` the IP list arm becomes:

```rust
        pce::write::ObjectKind::IpList => load_ip_lists(model, CacheMode::Bypass),
```

In `objects_key` delete `ObjectKind::IpLists => drive_table(&mut o.ip_lists, k, page),`. In `objects_edit_key` delete the three `ObjectKind::IpLists` arms and change its doc comment to `/// \`n\` / \`e\` / \`x\` on Objects: labels and services; label groups are read-only.`

After `load_workloads` add:

```rust
fn ip_lists_page(model: &Model) -> usize {
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    crate::tui::ui::layout::chrome(area)
        .map(|c| crate::tui::ui::screens::iplists::table_rows(c.body, model.ip_lists.show_detail))
        .unwrap_or(10)
        .max(1)
}

/// Loads the IP lists once. Waits for the name index like Objects does: the
/// index reads the same list, so after it the load is a cache hit.
fn load_missing_ip_lists(model: &mut Model) -> Vec<Cmd> {
    if model.index().is_none() || model.ip_lists.loaded_at.is_some() || model.ip_lists.loading {
        return Vec::new();
    }
    load_ip_lists(model, CacheMode::Use)
}

fn load_ip_lists(model: &mut Model, mode: CacheMode) -> Vec<Cmd> {
    model.ip_lists.loading = true;
    model.ip_lists.auto.last_run = Some(Instant::now());
    model.ip_lists.auto.from_tick = false;
    vec![Cmd::LoadIpLists(mode)]
}

fn ip_lists_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let page = ip_lists_page(model);
    if !model.ip_lists.table.is_searching() {
        match k.code {
            KeyCode::Char('r') => {
                if model.index().is_none() {
                    if model.objects.index_failed {
                        model.objects.index_failed = false;
                        model.workloads.index_failed = false;
                        model.status = StatusLine::info("loading name index…");
                        return vec![Cmd::LoadIndex(CacheMode::Use)];
                    }
                    model.status = StatusLine::info("objects are still loading");
                    return Vec::new();
                }
                if model.ip_lists.loading {
                    model.status = StatusLine::info("IP lists already loading");
                    return Vec::new();
                }
                model.ip_lists.auto.rearm();
                return load_ip_lists(model, CacheMode::Bypass);
            }
            KeyCode::Enter => {
                let s = &mut model.ip_lists;
                s.show_detail = !s.show_detail && s.table.selected_item().is_some();
                return Vec::new();
            }
            KeyCode::Char('a') => {
                model.status = model.ip_lists.auto.toggle();
                return Vec::new();
            }
            KeyCode::Char(c @ ('n' | 'e' | 'x')) => return ip_lists_edit_key(model, c),
            _ => {}
        }
    }
    drive_table(&mut model.ip_lists.table, k, page);
    Vec::new()
}

/// `n` / `e` / `x` on IP lists, through the Plan 4 intents.
fn ip_lists_edit_key(model: &mut Model, c: char) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    let selected = model.ip_lists.table.selected_item().map(|r| r.list.clone());
    let intent = match c {
        'n' => Some(EditIntent::CreateIpList),
        'e' => selected.map(EditIntent::UpdateIpList),
        _ => selected.map(EditIntent::DeleteIpList),
    };
    let Some(intent) = intent else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
    begin_edit(model, intent)
}
```

`crates/illumio-tui/src/tui/commands.rs`: after the `"events" | "e"` arm add:

```rust
        "iplists" | "ip" => no_arg(Command::Screen(Screen::IpLists)),
```

`crates/illumio-tui/src/tui/ui/mod.rs`: replace `tab_labels`, `tab_at`, `tab_x` and the first line of `render_tabs`:

```rust
/// Tab labels for a row `width` cells wide: the full titles when the whole
/// row fits, else the short ones. `Tabs` draws each between `TAB_PAD` cells.
fn tab_labels(width: u16) -> Vec<String> {
    let labels = |title: fn(Screen) -> &'static str| -> Vec<String> {
        Screen::ALL
            .iter()
            .enumerate()
            .map(|(i, s)| format!(" {} {} ", i + 1, title(*s)))
            .collect()
    };
    let full = labels(Screen::title);
    if row_width(&full) <= width {
        full
    } else {
        labels(Screen::short_title)
    }
}

/// Cells the whole tab row takes: every tab and the dividers between them.
fn row_width(labels: &[String]) -> u16 {
    let divider = TAB_DIVIDER.chars().count() as u16;
    let tabs: u16 = labels.iter().map(|l| tab_width(l)).sum();
    tabs + divider * (labels.len() as u16).saturating_sub(1)
}
```

```rust
/// Which tab a click at column `x` on the tabs row hits, for a row `width` cells wide.
/// Dividers, the space past the last tab and clipped cells hit nothing.
pub fn tab_at(x: u16, width: u16) -> Option<Screen> {
    if x >= width {
        return None;
    }
    let divider = TAB_DIVIDER.chars().count() as u16;
    let mut start = 0u16;
    for (label, screen) in tab_labels(width).iter().zip(Screen::ALL) {
        let w = tab_width(label);
        if x >= start && x < start + w {
            return Some(screen);
        }
        start += w + divider;
    }
    None
}

/// A column inside the given tab's label on a row `width` cells wide (test helper for `tab_at`).
pub fn tab_x(screen: Screen, width: u16) -> u16 {
    let divider = TAB_DIVIDER.chars().count() as u16;
    let mut start = 0u16;
    for (label, s) in tab_labels(width).iter().zip(Screen::ALL) {
        if s == screen {
            return start + TAB_PAD.chars().count() as u16;
        }
        start += tab_width(label) + divider;
    }
    0
}
```

and in `render_tabs`: `let titles: Vec<Line> = tab_labels(area.width).into_iter().map(Line::from).collect();`. In `render_body`, after the Events block:

```rust
    if model.screen == Screen::IpLists && model.is_connected() {
        screens::iplists::render_screen(model, f, area);
        return;
    }
```

`crates/illumio-tui/src/tui/ui/screens/objects.rs`: module doc becomes `//! Objects screen: a tab strip of the three object kinds (labels, label groups, services), the current kind's table, and an optional detail pane for the selected object.`; the not-loaded panel title `"Labels & objects"` becomes `"Objects"`; delete the `ObjectKind::IpLists` arms in `render_table` (both matches) and `render_detail`, delete `fn ip_list_lines`, and drop `IpList` from `use pce::models::{IpList, Label, LabelGroup, Service};`.

`crates/illumio-tui/src/tui/ui/widgets/overlay.rs`: in `HELP_LINES` the first entry and the second `:` line become

```rust
    (
        "1 2 3 4 5 6",
        "Traffic · Policy · Objects · Workloads · Events · IPs",
    ),
    (":", "command prompt: :traffic :policy :objects :workloads"),
    ("", ":events  :iplists  :filter <expr>  :window <dur>"),
```

and after `("Objects  Tab / Enter", "next object type · detail"),` add

```rust
    (
        "IP lists Enter",
        "detail: ranges · FQDNs · size · rules using it",
    ),
```

In `screen_hint` add before the Events arm:

```rust
        (Screen::IpLists, true) => "n:new e:edit x:del  Enter detail  r refresh",
        (Screen::IpLists, false) => "Enter detail  r refresh  a auto  s sort  / search",
```

Replace the test-only file `crates/illumio-tui/src/tui/ui/screens/iplists.rs` with the module below, followed by the Step 1 test module unchanged:

```rust
//! IP lists screen: every draft IP list with its size, entry counts and how
//! many loaded rules use it, and a detail pane for the selected one (ranges,
//! FQDNs, size, the rules by name). No request beyond the list itself.
use crate::tui::model::{IpListRow, Model, rule_kind_name};
use crate::tui::ui::screens::objects::{DetailLine, clip, items};
use crate::tui::ui::widgets::bars::age_text;
use crate::tui::ui::widgets::panel::panel;
use crate::tui::ui::widgets::table::{header_meta, render};
use pce::models::Actor;
use pce::{IpListSize, NameIndex, describe_actor};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use std::time::{Duration, Instant};

/// Rows of the detail pane, borders included.
const DETAIL_HEIGHT: u16 = 10;

/// `(table, detail)` areas. Shared by the view and `update` (page sizes); the
/// detail is dropped when the terminal is too short to keep a usable table.
pub fn layout(body: Rect, show_detail: bool) -> (Rect, Rect) {
    let detail_h = if show_detail && body.height >= 18 {
        DETAIL_HEIGHT
    } else {
        0
    };
    let rows = Layout::vertical([Constraint::Min(4), Constraint::Length(detail_h)]).split(body);
    (rows[0], rows[1])
}

/// Rows of data visible in the table (minus border and header).
pub fn table_rows(body: Rect, show_detail: bool) -> usize {
    layout(body, show_detail).0.height.saturating_sub(3) as usize
}

pub fn render_screen(model: &Model, f: &mut Frame, body: Rect) {
    let t = &model.theme;
    if model.index().is_none() {
        let note = if model.objects.index_failed {
            "no objects loaded · r retries"
        } else {
            "loading objects…"
        };
        f.render_widget(
            Paragraph::new(Span::styled(note, t.dim_style())).block(panel("IP lists", "", t, true)),
            body,
        );
        return;
    }
    let s = &model.ip_lists;
    let (table, detail) = layout(body, s.show_detail);
    let freshness = if s.loading {
        "loading…".to_string()
    } else {
        s.loaded_at
            .map(|l| {
                format!(
                    "cached {}",
                    age_text(Duration::from_secs(s.age_secs) + l.elapsed())
                )
            })
            .unwrap_or_default()
    };
    let (shown, total) = (s.table.len(), s.table.rows().len());
    let noun = if total == 1 { "row" } else { "rows" };
    let rows = if shown == total {
        format!("{total} {noun}")
    } else {
        format!("{shown}/{total} {noun}")
    };
    let mut meta = if freshness.is_empty() {
        rows
    } else {
        format!("{freshness} · {rows}")
    };
    meta.push_str(&s.auto.meta(Instant::now()));
    let sort_meta = header_meta(&s.table);
    if !sort_meta.is_empty() {
        meta = format!("{sort_meta} · {meta}");
    }
    render(
        f,
        table,
        &s.table,
        t,
        panel("IP lists", &meta, t, true),
        &|_: &IpListRow| Style::default(),
    );
    if detail.height > 0 {
        render_detail(model, f, detail);
    }
}

fn render_detail(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let block = panel("Detail", "Enter hides", t, false);
    let capacity = block.inner(area).height as usize;
    let Some(row) = model.ip_lists.table.selected_item() else {
        f.render_widget(
            Paragraph::new(Span::styled("no IP list selected", t.dim_style())).block(block),
            area,
        );
        return;
    };
    let text: Vec<Line> = clip(detail_lines(model, row), capacity)
        .into_iter()
        .map(|(head, body)| match head {
            Some(h) => Line::from(vec![
                Span::styled(format!("{h:<8}"), t.header_style()),
                Span::raw(body),
            ]),
            None => Line::from(vec![Span::raw("        "), Span::raw(body)]),
        })
        .collect();
    f.render_widget(Paragraph::new(text).block(block), area);
}

/// Name and description, size, one line per range (`!` before an
/// exclusion, its description after), the FQDNs, and the loaded rules that
/// name the list.
fn detail_lines(model: &Model, row: &IpListRow) -> Vec<DetailLine> {
    let l = &row.list;
    let mut out = vec![
        (
            Some("list"),
            match &l.description {
                Some(d) if !d.is_empty() => format!("{}  {d}", l.name),
                _ => l.name.clone(),
            },
        ),
        (Some("size"), size_text(&row.size)),
    ];
    let ranges: Vec<String> = l
        .ip_ranges
        .iter()
        .map(|r| {
            let mut s = String::new();
            if r.exclusion.unwrap_or(false) {
                s.push('!');
            }
            s.push_str(&r.from_ip);
            if let Some(to) = &r.to_ip {
                s.push('-');
                s.push_str(to);
            }
            if let Some(d) = r.description.as_deref().filter(|d| !d.is_empty()) {
                s.push_str("  ");
                s.push_str(d);
            }
            s
        })
        .collect();
    if !ranges.is_empty() || l.fqdns.is_empty() {
        out.extend(items("ranges", ranges));
    }
    if !l.fqdns.is_empty() {
        out.extend(items(
            "fqdns",
            l.fqdns.iter().map(|f| f.fqdn.clone()).collect(),
        ));
    }
    out.extend(used_by_lines(model, &l.href));
    out
}

/// `<ruleset> · <kind> · <providers> ← <consumers>` per rule, or where to
/// load the rules from.
fn used_by_lines(model: &Model, href: &str) -> Vec<DetailLine> {
    let Some(uses) = model.policy.ip_list_uses(href) else {
        return vec![(Some("used by"), "load Policy (2) to see rules".into())];
    };
    let empty = NameIndex::default();
    let idx = model.index().map_or(&empty, |i| i.as_ref());
    let names = |v: &[Actor]| {
        v.iter()
            .map(|a| describe_actor(a, idx))
            .collect::<Vec<_>>()
            .join(", ")
    };
    items(
        "used by",
        uses.iter()
            .map(|u| {
                format!(
                    "{} · {} · {} ← {}",
                    u.ruleset,
                    rule_kind_name(u.kind),
                    names(&u.providers),
                    names(&u.consumers)
                )
            })
            .collect(),
    )
}

/// `65.5K addresses (65536 IPv4, 0 IPv6)`, or the size alone for `any` and
/// FQDN-only lists.
fn size_text(s: &IpListSize) -> String {
    if s.unbounded.is_some() || (s.ranges == 0 && s.fqdns > 0) {
        return s.display();
    }
    format!("{} addresses ({} IPv4, {} IPv6)", s.display(), s.v4, s.v6)
}
```

`crates/illumio-tui/src/tui/test_support.rs`: in `model_with_objects` delete the `o.ip_lists.set_rows(…)` statement, change `o.ages = [Some(3); 4];` to `o.ages = [Some(3); 3];` and the doc comment to `/// A connected model with the three fixture object lists loaded (ages of 3 s).`; after it add:

```rust
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
```

- [ ] **Step 4: Run the unit tests**

Run: `cargo fmt --all && cargo test -p illumio-tui --lib`
Expected: PASS (the new `model`, `commands`, `ui`, `iplists` and `update` tests and every existing unit test).

- [ ] **Step 5: Create and read the snapshots, run everything**

Run: `git rm crates/illumio-tui/tests/snapshots/tui_snapshots__objects_iplists_detail_80x24.snap`

Run: `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots && git status --short crates/illumio-tui/tests/snapshots | awk '{print $1}' | sort | uniq -c`
Expected: `1 D` (`objects_iplists_detail_80x24`), `3 ??` (`tui_snapshots__ip_lists_80x24.snap`, `tui_snapshots__ip_lists_160x48.snap`, `tui_snapshots__ip_lists_light_styled_80x24.snap`) and `61 M`: every snapshot that shows the tab row (58, all but `events_detail_end_80x24`, the three help ones and `table_widget_40x10`) plus `help_overlay_80x24`, `help_overlay_end_80x24` and `help_overlay_end_light_styled_80x24`.

Check that nothing else moved: every changed line outside the help snapshots is the tab row, the Objects strip (snapshots drawn on the Objects screen, overlays included) or a colour-run line of a styled snapshot:

```bash
git diff -U0 -- crates/illumio-tui/tests/snapshots ':!*help_overlay*' \
  | grep '^[-+][^-+]' | grep -v 'Traf' | grep -v 'Label groups' | grep -v '^[-+]    @'
```

Expected: no output. The tab row reads `  1 Traf  │  2 Pol  │  3 Obj  │  4 Wl  │  5 Ev  │  6 IPs` at 80 columns and `… │  3 Objects  │ … │  6 IP lists` at 160; the strip reads `  Labels  │  Label groups  │  Services`. Read the three help diffs (the new number line, `:iplists`, the `IP lists Enter` line, the scroll range `of 30`) and the three new snapshots: the IP lists table (`name`, `size` right-aligned with `any` and `65.5K`, `ranges`, `fqdns`, `used by` `1`, `pending` with `create`), the detail with `size    65.5K addresses (65536 IPv4, 0 IPv6)` and `used by Shop-Prod · allow · role=web ← ipl:Corp`, and the hint `Enter detail  r refresh  a auto  s sort  / search`.

Run: `cargo test -p illumio-tui`
Expected: PASS, including `opening_ip_lists_costs_nothing_after_startup_and_r_bypasses_once` and the changed `update_ip_list_puts_only_the_changed_ranges`.

- [ ] **Step 6: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p illumio-tui
git add crates/illumio-tui/src crates/illumio-tui/tests
git commit -m "feat(tui): IP lists move to their own screen 6 with size and used-by

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 4: IP lists screen (write side, used-by recompute, CLI size)

**Files:**
- Modify: `crates/illumio-tui/src/tui/update.rs` (`Msg::RuleSetsLoaded` ~line 263, `Msg::DenyRulesLoaded` ~line 312, `policy_key`'s `d` arm ~line 1347; tests)
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` (`HELP_LINES` n/e/x entry)
- Modify: `crates/illumio-tui/src/cli/objects.rs` (imports lines 1-13; `impl TableRows for Vec<IpList>` ~lines 164-180 replaced; `ip_lists` ~line 375)
- Test: `crates/illumio-tui/tests/tui_runtime.rs` (append), `crates/illumio-tui/tests/cli.rs` (`other_object_lists` ~line 322)

**Interfaces:**
- Consumes: `IpListsState::recount`, `ip_lists_edit_key`, `model_with_ip_lists` (Task 3); `IpListSize`, `ip_list_size` (Task 1); `PolicyState::clear` (`model.rs` ~line 1030); `EditIntent::{CreateIpList, DeleteIpList}`, `WriteIntent::DeleteIpList { href, name }` (`tui/edit.rs`); `Form::title` (`tui/form.rs` ~line 38); `Context::emit`, `cache_label` (`cli/context.rs`); `mount_lists`, `key_session`, `writes_model`, `tui_guard`, `typed`, `ch`, `writes_seen` (`tests/tui_runtime.rs`); `cmd`, `stdout_json`, `mount_objects` (`tests/cli.rs`).
- Produces:

```rust
// cli/objects.rs
#[derive(Debug, Serialize)]
pub struct IpListOut { #[serde(flatten)] pub list: IpList, pub size: IpListSize }
impl TableRows for Vec<IpListOut> { /* name, size, ranges, pending, href */ }
```

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `crates/illumio-tui/src/tui/update.rs`:

```rust
    #[test]
    fn ip_list_keys_plan_through_the_plan_4_intents_and_refuse_without_writes_or_a_row() {
        use crate::tui::edit::WriteIntent;
        use crate::tui::test_support::model_with_ip_lists;
        let mut m = model_with_ip_lists();
        m.screen = Screen::IpLists;
        for c in ['n', 'e', 'x'] {
            assert!(update(&mut m, key(c)).is_empty(), "{c}");
            assert_eq!(m.status.text, "started without --allow-writes");
            assert_eq!(m.status.kind, StatusKind::Warn);
            assert_eq!(m.overlay, Overlay::None);
        }
        let mut m = writes_on(model_with_ip_lists());
        m.screen = Screen::IpLists;
        assert!(update(&mut m, key('n')).is_empty());
        let Overlay::Form(f) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(f.title, "New IP list");
        update(&mut m, kc(KeyCode::Esc));
        assert_eq!(m.overlay, Overlay::None);
        update(&mut m, key('j'));
        assert_eq!(
            update(&mut m, key('x')),
            vec![Cmd::PlanWrite(WriteIntent::DeleteIpList {
                href: "/orgs/5/sec_policy/draft/ip_lists/21".into(),
                name: "Corp".into(),
            })]
        );
        assert!(m.writes.in_flight);
        let mut m = writes_on(model_fixture());
        m.screen = Screen::IpLists;
        for c in ['e', 'x'] {
            assert!(update(&mut m, key(c)).is_empty(), "{c}");
            assert_eq!(m.status.text, "no row selected");
        }
    }

    #[test]
    fn used_by_follows_the_rulesets_as_they_load_and_switch_scope() {
        let mut m = model_fixture();
        m.screen = Screen::IpLists;
        let lists =
            serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json")).unwrap();
        update(&mut m, Msg::IpListsLoaded(Ok((lists, 0))));
        let used = |m: &Model| {
            m.ip_lists
                .table
                .rows()
                .iter()
                .map(|r| r.used_by)
                .collect::<Vec<_>>()
        };
        assert_eq!(used(&m), vec![None, None]);
        let raw = serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
        update(
            &mut m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Draft,
                result: Ok((raw, 0)),
            },
        );
        assert_eq!(used(&m), vec![Some(1), Some(1)]);
        // Global-Infra's deny rules come from the endpoint; one names Corp.
        let fetched: pce::models::DenyRule = serde_json::from_value(serde_json::json!({
            "href": "/orgs/5/sec_policy/draft/rule_sets/101/deny_rules/9",
            "providers": [{"actors": "ams"}],
            "consumers": [{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}],
            "ingress_services": [{"port": 23, "proto": 6}]
        }))
        .unwrap();
        update(
            &mut m,
            Msg::DenyRulesLoaded {
                href: "/orgs/5/sec_policy/draft/rule_sets/101".into(),
                result: Ok(vec![fetched]),
            },
        );
        assert_eq!(used(&m), vec![Some(1), Some(2)]);
        m.screen = Screen::Policy;
        update(&mut m, key('d'));
        assert_eq!(used(&m), vec![None, None], "the active scope is not counted");
    }
```

In `help_scrolls_to_its_last_line_and_reopens_at_the_top` add after the `IP lists Enter` assertion:

```rust
        assert!(
            HELP_LINES
                .iter()
                .any(|(_, d)| *d == "new/edit/delete: Objects, IP lists, Workloads, Policy")
        );
```

In `crates/illumio-tui/tests/cli.rs`, in `other_object_lists` replace the `iplists` part (from `let out = stdout_json(cmd(&server).arg("iplists")` through the `iplists --table` assertion) with:

```rust
    let out = stdout_json(cmd(&server).arg("iplists").assert().success());
    assert_eq!(out["data"][1]["name"], "Corp");
    assert_eq!(
        out["data"][1]["size"],
        json!({"v4": "65536", "v6": "0", "unbounded": null, "ranges": 1, "fqdns": 1})
    );
    assert_eq!(out["data"][0]["size"]["unbounded"], "0.0.0.0/0");
    assert_eq!(
        out["data"][1]["ip_ranges"][0]["from_ip"], "10.1.0.0",
        "the list keeps its own fields"
    );
    assert_eq!(out["meta"]["api_calls"], 1, "the size costs no request");
    cmd(&server)
        .args(["iplists", "--table"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("10.1.0.0-10.1.255.255, *.corp.example")
                .and(predicate::str::contains("65.5K"))
                .and(predicate::str::contains("any (0.0.0.0/0)")),
        );
```

Append to `crates/illumio-tui/tests/tui_runtime.rs`:

```rust
#[tokio::test]
async fn create_ip_list_on_the_ip_lists_screen_posts_once_and_reloads_once() {
    let server = MockServer::start().await;
    mount_lists(&server, 1, 2, 1).await; // IP lists: startup + the reload
    let body = json!({"name": "Lab", "ip_ranges": [{"from_ip": "10.9.0.0/16"}], "fqdns": []});
    let mut created = body.clone();
    created["href"] = json!("/orgs/5/sec_policy/draft/ip_lists/22");
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists"))
        .and(body_json(body))
        .respond_with(ResponseTemplate::new(201).set_body_json(created))
        .expect(1)
        .mount(&server)
        .await;
    let audit = MemoryAudit::new();
    let mut keys = vec![(500, ch('6')), (200, ch('n'))];
    keys.extend(typed("Lab"));
    keys.extend([(20, KeyCode::Tab), (20, KeyCode::Tab)]);
    keys.extend(typed("10.9.0.0/16"));
    keys.extend([(50, KeyCode::Enter), (300, ch('y')), (400, KeyCode::F(12))]);
    let model = key_session(
        &server,
        writes_model(),
        Some(tui_guard(audit.clone())),
        keys,
    )
    .await;
    assert_eq!(
        model.status.text,
        "created /orgs/5/sec_policy/draft/ip_lists/22 (pending)",
        "the reload keeps the write's report"
    );
    assert_eq!(
        model.stats.calls, 7,
        "health + four lists + POST + one IP lists reload; opening the screen was a cache hit"
    );
    assert_eq!(writes_seen(&server).await, vec!["POST"]);
    assert_eq!(audit.entries().len(), 1);
    assert_eq!(model.ip_lists.table.len(), 2, "the mock list did not change");
    assert_eq!(model.busy, 0);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --lib tui::update:: && cargo test -p illumio-tui --test cli other_object_lists`
Expected: FAIL: `used_by_follows_the_rulesets_as_they_load_and_switch_scope` (`[None, None]` after `RuleSetsLoaded`), `help_scrolls_to_its_last_line_and_reopens_at_the_top` (no such help line), `other_object_lists` (`data[1].size` is null). `ip_list_keys_plan_through_the_plan_4_intents_and_refuse_without_writes_or_a_row` already passes (Task 3 bound the keys); it is the coverage this task adds.

- [ ] **Step 3: Implement**

`crates/illumio-tui/src/tui/update.rs`:

In `Msg::RuleSetsLoaded`, `Ok` arm, after the `match model.index().cloned() { … }` statement:

```rust
                    model.ip_lists.recount(&model.policy);
```

In `Msg::DenyRulesLoaded`, `Ok(d)` arm, after the `if let Some(idx) = … { … }` block:

```rust
                    model.ip_lists.recount(&model.policy);
```

In `policy_key`'s `KeyCode::Char('d')` arm, after `model.policy.clear();`:

```rust
                model.ip_lists.recount(&model.policy);
```

`crates/illumio-tui/src/tui/ui/widgets/overlay.rs`: the n/e/x entry becomes

```rust
    (
        "n / e / x (writes)",
        "new/edit/delete: Objects, IP lists, Workloads, Policy",
    ),
```

`crates/illumio-tui/src/cli/objects.rs`: add `use pce::{IpListSize, ip_list_size};` and `use serde::Serialize;` to the imports; replace `impl TableRows for Vec<IpList> { … }` with:

```rust
/// An IP list as `iplists` prints it: the list as the PCE returned it plus
/// its `size` (no request).
#[derive(Debug, Serialize)]
pub struct IpListOut {
    #[serde(flatten)]
    pub list: IpList,
    pub size: IpListSize,
}

impl TableRows for Vec<IpListOut> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["name", "size", "ranges", "pending", "href"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .map(|o| {
                vec![
                    o.list.name.clone(),
                    o.size.display(),
                    o.list.summary(),
                    pending(&o.list.update_type),
                    o.list.href.clone(),
                ]
            })
            .collect()
    }
}
```

and `ip_lists` becomes:

```rust
pub async fn ip_lists(ctx: &Context) -> Result<(), CliError> {
    let c = ctx.pce.ip_lists(ctx.cache_mode).await?;
    let data: Vec<IpListOut> = c
        .value
        .into_iter()
        .map(|list| IpListOut {
            size: ip_list_size(&list),
            list,
        })
        .collect();
    ctx.emit(
        &data,
        cache_label(c.hit, ctx.cache_mode),
        c.truncated,
        None,
    )
}
```

- [ ] **Step 4: Run the unit tests**

Run: `cargo fmt --all && cargo test -p illumio-tui --lib`
Expected: PASS.

- [ ] **Step 5: Re-record the help snapshots, run everything**

Run: `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots && git status --short crates/illumio-tui/tests/snapshots`
Expected: exactly two modified files, `crates/illumio-tui/tests/snapshots/tui_snapshots__help_overlay_end_80x24.snap` and `crates/illumio-tui/tests/snapshots/tui_snapshots__help_overlay_end_light_styled_80x24.snap` (the n/e/x line; the first help page does not reach it). Read both.

Run: `cargo test -p illumio-tui`
Expected: PASS, including `create_ip_list_on_the_ip_lists_screen_posts_once_and_reloads_once` and `other_object_lists`.

- [ ] **Step 6: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p illumio-tui
git add crates/illumio-tui/src/tui/update.rs crates/illumio-tui/src/tui/ui/widgets/overlay.rs crates/illumio-tui/src/cli/objects.rs crates/illumio-tui/tests/tui_runtime.rs crates/illumio-tui/tests/cli.rs crates/illumio-tui/tests/snapshots/tui_snapshots__help_overlay_end_80x24.snap crates/illumio-tui/tests/snapshots/tui_snapshots__help_overlay_end_light_styled_80x24.snap
git commit -m "feat: IP list keys and used-by follow the rulesets; iplists prints sizes

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 5: Workload cache plumbing and rule detail impact

**Files:**
- Create: `crates/illumio-tui/src/tui/impact.rs`
- Modify: `crates/illumio-tui/src/tui/mod.rs` (`pub mod impact;` after `pub mod form;`)
- Modify: `crates/illumio-tui/src/tui/model.rs` (imports; `RuleRow` before `rule_columns` ~line 877; `rule_columns` gains `P/C`; `PolicyState.rules` ~line 947; `set_rule_sets` ~1019, `rebuild_rules` ~1045; `WorkloadsState` ~1583-1616 and `impact_source`; `Model::rebuild_rules` after `index` ~line 1941)
- Modify: `crates/illumio-tui/src/tui/msg.rs` (`Msg::WorkloadsLoaded` ~line 86)
- Modify: `crates/illumio-tui/src/tui/app.rs` (`Cmd::LoadWorkloads` ~line 339)
- Modify: `crates/illumio-tui/src/tui/update.rs` (`set_rule_sets`/`rebuild_rules` calls ~lines 136, 284, 317, 1271, 1430; `Msg::WorkloadsLoaded` ~line 330; the `Overlay::RuleDetail(_) | Overlay::Error(_)` arm ~line 1024; `policy_key` ~line 1343 and its `Enter` arm ~line 1399; new `impact_key` after `load_workloads`; tests ~lines 3605, 5198, 6371 and new ones)
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` (`render_rule_detail` ~line 503; `HELP_LINES`)
- Modify: `crates/illumio-tui/src/tui/test_support.rs` (`model_with_policy` ~line 88; new `with_workloads`)
- Test: `crates/illumio-tui/tests/tui_snapshots.rs` (`policy_rule_detail_overlay_80x24` ~line 268; append), `crates/illumio-tui/tests/tui_runtime.rs` (append)

**Interfaces:**
- Consumes: `Impact`, `ImpactInputs`, `RuleActors`, `rule_impact`, `IpListRef::line`, `Impact::{pc, summary}` (Task 2); `PolicyState::{find_rule, find_deny_rule}` (`model.rs` ~lines 1080-1107); `RuleView::{from_rule, from_deny}`; `labelled` (`overlay.rs` ~line 495); `load_workloads` (`update.rs` ~line 1469); `model_with_policy`, `fixture_index`, `render_to_string`, `render_styled`; runtime helpers `mount_objects`, `mount_rule_sets`, `mount_workloads`, `key_session`.
- Produces:

```rust
// tui/model.rs
#[derive(Debug, Clone)] pub struct RuleRow { pub view: RuleView, pub pc: String }
impl std::ops::Deref for RuleRow { type Target = RuleView; }
pub fn rule_columns() -> Vec<Column<RuleRow>>;            // kind, on, consumers, providers, services, P/C, pending
pub struct PolicyState { /* … */ pub rules: TableView<RuleRow> }
impl PolicyState {
    pub fn set_rule_sets(&mut self, raw: Vec<RuleSet>, idx: &NameIndex, workloads: Option<(&[Workload], bool)>);
    pub fn rebuild_rules(&mut self, idx: &NameIndex, workloads: Option<(&[Workload], bool)>);
}
pub struct WorkloadsState { /* … */ pub truncated: bool, pub error: Option<String> }
impl WorkloadsState { pub fn impact_source(&self) -> Option<(&[Workload], bool)>; }
impl Model { pub fn rebuild_rules(&mut self); }
// tui/msg.rs
Msg::WorkloadsLoaded(Result<(Vec<Workload>, u64, bool), String>)
// tui/impact.rs
pub const PRESS_I: &str;
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Source<'a> { Cold, Loading, Failed(&'a str), Ready { workloads: &'a [Workload], truncated: bool } }
pub fn source(w: &WorkloadsState) -> Source<'_>;
pub fn compute(scopes: &[Vec<Actor>], actors: &RuleActors, idx: &NameIndex, workloads: &[Workload], truncated: bool) -> Impact;
pub fn pc_cell(scopes: &[Vec<Actor>], actors: &RuleActors, idx: &NameIndex, workloads: Option<(&[Workload], bool)>) -> String;
pub fn loaded_rule<'a>(p: &'a PolicyState, href: &str) -> Option<(&'a [Vec<Actor>], RuleActors)>;
pub fn ip_list_lines(i: &Impact) -> Vec<String>;
pub fn rule_detail_rows(model: &Model, href: &str) -> Vec<(&'static str, String)>;
// tui/test_support.rs
pub fn with_workloads(m: Model) -> Model;
// tui/update.rs (private)
fn impact_key(model: &mut Model) -> Vec<Cmd>;
```

- [ ] **Step 1: Write the failing tests**

Create `crates/illumio-tui/src/tui/impact.rs` with only this module, and add `pub mod impact;` to `crates/illumio-tui/src/tui/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::{model_with_policy, with_workloads};

    const RULE_2: &str = "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2";
    const DENY_8: &str = "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8";

    fn row(label: &'static str, text: &str) -> (&'static str, String) {
        (label, text.to_string())
    }

    #[test]
    fn the_rule_detail_rows_follow_the_workload_cache() {
        let mut m = model_with_policy();
        assert_eq!(rule_detail_rows(&m, RULE_2), vec![row("impact", PRESS_I)]);
        m.workloads.loading = true;
        assert_eq!(
            rule_detail_rows(&m, RULE_2),
            vec![row("impact", "loading workloads…")]
        );
        m.workloads.loading = false;
        m.workloads.error = Some("timeout".into());
        assert_eq!(
            rule_detail_rows(&m, RULE_2),
            vec![row("impact", "unavailable (timeout)")]
        );
        let m = with_workloads(model_with_policy());
        assert_eq!(
            rule_detail_rows(&m, RULE_2),
            vec![
                row("impact", "1 provider · 0 consumers"),
                row("ip lists", "Corp: 65.5K addresses"),
            ]
        );
        assert_eq!(
            rule_detail_rows(&m, DENY_8),
            vec![row("impact", "1 provider · 2 consumers")]
        );
        assert_eq!(
            rule_detail_rows(&m, "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/99"),
            vec![row("impact", "rule not loaded")]
        );
    }

    #[test]
    fn ip_list_rows_stop_at_three() {
        let one = pce::IpListRef {
            name: "L".into(),
            href: "/orgs/5/sec_policy/draft/ip_lists/1".into(),
            size: None,
            side: pce::Side::Consumer,
        };
        let impact = Impact {
            providers: 0,
            consumers: 0,
            per_scope: vec![],
            ip_lists: vec![one; 5],
            unsupported: vec![],
            truncated: false,
        };
        assert_eq!(
            ip_list_lines(&impact),
            vec!["L: ? addresses", "L: ? addresses", "L: ? addresses", "+2 more"]
        );
    }

    #[test]
    fn the_rules_pane_shows_p_c_once_the_workloads_are_cached() {
        let m = model_with_policy();
        assert!(m.policy.rules.rows().iter().all(|r| r.pc == "-"));
        let m = with_workloads(model_with_policy());
        let pcs: Vec<&str> = m.policy.rules.rows().iter().map(|r| r.pc.as_str()).collect();
        assert_eq!(
            pcs,
            vec!["1/2", "1/1", "1/0", "2/0"],
            "override deny, allow, allow, deny"
        );
        let titles: Vec<&str> = m.policy.rules.columns.iter().map(|c| c.title).collect();
        assert_eq!(
            titles,
            vec!["kind", "on", "consumers", "providers", "services", "P/C", "pending"]
        );
    }
}
```

In `crates/illumio-tui/src/tui/update.rs` tests: change `update(&mut m, Msg::WorkloadsLoaded(Ok((wl, 0))));` (~line 3605) to `update(&mut m, Msg::WorkloadsLoaded(Ok((wl, 0, false))));` and `m.policy.set_rule_sets(raw, &fixture_index());` (~line 6371) to `m.policy.set_rule_sets(raw, &fixture_index(), None);`. In `help_scrolls_to_its_last_line_and_reopens_at_the_top` the counts become:

```rust
        assert_eq!(HELP_LINES.len(), 31);
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Policy   i"));
```

```rust
        assert_eq!(last, 9, "31 lines, 22 rows inside the box");
```

Append to `mod tests` in `update.rs`:

```rust
    #[test]
    fn i_loads_the_workloads_once_for_the_impact_from_policy_and_the_rule_detail() {
        use crate::tui::test_support::model_with_policy;
        let mut m = model_with_policy();
        m.screen = Screen::Policy;
        assert_eq!(
            update(&mut m, key('i')),
            vec![Cmd::LoadWorkloads(pce::CacheMode::Use)]
        );
        assert_eq!(m.status.text, "loading workloads for impact (1 request)");
        assert!(update(&mut m, key('i')).is_empty());
        assert_eq!(m.status.text, "workloads already loading");
        let wl = serde_json::from_str(include_str!("../../../../fixtures/workloads.json")).unwrap();
        update(&mut m, Msg::WorkloadsLoaded(Ok((wl, 0, false))));
        assert_eq!(m.policy.rules.rows()[1].pc, "1/1", "the rules pane follows");
        assert!(update(&mut m, key('i')).is_empty());
        assert_eq!(
            m.status.text,
            "4 workloads already loaded; r on Workloads (4) refreshes them"
        );
        // In the rule detail: `i` loads, the detail stays, Esc still closes.
        let mut m = model_with_policy();
        m.screen = Screen::Policy;
        update(&mut m, kc(KeyCode::Tab));
        update(&mut m, enter());
        assert!(matches!(m.overlay, Overlay::RuleDetail(_)));
        assert_eq!(
            update(&mut m, key('i')),
            vec![Cmd::LoadWorkloads(pce::CacheMode::Use)]
        );
        assert!(matches!(m.overlay, Overlay::RuleDetail(_)), "the detail stays open");
        update(&mut m, kc(KeyCode::Esc));
        assert_eq!(m.overlay, Overlay::None);
    }

    #[test]
    fn a_truncated_or_failed_workload_read_shows_in_the_impact() {
        use crate::tui::impact::rule_detail_rows;
        use crate::tui::test_support::model_with_policy;
        let rule = "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1";
        let mut m = model_with_policy();
        let wl = serde_json::from_str(include_str!("../../../../fixtures/workloads.json")).unwrap();
        update(&mut m, Msg::WorkloadsLoaded(Ok((wl, 0, true))));
        assert!(m.workloads.truncated);
        assert_eq!(m.policy.rules.rows()[1].pc, "≥1/≥1");
        assert_eq!(
            rule_detail_rows(&m, rule)[0].1,
            "≥1 provider · ≥1 consumer (workload list truncated)"
        );
        let mut m = model_with_policy();
        update(&mut m, Msg::WorkloadsLoaded(Err("boom".into())));
        assert_eq!(m.workloads.error.as_deref(), Some("boom"));
        assert_eq!(rule_detail_rows(&m, rule)[0].1, "unavailable (boom)");
        m.screen = Screen::Policy;
        assert_eq!(update(&mut m, key('i')).len(), 1, "i tries again");
        assert_eq!(m.workloads.error, None);
    }
```

In `crates/illumio-tui/tests/tui_snapshots.rs`, `policy_rule_detail_overlay_80x24`: `let rule = m.policy.rules.rows()[0].clone();` becomes `let rule = m.policy.rules.rows()[0].view.clone();`, and after the `override_deny` assertion add `assert!(out.contains("press i to load workloads (1 request)"), "{out}");`. Append:

```rust
#[test]
fn rule_detail_and_rules_pane_show_the_impact_80x24() {
    use illumio_tui::tui::model::Overlay;
    use illumio_tui::tui::test_support::{model_with_policy, with_workloads};
    let mut m = with_workloads(model_with_policy());
    m.screen = Screen::Policy;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("P/C") && out.contains("1/2"), "{out}");
    insta::assert_snapshot!("policy_impact_80x24", out);
    let mut l = with_workloads(model_with_policy());
    l.screen = Screen::Policy;
    l.set_theme("light");
    insta::assert_snapshot!(
        "policy_impact_light_styled_80x24",
        render_styled(&l, 80, 24)
    );
    let rule = m.policy.rules.rows()[2].view.clone(); // corp to web
    m.overlay = Overlay::RuleDetail(rule.clone());
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("1 provider · 0 consumers"), "{out}");
    assert!(out.contains("Corp: 65.5K addresses"), "{out}");
    insta::assert_snapshot!("rule_detail_impact_80x24", out);
    l.overlay = Overlay::RuleDetail(rule);
    insta::assert_snapshot!(
        "rule_detail_impact_light_styled_80x24",
        render_styled(&l, 80, 24)
    );
}
```

Append to `crates/illumio-tui/tests/tui_runtime.rs`:

```rust
#[tokio::test]
async fn i_on_a_rule_detail_loads_workloads_once_and_a_second_i_is_free() {
    use illumio_tui::tui::impact::rule_detail_rows;
    use illumio_tui::tui::model::Overlay;
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_rule_sets(&server, 1).await;
    mount_workloads(&server, 1).await;
    let keys = vec![
        (500, ch('2')),
        (300, KeyCode::Tab),
        (50, KeyCode::Enter), // the override deny rule sorts first
        (50, ch('i')),
        (400, ch('i')),
        (200, KeyCode::F(12)),
    ];
    let model = key_session(&server, model(), None, keys).await;
    assert_eq!(
        model.stats.calls, 7,
        "health + four lists + rulesets + one workloads read"
    );
    let Overlay::RuleDetail(rule) = &model.overlay else {
        panic!("{:?}", model.overlay)
    };
    assert_eq!(
        rule_detail_rows(&model, &rule.href)[0],
        ("impact", "1 provider · 2 consumers".to_string())
    );
    assert_eq!(model.policy.rules.rows()[0].pc, "1/2");
    assert_eq!(
        model.status.text,
        "4 workloads already loaded; r on Workloads (4) refreshes them"
    );
    assert_eq!(model.busy, 0);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --lib`
Expected: FAIL to compile (`cannot find function rule_detail_rows`, `no field pc`, `WorkloadsLoaded` takes a 3-tuple, `set_rule_sets` takes 3 arguments, `no function with_workloads`).

- [ ] **Step 3: Implement**

`crates/illumio-tui/src/tui/model.rs`. Add `use pce::impact::RuleActors;` to the imports. Before `rule_columns` add:

```rust
/// A rules-pane row: the resolved rule and its `P/C` cell (`12/3`, `-`
/// until the workloads are cached). The cell is computed when the rows are
/// built: a column's `fn` pointer cannot see the workload cache.
#[derive(Debug, Clone)]
pub struct RuleRow {
    pub view: RuleView,
    pub pc: String,
}

impl std::ops::Deref for RuleRow {
    type Target = RuleView;

    fn deref(&self) -> &RuleView {
        &self.view
    }
}
```

`rule_columns` becomes `pub fn rule_columns() -> Vec<Column<RuleRow>>` (its existing cells keep compiling through `Deref`), with this column inserted before `pending`:

```rust
        Column {
            title: "P/C",
            width: 7,
            cell: |r| r.pc.clone(),
            key: |r| SortKey::Text(r.pc.clone()),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
```

In `PolicyState` the field becomes `pub rules: TableView<RuleRow>,`; `set_rule_sets` and `rebuild_rules` become:

```rust
    pub fn set_rule_sets(
        &mut self,
        raw: Vec<RuleSet>,
        idx: &NameIndex,
        workloads: Option<(&[Workload], bool)>,
    ) {
        self.rulesets.set_rows(
            raw.iter()
                .map(|rs| RuleSetView::from_rule_set(rs, idx))
                .collect(),
        );
        self.raw = raw;
        self.rebuild_rules(idx, workloads);
    }
```

```rust
    /// Rules of the selected ruleset in evaluation order: override deny rules,
    /// then allow rules, then deny rules (inline or fetched fallback). The
    /// order within each group is the PCE's. `workloads` (the cache, when
    /// loaded) fills the `P/C` column.
    pub fn rebuild_rules(&mut self, idx: &NameIndex, workloads: Option<(&[Workload], bool)>) {
        let Some(href) = self.selected_href() else {
            self.rules.set_rows(Vec::new());
            return;
        };
        let Some(rs) = self.raw.iter().find(|r| r.href == href) else {
            self.rules.set_rows(Vec::new());
            return;
        };
        let pc = |a: &RuleActors| crate::tui::impact::pc_cell(&rs.scopes, a, idx, workloads);
        let mut rows: Vec<RuleRow> = rs
            .rules
            .iter()
            .map(|r| RuleRow {
                view: RuleView::from_rule(r, idx),
                pc: pc(&RuleActors::of_rule(r)),
            })
            .collect();
        let deny: &[DenyRule] = if rs.deny_rules.is_empty() {
            self.fallback_deny.get(&href).map_or(&[], Vec::as_slice)
        } else {
            &rs.deny_rules
        };
        rows.extend(deny.iter().map(|d| RuleRow {
            view: RuleView::from_deny(d, idx),
            pc: pc(&RuleActors::of_deny(d)),
        }));
        rows.sort_by_key(|r| rule_rank(r.kind));
        self.rules.set_rows(rows);
    }
```

In `WorkloadsState` add after `pub raw: Vec<Workload>,`:

```rust
    /// The last read was cut short: impact counts are lower bounds.
    pub truncated: bool,
    /// Why the last read failed, until one succeeds (`impact: unavailable`).
    pub error: Option<String>,
```

initialise them with `truncated: false, error: None,` in `WorkloadsState::new`, and add to `impl WorkloadsState`:

```rust
    /// The cached workloads and their `truncated` flag, once loaded.
    pub fn impact_source(&self) -> Option<(&[Workload], bool)> {
        self.loaded_at
            .is_some()
            .then_some((self.raw.as_slice(), self.truncated))
    }
```

After `Model::index` add:

```rust
    /// Rebuilds the rules pane from what is loaded, its `P/C` column from
    /// the cached workloads (`-` without them). No request.
    pub fn rebuild_rules(&mut self) {
        let Some(idx) = self.index().cloned() else {
            return;
        };
        let workloads = self.workloads.impact_source();
        self.policy.rebuild_rules(&idx, workloads);
    }
```

`crates/illumio-tui/src/tui/msg.rs`:

```rust
    /// Workloads, the cache age in seconds and whether the PCE holds more
    /// than were returned.
    WorkloadsLoaded(Result<(Vec<Workload>, u64, bool), String>),
```

`crates/illumio-tui/src/tui/app.rs`, `Cmd::LoadWorkloads`: `.map(|c| (c.value, c.age.as_secs(), c.truncated))`.

`crates/illumio-tui/src/tui/update.rs`:

- `model.policy.set_rule_sets(raw, &idx);` (~lines 136 and 284) becomes `model.policy.set_rule_sets(raw, &idx, model.workloads.impact_source());`, and `model.policy.rebuild_rules(&idx);` (~lines 317, 1271, 1430) becomes `model.policy.rebuild_rules(&idx, model.workloads.impact_source());`.
- `Msg::WorkloadsLoaded` becomes:

```rust
        Msg::WorkloadsLoaded(result) => {
            model.workloads.loading = false;
            match result {
                Ok((rows, age, truncated)) => {
                    let w = &mut model.workloads;
                    w.raw = rows;
                    w.truncated = truncated;
                    w.error = None;
                    w.rebuild();
                    w.loaded_at = Some(Instant::now());
                    w.auto.from_tick = false;
                    w.age_secs = age;
                    w.show_detail &= w.table.selected_item().is_some();
                    model.status =
                        StatusLine::info(format!("{} workloads", model.workloads.table.len()));
                    // The rules pane's P/C follows; an open rule detail or
                    // confirm reads the cache when it is drawn, so no overlay
                    // is touched here.
                    model.rebuild_rules();
                }
                Err(e) => {
                    model.workloads.error = Some(e.clone());
                    let tripped = model.workloads.auto.take_tick_failure();
                    fail(model, "could not load workloads", &e, tripped);
                }
            }
            Vec::new()
        }
```

- The `Overlay::RuleDetail(_) | Overlay::Error(_)` arm becomes two arms:

```rust
        Overlay::RuleDetail(_) => match k.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                model.overlay = Overlay::None;
                Vec::new()
            }
            KeyCode::Char('i') => impact_key(model),
            _ => Vec::new(),
        },
        Overlay::Error(_) => {
            if matches!(k.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q')) {
                model.overlay = Overlay::None;
            }
            Vec::new()
        }
```

- In `policy_key`, in the `match k.code` of the non-searching block, after the `KeyCode::Char('a')` arm:

```rust
            KeyCode::Char('i') => return impact_key(model),
```

  and in its `KeyCode::Enter if model.policy.pane == PolicyPane::Rules` arm `Overlay::RuleDetail(r.clone())` becomes `Overlay::RuleDetail(r.view.clone())`.
- After `load_workloads` add:

```rust
/// `i` on Policy, in a rule detail or a rule's confirm: the workload list
/// behind the impact, read once. A list already cached or on its way costs
/// nothing.
fn impact_key(model: &mut Model) -> Vec<Cmd> {
    if !model.is_connected() {
        return Vec::new();
    }
    if model.workloads.loading {
        model.status = StatusLine::info("workloads already loading");
        return Vec::new();
    }
    if model.workloads.loaded_at.is_some() {
        model.status = StatusLine::info(format!(
            "{} workloads already loaded; r on Workloads (4) refreshes them",
            model.workloads.raw.len()
        ));
        return Vec::new();
    }
    model.workloads.error = None;
    model.status = StatusLine::info("loading workloads for impact (1 request)");
    load_workloads(model, CacheMode::Use)
}
```

`crates/illumio-tui/src/tui/ui/widgets/overlay.rs`: in `render_rule_detail` the box becomes `centered(area, 76, 16)`, `let lines = vec![…];` becomes `let mut lines = vec![…];`, and after it:

```rust
    lines.extend(
        crate::tui::impact::rule_detail_rows(model, &rule.href)
            .into_iter()
            .map(|(label, text)| labelled(model, label, text)),
    );
```

In `HELP_LINES`, after the `"Policy   N"` entry:

```rust
    ("Policy   i", "rule impact: load the workloads once (1 request)"),
```

`crates/illumio-tui/src/tui/test_support.rs`: `m.policy.set_rule_sets(raw, &idx);` becomes `m.policy.set_rule_sets(raw, &idx, None);`; after `model_with_policy` add:

```rust
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
```

Replace the test-only `crates/illumio-tui/src/tui/impact.rs` with the module below, followed by the Step 1 test module unchanged:

```rust
//! Rule impact as the TUI shows it: the rules pane's `P/C` cells and the
//! rule detail's impact rows. Computed from the loaded rulesets, the name
//! index and the cached workloads; never a request (`i` asks for the
//! workloads, in `update.rs`).
use crate::tui::model::{Model, PolicyState, WorkloadsState};
use pce::NameIndex;
use pce::impact::{Impact, ImpactInputs, RuleActors, rule_impact};
use pce::models::{Actor, Workload};

/// What the impact says while the workloads are not cached.
pub const PRESS_I: &str = "press i to load workloads (1 request)";
const LOADING: &str = "loading workloads…";
/// IP list rows shown before `+N more`.
const MAX_IP_LIST_ROWS: usize = 3;

/// Where the workload list behind the impact stands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Source<'a> {
    Cold,
    Loading,
    Failed(&'a str),
    Ready {
        workloads: &'a [Workload],
        truncated: bool,
    },
}

pub fn source(w: &WorkloadsState) -> Source<'_> {
    if let Some((workloads, truncated)) = w.impact_source() {
        Source::Ready {
            workloads,
            truncated,
        }
    } else if w.loading {
        Source::Loading
    } else if let Some(e) = w.error.as_deref() {
        Source::Failed(e)
    } else {
        Source::Cold
    }
}

/// The impact of a rule with these scopes and actors over `workloads`.
pub fn compute(
    scopes: &[Vec<Actor>],
    actors: &RuleActors,
    idx: &NameIndex,
    workloads: &[Workload],
    truncated: bool,
) -> Impact {
    let inputs = ImpactInputs {
        workloads,
        index: idx,
        truncated,
    };
    rule_impact(
        scopes,
        &actors.providers,
        &actors.consumers,
        actors.unscoped_consumers,
        &inputs,
    )
}

/// A rules-pane `P/C` cell: `12/3`, or `-` without cached workloads.
pub fn pc_cell(
    scopes: &[Vec<Actor>],
    actors: &RuleActors,
    idx: &NameIndex,
    workloads: Option<(&[Workload], bool)>,
) -> String {
    match workloads {
        Some((w, truncated)) => compute(scopes, actors, idx, w, truncated).pc(),
        None => "-".to_string(),
    }
}

/// The same ruleset in either scope (an active href folds to draft).
fn same_policy_href(a: &str, b: &str) -> bool {
    let fold = |h: &str| h.replacen("/sec_policy/active/", "/sec_policy/draft/", 1);
    fold(a) == fold(b)
}

/// A loaded ruleset's scopes by href.
pub(crate) fn scopes_of<'a>(p: &'a PolicyState, rs_href: &str) -> Option<&'a [Vec<Actor>]> {
    p.raw
        .iter()
        .find(|rs| same_policy_href(&rs.href, rs_href))
        .map(|rs| rs.scopes.as_slice())
}

/// A loaded rule's ruleset scopes and actors (allow, deny or fetched deny).
pub fn loaded_rule<'a>(p: &'a PolicyState, href: &str) -> Option<(&'a [Vec<Actor>], RuleActors)> {
    let (actors, rs_href) = match p.find_rule(href) {
        Some((r, rs)) => (RuleActors::of_rule(&r), rs),
        None => {
            let (d, rs) = p.find_deny_rule(href)?;
            (RuleActors::of_deny(&d), rs)
        }
    };
    Some((scopes_of(p, &rs_href)?, actors))
}

/// One line per IP list actor (`Corp: 65.5K addresses`), at most three,
/// then `+N more`.
pub fn ip_list_lines(i: &Impact) -> Vec<String> {
    let mut out: Vec<String> = i
        .ip_lists
        .iter()
        .take(MAX_IP_LIST_ROWS)
        .map(|l| l.line())
        .collect();
    if i.ip_lists.len() > MAX_IP_LIST_ROWS {
        out.push(format!("+{} more", i.ip_lists.len() - MAX_IP_LIST_ROWS));
    }
    out
}

/// What the impact says without an answer yet: press `i`, loading, or the
/// failure. (`Ready` has an answer; callers match it first.)
pub(crate) fn waiting_text(s: Source<'_>) -> String {
    match s {
        Source::Cold | Source::Ready { .. } => PRESS_I.to_string(),
        Source::Loading => LOADING.to_string(),
        Source::Failed(e) => format!("unavailable ({e})"),
    }
}

/// The rule detail's impact rows, `(label, text)`: the counts and one row
/// per IP list, or what it takes to get them.
pub fn rule_detail_rows(model: &Model, href: &str) -> Vec<(&'static str, String)> {
    let (workloads, truncated) = match source(&model.workloads) {
        Source::Ready {
            workloads,
            truncated,
        } => (workloads, truncated),
        waiting => return vec![("impact", waiting_text(waiting))],
    };
    let (Some(idx), Some((scopes, actors))) = (model.index(), loaded_rule(&model.policy, href))
    else {
        return vec![("impact", "rule not loaded".to_string())];
    };
    let impact = compute(scopes, &actors, idx, workloads, truncated);
    let mut rows = vec![("impact", impact.summary())];
    rows.extend(
        ip_list_lines(&impact)
            .into_iter()
            .enumerate()
            .map(|(i, l)| (if i == 0 { "ip lists" } else { "" }, l)),
    );
    rows
}
```

- [ ] **Step 4: Run the unit tests**

Run: `cargo fmt --all && cargo test -p illumio-tui --lib`
Expected: PASS (the three `impact` tests, the two new `update` tests and every existing unit test).

- [ ] **Step 5: Create and read the snapshots, run everything**

Run: `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots && git status --short crates/illumio-tui/tests/snapshots`
Expected: four new files, `tui_snapshots__policy_impact_80x24.snap`, `tui_snapshots__policy_impact_light_styled_80x24.snap`, `tui_snapshots__rule_detail_impact_80x24.snap`, `tui_snapshots__rule_detail_impact_light_styled_80x24.snap`; modified files only from this set: `policy_80x24`, `policy_160x48`, `light_policy_160x48`, `policy_rule_detail_80x24`, `policy_pending_80x24`, `policy_pending_marks_80x24`, `policy_pending_marks_light_styled_80x24`, `pending_diff_80x24`, `pending_diff_light_styled_80x24`, `pending_rule_set_diff_80x24`, `pending_rule_set_diff_light_styled_80x24`, `rule_set_form_80x24`, `rule_set_form_light_styled_80x24`, `form_rule_80x24`, `form_rule_light_styled_80x24`, `form_deny_rule_80x24`, `form_deny_rule_light_styled_80x24` (the Policy screen behind the overlay), and `help_overlay_80x24`, `help_overlay_end_80x24`, `help_overlay_end_light_styled_80x24` (the `Policy   i` line). Read every diff: outside the help files only the rules pane changes (a `P/C` header, `-` cells, narrower flexible columns), and in `policy_rule_detail_80x24` the detail box is two rows taller with `impact      press i to load workloads (1 request)`. Read the new snapshots: `P/C` reads `1/2`, `1/1`, `1/0`, `2/0`; the detail for `corp to web` ends with `impact      1 provider · 0 consumers` and `ip lists    Corp: 65.5K addresses`.

Run: `cargo test -p illumio-tui`
Expected: PASS, including `i_on_a_rule_detail_loads_workloads_once_and_a_second_i_is_free`.

- [ ] **Step 6: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p illumio-tui
git add crates/illumio-tui/src crates/illumio-tui/tests
git commit -m "feat(tui): i loads the workloads once; rule detail and rules pane show the impact

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 6: Confirm overlay impact line

**Files:**
- Modify: `crates/illumio-tui/src/tui/impact.rs` (imports; `rule_request`, `confirm_rows`, `confirm_line`; tests)
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` (`confirm_write_layout` ~line 400, `confirm_write_last_scroll` ~line 415, `render_confirm_write` ~line 429)
- Modify: `crates/illumio-tui/src/tui/update.rs` (the `Overlay::ConfirmWrite` arm ~line 928; test)
- Modify: `crates/illumio-tui/src/tui/test_support.rs` (new `rule_post_plan` after `ip_list_delete_plan` ~line 214)
- Test: `crates/illumio-tui/tests/tui_snapshots.rs`, `crates/illumio-tui/tests/tui_runtime.rs` (append)

**Interfaces:**
- Consumes: `RuleActors::planned`, `rule_set_href_of`, `is_rule_href` (Task 2); `source`, `waiting_text`, `scopes_of`, `loaded_rule`, `compute` (Task 5, `tui/impact.rs`); `impact_key` (Task 5, `update.rs`); `Plan::Send { request: WriteRequest { method, path, body, summary }, kind }` (`pce/src/write/mod.rs` ~lines 103-123); `WriteMethod` (`pce/src/client.rs` ~line 67); `label_post_plan` (`test_support.rs` ~line 191); runtime helpers `mount_rule_sets`, `mount_workloads`, `pce_for`, `tui_guard`, `press`, `ch`, `writes_seen`; `model_with_flows`, `fixture_index`.
- Produces:

```rust
// tui/impact.rs
pub fn confirm_rows(plan: &Plan) -> u16;                       // 1 for a rule or deny-rule POST/PUT, else 0
pub fn confirm_line(model: &Model, plan: &Plan) -> Option<String>; // "impact: …"
// tui/ui/widgets/overlay.rs
pub fn confirm_write_last_scroll(req: &WriteRequest, area: Rect, extra: u16) -> usize;
// tui/test_support.rs
pub fn rule_post_plan() -> pce::write::Plan; // a new allow rule in Shop-Prod: role=web ← iplist Corp, 443/tcp, unscoped
```

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/src/tui/test_support.rs` add after `ip_list_delete_plan` (a fixture, not code under test):

```rust
/// The plan for a new allow rule in Shop-Prod: role=web provides 443/tcp
/// to the Corp IP list, consumers unscoped.
pub fn rule_post_plan() -> pce::write::Plan {
    pce::write::Plan::Send {
        request: pce::write::WriteRequest {
            method: pce::WriteMethod::Post,
            path: "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules".into(),
            body: Some(serde_json::json!({
                "enabled": true,
                "providers": [{"label": {"href": "/orgs/5/labels/1"}}],
                "consumers": [{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}],
                "ingress_services": [{"port": 443, "proto": 6}],
                "unscoped_consumers": true
            })),
            summary: "create rule in ruleset 100".into(),
        },
        kind: pce::write::ObjectKind::Rule,
    }
}
```

Append to `mod tests` in `crates/illumio-tui/src/tui/impact.rs`:

```rust
    #[test]
    fn the_confirm_line_reads_the_planned_body() {
        use crate::tui::test_support::{label_post_plan, model_fixture, rule_post_plan};
        use pce::WriteMethod;
        use pce::write::{Plan, WriteRequest};
        let m = model_with_policy();
        assert_eq!(confirm_line(&m, &label_post_plan()), None);
        assert_eq!(confirm_rows(&label_post_plan()), 0);
        assert_eq!(confirm_rows(&rule_post_plan()), 1);
        assert_eq!(
            confirm_line(&m, &rule_post_plan()).as_deref(),
            Some("impact: press i to load workloads (1 request)")
        );
        let m = with_workloads(model_with_policy());
        assert_eq!(
            confirm_line(&m, &rule_post_plan()).as_deref(),
            Some("impact: 1 provider · 0 consumers")
        );
        // An update sends only what changes; the rest is the loaded rule's.
        let update = Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Put,
                path: RULE_2.into(),
                body: Some(serde_json::json!({"consumers": [
                    {"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}},
                    {"actors": "ams"}
                ]})),
                summary: "update rule".into(),
            },
            kind: pce::write::ObjectKind::Rule,
        };
        assert_eq!(confirm_rows(&update), 1);
        assert_eq!(
            confirm_line(&m, &update).as_deref(),
            Some("impact: 1 provider · 4 consumers"),
            "unscoped, as loaded"
        );
        let delete = Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Delete,
                path: RULE_2.into(),
                body: None,
                summary: "delete rule".into(),
            },
            kind: pce::write::ObjectKind::Rule,
        };
        assert_eq!(confirm_rows(&delete), 0);
        assert_eq!(confirm_line(&m, &delete), None);
        let lone = with_workloads(model_fixture());
        assert_eq!(
            confirm_line(&lone, &rule_post_plan()).as_deref(),
            Some("impact: ruleset not loaded; open Policy (2) first")
        );
    }
```

Append to `mod tests` in `crates/illumio-tui/src/tui/update.rs`:

```rust
    #[test]
    fn i_in_a_rule_confirm_loads_workloads_and_keeps_the_request_on_screen() {
        use crate::tui::test_support::{label_post_plan, model_with_policy, rule_post_plan};
        let mut m = writes_on(model_with_policy());
        update(
            &mut m,
            Msg::Planned(Ok((rule_post_plan(), pce::write::ObjectKind::Rule))),
        );
        let shown = m.overlay.clone();
        assert!(matches!(shown, Overlay::ConfirmWrite { .. }));
        assert_eq!(
            update(&mut m, key('i')),
            vec![Cmd::LoadWorkloads(pce::CacheMode::Use)]
        );
        assert_eq!(m.overlay, shown, "the confirm stays");
        let wl = serde_json::from_str(include_str!("../../../../fixtures/workloads.json")).unwrap();
        update(&mut m, Msg::WorkloadsLoaded(Ok((wl, 0, false))));
        assert_eq!(m.overlay, shown, "the answer never replaces the confirm");
        let Overlay::ConfirmWrite { plan, .. } = &m.overlay else {
            unreachable!()
        };
        assert_eq!(
            crate::tui::impact::confirm_line(&m, plan).as_deref(),
            Some("impact: 1 provider · 0 consumers")
        );
        let cmds = update(&mut m, key('y'));
        assert!(matches!(cmds.as_slice(), [Cmd::ExecuteWrite { .. }]), "{cmds:?}");
        // A label's confirm has no impact: `i` does nothing there.
        let mut m = writes_on(model_with_policy());
        update(
            &mut m,
            Msg::Planned(Ok((label_post_plan(), pce::write::ObjectKind::Label))),
        );
        assert!(update(&mut m, key('i')).is_empty());
        assert!(!m.workloads.loading);
    }
```

Append to `crates/illumio-tui/tests/tui_snapshots.rs`:

```rust
#[test]
fn confirm_rule_overlay_shows_the_impact_80x24() {
    use illumio_tui::tui::model::{Overlay, WritesState};
    use illumio_tui::tui::test_support::{model_with_policy, rule_post_plan, with_workloads};
    let mut m = with_workloads(model_with_policy());
    m.writes = WritesState::on();
    m.screen = Screen::Policy;
    m.overlay = Overlay::ConfirmWrite {
        plan: rule_post_plan(),
        kind: pce::write::ObjectKind::Rule,
        scroll: 0,
    };
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("impact: 1 provider · 0 consumers"), "{out}");
    assert!(
        out.contains("POST /orgs/5/sec_policy/draft/rule_sets/100/sec_rules"),
        "{out}"
    );
    insta::assert_snapshot!("confirm_rule_impact_80x24", out);
    let mut cold = model_with_policy();
    cold.writes = WritesState::on();
    cold.screen = Screen::Policy;
    cold.overlay = m.overlay.clone();
    assert!(
        render_to_string(&cold, 80, 24).contains("impact: press i to load workloads (1 request)")
    );
    m.set_theme("light");
    insta::assert_snapshot!(
        "confirm_rule_impact_light_styled_80x24",
        render_styled(&m, 80, 24)
    );
}
```

Append to `crates/illumio-tui/tests/tui_runtime.rs`:

```rust
#[tokio::test]
async fn w_confirm_shows_the_impact_after_i_and_y_still_posts_once() {
    use illumio_tui::tui::impact::confirm_line;
    use illumio_tui::tui::model::Overlay;
    use illumio_tui::tui::test_support::{fixture_index, model_with_flows};
    let server = MockServer::start().await;
    mount_rule_sets(&server, 2).await; // the name lookup + the reload
    mount_workloads(&server, 1).await;
    let body = json!({
        "enabled": true,
        "providers": [{"label": {"href": "/orgs/5/labels/1"}}],
        "consumers": [{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}],
        "ingress_services": [{"port": 8443, "proto": 6}],
        "unscoped_consumers": true,
        "description": "allow 10.1.4.9 to host-1.example on 8443/tcp",
        "resolve_labels_as": {"providers": ["workloads"], "consumers": ["workloads"]}
    });
    let mut created = body.clone();
    created["href"] = json!("/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9");
    Mock::given(method("POST"))
        .and(path(
            "/api/v2/orgs/5/sec_policy/draft/rule_sets/100/sec_rules",
        ))
        .and(body_json(body))
        .respond_with(ResponseTemplate::new(201).set_body_json(created))
        .expect(1)
        .mount(&server)
        .await;
    let audit = MemoryAudit::new();
    // The fixture flows on screen and the draft rulesets loaded: the w form
    // pre-fills Shop-Prod, and the confirm finds its scope.
    let mut m = model_with_flows();
    m.writes = WritesState::on();
    let raw = serde_json::from_str(include_str!("../../../fixtures/rule_sets.json")).unwrap();
    m.policy.set_rule_sets(raw, &fixture_index(), None);
    let (mut rt, _tx) = Runtime::with_writer(pce_for(&server), m, tui_guard(audit.clone()));
    let keys = vec![
        (100, ch('j')), // the potentially blocked flow
        (50, ch('w')),
        (50, KeyCode::Enter),
        (300, ch('i')),
        (300, ch('y')),
        (400, KeyCode::F(12)),
    ];
    let mut lines: Vec<String> = Vec::new();
    let model = run_loop(&mut rt, Box::pin(press(keys)), &mut |m| {
        if let Overlay::ConfirmWrite { plan, .. } = &m.overlay
            && let Some(l) = confirm_line(m, plan)
            && lines.last() != Some(&l)
        {
            lines.push(l);
        }
    })
    .await;
    assert_eq!(
        lines.first().map(String::as_str),
        Some("impact: press i to load workloads (1 request)")
    );
    assert_eq!(
        lines.last().map(String::as_str),
        Some("impact: 1 provider · 0 consumers")
    );
    assert_eq!(
        model.status.text,
        "created /orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9 (pending)"
    );
    assert_eq!(
        model.stats.calls, 4,
        "the rulesets GET for the name, one workloads read, the POST, one rulesets reload"
    );
    assert_eq!(writes_seen(&server).await, vec!["POST"]);
    assert_eq!(audit.entries().len(), 1);
    assert_eq!(model.busy, 0);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --lib tui::impact:: tui::update::tests::i_in_a_rule_confirm`
Expected: FAIL to compile (`cannot find function confirm_line`, `confirm_rows`).

- [ ] **Step 3: Implement**

`crates/illumio-tui/src/tui/impact.rs`: extend the module doc's first sentence to `… the rule detail's impact rows and the confirm overlay's impact line.`, add to the imports

```rust
use pce::WriteMethod;
use pce::impact::{is_rule_href, rule_set_href_of};
use pce::write::{Plan, WriteRequest};
```

and add after `rule_detail_rows`:

```rust
/// A rule or deny-rule create or update: the request the confirm overlay
/// adds an impact line to.
fn rule_request(plan: &Plan) -> Option<&WriteRequest> {
    match plan {
        Plan::Send {
            request,
            kind: pce::write::ObjectKind::Rule | pce::write::ObjectKind::DenyRule,
        } if request.method != WriteMethod::Delete => Some(request),
        _ => None,
    }
}

/// Rows the confirm overlay adds for `plan`: one for a rule or deny-rule
/// create or update, none otherwise.
pub fn confirm_rows(plan: &Plan) -> u16 {
    u16::from(rule_request(plan).is_some())
}

/// The confirm overlay's impact line for a rule plan: the planned body's
/// providers and consumers (an update's untouched sides from the loaded
/// rule) in the target ruleset's scopes over the cached workloads; or
/// what it takes to get there. `None` for any other plan.
pub fn confirm_line(model: &Model, plan: &Plan) -> Option<String> {
    let request = rule_request(plan)?;
    let (workloads, truncated) = match source(&model.workloads) {
        Source::Ready {
            workloads,
            truncated,
        } => (workloads, truncated),
        waiting => return Some(format!("impact: {}", waiting_text(waiting))),
    };
    let p = &model.policy;
    let scopes = rule_set_href_of(&request.path).and_then(|rs| scopes_of(p, rs));
    let (Some(idx), Some(scopes)) = (model.index(), scopes) else {
        return Some("impact: ruleset not loaded; open Policy (2) first".to_string());
    };
    let base = if is_rule_href(&request.path) {
        loaded_rule(p, &request.path).map(|(_, a)| a)
    } else {
        None
    };
    let actors = RuleActors::planned(request.body.as_ref(), base.as_ref());
    Some(format!(
        "impact: {}",
        compute(scopes, &actors, idx, workloads, truncated).summary()
    ))
}
```

`crates/illumio-tui/src/tui/ui/widgets/overlay.rs`: the layout takes the extra rows:

```rust
/// The confirm box in `area`, the request body wrapped to its inner width
/// (the box is the dry run: nothing may be clipped), and how many body rows
/// fit. `extra` rows sit above the body (the impact line of a rule plan).
/// Shared by the renderer and the scroll keys so they cannot drift.
fn confirm_write_layout(req: &WriteRequest, area: Rect, extra: u16) -> (Rect, Vec<String>, usize) {
    let width = usize::from(CONFIRM_WIDTH.min(area.width).saturating_sub(2));
    let body: Vec<String> = crate::tui::edit::request_body_lines(req)
        .iter()
        .flat_map(|l| wrap_chars(&plain(l), width))
        .collect();
    let chrome = CONFIRM_CHROME + extra;
    let height = u16::try_from(body.len())
        .unwrap_or(u16::MAX)
        .saturating_add(chrome);
    let r = centered(area, CONFIRM_WIDTH, height);
    let rows = usize::from(r.height.saturating_sub(chrome));
    (r, body, rows)
}

/// The first wrapped body line shown when scrolled to the end.
pub fn confirm_write_last_scroll(req: &WriteRequest, area: Rect, extra: u16) -> usize {
    let (_, body, rows) = confirm_write_layout(req, area, extra);
    body.len().saturating_sub(rows)
}
```

and in `render_confirm_write` replace `let (r, body, rows) = confirm_write_layout(req, area);` and the `let mut lines = vec![…];` statement with:

```rust
    let extra = crate::tui::impact::confirm_rows(plan);
    let (r, body, rows) = confirm_write_layout(req, area, extra);
```

```rust
    let mut lines = vec![Line::from(Span::styled(
        plain(&req.summary),
        Style::new().fg(t.warn),
    ))];
    if let Some(impact) = crate::tui::impact::confirm_line(model, plan) {
        lines.push(Line::from(Span::styled(impact, Style::new().fg(t.accent))));
    }
    lines.push(Line::from(Span::styled(
        format!("{} {}", req.method.as_str(), plain(&req.path)),
        t.header_style(),
    )));
    lines.push(Line::from(""));
```

(the `f.render_widget(Clear, r);`, `first`, `meta` and the body `lines.extend(…)` stay as they are).

`crates/illumio-tui/src/tui/update.rs`, `Overlay::ConfirmWrite { plan, kind, scroll }` arm: the first lines become

```rust
            let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
            let extra = crate::tui::impact::confirm_rows(&plan);
            let last = plan
                .request()
                .map_or(0, |r| confirm_write_last_scroll(r, area, extra));
```

and before its `_ => Vec::new(),` add

```rust
                KeyCode::Char('i') if extra > 0 => impact_key(model),
```

- [ ] **Step 4: Run the unit tests**

Run: `cargo fmt --all && cargo test -p illumio-tui --lib`
Expected: PASS (`the_confirm_line_reads_the_planned_body`, `i_in_a_rule_confirm_loads_workloads_and_keeps_the_request_on_screen`, and the existing confirm scroll tests unchanged: a label plan adds no row).

- [ ] **Step 5: Create and read the snapshots, run everything**

Run: `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots && git status --short crates/illumio-tui/tests/snapshots`
Expected: exactly two new files, `crates/illumio-tui/tests/snapshots/tui_snapshots__confirm_rule_impact_80x24.snap` and `crates/illumio-tui/tests/snapshots/tui_snapshots__confirm_rule_impact_light_styled_80x24.snap`; no existing snapshot changes (label, IP list and provision confirms add no row). Read both: the summary `create rule in ruleset 100`, then `impact: 1 provider · 0 consumers`, then `POST /orgs/5/sec_policy/draft/rule_sets/100/sec_rules`, the body below, `j/k scroll · y send · n cancel` when it does not fit.

Run: `cargo test -p illumio-tui`
Expected: PASS, including `w_confirm_shows_the_impact_after_i_and_y_still_posts_once`.

- [ ] **Step 6: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p illumio-tui
git add crates/illumio-tui/src/tui/impact.rs crates/illumio-tui/src/tui/ui/widgets/overlay.rs crates/illumio-tui/src/tui/update.rs crates/illumio-tui/src/tui/test_support.rs crates/illumio-tui/tests/tui_snapshots.rs crates/illumio-tui/tests/tui_runtime.rs crates/illumio-tui/tests/snapshots/tui_snapshots__confirm_rule_impact_80x24.snap crates/illumio-tui/tests/snapshots/tui_snapshots__confirm_rule_impact_light_styled_80x24.snap
git commit -m "feat(tui): the rule confirm shows the planned rule's impact; i loads it

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 7: CLI `--impact` for existing rules

**Files:**
- Modify: `crates/illumio-tui/src/cli/policy.rs` (imports lines 4-18; `RulesCmd` doc ~line 109; `RulesAction` ~line 120; new `RuleImpactArgs` after `RuleDeleteArgs` ~line 232; `RulesArgs` ~line 314; `RulesOutput` ~line 325 and its `TableRows` ~line 363; `run` ~line 422; `rules` ~line 465; new `kind_text`, `deny_rules_of`, `impact_workloads`, `rule_impact_one`)
- Test: `crates/illumio-tui/tests/cli.rs` (`every_flag_has_help_text` ~line 46; append)

**Interfaces:**
- Consumes: `Impact`, `ImpactInputs`, `RuleActors`, `rule_impact`, `rule_set_href_of`, `Impact::{pc, summary}`, `IpListRef::line` (Task 2); `Pce::{name_index, rule_sets, deny_rules, workloads}` (`pce/src/lib.rs` ~lines 189, 239, 253, 205), `Cached` (~line 46), `WorkloadFilter::default` (`models/workloads.rs`); `checked_nested_href` (`pce/src/write/mod.rs` ~line 249); `find_ruleset`, `names`, `scope` (`cli/policy.rs` ~lines 458, 359, 331); `Context::emit`, `cache_label` (`cli/context.rs` ~lines 74, 113); `CliError::Pce(PceError::NotFound { path })` (exit 4); test helpers `cmd`, `stdout_json`, `mount_policy` (`tests/cli.rs` ~lines 11, 29, 498).
- Produces:

```rust
// cli/policy.rs
pub enum RulesAction { Add(RuleAddArgs), Update(RuleUpdateArgs), Delete(RuleDeleteArgs), Impact(RuleImpactArgs) }
#[derive(Args, Debug)] pub struct RuleImpactArgs { pub rule: String }
#[derive(Args, Debug, Clone)] pub struct RulesArgs { pub ruleset: Option<String>, pub active: bool, pub draft: bool, pub impact: bool }
#[derive(Debug, Serialize)] pub struct RuleOut { #[serde(flatten)] pub rule: RuleView, #[serde(skip_serializing_if = "Option::is_none")] pub impact: Option<Impact> }
#[derive(Debug, Serialize)] pub struct RulesOutput { pub ruleset: RuleSetView, pub rules: Vec<RuleOut>, #[serde(skip)] pub with_impact: bool }
async fn deny_rules_of(ctx: &Context, rs: &RuleSet) -> Result<Vec<DenyRule>, CliError>;
async fn impact_workloads(ctx: &Context) -> Result<Cached<Vec<Workload>>, CliError>;
async fn rule_impact_one(ctx: &Context, a: &RuleImpactArgs) -> Result<(), CliError>;
```

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/tests/cli.rs`, `every_flag_has_help_text`, add `vec!["policy", "rules", "impact", "--help"],` after the `policy rules delete` entry. Append:

```rust
/// The fixture workloads behind `--impact`, read `n` times (once per command
/// that needs them); `total` makes the PCE report more than it returned.
async fn mount_workloads(server: &MockServer, n: u64, total: Option<u64>) {
    use wiremock::matchers::query_param;
    let mut response = ResponseTemplate::new(200)
        .set_body_string(include_str!("../../../fixtures/workloads.json"))
        .insert_header("Content-Type", "application/json");
    if let Some(t) = total {
        response = response.insert_header("X-Total-Count", t.to_string());
    }
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(query_param("max_results", "10000"))
        .respond_with(response)
        .expect(n)
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn policy_rules_impact_reads_the_workloads_once_per_command() {
    let server = MockServer::start().await;
    mount_policy(&server).await;
    mount_workloads(&server, 2, None).await; // the JSON and the table command
    let out = stdout_json(
        cmd(&server)
            .args(["policy", "rules", "shop-prod", "--impact"])
            .assert()
            .success(),
    );
    let rules = out["data"]["rules"].as_array().unwrap();
    let pc = |i: usize| {
        (
            rules[i]["impact"]["providers"].clone(),
            rules[i]["impact"]["consumers"].clone(),
        )
    };
    assert_eq!(pc(0), (json!(1), json!(1)), "db ← web");
    assert_eq!(pc(1), (json!(1), json!(0)), "an IP list consumer matches no workload");
    assert_eq!(pc(2), (json!(2), json!(0)));
    assert_eq!(pc(3), (json!(1), json!(2)));
    assert_eq!(
        rules[0]["impact"]["per_scope"],
        json!([{"scope": ["app=shop", "env=prod"], "providers": 1, "consumers": 1}])
    );
    assert_eq!(
        rules[1]["impact"]["ip_lists"],
        json!([{
            "name": "Corp", "href": "/orgs/5/sec_policy/draft/ip_lists/21",
            "size": {"v4": "65536", "v6": "0", "unbounded": null, "ranges": 1, "fqdns": 1},
            "side": "consumer"
        }])
    );
    assert_eq!(
        rules[2]["impact"]["ip_lists"][0]["size"]["unbounded"],
        "0.0.0.0/0"
    );
    assert_eq!(rules[0]["impact"]["unsupported"], json!([]));
    assert_eq!(rules[0]["impact"]["truncated"], false);
    assert_eq!(
        rules[1]["consumers"][0]["name"], "ipl:Corp",
        "the rule's own fields stay"
    );
    assert_eq!(out["meta"]["truncated"], false);
    assert_eq!(
        out["meta"]["api_calls"], 6,
        "four object lists + rulesets + one workloads read"
    );
    cmd(&server)
        .args(["policy", "rules", "shop-prod", "--impact", "--table"])
        .assert()
        .success()
        .stdout(predicate::str::contains("P/C").and(predicate::str::contains("1/2")));
    let out = stdout_json(
        cmd(&server)
            .args(["policy", "rules", "shop-prod"])
            .assert()
            .success(),
    );
    assert!(
        out["data"]["rules"][0].get("impact").is_none(),
        "no --impact, no field and no workloads read"
    );
    assert_eq!(out["meta"]["api_calls"], 5);
}

#[tokio::test(flavor = "multi_thread")]
async fn policy_rules_impact_reports_one_rule_by_href() {
    let server = MockServer::start().await;
    // A ruleset href where a rule href belongs: exit 2 before any request.
    let a = cmd(&server)
        .args(["policy", "rules", "impact", "/orgs/5/sec_policy/draft/rule_sets/100"])
        .assert()
        .code(2);
    assert!(
        String::from_utf8_lossy(&a.get_output().stderr).contains("sec_rules"),
        "names the expected shape"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    mount_policy(&server).await;
    mount_workloads(&server, 3, Some(40)).await; // the three rules found below
    let out = stdout_json(
        cmd(&server)
            .args([
                "policy",
                "rules",
                "impact",
                "/orgs/5/sec_policy/active/rule_sets/100/sec_rules/2",
            ])
            .assert()
            .success(),
    );
    assert_eq!(
        out["data"]["href"],
        "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2"
    );
    assert_eq!(out["data"]["impact"]["providers"], 1);
    assert_eq!(out["data"]["impact"]["ip_lists"][0]["name"], "Corp");
    assert_eq!(out["data"]["impact"]["truncated"], true);
    assert_eq!(out["meta"]["truncated"], true, "the workload read was cut short");
    assert_eq!(out["meta"]["api_calls"], 6);
    let out = stdout_json(
        cmd(&server)
            .args([
                "policy",
                "rules",
                "impact",
                "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8",
            ])
            .assert()
            .success(),
    );
    assert_eq!(out["data"]["kind"], "override_deny");
    assert_eq!(
        (
            out["data"]["impact"]["providers"].clone(),
            out["data"]["impact"]["consumers"].clone()
        ),
        (json!(1), json!(2))
    );
    let a = cmd(&server)
        .args([
            "policy",
            "rules",
            "impact",
            "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/99",
        ])
        .assert()
        .code(4);
    let err: Value = serde_json::from_slice(&a.get_output().stderr).unwrap();
    assert_eq!(err["error"]["kind"], "not_found");
    cmd(&server)
        .args([
            "policy",
            "rules",
            "impact",
            "/orgs/5/sec_policy/draft/rule_sets/101/sec_rules/3",
            "--table",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("impact")
                .and(predicate::str::contains("scope All"))
                .and(predicate::str::contains("4/2")),
        );
}
```


- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --test cli policy_rules_impact`
Expected: FAIL (`unexpected argument '--impact'`, exit 2 where success is expected; `impact` is taken as a ruleset name).

- [ ] **Step 3: Implement** (`crates/illumio-tui/src/cli/policy.rs`)

Imports become:

```rust
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::TableRows;
use super::write::{
    self, ActorToken, Lists, ServiceToken, WriteFlags, is_href, parse_actors, parse_scopes,
    parse_services, resolve_actors, resolve_scopes, resolve_services, run_plan,
};
use clap::{Args, Subcommand};
use pce::impact::{Impact, ImpactInputs, RuleActors, rule_impact, rule_set_href_of};
use pce::models::{
    DenyRule, PendingChange, PolicyScope, ResolveLabelsAs, RuleSet, Workload, WorkloadFilter,
};
use pce::write::{
    ActorRef, DenyRuleDraft, DenyRulePatch, IfMissing, ProvisionRequest, ProvisionSubset,
    RuleDraft, RulePatch, RuleSetDraft, RuleSetPatch, ServiceRef, WriteError, checked_nested_href,
};
use pce::{Cached, PceError, RuleKind, RuleSetView, RuleView};
use serde::Serialize;
```

`RulesCmd`'s doc becomes `/// \`policy rules <ruleset>\` lists; \`add\`, \`update\` and \`delete\` write; \`impact\` reports one rule.`; `RulesAction` gains

```rust
    /// Report one rule's impact (workloads matched as providers and consumers), addressed by href
    Impact(RuleImpactArgs),
```

after `RuleDeleteArgs` add

```rust
#[derive(Args, Debug)]
pub struct RuleImpactArgs {
    /// Rule or deny rule href: /orgs/N/sec_policy/draft/rule_sets/ID/sec_rules/ID (or deny_rules/ID)
    pub rule: String,
}
```

`RulesArgs` gains, after `draft`:

```rust
    /// Add each rule's impact: workloads matched as providers and consumers (one workloads read, cached)
    #[arg(long)]
    pub impact: bool,
```

Replace `RulesOutput` and its `TableRows` with:

```rust
/// One rule as `policy rules` prints it; `impact` with `--impact`.
#[derive(Debug, Serialize)]
pub struct RuleOut {
    #[serde(flatten)]
    pub rule: RuleView,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impact: Option<Impact>,
}

#[derive(Debug, Serialize)]
pub struct RulesOutput {
    pub ruleset: RuleSetView,
    pub rules: Vec<RuleOut>,
    /// `--impact` was given: the table gains a `P/C` column.
    #[serde(skip)]
    pub with_impact: bool,
}
```

```rust
/// `allow`, `deny`, `override_deny`, as in JSON.
fn kind_text(kind: RuleKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

impl TableRows for RulesOutput {
    fn headers(&self) -> Vec<&'static str> {
        let mut h = vec!["kind", "enabled", "consumers", "providers", "services"];
        if self.with_impact {
            h.push("P/C");
        }
        h.extend(["description", "pending"]);
        h
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.rules
            .iter()
            .map(|o| {
                let r = &o.rule;
                let mut row = vec![
                    kind_text(r.kind),
                    r.enabled.to_string(),
                    names(r.consumers.iter().map(|a| a.name.as_str())),
                    names(r.providers.iter().map(|a| a.name.as_str())),
                    names(r.services.iter().map(|s| s.name.as_str())),
                ];
                if self.with_impact {
                    row.push(
                        o.impact
                            .as_ref()
                            .map_or_else(|| "-".to_string(), Impact::pc),
                    );
                }
                row.extend([
                    r.description.clone().unwrap_or_default(),
                    r.update_type.clone().unwrap_or_default(),
                ]);
                row
            })
            .collect()
    }
}

impl TableRows for RuleOut {
    fn headers(&self) -> Vec<&'static str> {
        vec!["field", "value"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let r = &self.rule;
        let mut rows = vec![
            vec!["href".to_string(), r.href.clone()],
            vec!["kind".to_string(), kind_text(r.kind)],
            vec![
                "providers".to_string(),
                names(r.providers.iter().map(|a| a.name.as_str())),
            ],
            vec![
                "consumers".to_string(),
                names(r.consumers.iter().map(|a| a.name.as_str())),
            ],
            vec![
                "services".to_string(),
                names(r.services.iter().map(|s| s.name.as_str())),
            ],
        ];
        if let Some(i) = &self.impact {
            rows.push(vec!["impact".to_string(), i.summary()]);
            for s in &i.per_scope {
                let scope = if s.scope.is_empty() {
                    "All".to_string()
                } else {
                    s.scope.join(" & ")
                };
                rows.push(vec![
                    format!("scope {scope}"),
                    format!("{}/{}", s.providers, s.consumers),
                ]);
            }
            rows.extend(i.ip_lists.iter().map(|l| vec!["ip list".to_string(), l.line()]));
            rows.extend(
                i.unsupported
                    .iter()
                    .map(|u| vec!["unsupported".to_string(), u.clone()]),
            );
        }
        rows
    }
}
```

In `run`, the `PolicyCmd::Rules(c)` match gains `Some(RulesAction::Impact(a)) => rule_impact_one(ctx, a).await,`. Replace `rules` with the version below and add the helpers after it:

```rust
async fn rules(ctx: &Context, args: &RulesArgs) -> Result<(), CliError> {
    let Some(needle) = args.ruleset.as_deref() else {
        return Err(CliError::Args(
            "policy rules needs a ruleset name or href (or one of add, update, delete, impact)"
                .into(),
        ));
    };
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
    let c = ctx
        .pce
        .rule_sets(scope(args.active), ctx.cache_mode)
        .await?;
    let rs = find_ruleset(&c.value, needle).ok_or_else(|| {
        let names: Vec<&str> = c.value.iter().map(|r| r.name.as_str()).collect();
        CliError::Args(format!(
            "ruleset {needle:?} not found; known: {}",
            names.join(", ")
        ))
    })?;
    let deny = deny_rules_of(ctx, rs).await?;
    let views: Vec<RuleView> = rs
        .rules
        .iter()
        .map(|r| RuleView::from_rule(r, &idx))
        .chain(deny.iter().map(|d| RuleView::from_deny(d, &idx)))
        .collect();
    let mut truncated = c.truncated;
    let impacts: Vec<Option<Impact>> = if args.impact {
        let w = impact_workloads(ctx).await?;
        truncated |= w.truncated;
        let inputs = ImpactInputs {
            workloads: &w.value,
            index: idx.as_ref(),
            truncated: w.truncated,
        };
        rs.rules
            .iter()
            .map(RuleActors::of_rule)
            .chain(deny.iter().map(RuleActors::of_deny))
            .map(|a| {
                Some(rule_impact(
                    &rs.scopes,
                    &a.providers,
                    &a.consumers,
                    a.unscoped_consumers,
                    &inputs,
                ))
            })
            .collect()
    } else {
        vec![None; views.len()]
    };
    let out = RulesOutput {
        ruleset: RuleSetView::from_rule_set(rs, &idx),
        rules: views
            .into_iter()
            .zip(impacts)
            .map(|(rule, impact)| RuleOut { rule, impact })
            .collect(),
        with_impact: args.impact,
    };
    ctx.emit(&out, cache_label(c.hit, ctx.cache_mode), truncated, None)
}

/// A ruleset's deny rules: inline, or from the per-ruleset endpoint when the
/// inline list is empty (older PCEs; others have no such endpoint, a 404
/// there means none).
async fn deny_rules_of(ctx: &Context, rs: &RuleSet) -> Result<Vec<DenyRule>, CliError> {
    if !rs.deny_rules.is_empty() {
        return Ok(rs.deny_rules.clone());
    }
    match ctx.pce.deny_rules(&rs.href, ctx.cache_mode).await {
        Ok(d) => Ok(d.value),
        Err(PceError::NotFound { .. }) => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

/// The workload list behind `--impact`: one read, through the cache.
async fn impact_workloads(ctx: &Context) -> Result<Cached<Vec<Workload>>, CliError> {
    Ok(ctx
        .pce
        .workloads(&WorkloadFilter::default(), ctx.cache_mode)
        .await?)
}

/// `policy rules impact <href>`: one rule or deny rule of the draft
/// rulesets (an active href folds to draft) with its impact; exit 2 for an
/// href of the wrong shape, 4 when no such rule is loaded.
async fn rule_impact_one(ctx: &Context, a: &RuleImpactArgs) -> Result<(), CliError> {
    let child = if a.rule.contains("/deny_rules/") {
        "deny_rules"
    } else {
        "sec_rules"
    };
    let href = checked_nested_href(&a.rule, "rule_sets", child, ctx.config.org_id)?;
    let not_found = || CliError::Pce(PceError::NotFound { path: href.clone() });
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
    let c = ctx
        .pce
        .rule_sets(PolicyScope::Draft, ctx.cache_mode)
        .await?;
    let rs = rule_set_href_of(&href)
        .and_then(|h| c.value.iter().find(|r| r.href == h))
        .ok_or_else(not_found)?;
    let (rule, actors) = if child == "sec_rules" {
        let r = rs
            .rules
            .iter()
            .find(|r| r.href == href)
            .ok_or_else(not_found)?;
        (RuleView::from_rule(r, &idx), RuleActors::of_rule(r))
    } else {
        let deny = deny_rules_of(ctx, rs).await?;
        let d = deny
            .iter()
            .find(|d| d.href == href)
            .ok_or_else(not_found)?;
        (RuleView::from_deny(d, &idx), RuleActors::of_deny(d))
    };
    let w = impact_workloads(ctx).await?;
    let inputs = ImpactInputs {
        workloads: &w.value,
        index: idx.as_ref(),
        truncated: w.truncated,
    };
    let impact = rule_impact(
        &rs.scopes,
        &actors.providers,
        &actors.consumers,
        actors.unscoped_consumers,
        &inputs,
    );
    let out = RuleOut {
        rule,
        impact: Some(impact),
    };
    ctx.emit(
        &out,
        cache_label(c.hit, ctx.cache_mode),
        c.truncated || w.truncated,
        None,
    )
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo fmt --all && cargo test -p illumio-tui --test cli`
Expected: PASS (the two new tests, `every_flag_has_help_text` with `policy rules impact --help`, and `policy_rules_include_deny_rules_and_tolerate_missing_endpoint` unchanged: still 5 and 6 calls).

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p illumio-tui
git add crates/illumio-tui/src/cli/policy.rs crates/illumio-tui/tests/cli.rs
git commit -m "feat(cli): policy rules --impact and policy rules impact <href>

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 8: CLI `--impact` on rule writes

**Files:**
- Modify: `crates/illumio-tui/src/cli/write.rs` (imports lines 5-19; `WriteOutput` ~line 75 and its `TableRows` ~line 134; `run_plan`, `run_provision_plan`, `send` ~lines 168-255; new `Extra`, `run_rule_plan`, `impact_rows`)
- Modify: `crates/illumio-tui/src/cli/context.rs` (`emit_write` ~line 94; new `emit_write_with`)
- Modify: `crates/illumio-tui/src/cli/policy.rs` (imports; `RuleAddArgs` ~176, `RuleUpdateArgs` ~201, `DenyRuleAddArgs` ~234, `DenyRuleUpdateArgs` ~253 gain `impact`; `rule_add` ~754, `rule_update` ~772, `deny_rule_add` ~811, `deny_rule_update` ~831; new `planned_impact`)
- Test: `crates/illumio-tui/tests/cli_writes.rs` (`ALL_WRITES` ~lines 134-241; append to `mod policy`)

**Interfaces:**
- Consumes: `RuleActors::planned`, `rule_set_href_of`, `is_rule_href`, `rule_impact`, `Impact::{summary}`, `IpListRef::line` (Task 2); `deny_rules_of`, `impact_workloads` (Task 7); `write::prepare`, `Plan::request` (`pce/src/write/mod.rs` ~line 117); `Written`, `WriteRequest`, `provision_counts`, `WriteMeta`; test helpers `writer`, `cmd`, `stdout_json`, `stderr_json`, `audit_lines`, `count` (`tests/cli_writes.rs` ~lines 12-95) and, in `mod policy`, `RS_100`, `RULE_2`, `DENY_8`, `mount_lists`, `mount_get`, `gets`, `fixture_rule_sets`, `fixture_workloads` (~lines 1474-1540).
- Produces:

```rust
// cli/write.rs
pub enum WriteOutput {
    /* DryRun, Written, Skipped, ProvisionDryRun, Provisioned as before */
    DryRunImpact { #[serde(flatten)] request: WriteRequest, impact: Impact },
    WrittenImpact { #[serde(flatten)] written: Written, impact: Impact },
}
pub async fn run_rule_plan(ctx: &Context, flags: &WriteFlags, guard: &WriteGuard, plan: Plan, impact: Option<(Impact, bool)>) -> Result<(), CliError>;
// cli/context.rs
impl Context { pub fn emit_write_with(&self, data: &WriteOutput, write: Option<WriteMeta>, truncated: bool) -> Result<(), CliError>; }
// cli/policy.rs
RuleAddArgs { .., pub impact: bool } // and RuleUpdateArgs, DenyRuleAddArgs, DenyRuleUpdateArgs
async fn planned_impact(ctx: &Context, plan: &Plan) -> Result<Option<(Impact, bool)>, CliError>;
```

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/tests/cli_writes.rs` the guard table grows to 27: change `const ALL_WRITES: [&[&str]; 23] = [` to `const ALL_WRITES: [&[&str]; 27] = [` and add before `&["policy", "provision", "--all"],`:

```rust
    &[
        "policy",
        "rules",
        "add",
        "/orgs/5/sec_policy/draft/rule_sets/1",
        "--provider",
        "all",
        "--consumer",
        "all",
        "--service",
        "443/tcp",
        "--impact",
    ],
    &[
        "policy",
        "rules",
        "update",
        "/orgs/5/sec_policy/draft/rule_sets/1/sec_rules/2",
        "--description",
        "d",
        "--impact",
    ],
    &[
        "policy",
        "deny-rules",
        "add",
        "/orgs/5/sec_policy/draft/rule_sets/1",
        "--provider",
        "all",
        "--consumer",
        "all",
        "--service",
        "22/tcp",
        "--impact",
    ],
    &[
        "policy",
        "deny-rules",
        "update",
        "/orgs/5/sec_policy/draft/rule_sets/1/deny_rules/3",
        "--override",
        "--impact",
    ],
```

Append to `mod policy`:

```rust
    const SHOP_RULE: [&str; 10] = [
        "policy",
        "rules",
        "add",
        "Shop-Prod",
        "--provider",
        "role=web",
        "--consumer",
        "iplist:Corp",
        "--service",
        "443/tcp",
    ];

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_add_dry_run_impact_reads_workloads_once_and_sends_nothing() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        mount_get(&server, "/orgs/5/workloads", fixture_workloads()).await;
        let out = stdout_json(
            writer(&server, state.path())
                .args(SHOP_RULE)
                .args(["--dry-run", "--impact"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["method"], "POST");
        assert_eq!(out["data"]["path"], format!("{RS_100}/sec_rules"));
        assert_eq!(
            out["data"]["body"]["providers"],
            json!([{"label": {"href": "/orgs/5/labels/1"}}])
        );
        let impact = &out["data"]["impact"];
        assert_eq!(
            (impact["providers"].clone(), impact["consumers"].clone()),
            (json!(1), json!(0))
        );
        assert_eq!(
            impact["per_scope"],
            json!([{"scope": ["app=shop", "env=prod"], "providers": 1, "consumers": 0}])
        );
        assert_eq!(impact["ip_lists"][0]["name"], "Corp");
        assert_eq!(impact["ip_lists"][0]["side"], "consumer");
        assert_eq!(out["meta"]["truncated"], false);
        assert_eq!(gets(&server, "/orgs/5/workloads").await, 1);
        assert_eq!(
            gets(&server, "/orgs/5/sec_policy/draft/rule_sets").await,
            1,
            "the name and the scopes share one read"
        );
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method.as_str() == "GET"),
            "a dry run sends no write"
        );
        assert!(audit_lines(state.path()).is_empty());
        // Without --impact the dry run keeps its shape and reads no workloads.
        let out = stdout_json(
            writer(&server, state.path())
                .args(SHOP_RULE)
                .arg("--dry-run")
                .assert()
                .success(),
        );
        assert!(out["data"].get("impact").is_none());
        assert_eq!(gets(&server, "/orgs/5/workloads").await, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_add_yes_impact_is_computed_before_the_post() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        mount_get(&server, "/orgs/5/workloads", fixture_workloads()).await;
        Mock::given(method("POST"))
            .and(path(format!("/api/v2{RS_100}/sec_rules")))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "href": format!("{RS_100}/sec_rules/9"), "enabled": true,
                "providers": [{"label": {"href": "/orgs/5/labels/1"}}],
                "consumers": [{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}],
                "ingress_services": [{"port": 443, "proto": 6}]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args(SHOP_RULE)
                .args(["--yes", "--impact"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["href"], format!("{RS_100}/sec_rules/9"));
        assert_eq!(out["data"]["pending"], true);
        assert_eq!(out["data"]["impact"]["providers"], 1);
        assert_eq!(out["meta"]["write"]["method"], "POST");
        let requests = server.received_requests().await.unwrap();
        let at = |m: &str, p: &str| {
            requests
                .iter()
                .position(|r| r.method.as_str() == m && r.url.path() == p)
                .unwrap()
        };
        assert!(
            at("GET", "/api/v2/orgs/5/workloads") < at("POST", &format!("/api/v2{RS_100}/sec_rules")),
            "the impact is computed before the send"
        );
        assert_eq!(audit_lines(state.path()).len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rule_and_deny_rule_updates_with_impact_merge_the_loaded_rule() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        mount_get(&server, "/orgs/5/workloads", fixture_workloads()).await;
        mount_get(&server, RULE_2, fixture_rule_sets()[0]["rules"][1].clone()).await;
        let deny: Value =
            serde_json::from_str(include_str!("../../../fixtures/deny_rules.json")).unwrap();
        mount_get(&server, DENY_8, deny[1].clone()).await;
        let pc = |out: &Value| {
            (
                out["data"]["impact"]["providers"].clone(),
                out["data"]["impact"]["consumers"].clone(),
            )
        };
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "rules",
                    "update",
                    RULE_2,
                    "--add-consumer",
                    "all",
                    "--dry-run",
                    "--impact",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["method"], "PUT");
        assert_eq!(
            pc(&out),
            (json!(1), json!(4)),
            "providers from the loaded rule; unscoped, so every workload consumes"
        );
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "deny-rules",
                    "update",
                    DENY_8,
                    "--add-provider",
                    "role=web",
                    "--dry-run",
                    "--impact",
                ])
                .assert()
                .success(),
        );
        assert_eq!(
            pc(&out),
            (json!(2), json!(2)),
            "role=db or role=web inside the scope; deny consumers stay scoped"
        );
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "deny-rules",
                    "add",
                    "Shop-Prod",
                    "--provider",
                    "all",
                    "--consumer",
                    "role=web",
                    "--service",
                    "22/tcp",
                    "--dry-run",
                    "--impact",
                ])
                .assert()
                .success(),
        );
        assert_eq!(pc(&out), (json!(2), json!(1)));
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method.as_str() == "GET")
        );
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --test cli_writes policy::`
Expected: FAIL (`unexpected argument '--impact'`: exit 2 where success, and in `every_write_command_is_guarded_and_needs_yes` exit 2 where 6 is expected for the four new entries).

- [ ] **Step 3: Implement**

`crates/illumio-tui/src/cli/context.rs`: `emit_write` delegates to a new method:

```rust
    /// Envelope for a write command: `cache` is always `none`.
    pub fn emit_write(
        &self,
        data: &super::write::WriteOutput,
        write: Option<WriteMeta>,
    ) -> Result<(), CliError> {
        self.emit_write_with(data, write, false)
    }

    /// [`Context::emit_write`] with `meta.truncated` from a read the output
    /// depends on (the workload list behind `--impact`).
    pub fn emit_write_with(
        &self,
        data: &super::write::WriteOutput,
        write: Option<WriteMeta>,
        truncated: bool,
    ) -> Result<(), CliError> {
        let mut meta = self.meta("none", truncated, None);
        meta.write = write;
        let mut out = std::io::stdout().lock();
        match self.format {
            OutputFormat::Json => write_json(&mut out, data, &meta)?,
            OutputFormat::Table => {
                write_table(&mut out, data)?;
                eprintln!("{}", meta_line(&meta));
            }
        }
        Ok(())
    }
```

`crates/illumio-tui/src/cli/write.rs`: add `use pce::Impact;` to the imports; add to `WriteOutput` after `Provisioned`:

```rust
    /// A rule or deny-rule dry run with `--impact`: the request and the
    /// impact of what it would send.
    DryRunImpact {
        #[serde(flatten)]
        request: WriteRequest,
        impact: Impact,
    },
    /// A rule or deny-rule write sent with `--impact`: the result and the
    /// impact computed before the send.
    WrittenImpact {
        #[serde(flatten)]
        written: Written,
        impact: Impact,
    },
```

after `counts_text` add

```rust
/// `impact` and one `ip list` row per IP-list actor.
fn impact_rows(i: &Impact) -> Vec<Vec<String>> {
    let mut rows = vec![row("impact", i.summary())];
    rows.extend(i.ip_lists.iter().map(|l| row("ip list", l.line())));
    rows
}
```

and to `TableRows for WriteOutput`'s match:

```rust
            WriteOutput::DryRunImpact { request, impact } => {
                let mut rows = request_rows(request);
                rows.extend(impact_rows(impact));
                rows
            }
            WriteOutput::WrittenImpact { written, impact } => {
                let mut rows = written_rows(written);
                rows.extend(impact_rows(impact));
                rows
            }
```

Replace `run_plan`, `run_provision_plan` and `send` with:

```rust
/// What a write's output carries besides the request or the result.
enum Extra {
    None,
    /// `policy provision`: object counts from `change_subset`.
    Counts,
    /// `--impact` on a rule write, and whether its workload read was truncated.
    Impact(Impact, bool),
}

/// Dry run prints the request; otherwise the plan is executed once and the
/// result printed with `meta.write`.
pub async fn run_plan(
    ctx: &Context,
    flags: &WriteFlags,
    guard: &WriteGuard,
    plan: Plan,
) -> Result<(), CliError> {
    send(ctx, flags, guard, plan, Extra::None).await
}

/// [`run_plan`] for `policy provision`: the dry run and the result carry
/// `counts` (object type to number of hrefs in `change_subset`).
pub async fn run_provision_plan(
    ctx: &Context,
    flags: &WriteFlags,
    guard: &WriteGuard,
    plan: Plan,
) -> Result<(), CliError> {
    send(ctx, flags, guard, plan, Extra::Counts).await
}

/// [`run_plan`] for a rule or deny-rule write: with an `impact` (from
/// `--impact`, computed before anything is sent) the dry run and the result
/// carry it, and `meta.truncated` reports a truncated workload read.
pub async fn run_rule_plan(
    ctx: &Context,
    flags: &WriteFlags,
    guard: &WriteGuard,
    plan: Plan,
    impact: Option<(Impact, bool)>,
) -> Result<(), CliError> {
    let extra = match impact {
        Some((impact, truncated)) => Extra::Impact(impact, truncated),
        None => Extra::None,
    };
    send(ctx, flags, guard, plan, extra).await
}

async fn send(
    ctx: &Context,
    flags: &WriteFlags,
    guard: &WriteGuard,
    plan: Plan,
    extra: Extra,
) -> Result<(), CliError> {
    let truncated = matches!(extra, Extra::Impact(_, true));
    let (request, kind) = match plan {
        Plan::Noop {
            existing_href,
            reason,
        } => {
            return ctx.emit_write_with(
                &WriteOutput::Skipped {
                    skipped: true,
                    href: existing_href,
                    reason,
                },
                None,
                truncated,
            );
        }
        Plan::Send { request, kind } => (request, kind),
    };
    let counts = matches!(extra, Extra::Counts)
        .then(|| provision_counts(request.body.as_ref().unwrap_or(&serde_json::Value::Null)));
    if flags.dry_run {
        let out = match (extra, counts) {
            (Extra::Impact(impact, _), _) => WriteOutput::DryRunImpact { request, impact },
            (_, Some(counts)) => WriteOutput::ProvisionDryRun { request, counts },
            _ => WriteOutput::DryRun(request),
        };
        return ctx.emit_write_with(&out, None, truncated);
    }
    let method = request.method;
    let path = request.path.clone();
    match ctx
        .pce
        .writer(guard)
        .execute(Plan::Send { request, kind })
        .await?
    {
        Outcome::Written(w) => {
            let meta = WriteMeta {
                method,
                path,
                status: w.status,
                audit: guard.audit_location(),
            };
            let out = match (extra, counts) {
                (Extra::Impact(impact, _), _) => WriteOutput::WrittenImpact { written: w, impact },
                (_, Some(counts)) => WriteOutput::Provisioned { written: w, counts },
                _ => WriteOutput::Written(w),
            };
            ctx.emit_write_with(&out, Some(meta), truncated)
        }
        Outcome::Skipped {
            existing_href,
            reason,
        } => ctx.emit_write_with(
            &WriteOutput::Skipped {
                skipped: true,
                href: existing_href,
                reason,
            },
            None,
            truncated,
        ),
    }
}
```

`crates/illumio-tui/src/cli/policy.rs`: add `is_rule_href` to the `pce::impact` import and `use pce::write::Plan;` (or add `Plan` to the existing `pce::write::{…}` list); add `run_rule_plan` to the `super::write::{…}` import. `RuleAddArgs`, `RuleUpdateArgs`, `DenyRuleAddArgs` and `DenyRuleUpdateArgs` each gain, before `#[command(flatten)] pub write: WriteFlags,`:

```rust
    /// Also report the rule's impact (workloads matched as providers and consumers), computed before anything is sent; one workloads read, cached
    #[arg(long)]
    pub impact: bool,
```

Add after `impact_workloads`:

```rust
/// `--impact` on a rule write: the impact of what the plan would send (its
/// body, an update's untouched sides from the loaded rule) in the target
/// ruleset's scopes over the workloads; the rulesets and the workloads are
/// cached reads, made after the guard and the plan. `None` for a plan with
/// nothing to send.
async fn planned_impact(ctx: &Context, plan: &Plan) -> Result<Option<(Impact, bool)>, CliError> {
    let Some(req) = plan.request() else {
        return Ok(None);
    };
    let not_found = |p: &str| CliError::Pce(PceError::NotFound { path: p.to_string() });
    let rs_href = rule_set_href_of(&req.path).ok_or_else(|| not_found(&req.path))?;
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
    let sets = ctx
        .pce
        .rule_sets(PolicyScope::Draft, ctx.cache_mode)
        .await?;
    let rs = sets
        .value
        .iter()
        .find(|r| r.href == rs_href)
        .ok_or_else(|| not_found(rs_href))?;
    let base = if !is_rule_href(&req.path) {
        None
    } else if req.path.contains("/deny_rules/") {
        deny_rules_of(ctx, rs)
            .await?
            .iter()
            .find(|d| d.href == req.path)
            .map(RuleActors::of_deny)
    } else {
        rs.rules
            .iter()
            .find(|r| r.href == req.path)
            .map(RuleActors::of_rule)
    };
    let actors = RuleActors::planned(req.body.as_ref(), base.as_ref());
    let w = impact_workloads(ctx).await?;
    let inputs = ImpactInputs {
        workloads: &w.value,
        index: idx.as_ref(),
        truncated: w.truncated,
    };
    let impact = rule_impact(
        &rs.scopes,
        &actors.providers,
        &actors.consumers,
        actors.unscoped_consumers,
        &inputs,
    );
    Ok(Some((impact, sets.truncated || w.truncated)))
}
```

In `rule_add`, `rule_update`, `deny_rule_add` and `deny_rule_update`, the last line `run_plan(ctx, &a.write, &guard, plan).await` becomes:

```rust
    let impact = if a.impact {
        planned_impact(ctx, &plan).await?
    } else {
        None
    };
    run_rule_plan(ctx, &a.write, &guard, plan, impact).await
```

(`rule_delete`, `deny_rule_delete` and the ruleset commands keep `run_plan`.)

- [ ] **Step 4: Run the tests**

Run: `cargo fmt --all && cargo test -p illumio-tui --test cli_writes && cargo test -p illumio-tui --test cli`
Expected: PASS (the three new tests; `every_write_command_is_guarded_and_needs_yes` with 27 entries, every one exit 6 before any request; every existing dry-run and send test unchanged).

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p illumio-tui
git add crates/illumio-tui/src/cli/write.rs crates/illumio-tui/src/cli/context.rs crates/illumio-tui/src/cli/policy.rs crates/illumio-tui/tests/cli_writes.rs
git commit -m "feat(cli): --impact on rule and deny-rule writes, in dry runs and results

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 9: Docs, tracking, reads-only smoke, final gate

**Files:**
- Modify: `README.md` (`## TUI` screens paragraph ~lines 80-84, Policy bullet ~90-93, `Labels & objects` bullet ~94, startup paragraph ~107-110; `## CLI` examples ~116-127; `### Policy writes` before the `policy provision --all` paragraph ~line 233; `### Editing in the TUI` first paragraph ~line 251)
- Modify: `docs/architecture.md` (TUI module table ~lines 42-54; new `### IP lists and rule impact` before `## Testing` ~line 218)
- Modify: `docs/index.html` (screens list ~lines 194-200; features `<ul>` after line 97)
- Modify: `tasks/todo.md` (append the Plan 7 checklist and review)
- `tasks/lessons.md`: no change unless the execution produced a correction (then append a `## Plan 7 (IP lists and rule impact)` section).
- Create (scratchpad only, never committed): `$SCRATCH/plan7_smoke.py`, where `$SCRATCH` is the executing session's scratchpad directory

**Interfaces:**
- Consumes: the built binary `target/debug/illumio-tui`; `../illumio-mcp-server/.env` (credentials of the read-only test PCE demo100, read by `--env-file`, never copied, printed or committed); a scratch venv with `pyte` (Plans 4 and 6 smoke).
- Produces: documentation only; no code changes.

- [ ] **Step 1: README**

Replace the screens paragraph (`Five screens: … a click on a tab switches screen.`) with:

```markdown
Six screens: `1` Traffic, `2` Policy, `3` Objects, `4` Workloads, `5` Events, `6` IP lists. The
`:` prompt takes `:traffic`, `:policy`, `:objects`, `:workloads`, `:events`, `:iplists`,
`:filter <expr>`, `:window <dur>`, `:profile <name>`, `:draft`, `:active`, `:help` and `:q`.
`?` shows the full key map, `Esc` closes overlays, `q` or `Ctrl-C` quits. The mouse wheel
scrolls the current table and a click on a tab switches screen; on a terminal narrower than
the full tab row the tabs read `Traf`, `Pol`, `Obj`, `Wl`, `Ev`, `IPs`.
```

Replace the Policy bullet and the `Labels & objects` bullet with:

```markdown
- **Policy**: `d` toggle draft / active, `p` pending changes, `Tab` switch panes, `Enter` rule
  detail, or on a ruleset load its deny rules. Rules are listed in evaluation order: override
  deny, allow, deny. With writes on, `n` / `e` / `x` create, edit and delete rulesets and rules
  in draft (`N` a deny rule), and the pending overlay marks with `Space` and provisions with `P`.
  `i` reads the workload list once (unless the Workloads screen already did) for rule impact:
  the rule detail says how many workloads the rule matches (`impact      12 providers · 3
  consumers`, `≥` when the list was truncated) and the size of each IP list it names, the
  rules pane's `P/C` column fills in, and a rule's confirm overlay shows the same for the rule
  about to be written. Impact uses the ruleset's scopes, unscoped consumers, label groups (with
  their sub-groups), exclusions, `all` and workload actors; IP lists are sized, never expanded.
- **Objects**: labels, label groups and services; `Tab` next type, `Enter` detail.
- **IP lists**: name, size (addresses after exclusions: `65.5K`; `any` for `0.0.0.0/0`, `::/0`
  or 16M addresses and more), ranges, FQDNs, `used by` (rules of the loaded draft rulesets that
  name the list; `-` until Policy is loaded) and pending; `Enter` shows the ranges, FQDNs, the
  size and those rules. With writes on, `n` / `e` / `x` create, edit and delete.
```

Replace the startup paragraph with:

```markdown
Startup makes exactly five requests (health plus the four object lists). The first visit to
Workloads and to Events costs one request each; going back to them costs none. IP lists (`6`)
cost none: the startup load already read them. Rule impact costs one workloads read on `i`,
none when the list is cached. Everything else is caused by a key you press. The header shows
the API call and byte counters at all times.
```

In the `## CLI` examples, after `illumio-tui policy rules "Web-DB"` add:

```
    illumio-tui policy rules "Web-DB" --impact --json
    illumio-tui policy rules impact /orgs/1/sec_policy/draft/rule_sets/7/sec_rules/3
    illumio-tui iplists --table
```

and after the paragraph that ends with `config, 3 auth, 4 PCE error, 5 rate limited, 6 write refused by the guard.` add:

```markdown
`--impact` on `policy rules <ruleset>` adds each rule's `impact` (JSON) or a `P/C` column
(table): `providers` and `consumers` (workloads matched, unions over the ruleset's scopes),
`per_scope`, `ip_lists` (name, href, `size`, side), `unsupported` (virtual services, not
counted) and `truncated`. `policy rules impact <rule-href>` prints one rule the same way (exit
2 for an href of the wrong shape, 4 when the rule is not found). It costs one workloads read,
through the cache; `meta.truncated` includes it. `iplists` gives every list a `size`: `v4` and
`v6` address counts as strings, `unbounded` (`0.0.0.0/0`, `::/0` or `≥16M addresses`), `ranges`
and `fqdns`.
```

In `### Policy writes`, before the paragraph that starts with `` `policy provision --all` reads the pending list once ``, add:

```markdown
`--impact` on `policy rules add|update` and `policy deny-rules add|update` reports what the
rule will touch, before anything is sent: with `--dry-run`, `data` is the request plus
`impact`; with `--yes`, the result plus `impact`. The rule's sides come from the planned body
(an update's untouched sides from the loaded rule), the scopes from the draft ruleset, and the
workloads from one cached read made after the guard and the plan, so a refused write costs
nothing.

    illumio-tui --allow-writes policy rules add Shop-Prod --provider role=web --consumer iplist:Corp --service 443/tcp --dry-run --impact --json
```

In `### Editing in the TUI`, change "the header shows `✎ writes` before the host and two screens take three keys: on Labels &
objects (labels, IP lists, services) and Workloads (unmanaged workloads only)" to "the header shows `✎ writes` before the host and three screens take three keys: on Objects (labels, services), IP lists and Workloads (unmanaged workloads only)".

- [ ] **Step 2: Architecture** (`docs/architecture.md`)

In the TUI module table, after the `diff` row add:

```markdown
| `impact` | rule impact as the TUI shows it: `P/C` cells, the rule detail's rows, the confirm line |
```

Insert before `## Testing`:

```markdown
### IP lists and rule impact

`pce::iplist_size` counts an IP list's addresses per family with std
integers (`Ipv4Addr` as `u32`, `Ipv6Addr` as `u128`): included ranges are
merged, exclusions subtract only their overlap, a CIDR's host bits are
ignored, and an included `0.0.0.0/0` or `::/0`, or 2^24 addresses and more,
makes the list `unbounded`. JSON carries `v4`/`v6` as strings.

`pce::impact::rule_impact` evaluates a rule over a workload list: for each
ruleset scope (none means one empty scope, everywhere) the providers are the
workloads inside the scope that match the provider actors, the consumers
those that match the consumer actors, inside the scope unless
`unscoped_consumers`; the totals are unions over the scopes. Labels and
label groups are grouped by key (AND across keys, OR within one; a group
counts under its own `key`, its sub-groups expanded once, cycles ended);
`all` matches everything, a workload actor its href, IP lists and virtual
services nothing (lists are reported with their size, virtual services as
`unsupported`); exclusions remove last. Label groups and IP lists are looked
up in the `NameIndex` (`label_group`, `ip_list`). `RuleActors` carries a
rule's sides from a loaded rule or deny rule, or from a planned request body
merged over the loaded rule, so the CLI and the TUI evaluate exactly what
would be sent.

The TUI reads the workload list only on `i` (`Cmd::LoadWorkloads(Use)`,
nothing when it is cached or on its way). `tui/impact.rs` computes from
`PolicyState.raw`, the index and `WorkloadsState.raw`: the rules pane's
`P/C` cells when the rows are built (`RuleRow`; a column's `fn` cannot see
the cache), the rule detail's rows and the confirm overlay's line when they
are drawn, so a late `WorkloadsLoaded` only rebuilds the rules pane and
never touches an overlay. IP lists are their own screen (`Screen::IpLists`,
`IpListsState`): the list is the index's own (a cache hit after startup),
sizes are computed once per load, and `used_by` counts the loaded draft
rules that name the list, recomputed when the rulesets or their fetched deny
rules arrive.

The CLI's `--impact` costs one workloads read through the cache; on a write
it runs after the guard and the plan and before the send, and its
`truncated` flag is OR'd into `meta.truncated`.
```

- [ ] **Step 3: Website** (`docs/index.html`)

In the screens list, change `<h3>Four screens</h3>` to `<h3>Screens</h3>`, replace the `<kbd>3</kbd>` item with

```html
        <li><kbd>3</kbd><strong>Objects.</strong> Labels, label groups and services, with cache age shown and per-type refresh.</li>
```

and add after the `<kbd>4</kbd>` item:

```html
        <li><kbd>6</kbd><strong>IP lists.</strong> Every IP list with its size in addresses (exclusions taken out), its ranges and FQDNs, and the rules that use it.</li>
```

After the `<li>` that starts "Edits policy in the TUI too" (line 97) add:

```html
    <li>Shows what a rule touches: <code>i</code> in the TUI and <code>--impact</code> in the CLI count the workloads a rule matches as providers and consumers, with ruleset scopes, label groups and exclusions, from one cached workload read, before and after you write it.</li>
```

- [ ] **Step 4: Tracking** (append to `tasks/todo.md`)

```markdown
## Plan 7: IP lists screen and rule impact
Spec: docs/superpowers/specs/2026-10-11-iplists-impact-design.md · Plan: docs/superpowers/plans/2026-10-11-iplists-impact.md
- [x] Task 1 pce::iplist_size
- [x] Task 2 pce::impact
- [x] Task 3 IP lists screen (read side)
- [x] Task 4 IP lists screen (write side, used-by recompute, CLI size)
- [x] Task 5 Workload cache plumbing and rule detail impact
- [x] Task 6 Confirm overlay impact line
- [x] Task 7 CLI --impact for existing rules
- [x] Task 8 CLI --impact on rule writes
- [x] Task 9 Docs, tracking, reads-only smoke, final gate
```

Tasks 1-8 have landed when this task runs; Task 9 is ticked in the same commit as this file (Step 8).

- [ ] **Step 5: Final gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS; note the test count for the review.

Run: `git diff --stat c0aa870 HEAD -- crates/pce`
Expected: `crates/pce/src/iplist_size.rs` and `crates/pce/src/impact.rs` (new), `crates/pce/src/resolve.rs` (`canon` visibility, two accessors, one test), `crates/pce/src/lib.rs` (two modules, two re-exports). No write path changed: `git diff c0aa870 HEAD -- crates/pce/src/write` is empty.

Run: `git diff c0aa870 HEAD -- Cargo.toml crates/*/Cargo.toml Cargo.lock`
Expected: empty (no new dependency).

- [ ] **Step 6: Reads-only smoke against demo100**

No write may be sent to demo100: the TUI runs without `--allow-writes` (the guard stays off), the CLI commands are reads, `PCE_LIVE_WRITES` stays unset, and every run uses a scratch `XDG_STATE_HOME`, so the real audit log is never touched.

Set up (reuse the Plan 6 venv if it is still in this session's scratchpad):

```bash
cargo build -p illumio-tui
python3 -m venv "$SCRATCH/venv" && "$SCRATCH/venv/bin/pip" install pyte
mkdir -p "$SCRATCH/state7"
```

Create `$SCRATCH/plan7_smoke.py`:

```python
#!/usr/bin/env python3
"""Plan 7 smoke against the read-only test PCE: the IP lists screen, rule
impact in the TUI, and the CLI's --impact and sizes. Reads only: the TUI runs
without --allow-writes and no step contains 'y'."""
import fcntl, json, os, pty, re, select, struct, subprocess, sys, termios, time
import pyte

BIN, ENV_FILE, STATE = sys.argv[1], sys.argv[2], sys.argv[3]
COLS, ROWS = 100, 30  # wide enough for the full tab row (84 cells)
AUDIT = os.path.join(STATE, "illumio-tui", "writes.jsonl")


def child_env():
    env = {k: v for k, v in os.environ.items() if k not in ("ILLUMIO_WRITES", "PCE_LIVE_WRITES")}
    env["XDG_STATE_HOME"] = STATE
    return env


def tui(steps):
    for _, keys in steps:
        assert "y" not in keys and "Y" not in keys, "the smoke never presses y"
    pid, fd = pty.fork()
    if pid == 0:
        os.execve(BIN, [BIN, "--env-file", ENV_FILE], child_env())
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
    screen = pyte.Screen(COLS, ROWS)
    stream = pyte.ByteStream(screen)

    def pump(seconds):
        end = time.time() + seconds
        while time.time() < end:
            ready, _, _ = select.select([fd], [], [], 0.05)
            if ready:
                try:
                    stream.feed(os.read(fd, 65536))
                except OSError:
                    return

    def calls():
        m = re.search(r"api (\d+) calls", screen.display[0])
        return int(m.group(1)) if m else -1

    pump(6)
    seen = [(calls(), list(screen.display))]
    for wait, keys in steps:
        for k in keys:
            os.write(fd, k.encode())
            pump(0.15)
        pump(wait)
        seen.append((calls(), list(screen.display)))
    for _ in range(3):
        os.write(fd, b"\x1b")
        pump(0.3)
    os.write(fd, b"q")
    pump(1)
    os.waitpid(pid, 0)
    return seen


def text(display):
    return "\n".join(display)


def cli(*args):
    out = subprocess.run([BIN, "--env-file", ENV_FILE, "--json", *args], env=child_env(),
                         capture_output=True, text=True, timeout=120)
    assert out.returncode == 0, (args, out.returncode, out.stderr[:500])
    return json.loads(out.stdout)


s = tui([
    (2.0, "6"),      # 1 IP lists: the startup load already read them
    (0.5, "\r"),     # 2 detail of the first list
    (3.0, "2"),      # 3 Policy: the draft rulesets (and maybe a deny-rule fallback)
    (0.5, "\t"),     # 4 rules pane
    (0.5, "\r"),     # 5 rule detail, if the first ruleset has a rule
    (6.0, "i"),      # 6 one workloads read
    (1.0, "i"),      # 7 a second i costs nothing
])
assert "6 IP lists" in s[0][1][1], s[0][1][1]
assert s[1][0] == s[0][0], ("opening IP lists made a request", s[0][0], s[1][0])
assert "IP lists" in text(s[1][1]) and "size" in text(s[1][1])
assert "size" in text(s[2][1]), "the detail shows the size line"
print("IP lists screen:\n" + text(s[2][1]))
assert s[6][0] == s[5][0] + 1, ("i made other than one request", s[5][0], s[6][0])
assert s[7][0] == s[6][0], ("a second i made a request", s[6][0], s[7][0])
detail = text(s[6][1])
if "Esc closes" in text(s[5][1]):
    assert "impact" in detail and "provider" in detail, detail
    print("rule detail with impact:\n" + detail)
else:
    print("the first ruleset showed no rule; i still read the workloads once:", s[6][1][ROWS - 1])
print("TUI calls per step", [c for c, _ in s])

sets = cli("policy", "rulesets")
first = sets["data"][0]["name"]
rules = cli("policy", "rules", first, "--impact")
with_impact = [r for r in rules["data"]["rules"] if "impact" in r]
assert len(with_impact) == len(rules["data"]["rules"]), "every rule carries impact"
print(f"policy rules {first!r} --impact: {len(with_impact)} rules, api_calls",
      rules["meta"]["api_calls"], "truncated", rules["meta"]["truncated"],
      [(r["impact"]["providers"], r["impact"]["consumers"]) for r in with_impact[:5]])
lists = cli("iplists")
assert all("size" in l for l in lists["data"])
print("iplists: api_calls", lists["meta"]["api_calls"],
      [(l["name"], l["size"]["unbounded"] or l["size"]["v4"]) for l in lists["data"][:6]])
assert not os.path.exists(AUDIT), "an audit file exists: something was sent"
print("audit log: none")
```

Run it:

```bash
"$SCRATCH/venv/bin/python" -I "$SCRATCH/plan7_smoke.py" target/debug/illumio-tui ../illumio-mcp-server/.env "$SCRATCH/state7"
```

Expected: `6 IP lists` in the full tab row; opening IP lists costs no request; the detail shows a `size` line; `i` costs exactly one request and a second `i` none; the rule detail shows `impact` (or the recorded reason when the first ruleset has no rule); every rule of the first ruleset carries `impact` in the CLI; every IP list carries `size`; no audit file. Record the per-step call counts and the CLI `api_calls`. If an assertion fails, stop and debug (systematic-debugging); do not retry against the PCE in a loop.

- [ ] **Step 7: Review section** (append to `tasks/todo.md` under the Plan 7 checklist)

Write `## Review (Plan 7)` with: what shipped (one line per user-facing feature: the IP lists screen and its keys, sizes in TUI and CLI, `i` and the impact in the rule detail, the rules pane and the confirm, `policy rules --impact`, `policy rules impact`, `--impact` on rule writes), the final gate's test count from Step 5, the `crates/pce` diff summary and the empty dependency diff, the smoke's per-step call counts and CLI `api_calls` (and whether a rule detail was exercised on demo100, or why not), the statement that no write was sent to any PCE, and every deviation reported by a task.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add README.md docs/architecture.md docs/index.html tasks/todo.md
git commit -m "docs: IP lists screen, rule impact and --impact; Plan 7 tracking and review

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```
