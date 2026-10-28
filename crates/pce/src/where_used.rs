//! Where-used: every place the draft policy and the label groups name one
//! label, label group, IP list or service. Ruleset scopes (labels and label
//! groups), rule providers and consumers (labels, label groups, IP lists),
//! rule services, and the label groups that hold it, directly or through a
//! nested group (`via`); for a label also the pairing profiles that give it
//! ([`pairing_profiles`]). A label group the target reaches through is
//! followed into scopes and rules too: a rule naming a group that holds the
//! label is listed `via` that group. Pure: the TUI and the CLI hand in what
//! they loaded; how many workloads carry a label is theirs to count
//! ([`WorkloadCount`]).
//!
//! Not searched: virtual services, enforcement boundaries as their own
//! objects and container workload profiles ([`NOT_SEARCHED`]). A pairing
//! profile holds labels only, so for the other kinds there is nothing in
//! them to find.
use crate::models::pairing_profiles::LOCKS;
use crate::models::{
    Actor, DenyRule, IngressService, LabelGroup, PairingProfile, RuleSet, Workload,
};
use crate::resolve::{RuleKind, canon, pane_order};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// What where-used leaves out, in one line, so an empty result is not read
/// as "unused everywhere".
pub const NOT_SEARCHED: &str =
    "not searched: virtual services, enforcement boundaries, container workload profiles";

/// The kind of object searched for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectType {
    Label,
    LabelGroup,
    IpList,
    Service,
}

impl ObjectType {
    /// `label`, `label group`, `IP list`, `service`.
    pub fn word(self) -> &'static str {
        match self {
            ObjectType::Label => "label",
            ObjectType::LabelGroup => "label group",
            ObjectType::IpList => "IP list",
            ObjectType::Service => "service",
        }
    }
}

/// The object searched for, by href (draft or active: compared as drafts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub kind: ObjectType,
    pub href: String,
}

impl Target {
    pub fn new(kind: ObjectType, href: impl Into<String>) -> Self {
        Self {
            kind,
            href: href.into(),
        }
    }
}

/// What a reference is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    /// A ruleset whose scopes name it.
    Scope,
    /// A rule (allow, deny or override deny) naming it on one side.
    Rule,
    /// A label group holding it.
    LabelGroup,
    /// A pairing profile giving a label to the workloads it pairs.
    PairingProfile,
    /// The workloads carrying a label (their count).
    Workloads,
}

impl RefKind {
    /// The sections, in the order they are listed.
    pub const ALL: [RefKind; 5] = [
        RefKind::Scope,
        RefKind::Rule,
        RefKind::LabelGroup,
        RefKind::PairingProfile,
        RefKind::Workloads,
    ];

    /// The section's title: `Rulesets`, `Rules`, `Label groups`, `Pairing
    /// profiles`, `Workloads`.
    pub fn title(self) -> &'static str {
        match self {
            RefKind::Scope => "Rulesets",
            RefKind::Rule => "Rules",
            RefKind::LabelGroup => "Label groups",
            RefKind::PairingProfile => "Pairing profiles",
            RefKind::Workloads => "Workloads",
        }
    }

    /// `scope`, `rule`, `label_group`, `pairing_profile`, `workloads`, as
    /// JSON names it.
    pub fn as_str(self) -> &'static str {
        match self {
            RefKind::Scope => "scope",
            RefKind::Rule => "rule",
            RefKind::LabelGroup => "label_group",
            RefKind::PairingProfile => "pairing_profile",
            RefKind::Workloads => "workloads",
        }
    }
}

/// The side of a rule that names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RefSide {
    Provider,
    Consumer,
    Service,
}

impl RefSide {
    pub fn as_str(self) -> &'static str {
        match self {
            RefSide::Provider => "provider",
            RefSide::Consumer => "consumer",
            RefSide::Service => "service",
        }
    }
}

/// A rule's action as where-used words it: `allow`, `deny`, `override deny`.
pub fn action_text(kind: RuleKind) -> &'static str {
    match kind {
        RuleKind::Allow => "allow",
        RuleKind::Deny => "deny",
        RuleKind::OverrideDeny => "override deny",
    }
}

/// How many workloads carry a label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct WorkloadCount {
    pub count: usize,
    /// `false`: a lower bound (`≥count`): the loaded workload list was
    /// truncated, or a one-row probe found one.
    pub exact: bool,
}

impl WorkloadCount {
    /// From a loaded workload list: those carrying `href`; a truncated list
    /// gives a lower bound. Counted as `u`'s usage counts are
    /// ([`crate::workload_labels`]).
    pub fn from_loaded(workloads: &[Workload], truncated: bool, href: &str) -> Self {
        let href = canon(href);
        let count = crate::workload_labels(workloads)
            .iter()
            .filter(|set| set.contains(href.as_ref()))
            .count();
        Self {
            count,
            exact: !truncated,
        }
    }

    /// From a read filtered by the label with `max_results=1`: no row is an
    /// exact zero; one row says only "at least one". The PCE's
    /// `X-Total-Count` cannot say more: it is the size of the whole
    /// collection, whatever the filter (see `Pce::cached_list`).
    pub fn from_probe(rows: usize) -> Self {
        Self {
            count: rows.min(1),
            exact: rows == 0,
        }
    }

    /// `12`, or `≥1` for a lower bound.
    pub fn text(&self) -> String {
        if self.exact {
            self.count.to_string()
        } else {
            format!("≥{}", self.count)
        }
    }
}

/// One reference. `href` is the rule's (`Rule`), the ruleset's (`Scope`),
/// the group's (`LabelGroup`), the profile's (`PairingProfile`) or the
/// label's (`Workloads`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reference {
    pub kind: RefKind,
    /// The ruleset's name (`Scope`, `Rule`).
    pub ruleset: Option<String>,
    pub ruleset_href: Option<String>,
    /// The rule's position in its ruleset as the Policy screen lists it
    /// ([`pane_order`]), from 1.
    pub rule: Option<usize>,
    pub side: Option<RefSide>,
    pub action: Option<RuleKind>,
    /// A draft change not provisioned yet, as the PCE's `update_type`
    /// names it (`create`, `update`, `delete`): the rule's, else its
    /// ruleset's (a scope: the ruleset's; a group: the group's). `None`:
    /// provisioned as it is.
    pub pending: Option<String>,
    /// The rule or scope names it as an exclusion (`exclusion: true`).
    pub excluded: bool,
    /// The rule or its ruleset is disabled (a scope: the ruleset; a
    /// pairing profile: `enabled: false`).
    pub disabled: bool,
    /// The label group it is reached through, when not named directly.
    pub via: Option<String>,
    /// The label group's or the pairing profile's name.
    pub name: Option<String>,
    /// How many workloads carry the label (`Workloads`).
    pub count: Option<usize>,
    /// Whether `count` is exact (`Workloads`).
    pub exact: Option<bool>,
    /// Whether the pairing profile locks the label's key (`PairingProfile`
    /// of a key with a lock: role, app, env, loc; `None` otherwise).
    pub locked: Option<bool>,
    pub href: String,
}

impl Reference {
    fn blank(kind: RefKind, href: &str) -> Self {
        Self {
            kind,
            ruleset: None,
            ruleset_href: None,
            rule: None,
            side: None,
            action: None,
            pending: None,
            excluded: false,
            disabled: false,
            via: None,
            name: None,
            count: None,
            exact: None,
            locked: None,
            href: href.to_string(),
        }
    }

    /// The `Workloads` row of label `href`.
    pub fn workloads(href: &str, count: WorkloadCount) -> Self {
        Self {
            count: Some(count.count),
            exact: Some(count.exact),
            ..Self::blank(RefKind::Workloads, href)
        }
    }

    /// `excluded`, `pending create|update|delete`, `disabled`, those that
    /// apply, space-joined.
    pub fn flags(&self) -> String {
        let mut f = Vec::new();
        if self.excluded {
            f.push("excluded".to_string());
        }
        if let Some(u) = &self.pending {
            f.push(format!("pending {u}"));
        }
        if self.disabled {
            f.push("disabled".to_string());
        }
        f.join(" ")
    }

    /// The row on one line: [`Self::what`], then `via G`.
    pub fn line(&self) -> String {
        match &self.via {
            Some(v) => format!("{} · via {v}", self.what()),
            None => self.what(),
        }
    }

    /// The row without its `via`: `Shop-Prod · #2 · provider · allow` (a
    /// rule), `Shop-Prod` (a scope), the group's name, the profile's name
    /// (with `locked`), `2 workloads`; then the flags.
    pub fn what(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        match self.kind {
            RefKind::Rule => {
                parts.push(self.ruleset.clone().unwrap_or_default());
                parts.push(format!("#{}", self.rule.unwrap_or(0)));
                parts.extend(self.side.map(|s| s.as_str().to_string()));
                parts.extend(self.action.map(|a| action_text(a).to_string()));
            }
            RefKind::Scope => parts.push(self.ruleset.clone().unwrap_or_default()),
            RefKind::LabelGroup => parts.push(self.name.clone().unwrap_or_default()),
            RefKind::PairingProfile => {
                parts.push(self.name.clone().unwrap_or_default());
                if self.locked == Some(true) {
                    parts.push("locked".to_string());
                }
            }
            RefKind::Workloads => {
                let n = WorkloadCount {
                    count: self.count.unwrap_or(0),
                    exact: self.exact.unwrap_or(true),
                };
                let word = if n.count == 1 && n.exact {
                    "workload"
                } else {
                    "workloads"
                };
                parts.push(format!("{} {word}", n.text()));
            }
        }
        let flags = self.flags();
        if !flags.is_empty() {
            parts.push(flags);
        }
        parts.join(" · ")
    }
}

/// What the search reads.
#[derive(Debug, Clone, Copy)]
pub struct Inputs<'a> {
    /// The draft rulesets, with their inline deny rules.
    pub rule_sets: &'a [RuleSet],
    /// Deny rules read from the per-ruleset endpoint, by ruleset href, for
    /// rulesets whose inline list is empty.
    pub fallback_deny: &'a HashMap<String, Vec<DenyRule>>,
    pub groups: &'a [LabelGroup],
    /// This PCE may keep deny rules apart from the ruleset list: a ruleset
    /// with no inline deny rules and none read apart counts in
    /// [`WhereUsed::deny_unread`].
    pub deny_apart: bool,
}

/// The references found, in section order: scopes, rules, label groups.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WhereUsed {
    pub refs: Vec<Reference>,
    /// Rulesets whose deny rules were not read (they may name it).
    pub deny_unread: usize,
}

impl WhereUsed {
    /// How many references of `kind`.
    pub fn count(&self, kind: RefKind) -> usize {
        self.refs.iter().filter(|r| r.kind == kind).count()
    }
}

/// The pairing profile field locking label key `key` (`role_label_lock`
/// for `role`), as [`LOCKS`] names it; `None` for a key with no lock.
pub fn label_lock_field(key: &str) -> Option<&'static str> {
    LOCKS
        .iter()
        .find(|(name, field)| *name == key && field.ends_with("_label_lock"))
        .map(|(_, field)| *field)
}

/// The pairing profiles giving label `href` (of key `key`), in list order:
/// one row each, `locked` from the profile's lock of that key (`None` when
/// the key has no lock or the profile does not say).
pub fn pairing_profiles(href: &str, key: &str, profiles: &[PairingProfile]) -> Vec<Reference> {
    let want = canon(href);
    let lock = label_lock_field(key);
    profiles
        .iter()
        .filter(|p| p.labels.iter().any(|l| canon(&l.href) == want))
        .map(|p| Reference {
            name: Some(p.name.clone()),
            disabled: p.enabled == Some(false),
            locked: lock.and_then(|f| p.extra.get(f)).and_then(|v| v.as_bool()),
            ..Reference::blank(RefKind::PairingProfile, &p.href)
        })
        .collect()
}

/// The groups holding `target`, directly (`via: None`) or through a nested
/// group (`via`: the member group it is held through), nearest first. A
/// cycle of groups ends: each group is listed once, the target never.
pub fn holders<'a>(
    target: &Target,
    groups: &'a [LabelGroup],
) -> Vec<(&'a LabelGroup, Option<&'a LabelGroup>)> {
    let t = canon(&target.href).into_owned();
    let mut out: Vec<(&LabelGroup, Option<&LabelGroup>)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::from([t.clone()]);
    let direct = |g: &LabelGroup| match target.kind {
        ObjectType::Label => g.labels.iter().any(|r| canon(&r.href) == t),
        ObjectType::LabelGroup => g.sub_groups.iter().any(|r| canon(&r.href) == t),
        ObjectType::IpList | ObjectType::Service => false,
    };
    let mut frontier: Vec<&LabelGroup> = Vec::new();
    for g in groups.iter().filter(|g| direct(g)) {
        if seen.insert(canon(&g.href).into_owned()) {
            out.push((g, None));
            frontier.push(g);
        }
    }
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for g in groups {
            if seen.contains(canon(&g.href).as_ref()) {
                continue;
            }
            let via = g.sub_groups.iter().find_map(|r| {
                frontier
                    .iter()
                    .copied()
                    .find(|f| canon(&f.href) == canon(&r.href))
            });
            if let Some(via) = via {
                seen.insert(canon(&g.href).into_owned());
                out.push((g, Some(via)));
                next.push(g);
            }
        }
        frontier = next;
    }
    out
}

/// Every reference to `target` in `inputs`: scopes, then rules (each
/// ruleset's in Policy-screen order, provider, consumer, service), then the
/// label groups holding it.
pub fn where_used(target: &Target, inputs: &Inputs) -> WhereUsed {
    let held = holders(target, inputs.groups);
    // What an actor may name to reach the target, and through which group.
    let mut reach: HashMap<String, Option<String>> = HashMap::new();
    reach.insert(canon(&target.href).into_owned(), None);
    for (g, _) in &held {
        reach.insert(canon(&g.href).into_owned(), Some(g.name.clone()));
    }
    // An actor that reaches it: through which group, and whether excluded.
    let actor_via = |a: &Actor| -> Option<(Option<String>, bool)> {
        let r = match target.kind {
            ObjectType::Label | ObjectType::LabelGroup => {
                a.label.as_ref().or(a.label_group.as_ref())
            }
            ObjectType::IpList => a.ip_list.as_ref(),
            ObjectType::Service => None,
        }?;
        let via = reach.get(canon(&r.href).as_ref()).cloned()?;
        Some((via, a.exclusion == Some(true)))
    };
    // The best match on one side: named directly beats through a group.
    let side_via = |actors: &[Actor]| -> Option<(Option<String>, bool)> {
        let mut found: Option<(Option<String>, bool)> = None;
        for v in actors.iter().filter_map(actor_via) {
            if v.0.is_none() {
                return Some(v);
            }
            found.get_or_insert(v);
        }
        found
    };
    let update = |u: &Option<String>| u.clone().filter(|s| !s.is_empty());
    let service_named = |services: &[IngressService]| {
        target.kind == ObjectType::Service
            && services.iter().any(|s| match s {
                IngressService::Ref(r) => canon(&r.href) == canon(&target.href),
                IngressService::Port { .. } => false,
            })
    };

    let mut scopes = Vec::new();
    let mut rules = Vec::new();
    let mut deny_unread = 0;
    for rs in inputs.rule_sets {
        let set_pending = update(&rs.update_type);
        let in_scope = side_via(&rs.scopes.iter().flatten().cloned().collect::<Vec<_>>());
        if let Some((via, excluded)) = in_scope {
            scopes.push(Reference {
                ruleset: Some(rs.name.clone()),
                ruleset_href: Some(rs.href.clone()),
                pending: set_pending.clone(),
                excluded,
                disabled: !rs.enabled,
                via,
                ..Reference::blank(RefKind::Scope, &rs.href)
            });
        }
        let deny: &[DenyRule] = if rs.deny_rules.is_empty() {
            match inputs.fallback_deny.get(&rs.href) {
                Some(d) => d,
                None => {
                    deny_unread += usize::from(inputs.deny_apart);
                    &[]
                }
            }
        } else {
            &rs.deny_rules
        };
        // One entry per rule, numbered in the Policy screen's order.
        struct One<'r> {
            kind: RuleKind,
            unscoped: bool,
            href: &'r str,
            enabled: bool,
            pending: Option<String>,
            providers: &'r [Actor],
            consumers: &'r [Actor],
            services: &'r [IngressService],
        }
        let mut all: Vec<One> = rs
            .rules
            .iter()
            .map(|r| One {
                kind: RuleKind::Allow,
                unscoped: r.unscoped_consumers.unwrap_or(false),
                href: &r.href,
                enabled: r.enabled,
                pending: update(&r.update_type),
                providers: &r.providers,
                consumers: &r.consumers,
                services: &r.ingress_services,
            })
            .chain(deny.iter().map(|d| One {
                kind: if d.override_ {
                    RuleKind::OverrideDeny
                } else {
                    RuleKind::Deny
                },
                unscoped: d.unscoped_consumers.unwrap_or(false),
                href: &d.href,
                enabled: d.enabled,
                pending: update(&d.update_type),
                providers: &d.providers,
                consumers: &d.consumers,
                services: &d.ingress_services,
            }))
            .collect();
        all.sort_by_key(|r| pane_order(r.kind, r.unscoped));
        for (i, r) in all.iter().enumerate() {
            let row = |side: RefSide, (via, excluded): (Option<String>, bool)| Reference {
                ruleset: Some(rs.name.clone()),
                ruleset_href: Some(rs.href.clone()),
                rule: Some(i + 1),
                side: Some(side),
                action: Some(r.kind),
                pending: r.pending.clone().or_else(|| set_pending.clone()),
                excluded,
                disabled: !r.enabled || !rs.enabled,
                via,
                ..Reference::blank(RefKind::Rule, r.href)
            };
            if let Some(hit) = side_via(r.providers) {
                rules.push(row(RefSide::Provider, hit));
            }
            if let Some(hit) = side_via(r.consumers) {
                rules.push(row(RefSide::Consumer, hit));
            }
            if service_named(r.services) {
                rules.push(row(RefSide::Service, (None, false)));
            }
        }
    }
    let groups = held.into_iter().map(|(g, via)| Reference {
        name: Some(g.name.clone()),
        pending: update(&g.update_type),
        via: via.map(|v| v.name.clone()),
        ..Reference::blank(RefKind::LabelGroup, &g.href)
    });
    let mut refs = scopes;
    refs.extend(rules);
    refs.extend(groups);
    WhereUsed { refs, deny_unread }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rule_sets() -> Vec<RuleSet> {
        serde_json::from_str(include_str!("../../../fixtures/rule_sets.json")).unwrap()
    }

    fn groups() -> Vec<LabelGroup> {
        serde_json::from_str(include_str!("../../../fixtures/label_groups.json")).unwrap()
    }

    fn workloads() -> Vec<Workload> {
        serde_json::from_str(include_str!("../../../fixtures/workloads.json")).unwrap()
    }

    fn group(href: u32, name: &str, labels: &[u32], subs: &[u32]) -> LabelGroup {
        serde_json::from_value(json!({
            "href": format!("/orgs/5/sec_policy/draft/label_groups/{href}"),
            "name": name, "key": "role",
            "labels": labels.iter().map(|l| json!({"href": format!("/orgs/5/labels/{l}")})).collect::<Vec<_>>(),
            "sub_groups": subs.iter().map(|g| json!({"href": format!("/orgs/5/sec_policy/draft/label_groups/{g}")})).collect::<Vec<_>>(),
        }))
        .unwrap()
    }

    fn search(target: Target, sets: &[RuleSet], groups: &[LabelGroup]) -> WhereUsed {
        let none = HashMap::new();
        where_used(
            &target,
            &Inputs {
                rule_sets: sets,
                fallback_deny: &none,
                groups,
                deny_apart: false,
            },
        )
    }

    fn label(n: u32) -> Target {
        Target::new(ObjectType::Label, format!("/orgs/5/labels/{n}"))
    }

    fn lines(w: &WhereUsed) -> Vec<String> {
        w.refs
            .iter()
            .map(|r| format!("{} {}", r.kind.as_str(), r.line()))
            .collect()
    }

    #[test]
    fn a_label_in_scopes_rules_and_groups() {
        // env=prod: Shop-Prod's scope, the group `All prod envs`, and the
        // disabled, pending Global-Infra rule naming that group.
        let w = search(label(4), &rule_sets(), &groups());
        assert_eq!(
            lines(&w),
            [
                "scope Shop-Prod",
                "rule Global-Infra · #1 · consumer · allow · pending create disabled · via All prod envs",
                "label_group All prod envs",
            ]
        );
        assert_eq!(w.count(RefKind::Rule), 1);
        assert_eq!(w.refs[0].href, "/orgs/5/sec_policy/draft/rule_sets/100");
        assert_eq!(
            w.refs[1].href,
            "/orgs/5/sec_policy/draft/rule_sets/101/sec_rules/3"
        );
    }

    #[test]
    fn rules_are_numbered_as_the_policy_screen_lists_them_with_deny_kinds_marked() {
        // role=db (labels/2): provider of allow rule 1 and of the override
        // deny 8, which the Policy screen lists first.
        let w = search(label(2), &rule_sets(), &groups());
        assert_eq!(
            lines(&w),
            [
                "rule Shop-Prod · #1 · provider · override deny",
                "rule Shop-Prod · #2 · provider · allow",
            ]
        );
        assert_eq!(w.refs[0].action, Some(RuleKind::OverrideDeny));
        // role=web: consumer of rule 1, provider of the extra-scope rule 2
        // (pending update).
        let w = search(label(1), &rule_sets(), &groups());
        assert_eq!(
            lines(&w),
            [
                "rule Shop-Prod · #2 · consumer · allow",
                "rule Shop-Prod · #3 · provider · allow · pending update",
            ]
        );
    }

    #[test]
    fn an_ip_list_and_a_service() {
        let ipl = Target::new(ObjectType::IpList, "/orgs/5/sec_policy/draft/ip_lists/20");
        assert_eq!(
            lines(&search(ipl, &rule_sets(), &groups())),
            ["rule Shop-Prod · #4 · consumer · deny"]
        );
        // An active href is the same object.
        let corp = Target::new(ObjectType::IpList, "/orgs/5/sec_policy/active/ip_lists/21");
        assert_eq!(
            lines(&search(corp, &rule_sets(), &groups())),
            ["rule Shop-Prod · #3 · consumer · allow · pending update"]
        );
        let svc = Target::new(ObjectType::Service, "/orgs/5/sec_policy/draft/services/31");
        let w = search(svc, &rule_sets(), &groups());
        assert_eq!(lines(&w), ["rule Shop-Prod · #2 · service · allow"]);
        assert_eq!(w.refs[0].side, Some(RefSide::Service));
        // Inline ports are no service object.
        let web = Target::new(ObjectType::Service, "/orgs/5/sec_policy/draft/services/32");
        assert!(search(web, &rule_sets(), &groups()).refs.is_empty());
    }

    #[test]
    fn disabled_marks_the_rule_or_its_ruleset() {
        let mut sets = rule_sets();
        sets[0].rules[0].enabled = false;
        let w = search(label(2), &sets, &groups());
        assert!(!w.refs[0].disabled, "the override deny is on");
        assert!(w.refs[1].disabled, "rule 1 is off");
        assert_eq!(
            w.refs[1].line(),
            "Shop-Prod · #2 · provider · allow · disabled"
        );
        let svc = Target::new(ObjectType::Service, "/orgs/5/sec_policy/draft/services/30");
        let w = search(svc, &rule_sets(), &groups());
        assert!(w.refs[0].disabled, "Global-Infra is disabled");
    }

    #[test]
    fn pending_names_the_draft_change() {
        let svc = Target::new(ObjectType::Service, "/orgs/5/sec_policy/draft/services/30");
        let w = search(svc, &rule_sets(), &groups());
        assert_eq!(
            w.refs[0].pending.as_deref(),
            Some("create"),
            "rule and ruleset are pending creation"
        );
        // Provisioned (no update_type anywhere): not pending.
        let mut sets = rule_sets();
        sets[1].update_type = None;
        sets[1].rules[0].update_type = None;
        let svc = Target::new(ObjectType::Service, "/orgs/5/sec_policy/draft/services/30");
        assert_eq!(search(svc, &sets, &groups()).refs[0].pending, None);
        // A pending ruleset makes its scope row pending too.
        let mut sets = rule_sets();
        sets[0].update_type = Some("update".into());
        let w = search(label(3), &sets, &groups());
        assert_eq!(w.refs[0].line(), "Shop-Prod · pending update");
        // Pending deletion says so: not to be read as new.
        let mut sets = rule_sets();
        sets[0].rules[0].update_type = Some("delete".into());
        let svc = Target::new(ObjectType::Service, "/orgs/5/sec_policy/draft/services/31");
        let w = search(svc, &sets, &groups());
        assert_eq!(
            w.refs[0].line(),
            "Shop-Prod · #2 · service · allow · pending delete"
        );
    }

    #[test]
    fn nested_groups_are_followed_with_via_and_a_cycle_ends() {
        // 12 holds label 1 and group 13; 13 holds 14; 14 holds label 7 and
        // group 12 (a cycle); 15 holds 14.
        let gs = vec![
            group(12, "Web tier", &[1], &[13]),
            group(13, "Mid tier", &[], &[14]),
            group(14, "Leaf", &[7], &[12]),
            group(15, "Other", &[], &[14]),
        ];
        let w = search(label(7), &[], &gs);
        assert_eq!(
            lines(&w),
            [
                "label_group Leaf",
                "label_group Mid tier · via Leaf",
                "label_group Other · via Leaf",
                "label_group Web tier · via Mid tier",
            ]
        );
        // Searching a group finds the groups holding it, never itself.
        let t = Target::new(
            ObjectType::LabelGroup,
            "/orgs/5/sec_policy/draft/label_groups/12",
        );
        assert_eq!(
            lines(&search(t, &[], &gs)),
            [
                "label_group Leaf",
                "label_group Mid tier · via Leaf",
                "label_group Other · via Leaf",
            ]
        );
    }

    #[test]
    fn a_label_group_in_rules_and_in_the_groups_holding_it() {
        let mut gs = groups();
        gs.push(group(11, "Everything", &[], &[10]));
        let t = Target::new(
            ObjectType::LabelGroup,
            "/orgs/5/sec_policy/active/label_groups/10",
        );
        let w = search(t, &rule_sets(), &gs);
        assert_eq!(
            lines(&w),
            [
                "rule Global-Infra · #1 · consumer · allow · pending create disabled",
                "label_group Everything",
            ]
        );
        assert_eq!(w.refs[0].via, None, "named directly");
    }

    #[test]
    fn named_directly_beats_through_a_group_on_the_same_side() {
        let mut sets = rule_sets();
        sets[1].rules[0]
            .consumers
            .push(serde_json::from_value(json!({"label": {"href": "/orgs/5/labels/4"}})).unwrap());
        let w = search(label(4), &sets, &groups());
        assert_eq!(w.refs[1].via, None);
    }

    #[test]
    fn an_excluded_actor_is_marked() {
        let mut sets = rule_sets();
        sets[0].rules[0].providers[0].exclusion = Some(true);
        let w = search(label(2), &sets, &groups());
        assert!(w.refs[1].excluded);
        assert_eq!(
            w.refs[1].line(),
            "Shop-Prod · #2 · provider · allow · excluded"
        );
        assert!(!w.refs[0].excluded);
    }

    #[test]
    fn deny_rules_read_apart_are_searched_and_unread_ones_counted() {
        let mut sets = rule_sets();
        sets[0].deny_rules.clear();
        let deny: Vec<DenyRule> =
            serde_json::from_str(include_str!("../../../fixtures/deny_rules.json")).unwrap();
        let fallback = HashMap::from([(sets[0].href.clone(), deny)]);
        let ipl = Target::new(ObjectType::IpList, "/orgs/5/sec_policy/draft/ip_lists/20");
        let w = where_used(
            &ipl,
            &Inputs {
                rule_sets: &sets,
                fallback_deny: &fallback,
                groups: &[],
                deny_apart: true,
            },
        );
        assert_eq!(w.deny_unread, 1, "Global-Infra's were not read");
        assert!(w.refs.iter().any(|r| r.action == Some(RuleKind::Deny)));
        let none = HashMap::new();
        let w = where_used(
            &ipl,
            &Inputs {
                rule_sets: &sets,
                fallback_deny: &none,
                groups: &[],
                deny_apart: false,
            },
        );
        assert_eq!(w.deny_unread, 0, "inline lists: an empty one means none");
    }

    fn profiles() -> Vec<PairingProfile> {
        serde_json::from_str(include_str!("../../../fixtures/pairing_profiles.json")).unwrap()
    }

    #[test]
    fn a_label_in_a_pairing_profile_with_its_lock() {
        // shop-prod gives role=web and app=shop, both locked, and is off.
        let rows = pairing_profiles("/orgs/5/labels/1", "role", &profiles());
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.kind, RefKind::PairingProfile);
        assert_eq!(r.name.as_deref(), Some("shop-prod"));
        assert_eq!(r.href, "/orgs/5/pairing_profiles/7");
        assert_eq!(r.locked, Some(true));
        assert!(r.disabled);
        assert_eq!(r.line(), "shop-prod · locked · disabled");
        assert_eq!(
            (
                r.ruleset.as_ref(),
                r.rule,
                r.side,
                r.action,
                r.count,
                r.exact
            ),
            (None, None, None, None, None, None)
        );
        // Not in any profile: no row.
        assert!(pairing_profiles("/orgs/5/labels/4", "env", &profiles()).is_empty());
    }

    #[test]
    fn locked_is_the_keys_lock_true_false_or_unknown() {
        let p = |extra: serde_json::Value| -> PairingProfile {
            let mut v = json!({
                "href": "/orgs/5/pairing_profiles/3", "name": "eu",
                "enabled": true,
                "labels": [{"href": "/orgs/5/labels/5"}],
            });
            v.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            serde_json::from_value(v).unwrap()
        };
        let locked = |key: &str, extra: serde_json::Value| {
            pairing_profiles("/orgs/5/labels/5", key, &[p(extra)])[0].locked
        };
        assert_eq!(locked("loc", json!({"loc_label_lock": true})), Some(true));
        assert_eq!(locked("loc", json!({"loc_label_lock": false})), Some(false));
        assert_eq!(locked("loc", json!({})), None, "the profile does not say");
        // A key with no lock field: never locked or not.
        assert_eq!(locked("bu", json!({"loc_label_lock": true})), None);
        let r = &pairing_profiles(
            "/orgs/5/labels/5",
            "loc",
            &[p(json!({"loc_label_lock": false}))],
        )[0];
        assert!(!r.disabled);
        assert_eq!(r.line(), "eu");
        assert_eq!(label_lock_field("env"), Some("env_label_lock"));
        assert_eq!(label_lock_field("enforcement"), None);
    }

    #[test]
    fn pairing_profiles_follow_label_groups_in_the_section_order() {
        let order = |k: RefKind| RefKind::ALL.iter().position(|x| *x == k).unwrap();
        assert!(order(RefKind::LabelGroup) < order(RefKind::PairingProfile));
        assert!(order(RefKind::PairingProfile) < order(RefKind::Workloads));
        assert_eq!(RefKind::PairingProfile.as_str(), "pairing_profile");
        assert!(!NOT_SEARCHED.contains("pairing"));
    }

    #[test]
    fn workload_count_from_the_loaded_list_or_a_probe() {
        let w = workloads();
        let n = WorkloadCount::from_loaded(&w, false, "/orgs/5/labels/4");
        assert_eq!((n.count, n.exact, n.text()), (2, true, "2".to_string()));
        let n = WorkloadCount::from_loaded(&w, true, "/orgs/5/labels/5");
        assert_eq!((n.count, n.text()), (0, "≥0".to_string()));
        assert_eq!(
            WorkloadCount::from_probe(0),
            WorkloadCount {
                count: 0,
                exact: true
            }
        );
        let one = WorkloadCount::from_probe(1);
        assert_eq!((one.text(), one.exact), ("≥1".to_string(), false));
        let r = Reference::workloads("/orgs/5/labels/4", one);
        assert_eq!(r.line(), "≥1 workloads");
        let r = Reference::workloads(
            "/orgs/5/labels/4",
            WorkloadCount {
                count: 1,
                exact: true,
            },
        );
        assert_eq!(r.line(), "1 workload");
    }
}
