//! A proposed rule against its target ruleset's draft rules: already
//! covered (`Exists`), the same actors lacking services (`AddServices`),
//! or new (`Create`). A false "covered" silently drops a needed rule, so
//! every test here errs towards `Create`.
//!
//! Sides are compared as sets with Illumio's semantics: a rule's labels
//! AND across keys and OR within one key; `ams`, IP lists and workloads
//! add to that. An existing side covers a proposed one when it holds
//! every workload the proposed one names.
use super::{GenActor, GenRule, RuleAction, Target, actor_key};
use crate::models::{Actor, IngressService, Rule, RuleSet};
use crate::resolve::{NameIndex, canon};
use crate::write::{ActorKind, ActorRef, ServiceRef};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn classify(
    rule: &GenRule,
    target: &Target,
    drafts: &[RuleSet],
    idx: &NameIndex,
) -> RuleAction {
    let Target::Existing { href, .. } = target else {
        return RuleAction::Create;
    };
    let Some(rs) = drafts.iter().find(|r| &r.href == href) else {
        return RuleAction::Create;
    };
    let live: Vec<&Rule> = rs.rules.iter().filter(|r| eligible(r)).collect();
    if let Some(r) = live.iter().find(|r| covers(r, rule, idx)) {
        return RuleAction::Exists {
            rule_href: r.href.clone(),
        };
    }
    for r in &live {
        if same_actors(r, rule) {
            let add: Vec<ServiceRef> = rule
                .services
                .iter()
                .filter(|s| !service_covered(&r.ingress_services, s, idx))
                .cloned()
                .collect();
            if !add.is_empty() {
                return RuleAction::AddServices {
                    rule_href: r.href.clone(),
                    add,
                };
            }
        }
    }
    RuleAction::Create
}

/// A rule that can cover anything: enabled, not pending deletion, for
/// workloads, not narrowed by users, machine authentication or network.
fn eligible(r: &Rule) -> bool {
    let workloads = |v: &[String]| v.iter().any(|x| x == "workloads");
    r.enabled
        && r.update_type.as_deref() != Some("delete")
        && r.resolve_labels_as
            .as_ref()
            .is_none_or(|x| workloads(&x.providers) && workloads(&x.consumers))
        && r.machine_auth != Some(true)
        && r.network_type
            .as_deref()
            .is_none_or(|n| matches!(n, "brn" | "all"))
        && r.extra
            .get("consuming_security_principals")
            .and_then(|v| v.as_array())
            .is_none_or(Vec::is_empty)
}

fn covers(existing: &Rule, rule: &GenRule, idx: &NameIndex) -> bool {
    existing.unscoped_consumers.unwrap_or(false) == rule.unscoped_consumers
        && side_covers(&existing.providers, &rule.providers, true, idx)
        && side_covers(
            &existing.consumers,
            &rule.consumers,
            !rule.unscoped_consumers,
            idx,
        )
        && rule
            .services
            .iter()
            .all(|s| service_covered(&existing.ingress_services, s, idx))
}

/// Whether `existing` holds every workload `proposed` names. `in_scope`:
/// the side resolves within the ruleset's scope, so `ams` there covers
/// any workload actor.
fn side_covers(existing: &[Actor], proposed: &[GenActor], in_scope: bool, idx: &NameIndex) -> bool {
    if existing.iter().any(|a| a.exclusion == Some(true)) {
        return false;
    }
    let ams = existing.iter().any(|a| a.actors.as_deref() == Some("ams"));
    let has_ref = |kind: &str, href: &str| {
        existing.iter().any(|a| {
            let r = match kind {
                "ip_list" => a.ip_list.as_ref(),
                _ => a.workload.as_ref(),
            };
            r.is_some_and(|r| canon(&r.href) == canon(href))
        })
    };
    let mut labels: BTreeMap<String, String> = BTreeMap::new();
    for p in proposed {
        let GenActor::Ref(a) = p else {
            return false; // an IP list that does not exist yet
        };
        let href = a.href.as_deref().unwrap_or_default();
        match a.kind {
            ActorKind::All if !ams => return false,
            ActorKind::All => {}
            ActorKind::IpList if !has_ref("ip_list", href) => return false,
            ActorKind::IpList => {}
            ActorKind::Workload if !(has_ref("workload", href) || (ams && in_scope)) => {
                return false;
            }
            ActorKind::Workload => {}
            ActorKind::Label => match idx.label(href) {
                Some(l) => {
                    labels.insert(l.key.clone(), href.to_string());
                }
                None => return false,
            },
            ActorKind::LabelGroup => return false, // never proposed
        }
    }
    if labels.is_empty() || (ams && in_scope) {
        return true;
    }
    match label_constraint(existing, idx) {
        Some(c) if !c.is_empty() => c
            .iter()
            .all(|(k, allowed)| labels.get(k).is_some_and(|h| allowed.contains(h))),
        _ => false,
    }
}

/// The existing side's labels and label groups by key: the hrefs each key
/// allows. `None` when one cannot be resolved (then nothing is proven).
fn label_constraint(
    existing: &[Actor],
    idx: &NameIndex,
) -> Option<BTreeMap<String, BTreeSet<String>>> {
    let mut c: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for a in existing {
        if let Some(l) = &a.label {
            let key = idx.label(&l.href)?.key.clone();
            c.entry(key).or_default().insert(l.href.clone());
        } else if let Some(g) = &a.label_group {
            let key = idx.label_group(&g.href)?.key.clone();
            c.entry(key)
                .or_default()
                .extend(crate::impact::group_members(&g.href, idx));
        }
    }
    Some(c)
}

/// The same actors on both sides and the same flag: the rule a service
/// can be added to.
fn same_actors(existing: &Rule, rule: &GenRule) -> bool {
    let keys = |actors: &[Actor]| -> Option<BTreeSet<String>> {
        actors
            .iter()
            .map(|a| ActorRef::from_actor(a).ok().map(|r| actor_key(&r)))
            .collect()
    };
    let mine = |actors: &[GenActor]| -> BTreeSet<String> {
        actors.iter().map(GenActor::sort_key).collect()
    };
    existing.unscoped_consumers.unwrap_or(false) == rule.unscoped_consumers
        && keys(&existing.providers) == Some(mine(&rule.providers))
        && keys(&existing.consumers) == Some(mine(&rule.consumers))
}

/// One port entry: a protocol (`-1` all), a port range (`None` all ports).
#[derive(Debug, Clone, Copy)]
struct Entry {
    proto: i64,
    from: Option<u16>,
    to: Option<u16>,
}

impl Entry {
    fn covers(&self, p: &Entry) -> bool {
        if self.proto == -1 {
            return true;
        }
        if self.proto != p.proto {
            return false;
        }
        let Some(from) = self.from else {
            return true;
        };
        let Some(pf) = p.from else {
            return false;
        };
        from <= pf && p.to.unwrap_or(pf) <= self.to.unwrap_or(from)
    }
}

/// A service object's entries; `None` when unknown or narrowed by an ICMP
/// type (only its own href covers it then).
fn object_entries(href: &str, idx: &NameIndex) -> Option<Vec<Entry>> {
    let s = idx.service(href)?;
    s.service_ports
        .iter()
        .map(|p| {
            if p.icmp_type.is_some() {
                return None;
            }
            Some(Entry {
                proto: p.proto?,
                from: p.port,
                to: p.to_port,
            })
        })
        .collect::<Option<Vec<Entry>>>()
        .filter(|v| !v.is_empty())
}

/// Whether the existing services cover `s`: the same object, or entries
/// that hold every port `s` stands for (All Services holds all).
fn service_covered(existing: &[IngressService], s: &ServiceRef, idx: &NameIndex) -> bool {
    let mut hrefs = BTreeSet::new();
    let mut entries = Vec::new();
    for e in existing {
        match e {
            IngressService::Ref(r) => {
                hrefs.insert(canon(&r.href).into_owned());
                entries.extend(object_entries(&r.href, idx).unwrap_or_default());
            }
            IngressService::Port {
                port,
                to_port,
                proto,
            } => entries.push(Entry {
                proto: *proto,
                from: *port,
                to: *to_port,
            }),
        }
    }
    let wanted = match s {
        ServiceRef::Href(h) if hrefs.contains(canon(h).as_ref()) => return true,
        ServiceRef::Href(h) => match object_entries(h, idx) {
            Some(v) => v,
            None => return false,
        },
        ServiceRef::Port(p) => vec![Entry {
            proto: p.proto,
            from: p.port,
            to: p.to_port,
        }],
    };
    wanted.iter().all(|w| entries.iter().any(|e| e.covers(w)))
}

#[cfg(test)]
mod tests {
    use super::super::{Coverage, GenRule, Proposal, TargetScope};
    use super::*;
    use crate::resolve::ResolvedLabel;
    use crate::traffic::resolved::tests::rich_index;
    use crate::write::PortSpec;
    use serde_json::json;

    const RS: &str = "/orgs/5/sec_policy/draft/rule_sets/100";

    fn port(s: &str) -> ServiceRef {
        ServiceRef::Port(PortSpec::parse(s).unwrap())
    }

    fn lbl(id: u32) -> GenActor {
        GenActor::label(&format!("/orgs/5/labels/{id}"))
    }

    fn target() -> Target {
        Target::Existing {
            href: RS.into(),
            name: "Shop-Prod".into(),
        }
    }

    /// Shop-Prod with exactly these rules.
    fn drafts(rules: serde_json::Value) -> Vec<RuleSet> {
        let mut sets: Vec<RuleSet> =
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
        sets[0].rules = serde_json::from_value(rules).unwrap();
        sets
    }

    fn rule(json: serde_json::Value) -> serde_json::Value {
        let mut r = json!({
            "href": format!("{RS}/sec_rules/9"), "enabled": true,
            "providers": [{"actors": "ams"}], "consumers": [{"actors": "ams"}],
            "ingress_services": [{"port": 5432, "proto": 6}], "unscoped_consumers": false
        });
        for (k, v) in json.as_object().unwrap() {
            r[k] = v.clone();
        }
        r
    }

    fn proposed(
        providers: Vec<GenActor>,
        consumers: Vec<GenActor>,
        unscoped: bool,
        services: Vec<ServiceRef>,
    ) -> GenRule {
        GenRule {
            target: 0,
            providers,
            consumers,
            unscoped_consumers: unscoped,
            services,
            rows: vec![0],
            action: RuleAction::Create,
            description: String::new(),
        }
    }

    fn intra() -> GenRule {
        proposed(
            vec![GenActor::all()],
            vec![GenActor::all()],
            false,
            vec![port("5432/tcp")],
        )
    }

    /// hr/prod into shop/prod: `app=hr, env=prod`, unscoped.
    fn hr() -> GenRule {
        proposed(
            vec![GenActor::all()],
            vec![lbl(103), lbl(4)],
            true,
            vec![port("5432/tcp")],
        )
    }

    fn classify_in(rules: serde_json::Value, r: &GenRule) -> RuleAction {
        classify(r, &target(), &drafts(json!([rules])), &rich_index())
    }

    fn exists() -> RuleAction {
        RuleAction::Exists {
            rule_href: format!("{RS}/sec_rules/9"),
        }
    }

    #[test]
    fn the_same_rule_exists() {
        assert_eq!(classify_in(rule(json!({})), &intra()), exists());
    }

    #[test]
    fn missing_services_on_the_same_actors_are_added() {
        let r = rule(json!({"ingress_services": [{"port": 443, "proto": 6}]}));
        assert_eq!(
            classify_in(r, &intra()),
            RuleAction::AddServices {
                rule_href: format!("{RS}/sec_rules/9"),
                add: vec![port("5432/tcp")]
            }
        );
    }

    #[test]
    fn all_services_a_range_and_a_service_object_cover_a_port() {
        for svc in [
            json!([{"href": "/orgs/5/sec_policy/draft/services/30"}]),
            json!([{"port": 5000, "to_port": 6000, "proto": 6}]),
            json!([{"href": "/orgs/5/sec_policy/draft/services/31"}]),
        ] {
            assert_eq!(
                classify_in(rule(json!({"ingress_services": svc})), &intra()),
                exists(),
                "{svc}"
            );
        }
        let udp = rule(json!({"ingress_services": [{"port": 5432, "proto": 17}]}));
        assert_ne!(classify_in(udp, &intra()), exists());
    }

    #[test]
    fn a_label_group_member_covers_and_a_broader_side_covers() {
        let grouped = rule(json!({
            "consumers": [{"label": {"href": "/orgs/5/labels/103"}},
                          {"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/10"}}],
            "unscoped_consumers": true
        }));
        assert_eq!(classify_in(grouped, &hr()), exists());
        let app_only = rule(json!({
            "consumers": [{"label": {"href": "/orgs/5/labels/103"}}], "unscoped_consumers": true
        }));
        assert_eq!(
            classify_in(app_only, &hr()),
            exists(),
            "every env of hr holds hr/prod"
        );
    }

    #[test]
    fn a_narrower_existing_rule_is_not_a_cover() {
        let narrower = rule(json!({
            "consumers": [{"label": {"href": "/orgs/5/labels/103"}}, {"label": {"href": "/orgs/5/labels/4"}},
                          {"label": {"href": "/orgs/5/labels/1"}}],
            "unscoped_consumers": true
        }));
        assert_eq!(classify_in(narrower, &hr()), RuleAction::Create);
        let role_only = rule(json!({"providers": [{"label": {"href": "/orgs/5/labels/2"}}]}));
        assert_eq!(
            classify_in(role_only, &intra()),
            RuleAction::Create,
            "role=db is not all workloads"
        );
    }

    #[test]
    fn ams_covers_on_the_in_scope_side_only() {
        let ams = rule(json!({}));
        let scoped = proposed(
            vec![GenActor::all()],
            vec![lbl(1)],
            false,
            vec![port("5432/tcp")],
        );
        assert_eq!(classify_in(ams.clone(), &scoped), exists());
        let mut unscoped_ams = ams;
        unscoped_ams["unscoped_consumers"] = json!(true);
        assert_eq!(classify_in(unscoped_ams, &hr()), RuleAction::Create);
    }

    #[test]
    fn a_different_unscoped_flag_is_another_rule() {
        let r = rule(json!({"unscoped_consumers": true}));
        assert_eq!(classify_in(r, &intra()), RuleAction::Create);
    }

    #[test]
    fn disabled_pending_delete_exclusions_and_user_rules_never_cover() {
        for extra in [
            json!({"enabled": false}),
            json!({"update_type": "delete"}),
            json!({"consuming_security_principals": [{"href": "/orgs/5/security_principals/1"}]}),
            json!({"machine_auth": true}),
            json!({"resolve_labels_as": {"providers": ["virtual_services"], "consumers": ["workloads"]}}),
            json!({"consumers": [{"actors": "ams"}, {"label": {"href": "/orgs/5/labels/1"}, "exclusion": true}]}),
        ] {
            assert_eq!(
                classify_in(rule(extra.clone()), &intra()),
                RuleAction::Create,
                "{extra}"
            );
        }
    }

    #[test]
    fn ip_list_consumers_match_by_href_and_a_new_list_never_exists() {
        let corp = GenActor::Ref(ActorRef::new(
            ActorKind::IpList,
            "/orgs/5/sec_policy/draft/ip_lists/21",
        ));
        let mine = proposed(
            vec![GenActor::all()],
            vec![corp],
            false,
            vec![port("443/tcp")],
        );
        let r = rule(json!({
            "consumers": [{"ip_list": {"href": "/orgs/5/sec_policy/active/ip_lists/21"}}],
            "ingress_services": [{"port": 443, "proto": 6}]
        }));
        assert_eq!(
            classify_in(r.clone(), &mine),
            exists(),
            "an active href is its draft"
        );
        let new = proposed(
            vec![GenActor::all()],
            vec![GenActor::NewIpList(0)],
            false,
            vec![port("443/tcp")],
        );
        assert_eq!(classify_in(r, &new), RuleAction::Create);
    }

    #[test]
    fn a_ruleset_to_create_has_nothing_to_dedupe() {
        let d = drafts(json!([rule(json!({}))]));
        let t = Target::Create(crate::write::RuleSetDraft {
            name: "RF-shop-prod".into(),
            description: None,
            enabled: true,
            scopes: vec![],
        });
        assert_eq!(
            classify(&intra(), &t, &d, &rich_index()),
            RuleAction::Create
        );
    }

    #[test]
    fn refresh_reclassifies_against_new_drafts() {
        let mut p = Proposal {
            scopes: vec![TargetScope {
                values: vec!["shop".into(), "prod".into()],
                labels: vec![
                    ResolvedLabel {
                        href: "/orgs/5/labels/3".into(),
                        key: "app".into(),
                        value: "shop".into(),
                    },
                    ResolvedLabel {
                        href: "/orgs/5/labels/4".into(),
                        key: "env".into(),
                        value: "prod".into(),
                    },
                ],
            }],
            targets: Vec::new(),
            ip_lists: Vec::new(),
            rules: vec![intra()],
            coverage: Coverage::default(),
        };
        let idx = rich_index();
        p.refresh(&drafts(json!([])), &idx, "RF-{app}-{env}", None);
        assert_eq!(p.rules[0].action, RuleAction::Create);
        assert!(p.writes());
        p.refresh(
            &drafts(json!([rule(json!({}))])),
            &idx,
            "RF-{app}-{env}",
            None,
        );
        assert_eq!(p.rules[0].action, exists());
        assert!(!p.writes());
        assert_eq!(p.counts().exists, 1);
    }

    /// Review focus 1: the recorded demo PCE's scoped ruleset `14. pos |
    /// pci` holds an IP list consumer with `unscoped_consumers: false`,
    /// as the generator proposes for an IP peer.
    #[test]
    fn recorded_scoped_rulesets_name_ip_list_consumers_in_scope() {
        let sets: Vec<RuleSet> =
            serde_json::from_str(include_str!("../../../../fixtures/recorded/rule_sets.json"))
                .unwrap();
        let scoped: Vec<&RuleSet> = sets
            .iter()
            .filter(|r| r.scopes.iter().any(|s| !s.is_empty()))
            .collect();
        let ipl: Vec<&Rule> = scoped
            .iter()
            .flat_map(|r| &r.rules)
            .filter(|r| r.consumers.iter().any(|a| a.ip_list.is_some()))
            .collect();
        assert!(!ipl.is_empty());
        assert!(ipl.iter().all(|r| r.unscoped_consumers == Some(false)));
    }
}
