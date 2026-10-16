//! The policy generator: the allow rules one app needs, from its own
//! observed traffic. [`rows`] turns the app's flows into rows (one per
//! peer and side); [`propose`] turns the rows and the slider stop into
//! rules, merged, and classified against the draft rulesets (create, add
//! services, exists). Pure: no requests. The TUI's generator and the CLI's
//! `gen` share it.
pub mod dedupe;
pub mod derive;
pub mod rows;
pub mod services;
pub mod target;

pub use derive::{GenInput, propose};
pub use rows::{MANY_SERVICES, MAX_IP_ROWS, excluded, ip_list_name, is_any_ip_list, rows};
pub use services::{Expr, express, flow_spec, fold_ports};
pub use target::{
    Covering, DEFAULT_RULESET_NAME, RulesetChoice, check_ruleset_name, default_ruleset_name,
    keys_refusal, narrow, pick_ruleset, render_ruleset_name, rulesets_covering,
};

use crate::models::PolicyDecision;
use crate::resolve::{NameIndex, ResolvedLabel};
use crate::traffic::{GroupKey, ResolvedFlow};
use crate::write::batch::step_placeholder;
use crate::write::{
    ActorKind, ActorRef, IpListDraft, PortSpec, RuleDraft, RuleSetDraft, ServiceRef,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

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
            RowFlag::UnsupportedProto(6) => "tcp without a port: not proposed".into(),
            RowFlag::UnsupportedProto(17) => "udp without a port: not proposed".into(),
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

    fn merge(&mut self, o: &DecisionCounts) {
        self.allowed += o.allowed;
        self.blocked += o.blocked;
        if let Some(n) = o.draft_allowed {
            *self.draft_allowed.get_or_insert(0) += n;
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
    /// The flow records behind it.
    pub flows: u64,
    /// Those an override deny rule of the target ruleset blocks whatever
    /// this rule allows (`propose` finds them), and those deny rules.
    pub denied_flows: u64,
    pub denied_by: Vec<String>,
    /// The flow records behind it as the override-deny check needs them
    /// (ends and service), by count: `refresh` checks them again.
    pub seen: BTreeMap<SeenFlow, u64>,
}

/// One end of a flow record as the override-deny check reads it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SeenEnd {
    pub workload: Option<String>,
    /// Label hrefs.
    pub labels: Vec<String>,
    /// The IP lists Explorer reported, by name.
    pub ip_lists: Vec<String>,
}

/// A flow record's ends and service, without its counters.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SeenFlow {
    pub src: SeenEnd,
    pub dst: SeenEnd,
    pub spec: PortSpec,
}

impl SeenFlow {
    pub fn of(f: &ResolvedFlow) -> Self {
        let end = |e: &crate::traffic::ResolvedEndpoint| SeenEnd {
            workload: e.workload_href.clone(),
            labels: e.labels.iter().map(|l| l.href.clone()).collect(),
            ip_lists: e.ip_lists.clone(),
        };
        SeenFlow {
            src: end(&f.src),
            dst: end(&f.dst),
            spec: services::flow_spec(f),
        }
    }
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
    /// Behind a rule whose ruleset is ambiguous or cannot be named.
    pub no_target: u64,
    /// Behind a rule, but an override deny rule blocks them anyway.
    pub override_deny: u64,
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

    /// Targets, actions and override denies again against `drafts`: a
    /// fresh draft read at plan time, or the rulesets just loaded. `picked`: the app ruleset
    /// `R` chose among several.
    pub fn refresh(
        &mut self,
        drafts: &[crate::models::RuleSet],
        idx: &NameIndex,
        template: &str,
        picked: Option<&str>,
    ) {
        self.targets = Vec::with_capacity(self.scopes.len());
        for (i, s) in self.scopes.iter().enumerate() {
            let mut t = target::resolve(s, drafts, idx, template, picked.filter(|_| i == 0));
            // Two new rulesets of one name: the batch would create one.
            if let Target::Create(d) = &t
                && self
                    .targets
                    .iter()
                    .any(|o| matches!(o, Target::Create(e) if e.name.eq_ignore_ascii_case(&d.name)))
            {
                t = Target::None(format!(
                    "{} would get the name {} of another new ruleset",
                    s.text(),
                    d.name
                ));
            }
            self.targets.push(t);
        }
        for r in &mut self.rules {
            let (scope, t) = (&self.scopes[r.target], &self.targets[r.target]);
            if !target::narrow(scope, t, drafts) {
                pin_to_scope(&mut r.providers, scope, idx);
                if !r.unscoped_consumers {
                    pin_to_scope(&mut r.consumers, scope, idx);
                }
            }
            r.action = dedupe::classify(r, t, drafts, idx);
            (r.denied_flows, r.denied_by) = dedupe::override_denied(&r.seen, drafts, idx);
        }
        self.recount();
    }

    /// `covered`, `no_target`, `override_deny` and `uncovered` from the
    /// rules; the other reasons stay as `propose` counted them.
    pub(crate) fn recount(&mut self) {
        let c = &mut self.coverage;
        (c.covered, c.no_target, c.override_deny) = (0, 0, 0);
        for r in &self.rules {
            match self.targets.get(r.target) {
                Some(Target::Existing { .. } | Target::Create(_)) => {
                    c.covered += r.flows - r.denied_flows.min(r.flows);
                    c.override_deny += r.denied_flows.min(r.flows);
                }
                _ => c.no_target += r.flows,
            }
        }
        c.uncovered = c.unchecked
            + c.folded_ips
            + c.unsupported_proto
            + c.broadcast_multicast
            + c.no_target
            + c.override_deny;
    }
}

/// An in-scope side of a rule whose ruleset is scoped wider than the
/// target scope (fewer keys, a label group, a second scope): `ams` and a
/// bare role there would reach every workload of that wider scope, so the
/// side names the scope's labels itself (`ams` → `app=shop, env=prod`,
/// `role=db` → `app=shop, env=prod, role=db`). Sides with other actors,
/// or already naming a scope key, stay.
fn pin_to_scope(side: &mut Vec<GenActor>, scope: &TargetScope, idx: &NameIndex) {
    let scope_keys: BTreeSet<&str> = scope.labels.iter().map(|l| l.key.as_str()).collect();
    let mut kept = Vec::new();
    for a in side.iter() {
        let GenActor::Ref(r) = a else {
            return;
        };
        if r.exclusion {
            return;
        }
        match r.kind {
            ActorKind::All => {}
            ActorKind::Label => {
                let key = r
                    .href
                    .as_deref()
                    .and_then(|h| idx.label(h))
                    .map(|l| l.key.as_str());
                match key {
                    Some(k) if !scope_keys.contains(k) => kept.push(a.clone()),
                    _ => return,
                }
            }
            _ => return,
        }
    }
    if side.is_empty() || scope.labels.is_empty() {
        return;
    }
    *side = scope
        .labels
        .iter()
        .map(|l| GenActor::label(&l.href))
        .chain(kept)
        .collect();
}

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
            flows: 1,
            denied_flows: 0,
            denied_by: Vec::new(),
            seen: BTreeMap::new(),
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
    fn two_new_rulesets_of_one_name_leave_the_second_without_a_target() {
        let scope = |app: &str, env: &str, n: u32| TargetScope {
            values: vec![app.into(), env.into()],
            labels: vec![
                ResolvedLabel {
                    href: format!("/orgs/5/labels/{n}"),
                    key: "app".into(),
                    value: app.into(),
                },
                ResolvedLabel {
                    href: format!("/orgs/5/labels/{}", n + 1),
                    key: "env".into(),
                    value: env.into(),
                },
            ],
        };
        let mut p = Proposal {
            scopes: vec![scope("ab", "c", 50), scope("a", "bc", 60)],
            targets: Vec::new(),
            ip_lists: Vec::new(),
            rules: vec![rule(1, RuleAction::Create)],
            coverage: Coverage::default(),
        };
        let idx = NameIndex::new(&[], &[], &[], &[]);
        p.refresh(&[], &idx, "RF-{app}{env}", None);
        assert_eq!(p.targets[0].name(), "RF-abc");
        assert_eq!(
            p.targets[1],
            Target::None("a/bc would get the name RF-abc of another new ruleset".into())
        );
        assert_eq!(p.coverage.no_target, 1);
        assert!(p.blocker().is_some());
    }

    #[test]
    fn row_flags_read_as_the_rows_show_them() {
        let keys = vec!["app".to_string(), "env".to_string()];
        assert_eq!(
            RowFlag::EnvMismatch.text(Side::Inbound, &keys),
            "env mismatch"
        );
        assert_eq!(
            RowFlag::UsesIpList("Corp".into()).text(Side::Inbound, &keys),
            "uses IP list Corp"
        );
        assert_eq!(
            RowFlag::ManyServices(40).text(Side::Inbound, &keys),
            "40 services: consider app·all or role·all"
        );
        assert_eq!(
            RowFlag::Ephemeral.text(Side::Outbound, &keys),
            "ephemeral port?"
        );
    }
}
