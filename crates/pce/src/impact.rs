//! Rule impact: how many workloads a rule's providers and consumers match,
//! evaluated with its ruleset's scopes over a workload list. Pure: the
//! caller brings the workloads (one cached read) and the name index; IP
//! lists are reported with their size, never expanded into peers.
use crate::iplist_size::{IpListSize, ip_list_size};
use crate::models::{Actor, DenyRule, Rule, Workload};
use crate::resolve::{NameIndex, canon, describe_actor};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

/// What a rule is evaluated over. Label groups and IP lists are looked up
/// in `index`, which is built from the same lists.
#[derive(Debug, Clone, Copy)]
pub struct ImpactInputs<'a> {
    pub workloads: &'a [Workload],
    pub index: &'a NameIndex,
    /// The workload read was cut short (`Cached::truncated`).
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Provider,
    Consumer,
}

/// An IP-list actor of the rule, with its size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IpListRef {
    /// The list's name, or its href when the index does not know it.
    pub name: String,
    pub href: String,
    /// `None` when the index does not know the list.
    pub size: Option<IpListSize>,
    pub side: Side,
}

impl IpListRef {
    /// `Corp: 65.5K addresses`, `Any: any (0.0.0.0/0)`, `DNS: 3 FQDNs`,
    /// `<href>: ? addresses`.
    pub fn line(&self) -> String {
        match &self.size {
            None => format!("{}: ? addresses", self.name),
            Some(s) if s.unbounded.is_some() || (s.ranges == 0 && s.fqdns > 0) => {
                format!("{}: {}", self.name, s.display())
            }
            Some(s) => format!("{}: {} addresses", self.name, s.display()),
        }
    }
}

/// One scope's share of the impact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScopeImpact {
    /// The scope's members by name (`app=shop`, `group:Prod`); empty: everywhere.
    pub scope: Vec<String>,
    pub providers: usize,
    pub consumers: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Impact {
    /// Workloads matched as providers in any scope.
    pub providers: usize,
    /// Workloads matched as consumers in any scope (anywhere when unscoped).
    pub consumers: usize,
    pub per_scope: Vec<ScopeImpact>,
    pub ip_lists: Vec<IpListRef>,
    /// Virtual service and virtual server hrefs: not evaluated, not counted.
    pub unsupported: Vec<String>,
    /// The workload list was truncated: the counts are lower bounds.
    pub truncated: bool,
}

fn counted(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

impl Impact {
    /// `12/3`; `≥12/≥3` when the workload list was truncated.
    pub fn pc(&self) -> String {
        let ge = if self.truncated { "≥" } else { "" };
        format!("{ge}{}/{ge}{}", self.providers, self.consumers)
    }

    /// `12 providers · 3 consumers`; with a truncated list
    /// `≥12 providers · ≥3 consumers (workload list truncated)`.
    pub fn summary(&self) -> String {
        let ge = if self.truncated { "≥" } else { "" };
        let mut s = format!(
            "{ge}{} · {ge}{}",
            counted(self.providers, "provider"),
            counted(self.consumers, "consumer")
        );
        if self.truncated {
            s.push_str(" (workload list truncated)");
        }
        s
    }
}

/// The sides of a rule the evaluator needs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuleActors {
    pub providers: Vec<Actor>,
    pub consumers: Vec<Actor>,
    pub unscoped_consumers: bool,
}

impl RuleActors {
    pub fn of_rule(r: &Rule) -> Self {
        Self {
            providers: r.providers.clone(),
            consumers: r.consumers.clone(),
            unscoped_consumers: r.unscoped_consumers.unwrap_or(false),
        }
    }

    /// A deny rule's consumers are always scoped.
    pub fn of_deny(d: &DenyRule) -> Self {
        Self {
            providers: d.providers.clone(),
            consumers: d.consumers.clone(),
            unscoped_consumers: false,
        }
    }

    /// What a planned rule write leaves: the body's `providers`,
    /// `consumers` and `unscoped_consumers` where it has them, `base`'s
    /// (the loaded rule, for an update) otherwise.
    pub fn planned(body: Option<&Value>, base: Option<&RuleActors>) -> Self {
        let base = base.cloned().unwrap_or_default();
        let side = |key: &str, old: Vec<Actor>| {
            body.and_then(|b| b.get(key))
                .and_then(|v| serde_json::from_value::<Vec<Actor>>(v.clone()).ok())
                .unwrap_or(old)
        };
        Self {
            providers: side("providers", base.providers),
            consumers: side("consumers", base.consumers),
            unscoped_consumers: body
                .and_then(|b| b.get("unscoped_consumers"))
                .and_then(Value::as_bool)
                .unwrap_or(base.unscoped_consumers),
        }
    }
}

/// `/orgs/5/sec_policy/draft/rule_sets/100` for a rule write's path
/// (`…/rule_sets/100/sec_rules` to create, `…/sec_rules/1` to update).
pub fn rule_set_href_of(path: &str) -> Option<&str> {
    const SEGMENT: &str = "/rule_sets/";
    let start = path.find(SEGMENT)? + SEGMENT.len();
    let id = path[start..].find('/').unwrap_or(path.len() - start);
    (id > 0).then(|| &path[..start + id])
}

/// Whether a rule write path names one rule (an update) rather than a
/// ruleset's rule collection (a create).
pub fn is_rule_href(path: &str) -> bool {
    let mut parts = path.trim_end_matches('/').rsplit('/');
    let last = parts.next().unwrap_or("");
    !last.is_empty() && matches!(parts.next(), Some("sec_rules" | "deny_rules"))
}

/// The label hrefs a group stands for: its labels and its sub-groups',
/// recursively. Each group is visited once, so a cycle ends; an unknown
/// group adds nothing.
fn group_members(href: &str, idx: &NameIndex) -> HashSet<String> {
    let mut members = HashSet::new();
    let mut seen = HashSet::new();
    let mut todo = vec![canon(href).into_owned()];
    while let Some(h) = todo.pop() {
        if !seen.insert(h.clone()) {
            continue;
        }
        if let Some(g) = idx.label_group(&h) {
            members.extend(g.labels.iter().map(|r| r.href.clone()));
            todo.extend(g.sub_groups.iter().map(|r| canon(&r.href).into_owned()));
        }
    }
    members
}

/// One side's actors, ready to test workloads against.
#[derive(Default)]
struct Matcher {
    all: bool,
    workloads: HashSet<String>,
    /// Label key to the label hrefs that satisfy it: AND across keys, OR
    /// within one. A group counts under its own `key`.
    by_key: BTreeMap<String, HashSet<String>>,
    /// Labels whose carriers are removed at the end.
    excluded: HashSet<String>,
}

impl Matcher {
    fn new(actors: &[Actor], idx: &NameIndex) -> Self {
        let mut m = Self::default();
        for a in actors {
            let excluded = a.exclusion == Some(true);
            if a.actors.as_deref() == Some("ams") {
                m.all |= !excluded;
            } else if let Some(l) = &a.label {
                if excluded {
                    m.excluded.insert(l.href.clone());
                } else {
                    let key = idx
                        .label(&l.href)
                        .map_or_else(|| format!("?{}", l.href), |x| x.key.clone());
                    m.by_key.entry(key).or_default().insert(l.href.clone());
                }
            } else if let Some(g) = &a.label_group {
                let members = group_members(&g.href, idx);
                if excluded {
                    m.excluded.extend(members);
                } else {
                    let key = idx
                        .label_group(&g.href)
                        .map(|x| x.key.clone())
                        .filter(|k| !k.is_empty())
                        .unwrap_or_else(|| format!("?{}", g.href));
                    m.by_key.entry(key).or_default().extend(members);
                }
            } else if let Some(w) = &a.workload
                && !excluded
            {
                m.workloads.insert(canon(&w.href).into_owned());
            }
            // IP lists, virtual services and virtual servers match no workload.
        }
        m
    }

    fn matches(&self, href: &str, labels: &HashSet<&str>) -> bool {
        if self.excluded.iter().any(|h| labels.contains(h.as_str())) {
            return false;
        }
        self.all
            || self.workloads.contains(canon(href).as_ref())
            || (!self.by_key.is_empty()
                && self
                    .by_key
                    .values()
                    .all(|hrefs| hrefs.iter().any(|h| labels.contains(h.as_str()))))
    }
}

/// One scope: per member the label hrefs that satisfy it (`None` for
/// `all`) and whether it is an exclusion. Every member must hold.
struct Scope(Vec<(Option<HashSet<String>>, bool)>);

impl Scope {
    fn new(members: &[Actor], idx: &NameIndex) -> Self {
        Self(
            members
                .iter()
                .map(|a| {
                    let hrefs = if a.actors.as_deref() == Some("ams") {
                        None
                    } else if let Some(l) = &a.label {
                        Some(HashSet::from([l.href.clone()]))
                    } else if let Some(g) = &a.label_group {
                        Some(group_members(&g.href, idx))
                    } else {
                        // Any other kind in a scope matches nothing.
                        Some(HashSet::new())
                    };
                    (hrefs, a.exclusion == Some(true))
                })
                .collect(),
        )
    }

    fn contains(&self, labels: &HashSet<&str>) -> bool {
        self.0.iter().all(|(hrefs, excluded)| {
            let hit = hrefs
                .as_ref()
                .is_none_or(|s| s.iter().any(|h| labels.contains(h.as_str())));
            hit != *excluded
        })
    }
}

/// Per scope `Si`: providers `{w in Si matching providers}`, consumers
/// `{w matching consumers}` kept to `Si` unless `unscoped_consumers`; the
/// totals are the unions over the scopes. A ruleset without scopes is one
/// empty scope (everywhere).
pub fn rule_impact(
    scopes: &[Vec<Actor>],
    providers: &[Actor],
    consumers: &[Actor],
    unscoped_consumers: bool,
    inputs: &ImpactInputs,
) -> Impact {
    let labels = workload_labels(inputs.workloads);
    rule_impact_with(
        scopes,
        providers,
        consumers,
        unscoped_consumers,
        inputs,
        &labels,
    )
}

/// Each workload's label hrefs, in list order. Built once and shared by
/// `rule_impact_with` when many rules are evaluated over one workload list.
pub fn workload_labels(workloads: &[Workload]) -> Vec<HashSet<&str>> {
    workloads
        .iter()
        .map(|w| w.labels.iter().map(|l| l.href.as_str()).collect())
        .collect()
}

/// `rule_impact` with the workloads' label sets (`workload_labels` of
/// `inputs.workloads`) brought by the caller.
pub fn rule_impact_with(
    scopes: &[Vec<Actor>],
    providers: &[Actor],
    consumers: &[Actor],
    unscoped_consumers: bool,
    inputs: &ImpactInputs,
    labels: &[HashSet<&str>],
) -> Impact {
    debug_assert_eq!(labels.len(), inputs.workloads.len());
    let idx = inputs.index;
    let everywhere = [Vec::new()];
    let scopes = if scopes.is_empty() {
        &everywhere[..]
    } else {
        scopes
    };
    let p = Matcher::new(providers, idx);
    let c = Matcher::new(consumers, idx);
    let is_p: Vec<bool> = inputs
        .workloads
        .iter()
        .zip(labels)
        .map(|(w, l)| p.matches(&w.href, l))
        .collect();
    let is_c: Vec<bool> = inputs
        .workloads
        .iter()
        .zip(labels)
        .map(|(w, l)| c.matches(&w.href, l))
        .collect();
    let mut all_p = HashSet::new();
    let mut all_c = HashSet::new();
    let mut per_scope = Vec::with_capacity(scopes.len());
    for members in scopes {
        let scope = Scope::new(members, idx);
        let (mut sp, mut sc) = (0, 0);
        for (i, l) in labels.iter().enumerate() {
            let inside = scope.contains(l);
            if inside && is_p[i] {
                all_p.insert(i);
                sp += 1;
            }
            if is_c[i] && (unscoped_consumers || inside) {
                all_c.insert(i);
                sc += 1;
            }
        }
        per_scope.push(ScopeImpact {
            scope: members.iter().map(|a| describe_actor(a, idx)).collect(),
            providers: sp,
            consumers: sc,
        });
    }
    let mut ip_lists = Vec::new();
    let mut unsupported = Vec::new();
    for (side, actors) in [(Side::Provider, providers), (Side::Consumer, consumers)] {
        for a in actors {
            if let Some(r) = &a.ip_list {
                let list = idx.ip_list(&r.href);
                ip_lists.push(IpListRef {
                    name: list.map_or_else(|| r.href.clone(), |l| l.name.clone()),
                    href: r.href.clone(),
                    size: list.map(ip_list_size),
                    side,
                });
            }
            for v in [&a.virtual_service, &a.virtual_server]
                .into_iter()
                .flatten()
            {
                unsupported.push(v.href.clone());
            }
        }
    }
    Impact {
        providers: all_p.len(),
        consumers: all_c.len(),
        per_scope,
        ip_lists,
        unsupported,
        truncated: inputs.truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{IpList, Label, LabelGroup, RuleSet};
    use serde_json::{Value, json};

    /// The fixture index plus two role groups that contain each other: a
    /// sub-group, a cycle, and an active href among the sub-groups.
    fn index() -> NameIndex {
        let labels: Vec<Label> =
            serde_json::from_str(include_str!("../../../fixtures/labels.json")).unwrap();
        let mut groups: Vec<LabelGroup> =
            serde_json::from_str(include_str!("../../../fixtures/label_groups.json")).unwrap();
        groups.push(
            serde_json::from_value(json!({
                "href": "/orgs/5/sec_policy/draft/label_groups/12", "name": "Web tier", "key": "role",
                "labels": [{"href": "/orgs/5/labels/1"}],
                "sub_groups": [{"href": "/orgs/5/sec_policy/draft/label_groups/13"}]
            }))
            .unwrap(),
        );
        groups.push(
            serde_json::from_value(json!({
                "href": "/orgs/5/sec_policy/draft/label_groups/13", "name": "Db tier", "key": "role",
                "labels": [{"href": "/orgs/5/labels/2"}],
                "sub_groups": [{"href": "/orgs/5/sec_policy/active/label_groups/12"}]
            }))
            .unwrap(),
        );
        let lists: Vec<IpList> =
            serde_json::from_str(include_str!("../../../fixtures/ip_lists.json")).unwrap();
        NameIndex::new(&labels, &groups, &lists, &[])
    }

    fn workloads() -> Vec<Workload> {
        serde_json::from_str(include_str!("../../../fixtures/workloads.json")).unwrap()
    }

    fn rule_sets() -> Vec<RuleSet> {
        serde_json::from_str(include_str!("../../../fixtures/rule_sets.json")).unwrap()
    }

    fn actors(v: Value) -> Vec<Actor> {
        serde_json::from_value(v).unwrap()
    }

    fn label(n: u32) -> Value {
        json!({"label": {"href": format!("/orgs/5/labels/{n}")}})
    }

    fn all() -> Value {
        json!([{"actors": "ams"}])
    }

    /// Shop-Prod's one scope: app=shop & env=prod.
    fn shop() -> Vec<Vec<Actor>> {
        vec![actors(json!([label(3), label(4)]))]
    }

    fn run(scopes: &[Vec<Actor>], providers: Value, consumers: Value, unscoped: bool) -> Impact {
        let idx = index();
        let w = workloads();
        let inputs = ImpactInputs {
            workloads: &w,
            index: &idx,
            truncated: false,
        };
        rule_impact(
            scopes,
            &actors(providers),
            &actors(consumers),
            unscoped,
            &inputs,
        )
    }

    const EVERYWHERE: &[Vec<Actor>] = &[];

    #[test]
    fn the_fixture_rules_count_their_scoped_sides() {
        let sets = rule_sets();
        let idx = index();
        let w = workloads();
        let inputs = ImpactInputs {
            workloads: &w,
            index: &idx,
            truncated: false,
        };
        let at = |a: RuleActors, scopes: &[Vec<Actor>]| {
            let i = rule_impact(
                scopes,
                &a.providers,
                &a.consumers,
                a.unscoped_consumers,
                &inputs,
            );
            (i.providers, i.consumers)
        };
        let shop = &sets[0];
        assert_eq!(
            at(RuleActors::of_rule(&shop.rules[0]), &shop.scopes),
            (1, 1),
            "db ← web"
        );
        assert_eq!(
            at(RuleActors::of_rule(&shop.rules[1]), &shop.scopes),
            (1, 0),
            "an IP list consumer matches no workload"
        );
        assert_eq!(
            at(RuleActors::of_deny(&shop.deny_rules[0]), &shop.scopes),
            (2, 0)
        );
        assert_eq!(
            at(RuleActors::of_deny(&shop.deny_rules[1]), &shop.scopes),
            (1, 2),
            "a deny rule's consumers stay inside the scope"
        );
        let infra = &sets[1];
        assert_eq!(
            at(RuleActors::of_rule(&infra.rules[0]), &infra.scopes),
            (4, 2),
            "an empty scope is everywhere; the group stands for env=prod"
        );
        assert_eq!(
            at(RuleActors::of_rule(&infra.rules[0]), EVERYWHERE),
            (4, 2),
            "no scopes at all is one empty scope"
        );
    }

    #[test]
    fn per_scope_counts_each_scope_and_the_totals_are_unions() {
        let scopes = vec![shop()[0].clone(), actors(json!([label(2)]))];
        let i = run(&scopes, all(), json!([label(1)]), false);
        assert_eq!((i.providers, i.consumers), (2, 1));
        assert_eq!(
            i.per_scope,
            vec![
                ScopeImpact {
                    scope: vec!["app=shop".into(), "env=prod".into()],
                    providers: 2,
                    consumers: 1
                },
                ScopeImpact {
                    scope: vec!["role=db".into()],
                    providers: 1,
                    consumers: 0
                },
            ]
        );
        let unscoped = run(&shop(), json!([label(1)]), all(), true);
        assert_eq!((unscoped.providers, unscoped.consumers), (1, 4));
        let scoped = run(&shop(), json!([label(1)]), all(), false);
        assert_eq!(scoped.consumers, 2);
        assert_eq!(
            run(EVERYWHERE, all(), all(), false).per_scope[0].scope,
            Vec::<String>::new()
        );
    }

    #[test]
    fn labels_and_across_keys_or_within_and_exclusions_apply_last() {
        // (role=web OR role=db) AND app=shop
        assert_eq!(
            run(
                EVERYWHERE,
                json!([label(1), label(2), label(3)]),
                all(),
                false
            )
            .providers,
            2
        );
        // role=web AND env=prod
        assert_eq!(
            run(EVERYWHERE, json!([label(1), label(4)]), all(), false).providers,
            1
        );
        // all minus env=prod, as a label and as a group
        let minus_label =
            json!([{"actors": "ams"}, {"label": {"href": "/orgs/5/labels/4"}, "exclusion": true}]);
        assert_eq!(run(EVERYWHERE, minus_label, all(), false).providers, 2);
        let minus_group = json!([
            {"actors": "ams"},
            {"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/10"}, "exclusion": true}
        ]);
        assert_eq!(run(EVERYWHERE, minus_group, all(), false).providers, 2);
        // Only an exclusion: nothing positive, nothing matches.
        let only = json!([{"label": {"href": "/orgs/5/labels/4"}, "exclusion": true}]);
        assert_eq!(run(EVERYWHERE, only, all(), false).providers, 0);
        // A label the index does not know is a key of its own: nobody carries it.
        assert_eq!(
            run(EVERYWHERE, json!([label(99)]), all(), false).providers,
            0
        );
        // A scope exclusion keeps out what carries it.
        let scope = vec![actors(
            json!([label(3), {"label": {"href": "/orgs/5/labels/1"}, "exclusion": true}]),
        )];
        assert_eq!(
            run(&scope, all(), all(), false).providers,
            1,
            "app=shop but not role=web"
        );
    }

    #[test]
    fn label_groups_expand_sub_groups_once_even_in_a_cycle() {
        let web_tier =
            json!([{"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/12"}}]);
        assert_eq!(
            run(EVERYWHERE, web_tier, all(), false).providers,
            2,
            "web, plus db through the sub-group"
        );
        // An active href folds to draft; the group's key is role, ANDed with app=shop.
        let mixed = json!([{"label_group": {"href": "/orgs/5/sec_policy/active/label_groups/13"}}, label(3)]);
        assert_eq!(run(EVERYWHERE, mixed, all(), false).providers, 2);
        // A scope member may be a group: env=prod through "All prod envs".
        let scope = vec![actors(
            json!([{"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/10"}}]),
        )];
        assert_eq!(run(&scope, all(), all(), false).providers, 2);
        // A group the index does not know matches nothing.
        let unknown =
            json!([{"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/77"}}]);
        assert_eq!(run(EVERYWHERE, unknown, all(), false).providers, 0);
    }

    #[test]
    fn workload_actors_ip_lists_and_virtual_services() {
        let i = run(
            EVERYWHERE,
            json!([{"workload": {"href": "/orgs/5/workloads/aaaa-3"}}]),
            json!([
                {"ip_list": {"href": "/orgs/5/sec_policy/active/ip_lists/21"}},
                {"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/99"}},
                {"virtual_service": {"href": "/orgs/5/sec_policy/draft/virtual_services/v1"}}
            ]),
            false,
        );
        assert_eq!(
            (i.providers, i.consumers),
            (1, 0),
            "the unmanaged workload by href; lists and services match nobody"
        );
        assert_eq!(i.ip_lists[0].name, "Corp");
        assert_eq!(i.ip_lists[0].side, Side::Consumer);
        assert_eq!(
            i.ip_lists[0]
                .size
                .as_ref()
                .map(IpListSize::display)
                .as_deref(),
            Some("65.5K")
        );
        assert_eq!(i.ip_lists[0].line(), "Corp: 65.5K addresses");
        assert_eq!(i.ip_lists[1].name, "/orgs/5/sec_policy/draft/ip_lists/99");
        assert_eq!(i.ip_lists[1].size, None);
        assert_eq!(
            i.ip_lists[1].line(),
            "/orgs/5/sec_policy/draft/ip_lists/99: ? addresses"
        );
        assert_eq!(
            i.unsupported,
            vec!["/orgs/5/sec_policy/draft/virtual_services/v1".to_string()]
        );
        let any = run(
            EVERYWHERE,
            json!([{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/20"}}]),
            all(),
            false,
        );
        assert_eq!(any.ip_lists[0].side, Side::Provider);
        assert_eq!(
            any.ip_lists[0].line(),
            "Any (0.0.0.0/0 and ::/0): any (0.0.0.0/0)"
        );
    }

    #[test]
    fn truncated_is_copied_and_counts_never_exceed_the_list() {
        let idx = index();
        let w = workloads();
        let inputs = ImpactInputs {
            workloads: &w,
            index: &idx,
            truncated: true,
        };
        let i = rule_impact(&[], &actors(all()), &actors(all()), false, &inputs);
        assert!(i.truncated);
        assert_eq!(i.pc(), "≥4/≥4");
        assert_eq!(
            i.summary(),
            "≥4 providers · ≥4 consumers (workload list truncated)"
        );
        // Whatever the rule, no side counts more workloads than there are,
        // and no scope more than the total.
        for rs in rule_sets() {
            let sides = rs
                .rules
                .iter()
                .map(RuleActors::of_rule)
                .chain(rs.deny_rules.iter().map(RuleActors::of_deny));
            for a in sides {
                let i = rule_impact(
                    &rs.scopes,
                    &a.providers,
                    &a.consumers,
                    a.unscoped_consumers,
                    &inputs,
                );
                assert!(i.providers <= w.len() && i.consumers <= w.len());
                for s in &i.per_scope {
                    assert!(s.providers <= i.providers && s.consumers <= i.consumers);
                }
            }
        }
    }

    #[test]
    fn shared_label_sets_give_the_same_impact_as_rule_impact() {
        let idx = index();
        let w = workloads();
        let inputs = ImpactInputs {
            workloads: &w,
            index: &idx,
            truncated: false,
        };
        let labels = workload_labels(&w);
        assert_eq!(labels.len(), w.len());
        for rs in rule_sets() {
            for a in rs.rules.iter().map(RuleActors::of_rule) {
                let args = (&rs.scopes[..], &a.providers[..], &a.consumers[..]);
                assert_eq!(
                    rule_impact(args.0, args.1, args.2, a.unscoped_consumers, &inputs),
                    rule_impact_with(
                        args.0,
                        args.1,
                        args.2,
                        a.unscoped_consumers,
                        &inputs,
                        &labels
                    )
                );
            }
        }
    }

    #[test]
    fn summary_and_pc_read_naturally() {
        let i = run(&shop(), json!([label(2)]), json!([label(1)]), false);
        assert_eq!(i.summary(), "1 provider · 1 consumer");
        assert_eq!(i.pc(), "1/1");
        let none = run(&shop(), json!([label(5)]), all(), false);
        assert_eq!(none.summary(), "0 providers · 2 consumers");
        assert_eq!(none.pc(), "0/2");
    }

    #[test]
    fn planned_actors_take_the_body_over_the_loaded_rule() {
        let sets = rule_sets();
        let base = RuleActors::of_rule(&sets[0].rules[1]); // web ← Corp, unscoped
        assert!(base.unscoped_consumers);
        let body = json!({"consumers": [
            {"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}},
            {"actors": "ams"}
        ]});
        let p = RuleActors::planned(Some(&body), Some(&base));
        assert_eq!(
            p.providers, base.providers,
            "an update without providers keeps them"
        );
        assert_eq!(p.consumers.len(), 2);
        assert!(p.unscoped_consumers);
        let scoped = RuleActors::planned(Some(&json!({"unscoped_consumers": false})), Some(&base));
        assert!(!scoped.unscoped_consumers);
        let created = RuleActors::planned(
            Some(
                &json!({"providers": [label(1)], "consumers": [label(2)], "unscoped_consumers": true}),
            ),
            None,
        );
        assert_eq!(
            (
                created.providers.len(),
                created.consumers.len(),
                created.unscoped_consumers
            ),
            (1, 1, true)
        );
        assert_eq!(RuleActors::planned(None, None), RuleActors::default());
        assert!(!RuleActors::of_deny(&sets[0].deny_rules[0]).unscoped_consumers);
    }

    #[test]
    fn rule_write_paths_name_their_ruleset() {
        let rs = "/orgs/5/sec_policy/draft/rule_sets/100";
        assert_eq!(rule_set_href_of(&format!("{rs}/sec_rules")), Some(rs));
        assert_eq!(rule_set_href_of(&format!("{rs}/deny_rules/8")), Some(rs));
        assert_eq!(rule_set_href_of(rs), Some(rs));
        assert_eq!(
            rule_set_href_of("/orgs/5/sec_policy/draft/rule_sets/"),
            None
        );
        assert_eq!(rule_set_href_of("/orgs/5/labels/1"), None);
        assert!(!is_rule_href(&format!("{rs}/sec_rules")));
        assert!(is_rule_href(&format!("{rs}/sec_rules/1")));
        assert!(is_rule_href(&format!("{rs}/deny_rules/8")));
        assert!(!is_rule_href(rs));
    }

    #[test]
    fn json_has_the_documented_shape() {
        let i = run(
            &shop(),
            json!([label(1)]),
            json!([{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}]),
            true,
        );
        assert_eq!(
            serde_json::to_value(&i).unwrap(),
            json!({
                "providers": 1, "consumers": 0,
                "per_scope": [{"scope": ["app=shop", "env=prod"], "providers": 1, "consumers": 0}],
                "ip_lists": [{
                    "name": "Corp", "href": "/orgs/5/sec_policy/draft/ip_lists/21",
                    "size": {"v4": "65536", "v6": "0", "unbounded": null, "ranges": 1, "fqdns": 1},
                    "side": "consumer"
                }],
                "unsupported": [], "truncated": false
            })
        );
    }
}
