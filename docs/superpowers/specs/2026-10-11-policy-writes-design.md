# illumio-tui — Design Spec (Stage 4, Plan 5: policy writes in the library and CLI)

Date: 2026-10-11
Status: approved in conversation (sections 1–5, container/scope model confirmed)
Author: Alex Goller with Claude
Tracks: GitHub issue #1 (stage 3/4 read-write operations)
Builds on: `docs/superpowers/specs/2026-10-10-pce-writes-design.md` (Plan 3 write layer)

## 1. Purpose and scope

Plan 5 adds the policy writes to the `pce` library and the CLI: **rulesets,
allow rules, deny rules, provisioning**, and **label changes on VEN-managed
workloads**. The same rules as Plan 3 apply: a guarded writer, draft as the
only policy target, one send per confirmation, an audit line per send, the
dry run as the default in the CLI, and every request user-caused and
visible.

The TUI side (rule editing on the Policy screen, provisioning from the
pending list, `w` on a blocked flow) is Plan 6 and is out of scope here.

Success:

- `illumio-tui policy rules add Web-Tier --provider app=web --consumer
  iplist:Corp --service 443/tcp --allow-writes --yes --json` returns the new
  rule's nested href with `pending: true`; with `--dry-run` it prints the
  exact POST under the ruleset's href and sends nothing.
- `illumio-tui policy provision --all --dry-run` prints the exact
  `POST /orgs/N/sec_policy` body listing every pending href by type; with
  `--yes` it sends once and reports the new policy version.
- `illumio-tui workloads update <host> --add-label env=prod --managed-labels`
  relabels a VEN-managed workload with a PUT that carries only `labels`;
  without `--managed-labels`, or with any other change, it is refused before
  any request.
- A rule round trip never loses fields the models do not know.

## 2. Data model (confirmed)

- A **ruleset** is the container. Its `scopes` is a list of scopes: the
  outer list is the alternatives, each inner list is one scope made of
  labels or label groups, at most one per label dimension. An empty outer
  list means the ruleset applies everywhere.
- A **rule** or **deny rule** exists only inside a ruleset. Its href is
  nested: `/orgs/N/sec_policy/draft/rule_sets/<id>/sec_rules/<id>` or
  `.../deny_rules/<id>`. Create is a POST under the ruleset's href; update
  and delete address the nested href. A rule cannot move between rulesets.
- Providers are evaluated inside the ruleset's scopes; `unscoped_consumers`
  lets consumers come from outside them. A rule draft carries no scopes.
- A ruleset PUT never carries `rules` or `deny_rules`.
- Provisioning works on draft policy objects (rulesets, IP lists,
  services, label groups and any other type the pending list names): a
  changed rule shows up as its ruleset being pending.

## 3. Library

### 3.1 Models

`Rule`, `DenyRule`, `RuleSet` and `Actor` gain `#[serde(flatten)] extra:
Map<String, Value>` so a round trip carries fields the models do not name
(`sec_connect`, `stateless`, `machine_auth`, `network_type`, future ones).
`IngressService` is unchanged (`Ref` or inline `{port, to_port, proto}`).

### 3.2 Object kinds and cache

```rust
pub enum ObjectKind { Label, IpList, Service, Workload, RuleSet, Rule, DenyRule, Provision }
impl ObjectKind {
    pub fn cache_prefixes(self) -> &'static [&'static str];
    // Label/IpList/Service: ["objects:"]; Workload: ["workloads"];
    // RuleSet/Rule/DenyRule: ["policy:"]; Provision: ["policy:", "objects:"]
    pub fn pending(self) -> bool; // IpList, Service, RuleSet, Rule, DenyRule: true; Provision, Label, Workload: false
}
```

`Writer::execute` invalidates every prefix of the kind. The existing
`cache_prefix` callers move to `cache_prefixes`.

### 3.3 Hrefs

```rust
pub fn checked_nested_href(href: &str, parent: &str, child: &str, org_id: u64) -> Result<String, WriteError>
```

Accepts only `/orgs/<org>/sec_policy/<draft|active>/<parent>/<id>/<child>/<id>`,
each id `[A-Za-z0-9-]+`, strict by component (no `..`, no empty segment),
`active` rewritten to `draft`, a foreign org refused. `checked_href` is
unchanged and keeps refusing nested paths. `parent_href(nested) -> String`
returns the ruleset href of a nested href.

### 3.4 Actors and services in drafts

```rust
pub enum ActorKind { Label, LabelGroup, IpList, Workload, All }
pub struct ActorRef { pub kind: ActorKind, pub href: Option<String>, pub exclusion: bool }
// All has no href and serialises as {"actors": "ams"}; exclusion only with Label/LabelGroup/IpList
pub enum ServiceRef { Href(String), Port(PortSpec) }
```

`ActorRef::to_actor() -> Actor` and `ActorRef::from_actor(&Actor) ->
Result<ActorRef, String>` (virtual services and virtual servers are
refused: "virtual_service actors are not supported in this version"; an
actor that matches an existing entry is matched by `same_actor`, which
compares kind, href and exclusion). `ServiceRef::to_ingress()` /
`from_ingress()`; equality for inline ports via `PortSpec`.

### 3.5 Ruleset planners (`write/rule_sets.rs`)

```rust
pub struct RuleSetDraft { pub name: String, pub description: Option<String>, pub enabled: bool, pub scopes: Vec<Vec<ActorRef>> }
pub struct RuleSetPatch { pub name: Option<String>, pub description: Option<String>, pub enabled: Option<bool>, pub set_scopes: Option<Vec<Vec<ActorRef>>> }
impl Writer<'_> {
    pub async fn plan_create_rule_set(&self, draft: &RuleSetDraft, if_missing: IfMissing) -> Result<Plan, WriteError>;
    pub async fn plan_update_rule_set(&self, href: &str, patch: &RuleSetPatch) -> Result<Plan, WriteError>;
    pub fn plan_delete_rule_set(&self, href: &str) -> Result<Plan, WriteError>;
}
```

- Validation: name non-empty; every scope actor is a label or a label group
  (anything else: "scopes take labels and label groups only"); no two
  labels of the same key in one scope is not checked locally (the index is
  not available in the library) and left to the PCE.
- `SkipExisting` looks the name up in the draft ruleset list (one GET,
  exact case-insensitive match, pending deletes skipped); 1 → `Noop`,
  more → `Invalid`.
- Create: `POST sec_policy/draft/rule_sets` with `{name, description?,
  enabled, scopes}`; `scopes: [[]]` (one empty scope) when unscoped, which
  is how a real PCE returns an unscoped ruleset.
- Update: `checked_href(href, "rule_sets")`, one GET, body holds only the
  changed keys; `set_scopes` replaces the whole list; `rules` and
  `deny_rules` never appear.
- Delete: `DELETE <href>`; summary `delete rule_set <name|href>`.

### 3.6 Rule and deny-rule planners (`write/rules.rs`)

```rust
pub struct RuleDraft {
    pub description: Option<String>, pub enabled: bool,
    pub providers: Vec<ActorRef>, pub consumers: Vec<ActorRef>, pub services: Vec<ServiceRef>,
    pub unscoped_consumers: bool, pub resolve_labels_as: Option<ResolveLabelsAs>,
}
pub struct RulePatch {
    pub description: Option<String>, pub enabled: Option<bool>,
    pub add_providers: Vec<ActorRef>, pub remove_providers: Vec<ActorRef>,
    pub add_consumers: Vec<ActorRef>, pub remove_consumers: Vec<ActorRef>,
    pub add_services: Vec<ServiceRef>, pub remove_services: Vec<ServiceRef>,
    pub unscoped_consumers: Option<bool>, pub resolve_labels_as: Option<ResolveLabelsAs>,
}
pub struct DenyRuleDraft { pub description: Option<String>, pub enabled: bool, pub providers: Vec<ActorRef>, pub consumers: Vec<ActorRef>, pub services: Vec<ServiceRef>, pub override_: bool }
pub struct DenyRulePatch {
    pub description: Option<String>, pub enabled: Option<bool>,
    pub add_providers: Vec<ActorRef>, pub remove_providers: Vec<ActorRef>,
    pub add_consumers: Vec<ActorRef>, pub remove_consumers: Vec<ActorRef>,
    pub add_services: Vec<ServiceRef>, pub remove_services: Vec<ServiceRef>,
    pub override_: Option<bool>,
}
impl Writer<'_> {
    pub fn plan_create_rule(&self, ruleset_href: &str, draft: &RuleDraft) -> Result<Plan, WriteError>;
    pub async fn plan_update_rule(&self, href: &str, patch: &RulePatch) -> Result<Plan, WriteError>;
    pub fn plan_delete_rule(&self, href: &str) -> Result<Plan, WriteError>;
    pub fn plan_create_deny_rule(&self, ruleset_href: &str, draft: &DenyRuleDraft) -> Result<Plan, WriteError>;
    pub async fn plan_update_deny_rule(&self, href: &str, patch: &DenyRulePatch) -> Result<Plan, WriteError>;
    pub fn plan_delete_deny_rule(&self, href: &str) -> Result<Plan, WriteError>;
}
```

- Validation: at least one provider, one consumer and one service; `All`
  cannot be excluded; a draft with `resolve_labels_as` must name both
  sides (`providers` and `consumers` each a non-empty list of
  `workloads`/`virtual_services`); default when absent is omitted from the
  body so the PCE applies its default.
- Create: `checked_href(ruleset_href, "rule_sets")`, then `POST
  <ruleset>/sec_rules` (or `/deny_rules`) with the draft's body. No GET.
- Update: `checked_nested_href(href, "rule_sets", "sec_rules"|"deny_rules")`,
  one GET of the rule, patch applied against it: a text or bool field equal
  to the current value contributes nothing; actor and service lists are
  diffed with `same_actor`/service equality, a removal of an absent entry
  is `Invalid`, and a changed list is sent whole with the untouched entries
  carried as the PCE returned them (their `extra` included). An empty
  patch is `Invalid` before any request.
- Delete: `DELETE <nested href>`; summary `delete rule <href>`.
- `Written.href` for a create is the nested href the PCE returns; `Written.pending` is true.

### 3.7 Provisioning (`write/provision.rs`)

```rust
pub enum ProvisionSubset { All, Hrefs(Vec<String>) }
pub struct ProvisionRequest { pub description: Option<String>, pub subset: ProvisionSubset }
pub fn provision_counts(body: &Value) -> BTreeMap<String, usize> // object_type → count, from `change_subset`
impl Writer<'_> {
    pub async fn plan_provision(&self, req: &ProvisionRequest) -> Result<Plan, WriteError>;
}
```

- One GET of `sec_policy/pending` (through the cache with `Bypass`, so the
  plan reflects the PCE now). `All` becomes the explicit list of pending
  hrefs; `Hrefs` must each be in the pending list (else `Invalid` naming
  the href) and are normalised through `checked_href` for their kind.
  An empty resulting set is `Noop { reason: "nothing pending" }`.
- Body: `{"update_description": <description or "illumio-tui provision">,
  "change_subset": {"rule_sets": [{"href": ..}], "ip_lists": [..],
  "services": [..], "label_groups": [..], ...}}`, one key per pending
  object type (grouped generically by the href's type segment, so `All`
  always means all; the PCE decides which types it accepts), only
  non-empty keys.
  Rule hrefs never appear: a pending rule is provisioned through its
  ruleset, and `All` already lists the ruleset.
- `POST sec_policy`; kind `Provision`; summary `provision 3 rule_sets, 1 ip_list`
  (built from `provision_counts`).
- The response body (the new version, `href: /orgs/N/sec_policy/<version>`)
  is `Written.object`; `pending` is false. The audit entry carries the body.
- Reverting pending changes is out of scope.

### 3.8 Managed workload labels (`write/workloads.rs`)

```rust
pub struct LabelsPatch { pub set_labels: Option<Vec<String>>, pub add_labels: Vec<String>, pub remove_labels: Vec<LabelRemoval> }
impl Writer<'_> {
    pub async fn plan_relabel_workload(&self, href: &str, patch: &LabelsPatch, idx: &NameIndex) -> Result<Plan, WriteError>;
}
```

One GET of the workload, no managed check, PUT with only `labels`
(hrefs). An empty patch, or a `set_labels` together with add/remove, is
`Invalid`. `plan_update_workload` keeps refusing managed workloads for every
other change.

## 4. CLI

### 4.1 Commands

`PolicyCmd` gains write variants, each flattening `WriteFlags`:

```
policy rulesets add <name> [--description D] [--disabled] [--scope L[,L..]]... [--if-missing]
policy rulesets update <ruleset> [--name N] [--description D] [--enabled|--disabled] [--scope ...]... [--clear-scopes]
policy rulesets delete <ruleset>
policy rules add <ruleset> --provider A... --consumer A... --service S... [--description D] [--disabled] [--unscoped-consumers] [--resolve-providers-as workloads|virtual_services] [--resolve-consumers-as ...]
policy rules update <rule-href> [--description D] [--enabled|--disabled] [--add-provider A]... [--remove-provider A]... [--add-consumer A]... [--remove-consumer A]... [--add-service S]... [--remove-service S]... [--unscoped-consumers|--scoped-consumers]
policy rules delete <rule-href>
policy deny-rules add <ruleset> --provider A... --consumer A... --service S... [--description D] [--disabled] [--override]
policy deny-rules update <deny-href> [...same list flags...] [--override|--no-override]
policy deny-rules delete <deny-href>
policy provision (--all | <href|ruleset-name>...) [--description D]
workloads update <workload> ... --managed-labels
```

`<ruleset>` is a name or href, resolved with one draft ruleset list GET
(`find_ruleset`: exact href or case-insensitive name; ambiguity refused).
Rules and deny rules are addressed by href only. `--scope` with no value
is not allowed; `--clear-scopes` empties the list. `--managed-labels` is
accepted only when the patch is labels-only; otherwise exit 2 with
"--managed-labels allows label changes only".

### 4.2 Actor and service grammar

| token | meaning |
|-------|---------|
| `app=web` or `label:app=web` | a label, resolved through one labels GET |
| `group:Name` | a label group, resolved through one label-groups GET |
| `iplist:Corp` | an IP list (draft list GET) |
| `workload:HOST` | a workload, by hostname then name (`find_workload`, max 500) |
| `all` | every workload (`actors: ams`) |
| `!<token>` | exclusion (labels, label groups, IP lists only) |
| `HTTPS` / `service:HTTPS` | a service by name (draft list GET) |
| `443/tcp`, `8000-8100/udp`, `icmp` | an inline service via `PortSpec::parse` |
| `/orgs/N/...` | an href of the matching kind, checked, no request |

Each kind's list is fetched at most once per invocation and only when a
token needs it. Unknown names fail with the Plan 3 style messages (known
values, `similar:` up to 5) before any write.

### 4.3 Output and exit codes

Unchanged envelope: `--dry-run` prints the `WriteRequest`; a send prints
`Written` with `meta.write`; `Skipped` for `Noop`. Exit codes as Plan 3
(2 invalid, 4 unanswered, 6 refused). `policy provision` adds a `counts`
map to the dry-run and written outputs (`provision_counts` over the request
body's `change_subset`) and prints `provisioned <version href>` in table mode.

## 5. Errors

| condition | behaviour |
|-----------|-----------|
| rule with no provider, consumer or service | `Invalid`, exit 2, no request |
| scope actor that is not a label or label group | `Invalid` before any request |
| href of the wrong shape (flat where nested expected or vice versa) | `Invalid` naming the expected shape |
| ruleset name ambiguous or unknown | exit 2 listing candidates / similar names |
| provision href not pending | `Invalid` naming it; nothing sent |
| nothing pending with `--all` | `Skipped`, exit 0 |
| managed workload, non-label change with `--managed-labels` | exit 2 before any request |
| PCE rejects (400/406/409/422) | `Rejected` with the body, exit as Plan 3 |
| timeout/network/5xx after send | `Unanswered`, exit 4, audit `phase: sent` |

## 6. Testing

- `crates/pce/tests/write_contract.rs` (or a new `policy_writes.rs`):
  nested href acceptance and refusals (dot segments, wrong parent, active
  rewrite, foreign org); ruleset create body with two scopes; ruleset
  update sends only changed keys and never `rules`; rule create under the
  ruleset href with mixed actors, an exclusion, an `all` consumer, a
  service href and an inline port; rule update: add one provider keeps
  the other providers' `extra` fields and sends `providers` whole;
  removing an absent consumer is `Invalid`; deny rule with `override`;
  provision `All` from `pending_dict.json`/`pending_list.json` produces
  the grouped body with no rule hrefs; provision of a non-pending href is
  `Invalid`; relabel of a managed workload sends only `labels`; managed
  non-label update still refused; extras survive a `Rule` round trip.
- `crates/illumio-tui/tests/cli_writes.rs`: each subcommand's dry run
  body; the actor grammar incl. `!` and `all`; one list GET per kind used;
  ruleset by name and by href; `--managed-labels` acceptance and refusal;
  the guard table extended with the new commands; provision `--all` and
  explicit hrefs; table and JSON outputs.
- `tests/live_writes.rs` gains ruleset/rule/provision cases, opt-in,
  never run against demo100.
- Smoke on demo100: dry runs only (`policy provision --all --dry-run`,
  `policy rules add ... --dry-run`), request counts recorded.

## 7. Out of scope

TUI editing of policy (Plan 6), reverting pending changes, virtual
service and virtual server actors, label group writes, editing
`sec_connect`/`stateless`/`machine_auth`/`network_type` (carried through
`extra`), rule reordering, per-rule provisioning.

## 8. File map

| file | change |
|------|--------|
| `crates/pce/src/models/policy.rs` | `extra` on `Rule`, `DenyRule`, `RuleSet`, `Actor` |
| `crates/pce/src/write/mod.rs` | `ObjectKind::{RuleSet, Rule, DenyRule, Provision}`, `cache_prefixes`, `checked_nested_href`, `parent_href` |
| `crates/pce/src/write/actors.rs` (new) | `ActorKind`, `ActorRef`, `ServiceRef`, conversions |
| `crates/pce/src/write/rule_sets.rs` (new) | ruleset draft/patch/planners |
| `crates/pce/src/write/rules.rs` (new) | rule and deny-rule drafts/patches/planners |
| `crates/pce/src/write/provision.rs` (new) | provisioning |
| `crates/pce/src/write/workloads.rs` | `LabelsPatch`, `plan_relabel_workload` |
| `crates/pce/src/resolve.rs` | `label_group_href(name)` |
| `crates/illumio-tui/src/cli/policy.rs` | write subcommands, provision |
| `crates/illumio-tui/src/cli/write.rs` | actor/service token resolution, `resolve_label_group`, `resolve_workload` |
| `crates/illumio-tui/src/cli/workloads.rs` | `--managed-labels` |
| tests as in section 6; `README.md`, `docs/architecture.md`, `docs/index.html` | policy writes, provisioning |
