# Policy generator: design

Date: 2026-10-16. Status: approved by Alex in conversation on 2026-10-16, awaiting spec review.

## Goal

Propose the allow rules one app needs, from the app's own observed traffic, and write them in one confirmed step. The admin chooses how coarse the rules are (app or role, all services or the services seen), which peers get rules, and whether outbound rules go into the peers' rulesets.

Hard rules, unchanged:

- Every PCE request is caused by the user, cached where it is a read, counted and visible. Nothing refreshes on its own.
- Writes need `:writes on` (profile `writes = true`, audit log writable). Every write goes through a confirm that shows the exact requests. Nothing is ever auto-confirmed.
- CLI `--yes` semantics stay as they are. No new dependencies.

## Definitions

- **App:** a focus group (`Focus`, `pce::traffic::focus`) whose keys are the profile's scope keys (`app,env` by default), for example `shop/prod`. The generator refuses any other keys. Its rules go into scoped rulesets, and only the scope keys say which ruleset that is. The refusal reads: `the generator needs g = app,env (the scope keys)`.
- **Role key:** the profile's tier key, which is the last entry of `KeyOrder::first()` after the scope keys (`role` by default).
- **Peer:** the other end of a flow, seen from the app.
  - an **app peer**: a workload with a label under every scope key;
  - an **IP peer**: an endpoint with no workload;
  - an **unlabelled workload**: a workload that lacks a scope label.
- **Intra-app:** flows whose source and destination are both in the app.

### Illumio semantics the generator relies on

These are verified against `write/rules.rs` and the `w` form's pre-fill (`edit::flow_rule_fields`).

- A scoped ruleset's rules apply only inside its scope.
  - **Providers.** Label-based providers (labels, label groups, `ams` "All Workloads") resolve within the scope. Providers outside the scope are impossible in that ruleset. IP list providers are not workloads, so they are not confined by the scope.
  - **Consumers.** Consumers resolve within the scope unless the rule has `unscoped_consumers: true` (an extra-scope rule). Their labels then match anywhere.
- So a rule that lets app B reach app A must live in A's ruleset (A provides). Inbound rules go into the app's own ruleset. Outbound rules to other apps go into the peers' rulesets.
- An outbound rule to an IP peer goes into the app's own ruleset, with the IP list as provider and the app's workloads as in-scope consumers.
- The PCE programs both ends from one rule. An app-to-app pair never needs a second, "egress" rule.

## Approach

**Chosen: a pure generator in `pce` over the loaded flows, plus a write batch with one confirm.**

- **`pce::generate` (new, pure).** It builds rows from the flows, rules from the rows and the slider, and dedupes against the draft rulesets. The TUI and the CLI share it, as they share `pce::app`.
- **`pce::write::batch` (new).** An ordered list of plans whose later steps can name the href that an earlier step creates. It is sent one request at a time, stops at the first failure, and audits each request.
- **TUI.** A new overlay, `Overlay::Generator`, holds its state in `Model::gen_view`, in the same style as `AppView`.

**Rejected:**

- One existing confirm per write, chained the way `rule_after_ruleset` chains one rule after `Ctrl-R`'s ruleset. That chaining handles exactly one follow-up step. With N rules, N prompts would be unusable, and a `y` typed for one prompt could land on the next.
- Creating a ruleset with its rules inline in one POST. `RuleSetDraft` does not carry rules. The batch would still need dependent steps for IP lists, and a single failed rule would sink the whole ruleset.
- Opening N `w` forms. This does not scale and cannot dedupe across rules.

## What exists and what is new

| Piece | Status |
|---|---|
| App overlay `A`, `AppView`, `app_peers`, `Focus::keeps`, `flows_for` / `partial_for` | exists (`tui/app_view.rs`, `pce::app`, `tui/model.rs`) |
| App picker, `AppPickerPurpose::{Focus, Overlay}` | exists; adds `Generator` |
| Focused re-query (`r`, one query per side, confirmed above the threshold) | exists (`start_focused`) |
| Draft rulesets for forms: `Model::draft_rulesets()`, `PolicyState.form_drafts`, `load_form_drafts` (one cached GET, counted on the status line) | exists, reused as is |
| Ruleset choice by destination scope: `edit::ruleset_pick`, `rulesets_covering`, `scope_of`, `tier_of` | exists; the generator calls `ruleset_pick` with a synthetic flow per target, so no logic is copied |
| `CreateRuleSetIfMissing` → `plan_create_rule_set(.., IfMissing::SkipExisting)` (fresh read, `Plan::Noop` with the existing href) | exists; the batch planner uses a variant that reads once for all rulesets (new: `plan_create_rule_sets_if_missing`) |
| `plan_create_rule`, `plan_update_rule` + `RulePatch.add_services` (fresh GET of the rule, list sent whole) | exists |
| IP list create: `plan_create_ip_list(.., IfMissing::SkipExisting)`, `IpListDraft`, `IpRangeSpec` | exists, so IP list creation is in scope for v1 |
| Inline services: `ServiceRef::Port(PortSpec)`, tcp/udp only (`INLINE_SERVICE`); ICMP via `NameIndex::icmp_service_href` | exists |
| `ObjectKind::cache_prefixes` invalidation, audit per `Writer::execute` | exists, reused per step |
| `finish_form` / `close_form` back to the app overlay (`form_from_app`) | exists; the generator gets the same "back to where it came from" handling |
| Post-write draft re-evaluation `redraft_after_write` (#32) | exists; called once per batch |
| Record-key merge (`merge_flows`, `union_flows`), never summing lifetime counters | exists; the generator only reads merged flows |
| `pce::impact::group_members` | exists; used for dedupe against label-group actors |
| `pce::generate` (rows, derivation, dedupe, coverage) | **new** |
| `pce::write::batch` (`Batch`, `Writer::execute_batch`) | **new** |
| `Overlay::Generator`, `Overlay::ConfirmBatch`, `GenView`, `:gen`, `G` | **new** |
| `TrafficState::complete_for(focus)`: truncated or incomplete for the result actually used | **new**. `partial_for` returns `false` for a covering focused result even when that result is itself truncated, and it ignores `incomplete`. |
| CLI `illumio-tui gen` (read-only) | **new** |

## Completeness: refuse, then offer the re-query

**Ruling.** The generator shows its rows from incomplete data, but refuses to plan or print rules from it. `Enter` and the CLI refuse while the data used is incomplete. Incomplete means any of the following:

- the result is truncated;
- `incomplete` is set (a supercluster region did not answer);
- the result is not a focused result covering the needed sides, and the unfocused result is truncated.

**Why.** Missing flows become missing rules. Once the app is enforced, a missing rule is an outage. The rows stay visible so the admin can see what the data holds. Proposing rules from that data would read as a complete answer when it is not.

- **Sides checked.** Inbound needs the `in` side. Outbound, once it is turned on, also needs the `out` side. Each side is checked with `complete_for(focus.with_direction(..))`.
- **Header.** It shows `incomplete: r re-queries shop/prod (2 queries)`.
- **`r`.** It runs the existing focused query (`start_focused`, the same cost guard and confirm as app focus). The generator rebuilds when the answer lands, as `rebuild_app_traffic` does for the overlay.
- **Still truncated.** If the focused result is truncated too, the header says `still capped at N: B raises the cap, or narrow the window`. `B` and `]` are not duplicated in the generator.
- **No flows.** A window with no flows for the app shows `no flows in the window: nothing to propose`.

## TUI

### Opening

- `G` in the app overlay opens the generator for the overlay's app. `:gen <app>` opens it by name, and `:gen` alone opens the app picker with `AppPickerPurpose::Generator`.
- **Requests on open.** If `Model::draft_rulesets()` is `None`, opening calls `load_form_drafts`: one GET, cached, counted in the generator's header (`1 req`). Nothing else is sent before `Enter`.
  - The Policy screen's draft list counts when it is loaded, and so does a `w` form's `form_drafts`.
  - Until the rulesets arrive, the rows render and the targets show `loading rulesets`.
- `Esc` goes back to where the generator came from: the app overlay (its view is kept, no request), or the screen behind it. `finish_form` and `close_form` get a `gen_from_app` twin of `form_from_app`.
- **Writes off.** The generator still opens (preview and `p` work). `Enter` gives the usual `writes_refused` text.

### Layout

```
┌ Generate · shop/prod · last 24h · 1 310 flows · complete ── 1 req · cached ┐
│ granularity  app·all ─[app·ports]─ role·ports ─ role·all        ←/→  </> row │
│ target  RS shop-prod (draft, 12 rules)                                        │
├ Inbound → shop-prod ─────────────────────────────────────────────────────────┤
│ [x] intra-app            app·ports   tcp/5432 tcp/6379      ✓ 402            │
│ [x] web/prod             app·ports   tcp/443                ✓ 812  ✗ 3      │
│ [x] ops/prod             role·ports* tcp/22                 ✗ 41             │
│ [ ] web/dev  ⚠ env mismatch          tcp/443                ✗ 12             │
│ [ ] 10.4.1.9 ⚠ needs an IP list      tcp/443                ✗ 6              │
├ Outbound (o: off) ───────────────────────────────────────────────────────────┤
│ 7 rows hidden: o shows them                                                   │
├──────────────────────────────────────────────────────────────────────────────┤
│ 5 rules (4 new · 1 adds services) · covers 1 217 of 1 310 flows · 93 not     │
│ p lists the rules · Enter plans the writes · Space toggles · o outbound      │
└──────────────────────────────────────────────────────────────────────────────┘
```

- **Header.** It shows the app, window, flow count (flow records, after the record-key merge) and completeness, then the generator's own request count and cache hits.
- **Slider.** Four stops, in this order: `app·all`, `app·ports`, `role·ports`, `role·all`.
  - The default is `app·ports`.
  - `←`/`→` (and `h`/`l`) move the global stop.
  - `<`/`>` override the selected row. The row then shows its own stop with a `*`, and `=` drops the override.
- **Target line.** It names the app's ruleset, from `ruleset_pick` on the app's scope:
  - the one existing ruleset: `RS shop-prod (draft, 12 rules)`;
  - a ruleset to create: `new ruleset shop-prod, scoped app=shop, env=prod`;
  - several covering rulesets: `3 rulesets cover shop/prod: R picks`. `R` cycles them in `covering` order, and `Enter` refuses until one is picked.
- **Inbound rows** (the app's ruleset):
  - one intra-app row;
  - one row per app peer (the peer's scope values);
  - one row per IP peer;
  - one row per unlabelled workload.
- **Columns.** Each row shows:
  - its checkbox;
  - its stop;
  - the services seen, or `all services` at an `·all` stop;
  - the reported decision counts (`✓` allowed, `✗` blocked or potentially blocked);
  - with draft decisions loaded (#29), a `would allow N` mark. The mark is information only and does not change what is proposed.
- **Outbound rows** (`o` toggles the section, off by default):
  - one row per app peer, written into the peer's ruleset (its target is shown on the row);
  - one row per IP peer, written into the app's own ruleset.
  - Intra-app traffic is inbound only, so it never appears here.
- **Keys.**
  - `j`/`k` move;
  - `Tab` switches section;
  - `Space` toggles the checkbox;
  - `p` opens the exact rule list. It shows one block per write, as the confirm will show it, scrollable, and `Esc` goes back;
  - `Enter` plans;
  - `r` re-queries (only when the data is incomplete).

### Row defaults

| Row | Default | Flag |
|---|---|---|
| intra-app | checked | — |
| app peer, same values under every scope key except the first (same env) | checked | — |
| app peer, different env | unchecked | `⚠ env mismatch` |
| IP peer in an existing IP list other than the built-in Any list | unchecked | `⚠ uses IP list <name>` |
| IP peer in no such list | unchecked | `⚠ needs an IP list` (checking it plans one) |
| unlabelled workload | unchecked | `⚠ no scope labels: rule names the workload` (inbound). Outbound: `⚠ no scope labels: no ruleset to write into`, and the row cannot be checked |
| a row whose only services cannot be expressed (below) | unchecked, cannot be checked | `⚠ proto N needs a service object` |

The env rule applies only when the scope keys have a second key. The built-in Any list is identified the way `icmp_service` identifies built-ins: `external_data_set == "com.illumio.ilo_defaults"`, else the name `Any (0.0.0.0/0 and ::/0)`.

## Rule derivation

Inputs:

- the app's flows, from `TrafficState::flows_for` (already one row per record);
- the rows with their checkbox and effective stop;
- the role key;
- the `NameIndex`;
- the draft rulesets.

The derivation is pure and needs no request.

### 1. Flow → row

- **Excluded before any row is built:**
  - destination multicast (`224.0.0.0/4`, `ff00::/8`);
  - limited broadcast (`255.255.255.255`).
  - Subnet broadcasts cannot be told apart without the netmask. They land on an IP peer row, which is unchecked by default.
  - Excluded flows count as `not covered: broadcast/multicast` in the preview.
- **Side.** `in` when the destination is in the app (`group_key == app`), `out` when the source is.
  - A self-flow is the intra-app row, inbound only.
  - A flow that is in only on `out` goes to an outbound row.
- **Peer.** It is chosen by the other endpoint:
  - a workload with every scope label → app peer (its scope values);
  - a workload without them → unlabelled workload (its href);
  - no workload → IP peer, keyed by its IP.
- **Many IP peers.** Above 25 IP peer rows per section, the rest (by connections) fold into one read-only row, `N more IPs: not proposed`. Those flows count as uncovered.

### 2. Row → rules, by stop

Notation:

- `A` is the app's scope labels (`app=shop, env=prod`).
- `P` is the peer's scope labels.
- `role(e)` is the endpoint's role label (`tier_of`).
- `ams` is All Workloads.

**Inbound** (target: the app's ruleset):

| Row | Stop | Providers | Consumers | `unscoped_consumers` |
|---|---|---|---|---|
| intra-app | app·* | `ams` | `ams` | false |
| intra-app | role·* | `role(dst)` | `role(src)` | false |
| app peer | app·* | `ams` | `P` | true |
| app peer | role·* | `role(dst)` | `P + role(src)` | true |
| IP peer | any | `ams` (app·*) or `role(dst)` (role·*) | the IP list | false |
| unlabelled workload | any | `ams` or `role(dst)` | `workload:<href>` | true |

**Outbound** (target: the peer's ruleset for app peers, the app's own for IP peers):

| Row | Stop | Providers | Consumers | `unscoped_consumers` |
|---|---|---|---|---|
| app peer | app·* | `ams` (within the peer's scope) | `A` | true |
| app peer | role·* | `role(dst)` | `A + role(src)` | true |
| IP peer | app·* / role·* | the IP list | `ams` / `role(src)` | false |

- **Services.**
  - At `·all` stops, every rule takes the built-in All Services object (looked up in the index like `icmp_service`). If it is missing, the `·all` stops are disabled and the slider says why.
  - At `·ports` stops, a rule takes the union of the services seen on its flows (see Services).
- **Role granularity splits a row.** It makes one rule per distinct (consumer role, provider role) pair seen in the row's flows, each with that pair's services.
- **Missing role (ruling).** At role granularity, an endpoint without a role label falls back to the app level on that side: `ams` for the app's side or the intra-app side, `P` for a peer. The row is flagged `⚠ N flows without role: app-level for those`. Dropping those flows would silently leave them uncovered. Refusing the whole row would punish the labelled majority.
- **Merging.** Rules with the same target, the same provider set, the same consumer set and the same `unscoped_consumers` merge into one rule, with their services unioned. Several rows can therefore feed one rule, for example two peers at the same role stop. The rule keeps the rows it came from, for `p`.
- **Descriptions.** For example `generated: web/prod → shop/prod on 443/tcp, from 24h of traffic (illumio-tui)`, kept under the PCE's limit by truncating the service list with `+N`.

### 3. Services: inline ports

**Ruling.** tcp and udp services are inline `port/proto` (`ServiceRef::Port`). Existing service objects are never matched by their ports.

- **Why inline.** Inline ports say exactly what was seen and create no object sprawl. Dedupe is exact (`PortSpec` equality and range containment). Matching "HTTPS" by its ports is ambiguous when several objects share a port, and it changes meaning when someone edits the object.
- **Exceptions:**
  - icmp/icmpv6 use the ICMP service object (`icmp_service_href`), as `w` does today.
  - Any other protocol (GRE, ESP) cannot be inline (`INLINE_SERVICE`). It uses an existing service whose only port entry is exactly that protocol, if the index has exactly one. Otherwise that service is dropped from the rule, the row is flagged, and those flows count as uncovered. The generator creates no service objects in v1.
- **Port ranges.** Explorer reports single ports. Consecutive ports of one protocol on one rule fold into a range (`8000-8010/tcp`), but only when every port in the range was seen. The rule stays exact and gets shorter.
- **Large service lists.** A row whose rule would carry more than 32 service entries after folding is flagged `⚠ N services: consider app·all or role·all`. It is still proposed as is; the admin decides.
- **Ephemeral ports.** Destination ports ≥ 49152 are flagged `⚠ ephemeral port?` (dynamic RPC and similar) and stay proposed.

### 4. Dedupe against the draft

This runs against the target ruleset's live draft rules. Pending deletions (`update_type == "delete"`) and disabled rules are ignored. Each proposed rule is classified:

- **Already in draft.** An existing rule covers it:
  - each proposed provider and consumer is matched by an existing actor: the same label, a label group whose `group_members` hold it, the same IP list or workload, or `ams` on the in-scope side;
  - `unscoped_consumers` is the same;
  - the existing services cover every proposed service: All Services; a port range containing the port; a service object expanded through the index's `service_ports`.
  - The rule is not written. `p` shows it as `exists: <rule href>`.
- **Adds services.** An existing rule has exactly the same actor sets and flag, but lacks some services. The batch plans `UpdateRule` with `RulePatch.add_services` (the existing path: one fresh GET of the rule at plan time, list sent whole). This is how "merge services per rule" reaches the PCE without a near-duplicate rule.
- **Create.** Otherwise a new rule is created.

A target ruleset that does not exist yet has nothing to dedupe against, so every rule in it is created.

### 5. Coverage preview (local)

- Every flow kept by the generator belongs to exactly one row. A flow is **covered** when its row is checked and its service is in the rule or rules the row produced, or was already in draft.
- Everything else is **not covered**, by reason: unchecked row, folded IPs, unsupported protocol, broadcast/multicast.
- The preview line shows the counts of flow records. `p` shows the reasons. No request is made.

## Writes: one batch, one confirm

### Planning (`Enter`)

`Enter` is refused, with the reason on the status line, if any of these holds:

- writes are off;
- the data is incomplete;
- no row is checked;
- the target ruleset is ambiguous;
- the draft rulesets are not loaded;
- a write is in flight.

Otherwise `Cmd::PlanBatch` builds the batch through the guard. Plan-time reads, all past the cache and counted in the confirm header:

1. One fresh draft ruleset read (`rule_sets(Draft, Bypass)`).
   - It serves as the create-if-missing check for every ruleset the batch creates (`plan_create_rule_sets_if_missing`).
   - Dedupe runs again on it, so a rule someone added since the cached read is not duplicated.
   - If the read is truncated, planning is refused: dedupe cannot be proven.
2. One fresh IP list read, only if the batch creates IP lists (`IfMissing::SkipExisting`). An existing list of the same name is reused through `Plan::Noop` and its href.
3. One GET per `UpdateRule` (the existing `plan_update_rule`).

**Step order:**

1. IP list creates;
2. ruleset creates;
3. rule updates;
4. rule creates.

**Placeholders.** A step that needs an href created by an earlier step carries a placeholder, `{step:N}`, for:

- a rule's ruleset path;
- an IP list actor's href.

The executor replaces the placeholder with that step's `Written.href`, or with the `existing_href` of a `Noop`, just before sending. A placeholder that is still unresolved stops the batch. A placeholder never passes `checked_href`, so it cannot be sent by mistake.

**Created objects:**

- **New ruleset.** Named by the profile setting `[profiles.<name>.generator] ruleset_name`, a template over the scope keys; the default is `"{app}-{env}"` (`shop-prod`), e.g. `"RS-{app}-{env}"`. Unknown placeholders are a config error at load, naming the key. The CLI `gen` uses the same setting, scoped by the app's scope labels exactly as `edit::new_ruleset_intent` scopes one, `enabled: true`.
- **New IP list.** One per checked IP peer row, named `<app>-<env> peer <ip>`, with one range (`<ip>`). It is reused by name if it already exists.

### The confirm (`Overlay::ConfirmBatch`)

- **Rendering.** It uses the same widget and keys as `ConfirmWrite`:
  - one block per request: method, path (placeholders spelled `<href of step 2: create rule_set shop-prod>`) and body;
  - `y` sends the whole batch, `n`/`Esc` cancel, and scrolling works as in the single confirm.
- **Header.** For example `7 requests: 1 ip_list, 1 rule_set, 1 rule update, 4 rules · planned with 3 reads`.
- **Impact lines.** The single-rule impact line (`refresh_confirm_impact`) is not computed for a batch in v1.
- **Guards.** The in-flight guard and the discard rule from the write layer lessons apply unchanged:
  - no new edit while a plan or a batch is in flight;
  - a plan that lands over another overlay is discarded.

### Executing (`Writer::execute_batch`)

- **Sending.** It is sequential: one `Writer::execute` per step (audit entry, cache invalidation by `ObjectKind`, re-fetch on a 204), throttled by the client's limiter like any request.
- **Stopping.** The batch stops at the first error. It is never retried, and nothing is rolled back: these are draft objects, and the pending overlay's `R` reverts them.
- **Outcome.** `BatchOutcome` lists each step as one of:
  - `Written`;
  - `Skipped` (a `Noop`);
  - `Failed(e)`;
  - `NotSent`.
- **Status line.** For example `wrote 5 of 7 (pending); step 6 failed: …; 1 not sent`.
  - After a `WriteError::Unanswered`, the batch stops, the status says the step may have applied, and the policy view reloads (`reload_after_write`), as for a single write.
- **After the batch:**
  - The policy view reloads once per batch, not per step.
  - `redraft_after_write(model, from_flow = true, waits = false)` runs **once**. If draft decisions are on screen, or the result is draftable without a confirm, the draft is evaluated again (#32). Otherwise the status line says `D re-evaluates`.
  - The generator reopens on the reloaded drafts, so rules that were written now show as `exists`. A partly failed batch resumes naturally with the next `Enter`.

## CLI: `illumio-tui gen` (v1, read-only)

**Ruling: yes, in v1, as a dry run.** It runs the same `pce::generate` code, so reviewers and agents can check the derivation without the TUI. It also documents what `Enter` would write. It never writes. A write mode (`--apply --yes`) is out of scope.

```
illumio-tui gen <app> [--granularity app-all|app-ports|role-ports|role-all]
                      [--outbound] [--window 24h] [--max-results N] [--yes]
```

- **Rows.** The TUI's row defaults apply: same-env peers and intra-app checked, others listed with `checked: false`.
- **`--granularity`.** It sets the global stop (default `app-ports`). There are no per-row overrides in v1.
- **Requests:**
  - the focused traffic query: `in`, plus `out` with `--outbound`. This is `cli_focus`/`fetch` as `app` uses them, and the same `--yes` threshold applies (window > 7d, cap > 10000);
  - the labels and name index (cached);
  - one draft ruleset read (cached; `--no-cache` bypasses it).
  - `meta` reports requests, bytes and cache hits.
- **Incomplete data.** A truncated result exits 2: `traffic capped at N for shop/prod: raise --max-results or narrow --window; no rules proposed`. An incomplete one (a region did not answer) exits 4. No rules are printed in either case.
- **Other errors.** An unknown app exits 2 with the closest matches, as `app` does. So do keys that are not the scope keys.

JSON (the default when piped):

```json
{
  "app": "shop/prod", "keys": ["app", "env"], "window": "24h",
  "granularity": "app-ports", "outbound": false,
  "target": {"name": "shop-prod", "href": "/orgs/1/sec_policy/draft/rule_sets/12", "action": "existing"},
  "rows": [
    {"side": "inbound", "peer": "web/prod", "kind": "app", "checked": true,
     "granularity": "app-ports", "flows": 815, "services": ["443/tcp"],
     "decisions": {"allowed": 812, "blocked": 3}, "warnings": []}
  ],
  "ip_lists": [],
  "rules": [
    {"ruleset": "shop-prod", "action": "create",
     "providers": ["all workloads"], "consumers": ["app=web", "env=prod"],
     "unscoped_consumers": true, "services": ["443/tcp"], "rows": [1],
     "existing_rule": null, "body": {"providers": [{"actors": "ams"}], "...": "..."}}
  ],
  "preview": {"rules": 5, "create": 4, "add_services": 1, "exists": 2,
              "covered_flows": 1217, "uncovered_flows": 93,
              "uncovered_by": {"unchecked": 81, "folded_ips": 0, "unsupported_proto": 4, "broadcast_multicast": 8}},
  "meta": {"requests": 2, "cached": 1, "unit": "flow records"}
}
```

- The value of `action` is `create`, `add_services` or `exists`.
- `body` is the exact PCE JSON (`RuleDraft::to_body`). For a ruleset or IP list that does not exist yet, it holds the `{step:N}` placeholders.
- `--table` prints the rules: ruleset, action, consumers → providers, services, unscoped. `--csv` prints the same columns in full.

## Data model (new, `pce::generate`)

```rust
pub enum Stop { AppAll, AppPorts, RolePorts, RoleAll }          // slider order; default AppPorts
pub enum Side { Inbound, Outbound }
pub enum PeerKind { Intra, App(GroupKey), Ip { ip: String, list: Option<String> },
                    Workload { href: String, name: String }, FoldedIps(usize) }
pub enum RowFlag { EnvMismatch, UsesIpList(String), NeedsIpList, NoScopeLabels,
                   NoRole { flows: u64 }, UnsupportedProto(i64), ManyServices(usize), Ephemeral }
pub struct GenRow { pub side: Side, pub peer: PeerKind, pub checked: bool, pub checkable: bool,
                    pub stop: Option<Stop>, pub flags: Vec<RowFlag>, pub flows: u64,
                    pub decisions: DecisionCounts, pub services: BTreeSet<PortSpec> }
pub enum GenActor { Ref(ActorRef), NewIpList(usize) }          // index into Proposal::ip_lists
pub enum Target { Existing { href: String, name: String }, Create(RuleSetDraft),
                  Ambiguous(Vec<String>), None(String) }
pub enum RuleAction { Create, AddServices { rule_href: String, add: Vec<ServiceRef> },
                      Exists { rule_href: String } }
pub struct GenRule { pub target: usize, pub providers: Vec<GenActor>, pub consumers: Vec<GenActor>,
                     pub unscoped_consumers: bool, pub services: Vec<ServiceRef>,
                     pub rows: Vec<usize>, pub action: RuleAction, pub description: String }
pub struct Proposal { pub targets: Vec<Target>, pub ip_lists: Vec<IpListDraft>,
                      pub rules: Vec<GenRule>, pub coverage: Coverage }

pub fn rows(flows: &[ResolvedFlow], focus: &Focus, tier_key: &str, idx: &NameIndex) -> Vec<GenRow>;
pub fn propose(rows: &[GenRow], global: Stop, flows: &[ResolvedFlow], focus: &Focus,
               tier_key: &str, drafts: &[RuleSet], idx: &NameIndex) -> Proposal;
```

- **Rows.** `rows` is computed once per flow set and keeps its checkbox and override state across rebuilds by peer key. `propose` runs on every key press. It is pure and touches no network.
- **Outbound targets.** `Target` for an outbound app peer comes from `edit::ruleset_pick` on the peer's scope. The `pce` crate gets a scope-only version, `pce::generate::pick_ruleset(scope_labels, drafts, idx)`. `edit::ruleset_pick` is changed to call it, so the logic exists once.
- **`pce::write::batch`:**

```rust
pub struct BatchStep { pub plan: Plan, pub kind: ObjectKind, pub needs: Vec<usize> }
pub struct Batch { pub steps: Vec<BatchStep>, pub reads: u32 }
pub enum StepResult { Written(Written), Skipped { existing_href: String }, Failed(WriteError), NotSent }
impl Writer<'_> {
    pub async fn plan_create_rule_sets_if_missing(&self, d: &[RuleSetDraft]) -> Result<Vec<Plan>, WriteError>;
    pub async fn execute_batch(&self, b: Batch) -> Vec<StepResult>;
}
```

- **TUI.**
  - `GenView` holds the focus, the window, `seq`, the rows, the global stop, the section and cursor, `outbound: bool`, the picked target index, `requests`/`cached` and `from_app`.
  - The `Msg` and `Cmd` variants are `PlanBatch(Batch inputs)`, `BatchPlanned(Result<Batch>)`, `ExecuteBatch(Batch)` and `BatchDone(Vec<StepResult>)`.

## Edge cases

- **Label groups.** A scope or existing rule made of label groups covers the app or proposal through `group_members` (dedupe and `ruleset_pick` both). The generator never proposes label groups.
- **Unmanaged workloads.** They are workloads and carry labels, so they are app peers when labelled. Their flows are reported from the managed side only; the generator proposes what was seen.
- **Several envs.** The app is one `app/env`. Peers in other envs default to unchecked with the flag, and intra-app means the same app and env.
- **The app in several rulesets.** `R` picks among the covering rulesets. Rules are never split across rulesets.
- **Several IP lists for one IP peer.** The narrowest non-Any list is the one the PCE lists first (`ip_lists.first()`, as `group_key` uses). It is named on the row.
- **ICMP.** If no ICMP service object exists, ICMP flows count as `unsupported proto` (uncovered).
- **Pending deletions.** A target ruleset pending deletion is not a target. If it is the only match, the generator plans a create, and the fresh read at plan time confirms that no live one exists.
- **Profile keys changed** (`g`) while the generator is open: it closes with `g changed: G reopens`.
- **Late answers.** A traffic or ruleset answer for an earlier `seq` is dropped, as in the app overlay.

## Errors

- Ruleset load fails: the target line shows the error, `r` retries (1 request, cached), and `Enter` is refused.
- A plan-time read fails: nothing is sent, the error overlay opens, and the generator stays open with its state.
- A batch step fails: see Executing. The audit log holds every sent step.

## Testing

- **`pce::generate` units:**
  - every stop, for each row kind and side, against the derivation tables;
  - the role split, and the no-role fallback with its flag;
  - merging across rows;
  - excluding broadcast and multicast;
  - IP folding above 25 rows;
  - ICMP to the service object; another protocol with exactly one matching object, and with none;
  - port folding, only over consecutive seen ports;
  - the flags for 32+ services and ephemeral ports;
  - env-mismatch defaults; the single-scope-key case (no env rule);
  - dedupe: exact match → `exists`; a label-group member → `exists`; All Services covers; a range covers; missing services → `add_services`; disabled and pending-delete rules ignored;
  - coverage counts per reason.
  - Fixtures come from `fixtures/recorded/` where they exist (unscoped rulesets are `[[]]`).
- **Batch (wiremock):**
  - order;
  - placeholder substitution from `Written.href` and from a `Noop`'s href;
  - stop at the first failure (the rest `NotSent`);
  - `Unanswered` stops;
  - one audit entry per sent step;
  - cache prefixes dropped per kind;
  - no request before `execute_batch`;
  - a leftover placeholder refused before sending;
  - one ruleset read for several ruleset creates.
- **TUI:**
  - `G` and `:gen` open the generator; on open there is at most one request (the cached draft rulesets), counted;
  - `←`/`→`, `<`/`>`/`=`, `Space`, `o`, `R`, `p`;
  - incomplete data: `Enter` refused, `r` re-queries;
  - writes off: `Enter` refused;
  - the confirm lists every request, and `y` sends exactly one `ExecuteBatch`;
  - a `y` typed during planning confirms nothing;
  - one `redraft_after_write` per batch;
  - `Esc` back to the app overlay with nothing sent;
  - insta snapshots of the layout, `p` and the confirm.
- **CLI:**
  - the JSON shape;
  - `--granularity` per stop;
  - `--outbound` adds the `out` query (meta 2 requests);
  - a truncated result exits 2 with no rules;
  - an unknown app exits 2;
  - the `--table` and `--csv` columns.
- **Read-only smoke on demo100** (no `--yes`, no `y`, writes off): `gen <app>` and `gen <app> --granularity role-ports --outbound` once each. Report the rule counts and coverage.

## Out of scope (v1)

- Writing from the CLI (`gen --apply`).
- Deny rules and override denies.
- Enforcement boundaries.
- Creating service objects or label groups.
- Virtual services and `resolve_labels_as` other than workloads.
- Several apps at once.
- Provisioning: the pending overlay's `P` remains the way.
- Tightening or deleting existing broad rules.
- Process- or user-based rules.
- FQDN-based IP lists.
- Merging IP peers into CIDRs.
- Impact lines in the batch confirm.
- Per-row overrides in the CLI.

## Review focus

1. **IP list consumers.** `unscoped_consumers: false` for IP list consumers in the app's ruleset, and an IP list provider in a scoped ruleset for outbound to IPs. Confirm both against recorded PCE rules before the implementation relies on them.
2. **Dedupe coverage.** The coverage test with `ams` on the in-scope side, label groups and service objects. A false `exists` silently drops a needed rule.
3. **The fresh read at `Enter`.** It re-runs dedupe and the create-if-missing check, and is refused when truncated. Then check placeholder handling: a placeholder never reaches the wire.
4. **The completeness gate.** `complete_for`, including a focused result that is itself truncated, which today's `partial_for` misses.
5. **The batch confirm.** It keeps every guarantee of `ConfirmWrite`: the in-flight guard, discard over another overlay, never auto-confirm, one `redraft_after_write`.

## Delivery

Three GitHub issues, each with its own worktree, review and merge:

1. `pce::generate` (rows, derivation, dedupe, coverage) and `illumio-tui gen` (read-only). This reviews the derivation with no write code.
2. `pce::write::batch` (`plan_create_rule_sets_if_missing`, `execute_batch`) and `Overlay::ConfirmBatch`.
3. The TUI generator (`G`, `:gen`, `GenView`, `complete_for`). Docs: README, docs/index.html, in-TUI help.

## Decisions (Alex, 2026-10-16)

1. **Already-allowed rows stay checked** by default, so the app's ruleset spells out its own traffic.
2. **New ruleset name is a profile setting**: `[profiles.<name>.generator] ruleset_name`, default `"{app}-{env}"` (see Planning).
3. **One IP list per IP peer**, named `<app>-<env> peer <ip>`, as specified.
