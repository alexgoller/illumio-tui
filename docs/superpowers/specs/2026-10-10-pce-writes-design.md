# illumio-tui — Design Spec (Stage 3, Plan 3: object writes in the library and CLI)

Date: 2026-10-10
Status: approved in conversation (sections 1–3), scope amended to four object kinds
Author: Alex Goller with Claude
Tracks: GitHub issue #1 (stage 3/4 read-write operations)

## 1. Purpose and scope

Plan 3 adds the first writes to illumio-tui: a guarded write layer in the
`pce` library and agent-facing CLI commands that create, update and delete
**labels, IP lists, services and unmanaged workloads**. Everything the
read-only tool promises stays true: every request is user-caused, throttled,
counted, and visible, and nothing is ever sent twice.

Rulesets, rules and deny rules (Plan 4 scope now), the TUI editors and pending
diff, provisioning, and label changes on VEN-managed workloads are out of
scope for this plan (section 7).

Success:

- An agent can run `illumio-tui labels add app=web --allow-writes --yes --json`
  against a profile with `writes = true` and get a stable envelope with the
  new href and `pending: true`; the same command with `--dry-run` prints the
  exact request and sends nothing.
- Every write appears in the local audit log with the body that was sent and
  the status that came back.
- A profile without `writes = true`, or an invocation without `--allow-writes`,
  cannot write, and says so before any request.
- Known PCE write pitfalls are encoded as contract tests, not folklore.

## 2. Guiding rules (from issue #1, unchanged)

- Writes are a separate capability, never a side effect of a read.
- Draft is the only policy write target. Provisioning is a separate stage.
- POST, PUT and DELETE are sent exactly once. No automatic retry, ever.
- `--dry-run` prints the request; `--yes` is required to send it.
- Every sent write is appended to a local audit log.
- The CLI envelope, exit codes 1–5 and all read commands are untouched.

## 3. Library: `pce::write`

### 3.1 Client

`PceClient` gains one method for mutations:

```rust
pub enum WriteMethod { Post, Put, Delete }

pub async fn write(
    &self,
    method: WriteMethod,
    api_path: &str,
    body: Option<&serde_json::Value>,
) -> Result<ApiResponse<serde_json::Value>>
```

- Same limiter, pause-on-429/503 and byte/call accounting as `get`.
- `retry = false`: a 429 or 503 on a write returns the error at once.
- 204 and empty bodies decode to `Value::Null`.
- 400, 406, 409 and 422 map to the new `PceError::Rejected { status, body }`
  (kind `rejected`, exit code 4). `pce_error_message` formats the body as
  today. GETs keep `InvalidFilter`.
- `post` (used by the Explorer query) is unchanged.

### 3.2 Guard

```rust
pub struct WriteGuard { audit: Box<dyn AuditSink> }

impl WriteGuard {
    pub fn new(profile_allows: bool, flag_allows: bool, audit: Box<dyn AuditSink>)
        -> Result<WriteGuard, WriteRefused>;
}

pub enum WriteRefused { ProfileDenies, FlagMissing, AuditUnwritable(String) }
```

`Pce::writer(&self, guard: &WriteGuard) -> Writer<'_>` is the only way to
obtain a `Writer`; no other public function in `pce` sends a write.

### 3.3 Drafts and plans (two-phase)

Each object kind has a draft struct with `validate() -> Result<(), Vec<String>>`
and a crate-private `to_body() -> Value`:

| Draft | Creates / updates | Endpoint (under `/orgs/{org}`) |
| --- | --- | --- |
| `LabelDraft { key, value }` | label | `labels`, `labels/{id}` |
| `IpListDraft { name, description, ranges: Vec<IpRangeSpec>, fqdns }` | IP list | `sec_policy/draft/ip_lists[/{id}]` |
| `ServiceDraft { name, description, ports: Vec<PortSpec> }` | service | `sec_policy/draft/services[/{id}]` |
| `WorkloadDraft { name, hostname, description, interfaces: Vec<InterfaceSpec>, labels: Vec<href> }` | unmanaged workload | `workloads[/{id}]` |

Phase one is pure and never sends anything:

```rust
pub struct WriteRequest { pub method: WriteMethod, pub path: String,
                         pub body: Option<Value>, pub summary: String }

pub enum Plan {
    Send { request: WriteRequest, kind: ObjectKind, invalidates: &'static str },
    Noop { existing_href: String, reason: String },
}
```

`Writer::plan_create(draft, IfMissing)`, `plan_update(href, draft)`,
`plan_delete(href, kind)` build a `Plan`. `IfMissing::Skip` makes the writer
refresh the relevant list with `CacheMode::Bypass`, look for an **exact**
match (label key and value; IP list and service name, case-sensitive;
workload hostname, or name when no hostname was given) and return
`Plan::Noop` when it exists. Name lookups on the PCE (`?name=`, `?key=`,
`?hostname=`) are substring matches, so the exact comparison is always done
client-side.

Phase two sends once:

```rust
pub struct Written {
    pub href: String,
    pub object: Value,          // created/updated object, re-fetched after 204
    pub pending: bool,          // true for policy objects (ip lists, services)
    pub status: u16,
    pub dropped_fields: Vec<String>,
}

pub async fn execute(&self, plan: Plan, source: &str) -> Result<Outcome>
pub enum Outcome { Written(Written), Skipped { existing_href: String } }
```

`execute`:

1. Checks the audit file can be opened for append (else `WriteRefused::AuditUnwritable`, before sending).
2. Sends the request exactly once.
3. Appends one audit entry with the response (section 4). If the request was
   sent but no response arrived (timeout, connection reset), appends a
   `{"phase":"sent"}` entry and returns the error with the text
   `write may have applied; check the PCE before retrying`.
4. Invalidates the cache prefix of the kind (`objects:` for labels, IP lists
   and services, which also drops `objects:index`; `workloads` for workloads).
5. For a 204 or empty body re-fetches the object with one GET on its href.
6. Fills `dropped_fields`: every top-level key that was sent with a non-empty
   value and comes back empty, null or missing. (The PCE answers 201 and
   silently drops a service port list it does not accept.)

### 3.4 Kind-specific rules

- **Labels.** `key` and `value` non-empty, trimmed. Update changes `value`
  only (the PCE does not allow key changes). Delete sends `DELETE labels/{id}`;
  a label in use is a `Rejected` error from the PCE, reported as is.
- **IP lists.** `IpRangeSpec::parse` accepts `A`, `A-B`, `A/N`, IPv4 or IPv6,
  with an `exclusion` flag; the body uses `from_ip`/`to_ip`/`exclusion` as the
  PCE expects. `fqdns` are `{fqdn}` objects. At least one range or FQDN.
  Update is a full `PUT` of the list computed locally from the re-fetched
  object plus the requested adds and removes; a remove that matches nothing is
  a validation error (nothing is sent).
- **Services.** `PortSpec::parse` accepts `P/PROTO`, `P-Q/PROTO`, `icmp`,
  `icmpv6`, `PROTO` with tcp 6, udp 17, icmp 1, icmpv6 58 or a number. The body
  carries exactly one port list key, `service_ports`; `windows_services` is
  never sent. At least one port on create. Update follows the IP list pattern.
- **Unmanaged workloads.** `InterfaceSpec { name, address, cidr_block }`, names
  default to `eth0`, `eth1`, …; addresses parse as IPv4/IPv6 with an optional
  `/N`. At least one interface or a hostname. `labels` are hrefs resolved by
  exact `key=value` through the name index; an unknown label is a validation
  error listing the known values for that key. Create is `POST workloads`
  (201 with the object). Update and delete first re-fetch the workload by href;
  a workload with `managed: true` is refused locally (`invalid`, exit 2) because
  the PCE owns its interfaces and hostname and managed label changes are a
  later plan. Delete sends `DELETE workloads/{id}`. Label updates send the full
  label list (the PCE replaces the array wholesale).
- Hrefs with `/sec_policy/active/` are rewritten to `/draft/`; any other
  `sec_policy` scope is refused.

## 4. Audit log and cache rules (approved section 2)

```rust
pub trait AuditSink: Send + Sync { fn append(&self, entry: &AuditEntry) -> std::io::Result<()>; }
pub struct JsonlAudit { path: PathBuf }   // $XDG_STATE_HOME/illumio-tui/writes.jsonl,
                                           // else ~/.local/state/illumio-tui/writes.jsonl
```

- The file is created with mode 0600 (Unix); one JSON object per line:
  `ts, profile, host, org_id, method, path, body, status, href, error, source`
  (`source` is `cli` or `tui`). A sent-but-unanswered request gets an entry
  with `phase: "sent"` and no status.
- Dry runs and no-ops are not logged. Request bodies never contain
  credentials, so the log is safe to share.
- Cache: `IfMissing` lookups bypass the cache; after a sent write the kind's
  prefix is invalidated; nothing else is refreshed unsolicited.
- A `NullAudit` sink exists for tests.

## 5. CLI commands and contract (approved section 3, amended scope)

### 5.1 Enabling writes

- Profile key `writes = true`; env `ILLUMIO_WRITES=1|true|yes` outranks the
  profile like the other settings; `ResolvedConfig.writes: bool`.
- Global flag `--allow-writes`. Every write command needs both the profile and
  the flag. Missing either is exit code 6 before any request:
  `{"error":{"kind":"write_refused","message":"…","exit_code":6}}`
  (`CliError::Refused`).
- `health` reports `"writes": true|false` (profile setting) so an agent can
  check first.

### 5.2 Common behaviour

- `--dry-run`: `data` is the `WriteRequest` (`method, path, body, summary`),
  `meta.cache = "none"`, no request, no audit entry. A dry run still needs
  writes to be enabled, so the printed request is what would really be sent.
- Without `--dry-run`, `--yes` is required; its absence is exit 2 with
  `re-run with --yes to send`.
- Success: `data = Written { href, pending, object, dropped_fields }`;
  `meta.write = { method, path, status, audit }` (audit is the log path).
  `--if-missing` no-op: `data = { "skipped": true, "href": "…" }`, no
  `meta.write`.
- `--table` renders both as two-column key/value tables.
- Names resolve by exact match through the name index (labels, IP lists,
  services) or a filtered workload list (hostname, then name). No match, or
  more than one, is exit 2 listing the candidates, before any request. An
  argument that starts with `/orgs/` is used as an href unchanged.
- Update commands on IP lists and services compute the full replacement list
  locally from the re-fetched object; the PCE replaces arrays wholesale.

### 5.3 Commands

```
illumio-tui labels add KEY=VALUE [--if-missing]
illumio-tui labels update HREF|KEY=VALUE --value NEW
illumio-tui labels delete HREF|KEY=VALUE

illumio-tui iplists add NAME [--range A[-B|/N]]... [--exclude A[-B|/N]]... [--fqdn F]...
                            [--description D] [--if-missing]
illumio-tui iplists update NAME|HREF [--name NEW] [--description D]
                            [--add-range R]... [--remove-range R]...
                            [--add-exclude R]... [--remove-exclude R]...
                            [--add-fqdn F]... [--remove-fqdn F]...
illumio-tui iplists delete NAME|HREF

illumio-tui services add NAME --port P[-Q]/PROTO... [--description D] [--if-missing]
illumio-tui services update NAME|HREF [--name NEW] [--description D]
                             [--add-port SPEC]... [--remove-port SPEC]...
illumio-tui services delete NAME|HREF

illumio-tui workloads add NAME [--hostname H] [--ip ADDR[/N]]... [--label K=V]...
                              [--description D] [--if-missing]
illumio-tui workloads update HOSTNAME|NAME|HREF [--name N] [--hostname H] [--description D]
                              [--add-ip ADDR[/N]]... [--remove-ip ADDR]...
                              [--set-labels K=V,...] [--add-label K=V]... [--remove-label K[=V]]...
illumio-tui workloads delete HOSTNAME|NAME|HREF
```

`workloads` keeps its listing behaviour when no subcommand is given (clap
`args_conflicts_with_subcommands`); `labels`, `iplists` and `services`
likewise. `--remove-label K` removes whatever label of key `K` the workload
carries; `--set-labels` replaces the whole set and conflicts with the add and
remove flags. `workloads add` without `--hostname` sends `hostname = NAME`.

### 5.4 Exit codes

| Code | Meaning |
| --- | --- |
| 0 | sent and accepted, or dry run, or `--if-missing` no-op |
| 2 | arguments, config, validation, ambiguous or unknown name, missing `--yes`, managed workload |
| 3 | authentication |
| 4 | PCE error, including `rejected` (400/406/409/422) and `write may have applied` |
| 5 | rate limited |
| 6 | write refused by the guard (profile, flag, or unwritable audit file) |

## 6. Testing

- **Unit (pce):** every `validate()` rule; `IpRangeSpec`, `PortSpec`,
  `InterfaceSpec` parsers; `to_body()` snapshots for each kind, including the
  pitfall checks: `service_ports` is the only service list key, IP ranges use
  `from_ip`/`to_ip`/`exclusion`, workload labels are `{href}` only, active hrefs
  are rewritten to draft; `dropped_fields` detection; `NameIndex` exact
  matching.
- **Contract (pce, wiremock):** one test per command path asserting the exact
  method, path, headers and body, the 201/204 handling with re-fetch, the
  `Rejected` mapping with the PCE error array, no retry on 429 and 503 for
  writes (expect exactly one request), the `IfMissing` bypass-and-skip path,
  the sent-but-unanswered audit entry, cache invalidation after a write.
- **Audit:** `JsonlAudit` creates the directory and file with 0600, appends
  one line per write, never logs dry runs; an unwritable path refuses before
  sending (wiremock expects zero requests).
- **CLI (assert_cmd + wiremock):** exit 6 without `--allow-writes` and without
  `writes = true`; exit 2 without `--yes`; `--dry-run` sends nothing and prints
  the request; success envelope shape with `meta.write`; `--if-missing` skip;
  ambiguous name lists candidates; managed workload refused; no output ever
  contains the API secret; `every_flag_has_help_text` extended to the new
  subcommands.
- **Live (opt-in):** `PCE_LIVE_WRITES=1` plus the usual env runs one test that
  creates and deletes a label `illumio-tui-smoke-<timestamp>` and nothing
  else, only against a PCE the operator designates as non-production. It is
  never run against the shared demo PCE, and never in CI.

## 7. Out of scope for Plan 3

- Rulesets, rules and deny rules (next plan, same library pattern).
- TUI editors, the `writes` header marker, and the pending diff (Plan 4).
- Provisioning and revert; label changes on VEN-managed workloads;
  enforcement-mode changes; VEN operations; bulk endpoints.
- Label groups.

## 8. Files

| Area | Files |
| --- | --- |
| client | `crates/pce/src/client.rs` (`write`, `WriteMethod`), `error.rs` (`Rejected`) |
| write layer | `crates/pce/src/write/{mod.rs, guard.rs, audit.rs, drafts.rs, labels.rs, ip_lists.rs, services.rs, workloads.rs}` |
| config | `crates/illumio-tui/src/config.rs` (`writes`), `cli/mod.rs` (`--allow-writes`), `cli/error.rs` (`Refused`) |
| commands | `crates/illumio-tui/src/cli/{objects.rs, workloads.rs}` gain subcommands; `cli/write.rs` shares the guard, dry-run and envelope logic |
| tests | `crates/pce/tests/write_contract.rs`, `crates/illumio-tui/tests/cli_writes.rs`, `crates/pce/tests/live_writes.rs` (opt-in) |
| docs | `README.md` (writes section), `docs/architecture.md`, `docs/index.html` |
