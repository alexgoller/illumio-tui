# Policy Generator (1 of 3): `pce::generate` and `illumio-tui gen` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A pure `pce::generate` module that turns one app's observed flows into rows (one per peer and side, with default checkboxes and flags), the rows and a granularity stop into merged allow rules, and classifies each rule against the draft rulesets (create, add services, exists), with a coverage preview; plus a read-only `illumio-tui gen <app>` that prints exactly that, and the profile setting `[profiles.<name>.generator] ruleset_name`.

**Architecture:** `pce::generate` has five files: `mod.rs` (the data model of the spec's "Data model" section), `services.rs` (a flow's service as a rule names it: inline tcp/udp, the ICMP object, a single-protocol object; port folding), `target.rs` (which ruleset a scope's rules go into; the `w` form's `ruleset_pick` logic moves here so it exists once; the ruleset name template), `rows.rs` (flows to rows, defaults, flags, IP folding), `dedupe.rs` (an existing rule covers a proposed one, or lacks only services) and `derive.rs` (`propose`: rows and stop to rules, merging, descriptions, coverage). Nothing in it makes a request. The CLI fetches the focused traffic, the name index and the draft rulesets through the existing cached paths (`cli_focus`, `fetch`, `name_index`, `rule_sets`) and prints one envelope.

**Tech Stack:** Rust 2024 (rust-version 1.88), serde/serde_json, clap 4, wiremock 0.6, assert_cmd. No new dependency.

**Spec:** `docs/superpowers/specs/2026-10-16-policy-generator-design.md` (binding), delivery item 1, including "Decisions (Alex, 2026-10-16)". Plan 2 (`2026-10-16-policy-generator-2-batch.md`) builds the write batch on the `Proposal` this plan produces; Plan 3 (`2026-10-16-policy-generator-3-tui.md`) the TUI on both.

**Tasks:** 8, in this order: 1 prerequisites (ordered `PortSpec`, the Any list's data set, All Services, step placeholders); 2 the data model, services and ruleset targets (`edit::ruleset_pick` calls them); 3 rows; 4 dedupe; 5 derivation (`propose`); 6 the profile setting; 7 `illumio-tui gen`; 8 README, tracking, read-only smoke, final gate.

## Global Constraints

- The gate is `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`. Run `cargo fmt --all` first and accept its formatting.
- No new dependencies. `git diff main -- '*Cargo.toml' Cargo.lock` must be empty.
- Never hammer the PCE: every request is caused by the user, throttled, cached where it is a read, counted and visible. Nothing refreshes on its own.
- Writes are gated (profile `writes = true`, audit log writable; the TUI after `:writes on`) and always confirmed; nothing is ever auto-confirmed. CLI `--yes` semantics stay as they are. This plan writes nothing: `illumio-tui gen` is read-only.
- Tests use recorded fixtures (`fixtures/`, `fixtures/recorded/`) and mocks (wiremock) only. Live smokes on the demo PCE are read-only: never `--yes`, never `y`, writes off; they run with `--env-file ../illumio-mcp-server/.env`. Never read or print that file.
- Verified on demo100 (2026-10-16): IP list consumers in a scoped ruleset appear with `unscoped_consumers: false`; cross-app inbound rules use `unscoped_consumers: true`. The derivation tables rely on both.
- New rulesets are named by `[profiles.<name>.generator] ruleset_name`, default `"RF-{app}-{env}"` (`RF-shop-prod`). New IP lists are named `<app>-<env> peer <ip>` (`shop-prod peer 10.4.1.9`), one per IP peer.
- `gen` is a reserved keyword in Rust 2024: modules are `generate` (`pce::generate`, `cli::generate`), never `gen`.
- Every commit message ends with these two lines:
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx

## Spec deviations and resolutions (read before Task 1)

- **Dedupe compares actor sets, not single actors.** A rule's labels AND across keys and OR within one key, so "each proposed actor is matched by an existing actor" would call a narrower rule (`app=hr, env=prod, role=api`) a cover of `app=hr, env=prod`: a false `exists`, a dropped rule. Ruled: an existing side covers a proposed one when it holds every workload the proposed one names: `ams` (on the in-scope side), the same IP list or workload, or a label constraint each of whose keys the proposed labels satisfy (a label group through `group_members`). Unresolvable labels, exclusions, disabled or pending-delete rules, rules narrowed to users (`consuming_security_principals`), machine authentication, a `network_type` other than `brn`/`all`, or `resolve_labels_as` without `workloads` never cover.
- **`propose` takes a `GenInput`.** Besides the spec's arguments it needs the ruleset name template, the window (descriptions), the outbound switch and the ruleset `R` picked: `propose(rows, global, &GenInput { flows, focus, tier_key, drafts, idx, ruleset_name, window, outbound, picked })`.
- **`pick_ruleset(scope, has, drafts, idx)` takes the endpoint's labels too.** The `w` form covers a destination through any of its labels (a scope `app, env, loc=eu` covers a destination with `loc=eu`); keeping that needs `has`. The generator passes its app's scope labels for both, so a ruleset scoped by a label the app's workloads may lack never covers the app.
- **Default ruleset name.** `RF-` and every scope key: `RF-{app}-{env}` for the default keys (the spec's default); with other scope keys the default follows them (`RF-{site}`), so it never fails the load check.
- **Coverage counts the sections in force:** inbound, and outbound when it is on. A protocol nothing expresses counts as `unsupported_proto` whether its row is checked or not.
- **New flag `RowFlag::NoAddress`:** an IP peer without an address cannot be named in an IP list; its row cannot be checked.
- **An IP list named as the batch would name it** (`shop-prod peer 10.4.1.9`) that already exists is used like any existing list (`UsesIpList`).
- **CLI meta.** The envelope keeps its `meta` (`api_calls`, `cache`, `queries`, `truncated`, `incomplete`); the spec's `meta.requests/cached/unit` are those fields (`meta.queries` is 2 with `--outbound`). The units go in the table's unit line.
- **CLI refusals.** A truncated draft ruleset read exits 2 too (dedupe cannot be proven). An incomplete traffic result exits 4 through a new `CliError::Incomplete` (kind `incomplete`). `--granularity app-all|role-all` without an All Services object exits 2.
- **`body` of an `add_services` rule** in the JSON is `{"add_services": [...]}`: the real `PUT` body needs the rule's fresh GET at plan time (Plan 2).
- **Descriptions** are cut at 255 characters (`DESCRIPTION_MAX`), the service list with `+N`, each side's names with `+N` past 60.
- **README.** The CLI command and the setting are documented here (they ship with this plan); the TUI and `docs/index.html` follow in Plan 3.

## Review Focus

- **A narrower existing rule** (an extra `role` label on the consumers) must never count as covering a proposed rule. Pinned in Task 4 (`a_narrower_existing_rule_is_not_a_cover`).
- **IP list consumers in a scoped ruleset** are proposed with `unscoped_consumers: false`, as the recorded demo PCE holds them. Pinned in Task 4 (`recorded_scoped_rulesets_name_ip_list_consumers_in_scope`).
- **Capped or region-incomplete traffic** prints no rules from the CLI (exit 2 or 4, empty stdout). Pinned in Task 7 (`gen_refuses_capped_or_incomplete_traffic_and_unknown_apps`).
- **An ICMP flow on a PCE without an ICMP service** is never silently "covered": it counts as `unsupported_proto` and its row cannot be checked. Pinned in Task 5 (`coverage_counts_every_kept_flow_once_by_reason`) and Task 7 (`gen_prints_rows_rules_and_the_preview_and_writes_nothing`).
- **A ruleset name template naming a non-scope key** (`RF-{app}-{role}`) fails the config load and names the key. Pinned in Task 6 (`generator_ruleset_name_defaults_to_rf_and_names_a_bad_key`).

---

### Task 1: pce prerequisites: ordered `PortSpec`, the Any list's data set, All Services, step placeholders

**Files:**
- Create: `crates/pce/src/write/batch.rs`
- Modify: `crates/pce/src/write/specs.rs`
- Modify: `crates/pce/src/resolve.rs`
- Modify: `crates/pce/src/models/labels.rs`
- Modify: `crates/pce/src/write/mod.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces:

```rust
// pce::write::specs
impl Ord for PortSpec;            // by (proto, port, to_port); also Hash
// pce::models::IpList
pub external_data_set: Option<String>,   // serde default, skipped when None
// pce::resolve::NameIndex
pub fn all_services(&self) -> Option<&Service>;
pub fn all_services_href(&self) -> Option<&str>;
// pce::write::batch
pub fn step_placeholder(step: usize) -> String;     // "{step:3}"
pub fn parse_placeholder(s: &str) -> Option<usize>; // "{step:3}" -> 3
```

- [ ] **Step 1: Write the failing tests**

At the end of the test module of `crates/pce/src/write/specs.rs`, replace:

```rust
        assert_eq!(
            i.to_json(),
            json!({"name": "eth0", "address": "10.0.0.5", "cidr_block": 24, "friendly_name": "lan"})
        );
    }
}
```

with:

```rust
        assert_eq!(
            i.to_json(),
            json!({"name": "eth0", "address": "10.0.0.5", "cidr_block": 24, "friendly_name": "lan"})
        );
    }

    #[test]
    fn port_specs_sort_by_protocol_then_port() {
        let mut v: Vec<PortSpec> = ["53/udp", "443/tcp", "icmp", "22/tcp", "8000-8010/tcp"]
            .iter()
            .map(|s| PortSpec::parse(s).unwrap())
            .collect();
        v.sort();
        let shown: Vec<String> = v.iter().map(PortSpec::display).collect();
        assert_eq!(shown, vec!["icmp", "22/tcp", "443/tcp", "8000-8010/tcp", "53/udp"]);
    }
}
```

In the test module of `crates/pce/src/resolve.rs`, before `resolves_labels_both_ways`, replace:

```rust
    #[test]
    fn resolves_labels_both_ways() {
```

with:

```rust
    #[test]
    fn all_services_is_the_built_in_one_else_the_any_protocol_else_the_name() {
        let svc = |n: u32, name: &str, v: serde_json::Value| -> Service {
            let mut s = serde_json::json!({
                "href": format!("/orgs/5/sec_policy/draft/services/{n}"), "name": name
            });
            s.as_object_mut()
                .unwrap()
                .extend(v.as_object().unwrap().clone());
            serde_json::from_value(s).unwrap()
        };
        let idx = |v: &[Service]| NameIndex::new(&[], &[], &[], v);
        // The fixture's All Services is the protocol -1 entry.
        assert_eq!(
            index().all_services_href(),
            Some("/orgs/5/sec_policy/draft/services/30")
        );
        let ilo = svc(
            1,
            "S-ALL",
            serde_json::json!({"service_ports": [{"proto": 6}],
                "external_data_set": "com.illumio.ilo_defaults",
                "external_data_reference": "All Services"}),
        );
        let any = svc(2, "Everything", serde_json::json!({"service_ports": [{"proto": -1}]}));
        let named = svc(3, "all services", serde_json::json!({"service_ports": []}));
        assert_eq!(
            idx(&[any.clone(), named.clone(), ilo]).all_services().unwrap().name,
            "S-ALL"
        );
        assert_eq!(
            idx(&[named.clone(), any]).all_services().unwrap().name,
            "Everything"
        );
        assert_eq!(idx(&[named]).all_services().unwrap().name, "all services");
        assert_eq!(idx(&[]).all_services_href(), None);
    }

    #[test]
    fn resolves_labels_both_ways() {
```

At the end of the test module of `crates/pce/src/models/labels.rs`, replace:

```rust
        assert_eq!(services[2].summary(), "80/tcp, 443/tcp, 8000-8099/tcp");
    }
}
```

with:

```rust
        assert_eq!(services[2].summary(), "80/tcp, 443/tcp, 8000-8099/tcp");
    }

    #[test]
    fn ip_lists_keep_their_external_data_set() {
        let l: IpList = serde_json::from_value(serde_json::json!({
            "href": "/orgs/5/sec_policy/draft/ip_lists/1", "name": "Any (0.0.0.0/0 and ::/0)",
            "external_data_set": "com.illumio.ilo_defaults"
        }))
        .unwrap();
        assert_eq!(
            l.external_data_set.as_deref(),
            Some("com.illumio.ilo_defaults")
        );
        let back = serde_json::to_value(&l).unwrap();
        assert_eq!(back["external_data_set"], "com.illumio.ilo_defaults");
        let lists: Vec<IpList> =
            serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json")).unwrap();
        assert!(
            serde_json::to_value(&lists[0])
                .unwrap()
                .get("external_data_set")
                .is_none(),
            "absent stays absent"
        );
    }
}
```

Create `crates/pce/src/write/batch.rs` with its test only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_round_trip_and_never_pass_as_hrefs() {
        assert_eq!(step_placeholder(3), "{step:3}");
        assert_eq!(parse_placeholder("{step:3}"), Some(3));
        assert_eq!(parse_placeholder("{step:x}"), None);
        assert_eq!(
            parse_placeholder("/orgs/5/sec_policy/draft/rule_sets/1"),
            None
        );
        assert!(super::super::checked_policy_href(&step_placeholder(1), "rule_sets", 5).is_err());
        assert!(super::super::checked_href(&step_placeholder(0), "ip_lists", 5).is_err());
    }
}
```

In `crates/pce/src/write/mod.rs`, declare it:

```rust
//! through [`crate::Pce::writer`] with a [`WriteGuard`].
pub mod actors;
pub mod audit;
pub mod guard;
pub mod ip_lists;
pub mod labels;
```

with:

```rust
//! through [`crate::Pce::writer`] with a [`WriteGuard`].
pub mod actors;
pub mod audit;
pub mod batch;
pub mod guard;
pub mod ip_lists;
pub mod labels;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p pce --lib`
Expected: FAIL to compile: "no method named `all_services_href`", "no field `external_data_set`", "cannot find function `step_placeholder`", "the trait bound `PortSpec: Ord` is not satisfied".

- [ ] **Step 3: Write the implementation**

In `crates/pce/src/write/specs.rs`, replace the `PortSpec` definition (the order sorts a row's services by protocol, then port, and lets port folding walk them in order):

```rust
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortSpec {
    pub port: Option<u16>,
    pub to_port: Option<u16>,
    pub proto: i64,
}

impl PortSpec {
    /// `P/PROTO`, `P-Q/PROTO`, or a bare protocol by name (`icmp`, `udp`).
    /// A bare number is refused: `22` reads as a port but would mean "all
```

with:

```rust
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PortSpec {
    pub port: Option<u16>,
    pub to_port: Option<u16>,
    pub proto: i64,
}

/// By protocol, then port, then range end: `443/tcp` before `53/udp`.
impl Ord for PortSpec {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.proto, self.port, self.to_port).cmp(&(other.proto, other.port, other.to_port))
    }
}

impl PartialOrd for PortSpec {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PortSpec {
    /// `P/PROTO`, `P-Q/PROTO`, or a bare protocol by name (`icmp`, `udp`).
    /// A bare number is refused: `22` reads as a port but would mean "all
```

In `crates/pce/src/models/labels.rs`, add the field to `IpList` (no struct literal builds an `IpList`; every construction deserializes):

```rust
    pub fqdns: Vec<Fqdn>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_type: Option<String>,
}

impl IpList {
```

with:

```rust
    pub fqdns: Vec<Fqdn>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_type: Option<String>,
    /// `com.illumio.ilo_defaults` for the PCE's built-in Any list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_data_set: Option<String>,
}

impl IpList {
```

In `crates/pce/src/resolve.rs`, after `icmp_service_href`, add:

```rust
        self.icmp_service().map(|s| s.href.as_str())
    }

    /// The one live service named exactly `name` (like the CLI's lookup:
    /// case-sensitive, pending deletions excluded). `None` when there is no
    /// such service, or several.
```

with:

```rust
        self.icmp_service().map(|s| s.href.as_str())
    }

    /// The All Services object a rule uses for "every service": the
    /// built-in one (`ilo_defaults`/`All Services`, whatever its name),
    /// else the first service whose only entry is protocol `-1` without a
    /// port, else one named `All Services`.
    pub fn all_services(&self) -> Option<&Service> {
        let all = self.services();
        let by = |f: &dyn Fn(&Service) -> bool| all.iter().copied().find(|s| f(s));
        by(&|s| {
            s.external_data_set.as_deref() == Some("com.illumio.ilo_defaults")
                && s.external_data_reference.as_deref() == Some("All Services")
        })
        .or_else(|| {
            by(&|s| {
                matches!(s.service_ports.as_slice(), [p] if p.proto == Some(-1) && p.port.is_none())
            })
        })
        .or_else(|| by(&|s| s.name.eq_ignore_ascii_case("All Services")))
    }

    pub fn all_services_href(&self) -> Option<&str> {
        self.all_services().map(|s| s.href.as_str())
    }

    /// The one live service named exactly `name` (like the CLI's lookup:
    /// case-sensitive, pending deletions excluded). `None` when there is no
    /// such service, or several.
```

Above the test in `crates/pce/src/write/batch.rs`, add (Plan 2 adds the batch itself to this file):

```rust
//! A batch of writes whose later steps name what earlier ones create. A
//! step that needs such an href carries the placeholder `{step:N}`; the
//! executor (a later plan) puts the href in just before sending. A
//! placeholder never passes `checked_href`, so it cannot reach the wire.

/// `{step:3}`: the href step 3 creates (or finds).
pub fn step_placeholder(step: usize) -> String {
    format!("{{step:{step}}}")
}

/// The step a whole placeholder names: `{step:3}` → 3.
pub fn parse_placeholder(s: &str) -> Option<usize> {
    s.strip_prefix("{step:")?.strip_suffix('}')?.parse().ok()
}

```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p pce --lib -- port_specs_sort all_services ip_lists_keep placeholders_round_trip`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/pce/src/write/specs.rs crates/pce/src/models/labels.rs crates/pce/src/resolve.rs crates/pce/src/write/batch.rs crates/pce/src/write/mod.rs
git commit -m "pce: ordered PortSpec, IP list data set, All Services lookup, step placeholders

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

### Task 2: `pce::generate`: the data model, services and ruleset targets

**Files:**
- Create: `crates/pce/src/generate/services.rs`
- Create: `crates/pce/src/generate/target.rs`
- Create: `crates/pce/src/generate/mod.rs`
- Modify: `crates/pce/src/lib.rs`
- Modify: `crates/illumio-tui/src/tui/edit.rs`

**Interfaces:**
- Consumes: Task 1 (`PortSpec: Ord`, `NameIndex::all_services_href`, `step_placeholder`); `pce::impact::group_members`; `RuleDraft::to_body`.
- Produces (`pce::generate`, re-exported names as listed):

```rust
pub enum Stop { AppAll, AppPorts /* default */, RolePorts, RoleAll }
impl Stop { pub const ALL; fn index; fn prev; fn next; fn is_role; fn is_all; fn ports; fn text /* "app·all" */; fn cli /* "app-all" */ }
pub enum Side { Inbound, Outbound }                      // serde "inbound"/"outbound"
pub enum PeerKind { Intra, App(GroupKey), Ip { ip: String, list: Option<String> },
                    Workload { href: String, name: String }, FoldedIps(usize) }
pub enum PeerKey { Intra, App(GroupKey), Ip(String), Workload(String), Folded }
pub struct RowKey { pub side: Side, pub peer: PeerKey }
pub enum RowFlag { EnvMismatch, UsesIpList(String), NeedsIpList, NoScopeLabels,
                   NoRole { flows: u64 }, UnsupportedProto(i64), ManyServices(usize), Ephemeral, NoAddress }
impl RowFlag { pub fn text(&self, side: Side, scope_keys: &[String]) -> String }
pub struct DecisionCounts { pub allowed: u64, pub blocked: u64, pub draft_allowed: Option<u64> }
impl DecisionCounts { pub fn add(&mut self, f: &ResolvedFlow) }
pub struct GenRow { pub side, pub peer: PeerKind, pub checked: bool, pub checkable: bool,
                    pub stop: Option<Stop>, pub flags: Vec<RowFlag>, pub flows: u64,
                    pub connections: u64, pub decisions: DecisionCounts, pub services: BTreeSet<PortSpec> }
impl GenRow { fn key(&self) -> RowKey; fn effective(&self, global: Stop) -> Stop; fn proposed(&self) -> bool;
              fn peer_text(&self) -> String; fn kind_text(&self) -> &'static str;
              fn flag_texts(&self, stop: Stop, scope_keys: &[String]) -> Vec<String> }
pub enum GenActor { Ref(ActorRef), NewIpList(usize) }
impl GenActor { fn all(); fn label(href); fn sort_key(&self) -> String;
                fn to_ref(&self, p: &Proposal) -> ActorRef; fn text(&self, p: &Proposal, idx: &NameIndex) -> String }
pub struct TargetScope { pub values: GroupKey, pub labels: Vec<ResolvedLabel> }   // fn text() -> "shop/prod"
pub enum Target { Existing { href, name }, Create(RuleSetDraft), Ambiguous(Vec<Covering>), None(String) }
impl Target { fn action(&self) -> &'static str; fn name(&self) -> String }
pub enum RuleAction { Create, AddServices { rule_href: String, add: Vec<ServiceRef> }, Exists { rule_href: String } }
pub struct GenRule { pub target: usize, pub providers: Vec<GenActor>, pub consumers: Vec<GenActor>,
                     pub unscoped_consumers: bool, pub services: Vec<ServiceRef>, pub rows: Vec<usize>,
                     pub action: RuleAction, pub description: String }
impl GenRule { fn ruleset_ref(&self, p) -> Option<String>; fn to_draft(&self, p) -> RuleDraft; fn body(&self, p) -> Value }
pub struct Coverage { covered, uncovered, unchecked, folded_ips, unsupported_proto, broadcast_multicast: u64 }
pub struct RuleCounts { rules, create, add_services, exists: usize }
pub struct Proposal { pub scopes: Vec<TargetScope>, pub targets: Vec<Target>, pub ip_lists: Vec<IpListDraft>,
                      pub rules: Vec<GenRule>, pub coverage: Coverage }
impl Proposal { fn counts; fn writes; fn ip_list_step(i) -> usize; fn ruleset_step(t) -> Option<usize>; fn blocker() -> Option<String> }
// services.rs
pub fn flow_spec(f: &ResolvedFlow) -> PortSpec;
pub enum Expr { Inline(PortSpec), Object(String), Unsupported(i64) }
pub fn express(spec: &PortSpec, idx: &NameIndex) -> Expr;
pub fn fold_ports(ports: &BTreeSet<PortSpec>) -> Vec<PortSpec>;
// target.rs
pub const DEFAULT_RULESET_NAME: &str = "RF-{app}-{env}";
pub fn default_ruleset_name(scope_keys: &[String]) -> String;
pub fn check_ruleset_name(template: &str, scope_keys: &[String]) -> Result<(), String>;
pub fn render_ruleset_name(template: &str, keys: &[String], values: &[String]) -> String;
pub fn keys_refusal(keys: &[String], scope_keys: &[String]) -> Option<String>;
pub fn rulesets_covering<'r>(sets: &'r [RuleSet], has: &BTreeSet<&str>, want: &BTreeSet<&str>, idx: &NameIndex) -> Vec<(&'r RuleSet, bool)>;
pub struct Covering { pub href: String, pub name: String, pub exact: bool }
pub struct RulesetChoice { pub covering: Vec<Covering>, pub picked: Option<usize> }
pub fn pick_ruleset(scope: &[&str], has: &[&str], sets: &[RuleSet], idx: &NameIndex) -> RulesetChoice;
pub fn resolve(scope: &TargetScope, drafts: &[RuleSet], idx: &NameIndex, template: &str, picked: Option<&str>) -> Target;
```

- [ ] **Step 1: Write the failing tests**

Create `crates/pce/src/generate/services.rs` with its tests only (the code follows in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{IpList, Label, LabelGroup, Service};
    use crate::traffic::resolved::tests::rich_flows;

    fn spec(s: &str) -> PortSpec {
        PortSpec::parse(s).unwrap()
    }

    fn index_with(services: &[Service]) -> NameIndex {
        let labels: Vec<Label> = Vec::new();
        let groups: Vec<LabelGroup> = Vec::new();
        let lists: Vec<IpList> = Vec::new();
        NameIndex::new(&labels, &groups, &lists, services)
    }

    fn service(href: &str, name: &str, ports: serde_json::Value) -> Service {
        serde_json::from_value(serde_json::json!({
            "href": href, "name": name, "service_ports": ports
        }))
        .unwrap()
    }

    #[test]
    fn a_flow_keeps_its_port_only_for_tcp_and_udp() {
        let f = rich_flows();
        assert_eq!(flow_spec(&f[0]), spec("5432/tcp"));
        assert_eq!(flow_spec(&f[27]), spec("53/udp"));
        assert_eq!(flow_spec(&f[25]), spec("icmp"), "flow 25 is icmp");
    }

    #[test]
    fn icmp_takes_the_icmp_object_and_other_protocols_the_single_matching_one() {
        let none = index_with(&[]);
        assert_eq!(
            express(&spec("443/tcp"), &none),
            Expr::Inline(spec("443/tcp"))
        );
        assert_eq!(express(&spec("icmp"), &none), Expr::Unsupported(1));
        let gre = PortSpec {
            port: None,
            to_port: None,
            proto: 47,
        };
        assert_eq!(express(&gre, &none), Expr::Unsupported(47));
        let one = index_with(&[
            service(
                "/orgs/5/sec_policy/draft/services/7",
                "ICMP",
                serde_json::json!([{"proto": 1}]),
            ),
            service(
                "/orgs/5/sec_policy/draft/services/8",
                "GRE",
                serde_json::json!([{"proto": 47}]),
            ),
        ]);
        assert_eq!(
            express(&spec("icmp"), &one),
            Expr::Object("/orgs/5/sec_policy/draft/services/7".into())
        );
        assert_eq!(
            express(&gre, &one),
            Expr::Object("/orgs/5/sec_policy/draft/services/8".into())
        );
        let two = index_with(&[
            service(
                "/orgs/5/sec_policy/draft/services/8",
                "GRE",
                serde_json::json!([{"proto": 47}]),
            ),
            service(
                "/orgs/5/sec_policy/draft/services/9",
                "GRE2",
                serde_json::json!([{"proto": 47}]),
            ),
        ]);
        assert_eq!(
            express(&gre, &two),
            Expr::Unsupported(47),
            "two match: none is taken"
        );
        let unknown = PortSpec {
            port: None,
            to_port: None,
            proto: -1,
        };
        assert_eq!(express(&unknown, &one), Expr::Unsupported(-1));
    }

    #[test]
    fn only_consecutive_seen_ports_fold_into_a_range() {
        let ports: BTreeSet<PortSpec> = [
            "8000/tcp", "8001/tcp", "8002/tcp", "8004/tcp", "8003/udp", "53/udp",
        ]
        .into_iter()
        .map(spec)
        .collect();
        let folded: Vec<String> = fold_ports(&ports).iter().map(PortSpec::display).collect();
        assert_eq!(
            folded,
            vec!["8000-8002/tcp", "8004/tcp", "53/udp", "8003/udp"]
        );
    }
}
```

Create `crates/pce/src/generate/target.rs` with its tests only (the code follows in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::ResolvedLabel;
    use crate::traffic::resolved::tests::rich_index;

    fn keys() -> Vec<String> {
        vec!["app".into(), "env".into()]
    }

    fn rule_sets() -> Vec<RuleSet> {
        serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap()
    }

    fn label(href: &str, key: &str, value: &str) -> ResolvedLabel {
        ResolvedLabel {
            href: href.into(),
            key: key.into(),
            value: value.into(),
        }
    }

    pub(crate) fn shop_scope() -> TargetScope {
        TargetScope {
            values: vec!["shop".into(), "prod".into()],
            labels: vec![
                label("/orgs/5/labels/3", "app", "shop"),
                label("/orgs/5/labels/4", "env", "prod"),
            ],
        }
    }

    #[test]
    fn the_default_template_is_rf_and_the_scope_keys() {
        assert_eq!(default_ruleset_name(&keys()), DEFAULT_RULESET_NAME);
        assert_eq!(default_ruleset_name(&["site".into()]), "RF-{site}");
        assert_eq!(
            render_ruleset_name(
                DEFAULT_RULESET_NAME,
                &keys(),
                &["shop".into(), "prod".into()]
            ),
            "RF-shop-prod"
        );
    }

    #[test]
    fn a_template_names_scope_keys_only_and_closes_its_braces() {
        assert!(check_ruleset_name("RS-{app}-{env}", &keys()).is_ok());
        assert!(check_ruleset_name("fixed name", &keys()).is_ok());
        assert_eq!(
            check_ruleset_name("RF-{app}-{role}", &keys()).unwrap_err(),
            "generator.ruleset_name \"RF-{app}-{role}\": {role} is not a scope key (app, env)"
        );
        assert!(
            check_ruleset_name("RF-{app", &keys())
                .unwrap_err()
                .contains("not closed")
        );
        assert!(check_ruleset_name("  ", &keys()).is_err());
    }

    #[test]
    fn only_the_scope_keys_are_accepted() {
        assert_eq!(keys_refusal(&keys(), &keys()), None);
        assert_eq!(
            keys_refusal(&["app".into(), "env".into(), "role".into()], &keys()).as_deref(),
            Some("the generator needs g = app,env (the scope keys)")
        );
    }

    #[test]
    fn the_exactly_scoped_ruleset_is_picked_and_pending_deletes_are_not_targets() {
        let idx = rich_index();
        let sets = rule_sets();
        let t = resolve(&shop_scope(), &sets, &idx, DEFAULT_RULESET_NAME, None);
        assert_eq!(
            t,
            Target::Existing {
                href: "/orgs/5/sec_policy/draft/rule_sets/100".into(),
                name: "Shop-Prod".into()
            }
        );
        let mut gone = sets.clone();
        gone[0].update_type = Some("delete".into());
        let Target::Create(d) = resolve(&shop_scope(), &gone, &idx, DEFAULT_RULESET_NAME, None)
        else {
            panic!("a ruleset pending deletion is no target")
        };
        assert_eq!(d.name, "RF-shop-prod");
        assert!(d.enabled);
        assert_eq!(
            d.scopes,
            vec![vec![
                ActorRef::new(ActorKind::Label, "/orgs/5/labels/3"),
                ActorRef::new(ActorKind::Label, "/orgs/5/labels/4"),
            ]]
        );
    }

    #[test]
    fn several_covering_rulesets_are_ambiguous_until_one_is_picked() {
        let idx = rich_index();
        let mut sets = rule_sets();
        let mut twin = sets[0].clone();
        twin.href = "/orgs/5/sec_policy/draft/rule_sets/102".into();
        twin.name = "Shop-Prod-2".into();
        sets.push(twin);
        let Target::Ambiguous(c) = resolve(&shop_scope(), &sets, &idx, DEFAULT_RULESET_NAME, None)
        else {
            panic!("two exact rulesets")
        };
        assert_eq!(c.len(), 2);
        let t = resolve(
            &shop_scope(),
            &sets,
            &idx,
            DEFAULT_RULESET_NAME,
            Some("/orgs/5/sec_policy/draft/rule_sets/102"),
        );
        assert_eq!(
            t,
            Target::Existing {
                href: "/orgs/5/sec_policy/draft/rule_sets/102".into(),
                name: "Shop-Prod-2".into()
            }
        );
    }

    #[test]
    fn a_label_group_scope_covers_through_its_members() {
        let idx = rich_index();
        let mut sets = rule_sets();
        // app=shop and the group "All prod envs" (env=prod).
        sets[0].scopes = vec![vec![
            serde_json::from_value(serde_json::json!({"label": {"href": "/orgs/5/labels/3"}}))
                .unwrap(),
            serde_json::from_value(serde_json::json!(
                {"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/10"}}
            ))
            .unwrap(),
        ]];
        let hrefs = ["/orgs/5/labels/3", "/orgs/5/labels/4"];
        let c = pick_ruleset(&hrefs, &hrefs, &sets, &idx);
        assert_eq!(c.covering.len(), 1);
        assert!(!c.covering[0].exact);
        assert_eq!(c.picked, Some(0), "the only one covering it");
    }
}
```

Create `crates/pce/src/generate/mod.rs` with the two modules and the tests of the data model (Step 3 replaces this file):

```rust
//! The policy generator (filled in Step 3).
pub mod services;
pub mod target;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slider_stops_at_both_ends_and_names_its_stops() {
        assert_eq!(Stop::default(), Stop::AppPorts);
        assert_eq!(Stop::AppAll.prev(), Stop::AppAll);
        assert_eq!(Stop::RoleAll.next(), Stop::RoleAll);
        assert_eq!(Stop::AppPorts.next(), Stop::RolePorts);
        assert_eq!(Stop::RoleAll.ports(), Stop::RolePorts);
        assert_eq!(Stop::AppAll.text(), "app·all");
        assert_eq!(Stop::RolePorts.cli(), "role-ports");
        assert_eq!(serde_json::to_value(Stop::RoleAll).unwrap(), "role-all");
        assert!(Stop::RolePorts.is_role() && !Stop::AppAll.is_role());
        assert!(Stop::RoleAll.is_all() && !Stop::AppPorts.is_all());
    }

    fn shop_scope() -> TargetScope {
        TargetScope {
            values: vec!["shop".into(), "prod".into()],
            labels: Vec::new(),
        }
    }

    fn create(name: &str) -> Target {
        Target::Create(RuleSetDraft {
            name: name.into(),
            description: None,
            enabled: true,
            scopes: Vec::new(),
        })
    }

    fn rule(target: usize, action: RuleAction) -> GenRule {
        GenRule {
            target,
            providers: vec![GenActor::all()],
            consumers: vec![GenActor::NewIpList(0)],
            unscoped_consumers: false,
            services: Vec::new(),
            rows: vec![0],
            action,
            description: String::new(),
        }
    }

    #[test]
    fn ruleset_steps_follow_the_ip_lists_in_target_order() {
        let p = Proposal {
            scopes: vec![shop_scope(), shop_scope(), shop_scope()],
            targets: vec![
                Target::Existing {
                    href: "/orgs/5/sec_policy/draft/rule_sets/1".into(),
                    name: "A".into(),
                },
                create("RF-b"),
                create("RF-c"),
            ],
            ip_lists: vec![IpListDraft {
                name: "shop-prod peer 10.0.0.1".into(),
                description: None,
                ranges: Vec::new(),
                fqdns: Vec::new(),
            }],
            rules: vec![rule(2, RuleAction::Create)],
            coverage: Coverage::default(),
        };
        assert_eq!(p.ruleset_step(0), None, "it exists");
        assert_eq!(p.ruleset_step(1), Some(1), "after the one IP list");
        assert_eq!(p.ruleset_step(2), Some(2));
        assert_eq!(p.ip_list_step(0), 0);
        assert_eq!(
            GenActor::NewIpList(0).to_ref(&p).href.as_deref(),
            Some("{step:0}")
        );
        assert_eq!(p.rules[0].ruleset_ref(&p).as_deref(), Some("{step:2}"));
        let d = p.rules[0].to_draft(&p);
        assert!(d.enabled && !d.unscoped_consumers);
        assert_eq!(
            p.rules[0].body(&p)["consumers"],
            serde_json::json!([{"ip_list": {"href": "{step:0}"}}])
        );
        assert_eq!(
            p.counts(),
            RuleCounts {
                rules: 1,
                create: 1,
                ..RuleCounts::default()
            }
        );
    }

    #[test]
    fn a_rule_to_write_into_an_ambiguous_target_blocks_the_plan() {
        let covering = vec![
            Covering {
                href: "/orgs/5/sec_policy/draft/rule_sets/1".into(),
                name: "A".into(),
                exact: true,
            },
            Covering {
                href: "/orgs/5/sec_policy/draft/rule_sets/2".into(),
                name: "B".into(),
                exact: true,
            },
        ];
        let mut p = Proposal {
            scopes: vec![shop_scope()],
            targets: vec![Target::Ambiguous(covering)],
            ip_lists: Vec::new(),
            rules: vec![rule(0, RuleAction::Create)],
            coverage: Coverage::default(),
        };
        assert_eq!(
            p.blocker().as_deref(),
            Some("2 rulesets cover shop/prod: R picks")
        );
        p.rules[0].action = RuleAction::Exists {
            rule_href: "/orgs/5/sec_policy/draft/rule_sets/1/sec_rules/1".into(),
        };
        assert_eq!(p.blocker(), None, "nothing to write there");
        assert!(!p.writes());
    }

    #[test]
    fn row_flags_read_as_the_rows_show_them() {
        let keys = vec!["app".to_string(), "env".to_string()];
        assert_eq!(RowFlag::EnvMismatch.text(Side::Inbound, &keys), "env mismatch");
        assert_eq!(
            RowFlag::UsesIpList("Corp".into()).text(Side::Inbound, &keys),
            "uses IP list Corp"
        );
        assert_eq!(
            RowFlag::ManyServices(40).text(Side::Inbound, &keys),
            "40 services: consider app·all or role·all"
        );
        assert_eq!(RowFlag::Ephemeral.text(Side::Outbound, &keys), "ephemeral port?");
    }
}
```

In `crates/pce/src/lib.rs`, declare the module:

```rust
pub mod cache;
pub mod client;
pub mod error;
pub mod impact;
pub mod iplist_size;
pub mod key_order;
```

with:

```rust
pub mod cache;
pub mod client;
pub mod error;
pub mod generate;
pub mod impact;
pub mod iplist_size;
pub mod key_order;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p pce --lib generate`
Expected: FAIL to compile: "cannot find type `Stop` in this scope", "cannot find function `flow_spec`", "cannot find function `resolve`".

- [ ] **Step 3: Write the implementation**

Replace `crates/pce/src/generate/mod.rs` with the data model (the tests from Step 1 stay at its end, unchanged):

```rust
//! The policy generator: the allow rules one app needs, from its own
//! observed traffic. [`rows`] turns the app's flows into rows (one per
//! peer and side); [`propose`] turns the rows and the slider stop into
//! rules, merged, and classified against the draft rulesets (create, add
//! services, exists). Pure: no requests. The TUI's generator and the CLI's
//! `gen` share it.
pub mod services;
pub mod target;

pub use services::{Expr, express, flow_spec, fold_ports};
pub use target::{
    Covering, DEFAULT_RULESET_NAME, RulesetChoice, check_ruleset_name, default_ruleset_name,
    keys_refusal, pick_ruleset, render_ruleset_name, rulesets_covering,
};

use crate::models::PolicyDecision;
use crate::resolve::{NameIndex, ResolvedLabel};
use crate::traffic::{GroupKey, ResolvedFlow};
use crate::write::batch::step_placeholder;
use crate::write::{
    ActorKind, ActorRef, IpListDraft, PortSpec, RuleDraft, RuleSetDraft, ServiceRef,
};
use serde::Serialize;
use std::collections::BTreeSet;

/// How coarse the rules are: the slider's four stops, in slider order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stop {
    /// All Workloads of the app, All Services.
    AppAll,
    /// All Workloads of the app, the services seen.
    #[default]
    AppPorts,
    /// One rule per role pair, the services seen.
    RolePorts,
    /// One rule per role pair, All Services.
    RoleAll,
}

impl Stop {
    pub const ALL: [Stop; 4] = [Stop::AppAll, Stop::AppPorts, Stop::RolePorts, Stop::RoleAll];

    pub fn index(self) -> usize {
        Stop::ALL.iter().position(|s| *s == self).unwrap_or(1)
    }

    /// `←`: one stop left, stopping at the first.
    pub fn prev(self) -> Self {
        Stop::ALL[self.index().saturating_sub(1)]
    }

    /// `→`: one stop right, stopping at the last.
    pub fn next(self) -> Self {
        Stop::ALL[(self.index() + 1).min(Stop::ALL.len() - 1)]
    }

    pub fn is_role(self) -> bool {
        matches!(self, Stop::RolePorts | Stop::RoleAll)
    }

    pub fn is_all(self) -> bool {
        matches!(self, Stop::AppAll | Stop::RoleAll)
    }

    /// The same granularity with the services seen (`app·all` → `app·ports`).
    pub fn ports(self) -> Self {
        match self {
            Stop::AppAll => Stop::AppPorts,
            Stop::RoleAll => Stop::RolePorts,
            s => s,
        }
    }

    /// `app·ports`: the slider and the rows.
    pub fn text(self) -> &'static str {
        match self {
            Stop::AppAll => "app·all",
            Stop::AppPorts => "app·ports",
            Stop::RolePorts => "role·ports",
            Stop::RoleAll => "role·all",
        }
    }

    /// `app-ports`: the CLI's `--granularity` and the JSON.
    pub fn cli(self) -> &'static str {
        match self {
            Stop::AppAll => "app-all",
            Stop::AppPorts => "app-ports",
            Stop::RolePorts => "role-ports",
            Stop::RoleAll => "role-all",
        }
    }
}

/// Which way a row's flows go, seen from the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// Into the app (its own ruleset).
    Inbound,
    /// Out of the app (the peer's ruleset, or the app's own for an IP).
    Outbound,
}

/// The other end of a row's flows.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PeerKind {
    /// Source and destination both in the app.
    Intra,
    /// A workload with a label under every scope key: its values.
    App(GroupKey),
    /// An endpoint with no workload; `list` is the IP list the rule uses
    /// (an existing one other than the built-in Any list), `None` when one
    /// would be created.
    Ip { ip: String, list: Option<String> },
    /// A workload without a label under some scope key.
    Workload { href: String, name: String },
    /// IP peers past [`MAX_IP_ROWS`] in one section: never proposed.
    FoldedIps(usize),
}

/// What identifies a row across rebuilds (the checkbox and the override
/// follow it).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PeerKey {
    Intra,
    App(GroupKey),
    Ip(String),
    Workload(String),
    Folded,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RowKey {
    pub side: Side,
    pub peer: PeerKey,
}

/// Why a row is unchecked, cannot be checked, or deserves a look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowFlag {
    /// An app peer in another env (another value under a scope key other
    /// than the first).
    EnvMismatch,
    /// The IP peer is in this existing IP list: the rule names it.
    UsesIpList(String),
    /// No IP list holds the IP peer: checking the row plans one.
    NeedsIpList,
    /// A workload without every scope label.
    NoScopeLabels,
    /// At role granularity these flows fall back to the app level.
    NoRole { flows: u64 },
    /// A protocol no inline service or single service object expresses.
    UnsupportedProto(i64),
    /// More service entries than [`MANY_SERVICES`] after folding.
    ManyServices(usize),
    /// A destination port of 49152 or above.
    Ephemeral,
    /// The IP peer has no address to put into an IP list.
    NoAddress,
}

impl RowFlag {
    /// The row's `⚠` text. `scope_keys` names the env mismatch.
    pub fn text(&self, side: Side, scope_keys: &[String]) -> String {
        match self {
            RowFlag::EnvMismatch => {
                let rest = scope_keys.get(1..).unwrap_or_default().join(",");
                format!("{rest} mismatch")
            }
            RowFlag::UsesIpList(n) => format!("uses IP list {n}"),
            RowFlag::NeedsIpList => "needs an IP list".into(),
            RowFlag::NoScopeLabels if side == Side::Inbound => {
                "no scope labels: rule names the workload".into()
            }
            RowFlag::NoScopeLabels => "no scope labels: no ruleset to write into".into(),
            RowFlag::NoRole { flows } => format!("{flows} flows without role: app-level for those"),
            RowFlag::UnsupportedProto(p) => format!("proto {p} needs a service object"),
            RowFlag::ManyServices(n) => format!("{n} services: consider app·all or role·all"),
            RowFlag::Ephemeral => "ephemeral port?".into(),
            RowFlag::NoAddress => "no address: no IP list can name it".into(),
        }
    }
}

/// A row's flow records by reported decision.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct DecisionCounts {
    pub allowed: u64,
    /// Blocked and potentially blocked.
    pub blocked: u64,
    /// With draft decisions loaded: the flows the draft would allow.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft_allowed: Option<u64>,
}

impl DecisionCounts {
    pub fn add(&mut self, f: &ResolvedFlow) {
        match f.policy_decision {
            PolicyDecision::Allowed => self.allowed += 1,
            PolicyDecision::Blocked | PolicyDecision::PotentiallyBlocked => self.blocked += 1,
            PolicyDecision::Unknown => {}
        }
        if let Some(d) = &f.draft_policy_decision {
            let n = self.draft_allowed.get_or_insert(0);
            *n += u64::from(d.class() == PolicyDecision::Allowed);
        }
    }
}

/// One row of the generator: one peer on one side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenRow {
    pub side: Side,
    pub peer: PeerKind,
    pub checked: bool,
    /// `false`: `Space` cannot check it (a flag says why).
    pub checkable: bool,
    /// The row's own stop (`<` / `>`); `None` follows the slider.
    pub stop: Option<Stop>,
    pub flags: Vec<RowFlag>,
    /// Flow records.
    pub flows: u64,
    /// Their lifetime connection counts, summed (folds IPs by it).
    pub connections: u64,
    pub decisions: DecisionCounts,
    /// The services seen: tcp/udp with their port, other protocols without.
    pub services: BTreeSet<PortSpec>,
}

impl GenRow {
    pub fn key(&self) -> RowKey {
        let peer = match &self.peer {
            PeerKind::Intra => PeerKey::Intra,
            PeerKind::App(v) => PeerKey::App(v.clone()),
            PeerKind::Ip { ip, .. } => PeerKey::Ip(ip.clone()),
            PeerKind::Workload { href, .. } => PeerKey::Workload(href.clone()),
            PeerKind::FoldedIps(_) => PeerKey::Folded,
        };
        RowKey {
            side: self.side,
            peer,
        }
    }

    /// The stop in force: the row's own, else the slider's.
    pub fn effective(&self, global: Stop) -> Stop {
        self.stop.unwrap_or(global)
    }

    /// Checked and allowed to be: its flows are proposed.
    pub fn proposed(&self) -> bool {
        self.checked && self.checkable
    }

    /// `intra-app`, `web/prod`, `10.4.1.9`, the workload's name, `3 more IPs`.
    pub fn peer_text(&self) -> String {
        match &self.peer {
            PeerKind::Intra => "intra-app".into(),
            PeerKind::App(v) => v.join("/"),
            PeerKind::Ip { ip, .. } => ip.clone(),
            PeerKind::Workload { name, .. } => name.clone(),
            PeerKind::FoldedIps(n) => format!("{n} more IPs: not proposed"),
        }
    }

    /// `intra`, `app`, `ip`, `workload`, `folded_ips`: the JSON's `kind`.
    pub fn kind_text(&self) -> &'static str {
        match self.peer {
            PeerKind::Intra => "intra",
            PeerKind::App(_) => "app",
            PeerKind::Ip { .. } => "ip",
            PeerKind::Workload { .. } => "workload",
            PeerKind::FoldedIps(_) => "folded_ips",
        }
    }

    /// The flags worth showing at `stop`: the missing-role one only at a
    /// role stop.
    pub fn flag_texts(&self, stop: Stop, scope_keys: &[String]) -> Vec<String> {
        self.flags
            .iter()
            .filter(|f| stop.is_role() || !matches!(f, RowFlag::NoRole { .. }))
            .map(|f| f.text(self.side, scope_keys))
            .collect()
    }
}

/// A provider or consumer of a proposed rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenActor {
    Ref(ActorRef),
    /// The IP list `Proposal::ip_lists[i]`, created by the batch.
    NewIpList(usize),
}

impl GenActor {
    pub fn all() -> Self {
        GenActor::Ref(ActorRef::all())
    }

    pub fn label(href: &str) -> Self {
        GenActor::Ref(ActorRef::new(ActorKind::Label, href))
    }

    /// Identity for merging and comparing sets: kind and (draft) href.
    pub fn sort_key(&self) -> String {
        match self {
            GenActor::Ref(a) => actor_key(a),
            GenActor::NewIpList(i) => format!("new_ip_list:{i}"),
        }
    }

    /// The actor a request carries: a new IP list as its step placeholder.
    pub fn to_ref(&self, p: &Proposal) -> ActorRef {
        match self {
            GenActor::Ref(a) => a.clone(),
            GenActor::NewIpList(i) => {
                ActorRef::new(ActorKind::IpList, step_placeholder(p.ip_list_step(*i)))
            }
        }
    }

    /// `all workloads`, `app=web`, `group:prod`, `ipl:Corp`, `wl:<id>`,
    /// `new ipl:shop-prod peer 10.4.1.9`.
    pub fn text(&self, p: &Proposal, idx: &NameIndex) -> String {
        match self {
            GenActor::NewIpList(i) => format!(
                "new ipl:{}",
                p.ip_lists.get(*i).map_or("?", |d| d.name.as_str())
            ),
            GenActor::Ref(a) => {
                let h = a.href.as_deref().unwrap_or_default();
                match a.kind {
                    ActorKind::All => "all workloads".into(),
                    ActorKind::Label => idx.label_text(h),
                    ActorKind::LabelGroup => {
                        format!("group:{}", idx.label_group_name(h).unwrap_or(h))
                    }
                    ActorKind::IpList => format!("ipl:{}", idx.ip_list_name(h).unwrap_or(h)),
                    ActorKind::Workload => {
                        format!("wl:{}", h.rsplit('/').next().unwrap_or(h))
                    }
                }
            }
        }
    }
}

/// `ActorRef`'s identity as a string; IP list and label group hrefs in
/// their draft form, so an active href matches its draft.
pub(crate) fn actor_key(a: &ActorRef) -> String {
    let href = a
        .href
        .as_deref()
        .map(crate::resolve::canon)
        .unwrap_or_default();
    let excl = if a.exclusion { "!" } else { "" };
    format!("{excl}{}:{href}", a.kind.as_str())
}

/// The labels a target ruleset is scoped by: the app's or a peer's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetScope {
    /// `["shop", "prod"]`.
    pub values: GroupKey,
    /// One label per scope key, in key order.
    pub labels: Vec<ResolvedLabel>,
}

impl TargetScope {
    pub fn text(&self) -> String {
        self.values.join("/")
    }
}

/// The ruleset a target scope's rules go into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Existing {
        href: String,
        name: String,
    },
    /// No draft ruleset covers the scope: the batch creates this one.
    Create(RuleSetDraft),
    /// Several cover it and none is the one: `R` picks (the app's only).
    Ambiguous(Vec<Covering>),
    /// No ruleset can be named (why).
    None(String),
}

impl Target {
    /// `existing`, `create`, `ambiguous`, `none`: the JSON's `action`.
    pub fn action(&self) -> &'static str {
        match self {
            Target::Existing { .. } => "existing",
            Target::Create(_) => "create",
            Target::Ambiguous(_) => "ambiguous",
            Target::None(_) => "none",
        }
    }

    /// The ruleset's name, or what stands for it.
    pub fn name(&self) -> String {
        match self {
            Target::Existing { name, .. } => name.clone(),
            Target::Create(d) => d.name.clone(),
            Target::Ambiguous(c) => format!("{} rulesets", c.len()),
            Target::None(why) => why.clone(),
        }
    }
}

/// What the batch does with a proposed rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleAction {
    Create,
    /// An existing rule with the same actors lacks these services.
    AddServices {
        rule_href: String,
        add: Vec<ServiceRef>,
    },
    /// An existing rule already covers it: nothing is written.
    Exists {
        rule_href: String,
    },
}

impl RuleAction {
    /// `create`, `add_services`, `exists`.
    pub fn as_str(&self) -> &'static str {
        match self {
            RuleAction::Create => "create",
            RuleAction::AddServices { .. } => "add_services",
            RuleAction::Exists { .. } => "exists",
        }
    }
}

/// One proposed allow rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenRule {
    /// Index into `Proposal::targets` (and `scopes`); 0 is the app's.
    pub target: usize,
    pub providers: Vec<GenActor>,
    pub consumers: Vec<GenActor>,
    pub unscoped_consumers: bool,
    pub services: Vec<ServiceRef>,
    /// The rows it came from (indexes into the rows `propose` was given).
    pub rows: Vec<usize>,
    pub action: RuleAction,
    pub description: String,
}

impl GenRule {
    /// The ruleset href a create is sent under: an existing one, or the
    /// placeholder of the step that creates it.
    pub fn ruleset_ref(&self, p: &Proposal) -> Option<String> {
        match p.targets.get(self.target)? {
            Target::Existing { href, .. } => Some(href.clone()),
            Target::Create(_) => p.ruleset_step(self.target).map(step_placeholder),
            Target::Ambiguous(_) | Target::None(_) => None,
        }
    }

    /// The rule as the batch creates it.
    pub fn to_draft(&self, p: &Proposal) -> RuleDraft {
        RuleDraft {
            description: Some(self.description.clone()),
            enabled: true,
            providers: self.providers.iter().map(|a| a.to_ref(p)).collect(),
            consumers: self.consumers.iter().map(|a| a.to_ref(p)).collect(),
            services: self.services.clone(),
            unscoped_consumers: self.unscoped_consumers,
            resolve_labels_as: None,
        }
    }

    /// The PCE JSON a create sends (with `{step:N}` placeholders for what
    /// the batch creates first); for an update, the services it adds.
    pub fn body(&self, p: &Proposal) -> serde_json::Value {
        match &self.action {
            RuleAction::Create => self.to_draft(p).to_body(),
            RuleAction::AddServices { add, .. } => serde_json::json!({
                "add_services": add.iter().map(ServiceRef::to_json).collect::<Vec<_>>()
            }),
            RuleAction::Exists { .. } => serde_json::Value::Null,
        }
    }
}

/// The flow records of the sections in force, by what happens to them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Coverage {
    pub covered: u64,
    pub uncovered: u64,
    pub unchecked: u64,
    pub folded_ips: u64,
    pub unsupported_proto: u64,
    pub broadcast_multicast: u64,
}

/// How many rules there are and what the batch does with them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct RuleCounts {
    pub rules: usize,
    pub create: usize,
    pub add_services: usize,
    pub exists: usize,
}

/// Everything `propose` derives, ready to preview or to plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    /// One per target, same index: 0 is the app's own scope.
    pub scopes: Vec<TargetScope>,
    pub targets: Vec<Target>,
    pub ip_lists: Vec<IpListDraft>,
    pub rules: Vec<GenRule>,
    pub coverage: Coverage,
}

impl Proposal {
    pub fn counts(&self) -> RuleCounts {
        let mut c = RuleCounts {
            rules: self.rules.len(),
            ..RuleCounts::default()
        };
        for r in &self.rules {
            match r.action {
                RuleAction::Create => c.create += 1,
                RuleAction::AddServices { .. } => c.add_services += 1,
                RuleAction::Exists { .. } => c.exists += 1,
            }
        }
        c
    }

    /// Whether anything would be written.
    pub fn writes(&self) -> bool {
        self.rules
            .iter()
            .any(|r| !matches!(r.action, RuleAction::Exists { .. }))
    }

    /// The batch step that creates IP list `i`: IP lists go first.
    pub fn ip_list_step(&self, i: usize) -> usize {
        i
    }

    /// The batch step that creates target `t`'s ruleset (after the IP
    /// lists, in target order); `None` unless it is created.
    pub fn ruleset_step(&self, t: usize) -> Option<usize> {
        if !matches!(self.targets.get(t), Some(Target::Create(_))) {
            return None;
        }
        let before = self.targets[..t]
            .iter()
            .filter(|x| matches!(x, Target::Create(_)))
            .count();
        Some(self.ip_lists.len() + before)
    }

    /// Why the rules cannot be planned as they are: a rule to write whose
    /// target is ambiguous or has no ruleset. `None` when they can.
    pub fn blocker(&self) -> Option<String> {
        self.rules
            .iter()
            .filter(|r| !matches!(r.action, RuleAction::Exists { .. }))
            .find_map(|r| match self.targets.get(r.target) {
                Some(Target::Ambiguous(c)) => Some(format!(
                    "{} rulesets cover {}: {}",
                    c.len(),
                    self.scopes[r.target].text(),
                    if r.target == 0 {
                        "R picks"
                    } else {
                        "uncheck its rows or narrow its rulesets"
                    }
                )),
                Some(Target::None(why)) => Some(why.clone()),
                _ => None,
            })
    }
}
```

Above the tests in `crates/pce/src/generate/services.rs`, add:

```rust
//! A flow's service as a rule names it: tcp and udp inline (`443/tcp`),
//! icmp through the ICMP service object, another protocol through the one
//! service whose only entry is that protocol. Existing service objects are
//! never matched by their ports.
use crate::models::proto_number;
use crate::resolve::NameIndex;
use crate::traffic::ResolvedFlow;
use crate::write::PortSpec;
use std::collections::BTreeSet;

/// A flow's port and protocol: tcp and udp keep their port, other
/// protocols have none. An unknown protocol is `-1`.
pub fn flow_spec(f: &ResolvedFlow) -> PortSpec {
    let proto = proto_number(&f.proto).map_or(-1, i64::from);
    let port = if matches!(proto, 6 | 17) {
        f.port
    } else {
        None
    };
    PortSpec {
        port,
        to_port: None,
        proto,
    }
}

/// How a seen service goes into a rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// tcp or udp, inline.
    Inline(PortSpec),
    /// A service object's href.
    Object(String),
    /// Nothing expresses it: the protocol number.
    Unsupported(i64),
}

pub fn express(spec: &PortSpec, idx: &NameIndex) -> Expr {
    match spec.proto {
        6 | 17 => Expr::Inline(spec.clone()),
        1 | 58 => idx
            .icmp_service_href()
            .map_or(Expr::Unsupported(spec.proto), |h| {
                Expr::Object(h.to_string())
            }),
        -1 => Expr::Unsupported(-1),
        p => single_proto_service(idx, p).map_or(Expr::Unsupported(p), Expr::Object),
    }
}

/// The one live service whose only entry is protocol `proto` without a
/// port; `None` for none or several.
fn single_proto_service(idx: &NameIndex, proto: i64) -> Option<String> {
    let hits: Vec<&crate::models::Service> = idx
        .services()
        .into_iter()
        .filter(|s| {
            matches!(s.service_ports.as_slice(), [p] if p.proto == Some(proto) && p.port.is_none())
        })
        .collect();
    match hits.as_slice() {
        [one] => Some(one.href.clone()),
        _ => None,
    }
}

/// Consecutive ports of one protocol as one range (`8000-8002/tcp`):
/// only ports that were all seen, so the rule stays exact.
pub fn fold_ports(ports: &BTreeSet<PortSpec>) -> Vec<PortSpec> {
    let mut out: Vec<PortSpec> = Vec::new();
    for p in ports {
        if let Some(last) = out.last_mut()
            && last.proto == p.proto
            && p.to_port.is_none()
            && let (Some(start), Some(port)) = (last.port, p.port)
            && last.to_port.unwrap_or(start).checked_add(1) == Some(port)
        {
            last.to_port = Some(port);
            continue;
        }
        out.push(p.clone());
    }
    out
}

```

Above the tests in `crates/pce/src/generate/target.rs`, add:

```rust
//! Which ruleset a scope's rules go into: the draft rulesets whose scope
//! covers it (the logic the `w` form's pre-fill uses, moved here so it
//! exists once), the name of a ruleset to create, and the keys check.
use super::{Target, TargetScope};
use crate::models::{Actor, RuleSet};
use crate::resolve::NameIndex;
use crate::write::{ActorKind, ActorRef, RuleSetDraft};
use serde::Serialize;
use std::collections::BTreeSet;

/// `generator.ruleset_name` when the profile names none and the scope keys
/// are the default ones: `RF-shop-prod`, the ringfence convention.
pub const DEFAULT_RULESET_NAME: &str = "RF-{app}-{env}";

/// The default template for any scope keys: `RF-` and every key, `-`
/// apart (`RF-{app}-{env}` for the default ones).
pub fn default_ruleset_name(scope_keys: &[String]) -> String {
    let keys: Vec<String> = scope_keys.iter().map(|k| format!("{{{k}}}")).collect();
    format!("RF-{}", keys.join("-"))
}

/// Every `{key}` in `template` must be a scope key; braces must close; the
/// template must not be blank. The error names the key.
pub fn check_ruleset_name(template: &str, scope_keys: &[String]) -> Result<(), String> {
    if template.trim().is_empty() {
        return Err("generator.ruleset_name must not be empty".into());
    }
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            return Err(format!(
                "generator.ruleset_name {template:?}: a {{ is not closed"
            ));
        };
        let key = &after[..close];
        if !scope_keys.iter().any(|k| k == key) {
            return Err(format!(
                "generator.ruleset_name {template:?}: {{{key}}} is not a scope key ({})",
                scope_keys.join(", ")
            ));
        }
        rest = &after[close + 1..];
    }
    Ok(())
}

/// `template` with each `{key}` replaced by the scope's value for it.
pub fn render_ruleset_name(template: &str, keys: &[String], values: &[String]) -> String {
    let mut out = template.to_string();
    for (k, v) in keys.iter().zip(values) {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out.trim().to_string()
}

/// The generator works on apps of the scope keys only: their rules go
/// into scoped rulesets, and only the scope keys say which.
pub fn keys_refusal(keys: &[String], scope_keys: &[String]) -> Option<String> {
    (keys != scope_keys).then(|| {
        format!(
            "the generator needs g = {} (the scope keys)",
            scope_keys.join(",")
        )
    })
}

/// The live draft rulesets with a scope that covers an endpoint holding
/// the labels `has`: each actor of one of its scopes is a label in `has`
/// (under any key) or a label group with such a label among its members.
/// The empty, global scope and other actors do not count. Those scoped
/// exactly like `want` (no group) come first (`true`), then the others,
/// the more actors the earlier, each in list order.
pub fn rulesets_covering<'r>(
    sets: &'r [RuleSet],
    has: &BTreeSet<&str>,
    want: &BTreeSet<&str>,
    idx: &NameIndex,
) -> Vec<(&'r RuleSet, bool)> {
    // `Some((exact, actors))` when the scope covers the endpoint.
    let fit = |s: &Vec<Actor>| -> Option<(bool, usize)> {
        if s.is_empty() {
            return None;
        }
        let mut labels = BTreeSet::new();
        let mut grouped = false;
        for a in s {
            if let Some(l) = &a.label {
                labels.insert(*has.get(l.href.as_str())?);
            } else if let Some(g) = &a.label_group {
                grouped = true;
                crate::impact::group_members(&g.href, idx)
                    .iter()
                    .any(|m| has.contains(m.as_str()))
                    .then_some(())?;
            } else {
                return None;
            }
        }
        let exact = !grouped && labels.into_iter().eq(want.iter().copied());
        Some((exact, s.len()))
    };
    let mut hits: Vec<(&RuleSet, (bool, usize))> = sets
        .iter()
        .filter(|rs| rs.update_type.as_deref() != Some("delete"))
        .filter_map(|rs| rs.scopes.iter().filter_map(fit).max().map(|f| (rs, f)))
        .collect();
    hits.sort_by_key(|(_, (exact, n))| (!exact, std::cmp::Reverse(*n)));
    hits.into_iter()
        .map(|(rs, (exact, _))| (rs, exact))
        .collect()
}

/// One draft ruleset whose scope covers the scope asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Covering {
    pub href: String,
    pub name: String,
    /// Scoped exactly by the scope's labels.
    pub exact: bool,
}

/// The covering rulesets, best first, and the one to use: the only exact
/// one, else the only one; `None` leaves the choice to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulesetChoice {
    pub covering: Vec<Covering>,
    pub picked: Option<usize>,
}

/// [`rulesets_covering`] and the pick, by label hrefs: `scope` the labels
/// a ruleset would be scoped by, `has` every label the endpoint carries
/// (for the generator's app, the scope labels themselves).
pub fn pick_ruleset(
    scope: &[&str],
    has: &[&str],
    sets: &[RuleSet],
    idx: &NameIndex,
) -> RulesetChoice {
    let want: BTreeSet<&str> = scope.iter().copied().collect();
    let has: BTreeSet<&str> = has.iter().copied().collect();
    let covering: Vec<Covering> = rulesets_covering(sets, &has, &want, idx)
        .into_iter()
        .map(|(rs, exact)| Covering {
            href: rs.href.clone(),
            name: rs.name.clone(),
            exact,
        })
        .collect();
    let exact: Vec<usize> = covering
        .iter()
        .enumerate()
        .filter(|(_, c)| c.exact)
        .map(|(i, _)| i)
        .collect();
    let picked = match (exact.as_slice(), covering.len()) {
        ([one], _) => Some(*one),
        ([], 1) => Some(0),
        _ => None,
    };
    RulesetChoice { covering, picked }
}

/// The target of one scope against `drafts`: the picked covering ruleset
/// (`picked` overrides among several), a ruleset to create when none
/// covers it, else ambiguous.
pub fn resolve(
    scope: &TargetScope,
    drafts: &[RuleSet],
    idx: &NameIndex,
    template: &str,
    picked: Option<&str>,
) -> Target {
    if scope.labels.is_empty() || scope.labels.len() != scope.values.len() {
        return Target::None(format!("{} has no scope labels", scope.text()));
    }
    let hrefs: Vec<&str> = scope.labels.iter().map(|l| l.href.as_str()).collect();
    let choice = pick_ruleset(&hrefs, &hrefs, drafts, idx);
    let chosen = picked
        .and_then(|h| choice.covering.iter().find(|c| c.href == h))
        .or_else(|| choice.picked.map(|i| &choice.covering[i]));
    if let Some(c) = chosen {
        return Target::Existing {
            href: c.href.clone(),
            name: c.name.clone(),
        };
    }
    if !choice.covering.is_empty() {
        return Target::Ambiguous(choice.covering);
    }
    let keys: Vec<String> = scope.labels.iter().map(|l| l.key.clone()).collect();
    Target::Create(RuleSetDraft {
        name: render_ruleset_name(template, &keys, &scope.values),
        description: None,
        enabled: true,
        scopes: vec![
            scope
                .labels
                .iter()
                .map(|l| ActorRef::new(ActorKind::Label, l.href.clone()))
                .collect(),
        ],
    })
}

```

In `crates/illumio-tui/src/tui/edit.rs`, the covering logic moves to `pce::generate`; replace the private `rulesets_covering`:

```rust
    }
}

/// The live draft rulesets with a scope that covers the destination `dst`:
/// each actor of one of its scopes is a label `dst` has (under any key, so
/// a scope `app, env, loc=eu` covers a destination with `loc=eu`) or a
/// label group with such a label among its members (through `idx`). The
/// empty, global scope and other actors do not count. Those scoped exactly
/// like `scope` (`dst`'s labels under the scope keys, no group) come first
/// (`true`), then the others, the more actors the earlier, each in list
/// order.
fn rulesets_covering<'r>(
    sets: &'r [RuleSet],
    dst: &ResolvedEndpoint,
```

with:

```rust
    }
}

/// The live draft rulesets with a scope that covers the destination `dst`
/// (`pce::generate::rulesets_covering` over its labels): those scoped
/// exactly like `scope` first, then the others.
fn rulesets_covering<'r>(
    sets: &'r [RuleSet],
    dst: &ResolvedEndpoint,
```

In `crates/illumio-tui/src/tui/edit.rs`, replace:

```rust
) -> Vec<(&'r RuleSet, bool)> {
    let want: BTreeSet<&str> = scope.iter().map(|l| l.href.as_str()).collect();
    let has: BTreeSet<&str> = dst.labels.iter().map(|l| l.href.as_str()).collect();
    // `Some((exact, actors))` when the scope covers `dst`.
    let fit = |s: &Vec<Actor>| -> Option<(bool, usize)> {
        if s.is_empty() {
            return None;
        }
        let mut labels = BTreeSet::new();
        let mut grouped = false;
        for a in s {
            if let Some(l) = &a.label {
                labels.insert(has.get(l.href.as_str())?);
            } else if let Some(g) = &a.label_group {
                grouped = true;
                pce::impact::group_members(&g.href, idx)
                    .iter()
                    .any(|m| has.contains(m.as_str()))
                    .then_some(())?;
            } else {
                return None;
            }
        }
        let exact = !grouped && labels.into_iter().copied().eq(want.iter().copied());
        Some((exact, s.len()))
    };
    let mut hits: Vec<(&RuleSet, (bool, usize))> = sets
        .iter()
        .filter(|rs| rs.update_type.as_deref() != Some("delete"))
        .filter_map(|rs| rs.scopes.iter().filter_map(fit).max().map(|f| (rs, f)))
        .collect();
    hits.sort_by_key(|(_, (exact, n))| (!exact, std::cmp::Reverse(*n)));
    hits.into_iter()
        .map(|(rs, (exact, _))| (rs, exact))
        .collect()
}

/// How the description names a side: its group (`shop/prod`) when the
```

with:

```rust
) -> Vec<(&'r RuleSet, bool)> {
    let want: BTreeSet<&str> = scope.iter().map(|l| l.href.as_str()).collect();
    let has: BTreeSet<&str> = dst.labels.iter().map(|l| l.href.as_str()).collect();
    pce::generate::rulesets_covering(sets, &has, &want, idx)
}

/// How the description names a side: its group (`shop/prod`) when the
```

and in `ruleset_pick`, take the pick from `pce::generate::pick_ruleset`:

```rust
            covering: Vec::new(),
        };
    };
    let hits = rulesets_covering(sets, &f.dst, &dst_scope, idx);
    let covering: Vec<String> = hits.iter().map(|(rs, _)| rs.name.clone()).collect();
    let exact: Vec<&str> = hits
        .iter()
        .filter(|(_, e)| *e)
        .map(|(rs, _)| rs.name.as_str())
        .collect();
    let covers_scope = p
        .group_keys
        .is_none_or(|keys| p.scope_keys.iter().all(|k| keys.contains(k)));
    let name = match (exact.as_slice(), covering.as_slice()) {
        _ if !covers_scope => None,
        ([one], _) => Some(one.to_string()),
        ([], [one]) => Some(one.clone()),
        _ => None,
    };

    let hint = match covering.len() {
        _ if !covers_scope => Cow::Borrowed(SPANS_SCOPES_HINT),
```

with:

```rust
            covering: Vec::new(),
        };
    };
    let want: Vec<&str> = dst_scope.iter().map(|l| l.href.as_str()).collect();
    let has: Vec<&str> = f.dst.labels.iter().map(|l| l.href.as_str()).collect();
    let choice = pce::generate::pick_ruleset(&want, &has, sets, idx);
    let covering: Vec<String> = choice.covering.iter().map(|c| c.name.clone()).collect();
    let covers_scope = p
        .group_keys
        .is_none_or(|keys| p.scope_keys.iter().all(|k| keys.contains(k)));
    let name = choice
        .picked
        .filter(|_| covers_scope)
        .map(|i| choice.covering[i].name.clone());

    let hint = match covering.len() {
        _ if !covers_scope => Cow::Borrowed(SPANS_SCOPES_HINT),
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p pce --lib generate && cargo test -p illumio-tui --lib edit`
Expected: PASS: the generate tests (services, target, data model) and every existing `edit` test (the `w` form's ruleset pick is unchanged).

- [ ] **Step 5: Commit**

```bash
git add crates/pce/src/lib.rs crates/pce/src/generate crates/illumio-tui/src/tui/edit.rs
git commit -m "pce::generate: data model, services, ruleset targets; ruleset_pick uses them

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

### Task 3: `pce::generate::rows`: flows to rows

**Files:**
- Create: `crates/pce/src/generate/rows.rs`
- Modify: `crates/pce/src/generate/mod.rs`

**Interfaces:**
- Consumes: Task 2 (`GenRow`, `PeerKind`, `PeerKey`, `RowKey`, `RowFlag`, `DecisionCounts`, `express`, `flow_spec`, `fold_ports`); `pce::traffic::group_key`; `NameIndex::ip_list_href`, `ip_list`.
- Produces:

```rust
pub const MAX_IP_ROWS: usize = 25;
pub const MANY_SERVICES: usize = 32;
pub const EPHEMERAL_FROM: u16 = 49152;
pub fn excluded(f: &ResolvedFlow) -> bool;           // multicast or limited broadcast destination
pub fn is_any_ip_list(l: &IpList) -> bool;
pub fn ip_list_name(app: &[String], ip: &str) -> String;   // "shop-prod peer 10.4.1.9"
pub fn rows(flows: &[ResolvedFlow], focus: &Focus, tier_key: &str, idx: &NameIndex) -> Vec<GenRow>;
pub(crate) fn side_of(f: &ResolvedFlow, focus: &Focus) -> Option<Side>;
pub(crate) fn row_key(f: &ResolvedFlow, focus: &Focus) -> Option<RowKey>;
pub(crate) fn scope_values(ep: &ResolvedEndpoint, keys: &[String]) -> Option<Vec<String>>;
// test helpers, `pub(crate) mod tests`: shop(), texts(rows), to_ip(i, ip), intra_on(port)
```

Rows come in this order: inbound first; in each section intra-app, app peers (by flows, then name), unlabelled workloads, IP peers (by connections, at most 25; the rest fold into one read-only row), the folded row. Defaults follow the spec's "Row defaults" table.

- [ ] **Step 1: Write the failing tests**

The fixtures: `rich_flows()` holds 8 flows into shop/prod (4 intra-app, hr/prod, shop/dev, two from 10.1.4.9 in the Corp list) and 2 out of it (crm/prod, 203.0.113.7 in no list).

Create `crates/pce/src/generate/rows.rs` with its tests only (the code follows in Step 3):

```rust
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::traffic::parse_app;
    use crate::traffic::resolved::tests::{rich_flows, rich_index};

    pub(crate) fn shop() -> Focus {
        Focus::new(parse_app("shop/prod"), vec!["app".into(), "env".into()])
    }

    pub(crate) fn texts(rows: &[GenRow]) -> Vec<(Side, String, bool, bool)> {
        rows.iter()
            .map(|r| (r.side, r.peer_text(), r.checked, r.checkable))
            .collect()
    }

    /// Flow `i` of the rich fixture with its destination IP replaced (and
    /// no workload there).
    pub(crate) fn to_ip(i: usize, ip: &str) -> ResolvedFlow {
        let mut f = rich_flows()[i].clone();
        f.dst = crate::traffic::ResolvedEndpoint {
            ip: Some(ip.into()),
            ..Default::default()
        };
        f
    }

    /// Flow 0 (shop web to shop db, intra-app) on another port.
    pub(crate) fn intra_on(port: u16) -> ResolvedFlow {
        let mut f = rich_flows()[0].clone();
        f.port = Some(port);
        f
    }

    #[test]
    fn shop_prod_has_four_inbound_and_two_outbound_rows_at_their_defaults() {
        let r = rows(&rich_flows(), &shop(), "role", &rich_index());
        use Side::*;
        assert_eq!(
            texts(&r),
            vec![
                (Inbound, "intra-app".into(), true, true),
                (Inbound, "hr/prod".into(), true, true),
                (Inbound, "shop/dev".into(), false, true),
                (Inbound, "10.1.4.9".into(), false, true),
                (Outbound, "crm/prod".into(), true, true),
                (Outbound, "203.0.113.7".into(), false, true),
            ]
        );
        assert_eq!(
            (r[0].flows, r[0].connections),
            (4, 821),
            "flows 0, 1, 26, 29"
        );
        assert_eq!(r[0].decisions.allowed, 4);
        assert_eq!(
            r[1].decisions.blocked, 1,
            "potentially blocked counts as blocked"
        );
        assert_eq!(r[2].flags, vec![RowFlag::EnvMismatch]);
        assert_eq!(
            r[2].flag_texts(super::super::Stop::AppPorts, &shop().keys),
            vec!["env mismatch"]
        );
        assert_eq!(r[3].flags, vec![RowFlag::UsesIpList("Corp".into())]);
        assert_eq!(
            r[3].services
                .iter()
                .map(PortSpec::display)
                .collect::<Vec<_>>(),
            vec!["443/tcp", "5432/tcp"]
        );
        assert_eq!(r[5].flags, vec![RowFlag::NeedsIpList]);
        assert_eq!(
            r[3].peer,
            PeerKind::Ip {
                ip: "10.1.4.9".into(),
                list: Some("Corp".into())
            }
        );
    }

    #[test]
    fn broadcast_and_multicast_never_make_a_row() {
        let mut f = rich_flows();
        f.push(to_ip(22, "224.0.0.251"));
        f.push(to_ip(22, "255.255.255.255"));
        f.push(to_ip(22, "ff02::fb"));
        assert!(excluded(&f[30]) && excluded(&f[31]) && excluded(&f[32]));
        let r = rows(&f, &shop(), "role", &rich_index());
        assert_eq!(r.len(), 6, "no row for the three");
        assert!(
            !excluded(&to_ip(22, "10.255.255.255")),
            "a subnet broadcast is an IP peer"
        );
    }

    #[test]
    fn an_unlabelled_workload_is_its_own_row_checkable_inbound_only() {
        let mut f = rich_flows();
        let mut inb = f[12].clone(); // hr/prod web -> shop db
        inb.src.labels.retain(|l| l.key != "env");
        let mut out = f[2].clone(); // shop web -> crm web
        out.dst.labels.clear();
        f.extend([inb, out]);
        let r = rows(&f, &shop(), "role", &rich_index());
        let wl: Vec<&GenRow> = r
            .iter()
            .filter(|r| matches!(r.peer, PeerKind::Workload { .. }))
            .collect();
        assert_eq!(wl.len(), 2);
        assert_eq!(
            (wl[0].side, wl[0].checked, wl[0].checkable),
            (Side::Inbound, false, true)
        );
        assert_eq!(
            (wl[1].side, wl[1].checked, wl[1].checkable),
            (Side::Outbound, false, false)
        );
        assert_eq!(
            wl[0].flags[0].text(Side::Inbound, &shop().keys),
            "no scope labels: rule names the workload"
        );
        assert_eq!(
            wl[1].flags[0].text(Side::Outbound, &shop().keys),
            "no scope labels: no ruleset to write into"
        );
        assert_eq!(
            wl[0].peer,
            PeerKind::Workload {
                href: "/orgs/5/workloads/rich-hr-web-p".into(),
                name: f[12].src.display_name()
            }
        );
    }

    #[test]
    fn ip_peers_past_25_fold_into_one_read_only_row_by_connections() {
        let mut f = rich_flows();
        for i in 0..27u64 {
            let mut x = to_ip(22, &format!("198.51.100.{i}"));
            x.num_connections = 100 + i;
            f.push(x);
        }
        let r = rows(&f, &shop(), "role", &rich_index());
        let out: Vec<&GenRow> = r.iter().filter(|r| r.side == Side::Outbound).collect();
        // crm/prod, 25 IPs, the folded row.
        assert_eq!(out.len(), 27);
        let folded = out.last().unwrap();
        assert_eq!(
            folded.peer,
            PeerKind::FoldedIps(3),
            "the 3 smallest: 203.0.113.7, .0, .1"
        );
        assert_eq!(folded.peer_text(), "3 more IPs: not proposed");
        assert!(!folded.checked && !folded.checkable);
        assert_eq!(out[1].peer_text(), "198.51.100.26", "the largest first");
    }

    #[test]
    fn only_unsupported_services_make_a_row_uncheckable() {
        let mut f = rich_flows();
        let mut icmp = f[12].clone(); // hr/prod -> shop/prod
        icmp.port = None;
        icmp.proto = "icmp".into();
        icmp.src.labels.iter_mut().for_each(|l| {
            if l.key == "app" {
                l.value = "erp".into();
                l.href = "/orgs/5/labels/102".into();
            }
        });
        f.push(icmp);
        let r = rows(&f, &shop(), "role", &rich_index());
        let erp = r.iter().find(|r| r.peer_text() == "erp/prod").unwrap();
        assert_eq!(erp.flags, vec![RowFlag::UnsupportedProto(1)]);
        assert!(
            !erp.checked && !erp.checkable,
            "the fixture has no ICMP service"
        );
        assert_eq!(
            erp.flags[0].text(Side::Inbound, &[]),
            "proto 1 needs a service object"
        );
    }

    #[test]
    fn many_services_and_ephemeral_ports_are_flagged_and_stay_checked() {
        let mut f = rich_flows();
        f.extend((0..33u16).map(|i| intra_on(1000 + 2 * i)));
        f.push(intra_on(50_000));
        let r = rows(&f, &shop(), "role", &rich_index());
        assert!(r[0].checked);
        assert!(
            r[0].flags.contains(&RowFlag::ManyServices(35)),
            "{:?}",
            r[0].flags
        );
        assert!(r[0].flags.contains(&RowFlag::Ephemeral));
    }

    #[test]
    fn a_flow_without_a_role_flags_the_row_only_at_role_stops() {
        let mut f = rich_flows();
        let mut x = f[12].clone();
        x.src.labels.retain(|l| l.key != "role");
        f.push(x);
        let r = rows(&f, &shop(), "role", &rich_index());
        let hr = r.iter().find(|r| r.peer_text() == "hr/prod").unwrap();
        assert_eq!(hr.flags, vec![RowFlag::NoRole { flows: 1 }]);
        assert!(hr.flag_texts(super::super::Stop::AppPorts, &[]).is_empty());
        assert_eq!(
            hr.flag_texts(super::super::Stop::RolePorts, &[]),
            vec!["1 flows without role: app-level for those"]
        );
    }

    #[test]
    fn with_one_scope_key_every_app_peer_is_checked() {
        let shop = Focus::new(parse_app("shop"), vec!["app".into()]);
        let r = rows(&rich_flows(), &shop, "role", &rich_index());
        assert!(
            r.iter()
                .filter(|r| matches!(r.peer, PeerKind::App(_)))
                .all(|r| r.checked && r.flags.is_empty()),
            "{:?}",
            texts(&r)
        );
    }

    #[test]
    fn the_any_list_is_known_by_its_data_set_or_its_name() {
        let any: IpList = serde_json::from_value(serde_json::json!({
            "href": "/orgs/5/sec_policy/draft/ip_lists/1", "name": "Everything",
            "external_data_set": "com.illumio.ilo_defaults"
        }))
        .unwrap();
        assert!(is_any_ip_list(&any));
        let named: IpList = serde_json::from_value(serde_json::json!({
            "href": "/orgs/5/sec_policy/draft/ip_lists/2", "name": "Any (0.0.0.0/0 and ::/0)"
        }))
        .unwrap();
        assert!(is_any_ip_list(&named));
        let mut f = rich_flows()[17].clone();
        f.src.ip_lists = vec!["Any (0.0.0.0/0 and ::/0)".into()];
        let r = rows(&[f], &shop(), "role", &rich_index());
        assert_eq!(
            r[0].flags,
            vec![RowFlag::NeedsIpList],
            "the Any list is never named"
        );
        assert_eq!(
            ip_list_name(&shop().app, "10.1.4.9"),
            "shop-prod peer 10.1.4.9"
        );
    }
}
```

In `crates/pce/src/generate/mod.rs`, declare it:

```rust
pub mod services;
```

with:

```rust
pub mod rows;
pub mod services;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p pce --lib generate::rows`
Expected: FAIL to compile: "cannot find function `rows` in this scope".

- [ ] **Step 3: Write the implementation**

Above the tests in `crates/pce/src/generate/rows.rs`, add:

```rust
//! Flows to rows: one intra-app row, one row per app peer, IP peer and
//! unlabelled workload, on each side, with their default checkbox and
//! their flags. Broadcast and multicast are left out first.
use super::services::{Expr, express, flow_spec, fold_ports};
use super::{DecisionCounts, GenRow, PeerKey, PeerKind, RowFlag, RowKey, Side};
use crate::models::IpList;
use crate::resolve::NameIndex;
use crate::traffic::{Focus, ResolvedEndpoint, ResolvedFlow, group_key};
use crate::write::PortSpec;
use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;

/// IP peer rows kept per section; the rest fold into one row.
pub const MAX_IP_ROWS: usize = 25;
/// A rule with more service entries than this is flagged.
pub const MANY_SERVICES: usize = 32;
/// Destination ports from here up are flagged as possibly ephemeral.
pub const EPHEMERAL_FROM: u16 = 49152;

/// The PCE's built-in Any list (`0.0.0.0/0` and `::/0`).
const ANY_LIST: &str = "Any (0.0.0.0/0 and ::/0)";

/// A multicast (`224.0.0.0/4`, `ff00::/8`) or limited broadcast
/// (`255.255.255.255`) destination: never proposed.
pub fn excluded(f: &ResolvedFlow) -> bool {
    f.dst
        .ip
        .as_deref()
        .and_then(|s| s.parse::<IpAddr>().ok())
        .is_some_and(|ip| match ip {
            IpAddr::V4(v4) => v4.is_multicast() || v4.is_broadcast(),
            IpAddr::V6(v6) => v6.is_multicast(),
        })
}

/// The built-in Any list, by its external data set, else by its name.
pub fn is_any_ip_list(l: &IpList) -> bool {
    l.external_data_set.as_deref() == Some("com.illumio.ilo_defaults") || l.name == ANY_LIST
}

/// The IP list the batch creates for an IP peer: `shop-prod peer 10.4.1.9`.
pub fn ip_list_name(app: &[String], ip: &str) -> String {
    format!("{} peer {ip}", app.join("-"))
}

/// `in` when the destination is the app (a self-flow too), `out` when
/// only the source is; `None` for a flow that is not the app's.
pub(crate) fn side_of(f: &ResolvedFlow, focus: &Focus) -> Option<Side> {
    if group_key(&f.dst, &focus.keys) == focus.app {
        Some(Side::Inbound)
    } else if group_key(&f.src, &focus.keys) == focus.app {
        Some(Side::Outbound)
    } else {
        None
    }
}

/// The endpoint's value under every key, or `None` when one is missing.
pub(crate) fn scope_values(ep: &ResolvedEndpoint, keys: &[String]) -> Option<Vec<String>> {
    if !ep.is_workload() {
        return None;
    }
    keys.iter()
        .map(|k| ep.label_value(k).map(str::to_string))
        .collect()
}

/// The row a flow belongs to; `None` when excluded or not the app's.
pub(crate) fn row_key(f: &ResolvedFlow, focus: &Focus) -> Option<RowKey> {
    if excluded(f) {
        return None;
    }
    let side = side_of(f, focus)?;
    let other = match side {
        Side::Inbound if group_key(&f.src, &focus.keys) == focus.app => {
            return Some(RowKey {
                side,
                peer: PeerKey::Intra,
            });
        }
        Side::Inbound => &f.src,
        Side::Outbound => &f.dst,
    };
    let peer = if other.is_workload() {
        match scope_values(other, &focus.keys) {
            Some(v) => PeerKey::App(v),
            None => PeerKey::Workload(other.workload_href.clone().unwrap_or_default()),
        }
    } else {
        PeerKey::Ip(other.ip.clone().unwrap_or_else(|| "?".into()))
    };
    Some(RowKey { side, peer })
}

/// Whether role granularity misses a role on a side it uses.
fn lacks_role(f: &ResolvedFlow, key: &RowKey, tier_key: &str) -> bool {
    let role = |e: &ResolvedEndpoint| e.label_value(tier_key).is_none();
    match (&key.side, &key.peer) {
        (Side::Inbound, PeerKey::Intra | PeerKey::App(_)) => role(&f.src) || role(&f.dst),
        (Side::Inbound, _) => role(&f.dst),
        (Side::Outbound, PeerKey::App(_)) => role(&f.src) || role(&f.dst),
        (Side::Outbound, _) => role(&f.src),
    }
}

/// The first IP list the PCE reported for the endpoint other than the
/// built-in Any list, else an existing list named as the batch would name
/// it; `None` when the batch creates one.
fn peer_list(ep: &ResolvedEndpoint, app: &[String], idx: &NameIndex) -> Option<String> {
    let any = |name: &str| match idx.ip_list_href(name).and_then(|h| idx.ip_list(h)) {
        Some(l) => is_any_ip_list(l),
        None => name == ANY_LIST,
    };
    ep.ip_lists.iter().find(|n| !any(n)).cloned().or_else(|| {
        let ip = ep.ip.as_deref()?;
        let name = ip_list_name(app, ip);
        idx.ip_list_href(&name).map(|_| name)
    })
}

#[derive(Default)]
struct Acc {
    flows: u64,
    connections: u64,
    decisions: DecisionCounts,
    services: BTreeSet<PortSpec>,
    no_role: u64,
    ephemeral: bool,
    /// The IP peer's list, the workload's name: from the first flow.
    list: Option<String>,
    name: String,
}

/// The rows of `flows` for the app `focus` names (its keys must be the
/// scope keys; the caller checks). Inbound first, then outbound; in each:
/// intra-app, app peers (by flows), unlabelled workloads, IP peers (by
/// connections, at most [`MAX_IP_ROWS`], the rest folded). Every row's
/// checkbox at its default.
pub fn rows(flows: &[ResolvedFlow], focus: &Focus, tier_key: &str, idx: &NameIndex) -> Vec<GenRow> {
    let mut acc: BTreeMap<RowKey, Acc> = BTreeMap::new();
    for f in flows {
        let Some(key) = row_key(f, focus) else {
            continue;
        };
        let a = acc.entry(key.clone()).or_default();
        if a.flows == 0 {
            let other = match key.side {
                Side::Inbound => &f.src,
                Side::Outbound => &f.dst,
            };
            a.name = other.display_name();
            if matches!(key.peer, PeerKey::Ip(_)) {
                a.list = peer_list(other, &focus.app, idx);
            }
        }
        a.flows += 1;
        a.connections += f.num_connections;
        a.decisions.add(f);
        let spec = flow_spec(f);
        a.ephemeral |= spec.port.is_some_and(|p| p >= EPHEMERAL_FROM);
        a.services.insert(spec);
        a.no_role += u64::from(lacks_role(f, &key, tier_key));
    }
    let mut out = Vec::new();
    for side in [Side::Inbound, Side::Outbound] {
        let mut rows: Vec<GenRow> = acc
            .iter()
            .filter(|(k, _)| k.side == side)
            .map(|(k, a)| row(k, a, focus, idx))
            .collect();
        rows.sort_by_key(order);
        let ips = rows
            .iter()
            .filter(|r| matches!(r.peer, PeerKind::Ip { .. }))
            .count();
        if ips > MAX_IP_ROWS {
            let first_ip = rows
                .iter()
                .position(|r| matches!(r.peer, PeerKind::Ip { .. }))
                .unwrap_or(rows.len());
            let rest: Vec<GenRow> = rows.split_off(first_ip + MAX_IP_ROWS);
            rows.push(fold(side, &rest));
        }
        out.extend(rows);
    }
    out
}

/// Intra-app, app peers, workloads, IP peers, folded; then by size.
fn order(r: &GenRow) -> (u8, std::cmp::Reverse<u64>, String) {
    match &r.peer {
        PeerKind::Intra => (0, std::cmp::Reverse(0), String::new()),
        PeerKind::App(v) => (1, std::cmp::Reverse(r.flows), v.join("/")),
        PeerKind::Workload { href, .. } => (2, std::cmp::Reverse(r.flows), href.clone()),
        PeerKind::Ip { ip, .. } => (3, std::cmp::Reverse(r.connections), ip.clone()),
        PeerKind::FoldedIps(_) => (4, std::cmp::Reverse(0), String::new()),
    }
}

fn row(key: &RowKey, a: &Acc, focus: &Focus, idx: &NameIndex) -> GenRow {
    let mut flags = Vec::new();
    let (peer, mut checked, mut checkable) = match &key.peer {
        PeerKey::Intra => (PeerKind::Intra, true, true),
        PeerKey::App(v) => {
            let same_env = v.get(1..) == focus.app.get(1..);
            if !same_env {
                flags.push(RowFlag::EnvMismatch);
            }
            (PeerKind::App(v.clone()), same_env, true)
        }
        PeerKey::Ip(ip) => {
            let addressable = ip.parse::<IpAddr>().is_ok();
            match (&a.list, addressable) {
                (Some(n), _) => flags.push(RowFlag::UsesIpList(n.clone())),
                (None, true) => flags.push(RowFlag::NeedsIpList),
                (None, false) => flags.push(RowFlag::NoAddress),
            }
            let peer = PeerKind::Ip {
                ip: ip.clone(),
                list: a.list.clone(),
            };
            (peer, false, a.list.is_some() || addressable)
        }
        PeerKey::Workload(href) => {
            flags.push(RowFlag::NoScopeLabels);
            let peer = PeerKind::Workload {
                href: href.clone(),
                name: a.name.clone(),
            };
            (peer, false, key.side == Side::Inbound)
        }
        PeerKey::Folded => (PeerKind::FoldedIps(0), false, false),
    };
    let mut inline = BTreeSet::new();
    let mut objects = BTreeSet::new();
    let mut unsupported = BTreeSet::new();
    for s in &a.services {
        match express(s, idx) {
            Expr::Inline(p) => {
                inline.insert(p);
            }
            Expr::Object(h) => {
                objects.insert(h);
            }
            Expr::Unsupported(p) => {
                unsupported.insert(p);
            }
        }
    }
    flags.extend(unsupported.iter().map(|p| RowFlag::UnsupportedProto(*p)));
    if inline.is_empty() && objects.is_empty() {
        checked = false;
        checkable = false;
    }
    let entries = fold_ports(&inline).len() + objects.len();
    if entries > MANY_SERVICES {
        flags.push(RowFlag::ManyServices(entries));
    }
    if a.ephemeral {
        flags.push(RowFlag::Ephemeral);
    }
    if a.no_role > 0 {
        flags.push(RowFlag::NoRole { flows: a.no_role });
    }
    GenRow {
        side: key.side,
        peer,
        checked,
        checkable,
        stop: None,
        flags,
        flows: a.flows,
        connections: a.connections,
        decisions: a.decisions,
        services: a.services.clone(),
    }
}

/// The IP rows past the limit as one read-only row.
fn fold(side: Side, rest: &[GenRow]) -> GenRow {
    let mut decisions = DecisionCounts::default();
    for r in rest {
        decisions.merge(&r.decisions);
    }
    GenRow {
        side,
        peer: PeerKind::FoldedIps(rest.len()),
        checked: false,
        checkable: false,
        stop: None,
        flags: Vec::new(),
        flows: rest.iter().map(|r| r.flows).sum(),
        connections: rest.iter().map(|r| r.connections).sum(),
        decisions,
        services: rest
            .iter()
            .flat_map(|r| r.services.iter().cloned())
            .collect(),
    }
}

```

In `crates/pce/src/generate/mod.rs`, re-export it:

```rust
pub use services::
```

with:

```rust
pub use rows::{MANY_SERVICES, MAX_IP_ROWS, excluded, ip_list_name, is_any_ip_list, rows};
pub use services::
```

and give `DecisionCounts` the sum the folded row needs:

```rust
            *n += u64::from(d.class() == PolicyDecision::Allowed);
        }
    }
```

with:

```rust
            *n += u64::from(d.class() == PolicyDecision::Allowed);
        }
    }

    fn merge(&mut self, o: &DecisionCounts) {
        self.allowed += o.allowed;
        self.blocked += o.blocked;
        if let Some(n) = o.draft_allowed {
            *self.draft_allowed.get_or_insert(0) += n;
        }
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p pce --lib generate::rows`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/pce/src/generate
git commit -m "pce::generate: rows from flows, defaults, flags, IP folding

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

### Task 4: `pce::generate::dedupe` and `Proposal::refresh`

**Files:**
- Create: `crates/pce/src/generate/dedupe.rs`
- Modify: `crates/pce/src/generate/mod.rs`

**Interfaces:**
- Consumes: Task 2 (`GenRule`, `GenActor::sort_key`, `actor_key`, `Target`, `target::resolve`); `pce::impact::group_members`; `pce::resolve::canon`; `ActorRef::from_actor`.
- Produces:

```rust
pub(crate) fn classify(rule: &GenRule, target: &Target, drafts: &[RuleSet], idx: &NameIndex) -> RuleAction;
impl Proposal {
    /// Targets and actions again: the fresh read at plan time (Plan 2), or the rulesets just loaded (Plan 3).
    pub fn refresh(&mut self, drafts: &[RuleSet], idx: &NameIndex, template: &str, picked: Option<&str>);
}
```

The cover test is the one in "Spec deviations": sides as sets with AND across keys, `ams` on the in-scope side only (providers always; consumers when `unscoped_consumers` is false), services through All Services, a range, inline ports or a service object's ports; an ICMP-typed entry covers only its own href.

- [ ] **Step 1: Write the failing tests**

Create `crates/pce/src/generate/dedupe.rs` with its tests only (the code follows in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::super::{Coverage, GenRule, Proposal, TargetScope};
    use super::*;
    use crate::resolve::ResolvedLabel;
    use crate::traffic::resolved::tests::rich_index;
    use crate::write::PortSpec;
    use serde_json::json;

    const RS: &str = "/orgs/5/sec_policy/draft/rule_sets/100";

    fn port(s: &str) -> ServiceRef {
        ServiceRef::Port(PortSpec::parse(s).unwrap())
    }

    fn lbl(id: u32) -> GenActor {
        GenActor::label(&format!("/orgs/5/labels/{id}"))
    }

    fn target() -> Target {
        Target::Existing {
            href: RS.into(),
            name: "Shop-Prod".into(),
        }
    }

    /// Shop-Prod with exactly these rules.
    fn drafts(rules: serde_json::Value) -> Vec<RuleSet> {
        let mut sets: Vec<RuleSet> =
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
        sets[0].rules = serde_json::from_value(rules).unwrap();
        sets
    }

    fn rule(json: serde_json::Value) -> serde_json::Value {
        let mut r = json!({
            "href": format!("{RS}/sec_rules/9"), "enabled": true,
            "providers": [{"actors": "ams"}], "consumers": [{"actors": "ams"}],
            "ingress_services": [{"port": 5432, "proto": 6}], "unscoped_consumers": false
        });
        for (k, v) in json.as_object().unwrap() {
            r[k] = v.clone();
        }
        r
    }

    fn proposed(
        providers: Vec<GenActor>,
        consumers: Vec<GenActor>,
        unscoped: bool,
        services: Vec<ServiceRef>,
    ) -> GenRule {
        GenRule {
            target: 0,
            providers,
            consumers,
            unscoped_consumers: unscoped,
            services,
            rows: vec![0],
            action: RuleAction::Create,
            description: String::new(),
        }
    }

    fn intra() -> GenRule {
        proposed(
            vec![GenActor::all()],
            vec![GenActor::all()],
            false,
            vec![port("5432/tcp")],
        )
    }

    /// hr/prod into shop/prod: `app=hr, env=prod`, unscoped.
    fn hr() -> GenRule {
        proposed(
            vec![GenActor::all()],
            vec![lbl(103), lbl(4)],
            true,
            vec![port("5432/tcp")],
        )
    }

    fn classify_in(rules: serde_json::Value, r: &GenRule) -> RuleAction {
        classify(r, &target(), &drafts(json!([rules])), &rich_index())
    }

    fn exists() -> RuleAction {
        RuleAction::Exists {
            rule_href: format!("{RS}/sec_rules/9"),
        }
    }

    #[test]
    fn the_same_rule_exists() {
        assert_eq!(classify_in(rule(json!({})), &intra()), exists());
    }

    #[test]
    fn missing_services_on_the_same_actors_are_added() {
        let r = rule(json!({"ingress_services": [{"port": 443, "proto": 6}]}));
        assert_eq!(
            classify_in(r, &intra()),
            RuleAction::AddServices {
                rule_href: format!("{RS}/sec_rules/9"),
                add: vec![port("5432/tcp")]
            }
        );
    }

    #[test]
    fn all_services_a_range_and_a_service_object_cover_a_port() {
        for svc in [
            json!([{"href": "/orgs/5/sec_policy/draft/services/30"}]),
            json!([{"port": 5000, "to_port": 6000, "proto": 6}]),
            json!([{"href": "/orgs/5/sec_policy/draft/services/31"}]),
        ] {
            assert_eq!(
                classify_in(rule(json!({"ingress_services": svc})), &intra()),
                exists(),
                "{svc}"
            );
        }
        let udp = rule(json!({"ingress_services": [{"port": 5432, "proto": 17}]}));
        assert_ne!(classify_in(udp, &intra()), exists());
    }

    #[test]
    fn a_label_group_member_covers_and_a_broader_side_covers() {
        let grouped = rule(json!({
            "consumers": [{"label": {"href": "/orgs/5/labels/103"}},
                          {"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/10"}}],
            "unscoped_consumers": true
        }));
        assert_eq!(classify_in(grouped, &hr()), exists());
        let app_only = rule(json!({
            "consumers": [{"label": {"href": "/orgs/5/labels/103"}}], "unscoped_consumers": true
        }));
        assert_eq!(
            classify_in(app_only, &hr()),
            exists(),
            "every env of hr holds hr/prod"
        );
    }

    #[test]
    fn a_narrower_existing_rule_is_not_a_cover() {
        let narrower = rule(json!({
            "consumers": [{"label": {"href": "/orgs/5/labels/103"}}, {"label": {"href": "/orgs/5/labels/4"}},
                          {"label": {"href": "/orgs/5/labels/1"}}],
            "unscoped_consumers": true
        }));
        assert_eq!(classify_in(narrower, &hr()), RuleAction::Create);
        let role_only = rule(json!({"providers": [{"label": {"href": "/orgs/5/labels/2"}}]}));
        assert_eq!(
            classify_in(role_only, &intra()),
            RuleAction::Create,
            "role=db is not all workloads"
        );
    }

    #[test]
    fn ams_covers_on_the_in_scope_side_only() {
        let ams = rule(json!({}));
        let scoped = proposed(
            vec![GenActor::all()],
            vec![lbl(1)],
            false,
            vec![port("5432/tcp")],
        );
        assert_eq!(classify_in(ams.clone(), &scoped), exists());
        let mut unscoped_ams = ams;
        unscoped_ams["unscoped_consumers"] = json!(true);
        assert_eq!(classify_in(unscoped_ams, &hr()), RuleAction::Create);
    }

    #[test]
    fn a_different_unscoped_flag_is_another_rule() {
        let r = rule(json!({"unscoped_consumers": true}));
        assert_eq!(classify_in(r, &intra()), RuleAction::Create);
    }

    #[test]
    fn disabled_pending_delete_exclusions_and_user_rules_never_cover() {
        for extra in [
            json!({"enabled": false}),
            json!({"update_type": "delete"}),
            json!({"consuming_security_principals": [{"href": "/orgs/5/security_principals/1"}]}),
            json!({"machine_auth": true}),
            json!({"resolve_labels_as": {"providers": ["virtual_services"], "consumers": ["workloads"]}}),
            json!({"consumers": [{"actors": "ams"}, {"label": {"href": "/orgs/5/labels/1"}, "exclusion": true}]}),
        ] {
            assert_eq!(
                classify_in(rule(extra.clone()), &intra()),
                RuleAction::Create,
                "{extra}"
            );
        }
    }

    #[test]
    fn ip_list_consumers_match_by_href_and_a_new_list_never_exists() {
        let corp = GenActor::Ref(ActorRef::new(
            ActorKind::IpList,
            "/orgs/5/sec_policy/draft/ip_lists/21",
        ));
        let mine = proposed(
            vec![GenActor::all()],
            vec![corp],
            false,
            vec![port("443/tcp")],
        );
        let r = rule(json!({
            "consumers": [{"ip_list": {"href": "/orgs/5/sec_policy/active/ip_lists/21"}}],
            "ingress_services": [{"port": 443, "proto": 6}]
        }));
        assert_eq!(
            classify_in(r.clone(), &mine),
            exists(),
            "an active href is its draft"
        );
        let new = proposed(
            vec![GenActor::all()],
            vec![GenActor::NewIpList(0)],
            false,
            vec![port("443/tcp")],
        );
        assert_eq!(classify_in(r, &new), RuleAction::Create);
    }

    #[test]
    fn a_ruleset_to_create_has_nothing_to_dedupe() {
        let d = drafts(json!([rule(json!({}))]));
        let t = Target::Create(crate::write::RuleSetDraft {
            name: "RF-shop-prod".into(),
            description: None,
            enabled: true,
            scopes: vec![],
        });
        assert_eq!(
            classify(&intra(), &t, &d, &rich_index()),
            RuleAction::Create
        );
    }

    #[test]
    fn refresh_reclassifies_against_new_drafts() {
        let mut p = Proposal {
            scopes: vec![TargetScope {
                values: vec!["shop".into(), "prod".into()],
                labels: vec![
                    ResolvedLabel {
                        href: "/orgs/5/labels/3".into(),
                        key: "app".into(),
                        value: "shop".into(),
                    },
                    ResolvedLabel {
                        href: "/orgs/5/labels/4".into(),
                        key: "env".into(),
                        value: "prod".into(),
                    },
                ],
            }],
            targets: Vec::new(),
            ip_lists: Vec::new(),
            rules: vec![intra()],
            coverage: Coverage::default(),
        };
        let idx = rich_index();
        p.refresh(&drafts(json!([])), &idx, "RF-{app}-{env}", None);
        assert_eq!(p.rules[0].action, RuleAction::Create);
        assert!(p.writes());
        p.refresh(
            &drafts(json!([rule(json!({}))])),
            &idx,
            "RF-{app}-{env}",
            None,
        );
        assert_eq!(p.rules[0].action, exists());
        assert!(!p.writes());
        assert_eq!(p.counts().exists, 1);
    }

    /// Review focus 1: the recorded demo PCE's scoped ruleset `14. pos |
    /// pci` holds an IP list consumer with `unscoped_consumers: false`,
    /// as the generator proposes for an IP peer.
    #[test]
    fn recorded_scoped_rulesets_name_ip_list_consumers_in_scope() {
        let sets: Vec<RuleSet> =
            serde_json::from_str(include_str!("../../../../fixtures/recorded/rule_sets.json"))
                .unwrap();
        let scoped: Vec<&RuleSet> = sets
            .iter()
            .filter(|r| r.scopes.iter().any(|s| !s.is_empty()))
            .collect();
        let ipl: Vec<&Rule> = scoped
            .iter()
            .flat_map(|r| &r.rules)
            .filter(|r| r.consumers.iter().any(|a| a.ip_list.is_some()))
            .collect();
        assert!(!ipl.is_empty());
        assert!(ipl.iter().all(|r| r.unscoped_consumers == Some(false)));
    }
}
```

In `crates/pce/src/generate/mod.rs`, declare it:

```rust
pub mod rows;
```

with:

```rust
pub mod dedupe;
pub mod rows;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p pce --lib generate::dedupe`
Expected: FAIL to compile: "cannot find function `classify`", "no method named `refresh`".

- [ ] **Step 3: Write the implementation**

Above the tests in `crates/pce/src/generate/dedupe.rs`, add:

```rust
//! A proposed rule against its target ruleset's draft rules: already
//! covered (`Exists`), the same actors lacking services (`AddServices`),
//! or new (`Create`). A false "covered" silently drops a needed rule, so
//! every test here errs towards `Create`.
//!
//! Sides are compared as sets with Illumio's semantics: a rule's labels
//! AND across keys and OR within one key; `ams`, IP lists and workloads
//! add to that. An existing side covers a proposed one when it holds
//! every workload the proposed one names.
use super::{GenActor, GenRule, RuleAction, Target, actor_key};
use crate::models::{Actor, IngressService, Rule, RuleSet};
use crate::resolve::{NameIndex, canon};
use crate::write::{ActorKind, ActorRef, ServiceRef};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn classify(
    rule: &GenRule,
    target: &Target,
    drafts: &[RuleSet],
    idx: &NameIndex,
) -> RuleAction {
    let Target::Existing { href, .. } = target else {
        return RuleAction::Create;
    };
    let Some(rs) = drafts.iter().find(|r| &r.href == href) else {
        return RuleAction::Create;
    };
    let live: Vec<&Rule> = rs.rules.iter().filter(|r| eligible(r)).collect();
    if let Some(r) = live.iter().find(|r| covers(r, rule, idx)) {
        return RuleAction::Exists {
            rule_href: r.href.clone(),
        };
    }
    for r in &live {
        if same_actors(r, rule) {
            let add: Vec<ServiceRef> = rule
                .services
                .iter()
                .filter(|s| !service_covered(&r.ingress_services, s, idx))
                .cloned()
                .collect();
            if !add.is_empty() {
                return RuleAction::AddServices {
                    rule_href: r.href.clone(),
                    add,
                };
            }
        }
    }
    RuleAction::Create
}

/// A rule that can cover anything: enabled, not pending deletion, for
/// workloads, not narrowed by users, machine authentication or network.
fn eligible(r: &Rule) -> bool {
    let workloads = |v: &[String]| v.iter().any(|x| x == "workloads");
    r.enabled
        && r.update_type.as_deref() != Some("delete")
        && r.resolve_labels_as
            .as_ref()
            .is_none_or(|x| workloads(&x.providers) && workloads(&x.consumers))
        && r.machine_auth != Some(true)
        && r.network_type
            .as_deref()
            .is_none_or(|n| matches!(n, "brn" | "all"))
        && r.extra
            .get("consuming_security_principals")
            .and_then(|v| v.as_array())
            .is_none_or(Vec::is_empty)
}

fn covers(existing: &Rule, rule: &GenRule, idx: &NameIndex) -> bool {
    existing.unscoped_consumers.unwrap_or(false) == rule.unscoped_consumers
        && side_covers(&existing.providers, &rule.providers, true, idx)
        && side_covers(
            &existing.consumers,
            &rule.consumers,
            !rule.unscoped_consumers,
            idx,
        )
        && rule
            .services
            .iter()
            .all(|s| service_covered(&existing.ingress_services, s, idx))
}

/// Whether `existing` holds every workload `proposed` names. `in_scope`:
/// the side resolves within the ruleset's scope, so `ams` there covers
/// any workload actor.
fn side_covers(existing: &[Actor], proposed: &[GenActor], in_scope: bool, idx: &NameIndex) -> bool {
    if existing.iter().any(|a| a.exclusion == Some(true)) {
        return false;
    }
    let ams = existing.iter().any(|a| a.actors.as_deref() == Some("ams"));
    let has_ref = |kind: &str, href: &str| {
        existing.iter().any(|a| {
            let r = match kind {
                "ip_list" => a.ip_list.as_ref(),
                _ => a.workload.as_ref(),
            };
            r.is_some_and(|r| canon(&r.href) == canon(href))
        })
    };
    let mut labels: BTreeMap<String, String> = BTreeMap::new();
    for p in proposed {
        let GenActor::Ref(a) = p else {
            return false; // an IP list that does not exist yet
        };
        let href = a.href.as_deref().unwrap_or_default();
        match a.kind {
            ActorKind::All if !ams => return false,
            ActorKind::All => {}
            ActorKind::IpList if !has_ref("ip_list", href) => return false,
            ActorKind::IpList => {}
            ActorKind::Workload if !(has_ref("workload", href) || (ams && in_scope)) => {
                return false;
            }
            ActorKind::Workload => {}
            ActorKind::Label => match idx.label(href) {
                Some(l) => {
                    labels.insert(l.key.clone(), href.to_string());
                }
                None => return false,
            },
            ActorKind::LabelGroup => return false, // never proposed
        }
    }
    if labels.is_empty() || (ams && in_scope) {
        return true;
    }
    match label_constraint(existing, idx) {
        Some(c) if !c.is_empty() => c
            .iter()
            .all(|(k, allowed)| labels.get(k).is_some_and(|h| allowed.contains(h))),
        _ => false,
    }
}

/// The existing side's labels and label groups by key: the hrefs each key
/// allows. `None` when one cannot be resolved (then nothing is proven).
fn label_constraint(
    existing: &[Actor],
    idx: &NameIndex,
) -> Option<BTreeMap<String, BTreeSet<String>>> {
    let mut c: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for a in existing {
        if let Some(l) = &a.label {
            let key = idx.label(&l.href)?.key.clone();
            c.entry(key).or_default().insert(l.href.clone());
        } else if let Some(g) = &a.label_group {
            let key = idx.label_group(&g.href)?.key.clone();
            c.entry(key)
                .or_default()
                .extend(crate::impact::group_members(&g.href, idx));
        }
    }
    Some(c)
}

/// The same actors on both sides and the same flag: the rule a service
/// can be added to.
fn same_actors(existing: &Rule, rule: &GenRule) -> bool {
    let keys = |actors: &[Actor]| -> Option<BTreeSet<String>> {
        actors
            .iter()
            .map(|a| ActorRef::from_actor(a).ok().map(|r| actor_key(&r)))
            .collect()
    };
    let mine = |actors: &[GenActor]| -> BTreeSet<String> {
        actors.iter().map(GenActor::sort_key).collect()
    };
    existing.unscoped_consumers.unwrap_or(false) == rule.unscoped_consumers
        && keys(&existing.providers) == Some(mine(&rule.providers))
        && keys(&existing.consumers) == Some(mine(&rule.consumers))
}

/// One port entry: a protocol (`-1` all), a port range (`None` all ports).
#[derive(Debug, Clone, Copy)]
struct Entry {
    proto: i64,
    from: Option<u16>,
    to: Option<u16>,
}

impl Entry {
    fn covers(&self, p: &Entry) -> bool {
        if self.proto == -1 {
            return true;
        }
        if self.proto != p.proto {
            return false;
        }
        let Some(from) = self.from else {
            return true;
        };
        let Some(pf) = p.from else {
            return false;
        };
        from <= pf && p.to.unwrap_or(pf) <= self.to.unwrap_or(from)
    }
}

/// A service object's entries; `None` when unknown or narrowed by an ICMP
/// type (only its own href covers it then).
fn object_entries(href: &str, idx: &NameIndex) -> Option<Vec<Entry>> {
    let s = idx.service(href)?;
    s.service_ports
        .iter()
        .map(|p| {
            if p.icmp_type.is_some() {
                return None;
            }
            Some(Entry {
                proto: p.proto?,
                from: p.port,
                to: p.to_port,
            })
        })
        .collect::<Option<Vec<Entry>>>()
        .filter(|v| !v.is_empty())
}

/// Whether the existing services cover `s`: the same object, or entries
/// that hold every port `s` stands for (All Services holds all).
fn service_covered(existing: &[IngressService], s: &ServiceRef, idx: &NameIndex) -> bool {
    let mut hrefs = BTreeSet::new();
    let mut entries = Vec::new();
    for e in existing {
        match e {
            IngressService::Ref(r) => {
                hrefs.insert(canon(&r.href).into_owned());
                entries.extend(object_entries(&r.href, idx).unwrap_or_default());
            }
            IngressService::Port {
                port,
                to_port,
                proto,
            } => entries.push(Entry {
                proto: *proto,
                from: *port,
                to: *to_port,
            }),
        }
    }
    let wanted = match s {
        ServiceRef::Href(h) if hrefs.contains(canon(h).as_ref()) => return true,
        ServiceRef::Href(h) => match object_entries(h, idx) {
            Some(v) => v,
            None => return false,
        },
        ServiceRef::Port(p) => vec![Entry {
            proto: p.proto,
            from: p.port,
            to: p.to_port,
        }],
    };
    wanted.iter().all(|w| entries.iter().any(|e| e.covers(w)))
}

```

In `crates/pce/src/generate/mod.rs`, after `Proposal::blocker`, add `refresh`:

```rust
                Some(Target::None(why)) => Some(why.clone()),
                _ => None,
            })
    }
```

with:

```rust
                Some(Target::None(why)) => Some(why.clone()),
                _ => None,
            })
    }

    /// Targets and actions again against `drafts`: a fresh draft read at
    /// plan time, or the rulesets just loaded. `picked`: the app ruleset
    /// `R` chose among several.
    pub fn refresh(
        &mut self,
        drafts: &[crate::models::RuleSet],
        idx: &NameIndex,
        template: &str,
        picked: Option<&str>,
    ) {
        self.targets = self
            .scopes
            .iter()
            .enumerate()
            .map(|(i, s)| target::resolve(s, drafts, idx, template, picked.filter(|_| i == 0)))
            .collect();
        for r in &mut self.rules {
            r.action = dedupe::classify(r, &self.targets[r.target], drafts, idx);
        }
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p pce --lib generate::dedupe`
Expected: PASS, `recorded_scoped_rulesets_name_ip_list_consumers_in_scope` among them.

- [ ] **Step 5: Commit**

```bash
git add crates/pce/src/generate
git commit -m "pce::generate: dedupe against the draft rules, Proposal::refresh

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

### Task 5: `pce::generate::derive`: `propose`

**Files:**
- Create: `crates/pce/src/generate/derive.rs`
- Modify: `crates/pce/src/generate/mod.rs`

**Interfaces:**
- Consumes: Tasks 2-4 (`rows`, `row_key`, `side_of`, `excluded`, `ip_list_name`, `express`, `flow_spec`, `fold_ports`, `Proposal::refresh`); `NameIndex::all_services_href`, `label_href`, `ip_list_href`, `service_name`; `IpRangeSpec::parse`.
- Produces (Plans 2 and 3 call these by these names):

```rust
pub const DESCRIPTION_MAX: usize = 255;
#[derive(Debug, Clone, Copy)]
pub struct GenInput<'a> {
    pub flows: &'a [ResolvedFlow], pub focus: &'a Focus, pub tier_key: &'a str,
    pub drafts: &'a [RuleSet], pub idx: &'a NameIndex, pub ruleset_name: &'a str,
    pub window: &'a str, pub outbound: bool, pub picked: Option<&'a str>,
}
pub fn propose(rows: &[GenRow], global: Stop, input: &GenInput) -> Proposal;
```

`propose` keeps target 0 for the app's own scope; an outbound app peer adds its scope as a further target, created when no draft ruleset covers it. Rules are ordered by target, then by the first row behind them.

- [ ] **Step 1: Write the failing tests**

Create `crates/pce/src/generate/derive.rs` with its tests only (the code follows in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::super::rows::rows;
    use super::super::rows::tests::{intra_on, shop, to_ip};
    use super::super::{DEFAULT_RULESET_NAME, Target};
    use super::*;
    use crate::traffic::resolved::tests::{rich_flows, rich_index};

    fn sets() -> Vec<RuleSet> {
        serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap()
    }

    /// `propose` on the rich fixture for shop/prod, the outbound section
    /// on or off, against the fixture rulesets.
    fn run(
        flows: &[ResolvedFlow],
        stop: Stop,
        outbound: bool,
        edit: impl Fn(&mut Vec<GenRow>),
    ) -> (Vec<GenRow>, Proposal) {
        let idx = rich_index();
        let focus = shop();
        let mut r = rows(flows, &focus, "role", &idx);
        edit(&mut r);
        let drafts = sets();
        let input = GenInput {
            flows,
            focus: &focus,
            tier_key: "role",
            drafts: &drafts,
            idx: &idx,
            ruleset_name: DEFAULT_RULESET_NAME,
            window: "24h",
            outbound,
            picked: None,
        };
        let p = propose(&r, stop, &input);
        (r, p)
    }

    /// Each rule as `providers -> consumers unscoped services`, by href.
    fn shape(p: &Proposal) -> Vec<String> {
        let idx = rich_index();
        p.rules
            .iter()
            .map(|r| {
                let side = |a: &[GenActor]| {
                    a.iter()
                        .map(|x| x.text(p, &idx))
                        .collect::<Vec<_>>()
                        .join("+")
                };
                let svc: Vec<String> = r.services.iter().map(ServiceRef::display).collect();
                format!(
                    "t{} {} <- {}{} {}",
                    r.target,
                    side(&r.providers),
                    side(&r.consumers),
                    if r.unscoped_consumers {
                        " unscoped"
                    } else {
                        ""
                    },
                    svc.join(",")
                )
            })
            .collect()
    }

    #[test]
    fn app_ports_proposes_intra_and_the_same_env_peer_into_the_app_ruleset() {
        let (_, p) = run(&rich_flows(), Stop::AppPorts, false, |_| {});
        assert_eq!(
            shape(&p),
            vec![
                "t0 all workloads <- all workloads 5432/tcp",
                "t0 all workloads <- app=hr+env=prod unscoped 5432/tcp",
            ]
        );
        assert_eq!(
            p.coverage,
            Coverage {
                covered: 5,
                uncovered: 3,
                unchecked: 3,
                ..Coverage::default()
            }
        );
        assert_eq!(p.rules[0].rows, vec![0]);
        assert_eq!(
            p.rules[1].description,
            "generated: hr/prod → shop/prod on 5432/tcp, from 24h of traffic (illumio-tui)"
        );
        assert!(p.ip_lists.is_empty());
    }

    #[test]
    fn role_ports_splits_rows_by_role_pair() {
        let (_, p) = run(&rich_flows(), Stop::RolePorts, false, |r| {
            r[3].checked = true
        });
        assert_eq!(
            shape(&p),
            vec![
                "t0 role=db <- role=web 5432/tcp",
                "t0 role=db <- app=hr+env=prod+role=web unscoped 5432/tcp",
                "t0 role=web <- ipl:Corp 443/tcp",
                "t0 role=db <- ipl:Corp 5432/tcp",
            ]
        );
    }

    #[test]
    fn all_stops_take_the_all_services_object() {
        let (_, p) = run(&rich_flows(), Stop::AppAll, false, |_| {});
        assert!(
            p.rules.iter().all(|r| r.services
                == vec![ServiceRef::Href(
                    "/orgs/5/sec_policy/draft/services/30".into()
                )]),
            "{:?}",
            shape(&p)
        );
        assert!(p.rules[0].description.contains("on all services"));
    }

    #[test]
    fn a_row_override_beats_the_slider() {
        let (_, p) = run(&rich_flows(), Stop::AppPorts, false, |r| {
            r[0].stop = Some(Stop::RolePorts)
        });
        assert_eq!(shape(&p)[0], "t0 role=db <- role=web 5432/tcp");
        assert_eq!(
            shape(&p)[1],
            "t0 all workloads <- app=hr+env=prod unscoped 5432/tcp"
        );
    }

    #[test]
    fn rows_with_the_same_actors_merge_into_one_rule() {
        let mut f = rich_flows();
        let mut redis = f[12].clone(); // hr/prod -> shop/prod, another port
        redis.port = Some(6379);
        f.push(redis);
        let mut other = f[17].clone(); // a second Corp address -> shop web
        other.src.ip = Some("10.1.4.10".into());
        f.push(other);
        let (r, p) = run(&f, Stop::AppPorts, false, |r| {
            for row in r.iter_mut() {
                if matches!(row.peer, PeerKind::Ip { .. }) {
                    row.checked = true;
                }
            }
        });
        assert_eq!(r[3].peer_text(), "10.1.4.9");
        assert_eq!(r[4].peer_text(), "10.1.4.10");
        assert_eq!(
            shape(&p),
            vec![
                "t0 all workloads <- all workloads 5432/tcp",
                "t0 all workloads <- app=hr+env=prod unscoped 5432/tcp,6379/tcp",
                "t0 all workloads <- ipl:Corp 443/tcp,5432/tcp",
            ]
        );
        assert_eq!(p.rules[2].rows, vec![3, 4], "two IP peers, one rule");
        assert_eq!(
            p.rules[2].description,
            "generated: 10.1.4.10, 10.1.4.9 → shop/prod on 443/tcp, 5432/tcp, from 24h of traffic (illumio-tui)"
        );
    }

    #[test]
    fn outbound_rules_go_into_the_peer_ruleset_or_name_a_new_ip_list() {
        let (_, p) = run(&rich_flows(), Stop::AppPorts, true, |r| r[5].checked = true);
        assert_eq!(
            shape(&p)[2..],
            [
                "t0 new ipl:shop-prod peer 203.0.113.7 <- all workloads 443/tcp".to_string(),
                "t1 all workloads <- app=shop+env=prod unscoped 443/tcp".to_string(),
            ]
        );
        assert_eq!(p.scopes[1].text(), "crm/prod");
        let Target::Create(d) = &p.targets[1] else {
            panic!("no ruleset covers crm/prod: {:?}", p.targets[1])
        };
        assert_eq!(d.name, "RF-crm-prod");
        assert_eq!(p.ip_lists.len(), 1);
        assert_eq!(p.ip_lists[0].name, "shop-prod peer 203.0.113.7");
        assert_eq!(p.ip_lists[0].ranges[0].from_ip, "203.0.113.7");
        assert_eq!(p.coverage.covered, 7, "5 inbound, 2 outbound");
        // The batch creates the IP list (step 0), then the ruleset (step 1).
        assert_eq!(p.ruleset_step(1), Some(1));
        assert_eq!(p.ruleset_step(0), None, "the app's ruleset exists");
        let new_list = p
            .rules
            .iter()
            .find(|r| r.providers[0] == GenActor::NewIpList(0))
            .unwrap();
        assert_eq!(
            new_list.to_draft(&p).providers[0].href.as_deref(),
            Some("{step:0}")
        );
        let crm = p.rules.iter().find(|r| r.target == 1).unwrap();
        assert_eq!(crm.ruleset_ref(&p).as_deref(), Some("{step:1}"));
        assert_eq!(
            crm.body(&p)["consumers"],
            serde_json::json!([{"label": {"href": "/orgs/5/labels/3"}}, {"label": {"href": "/orgs/5/labels/4"}}])
        );
    }

    #[test]
    fn coverage_counts_every_kept_flow_once_by_reason() {
        let mut f = rich_flows();
        f.push(to_ip(22, "224.0.0.251")); // outbound multicast
        let mut icmp = intra_on(0);
        icmp.port = None;
        icmp.proto = "icmp".into();
        f.push(icmp); // intra, no ICMP service in the fixture
        let (_, p) = run(&f, Stop::AppPorts, true, |_| {});
        assert_eq!(
            p.coverage,
            Coverage {
                covered: 6,
                uncovered: 6,
                unchecked: 4,
                folded_ips: 0,
                unsupported_proto: 1,
                broadcast_multicast: 1,
            }
        );
        let (_, p) = run(&f, Stop::AppPorts, false, |_| {});
        assert_eq!(
            p.coverage.broadcast_multicast, 0,
            "outbound off: not counted"
        );
    }

    #[test]
    fn a_long_service_list_is_cut_in_the_description() {
        let mut f = rich_flows();
        f.extend((0..60u16).map(|i| intra_on(10_000 + 2 * i)));
        let (_, p) = run(&f, Stop::AppPorts, false, |_| {});
        let d = &p.rules[0].description;
        assert!(d.chars().count() <= DESCRIPTION_MAX, "{d}");
        assert!(d.contains(" +"), "{d}");
        assert!(d.ends_with("from 24h of traffic (illumio-tui)"), "{d}");
    }
}
```

In `crates/pce/src/generate/mod.rs`, declare it:

```rust
pub mod dedupe;
pub mod rows;
```

with:

```rust
pub mod dedupe;
pub mod derive;
pub mod rows;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p pce --lib generate::derive`
Expected: FAIL to compile: "cannot find function `propose`".

- [ ] **Step 3: Write the implementation**

Above the tests in `crates/pce/src/generate/derive.rs`, add:

```rust
//! Rows and a stop to rules: providers, consumers and `unscoped_consumers`
//! by the derivation tables, one rule per (target, providers, consumers,
//! flag) with the services of every flow behind it, the coverage of the
//! flows, the IP lists to create. Pure.
use super::rows::{excluded, ip_list_name, row_key, side_of};
use super::services::{Expr, express, flow_spec, fold_ports};
use super::{
    Coverage, GenActor, GenRow, GenRule, PeerKey, PeerKind, Proposal, RowKey, RuleAction, Side,
    Stop, TargetScope,
};
use crate::models::RuleSet;
use crate::resolve::{NameIndex, ResolvedLabel};
use crate::traffic::{Focus, ResolvedEndpoint, ResolvedFlow};
use crate::write::{ActorKind, ActorRef, IpListDraft, IpRangeSpec, PortSpec, ServiceRef};
use std::collections::{BTreeMap, BTreeSet};

/// A rule description's longest form; the service list is cut with `+N`.
pub const DESCRIPTION_MAX: usize = 255;

/// What `propose` reads besides the rows and the stop. Nothing here is
/// fetched: the caller passes what is loaded.
#[derive(Debug, Clone, Copy)]
pub struct GenInput<'a> {
    /// The app's flows, one per record (`TrafficState::flows_for`).
    pub flows: &'a [ResolvedFlow],
    pub focus: &'a Focus,
    /// The profile's tier key (`role`).
    pub tier_key: &'a str,
    /// The draft rulesets the rules are deduplicated against.
    pub drafts: &'a [RuleSet],
    pub idx: &'a NameIndex,
    /// `generator.ruleset_name`, for a ruleset to create.
    pub ruleset_name: &'a str,
    /// The traffic window (`24h`), for the descriptions.
    pub window: &'a str,
    /// The outbound section is on (`o`, `--outbound`).
    pub outbound: bool,
    /// The app ruleset `R` picked among several.
    pub picked: Option<&'a str>,
}

/// Row index by key; IP flows of folded peers find the folded row.
fn row_index(rows: &[GenRow]) -> BTreeMap<RowKey, usize> {
    rows.iter().enumerate().map(|(i, r)| (r.key(), i)).collect()
}

/// The rules `rows` (checkboxes and overrides as they are) need at the
/// slider's `global` stop, classified against `input.drafts`.
pub fn propose(rows: &[GenRow], global: Stop, input: &GenInput) -> Proposal {
    let mut p = derive(rows, global, input);
    p.refresh(input.drafts, input.idx, input.ruleset_name, input.picked);
    p
}

/// One rule being built.
struct Group {
    target: usize,
    providers: Vec<GenActor>,
    consumers: Vec<GenActor>,
    unscoped: bool,
    inline: BTreeSet<PortSpec>,
    objects: BTreeSet<String>,
    rows: BTreeSet<usize>,
    from: BTreeSet<String>,
    to: BTreeSet<String>,
}

fn keys_of(a: &[GenActor]) -> Vec<String> {
    let mut k: Vec<String> = a.iter().map(GenActor::sort_key).collect();
    k.sort();
    k
}

fn label_actor(l: &ResolvedLabel) -> GenActor {
    GenActor::label(&l.href)
}

/// The endpoint's labels under `keys`, in key order.
fn scope_labels(ep: &ResolvedEndpoint, keys: &[String]) -> Vec<ResolvedLabel> {
    keys.iter()
        .filter_map(|k| ep.labels.iter().find(|l| &l.key == k).cloned())
        .collect()
}

fn role(ep: &ResolvedEndpoint, tier_key: &str) -> Option<GenActor> {
    ep.labels
        .iter()
        .find(|l| l.key == tier_key)
        .map(label_actor)
}

/// The app's scope labels from the index, else from one of its flows.
fn app_scope(input: &GenInput) -> TargetScope {
    let f = input.focus;
    let from_index: Option<Vec<ResolvedLabel>> = f
        .keys
        .iter()
        .zip(&f.app)
        .map(|(k, v)| {
            input.idx.label_href(k, v).map(|h| ResolvedLabel {
                href: h.to_string(),
                key: k.clone(),
                value: v.clone(),
            })
        })
        .collect();
    let labels = from_index.unwrap_or_else(|| {
        input
            .flows
            .iter()
            .find_map(|x| match side_of(x, f)? {
                Side::Inbound => Some(scope_labels(&x.dst, &f.keys)),
                Side::Outbound => Some(scope_labels(&x.src, &f.keys)),
            })
            .unwrap_or_default()
    });
    TargetScope {
        values: f.app.clone(),
        labels,
    }
}

struct Ctx<'a> {
    input: &'a GenInput<'a>,
    scopes: Vec<TargetScope>,
    ip_lists: Vec<IpListDraft>,
}

impl Ctx<'_> {
    /// The target index of a peer's scope, added on first use.
    fn peer_target(&mut self, values: &[String], ep: &ResolvedEndpoint) -> usize {
        if let Some(i) = self.scopes.iter().position(|s| s.values == values) {
            return i;
        }
        self.scopes.push(TargetScope {
            values: values.to_vec(),
            labels: scope_labels(ep, &self.input.focus.keys),
        });
        self.scopes.len() - 1
    }

    /// The IP list actor of an IP row: the existing list, else the new one.
    fn ip_actor(&mut self, ip: &str, list: Option<&str>) -> GenActor {
        if let Some(h) = list.and_then(|n| self.input.idx.ip_list_href(n)) {
            return GenActor::Ref(ActorRef::new(ActorKind::IpList, h));
        }
        let name = ip_list_name(&self.input.focus.app, ip);
        if let Some(i) = self.ip_lists.iter().position(|d| d.name == name) {
            return GenActor::NewIpList(i);
        }
        let Ok(range) = IpRangeSpec::parse(ip, false) else {
            // `rows` makes such a row uncheckable; never reached.
            return GenActor::NewIpList(usize::MAX);
        };
        self.ip_lists.push(IpListDraft {
            name,
            description: None,
            ranges: vec![range],
            fqdns: Vec::new(),
        });
        GenActor::NewIpList(self.ip_lists.len() - 1)
    }

    /// Target, providers, consumers and `unscoped_consumers` of one flow of
    /// `row` at `stop`, by the derivation tables; `None` for a row that
    /// proposes nothing.
    fn actors(
        &mut self,
        f: &ResolvedFlow,
        row: &GenRow,
        stop: Stop,
    ) -> Option<(usize, Vec<GenActor>, Vec<GenActor>, bool)> {
        let keys = self.input.focus.keys.clone();
        let tier = self.input.tier_key;
        let by_role = stop.is_role();
        // Providers sit in the target's scope: `ams` there, or the role.
        let provider = |ep: &ResolvedEndpoint| {
            vec![
                by_role
                    .then(|| role(ep, tier))
                    .flatten()
                    .unwrap_or_else(GenActor::all),
            ]
        };
        // A labelled side: its scope labels, plus its role at role stops.
        let side = |ep: &ResolvedEndpoint| {
            let mut v: Vec<GenActor> = scope_labels(ep, &keys).iter().map(label_actor).collect();
            if let Some(r) = by_role.then(|| role(ep, tier)).flatten() {
                v.push(r);
            }
            v
        };
        Some(match (row.side, &row.peer) {
            (Side::Inbound, PeerKind::Intra) => {
                let cons = by_role
                    .then(|| role(&f.src, tier))
                    .flatten()
                    .unwrap_or_else(GenActor::all);
                (0, provider(&f.dst), vec![cons], false)
            }
            (Side::Inbound, PeerKind::App(_)) => (0, provider(&f.dst), side(&f.src), true),
            (Side::Inbound, PeerKind::Ip { ip, list }) => {
                let cons = self.ip_actor(ip, list.as_deref());
                (0, provider(&f.dst), vec![cons], false)
            }
            (Side::Inbound, PeerKind::Workload { href, .. }) => (
                0,
                provider(&f.dst),
                vec![GenActor::Ref(ActorRef::new(ActorKind::Workload, href))],
                true,
            ),
            (Side::Outbound, PeerKind::App(values)) => {
                let t = self.peer_target(values, &f.dst);
                (t, provider(&f.dst), side(&f.src), true)
            }
            (Side::Outbound, PeerKind::Ip { ip, list }) => {
                let prov = self.ip_actor(ip, list.as_deref());
                let cons = by_role
                    .then(|| role(&f.src, tier))
                    .flatten()
                    .unwrap_or_else(GenActor::all);
                (0, vec![prov], vec![cons], false)
            }
            _ => return None,
        })
    }
}

/// The names a description gives a row's two ends: `(from, to)`.
fn ends(row: &GenRow, app: &str) -> (String, String) {
    let peer = match &row.peer {
        PeerKind::Intra => app.to_string(),
        _ => row.peer_text(),
    };
    match row.side {
        Side::Inbound => (peer, app.to_string()),
        Side::Outbound => (app.to_string(), peer),
    }
}

/// `items` joined with `, `, cut with `+N` once it would pass `budget`
/// characters (at least one item stays).
fn capped(items: &[String], budget: usize) -> String {
    for keep in (1..=items.len()).rev() {
        let mut s = items[..keep].join(", ");
        if keep < items.len() {
            s.push_str(&format!(" +{}", items.len() - keep));
        }
        if s.chars().count() <= budget || keep == 1 {
            return s;
        }
    }
    String::new()
}

fn service_text(s: &ServiceRef, all: Option<&str>, idx: &NameIndex) -> String {
    match s {
        ServiceRef::Port(p) => p.display(),
        ServiceRef::Href(h) if Some(h.as_str()) == all => "all services".into(),
        ServiceRef::Href(h) => idx.service_name(h).unwrap_or(h).to_string(),
    }
}

/// `generated: web/prod → shop/prod on 443/tcp, from 24h of traffic
/// (illumio-tui)`, at most [`DESCRIPTION_MAX`] characters.
fn description(from: &[String], to: &[String], services: &[String], window: &str) -> String {
    let head = |f: &str, t: &str, s: &str| {
        format!("generated: {f} → {t} on {s}, from {window} of traffic (illumio-tui)")
    };
    let f = capped(from, 60);
    let t = capped(to, 60);
    let fixed = head(&f, &t, "").chars().count();
    let s = capped(services, DESCRIPTION_MAX.saturating_sub(fixed));
    head(&f, &t, &s)
}

fn derive(rows: &[GenRow], global: Stop, input: &GenInput) -> Proposal {
    let focus = input.focus;
    let app = focus.app_text();
    let index = row_index(rows);
    let all_services = input.idx.all_services_href().map(str::to_string);
    let mut ctx = Ctx {
        input,
        scopes: vec![app_scope(input)],
        ip_lists: Vec::new(),
    };
    let mut groups: BTreeMap<(usize, Vec<String>, Vec<String>, bool), Group> = BTreeMap::new();
    let mut cov = Coverage::default();
    let in_force = |s: Side| s == Side::Inbound || input.outbound;
    for f in input.flows {
        if excluded(f) {
            if side_of(f, focus).is_some_and(in_force) {
                cov.broadcast_multicast += 1;
            }
            continue;
        }
        let Some(key) = row_key(f, focus) else {
            continue;
        };
        if !in_force(key.side) {
            continue;
        }
        let folded = RowKey {
            side: key.side,
            peer: PeerKey::Folded,
        };
        let Some(&ri) = index.get(&key).or_else(|| index.get(&folded)) else {
            continue;
        };
        let row = &rows[ri];
        if matches!(row.peer, PeerKind::FoldedIps(_)) {
            cov.folded_ips += 1;
            continue;
        }
        let mut stop = row.effective(global);
        if stop.is_all() && all_services.is_none() {
            stop = stop.ports();
        }
        let svc = match (&all_services, stop.is_all()) {
            (Some(h), true) if row.proposed() => Expr::Object(h.clone()),
            _ => express(&flow_spec(f), input.idx),
        };
        // A service nothing expresses is that, checked or not.
        if let Expr::Unsupported(_) = svc {
            cov.unsupported_proto += 1;
            continue;
        }
        if !row.proposed() {
            cov.unchecked += 1;
            continue;
        }
        let Some((target, providers, consumers, unscoped)) = ctx.actors(f, row, stop) else {
            cov.unchecked += 1;
            continue;
        };
        let k = (target, keys_of(&providers), keys_of(&consumers), unscoped);
        let g = groups.entry(k).or_insert_with(|| Group {
            target,
            providers,
            consumers,
            unscoped,
            inline: BTreeSet::new(),
            objects: BTreeSet::new(),
            rows: BTreeSet::new(),
            from: BTreeSet::new(),
            to: BTreeSet::new(),
        });
        match svc {
            Expr::Inline(p) => {
                g.inline.insert(p);
            }
            Expr::Object(h) => {
                g.objects.insert(h);
            }
            Expr::Unsupported(_) => {}
        }
        g.rows.insert(ri);
        let (from, to) = ends(row, &app);
        g.from.insert(from);
        g.to.insert(to);
        cov.covered += 1;
    }
    cov.uncovered =
        cov.unchecked + cov.folded_ips + cov.unsupported_proto + cov.broadcast_multicast;
    let mut groups: Vec<Group> = groups.into_values().collect();
    // The app's ruleset first, then by the first row behind the rule.
    groups.sort_by_key(|g| (g.target, g.rows.first().copied()));
    let rules = groups
        .into_iter()
        .map(|g| {
            let services: Vec<ServiceRef> = match &all_services {
                Some(h) if g.objects.contains(h) => vec![ServiceRef::Href(h.clone())],
                _ => fold_ports(&g.inline)
                    .into_iter()
                    .map(ServiceRef::Port)
                    .chain(g.objects.iter().cloned().map(ServiceRef::Href))
                    .collect(),
            };
            let names: Vec<String> = services
                .iter()
                .map(|s| service_text(s, all_services.as_deref(), input.idx))
                .collect();
            let from: Vec<String> = g.from.into_iter().collect();
            let to: Vec<String> = g.to.into_iter().collect();
            GenRule {
                target: g.target,
                providers: g.providers,
                consumers: g.consumers,
                unscoped_consumers: g.unscoped,
                description: description(&from, &to, &names, input.window),
                services,
                rows: g.rows.into_iter().collect(),
                action: RuleAction::Create,
            }
        })
        .collect();
    Proposal {
        scopes: ctx.scopes,
        targets: Vec::new(),
        ip_lists: ctx.ip_lists,
        rules,
        coverage: cov,
    }
}

```

In `crates/pce/src/generate/mod.rs`, re-export it:

```rust
pub use rows::{MANY_SERVICES, MAX_IP_ROWS, excluded, ip_list_name, is_any_ip_list, rows};
```

with:

```rust
pub use derive::{GenInput, propose};
pub use rows::{MANY_SERVICES, MAX_IP_ROWS, excluded, ip_list_name, is_any_ip_list, rows};
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p pce --lib generate`
Expected: PASS: every generate test.

- [ ] **Step 5: Run clippy**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/pce/src/generate
git commit -m "pce::generate: propose (rules by stop, merging, coverage, descriptions)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

### Task 6: `[profiles.<name>.generator] ruleset_name`

**Files:**
- Modify: `crates/illumio-tui/src/config.rs`

**Interfaces:**
- Consumes: Task 2 (`pce::generate::check_ruleset_name`, `default_ruleset_name`).
- Produces:

```rust
// crate::config
pub struct GeneratorConfig { pub ruleset_name: Option<String> }
pub struct Profile { /* … */ pub generator: Option<GeneratorConfig> }
pub struct ResolvedConfig { /* … */ pub generator_ruleset_name: String }   // checked at load
```

- [ ] **Step 1: Write the failing test**

In the test module of `crates/illumio-tui/src/config.rs`, before `profile_debug_redacts_secret`, replace:

```rust
        );
    }

    #[test]
    fn profile_debug_redacts_secret() {
        let mut p = Profile {
```

with:

```rust
        );
    }

    #[test]
    fn generator_ruleset_name_defaults_to_rf_and_names_a_bad_key() {
        let r = resolve(&file(), &Overrides::default(), &env(&[])).unwrap();
        assert_eq!(r.generator_ruleset_name, "RF-{app}-{env}");
        let with = |tail: &str| -> ConfigFile {
            toml::from_str(&format!(
                "[profiles.default]\nhost = \"h\"\norg_id = 1\napi_key = \"k\"\napi_secret = \"s\"\n{tail}"
            ))
            .unwrap()
        };
        let f = with("[profiles.default.generator]\nruleset_name = \"RS-{app}-{env}\"\n");
        let r = resolve(&f, &Overrides::default(), &env(&[])).unwrap();
        assert_eq!(r.generator_ruleset_name, "RS-{app}-{env}");
        let shown = format!("{:?}", f.profiles["default"]);
        assert!(shown.contains("RS-{app}-{env}"), "{shown}");
        let f = with("[profiles.default.generator]\nruleset_name = \"RF-{app}-{role}\"\n");
        let err = resolve(&f, &Overrides::default(), &env(&[]))
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "generator.ruleset_name \"RF-{app}-{role}\": {role} is not a scope key (app, env)"
        );
        let f = with("[profiles.default.labels]\nscope_keys = [\"site\"]\n");
        let r = resolve(&f, &Overrides::default(), &env(&[])).unwrap();
        assert_eq!(
            r.generator_ruleset_name, "RF-{site}",
            "the default follows the scope keys"
        );
    }

    #[test]
    fn profile_debug_redacts_secret() {
        let mut p = Profile {
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p illumio-tui --lib config`
Expected: FAIL to compile: "no field `generator_ruleset_name` on type `ResolvedConfig`".

- [ ] **Step 3: Write the implementation**

In `crates/illumio-tui/src/config.rs`, add the profile field:

```rust
    /// The label model rules are written in (`[profiles.<name>.labels]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labels: Option<LabelsConfig>,
    /// Unknown keys, round-tripped verbatim.
    #[serde(default, flatten)]
    pub extra: toml::Table,
```

with:

```rust
    /// The label model rules are written in (`[profiles.<name>.labels]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labels: Option<LabelsConfig>,
    /// The policy generator's settings (`[profiles.<name>.generator]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<GeneratorConfig>,
    /// Unknown keys, round-tripped verbatim.
    #[serde(default, flatten)]
    pub extra: toml::Table,
```

In `crates/illumio-tui/src/config.rs`, replace:

```rust
            .field("writes", &self.writes)
            .field("limits", &self.limits)
            .field("labels", &self.labels)
            // Unknown keys could be anything; print their names only.
            .field("extra", &self.extra.keys().collect::<Vec<_>>())
            .finish()
```

with:

```rust
            .field("writes", &self.writes)
            .field("limits", &self.limits)
            .field("labels", &self.labels)
            .field("generator", &self.generator)
            // Unknown keys could be anything; print their names only.
            .field("extra", &self.extra.keys().collect::<Vec<_>>())
            .finish()
```

In `crates/illumio-tui/src/config.rs`, replace:

```rust
    pub ignore_keys: Option<Vec<String>>,
}

pub const DEFAULT_SCOPE_KEYS: [&str; 2] = ["app", "env"];
pub const DEFAULT_TIER_KEY: &str = "role";

```

with:

```rust
    pub ignore_keys: Option<Vec<String>>,
}

/// The policy generator: `ruleset_name` names a ruleset it creates, a
/// template over the scope keys (`RF-{app}-{env}` → `RF-shop-prod`).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct GeneratorConfig {
    pub ruleset_name: Option<String>,
}

pub const DEFAULT_SCOPE_KEYS: [&str; 2] = ["app", "env"];
pub const DEFAULT_TIER_KEY: &str = "role";

```

In `crates/illumio-tui/src/config.rs`, replace:

```rust
    pub tier_key: String,
    /// Label keys hidden from label text; never a scope key or the tier key.
    pub ignore_keys: Vec<String>,
}

impl ResolvedConfig {
```

with:

```rust
    pub tier_key: String,
    /// Label keys hidden from label text; never a scope key or the tier key.
    pub ignore_keys: Vec<String>,
    /// `generator.ruleset_name`, checked against the scope keys; default
    /// `RF-` and every scope key (`RF-{app}-{env}`).
    pub generator_ruleset_name: String,
}

impl ResolvedConfig {
```

In `crates/illumio-tui/src/config.rs`, replace:

```rust
        }
    }

    Ok(ResolvedConfig {
        profile_name,
        scheme,
```

with:

```rust
        }
    }

    let generator_ruleset_name = match profile
        .generator
        .as_ref()
        .and_then(|g| g.ruleset_name.clone())
    {
        Some(t) => {
            pce::generate::check_ruleset_name(&t, &scope_keys).map_err(ConfigError::Message)?;
            t
        }
        None => pce::generate::default_ruleset_name(&scope_keys),
    };

    Ok(ResolvedConfig {
        profile_name,
        scheme,
```

In `crates/illumio-tui/src/config.rs`, replace:

```rust
        scope_keys,
        tier_key,
        ignore_keys: ignored,
    })
}

```

with:

```rust
        scope_keys,
        tier_key,
        ignore_keys: ignored,
        generator_ruleset_name,
    })
}

```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p illumio-tui --lib config`
Expected: PASS (every config test).

- [ ] **Step 5: Commit**

```bash
git add crates/illumio-tui/src/config.rs
git commit -m "config: [profiles.<name>.generator] ruleset_name, checked at load, default RF-{app}-{env}

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

### Task 7: `illumio-tui gen <app>` (read-only)

**Files:**
- Create: `crates/illumio-tui/src/cli/generate.rs`
- Modify: `crates/illumio-tui/tests/cli.rs`
- Modify: `crates/illumio-tui/src/cli/error.rs`
- Modify: `crates/illumio-tui/src/cli/mod.rs`

**Interfaces:**
- Consumes: Tasks 2-6 (`rows`, `propose`, `GenInput`, `keys_refusal`, `Stop`, `Proposal::counts`, `GenRule::body`, `GenActor::text`, `Target::name`/`action`, `ResolvedConfig::generator_ruleset_name`); `cli::traffic::{cli_focus, fetch, FilterArgs, ByArg, DirectionArg}`; `Fetched::meta`; `Context::emit_meta`; `cache_label`.
- Produces: `cli::generate::{GenArgs, GranularityArg, GenReport, run}`; `Command::Gen`; `CliError::Incomplete(String)` (exit 4, kind `incomplete`).

Requests: the focused traffic query (`in`; `out` too with `--outbound`), through `fetch` and its cost guard (`--yes` above 7 days or a cap above 10000); the name index and the draft rulesets, cached (`--no-cache` bypasses them). Nothing is written.

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/tests/cli.rs`, `every_flag_has_help_text` covers the new command:

```rust
        vec!["traffic", "groups", "--help"],
        vec!["traffic", "mesh", "--help"],
        vec!["app", "--help"],
        vec!["policy", "pending", "--help"],
        vec!["policy", "rulesets", "--help"],
        vec!["policy", "rules", "--help"],
```

with:

```rust
        vec!["traffic", "groups", "--help"],
        vec!["traffic", "mesh", "--help"],
        vec!["app", "--help"],
        vec!["gen", "--help"],
        vec!["policy", "pending", "--help"],
        vec!["policy", "rulesets", "--help"],
        vec!["policy", "rules", "--help"],
```

Append to `crates/illumio-tui/tests/cli.rs`:

```rust

/// The traffic (any POST answers the 4-flow fixture), the object lists and
/// the draft rulesets: what `gen` reads.
/// `runs` commands, `posts` traffic queries each.
async fn mount_gen(server: &MockServer, runs: u64, posts: u64) {
    mount_traffic(server, runs * posts).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/rule_sets.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(runs)
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn gen_prints_rows_rules_and_the_preview_and_writes_nothing() {
    let server = MockServer::start().await;
    mount_gen(&server, 1, 1).await;
    let out = stdout_json(
        cmd(&server)
            .args(["gen", "shop/prod", "--window", "1h"])
            .assert()
            .success(),
    );
    let d = &out["data"];
    assert_eq!(d["app"], "shop/prod");
    assert_eq!(d["keys"], json!(["app", "env"]));
    assert_eq!(d["granularity"], "app-ports");
    assert_eq!(d["outbound"], false);
    assert_eq!(
        d["target"],
        json!({"name": "Shop-Prod", "href": "/orgs/5/sec_policy/draft/rule_sets/100", "action": "existing"})
    );
    let peers: Vec<&str> = d["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["peer"].as_str().unwrap())
        .collect();
    assert_eq!(peers, vec!["intra-app", "unmanaged-nas", "10.1.4.9"]);
    assert_eq!(d["rows"][0]["checked"], true);
    assert_eq!(d["rows"][0]["services"], json!(["5432/tcp"]));
    assert_eq!(
        d["rows"][1]["checkable"], false,
        "icmp only, no ICMP service"
    );
    assert_eq!(d["rows"][2]["warnings"], json!(["uses IP list Corp"]));
    assert_eq!(d["rules"].as_array().unwrap().len(), 1);
    let r = &d["rules"][0];
    assert_eq!(r["ruleset"], "Shop-Prod");
    assert_eq!(r["action"], "create");
    assert_eq!(r["providers"], json!(["all workloads"]));
    assert_eq!(r["consumers"], json!(["all workloads"]));
    assert_eq!(r["unscoped_consumers"], false);
    assert_eq!(r["rows"], json!([1]));
    assert_eq!(r["body"]["providers"], json!([{"actors": "ams"}]));
    assert_eq!(
        r["body"]["ingress_services"],
        json!([{"port": 5432, "proto": 6}])
    );
    assert_eq!(d["preview"]["rules"], 1);
    assert_eq!(d["preview"]["create"], 1);
    assert_eq!(d["preview"]["covered_flows"], 1);
    assert_eq!(d["preview"]["uncovered_flows"], 2);
    assert_eq!(d["preview"]["uncovered_by"]["unchecked"], 1);
    assert_eq!(d["preview"]["uncovered_by"]["unsupported_proto"], 1);
    assert_eq!(out["meta"]["queries"], 1, "the in side only");
    let sent = server.received_requests().await.unwrap();
    assert!(
        sent.iter()
            .all(|r| r.method.as_str() == "GET" || r.url.path().ends_with("/async_queries")),
        "only the traffic query is a POST"
    );
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn gen_granularity_role_ports_finds_the_existing_rule() {
    let server = MockServer::start().await;
    mount_gen(&server, 3, 1).await;
    let out = stdout_json(
        cmd(&server)
            .args([
                "gen",
                "shop/prod",
                "--window",
                "1h",
                "--granularity",
                "role-ports",
            ])
            .assert()
            .success(),
    );
    let r = &out["data"]["rules"][0];
    assert_eq!(r["providers"], json!(["role=db"]));
    assert_eq!(r["consumers"], json!(["role=web"]));
    assert_eq!(r["action"], "exists");
    assert_eq!(
        r["existing_rule"],
        "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1"
    );
    assert!(r["body"].is_null());
    assert_eq!(out["data"]["preview"]["exists"], 1);
    for g in ["app-all", "role-all"] {
        let out = stdout_json(
            cmd(&server)
                .args(["gen", "shop/prod", "--window", "1h", "--granularity", g])
                .assert()
                .success(),
        );
        assert_eq!(
            out["data"]["rules"][0]["services"],
            json!(["all services"]),
            "{g}"
        );
        assert_eq!(
            out["data"]["rows"][0]["services"],
            json!(["all services"]),
            "{g}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn gen_outbound_adds_the_out_query_and_its_rows() {
    let server = MockServer::start().await;
    mount_gen(&server, 1, 2).await;
    let out = stdout_json(
        cmd(&server)
            .args(["gen", "shop/prod", "--window", "1h", "--outbound"])
            .assert()
            .success(),
    );
    assert_eq!(out["meta"]["queries"], 2);
    let rows = out["data"]["rows"].as_array().unwrap();
    let last = rows.last().unwrap();
    assert_eq!(last["side"], "outbound");
    assert_eq!(last["peer"], "93.184.216.34");
    assert_eq!(last["warnings"], json!(["needs an IP list"]));
    assert_eq!(last["checked"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn gen_refuses_capped_or_incomplete_traffic_and_unknown_apps() {
    let server = MockServer::start().await;
    // One capped query; the unknown apps are refused before any traffic.
    mount_traffic(&server, 1).await;
    let a = cmd(&server)
        .args(["gen", "shop/prod", "--window", "1h", "--max-results", "4"])
        .assert()
        .code(2);
    assert!(a.get_output().stdout.is_empty(), "no rules printed");
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(
        err.contains("traffic capped at 4 for shop/prod: raise --max-results or narrow --window; no rules proposed"),
        "{err}"
    );
    let a = cmd(&server)
        .args(["gen", "shpo/prod", "--window", "1h"])
        .assert()
        .code(2);
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(err.contains("closest: shop"), "{err}");
    let a = cmd(&server)
        .args(["gen", "shop", "--window", "1h"])
        .assert()
        .code(2);
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(err.contains("write it as <app>/<env>"), "{err}");

    let server = MockServer::start().await;
    mount_objects(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .respond_with(
            ResponseTemplate::new(202).set_body_json(json!({"href": QHREF, "status": "queued"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "completed", "result": {"href": format!("{QHREF}/download")},
            "regions": [{"pce_fqdn": "a", "responded": true, "flows_count": 4},
                        {"pce_fqdn": "b", "responded": false}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}/download")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/traffic_flows.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .mount(&server)
        .await;
    let a = cmd(&server)
        .args(["gen", "shop/prod", "--window", "1h"])
        .assert()
        .code(4);
    assert!(a.get_output().stdout.is_empty());
    let err: Value = serde_json::from_slice(&a.get_output().stderr).unwrap();
    assert_eq!(err["error"]["kind"], "incomplete");
}

#[tokio::test(flavor = "multi_thread")]
async fn gen_table_and_csv_print_the_rule_columns() {
    let server = MockServer::start().await;
    mount_gen(&server, 2, 1).await;
    let a = cmd(&server)
        .args(["gen", "shop/prod", "--window", "1h", "--table"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&a.get_output().stdout).to_string();
    assert!(text.contains("consumers → providers"), "{text}");
    assert!(text.contains("all workloads → all workloads"), "{text}");
    let a = cmd(&server)
        .args(["gen", "shop/prod", "--window", "1h", "--csv"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&a.get_output().stdout).to_string();
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("ruleset,action,consumers,providers,services,unscoped,existing_rule,description")
    );
    let row = lines.next().unwrap();
    assert!(
        row.starts_with(
            "Shop-Prod,create,all workloads,all workloads,5432/tcp,false,,\"generated: shop/prod → shop/prod on 5432/tcp, from 1h of traffic"
        ),
        "{row}"
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p illumio-tui --test cli gen_`
Expected: FAIL: `gen` is not a subcommand yet (clap exits 2 with "unrecognized subcommand").

- [ ] **Step 3: Write the implementation**

In `crates/illumio-tui/src/cli/error.rs`, add the variant:

```rust
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

impl CliError {
```

with:

```rust
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
    /// The data is missing rows (a PCE region did not answer): exit 4.
    #[error("{0}")]
    Incomplete(String),
}

impl CliError {
```

In `crates/illumio-tui/src/cli/error.rs`, replace:

```rust
            CliError::Pce(PceError::Auth { .. }) => 3,
            CliError::Pce(PceError::RateLimited { .. }) => 5,
            CliError::Pce(PceError::Invalid(_)) => 2,
            CliError::Pce(_) => 4,
            CliError::Write(e) => write_exit_code(e),
            CliError::Io(_) | CliError::Other(_) => 1,
        }
```

with:

```rust
            CliError::Pce(PceError::Auth { .. }) => 3,
            CliError::Pce(PceError::RateLimited { .. }) => 5,
            CliError::Pce(PceError::Invalid(_)) => 2,
            CliError::Pce(_) | CliError::Incomplete(_) => 4,
            CliError::Write(e) => write_exit_code(e),
            CliError::Io(_) | CliError::Other(_) => 1,
        }
```

In `crates/illumio-tui/src/cli/error.rs`, replace:

```rust
            CliError::Write(e) => e.kind().into(),
            CliError::Io(_) => "io".into(),
            CliError::Other(_) => "error".into(),
        }
    }

```

with:

```rust
            CliError::Write(e) => e.kind().into(),
            CliError::Io(_) => "io".into(),
            CliError::Other(_) => "error".into(),
            CliError::Incomplete(_) => "incomplete".into(),
        }
    }

```

Create `crates/illumio-tui/src/cli/generate.rs`:

```rust
//! `illumio-tui gen <app>`: the allow rules the app's observed traffic
//! needs, as the TUI's generator would plan them, printed and never
//! written. The same `pce::generate` code, the TUI's row defaults, one
//! global granularity.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{TableRows, join_list};
use super::traffic::{ByArg, DirectionArg, FilterArgs, cli_focus, fetch};
use clap::{Args, ValueEnum};
use pce::generate::{
    Coverage, GenInput, GenRow, Proposal, RuleCounts, Stop, keys_refusal, propose, rows,
};
use pce::models::PolicyScope;
use pce::write::PortSpec;
use serde::Serialize;

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GranularityArg {
    /// All workloads of the app, All Services
    AppAll,
    /// All workloads of the app, the services seen (default)
    #[default]
    AppPorts,
    /// One rule per role pair, the services seen
    RolePorts,
    /// One rule per role pair, All Services
    RoleAll,
}

impl From<GranularityArg> for Stop {
    fn from(g: GranularityArg) -> Self {
        match g {
            GranularityArg::AppAll => Stop::AppAll,
            GranularityArg::AppPorts => Stop::AppPorts,
            GranularityArg::RolePorts => Stop::RolePorts,
            GranularityArg::RoleAll => Stop::RoleAll,
        }
    }
}

#[derive(Args, Debug, Clone, Default)]
pub struct GenArgs {
    /// The app: one value per scope key, e.g. shop/prod
    pub app: String,
    /// How coarse the rules are (default app-ports)
    #[arg(long, value_enum, default_value_t)]
    pub granularity: GranularityArg,
    /// Also the outbound rules (into the peers' rulesets; IP peers into the app's): a second
    /// traffic query
    #[arg(long)]
    pub outbound: bool,
    /// Relative window ending now, e.g. 1h, 24h, 7d (default from profile, 24h)
    #[arg(long)]
    pub window: Option<String>,
    /// Row cap per traffic query, up to 200000 (default from profile; > 10000 needs --yes)
    #[arg(long)]
    pub max_results: Option<u32>,
    /// Confirm an expensive query (window > 7d, max_results > 10000)
    #[arg(long)]
    pub yes: bool,
}

#[derive(Debug, Serialize)]
pub struct TargetOut {
    pub name: String,
    pub href: Option<String>,
    /// `existing`, `create`, `ambiguous` or `none`.
    pub action: &'static str,
}

#[derive(Debug, Serialize)]
pub struct RowOut {
    pub side: pce::generate::Side,
    pub peer: String,
    pub kind: &'static str,
    pub checked: bool,
    pub checkable: bool,
    pub granularity: &'static str,
    pub flows: u64,
    pub services: Vec<String>,
    pub decisions: pce::generate::DecisionCounts,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct IpListOut {
    pub name: String,
    pub ranges: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct RuleOut {
    pub ruleset: String,
    /// `create`, `add_services` or `exists`.
    pub action: &'static str,
    pub providers: Vec<String>,
    pub consumers: Vec<String>,
    pub unscoped_consumers: bool,
    pub services: Vec<String>,
    /// 1-based, into `rows`.
    pub rows: Vec<usize>,
    pub existing_rule: Option<String>,
    pub description: String,
    /// The PCE JSON a create sends (`{step:N}` for what the batch creates
    /// first); for `add_services`, the services the update adds.
    pub body: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct Preview {
    #[serde(flatten)]
    pub counts: RuleCounts,
    pub covered_flows: u64,
    pub uncovered_flows: u64,
    pub uncovered_by: Coverage,
}

#[derive(Debug, Serialize)]
pub struct GenReport {
    pub app: String,
    pub keys: Vec<String>,
    pub window: String,
    pub granularity: &'static str,
    pub outbound: bool,
    pub target: TargetOut,
    pub rows: Vec<RowOut>,
    pub ip_lists: Vec<IpListOut>,
    pub rules: Vec<RuleOut>,
    pub preview: Preview,
}

impl TableRows for GenReport {
    fn units(&self) -> Option<&'static str> {
        Some("rules proposed from flow records; nothing is written")
    }
    fn headers(&self) -> Vec<&'static str> {
        vec![
            "ruleset",
            "action",
            "consumers → providers",
            "services",
            "unscoped",
        ]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.rules
            .iter()
            .map(|r| {
                vec![
                    r.ruleset.clone(),
                    r.action.to_string(),
                    format!("{} → {}", r.consumers.join(", "), r.providers.join(", ")),
                    r.services.join(", "),
                    r.unscoped_consumers.to_string(),
                ]
            })
            .collect()
    }
    fn full_headers(&self) -> Vec<&'static str> {
        vec![
            "ruleset",
            "action",
            "consumers",
            "providers",
            "services",
            "unscoped",
            "existing_rule",
            "description",
        ]
    }
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.rules
            .iter()
            .map(|r| {
                vec![
                    r.ruleset.clone(),
                    r.action.to_string(),
                    join_list(&r.consumers),
                    join_list(&r.providers),
                    join_list(&r.services),
                    r.unscoped_consumers.to_string(),
                    r.existing_rule.clone().unwrap_or_default(),
                    r.description.clone(),
                ]
            })
            .collect()
    }
}

fn service_texts(services: &std::collections::BTreeSet<PortSpec>) -> Vec<String> {
    services.iter().map(PortSpec::display).collect()
}

fn row_out(r: &GenRow, global: Stop, scope_keys: &[String]) -> RowOut {
    let stop = r.effective(global);
    RowOut {
        side: r.side,
        peer: r.peer_text(),
        kind: r.kind_text(),
        checked: r.proposed(),
        checkable: r.checkable,
        granularity: stop.cli(),
        flows: r.flows,
        services: if stop.is_all() {
            vec!["all services".into()]
        } else {
            service_texts(&r.services)
        },
        decisions: r.decisions,
        warnings: r.flag_texts(stop, scope_keys),
    }
}

fn rules_out(p: &Proposal, idx: &pce::NameIndex) -> Vec<RuleOut> {
    use pce::generate::RuleAction;
    let all = idx.all_services_href();
    p.rules
        .iter()
        .map(|r| RuleOut {
            ruleset: p.targets[r.target].name(),
            action: r.action.as_str(),
            providers: r.providers.iter().map(|a| a.text(p, idx)).collect(),
            consumers: r.consumers.iter().map(|a| a.text(p, idx)).collect(),
            unscoped_consumers: r.unscoped_consumers,
            services: r
                .services
                .iter()
                .map(|s| match s {
                    pce::write::ServiceRef::Href(h) if Some(h.as_str()) == all => {
                        "all services".to_string()
                    }
                    pce::write::ServiceRef::Href(h) => idx.service_name(h).unwrap_or(h).to_string(),
                    pce::write::ServiceRef::Port(port) => port.display(),
                })
                .collect(),
            rows: r.rows.iter().map(|i| i + 1).collect(),
            existing_rule: match &r.action {
                RuleAction::Create => None,
                RuleAction::AddServices { rule_href, .. } | RuleAction::Exists { rule_href } => {
                    Some(rule_href.clone())
                }
            },
            description: r.description.clone(),
            body: r.body(p),
        })
        .collect()
}

pub async fn run(ctx: &Context, args: &GenArgs) -> Result<(), CliError> {
    let keys = ctx.config.scope_keys.clone();
    let global = Stop::from(args.granularity);
    let filter = FilterArgs {
        window: args.window.clone(),
        max_results: args.max_results,
        yes: args.yes,
        app: Some(args.app.clone()),
        direction: Some(if args.outbound {
            DirectionArg::Both
        } else {
            DirectionArg::In
        }),
        ..FilterArgs::default()
    };
    let focus = cli_focus(&filter, &keys, ByArg::Pairs)?
        .ok_or_else(|| CliError::Other("--app is set".into()))?;
    if let Some(e) = keys_refusal(&focus.focus.keys, &keys) {
        return Err(CliError::Args(e));
    }
    if !focus.focus.is_label_group() {
        return Err(CliError::Args(format!(
            "{} is not an app of labels: write it as {}",
            args.app,
            keys.iter()
                .map(|k| format!("<{k}>"))
                .collect::<Vec<_>>()
                .join("/")
        )));
    }
    let Some(fetched) = fetch(ctx, &filter, Some(&focus)).await? else {
        return Ok(());
    };
    let focus = focus.focus;
    let app = focus.app_text();
    if fetched.truncated {
        return Err(CliError::Args(format!(
            "traffic capped at {} for {app}: raise --max-results or narrow --window; no rules proposed",
            fetched.cap
        )));
    }
    if fetched.incomplete {
        return Err(CliError::Incomplete(format!(
            "a PCE region did not answer the traffic query for {app}: run it again; no rules proposed"
        )));
    }
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
    if global.is_all() && idx.all_services_href().is_none() {
        return Err(CliError::Args(format!(
            "--granularity {}: the PCE has no All Services object; use app-ports or role-ports",
            global.cli()
        )));
    }
    let drafts = ctx
        .pce
        .rule_sets(PolicyScope::Draft, ctx.cache_mode)
        .await?;
    if drafts.truncated {
        return Err(CliError::Args(
            "too many draft rulesets to check the rules against; no rules proposed".into(),
        ));
    }
    let tier = ctx.config.tier_key.clone();
    let window = args
        .window
        .clone()
        .unwrap_or_else(|| ctx.config.traffic_default_window.clone());
    let gen_rows = rows(&fetched.flows, &focus, &tier, &idx);
    let input = GenInput {
        flows: &fetched.flows,
        focus: &focus,
        tier_key: &tier,
        drafts: &drafts.value,
        idx: &idx,
        ruleset_name: &ctx.config.generator_ruleset_name,
        window: &window,
        outbound: args.outbound,
        picked: None,
    };
    let p = propose(&gen_rows, global, &input);
    let t = &p.targets[0];
    let report = GenReport {
        app: app.clone(),
        keys: keys.clone(),
        window,
        granularity: global.cli(),
        outbound: args.outbound,
        target: TargetOut {
            name: t.name(),
            href: match t {
                pce::generate::Target::Existing { href, .. } => Some(href.clone()),
                _ => None,
            },
            action: t.action(),
        },
        rows: gen_rows
            .iter()
            .filter(|r| args.outbound || r.side == pce::generate::Side::Inbound)
            .map(|r| row_out(r, global, &keys))
            .collect(),
        ip_lists: p
            .ip_lists
            .iter()
            .map(|d| IpListOut {
                name: d.name.clone(),
                ranges: d.ranges.iter().map(|r| r.display()).collect(),
            })
            .collect(),
        rules: rules_out(&p, &idx),
        preview: Preview {
            counts: p.counts(),
            covered_flows: p.coverage.covered,
            uncovered_flows: p.coverage.uncovered,
            uncovered_by: p.coverage,
        },
    };
    let mut meta = fetched.meta(ctx);
    meta.cache = cache_label(fetched.cache == "hit" && drafts.hit, ctx.cache_mode);
    ctx.emit_meta(&report, &meta)
}
```

In `crates/illumio-tui/src/cli/mod.rs`, declare the module, the subcommand and its dispatch:

```rust
pub mod context;
pub mod error;
pub mod events;
pub mod health;
pub mod objects;
pub mod output;
```

with:

```rust
pub mod context;
pub mod error;
pub mod events;
pub mod generate;
pub mod health;
pub mod objects;
pub mod output;
```

In `crates/illumio-tui/src/cli/mod.rs`, replace:

```rust
    /// Peers: flows counts flow records; conns and bytes are the lifetime counts of those
    /// records, as in the PCE UI.
    App(app::AppArgs),
    /// Rulesets, rules, deny rules, pending changes, provisioning
    #[command(subcommand)]
    Policy(policy::PolicyCmd),
```

with:

```rust
    /// Peers: flows counts flow records; conns and bytes are the lifetime counts of those
    /// records, as in the PCE UI.
    App(app::AppArgs),
    /// Propose the allow rules one app needs, from its traffic (read-only)
    ///
    /// The TUI generator's rules for the app (G there), printed and never written: rows per
    /// peer, the rules at one granularity, and what is already in the draft.
    Gen(generate::GenArgs),
    /// Rulesets, rules, deny rules, pending changes, provisioning
    #[command(subcommand)]
    Policy(policy::PolicyCmd),
```

In `crates/illumio-tui/src/cli/mod.rs`, replace:

```rust
        Command::Events(a) => events::run(&ctx, &a).await,
        Command::Workloads(a) => workloads::workloads_cmd(&ctx, &a).await,
        Command::App(a) => app::run(&ctx, &a).await,
        Command::Policy(p) => policy::run(&ctx, &p).await,
        Command::Traffic(t) => traffic::run(&ctx, &t).await,
        Command::Profile(_) => unreachable!("handled above"),
```

with:

```rust
        Command::Events(a) => events::run(&ctx, &a).await,
        Command::Workloads(a) => workloads::workloads_cmd(&ctx, &a).await,
        Command::App(a) => app::run(&ctx, &a).await,
        Command::Gen(a) => generate::run(&ctx, &a).await,
        Command::Policy(p) => policy::run(&ctx, &p).await,
        Command::Traffic(t) => traffic::run(&ctx, &t).await,
        Command::Profile(_) => unreachable!("handled above"),
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p illumio-tui --test cli gen_ && cargo test -p illumio-tui --test cli every_flag_has_help_text`
Expected: PASS (6 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/illumio-tui/src/cli crates/illumio-tui/tests/cli.rs
git commit -m "cli: illumio-tui gen <app> (read-only rule proposal)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

### Task 8: README, tracking, read-only smoke, final gate

**Files:**
- Modify: `README.md`
- Modify: `tasks/todo.md`

**Interfaces:**
- Consumes: everything above.
- Produces: the README's `gen` reference and the setting; the tracking section.

- [ ] **Step 1: Document the command and the setting**

In `README.md`, add the examples to the CLI list:

```markdown
    illumio-tui app shop/prod --direction in --csv --section inbound > inbound.csv
```

with:

```markdown
    illumio-tui app shop/prod --direction in --csv --section inbound > inbound.csv
    illumio-tui gen shop/prod --window 24h --json
    illumio-tui gen shop/prod --granularity role-ports --outbound --table
```

After the `illumio-tui app` paragraphs, add the command's reference:

```markdown
`traffic query --blocked` prints the `--by in --show blocked`
```

with:

```markdown
`illumio-tui gen <app> [--granularity app-all|app-ports|role-ports|role-all] [--outbound]
[--window W] [--max-results N] [--yes]` proposes the allow rules the app's observed traffic
needs, as the TUI's generator would plan them, and writes nothing. `rows` holds one row per
peer and side (intra-app, each app peer, IP peer and unlabelled workload) with its default
checkbox (`checked`: intra-app and same-env app peers), the services seen, the reported
decisions and its warnings (`env mismatch`, `uses IP list Corp`, `needs an IP list`,
`ephemeral port?`, ...); `rules` the merged rules at the chosen granularity, each with its
`ruleset`, `action` (`create`, `add_services` or `exists`, against the draft rulesets),
`providers`, `consumers`, `unscoped_consumers`, `services`, `rows` and the exact PCE `body`
(`{step:N}` stands for an IP list or ruleset the batch would create first); `ip_lists` the
IP lists it would create (`shop-prod peer 10.4.1.9`); `preview` the counts and the flow
records covered or not, by reason. tcp and udp services are inline `port/proto` (consecutive
seen ports fold into a range), icmp uses the ICMP service, other protocols the one service of
exactly that protocol, else they are not covered. It costs the focused traffic query (`in`,
plus `out` with `--outbound`), the name index and one draft ruleset read, cached; `meta`
counts them. A capped result exits 2 and an incomplete one (a region did not answer) exits 4,
with no rules printed. `--table` and `--csv` print the rules. A new ruleset is named by the
profile's `[profiles.<name>.generator] ruleset_name` (below).

`traffic query --blocked` prints the `--by in --show blocked`
```

At the end of the "Label model" section, add the setting:

```markdown
`g` picker, and the Labels tab until `i`; JSON output and the workload edit form keep
them, and a scope or tier key is never ignored.
```

with:

```markdown
`g` picker, and the Labels tab until `i`; JSON output and the workload edit form keep
them, and a scope or tier key is never ignored.

The policy generator names a ruleset it creates by a template over the scope keys:

    [profiles.prod.generator]
    ruleset_name = "RF-{app}-{env}"   # the default: RF-shop-prod

Every `{key}` must be a scope key; another one fails the load and names it.
```

Append to `tasks/todo.md` (tick the boxes as the tasks land; the review section follows the smoke):

```markdown

## Policy generator 1: derive + `gen` (plan docs/superpowers/plans/2026-10-16-policy-generator-1-derive.md)
- [x] Task 1 pce prerequisites (PortSpec order, IP list data set, All Services, placeholders)
- [x] Task 2 pce::generate data model, services, targets; ruleset_pick uses them
- [x] Task 3 rows
- [x] Task 4 dedupe, Proposal::refresh
- [x] Task 5 propose
- [x] Task 6 generator.ruleset_name
- [x] Task 7 illumio-tui gen
- [x] Task 8 README, smoke, gate
```

- [ ] **Step 2: Run the final gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && git diff main -- '*Cargo.toml' Cargo.lock`
Expected: every test passes, no warning, the diff is empty.

- [ ] **Step 3: Read-only smoke on demo100**

Writes off, no `--yes`. Pick an app with flows from the last 24 hours first (`illumio-tui --env-file ../illumio-mcp-server/.env traffic groups --window 24h --json | jq -r '.data[0].dst | join("/")'`), then run each command once:

```bash
illumio-tui --env-file ../illumio-mcp-server/.env gen <app> --window 24h --json > /tmp/gen-1.json; echo "exit $?"
illumio-tui --env-file ../illumio-mcp-server/.env gen <app> --window 24h --granularity role-ports --outbound --json > /tmp/gen-2.json; echo "exit $?"
jq '{target: .data.target, preview: .data.preview, meta: .meta}' /tmp/gen-1.json /tmp/gen-2.json
```

Expected: exit 0 (or 2 with the capped message and no rules: then narrow `--window` and run again), `meta.queries` 1 then 2, `target.action` `existing` or `create`. Report the rule counts and the coverage in the review section. Nothing is written: the audit log does not grow (`wc -l ~/.local/state/illumio-tui/writes.jsonl` before and after).

- [ ] **Step 4: Write the review section**

Append a `## Review (Policy generator 1)` section to `tasks/todo.md`: the tasks, the test count, the smoke's rule counts and coverage, "nothing was written to any PCE".

- [ ] **Step 5: Commit**

```bash
git add README.md tasks/todo.md
git commit -m "docs: illumio-tui gen and generator.ruleset_name

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

## Self-review notes

- Spec coverage: rows (Task 3), derivation tables and merging (Task 5), services (Tasks 2, 5), dedupe (Task 4), coverage (Task 5), the ruleset setting (Task 6), the CLI with its JSON shape, `--granularity`, `--outbound`, exits 2 and 4, `--table`/`--csv` (Task 7), the shared `pick_ruleset` (Task 2).
- Left to Plan 2: the batch (`Batch`, `plan_batch`, `execute_batch`, `plan_create_rule_sets_if_missing`), the batch confirm. Left to Plan 3: the TUI generator and `complete_for`.
- Verification: every step of this plan was applied in order to a clean checkout of main (after the plans before it); each task built clean under `cargo clippy --workspace --all-targets -- -D warnings` with its own tests passing, and the gate passed at the end.
