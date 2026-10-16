# Policy Generator (3 of 3): the TUI generator (`G` / `:gen`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `G` in the app overlay (or `:gen <app>`, or `:gen` and the picker) opens the policy generator for one app: the slider (`app·all`, `app·ports`, `role·ports`, `role·all`), the target ruleset, the inbound and (on `o`) outbound rows with their checkboxes, per-row overrides and flags, the coverage preview, `p` for the exact rule list, `r` to re-query incomplete data, and `Enter` to plan every write into Plan 2's single batch confirm. `TrafficState::complete_for` says whether the data the generator reads is whole. Docs: README, `docs/index.html`, in-TUI help.

**Architecture:** `GenView` (new `tui/gen_view.rs`, pure) holds the rows, slider, section, cursors, outbound switch, the picked ruleset, the request counters, the drafts' state and the current `Proposal`; its methods move the cursor and the stops and build every text the renderer prints. `update`'s child module `update/generator.rs` opens it, routes its keys, rebuilds the rows when traffic lands and the proposal when rulesets land (`pce::generate::{rows, propose}`, no request), runs `r` through `start_focused`, and emits Plan 2's `Cmd::PlanBatch` on `Enter`. Plan 2's three return points (`batch_lands`, `batch_plan_failed`, `batch_closed`) now mean the generator. `ui::widgets::generator` draws it full-screen like the app overlay.

**Tech Stack:** Rust 2024 (rust-version 1.88), ratatui 0.30, crossterm, insta (plain snapshot files). No new dependency.

**Spec:** `docs/superpowers/specs/2026-10-16-policy-generator-design.md` (binding), delivery item 3, sections "Completeness", "TUI" and the TUI tests. Builds on Plans 1 and 2 (`2026-10-16-policy-generator-1-derive.md`, `2026-10-16-policy-generator-2-batch.md`), both merged first.

**Tasks:** 5, in this order: 1 `complete_for` and the profile's ruleset name in the TUI's limits; 2 `GenView`; 3 the generator's transitions (open, keys, rebuilds, `r`, `Enter`, the batch's return); 4 drawing it, snapshots, help; 5 docs, tracking, read-only smoke, final gate.

## Global Constraints

- The gate is `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`. Run `cargo fmt --all` first and accept its formatting.
- No new dependencies. `git diff main -- '*Cargo.toml' Cargo.lock` must be empty.
- Never hammer the PCE: every request is caused by the user, throttled, cached where it is a read, counted and visible. Nothing refreshes on its own.
- Writes are gated (profile `writes = true`, audit log writable; the TUI after `:writes on`) and always confirmed; nothing is ever auto-confirmed. CLI `--yes` semantics stay as they are. The generator previews with writes off (`p` works); `Enter` gives the usual refusal. With writes on, `Enter` plans (reads only) and the batch confirm of Plan 2 sends nothing before `y`.
- Tests use recorded fixtures (`fixtures/`, `fixtures/recorded/`) and mocks (wiremock) only. Live smokes on the demo PCE are read-only: never `--yes`, never `y`, writes off; they run with `--env-file ../illumio-mcp-server/.env`. Never read or print that file.
- Verified on demo100 (2026-10-16): IP list consumers in a scoped ruleset appear with `unscoped_consumers: false`; cross-app inbound rules use `unscoped_consumers: true`. The derivation tables rely on both.
- Opening the generator sends at most one request (the draft rulesets, cached), counted in its header; reopening the same app sends nothing. Keys other than `r` and `Enter` send nothing. `r` runs the focused query through the existing cost guard (asking past 7 days or a cap above 10000).
- Snapshots are insta files, updated with `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots`, then `find crates/illumio-tui/tests/snapshots -name '*.snap.new' -delete`. Read every new or changed `.snap` before committing.
- Elm-style TUI: `update` is pure and returns `Cmd`s; the runtime executes them. The generator's state is `Model::gen_view` (`tui/gen_view.rs`), its transitions a child module of `update` (`tui/update/generator.rs`).
- New rulesets are named by `[profiles.<name>.generator] ruleset_name`, default `"RF-{app}-{env}"` (`RF-shop-prod`). New IP lists are named `<app>-<env> peer <ip>` (`shop-prod peer 10.4.1.9`), one per IP peer.
- `gen` is a reserved keyword in Rust 2024: modules are `generate` (`pce::generate`, `cli::generate`), never `gen`.
- Every commit message ends with these two lines:
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx

## Spec deviations and resolutions (read before Task 1)

- **The generator stays open while `Enter` plans** (`GenView::planning`, the footer says so). A plan lands only over the generator that is still planning (`batch_lands`); after `Esc`, or over any other overlay, it is discarded with `requests discarded, nothing sent`. A `y` typed meanwhile goes to the generator, where it does nothing.
- **A plan-time error shows in the generator's footer** (`plan failed, nothing sent: …`) and on the status line, with the generator open and its state kept; an error overlay would hide it and `Esc` would close it. Without the generator open, the error goes through `fail` as before.
- **`R` picks among the rulesets covering the app only.** An outbound peer whose scope several rulesets cover blocks `Enter` with `2 rulesets cover crm/prod: uncheck its rows or narrow its rulesets` (`Proposal::blocker`).
- **`r` focuses the Traffic screen on the generator's app** (`TrafficState::set_focus`) so the focused answer is applied (`FocusedDone` only applies the focus in force); the Traffic screen shows that focus afterwards. Past the cost guard a new `PendingAction::GenRequery` asks, and `y`/`n` return to the generator. When the drafts failed to load, `r` loads them again instead.
- **Completeness texts:** `incomplete: r re-queries shop/prod (1 query)` (2 with outbound), `no traffic loaded: r queries shop/prod (…)`, `still capped at 1000: B raises the cap, or narrow the window` (then `r` does nothing: the focused result itself is capped). The header ends `· complete` or `· incomplete`.
- **After a batch** the generator reopens; its proposal is rebuilt from whatever draft rulesets arrive next (the write's own reload), so written rules show as `exists`. When no screen holds the drafts, reopening asks for them once (counted).
- **The `w`-form stale-drafts rule covers the generator too:** a draft load that a policy write made stale is asked again when the generator is waiting for it.
- **`G` is not in the app overlay's status hints:** at 80 columns it would cut the overlay's own status line; the help lists it.
- **`·all` stops are skipped by `←`/`→` and `<`/`>` without an All Services object** and shown in parentheses on the slider.
- **Profile switch:** `reset_data` drops the generator; a generator whose keys no longer are the scope keys closes with `g changed: G reopens the generator`.

## Review Focus

- **A focused result that is itself truncated** must keep `Enter` refused (`partial_for` calls it whole). Pinned in Task 1 (`complete_for_judges_the_result_actually_used`) and Task 3 (`a_capped_focused_result_says_so_and_r_does_not_repeat_it`).
- **`Esc` while the plan is on its way:** the late plan is discarded, nothing opens. Pinned in Task 3 (`a_plan_landing_after_esc_is_discarded`).
- **Opening without loaded drafts** costs exactly one counted request; its cached answer shows `1 req · cached`; reopening costs none. Pinned in Task 3 (`opening_without_drafts_costs_one_counted_request`).
- **`g` keys other than the scope keys** never open a generator. Pinned in Task 3 (`keys_other_than_the_scope_keys_are_refused`).
- **`r` on a costly window** asks first and comes back to the generator either way. Pinned in Task 3 (`r_past_the_cost_threshold_asks_and_comes_back`).

---

### Task 1: `TrafficState::complete_for` and the ruleset name in `Limits`

**Files:**
- Modify: `crates/illumio-tui/src/tui/model.rs`
- Modify: `crates/illumio-tui/src/tui/app.rs`
- Modify: `crates/illumio-tui/src/tui/update.rs`

**Interfaces:**
- Consumes: `FocusResult::covers`, `TrafficData::{truncated, incomplete, filter}`; Plan 1's `ResolvedConfig::generator_ruleset_name`, `pce::generate::DEFAULT_RULESET_NAME`.
- Produces:

```rust
// tui::model
pub enum Gap { NoData, Capped { cap: u32, focused: bool }, Region { focused: bool } }
impl TrafficState { pub fn complete_for(&self, focus: &Focus) -> Result<(), Gap>; }
pub struct Limits { /* … */ pub ruleset_name: String }   // generator.ruleset_name
```

- [ ] **Step 1: Write the failing tests**

In the test module of `crates/illumio-tui/src/tui/model.rs`, add at its top:

```rust
mod tests {
    use super::*;

    fn at(s: &str) -> jiff::Timestamp {
        s.parse().unwrap()
    }
```

with:

```rust
mod tests {
    use super::*;

    #[test]
    fn complete_for_judges_the_result_actually_used() {
        use crate::tui::test_support::model_with_flows;
        use pce::traffic::{FocusDirection, parse_app};
        let shop = Focus::new(parse_app("shop/prod"), vec!["app".into(), "env".into()])
            .with_direction(FocusDirection::In);
        let mut m = model_with_flows();
        assert_eq!(m.traffic.complete_for(&shop), Ok(()));
        m.traffic.data.as_mut().unwrap().incomplete = true;
        assert_eq!(
            m.traffic.complete_for(&shop),
            Err(Gap::Region { focused: false })
        );
        m.traffic.data.as_mut().unwrap().incomplete = false;
        m.traffic.data.as_mut().unwrap().truncated = true;
        assert_eq!(
            m.traffic.complete_for(&shop),
            Err(Gap::Capped {
                cap: 1000,
                focused: false
            })
        );
        // A focused result for the app, itself truncated: partial_for
        // calls it whole, complete_for does not.
        let data = TrafficData {
            flows: m.traffic.loaded_flows().to_vec(),
            truncated: true,
            incomplete: false,
            elapsed: Duration::ZERO,
            cache_hit: false,
            fetched_at: Instant::now(),
            filter: m.traffic.filter.clone(),
            range: None,
            hrefs: Vec::new(),
            draft: None,
        };
        m.traffic.apply_focused(shop.clone(), 1, data);
        assert!(!m.traffic.partial_for(&shop));
        assert_eq!(
            m.traffic.complete_for(&shop),
            Err(Gap::Capped {
                cap: 1000,
                focused: true
            })
        );
        // The out side is not covered by an `in` result: the unfocused one counts.
        let out = shop.clone().with_direction(FocusDirection::Out);
        assert_eq!(
            m.traffic.complete_for(&out),
            Err(Gap::Capped {
                cap: 1000,
                focused: false
            })
        );
        let empty = crate::tui::test_support::model_fixture();
        assert_eq!(empty.traffic.complete_for(&shop), Err(Gap::NoData));
    }

    fn at(s: &str) -> jiff::Timestamp {
        s.parse().unwrap()
    }
```

and in `limits_from_config_carry_the_ignored_keys`:

```rust
        let limits = Limits::from_config(&cfg);
        assert_eq!(limits.ignore_keys, vec!["os"], "app is a scope key");
```

with:

```rust
        let limits = Limits::from_config(&cfg);
        assert_eq!(limits.ignore_keys, vec!["os"], "app is a scope key");
        assert_eq!(limits.ruleset_name, "RF-{app}-{env}");
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p illumio-tui --lib model`
Expected: FAIL to compile: "no method named `complete_for`", "cannot find type `Gap`", "no field `ruleset_name`".

- [ ] **Step 3: Write the implementation**

In `crates/illumio-tui/src/tui/model.rs`, the gap, before `queries_text`'s re-export:

```rust
    }
}

/// `1 query`, `2 queries` (shared with the CLI's meta line).
pub use pce::traffic::queries_text;

```

with:

```rust
    }
}

/// Why the loaded traffic may miss flows of an app (`complete_for`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gap {
    /// No result is loaded.
    NoData,
    /// The result used hit its cap; `focused`: it is the app's own focused
    /// result, so only a larger cap or a narrower window helps.
    Capped { cap: u32, focused: bool },
    /// A supercluster region did not answer.
    Region { focused: bool },
}

/// `1 query`, `2 queries` (shared with the CLI's meta line).
pub use pce::traffic::queries_text;

```

and `complete_for`, before `window_bounds`:

```rust
        }
    }

    /// The loaded result's window, `(start, end)`: its range's end and the
    /// filter's window back from it. `None` before a result with bounds.
    pub fn window_bounds(&self) -> Option<(jiff::Timestamp, jiff::Timestamp)> {
```

with:

```rust
        }
    }

    /// Whether `flows_for(focus)` holds every flow of `focus`: the result
    /// actually used is neither truncated nor incomplete. Unlike
    /// [`Self::partial_for`], a covering focused result that is itself
    /// truncated is a gap, and so is a region that did not answer.
    pub fn complete_for(&self, focus: &Focus) -> Result<(), Gap> {
        let focused = self.focus_result.as_ref().is_some_and(|r| r.covers(focus));
        let used = match (&self.unfocused, focused) {
            (_, true) => self.data.as_ref(),
            (Some(u), false) => Some(u),
            (None, false) => self.data.as_ref(),
        };
        let Some(d) = used else {
            return Err(Gap::NoData);
        };
        if d.truncated {
            return Err(Gap::Capped {
                cap: d.filter.max_results,
                focused,
            });
        }
        if d.incomplete {
            return Err(Gap::Region { focused });
        }
        Ok(())
    }

    /// The loaded result's window, `(start, end)`: its range's end and the
    /// filter's window back from it. `None` before a result with bounds.
    pub fn window_bounds(&self) -> Option<(jiff::Timestamp, jiff::Timestamp)> {
```

The ruleset name in `Limits`:

```rust
    pub tier_key: String,
    /// Label keys hidden from label text (columns, the `g` picker).
    pub ignore_keys: Vec<String>,
}

impl Limits {
```

with:

```rust
    pub tier_key: String,
    /// Label keys hidden from label text (columns, the `g` picker).
    pub ignore_keys: Vec<String>,
    /// `generator.ruleset_name`: the name of a ruleset the generator creates.
    pub ruleset_name: String,
}

impl Limits {
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
            scope_keys: cfg.scope_keys.clone(),
            tier_key: cfg.tier_key.clone(),
            ignore_keys: cfg.ignore_keys.clone(),
        }
    }

```

with:

```rust
            scope_keys: cfg.scope_keys.clone(),
            tier_key: cfg.tier_key.clone(),
            ignore_keys: cfg.ignore_keys.clone(),
            ruleset_name: cfg.generator_ruleset_name.clone(),
        }
    }

```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
                scope_keys,
                tier_key: crate::config::DEFAULT_TIER_KEY.into(),
                ignore_keys: Vec::new(),
            },
            screen: Screen::Traffic,
            overlay: Overlay::None,
```

with:

```rust
                scope_keys,
                tier_key: crate::config::DEFAULT_TIER_KEY.into(),
                ignore_keys: Vec::new(),
                ruleset_name: pce::generate::DEFAULT_RULESET_NAME.into(),
            },
            screen: Screen::Traffic,
            overlay: Overlay::None,
```

Every other `Limits` literal names it too. In `crates/illumio-tui/src/tui/app.rs` (the test `limits()`):

```rust
            scope_keys: vec!["app".into(), "env".into()],
            tier_key: "role".into(),
            ignore_keys: Vec::new(),
        }
    }

```

with:

```rust
            scope_keys: vec!["app".into(), "env".into()],
            tier_key: "role".into(),
            ignore_keys: Vec::new(),
            ruleset_name: "RF-{app}-{env}".into(),
        }
    }

```

and in the three `Msg::ProfileSwitched` tests of `crates/illumio-tui/src/tui/update.rs`:

```rust
                        scope_keys: vec!["site".into()],
                        tier_key: "tier".into(),
                        ignore_keys: Vec::new(),
                    },
                ))),
            ),
```

with:

```rust
                        scope_keys: vec!["site".into()],
                        tier_key: "tier".into(),
                        ignore_keys: Vec::new(),
                        ruleset_name: "RF-{site}".into(),
                    },
                ))),
            ),
```

In `crates/illumio-tui/src/tui/update.rs`, replace:

```rust
                    scope_keys: vec!["site".into()],
                    tier_key: "tier".into(),
                    ignore_keys: vec!["os".into()],
                },
            ))),
        );
```

with:

```rust
                    scope_keys: vec!["site".into()],
                    tier_key: "tier".into(),
                    ignore_keys: vec!["os".into()],
                    ruleset_name: "RF-{site}".into(),
                },
            ))),
        );
```

In `crates/illumio-tui/src/tui/update.rs`, replace:

```rust
                    scope_keys: vec!["app".into(), "env".into()],
                    tier_key: "role".into(),
                    ignore_keys: Vec::new(),
                },
            ))),
        );
```

with:

```rust
                    scope_keys: vec!["app".into(), "env".into()],
                    tier_key: "role".into(),
                    ignore_keys: Vec::new(),
                    ruleset_name: "RF-{app}-{env}".into(),
                },
            ))),
        );
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p illumio-tui --lib model && cargo test -p illumio-tui --lib profile`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/illumio-tui/src/tui/model.rs crates/illumio-tui/src/tui/app.rs crates/illumio-tui/src/tui/update.rs
git commit -m "tui: complete_for (the result actually used), ruleset name in Limits

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

### Task 2: `GenView`: the generator's state

**Files:**
- Create: `crates/illumio-tui/src/tui/gen_view.rs`
- Modify: `crates/illumio-tui/src/tui/mod.rs`

**Interfaces:**
- Consumes: Plan 1 (`GenRow`, `RowKey`, `PeerKind`, `Proposal`, `Side`, `Stop`, `Target`, `GenActor::text`, `RowFlag` texts, `Proposal::counts`); `NameIndex::{label_text, all_services_href, service_name}`.
- Produces (`tui::gen_view`):

```rust
pub enum Drafts { Loading, Loaded, Failed(String) }
pub struct GenView {
    pub focus: Focus, pub window: String, pub seq: u64, pub rows: Vec<GenRow>, pub global: Stop,
    pub section: Side, pub cursor: [usize; 2], pub outbound: bool, pub picked: Option<String>,
    pub requests: u32, pub cached: u32, pub drafts_sent: bool, pub drafts: Drafts,
    pub proposal: Option<Proposal>, pub gap: Option<String>, pub requery: bool,
    pub all_services: bool, pub list: Option<u16>, pub planning: bool, pub error: Option<String>,
}
impl GenView {
    pub fn new(focus: Focus, window: String, seq: u64) -> Self;
    pub fn set_rows(&mut self, rows: Vec<GenRow>);          // checkboxes and overrides kept by key
    pub fn section_rows(&self, s: Side) -> Vec<usize>;
    pub fn selected(&self) -> Option<usize>;
    pub fn move_cursor(&mut self, delta: isize);
    pub fn switch_section(&mut self);
    pub fn toggle_outbound(&mut self);                      // focus direction in <-> both
    pub fn toggle(&mut self, scope_keys: &[String]) -> Result<(), String>;
    pub fn slide(&mut self, dir: isize);                    // the slider
    pub fn slide_row(&mut self, dir: isize);                // the row's own stop
    pub fn clear_row_stop(&mut self);
    pub fn pick_next(&mut self) -> Result<String, String>;  // R
    pub fn header_title(&self) -> String;  pub fn header_meta(&self) -> String;
    pub fn slider_line(&self) -> String;
    pub fn target_line(&self, drafts: Option<&[RuleSet]>, idx: &NameIndex) -> String;
    pub fn target_name(&self) -> String;   pub fn row_target(&self, row: &GenRow) -> Option<String>;
    pub fn footer(&self) -> String;        pub fn rule_list(&self, idx: &NameIndex) -> Vec<String>;
}
```

- [ ] **Step 1: Write the failing tests**

Create `crates/illumio-tui/src/tui/gen_view.rs` with its tests only (the code follows in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::fixture_flows_rich;
    use pce::traffic::{FocusDirection, parse_app};

    fn shop() -> Focus {
        Focus::new(parse_app("shop/prod"), vec!["app".into(), "env".into()])
            .with_direction(FocusDirection::In)
    }

    fn view() -> GenView {
        let idx = crate::tui::test_support::fixture_index();
        let mut v = GenView::new(shop(), "24h".into(), 1);
        v.set_rows(pce::generate::rows(
            &fixture_flows_rich(),
            &shop(),
            "role",
            &idx,
        ));
        v
    }

    #[test]
    fn checkboxes_and_overrides_survive_new_rows() {
        let mut v = view();
        v.move_cursor(2); // shop/dev
        v.toggle(&shop().keys).unwrap();
        v.slide_row(1);
        assert_eq!(v.rows[2].stop, Some(Stop::RolePorts));
        let idx = crate::tui::test_support::fixture_index();
        v.set_rows(pce::generate::rows(
            &fixture_flows_rich(),
            &shop(),
            "role",
            &idx,
        ));
        assert!(v.rows[2].checked, "kept by its key");
        assert_eq!(v.rows[2].stop, Some(Stop::RolePorts));
        v.clear_row_stop();
        assert_eq!(v.rows[2].stop, None);
    }

    #[test]
    fn the_slider_stops_at_its_ends_and_skips_all_stops_without_all_services() {
        let mut v = view();
        v.slide(-1);
        assert_eq!(v.global, Stop::AppAll);
        v.slide(-1);
        assert_eq!(v.global, Stop::AppAll);
        v.global = Stop::AppPorts;
        v.all_services = false;
        v.slide(-1);
        assert_eq!(v.global, Stop::AppPorts, "app·all is off");
        v.slide(1);
        v.slide(1);
        assert_eq!(v.global, Stop::RolePorts, "role·all is off");
        assert_eq!(
            v.slider_line(),
            "granularity  (app·all) ─ app·ports ─ [role·ports] ─ (role·all)"
        );
    }

    #[test]
    fn tab_reaches_outbound_only_when_it_is_shown() {
        let mut v = view();
        v.switch_section();
        assert_eq!(v.section, Side::Inbound);
        v.toggle_outbound();
        assert_eq!(v.focus.direction, FocusDirection::Both);
        v.switch_section();
        assert_eq!(v.section, Side::Outbound);
        assert_eq!(v.selected(), Some(4), "crm/prod, the first outbound row");
        v.toggle_outbound();
        assert_eq!(v.section, Side::Inbound);
    }

    #[test]
    fn an_uncheckable_row_says_why() {
        let mut v = view();
        v.toggle_outbound();
        v.section = Side::Outbound;
        v.rows[4].checkable = false;
        v.rows[4].flags = vec![pce::generate::RowFlag::NoScopeLabels];
        assert_eq!(
            v.toggle(&shop().keys).unwrap_err(),
            "crm/prod cannot be checked: no scope labels: no ruleset to write into"
        );
    }

    #[test]
    fn the_header_counts_the_sections_in_force() {
        let mut v = view();
        assert_eq!(
            v.header_title(),
            "Generate · shop/prod · last 24h · 8 flows · complete"
        );
        v.toggle_outbound();
        assert!(v.header_title().contains("10 flows"));
        v.gap = Some("incomplete".into());
        assert!(v.header_title().ends_with("· incomplete"));
        assert_eq!(v.header_meta(), "0 req · cached");
        v.requests = 1;
        assert_eq!(v.header_meta(), "1 req");
    }
}
```

In `crates/illumio-tui/src/tui/mod.rs`, declare it:

```rust
pub mod export;
pub mod filter;
pub mod form;
pub mod impact;
pub mod model;
pub mod msg;
```

with:

```rust
pub mod export;
pub mod filter;
pub mod form;
pub mod gen_view;
pub mod impact;
pub mod model;
pub mod msg;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p illumio-tui --lib gen_view`
Expected: FAIL to compile: "cannot find struct `GenView`".

- [ ] **Step 3: Write the implementation**

Above the tests in `crates/illumio-tui/src/tui/gen_view.rs`, add:

```rust
//! The policy generator's state (`G` / `:gen`): the app's rows with their
//! checkboxes and stops, the slider, the section and cursor, and the
//! proposal they lead to. Pure: `update` fills it, `ui::widgets::generator`
//! draws it. Nothing here makes a request.
use pce::NameIndex;
use pce::generate::{GenRow, PeerKind, Proposal, RowKey, Side, Stop, Target};
use pce::traffic::Focus;
use std::collections::BTreeMap;

/// The draft rulesets the proposal is checked against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drafts {
    Loading,
    Loaded,
    /// The load failed; `r` retries.
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct GenView {
    /// The app, `in` (or `both` with the outbound section on).
    pub focus: Focus,
    /// The traffic window, as the filter names it (`24h`).
    pub window: String,
    pub seq: u64,
    pub rows: Vec<GenRow>,
    /// The slider.
    pub global: Stop,
    pub section: Side,
    /// One cursor per section (inbound, outbound), into its rows.
    pub cursor: [usize; 2],
    /// `o`: the outbound section is shown and proposed.
    pub outbound: bool,
    /// The app ruleset `R` picked among several (href).
    pub picked: Option<String>,
    /// Requests this generator sent (the draft rulesets).
    pub requests: u32,
    /// Of those, the ones answered from the cache.
    pub cached: u32,
    /// The draft ruleset load was sent by this generator.
    pub drafts_sent: bool,
    pub drafts: Drafts,
    /// The rules the rows lead to; `None` until the draft rulesets are in.
    pub proposal: Option<Proposal>,
    /// Why the data may miss flows (the header's text); `None`: complete.
    pub gap: Option<String>,
    /// `r` can close the gap (a focused query); not when the focused
    /// result itself is capped.
    pub requery: bool,
    /// The PCE has an All Services object (the `·all` stops work).
    pub all_services: bool,
    /// `p`: the rule list is shown, scrolled to this line.
    pub list: Option<u16>,
    /// `Enter` sent a plan; its answer is on the way.
    pub planning: bool,
    /// The last plan's error (nothing was sent).
    pub error: Option<String>,
}

fn side_index(s: Side) -> usize {
    match s {
        Side::Inbound => 0,
        Side::Outbound => 1,
    }
}

impl GenView {
    pub fn new(focus: Focus, window: String, seq: u64) -> Self {
        Self {
            focus,
            window,
            seq,
            rows: Vec::new(),
            global: Stop::default(),
            section: Side::Inbound,
            cursor: [0; 2],
            outbound: false,
            picked: None,
            requests: 0,
            cached: 0,
            drafts_sent: false,
            drafts: Drafts::Loading,
            proposal: None,
            gap: None,
            requery: false,
            all_services: true,
            list: None,
            planning: false,
            error: None,
        }
    }

    /// New rows; checkboxes and overrides follow their row by key.
    pub fn set_rows(&mut self, rows: Vec<GenRow>) {
        let old: BTreeMap<RowKey, (bool, Option<Stop>)> = self
            .rows
            .iter()
            .map(|r| (r.key(), (r.checked, r.stop)))
            .collect();
        self.rows = rows;
        for r in &mut self.rows {
            if let Some((checked, stop)) = old.get(&r.key()) {
                r.checked = *checked && r.checkable;
                r.stop = *stop;
            }
        }
        for s in [Side::Inbound, Side::Outbound] {
            let n = self.section_rows(s).len();
            let c = &mut self.cursor[side_index(s)];
            *c = (*c).min(n.saturating_sub(1));
        }
    }

    /// The indexes (into `rows`) of one section's rows.
    pub fn section_rows(&self, s: Side) -> Vec<usize> {
        (0..self.rows.len())
            .filter(|i| self.rows[*i].side == s)
            .collect()
    }

    /// The row under the cursor (an index into `rows`).
    pub fn selected(&self) -> Option<usize> {
        self.section_rows(self.section)
            .get(self.cursor[side_index(self.section)])
            .copied()
    }

    /// `j` / `k`, clamped to the section.
    pub fn move_cursor(&mut self, delta: isize) {
        let n = self.section_rows(self.section).len();
        let c = &mut self.cursor[side_index(self.section)];
        *c = c.saturating_add_signed(delta).min(n.saturating_sub(1));
    }

    /// `Tab`: the other section, when it is shown.
    pub fn switch_section(&mut self) {
        self.section = match self.section {
            Side::Inbound if self.outbound => Side::Outbound,
            _ => Side::Inbound,
        };
    }

    /// `o`: the outbound section on or off; the focus follows (`both`
    /// needs the out side's flows too).
    pub fn toggle_outbound(&mut self) {
        self.outbound = !self.outbound;
        self.focus.direction = if self.outbound {
            pce::traffic::FocusDirection::Both
        } else {
            pce::traffic::FocusDirection::In
        };
        if !self.outbound {
            self.section = Side::Inbound;
        }
    }

    /// `Space`; the error says why a row cannot be checked.
    pub fn toggle(&mut self, scope_keys: &[String]) -> Result<(), String> {
        let i = self.selected().ok_or("no row here")?;
        let r = &mut self.rows[i];
        if !r.checkable {
            let why = r
                .flag_texts(r.effective(self.global), scope_keys)
                .into_iter()
                .next()
                .unwrap_or_else(|| "this row is not proposed".into());
            return Err(format!("{} cannot be checked: {why}", r.peer_text()));
        }
        r.checked = !r.checked;
        Ok(())
    }

    /// The next stop from `from` in `dir` (-1, +1), skipping the `·all`
    /// stops without an All Services object; `None` at the end.
    fn step(&self, from: Stop, dir: isize) -> Option<Stop> {
        let mut s = from;
        loop {
            let next = if dir < 0 { s.prev() } else { s.next() };
            if next == s {
                return None;
            }
            if self.all_services || !next.is_all() {
                return Some(next);
            }
            s = next;
        }
    }

    /// `←` / `→`: the slider.
    pub fn slide(&mut self, dir: isize) {
        if let Some(s) = self.step(self.global, dir) {
            self.global = s;
        }
    }

    /// `<` / `>`: the selected row's own stop, from the one in force.
    pub fn slide_row(&mut self, dir: isize) {
        let Some(i) = self.selected() else {
            return;
        };
        let from = self.rows[i].effective(self.global);
        if let Some(s) = self.step(from, dir) {
            self.rows[i].stop = Some(s);
        }
    }

    /// `=`: the selected row follows the slider again.
    pub fn clear_row_stop(&mut self) {
        if let Some(i) = self.selected() {
            self.rows[i].stop = None;
        }
    }

    /// `R`: the next of several rulesets covering the app.
    pub fn pick_next(&mut self) -> Result<String, String> {
        let Some(Target::Ambiguous(c)) = self.proposal.as_ref().and_then(|p| p.targets.first())
        else {
            return Err("one ruleset or none covers the app: nothing to pick".into());
        };
        let at = self
            .picked
            .as_ref()
            .and_then(|h| c.iter().position(|x| &x.href == h));
        let next = &c[at.map_or(0, |i| (i + 1) % c.len())];
        self.picked = Some(next.href.clone());
        Ok(next.name.clone())
    }

    /// The rows of the sections in force.
    fn in_force(&self) -> impl Iterator<Item = &GenRow> {
        self.rows
            .iter()
            .filter(|r| r.side == Side::Inbound || self.outbound)
    }

    /// `Generate · shop/prod · last 24h · 1310 flows · complete`.
    pub fn header_title(&self) -> String {
        let flows: u64 = self.in_force().map(|r| r.flows).sum();
        format!(
            "Generate · {} · last {} · {flows} flows · {}",
            self.focus.app_text(),
            self.window,
            if self.gap.is_some() {
                "incomplete"
            } else {
                "complete"
            }
        )
    }

    /// `1 req · cached`, `0 req · cached`.
    pub fn header_meta(&self) -> String {
        let mut s = format!("{} req", self.requests);
        if self.requests == 0 || self.cached >= self.requests {
            s.push_str(" · cached");
        }
        s
    }

    /// The slider line: the stop in force in brackets.
    pub fn slider_line(&self) -> String {
        let stops: Vec<String> = Stop::ALL
            .iter()
            .map(|s| {
                let t = if !self.all_services && s.is_all() {
                    format!("({})", s.text())
                } else {
                    s.text().to_string()
                };
                if *s == self.global {
                    format!("[{t}]")
                } else {
                    t
                }
            })
            .collect();
        format!("granularity  {}", stops.join(" ─ "))
    }

    /// `target  Shop-Prod (draft, 2 rules)`, `target  new ruleset
    /// RF-shop-prod, scoped app=shop, env=prod`, `3 rulesets cover
    /// shop/prod: R picks`, `loading rulesets`.
    pub fn target_line(&self, drafts: Option<&[pce::models::RuleSet]>, idx: &NameIndex) -> String {
        let t = match (&self.drafts, &self.proposal) {
            (Drafts::Failed(e), _) => format!("rulesets: {e} · r retries"),
            (_, None) => "loading rulesets".into(),
            (_, Some(p)) => match p.targets.first() {
                Some(Target::Existing { href, name }) => {
                    let rules = drafts
                        .and_then(|d| d.iter().find(|r| &r.href == href))
                        .map_or(0, |r| r.rules.len());
                    format!("{name} (draft, {rules} rules)")
                }
                Some(Target::Create(d)) => {
                    let scope: Vec<String> = d
                        .scopes
                        .iter()
                        .flatten()
                        .map(|a| idx.label_text(a.href.as_deref().unwrap_or_default()))
                        .collect();
                    format!("new ruleset {}, scoped {}", d.name, scope.join(", "))
                }
                Some(Target::Ambiguous(c)) => {
                    format!(
                        "{} rulesets cover {}: R picks",
                        c.len(),
                        self.focus.app_text()
                    )
                }
                Some(Target::None(why)) => why.clone(),
                None => "no target".into(),
            },
        };
        format!("target  {t}")
    }

    /// The name a section's title gives the app's ruleset.
    pub fn target_name(&self) -> String {
        self.proposal
            .as_ref()
            .and_then(|p| p.targets.first())
            .map_or_else(|| self.focus.app_text(), Target::name)
    }

    /// An outbound row's target: the peer's ruleset (or the app's for an IP).
    pub fn row_target(&self, row: &GenRow) -> Option<String> {
        let p = self.proposal.as_ref()?;
        let PeerKind::App(values) = &row.peer else {
            return (row.side == Side::Outbound).then(|| self.target_name());
        };
        if row.side != Side::Outbound {
            return None;
        }
        let i = p.scopes.iter().position(|s| &s.values == values)?;
        Some(p.targets.get(i)?.name())
    }

    /// The footer: what the proposal holds and covers, or the plan's error.
    pub fn footer(&self) -> String {
        if let Some(e) = &self.error {
            return format!("plan failed, nothing sent: {e}");
        }
        if self.planning {
            return "planning: reading the drafts fresh…".into();
        }
        let flows: u64 = self.in_force().map(|r| r.flows).sum();
        if flows == 0 {
            return "no flows in the window: nothing to propose".into();
        }
        let Some(p) = &self.proposal else {
            return "rules follow when the draft rulesets arrive".into();
        };
        let c = p.counts();
        format!(
            "{} rules ({} new · {} add services · {} exist) · covers {} of {} flows · {} not",
            c.rules,
            c.create,
            c.add_services,
            c.exists,
            p.coverage.covered,
            p.coverage.covered + p.coverage.uncovered,
            p.coverage.uncovered
        )
    }

    /// `p`: one block per write, as the confirm will show them, then what
    /// is not covered and why.
    pub fn rule_list(&self, idx: &NameIndex) -> Vec<String> {
        use pce::generate::RuleAction;
        let Some(p) = &self.proposal else {
            return vec!["the draft rulesets are not loaded yet".into()];
        };
        let mut out = Vec::new();
        for d in &p.ip_lists {
            let ranges: Vec<String> = d.ranges.iter().map(|r| r.display()).collect();
            out.push(format!("new ip_list {} ({})", d.name, ranges.join(", ")));
        }
        for t in &p.targets {
            if let Target::Create(d) = t {
                out.push(format!("new rule_set {}", d.name));
            }
        }
        for (i, r) in p.rules.iter().enumerate() {
            let side = |a: &[pce::generate::GenActor]| {
                a.iter()
                    .map(|x| x.text(p, idx))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let services: Vec<String> = r
                .services
                .iter()
                .map(|s| match s {
                    pce::write::ServiceRef::Href(h)
                        if Some(h.as_str()) == idx.all_services_href() =>
                    {
                        "all services".to_string()
                    }
                    pce::write::ServiceRef::Href(h) => idx.service_name(h).unwrap_or(h).to_string(),
                    pce::write::ServiceRef::Port(x) => x.display(),
                })
                .collect();
            let what = match &r.action {
                RuleAction::Create => "create".to_string(),
                RuleAction::AddServices { rule_href, add } => format!(
                    "add {} to {rule_href}",
                    add.iter()
                        .map(|s| s.display())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                RuleAction::Exists { rule_href } => format!("exists: {rule_href}"),
            };
            out.push(format!(
                "{}. {what} · {}: {} → {} on {}{}",
                i + 1,
                p.targets[r.target].name(),
                side(&r.consumers),
                side(&r.providers),
                services.join(", "),
                if r.unscoped_consumers {
                    " · unscoped"
                } else {
                    ""
                }
            ));
            out.push(format!("   {}", r.description));
        }
        let c = &p.coverage;
        out.push(format!(
            "not covered: {} unchecked · {} folded IPs · {} unsupported proto · {} broadcast/multicast",
            c.unchecked, c.folded_ips, c.unsupported_proto, c.broadcast_multicast
        ));
        out
    }
}

```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p illumio-tui --lib gen_view`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/illumio-tui/src/tui/gen_view.rs crates/illumio-tui/src/tui/mod.rs
git commit -m "tui: GenView, the generator's state

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

### Task 3: The generator's transitions: open, keys, rebuilds, `r`, `Enter`, the batch's return

**Files:**
- Create: `crates/illumio-tui/src/tui/update/generator.rs`
- Create: `crates/illumio-tui/src/tui/ui/widgets/generator.rs`
- Modify: `crates/illumio-tui/src/tui/update.rs`
- Modify: `crates/illumio-tui/src/tui/commands.rs`
- Modify: `crates/illumio-tui/src/tui/model.rs`
- Modify: `crates/illumio-tui/src/tui/app_picker.rs`
- Modify: `crates/illumio-tui/src/tui/ui/widgets/mod.rs`
- Modify: `crates/illumio-tui/src/tui/ui/mod.rs`
- Modify: `crates/illumio-tui/src/tui/ui/widgets/status.rs`

**Interfaces:**
- Consumes: Tasks 1-2; Plan 1 (`rows`, `propose`, `GenInput`, `keys_refusal`); Plan 2 (`Cmd::PlanBatch`, `Msg::BatchPlanned`, `BatchInput`, `batch_lands`, `batch_plan_failed`, `batch_closed`, `batch_tests`); `load_form_drafts`, `start_focused`, `writes_refused`, `open_app_picker`, `TrafficState::{flows_for, set_focus, loaded_apps}`, `pce::traffic::{app_labels, closest, parse_app}`.
- Produces:

```rust
// tui::model
Overlay::Generator
PendingAction::GenRequery(TrafficFilter, pce::traffic::Focus)
pub struct Model { /* … */ pub gen_view: Option<GenView>, pub gen_from_app: bool, pub gen_seq: u64 }
// tui::app_picker
AppPickerPurpose::Generator
// tui::commands
Command::Gen(Option<String>)                      // :gen [<app>]
// tui::update::generator (child module of update)
pub(super) fn open_gen(model: &mut Model, app: GroupKey, from_app: bool) -> Vec<Cmd>;
pub(super) fn ensure_drafts(model: &mut Model) -> Vec<Cmd>;
pub(super) fn rebuild_gen(model: &mut Model);
pub(super) fn regen(model: &mut Model);
pub(super) fn gen_drafts_answer(model: &mut Model, outcome: Result<bool, &str>);
pub(super) fn gen_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd>;
pub(super) fn gen_requery(model: &mut Model, filter: TrafficFilter, focus: Focus) -> Vec<Cmd>;
// tui::ui::widgets::generator
pub fn render_generator(model: &Model, f: &mut Frame, area: Rect);   // drawn here, snapshotted in Task 4
```

Keys: `j`/`k` rows, `Tab` section (outbound when shown), `Space` checkbox, `←`/`→` (`h`/`l`) the slider, `<`/`>` the row's own stop, `=` drops it, `o` the outbound section, `R` the next covering ruleset, `p` the rule list (`j`/`k` scroll, `Esc`/`p` back), `r` reload or re-query, `Enter` plan, `Esc`/`q` back (to the app overlay when opened from it). `Enter` is refused, with the reason, for writes off or in flight, incomplete data, no row checked, drafts failed or missing, an ambiguous target, nothing to write.

- [ ] **Step 1: Write the failing tests**

Create `crates/illumio-tui/src/tui/update/generator.rs` with its tests only (the code follows in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::super::update;
    use crate::tui::commands::Command;
    use crate::tui::gen_view::{Drafts, GenView};
    use crate::tui::model::{Model, Overlay, PendingAction, WritesState};
    use crate::tui::msg::{Cmd, FocusedOutcome, Msg, QueryOutcome};
    use crate::tui::test_support::{model_with_window, with_policy};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use pce::CacheMode;
    use pce::generate::{Side, Stop};
    use pce::models::PolicyScope;

    fn key(c: char) -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    fn kc(code: KeyCode) -> Msg {
        Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn view(m: &Model) -> &GenView {
        m.gen_view.as_ref().expect("the generator's state")
    }

    /// The rich flows of a known 24h window, the rulesets loaded.
    fn loaded() -> Model {
        with_policy(model_with_window("24h"))
    }

    fn open_shop(m: &mut Model) -> Vec<Cmd> {
        update(m, Msg::Command(Command::Gen(Some("shop/prod".into()))))
    }

    #[test]
    fn g_in_the_app_overlay_opens_it_for_that_app_with_nothing_sent() {
        let mut m = loaded();
        update(&mut m, Msg::Command(Command::App(Some("shop/prod".into()))));
        assert_eq!(update(&mut m, key('G')), Vec::new());
        assert_eq!(m.overlay, Overlay::Generator);
        assert!(m.gen_from_app);
        let v = view(&m);
        assert_eq!(v.focus.app_text(), "shop/prod");
        assert_eq!(v.rows.len(), 6);
        assert_eq!(v.header_meta(), "0 req · cached");
        assert_eq!(v.drafts, Drafts::Loaded);
        assert_eq!(v.proposal.as_ref().unwrap().rules.len(), 2);
        assert_eq!(update(&mut m, kc(KeyCode::Esc)), Vec::new());
        assert_eq!(m.overlay, Overlay::App, "back where it came from");
    }

    #[test]
    fn opening_without_drafts_costs_one_counted_request() {
        let mut m = model_with_window("24h");
        let cmds = open_shop(&mut m);
        assert_eq!(
            cmds,
            vec![Cmd::LoadRuleSets {
                scope: PolicyScope::Draft,
                mode: CacheMode::Use
            }]
        );
        assert_eq!(view(&m).header_meta(), "1 req");
        assert!(view(&m).proposal.is_none());
        assert!(!m.gen_from_app);
        let rs: Vec<pce::models::RuleSet> =
            serde_json::from_str(include_str!("../../../../../fixtures/rule_sets.json")).unwrap();
        update(
            &mut m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Draft,
                result: Ok((rs, 3, true)),
            },
        );
        assert_eq!(view(&m).header_meta(), "1 req · cached");
        assert!(view(&m).proposal.is_some());
        assert_eq!(update(&mut m, kc(KeyCode::Esc)), Vec::new());
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(open_shop(&mut m), Vec::new(), "reopening sends nothing");
    }

    #[test]
    fn keys_other_than_the_scope_keys_are_refused() {
        let mut m = loaded();
        m.traffic.keys = vec!["app".into(), "env".into(), "role".into()];
        assert_eq!(
            update(
                &mut m,
                Msg::Command(Command::Gen(Some("shop/prod/web".into())))
            ),
            Vec::new()
        );
        assert_eq!(
            m.status.text,
            "the generator needs g = app,env (the scope keys)"
        );
        assert_eq!(m.overlay, Overlay::None);
        assert!(m.gen_view.is_none());
    }

    #[test]
    fn the_slider_rows_and_sections_change_the_proposal_without_requests() {
        let mut m = loaded();
        open_shop(&mut m);
        for msg in [
            kc(KeyCode::Left),
            key('j'),
            key('j'),
            key(' '),
            key('>'),
            key('o'),
            key('p'),
        ] {
            assert_eq!(update(&mut m, msg), Vec::new());
        }
        let v = view(&m);
        assert_eq!(v.global, Stop::AppAll);
        assert!(v.rows[2].checked, "shop/dev checked");
        assert_eq!(v.rows[2].stop, Some(Stop::AppPorts));
        assert!(v.outbound);
        assert_eq!(v.list, Some(0));
        let p = v.proposal.as_ref().unwrap();
        assert_eq!(p.counts().rules, 4, "intra, hr, shop/dev, crm (outbound)");
        update(&mut m, kc(KeyCode::Esc));
        assert_eq!(view(&m).list, None, "Esc leaves the list first");
        assert_eq!(m.overlay, Overlay::Generator);
        update(&mut m, kc(KeyCode::Tab));
        assert_eq!(view(&m).section, Side::Outbound);
        update(&mut m, key('='));
        update(&mut m, key('R'));
        assert_eq!(
            m.status.text,
            "one ruleset or none covers the app: nothing to pick"
        );
    }

    #[test]
    fn incomplete_data_refuses_enter_and_r_runs_the_focused_query() {
        let mut m = loaded();
        m.writes = WritesState::on();
        m.traffic.data.as_mut().unwrap().truncated = true;
        open_shop(&mut m);
        assert_eq!(
            view(&m).gap.as_deref(),
            Some("incomplete: r re-queries shop/prod (1 query)")
        );
        assert_eq!(update(&mut m, kc(KeyCode::Enter)), Vec::new());
        assert_eq!(
            m.status.text,
            "incomplete: r re-queries shop/prod (1 query): no rules from incomplete data"
        );
        let cmds = update(&mut m, key('r'));
        assert!(
            matches!(&cmds[..], [Cmd::RunFocused { focus, .. }] if focus.app_text() == "shop/prod"),
            "{cmds:?}"
        );
        assert_eq!(m.overlay, Overlay::Generator);
        // The focused answer lands: the generator rebuilds, complete.
        let Cmd::RunFocused {
            seq, filter, focus, ..
        } = cmds[0].clone()
        else {
            unreachable!()
        };
        let flows = m.traffic.flows_for(&focus).to_vec();
        update(
            &mut m,
            Msg::FocusedDone {
                seq,
                result: Ok(FocusedOutcome {
                    outcome: QueryOutcome {
                        flows,
                        truncated: false,
                        incomplete: false,
                        elapsed: std::time::Duration::ZERO,
                        hit: false,
                        filter,
                        bounds: None,
                        hrefs: Vec::new(),
                    },
                    focus,
                    queries: 1,
                }),
            },
        );
        assert_eq!(view(&m).gap, None, "complete now");
    }

    #[test]
    fn a_capped_focused_result_says_so_and_r_does_not_repeat_it() {
        let mut m = loaded();
        m.traffic.data.as_mut().unwrap().truncated = true;
        open_shop(&mut m);
        let focus = view(&m).focus.clone();
        let data = crate::tui::model::TrafficData {
            flows: m.traffic.flows_for(&focus).to_vec(),
            truncated: true,
            incomplete: false,
            elapsed: std::time::Duration::ZERO,
            cache_hit: false,
            fetched_at: std::time::Instant::now(),
            filter: m.traffic.filter.clone(),
            range: None,
            hrefs: Vec::new(),
            draft: None,
        };
        m.traffic.apply_focused(focus, 1, data);
        assert!(
            !m.traffic.partial_for(&view(&m).focus),
            "partial_for misses it"
        );
        super::rebuild_gen(&mut m);
        assert_eq!(
            view(&m).gap.as_deref(),
            Some("still capped at 1000: B raises the cap, or narrow the window")
        );
        assert_eq!(update(&mut m, key('r')), Vec::new());
    }

    #[test]
    fn r_past_the_cost_threshold_asks_and_comes_back() {
        let mut m = loaded();
        m.traffic.data.as_mut().unwrap().truncated = true;
        m.traffic.filter.window = "30d".into();
        open_shop(&mut m);
        assert_eq!(update(&mut m, key('r')), Vec::new());
        assert!(matches!(
            &m.overlay,
            Overlay::Confirm {
                action: PendingAction::GenRequery(..),
                ..
            }
        ));
        assert_eq!(update(&mut m, key('n')), Vec::new());
        assert_eq!(m.overlay, Overlay::Generator);
        update(&mut m, key('r'));
        let cmds = update(&mut m, key('y'));
        assert!(matches!(&cmds[..], [Cmd::RunFocused { .. }]), "{cmds:?}");
        assert_eq!(m.overlay, Overlay::Generator);
    }

    #[test]
    fn enter_with_writes_off_is_refused() {
        let mut m = loaded();
        open_shop(&mut m);
        assert_eq!(update(&mut m, kc(KeyCode::Enter)), Vec::new());
        assert_eq!(m.status.text, crate::tui::model::WRITES_OFF);
    }

    #[test]
    fn enter_plans_once_and_a_y_typed_meanwhile_confirms_nothing() {
        let mut m = loaded();
        m.writes = WritesState::on();
        open_shop(&mut m);
        let cmds = update(&mut m, kc(KeyCode::Enter));
        let [Cmd::PlanBatch { input, .. }] = &cmds[..] else {
            panic!("{cmds:?}")
        };
        assert_eq!(input.ruleset_name, "RF-{app}-{env}");
        assert_eq!(input.proposal.rules.len(), 2);
        assert!(view(&m).planning && m.writes.in_flight);
        assert_eq!(update(&mut m, key('y')), Vec::new());
        assert_eq!(update(&mut m, kc(KeyCode::Enter)), Vec::new(), "in flight");
        assert_eq!(m.status.text, "a write is already in progress");
        update(
            &mut m,
            Msg::BatchPlanned(Ok(crate::tui::test_support::batch_plan())),
        );
        assert!(matches!(m.overlay, Overlay::ConfirmBatch { .. }));
        update(&mut m, key('n'));
        assert_eq!(m.overlay, Overlay::Generator);
    }

    #[test]
    fn a_failed_plan_keeps_the_generator_and_shows_why() {
        let mut m = loaded();
        m.writes = WritesState::on();
        open_shop(&mut m);
        update(&mut m, kc(KeyCode::Enter));
        update(
            &mut m,
            Msg::BatchPlanned(Err("too many draft rulesets".into())),
        );
        assert_eq!(m.overlay, Overlay::Generator);
        assert_eq!(
            view(&m).footer(),
            "plan failed, nothing sent: too many draft rulesets"
        );
        assert!(!view(&m).planning);
    }

    #[test]
    fn a_plan_landing_after_esc_is_discarded() {
        let mut m = loaded();
        m.writes = WritesState::on();
        open_shop(&mut m);
        update(&mut m, kc(KeyCode::Enter));
        update(&mut m, kc(KeyCode::Esc));
        update(
            &mut m,
            Msg::BatchPlanned(Ok(crate::tui::test_support::batch_plan())),
        );
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.status.text, "requests discarded, nothing sent");
    }

    #[test]
    fn nothing_checked_is_refused() {
        let mut m = loaded();
        m.writes = WritesState::on();
        open_shop(&mut m);
        update(&mut m, key(' ')); // intra off
        update(&mut m, key('j'));
        update(&mut m, key(' ')); // hr off
        assert_eq!(update(&mut m, kc(KeyCode::Enter)), Vec::new());
        assert_eq!(m.status.text, "no row is checked: Space checks one");
    }

    #[test]
    fn gen_alone_opens_the_picker_for_the_generator() {
        let mut m = loaded();
        update(&mut m, Msg::Command(Command::Gen(None)));
        let Overlay::AppPicker(p) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(
            p.purpose,
            crate::tui::app_picker::AppPickerPurpose::Generator
        );
        for c in "shop/prod".chars() {
            update(&mut m, key(c));
        }
        update(&mut m, kc(KeyCode::Enter));
        assert_eq!(m.overlay, Overlay::Generator);
    }
}
```

In `crates/illumio-tui/src/tui/update.rs`, declare the child module (the file lives in `src/tui/update/`):

```rust
use pce::{CacheMode, RuleKind, WriteMethod};
use std::time::Instant;

/// The status and header note for a result a supercluster region did not
/// answer.
pub const INCOMPLETE: &str = " · incomplete: a PCE region did not respond";
```

with:

```rust
use pce::{CacheMode, RuleKind, WriteMethod};
use std::time::Instant;

mod generator;

/// The status and header note for a result a supercluster region did not
/// answer.
pub const INCOMPLETE: &str = " · incomplete: a PCE region did not respond";
```

Plan 2's batch tests now run with the generator planning, and the confirm returns to it. In the `batch_tests` module, replace `armed`:

```rust
        vec![ObjectKind::IpList, ObjectKind::RuleSet, ObjectKind::Rule]
    }

    fn armed() -> Model {
        let mut m = model_with_flows();
        m.writes = crate::tui::model::WritesState::on();
        m.writes.in_flight = true; // the plan is on its way
        m
    }

```

with:

```rust
        vec![ObjectKind::IpList, ObjectKind::RuleSet, ObjectKind::Rule]
    }

    /// Writes on, the draft rulesets loaded, the generator open for
    /// shop/prod and planning.
    fn armed() -> Model {
        let mut m = crate::tui::test_support::with_policy(model_with_flows());
        m.writes = crate::tui::model::WritesState::on();
        m.writes.in_flight = true; // the plan is on its way
        let focus = pce::traffic::Focus::new(
            pce::traffic::parse_app("shop/prod"),
            vec!["app".into(), "env".into()],
        );
        let mut v = crate::tui::gen_view::GenView::new(focus, "24h".into(), 1);
        v.planning = true;
        m.gen_view = Some(v);
        m.overlay = Overlay::Generator;
        m
    }

```

In `crates/illumio-tui/src/tui/update.rs`, replace:

```rust
        let mut m = armed();
        update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
        assert_eq!(update(&mut m, key('n')), Vec::new());
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.status.text, "cancelled, nothing sent");
    }

```

with:

```rust
        let mut m = armed();
        update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
        assert_eq!(update(&mut m, key('n')), Vec::new());
        assert_eq!(m.overlay, Overlay::Generator, "back to the generator");
        assert_eq!(m.status.text, "cancelled, nothing sent");
    }

```

In `crates/illumio-tui/src/tui/update.rs`, replace:

```rust
            1,
            "one redraft per batch"
        );
        assert_eq!(m.overlay, Overlay::None);
    }

    #[test]
```

with:

```rust
            1,
            "one redraft per batch"
        );
        assert_eq!(m.overlay, Overlay::Generator, "the generator reopens");
    }

    #[test]
```

In the tests of `crates/illumio-tui/src/tui/commands.rs`:

```rust
    #[test]
    fn parses_every_documented_command() {
        assert_eq!(parse("app").unwrap(), Command::App(None));
        assert_eq!(
            parse("app shop/prod").unwrap(),
            Command::App(Some("shop/prod".into()))
```

with:

```rust
    #[test]
    fn parses_every_documented_command() {
        assert_eq!(parse("app").unwrap(), Command::App(None));
        assert_eq!(parse("gen").unwrap(), Command::Gen(None));
        assert_eq!(
            parse("gen shop/prod").unwrap(),
            Command::Gen(Some("shop/prod".into()))
        );
        assert_eq!(
            parse("app shop/prod").unwrap(),
            Command::App(Some("shop/prod".into()))
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p illumio-tui --lib generator`
Expected: FAIL to compile: "no variant named `Gen`", "no variant named `Generator`", "cannot find function `open_gen`".

- [ ] **Step 3: Write the state, the command and the picker purpose**

In `crates/illumio-tui/src/tui/model.rs`:

```rust
    AppPicker(AppPicker),
    /// `A` / `:app`: the app overlay; its state is `Model::app_view`.
    App,
    /// `f` on the Labels tab: the label keys it shows.
    LabelKeys(KeyPicker),
    /// The selected policy rule in full.
```

with:

```rust
    AppPicker(AppPicker),
    /// `A` / `:app`: the app overlay; its state is `Model::app_view`.
    App,
    /// `G` / `:gen`: the policy generator; its state is `Model::gen_view`.
    Generator,
    /// `f` on the Labels tab: the label keys it shows.
    LabelKeys(KeyPicker),
    /// The selected policy rule in full.
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
            (Overlay::None, Overlay::None)
            | (Overlay::Help, Overlay::Help)
            | (Overlay::Pending, Overlay::Pending)
            | (Overlay::App, Overlay::App) => true,
            (Overlay::Prompt { input: a }, Overlay::Prompt { input: b })
            | (Overlay::Error(a), Overlay::Error(b)) => a == b,
            (Overlay::GroupKeys(a), Overlay::GroupKeys(b))
```

with:

```rust
            (Overlay::None, Overlay::None)
            | (Overlay::Help, Overlay::Help)
            | (Overlay::Pending, Overlay::Pending)
            | (Overlay::App, Overlay::App)
            | (Overlay::Generator, Overlay::Generator) => true,
            (Overlay::Prompt { input: a }, Overlay::Prompt { input: b })
            | (Overlay::Error(a), Overlay::Error(b)) => a == b,
            (Overlay::GroupKeys(a), Overlay::GroupKeys(b))
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
    },
    /// `r` in the app overlay: the workloads and the rulesets again, past the cache.
    ReloadApp,
}

#[derive(Debug)]
```

with:

```rust
    },
    /// `r` in the app overlay: the workloads and the rulesets again, past the cache.
    ReloadApp,
    /// `r` in the generator on incomplete data, past a cost threshold: the
    /// focused query for its app; `y` and `n` go back to the generator.
    GenRequery(TrafficFilter, pce::traffic::Focus),
}

#[derive(Debug)]
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
    pub form_from_app: bool,
    /// The last app overlay's sequence number: answers of older ones are dropped.
    pub app_seq: u64,
    pub connection: Connection,
    pub stats: StatsSnapshot,
    pub size: (u16, u16),
```

with:

```rust
    pub form_from_app: bool,
    /// The last app overlay's sequence number: answers of older ones are dropped.
    pub app_seq: u64,
    /// The generator's state while it is open (and after, until the next
    /// `G` for another app: reopening the same app keeps it).
    pub gen_view: Option<crate::tui::gen_view::GenView>,
    /// The generator was opened from the app overlay: `Esc` returns there.
    pub gen_from_app: bool,
    /// The last generator's sequence number.
    pub gen_seq: u64,
    pub connection: Connection,
    pub stats: StatsSnapshot,
    pub size: (u16, u16),
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
            app_view: None,
            form_from_app: false,
            app_seq: 0,
            connection: Connection::Connecting,
            stats: StatsSnapshot { calls: 0, bytes: 0 },
            size,
```

with:

```rust
            app_view: None,
            form_from_app: false,
            app_seq: 0,
            gen_view: None,
            gen_from_app: false,
            gen_seq: 0,
            connection: Connection::Connecting,
            stats: StatsSnapshot { calls: 0, bytes: 0 },
            size,
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
        self.events = EventsState::new(min);
        self.app_view = None;
        self.form_from_app = false;
        self.stats = StatsSnapshot { calls: 0, bytes: 0 };
        self.busy = 0;
        self.overlay = Overlay::None;
```

with:

```rust
        self.events = EventsState::new(min);
        self.app_view = None;
        self.form_from_app = false;
        self.gen_view = None;
        self.gen_from_app = false;
        self.stats = StatsSnapshot { calls: 0, bytes: 0 };
        self.busy = 0;
        self.overlay = Overlay::None;
```

In `crates/illumio-tui/src/tui/commands.rs`:

```rust
    Focus(Option<String>),
    /// `:app <app>` opens the app overlay; `:app` alone opens the picker.
    App(Option<String>),
    Profile(String),
    /// `:writes on` / `:writes off`; bare `:writes` shows the state.
    Writes(Option<bool>),
```

with:

```rust
    Focus(Option<String>),
    /// `:app <app>` opens the app overlay; `:app` alone opens the picker.
    App(Option<String>),
    /// `:gen <app>` opens the policy generator; `:gen` alone opens the picker.
    Gen(Option<String>),
    Profile(String),
    /// `:writes on` / `:writes off`; bare `:writes` shows the state.
    Writes(Option<bool>),
```

In `crates/illumio-tui/src/tui/commands.rs`, replace:

```rust
        ),
        "focus" => Ok(Command::Focus((!arg.is_empty()).then(|| arg.to_string()))),
        "app" => Ok(Command::App((!arg.is_empty()).then(|| arg.to_string()))),
        "window" => with_arg(Command::Window, "window <duration>  (e.g. 1h, 24h, 7d)"),
        "profile" => with_arg(Command::Profile, "profile <name>"),
        "writes" => match arg {
```

with:

```rust
        ),
        "focus" => Ok(Command::Focus((!arg.is_empty()).then(|| arg.to_string()))),
        "app" => Ok(Command::App((!arg.is_empty()).then(|| arg.to_string()))),
        "gen" => Ok(Command::Gen((!arg.is_empty()).then(|| arg.to_string()))),
        "window" => with_arg(Command::Window, "window <duration>  (e.g. 1h, 24h, 7d)"),
        "profile" => with_arg(Command::Profile, "profile <name>"),
        "writes" => match arg {
```

In `crates/illumio-tui/src/tui/app_picker.rs`:

```rust
    Focus,
    /// `A` / `:app`: open the app overlay for it.
    Overlay,
}

#[derive(Debug, Clone, PartialEq, Eq)]
```

with:

```rust
    Focus,
    /// `A` / `:app`: open the app overlay for it.
    Overlay,
    /// `:gen`: open the policy generator for it.
    Generator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
```

- [ ] **Step 4: Write the transitions**

Above the tests in `crates/illumio-tui/src/tui/update/generator.rs`, add:

```rust
//! The generator's transitions (`G` / `:gen`): opening it, its keys, its
//! rebuilds when traffic or rulesets land, and `Enter`'s plan. Pure; the
//! only requests are the cached draft rulesets on open, the focused
//! re-query on `r` (through the cost guard) and the plan on `Enter`.
use super::{
    Cmd, Model, Overlay, PendingAction, StatusLine, load_form_drafts, start_focused, writes_refused,
};
use crate::tui::gen_view::{Drafts, GenView};
use crate::tui::model::Gap;
use crossterm::event::{KeyCode, KeyEvent};
use pce::CacheMode;
use pce::generate::{GenInput, Side, keys_refusal, propose, rows};
use pce::traffic::{Focus, FocusDirection, queries_text};
use pce::write::BatchInput;

/// Opens the generator for `app` under the keys in force. Reopening the
/// same app keeps its rows, checkboxes and slider. The only request: the
/// draft rulesets when no screen or form holds them (cached, counted).
pub(super) fn open_gen(model: &mut Model, app: pce::traffic::GroupKey, from_app: bool) -> Vec<Cmd> {
    let keys = model.traffic.keys.clone();
    if let Some(e) = keys_refusal(&keys, &model.limits.scope_keys) {
        model.status = StatusLine::warn(e);
        return Vec::new();
    }
    let focus = Focus::new(app, keys).with_direction(FocusDirection::In);
    if !focus.is_label_group() {
        model.status = StatusLine::warn(format!(
            "{} is not an app of labels: the generator scopes rules by them",
            focus.app_text()
        ));
        return Vec::new();
    }
    let Some(idx) = model.index().cloned() else {
        model.status = StatusLine::info("the name index is not loaded yet");
        return Vec::new();
    };
    let window = model.traffic.filter.window.clone();
    let mut v = match model.gen_view.take() {
        Some(mut v) if v.focus.same_app(&focus) => {
            v.window = window;
            v
        }
        _ => {
            model.gen_seq += 1;
            GenView::new(focus, window, model.gen_seq)
        }
    };
    v.list = None;
    v.error = None;
    v.planning = false;
    v.all_services = idx.all_services_href().is_some();
    model.gen_from_app = from_app;
    model.gen_view = Some(v);
    let cmds = ensure_drafts(model);
    model.overlay = Overlay::Generator;
    rebuild_gen(model);
    if let Some(v) = &model.gen_view {
        model.status = StatusLine::info(format!(
            "{} · generator · {} req",
            v.focus.app_text(),
            cmds.len()
        ));
    }
    cmds
}

/// The draft rulesets for the open generator: held (nothing sent), on
/// their way, or one load now, counted in its header.
pub(super) fn ensure_drafts(model: &mut Model) -> Vec<Cmd> {
    if model.draft_rulesets().is_some() {
        if let Some(v) = model.gen_view.as_mut() {
            v.drafts = Drafts::Loaded;
        }
        return Vec::new();
    }
    let cmds = load_form_drafts(model);
    if let Some(v) = model.gen_view.as_mut() {
        v.drafts = Drafts::Loading;
        v.requests += u32::try_from(cmds.len()).unwrap_or(u32::MAX);
        v.drafts_sent = !cmds.is_empty();
    }
    cmds
}

/// What the header says when the data the generator reads may miss flows
/// (`None`: complete), and whether `r` can help.
fn gap_text(model: &Model, v: &GenView) -> (Option<String>, bool) {
    let mut sides = vec![FocusDirection::In];
    if v.outbound {
        sides.push(FocusDirection::Out);
    }
    let n = v.focus.direction.query_count();
    let app = v.focus.app_text();
    for d in sides {
        let side = v.focus.clone().with_direction(d);
        match model.traffic.complete_for(&side) {
            Ok(()) => {}
            Err(Gap::Capped { cap, focused: true }) => {
                return (
                    Some(format!(
                        "still capped at {cap}: B raises the cap, or narrow the window"
                    )),
                    false,
                );
            }
            Err(Gap::NoData) => {
                return (
                    Some(format!(
                        "no traffic loaded: r queries {app} ({})",
                        queries_text(n)
                    )),
                    true,
                );
            }
            Err(_) => {
                return (
                    Some(format!(
                        "incomplete: r re-queries {app} ({})",
                        queries_text(n)
                    )),
                    true,
                );
            }
        }
    }
    (None, false)
}

/// The rows again from the loaded flows (checkboxes kept), the gap, and
/// the proposal. After a traffic answer, `o`, and on open. No request.
pub(super) fn rebuild_gen(model: &mut Model) {
    let Some(idx) = model.index().cloned() else {
        return;
    };
    let Some(v) = model.gen_view.as_ref() else {
        return;
    };
    if v.focus.keys != model.limits.scope_keys {
        // The label model changed under it (a profile switch).
        model.gen_view = None;
        if model.overlay == Overlay::Generator {
            model.overlay = Overlay::None;
        }
        model.status = StatusLine::warn("g changed: G reopens the generator");
        return;
    }
    let fresh = rows(
        model.traffic.flows_for(&v.focus),
        &v.focus,
        &model.limits.tier_key,
        &idx,
    );
    let (gap, requery) = gap_text(model, v);
    if let Some(v) = model.gen_view.as_mut() {
        v.set_rows(fresh);
        v.gap = gap;
        v.requery = requery;
    }
    regen(model);
}

/// The proposal again from the rows (every key press). Pure.
pub(super) fn regen(model: &mut Model) {
    let Some(idx) = model.index().cloned() else {
        return;
    };
    let p = {
        let (Some(v), Some(drafts)) = (model.gen_view.as_ref(), model.draft_rulesets()) else {
            return;
        };
        let input = GenInput {
            flows: model.traffic.flows_for(&v.focus),
            focus: &v.focus,
            tier_key: &model.limits.tier_key,
            drafts,
            idx: &idx,
            ruleset_name: &model.limits.ruleset_name,
            window: &v.window,
            outbound: v.outbound,
            picked: v.picked.as_deref(),
        };
        propose(&v.rows, v.global, &input)
    };
    if let Some(v) = model.gen_view.as_mut() {
        v.proposal = Some(p);
        v.drafts = Drafts::Loaded;
    }
}

/// Draft rulesets arrived (`Ok(hit)`) or failed to. Fills the view only.
pub(super) fn gen_drafts_answer(model: &mut Model, outcome: Result<bool, &str>) {
    let Some(v) = model.gen_view.as_mut() else {
        return;
    };
    match outcome {
        // Any draft load (its own, or the reload after a write) refreshes it.
        Ok(hit) => {
            if std::mem::take(&mut v.drafts_sent) && hit {
                v.cached += 1;
            }
            regen(model);
        }
        Err(e) if v.drafts == Drafts::Loading => v.drafts = Drafts::Failed(e.to_string()),
        Err(_) => {}
    }
}

/// `Esc` / `q`: back to the app overlay it came from (its view is kept),
/// else to the screen.
fn close_gen(model: &mut Model) {
    let back = std::mem::take(&mut model.gen_from_app) && model.app_view.is_some();
    model.overlay = if back { Overlay::App } else { Overlay::None };
}

/// The generator's keys. Only `r` (a load or the focused query, through
/// the cost guard) and `Enter` (the plan) can cause a request.
pub(super) fn gen_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let scope_keys = model.limits.scope_keys.clone();
    let Some(v) = model.gen_view.as_mut() else {
        model.overlay = Overlay::None;
        return Vec::new();
    };
    if let Some(scroll) = v.list {
        match k.code {
            KeyCode::Esc | KeyCode::Char('p') | KeyCode::Char('q') => v.list = None,
            KeyCode::Down | KeyCode::Char('j') => v.list = Some(scroll.saturating_add(1)),
            KeyCode::Up | KeyCode::Char('k') => v.list = Some(scroll.saturating_sub(1)),
            _ => {}
        }
        return Vec::new();
    }
    match k.code {
        KeyCode::Esc | KeyCode::Char('q') => close_gen(model),
        KeyCode::Down | KeyCode::Char('j') => v.move_cursor(1),
        KeyCode::Up | KeyCode::Char('k') => v.move_cursor(-1),
        KeyCode::Tab => v.switch_section(),
        KeyCode::Char(' ') => {
            if let Err(e) = v.toggle(&scope_keys) {
                model.status = StatusLine::warn(e);
            }
            regen(model);
        }
        KeyCode::Left | KeyCode::Char('h') => {
            v.slide(-1);
            regen(model);
        }
        KeyCode::Right | KeyCode::Char('l') => {
            v.slide(1);
            regen(model);
        }
        KeyCode::Char('<') => {
            v.slide_row(-1);
            regen(model);
        }
        KeyCode::Char('>') => {
            v.slide_row(1);
            regen(model);
        }
        KeyCode::Char('=') => {
            v.clear_row_stop();
            regen(model);
        }
        KeyCode::Char('o') => {
            v.toggle_outbound();
            rebuild_gen(model);
        }
        KeyCode::Char('R') => {
            match v.pick_next() {
                Ok(name) => model.status = StatusLine::info(format!("rules go into {name}")),
                Err(e) => model.status = StatusLine::info(e),
            }
            regen(model);
        }
        KeyCode::Char('p') => v.list = Some(0),
        KeyCode::Char('r') => return gen_reload(model),
        KeyCode::Enter => return gen_enter(model),
        _ => {}
    }
    Vec::new()
}

/// `r`: the draft rulesets again after a failed load, else the focused
/// query for the app when the data is incomplete (asking past the cost
/// threshold). The traffic screen's focus becomes the generator's.
fn gen_reload(model: &mut Model) -> Vec<Cmd> {
    let Some(v) = model.gen_view.as_ref() else {
        return Vec::new();
    };
    if matches!(v.drafts, Drafts::Failed(_)) {
        return ensure_drafts(model);
    }
    if !v.requery {
        model.status = StatusLine::info(match &v.gap {
            Some(g) => g.clone(),
            None => "the data is complete: nothing to re-query".into(),
        });
        return Vec::new();
    }
    let focus = v.focus.clone();
    let filter = model.traffic.filter.clone();
    model.traffic.set_focus(focus.clone());
    if let Some(reason) = filter.needs_confirmation() {
        model.overlay = Overlay::Confirm {
            reason: format!(
                "{reason}. Run {} for {} anyway? (y/n)",
                queries_text(focus.direction.query_count()),
                focus.app_text()
            ),
            action: PendingAction::GenRequery(filter, focus),
        };
        return Vec::new();
    }
    start_focused(model, filter, focus, CacheMode::Use)
}

/// `y` on `r`'s prompt: the focused query, back to the generator.
pub(super) fn gen_requery(
    model: &mut Model,
    filter: crate::tui::filter::TrafficFilter,
    focus: Focus,
) -> Vec<Cmd> {
    let cmds = start_focused(model, filter, focus, CacheMode::Use);
    model.overlay = Overlay::Generator;
    cmds
}

/// `Enter`: refused with the reason, or one `Cmd::PlanBatch` (its reads
/// are counted in the confirm). The generator stays open while it plans.
fn gen_enter(model: &mut Model) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    let Some(v) = model.gen_view.as_ref() else {
        return Vec::new();
    };
    let refusal = if let Some(g) = &v.gap {
        Some(format!("{g}: no rules from incomplete data"))
    } else if !v
        .rows
        .iter()
        .any(|r| r.proposed() && (r.side == Side::Inbound || v.outbound))
    {
        Some("no row is checked: Space checks one".to_string())
    } else if let Drafts::Failed(e) = &v.drafts {
        Some(format!("the draft rulesets did not load ({e}): r retries"))
    } else if let Some(p) = &v.proposal {
        p.blocker().or_else(|| {
            (!p.writes())
                .then(|| "every proposed rule is in the draft already: nothing to write".into())
        })
    } else {
        Some("the draft rulesets are not loaded yet".to_string())
    };
    if let Some(why) = refusal {
        model.status = StatusLine::warn(why);
        return Vec::new();
    }
    let (Some(p), Some(index)) = (v.proposal.clone(), model.index().cloned()) else {
        return Vec::new();
    };
    let input = BatchInput {
        proposal: p,
        ruleset_name: model.limits.ruleset_name.clone(),
        picked: v.picked.clone(),
    };
    if let Some(v) = model.gen_view.as_mut() {
        v.planning = true;
        v.error = None;
    }
    model.writes.in_flight = true;
    model.status = StatusLine::info("planning: the draft rulesets, read fresh…");
    vec![Cmd::PlanBatch {
        input: Box::new(input),
        index,
    }]
}

```

In `crates/illumio-tui/src/tui/update.rs`, a traffic answer rebuilds an open generator:

```rust
    if traffic_answer && app_open {
        rebuild_app_traffic(model);
    }
    cmds
}

```

with:

```rust
    if traffic_answer && app_open {
        rebuild_app_traffic(model);
    }
    // The same for the generator: its rows and its gap follow.
    let gen_open = matches!(
        model.overlay,
        Overlay::Generator
            | Overlay::Confirm {
                action: PendingAction::GenRequery(..),
                ..
            }
    );
    if traffic_answer && gen_open {
        generator::rebuild_gen(model);
    }
    cmds
}

```

a stale draft load is asked again for it too:

```rust
                let waiting = matches!(
                    &model.overlay,
                    Overlay::Form(f) if matches!(f.intent, EditIntent::RuleFromFlow(_))
                );
                return if waiting {
                    load_form_drafts(model)
                } else {
```

with:

```rust
                let waiting = matches!(
                    &model.overlay,
                    Overlay::Form(f) if matches!(f.intent, EditIntent::RuleFromFlow(_))
                ) || model.overlay == Overlay::Generator;
                return if waiting {
                    load_form_drafts(model)
                } else {
```

the draft rulesets' answers reach it (the `w` form's own load):

```rust
                };
            }
            match result {
                Ok((raw, _, _)) => {
                    model.policy.form_drafts = Some(raw);
                    refill_rule_form(model);
                }
                Err(e) => model.status = StatusLine::warn(format!("draft rulesets: {e}")),
            }
            Vec::new()
        }
```

with:

```rust
                };
            }
            match result {
                Ok((raw, _, hit)) => {
                    model.policy.form_drafts = Some(raw);
                    refill_rule_form(model);
                    generator::gen_drafts_answer(model, Ok(hit));
                }
                Err(e) => {
                    model.status = StatusLine::warn(format!("draft rulesets: {e}"));
                    generator::gen_drafts_answer(model, Err(&e));
                }
            }
            Vec::new()
        }
```

(the Policy screen's load):

```rust
                        CacheMode::Use
                    };
                    refresh_app_policy(model, scope, Ok(hit));
                    // Off the Policy screen (the app overlay, `u` on Labels)
                    // nothing shows the deny rules: `switch` asks on entry.
                    if model.screen == Screen::Policy {
```

with:

```rust
                        CacheMode::Use
                    };
                    refresh_app_policy(model, scope, Ok(hit));
                    if scope == PolicyScope::Draft {
                        generator::gen_drafts_answer(model, Ok(hit));
                    }
                    // Off the Policy screen (the app overlay, `u` on Labels)
                    // nothing shows the deny rules: `switch` asks on entry.
                    if model.screen == Screen::Policy {
```

In `crates/illumio-tui/src/tui/update.rs`, replace:

```rust
                    let tripped = model.policy.auto.take_tick_failure();
                    fail(model, "rulesets", &e, tripped);
                    refresh_app_policy(model, scope, Err(&e));
                    Vec::new()
                }
            }
```

with:

```rust
                    let tripped = model.policy.auto.take_tick_failure();
                    fail(model, "rulesets", &e, tripped);
                    refresh_app_policy(model, scope, Err(&e));
                    if scope == PolicyScope::Draft {
                        generator::gen_drafts_answer(model, Err(&e));
                    }
                    Vec::new()
                }
            }
```

the plan's answer ends its planning:

```rust
        }
        Msg::BatchPlanned(result) => {
            model.writes.in_flight = false;
            match result {
                // Keys typed while the batch was planned went elsewhere; a
                // `y` there must not confirm this.
                Ok(_) if !batch_lands(model) => {
                    model.status = StatusLine::warn("requests discarded, nothing sent");
                }
                Ok(batch) => {
```

with:

```rust
        }
        Msg::BatchPlanned(result) => {
            model.writes.in_flight = false;
            let planned_here = batch_lands(model);
            if let Some(v) = model.gen_view.as_mut() {
                v.planning = false;
            }
            match result {
                // Keys typed while the batch was planned went elsewhere; a
                // `y` there must not confirm this.
                Ok(_) if !planned_here => {
                    model.status = StatusLine::warn("requests discarded, nothing sent");
                }
                Ok(batch) => {
```

the re-query prompt goes back to it:

```rust
                        hrefs,
                    } => start_draft(model, filter, focus, hrefs),
                    PendingAction::ReloadApp => app_reload(model),
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
```

with:

```rust
                        hrefs,
                    } => start_draft(model, filter, focus, hrefs),
                    PendingAction::ReloadApp => app_reload(model),
                    PendingAction::GenRequery(f, focus) => generator::gen_requery(model, f, focus),
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
```

In `crates/illumio-tui/src/tui/update.rs`, replace:

```rust
                if matches!(action, PendingAction::ReloadApp) {
                    model.overlay = Overlay::App;
                }
                Vec::new()
            }
            _ => Vec::new(),
```

with:

```rust
                if matches!(action, PendingAction::ReloadApp) {
                    model.overlay = Overlay::App;
                }
                if matches!(action, PendingAction::GenRequery(..)) {
                    model.overlay = Overlay::Generator;
                }
                Vec::new()
            }
            _ => Vec::new(),
```

the picker and the overlay keys:

```rust
                        Vec::new()
                    }
                    AppPickerPurpose::Overlay => open_app_by_name(model, &app),
                }
            }
        },
        Overlay::App => app_overlay_key(model, k),
        Overlay::None => on_screen_key(model, k),
    }
}
```

with:

```rust
                        Vec::new()
                    }
                    AppPickerPurpose::Overlay => open_app_by_name(model, &app),
                    AppPickerPurpose::Generator => gen_by_name(model, &app),
                }
            }
        },
        Overlay::App => app_overlay_key(model, k),
        Overlay::Generator => generator::gen_key(model, k),
        Overlay::None => on_screen_key(model, k),
    }
}
```

`:gen <app>` finds the app as `:app` does:

```rust
    open_app(model, focus)
}

/// Opens the overlay for `focus`. The traffic panels come from the loaded
/// result (no request). The workloads come from the Workloads screen when
/// it holds every workload; the rulesets from the Policy screen when it is
```

with:

```rust
    open_app(model, focus)
}

/// `:gen <app>` and the picker: as `:app` finds an app (the loaded result
/// or the labels), then the generator for it.
fn gen_by_name(model: &mut Model, name: &str) -> Vec<Cmd> {
    let keys = model.traffic.keys.clone();
    let app = pce::traffic::parse_app(name);
    let loaded = model.traffic.loaded_apps();
    let labelled = model
        .index()
        .map(|idx| pce::traffic::app_labels(&app, &keys, idx));
    if !loaded.iter().any(|a| a == name) && !matches!(labelled, Some(Ok(_))) {
        let near = pce::traffic::closest(name, &loaded, 3);
        model.status = StatusLine::warn(match (near.is_empty(), labelled) {
            (false, _) => format!(
                "unknown app {name:?}; closest: {}; nothing sent",
                near.join(", ")
            ),
            (true, Some(Err(e))) => format!("{e}; nothing sent"),
            (true, _) => format!("unknown app {name:?}; nothing sent"),
        });
        return Vec::new();
    }
    generator::open_gen(model, app, false)
}

/// Opens the overlay for `focus`. The traffic panels come from the loaded
/// result (no request). The workloads come from the Workloads screen when
/// it holds every workload; the rulesets from the Policy screen when it is
```

`G` in the app overlay:

```rust
        KeyCode::Enter => return app_enter(model),
        KeyCode::Char('w') => return app_rule_key(model),
        KeyCode::Char('r') => app_reload_key(model),
        _ => {}
    }
    Vec::new()
```

with:

```rust
        KeyCode::Enter => return app_enter(model),
        KeyCode::Char('w') => return app_rule_key(model),
        KeyCode::Char('r') => app_reload_key(model),
        KeyCode::Char('G') => {
            let app = v.focus.app.clone();
            return generator::open_gen(model, app, true);
        }
        _ => {}
    }
    Vec::new()
```

Plan 2's return points now mean the generator:

```rust
    }
}

/// Whether a planned batch may open its confirm: only over the screen it
/// was planned from (nothing else open).
fn batch_lands(model: &Model) -> bool {
    model.overlay == Overlay::None
}

/// A batch could not be planned: nothing was sent.
fn batch_plan_failed(model: &mut Model, e: &str) {
    fail(model, "plan", e, false);
}

/// The batch confirm or its answer is done: back to the screen. What it
/// reopens may need to load (none here).
fn batch_closed(model: &mut Model) -> Vec<Cmd> {
    model.overlay = Overlay::None;
    Vec::new()
}

/// `wrote 5 of 7 (pending); step 6 failed: …; 1 not sent`, and whether a
```

with:

```rust
    }
}

/// Whether a planned batch may open its confirm: only over the generator
/// that planned it (still open, still planning).
fn batch_lands(model: &Model) -> bool {
    model.overlay == Overlay::Generator && model.gen_view.as_ref().is_some_and(|v| v.planning)
}

/// A batch could not be planned: nothing was sent; the generator keeps
/// its state and shows why.
fn batch_plan_failed(model: &mut Model, e: &str) {
    model.status = StatusLine::error(format!("plan: {e}"));
    match model.gen_view.as_mut() {
        Some(v) if model.overlay == Overlay::Generator => v.error = Some(e.to_string()),
        _ => fail(model, "plan", e, false),
    }
}

/// The batch confirm or its answer is done: back to the generator (the
/// drafts it reads load again after a write: one request, counted).
fn batch_closed(model: &mut Model) -> Vec<Cmd> {
    if model.gen_view.is_none() {
        model.overlay = Overlay::None;
        return Vec::new();
    }
    model.overlay = Overlay::Generator;
    let cmds = generator::ensure_drafts(model);
    generator::regen(model);
    cmds
}

/// `wrote 5 of 7 (pending); step 6 failed: …; 1 not sent`, and whether a
```

and the command:

```rust
            open_app_picker(model, AppPickerPurpose::Overlay, "");
            Vec::new()
        }
        Command::Profile(name) => {
            if model.writes.in_flight {
                model.status = StatusLine::warn("finish or cancel the write first");
```

with:

```rust
            open_app_picker(model, AppPickerPurpose::Overlay, "");
            Vec::new()
        }
        Command::Gen(Some(name)) => gen_by_name(model, &name),
        Command::Gen(None) => {
            open_app_picker(model, AppPickerPurpose::Generator, "");
            Vec::new()
        }
        Command::Profile(name) => {
            if model.writes.in_flight {
                model.status = StatusLine::warn("finish or cancel the write first");
```

Create `crates/illumio-tui/src/tui/ui/widgets/generator.rs` (the overlay must be drawn for `view`'s match to be complete; Task 4 snapshots it):

```rust
//! The policy generator (`G` / `:gen`), btop style: the slider and the
//! target, the inbound and outbound rows with their checkboxes, the
//! preview; `p` swaps the rows for the exact rule list.
use crate::tui::gen_view::GenView;
use crate::tui::model::Model;
use crate::tui::ui::theme::Theme;
use crate::tui::ui::widgets::panel::panel;
use crate::tui::ui::widgets::table::{clip_width, fit as fit_cells};
use pce::generate::{GenRow, Side};
use pce::write::PortSpec;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

/// `s` padded to `w` cells, or cut with `…` when wider.
fn fit(s: &str, w: usize) -> String {
    if Span::raw(s).width() <= w || w == 0 {
        fit_cells(s, w, false)
    } else {
        fit_cells(&format!("{}…", clip_width(s, w - 1)), w, false)
    }
}

/// One row: checkbox, peer, stop (`*` when overridden), services,
/// decisions, the draft's mark, the flags; an outbound row its target.
fn row_line<'a>(v: &GenView, r: &GenRow, scope_keys: &[String], t: &Theme) -> Line<'a> {
    let stop = r.effective(v.global);
    let check = match (r.checkable, r.checked) {
        (false, _) => " - ",
        (true, true) => "[x]",
        (true, false) => "[ ]",
    };
    let services = if stop.is_all() {
        "all services".to_string()
    } else {
        r.services
            .iter()
            .map(PortSpec::display)
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut decisions = String::new();
    if r.decisions.allowed > 0 {
        decisions.push_str(&format!("✓ {} ", r.decisions.allowed));
    }
    if r.decisions.blocked > 0 {
        decisions.push_str(&format!("✗ {} ", r.decisions.blocked));
    }
    if let Some(n) = r.decisions.draft_allowed.filter(|n| *n > 0) {
        decisions.push_str(&format!("would allow {n} "));
    }
    let mut spans = vec![
        Span::styled(format!("{check} "), Style::new().fg(t.accent)),
        Span::styled(fit(&r.peer_text(), 18), t.base()),
        Span::raw(" "),
        Span::styled(
            fit(
                &format!("{}{}", stop.text(), if r.stop.is_some() { "*" } else { "" }),
                11,
            ),
            t.dim_style(),
        ),
        Span::raw(" "),
        Span::styled(fit(&services, 18), t.base()),
        Span::raw(" "),
        Span::styled(decisions, t.base()),
    ];
    if let Some(target) = v.row_target(r) {
        spans.push(Span::styled(format!("→ {target} "), t.dim_style()));
    }
    for f in r.flag_texts(stop, scope_keys) {
        spans.push(Span::styled(format!("⚠ {f} "), Style::new().fg(t.warn)));
    }
    Line::from(spans)
}

pub fn render_generator(model: &Model, f: &mut Frame, area: Rect) {
    let Some(v) = &model.gen_view else {
        return;
    };
    let t = &model.theme;
    let body = crate::tui::ui::layout::chrome(area).map_or(area, |c| c.body);
    f.render_widget(Clear, body);
    let (title, meta) = (v.header_title(), v.header_meta());
    let outer = panel(&title, &meta, t, true).style(t.base());
    let inner = outer.inner(body);
    f.render_widget(outer, body);
    let parts = Layout::vertical([
        Constraint::Length(2),
        Constraint::Fill(1),
        Constraint::Length(2),
    ])
    .split(inner);
    let empty = pce::NameIndex::default();
    let idx = model.index().map_or(&empty, |i| i.as_ref());
    let mut top = vec![
        Line::from(Span::styled(v.slider_line(), t.base())),
        Line::from(Span::styled(
            v.target_line(model.draft_rulesets(), idx),
            t.header_style(),
        )),
    ];
    if let Some(g) = &v.gap {
        top[0] = Line::from(Span::styled(g.clone(), Style::new().fg(t.warn)));
    }
    f.render_widget(Paragraph::new(top), parts[0]);
    let height = usize::from(parts[1].height);
    let (lines, cursor) = match v.list {
        Some(scroll) => {
            let all: Vec<Line> = v
                .rule_list(idx)
                .into_iter()
                .map(|l| Line::from(Span::styled(l, t.base())))
                .collect();
            let first = usize::from(scroll).min(all.len().saturating_sub(height));
            (all.into_iter().skip(first).collect::<Vec<_>>(), None)
        }
        None => {
            let mut all = Vec::new();
            let mut cursor = None;
            for side in [Side::Inbound, Side::Outbound] {
                let title = match side {
                    Side::Inbound => format!("Inbound → {}", v.target_name()),
                    Side::Outbound if v.outbound => "Outbound (o: on)".to_string(),
                    Side::Outbound => "Outbound (o: off)".to_string(),
                };
                all.push(Line::from(Span::styled(title, t.header_style())));
                let rows = v.section_rows(side);
                if side == Side::Outbound && !v.outbound {
                    all.push(Line::from(Span::styled(
                        format!("{} rows hidden: o shows them", rows.len()),
                        t.dim_style(),
                    )));
                    continue;
                }
                for i in rows {
                    if Some(i) == v.selected() && side == v.section {
                        cursor = Some(all.len());
                    }
                    all.push(row_line(v, &v.rows[i], &model.limits.scope_keys, t));
                }
            }
            (all, cursor)
        }
    };
    let first = cursor.map_or(0, |c| c.saturating_sub(height.saturating_sub(1)));
    let shown: Vec<Line> = lines
        .into_iter()
        .enumerate()
        .skip(first)
        .take(height)
        .map(|(i, l)| {
            if Some(i) == cursor {
                l.patch_style(Style::new().add_modifier(Modifier::REVERSED))
            } else {
                l
            }
        })
        .collect();
    f.render_widget(Paragraph::new(shown), parts[1]);
    let keys = if v.list.is_some() {
        "j/k scroll · Esc back to the rows"
    } else {
        "p lists the rules · Enter plans the writes · Space toggles · o outbound"
    };
    let footer = vec![
        Line::from(Span::styled(
            v.footer(),
            if v.error.is_some() {
                Style::new().fg(t.bad)
            } else {
                t.base()
            },
        )),
        Line::from(Span::styled(keys, t.dim_style())),
    ];
    f.render_widget(Paragraph::new(footer), parts[2]);
}
```

In `crates/illumio-tui/src/tui/ui/widgets/mod.rs`:

```rust
pub mod app_overlay;
pub mod bars;
pub mod overlay;
pub mod panel;
pub mod status;
```

with:

```rust
pub mod app_overlay;
pub mod bars;
pub mod generator;
pub mod overlay;
pub mod panel;
pub mod status;
```

In `crates/illumio-tui/src/tui/ui/mod.rs`:

```rust
        Overlay::GroupKeys(picker) => widgets::overlay::render_key_picker(model, picker, f, area),
        Overlay::AppPicker(p) => widgets::overlay::render_app_picker(model, p, f, area),
        Overlay::App => widgets::app_overlay::render_app(model, f, area),
        Overlay::LabelKeys(picker) => {
            widgets::overlay::render_label_key_picker(model, picker, f, area)
        }
```

with:

```rust
        Overlay::GroupKeys(picker) => widgets::overlay::render_key_picker(model, picker, f, area),
        Overlay::AppPicker(p) => widgets::overlay::render_app_picker(model, p, f, area),
        Overlay::App => widgets::app_overlay::render_app(model, f, area),
        Overlay::Generator => widgets::generator::render_generator(model, f, area),
        Overlay::LabelKeys(picker) => {
            widgets::overlay::render_label_key_picker(model, picker, f, area)
        }
```

In `crates/illumio-tui/src/tui/ui/widgets/status.rs`, the generator takes every key; its hints replace the screen's:

```rust

/// The app overlay's keys: it takes every key, so the screen's do not apply.
const APP_HINTS: &str = " │ Tab panel  X dir  Enter open  w rule  r reload  Esc back ";

pub fn render(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
```

with:

```rust

/// The app overlay's keys: it takes every key, so the screen's do not apply.
const APP_HINTS: &str = " │ Tab panel  X dir  Enter open  w rule  r reload  Esc back ";
/// The generator's keys, for the same reason.
const GEN_HINTS: &str = " │ ←/→ stop  </> row  R pick  r re-query  Esc back ";

pub fn render(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
```

In `crates/illumio-tui/src/tui/ui/widgets/status.rs`, replace:

```rust
    let base = screen_hint(model.screen, model.writes.enabled);
    let more = screen_hint_more(model.screen, model.traffic.view);
    let width = usize::from(area.width);
    let app = model.overlay == Overlay::App;
    let mut hints = if app {
        APP_HINTS.to_string()
    } else {
        format!(" {base}  : cmd  ? help ")
```

with:

```rust
    let base = screen_hint(model.screen, model.writes.enabled);
    let more = screen_hint_more(model.screen, model.traffic.view);
    let width = usize::from(area.width);
    let app = matches!(model.overlay, Overlay::App | Overlay::Generator);
    let mut hints = if model.overlay == Overlay::Generator {
        GEN_HINTS.to_string()
    } else if app {
        APP_HINTS.to_string()
    } else {
        format!(" {base}  : cmd  ? help ")
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p illumio-tui --lib generator && cargo test -p illumio-tui --lib batch_tests && cargo test -p illumio-tui --lib commands`
Expected: PASS (14 + 8 + every commands test).

- [ ] **Step 6: Commit**

```bash
git add crates/illumio-tui/src
git commit -m "tui: the generator (G, :gen): open, keys, rebuilds, r, Enter into the batch confirm

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

### Task 4: Drawing the generator: snapshots and help

**Files:**
- Modify: `crates/illumio-tui/tests/tui_snapshots.rs`
- Modify: `crates/illumio-tui/src/tui/update.rs`
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs`

**Interfaces:**
- Consumes: Task 3 (`render_generator`, `GenView` texts), `HELP_LINES`.
- Produces: four generator snapshots, the help lines (`:gen <app>`, `App G`, three `Generate` lines).

- [ ] **Step 1: Write the failing snapshot tests and the help assertions**

Append to `crates/illumio-tui/tests/tui_snapshots.rs`:

```rust

/// The rich flows in a known 24h window, the rulesets loaded, the
/// generator open for shop/prod: nothing sent.
fn generator_model() -> illumio_tui::tui::model::Model {
    use illumio_tui::tui::commands::Command;
    use illumio_tui::tui::msg::Msg;
    let mut m = with_policy(model_with_window("24h"));
    illumio_tui::tui::update(&mut m, Msg::Command(Command::Gen(Some("shop/prod".into()))));
    m
}

fn gen_key(m: &mut illumio_tui::tui::model::Model, c: char) {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    illumio_tui::tui::update(
        m,
        illumio_tui::tui::msg::Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)),
    );
}

#[test]
fn generator_80x24() {
    let m = generator_model();
    let out = render_to_string(&m, 80, 24);
    for s in [
        "Generate · shop/prod · last 24h · 8 flows · complete",
        "0 req · cached",
        "[app·ports]",
        "target  Shop-Prod (draft, 2 rules)",
        "Inbound → Shop-Prod",
        "[x] intra-app",
        "⚠ env mismatch",
        "Outbound (o: off)",
        "2 rows hidden: o shows them",
        "2 rules (2 new · 0 add services · 0 exist) · covers 5 of 8 flows · 3 not",
    ] {
        assert!(out.contains(s), "missing {s:?}:\n{out}");
    }
    insta::assert_snapshot!("generator_80x24", out);
}

#[test]
fn generator_outbound_role_ports_120x40() {
    let mut m = generator_model();
    gen_key(&mut m, 'o');
    gen_key(&mut m, 'l');
    let out = render_to_string(&m, 120, 40);
    for s in [
        "[role·ports]",
        "Outbound (o: on)",
        "→ RF-crm-prod",
        "⚠ needs an IP list",
    ] {
        assert!(out.contains(s), "missing {s:?}:\n{out}");
    }
    insta::assert_snapshot!("generator_outbound_role_ports_120x40", out);
}

#[test]
fn generator_rule_list_120x40() {
    let mut m = generator_model();
    gen_key(&mut m, 'p');
    let out = render_to_string(&m, 120, 40);
    assert!(
        out.contains("1. create · Shop-Prod: all workloads → all workloads on 5432/tcp"),
        "{out}"
    );
    assert!(out.contains("not covered: 3 unchecked"), "{out}");
    insta::assert_snapshot!("generator_rule_list_120x40", out);
}

#[test]
fn generator_incomplete_80x24() {
    let mut m = with_policy(model_with_window("24h"));
    m.traffic.data.as_mut().unwrap().truncated = true;
    {
        use illumio_tui::tui::commands::Command;
        use illumio_tui::tui::msg::Msg;
        illumio_tui::tui::update(&mut m, Msg::Command(Command::Gen(Some("shop/prod".into()))));
    }
    let out = render_to_string(&m, 80, 24);
    assert!(
        out.contains("incomplete: r re-queries shop/prod (1 query)"),
        "{out}"
    );
    insta::assert_snapshot!("generator_incomplete_80x24", out);
}
```

In `help_scrolls_to_its_last_line_and_reopens_at_the_top` (`crates/illumio-tui/src/tui/update.rs`), the help grows by five lines:

```rust
    fn help_scrolls_to_its_last_line_and_reopens_at_the_top() {
        use crate::tui::ui::widgets::overlay::{HELP_LINES, help_last_scroll};
        let mut m = model_fixture(); // 80x24
        assert_eq!(HELP_LINES.len(), 65);
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Traffic  A"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Traffic  D"));
        assert!(
```

with:

```rust
    fn help_scrolls_to_its_last_line_and_reopens_at_the_top() {
        use crate::tui::ui::widgets::overlay::{HELP_LINES, help_last_scroll};
        let mut m = model_fixture(); // 80x24
        assert_eq!(HELP_LINES.len(), 70);
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Traffic  A"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Traffic  D"));
        assert!(
```

In `crates/illumio-tui/src/tui/update.rs`, replace:

```rust
        );
        update(&mut m, key('?'));
        let last = help_last_scroll(24);
        assert_eq!(last, 43, "65 lines, 22 rows inside the box");
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "App-to-app x / X"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "App-to-app o"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "App-to-app d"));
```

with:

```rust
        );
        update(&mut m, key('?'));
        let last = help_last_scroll(24);
        assert_eq!(last, 48, "70 lines, 22 rows inside the box");
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "App-to-app x / X"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "App-to-app o"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "App-to-app d"));
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p illumio-tui --lib help_scrolls && cargo test -p illumio-tui --test tui_snapshots generator`
Expected: FAIL: the help has 65 lines, not 70; the snapshots do not exist yet.

- [ ] **Step 3: Write the help lines**

In `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` (`HELP_LINES`):

```rust
    ("", ":unicast  unicast flows only (again: every kind)"),
    ("", ":focus <app>  (alone: a picker of the loaded apps)"),
    ("", ":app <app>  the app overlay (alone: the picker)"),
    ("", ":writes [on|off]  :export[!] <path>"),
    ("", "(the table on screen as CSV; ! overwrites)"),
    ("?", "this help"),
```

with:

```rust
    ("", ":unicast  unicast flows only (again: every kind)"),
    ("", ":focus <app>  (alone: a picker of the loaded apps)"),
    ("", ":app <app>  the app overlay (alone: the picker)"),
    ("", ":gen <app>  the policy generator (alone: the picker)"),
    ("", ":writes [on|off]  :export[!] <path>"),
    ("", "(the table on screen as CSV; ! overwrites)"),
    ("?", "this help"),
```

In `crates/illumio-tui/src/tui/ui/widgets/overlay.rs`, replace:

```rust
        "App      w r Esc",
        "draft allow rule · reload panels (asks) · back",
    ),
    (
        "Traffic  Enter",
        "detail (flows) · drill into a row or section header",
```

with:

```rust
        "App      w r Esc",
        "draft allow rule · reload panels (asks) · back",
    ),
    (
        "App      G",
        "policy generator: the rules the app's traffic needs",
    ),
    (
        "Generate ←/→ < > =",
        "granularity · the row's own (*) · back to the slider",
    ),
    (
        "Generate Space o R p",
        "check a row · outbound · pick the ruleset · rule list",
    ),
    (
        "Generate Enter r Esc",
        "plan the writes, one confirm (writes) · re-query · back",
    ),
    (
        "Traffic  Enter",
        "detail (flows) · drill into a row or section header",
```

- [ ] **Step 4: Write the snapshots and read them**

Run: `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots && find crates/illumio-tui/tests/snapshots -name '*.snap.new' -delete && cargo test -p illumio-tui --test tui_snapshots`
Expected: PASS. New: `generator_80x24` (the slider with `[app·ports]`, `target  Shop-Prod (draft, 2 rules)`, four inbound rows, `2 rows hidden: o shows them`, the footer `2 rules (2 new · 0 add services · 0 exist) · covers 5 of 8 flows · 3 not`), `generator_outbound_role_ports_120x40` (`→ RF-crm-prod`, `1 exist`), `generator_rule_list_120x40`, `generator_incomplete_80x24`. Changed: the three help snapshots (`:gen <app>`, 70 lines). Nothing else may change.

- [ ] **Step 5: Commit**

```bash
git add crates/illumio-tui/src/tui/ui/widgets/overlay.rs crates/illumio-tui/src/tui/update.rs crates/illumio-tui/tests
git commit -m "tui: generator snapshots and help

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

### Task 5: Docs, tracking, read-only smoke, final gate

**Files:**
- Modify: `README.md`
- Modify: `docs/index.html`
- Modify: `tasks/todo.md`

**Interfaces:**
- Consumes: everything above.
- Produces: the README's generator paragraph, the site's, the tracking section.

- [ ] **Step 1: Document the generator**

In `README.md`, after the app overlay's description in the TUI section:

```markdown
  to the overlay without a request); `r` reloads the panels
  after asking; `Esc` returns.
```

with:

```markdown
  to the overlay without a request); `r` reloads the panels
  after asking; `Esc` returns.
  `G` in the app overlay (or `:gen shop/prod`, or `:gen` and the picker) opens the **policy
  generator** for that app: the allow rules its observed traffic needs, written in one
  confirmed step. One row per peer and side: intra-app, each app peer (`web/prod`), IP peer and
  unlabelled workload, with the services seen and the reported decisions (`✓` allowed, `✗`
  blocked or potentially blocked; with `D` loaded, `would allow N`). Intra-app and same-env
  peers start checked, the rest unchecked with the reason (`⚠ env mismatch`, `⚠ uses IP list
  Corp`, `⚠ needs an IP list`, which checking plans). The slider `app·all ─ app·ports ─
  role·ports ─ role·all` (`←`/`→`) sets how coarse the rules are: all workloads of the app or
  one rule per role pair, All Services or the services seen; `<`/`>` give a row its own stop
  (`*`), `=` drops it. `o` adds the outbound section: rules to other apps go into the peers'
  rulesets (a new `RF-crm-prod` when none covers the peer), to IP peers into the app's own.
  The target line names the app's ruleset (`R` picks when several cover it, or a new one by
  `generator.ruleset_name`, default `RF-{app}-{env}`). Rules already in the draft show as
  `exists` and are not written; an existing rule with the same actors gets the missing
  services. The footer counts the rules and the flow records they cover; `p` lists every rule
  and what is not covered, and why. Opening it costs at most the draft rulesets (one request,
  cached, counted in its header). On a truncated or incomplete result it shows the rows but
  refuses to plan (`incomplete: r re-queries shop/prod`); `r` runs the focused query through
  the usual cost guard. With writes on, `Enter` reads the draft rulesets fresh (dedupe again),
  then shows every request in one confirm (`6 requests: 1 ip_list, 1 rule_set, 4 rules ·
  planned with 2 reads`); `y` sends them in order and stops at the first failure (`wrote 5 of
  7 (pending); step 6 failed: …`). The policy view and the draft decisions are reloaded once
  per batch; provision with `P` in the pending overlay as usual. `Esc` returns.
```

In `docs/index.html`, after the app screen's paragraph:

```html
      <p><code>A</code> opens one screen for that app: inbound and outbound peers, its workloads with enforcement modes (workloads without a VEN counted as unmanaged) and VEN health, and the rulesets that cover it. It reuses what is loaded; opening it sends at most two requests, and every request is counted in its header.</p>
```

with:

```html
      <p><code>A</code> opens one screen for that app: inbound and outbound peers, its workloads with enforcement modes (workloads without a VEN counted as unmanaged) and VEN health, and the rulesets that cover it. It reuses what is loaded; opening it sends at most two requests, and every request is counted in its header.</p>
      <p><code>G</code> there (or <code>:gen</code>) opens the <strong>policy generator</strong>: the allow rules the app&rsquo;s own traffic needs, one row per peer with a checkbox, a slider from <em>app, all services</em> to <em>role pairs, the ports seen</em>, rules already in the draft marked <em>exists</em>, and a preview of the flows covered. <code>Enter</code> plans every write (IP lists, a ruleset, rule updates and creates) into one confirm; <code>y</code> sends them in order. On the CLI, <code>illumio-tui gen</code> prints the same proposal and writes nothing.</p>
```

and in the prompt's command list:

```html
<code>:profile &lt;name&gt;</code>, <code>:writes on|off</code>
```

with:

```html
<code>:profile &lt;name&gt;</code>, <code>:app</code>, <code>:gen</code>, <code>:writes on|off</code>
```

Append to `tasks/todo.md`:

```markdown

## Policy generator 3: the TUI (plan docs/superpowers/plans/2026-10-16-policy-generator-3-tui.md)
- [x] Task 1 complete_for, ruleset name in Limits
- [x] Task 2 GenView
- [x] Task 3 transitions: G, :gen, keys, r, Enter, the batch's return
- [x] Task 4 drawing, snapshots, help
- [x] Task 5 docs, smoke, gate
```

- [ ] **Step 2: Run the final gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && git diff main -- '*Cargo.toml' Cargo.lock`
Expected: every test passes, no warning, the diff is empty.

- [ ] **Step 3: Read-only TUI smoke on demo100**

Writes stay off (no `:writes on`), no `y`. Headless, in tmux, 120x40:

```bash
tmux new-session -d -s gen -x 120 -y 40 'illumio-tui --env-file ../illumio-mcp-server/.env'
sleep 6; tmux send-keys -t gen r; sleep 25
tmux send-keys -t gen A; sleep 3; tmux send-keys -t gen G; sleep 4
tmux capture-pane -pt gen > /tmp/gen-open.txt
tmux send-keys -t gen o l; sleep 1; tmux send-keys -t gen p; sleep 1
tmux capture-pane -pt gen > /tmp/gen-list.txt
tmux send-keys -t gen Enter; sleep 1; tmux capture-pane -pt gen > /tmp/gen-enter.txt
tmux send-keys -t gen Escape Escape Escape q; sleep 1; tmux kill-session -t gen 2>/dev/null
```

Expected: `/tmp/gen-open.txt` shows `Generate · <app> · last 24h`, `1 req` or `0 req`, a target line and rows; `/tmp/gen-list.txt` the rule list; `/tmp/gen-enter.txt` the status `writes off; :writes on to enable` (or the incomplete refusal). If the row under the cursor has no unambiguous app, `A` opens the picker: type the app shown by `illumio-tui gen` in Plan 1's smoke and `Enter`. Report the header, the footer counts and the api calls in the review. The audit log does not grow.

- [ ] **Step 4: Write the review section**

Append `## Review (Policy generator 3)` to `tasks/todo.md`: what shipped, the test count, the smoke, "nothing was written to any PCE".

- [ ] **Step 5: Commit**

```bash
git add README.md docs/index.html tasks/todo.md
git commit -m "docs: the policy generator in the TUI

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

## Self-review notes

- Spec coverage: opening (`G`, `:gen <app>`, `:gen` and the picker; one counted request; `Esc` back), the layout (header, slider, target line, sections, columns, footer), the keys, the row defaults (Plan 1), completeness (`complete_for`, the header texts, `r`, still capped, no flows), `Enter`'s refusals and `Cmd::PlanBatch`, the return after a batch, late answers (`seq`, `planning`), `g` changed, writes off: Tasks 1-4. The spec's TUI tests: Task 3 (`G`, `:gen`, at most one request, `←`/`→`, `<`/`>`/`=`, `Space`, `o`, `R`, `p`, incomplete `Enter`/`r`, writes off, `y` sends one `ExecuteBatch` (Plan 2), a `y` during planning, one redraft per batch (Plan 2), `Esc` back with nothing sent) and Task 4 (snapshots of the layout, `p` and, in Plan 2, the confirm).
- Verification: every step of this plan was applied in order to a clean checkout of main (after the plans before it); each task built clean under `cargo clippy --workspace --all-targets -- -D warnings` with its own tests passing, and the gate passed at the end.

## Notes from the part-2 review (2026-10-16)

- `batch_lands` requires `Overlay::None`: `Enter` in the generator must close `Overlay::Generator` before it emits `Cmd::PlanBatch` (or `batch_lands` must accept the generator), or every plan is discarded.
- Add "a profile switch is in progress" (`model.switching`) to `Enter`'s refusals. A batch also carries its `origin` (profile, host, org): `BatchPlanned` from another profile is discarded and `execute_batch` refuses one planned elsewhere.
- `batch_closed` is also where the batch report (`Overlay::BatchReport`, `:batch`) returns to.
