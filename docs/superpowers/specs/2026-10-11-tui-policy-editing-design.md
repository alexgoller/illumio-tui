# illumio-tui — Design Spec (Stage 4, Plan 6: policy editing in the TUI)

Date: 2026-10-11
Status: approved in conversation (sections 1–5 of the chat design)
Author: Alex Goller with Claude
Tracks: GitHub issue #1 (stage 3/4 read-write operations)
Builds on: `docs/superpowers/specs/2026-10-11-tui-editing-design.md` (Plan 4: TUI editing
scaffolding) and `docs/superpowers/specs/2026-10-11-policy-writes-design.md` (Plan 5: policy
planners, token grammar, provisioning)

## 1. Purpose and scope

Plan 6 brings the Plan 5 policy writes into the TUI: rulesets, allow rules and
deny rules are created, edited and deleted on the Policy screen; pending
changes are provisioned from the pending list after the exact change set is
shown; a blocked flow on the Traffic screen becomes a draft allow rule with
its labels and port pre-filled. The guard, the confirm-before-send overlay,
the single send, the audit log and the no-unsolicited-request rule of Plans 4
and 5 are unchanged.

Success:

- On Policy, rules pane, `n` opens a rule form for the selected ruleset;
  `Enter` plans it; the confirm overlay shows `POST
  /orgs/N/sec_policy/draft/rule_sets/<id>/sec_rules` with the exact body;
  `y` sends once; the rulesets reload once.
- In the pending overlay `P` plans a provision of everything (or of the
  marked entries); the confirm overlay shows the `change_subset` body; `y`
  sends once; the pending list and the Policy tables reload.
- On Traffic, `w` on a blocked flow opens a rule form pre-filled with the
  destination's labels as providers, the source's labels as consumers and
  `port/proto` as the service; submitting it plans a rule create in the
  named ruleset.
- With writes off, `n`/`N`/`e`/`x`/`P`/`w` print the reason and send nothing.

## 2. Decisions carried from the chat design

- Deny rules are editable (same form plus an Override field).
- `w` works on a flow row (Flows view or a drilled pair), never on the
  aggregated Blocked rows, which carry names rather than label hrefs.
- TUI forms resolve tokens through the loaded `NameIndex` with zero
  requests; `workload:` tokens are accepted only as hrefs (there is no
  workload index). The message for a `workload:NAME` token is "workload
  names resolve in the CLI; use workload:/orgs/… here".
- The ruleset for a new rule from `w` is typed by name or href and resolved
  with one draft ruleset list read at submit time (the only read a form
  causes). On the Policy screen the ruleset is the selected one.
- Scopes are typed as pipe-separated scopes of comma-separated label or
  label-group tokens: `app=web,env=prod | app=web,env=staging`; empty means
  everywhere (`[[]]`).

## 3. Shared token grammar

`ActorToken`, `ServiceToken`, `parse_scopes`, and the pure href helpers
(`actor_from_href_token`, `service_from_href_token`) move from
`crates/illumio-tui/src/cli/write.rs` into a new crate-level module
`crates/illumio-tui/src/tokens.rs`, used by both the CLI and the TUI. The
CLI's `Lists`/`resolve_*` stay in `cli/write.rs`. The TUI gets
`tui/edit::resolve_actor_tokens(tokens, &NameIndex, org_id) ->
Result<Vec<ActorRef>, String>` and `resolve_service_tokens(...)`: labels via
`label_href`, label groups via `label_group_href`, IP lists via
`ip_list_href`, services via a name lookup over the index's services, `all`,
exclusions, hrefs through the checkers; unknown names produce the Plan 4
style error ("unknown label app=web; known values for app: …", "no IP list
named Corp; similar: …"). No request is made.

## 4. Policy screen editing

### 4.1 Keys

| pane | key | action |
|------|-----|--------|
| rulesets | `n` | new ruleset form |
| rulesets | `e` | edit the selected ruleset |
| rulesets | `x` | delete the selected ruleset (confirm shows its name) |
| rules | `n` | new allow rule in the selected ruleset |
| rules | `N` | new deny rule in the selected ruleset |
| rules | `e` | edit the selected rule (allow or deny by its `RuleKind`) |
| rules | `x` | delete the selected rule |

All keys go through the Plan 4 refusals: writes off (reason in the status
line), a write in flight, no row selected. In the active scope (`d`) the
keys refuse with "switch to draft to edit policy". `N` is bound only on
the rules pane; on the rulesets pane it does nothing.

### 4.2 Forms

Ruleset: Name, Description, Enabled (`y`/`n`), Scopes (grammar of section
2). Edit pre-fills from the `RuleSet` in `PolicyState.raw`; the adapter
builds `RuleSetPatch` with only the changed fields (`set_scopes` only when
the Scopes text changed).

Rule: Providers, Consumers, Services (comma-separated tokens), Description,
Enabled, Unscoped consumers (`y`/`n`). Deny rule: the same minus Unscoped
consumers, plus Override (`y`/`n`). Edit pre-fills from the underlying
`Rule`/`DenyRule` found by href in `PolicyState.raw` (inline) or
`fallback_deny`; actors and services are rendered back to tokens through the
index (`app=web`, `group:Name`, `iplist:Corp`, `all`, `!…`,
`workload:/orgs/…`, `HTTPS`, `443/tcp`, `icmp`); an entry the grammar cannot
express (virtual service, typed ICMP, unknown shape) is shown as its href or
`?` and any change to that field is refused with "this rule has entries the
form cannot edit; use the CLI". Patches are computed text-first against the
pre-filled tokens (Plan 4 rule), producing `RulePatch`/`DenyRulePatch`
add/remove lists; the library carries untouched entries verbatim.

### 4.3 Intents

```rust
EditIntent::{CreateRuleSet, UpdateRuleSet(RuleSet), DeleteRuleSet(RuleSet),
             CreateRule { ruleset_href }, CreateDenyRule { ruleset_href },
             UpdateRule(Rule, ruleset_href), UpdateDenyRule(DenyRule, ruleset_href),
             DeleteRule(RuleView), DeleteDenyRule(RuleView),
             RuleFromFlow(ResolvedFlow)}
WriteIntent::{CreateRuleSet(RuleSetDraft), UpdateRuleSet { href, patch }, DeleteRuleSet { href, name },
              CreateRule { ruleset: RulesetTarget, draft: RuleDraft }, UpdateRule { href, patch }, DeleteRule { href },
              CreateDenyRule { ruleset_href, draft }, UpdateDenyRule { href, patch }, DeleteDenyRule { href },
              Provision(ProvisionRequest)}
enum RulesetTarget { Href(String), Name(String) }
```

`plan_intent` in `app.rs` maps each to the Plan 5 planners; `RulesetTarget::Name`
costs one `rule_sets(Draft, Use)` read before `plan_create_rule`.
`WriteIntent::kind()` maps to `pce::write::ObjectKind::{RuleSet, Rule, DenyRule, Provision}`.
After `WriteDone` for any policy kind the rulesets reload with bypass
(`Cmd::LoadRuleSets { scope: Draft, mode: Bypass }`) and `pending` is
cleared; after a `Provision` the pending list reloads too
(`Cmd::LoadPending(Bypass)`) and the Objects tables are invalidated by the
library's prefix list.

## 5. Provisioning from the pending list

`Overlay::Pending` gains marks: `Space` toggles the selected entry
(`PolicyState.pending_marked: BTreeSet<String>` of hrefs, cleared on reload),
rendered as `[x]`/`[ ]`. `P` plans `WriteIntent::Provision`: with marks,
`ProvisionSubset::Hrefs(marked)`; without, `ProvisionSubset::All`; the
description is `illumio-tui provision`. The confirm overlay shows the exact
`POST /orgs/N/sec_policy` body; `y` sends once; the status line shows
`provisioned <version href>: <counts>`; pending and rulesets reload. A
`Noop` ("nothing pending") goes to the status line. The pending overlay's
help line becomes `j/k select · Space mark · Enter diff · P provision · Esc`.

Ruleset diff: `Enter` on a pending `rule_sets` entry loads the active
ruleset list once (`rule_sets(Active, Use)`, 5-minute TTL) and shows
`Overlay::Diff` with lines for name, description, enabled and scopes
changes (`-`/`+`) and one line per rule added or removed (`+ allow app=web
← iplist:Corp 443/tcp`, by href; a rule present on both sides with a
different one-line summary shows as `-` old `+` new). The pending overlay's
"rule diffs arrive with Plan 5" text is removed.

## 6. Blocked flow to rule

On the Traffic screen, Flows view or a drilled pair, `w` on a selected
`ResolvedFlow` whose `policy_decision` is `Blocked` or `PotentiallyBlocked`
opens the rule form (`EditIntent::RuleFromFlow`) with: Ruleset (empty,
required: name or href), Providers = the destination's labels as `key=value`
tokens, or `iplist:<name>` for each of `dst.ip_lists` when it has no labels,
or `workload:<workload_href>` when it has a workload href and no labels, or
`all` when nothing is known; Consumers = the same for the source; Services =
`port/proto` (`icmp` when there is no port); Description = `allow <src name
or ip> to <dst name or ip> on <port/proto>`; Enabled = `y`; Unscoped
consumers = `y` when the source has no labels. `w` on an allowed flow says
"this flow is allowed; nothing to add". `w` on the Blocked or App-to-app
views says "select a flow in the Flows view (Enter on a pair)". Submitting
resolves the ruleset by name through one draft ruleset list read (or by href
with none) and plans `CreateRule` as in section 4.

## 7. Errors

| condition | behaviour |
|-----------|-----------|
| writes off / in flight / active scope / no selection | status warn, no request |
| token parse or resolution failure | `form.error`, form stays open, no request |
| `workload:NAME` token | `form.error` with the section 2 message |
| rule entries the form cannot express, field changed | `form.error`, no request |
| ruleset name unknown or ambiguous at submit | `form.error` naming similar rulesets after the one read |
| planner GET fails | status error / error overlay as Plan 4 |
| provision rejected (406) | error overlay with the body, no reload |
| unanswered | error overlay, reload |
| `P` with nothing pending | status info "nothing pending" |

## 8. Testing

- Unit: token rendering back from `Rule`/`DenyRule`/`RuleSet` (round trip
  through the index for every token kind, `?` for inexpressible entries),
  `build_intent` for create/update/no-change/refusals, the ruleset diff, the
  flow-to-form pre-fill for each endpoint shape.
- Snapshots (both themes): rule form, deny form, ruleset form, provision
  confirm, pending overlay with marks, ruleset diff, `w` form from the
  Traffic fixture.
- Runtime (wiremock, `MemoryAudit`): rule create from Policy (one POST,
  one rulesets GET); rule edit (one rule GET at plan, one PUT with only
  changed keys, one reload); `w` from a blocked flow (one rulesets GET to
  resolve the name, one POST, one reload); provision with two marks (one
  pending GET, one POST with exactly those hrefs, pending + rulesets
  reload); provision all; writes off: `n`/`N`/`e`/`x`/`P`/`w` zero requests;
  ruleset diff one active GET across two `Enter`s.
- Headless smoke on demo100: writes off refusals; writes on with forms
  opened and cancelled, `P` confirm cancelled with `n`, `w` form cancelled;
  zero writes, zero audit lines; `y` is never pressed.

## 9. Out of scope

Rule reordering, label group writes, virtual service actors, editing on the
aggregated Blocked view, revert of pending changes, per-rule provisioning,
in-field cursor movement (Plan 4 limit).

## 10. File map

| file | change |
|------|--------|
| `crates/illumio-tui/src/tokens.rs` (new) | grammar moved from `cli/write.rs` |
| `crates/illumio-tui/src/cli/write.rs` | uses `crate::tokens` |
| `crates/illumio-tui/src/tui/edit.rs` | new intents, token resolution over the index, forms, adapters, flow pre-fill |
| `crates/illumio-tui/src/tui/diff.rs` | `diff_rule_set`, `rule_set_diff_for` |
| `crates/illumio-tui/src/tui/model.rs` | `pending_marked`, scope refusals |
| `crates/illumio-tui/src/tui/msg.rs` | `WriteIntent` variants flow through `PlanWrite`; `LoadDiff` kind RuleSet |
| `crates/illumio-tui/src/tui/update.rs` | Policy keys `n`/`N`/`e`/`x`, Traffic `w`, pending `Space`/`P`, reloads |
| `crates/illumio-tui/src/tui/app.rs` | `plan_intent` arms, ruleset name read, diff task for rulesets |
| `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` | marks, help lines, hints |
| tests as in section 8; `README.md`, `docs/architecture.md`, `docs/index.html` | TUI policy editing |
