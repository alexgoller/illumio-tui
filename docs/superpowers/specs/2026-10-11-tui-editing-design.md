# illumio-tui — Design Spec (Stage 4, Plan 4: editing in the TUI)

Date: 2026-10-11
Status: approved in conversation (sections 1–5 of the chat design)
Author: Alex Goller with Claude
Tracks: GitHub issue #1 (stage 3/4 read-write operations)
Builds on: `docs/superpowers/specs/2026-10-10-pce-writes-design.md` (Plan 3)

## 1. Purpose and scope

Plan 4 lets a PCE admin do in the TUI what the CLI can do since Plan 3:
create, edit and delete **labels, IP lists, services and unmanaged
workloads**, through the same `pce::write` guard and planners, with the
exact request shown before it is sent, the same JSONL audit log, and a
visible "writes" marker in the header. It also adds a **pending diff**: the
pending-changes overlay becomes navigable and shows what a draft IP list or
service changes against its active version.

Everything the read-only TUI promises stays true: every request is
user-caused, throttled, counted and visible; nothing is sent twice; no
screen refreshes on its own.

Out of scope (section 10): label groups, rulesets, rules, deny rules,
provisioning, label changes on VEN-managed workloads, multi-line text,
mouse inside forms.

Success:

- With `writes = true` in the profile and `illumio-tui --allow-writes`, the
  header shows `✎ writes`; `n` on the Objects screen opens a form, `Enter`
  shows the exact `POST` with its JSON body, `y` sends it once, the status
  line reports the href and `pending`, and the table reloads once.
- Without either switch the header shows nothing, `n`/`e`/`x` print the
  refusal reason in the status line, and the request counter does not move.
- `e` on an IP list changes only what the user changed: the library's
  `IpListPatch` computes added and removed ranges, so the PUT carries only
  changed keys and keeps unknown fields (Plan 3 rules).
- `Enter` on a pending IP list or service shows added and removed entries in
  green and red after at most one extra GET per kind.
- Every TUI write appears in the audit log with `source: "tui"`.

## 2. Guiding rules (from issue #1 and Plan 3, unchanged)

- Writes are a separate capability, never a side effect of a read.
- Draft is the only policy write target.
- POST, PUT and DELETE are sent exactly once; no retry, ever.
- The confirm overlay is the TUI's dry run: nothing leaves the process
  before `y` on a screen that shows method, path and body.
- Every sent write is appended to the local audit log.
- No new request without a key press that asks for it.
- The CLI, its envelope and exit codes, and all read paths are untouched.

## 3. Runtime: the `writes` capability

### 3.1 What the model knows

```rust
pub struct WritesState {
    /// Both switches on and the audit sink writable.
    pub enabled: bool,
    /// Why not, for the status line: "profile has writes = false",
    /// "started without --allow-writes", "audit log not writable: <path>: <err>".
    pub reason: Option<String>,
}
```

`Model.writes: WritesState` is set at startup and replaced on every profile
switch. `Model::new_connecting` takes it as a sixth argument. The model never
holds the guard: it only knows whether editing keys may start a write flow.

### 3.2 Where the guard lives

`Runtime` gains `writer: Option<Arc<WriteGuard>>`. It is built in `run()`
and in the `:profile` switch closure, from the same inputs the CLI uses:

- `profile_allows = ctx.config.writes` (profile `writes = true` or
  `ILLUMIO_WRITES`);
- `flag_allows = global.allow_writes` (the `--allow-writes` the TUI was
  started with; `:profile` keeps it, like the other global flags);
- audit sink: `JsonlAudit::new(path)` with `pce::write::audit::default_path()`;
  `None` (no `$XDG_STATE_HOME` and no home) becomes `AuditUnwritable`;
- `WriteContext { profile, host, org_id, source: "tui" }`.

`WriteGuard::new` returns `Err(WriteRefused)` when any input is off; the
runtime keeps `writer = None` and sets `WritesState { enabled: false,
reason: Some(refused.to_string()) }`. The library does not change for this:
`WriteGuard` already owns a `Box<dyn AuditSink>` with `Send + Sync`, so it
can be shared through an `Arc` across spawned tasks.

`SwitchResult` becomes `Result<(Pce, ProfileInfo, Limits, Option<WriteGuard>,
WritesState), String>`; `finish_switch` installs all five. A switch that
fails leaves the old client and the old `writes` state in place, as today.

Tests construct the runtime with `Runtime::with_writer(pce, model, guard)`;
`Runtime::new(pce, model)` keeps `writer = None` so every existing runtime
test is unchanged.

### 3.3 Header marker

`render_header` appends ` · ✎ writes` after the glyph, in the theme's warn
colour, when `model.writes.enabled`. Nothing is shown otherwise. The marker
is part of both theme snapshots.

## 4. Editing flow

### 4.1 Keys

On the Objects screen (labels, IP lists, services; label groups are
read-only) and the Workloads screen:

| key | action |
|-----|--------|
| `n` | new object of the current kind |
| `e` | edit the selected row |
| `x` | delete the selected row |

All three first check `model.writes.enabled`; when false the status line
shows the reason as a warning and nothing else happens. `e` and `x` need a
selected row. On the Workloads screen `e` and `x` refuse rows that are not
unmanaged (same rule as the library: `managed == Some(false)` and no `ven`)
with "only unmanaged workloads can be edited here" in the status line, before
any form opens. On label groups all three keys say "label groups are
read-only in this version".

These keys are free on both screens today; the help overlay lists them in
one line each (section 7).

### 4.2 Intents

The screen key handler produces one `EditIntent`, a pure value that names
what the user wants, with the selected object cloned in:

```rust
pub enum EditIntent {
    CreateLabel,
    UpdateLabel(Label),
    DeleteLabel(Label),
    CreateIpList,
    UpdateIpList(IpList),
    DeleteIpList(IpList),
    CreateService,
    UpdateService(Service),
    DeleteService(Service),
    CreateWorkload,
    UpdateWorkload(Workload),
    DeleteWorkload(Workload),
}
```

Create and update intents open a form (4.3). Delete intents skip the form
and go straight to planning (section 5) with the row's name in the
confirmation text.

### 4.3 The form

One generic form struct, in `tui/form.rs`, modelled on `FilterEditor`, held
in `Overlay::Form(Form)`:

```rust
pub struct Form {
    pub title: String,            // "New IP list", "Edit service web"
    pub fields: Vec<Field>,
    pub active: usize,
    pub error: Option<String>,    // last validation failure, shown in red
    pub intent: EditIntent,       // what to build on Enter
}

pub struct Field {
    pub label: &'static str,      // "Name", "Ranges"
    pub value: String,
    pub hint: &'static str,       // "10.0.0.0/8, !10.0.5.0/24"
    pub required: bool,
}
```

Keys inside the form: `Tab`/`Down` next field, `Shift-Tab`/`Up` previous,
printable characters append, `Backspace` deletes, `Ctrl-U` clears the field,
`Enter` validates and builds the write intent, `Esc` closes without a
request. Fields are single-line; values longer than the field width scroll
horizontally so the cursor stays visible.

Fields per kind (edit forms pre-fill from the selected object; the key
of a label and the href of anything are never editable):

| kind | fields |
|------|--------|
| label | Key (create only), Value |
| IP list | Name, Description, Ranges, FQDNs |
| service | Name, Description, Ports |
| workload | Name, Hostname, Description, Addresses, Labels |

List fields are comma-separated text in the CLI grammar from Plan 3, parsed
with the existing `IpRangeSpec::parse`, `PortSpec::parse` and
`InterfaceSpec::parse`:

- Ranges: `10.0.0.0/8, 10.1.0.0-10.1.255.255, !10.0.5.0/24` (`!` marks an
  exclusion). Each range's description is not editable here; edited ranges
  keep theirs because the patch only adds and removes whole ranges.
- FQDNs: `*.corp.example, api.example.com`.
- Ports: `443/tcp, 8000-8100/udp, icmp`. A bare number is refused with the
  same "did you mean 22/tcp" message as the CLI.
- Addresses: `10.0.0.5, 10.0.0.6/24` (interface names are assigned as the
  library does for the CLI: `eth0`, `eth1`, … in order).
- Labels: `app=web, env=prod`, resolved through the loaded name index
  (`Connection::Connected { index }`); an unknown pair is a validation error
  naming the known values of that key. No request is made to resolve a
  label: when the index is not loaded the field says "label index not
  loaded, press r on Objects" and refuses.

### 4.4 Adapters: form to draft or patch

`tui/edit.rs` turns a validated form (or a delete intent) into one
`WriteIntent`, the value `Cmd::PlanWrite` carries:

```rust
pub enum WriteIntent {
    CreateLabel(LabelDraft),
    UpdateLabel { href: String, value: String },
    DeleteLabel { href: String },
    CreateIpList(IpListDraft),
    UpdateIpList { href: String, patch: IpListPatch },
    DeleteIpList { href: String },
    CreateService(ServiceDraft),
    UpdateService { href: String, patch: ServicePatch },
    DeleteService { href: String },
    CreateWorkload(WorkloadDraft),
    UpdateWorkload { href: String, patch: WorkloadPatch },
    DeleteWorkload { href: String },
}
```

The adapters build the drafts and patches as follows:

- Create: `LabelDraft`, `IpListDraft`, `ServiceDraft`, `WorkloadDraft`.
- Update: `IpListPatch`, `ServicePatch`, `WorkloadPatch` are computed
  against the pre-filled object with the same rule as the CLI: a text field
  that still equals the original contributes nothing; a list field is
  diffed into `add_*` and `remove_*` using `IpRangeSpec::same_range`,
  `PortSpec` equality, FQDN string equality and address equality. A label
  edit becomes `plan_update_label(href, value)`. An edit that changes nothing
  closes the form with "no changes" in the status line and sends nothing.
- Workload label edits use `WorkloadPatch::set_labels` with the full
  resolved list, because the field shows the full list.

The adapters are pure functions with unit tests and no PCE access.

## 5. Plan, confirm, execute

Two new commands, each caused by one key press:

```rust
Cmd::PlanWrite(WriteIntent),                 // from the form or a delete key
Cmd::ExecuteWrite { plan: Plan, kind: pce::write::ObjectKind },
```

```rust
Msg::Planned(Result<(Plan, pce::write::ObjectKind), String>),
Msg::WriteDone {
    kind: pce::write::ObjectKind,
    result: Result<Outcome, WriteError>,
},
```

Both result messages join the `task_result` list in `Runtime::apply` so
`busy` returns to zero. `pce::write::ObjectKind` is always written
qualified; the TUI's own `ObjectKind` (the Objects screen tabs) keeps its
name.

### 5.1 `PlanWrite`

The runtime spawns a task that builds `pce.writer(&guard)` and calls the matching
planner (`plan_create_*` with `IfMissing::Create`, `plan_update_*`,
`plan_delete_*`). Planners for IP lists, services and workloads fetch the
current object once (one GET, as in the CLI); label planners make no
request. `Plan::Noop` cannot occur with `IfMissing::Create` and is treated
like an error if it does.

On `Msg::Planned(Ok(..))` the model opens `Overlay::ConfirmWrite`:

```rust
Overlay::ConfirmWrite {
    plan: Plan,
    kind: pce::write::ObjectKind,
    scroll: u16,
}
```

The overlay shows the request summary, `METHOD path`, and the body as
pretty JSON (scrollable with `j`/`k`, `Up`/`Down`). Keys: `y` sends,
`n`/`Esc` cancels with "cancelled, nothing sent" in the status line. Deletes
show `DELETE /orgs/5/sec_policy/draft/ip_lists/9` with no body and the
object name in the summary. An `Err(String)` (validation, 404, ambiguity)
goes to the status line as an error, or to `Overlay::Error` when longer
than the line, exactly like other task failures (`fail`).

### 5.2 `ExecuteWrite`

On `y` the confirm overlay closes, the status line shows "sending …" and
`busy` is up until the result arrives. The task calls `writer.execute(plan)` once. `Msg::WriteDone`:

- `Ok(Outcome::Written(w))`: status line `created <href> (pending)` /
  `updated <href>` / `deleted <href>`; `w.warnings` are appended to the
  message; the table of that kind reloads with `CacheMode::Bypass` (one
  GET). The `Writer` already invalidated the cache prefix, and
  `policy:pending` is invalidated too so the next `p` sees the change.
- `Ok(Outcome::Skipped)`: status line with the reason, no reload.
- `Err(WriteError::Unanswered(_))`, also wrapped in `AuditFailed`:
  `Overlay::Error` with "the request was sent but not answered: " plus the
  error (whose text carries "check the PCE before retrying", and for a
  failed audit the audit log path); the table reloads so the user sees the
  real state.
- Other `Err`: `Overlay::Error` with the error's display (`Rejected` shows
  the PCE's body as the CLI does). Nothing reloads.

The key handler for `y` runs only in `ConfirmWrite`; a second `y` while
`busy` is impossible because the overlay is gone. One write is in flight at
a time (`WritesState.in_flight`, from `Cmd::PlanWrite` or
`Cmd::ExecuteWrite` until `Msg::Planned` or `Msg::WriteDone`): `n` / `e` /
`x` say "a write is already in progress" and `:profile` says "finish or
cancel the write first". A plan that lands while another overlay is open is
discarded ("request discarded, nothing sent"), so keys typed ahead into that
overlay never confirm it.

### 5.3 Guard refusals at run time

`WriteGuard::new` succeeded at startup, so a refusal at execute time can
only be `AuditUnwritable` or a library invariant; it surfaces through the
`Err` path above. The TUI never bypasses the guard.

## 6. Pending diff

### 6.1 Navigable pending overlay

`Overlay::Pending` gains a selection: `PolicyState.pending_selected: usize`,
moved with `j`/`k`, `Up`/`Down`. `Enter` on an entry whose `object_type` is
`ip_lists` or `services` issues `Cmd::LoadDiff { href: String, kind:
pce::write::ObjectKind }` (only `IpList` and `Service` occur); on any other
entry the status line says "rule diffs arrive with Plan 5" (or "no diff for
<object_type>" for label groups and others).

### 6.2 Library

Two read methods, same shape as `ip_lists` and `services`, cached 5 minutes
under their own keys:

```rust
pub async fn active_ip_lists(&self, mode: CacheMode) -> Result<Cached<Vec<IpList>>>  // objects:active_ip_lists
pub async fn active_services(&self, mode: CacheMode) -> Result<Cached<Vec<Service>>> // objects:active_services
```

Path: `sec_policy/active/ip_lists` and `sec_policy/active/services`. The
`objects:` cache prefix already covers them, so a write invalidates them
with the draft lists.

### 6.3 `LoadDiff`

The task loads the draft list and the active list of that kind through the
cache (`CacheMode::Use`); the draft list is usually already cached from the
Objects screen or the name index, so the common cost is one GET for the
active list, and zero when both are fresh. It returns
`Msg::DiffLoaded(Result<ObjectDiff, String>)`:

```rust
pub struct ObjectDiff {
    pub name: String,
    pub href: String,
    pub change: DiffKind,                 // Created, Updated, Deleted
    pub lines: Vec<DiffLine>,
}
pub enum DiffKind { Created, Updated, Deleted }
pub struct DiffLine { pub sign: Sign, pub text: String }  // Sign::{Same, Added, Removed}
```

The diff is computed by a pure function `diff_ip_list(active: Option<&IpList>,
draft: Option<&IpList>)` (and `diff_service`) in `tui/diff.rs`: one line for
name and description when changed (`- old` / `+ new`), one line per range,
FQDN or port that is only in the draft (`+`) or only in the active object
(`-`), unchanged entries listed with `Sign::Same` so the reader sees the
whole object. A created object has no active side: every line is `+`. A
deleted object (`change_type == "delete"`) has no draft side: every line is
`-`. If the draft entry is missing from the list although pending says it
exists, the overlay says so instead of showing an empty diff.

### 6.4 Overlay

`Overlay::Diff { diff: ObjectDiff, scroll: u16 }` renders a box titled
`<kind> <name> · <change>`, `+` lines in the theme's ok colour, `-` lines in
the error colour, unchanged lines dimmed. `j`/`k` scroll, `Esc` returns to
the pending list, `q` closes both. Unchanged `PendingChange` list behaviour
otherwise.

## 7. Help, hints and status

- `HELP_LINES` gains three lines: `n/e/x  new / edit / delete (Objects,
  Workloads; needs writes)`, `Enter  diff of a pending IP list or service`,
  and `y/n  send / cancel a shown request`. The 80x24 budget is exact today:
  the help overlay becomes scrollable (`j`/`k`) and the snapshot test checks
  the first page and the last line after scrolling to the end.
- `screen_hint` on Objects and Workloads adds `n:new e:edit x:del` when
  `writes.enabled`.
- Status messages use the existing `StatusLine::{info,warn,error}`.

## 8. Error handling

| condition | behaviour |
|-----------|-----------|
| writes disabled | status warn with the reason, no request |
| not unmanaged / label group | status warn, no form |
| validation failure in a form | `form.error` in red, field keeps focus |
| no changes in an edit | status info "no changes", form closes |
| planner GET fails (404, 401, timeout) | status error or `Overlay::Error`, no write |
| `Rejected` (400/406/409/422) | `Overlay::Error` with the PCE body, no reload |
| `Unanswered` | `Overlay::Error` with the "sent but not answered" text, reload |
| audit append failure after send | `WriteError::AuditFailed` surfaces as `Overlay::Error` naming the audit path; the write happened |
| profile switch while a form is open | overlay closes, as for every switch today |
| diff: object missing from a list | overlay text "<name> is not in the <draft/active> list" |

Nothing in this plan retries a request.

## 9. Testing

Unit tests (in the `illumio-tui` crate):

- `tui/form.rs`: navigation, editing keys, horizontal scroll, required
  fields.
- `tui/edit.rs`: every adapter: create drafts, patches with add/remove
  lists, "no changes", label resolution through a fixture index, bare port
  refusal, exclusion ranges.
- `tui/diff.rs`: created, updated (ranges added and removed, description
  changed), deleted, missing sides.
- `update.rs`: `n`/`e`/`x` with writes off (status, no `Cmd`), label group
  and managed-workload refusals, form to `Cmd::PlanWrite`, `Planned` to
  `ConfirmWrite`, `y` to `Cmd::ExecuteWrite`, `WriteDone` to reload and
  status, `Enter` on pending entries.

Snapshot tests (`tests/tui_snapshots.rs`, both themes): header marker on,
the IP list form, the confirm overlay with a body, the diff overlay, the
help overlay after scrolling to its end.

Runtime tests (`tests/tui_runtime.rs`, wiremock, `MemoryAudit` or a temp
audit file):

- writes disabled: `n`, `e`, `x` produce zero requests.
- create label: form, `Enter`, `y`; exactly one POST and one labels GET;
  the audit sink holds one entry with `source: "tui"`.
- update IP list: one GET at plan time, one PUT, one list GET; the PUT body
  contains only `ip_ranges`.
- delete service with a 406: one DELETE, `Overlay::Error` shows the body,
  no reload GET.
- cancelled confirm: zero writes.
- pending diff: `p`, `Enter`; exactly one `sec_policy/active/ip_lists` GET,
  a second `Enter` on the same kind issues none.
- `:profile` to a profile without `writes`: marker gone, `n` refused.

Gate: `cargo test --workspace`, `cargo clippy --workspace -D warnings`,
`cargo fmt --check`. The headless smoke against demo100 uses only paths that
send no write: writes-off refusals, a form opened and cancelled, the confirm
overlay cancelled with `n`, and the pending diff.

## 10. Out of scope

Label groups, rulesets, rules, deny rules, provisioning, labels on managed
workloads, per-range descriptions in the form, multi-line descriptions,
mouse inside forms, undo. Plan 5 covers policy objects and provisioning.

## 11. File map

| file | change |
|------|--------|
| `crates/pce/src/lib.rs` | `active_ip_lists`, `active_services` |
| `crates/illumio-tui/src/tui/model.rs` | `WritesState`, `Model.writes`, `Overlay::{Form, ConfirmWrite, Diff}`, `PolicyState.pending_selected` |
| `crates/illumio-tui/src/tui/msg.rs` | `Cmd::{PlanWrite, ExecuteWrite, LoadDiff}`, `Msg::{Planned, WriteDone, DiffLoaded}` |
| `crates/illumio-tui/src/tui/form.rs` (new) | `Form`, `Field`, key handling |
| `crates/illumio-tui/src/tui/edit.rs` (new) | `EditIntent`, `WriteIntent`, adapters |
| `crates/illumio-tui/src/tui/diff.rs` (new) | `ObjectDiff`, `diff_ip_list`, `diff_service` |
| `crates/illumio-tui/src/tui/update.rs` | keys, overlay handling, result messages |
| `crates/illumio-tui/src/tui/app.rs` | `writer`, guard construction, three tasks, `task_result` list |
| `crates/illumio-tui/src/tui/ui/mod.rs`, `ui/widgets/overlay.rs` | header marker, form, confirm, diff, scrollable help |
| `crates/illumio-tui/tests/tui_snapshots.rs`, `tests/tui_runtime.rs` | tests in section 9 |
| `README.md`, `docs/architecture.md`, `docs/index.html` | TUI editing, keys, marker |
