//! Editing in the TUI. A key produces an [`EditIntent`]; [`start`] turns it
//! into a pre-filled [`Form`] (create, update) or straight into a
//! [`WriteIntent`] (delete); [`build_intent`] turns a submitted form into the
//! draft or patch the library plans. Pure: nothing here reaches the PCE.
use crate::tokens::{
    ActorToken, ServiceToken, actor_from_href_token, service_from_href_token, similar, split_kv,
    with_exclusion, with_similar,
};
use crate::tui::form::{Field, Form};
use pce::NameIndex;
use pce::models::{Actor, Fqdn, IngressService, Interface, IpList, Label, Service, Workload};
use pce::write::{
    ActorKind, ActorRef, InterfaceSpec, IpListDraft, IpListPatch, IpRangeSpec, LabelDraft,
    PortSpec, ServiceDraft, ServicePatch, ServiceRef, WorkloadDraft, WorkloadPatch, WriteRequest,
};

/// What `n`, `e` or `x` on Objects or Workloads asks for.
#[derive(Debug, Clone, PartialEq)]
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

/// What `Cmd::PlanWrite` carries: the library draft or patch, or the href
/// (and, for the confirm text, the name) of the object to delete.
#[derive(Debug, Clone, PartialEq)]
pub enum WriteIntent {
    CreateLabel(LabelDraft),
    UpdateLabel { href: String, value: String },
    DeleteLabel { href: String, name: String },
    CreateIpList(IpListDraft),
    UpdateIpList { href: String, patch: IpListPatch },
    DeleteIpList { href: String, name: String },
    CreateService(ServiceDraft),
    UpdateService { href: String, patch: ServicePatch },
    DeleteService { href: String, name: String },
    CreateWorkload(WorkloadDraft),
    UpdateWorkload { href: String, patch: WorkloadPatch },
    DeleteWorkload { href: String, name: String },
}

impl WriteIntent {
    pub fn kind(&self) -> pce::write::ObjectKind {
        match self {
            WriteIntent::CreateLabel(_)
            | WriteIntent::UpdateLabel { .. }
            | WriteIntent::DeleteLabel { .. } => pce::write::ObjectKind::Label,
            WriteIntent::CreateIpList(_)
            | WriteIntent::UpdateIpList { .. }
            | WriteIntent::DeleteIpList { .. } => pce::write::ObjectKind::IpList,
            WriteIntent::CreateService(_)
            | WriteIntent::UpdateService { .. }
            | WriteIntent::DeleteService { .. } => pce::write::ObjectKind::Service,
            WriteIntent::CreateWorkload(_)
            | WriteIntent::UpdateWorkload { .. }
            | WriteIntent::DeleteWorkload { .. } => pce::write::ObjectKind::Workload,
        }
    }
}

/// Where an editing key leads: a form to fill, or (delete) a plan at once.
#[derive(Debug)]
pub enum Start {
    Form(Form),
    Plan(WriteIntent),
}

const RANGES_HINT: &str = "10.0.0.0/8, 10.1.0.0-10.1.255.255, !10.0.5.0/24";
const FQDNS_HINT: &str = "*.corp.example, api.example.com";
const PORTS_HINT: &str = "443/tcp, 8000-8100/udp, icmp";
const ADDRESSES_HINT: &str = "10.0.0.5, 10.0.0.6/24";
const LABELS_HINT: &str = "app=web, env=prod";

/// The form for a create or update (pre-filled from the object; the key of a
/// label and the href of anything are never editable), or the delete intent.
pub fn start(intent: EditIntent, index: Option<&NameIndex>) -> Start {
    let form = |title: String, fields: Vec<Field>, intent: EditIntent| {
        Start::Form(Form::new(title, fields, intent))
    };
    match intent {
        EditIntent::CreateLabel => form(
            "New label".into(),
            vec![
                Field::new("Key", "", "app, env, role, loc or your own", true),
                Field::new("Value", "", "web", true),
            ],
            EditIntent::CreateLabel,
        ),
        EditIntent::UpdateLabel(l) => {
            let title = format!("Edit label {}", l.display());
            let fields = vec![Field::new("Value", l.value.clone(), "web", true)];
            form(title, fields, EditIntent::UpdateLabel(l))
        }
        EditIntent::DeleteLabel(l) => Start::Plan(WriteIntent::DeleteLabel {
            name: l.display(),
            href: l.href,
        }),
        EditIntent::CreateIpList => form(
            "New IP list".into(),
            ip_list_fields(None),
            EditIntent::CreateIpList,
        ),
        EditIntent::UpdateIpList(l) => {
            let title = format!("Edit IP list {}", l.name);
            let fields = ip_list_fields(Some(&l));
            form(title, fields, EditIntent::UpdateIpList(l))
        }
        EditIntent::DeleteIpList(l) => Start::Plan(WriteIntent::DeleteIpList {
            name: l.name,
            href: l.href,
        }),
        EditIntent::CreateService => form(
            "New service".into(),
            service_fields(None),
            EditIntent::CreateService,
        ),
        EditIntent::UpdateService(s) => {
            let title = format!("Edit service {}", s.name);
            let fields = service_fields(Some(&s));
            form(title, fields, EditIntent::UpdateService(s))
        }
        EditIntent::DeleteService(s) => Start::Plan(WriteIntent::DeleteService {
            name: s.name,
            href: s.href,
        }),
        EditIntent::CreateWorkload => form(
            "New workload".into(),
            workload_fields(None, index),
            EditIntent::CreateWorkload,
        ),
        EditIntent::UpdateWorkload(w) => {
            let title = format!("Edit workload {}", w.display_name());
            let fields = workload_fields(Some(&w), index);
            form(title, fields, EditIntent::UpdateWorkload(w))
        }
        EditIntent::DeleteWorkload(w) => Start::Plan(WriteIntent::DeleteWorkload {
            name: w.display_name(),
            href: w.href,
        }),
    }
}

/// The form a create or update intent opens; `None` for a delete.
pub fn form_for(intent: EditIntent, index: Option<&NameIndex>) -> Option<Form> {
    match start(intent, index) {
        Start::Form(f) => Some(f),
        Start::Plan(_) => None,
    }
}

fn ip_list_fields(l: Option<&IpList>) -> Vec<Field> {
    vec![
        Field::new(
            "Name",
            l.map(|l| l.name.clone()).unwrap_or_default(),
            "Corp",
            true,
        ),
        Field::new(
            "Description",
            l.and_then(|l| l.description.clone()).unwrap_or_default(),
            "optional",
            false,
        ),
        Field::new(
            "Ranges",
            l.map(ranges_text).unwrap_or_default(),
            RANGES_HINT,
            false,
        ),
        Field::new(
            "FQDNs",
            l.map(fqdns_text).unwrap_or_default(),
            FQDNS_HINT,
            false,
        ),
    ]
}

fn service_fields(s: Option<&Service>) -> Vec<Field> {
    vec![
        Field::new(
            "Name",
            s.map(|s| s.name.clone()).unwrap_or_default(),
            "Web",
            true,
        ),
        Field::new(
            "Description",
            s.and_then(|s| s.description.clone()).unwrap_or_default(),
            "optional",
            false,
        ),
        Field::new(
            "Ports",
            s.map(ports_text).unwrap_or_default(),
            PORTS_HINT,
            true,
        ),
    ]
}

fn workload_fields(w: Option<&Workload>, index: Option<&NameIndex>) -> Vec<Field> {
    let text = |f: fn(&Workload) -> Option<String>| w.and_then(f).unwrap_or_default();
    vec![
        // A workload may have no name (hostname only); only a new one needs it.
        Field::new("Name", text(|w| w.name.clone()), "printer-1", w.is_none()),
        Field::new(
            "Hostname",
            text(|w| w.hostname.clone()),
            "defaults to the name",
            false,
        ),
        Field::new(
            "Description",
            text(|w| w.description.clone()),
            "optional",
            false,
        ),
        Field::new(
            "Addresses",
            w.map(addresses_text).unwrap_or_default(),
            ADDRESSES_HINT,
            false,
        ),
        Field::new(
            "Labels",
            w.map(|w| labels_text(w, index)).unwrap_or_default(),
            LABELS_HINT,
            false,
        ),
    ]
}

fn join(items: impl Iterator<Item = String>) -> String {
    items.collect::<Vec<_>>().join(", ")
}

fn ranges_text(l: &IpList) -> String {
    join(
        l.ip_ranges
            .iter()
            .map(|r| IpRangeSpec::from_range(r).display()),
    )
}

fn fqdns_text(l: &IpList) -> String {
    join(l.fqdns.iter().map(|f| f.fqdn.clone()))
}

fn ports_text(s: &Service) -> String {
    join(s.service_ports.iter().map(|p| p.display()))
}

fn addresses_text(w: &Workload) -> String {
    join(w.interfaces.iter().map(interface_text))
}

/// `ADDR/N` or `ADDR`, spelled as the PCE spells the address.
fn interface_text(i: &Interface) -> String {
    match i.cidr_block {
        Some(c) => format!("{}/{c}", i.address),
        None => i.address.clone(),
    }
}

/// `key=value` from the workload's own label entry, else from the index,
/// else the href (which `resolve_labels` accepts back as it is).
fn labels_text(w: &Workload, index: Option<&NameIndex>) -> String {
    join(w.labels.iter().map(|l| {
        match (&l.key, &l.value) {
            (Some(k), Some(v)) => format!("{k}={v}"),
            _ => index
                .and_then(|i| i.label(&l.href))
                .map(Label::display)
                .unwrap_or_else(|| l.href.clone()),
        }
    }))
}

/// Comma-separated entries, trimmed, empty ones dropped.
fn items(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

fn non_empty(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

/// `Some(new)` when a text field no longer equals the original.
fn changed(old: &str, new: String) -> Option<String> {
    (new != old.trim()).then_some(new)
}

/// Ranges in the CLI grammar (`!` marks an exclusion). An entry spelled
/// exactly as an existing range keeps that range, description and unknown
/// fields included.
fn parse_ranges(text: &str, existing: &[IpRangeSpec]) -> Result<Vec<IpRangeSpec>, String> {
    items(text)
        .iter()
        .map(|t| {
            if let Some(e) = existing.iter().find(|e| e.display() == *t) {
                return Ok(e.clone());
            }
            let (exclusion, body) = match t.strip_prefix('!') {
                Some(rest) => (true, rest.trim()),
                None => (false, t.as_str()),
            };
            IpRangeSpec::parse(body, exclusion).map_err(|e| format!("Ranges: {e}"))
        })
        .collect()
}

/// Ports added and removed by the edited list. An entry spelled exactly as
/// an existing port keeps it, so ports this tool cannot parse back (`any`,
/// ICMP types) survive an edit that does not touch them.
fn port_changes(s: &Service, tokens: &[String]) -> Result<(Vec<PortSpec>, Vec<PortSpec>), String> {
    let existing: Vec<Option<PortSpec>> = s
        .service_ports
        .iter()
        .map(PortSpec::from_service_port)
        .collect();
    let mut kept: Vec<PortSpec> = Vec::new();
    let mut add: Vec<PortSpec> = Vec::new();
    for t in tokens {
        if let Some(i) = s.service_ports.iter().position(|p| p.display() == *t) {
            if let Some(spec) = &existing[i] {
                kept.push(spec.clone());
            }
            continue;
        }
        let spec = PortSpec::parse(t).map_err(|e| format!("Ports: {e}"))?;
        if existing.iter().flatten().any(|o| *o == spec) {
            kept.push(spec);
        } else if !add.contains(&spec) {
            add.push(spec);
        }
    }
    let mut remove = Vec::new();
    for (p, spec) in s.service_ports.iter().zip(&existing) {
        if tokens.contains(&p.display()) {
            continue;
        }
        match spec {
            Some(spec) if !kept.contains(spec) => remove.push(spec.clone()),
            Some(_) => {}
            None => {
                return Err(format!(
                    "Ports: {} has no protocol and cannot be removed here; edit it in the PCE UI",
                    p.display()
                ));
            }
        }
    }
    Ok((add, remove))
}

/// Addresses added (as typed) and removed (by address). An entry spelled
/// exactly as shown keeps its interface, whatever the PCE's spelling; a
/// typed address that exists keeps its interface too, and with a new prefix
/// it is an add, which `WorkloadPatch::apply` turns into a prefix change of
/// that interface. Dropping a prefix has no such path and is refused.
fn address_changes(w: &Workload, tokens: &[String]) -> Result<(Vec<String>, Vec<String>), String> {
    let mut kept = vec![false; w.interfaces.len()];
    let mut add = Vec::new();
    for t in tokens {
        if let Some(n) = w.interfaces.iter().position(|i| interface_text(i) == *t) {
            kept[n] = true;
            continue;
        }
        let spec = InterfaceSpec::parse(t, String::new()).map_err(|e| format!("Addresses: {e}"))?;
        let Some(n) = w.interfaces.iter().position(|i| i.address == spec.address) else {
            add.push(t.clone());
            continue;
        };
        kept[n] = true;
        match (spec.cidr_block, w.interfaces[n].cidr_block) {
            (None, Some(_)) => {
                return Err("Addresses: removing a prefix is not supported here; \
                     remove the address and add it again"
                    .into());
            }
            (Some(new), old) if old != Some(new) => add.push(t.clone()),
            _ => {}
        }
    }
    let remove = w
        .interfaces
        .iter()
        .zip(&kept)
        .filter(|(_, k)| !**k)
        .map(|(i, _)| i.address.clone())
        .collect();
    Ok((add, remove))
}

/// `key=value` pairs (or hrefs) to label hrefs through the loaded index. No
/// request is ever made to resolve a label.
fn resolve_labels(text: &str, index: Option<&NameIndex>) -> Result<Vec<String>, String> {
    let wanted = items(text);
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    let Some(idx) = index else {
        return Err("Labels: label index not loaded, press r on Objects".into());
    };
    wanted
        .iter()
        .map(|t| {
            if t.starts_with("/orgs/") {
                return Ok(t.clone());
            }
            let (k, v) = t
                .split_once('=')
                .ok_or_else(|| format!("Labels: {t:?} is not key=value"))?;
            let (k, v) = (k.trim(), v.trim());
            idx.label_href(k, v).map(str::to_string).ok_or_else(|| {
                let known = idx.values_for_key(k);
                if known.is_empty() {
                    format!("Labels: unknown label {k}={v}; no labels have key {k}")
                } else {
                    format!(
                        "Labels: unknown label {k}={v}; known values for {k}: {}",
                        known.join(", ")
                    )
                }
            })
        })
        .collect()
}

const WORKLOAD_NAMES: &str = "workload names resolve in the CLI; use workload:/orgs/… here";

/// Provider, consumer or scope tokens to actors through the loaded index.
/// No request is made: names are looked up in `idx`, hrefs are checked by
/// the library's strict checkers, and `workload:` takes an href only.
pub fn resolve_actor_tokens(
    tokens: &[ActorToken],
    idx: &NameIndex,
    org_id: u64,
) -> Result<Vec<ActorRef>, String> {
    tokens
        .iter()
        .map(|t| resolve_actor_token(t, idx, org_id))
        .collect()
}

fn resolve_actor_token(
    token: &ActorToken,
    idx: &NameIndex,
    org_id: u64,
) -> Result<ActorRef, String> {
    let actor = match actor_from_href_token(token, org_id) {
        Some(checked) => checked.map_err(|e| e.to_string())?,
        None => match token {
            ActorToken::All => ActorRef::all(),
            ActorToken::Label { spec, exclusion } => {
                let (k, v) = split_kv(spec)?;
                let href = idx
                    .label_href(&k, &v)
                    .ok_or_else(|| unknown_label(idx, &k, &v))?;
                with_exclusion(ActorRef::new(ActorKind::Label, href), *exclusion)
            }
            ActorToken::LabelGroup { name, exclusion } => {
                let href = idx
                    .label_group_href(name)
                    .ok_or_else(|| format!("no label group named {name:?}"))?;
                with_exclusion(ActorRef::new(ActorKind::LabelGroup, href), *exclusion)
            }
            ActorToken::IpList { name, exclusion } => {
                let href = idx.ip_list_href(name).ok_or_else(|| {
                    let names = idx.ip_list_names();
                    with_similar(
                        format!("no IP list named {name:?}"),
                        &similar(name, names.iter().map(String::as_str)),
                    )
                })?;
                with_exclusion(ActorRef::new(ActorKind::IpList, href), *exclusion)
            }
            ActorToken::Workload { .. } => return Err(WORKLOAD_NAMES.into()),
            ActorToken::Href { href, exclusion } => with_exclusion(
                ActorRef::from_href(href, org_id).map_err(|e| e.to_string())?,
                *exclusion,
            ),
        },
    };
    actor.validate()?;
    Ok(actor)
}

fn unknown_label(idx: &NameIndex, k: &str, v: &str) -> String {
    let known = idx.values_for_key(k);
    if known.is_empty() {
        format!("unknown label {k}={v}; no labels have key {k}")
    } else {
        format!(
            "unknown label {k}={v}; known values for {k}: {}",
            known.join(", ")
        )
    }
}

/// Service tokens to rule services through the loaded index: a port as
/// typed, a name by exact match, an href through the strict checker.
pub fn resolve_service_tokens(
    tokens: &[ServiceToken],
    idx: &NameIndex,
    org_id: u64,
) -> Result<Vec<ServiceRef>, String> {
    tokens
        .iter()
        .map(|t| {
            if let Some(checked) = service_from_href_token(t, org_id) {
                return checked.map_err(|e| e.to_string());
            }
            match t {
                ServiceToken::Port(p) => Ok(ServiceRef::Port(p.clone())),
                ServiceToken::Name(n) => idx
                    .service_href(n)
                    .map(|h| ServiceRef::Href(h.to_string()))
                    .ok_or_else(|| format!("no service named {n:?}")),
                ServiceToken::Href(h) => {
                    ServiceRef::from_href(h, org_id).map_err(|e| e.to_string())
                }
            }
        })
        .collect()
}

/// A rule's or a scope's actors as form tokens: `app=web`, `group:Name`,
/// `iplist:Corp`, `all`, `!…`, `workload:/orgs/…`. An object the index
/// does not know, or whose name would not survive the comma split, shows
/// its href (which resolves back as it is). An actor the grammar cannot
/// express shows its href (virtual services and servers) or `?`.
pub fn actor_tokens(actors: &[Actor], idx: &NameIndex) -> Vec<String> {
    actors.iter().map(|a| actor_token(a, idx)).collect()
}

fn actor_token(a: &Actor, idx: &NameIndex) -> String {
    let Ok(r) = ActorRef::from_actor(a) else {
        return a
            .virtual_service
            .as_ref()
            .or(a.virtual_server.as_ref())
            .map_or_else(|| "?".to_string(), |v| v.href.clone());
    };
    let Some(href) = r.href else {
        return "all".into();
    };
    let named = match r.kind {
        ActorKind::Label => idx
            .label(&href)
            .filter(|l| !l.is_deleted())
            .map(Label::display),
        ActorKind::LabelGroup => idx.label_group_name(&href).map(|n| format!("group:{n}")),
        ActorKind::IpList => idx.ip_list_name(&href).map(|n| format!("iplist:{n}")),
        ActorKind::Workload => Some(format!("workload:{href}")),
        ActorKind::All => None,
    };
    let body = named.filter(|t| !t.contains([',', '|'])).unwrap_or(href);
    if r.exclusion {
        format!("!{body}")
    } else {
        body
    }
}

/// A rule's services as form tokens: the service's name (`service:NAME`
/// when the name would read as a port), `443/tcp`, `8000-8099/tcp`,
/// `icmp`; an unknown service as its href; an inline port the grammar
/// cannot type back (`any`, a bare protocol number) as `?`.
pub fn service_tokens(services: &[IngressService], idx: &NameIndex) -> Vec<String> {
    services.iter().map(|s| service_token(s, idx)).collect()
}

fn service_token(s: &IngressService, idx: &NameIndex) -> String {
    match ServiceRef::from_ingress(s) {
        ServiceRef::Href(href) => match idx.service_name(&href) {
            Some(n) if !n.contains([',', '|']) => {
                if ServiceToken::parse(n) == Ok(ServiceToken::Name(n.to_string())) {
                    n.to_string()
                } else {
                    format!("service:{n}")
                }
            }
            _ => href,
        },
        ServiceRef::Port(p) => port_token(&p).unwrap_or_else(|| "?".into()),
    }
}

/// `p` as typed (`443/tcp`, `icmp`) when the parser reads it back as `p`.
fn port_token(p: &PortSpec) -> Option<String> {
    let t = p.display();
    PortSpec::parse(&t).is_ok_and(|q| q == *p).then_some(t)
}

/// The submitted form as a write: `Ok(None)` when an edit changes nothing,
/// `Err` (shown in the form) when a field does not parse or validate.
pub fn build_intent(form: &Form, index: Option<&NameIndex>) -> Result<Option<WriteIntent>, String> {
    let v = |label: &str| form.value(label).trim().to_string();
    let invalid = |e: Vec<String>| e.join("; ");
    match &form.intent {
        EditIntent::CreateLabel => {
            let d = LabelDraft::new(v("Key"), v("Value"));
            d.validate().map_err(invalid)?;
            Ok(Some(WriteIntent::CreateLabel(d)))
        }
        EditIntent::UpdateLabel(l) => {
            let value = v("Value");
            if value == l.value.trim() {
                return Ok(None);
            }
            if value.is_empty() {
                return Err("label value must not be empty".into());
            }
            Ok(Some(WriteIntent::UpdateLabel {
                href: l.href.clone(),
                value,
            }))
        }
        EditIntent::CreateIpList => {
            let d = IpListDraft {
                name: v("Name"),
                description: non_empty(v("Description")),
                ranges: parse_ranges(&v("Ranges"), &[])?,
                fqdns: items(&v("FQDNs")).iter().map(Fqdn::new).collect(),
            };
            d.validate().map_err(invalid)?;
            Ok(Some(WriteIntent::CreateIpList(d)))
        }
        EditIntent::UpdateIpList(l) => {
            let old: Vec<IpRangeSpec> = l.ip_ranges.iter().map(IpRangeSpec::from_range).collect();
            let new = parse_ranges(&v("Ranges"), &old)?;
            let old_fqdns: Vec<String> = l.fqdns.iter().map(|f| f.fqdn.clone()).collect();
            let new_fqdns = items(&v("FQDNs"));
            let patch = IpListPatch {
                name: changed(&l.name, v("Name")),
                description: changed(l.description.as_deref().unwrap_or(""), v("Description")),
                add_ranges: new
                    .iter()
                    .filter(|n| !old.iter().any(|o| o.same_range(n)))
                    .cloned()
                    .collect(),
                remove_ranges: old
                    .iter()
                    .filter(|o| !new.iter().any(|n| n.same_range(o)))
                    .cloned()
                    .collect(),
                add_fqdns: new_fqdns
                    .iter()
                    .filter(|f| !old_fqdns.contains(f))
                    .cloned()
                    .collect(),
                remove_fqdns: old_fqdns
                    .iter()
                    .filter(|f| !new_fqdns.contains(f))
                    .cloned()
                    .collect(),
            };
            if patch.is_empty() {
                Ok(None)
            } else {
                Ok(Some(WriteIntent::UpdateIpList {
                    href: l.href.clone(),
                    patch,
                }))
            }
        }
        EditIntent::CreateService => {
            let ports = items(&v("Ports"))
                .iter()
                .map(|p| PortSpec::parse(p).map_err(|e| format!("Ports: {e}")))
                .collect::<Result<Vec<_>, _>>()?;
            let d = ServiceDraft {
                name: v("Name"),
                description: non_empty(v("Description")),
                ports,
            };
            d.validate().map_err(invalid)?;
            Ok(Some(WriteIntent::CreateService(d)))
        }
        EditIntent::UpdateService(s) => {
            let (add_ports, remove_ports) = port_changes(s, &items(&v("Ports")))?;
            let patch = ServicePatch {
                name: changed(&s.name, v("Name")),
                description: changed(s.description.as_deref().unwrap_or(""), v("Description")),
                add_ports,
                remove_ports,
            };
            if patch.is_empty() {
                Ok(None)
            } else {
                Ok(Some(WriteIntent::UpdateService {
                    href: s.href.clone(),
                    patch,
                }))
            }
        }
        EditIntent::CreateWorkload => {
            let interfaces = items(&v("Addresses"))
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    InterfaceSpec::parse(a, format!("eth{i}"))
                        .map_err(|e| format!("Addresses: {e}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let d = WorkloadDraft {
                name: v("Name"),
                hostname: non_empty(v("Hostname")),
                description: non_empty(v("Description")),
                interfaces,
                labels: resolve_labels(&v("Labels"), index)?,
            };
            d.validate().map_err(invalid)?;
            Ok(Some(WriteIntent::CreateWorkload(d)))
        }
        EditIntent::UpdateWorkload(w) => {
            let (add_addresses, remove_addresses) = address_changes(w, &items(&v("Addresses")))?;
            let labels = v("Labels");
            // The field shows the full list, so a changed field replaces it.
            let set_labels = if items(&labels) == items(&labels_text(w, index)) {
                None
            } else {
                Some(resolve_labels(&labels, index)?)
            };
            let patch = WorkloadPatch {
                name: changed(w.name.as_deref().unwrap_or(""), v("Name")),
                hostname: changed(w.hostname.as_deref().unwrap_or(""), v("Hostname")),
                description: changed(w.description.as_deref().unwrap_or(""), v("Description")),
                add_addresses,
                remove_addresses,
                set_labels,
                ..WorkloadPatch::default()
            };
            if patch.is_empty() {
                Ok(None)
            } else {
                Ok(Some(WriteIntent::UpdateWorkload {
                    href: w.href.clone(),
                    patch,
                }))
            }
        }
        // Deletes never open a form (`start`); a form never carries one.
        EditIntent::DeleteLabel(_)
        | EditIntent::DeleteIpList(_)
        | EditIntent::DeleteService(_)
        | EditIntent::DeleteWorkload(_) => Err("a delete has no form".into()),
    }
}

/// The request body as the confirm overlay shows it: pretty JSON, one entry
/// per line, or `(no body)` for a delete.
pub fn request_body_lines(req: &WriteRequest) -> Vec<String> {
    match &req.body {
        Some(b) => serde_json::to_string_pretty(b)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect(),
        None => vec!["(no body)".to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::fixture_index;
    use serde_json::json;

    fn labels() -> Vec<Label> {
        serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap()
    }
    fn ip_lists() -> Vec<IpList> {
        serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json")).unwrap()
    }
    fn services() -> Vec<Service> {
        serde_json::from_str(include_str!("../../../../fixtures/services.json")).unwrap()
    }
    fn workloads() -> Vec<Workload> {
        serde_json::from_str(include_str!("../../../../fixtures/workloads.json")).unwrap()
    }

    fn form(intent: EditIntent) -> Form {
        form_for(intent, Some(&*fixture_index())).expect("a create or update intent")
    }

    fn set(f: &mut Form, label: &str, value: &str) {
        f.fields
            .iter_mut()
            .find(|x| x.label == label)
            .unwrap_or_else(|| panic!("no field {label}"))
            .value = value.into();
    }

    fn build(f: &Form) -> Result<Option<WriteIntent>, String> {
        build_intent(f, Some(&*fixture_index()))
    }

    #[test]
    fn label_create_and_value_edit() {
        let mut f = form(EditIntent::CreateLabel);
        assert_eq!(
            f.fields.iter().map(|x| x.label).collect::<Vec<_>>(),
            vec!["Key", "Value"]
        );
        set(&mut f, "Key", " app ");
        set(&mut f, "Value", "web2");
        assert_eq!(
            build(&f).unwrap(),
            Some(WriteIntent::CreateLabel(LabelDraft::new("app", "web2")))
        );
        let mut f = form(EditIntent::UpdateLabel(labels()[0].clone())); // role=web
        assert_eq!(f.title, "Edit label role=web");
        assert_eq!(f.fields.len(), 1, "the key is not editable");
        assert_eq!(build(&f).unwrap(), None, "unchanged");
        set(&mut f, "Value", "www");
        assert_eq!(
            build(&f).unwrap(),
            Some(WriteIntent::UpdateLabel {
                href: "/orgs/5/labels/1".into(),
                value: "www".into()
            })
        );
    }

    #[test]
    fn ip_list_form_prefills_and_create_keeps_exclusions() {
        let f = form(EditIntent::UpdateIpList(ip_lists()[1].clone()));
        assert_eq!(f.title, "Edit IP list Corp");
        assert_eq!(f.value("Name"), "Corp");
        assert_eq!(f.value("Description"), "Office ranges");
        assert_eq!(f.value("Ranges"), "10.1.0.0-10.1.255.255");
        assert_eq!(f.value("FQDNs"), "*.corp.example");
        let mut f = form(EditIntent::CreateIpList);
        set(&mut f, "Name", "Lab");
        set(&mut f, "Ranges", "10.0.0.0/8, !10.0.5.0/24");
        set(&mut f, "FQDNs", "*.lab.example");
        let Some(WriteIntent::CreateIpList(d)) = build(&f).unwrap() else {
            panic!("not a create")
        };
        assert_eq!(d.name, "Lab");
        assert_eq!(d.description, None);
        assert_eq!(
            d.ranges,
            vec![
                IpRangeSpec::parse("10.0.0.0/8", false).unwrap(),
                IpRangeSpec::parse("10.0.5.0/24", true).unwrap(),
            ]
        );
        assert_eq!(d.fqdns, vec![Fqdn::new("*.lab.example")]);
        set(&mut f, "Ranges", "10.0.0.300");
        let err = build(&f).unwrap_err();
        assert!(err.starts_with("Ranges:"), "{err}");
    }

    #[test]
    fn ip_list_edit_adds_and_removes_only_what_changed() {
        let corp = ip_lists()[1].clone();
        let mut f = form(EditIntent::UpdateIpList(corp.clone()));
        assert_eq!(
            build(&f).unwrap(),
            None,
            "a pre-filled form changes nothing"
        );
        set(&mut f, "Description", "Office");
        set(&mut f, "Ranges", "10.2.0.0/16");
        set(&mut f, "FQDNs", "*.corp.example, api.example.com");
        assert_eq!(
            build(&f).unwrap(),
            Some(WriteIntent::UpdateIpList {
                href: "/orgs/5/sec_policy/draft/ip_lists/21".into(),
                patch: IpListPatch {
                    name: None,
                    description: Some("Office".into()),
                    add_ranges: vec![IpRangeSpec::parse("10.2.0.0/16", false).unwrap()],
                    remove_ranges: vec![IpRangeSpec::from_range(&corp.ip_ranges[0])],
                    add_fqdns: vec!["api.example.com".into()],
                    remove_fqdns: vec![],
                },
            })
        );
    }

    #[test]
    fn service_bare_port_is_refused_and_edits_diff_by_port() {
        let mut f = form(EditIntent::CreateService);
        set(&mut f, "Name", "SSH");
        set(&mut f, "Ports", "22");
        let err = build(&f).unwrap_err();
        assert!(
            err.contains("a bare number is ambiguous") && err.contains("22/tcp"),
            "{err}"
        );
        set(&mut f, "Ports", "22/tcp, icmp");
        let Some(WriteIntent::CreateService(d)) = build(&f).unwrap() else {
            panic!("not a create")
        };
        assert_eq!(
            d.ports,
            vec![
                PortSpec::parse("22/tcp").unwrap(),
                PortSpec::parse("icmp").unwrap()
            ]
        );
        let mut f = form(EditIntent::UpdateService(services()[2].clone())); // Web
        assert_eq!(f.value("Ports"), "80/tcp, 443/tcp, 8000-8099/tcp");
        assert_eq!(build(&f).unwrap(), None);
        set(&mut f, "Ports", "443/TCP, 8000-8099/tcp, 53/udp");
        let Some(WriteIntent::UpdateService { href, patch }) = build(&f).unwrap() else {
            panic!("not an update")
        };
        assert_eq!(href, "/orgs/5/sec_policy/draft/services/32");
        assert_eq!(patch.add_ports, vec![PortSpec::parse("53/udp").unwrap()]);
        assert_eq!(patch.remove_ports, vec![PortSpec::parse("80/tcp").unwrap()]);
        // `any` cannot be parsed back, but spelled as shown it is kept.
        let all = form(EditIntent::UpdateService(services()[0].clone()));
        assert_eq!(all.value("Ports"), "any");
        assert_eq!(build(&all).unwrap(), None);
    }

    #[test]
    fn workload_create_resolves_labels_through_the_index() {
        let mut f = form(EditIntent::CreateWorkload);
        set(&mut f, "Name", "printer-1");
        set(&mut f, "Addresses", "10.0.0.50/24, 10.0.0.51");
        set(&mut f, "Labels", "app=shop, env=prod");
        let Some(WriteIntent::CreateWorkload(d)) = build(&f).unwrap() else {
            panic!("not a create")
        };
        assert_eq!(
            d.interfaces,
            vec![
                InterfaceSpec::parse("10.0.0.50/24", "eth0".into()).unwrap(),
                InterfaceSpec::parse("10.0.0.51", "eth1".into()).unwrap(),
            ]
        );
        assert_eq!(d.labels, vec!["/orgs/5/labels/3", "/orgs/5/labels/4"]);
        assert_eq!(d.hostname, None);
        let host1 = form(EditIntent::UpdateWorkload(workloads()[0].clone()));
        assert_eq!(host1.value("Labels"), "role=web, app=shop, env=prod");
        assert_eq!(host1.value("Addresses"), "10.0.1.10/24");
    }

    #[test]
    fn unknown_labels_name_the_known_values_and_a_missing_index_refuses() {
        let mut f = form(EditIntent::CreateWorkload);
        set(&mut f, "Name", "printer-1");
        set(&mut f, "Labels", "app=nope");
        let err = build(&f).unwrap_err();
        assert!(err.contains("known values for app: shop"), "{err}");
        set(&mut f, "Labels", "tier=1");
        let err = build(&f).unwrap_err();
        assert!(err.contains("no labels have key tier"), "{err}");
        set(&mut f, "Labels", "app=shop");
        let err = build_intent(&f, None).unwrap_err();
        assert!(
            err.contains("label index not loaded, press r on Objects"),
            "{err}"
        );
        set(&mut f, "Labels", "");
        assert!(
            build_intent(&f, None).unwrap().is_some(),
            "no labels need no index"
        );
    }

    #[test]
    fn workload_edit_diffs_addresses_and_sets_the_full_label_list() {
        let nas = workloads()[2].clone(); // unmanaged-nas, eth0 10.0.3.30, no labels
        let mut f = form(EditIntent::UpdateWorkload(nas));
        assert_eq!(f.title, "Edit workload unmanaged-nas");
        assert_eq!(f.value("Addresses"), "10.0.3.30");
        assert_eq!(f.value("Labels"), "");
        assert_eq!(build(&f).unwrap(), None);
        set(&mut f, "Addresses", "10.0.3.30, 10.0.3.31/24");
        set(&mut f, "Labels", "env=prod");
        assert_eq!(
            build(&f).unwrap(),
            Some(WriteIntent::UpdateWorkload {
                href: "/orgs/5/workloads/aaaa-3".into(),
                patch: WorkloadPatch {
                    add_addresses: vec!["10.0.3.31/24".into()],
                    set_labels: Some(vec!["/orgs/5/labels/4".into()]),
                    ..WorkloadPatch::default()
                },
            })
        );
        set(&mut f, "Addresses", "10.0.3.31");
        set(&mut f, "Labels", "");
        let Some(WriteIntent::UpdateWorkload { patch, .. }) = build(&f).unwrap() else {
            panic!("not an update")
        };
        assert_eq!(patch.remove_addresses, vec!["10.0.3.30"]);
        assert_eq!(patch.add_addresses, vec!["10.0.3.31"]);
        assert_eq!(patch.set_labels, None, "labels unchanged");
    }

    #[test]
    fn untouched_addresses_are_matched_by_their_text_first() {
        // A PCE spelling the parser would canonicalise (`fe80::1/64`).
        let mut nas = workloads()[2].clone();
        nas.interfaces[0].address = "FE80::0001".into();
        nas.interfaces[0].cidr_block = Some(64);
        let f = form(EditIntent::UpdateWorkload(nas));
        assert_eq!(f.value("Addresses"), "FE80::0001/64");
        assert_eq!(build(&f).unwrap(), None, "an untouched form sends nothing");
    }

    #[test]
    fn removing_a_prefix_is_refused_and_changing_one_is_an_add() {
        let mut nas = workloads()[2].clone();
        nas.interfaces[0].cidr_block = Some(24);
        let mut f = form(EditIntent::UpdateWorkload(nas));
        assert_eq!(f.value("Addresses"), "10.0.3.30/24");
        set(&mut f, "Addresses", "10.0.3.30");
        assert_eq!(
            build(&f).unwrap_err(),
            "Addresses: removing a prefix is not supported here; remove the address and add it again"
        );
        set(&mut f, "Addresses", "10.0.3.30/16");
        let Some(WriteIntent::UpdateWorkload { patch, .. }) = build(&f).unwrap() else {
            panic!("not an update")
        };
        assert_eq!(patch.add_addresses, vec!["10.0.3.30/16"]);
        assert!(patch.remove_addresses.is_empty(), "the interface is kept");
    }

    #[test]
    fn deletes_skip_the_form_and_carry_the_name() {
        let Start::Plan(w) = start(EditIntent::DeleteIpList(ip_lists()[1].clone()), None) else {
            panic!("a delete opens no form")
        };
        assert_eq!(
            w,
            WriteIntent::DeleteIpList {
                href: "/orgs/5/sec_policy/draft/ip_lists/21".into(),
                name: "Corp".into()
            }
        );
        assert_eq!(w.kind(), pce::write::ObjectKind::IpList);
        let Start::Plan(w) = start(EditIntent::DeleteLabel(labels()[0].clone()), None) else {
            panic!("a delete opens no form")
        };
        assert_eq!(
            w,
            WriteIntent::DeleteLabel {
                href: "/orgs/5/labels/1".into(),
                name: "role=web".into()
            }
        );
        assert!(form_for(EditIntent::DeleteWorkload(workloads()[2].clone()), None).is_none());
    }

    fn rule_sets() -> Vec<pce::models::RuleSet> {
        serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap()
    }

    fn actor(v: serde_json::Value) -> Actor {
        serde_json::from_value(v).unwrap()
    }

    fn resolve_actors(tokens: &[String]) -> Result<Vec<ActorRef>, String> {
        let parsed = tokens
            .iter()
            .map(|t| ActorToken::parse(t))
            .collect::<Result<Vec<_>, _>>()?;
        resolve_actor_tokens(&parsed, &fixture_index(), 5)
    }

    #[test]
    fn actor_tokens_render_every_kind_and_resolve_back() {
        let sets = rule_sets();
        let actors = vec![
            sets[0].rules[0].providers[0].clone(), // label 2
            sets[0].rules[1].consumers[0].clone(), // IP list 21
            sets[1].rules[0].consumers[0].clone(), // label group 10
            sets[1].rules[0].providers[0].clone(), // ams
            actor(json!({"label": {"href": "/orgs/5/labels/1"}, "exclusion": true})),
            actor(json!({"workload": {"href": "/orgs/5/workloads/aaaa-1"}})),
            actor(json!({"label": {"href": "/orgs/5/labels/99"}})),
        ];
        let tokens = actor_tokens(&actors, &fixture_index());
        assert_eq!(
            tokens,
            vec![
                "role=db",
                "iplist:Corp",
                "group:All prod envs",
                "all",
                "!role=web",
                "workload:/orgs/5/workloads/aaaa-1",
                "/orgs/5/labels/99",
            ]
        );
        let want: Vec<ActorRef> = actors
            .iter()
            .map(|a| ActorRef::from_actor(a).unwrap())
            .collect();
        assert_eq!(
            resolve_actors(&tokens).unwrap(),
            want,
            "every token resolves back to its actor"
        );
    }

    #[test]
    fn service_tokens_render_names_and_ports_and_resolve_back() {
        let sets = rule_sets();
        let mut services = sets[0].rules[0].ingress_services.clone(); // PostgreSQL
        services.extend(sets[0].rules[1].ingress_services.clone()); // 443/tcp, 8000-8099/tcp
        services.extend(sets[1].rules[0].ingress_services.clone()); // All Services
        services.push(serde_json::from_value(json!({"proto": 1})).unwrap());
        let tokens = service_tokens(&services, &fixture_index());
        assert_eq!(
            tokens,
            vec![
                "PostgreSQL",
                "443/tcp",
                "8000-8099/tcp",
                "All Services",
                "icmp"
            ]
        );
        let parsed: Vec<ServiceToken> = tokens
            .iter()
            .map(|t| ServiceToken::parse(t).unwrap())
            .collect();
        let want: Vec<ServiceRef> = services.iter().map(ServiceRef::from_ingress).collect();
        assert_eq!(
            resolve_service_tokens(&parsed, &fixture_index(), 5).unwrap(),
            want
        );
        // A service named like a protocol keeps its name behind `service:`.
        let tcp: Service = serde_json::from_value(json!({
            "href": "/orgs/5/sec_policy/draft/services/40", "name": "tcp", "service_ports": []
        }))
        .unwrap();
        let idx = NameIndex::new(&[], &[], &[], &[tcp]);
        let svc = vec![IngressService::Ref(pce::models::Ref::new(
            "/orgs/5/sec_policy/draft/services/40",
        ))];
        assert_eq!(service_tokens(&svc, &idx), vec!["service:tcp"]);
        assert_eq!(
            resolve_service_tokens(&[ServiceToken::parse("service:tcp").unwrap()], &idx, 5)
                .unwrap(),
            vec![ServiceRef::Href(
                "/orgs/5/sec_policy/draft/services/40".into()
            )]
        );
    }

    #[test]
    fn inexpressible_entries_render_as_href_or_question_mark() {
        let idx = fixture_index();
        let actors = vec![
            actor(
                json!({"virtual_service": {"href": "/orgs/5/sec_policy/draft/virtual_services/9"}}),
            ),
            Actor::default(),
        ];
        assert_eq!(
            actor_tokens(&actors, &idx),
            vec!["/orgs/5/sec_policy/draft/virtual_services/9", "?"]
        );
        let services: Vec<IngressService> = serde_json::from_value(json!([
            {"proto": -1},
            {"proto": 47},
            {"href": "/orgs/5/sec_policy/draft/services/77"}
        ]))
        .unwrap();
        assert_eq!(
            service_tokens(&services, &idx),
            vec!["?", "?", "/orgs/5/sec_policy/draft/services/77"],
            "`any` and protocol 47 cannot be typed back; an unknown service keeps its href"
        );
    }

    #[test]
    fn resolution_errors_name_what_is_known_and_workload_names_stay_in_the_cli() {
        let err = |t: &str| resolve_actors(&[t.to_string()]).unwrap_err();
        assert_eq!(
            err("app=nope"),
            "unknown label app=nope; known values for app: shop"
        );
        assert_eq!(
            err("tier=1"),
            "unknown label tier=1; no labels have key tier"
        );
        assert_eq!(
            err("iplist:Cor"),
            r#"no IP list named "Cor"; similar: Corp"#
        );
        assert_eq!(err("group:Nope"), r#"no label group named "Nope""#);
        assert_eq!(
            err("workload:web-1.lab"),
            "workload names resolve in the CLI; use workload:/orgs/… here"
        );
        assert!(
            resolve_actors(&["/orgs/9/labels/1".into()]).is_err(),
            "wrong org"
        );
        assert_eq!(
            resolve_actors(&["workload:/orgs/5/workloads/aaaa-3".into()]).unwrap(),
            vec![ActorRef::new(
                ActorKind::Workload,
                "/orgs/5/workloads/aaaa-3"
            )]
        );
        let svc = |t: &str| {
            resolve_service_tokens(&[ServiceToken::parse(t).unwrap()], &fixture_index(), 5)
        };
        assert_eq!(svc("HTTPS").unwrap_err(), r#"no service named "HTTPS""#);
        assert_eq!(
            svc("443/tcp").unwrap(),
            vec![ServiceRef::Port(PortSpec::parse("443/tcp").unwrap())]
        );
    }
}
