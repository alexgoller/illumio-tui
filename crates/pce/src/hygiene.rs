//! Hygiene: the reverse of where-used. The labels, label groups, IP lists
//! and services nothing names, and the workloads missing label keys.
//!
//! "Used" is what where-used finds, built once over the same loaded data
//! ([`Used`]): ruleset scopes, rule providers, consumers and services
//! (allow, deny, override deny), the label groups holding a label or a
//! group, and for labels the pairing profiles giving them and the
//! workloads carrying them. An unused row is a candidate, never a verdict:
//! what where-used does not search ([`NOT_SEARCHED`]) may still name it.
//! The PCE's built-ins (the `Any (0.0.0.0/0 and ::/0)` IP list, `All
//! Services`, anything from `com.illumio.ilo_defaults`) are never listed.
//! Pure: the TUI and the CLI hand in what they loaded.
use crate::models::{
    Actor, DenyRule, IngressService, IpList, Label, LabelGroup, PairingProfile, Service, Workload,
};
use crate::resolve::canon;
use crate::where_used::Inputs;
use serde::Serialize;
use std::collections::HashSet;

pub use crate::where_used::NOT_SEARCHED;

/// The `external_data_set` the PCE gives its built-in objects.
const ILO_DEFAULTS: &str = "com.illumio.ilo_defaults";
/// The built-in IP list, by name (a PCE may not set `external_data_set`).
pub const ANY_IP_LIST: &str = "Any (0.0.0.0/0 and ::/0)";
/// The built-in service, by name.
pub const ALL_SERVICES: &str = "All Services";

/// The kind of a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Label,
    LabelGroup,
    IpList,
    Service,
    Workload,
}

impl Kind {
    /// The kinds, in the order rows are listed.
    pub const ALL: [Kind; 5] = [
        Kind::Label,
        Kind::LabelGroup,
        Kind::IpList,
        Kind::Service,
        Kind::Workload,
    ];

    /// `label`, `label_group`, `ip_list`, `service`, `workload`, as JSON
    /// and `--kind` name it.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Label => "label",
            Kind::LabelGroup => "label_group",
            Kind::IpList => "ip_list",
            Kind::Service => "service",
            Kind::Workload => "workload",
        }
    }

    /// From `--kind`'s words; `-` and `_` alike.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase().replace('-', "_");
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// `label`, `label group`, `IP list`, `service`, `workload`.
    pub fn word(self) -> &'static str {
        match self {
            Kind::Label => "label",
            Kind::LabelGroup => "label group",
            Kind::IpList => "IP list",
            Kind::Service => "service",
            Kind::Workload => "workload",
        }
    }

    /// `labels`, `label groups`, `IP lists`, `services`, `workloads`.
    pub fn plural(self) -> &'static str {
        match self {
            Kind::Label => "labels",
            Kind::LabelGroup => "label groups",
            Kind::IpList => "IP lists",
            Kind::Service => "services",
            Kind::Workload => "workloads",
        }
    }
}

/// One row: an unused object, or a workload missing label keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub kind: Kind,
    /// `key=value` for a label; a workload as the Workloads screen names it
    /// ([`Workload::display_name`]).
    pub name: String,
    pub href: String,
    /// `unused`, or `missing labels: app,env`.
    pub reason: String,
    /// The label keys a workload lacks; `None` for the other kinds.
    pub missing: Option<Vec<String>>,
}

/// Every href something names, by its draft form: built in one pass over
/// the loaded data.
#[derive(Debug, Clone, Default)]
pub struct Used {
    hrefs: HashSet<String>,
    /// Rulesets whose deny rules were not read (they may name more).
    pub deny_unread: usize,
}

impl Used {
    /// What the draft policy (scopes and every rule side), the label groups
    /// (their labels and sub-groups), the pairing profiles (their labels)
    /// and the workloads (their labels) name. Deny rules as where-used
    /// reads them: inline, else `fallback_deny`, else counted unread.
    pub fn new(policy: &Inputs, profiles: &[PairingProfile], workloads: &[Workload]) -> Self {
        let mut used = Self::default();
        let mut add = |href: &str| {
            used.hrefs.insert(canon(href).into_owned());
        };
        let actors = |list: &[Actor], add: &mut dyn FnMut(&str)| {
            for a in list {
                for r in [&a.label, &a.label_group, &a.ip_list].into_iter().flatten() {
                    add(&r.href);
                }
            }
        };
        let services = |list: &[IngressService], add: &mut dyn FnMut(&str)| {
            for s in list {
                if let IngressService::Ref(r) = s {
                    add(&r.href);
                }
            }
        };
        let mut deny_unread = 0;
        for rs in policy.rule_sets {
            for scope in &rs.scopes {
                actors(scope, &mut add);
            }
            for r in &rs.rules {
                actors(&r.providers, &mut add);
                actors(&r.consumers, &mut add);
                services(&r.ingress_services, &mut add);
            }
            let deny: &[DenyRule] = if rs.deny_rules.is_empty() {
                match policy.fallback_deny.get(&rs.href) {
                    Some(d) => d,
                    None => {
                        deny_unread += usize::from(policy.deny_apart);
                        &[]
                    }
                }
            } else {
                &rs.deny_rules
            };
            for d in deny {
                actors(&d.providers, &mut add);
                actors(&d.consumers, &mut add);
                services(&d.ingress_services, &mut add);
            }
        }
        for g in policy.groups {
            for r in g.labels.iter().chain(&g.sub_groups) {
                add(&r.href);
            }
        }
        for p in profiles {
            for l in &p.labels {
                add(&l.href);
            }
        }
        for w in workloads.iter().filter(|w| w.deleted != Some(true)) {
            for l in &w.labels {
                add(&l.href);
            }
        }
        used.deny_unread = deny_unread;
        used
    }

    /// Whether anything names `href` (draft or active: compared as drafts).
    pub fn contains(&self, href: &str) -> bool {
        self.hrefs.contains(canon(href).as_ref())
    }
}

/// The PCE's built-in IP list: `ilo_defaults`, or named `Any (0.0.0.0/0
/// and ::/0)`.
pub fn is_builtin_ip_list(l: &IpList) -> bool {
    l.external_data_set.as_deref() == Some(ILO_DEFAULTS) || l.name == ANY_IP_LIST
}

/// A PCE built-in service: `ilo_defaults` (All Services, ICMP, ...), or
/// named `All Services`.
pub fn is_builtin_service(s: &Service) -> bool {
    s.external_data_set.as_deref() == Some(ILO_DEFAULTS) || s.name == ALL_SERVICES
}

/// The objects to check: each kind's list, `None` for a kind not asked.
#[derive(Debug, Clone, Copy, Default)]
pub struct Objects<'a> {
    pub labels: Option<&'a [Label]>,
    pub groups: Option<&'a [LabelGroup]>,
    pub ip_lists: Option<&'a [IpList]>,
    pub services: Option<&'a [Service]>,
}

fn unused_row(kind: Kind, name: String, href: &str) -> Finding {
    Finding {
        kind,
        name,
        href: href.to_string(),
        reason: "unused".to_string(),
        missing: None,
    }
}

/// The objects `used` does not hold: labels (deleted ones left out), label
/// groups, IP lists and services (built-ins left out), each kind by name,
/// case-insensitively.
pub fn unused(objects: &Objects, used: &Used) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut kind = |mut rows: Vec<Finding>| {
        rows.sort_by_cached_key(|r| (r.name.to_ascii_lowercase(), r.href.clone()));
        out.extend(rows);
    };
    if let Some(labels) = objects.labels {
        kind(
            labels
                .iter()
                .filter(|l| !l.is_deleted() && !used.contains(&l.href))
                .map(|l| unused_row(Kind::Label, format!("{}={}", l.key, l.value), &l.href))
                .collect(),
        );
    }
    if let Some(groups) = objects.groups {
        kind(
            groups
                .iter()
                .filter(|g| !used.contains(&g.href))
                .map(|g| unused_row(Kind::LabelGroup, g.name.clone(), &g.href))
                .collect(),
        );
    }
    if let Some(lists) = objects.ip_lists {
        kind(
            lists
                .iter()
                .filter(|l| !is_builtin_ip_list(l) && !used.contains(&l.href))
                .map(|l| unused_row(Kind::IpList, l.name.clone(), &l.href))
                .collect(),
        );
    }
    if let Some(services) = objects.services {
        kind(
            services
                .iter()
                .filter(|s| !is_builtin_service(s) && !used.contains(&s.href))
                .map(|s| unused_row(Kind::Service, s.name.clone(), &s.href))
                .collect(),
        );
    }
    out
}

/// The workloads, managed and unmanaged, lacking one or more of `keys`,
/// in list order, each with the keys it lacks (in `keys`' order).
pub fn missing_labels(workloads: &[Workload], keys: &[String]) -> Vec<Finding> {
    workloads
        .iter()
        .filter(|w| w.deleted != Some(true))
        .filter_map(|w| {
            let missing = w.missing_label_keys(keys);
            (!missing.is_empty()).then(|| Finding {
                kind: Kind::Workload,
                name: w.display_name(),
                href: w.href.clone(),
                reason: format!("missing labels: {}", missing.join(",")),
                missing: Some(missing),
            })
        })
        .collect()
}

/// How many rows of each kind, every kind of `kinds` (zeros included).
pub fn counts_by_kind(rows: &[Finding], kinds: &[Kind]) -> Vec<(Kind, usize)> {
    kinds
        .iter()
        .map(|k| (*k, rows.iter().filter(|r| r.kind == *k).count()))
        .collect()
}

/// How many workloads lack each of `keys`, in `keys`' order (zeros
/// included).
pub fn counts_by_key(rows: &[Finding], keys: &[String]) -> Vec<(String, usize)> {
    keys.iter()
        .map(|k| {
            let n = rows
                .iter()
                .filter(|r| r.missing.as_ref().is_some_and(|m| m.contains(k)))
                .count();
            (k.clone(), n)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::RuleSet;
    use crate::where_used::{ObjectType, Target, where_used};
    use serde_json::json;
    use std::collections::HashMap;

    fn load<T: serde::de::DeserializeOwned>(s: &str) -> T {
        serde_json::from_str(s).unwrap()
    }
    fn rule_sets() -> Vec<RuleSet> {
        load(include_str!("../../../fixtures/rule_sets.json"))
    }
    fn groups() -> Vec<LabelGroup> {
        load(include_str!("../../../fixtures/label_groups.json"))
    }
    fn labels() -> Vec<Label> {
        load(include_str!("../../../fixtures/labels.json"))
    }
    fn ip_lists() -> Vec<IpList> {
        load(include_str!("../../../fixtures/ip_lists.json"))
    }
    fn services() -> Vec<Service> {
        load(include_str!("../../../fixtures/services.json"))
    }
    fn profiles() -> Vec<PairingProfile> {
        load(include_str!("../../../fixtures/pairing_profiles.json"))
    }
    fn workloads() -> Vec<Workload> {
        load(include_str!("../../../fixtures/workloads.json"))
    }

    fn used_by(sets: &[RuleSet], gs: &[LabelGroup], p: &[PairingProfile], w: &[Workload]) -> Used {
        let none = HashMap::new();
        Used::new(
            &Inputs {
                rule_sets: sets,
                fallback_deny: &none,
                groups: gs,
                deny_apart: false,
            },
            p,
            w,
        )
    }

    fn names(rows: &[Finding]) -> Vec<String> {
        rows.iter()
            .map(|r| format!("{} {}", r.kind.as_str(), r.name))
            .collect()
    }

    fn all<'a>(
        l: &'a [Label],
        g: &'a [LabelGroup],
        i: &'a [IpList],
        s: &'a [Service],
    ) -> Objects<'a> {
        Objects {
            labels: Some(l),
            groups: Some(g),
            ip_lists: Some(i),
            services: Some(s),
        }
    }

    #[test]
    fn the_fixtures_unused_objects_without_built_ins_or_deleted_labels() {
        let (l, g, i, s) = (labels(), groups(), ip_lists(), services());
        let used = used_by(&rule_sets(), &g, &profiles(), &workloads());
        // loc=eu: nothing holds it; app (deleted) is left out. Any and All
        // Services are built-ins; Web is named by no rule (inline ports
        // are no service object).
        assert_eq!(
            names(&unused(&all(&l, &g, &i, &s), &used)),
            ["label loc=eu", "service Web"]
        );
        let r = &unused(&all(&l, &g, &i, &s), &used)[0];
        assert_eq!(
            (r.href.as_str(), r.reason.as_str(), &r.missing),
            ("/orgs/5/labels/5", "unused", &None)
        );
    }

    #[test]
    fn each_place_that_names_an_object_makes_it_used() {
        let (l, i, s) = (labels(), ip_lists(), services());
        let g = vec![
            load::<LabelGroup>(
                r#"{"href":"/orgs/5/sec_policy/draft/label_groups/10","name":"All prod envs","labels":[{"href":"/orgs/5/labels/4"}]}"#,
            ),
            load::<LabelGroup>(
                r#"{"href":"/orgs/5/sec_policy/draft/label_groups/11","name":"Spare","labels":[]}"#,
            ),
        ];
        // Nothing at all: every live object is a candidate.
        let empty = used_by(&[], &[], &[], &[]);
        assert_eq!(
            names(&unused(&all(&l, &g, &i, &s), &empty)),
            [
                "label app=shop",
                "label env=prod",
                "label loc=eu",
                "label role=db",
                "label role=web",
                "label_group All prod envs",
                "label_group Spare",
                "ip_list Corp",
                "service PostgreSQL",
                "service Web",
            ]
        );
        // A label group holding env=prod; a group holding Spare.
        let mut g2 = g.clone();
        g2[0].sub_groups = vec![load(
            r#"{"href":"/orgs/5/sec_policy/active/label_groups/11"}"#,
        )];
        let used = used_by(&[], &g2, &[], &[]);
        let rows = names(&unused(&all(&l, &g2, &i, &s), &used));
        assert!(!rows.contains(&"label env=prod".to_string()));
        assert!(
            !rows.contains(&"label_group Spare".to_string()),
            "active href"
        );
        assert!(rows.contains(&"label_group All prod envs".to_string()));
        // A pairing profile gives role=web and app=shop.
        let used = used_by(&[], &[], &profiles(), &[]);
        let rows = names(&unused(&all(&l, &[], &[], &[]), &used));
        assert_eq!(rows, ["label env=prod", "label loc=eu", "label role=db"]);
        // Workloads carry role, app and env.
        let used = used_by(&[], &[], &[], &workloads());
        let rows = names(&unused(&all(&l, &[], &[], &[]), &used));
        assert_eq!(rows, ["label loc=eu"]);
        // The rulesets: scopes (app, env), rule sides (roles, IP lists,
        // the group), services (PostgreSQL, All Services), deny rules.
        let used = used_by(&rule_sets(), &[], &[], &[]);
        assert_eq!(
            names(&unused(&all(&l, &g, &i, &s), &used)),
            ["label loc=eu", "label_group Spare", "service Web"]
        );
    }

    #[test]
    fn built_ins_by_flag_or_name() {
        let any = |name: &str, set: Option<&str>| -> IpList {
            load(
                &json!({"href": "/orgs/5/sec_policy/draft/ip_lists/1", "name": name,
                "external_data_set": set})
                .to_string(),
            )
        };
        assert!(is_builtin_ip_list(&any(ANY_IP_LIST, None)));
        assert!(is_builtin_ip_list(&any("Anything", Some(ILO_DEFAULTS))));
        assert!(!is_builtin_ip_list(&any("Corp", None)));
        let svc = |name: &str, set: Option<&str>| -> Service {
            load(
                &json!({"href": "/orgs/5/sec_policy/draft/services/1", "name": name,
                "external_data_set": set})
                .to_string(),
            )
        };
        assert!(is_builtin_service(&svc(ALL_SERVICES, None)));
        assert!(is_builtin_service(&svc("ICMP", Some(ILO_DEFAULTS))));
        assert!(!is_builtin_service(&svc("Web", None)));
        let none = used_by(&[], &[], &[], &[]);
        let objs = Objects {
            ip_lists: Some(&[any("Anything", Some(ILO_DEFAULTS))]),
            services: Some(&[svc("ICMP", Some(ILO_DEFAULTS))]),
            ..Objects::default()
        };
        assert!(unused(&objs, &none).is_empty());
    }

    #[test]
    fn deny_rules_read_apart_count_and_unread_ones_are_counted() {
        let mut sets = rule_sets();
        sets[0].deny_rules.clear();
        let none = HashMap::new();
        let used = Used::new(
            &Inputs {
                rule_sets: &sets,
                fallback_deny: &none,
                groups: &[],
                deny_apart: true,
            },
            &[],
            &[],
        );
        assert_eq!(used.deny_unread, 2);
        // The deny rule naming the Any list (20) is not read.
        assert!(!used.contains("/orgs/5/sec_policy/draft/ip_lists/20"));
        let deny: Vec<DenyRule> = load(include_str!("../../../fixtures/deny_rules.json"));
        let fallback = HashMap::from([(sets[0].href.clone(), deny)]);
        let used = Used::new(
            &Inputs {
                rule_sets: &sets,
                fallback_deny: &fallback,
                groups: &[],
                deny_apart: true,
            },
            &[],
            &[],
        );
        assert_eq!(used.deny_unread, 1);
        assert!(used.contains("/orgs/5/sec_policy/draft/ip_lists/20"));
    }

    #[test]
    fn used_agrees_with_where_used_on_the_policy_and_groups() {
        // Every object where-used finds something for is used, and the
        // other way round (labels: before pairing profiles and workloads).
        let (sets, gs) = (rule_sets(), groups());
        let used = used_by(&sets, &gs, &[], &[]);
        let none = HashMap::new();
        let inputs = Inputs {
            rule_sets: &sets,
            fallback_deny: &none,
            groups: &gs,
            deny_apart: false,
        };
        let targets = labels()
            .iter()
            .map(|l| Target::new(ObjectType::Label, l.href.clone()))
            .chain(
                gs.iter()
                    .map(|g| Target::new(ObjectType::LabelGroup, g.href.clone())),
            )
            .chain(
                ip_lists()
                    .iter()
                    .map(|l| Target::new(ObjectType::IpList, l.href.clone())),
            )
            .chain(
                services()
                    .iter()
                    .map(|s| Target::new(ObjectType::Service, s.href.clone())),
            )
            .collect::<Vec<_>>();
        for t in targets {
            let found = !where_used(&t, &inputs).refs.is_empty();
            assert_eq!(used.contains(&t.href), found, "{}", t.href);
        }
    }

    #[test]
    fn workloads_missing_label_keys_with_counts_per_key() {
        let keys: Vec<String> = ["role", "app", "env", "loc"].map(String::from).to_vec();
        let rows = missing_labels(&workloads(), &keys);
        assert_eq!(
            rows.iter()
                .map(|r| (r.name.as_str(), r.reason.as_str()))
                .collect::<Vec<_>>(),
            [
                ("host-1.example", "missing labels: loc"),
                ("host-2.example", "missing labels: loc"),
                ("unmanaged-nas", "missing labels: role,app,env,loc"),
                ("host-4.example", "missing labels: role,app,env,loc"),
            ]
        );
        assert_eq!(rows[2].kind, Kind::Workload);
        assert_eq!(rows[2].href, "/orgs/5/workloads/aaaa-3");
        assert_eq!(rows[0].missing.as_deref(), Some(&["loc".to_string()][..]));
        assert_eq!(
            counts_by_key(&rows, &keys),
            [
                ("role".to_string(), 2),
                ("app".to_string(), 2),
                ("env".to_string(), 2),
                ("loc".to_string(), 4)
            ]
        );
        // Every key present: no row.
        let three: Vec<String> = ["role", "app", "env"].map(String::from).to_vec();
        assert_eq!(missing_labels(&workloads(), &three).len(), 2);
        assert_eq!(
            counts_by_kind(&rows, &[Kind::Label, Kind::Workload]),
            [(Kind::Label, 0), (Kind::Workload, 4)]
        );
    }

    #[test]
    fn kinds_parse_from_their_names() {
        assert_eq!(Kind::parse("label-group"), Some(Kind::LabelGroup));
        assert_eq!(Kind::parse(" IP_LIST "), Some(Kind::IpList));
        assert_eq!(Kind::parse("workload"), Some(Kind::Workload));
        assert_eq!(Kind::parse("rule"), None);
        assert_eq!(Kind::IpList.plural(), "IP lists");
    }
}
