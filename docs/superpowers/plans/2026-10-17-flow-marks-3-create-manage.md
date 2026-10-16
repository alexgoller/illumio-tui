# Flow marks (3 of 3): creating and managing marks in the TUI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `m` marks the row under the cursor (Flows, App-to-app pairs and sections, Mesh, Blocked, the app overlay's Inbound and Outbound): a form prefilled from the row, `Ctrl-A` / `Ctrl-T` / `Ctrl-U` to widen, `Ctrl-N` completion from loaded data, a live preview of what it matches, a required comment, saved to `marks.toml` through Plan 1's lock; `:marks [action]` lists every mark with its hits (`Enter` edit, `x` delete, `e` +30 days); and the help, README and site describe it.

**Architecture:** `MarkForm` wraps the generic `Form` (fields, keys, completion) with `MarkCtx` (origin, the edited mark, each side's kinds, the rows the preview counts in) and draws itself one row per field (`ui::widgets::marks`). Its submit never reaches `edit::build_intent` or `Cmd::PlanWrite`: `update::marks` applies one `MarkOp` and rebuilds the views, returning no `Cmd`. The prefill reuses each view's own drill (`Drill::matches`) to find a representative flow, so no group-key logic is copied. `:marks` keeps its state in `Model::marks_view` like the app overlay and the generator do.

**Tech Stack:** Rust 2024 (rust-version 1.88), ratatui 0.30, crossterm, insta (plain snapshot files), tempfile. No new dependency.

**Spec:** `docs/superpowers/specs/2026-10-17-flow-marks-design.md` (binding), delivery item 3. Builds on Plans 1 and 2 (`2026-10-17-flow-marks-1-model-cli.md`, `2026-10-17-flow-marks-2-display.md`), which must be merged first.

**Tasks:** 4, in this order: 1 field kinds, completion and `MarkForm`; 2 `m`: the form per view, its keys, live preview and save; 3 the `:marks` overlay; 4 help, README, site, tracking, final gate.

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
- Plans 1 and 2 are merged.
- No mark key returns a `Cmd`: the form's save is a local file write through `MarksStore::apply` (Plan 1's lock), never `edit::build_intent` or `Cmd::PlanWrite`, and it is not gated by `:writes`.
- Task 4 edits `README.md` and `docs/index.html`. Another branch may be editing them: rebase onto `main` first; if a `replace` anchor below is not found, put the same text after the same paragraph (the `D` paragraph of the TUI section, the CLI example list, the paragraph before `## Writes`; the site's generator paragraph and prompt list).

## Spec deviations and resolutions (read before Task 1)

- **The form** draws one row per field with its hint beside it (`ui::widgets::marks::render_mark_form`, as the spec's sketch), not `render_form`; it has a `Draft` field after `Decision` (Alex's decision 2: `draft_decision`). `Scope` takes `this PCE`, `any PCE` or a host (an edited mark scoped elsewhere keeps its host and org). The comment is checked at the save with the spec's message, not as a `required` field.
- **Keys**: `Ctrl-N`/`Ctrl-P` complete as every form does (from the index and the loaded workloads; on Action, Expires and Scope they cycle the fixed list, the next value first); `Ctrl-T` cycles the active side among the kinds the row allows; `Ctrl-A` swaps the active side only between its text and every label of the row's endpoint (the `w` form swaps all its fields); `Ctrl-U` clears a field (the generic form's key).
- **Grouped rows** take a representative flow with the row's own drill (`Drill::matches`, `TrafficState::first_flow`, as `w` does): its endpoints give `Ctrl-A`'s labels and `Ctrl-T`'s kinds; the group key gives the text. A `both` section header puts the app on the destination side.
- **The preview** counts the rows of the view the form came from (its own aggregator over the matching flows of the whole loaded result). Its second line is `K of them stay shown: <action> <id> wins` for a squelch and `K of them go to a stronger mark: <action> <id> wins` otherwise; a mark scoped elsewhere, expired or waiting for `D` says `not active here: …`.
- **`:marks`** keeps its state in `Model::marks_view` with a unit `Overlay::Marks` (as the app overlay and the generator do) instead of `Overlay::Marks(MarksView)`: a `TableView` is not `Clone`. Its rows' hits are computed when it opens and after each change. `e` opens the form with the comment alone (the spec's "asks for the reason again, prefilled"), then applies `MarkOp::Extend` (30 days from the later of now and the expiry, on the fresh read); on a `never` mark it says so and opens nothing. `x` goes through `Overlay::Confirm` with `PendingAction::DeleteMark(id)`; `y` and `n` return to `:marks`.
- **Help**: `m` joins the app overlay's `App      m w r Esc` line: the `Tab j/k X Enter` key text has no room for it in the 25-cell key column. `:marks` joins the `:profile … :help :q` line (it fits the 80-column box there).

## Review Focus

- **Two TUIs (or the TUI and a script) saving marks at once**: the form's save goes through the lock; a held lock keeps the form open with the error, nothing half-written. Pinned in Task 2 (`a_failed_write_keeps_the_form_open_with_the_error`).
- **Widening to `any`** on a row: the preview counts the flows of the whole loaded result (not only the focus) and says which stronger mark keeps some of them. Pinned in Task 2 (`widening_a_side_changes_the_preview`).
- **A mark deleted from `:marks` while flows depend on it**: the views rebuild at once (Corp's flows come back). Pinned in Task 3 (`x_and_y_delete_a_mark_and_n_keeps_it`).
- **`m` on an invalid file**: the problems open, not a form that could never save. Pinned in Task 3 (`an_invalid_file_shows_its_problems_where_m_and_marks_open`).
- **No request from any marks key** (`m`, `M`, `Ctrl-N`, `Enter`, `x`, `y`, `e`): every key test goes through `quiet`, which asserts no `Cmd`. Pinned in Tasks 2 and 3 (`update::marks_tests`).

---

### Task 1: Field kinds, completion and `MarkForm`

**Files:**
- Modify: `crates/illumio-tui/src/tui/complete.rs`
- Modify: `crates/illumio-tui/src/tui/edit.rs`
- Modify: `crates/illumio-tui/src/tui/form.rs`
- Create: `crates/illumio-tui/src/tui/mark_form.rs`
- Modify: `crates/illumio-tui/src/tui/mod.rs`

**Interfaces:**
- Consumes: Plan 1 (`Action`, `Mark`, `PceScope`, `ServiceSpec`, `parse_side`, `side_text`, `parse_expiry`, `parse_decision`, `expiry_date`).
- Produces:

```rust
// form.rs
FieldKind::{MarkAction, MarkExpiry, MarkScope, MarkSide}
// complete.rs
pub const THIS_PCE: &str = "this PCE";  pub const ANY_PCE: &str = "any PCE";
// edit.rs
EditIntent::Mark   // build_intent returns Err("internal: mark form")
// mark_form.rs
pub const ACTION, SRC, DST, SERVICE, DECISION, DRAFT, EXPIRES, SCOPE, COMMENT: usize;  // 0..=8
pub enum MarkOrigin { Traffic, App, Marks }
pub enum RowsOf { Flows, Pairs(Decisions), Services(Decisions), Sections(Option<Direction>, Decisions), Mesh(MeshColumns), Peers(Box<Focus>, Direction) }
pub struct SidePrefill { pub text: String, pub alt: Option<String>, pub kinds: Vec<String>, pub note: Option<&'static str> }
pub struct Prefill { pub what: String, pub action: Action, pub src: SidePrefill, pub dst: SidePrefill, pub service: String,
                     pub decision: String, pub draft: String, pub rows: RowsOf }
pub struct MarkCtx { pub origin, pub editing: Option<Mark>, pub extend: bool, pub expiry_edited: bool, pub kinds: [Vec<String>; 2], pub rows: RowsOf, pub host: String }
pub struct MarkForm { pub form: Form, pub ctx: MarkCtx, pub preview: Vec<String> }
pub fn endpoint_side(ep: &ResolvedEndpoint, keys: &[String]) -> SidePrefill;
pub fn group_side(group: &[String], keys: &[String], ep: Option<&ResolvedEndpoint>) -> SidePrefill;
pub fn any_side() -> SidePrefill;  pub fn decisions_text(d: Decisions) -> (String, String);  pub fn expires_text(e: Option<Timestamp>) -> String;
impl MarkForm { fn new(p: Prefill, origin: MarkOrigin, host: &str) -> Self; fn edit(m: &Mark, host: &str, extend: bool) -> Self;
                fn cycle_kind(&mut self); fn swap_labels(&mut self); fn sync_expiry(&mut self, expiry_before: &str);
                fn comment(&self) -> &str; fn mark(&self, now: Timestamp) -> Result<Mark, String> }
```

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/src/tui/complete.rs`, replace:

```rust

    #[test]
    fn filter_actors_offer_the_filter_grammar() {
        let all = ff(FieldKind::TrafficActors, "", &[], false);
```

with:

```rust

    #[test]
    fn mark_fields_cycle_their_lists_and_sides_offer_the_side_grammar() {
        assert_eq!(
            fx(FieldKind::MarkAction, "reviewed", &[]),
            s(&[
                "squelch",
                "critical",
                "important",
                "revisit",
                "accept-blocked",
                "reviewed"
            ]),
            "the next action first"
        );
        assert_eq!(
            fx(FieldKind::MarkScope, "this PCE", &[]),
            s(&["any PCE", "this PCE"])
        );
        assert_eq!(fx(FieldKind::MarkExpiry, "9", &[]), s(&["90d"]));
        assert_eq!(
            fx(FieldKind::MarkExpiry, "x", &[]).len(),
            5,
            "no match: all"
        );
        let side = ff(FieldKind::MarkSide, "", &[], false);
        assert_eq!(
            &side[..8],
            &s(&["app=", "env=", "loc=", "role=", "wl:", "ip:", "ipl:", "any"]),
            "keys, then the side grammar's words"
        );
        assert!(ff(FieldKind::MarkSide, "ipl:", &[], false).contains(&"ipl:Corp".to_string()));
        assert!(
            ff(FieldKind::MarkSide, "wl:", &[], true)
                .iter()
                .any(|c| c.starts_with("wl:")),
            "the loaded workloads"
        );
        assert!(
            ff(FieldKind::MarkSide, "ip:10", &[], false).is_empty(),
            "typed"
        );
        assert!(
            !ff(FieldKind::MarkSide, "", &["app=shop"], false).contains(&"app=".to_string()),
            "a key used in the side is not offered again"
        );
        assert_eq!(
            FieldKind::MarkSide.request("app=shop en"),
            Some(("en".to_string(), vec!["app=shop".to_string()]))
        );
    }

    #[test]
    fn filter_actors_offer_the_filter_grammar() {
        let all = ff(FieldKind::TrafficActors, "", &[], false);
```

In `crates/illumio-tui/src/tui/edit.rs`, replace:

```rust
    use serde_json::{Value, json};

    fn labels() -> Vec<Label> {
        serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap()
```

with:

```rust
    use serde_json::{Value, json};

    #[test]
    fn a_mark_form_never_builds_a_write() {
        let form = Form::new("New mark", Vec::new(), EditIntent::Mark);
        assert_eq!(
            build_intent(&form, None, 5),
            Err("internal: mark form".to_string())
        );
    }

    fn labels() -> Vec<Label> {
        serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap()
```

Create `crates/illumio-tui/src/tui/mark_form.rs` with its tests only (Step 3 adds the code above them):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use pce::resolve::ResolvedLabel;

    fn wl(host: &str, labels: &[(&str, &str)]) -> ResolvedEndpoint {
        ResolvedEndpoint {
            ip: Some("10.0.1.10".into()),
            hostname: Some(host.into()),
            workload_href: Some(format!("/orgs/5/workloads/{host}")),
            labels: labels
                .iter()
                .map(|(k, v)| ResolvedLabel {
                    href: format!("/orgs/5/labels/{k}{v}"),
                    key: (*k).into(),
                    value: (*v).into(),
                })
                .collect(),
            ..Default::default()
        }
    }

    fn keys() -> Vec<String> {
        vec!["app".into(), "env".into()]
    }

    fn now() -> jiff::Timestamp {
        "2026-10-17T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn a_workload_endpoint_gives_the_labels_under_the_keys_and_its_kinds() {
        let ep = wl(
            "web-01",
            &[("app", "shop"), ("env", "prod"), ("role", "web")],
        );
        let p = endpoint_side(&ep, &keys());
        assert_eq!(p.text, "app=shop env=prod");
        assert_eq!(p.alt.as_deref(), Some("app=shop env=prod role=web"));
        assert_eq!(
            p.kinds,
            vec!["app=shop env=prod", "wl:web-01", "ip:10.0.1.10", "any"]
        );
        let bare = wl("nas", &[]);
        assert_eq!(endpoint_side(&bare, &keys()).text, "wl:nas");
        let corp = ResolvedEndpoint {
            ip: Some("10.1.4.9".into()),
            ip_lists: vec!["Corp".into()],
            ..Default::default()
        };
        assert_eq!(endpoint_side(&corp, &keys()).text, "ipl:Corp");
        let fqdn = ResolvedEndpoint {
            fqdn: Some("x.example".into()),
            ..Default::default()
        };
        let f = endpoint_side(&fqdn, &keys());
        assert_eq!(
            (f.text.as_str(), f.note),
            ("any", Some("fqdn cannot be marked"))
        );
    }

    #[test]
    fn a_group_key_drops_missing_keys_and_names_ip_lists_and_hosts() {
        let ep = wl("web-01", &[("app", "shop"), ("role", "web")]);
        let g = |v: &[&str], ep: Option<&ResolvedEndpoint>| {
            let v: Vec<String> = v.iter().map(|s| s.to_string()).collect();
            group_side(&v, &keys(), ep).text
        };
        assert_eq!(g(&["shop", "-"], Some(&ep)), "app=shop");
        assert_eq!(g(&["ipl:Corp", "-"], None), "ipl:Corp");
        assert_eq!(g(&["10.0.3.30", "-"], None), "ip:10.0.3.30");
        assert_eq!(g(&["unmanaged-nas", "-"], None), "wl:unmanaged-nas");
    }

    fn form() -> MarkForm {
        let ep = wl(
            "web-01",
            &[("app", "shop"), ("env", "prod"), ("role", "web")],
        );
        let db = wl("db-01", &[("app", "shop"), ("env", "prod"), ("role", "db")]);
        MarkForm::new(
            Prefill {
                what: "Flows · web-01 → db-01 5432/tcp".into(),
                action: Action::Reviewed,
                src: endpoint_side(&ep, &keys()),
                dst: endpoint_side(&db, &keys()),
                service: "5432/tcp".into(),
                decision: String::new(),
                draft: String::new(),
                rows: RowsOf::Flows,
            },
            MarkOrigin::Traffic,
            "pce.example",
        )
    }

    #[test]
    fn the_form_builds_the_mark_it_shows_and_names_the_bad_field() {
        let mut f = form();
        assert_eq!(f.form.active, COMMENT, "the cursor waits on the comment");
        assert_eq!(f.form.value("Expires"), "30d", "reviewed's default");
        f.form.fields[COMMENT].value = "checked with the owner".into();
        let m = f.mark(now()).unwrap();
        assert_eq!(m.action, Action::Reviewed);
        assert_eq!(side_text(&m.src), "app=shop env=prod");
        assert_eq!(m.services[0].text(), "5432/tcp");
        assert_eq!(
            m.scope,
            Some(PceScope {
                host: "pce.example".into(),
                org: None
            })
        );
        assert_eq!(
            m.expires,
            Some(now() + jiff::SignedDuration::from_hours(24 * 30))
        );
        assert_eq!(m.created_on.as_deref(), Some("pce.example"));
        f.form.fields[SERVICE].value = "70000/tcp".into();
        assert!(f.mark(now()).unwrap_err().starts_with("Service: "));
        f.form.fields[SERVICE].value = String::new();
        f.form.fields[SRC].value = "wl:web-01".into();
        f.form.fields[SCOPE].value = ANY_PCE.into();
        assert!(f.mark(now()).unwrap_err().contains("workload needs pce"));
    }

    #[test]
    fn expires_follows_the_action_until_edited_and_ctrl_t_cycles_kinds() {
        let mut f = form();
        f.form.fields[ACTION].value = "squelch".into();
        f.sync_expiry("30d");
        assert_eq!(f.form.value("Expires"), "90d");
        f.form.fields[EXPIRES].value = "7d".into();
        f.sync_expiry("90d");
        f.form.fields[ACTION].value = "critical".into();
        f.sync_expiry("7d");
        assert_eq!(f.form.value("Expires"), "7d", "edited: kept");
        f.form.active = SRC;
        f.cycle_kind();
        assert_eq!(f.form.value("Source"), "wl:web-01");
        f.cycle_kind();
        f.cycle_kind();
        assert_eq!(f.form.value("Source"), "any");
        f.cycle_kind();
        assert_eq!(f.form.value("Source"), "app=shop env=prod", "around");
    }

    #[test]
    fn ctrl_a_swaps_one_side_for_every_label() {
        let mut f = form();
        f.form.active = DST;
        f.swap_labels();
        assert_eq!(f.form.value("Destination"), "app=shop env=prod role=db");
        assert_eq!(
            f.form.value("Source"),
            "app=shop env=prod",
            "the other side stays"
        );
        f.swap_labels();
        assert_eq!(f.form.value("Destination"), "app=shop env=prod");
        f.form.active = SERVICE;
        f.swap_labels();
        assert_eq!(f.form.value("Service"), "5432/tcp", "not a side: nothing");
    }

    #[test]
    fn editing_keeps_the_id_org_and_creation_and_extend_asks_only_why() {
        let mut m = form().mark(now()).unwrap();
        m.id = "m-abc123".into();
        m.comment = "first reason".into();
        m.scope = Some(PceScope {
            host: "pce.example".into(),
            org: Some(5),
        });
        m.created = Some(now());
        let mut e = MarkForm::edit(&m, "pce.example", false);
        assert_eq!(e.form.value("Scope"), THIS_PCE);
        assert_eq!(e.form.value("Comment"), "first reason");
        e.form.fields[COMMENT].value = "second reason".into();
        let back = e.mark(now()).unwrap();
        assert_eq!(back.id, "m-abc123");
        assert_eq!(back.scope.unwrap().org, Some(5));
        assert_eq!(back.created, Some(now()));
        assert_eq!(back.expires, m.expires, "the date round-trips");
        let x = MarkForm::edit(&m, "pce.example", true);
        assert_eq!(x.form.fields.len(), 1);
        assert_eq!(x.comment(), "first reason");
    }
}
```

In `crates/illumio-tui/src/tui/mod.rs`, replace:

```rust
pub mod gen_view;
pub mod impact;
pub mod model;
pub mod msg;
```

with:

```rust
pub mod gen_view;
pub mod impact;
pub mod mark_form;
pub mod model;
pub mod msg;
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p illumio-tui --lib -- mark_form complete::tests::mark_fields a_mark_form_never_builds_a_write`
Expected: FAIL: it does not compile (`no variant MarkSide`, `cannot find type MarkForm`).

- [ ] **Step 3: Implement**

In `crates/illumio-tui/src/tui/complete.rs`, replace:

```rust
        ),
        FieldKind::Window => window_steps(token),
    };
    let mut seen = std::collections::HashSet::new();
```

with:

```rust
        ),
        FieldKind::Window => window_steps(token),
        FieldKind::MarkAction => cycle_words(token, &pce::marks::Action::ALL.map(|a| a.as_str())),
        FieldKind::MarkExpiry => cycle_words(token, &["1d", "7d", "30d", "90d", "never"]),
        FieldKind::MarkScope => cycle_words(token, &[THIS_PCE, ANY_PCE]),
        FieldKind::MarkSide => mark_side(token, &used_keys(earlier), idx, workloads),
    };
    let mut seen = std::collections::HashSet::new();
```

In `crates/illumio-tui/src/tui/complete.rs`, replace:

```rust
    }
    labels(token, used, idx, &FILTER_WORDS, &named_all(&sources))
}

```

with:

```rust
    }
    labels(token, used, idx, &FILTER_WORDS, &named_all(&sources))
}

/// A mark form's scope words.
pub const THIS_PCE: &str = "this PCE";
pub const ANY_PCE: &str = "any PCE";

/// A fixed list cycled by `Ctrl-N`: the word after the one in the field
/// first (so the first `Ctrl-N` changes it), else those matching the
/// typed text, else all of them.
fn cycle_words(token: &str, words: &[&str]) -> Vec<String> {
    let t = token.trim();
    if let Some(i) = words.iter().position(|w| w.eq_ignore_ascii_case(t)) {
        return words[i + 1..]
            .iter()
            .chain(&words[..=i])
            .map(|w| w.to_string())
            .collect();
    }
    let hits = ranked(t, words.iter().map(|w| (w.to_string(), w.to_string())));
    if hits.is_empty() {
        words.iter().map(|w| w.to_string()).collect()
    } else {
        hits
    }
}

/// The prefixes a mark side takes besides labels.
const MARK_WORDS: [&str; 4] = ["wl:", "ip:", "ipl:", "any"];

/// A mark side: labels, `ipl:` names from the index, `wl:` names of the
/// loaded workloads, `ip:` (typed), `any`. Never a request.
fn mark_side(token: &str, used: &[String], idx: &NameIndex, workloads: &[Workload]) -> Vec<String> {
    if token
        .get(..3)
        .is_some_and(|h| h.eq_ignore_ascii_case("ip:"))
    {
        return Vec::new();
    }
    let sources = [
        ("ipl:", idx.ip_list_names()),
        ("wl:", workload_names(workloads)),
    ];
    if let Some(names) = prefixed(token, &sources) {
        return names;
    }
    labels(token, used, idx, &MARK_WORDS, &named_all(&sources))
}

```

In `crates/illumio-tui/src/tui/edit.rs`, replace:

```rust
    /// `w` on a blocked flow: a draft allow rule pre-filled from it.
    RuleFromFlow(Box<ResolvedFlow>),
}

```

with:

```rust
    /// `w` on a blocked flow: a draft allow rule pre-filled from it.
    RuleFromFlow(Box<ResolvedFlow>),
    /// The `m` form (`Overlay::MarkForm`): it never writes to the PCE, so
    /// nothing here builds a write from it.
    Mark,
}

```

In `crates/illumio-tui/src/tui/edit.rs`, replace:

```rust
            Start::Plan(WriteIntent::DeleteDenyRule { href, name })
        }
        EditIntent::RuleFromFlow(f) => {
            let (fields, alt, covering) = flow_rule_fields(&f, idx, prefill);
```

with:

```rust
            Start::Plan(WriteIntent::DeleteDenyRule { href, name })
        }
        // The mark form is built by `mark_form`, never here.
        EditIntent::Mark => Start::Form(Form::new("New mark", Vec::new(), EditIntent::Mark)),
        EditIntent::RuleFromFlow(f) => {
            let (fields, alt, covering) = flow_rule_fields(&f, idx, prefill);
```

In `crates/illumio-tui/src/tui/edit.rs`, replace:

```rust
            Ok(Some(WriteIntent::CreateRule { ruleset, draft }))
        }
        // Deletes never open a form (`start`); a form never carries one.
        EditIntent::DeleteLabel(_)
```

with:

```rust
            Ok(Some(WriteIntent::CreateRule { ruleset, draft }))
        }
        // `Overlay::MarkForm` submits to `marks_submit`; never here.
        EditIntent::Mark => Err("internal: mark form".into()),
        // Deletes never open a form (`start`); a form never carries one.
        EditIntent::DeleteLabel(_)
```

In `crates/illumio-tui/src/tui/form.rs`, replace:

```rust
    /// A traffic window (the whole field): the `+`/`-` step ladder.
    Window,
}

```

with:

```rust
    /// A traffic window (the whole field): the `+`/`-` step ladder.
    Window,
    /// A mark's action (the whole field): the six actions, cycling.
    MarkAction,
    /// A mark's expiry (the whole field): `1d 7d 30d 90d never`, cycling.
    MarkExpiry,
    /// A mark's scope (the whole field): `this PCE`, `any PCE`, cycling.
    MarkScope,
    /// A mark's side in the side grammar: labels (`key=value`, tokens
    /// apart by spaces or commas), `ipl:` and `wl:` names, `ip:`, `any`.
    MarkSide,
}

```

In `crates/illumio-tui/src/tui/form.rs`, replace:

```rust
            }
            Self::Scope | Self::TrafficActors => &[',', '|'],
            Self::Text
            | Self::LabelKey
            | Self::LabelValue
            | Self::Ruleset
            | Self::TrafficService
            | Self::Window => &[],
        }
    }
```

with:

```rust
            }
            Self::Scope | Self::TrafficActors => &[',', '|'],
            Self::MarkSide => &[' ', ','],
            Self::Text
            | Self::LabelKey
            | Self::LabelValue
            | Self::Ruleset
            | Self::TrafficService
            | Self::Window
            | Self::MarkAction
            | Self::MarkExpiry
            | Self::MarkScope => &[],
        }
    }
```

In `crates/illumio-tui/src/tui/form.rs`, replace:

```rust
            | Self::Ruleset
            | Self::TrafficService
            | Self::Window => Vec::new(),
            Self::Labels | Self::Actors | Self::Services | Self::Decisions | Self::Transmission => {
                tokens(head)
```

with:

```rust
            | Self::Ruleset
            | Self::TrafficService
            | Self::Window
            | Self::MarkAction
            | Self::MarkExpiry
            | Self::MarkScope => Vec::new(),
            Self::MarkSide => head
                .split([' ', ','])
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(String::from)
                .collect(),
            Self::Labels | Self::Actors | Self::Services | Self::Decisions | Self::Transmission => {
                tokens(head)
```

In `crates/illumio-tui/src/tui/form.rs`, replace:

```rust
        matches!(
            self,
            Self::Labels | Self::Scope | Self::Actors | Self::TrafficActors
        )
    }
```

with:

```rust
        matches!(
            self,
            Self::Labels | Self::Scope | Self::Actors | Self::TrafficActors | Self::MarkSide
        )
    }
```

In `crates/illumio-tui/src/tui/mark_form.rs`, insert above the tests:

```rust
//! The `m` form (`Overlay::MarkForm`): the generic `Form` for its fields,
//! keys, completion, plus what a mark needs around it: where it came from,
//! the mark it edits, each side's kinds for `Ctrl-T`, the live preview.
//! Pure: `update::marks` opens, keys and saves it; a save writes only
//! `marks.toml`, never the PCE.
use crate::tui::complete::{ANY_PCE, THIS_PCE};
use crate::tui::edit::EditIntent;
use crate::tui::form::{Field, FieldKind, Form};
use pce::marks::{
    Action, Mark, PceScope, ServiceSpec, Side, parse_decision, parse_expiry, parse_side, side_text,
};
use pce::models::PolicyDecision;
use pce::traffic::{Decisions, Direction, Focus, MeshColumns, ResolvedEndpoint};
use std::collections::BTreeMap;

/// The fields, by position.
pub const ACTION: usize = 0;
pub const SRC: usize = 1;
pub const DST: usize = 2;
pub const SERVICE: usize = 3;
pub const DECISION: usize = 4;
pub const DRAFT: usize = 5;
pub const EXPIRES: usize = 6;
pub const SCOPE: usize = 7;
pub const COMMENT: usize = 8;

/// Where `Esc` and a save return to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkOrigin {
    Traffic,
    App,
    /// The `:marks` overlay (an edit or an extend).
    Marks,
}

/// The view the preview counts rows in: the rows of the view the form came
/// from that hold a matching flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowsOf {
    Flows,
    Pairs(Decisions),
    Services(Decisions),
    Sections(Option<Direction>, Decisions),
    Mesh(MeshColumns),
    Peers(Box<Focus>, Direction),
}

/// One side of the prefill: the text, `Ctrl-A`'s alternate (every label
/// of the row's endpoint), `Ctrl-T`'s kinds in cycle order, a note.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SidePrefill {
    pub text: String,
    pub alt: Option<String>,
    pub kinds: Vec<String>,
    pub note: Option<&'static str>,
}

/// What `m` fills the form with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefill {
    /// `App-to-app in · shop/prod ← monitoring/prod 5666/tcp`.
    pub what: String,
    pub action: Action,
    pub src: SidePrefill,
    pub dst: SidePrefill,
    pub service: String,
    pub decision: String,
    pub draft: String,
    pub rows: RowsOf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MarkCtx {
    pub origin: MarkOrigin,
    /// The mark edited (`:marks` Enter) or extended (`e`); `None`: new.
    pub editing: Option<Mark>,
    /// `e`: only the comment is asked; the save adds 30 days.
    pub extend: bool,
    /// Expires follows the action's default until the user edits it.
    pub expiry_edited: bool,
    /// `Ctrl-T`: each side's kinds (source, destination).
    pub kinds: [Vec<String>; 2],
    pub rows: RowsOf,
    /// The profile's host: `this PCE`.
    pub host: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MarkForm {
    pub form: Form,
    pub ctx: MarkCtx,
    /// The live preview's lines (`matches 3 rows · 41 flows …`).
    pub preview: Vec<String>,
}

fn quote_side(labels: BTreeMap<String, Vec<String>>) -> String {
    side_text(&Side::Labels(labels))
}

/// `key=value` for each of `keys` the endpoint has, in the side grammar.
fn labels_under(ep: &ResolvedEndpoint, keys: &[String]) -> String {
    let m: BTreeMap<String, Vec<String>> = keys
        .iter()
        .filter_map(|k| ep.label_value(k).map(|v| (k.clone(), vec![v.to_string()])))
        .collect();
    if m.is_empty() {
        String::new()
    } else {
        quote_side(m)
    }
}

/// Every resolved label of the endpoint (unresolved `?` left out).
fn all_labels(ep: &ResolvedEndpoint) -> String {
    let m: BTreeMap<String, Vec<String>> = ep
        .labels
        .iter()
        .filter(|l| l.key != "?")
        .map(|l| (l.key.clone(), vec![l.value.clone()]))
        .collect();
    if m.is_empty() {
        String::new()
    } else {
        quote_side(m)
    }
}

/// The kinds an endpoint allows, in `Ctrl-T` order: labels, `wl:`, `ip:`,
/// `ipl:`, then `any`; `labels` first when given.
fn endpoint_kinds(ep: Option<&ResolvedEndpoint>, labels: &str) -> Vec<String> {
    let mut k = Vec::new();
    if !labels.is_empty() {
        k.push(labels.to_string());
    }
    if let Some(ep) = ep {
        if ep.is_workload()
            && let Some(h) = ep.hostname.as_deref().or(ep.name.as_deref())
        {
            k.push(format!("wl:{h}"));
        }
        if let Some(ip) = &ep.ip {
            k.push(format!("ip:{ip}"));
        }
        if let Some(l) = ep.ip_lists.first() {
            k.push(format!("ipl:{l}"));
        }
    }
    k.push("any".into());
    k.dedup();
    k
}

/// A flow's endpoint: the labels under the keys in force it has, else
/// `wl:<host>`; a non-workload its first reported IP list, else `ip:`; an
/// FQDN alone `any` (an FQDN cannot be marked).
pub fn endpoint_side(ep: &ResolvedEndpoint, keys: &[String]) -> SidePrefill {
    let labels = if ep.is_workload() {
        labels_under(ep, keys)
    } else {
        String::new()
    };
    let all = all_labels(ep);
    let kinds = endpoint_kinds(Some(ep), &labels);
    let (text, note) = if !labels.is_empty() {
        (labels.clone(), None)
    } else if let Some(h) = ep
        .is_workload()
        .then(|| ep.hostname.as_deref().or(ep.name.as_deref()))
        .flatten()
    {
        (format!("wl:{h}"), None)
    } else if let Some(l) = ep.ip_lists.first() {
        (format!("ipl:{l}"), None)
    } else if let Some(ip) = &ep.ip {
        (format!("ip:{ip}"), None)
    } else {
        ("any".to_string(), Some("fqdn cannot be marked"))
    };
    SidePrefill {
        alt: (ep.is_workload() && !all.is_empty() && all != text).then_some(all),
        text,
        kinds,
        note,
    }
}

/// A group key (pairs, sections, mesh, the app overlay): each value under
/// its key, `-` dropped (a mark cannot require a missing key); `ipl:X` as
/// is; a name group (an IP, an unlabelled host) `ip:` when it parses as
/// an address, else `wl:`. `ep` is one of the row's endpoints: `Ctrl-A`
/// and `Ctrl-T` offer its labels and kinds.
pub fn group_side(group: &[String], keys: &[String], ep: Option<&ResolvedEndpoint>) -> SidePrefill {
    let labelled = ep.is_some_and(|e| e.is_workload() && !e.labels.is_empty());
    let first = group.first().map(String::as_str).unwrap_or("");
    let text = if labelled {
        let m: BTreeMap<String, Vec<String>> = keys
            .iter()
            .zip(group)
            .filter(|(_, v)| v.as_str() != "-")
            .map(|(k, v)| (k.clone(), vec![v.clone()]))
            .collect();
        if m.is_empty() {
            "any".to_string()
        } else {
            quote_side(m)
        }
    } else if let Some(l) = first.strip_prefix("ipl:") {
        format!("ipl:{l}")
    } else if first.parse::<std::net::IpAddr>().is_ok() {
        format!("ip:{first}")
    } else if first.is_empty() || first == "-" {
        "any".to_string()
    } else {
        format!("wl:{first}")
    };
    let all = ep.map(all_labels).unwrap_or_default();
    let mut kinds = endpoint_kinds(ep, if labelled { &text } else { "" });
    if !kinds.contains(&text) {
        kinds.insert(0, text.clone());
    }
    SidePrefill {
        alt: (labelled && !all.is_empty() && all != text).then_some(all),
        text,
        kinds,
        note: None,
    }
}

/// The side `any`, for the peer side of a section header.
pub fn any_side() -> SidePrefill {
    SidePrefill {
        text: "any".into(),
        alt: None,
        kinds: vec!["any".into()],
        note: None,
    }
}

/// A decision filter as the Decision field writes it.
pub fn decisions_text(d: Decisions) -> (String, String) {
    match d {
        Decisions::All => (String::new(), String::new()),
        Decisions::Blocked => ("blocked, potentially_blocked".into(), String::new()),
        Decisions::BlockedOnly => ("blocked".into(), String::new()),
        Decisions::FixedByDraft => ("blocked, potentially_blocked".into(), "allowed".into()),
    }
}

/// The text of an expiry in the Expires field: `never`, `2027-01-15` for
/// the end of a UTC day, else RFC 3339.
pub fn expires_text(e: Option<jiff::Timestamp>) -> String {
    match e {
        None => "never".into(),
        Some(t) if t.strftime("%H:%M:%S").to_string() == "23:59:59" => pce::marks::expiry_date(t),
        Some(t) => t.to_string(),
    }
}

fn decision_list(d: &[PolicyDecision]) -> String {
    d.iter().map(|d| d.as_str()).collect::<Vec<_>>().join(", ")
}

fn scope_text(scope: Option<&PceScope>, host: &str) -> String {
    match scope {
        None => ANY_PCE.into(),
        Some(s) if s.host.eq_ignore_ascii_case(host) => THIS_PCE.into(),
        Some(s) => s.host.clone(),
    }
}

/// The fields, filled.
#[allow(clippy::too_many_arguments)]
fn fields(
    action: Action,
    src: &SidePrefill,
    dst: &SidePrefill,
    service: &str,
    decision: &str,
    draft: &str,
    expires: &str,
    scope: &str,
    comment: &str,
    host: &str,
) -> Vec<Field> {
    let side_hint = |p: &SidePrefill| -> String {
        let mut h = "Ctrl-A all labels · Ctrl-T kind".to_string();
        if let Some(n) = p.note {
            h = format!("{n} · {h}");
        }
        h
    };
    vec![
        Field::new(
            "Action",
            action.as_str(),
            "Ctrl-N: critical important revisit…",
            true,
        )
        .kind(FieldKind::MarkAction),
        Field::new("Source", src.text.clone(), side_hint(src), true).kind(FieldKind::MarkSide),
        Field::new("Destination", dst.text.clone(), side_hint(dst), true).kind(FieldKind::MarkSide),
        Field::new(
            "Service",
            service,
            "empty: any · 8000-8010/tcp · udp",
            false,
        ),
        Field::new("Decision", decision, "empty: any · blocked …", false)
            .kind(FieldKind::Decisions),
        Field::new("Draft", draft, "empty: any · matched with D loaded", false)
            .kind(FieldKind::Decisions),
        Field::new("Expires", expires, "1d 7d 30d 90d YYYY-MM-DD never", false)
            .kind(FieldKind::MarkExpiry),
        Field::new(
            "Scope",
            scope,
            format!("this PCE = {host} · Ctrl-N: any PCE"),
            false,
        )
        .kind(FieldKind::MarkScope),
        Field::new(
            "Comment",
            comment,
            "why: CHG-1182, owner confirmed (required)",
            false,
        ),
    ]
}

impl MarkForm {
    /// A new mark from a row: the action's default expiry, this PCE.
    pub fn new(p: Prefill, origin: MarkOrigin, host: &str) -> Self {
        let mut form = Form::new(
            format!("New mark · {}", p.what),
            fields(
                p.action,
                &p.src,
                &p.dst,
                &p.service,
                &p.decision,
                &p.draft,
                p.action.default_expiry(),
                THIS_PCE,
                "",
                host,
            ),
            EditIntent::Mark,
        );
        let alt: Vec<(usize, String)> = [(SRC, &p.src), (DST, &p.dst)]
            .into_iter()
            .filter_map(|(i, s)| s.alt.clone().map(|a| (i, a)))
            .collect();
        form.alt = (!alt.is_empty()).then_some(alt);
        form.active = COMMENT;
        Self {
            form,
            ctx: MarkCtx {
                origin,
                editing: None,
                extend: false,
                expiry_edited: false,
                kinds: [p.src.kinds, p.dst.kinds],
                rows: p.rows,
                host: host.to_string(),
            },
            preview: Vec::new(),
        }
    }

    /// The mark `m` edits (`:marks` Enter): its fields, its comment
    /// offered for change. `extend` (`e`): the comment only, prefilled.
    pub fn edit(m: &Mark, host: &str, extend: bool) -> Self {
        let side = |s: &Side| SidePrefill {
            text: side_text(s),
            alt: None,
            kinds: vec![side_text(s), "any".into()],
            note: None,
        };
        let (src, dst) = (side(&m.src), side(&m.dst));
        let all = fields(
            m.action,
            &src,
            &dst,
            &m.service_text().unwrap_or_default(),
            &decision_list(&m.decisions),
            &decision_list(&m.draft_decisions),
            &expires_text(m.expires),
            &scope_text(m.scope.as_ref(), host),
            &m.comment,
            host,
        );
        let (title, fields, active) = if extend {
            (
                format!("Extend {} by 30 days · why is it still needed?", m.id),
                vec![all[COMMENT].clone()],
                0,
            )
        } else {
            (format!("Edit mark {}", m.id), all, COMMENT)
        };
        let mut form = Form::new(title, fields, EditIntent::Mark);
        form.active = active;
        Self {
            form,
            ctx: MarkCtx {
                origin: MarkOrigin::Marks,
                editing: Some(m.clone()),
                extend,
                expiry_edited: true,
                kinds: [src.kinds, dst.kinds],
                rows: RowsOf::Flows,
                host: host.to_string(),
            },
            preview: Vec::new(),
        }
    }

    fn value(&self, i: usize) -> &str {
        self.form.fields.get(i).map_or("", |f| f.value.trim())
    }

    /// `Ctrl-T` on a side: the next kind the row allows (labels, `wl:`,
    /// `ip:`, `ipl:`, `any`); elsewhere nothing.
    pub fn cycle_kind(&mut self) {
        let side = match self.form.active {
            SRC => 0,
            DST => 1,
            _ => return,
        };
        let kinds = &self.ctx.kinds[side];
        if kinds.is_empty() {
            return;
        }
        let at = kinds
            .iter()
            .position(|k| k == self.form.fields[self.form.active].value.trim());
        let next = kinds[at.map_or(0, |i| (i + 1) % kinds.len())].clone();
        self.form.fields[self.form.active].value = next;
    }

    /// `Ctrl-A` on a side: its text and every label of the row's endpoint
    /// trade places (the other side stays); elsewhere nothing.
    pub fn swap_labels(&mut self) {
        let at = self.form.active;
        if let Some((_, other)) = self
            .form
            .alt
            .as_mut()
            .and_then(|alt| alt.iter_mut().find(|(i, _)| *i == at))
        {
            std::mem::swap(&mut self.form.fields[at].value, other);
        }
    }

    /// After a key: Expires follows the action's default until edited.
    pub fn sync_expiry(&mut self, expiry_before: &str) {
        if self.ctx.extend {
            return;
        }
        if self.value(EXPIRES) != expiry_before.trim() {
            self.ctx.expiry_edited = true;
        }
        if !self.ctx.expiry_edited
            && let Some(a) = Action::parse(self.value(ACTION))
        {
            self.form.fields[EXPIRES].value = a.default_expiry().into();
        }
    }

    /// The comment as typed.
    pub fn comment(&self) -> &str {
        if self.ctx.extend {
            self.value(0)
        } else {
            self.value(COMMENT)
        }
    }

    /// The mark the fields describe, checked as the file checks one; the
    /// first problem names its field. The edited mark keeps its id, org
    /// (on the same PCE) and creation.
    pub fn mark(&self, now: jiff::Timestamp) -> Result<Mark, String> {
        if self.ctx.extend {
            return self
                .ctx
                .editing
                .clone()
                .ok_or_else(|| "internal: nothing to extend".to_string());
        }
        let field = |name: &str, e: String| format!("{name}: {e}");
        let action = Action::parse(self.value(ACTION)).ok_or_else(|| {
            field(
                "Action",
                format!("{:?} is none of: {}", self.value(ACTION), Action::names()),
            )
        })?;
        let src = parse_side(self.value(SRC)).map_err(|e| field("Source", e))?;
        let dst = parse_side(self.value(DST)).map_err(|e| field("Destination", e))?;
        let services = self
            .value(SERVICE)
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| ServiceSpec::parse(s).map_err(|e| field("Service", e)))
            .collect::<Result<Vec<_>, _>>()?;
        let decisions = |name: &str, text: &str| {
            text.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"))
                .map(|s| parse_decision(s).map_err(|e| field(name, e)))
                .collect::<Result<Vec<_>, _>>()
        };
        let expires = parse_expiry(
            if self.value(EXPIRES).is_empty() {
                "never"
            } else {
                self.value(EXPIRES)
            },
            now,
        )
        .map_err(|e| field("Expires", e))?;
        let scope_v = self.value(SCOPE);
        let host = if scope_v.is_empty() || scope_v.to_lowercase().starts_with("any") {
            None
        } else if scope_v.to_lowercase().starts_with("this") {
            Some(self.ctx.host.clone())
        } else {
            Some(scope_v.to_string())
        };
        let old = self.ctx.editing.as_ref();
        let org = old
            .and_then(|m| m.scope.as_ref())
            .filter(|s| {
                host.as_deref()
                    .is_some_and(|h| s.host.eq_ignore_ascii_case(h))
            })
            .and_then(|s| s.org);
        let m = Mark {
            id: old.map(|m| m.id.clone()).unwrap_or_default(),
            action,
            src,
            dst,
            services,
            decisions: decisions("Decision", self.value(DECISION))?,
            draft_decisions: decisions("Draft", self.value(DRAFT))?,
            scope: host.map(|host| PceScope { host, org }),
            expires,
            comment: self.value(COMMENT).to_string(),
            created: old.and_then(|m| m.created),
            created_on: old
                .and_then(|m| m.created_on.clone())
                .or_else(|| Some(self.ctx.host.clone())),
        };
        if let Some(p) = m.problems().into_iter().next() {
            return Err(p);
        }
        Ok(m)
    }
}

```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p illumio-tui --lib -- mark_form complete::tests::mark_fields a_mark_form_never_builds_a_write`
Expected: PASS.

- [ ] **Step 5: Run the gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: no formatting change, no warning, every test passes.

- [ ] **Step 6: Commit**

```bash
git add crates/illumio-tui/src/tui/complete.rs crates/illumio-tui/src/tui/edit.rs crates/illumio-tui/src/tui/form.rs crates/illumio-tui/src/tui/mark_form.rs crates/illumio-tui/src/tui/mod.rs
git commit -m "feat(tui): mark form fields, completion and MarkForm

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

### Task 2: `m`: the form per view, its keys, the live preview, the save

**Files:**
- Modify: `crates/illumio-tui/src/tui/model.rs`
- Modify: `crates/illumio-tui/src/tui/ui/mod.rs`
- Create: `crates/illumio-tui/src/tui/ui/widgets/marks.rs`
- Modify: `crates/illumio-tui/src/tui/ui/widgets/mod.rs`
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs`
- Modify: `crates/illumio-tui/src/tui/update.rs`
- Create: `crates/illumio-tui/src/tui/update/marks.rs`
- Test (modify): `crates/illumio-tui/src/tui/update/marks_tests.rs`
- Test: `crates/illumio-tui/tests/snapshots/tui_snapshots__mark_form_from_blocked_100x30.snap`
- Test (modify): `crates/illumio-tui/tests/tui_snapshots.rs`

**Interfaces:**
- Consumes: Task 1; Plan 2 (`TrafficState::{marks, marks_pce, draft_loaded, first_flow, section_drill, section_decisions, rebuild_views}`, `rebuild_app_traffic`); Plan 1 (`MarkOp::{Add, Replace, Extend}`).
- Produces:

```rust
Overlay::MarkForm(Box<MarkForm>)
// update/marks.rs (child of update)
pub(super) fn mark_key(model: &mut Model) -> Vec<Cmd>;       // `m` on the Traffic screen; always empty
pub(super) fn mark_app_key(model: &mut Model) -> Vec<Cmd>;   // `m` in the app overlay
pub(super) fn form_key(model: &mut Model, mf: MarkForm, k: KeyEvent) -> Vec<Cmd>;
pub(crate) fn preview(t: &TrafficState, mf: &MarkForm, now: Timestamp) -> Vec<String>;
// ui/widgets/marks.rs
pub fn render_mark_form(model: &Model, mf: &MarkForm, f: &mut Frame, area: Rect);
// ui/widgets/overlay.rs: completion_line is pub(crate)
```

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/src/tui/ui/widgets/mod.rs`, replace:

```rust
pub mod bars;
pub mod generator;
pub mod overlay;
pub mod panel;
```

with:

```rust
pub mod bars;
pub mod generator;
pub mod marks;
pub mod overlay;
pub mod panel;
```

In `crates/illumio-tui/src/tui/update.rs`, replace:

```rust

mod generator;
#[cfg(test)]
mod marks_tests;
```

with:

```rust

mod generator;
mod marks;
#[cfg(test)]
mod marks_tests;
```

In `crates/illumio-tui/src/tui/update/marks_tests.rs`, replace:

```rust
//! The marks' keys on the Traffic screen: `M` toggles squelched rows with
//! a rebuild and never a request; a search being typed takes the key.
use crate::tui::model::TrafficView;
use crate::tui::msg::{Cmd, Msg};
use crate::tui::traffic_marks::tests::{RICH_MARKS, marked_model};
```

with:

```rust
//! The marks' keys on the Traffic screen: `M` toggles squelched rows with
//! a rebuild and never a request; a search being typed takes the key.
use crate::tui::mark_form::{COMMENT, MarkForm, SRC};
use crate::tui::model::{Model, Overlay, TrafficView};
use crate::tui::msg::{Cmd, Msg};
use crate::tui::traffic_marks::tests::{RICH_MARKS, marked_model};
```

In `crates/illumio-tui/src/tui/update/marks_tests.rs`, replace:

```rust
fn key(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

```

with:

```rust
fn key(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn ctrl(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

fn kc(code: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// Sends `msg` and asserts it asked for nothing: marks never reach the PCE.
#[track_caller]
fn quiet(m: &mut Model, msg: Msg) {
    assert_eq!(update(m, msg), Vec::<Cmd>::new());
}

#[track_caller]
fn form(m: &Model) -> &MarkForm {
    match &m.overlay {
        Overlay::MarkForm(f) => f,
        o => panic!("no mark form: {o:?}"),
    }
}

/// The form's fields as `(label, value)`, the comment left out.
fn values(m: &Model) -> Vec<(String, String)> {
    form(m)
        .form
        .fields
        .iter()
        .filter(|f| f.label != "Comment")
        .map(|f| (f.label.to_string(), f.value.clone()))
        .collect()
}

fn expect(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

fn typed(m: &mut Model, text: &str) {
    for c in text.chars() {
        quiet(m, key(c));
    }
}

```

In `crates/illumio-tui/src/tui/update/marks_tests.rs`, replace:

```rust
    assert_eq!(a.proposal, b.proposal);
}
```

with:

```rust
    assert_eq!(a.proposal, b.proposal);
}

#[test]
fn m_opens_the_form_prefilled_from_the_row_in_every_view() {
    use crate::tui::model::By;
    let (mut m, _dir) = marked_model(RICH_MARKS);
    let fields = |action, src, dst, svc, decision, expires| {
        expect(&[
            ("Action", action),
            ("Source", src),
            ("Destination", dst),
            ("Service", svc),
            ("Decision", decision),
            ("Draft", ""),
            ("Expires", expires),
            ("Scope", "this PCE"),
        ])
    };
    // Flows: the first shown flow (shop's web to crm's web on 443).
    quiet(&mut m, key('m'));
    assert_eq!(
        values(&m),
        fields(
            "reviewed",
            "app=shop env=prod",
            "app=crm env=prod",
            "443/tcp",
            "",
            "30d"
        )
    );
    assert!(
        form(&m)
            .form
            .title
            .starts_with("New mark · Flows · shop-web-p.example → crm-web-p.example 443/tcp"),
        "{}",
        form(&m).form.title
    );
    assert_eq!(form(&m).form.active, COMMENT);
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::None);
    // App-to-app pairs, then by service.
    m.traffic.view = TrafficView::Aggregate;
    quiet(&mut m, key('m'));
    assert_eq!(
        values(&m),
        fields(
            "reviewed",
            "app=crm env=prod",
            "app=crm env=prod",
            "",
            "",
            "30d"
        )
    );
    quiet(&mut m, kc(KeyCode::Esc));
    quiet(&mut m, key('o'));
    quiet(&mut m, key('m'));
    assert_eq!(form(&m).form.value("Service"), "3306/tcp");
    quiet(&mut m, kc(KeyCode::Esc));
    // Sections `in`: the header fills the app side only; a peer row both.
    m.traffic.apps_state.by = By::In;
    m.traffic.rebuild_sections();
    quiet(&mut m, key('m'));
    assert_eq!(
        values(&m),
        fields("reviewed", "any", "app=crm env=prod", "", "", "30d")
    );
    quiet(&mut m, kc(KeyCode::Esc));
    quiet(&mut m, key('j'));
    quiet(&mut m, key('m'));
    assert_eq!(
        values(&m),
        fields(
            "reviewed",
            "app=crm env=prod",
            "app=crm env=prod",
            "3306/tcp",
            "",
            "30d"
        )
    );
    quiet(&mut m, kc(KeyCode::Esc));
    // Mesh.
    m.traffic.view = TrafficView::Mesh;
    quiet(&mut m, key('m'));
    assert_eq!(form(&m).form.value("Service"), "3306/tcp");
    quiet(&mut m, kc(KeyCode::Esc));
    // Blocked: accept-blocked, the filter's decisions, 90 days.
    m.traffic.view = TrafficView::Blocked;
    quiet(&mut m, key('j'));
    quiet(&mut m, key('m'));
    assert_eq!(
        values(&m),
        fields(
            "accept-blocked",
            "app=erp env=dev",
            "app=erp env=prod",
            "8443/tcp",
            "blocked, potentially_blocked",
            "90d"
        )
    );
    quiet(&mut m, kc(KeyCode::Esc));
    // The treemap drills first.
    m.traffic.view = TrafficView::Treemap;
    quiet(&mut m, key('m'));
    assert_eq!(m.status.text, "m: Enter drills to the flows, mark there");
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn m_in_the_app_overlay_marks_peers_and_returns_there() {
    use crate::tui::commands::Command;
    let (mut m, _dir) = marked_model(RICH_MARKS);
    update(&mut m, Msg::Command(Command::App(Some("shop/prod".into()))));
    quiet(&mut m, key('m'));
    assert_eq!(
        values(&m),
        expect(&[
            ("Action", "reviewed"),
            ("Source", "app=hr env=prod"),
            ("Destination", "app=shop env=prod"),
            ("Service", "5432/tcp"),
            ("Decision", "potentially_blocked"),
            ("Draft", ""),
            ("Expires", "30d"),
            ("Scope", "this PCE"),
        ])
    );
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::App, "back where it came from");
    quiet(&mut m, kc(KeyCode::Tab));
    quiet(&mut m, kc(KeyCode::Tab));
    quiet(&mut m, key('m'));
    assert_eq!(m.status.text, "m marks traffic rows (Inbound, Outbound)");
    assert_eq!(m.overlay, Overlay::App);
}

#[test]
fn widening_a_side_changes_the_preview() {
    let (mut m, _dir) = marked_model(RICH_MARKS);
    quiet(&mut m, key('m'));
    assert_eq!(
        form(&m).preview,
        vec!["matches 1 row · 1 flow in this result"]
    );
    // To the Source field, then its kinds until `any`.
    for _ in 0..(COMMENT - SRC) {
        quiet(&mut m, kc(KeyCode::BackTab));
    }
    assert_eq!(form(&m).form.active, SRC);
    while form(&m).form.value("Source") != "any" {
        quiet(&mut m, ctrl('t'));
    }
    assert_eq!(
        form(&m).preview,
        vec![
            "matches 3 rows · 3 flows in this result",
            "1 of them go to a stronger mark: revisit m-hr wins",
        ]
    );
    // A squelch: hides them; hr's flow stays shown under its revisit mark.
    quiet(&mut m, kc(KeyCode::BackTab));
    quiet(&mut m, ctrl('u'));
    typed(&mut m, "squelch");
    assert_eq!(
        form(&m).form.value("Expires"),
        "90d",
        "the action's default"
    );
    assert_eq!(
        form(&m).preview,
        vec![
            "matches 3 rows · 3 flows in this result · hides them (M shows)",
            "1 of them stay shown: revisit m-hr wins",
        ]
    );
    // Ctrl-A: every label of the destination's endpoint.
    quiet(&mut m, kc(KeyCode::Tab));
    quiet(&mut m, kc(KeyCode::Tab));
    quiet(&mut m, ctrl('a'));
    assert_eq!(
        form(&m).form.value("Destination"),
        "app=crm datacenter=fra1 env=prod loc=us role=web"
    );
    // Ctrl-U on the service: any service, the same three flows here.
    quiet(&mut m, kc(KeyCode::Tab));
    quiet(&mut m, ctrl('u'));
    assert_eq!(form(&m).form.value("Service"), "");
    assert!(
        form(&m).preview[0].starts_with("matches 3 rows"),
        "{:?}",
        form(&m).preview
    );
    typed(&mut m, "x");
    assert!(
        form(&m)
            .form
            .error
            .as_deref()
            .unwrap()
            .starts_with("Service: "),
        "{:?}",
        form(&m).form.error
    );
    assert!(form(&m).preview.is_empty());
}

#[test]
fn enter_requires_a_comment_then_writes_the_file_and_rebuilds() {
    let (mut m, dir) = marked_model(RICH_MARKS);
    m.traffic.view = TrafficView::Aggregate;
    quiet(&mut m, key('m'));
    quiet(&mut m, kc(KeyCode::Enter));
    assert_eq!(
        form(&m).form.error.as_deref(),
        Some("comment required: why are these flows marked?")
    );
    assert_eq!(form(&m).form.active, COMMENT);
    typed(&mut m, "db traffic is known");
    quiet(&mut m, kc(KeyCode::Tab));
    quiet(&mut m, ctrl('u'));
    typed(&mut m, "squelch");
    quiet(&mut m, kc(KeyCode::Enter));
    assert_eq!(m.overlay, Overlay::None);
    assert!(
        m.status.text.starts_with("mark m-")
            && m.status
                .text
                .ends_with(" saved: squelch, 2 flows · :marks lists marks"),
        "{}",
        m.status.text
    );
    let text = std::fs::read_to_string(dir.path().join("marks.toml")).unwrap();
    assert!(text.contains("comment = \"db traffic is known\""), "{text}");
    assert!(text.contains("pce = \"pce.example\""), "this PCE: {text}");
    assert!(
        m.traffic
            .groups
            .rows()
            .iter()
            .all(|g| !(g.src.join("/") == "crm/prod" && g.dst.join("/") == "crm/prod")),
        "the views rebuilt: the pair is squelched"
    );
}

#[test]
fn a_failed_write_keeps_the_form_open_with_the_error() {
    let (mut m, dir) = marked_model(RICH_MARKS);
    quiet(&mut m, key('m'));
    typed(&mut m, "why");
    std::fs::write(dir.path().join("marks.toml.lock"), "1").unwrap();
    quiet(&mut m, kc(KeyCode::Enter));
    assert_eq!(
        form(&m).form.error.as_deref(),
        Some("marks.toml is locked by another illumio-tui (marks.toml.lock): try again")
    );
}
```

Create `crates/illumio-tui/tests/snapshots/tui_snapshots__mark_form_from_blocked_100x30.snap`:

```text
---
source: crates/illumio-tui/tests/tui_snapshots.rs
expression: "render_to_string(&m, 100, 30)"
---
 illumio-tui pce.example · org 5 ●                                            api 5 calls · 20.8 KB
  1 Traffic  │  2 Policy  │  3 Objects  │  4 Workloads  │  5 Events
╭ Filters ────────────────────────────────────────────────────── window 24h · cap 1000 · unbounded ╮
│window 24h  src *  dst *  svc *  decision all                                                     │
╰──────────────────────────────────────────────────────────────────────────────────────────────────╯
╭ Result ─────────────────────────────────────────────────────────────────────── 30 flows · 3s ago ╮
│30 flows allowed 8 pot.blocked 6 blocked 6  ██████████████████████████████████████████████████████│
╰──────────────────────────────────────────────────────────────────────────────────────────────────╯
╭ ╭ New mark · Blocked in · erp/prod ← erp/dev 8443/tcp ─── Tab next · Enter saves · Esc cancels ╮ ╮
│ │  Action       accept-blocked                       Ctrl-N: critical important revisit…       │ │
│ │  Source       app=erp env=dev                      Ctrl-A all labels · Ctrl-T kind           │ │
│▸│  Destination  app=erp env=prod                     Ctrl-A all labels · Ctrl-T kind           │2│
│ │  Service      8443/tcp                             empty: any · 8000-8010/tcp · udp          │5│
│ │  Decision     blocked, potentially_blocked         empty: any · blocked …                    │4│
│ │  Draft                                             empty: any · matched with D loaded        │1│
│ │  Expires      90d                                  1d 7d 30d 90d YYYY-MM-DD never            │ │
│ │  Scope        this PCE                             this PCE = pce.example · Ctrl-N: any PCE  │6│
│ │▸ Comment      CHG-1182, owner confirmed▏           why: CHG-1182, owner confirmed (required) │2│
│ │──────────────────────────────────────────────────────────────────────────────────────────────│1│
│ │matches 1 row · 1 flow in this result                                                         │ │
│ │Enter saves to marks.toml · Esc cancels                                                       │5│
│ ╰──────────────────────────────────────────────────────────────────────────────────────────────╯ │
│      crm/dev                                     api.partner.ex 443/tcp       1    18 10-10 07:19│
│    ▾ crm/prod  in · 1 group · 12 conns · 1 blocked                                               │
│      crm/dev                                     crm-db-p.examp 3306/tcp      1    12 10-10 07:20│
│  ◆ ▾ hr/prod  in · 1 group · 5 conns · 1 blocked                                                 │
│  ◆   hr/dev                                      hr-web-p.examp icmp          1     5 10-10 07:00│
│    ▾ shop/dev  in · 1 group · 3 conns · 1 pot.blocked                                            │
╰──────────────────────────────────────────────────────────────────────────────────────────────────╯
ready                             f filter  r run  a auto  v view  g group  / search  : cmd  ? help
```

In `crates/illumio-tui/tests/tui_snapshots.rs`, replace:

```rust
        render_to_string(&m, 120, 34)
    );
}
```

with:

```rust
        render_to_string(&m, 120, 34)
    );
}

#[test]
fn mark_form_from_blocked_100x30() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use illumio_tui::tui::model::TrafficView;
    use illumio_tui::tui::test_support::{RICH_MARKS, model_with_marks};
    use illumio_tui::tui::{Msg, update};
    let dir = tempfile::tempdir().unwrap();
    let mut m = model_with_marks(dir.path(), RICH_MARKS);
    m.traffic.view = TrafficView::Blocked;
    let key = |c| Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    update(&mut m, key('j'));
    update(&mut m, key('m'));
    for c in "CHG-1182, owner confirmed".chars() {
        update(&mut m, key(c));
    }
    insta::assert_snapshot!(
        "mark_form_from_blocked_100x30",
        render_to_string(&m, 100, 30)
    );
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p illumio-tui --lib marks_tests`
Expected: FAIL: it does not compile (`file not found for module marks`: Step 3 creates `update/marks.rs`).

- [ ] **Step 3: Implement**

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
    /// Create or edit form for a label, IP list, service or workload.
    Form(Form),
    /// The exact request a write would send; `y` sends it, `n`/`Esc` cancel.
    /// `scroll` is the first body line shown.
```

with:

```rust
    /// Create or edit form for a label, IP list, service or workload.
    Form(Form),
    /// `m`: a mark for the selected row; saved to `marks.toml`, never sent.
    MarkForm(Box<crate::tui::mark_form::MarkForm>),
    /// The exact request a write would send; `y` sends it, `n`/`Esc` cancel.
    /// `scroll` is the first body line shown.
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
            (Overlay::AppPicker(a), Overlay::AppPicker(b)) => a == b,
            (Overlay::Form(a), Overlay::Form(b)) => a == b,
            (
                Overlay::ConfirmWrite {
```

with:

```rust
            (Overlay::AppPicker(a), Overlay::AppPicker(b)) => a == b,
            (Overlay::Form(a), Overlay::Form(b)) => a == b,
            (Overlay::MarkForm(a), Overlay::MarkForm(b)) => a == b,
            (
                Overlay::ConfirmWrite {
```

In `crates/illumio-tui/src/tui/ui/mod.rs`, replace:

```rust
        }
        Overlay::Form(form) => widgets::overlay::render_form(model, form, f, area),
        Overlay::ConfirmWrite { plan, kind, scroll } => {
            widgets::overlay::render_confirm_write(model, plan, *kind, *scroll, f, area)
```

with:

```rust
        }
        Overlay::Form(form) => widgets::overlay::render_form(model, form, f, area),
        Overlay::MarkForm(mf) => widgets::marks::render_mark_form(model, mf, f, area),
        Overlay::ConfirmWrite { plan, kind, scroll } => {
            widgets::overlay::render_confirm_write(model, plan, *kind, *scroll, f, area)
```

Create `crates/illumio-tui/src/tui/ui/widgets/marks.rs`:

```rust
//! The mark form (`m`): one row per field, the value and its hint side by
//! side, then the live preview and the save line.
use crate::tui::form::{clip, visible_tail};
use crate::tui::mark_form::MarkForm;
use crate::tui::model::Model;
use crate::tui::ui::layout::centered;
use crate::tui::ui::screens::workloads::plain;
use crate::tui::ui::widgets::overlay::completion_line;
use crate::tui::ui::widgets::panel::panel;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

/// The label column.
const LABEL_W: usize = 13;
/// The value column.
const VALUE_W: usize = 34;

pub fn render_mark_form(model: &Model, mf: &MarkForm, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let form = &mf.form;
    let preview = mf.preview.len().max(1);
    let rows = form.fields.len() + 1 + preview + 1 + 2;
    let r = centered(area, 96, u16::try_from(rows).unwrap_or(u16::MAX));
    f.render_widget(Clear, r);
    let inner_w = (r.width as usize).saturating_sub(2);
    let value_w = VALUE_W.min(inner_w.saturating_sub(2 + LABEL_W + 4));
    let hint_w = inner_w.saturating_sub(2 + LABEL_W + value_w + 3);
    let block = panel(&form.title, "Tab next · Enter saves · Esc cancels", t, true);
    let mut lines: Vec<Line> = Vec::new();
    for (i, field) in form.fields.iter().enumerate() {
        let on = i == form.active;
        let shown = plain(&visible_tail(&field.value, value_w));
        let value = format!("{shown}{}", if on { "▏" } else { "" });
        let pad = (value_w + 1).saturating_sub(value.chars().count());
        let mut spans = vec![
            Span::styled(if on { "▸ " } else { "  " }, Style::new().fg(t.accent)),
            Span::styled(
                format!("{:<w$}", field.label, w = LABEL_W),
                t.header_style(),
            ),
            Span::styled(
                value,
                if on {
                    Style::new().fg(t.fg).bg(t.selected_bg)
                } else {
                    t.base()
                },
            ),
            Span::raw(" ".repeat(pad + 2)),
        ];
        match form.completion_strip().filter(|_| on) {
            Some(s) => spans.extend(completion_line(&s, 0, hint_w, t).spans),
            None => spans.push(Span::styled(
                clip(&plain(&field.hint), hint_w),
                t.dim_style(),
            )),
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(Span::styled("─".repeat(inner_w), t.dim_style())));
    if mf.preview.is_empty() {
        lines.push(Line::from(Span::styled(
            "preview: fix the fields above",
            t.dim_style(),
        )));
    }
    for p in &mf.preview {
        lines.push(Line::from(Span::styled(clip(p, inner_w), t.base())));
    }
    lines.push(match &form.error {
        Some(e) => Line::from(Span::styled(
            clip(&plain(e), inner_w),
            Style::new().fg(t.bad),
        )),
        None => Line::from(Span::styled(
            "Enter saves to marks.toml · Esc cancels",
            t.dim_style(),
        )),
    });
    f.render_widget(Paragraph::new(lines).block(block).style(t.base()), r);
}
```

In `crates/illumio-tui/src/tui/ui/widgets/overlay.rs`, replace:

```rust
/// in the accent colour, the others and `+N more` dim, two blanks apart,
/// `indent` cells in (under the value), cut to `width`.
fn completion_line<'a>(
    strip: &[(String, bool)],
    indent: usize,
```

with:

```rust
/// in the accent colour, the others and `+N more` dim, two blanks apart,
/// `indent` cells in (under the value), cut to `width`.
pub(crate) fn completion_line<'a>(
    strip: &[(String, bool)],
    indent: usize,
```

In `crates/illumio-tui/src/tui/update.rs`, replace:

```rust
            Vec::new()
        }
        Overlay::Form(mut form)
            if k.modifiers.contains(KeyModifiers::CONTROL)
```

with:

```rust
            Vec::new()
        }
        Overlay::MarkForm(mf) => marks::form_key(model, *mf, k),
        Overlay::Form(mut form)
            if k.modifiers.contains(KeyModifiers::CONTROL)
```

In `crates/illumio-tui/src/tui/update.rs`, replace:

```rust
        // Squelched flows shown or left out: a rebuild, no request.
        KeyCode::Char('M') => model.status = model.traffic.toggle_squelched(),
        KeyCode::Char('g') => {
            model.overlay = Overlay::GroupKeys(open_key_picker(model));
```

with:

```rust
        // Squelched flows shown or left out: a rebuild, no request.
        KeyCode::Char('M') => model.status = model.traffic.toggle_squelched(),
        // The mark form for the row: marks.toml only, no request.
        KeyCode::Char('m') => return marks::mark_key(model),
        KeyCode::Char('g') => {
            model.overlay = Overlay::GroupKeys(open_key_picker(model));
```

In `crates/illumio-tui/src/tui/update.rs`, replace:

```rust
        }
        KeyCode::Enter => return app_enter(model),
        KeyCode::Char('w') => return app_rule_key(model),
        KeyCode::Char('r') => app_reload_key(model),
```

with:

```rust
        }
        KeyCode::Enter => return app_enter(model),
        KeyCode::Char('m') => return marks::mark_app_key(model),
        KeyCode::Char('w') => return app_rule_key(model),
        KeyCode::Char('r') => app_reload_key(model),
```

Create `crates/illumio-tui/src/tui/update/marks.rs`:

```rust
//! The marks' keys: `m` opens the mark form prefilled from the row, its
//! keys and live preview, the save to `marks.toml`. No path here returns a
//! `Cmd`: marks never reach the PCE.
use super::complete_field;
use crate::marks::MarkOp;
use crate::tui::app_view::AppPanel;
use crate::tui::form::FormEvent;
use crate::tui::mark_form::{
    MarkForm, MarkOrigin, Prefill, RowsOf, SidePrefill, any_side, decisions_text, endpoint_side,
    group_side,
};
use crate::tui::model::{
    By, Drill, FlowRow, Model, Overlay, RowKind, SectionRow, StatusLine, TrafficState, TrafficView,
};
use crate::tui::msg::Cmd;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pce::marks::Action;
use pce::traffic::{
    Decisions, Direction, FlowGroup, MeshRow, ResolvedFlow, app_sections, app_sections_both,
    group_by_labels, group_by_service, mesh,
};
use std::collections::BTreeMap;

/// `m` on the Traffic screen: the form for the selected row, or why not.
/// Never a request.
pub(super) fn mark_key(model: &mut Model) -> Vec<Cmd> {
    let t = &model.traffic;
    if t.marks.error().is_some() {
        model.status = StatusLine::warn("marks.toml invalid: :marks shows the problems");
        return Vec::new();
    }
    if t.data.is_none() {
        model.status = StatusLine::info("no traffic loaded: r runs the query");
        return Vec::new();
    }
    let prefill = match t.view {
        TrafficView::Flows => t.flows.selected_item().map(|r| flow_prefill(t, r)),
        TrafficView::Aggregate if t.apps_state.by == By::Pairs => {
            t.pairs_table().selected_item().map(|g| pair_prefill(t, g))
        }
        TrafficView::Aggregate => t
            .apps
            .selected_item()
            .and_then(|r| section_prefill(t, r, false)),
        TrafficView::Mesh => t.mesh.selected_item().map(|r| mesh_prefill(t, r)),
        TrafficView::Blocked => t
            .blocked
            .selected_item()
            .and_then(|r| section_prefill(t, r, true)),
        TrafficView::Treemap => {
            model.status = StatusLine::info("m: Enter drills to the flows, mark there");
            return Vec::new();
        }
    };
    let Some(p) = prefill else {
        model.status = StatusLine::info("no row to mark");
        return Vec::new();
    };
    let host = model.profile.host.clone();
    open_form(model, MarkForm::new(p, MarkOrigin::Traffic, &host));
    Vec::new()
}

/// `m` in the app overlay: the selected peer of the Inbound or Outbound
/// panel; the Policy and Workloads panels say where marks go.
pub(super) fn mark_app_key(model: &mut Model) -> Vec<Cmd> {
    let Some(v) = model.app_view.as_ref() else {
        return Vec::new();
    };
    if matches!(v.panel, AppPanel::Policy | AppPanel::Workloads) {
        model.status = StatusLine::info("m marks traffic rows (Inbound, Outbound)");
        return Vec::new();
    }
    if model.traffic.marks.error().is_some() {
        model.status = StatusLine::warn("marks.toml invalid: :marks shows the problems");
        return Vec::new();
    }
    let Some((dir, p)) = v.selected_peer() else {
        model.status = StatusLine::info("no row to mark");
        return Vec::new();
    };
    let focus = v.focus.clone();
    let drill = Drill::Peer {
        app: focus.app.clone(),
        direction: dir,
        peer: p.peer.clone(),
        service: p.service.clone(),
        decisions: Decisions::All,
    };
    let t = &model.traffic;
    let rep = t.first_flow(|f| drill.matches(f, &focus.keys) && f.policy_decision == p.decision);
    let (peer_ep, app_ep) = split(rep.as_ref(), dir);
    let peer = group_side(&p.peer, &focus.keys, peer_ep);
    let app = group_side(&focus.app, &focus.keys, app_ep);
    let (src, dst) = match dir {
        Direction::In => (peer, app),
        Direction::Out => (app, peer),
    };
    let decision = match p.decision {
        pce::models::PolicyDecision::Unknown => String::new(),
        d => d.as_str().to_string(),
    };
    let prefill = Prefill {
        what: format!(
            "{} {} · {} {}",
            focus.app_text(),
            dir.as_str(),
            p.peer_text(),
            p.service
        ),
        action: Action::Reviewed,
        src,
        dst,
        service: p.service.clone(),
        decision,
        draft: String::new(),
        rows: RowsOf::Peers(Box::new(focus), dir),
    };
    let host = model.profile.host.clone();
    open_form(model, MarkForm::new(prefill, MarkOrigin::App, &host));
    Vec::new()
}

/// The endpoints of `f` on the peer and the app side of `dir`.
fn split(
    f: Option<&ResolvedFlow>,
    dir: Direction,
) -> (
    Option<&pce::traffic::ResolvedEndpoint>,
    Option<&pce::traffic::ResolvedEndpoint>,
) {
    match (f, dir) {
        (Some(f), Direction::In) => (Some(&f.src), Some(&f.dst)),
        (Some(f), Direction::Out) => (Some(&f.dst), Some(&f.src)),
        (None, _) => (None, None),
    }
}

fn flow_prefill(t: &TrafficState, r: &FlowRow) -> Prefill {
    let service = r.service_text();
    Prefill {
        what: format!(
            "Flows · {} → {} {service}",
            r.src.display_name(),
            r.dst.display_name()
        ),
        action: Action::Reviewed,
        src: endpoint_side(&r.src, &t.keys),
        dst: endpoint_side(&r.dst, &t.keys),
        service,
        decision: String::new(),
        draft: String::new(),
        rows: RowsOf::Flows,
    }
}

fn pair_prefill(t: &TrafficState, g: &FlowGroup) -> Prefill {
    let d = t.apps_state.decisions;
    let drill = match &g.service {
        Some(s) => Drill::Service(g.src.clone(), g.dst.clone(), s.clone(), d),
        None => Drill::Pair(g.src.clone(), g.dst.clone(), d),
    };
    let rep = t.first_flow(|f| drill.matches(f, &t.keys));
    let (decision, draft) = decisions_text(d);
    let service = g.service.clone().unwrap_or_default();
    Prefill {
        what: format!(
            "App-to-app pairs · {} → {} {service}",
            g.src.join("/"),
            g.dst.join("/")
        )
        .trim_end()
        .to_string(),
        action: Action::Reviewed,
        src: group_side(&g.src, &t.keys, rep.as_ref().map(|f| &f.src)),
        dst: group_side(&g.dst, &t.keys, rep.as_ref().map(|f| &f.dst)),
        service,
        decision,
        draft,
        rows: if g.service.is_some() {
            RowsOf::Services(d)
        } else {
            RowsOf::Pairs(d)
        },
    }
}

fn mesh_prefill(t: &TrafficState, r: &MeshRow) -> Prefill {
    let drill = Drill::Mesh {
        key: r.key(),
        cols: t.mesh_cols,
    };
    let rep = t.first_flow(|f| drill.matches(f, &t.keys));
    let service = r.service_text();
    Prefill {
        what: format!("Mesh · {} → {} {service}", r.src.join("/"), r.dst.join("/")),
        action: Action::Reviewed,
        src: group_side(&r.src, &t.keys, rep.as_ref().map(|f| &f.src)),
        dst: group_side(&r.dst, &t.keys, rep.as_ref().map(|f| &f.dst)),
        service,
        decision: String::new(),
        draft: String::new(),
        rows: RowsOf::Mesh(t.mesh_cols),
    }
}

/// A section's peer row (direction-aware: in `in` the peer is the
/// source) or its header (the app side only; the peer side `any`).
fn section_prefill(t: &TrafficState, r: &SectionRow, blocked: bool) -> Option<Prefill> {
    let drill = t.section_drill()?;
    let decisions = t.section_decisions();
    let (decision, draft) = decisions_text(decisions);
    let rep = t.first_flow(|f| drill.matches(f, &t.keys));
    let dir_of_view = if blocked {
        Some(Direction::In)
    } else {
        match t.apps_state.by {
            By::In => Some(Direction::In),
            By::Out => Some(Direction::Out),
            _ => None,
        }
    };
    let view = if blocked { "Blocked" } else { "App-to-app" };
    let app_text = r.app.join("/");
    let (src, dst, service, what) = match &r.kind {
        RowKind::Header { totals, .. } => {
            // A `both` header: the app as the destination.
            let dir = match totals.as_slice() {
                [(d, _)] => *d,
                _ => Direction::In,
            };
            let (_, app_ep) = split(rep.as_ref(), dir);
            let app = group_side(&r.app, &t.keys, app_ep);
            let (src, dst) = match dir {
                Direction::In => (any_side(), app),
                Direction::Out => (app, any_side()),
            };
            (src, dst, String::new(), format!("{view} · {app_text}"))
        }
        RowKind::Peer { direction, row, .. } => {
            let (peer_ep, app_ep) = split(rep.as_ref(), *direction);
            let peer: SidePrefill = group_side(&row.peer, &t.keys, peer_ep);
            let app = group_side(&r.app, &t.keys, app_ep);
            let what = format!(
                "{view} {} · {app_text} {} {} {}",
                direction.as_str(),
                direction.arrow(),
                row.peer_text(),
                row.service
            );
            match direction {
                Direction::In => (peer, app, row.service.clone(), what),
                Direction::Out => (app, peer, row.service.clone(), what),
            }
        }
    };
    Some(Prefill {
        what,
        action: if blocked {
            Action::AcceptBlocked
        } else {
            Action::Reviewed
        },
        src,
        dst,
        service,
        decision,
        draft,
        rows: RowsOf::Sections(dir_of_view, decisions),
    })
}

/// The rows of the view `rows` names that hold one of `flows`: the view's
/// own aggregation of them.
fn rows_of(t: &TrafficState, rows: &RowsOf, flows: &[ResolvedFlow]) -> usize {
    let keys = &t.keys;
    match rows {
        RowsOf::Flows => flows.len(),
        RowsOf::Pairs(d) => group_by_labels(&d.kept(flows), keys).len(),
        RowsOf::Services(d) => group_by_service(&d.kept(flows), keys).len(),
        RowsOf::Sections(Some(dir), d) => app_sections(flows, keys, *dir, *d)
            .iter()
            .map(|s| s.rows.len())
            .sum(),
        RowsOf::Sections(None, d) => app_sections_both(flows, keys, *d)
            .iter()
            .map(|s| s.rows.len())
            .sum(),
        RowsOf::Mesh(cols) => mesh(flows, keys, *cols).len(),
        RowsOf::Peers(focus, dir) => pce::app::app_peers(flows, focus, *dir).len(),
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The live preview: the loaded result (all of it, not only the focus)
/// classified against the active marks plus this one. Local; no request.
pub(crate) fn preview(t: &TrafficState, mf: &MarkForm, now: jiff::Timestamp) -> Vec<String> {
    let Ok(draft) = mf.mark(now) else {
        return Vec::new();
    };
    let matched: Vec<ResolvedFlow> = t
        .loaded_flows()
        .iter()
        .filter(|f| draft.matches(f))
        .cloned()
        .collect();
    if matched.is_empty() {
        return vec!["matches nothing in this result".into()];
    }
    let rows = rows_of(t, &mf.ctx.rows, &matched);
    let squelch = draft.action == Action::Squelch;
    let mut first = format!(
        "matches {} · {} in this result",
        plural(rows, "row", "rows"),
        plural(matched.len(), "flow", "flows")
    );
    if squelch {
        first.push_str(" · hides them (M shows)");
    }
    let mut lines = vec![first];
    // The file's marks (the edited one replaced) plus this one.
    let mut set = t.marks.set().cloned().unwrap_or_default();
    let at = match mf.ctx.editing.as_ref().and_then(|m| set.find(&m.id)) {
        Some(i) => {
            set.marks[i] = draft.clone();
            i
        }
        None => {
            set.marks.push(draft.clone());
            set.marks.len() - 1
        }
    };
    let active = set.active(Some(&t.marks_pce), now, t.draft_loaded());
    if !active.is_active(at) {
        lines.push("not active here: another PCE, expired, or waiting for D".into());
        return lines;
    }
    let mut winners: BTreeMap<usize, u64> = BTreeMap::new();
    for f in &matched {
        if let Some(h) = active.classify(f)
            && h.mark != at
        {
            *winners.entry(h.mark).or_default() += 1;
        }
    }
    let k: u64 = winners.values().sum();
    if let Some((&top, _)) = winners.iter().max_by_key(|(_, n)| **n) {
        let w = &set.marks[top];
        lines.push(format!(
            "{k} of them {}: {} {} wins",
            if squelch {
                "stay shown"
            } else {
                "go to a stronger mark"
            },
            w.action.as_str(),
            w.id
        ));
    }
    lines
}

/// The form's validation and preview after a key.
fn refresh(t: &TrafficState, mf: &mut MarkForm) {
    let now = jiff::Timestamp::now();
    mf.form.error = mf.mark(now).err();
    mf.preview = preview(t, mf, now);
}

fn open_form(model: &mut Model, mut mf: MarkForm) {
    refresh(&model.traffic, &mut mf);
    model.overlay = Overlay::MarkForm(Box::new(mf));
}

/// Back where the form came from.
fn close(model: &mut Model, origin: MarkOrigin) {
    model.overlay = match origin {
        MarkOrigin::App if model.app_view.is_some() => Overlay::App,
        _ => Overlay::None,
    };
}

/// The form's keys: `Ctrl-N`/`Ctrl-P` complete from the loaded data (no
/// request), `Ctrl-T` cycles a side's kind, `Ctrl-A` swaps all labels in,
/// `Enter` saves, `Esc` returns.
pub(super) fn form_key(model: &mut Model, mut mf: MarkForm, k: KeyEvent) -> Vec<Cmd> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let before = mf.form.value("Expires").to_string();
    if ctrl && matches!(k.code, KeyCode::Char('n' | 'p')) {
        complete_field(model, &mut mf.form, k.code == KeyCode::Char('n'));
    } else if ctrl && k.code == KeyCode::Char('t') {
        mf.cycle_kind();
    } else if ctrl && k.code == KeyCode::Char('a') {
        mf.swap_labels();
    } else {
        match mf.form.on_key(k) {
            FormEvent::Continue | FormEvent::NewRuleset => {}
            FormEvent::Cancel => {
                close(model, mf.ctx.origin);
                model.status = StatusLine::info("cancelled, nothing written");
                return Vec::new();
            }
            FormEvent::Submit => return submit(model, mf),
        }
    }
    mf.sync_expiry(&before);
    refresh(&model.traffic, &mut mf);
    model.overlay = Overlay::MarkForm(Box::new(mf));
    Vec::new()
}

/// `Enter`: the comment is required; then one operation on `marks.toml`
/// (lock, reload, write), the views rebuilt, back where the form came
/// from. A failure keeps the form open with the error. Never a `Cmd`.
fn submit(model: &mut Model, mut mf: MarkForm) -> Vec<Cmd> {
    let now = jiff::Timestamp::now();
    if mf.comment().is_empty() {
        mf.form.active = if mf.ctx.extend {
            0
        } else {
            crate::tui::mark_form::COMMENT
        };
        mf.form.error = Some("comment required: why are these flows marked?".into());
        model.overlay = Overlay::MarkForm(Box::new(mf));
        return Vec::new();
    }
    let mark = match mf.mark(now) {
        Ok(m) => m,
        Err(e) => {
            mf.form.error = Some(e);
            model.overlay = Overlay::MarkForm(Box::new(mf));
            return Vec::new();
        }
    };
    let flows = model
        .traffic
        .loaded_flows()
        .iter()
        .filter(|f| mark.matches(f))
        .count();
    let op = match (&mf.ctx.editing, mf.ctx.extend) {
        (Some(m), true) => MarkOp::Extend {
            id: m.id.clone(),
            days: 30,
            comment: Some(mf.comment().to_string()),
        },
        (Some(_), false) => MarkOp::Replace(mark.clone()),
        (None, _) => MarkOp::Add(mark.clone()),
    };
    match model.traffic.marks.apply(op, now) {
        Ok(done) => {
            let id = done.ids.first().cloned().unwrap_or(mark.id.clone());
            model.traffic.rebuild_views();
            super::rebuild_app_traffic(model);
            close(model, mf.ctx.origin);
            let backup = if done.backup.is_some() {
                " · marks.toml had comments: kept in marks.toml.bak"
            } else {
                ""
            };
            model.status = StatusLine::info(if mf.ctx.extend {
                format!("mark {id} extended by 30 days{backup}")
            } else {
                format!(
                    "mark {id} saved: {}, {} · :marks lists marks{backup}",
                    mark.action.as_str(),
                    plural(flows, "flow", "flows")
                )
            });
        }
        Err(e) => {
            mf.form.error = Some(e.to_string());
            model.overlay = Overlay::MarkForm(Box::new(mf));
        }
    }
    Vec::new()
}
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p illumio-tui --lib marks_tests && cargo test -p illumio-tui --test tui_snapshots`
Expected: PASS.

- [ ] **Step 5: Run the gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: no formatting change, no warning, every test passes.

- [ ] **Step 6: Commit**

```bash
git add crates/illumio-tui/src/tui/model.rs crates/illumio-tui/src/tui/ui/mod.rs crates/illumio-tui/src/tui/ui/widgets/marks.rs crates/illumio-tui/src/tui/ui/widgets/mod.rs crates/illumio-tui/src/tui/ui/widgets/overlay.rs crates/illumio-tui/src/tui/update.rs crates/illumio-tui/src/tui/update/marks.rs crates/illumio-tui/src/tui/update/marks_tests.rs crates/illumio-tui/tests/snapshots/tui_snapshots__mark_form_from_blocked_100x30.snap crates/illumio-tui/tests/tui_snapshots.rs
git commit -m "feat(tui): m marks the row: prefilled form, live preview, save to marks.toml

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

### Task 3: The `:marks` overlay: list, filter, edit, delete, extend

**Files:**
- Modify: `crates/illumio-tui/src/tui/commands.rs`
- Create: `crates/illumio-tui/src/tui/marks_view.rs`
- Modify: `crates/illumio-tui/src/tui/mod.rs`
- Modify: `crates/illumio-tui/src/tui/model.rs`
- Modify: `crates/illumio-tui/src/tui/ui/mod.rs`
- Modify: `crates/illumio-tui/src/tui/ui/widgets/marks.rs`
- Modify: `crates/illumio-tui/src/tui/update.rs`
- Modify: `crates/illumio-tui/src/tui/update/marks.rs`
- Test (modify): `crates/illumio-tui/src/tui/update/marks_tests.rs`
- Test: `crates/illumio-tui/tests/snapshots/tui_snapshots__marks_overlay_120x30.snap`
- Test (modify): `crates/illumio-tui/tests/tui_snapshots.rs`

**Interfaces:**
- Consumes: Tasks 1-2 (`MarkForm::edit`, `open_form`, `close`), Plan 1 (`MarkOp::Delete`, `MarksError::lines`, `expiry_date`).
- Produces:

```rust
Overlay::Marks;  PendingAction::DeleteMark(String);  Model.marks_view: Option<MarksView>;  Command::Marks(Option<String>)
// marks_view.rs
pub struct MarkRow { pub mark: Mark, pub active: bool, pub scope: String, pub expires: String, pub hits: String, pub hit_count: u64 }
pub fn mark_columns() -> Vec<Column<MarkRow>>;
pub struct MarksView { pub table: TableView<MarkRow>, pub filter: Option<Action>, pub problems: Option<Vec<String>>, pub path: String, pub active: usize, pub expired: usize }
impl MarksView { fn new(filter: Option<Action>) -> Self; fn rebuild(&mut self, t: &TrafficState, now: Timestamp); fn title; fn meta; fn selected }
// update/marks.rs
pub(super) fn open_marks(model: &mut Model, action: Option<&str>) -> Vec<Cmd>;
pub(super) fn overlay_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd>;
pub(super) fn delete_mark(model: &mut Model, id: &str) -> Vec<Cmd>;
// ui/widgets/marks.rs
pub fn render_marks(model: &Model, f: &mut Frame, area: Rect);
```

`m` on an invalid file now opens the overlay with the problems.

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/src/tui/commands.rs`, replace:

```rust

    #[test]
    fn parses_every_documented_command() {
        assert_eq!(parse("app").unwrap(), Command::App(None));
```

with:

```rust

    #[test]
    fn parses_marks_with_an_optional_action() {
        assert_eq!(parse("marks").unwrap(), Command::Marks(None));
        assert_eq!(
            parse("marks revisit").unwrap(),
            Command::Marks(Some("revisit".into()))
        );
    }

    #[test]
    fn parses_every_documented_command() {
        assert_eq!(parse("app").unwrap(), Command::App(None));
```

In `crates/illumio-tui/src/tui/mod.rs`, replace:

```rust
pub mod impact;
pub mod mark_form;
pub mod model;
pub mod msg;
```

with:

```rust
pub mod impact;
pub mod mark_form;
pub mod marks_view;
pub mod model;
pub mod msg;
```

In `crates/illumio-tui/src/tui/update/marks_tests.rs`, replace:

```rust
        Some("marks.toml is locked by another illumio-tui (marks.toml.lock): try again")
    );
}
```

with:

```rust
        Some("marks.toml is locked by another illumio-tui (marks.toml.lock): try again")
    );
}

fn view(m: &Model) -> &crate::tui::marks_view::MarksView {
    m.marks_view.as_ref().expect("the marks overlay's state")
}

fn command(m: &mut Model, text: &str) {
    quiet(m, key(':'));
    typed(m, text);
    quiet(m, kc(KeyCode::Enter));
}

#[test]
fn colon_marks_lists_every_mark_with_its_hits_and_filters_by_action() {
    let (mut m, _dir) = marked_model(RICH_MARKS);
    command(&mut m, "marks");
    assert_eq!(m.overlay, Overlay::Marks);
    let v = view(&m);
    assert_eq!(v.title(), "Marks · 6 active · 0 expired · marks.toml");
    let rows: Vec<(String, String)> = v
        .table
        .rows()
        .iter()
        .map(|r| (r.mark.id.clone(), r.hits.clone()))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("m-sq1".to_string(), "5 · 1 to stronger".to_string()),
            ("m-sq2".into(), "6 · 1 to stronger".into()),
            ("m-crit".into(), "2".into()),
            ("m-rev".into(), "2".into()),
            ("m-acc".into(), "2".into()),
            ("m-hr".into(), "4".into()),
        ]
    );
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::None);
    command(&mut m, "marks revisit");
    assert_eq!(view(&m).table.rows().len(), 1);
    assert_eq!(view(&m).meta(), "revisit only · :marks shows all");
    quiet(&mut m, kc(KeyCode::Esc));
    command(&mut m, "marks bogus");
    assert_eq!(
        m.status.text,
        "unknown action: critical important revisit accept-blocked reviewed squelch"
    );
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn enter_edits_a_mark_in_place_and_returns_to_the_overlay() {
    let (mut m, dir) = marked_model(RICH_MARKS);
    command(&mut m, "marks");
    quiet(&mut m, key('j'));
    quiet(&mut m, kc(KeyCode::Enter));
    assert_eq!(form(&m).form.title, "Edit mark m-sq2");
    assert_eq!(form(&m).form.value("Comment"), "app tier to its db");
    quiet(&mut m, ctrl('u'));
    typed(&mut m, "app tier, confirmed");
    quiet(&mut m, kc(KeyCode::Enter));
    assert_eq!(m.overlay, Overlay::Marks, "back to the list");
    let text = std::fs::read_to_string(dir.path().join("marks.toml")).unwrap();
    assert!(text.contains("id = \"m-sq2\""), "{text}");
    assert!(text.contains("comment = \"app tier, confirmed\""), "{text}");
    assert_eq!(view(&m).table.rows()[1].mark.comment, "app tier, confirmed");
}

#[test]
fn x_and_y_delete_a_mark_and_n_keeps_it() {
    let (mut m, dir) = marked_model(RICH_MARKS);
    command(&mut m, "marks");
    quiet(&mut m, key('x'));
    assert!(
        matches!(&m.overlay, Overlay::Confirm { reason, .. } if reason == "delete m-sq1 (squelch, 5 flows)? y/n"),
        "{:?}",
        m.overlay
    );
    quiet(&mut m, key('n'));
    assert_eq!(m.overlay, Overlay::Marks);
    assert_eq!(view(&m).table.rows().len(), 6);
    quiet(&mut m, key('x'));
    quiet(&mut m, key('y'));
    assert_eq!(m.overlay, Overlay::Marks);
    assert_eq!(m.status.text, "mark m-sq1 deleted");
    assert_eq!(view(&m).table.rows().len(), 5);
    let text = std::fs::read_to_string(dir.path().join("marks.toml")).unwrap();
    assert!(!text.contains("m-sq1"), "{text}");
    assert_eq!(
        m.traffic.marked.as_ref().unwrap().flows.flows,
        5,
        "Corp's flows are back in the views"
    );
}

#[test]
fn e_extends_by_thirty_days_asking_why_again_and_never_marks_stay() {
    let soon = (jiff::Timestamp::now() + jiff::SignedDuration::from_hours(48)).to_string();
    let text = format!(
        "[[mark]]\nid = \"m-soon01\"\naction = \"squelch\"\nsrc = {{ ip_list = \"Corp\" }}\ndst = \"any\"\nexpires = \"{soon}\"\ncomment = \"scanner\"\n\n[[mark]]\nid = \"m-never1\"\naction = \"critical\"\nsrc = \"any\"\ndst = \"any\"\ncomment = \"all of it\"\n"
    );
    let (mut m, _dir) = marked_model(&text);
    command(&mut m, "marks");
    let crit = view(&m)
        .table
        .rows()
        .iter()
        .position(|r| r.mark.id == "m-never1")
        .unwrap();
    for _ in 0..crit {
        quiet(&mut m, key('j'));
    }
    quiet(&mut m, key('e'));
    assert_eq!(m.status.text, "m-never1 never expires: nothing to extend");
    quiet(&mut m, key('k'));
    let soon_row = view(&m).selected().unwrap().mark.clone();
    assert_eq!(soon_row.id, "m-soon01");
    quiet(&mut m, key('e'));
    assert_eq!(form(&m).form.fields.len(), 1, "only why");
    assert_eq!(form(&m).comment(), "scanner");
    quiet(&mut m, ctrl('u'));
    typed(&mut m, "scanner still runs");
    quiet(&mut m, kc(KeyCode::Enter));
    assert_eq!(m.overlay, Overlay::Marks);
    assert_eq!(m.status.text, "mark m-soon01 extended by 30 days");
    let after = m.traffic.marks.set().unwrap().marks[0].clone();
    assert_eq!(
        after.expires,
        soon_row
            .expires
            .map(|e| e + jiff::SignedDuration::from_hours(24 * 30))
    );
    assert_eq!(after.comment, "scanner still runs");
}

#[test]
fn an_invalid_file_shows_its_problems_where_m_and_marks_open() {
    use crate::tui::test_support::render_to_string;
    let (mut m, _dir) = marked_model("[[mark]]\naction = \"squelch\"\nsrc = \"any\"\n");
    quiet(&mut m, key('m'));
    assert_eq!(m.overlay, Overlay::Marks, "the problems, not the form");
    let out = render_to_string(&m, 100, 24);
    assert!(out.contains("Marks · marks.toml invalid"), "{out}");
    assert!(out.contains("mark 1: dst is required"), "{out}");
}
```

Create `crates/illumio-tui/tests/snapshots/tui_snapshots__marks_overlay_120x30.snap`:

```text
---
source: crates/illumio-tui/tests/tui_snapshots.rs
expression: "render_to_string(&m, 120, 30)"
---
 illumio-tui pce.example · org 5 ●                                                                api 5 calls · 20.8 KB
  1 Traffic  │  2 Policy  │  3 Objects  │  4 Workloads  │  5 Events
╭ Marks · 6 active · 0 expired · marks.toml ─────────────────────────────────────────────────── :marks revisit filters ╮
│    action          match                               scope     expires    hits               comment               │
│▸ ~ squelch         ipl:Corp → any                      any PCE   never       5 · 1 to stronger scanner               │
│  ~ squelch         loc=eu role=web → role=db 5432/tcp  any PCE   never       6 · 1 to stronger app tier to its db    │
│  ● critical        any → app=shop env=prod role=db blo any PCE   never                       2 payments db           │
│  ✓ reviewed        app=erp env=prod → app=erp env=prod any PCE   never                       2 erp inside itself     │
│  ≈ accept-blocked  app=erp env=dev → any blocked|poten any PCE   never                       2 dev stays out (CHG-1) │
│  ◆ revisit         app=hr → any                        any PCE   never                       4 ask hr                │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
 Enter edit · x delete · e +30d · / search · Esc back
ready                                                 f filter  r run  a auto  v view  g group  / search  : cmd  ? help
```

In `crates/illumio-tui/tests/tui_snapshots.rs`, replace:

```rust
        render_to_string(&m, 100, 30)
    );
}
```

with:

```rust
        render_to_string(&m, 100, 30)
    );
}

#[test]
fn marks_overlay_120x30() {
    use illumio_tui::tui::commands::Command;
    use illumio_tui::tui::test_support::{RICH_MARKS, model_with_marks};
    use illumio_tui::tui::{Msg, update};
    let dir = tempfile::tempdir().unwrap();
    let mut m = model_with_marks(dir.path(), RICH_MARKS);
    update(&mut m, Msg::Command(Command::Marks(None)));
    insta::assert_snapshot!("marks_overlay_120x30", render_to_string(&m, 120, 30));
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p illumio-tui --lib -- marks_tests commands`
Expected: FAIL: it does not compile (`file not found for module marks_view`, `no variant Marks in Overlay`).

- [ ] **Step 3: Implement**

In `crates/illumio-tui/src/tui/commands.rs`, replace:

```rust
    /// `:batch`: the last sent batch, step by step.
    Batch,
    Quit,
    Help,
```

with:

```rust
    /// `:batch`: the last sent batch, step by step.
    Batch,
    /// `:marks [action]`: the marks overlay, filtered to one action.
    Marks(Option<String>),
    Quit,
    Help,
```

In `crates/illumio-tui/src/tui/commands.rs`, replace:

```rust
        "active" => no_arg(Command::Active),
        "batch" => no_arg(Command::Batch),
        "q" | "quit" => no_arg(Command::Quit),
        "help" | "h" | "?" => no_arg(Command::Help),
```

with:

```rust
        "active" => no_arg(Command::Active),
        "batch" => no_arg(Command::Batch),
        "marks" => Ok(Command::Marks((!arg.is_empty()).then(|| arg.to_string()))),
        "q" | "quit" => no_arg(Command::Quit),
        "help" | "h" | "?" => no_arg(Command::Help),
```

Create `crates/illumio-tui/src/tui/marks_view.rs`:

```rust
//! The `:marks` overlay's state: every mark of `marks.toml` as a table
//! row (active ones first, in file order, then the out-of-scope and the
//! expired ones, greyed), with its hits in the loaded result; or the
//! problems of an invalid file. Pure: built from what is loaded.
use crate::tui::model::TrafficState;
use crate::tui::table::{Column, SortKey, TableView};
use jiff::Timestamp;
use pce::marks::{Action, Mark, Marker};

/// One mark on the overlay.
#[derive(Debug, Clone)]
pub struct MarkRow {
    pub mark: Mark,
    pub active: bool,
    /// `this PCE`, `any PCE`, `other PCE`.
    pub scope: String,
    /// `never`, `12d left`, or the date an expired mark ended.
    pub expires: String,
    /// `412`, `40 · 3 to stronger`, `–` for an inactive mark.
    pub hits: String,
    pub hit_count: u64,
}

pub fn mark_columns() -> Vec<Column<MarkRow>> {
    let col = |title, width, cell: fn(&MarkRow) -> String, key, right| Column {
        title,
        width,
        cell,
        key,
        right,
        hide_below: 0,
        narrow: None,
        narrow_width: 0,
        full: None,
        full_title: None,
    };
    vec![
        col(
            "action",
            15,
            |r| r.mark.action.as_str().to_string(),
            |r| SortKey::Num(r.mark.action.index() as u64),
            false,
        ),
        col(
            "match",
            0,
            |r| r.mark.match_text(),
            |r| SortKey::Text(r.mark.match_text()),
            false,
        ),
        col(
            "scope",
            9,
            |r| r.scope.clone(),
            |r| SortKey::Text(r.scope.clone()),
            false,
        ),
        col(
            "expires",
            10,
            |r| r.expires.clone(),
            |r| {
                SortKey::Num(
                    r.mark
                        .expires
                        .map_or(u64::MAX, |t| u64::try_from(t.as_second()).unwrap_or(0)),
                )
            },
            false,
        ),
        col(
            "hits",
            18,
            |r| r.hits.clone(),
            |r| SortKey::Num(r.hit_count),
            true,
        ),
        col(
            "comment",
            22,
            |r| {
                if r.mark.comment.trim().is_empty() {
                    "(no comment)".into()
                } else {
                    r.mark.comment.clone()
                }
            },
            |r| SortKey::Text(r.mark.comment.clone()),
            false,
        ),
    ]
}

/// The gutter: an active mark's own glyph.
fn row_marker(r: &MarkRow) -> Option<Marker> {
    r.active.then(|| Marker::of(r.mark.action))
}

#[derive(Debug)]
pub struct MarksView {
    pub table: TableView<MarkRow>,
    /// `:marks <action>`: that action only.
    pub filter: Option<Action>,
    /// An invalid or unreadable file: its problems instead of rows.
    pub problems: Option<Vec<String>>,
    /// The file's path.
    pub path: String,
    pub active: usize,
    pub expired: usize,
}

/// `12d left`; under a day `<1d left`.
fn left(e: Timestamp, now: Timestamp) -> String {
    let days = (e.as_second() - now.as_second()) / 86_400;
    if days < 1 {
        "<1d left".into()
    } else {
        format!("{days}d left")
    }
}

impl MarksView {
    pub fn new(filter: Option<Action>) -> Self {
        let mut table = TableView::new(mark_columns());
        table.marker = Some(row_marker);
        Self {
            table,
            filter,
            problems: None,
            path: String::new(),
            active: 0,
            expired: 0,
        }
    }

    /// The rows again from the marks file and the loaded result; the
    /// cursor, sort and search stay. Classifies the loaded flows once per
    /// active mark: local, no request.
    pub fn rebuild(&mut self, t: &TrafficState, now: Timestamp) {
        self.path = t.marks.path().display().to_string();
        let Some(set) = t.marks.set() else {
            self.problems = Some(t.marks.error().map(|e| e.lines()).unwrap_or_default());
            self.table.set_rows(Vec::new());
            return;
        };
        self.problems = None;
        let active = set.active(Some(&t.marks_pce), now, t.draft_loaded());
        let flows = t.loaded_flows();
        let mut rows: Vec<MarkRow> = set
            .marks
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let on = active.is_active(i);
                let (hits, n) = if on {
                    let mut n = 0u64;
                    let mut stronger = 0u64;
                    for f in flows.iter().filter(|f| m.matches(f)) {
                        n += 1;
                        if active.classify(f).is_some_and(|h| h.mark != i) {
                            stronger += 1;
                        }
                    }
                    let text = if stronger > 0 {
                        format!("{n} · {stronger} to stronger")
                    } else {
                        n.to_string()
                    };
                    (text, n)
                } else {
                    ("–".to_string(), 0)
                };
                MarkRow {
                    active: on,
                    scope: match &m.scope {
                        None => "any PCE".into(),
                        Some(s) if s.covers(&t.marks_pce) => "this PCE".into(),
                        Some(_) => "other PCE".into(),
                    },
                    expires: match m.expires {
                        None => "never".into(),
                        Some(e) if m.is_expired(now) => pce::marks::expiry_date(e),
                        Some(e) => left(e, now),
                    },
                    hits,
                    hit_count: n,
                    mark: m.clone(),
                }
            })
            .filter(|r| self.filter.is_none_or(|a| r.mark.action == a))
            .collect();
        self.active = rows.iter().filter(|r| r.active).count();
        self.expired = rows.iter().filter(|r| r.mark.is_expired(now)).count();
        // Active first, in file order; then the rest, in file order.
        rows.sort_by_key(|r| !r.active);
        self.table.set_rows(rows);
    }

    /// `Marks · 14 active · 3 expired · marks.toml`.
    pub fn title(&self) -> String {
        let file = std::path::Path::new(&self.path)
            .file_name()
            .map_or_else(|| "marks.toml".into(), |n| n.to_string_lossy().to_string());
        match &self.problems {
            Some(_) => format!("Marks · {file} invalid"),
            None => format!(
                "Marks · {} active · {} expired · {file}",
                self.active, self.expired
            ),
        }
    }

    /// `:marks revisit filters`, or the filter in force.
    pub fn meta(&self) -> String {
        match self.filter {
            Some(a) => format!("{} only · :marks shows all", a.as_str()),
            None => ":marks revisit filters".into(),
        }
    }

    pub fn selected(&self) -> Option<&MarkRow> {
        self.table.selected_item()
    }
}
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
    /// `m`: a mark for the selected row; saved to `marks.toml`, never sent.
    MarkForm(Box<crate::tui::mark_form::MarkForm>),
    /// The exact request a write would send; `y` sends it, `n`/`Esc` cancel.
    /// `scroll` is the first body line shown.
```

with:

```rust
    /// `m`: a mark for the selected row; saved to `marks.toml`, never sent.
    MarkForm(Box<crate::tui::mark_form::MarkForm>),
    /// `:marks`: every mark; its state is `Model::marks_view`.
    Marks,
    /// The exact request a write would send; `y` sends it, `n`/`Esc` cancel.
    /// `scroll` is the first body line shown.
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
            | (Overlay::Pending, Overlay::Pending)
            | (Overlay::App, Overlay::App)
            | (Overlay::Generator, Overlay::Generator) => true,
            (Overlay::Prompt { input: a }, Overlay::Prompt { input: b })
            | (Overlay::Error(a), Overlay::Error(b)) => a == b,
```

with:

```rust
            | (Overlay::Pending, Overlay::Pending)
            | (Overlay::App, Overlay::App)
            | (Overlay::Generator, Overlay::Generator)
            | (Overlay::Marks, Overlay::Marks) => true,
            (Overlay::Prompt { input: a }, Overlay::Prompt { input: b })
            | (Overlay::Error(a), Overlay::Error(b)) => a == b,
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
    /// focused query for its app; `y` and `n` go back to the generator.
    GenRequery(TrafficFilter, pce::traffic::Focus),
}

```

with:

```rust
    /// focused query for its app; `y` and `n` go back to the generator.
    GenRequery(TrafficFilter, pce::traffic::Focus),
    /// `x` on `:marks`: this mark leaves `marks.toml`; `y` and `n` go back
    /// to the overlay. A local file only.
    DeleteMark(String),
}

```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
    /// The last generator's sequence number.
    pub gen_seq: u64,
    pub connection: Connection,
    pub stats: StatsSnapshot,
```

with:

```rust
    /// The last generator's sequence number.
    pub gen_seq: u64,
    /// The `:marks` overlay's state while it is open.
    pub marks_view: Option<crate::tui::marks_view::MarksView>,
    pub connection: Connection,
    pub stats: StatsSnapshot,
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
            gen_from_app: false,
            gen_seq: 0,
            connection: Connection::Connecting,
            stats: StatsSnapshot { calls: 0, bytes: 0 },
```

with:

```rust
            gen_from_app: false,
            gen_seq: 0,
            marks_view: None,
            connection: Connection::Connecting,
            stats: StatsSnapshot { calls: 0, bytes: 0 },
```

In `crates/illumio-tui/src/tui/ui/mod.rs`, replace:

```rust
        Overlay::Form(form) => widgets::overlay::render_form(model, form, f, area),
        Overlay::MarkForm(mf) => widgets::marks::render_mark_form(model, mf, f, area),
        Overlay::ConfirmWrite { plan, kind, scroll } => {
            widgets::overlay::render_confirm_write(model, plan, *kind, *scroll, f, area)
```

with:

```rust
        Overlay::Form(form) => widgets::overlay::render_form(model, form, f, area),
        Overlay::MarkForm(mf) => widgets::marks::render_mark_form(model, mf, f, area),
        Overlay::Marks => widgets::marks::render_marks(model, f, area),
        Overlay::ConfirmWrite { plan, kind, scroll } => {
            widgets::overlay::render_confirm_write(model, plan, *kind, *scroll, f, area)
```

In `crates/illumio-tui/src/tui/ui/widgets/marks.rs`, replace:

```rust
    f.render_widget(Paragraph::new(lines).block(block).style(t.base()), r);
}
```

with:

```rust
    f.render_widget(Paragraph::new(lines).block(block).style(t.base()), r);
}

/// The `:marks` overlay over the screen body: the marks table (inactive
/// rows greyed) and the keys, or the problems of an invalid file.
pub fn render_marks(model: &Model, f: &mut Frame, area: Rect) {
    let Some(v) = &model.marks_view else {
        return;
    };
    let t = &model.theme;
    let body = crate::tui::ui::layout::chrome(area).map_or(area, |c| c.body);
    f.render_widget(Clear, body);
    let title = v.title();
    let meta = v.meta();
    if let Some(problems) = &v.problems {
        let block = panel(&title, "Esc back", t, true).style(t.base());
        let mut lines = vec![Line::from(Span::styled(
            "marks.toml is invalid: marks are off until it is fixed by hand",
            Style::new().fg(t.warn),
        ))];
        lines.extend(
            problems
                .iter()
                .map(|p| Line::from(Span::styled(p.clone(), Style::new().fg(t.bad)))),
        );
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(v.path.clone(), t.dim_style())));
        f.render_widget(Paragraph::new(lines).block(block), body);
        return;
    }
    let rows = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Min(3),
        ratatui::layout::Constraint::Length(1),
    ])
    .split(body);
    let block = panel(&title, &meta, t, true).style(t.base());
    let dim = t.dim_style();
    let base = t.base();
    crate::tui::ui::widgets::table::render(f, rows[0], &v.table, t, block, &move |r| {
        if r.active { base } else { dim }
    });
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            " Enter edit · x delete · e +30d · / search · Esc back",
            dim,
        ))),
        rows[1],
    );
}
```

In `crates/illumio-tui/src/tui/update.rs`, replace:

```rust
                    PendingAction::ReloadApp => app_reload(model),
                    PendingAction::GenRequery(f, focus) => generator::gen_requery(model, f, focus),
                }
            }
```

with:

```rust
                    PendingAction::ReloadApp => app_reload(model),
                    PendingAction::GenRequery(f, focus) => generator::gen_requery(model, f, focus),
                    PendingAction::DeleteMark(id) => marks::delete_mark(model, &id),
                }
            }
```

In `crates/illumio-tui/src/tui/update.rs`, replace:

```rust
                if matches!(action, PendingAction::GenRequery(..)) {
                    model.overlay = Overlay::Generator;
                }
                Vec::new()
```

with:

```rust
                if matches!(action, PendingAction::GenRequery(..)) {
                    model.overlay = Overlay::Generator;
                }
                if matches!(action, PendingAction::DeleteMark(_)) {
                    model.overlay = Overlay::Marks;
                }
                Vec::new()
```

In `crates/illumio-tui/src/tui/update.rs`, replace:

```rust
        }
        Overlay::MarkForm(mf) => marks::form_key(model, *mf, k),
        Overlay::Form(mut form)
            if k.modifiers.contains(KeyModifiers::CONTROL)
```

with:

```rust
        }
        Overlay::MarkForm(mf) => marks::form_key(model, *mf, k),
        Overlay::Marks => marks::overlay_key(model, k),
        Overlay::Form(mut form)
            if k.modifiers.contains(KeyModifiers::CONTROL)
```

In `crates/illumio-tui/src/tui/update.rs`, replace:

```rust
            Vec::new()
        }
        Command::Filter(expr) => {
            let mut f = model.traffic.filter.clone();
```

with:

```rust
            Vec::new()
        }
        Command::Marks(action) => marks::open_marks(model, action.as_deref()),
        Command::Filter(expr) => {
            let mut f = model.traffic.filter.clone();
```

In `crates/illumio-tui/src/tui/update/marks.rs`, replace:

```rust
pub(super) fn mark_key(model: &mut Model) -> Vec<Cmd> {
    let t = &model.traffic;
    if t.marks.error().is_some() {
        model.status = StatusLine::warn("marks.toml invalid: :marks shows the problems");
        return Vec::new();
    }
    if t.data.is_none() {
```

with:

```rust
pub(super) fn mark_key(model: &mut Model) -> Vec<Cmd> {
    let t = &model.traffic;
    // An invalid file: its problems instead of the form.
    if t.marks.error().is_some() {
        return open_marks(model, None);
    }
    if t.data.is_none() {
```

In `crates/illumio-tui/src/tui/update/marks.rs`, replace:

```rust
    }
    if model.traffic.marks.error().is_some() {
        model.status = StatusLine::warn("marks.toml invalid: :marks shows the problems");
        return Vec::new();
    }
    let Some((dir, p)) = v.selected_peer() else {
```

with:

```rust
    }
    if model.traffic.marks.error().is_some() {
        return open_marks(model, None);
    }
    let Some((dir, p)) = v.selected_peer() else {
```

In `crates/illumio-tui/src/tui/update/marks.rs`, replace:

```rust
}

/// Back where the form came from.
fn close(model: &mut Model, origin: MarkOrigin) {
    model.overlay = match origin {
        MarkOrigin::App if model.app_view.is_some() => Overlay::App,
        _ => Overlay::None,
    };
}

```

with:

```rust
}

/// Back where the form came from: the `:marks` overlay rebuilt.
fn close(model: &mut Model, origin: MarkOrigin) {
    model.overlay = match origin {
        MarkOrigin::App if model.app_view.is_some() => Overlay::App,
        MarkOrigin::Marks if model.marks_view.is_some() => {
            rebuild_view(model);
            Overlay::Marks
        }
        _ => Overlay::None,
    };
}

fn rebuild_view(model: &mut Model) {
    if let Some(v) = model.marks_view.as_mut() {
        v.rebuild(&model.traffic, jiff::Timestamp::now());
    }
}

/// `:marks [action]`: every mark (one action's), or the problems of an
/// invalid file. The file is looked at first (one `stat`). No request.
pub(super) fn open_marks(model: &mut Model, action: Option<&str>) -> Vec<Cmd> {
    let filter = match action {
        None => None,
        Some(a) => match Action::parse(a) {
            Some(a) => Some(a),
            None => {
                model.status = StatusLine::error(format!("unknown action: {}", Action::names()));
                return Vec::new();
            }
        },
    };
    if model.traffic.marks.refresh() {
        model.traffic.rebuild_views();
    }
    let mut v = crate::tui::marks_view::MarksView::new(filter);
    v.rebuild(&model.traffic, jiff::Timestamp::now());
    model.marks_view = Some(v);
    model.overlay = Overlay::Marks;
    Vec::new()
}

/// The overlay's keys: `j`/`k` (and the table's keys, `/`, `s`), `Enter`
/// edits, `x` asks to delete, `e` extends by 30 days (asking why again),
/// `Esc`/`q` back. Every change is one operation on `marks.toml`.
pub(super) fn overlay_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let page = super::traffic_page(model);
    let Some(v) = model.marks_view.as_mut() else {
        model.overlay = Overlay::None;
        return Vec::new();
    };
    if v.table.is_searching() {
        super::search_key(&mut v.table, k);
        v.table.ensure_visible(page);
        return Vec::new();
    }
    let host = model.profile.host.clone();
    match k.code {
        KeyCode::Esc | KeyCode::Char('q') => model.overlay = Overlay::None,
        KeyCode::Enter => {
            if let Some(r) = v.selected() {
                let mf = MarkForm::edit(&r.mark, &host, false);
                open_form(model, mf);
            }
        }
        KeyCode::Char('x') => {
            if let Some(r) = v.selected() {
                model.overlay = Overlay::Confirm {
                    reason: format!(
                        "delete {} ({}, {})? y/n",
                        r.mark.id,
                        r.mark.action.as_str(),
                        plural(r.hit_count as usize, "flow", "flows")
                    ),
                    action: crate::tui::model::PendingAction::DeleteMark(r.mark.id.clone()),
                };
            }
        }
        KeyCode::Char('e') => match v.selected() {
            Some(r) if r.mark.expires.is_none() => {
                model.status =
                    StatusLine::info(format!("{} never expires: nothing to extend", r.mark.id));
            }
            Some(r) => {
                let mf = MarkForm::edit(&r.mark, &host, true);
                open_form(model, mf);
            }
            None => {}
        },
        _ => {
            super::table_key(&mut v.table, k, page);
            v.table.ensure_visible(page);
        }
    }
    Vec::new()
}

/// `y` on `delete m-…?`: the mark leaves the file (lock, reload, write),
/// the views rebuild, the overlay comes back. A local file only.
pub(super) fn delete_mark(model: &mut Model, id: &str) -> Vec<Cmd> {
    let now = jiff::Timestamp::now();
    model.status = match model
        .traffic
        .marks
        .apply(MarkOp::Delete(vec![id.to_string()]), now)
    {
        Ok(_) => {
            model.traffic.rebuild_views();
            super::rebuild_app_traffic(model);
            StatusLine::info(format!("mark {id} deleted"))
        }
        Err(e) => StatusLine::error(e.to_string()),
    };
    rebuild_view(model);
    model.overlay = Overlay::Marks;
    Vec::new()
}

```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p illumio-tui --lib -- marks_tests commands && cargo test -p illumio-tui --test tui_snapshots`
Expected: PASS.

- [ ] **Step 5: Run the gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: no formatting change, no warning, every test passes.

- [ ] **Step 6: Commit**

```bash
git add crates/illumio-tui/src/tui/commands.rs crates/illumio-tui/src/tui/marks_view.rs crates/illumio-tui/src/tui/mod.rs crates/illumio-tui/src/tui/model.rs crates/illumio-tui/src/tui/ui/mod.rs crates/illumio-tui/src/tui/ui/widgets/marks.rs crates/illumio-tui/src/tui/update.rs crates/illumio-tui/src/tui/update/marks.rs crates/illumio-tui/src/tui/update/marks_tests.rs crates/illumio-tui/tests/snapshots/tui_snapshots__marks_overlay_120x30.snap crates/illumio-tui/tests/tui_snapshots.rs
git commit -m "feat(tui): :marks lists, edits, deletes and extends marks

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

### Task 4: Help, README, site, tracking, final gate

**Files:**
- Modify: `README.md`
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs`
- Modify: `crates/illumio-tui/src/tui/update.rs`
- Test (modify): `crates/illumio-tui/tests/snapshots/tui_snapshots__help_overlay_80x24.snap`
- Test (modify): `crates/illumio-tui/tests/snapshots/tui_snapshots__help_overlay_end_80x24.snap`
- Test (modify): `crates/illumio-tui/tests/snapshots/tui_snapshots__help_overlay_end_light_styled_80x24.snap`
- Modify: `docs/index.html`

**Interfaces:**
- Consumes: Tasks 1-3, Plan 2's help line.
- Produces: the help lines `Traffic  m` and `:marks [action]` around `Traffic  M`, `:marks` on the `:profile` line, `m` on the app overlay's `App      m w r Esc` line (77 lines); the README's TUI paragraph, CLI examples and `### Flow marks`; the site's paragraph and prompt list.

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/src/tui/update.rs`, replace:

```rust
        use crate::tui::ui::widgets::overlay::{HELP_LINES, help_last_scroll};
        let mut m = model_fixture(); // 80x24
        assert_eq!(HELP_LINES.len(), 75);
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Traffic  M"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Events   J"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Traffic  A"));
```

with:

```rust
        use crate::tui::ui::widgets::overlay::{HELP_LINES, help_last_scroll};
        let mut m = model_fixture(); // 80x24
        assert_eq!(HELP_LINES.len(), 77);
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Traffic  M"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Traffic  m"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "App      m w r Esc"));
        assert!(
            HELP_LINES
                .iter()
                .any(|(_, d)| d.starts_with(":marks [action]"))
        );
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Events   J"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Traffic  A"));
```

In `crates/illumio-tui/src/tui/update.rs`, replace:

```rust
        update(&mut m, key('?'));
        let last = help_last_scroll(24);
        assert_eq!(last, 53, "75 lines, 22 rows inside the box");
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "App-to-app x / X"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "App-to-app o"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "App-to-app d"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Sections Space / z"));

        for _ in 0..53 {
            update(&mut m, key('j'));
        }
```

with:

```rust
        update(&mut m, key('?'));
        let last = help_last_scroll(24);
        assert_eq!(last, 55, "77 lines, 22 rows inside the box");
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "App-to-app x / X"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "App-to-app o"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "App-to-app d"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Sections Space / z"));

        for _ in 0..55 {
            update(&mut m, key('j'));
        }
```

In `crates/illumio-tui/tests/snapshots/tui_snapshots__help_overlay_80x24.snap`, replace:

```text
expression: out
---
╭ Keys ────────────────────────────────── 1-22 of 75 · j/k scroll · Esc closes ╮
│1 2 3 4 5                Traffic · Policy · Objects · Workloads · Events      │
│:                        command prompt: :traffic :policy :objects :workloads │
│                         :events  :iplists  :filter <expr>  :window <dur>     │
│                         :profile <name>  :draft  :active  :help  :q          │
│                         :unicast  unicast flows only (again: every kind)     │
│                         :focus <app>  (alone: a picker of the loaded apps)   │
```

with:

```text
expression: out
---
╭ Keys ────────────────────────────────── 1-22 of 77 · j/k scroll · Esc closes ╮
│1 2 3 4 5                Traffic · Policy · Objects · Workloads · Events      │
│:                        command prompt: :traffic :policy :objects :workloads │
│                         :events  :iplists  :filter <expr>  :window <dur>     │
│                         :profile <name>  :draft  :active  :marks  :help  :q  │
│                         :unicast  unicast flows only (again: every kind)     │
│                         :focus <app>  (alone: a picker of the loaded apps)   │
```

In `crates/illumio-tui/tests/snapshots/tui_snapshots__help_overlay_end_80x24.snap`, replace:

```text
expression: out
---
╭ Keys ───────────────────────────────── 54-75 of 75 · j/k scroll · Esc closes ╮
│w form   Ctrl-A Ctrl-R   all labels · create the typed ruleset                │
│Policy   d               toggle draft / active                                │
```

with:

```text
expression: out
---
╭ Keys ───────────────────────────────── 56-77 of 77 · j/k scroll · Esc closes ╮
│w form   Ctrl-A Ctrl-R   all labels · create the typed ruleset                │
│Policy   d               toggle draft / active                                │
```

In `crates/illumio-tui/tests/snapshots/tui_snapshots__help_overlay_end_light_styled_80x24.snap`, replace:

```text
expression: "render_styled(&m, 80, 24)"
---
╭ Keys ───────────────────────────────── 54-75 of 75 · j/k scroll · Esc closes ╮
    @0:Rgb(0, 102, 204)/Rgb(250, 250, 247) @2:Rgb(150, 50, 180)/Rgb(250, 250, 247) @6:Rgb(0, 102, 204)/Rgb(250, 250, 247) @41:Rgb(120, 124, 136)/Rgb(250, 250, 247) @78:Rgb(0, 102, 204)/Rgb(250, 250, 247)
│w form   Ctrl-A Ctrl-R   all labels · create the typed ruleset                │
```

with:

```text
expression: "render_styled(&m, 80, 24)"
---
╭ Keys ───────────────────────────────── 56-77 of 77 · j/k scroll · Esc closes ╮
    @0:Rgb(0, 102, 204)/Rgb(250, 250, 247) @2:Rgb(150, 50, 180)/Rgb(250, 250, 247) @6:Rgb(0, 102, 204)/Rgb(250, 250, 247) @41:Rgb(120, 124, 136)/Rgb(250, 250, 247) @78:Rgb(0, 102, 204)/Rgb(250, 250, 247)
│w form   Ctrl-A Ctrl-R   all labels · create the typed ruleset                │
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p illumio-tui help && cargo test -p illumio-tui --test tui_snapshots help_overlay`
Expected: FAIL: the help has 75 lines.

- [ ] **Step 3: Implement**

In `README.md`, replace:

```markdown
  would ask first (no query of this session behind the result, a large window or cap); if
  draft decisions were dropped, the status then says `draft cleared: D re-evaluates`.
- **Policy**: `d` toggle draft / active, `p` pending changes (`Enter` on one shows its diff against active), `Tab` switch panes, `Enter` rule
  detail, or on a ruleset load its deny rules. Rules are listed in evaluation order: override
```

with:

```markdown
  would ask first (no query of this session behind the result, a large window or cap); if
  draft decisions were dropped, the status then says `draft cleared: D re-evaluates`.
  `m` **marks** the row under the cursor (Flows, App-to-app pairs and sections, Mesh, Blocked,
  the app overlay's Inbound and Outbound): `squelch` hides known noise, `reviewed` and
  `accept-blocked` dim what you have judged, `critical`, `important` and `revisit` flag what
  matters (`●` `◆` in a gutter before the row; the strongest mark wins, so a squelch never hides
  a critical flow). The form is prefilled from the row (the labels under the keys in force,
  else `wl:`, `ipl:` or `ip:`; `Ctrl-A` every label of the endpoint, `Ctrl-T` another kind,
  `Ctrl-U` clears a field), defaults to this PCE and an expiry by action (squelch and
  accept-blocked 90 days, reviewed 30, revisit 7, critical and important never), previews what
  it matches in the loaded result, and needs a comment: why. `Enter` saves to `marks.toml`
  next to `config.toml`. Marks change nothing on the PCE and send no request. Squelched flows
  leave every view (a row goes when all its flows do; `1 ~1` beside a count says one is
  hidden) and the panel says `4 rows (9 flows) squelched · M shows them`; `M` shows them,
  dimmed `~`. Blocked counts the flows accepted as blocked apart (`10 blocked flows · 2
  accepted`). `:marks` lists every mark with its hits (`Enter` edit, `x` delete, `e` 30 more
  days); expired marks stay in the file, greyed. The generator's rules ignore marks; its
  header counts the squelched flows.
- **Policy**: `d` toggle draft / active, `p` pending changes (`Enter` on one shows its diff against active), `Tab` switch panes, `Enter` rule
  detail, or on a ruleset load its deny rules. Rules are listed in evaluation order: override
```

In `README.md`, replace:

```markdown
    illumio-tui traffic groups --window 1h --by in --show blocked --app pos/PCI
    illumio-tui traffic groups --window 1h --app shop/prod --direction in
    illumio-tui app shop/prod --window 24h --json
    illumio-tui app shop/prod --direction in --csv --section inbound > inbound.csv
```

with:

```markdown
    illumio-tui traffic groups --window 1h --by in --show blocked --app pos/PCI
    illumio-tui traffic groups --window 1h --app shop/prod --direction in
    illumio-tui marks add --action squelch --src app=monitoring --dst any --service 5666/tcp --comment "Nagios NRPE, known"
    illumio-tui marks list --table
    illumio-tui traffic groups --window 1h --by in --show-squelched
    illumio-tui app shop/prod --window 24h --json
    illumio-tui app shop/prod --direction in --csv --section inbound > inbound.csv
```

In `README.md`, replace:

```markdown
`v6` address counts as strings, `unbounded` (the included `0.0.0.0/0` or `::/0`, else `null`),
`ranges` and `fqdns`.

## Writes
```

with:

```markdown
`v6` address counts as strings, `unbounded` (the included `0.0.0.0/0` or `::/0`, else `null`),
`ranges` and `fqdns`.

### Flow marks

`illumio-tui marks list|add|rm|export|import` reads and writes `marks.toml` (next to
`config.toml`, mode 0600, rewritten under a lock from a fresh read; comments are not kept, and
the first rewrite of a commented file keeps it in `marks.toml.bak`). It sends no request and
needs no profile, except `add --pce` without a host. `add` needs `--comment` (why) and prints
the new id; `export` prints `{"marks": [...]}` (or `--format toml`), which `import` reads.
Traffic commands and `app` apply the marks: squelched flows are left out (`--show-squelched`
keeps them, `--no-marks` ignores the file), rows carry `mark` (`{"action", "id"}`; aggregated
rows also `flows`, the per-action `marks` and the `squelched` records they hide), tables a
last `mark` column, and `meta.squelched` counts the rows and flows left out. `gen` proposes the
same rules either way and reports `meta.squelched`. An invalid file is a warning (the command
runs without marks); `add`, `rm` and `import` refuse to write it (exit 2), a held lock exits 3.

## Writes
```

In `crates/illumio-tui/src/tui/ui/widgets/overlay.rs`, replace:

```rust
    (":", "command prompt: :traffic :policy :objects :workloads"),
    ("", ":events  :iplists  :filter <expr>  :window <dur>"),
    ("", ":profile <name>  :draft  :active  :help  :q"),
    ("", ":unicast  unicast flows only (again: every kind)"),
    ("", ":focus <app>  (alone: a picker of the loaded apps)"),
```

with:

```rust
    (":", "command prompt: :traffic :policy :objects :workloads"),
    ("", ":events  :iplists  :filter <expr>  :window <dur>"),
    ("", ":profile <name>  :draft  :active  :marks  :help  :q"),
    ("", ":unicast  unicast flows only (again: every kind)"),
    ("", ":focus <app>  (alone: a picker of the loaded apps)"),
```

In `crates/illumio-tui/src/tui/ui/widgets/overlay.rs`, replace:

```rust
    ),
    (
        "App      w r Esc",
        "draft allow rule · reload panels (asks) · back",
    ),
    (
```

with:

```rust
    ),
    (
        "App      m w r Esc",
        "mark · draft allow rule · reload panels (asks) · back",
    ),
    (
```

In `crates/illumio-tui/src/tui/ui/widgets/overlay.rs`, replace:

```rust
        "draft allow rule from a blocked flow or row (writes)",
    ),
    ("Traffic  M", "show or hide squelched rows (~)"),
    (
        "Form     Ctrl-N Ctrl-P",
```

with:

```rust
        "draft allow rule from a blocked flow or row (writes)",
    ),
    (
        "Traffic  m",
        "mark the row: squelch · reviewed · critical … (marks.toml)",
    ),
    ("Traffic  M", "show or hide squelched rows (~)"),
    (
        "",
        ":marks [action]  the marks: Enter edit · x delete · e +30d",
    ),
    (
        "Form     Ctrl-N Ctrl-P",
```

In `docs/index.html`, replace:

```html
        <li><kbd>5</kbd><strong>Events.</strong> Severity-coloured audit events, filtered by severity, type and window; <code>Enter</code> shows what changed as a diff: only the changed fields, <code>-</code> before and <code>+</code> after, lists diffed by element, hrefs read as names; <code>J</code> toggles the raw JSON; in the detail <code>j</code>/<code>k</code> (or the arrows) step through the events and <code>Ctrl-d</code>/<code>Ctrl-u</code> scroll.</li>
      </ul>
      <p>The <code>:</code> prompt takes <code>:traffic</code>, <code>:policy</code>, <code>:objects</code>, <code>:workloads</code>, <code>:events</code>, <code>:iplists</code>, <code>:filter</code>, <code>:window</code>, <code>:profile &lt;name&gt;</code>, <code>:focus</code>, <code>:app</code>, <code>:gen</code>, <code>:unicast</code>, <code>:export &lt;path&gt;</code>, <code>:writes on|off</code>, <code>:batch</code>, <code>:draft</code>, <code>:active</code>, <code>:help</code> and <code>:q</code>. <code>?</code> shows the full key map.</p>
    </div>
    <div>
```

with:

```html
        <li><kbd>5</kbd><strong>Events.</strong> Severity-coloured audit events, filtered by severity, type and window; <code>Enter</code> shows what changed as a diff: only the changed fields, <code>-</code> before and <code>+</code> after, lists diffed by element, hrefs read as names; <code>J</code> toggles the raw JSON; in the detail <code>j</code>/<code>k</code> (or the arrows) step through the events and <code>Ctrl-d</code>/<code>Ctrl-u</code> scroll.</li>
      </ul>
      <p>The <code>:</code> prompt takes <code>:traffic</code>, <code>:policy</code>, <code>:objects</code>, <code>:workloads</code>, <code>:events</code>, <code>:iplists</code>, <code>:filter</code>, <code>:window</code>, <code>:profile &lt;name&gt;</code>, <code>:focus</code>, <code>:app</code>, <code>:gen</code>, <code>:unicast</code>, <code>:marks</code>, <code>:export &lt;path&gt;</code>, <code>:writes on|off</code>, <code>:batch</code>, <code>:draft</code>, <code>:active</code>, <code>:help</code> and <code>:q</code>. <code>?</code> shows the full key map.</p>
    </div>
    <div>
```

In `docs/index.html`, replace:

```html
      <p><code>A</code> opens one screen for that app: inbound and outbound peers, its workloads with enforcement modes (workloads without a VEN counted as unmanaged) and VEN health, and the rulesets that cover it. It reuses what is loaded; opening it sends at most two requests, and every request is counted in its header.</p>
      <p><code>G</code> there (or <code>:gen</code>) opens the <strong>policy generator</strong>: the allow rules the app&rsquo;s own traffic needs, one row per peer with a checkbox, a slider from <em>app, all services</em> to <em>role pairs, the ports seen</em>, rules already in the draft marked <em>exists</em>, and a preview of the flows covered. IP peers that sit in an existing IP list are grouped into one row per list, so the rule names the list rather than a new list per address. A port many peers reach (DNS, NTP, a monitoring agent, a shared service) gets a fan-in line: <code>c</code> turns it into one rule from the built-in Any IP list (checked workload peers keep their own rules, which carry their outbound) or from All Workloads, with its catch and what that rule also allows spelled out. <code>Enter</code> plans every write (IP lists, a ruleset, rule updates and creates) into one confirm; <code>y</code> sends them in order and stops at the first failure, and <code>:batch</code> shows the steps again. On the CLI, <code>illumio-tui gen</code> prints the same proposal and writes nothing. IP list rows and fan-in consolidation are on <code>main</code> since v0.2.0 and ship with the next release.</p>
    </div>
  </div>
```

with:

```html
      <p><code>A</code> opens one screen for that app: inbound and outbound peers, its workloads with enforcement modes (workloads without a VEN counted as unmanaged) and VEN health, and the rulesets that cover it. It reuses what is loaded; opening it sends at most two requests, and every request is counted in its header.</p>
      <p><code>G</code> there (or <code>:gen</code>) opens the <strong>policy generator</strong>: the allow rules the app&rsquo;s own traffic needs, one row per peer with a checkbox, a slider from <em>app, all services</em> to <em>role pairs, the ports seen</em>, rules already in the draft marked <em>exists</em>, and a preview of the flows covered. IP peers that sit in an existing IP list are grouped into one row per list, so the rule names the list rather than a new list per address. A port many peers reach (DNS, NTP, a monitoring agent, a shared service) gets a fan-in line: <code>c</code> turns it into one rule from the built-in Any IP list (checked workload peers keep their own rules, which carry their outbound) or from All Workloads, with its catch and what that rule also allows spelled out. <code>Enter</code> plans every write (IP lists, a ruleset, rule updates and creates) into one confirm; <code>y</code> sends them in order and stops at the first failure, and <code>:batch</code> shows the steps again. On the CLI, <code>illumio-tui gen</code> prints the same proposal and writes nothing. IP list rows and fan-in consolidation are on <code>main</code> since v0.2.0 and ship with the next release.</p>
      <p><code>m</code> <strong>marks</strong> the row under the cursor: <em>squelch</em> hides known noise, <em>reviewed</em> and <em>accept-blocked</em> dim what you have judged, <em>critical</em>, <em>important</em> and <em>revisit</em> flag what matters. The form is prefilled from the row, previews what it matches, and asks why. Marks live in <code>marks.toml</code> next to the config, apply to every view and the CLI&rsquo;s traffic commands, and change nothing on the PCE. <code>M</code> shows the squelched rows, <code>:marks</code> lists and edits the marks.</p>
    </div>
  </div>
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p illumio-tui help && cargo test -p illumio-tui --test tui_snapshots help_overlay`
Expected: PASS.

- [ ] **Step 5: Run the gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: no formatting change, no warning, every test passes.

- [ ] **Step 6: Track the plan and review**

Append to `tasks/todo.md`:

```markdown
## Flow marks 3: create and manage (plan docs/superpowers/plans/2026-10-17-flow-marks-3-create-manage.md)
- [x] Task 1 field kinds, completion, MarkForm
- [x] Task 2 m: form, preview, save
- [x] Task 3 :marks overlay
- [x] Task 4 help, README, site, tracking, gate
```

Then append `## Review (Flow marks 3)`: what shipped, the test count, the snapshots, "no marks key sends a request; nothing was written to any PCE".

- [ ] **Step 7: Commit**

```bash
git add README.md crates/illumio-tui/src/tui/ui/widgets/overlay.rs crates/illumio-tui/src/tui/update.rs crates/illumio-tui/tests/snapshots/tui_snapshots__help_overlay_80x24.snap crates/illumio-tui/tests/snapshots/tui_snapshots__help_overlay_end_80x24.snap crates/illumio-tui/tests/snapshots/tui_snapshots__help_overlay_end_light_styled_80x24.snap docs/index.html tasks/todo.md
git commit -m "docs: flow marks in the help, README and site

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

## Self-review notes

- Spec coverage: the mark form (overlay, fields, `FieldKind`s, prefill per view and per row kind, widening with `Ctrl-U`/`Ctrl-A`/`Ctrl-T`, the live preview, validation, the required comment, the save and its status), `EditIntent::Mark` never reaching `build_intent`, `m` on the treemap and the Policy/Workloads panels, `:marks [action]` (rows, hits, keys, filter, the invalid file), the help lines, README and the site: Tasks 1-4.
- Verification: every step of this plan was applied in order to a clean checkout of main (after the plans before it); each task built clean under `cargo clippy --workspace --all-targets -- -D warnings` with its own tests passing, and the gate passed at the end.
