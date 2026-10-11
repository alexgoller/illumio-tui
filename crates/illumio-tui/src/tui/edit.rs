//! Editing in the TUI. A key produces an [`EditIntent`]; [`start`] turns it
//! into a pre-filled [`Form`] (create, update) or straight into a
//! [`WriteIntent`] (delete); [`build_intent`] turns a submitted form into the
//! draft or patch the library plans. Pure: nothing here reaches the PCE.
use crate::tokens::{
    ActorToken, ServiceToken, actor_from_href_token, is_href, parse_scope_tokens,
    service_from_href_token, similar, split_kv, with_exclusion, with_similar,
};
use crate::tui::form::{Field, Form};
use pce::models::{
    Actor, DenyRule, Fqdn, IngressService, Interface, IpList, Label, Rule, RuleSet, Service,
    Workload,
};
use pce::traffic::{ResolvedEndpoint, ResolvedFlow};
use pce::write::{
    ActorKind, ActorRef, DenyRuleDraft, DenyRulePatch, InterfaceSpec, IpListDraft, IpListPatch,
    IpRangeSpec, LabelDraft, PortSpec, ProvisionRequest, RuleDraft, RulePatch, RuleSetDraft,
    RuleSetPatch, ServiceDraft, ServicePatch, ServiceRef, WorkloadDraft, WorkloadPatch,
    WriteRequest, checked_policy_href,
};
use pce::{NameIndex, ResolvedLabel};
use std::borrow::Cow;
use std::collections::BTreeSet;

/// What a `w` form is pre-filled from besides the flow: the label model
/// (rulesets scoped by `scope_keys`, tiers selected by `tier_key`) and the
/// draft rulesets already loaded (empty when none are). Never a request.
#[derive(Debug, Clone, Copy)]
pub struct FlowPrefill<'a> {
    pub scope_keys: &'a [String],
    pub tier_key: &'a str,
    pub rulesets: &'a [RuleSet],
}

/// Where a new allow rule goes: the selected ruleset (Policy) or the name
/// or href typed into a `w` form. A name costs one draft ruleset read at
/// plan time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RulesetTarget {
    Href(String),
    Name(String),
}

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
    CreateRuleSet,
    UpdateRuleSet(RuleSet),
    DeleteRuleSet(RuleSet),
    CreateRule {
        ruleset_href: String,
    },
    CreateDenyRule {
        ruleset_href: String,
    },
    /// The rule as loaded and its ruleset's href.
    UpdateRule(Rule, String),
    UpdateDenyRule(DenyRule, String),
    DeleteRule {
        href: String,
        /// The rule as the rules pane shows it, for the confirm text.
        name: String,
    },
    DeleteDenyRule {
        href: String,
        /// The rule as the rules pane shows it, for the confirm text.
        name: String,
    },
    /// `w` on a blocked flow: a draft allow rule pre-filled from it.
    RuleFromFlow(Box<ResolvedFlow>),
}

/// What `Cmd::PlanWrite` carries: the library draft or patch, or the href
/// (and, for the confirm text, the name) of the object to delete.
#[derive(Debug, Clone, PartialEq)]
pub enum WriteIntent {
    CreateLabel(LabelDraft),
    UpdateLabel {
        href: String,
        value: String,
    },
    DeleteLabel {
        href: String,
        name: String,
    },
    CreateIpList(IpListDraft),
    UpdateIpList {
        href: String,
        patch: IpListPatch,
    },
    DeleteIpList {
        href: String,
        name: String,
    },
    CreateService(ServiceDraft),
    UpdateService {
        href: String,
        patch: ServicePatch,
    },
    DeleteService {
        href: String,
        name: String,
    },
    CreateWorkload(WorkloadDraft),
    UpdateWorkload {
        href: String,
        patch: WorkloadPatch,
    },
    DeleteWorkload {
        href: String,
        name: String,
    },
    CreateRuleSet(RuleSetDraft),
    UpdateRuleSet {
        href: String,
        patch: RuleSetPatch,
    },
    DeleteRuleSet {
        href: String,
        name: String,
    },
    CreateRule {
        ruleset: RulesetTarget,
        draft: RuleDraft,
    },
    UpdateRule {
        href: String,
        patch: RulePatch,
    },
    DeleteRule {
        href: String,
        /// The rule as the rules pane shows it, for the confirm text.
        name: String,
    },
    CreateDenyRule {
        ruleset_href: String,
        draft: DenyRuleDraft,
    },
    UpdateDenyRule {
        href: String,
        patch: DenyRulePatch,
    },
    DeleteDenyRule {
        href: String,
        /// The rule as the rules pane shows it, for the confirm text.
        name: String,
    },
    /// `P` in the pending overlay: the marked entries, or everything.
    Provision(ProvisionRequest),
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
            WriteIntent::CreateRuleSet(_)
            | WriteIntent::UpdateRuleSet { .. }
            | WriteIntent::DeleteRuleSet { .. } => pce::write::ObjectKind::RuleSet,
            WriteIntent::CreateRule { .. }
            | WriteIntent::UpdateRule { .. }
            | WriteIntent::DeleteRule { .. } => pce::write::ObjectKind::Rule,
            WriteIntent::CreateDenyRule { .. }
            | WriteIntent::UpdateDenyRule { .. }
            | WriteIntent::DeleteDenyRule { .. } => pce::write::ObjectKind::DenyRule,
            WriteIntent::Provision(_) => pce::write::ObjectKind::Provision,
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
pub fn start(intent: EditIntent, index: Option<&NameIndex>, prefill: FlowPrefill) -> Start {
    // Policy forms render actors through the index; without one they show hrefs.
    let empty = NameIndex::default();
    let idx = index.unwrap_or(&empty);
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
        EditIntent::CreateRuleSet => form(
            "New ruleset".into(),
            rule_set_fields(None, idx),
            EditIntent::CreateRuleSet,
        ),
        EditIntent::UpdateRuleSet(rs) => {
            let title = format!("Edit ruleset {}", rs.name);
            let fields = rule_set_fields(Some(&rs), idx);
            form(title, fields, EditIntent::UpdateRuleSet(rs))
        }
        EditIntent::DeleteRuleSet(rs) => Start::Plan(WriteIntent::DeleteRuleSet {
            name: rs.name,
            href: rs.href,
        }),
        EditIntent::CreateRule { ruleset_href } => form(
            format!("New allow rule in ruleset {}", href_id(&ruleset_href)),
            rule_fields(None, idx),
            EditIntent::CreateRule { ruleset_href },
        ),
        EditIntent::CreateDenyRule { ruleset_href } => form(
            format!("New deny rule in ruleset {}", href_id(&ruleset_href)),
            deny_rule_fields(None, idx),
            EditIntent::CreateDenyRule { ruleset_href },
        ),
        EditIntent::UpdateRule(r, rs) => {
            let title = format!(
                "Edit allow rule {} in ruleset {}",
                href_id(&r.href),
                href_id(&rs)
            );
            let fields = rule_fields(Some(&r), idx);
            form(title, fields, EditIntent::UpdateRule(r, rs))
        }
        EditIntent::UpdateDenyRule(d, rs) => {
            let title = format!(
                "Edit deny rule {} in ruleset {}",
                href_id(&d.href),
                href_id(&rs)
            );
            let fields = deny_rule_fields(Some(&d), idx);
            form(title, fields, EditIntent::UpdateDenyRule(d, rs))
        }
        EditIntent::DeleteRule { href, name } => {
            Start::Plan(WriteIntent::DeleteRule { href, name })
        }
        EditIntent::DeleteDenyRule { href, name } => {
            Start::Plan(WriteIntent::DeleteDenyRule { href, name })
        }
        EditIntent::RuleFromFlow(f) => {
            let (fields, alt) = flow_rule_fields(&f, prefill);
            let mut form = Form::new("Allow rule from flow", fields, EditIntent::RuleFromFlow(f));
            form.alt = Some(alt);
            Start::Form(form)
        }
    }
}

/// The form a create or update intent opens; `None` for a delete.
pub fn form_for(
    intent: EditIntent,
    index: Option<&NameIndex>,
    prefill: FlowPrefill,
) -> Option<Form> {
    match start(intent, index, prefill) {
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

const SCOPES_HINT: &str = "app=web, env=prod | app=db; empty = everywhere";
const YES_NO_HINT: &str = "y or n";
const NO_INDEX: &str = "the name index is not loaded; press r on Objects";
const CANNOT_EDIT_RULESET: &str = "this ruleset has entries the form cannot edit; use the CLI";

fn yn(on: bool) -> &'static str {
    if on { "y" } else { "n" }
}

/// `y`/`yes` or `n`/`no`, any case.
fn yes_no(form: &Form, label: &str) -> Result<bool, String> {
    match form.value(label).trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => Ok(true),
        "n" | "no" => Ok(false),
        other => Err(format!("{label}: {other:?} is not y or n")),
    }
}

fn need_index(index: Option<&NameIndex>) -> Result<&NameIndex, String> {
    index.ok_or_else(|| NO_INDEX.to_string())
}

fn rule_set_fields(rs: Option<&RuleSet>, idx: &NameIndex) -> Vec<Field> {
    vec![
        Field::new(
            "Name",
            rs.map(|r| r.name.clone()).unwrap_or_default(),
            "Shop-Prod",
            true,
        ),
        Field::new(
            "Description",
            rs.and_then(|r| r.description.clone()).unwrap_or_default(),
            "optional",
            false,
        ),
        Field::new(
            "Enabled",
            yn(rs.is_none_or(|r| r.enabled)),
            YES_NO_HINT,
            true,
        ),
        Field::new(
            "Scopes",
            rs.map(|r| scopes_text(&r.scopes, idx)).unwrap_or_default(),
            SCOPES_HINT,
            false,
        ),
    ]
}

/// Scopes as the Scopes field shows them, `app=web, env=prod | group:G`;
/// empty for a ruleset that applies everywhere (`[[]]` or `[]`).
pub fn scopes_text(scopes: &[Vec<Actor>], idx: &NameIndex) -> String {
    scopes
        .iter()
        .filter(|s| !s.is_empty())
        .map(|s| actor_tokens(s, idx).join(", "))
        .collect::<Vec<_>>()
        .join(" | ")
}

/// The Scopes text as tokens per scope, for the "did it change" test.
fn scope_items(text: &str) -> Vec<Vec<String>> {
    text.split('|')
        .map(items)
        .filter(|s| !s.is_empty())
        .collect()
}

/// The Scopes field to scopes: empty means everywhere; each `|`-separated
/// part is one scope of labels and label groups, resolved through the index.
fn resolve_scopes(text: &str, idx: &NameIndex, org_id: u64) -> Result<Vec<Vec<ActorRef>>, String> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let specs: Vec<String> = text.split('|').map(str::to_string).collect();
    let scopes = parse_scope_tokens(&specs).map_err(|e| format!("Scopes: {e}"))?;
    if scopes.iter().any(Vec::is_empty) {
        return Err(
            "Scopes: an empty scope between |; leave Scopes empty for a ruleset that applies everywhere"
                .into(),
        );
    }
    scopes
        .iter()
        .map(|s| resolve_actor_tokens(s, idx, org_id).map_err(|e| format!("Scopes: {e}")))
        .collect()
}

const ACTORS_HINT: &str = "app=web, group:Name, iplist:Corp, all, !env=dev";
const SERVICES_HINT: &str = "443/tcp, 8000-8100/udp, icmp, HTTPS";
const UNSCOPED_HINT: &str = "y: consumers outside the ruleset's scope";
const OVERRIDE_HINT: &str = "y: evaluated before allow rules";
const CANNOT_EDIT_RULE: &str = "this rule has entries the form cannot edit; use the CLI";

/// The last segment of an href, for form titles.
fn href_id(href: &str) -> &str {
    href.rsplit('/').next().unwrap_or(href)
}

/// Providers, Consumers and Services, pre-filled from a rule's lists.
fn side_fields(
    sides: Option<(&[Actor], &[Actor], &[IngressService])>,
    idx: &NameIndex,
) -> Vec<Field> {
    let (p, c, s) = sides.map_or_else(
        || (String::new(), String::new(), String::new()),
        |(p, c, s)| {
            (
                actor_tokens(p, idx).join(", "),
                actor_tokens(c, idx).join(", "),
                service_tokens(s, idx).join(", "),
            )
        },
    );
    vec![
        Field::new("Providers", p, ACTORS_HINT, true),
        Field::new("Consumers", c, ACTORS_HINT, true),
        Field::new("Services", s, SERVICES_HINT, true),
    ]
}

fn rule_fields(r: Option<&Rule>, idx: &NameIndex) -> Vec<Field> {
    let mut fields = side_fields(
        r.map(|r| (&r.providers[..], &r.consumers[..], &r.ingress_services[..])),
        idx,
    );
    fields.extend([
        Field::new(
            "Description",
            r.and_then(|r| r.description.clone()).unwrap_or_default(),
            "optional",
            false,
        ),
        Field::new(
            "Enabled",
            yn(r.is_none_or(|r| r.enabled)),
            YES_NO_HINT,
            true,
        ),
        Field::new(
            "Unscoped",
            yn(r.is_some_and(|r| r.unscoped_consumers.unwrap_or(false))),
            UNSCOPED_HINT,
            true,
        ),
    ]);
    fields
}

fn deny_rule_fields(d: Option<&DenyRule>, idx: &NameIndex) -> Vec<Field> {
    let mut fields = side_fields(
        d.map(|d| (&d.providers[..], &d.consumers[..], &d.ingress_services[..])),
        idx,
    );
    fields.extend([
        Field::new(
            "Description",
            d.and_then(|d| d.description.clone()).unwrap_or_default(),
            "optional",
            false,
        ),
        Field::new(
            "Enabled",
            yn(d.is_none_or(|d| d.enabled)),
            YES_NO_HINT,
            true,
        ),
        Field::new(
            "Override",
            yn(d.is_some_and(|d| d.override_)),
            OVERRIDE_HINT,
            true,
        ),
    ]);
    fields
}

/// A Providers or Consumers field to actors.
fn actor_field(
    form: &Form,
    label: &str,
    idx: &NameIndex,
    org_id: u64,
) -> Result<Vec<ActorRef>, String> {
    let tokens = items(form.value(label))
        .iter()
        .map(|t| ActorToken::parse(t.as_str()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("{label}: {e}"))?;
    resolve_actor_tokens(&tokens, idx, org_id).map_err(|e| format!("{label}: {e}"))
}

/// The Services field to rule services.
fn service_field(form: &Form, idx: &NameIndex, org_id: u64) -> Result<Vec<ServiceRef>, String> {
    let tokens = items(form.value("Services"))
        .iter()
        .map(|t| ServiceToken::parse(t.as_str()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Services: {e}"))?;
    resolve_service_tokens(&tokens, idx, org_id).map_err(|e| format!("Services: {e}"))
}

/// The new allow rule a Policy `n` form (or a `w` form) describes.
fn rule_draft(form: &Form, idx: &NameIndex, org_id: u64) -> Result<RuleDraft, String> {
    let d = RuleDraft {
        description: non_empty(form.value("Description").trim().to_string()),
        enabled: yes_no(form, "Enabled")?,
        providers: actor_field(form, "Providers", idx, org_id)?,
        consumers: actor_field(form, "Consumers", idx, org_id)?,
        services: service_field(form, idx, org_id)?,
        unscoped_consumers: yes_no(form, "Unscoped")?,
        resolve_labels_as: None,
    };
    d.validate().map_err(|e| e.join("; "))?;
    Ok(d)
}

fn deny_rule_draft(form: &Form, idx: &NameIndex, org_id: u64) -> Result<DenyRuleDraft, String> {
    let d = DenyRuleDraft {
        description: non_empty(form.value("Description").trim().to_string()),
        enabled: yes_no(form, "Enabled")?,
        providers: actor_field(form, "Providers", idx, org_id)?,
        consumers: actor_field(form, "Consumers", idx, org_id)?,
        services: service_field(form, idx, org_id)?,
        override_: yes_no(form, "Override")?,
    };
    d.validate().map_err(|e| e.join("; "))?;
    Ok(d)
}

fn actor_editable(a: &Actor) -> bool {
    ActorRef::from_actor(a).is_ok()
}

fn service_editable(s: &IngressService) -> bool {
    match ServiceRef::from_ingress(s) {
        ServiceRef::Href(_) => true,
        ServiceRef::Port(p) => port_token(&p).is_some(),
    }
}

/// What an edited Providers or Consumers field adds and removes, matched
/// against the shown tokens first: a token still there keeps the PCE's
/// entry, a token gone removes exactly that entry, a new token is resolved
/// through the index. A token retyped in another spelling of a shown
/// entry is neither added nor removed. A field holding an entry the
/// grammar cannot express may not change.
fn actor_changes(
    label: &str,
    current: &[Actor],
    text: &str,
    idx: &NameIndex,
    org_id: u64,
) -> Result<(Vec<ActorRef>, Vec<ActorRef>), String> {
    let shown = actor_tokens(current, idx);
    let typed = items(text);
    if typed == shown {
        return Ok((Vec::new(), Vec::new()));
    }
    if !current.iter().all(actor_editable) {
        return Err(format!("{label}: {CANNOT_EDIT_RULE}"));
    }
    let mut remove = Vec::new();
    for (a, t) in current.iter().zip(&shown) {
        if !typed.contains(t) {
            remove.push(ActorRef::from_actor(a).map_err(|e| format!("{label}: {e}"))?);
        }
    }
    let new = typed
        .iter()
        .filter(|t| !shown.contains(t))
        .map(|t| ActorToken::parse(t.as_str()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("{label}: {e}"))?;
    let mut add = resolve_actor_tokens(&new, idx, org_id).map_err(|e| format!("{label}: {e}"))?;
    add.retain(|a| match remove.iter().position(|r| r.same_actor(a)) {
        Some(i) => {
            remove.remove(i);
            false
        }
        None => true,
    });
    Ok((add, remove))
}

/// [`actor_changes`] for the Services field.
fn service_changes(
    current: &[IngressService],
    text: &str,
    idx: &NameIndex,
    org_id: u64,
) -> Result<(Vec<ServiceRef>, Vec<ServiceRef>), String> {
    let shown = service_tokens(current, idx);
    let typed = items(text);
    if typed == shown {
        return Ok((Vec::new(), Vec::new()));
    }
    if !current.iter().all(service_editable) {
        return Err(format!("Services: {CANNOT_EDIT_RULE}"));
    }
    let mut remove: Vec<ServiceRef> = current
        .iter()
        .zip(&shown)
        .filter(|(_, t)| !typed.contains(t))
        .map(|(s, _)| ServiceRef::from_ingress(s))
        .collect();
    let new = typed
        .iter()
        .filter(|t| !shown.contains(t))
        .map(|t| ServiceToken::parse(t.as_str()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Services: {e}"))?;
    let mut add =
        resolve_service_tokens(&new, idx, org_id).map_err(|e| format!("Services: {e}"))?;
    add.retain(|a| match remove.iter().position(|r| r.same_service(a)) {
        Some(i) => {
            remove.remove(i);
            false
        }
        None => true,
    });
    Ok((add, remove))
}

/// The one draft ruleset named `name` (any case; a pending deletion does
/// not count), for a rule typed into a named ruleset.
pub fn ruleset_href_named(sets: &[RuleSet], name: &str) -> Result<String, String> {
    let name = name.trim();
    let live = || {
        sets.iter()
            .filter(|rs| rs.update_type.as_deref() != Some("delete"))
    };
    let hits: Vec<&str> = live()
        .filter(|rs| rs.name.eq_ignore_ascii_case(name))
        .map(|rs| rs.href.as_str())
        .collect();
    match hits.as_slice() {
        [one] => Ok(one.to_string()),
        [] => Err(with_similar(
            unknown_ruleset(name),
            &similar(name, live().map(|rs| rs.name.as_str())),
        )),
        many => Err(format!(
            "{name:?} names several rulesets: {}; use the href",
            many.join(", ")
        )),
    }
}

/// The planner's answer for a ruleset name no draft ruleset has.
fn unknown_ruleset(name: &str) -> String {
    format!("no ruleset named {name:?}")
}

/// The scope of a `w` form's destination, when the form names a ruleset
/// (not an href) that `error` says does not exist.
fn unknown_ruleset_scope<'f>(
    form: &'f Form,
    error: &str,
    scope_keys: &[String],
) -> Option<Vec<&'f ResolvedLabel>> {
    let EditIntent::RuleFromFlow(f) = &form.intent else {
        return None;
    };
    let name = form.value("Ruleset").trim();
    let unknown = unknown_ruleset(name);
    let names_it = error == unknown || error.starts_with(&format!("{unknown};"));
    let scope = scope_of(&f.dst, scope_keys);
    (!is_href(name) && names_it && !scope.is_empty()).then_some(scope)
}

/// The error a `w` form comes back with when its ruleset name is unknown
/// and the destination has a scope: the planner's answer (similar names
/// included) and that `Ctrl-N` creates the ruleset.
pub fn new_ruleset_offer(form: &Form, error: &str, scope_keys: &[String]) -> Option<String> {
    unknown_ruleset_scope(form, error, scope_keys).map(|scope| {
        format!(
            "{error}; press Ctrl-N to create it scoped {}",
            labels_tokens(&scope)
        )
    })
}

/// `Ctrl-N`: the ruleset the form names, scoped like the destination, and
/// the rule to create in it once it exists.
pub fn new_ruleset_intent(
    form: &Form,
    index: Option<&NameIndex>,
    org_id: u64,
    scope_keys: &[String],
) -> Result<(WriteIntent, RuleDraft), String> {
    let EditIntent::RuleFromFlow(f) = &form.intent else {
        return Err("Ctrl-N creates a ruleset from a w form only".into());
    };
    let Some(WriteIntent::CreateRule {
        ruleset: RulesetTarget::Name(name),
        draft,
    }) = build_intent(form, index, org_id)?
    else {
        return Err("Ctrl-N creates a ruleset by name; type one".into());
    };
    let scope: Vec<ActorRef> = scope_of(&f.dst, scope_keys)
        .iter()
        .map(|l| ActorRef::new(ActorKind::Label, l.href.clone()))
        .collect();
    if scope.is_empty() {
        return Err("the destination has no scope labels to scope a ruleset by".into());
    }
    let ruleset = RuleSetDraft {
        name,
        description: None,
        enabled: true,
        scopes: vec![scope],
    };
    Ok((WriteIntent::CreateRuleSet(ruleset), draft))
}

const RULESET_HINT: &str = "ruleset name or href";

/// A label as an actor token; an unknown key or an awkward value keeps its href.
fn label_token(l: &ResolvedLabel) -> String {
    if l.key == "?" || l.value.contains([',', '|']) {
        l.href.clone()
    } else {
        format!("{}={}", l.key, l.value)
    }
}

/// The endpoint's labels whose key is a scope key, in `scope_keys` order
/// (a key the endpoint has no label for is absent).
pub fn scope_of<'e>(e: &'e ResolvedEndpoint, scope_keys: &[String]) -> Vec<&'e ResolvedLabel> {
    scope_keys
        .iter()
        .filter_map(|k| e.labels.iter().find(|l| &l.key == k))
        .collect()
}

/// The endpoint's tier: its label with the tier key.
pub fn tier_of<'e>(e: &'e ResolvedEndpoint, tier_key: &str) -> Option<&'e ResolvedLabel> {
    e.labels.iter().find(|l| l.key == tier_key)
}

/// Labels as `app=shop, env=prod`.
pub fn labels_tokens(labels: &[&ResolvedLabel]) -> String {
    join(labels.iter().map(|l| label_token(l)))
}

/// An endpoint without the labels asked for: its workload, else its IP
/// lists, else `all`.
fn unlabelled_tokens(e: &ResolvedEndpoint) -> Vec<String> {
    if let Some(h) = &e.workload_href {
        return vec![format!("workload:{h}")];
    }
    if !e.ip_lists.is_empty() {
        return e.ip_lists.iter().map(|n| format!("iplist:{n}")).collect();
    }
    vec!["all".into()]
}

/// An endpoint as actor tokens, the narrowest that is known: every label it
/// has, else its workload, else its IP lists, else `all`.
fn endpoint_tokens(e: &ResolvedEndpoint) -> Vec<String> {
    if e.labels.is_empty() {
        return unlabelled_tokens(e);
    }
    e.labels.iter().map(label_token).collect()
}

/// Label tokens, or the unlabelled fallback when there are none.
fn tokens_or_fallback(labels: Vec<&ResolvedLabel>, e: &ResolvedEndpoint) -> String {
    if labels.is_empty() {
        unlabelled_tokens(e).join(", ")
    } else {
        labels_tokens(&labels)
    }
}

/// The one live draft ruleset with a scope of exactly these labels (by
/// href); `None` when none or several have one.
fn ruleset_scoped(sets: &[RuleSet], scope: &[&ResolvedLabel]) -> Option<String> {
    let want: BTreeSet<&str> = scope.iter().map(|l| l.href.as_str()).collect();
    let equal = |s: &Vec<Actor>| {
        s.iter().all(|a| a.label.is_some())
            && s.iter()
                .filter_map(|a| a.label.as_ref().map(|r| r.href.as_str()))
                .collect::<BTreeSet<_>>()
                == want
    };
    let hits: Vec<&RuleSet> = sets
        .iter()
        .filter(|rs| rs.update_type.as_deref() != Some("delete"))
        .filter(|rs| rs.scopes.iter().any(equal))
        .collect();
    match hits.as_slice() {
        [one] => Some(one.name.clone()),
        _ => None,
    }
}

/// The `w` form in the label model: the ruleset whose scope is the
/// destination's scope, the destination's tier as providers, the source's
/// tier as consumers when it shares that scope (else its scope and tier,
/// unscoped), on the flow's port. The alternates are every attached label.
fn flow_rule_fields(f: &ResolvedFlow, p: FlowPrefill) -> (Vec<Field>, Vec<(usize, String)>) {
    let dst_scope = scope_of(&f.dst, p.scope_keys);
    let src_scope = scope_of(&f.src, p.scope_keys);
    let providers = tokens_or_fallback(tier_of(&f.dst, p.tier_key).into_iter().collect(), &f.dst);
    let same_scope = !dst_scope.is_empty()
        && dst_scope
            .iter()
            .map(|l| &l.href)
            .eq(src_scope.iter().map(|l| &l.href));
    let consumers = if same_scope {
        tokens_or_fallback(tier_of(&f.src, p.tier_key).into_iter().collect(), &f.src)
    } else {
        let mut labels = src_scope;
        labels.extend(tier_of(&f.src, p.tier_key));
        tokens_or_fallback(labels, &f.src)
    };
    let (ruleset, hint) = match ruleset_scoped(p.rulesets, &dst_scope) {
        _ if dst_scope.is_empty() => (String::new(), Cow::Borrowed(RULESET_HINT)),
        Some(name) => (name, Cow::Borrowed(RULESET_HINT)),
        None => (
            String::new(),
            Cow::Owned(format!("scope {}", labels_tokens(&dst_scope))),
        ),
    };
    let fields = vec![
        Field::new("Ruleset", ruleset, hint, true),
        Field::new("Providers", providers, ACTORS_HINT, true),
        Field::new("Consumers", consumers, ACTORS_HINT, true),
        Field::new("Services", f.service_text(), SERVICES_HINT, true),
        Field::new(
            "Description",
            format!(
                "allow {} to {} on {}",
                f.src.display_name(),
                f.dst.display_name(),
                f.service_text()
            ),
            "optional",
            false,
        ),
        Field::new("Enabled", "y", YES_NO_HINT, true),
        Field::new("Unscoped", yn(!same_scope), UNSCOPED_HINT, true),
    ];
    let alt = vec![
        (1, endpoint_tokens(&f.dst).join(", ")),
        (2, endpoint_tokens(&f.src).join(", ")),
    ];
    (fields, alt)
}

/// The Ruleset field of a `w` form: an href, checked without a request, or
/// a name, resolved at plan time with one draft ruleset read.
fn ruleset_target(text: &str, org_id: u64) -> Result<RulesetTarget, String> {
    if is_href(text) {
        return checked_policy_href(text, "rule_sets", org_id)
            .map(RulesetTarget::Href)
            .map_err(|e| format!("Ruleset: {e}"));
    }
    Ok(RulesetTarget::Name(text.to_string()))
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
///
/// `org_id` is the profile's org, for the href checkers.
pub fn build_intent(
    form: &Form,
    index: Option<&NameIndex>,
    org_id: u64,
) -> Result<Option<WriteIntent>, String> {
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
        EditIntent::CreateRuleSet => {
            let idx = need_index(index)?;
            let d = RuleSetDraft {
                name: v("Name"),
                description: non_empty(v("Description")),
                enabled: yes_no(form, "Enabled")?,
                scopes: resolve_scopes(&v("Scopes"), idx, org_id)?,
            };
            d.validate().map_err(invalid)?;
            Ok(Some(WriteIntent::CreateRuleSet(d)))
        }
        EditIntent::UpdateRuleSet(rs) => {
            let idx = need_index(index)?;
            let enabled = yes_no(form, "Enabled")?;
            let scopes = v("Scopes");
            // The field shows every scope, so a changed field replaces them all.
            let set_scopes = if scope_items(&scopes) == scope_items(&scopes_text(&rs.scopes, idx)) {
                None
            } else if rs
                .scopes
                .iter()
                .flatten()
                .any(|a| ActorRef::from_actor(a).is_err())
            {
                return Err(format!("Scopes: {CANNOT_EDIT_RULESET}"));
            } else {
                Some(resolve_scopes(&scopes, idx, org_id)?)
            };
            let patch = RuleSetPatch {
                name: changed(&rs.name, v("Name")),
                description: changed(rs.description.as_deref().unwrap_or(""), v("Description")),
                enabled: (enabled != rs.enabled).then_some(enabled),
                set_scopes,
            };
            patch.validate().map_err(invalid)?;
            if patch.is_empty() {
                Ok(None)
            } else {
                Ok(Some(WriteIntent::UpdateRuleSet {
                    href: rs.href.clone(),
                    patch,
                }))
            }
        }
        EditIntent::CreateRule { ruleset_href } => {
            let draft = rule_draft(form, need_index(index)?, org_id)?;
            Ok(Some(WriteIntent::CreateRule {
                ruleset: RulesetTarget::Href(ruleset_href.clone()),
                draft,
            }))
        }
        EditIntent::CreateDenyRule { ruleset_href } => {
            let draft = deny_rule_draft(form, need_index(index)?, org_id)?;
            Ok(Some(WriteIntent::CreateDenyRule {
                ruleset_href: ruleset_href.clone(),
                draft,
            }))
        }
        EditIntent::UpdateRule(r, _) => {
            let idx = need_index(index)?;
            let (add_providers, remove_providers) =
                actor_changes("Providers", &r.providers, &v("Providers"), idx, org_id)?;
            let (add_consumers, remove_consumers) =
                actor_changes("Consumers", &r.consumers, &v("Consumers"), idx, org_id)?;
            let (add_services, remove_services) =
                service_changes(&r.ingress_services, &v("Services"), idx, org_id)?;
            let enabled = yes_no(form, "Enabled")?;
            let unscoped = yes_no(form, "Unscoped")?;
            let patch = RulePatch {
                description: changed(r.description.as_deref().unwrap_or(""), v("Description")),
                enabled: (enabled != r.enabled).then_some(enabled),
                add_providers,
                remove_providers,
                add_consumers,
                remove_consumers,
                add_services,
                remove_services,
                unscoped_consumers: (unscoped != r.unscoped_consumers.unwrap_or(false))
                    .then_some(unscoped),
                resolve_labels_as: None,
            };
            if patch.is_empty() {
                Ok(None)
            } else {
                Ok(Some(WriteIntent::UpdateRule {
                    href: r.href.clone(),
                    patch,
                }))
            }
        }
        EditIntent::UpdateDenyRule(d, _) => {
            let idx = need_index(index)?;
            let (add_providers, remove_providers) =
                actor_changes("Providers", &d.providers, &v("Providers"), idx, org_id)?;
            let (add_consumers, remove_consumers) =
                actor_changes("Consumers", &d.consumers, &v("Consumers"), idx, org_id)?;
            let (add_services, remove_services) =
                service_changes(&d.ingress_services, &v("Services"), idx, org_id)?;
            let enabled = yes_no(form, "Enabled")?;
            let override_ = yes_no(form, "Override")?;
            let patch = DenyRulePatch {
                description: changed(d.description.as_deref().unwrap_or(""), v("Description")),
                enabled: (enabled != d.enabled).then_some(enabled),
                add_providers,
                remove_providers,
                add_consumers,
                remove_consumers,
                add_services,
                remove_services,
                override_: (override_ != d.override_).then_some(override_),
            };
            if patch.is_empty() {
                Ok(None)
            } else {
                Ok(Some(WriteIntent::UpdateDenyRule {
                    href: d.href.clone(),
                    patch,
                }))
            }
        }
        EditIntent::RuleFromFlow(_) => {
            let ruleset = ruleset_target(&v("Ruleset"), org_id)?;
            let draft = rule_draft(form, need_index(index)?, org_id)?;
            Ok(Some(WriteIntent::CreateRule { ruleset, draft }))
        }
        // Deletes never open a form (`start`); a form never carries one.
        EditIntent::DeleteLabel(_)
        | EditIntent::DeleteIpList(_)
        | EditIntent::DeleteService(_)
        | EditIntent::DeleteWorkload(_)
        | EditIntent::DeleteRuleSet(_)
        | EditIntent::DeleteRule { .. }
        | EditIntent::DeleteDenyRule { .. } => Err("a delete has no form".into()),
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

    /// For forms other than `w`.
    const NO_PREFILL: FlowPrefill<'static> = FlowPrefill {
        scope_keys: &[],
        tier_key: "role",
        rulesets: &[],
    };

    fn form(intent: EditIntent) -> Form {
        form_with(intent, &["app", "env"], "role", &[])
    }

    /// A form pre-filled with this label model and these draft rulesets.
    fn form_with(intent: EditIntent, scope: &[&str], tier: &str, sets: &[RuleSet]) -> Form {
        let scope_keys: Vec<String> = scope.iter().map(|k| k.to_string()).collect();
        let prefill = FlowPrefill {
            scope_keys: &scope_keys,
            tier_key: tier,
            rulesets: sets,
        };
        form_for(intent, Some(&*fixture_index()), prefill).expect("a create or update intent")
    }

    fn flow_form(flow: &ResolvedFlow) -> Form {
        form(EditIntent::RuleFromFlow(Box::new(flow.clone())))
    }

    fn label(href: &str, key: &str, value: &str) -> pce::ResolvedLabel {
        pce::ResolvedLabel {
            href: href.into(),
            key: key.into(),
            value: value.into(),
        }
    }

    fn set(f: &mut Form, label: &str, value: &str) {
        f.fields
            .iter_mut()
            .find(|x| x.label == label)
            .unwrap_or_else(|| panic!("no field {label}"))
            .value = value.into();
    }

    fn build(f: &Form) -> Result<Option<WriteIntent>, String> {
        build_intent(f, Some(&*fixture_index()), 5)
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
        let err = build_intent(&f, None, 5).unwrap_err();
        assert!(
            err.contains("label index not loaded, press r on Objects"),
            "{err}"
        );
        set(&mut f, "Labels", "");
        assert!(
            build_intent(&f, None, 5).unwrap().is_some(),
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
        let Start::Plan(w) = start(
            EditIntent::DeleteIpList(ip_lists()[1].clone()),
            None,
            NO_PREFILL,
        ) else {
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
        let Start::Plan(w) = start(
            EditIntent::DeleteLabel(labels()[0].clone()),
            None,
            NO_PREFILL,
        ) else {
            panic!("a delete opens no form")
        };
        assert_eq!(
            w,
            WriteIntent::DeleteLabel {
                href: "/orgs/5/labels/1".into(),
                name: "role=web".into()
            }
        );
        assert!(
            form_for(
                EditIntent::DeleteWorkload(workloads()[2].clone()),
                None,
                NO_PREFILL
            )
            .is_none()
        );
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

    #[test]
    fn rule_set_form_prefills_scopes_as_tokens() {
        let sets = rule_sets();
        let f = form(EditIntent::UpdateRuleSet(sets[0].clone()));
        assert_eq!(f.title, "Edit ruleset Shop-Prod");
        assert_eq!(
            f.fields.iter().map(|x| x.label).collect::<Vec<_>>(),
            vec!["Name", "Description", "Enabled", "Scopes"]
        );
        assert_eq!(f.value("Description"), "Ringfence");
        assert_eq!(f.value("Enabled"), "y");
        assert_eq!(f.value("Scopes"), "app=shop, env=prod");
        assert_eq!(
            build(&f).unwrap(),
            None,
            "a pre-filled form changes nothing"
        );
        let g = form(EditIntent::UpdateRuleSet(sets[1].clone()));
        assert_eq!(g.value("Enabled"), "n");
        assert_eq!(g.value("Scopes"), "", "[[]] applies everywhere");
        assert_eq!(build(&g).unwrap(), None);
        let n = form(EditIntent::CreateRuleSet);
        assert_eq!(n.title, "New ruleset");
        assert_eq!((n.value("Enabled"), n.value("Scopes")), ("y", ""));
    }

    #[test]
    fn rule_set_create_and_edit_build_drafts_and_patches() {
        let mut f = form(EditIntent::CreateRuleSet);
        set(&mut f, "Name", "Lab");
        set(&mut f, "Scopes", "role=web, env=prod | group:All prod envs");
        assert_eq!(
            build(&f).unwrap(),
            Some(WriteIntent::CreateRuleSet(RuleSetDraft {
                name: "Lab".into(),
                description: None,
                enabled: true,
                scopes: vec![
                    vec![
                        ActorRef::new(ActorKind::Label, "/orgs/5/labels/1"),
                        ActorRef::new(ActorKind::Label, "/orgs/5/labels/4"),
                    ],
                    vec![ActorRef::new(
                        ActorKind::LabelGroup,
                        "/orgs/5/sec_policy/draft/label_groups/10"
                    )],
                ],
            }))
        );
        set(&mut f, "Scopes", "iplist:Corp");
        assert_eq!(
            build(&f).unwrap_err(),
            "Scopes: scopes take labels and label groups only"
        );
        set(&mut f, "Scopes", "role=web |");
        let err = build(&f).unwrap_err();
        assert!(err.contains("an empty scope"), "{err}");
        set(&mut f, "Scopes", "");
        set(&mut f, "Enabled", "maybe");
        assert_eq!(build(&f).unwrap_err(), r#"Enabled: "maybe" is not y or n"#);
        // An edit sends what changed; the scopes only when their text changed.
        let mut e = form(EditIntent::UpdateRuleSet(rule_sets()[0].clone()));
        set(&mut e, "Description", "Ringfence shop");
        set(&mut e, "Enabled", "N");
        assert_eq!(
            build(&e).unwrap(),
            Some(WriteIntent::UpdateRuleSet {
                href: "/orgs/5/sec_policy/draft/rule_sets/100".into(),
                patch: RuleSetPatch {
                    description: Some("Ringfence shop".into()),
                    enabled: Some(false),
                    ..RuleSetPatch::default()
                },
            })
        );
        set(&mut e, "Scopes", "");
        let Some(WriteIntent::UpdateRuleSet { patch, .. }) = build(&e).unwrap() else {
            panic!("not an update")
        };
        assert_eq!(
            patch.set_scopes,
            Some(vec![]),
            "an emptied Scopes field means everywhere"
        );
        let Start::Plan(w) = start(
            EditIntent::DeleteRuleSet(rule_sets()[0].clone()),
            None,
            NO_PREFILL,
        ) else {
            panic!("a delete opens no form")
        };
        assert_eq!(
            w,
            WriteIntent::DeleteRuleSet {
                href: "/orgs/5/sec_policy/draft/rule_sets/100".into(),
                name: "Shop-Prod".into()
            }
        );
        assert_eq!(w.kind(), pce::write::ObjectKind::RuleSet);
    }

    const SHOP: &str = "/orgs/5/sec_policy/draft/rule_sets/100";

    #[test]
    fn rule_forms_create_allow_and_deny_rules_from_tokens() {
        let mut f = form(EditIntent::CreateRule {
            ruleset_href: SHOP.into(),
        });
        assert_eq!(f.title, "New allow rule in ruleset 100");
        assert_eq!(
            f.fields.iter().map(|x| x.label).collect::<Vec<_>>(),
            vec![
                "Providers",
                "Consumers",
                "Services",
                "Description",
                "Enabled",
                "Unscoped"
            ]
        );
        assert_eq!((f.value("Enabled"), f.value("Unscoped")), ("y", "n"));
        set(&mut f, "Providers", "role=db");
        set(&mut f, "Consumers", "role=web, !env=prod");
        set(&mut f, "Services", "PostgreSQL, 443/tcp");
        assert_eq!(
            build(&f).unwrap(),
            Some(WriteIntent::CreateRule {
                ruleset: RulesetTarget::Href(SHOP.into()),
                draft: RuleDraft {
                    description: None,
                    enabled: true,
                    providers: vec![ActorRef::new(ActorKind::Label, "/orgs/5/labels/2")],
                    consumers: vec![
                        ActorRef::new(ActorKind::Label, "/orgs/5/labels/1"),
                        ActorRef::new(ActorKind::Label, "/orgs/5/labels/4").excluded(),
                    ],
                    services: vec![
                        ServiceRef::Href("/orgs/5/sec_policy/draft/services/31".into()),
                        ServiceRef::Port(PortSpec::parse("443/tcp").unwrap()),
                    ],
                    unscoped_consumers: false,
                    resolve_labels_as: None,
                },
            })
        );
        set(&mut f, "Consumers", "workload:web-1");
        assert_eq!(
            build(&f).unwrap_err(),
            "Consumers: workload names resolve in the CLI; use workload:/orgs/… here"
        );
        let mut d = form(EditIntent::CreateDenyRule {
            ruleset_href: SHOP.into(),
        });
        assert_eq!(d.title, "New deny rule in ruleset 100");
        assert_eq!(
            d.fields.iter().map(|x| x.label).collect::<Vec<_>>(),
            vec![
                "Providers",
                "Consumers",
                "Services",
                "Description",
                "Enabled",
                "Override"
            ]
        );
        set(&mut d, "Providers", "all");
        set(&mut d, "Consumers", "iplist:Corp");
        set(&mut d, "Services", "23/tcp");
        set(&mut d, "Override", "y");
        assert_eq!(
            build(&d).unwrap(),
            Some(WriteIntent::CreateDenyRule {
                ruleset_href: SHOP.into(),
                draft: DenyRuleDraft {
                    description: None,
                    enabled: true,
                    providers: vec![ActorRef::all()],
                    consumers: vec![ActorRef::new(
                        ActorKind::IpList,
                        "/orgs/5/sec_policy/draft/ip_lists/21"
                    )],
                    services: vec![ServiceRef::Port(PortSpec::parse("23/tcp").unwrap())],
                    override_: true,
                },
            })
        );
    }

    #[test]
    fn rule_edit_patches_only_what_changed() {
        let corp = rule_sets()[0].rules[1].clone(); // role=web ← Corp, 443/tcp + 8000-8099/tcp
        let mut f = form(EditIntent::UpdateRule(corp, SHOP.into()));
        assert_eq!(f.title, "Edit allow rule 2 in ruleset 100");
        assert_eq!(f.value("Providers"), "role=web");
        assert_eq!(f.value("Consumers"), "iplist:Corp");
        assert_eq!(f.value("Services"), "443/tcp, 8000-8099/tcp");
        assert_eq!(f.value("Description"), "corp to web");
        assert_eq!(f.value("Unscoped"), "y");
        assert_eq!(
            build(&f).unwrap(),
            None,
            "a pre-filled form changes nothing"
        );
        set(&mut f, "Consumers", "iplist:Corp, role=db");
        set(&mut f, "Services", "443/tcp");
        set(&mut f, "Unscoped", "n");
        assert_eq!(
            build(&f).unwrap(),
            Some(WriteIntent::UpdateRule {
                href: "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2".into(),
                patch: RulePatch {
                    add_consumers: vec![ActorRef::new(ActorKind::Label, "/orgs/5/labels/2")],
                    remove_services: vec![ServiceRef::Port(
                        PortSpec::parse("8000-8099/tcp").unwrap()
                    )],
                    unscoped_consumers: Some(false),
                    ..RulePatch::default()
                },
            })
        );
        // Another spelling of a shown entry is neither added nor removed.
        let mut g = form(EditIntent::UpdateRule(
            rule_sets()[0].rules[0].clone(),
            SHOP.into(),
        ));
        assert_eq!(g.value("Consumers"), "role=web");
        set(&mut g, "Consumers", "label:role=web");
        assert_eq!(build(&g).unwrap(), None);
        let Start::Plan(w) = start(
            EditIntent::DeleteRule {
                href: "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1".into(),
                name: "allow role=db ← role=web PostgreSQL".into(),
            },
            None,
            NO_PREFILL,
        ) else {
            panic!("a delete opens no form")
        };
        assert_eq!(
            w,
            WriteIntent::DeleteRule {
                href: "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1".into(),
                name: "allow role=db ← role=web PostgreSQL".into(),
            }
        );
        assert_eq!(w.kind(), pce::write::ObjectKind::Rule);
    }

    #[test]
    fn inexpressible_entries_refuse_a_changed_field_only() {
        let mut rule = rule_sets()[0].rules[0].clone(); // web to db
        rule.providers.push(actor(
            json!({"virtual_service": {"href": "/orgs/5/sec_policy/draft/virtual_services/9"}}),
        ));
        rule.ingress_services
            .push(serde_json::from_value(json!({"proto": -1})).unwrap());
        let mut f = form(EditIntent::UpdateRule(rule, SHOP.into()));
        assert_eq!(
            f.value("Providers"),
            "role=db, /orgs/5/sec_policy/draft/virtual_services/9"
        );
        assert_eq!(f.value("Services"), "PostgreSQL, ?");
        assert_eq!(build(&f).unwrap(), None, "shown as it is, nothing changes");
        set(&mut f, "Description", "web to db, reviewed");
        let Some(WriteIntent::UpdateRule { patch, .. }) = build(&f).unwrap() else {
            panic!("not an update")
        };
        assert_eq!(
            patch,
            RulePatch {
                description: Some("web to db, reviewed".into()),
                ..RulePatch::default()
            },
            "the untouched lists are carried by the library as the PCE returned them"
        );
        set(&mut f, "Providers", "role=db");
        assert_eq!(
            build(&f).unwrap_err(),
            "Providers: this rule has entries the form cannot edit; use the CLI"
        );
        set(
            &mut f,
            "Providers",
            "role=db, /orgs/5/sec_policy/draft/virtual_services/9",
        );
        set(&mut f, "Services", "PostgreSQL, ?, 443/tcp");
        assert_eq!(
            build(&f).unwrap_err(),
            "Services: this rule has entries the form cannot edit; use the CLI"
        );
    }

    #[test]
    fn deny_rule_edit_flips_override_and_state() {
        let deny = rule_sets()[0].deny_rules[1].clone(); // override, role=db ← all, 22/tcp
        let mut f = form(EditIntent::UpdateDenyRule(deny, SHOP.into()));
        assert_eq!(f.title, "Edit deny rule 8 in ruleset 100");
        assert_eq!(
            (
                f.value("Providers"),
                f.value("Consumers"),
                f.value("Services"),
                f.value("Override")
            ),
            ("role=db", "all", "22/tcp", "y")
        );
        assert_eq!(build(&f).unwrap(), None);
        set(&mut f, "Override", "n");
        set(&mut f, "Enabled", "n");
        assert_eq!(
            build(&f).unwrap(),
            Some(WriteIntent::UpdateDenyRule {
                href: "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8".into(),
                patch: DenyRulePatch {
                    enabled: Some(false),
                    override_: Some(false),
                    ..DenyRulePatch::default()
                },
            })
        );
        let Start::Plan(w) = start(
            EditIntent::DeleteDenyRule {
                href: "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8".into(),
                name: "override_deny role=db ← All workloads 22/tcp".into(),
            },
            None,
            NO_PREFILL,
        ) else {
            panic!("a delete opens no form")
        };
        assert_eq!(w.kind(), pce::write::ObjectKind::DenyRule);
    }

    #[test]
    fn ruleset_names_resolve_to_one_live_draft_ruleset() {
        let mut sets = rule_sets();
        assert_eq!(ruleset_href_named(&sets, " shop-prod ").unwrap(), SHOP);
        assert_eq!(
            ruleset_href_named(&sets, "Shop").unwrap_err(),
            r#"no ruleset named "Shop"; similar: Shop-Prod"#
        );
        sets[0].update_type = Some("delete".into());
        assert_eq!(
            ruleset_href_named(&sets, "Shop-Prod").unwrap_err(),
            r#"no ruleset named "Shop-Prod""#,
            "a pending deletion does not count"
        );
        let mut twin = sets[1].clone();
        twin.href = "/orgs/5/sec_policy/draft/rule_sets/102".into();
        sets.push(twin);
        assert_eq!(
            ruleset_href_named(&sets, "Global-Infra").unwrap_err(),
            r#""Global-Infra" names several rulesets: /orgs/5/sec_policy/draft/rule_sets/101, /orgs/5/sec_policy/draft/rule_sets/102; use the href"#
        );
    }

    #[test]
    fn flow_prefill_covers_every_endpoint_shape() {
        let flows = crate::tui::test_support::fixture_flows();
        let f = flow_form(&flows[1]); // Corp -> host-1 (app=shop, env=prod, role=web), 8443/tcp
        assert_eq!(f.title, "Allow rule from flow");
        assert_eq!(
            f.fields.iter().map(|x| x.label).collect::<Vec<_>>(),
            vec![
                "Ruleset",
                "Providers",
                "Consumers",
                "Services",
                "Description",
                "Enabled",
                "Unscoped"
            ]
        );
        assert_eq!(f.value("Providers"), "role=web", "the tier only");
        assert_eq!(
            f.value("Consumers"),
            "iplist:Corp",
            "no labels: the narrowest known"
        );
        assert_eq!(f.value("Services"), "8443/tcp");
        assert_eq!(
            f.value("Description"),
            "allow 10.1.4.9 to host-1.example on 8443/tcp"
        );
        assert_eq!((f.value("Enabled"), f.value("Unscoped")), ("y", "y"));
        let g = flow_form(&flows[2]); // unmanaged-nas -> host-2, icmp
        assert_eq!(g.value("Providers"), "role=db");
        assert_eq!(g.value("Consumers"), "workload:/orgs/5/workloads/aaaa-3");
        assert_eq!(g.value("Unscoped"), "y", "a source without labels");
        assert_eq!(g.value("Services"), "icmp");
        assert_eq!(
            g.value("Description"),
            "allow unmanaged-nas to host-2.example on icmp"
        );
        let mut odd = flows[1].clone();
        odd.src = pce::traffic::ResolvedEndpoint {
            ip: Some("10.9.9.9".into()),
            ..pce::traffic::ResolvedEndpoint::default()
        };
        odd.dst
            .labels
            .push(label("/orgs/5/labels/99", "?", "/orgs/5/labels/99"));
        let o = flow_form(&odd);
        assert_eq!(
            o.value("Consumers"),
            "all",
            "nothing known about the source"
        );
        assert_eq!(o.value("Providers"), "role=web");
        assert_eq!(
            o.alt.as_ref().unwrap()[0],
            (
                1,
                "app=shop, env=prod, role=web, /orgs/5/labels/99".to_string()
            ),
            "an unknown label keeps its href among all labels"
        );
        odd.src.workload_href = Some("/orgs/5/workloads/aaaa-9".into());
        odd.src.ip_lists = vec!["Any (0.0.0.0/0 and ::/0)".into()];
        let w = flow_form(&odd);
        assert_eq!(
            w.value("Consumers"),
            "workload:/orgs/5/workloads/aaaa-9",
            "a workload is narrower than the IP lists of its address"
        );
    }

    #[test]
    fn flow_prefill_follows_the_label_model() {
        let flows = crate::tui::test_support::fixture_flows();
        // Same scope (app=shop, env=prod) on both sides: tiers only, scoped.
        let same = flow_form(&flows[0]);
        assert_eq!(same.value("Providers"), "role=db");
        assert_eq!(same.value("Consumers"), "role=web");
        assert_eq!(same.value("Unscoped"), "n");
        // A source in another scope: its scope and tier, unscoped.
        let mut other = flows[0].clone();
        other.src.labels = vec![
            label("/orgs/5/labels/3", "app", "shop"),
            label("/orgs/5/labels/7", "env", "dev"),
            label("/orgs/5/labels/5", "loc", "eu"),
            label("/orgs/5/labels/2", "role", "db"),
        ];
        let o = flow_form(&other);
        assert_eq!(
            o.value("Consumers"),
            "app=shop, env=dev, role=db",
            "loc is never pre-filled"
        );
        assert_eq!(o.value("Unscoped"), "y");
        // No tier on the destination: the narrowest known instead.
        let mut no_tier = flows[0].clone();
        no_tier.dst.labels.retain(|l| l.key != "role");
        no_tier
            .dst
            .labels
            .push(label("/orgs/5/labels/5", "loc", "eu"));
        let n = flow_form(&no_tier);
        assert_eq!(n.value("Providers"), "workload:/orgs/5/workloads/aaaa-2");
        // No scope on the destination: the hint stays, consumers are outside.
        let mut no_scope = flows[0].clone();
        no_scope.dst.labels.retain(|l| l.key == "role");
        let s = flow_form(&no_scope);
        assert_eq!(s.value("Ruleset"), "");
        assert_eq!(s.fields[0].hint, RULESET_HINT);
        assert_eq!(s.value("Providers"), "role=db");
        assert_eq!(s.value("Consumers"), "app=shop, env=prod, role=web");
        assert_eq!(s.value("Unscoped"), "y");
        // Configured keys: scoped by app alone, tiers by env.
        let c = form_with(
            EditIntent::RuleFromFlow(Box::new(flows[0].clone())),
            &["app"],
            "env",
            &[],
        );
        assert_eq!(c.value("Providers"), "env=prod");
        assert_eq!(c.value("Consumers"), "env=prod");
        assert_eq!(c.value("Unscoped"), "n");
        let c = form_with(
            EditIntent::RuleFromFlow(Box::new(flows[1].clone())),
            &["app"],
            "env",
            &[],
        );
        assert_eq!(c.fields[0].hint, "scope app=shop");
    }

    #[test]
    fn the_ruleset_is_the_one_whose_scope_is_the_destination_scope() {
        let flows = crate::tui::test_support::fixture_flows();
        let intent = || EditIntent::RuleFromFlow(Box::new(flows[1].clone()));
        // Shop-Prod is scoped app=shop, env=prod; Global-Infra has an empty scope.
        let one = form_with(intent(), &["app", "env"], "role", &rule_sets());
        assert_eq!(one.value("Ruleset"), "Shop-Prod");
        assert_eq!(one.fields[0].hint, RULESET_HINT);
        // A ruleset with several scopes matches through any of them.
        let mut sets = rule_sets();
        let mut more = sets[0].clone();
        more.href = "/orgs/5/sec_policy/draft/rule_sets/102".into();
        more.name = "Shop-Prod-2".into();
        more.scopes.insert(
            0,
            vec![actor(json!({"label": {"href": "/orgs/5/labels/5"}}))],
        );
        sets.push(more.clone());
        let two = form_with(intent(), &["app", "env"], "role", &sets);
        assert_eq!(two.value("Ruleset"), "", "two match: the user chooses");
        assert_eq!(two.fields[0].hint, "scope app=shop, env=prod");
        // A ruleset pending deletion does not count.
        sets[2].update_type = Some("delete".into());
        let back = form_with(intent(), &["app", "env"], "role", &sets);
        assert_eq!(back.value("Ruleset"), "Shop-Prod");
        // None matches: empty, the scope in the hint.
        let none = form_with(intent(), &["app", "env"], "role", &rule_sets()[1..]);
        assert_eq!(none.value("Ruleset"), "");
        assert_eq!(none.fields[0].hint, "scope app=shop, env=prod");
    }

    #[test]
    fn a_flow_form_keeps_every_label_one_key_away() {
        let flows = crate::tui::test_support::fixture_flows();
        let f = flow_form(&flows[0]);
        assert_eq!(
            f.alt,
            Some(vec![
                (1, "app=shop, env=prod, role=db".to_string()),
                (2, "app=shop, env=prod, role=web".to_string()),
            ])
        );
        assert!(!f.expanded);
        assert_eq!(
            form(EditIntent::CreateLabel).alt,
            None,
            "other forms have none"
        );
    }

    #[test]
    fn a_flow_rule_targets_a_ruleset_by_name_or_checked_href() {
        let flows = crate::tui::test_support::fixture_flows();
        let mut f = form(EditIntent::RuleFromFlow(Box::new(flows[1].clone())));
        set(&mut f, "Ruleset", " Shop-Prod ");
        let want = RuleDraft {
            description: Some("allow 10.1.4.9 to host-1.example on 8443/tcp".into()),
            enabled: true,
            providers: vec![ActorRef::new(ActorKind::Label, "/orgs/5/labels/1")],
            consumers: vec![ActorRef::new(
                ActorKind::IpList,
                "/orgs/5/sec_policy/draft/ip_lists/21",
            )],
            services: vec![ServiceRef::Port(PortSpec::parse("8443/tcp").unwrap())],
            unscoped_consumers: true,
            resolve_labels_as: None,
        };
        assert_eq!(
            build(&f).unwrap(),
            Some(WriteIntent::CreateRule {
                ruleset: RulesetTarget::Name("Shop-Prod".into()),
                draft: want.clone(),
            })
        );
        set(&mut f, "Ruleset", SHOP);
        assert_eq!(
            build(&f).unwrap(),
            Some(WriteIntent::CreateRule {
                ruleset: RulesetTarget::Href(SHOP.into()),
                draft: want,
            })
        );
        set(&mut f, "Ruleset", "/orgs/9/sec_policy/draft/rule_sets/100");
        let err = build(&f).unwrap_err();
        assert!(err.starts_with("Ruleset: "), "{err}");
    }
}
