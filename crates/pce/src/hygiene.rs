//! Hygiene: the reverse of where-used. The labels, label groups, IP lists
//! and services nothing names, and the workloads missing label keys.
//!
//! "Used" is built once over the loaded data ([`Used`]): every href the
//! draft rulesets carry anywhere (scopes, rule providers, consumers and
//! services, deny rules, and what this tool does not model, such as custom
//! iptables rules and egress services), the label groups holding a label
//! or a group, and for labels the pairing profiles giving them and the
//! workloads carrying them. A label the PCE's own usage flags
//! (`labels?usage=true`) say is held anywhere is used too.
//!
//! The rule: never call an object unused that may be used. A kind whose
//! holders were not all read (a list cut short or failed) is withheld,
//! unless the PCE's flags settle a label; rows that rest on rulesets
//! whose deny rules were not read are marked uncertain
//! ([`Finding::certain`]). Even a certain row is a candidate, never a
//! verdict: what is not searched ([`NOT_SEARCHED`]) may still name it.
//! The PCE's built-ins (the `Any (0.0.0.0/0 and ::/0)` IP list, `All
//! Services`, anything from `com.illumio.ilo_defaults`), deleted labels
//! and objects pending deletion are never listed. Pure: the TUI and the
//! CLI hand in what they loaded.
use crate::models::{IpList, Label, LabelGroup, PairingProfile, Service, Workload};
use crate::resolve::canon;
use crate::where_used::Inputs;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;

/// What hygiene does not search, in one line, so an unused row reads as a
/// candidate, not a verdict. Hygiene reads the draft policy only: an
/// object still in provisioned (active) policy whose last draft use was
/// removed is listed until that is provisioned.
pub const NOT_SEARCHED: &str = "not searched: active policy (draft only), virtual services, virtual servers, enforcement boundaries, container workloads and their profiles, cloud resources, label mapping rules, RBAC scopes, firewall settings";

/// Appended to [`NOT_SEARCHED`] when the PCE's label usage flags were read:
/// they name most of those holders for labels.
pub const LABELS_BY_FLAGS: &str = "labels: also the PCE's usage flags, which cover these holders";

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
    /// `false`: the verdict rests on rulesets whose deny rules were not
    /// read, which may name the object. Workload rows are always certain.
    pub certain: bool,
}

/// Every href something names, by its draft form: built in one pass over
/// the loaded data.
#[derive(Debug, Clone, Default)]
pub struct Used {
    hrefs: HashSet<String>,
    /// Rulesets whose deny rules were not read (they may name more).
    pub deny_unread: usize,
}

/// Every string under an `"href"` key in `v`, at any depth.
fn walk_hrefs(v: &Value, add: &mut dyn FnMut(&str)) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                match x {
                    Value::String(h) if k == "href" => add(h),
                    _ => walk_hrefs(x, add),
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| walk_hrefs(x, add)),
        _ => {}
    }
}

impl Used {
    /// What the draft policy, the label groups (their labels and
    /// sub-groups), the pairing profiles (their labels) and the workloads
    /// (their labels) name. A ruleset is walked whole, every `href` at any
    /// depth, modelled or not: a field this tool does not know still makes
    /// an object used. Deny rules as where-used reads them: inline, else
    /// `fallback_deny`, else counted unread.
    pub fn new(policy: &Inputs, profiles: &[PairingProfile], workloads: &[Workload]) -> Self {
        let mut used = Self::default();
        let mut add = |href: &str| {
            used.hrefs.insert(canon(href).into_owned());
        };
        let mut deny_unread = 0;
        for rs in policy.rule_sets {
            if let Ok(v) = serde_json::to_value(rs) {
                walk_hrefs(&v, &mut add);
            }
            if rs.deny_rules.is_empty() {
                match policy.fallback_deny.get(&rs.href) {
                    Some(d) => {
                        if let Ok(v) = serde_json::to_value(d) {
                            walk_hrefs(&v, &mut add);
                        }
                    }
                    None => deny_unread += usize::from(policy.deny_apart),
                }
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

/// A draft change deleting it, not provisioned yet: already on its way out.
fn pending_delete(update_type: &Option<String>) -> bool {
    update_type.as_deref() == Some("delete")
}

/// The objects to check: each kind's list, `None` for a kind not asked.
/// Labels carry the PCE's usage flags when read with `usage=true`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Objects<'a> {
    pub labels: Option<&'a [Label]>,
    pub groups: Option<&'a [LabelGroup]>,
    pub ip_lists: Option<&'a [IpList]>,
    pub services: Option<&'a [Service]>,
}

/// Which holders were not read in full: a list cut short, failed or
/// missing. A kind they may hold is withheld rather than guessed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Gaps {
    pub rule_sets: bool,
    pub groups: bool,
    pub profiles: bool,
    pub workloads: bool,
}

impl Gaps {
    /// Whether a holder of `kind` was not read in full.
    pub fn hit(&self, kind: Kind) -> bool {
        match kind {
            Kind::Label => self.rule_sets || self.groups || self.profiles || self.workloads,
            Kind::LabelGroup => self.rule_sets || self.groups,
            Kind::IpList | Kind::Service => self.rule_sets,
            Kind::Workload => false,
        }
    }

    /// What was not read in full, for a warning: `rulesets, workloads`.
    pub fn text(&self) -> String {
        [
            (self.rule_sets, "rulesets"),
            (self.groups, "label groups"),
            (self.profiles, "pairing profiles"),
            (self.workloads, "workloads"),
        ]
        .iter()
        .filter(|(b, _)| *b)
        .map(|(_, t)| *t)
        .collect::<Vec<_>>()
        .join(", ")
    }
}

/// The unused objects, and the kinds left out or uncertain.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Unused {
    /// Each kind by name, case-insensitively.
    pub rows: Vec<Finding>,
    /// Kinds not listed (labels: those the PCE's flags did not settle)
    /// because a list that may hold them was not read in full.
    pub withheld: Vec<Kind>,
    /// Kinds whose rows rest on rulesets whose deny rules were not read.
    pub uncertain: Vec<Kind>,
}

fn unused_row(kind: Kind, name: String, href: &str, certain: bool) -> Finding {
    Finding {
        kind,
        name,
        href: href.to_string(),
        reason: "unused".to_string(),
        missing: None,
        certain,
    }
}

/// The objects nothing holds. A label with the PCE's usage flags is
/// unused only when `used` does not hold it and no flag is set; its row is
/// certain. Otherwise a kind is withheld when `gaps` hits it, and its rows
/// are uncertain while deny rules are unread. Built-ins, deleted labels
/// and objects pending deletion are left out.
pub fn unused(objects: &Objects, used: &Used, gaps: Gaps) -> Unused {
    let mut out = Unused::default();
    let scan_certain = used.deny_unread == 0;
    let mut kind = |k: Kind, checked: Option<Vec<(String, &str, Option<bool>)>>| {
        // (name, href, Some(flags say used) | None: no flags)
        let Some(items) = checked else { return };
        let mut rows = Vec::new();
        let (mut withheld, mut scanned) = (false, false);
        for (name, href, flags) in items {
            match flags {
                Some(held) => {
                    if !held && !used.contains(href) {
                        rows.push(unused_row(k, name, href, true));
                    }
                }
                None if gaps.hit(k) => withheld = true,
                None => {
                    scanned = true;
                    if !used.contains(href) {
                        rows.push(unused_row(k, name, href, scan_certain));
                    }
                }
            }
        }
        rows.sort_by_cached_key(|r| (r.name.to_ascii_lowercase(), r.href.clone()));
        out.rows.extend(rows);
        if withheld {
            out.withheld.push(k);
        } else if scanned && !scan_certain {
            out.uncertain.push(k);
        }
    };
    kind(
        Kind::Label,
        objects.labels.map(|ls| {
            ls.iter()
                .filter(|l| !l.is_deleted())
                .map(|l| {
                    let flags = l.usage_flags().map(|f| f.is_used());
                    (format!("{}={}", l.key, l.value), l.href.as_str(), flags)
                })
                .collect()
        }),
    );
    kind(
        Kind::LabelGroup,
        objects.groups.map(|gs| {
            gs.iter()
                .filter(|g| !pending_delete(&g.update_type))
                .map(|g| (g.name.clone(), g.href.as_str(), None))
                .collect()
        }),
    );
    kind(
        Kind::IpList,
        objects.ip_lists.map(|ls| {
            ls.iter()
                .filter(|l| !is_builtin_ip_list(l) && !pending_delete(&l.update_type))
                .map(|l| (l.name.clone(), l.href.as_str(), None))
                .collect()
        }),
    );
    kind(
        Kind::Service,
        objects.services.map(|ss| {
            ss.iter()
                .filter(|s| !is_builtin_service(s) && !pending_delete(&s.update_type))
                .map(|s| (s.name.clone(), s.href.as_str(), None))
                .collect()
        }),
    );
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
                certain: true,
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
    use crate::models::{DenyRule, RuleSet};
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
            names(&unused(&all(&l, &g, &i, &s), &used, Gaps::default()).rows),
            ["label loc=eu", "service Web"]
        );
        let r = &unused(&all(&l, &g, &i, &s), &used, Gaps::default()).rows[0];
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
            names(&unused(&all(&l, &g, &i, &s), &empty, Gaps::default()).rows),
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
        let rows = names(&unused(&all(&l, &g2, &i, &s), &used, Gaps::default()).rows);
        assert!(!rows.contains(&"label env=prod".to_string()));
        assert!(
            !rows.contains(&"label_group Spare".to_string()),
            "active href"
        );
        assert!(rows.contains(&"label_group All prod envs".to_string()));
        // A pairing profile gives role=web and app=shop.
        let used = used_by(&[], &[], &profiles(), &[]);
        let rows = names(&unused(&all(&l, &[], &[], &[]), &used, Gaps::default()).rows);
        assert_eq!(rows, ["label env=prod", "label loc=eu", "label role=db"]);
        // Workloads carry role, app and env.
        let used = used_by(&[], &[], &[], &workloads());
        let rows = names(&unused(&all(&l, &[], &[], &[]), &used, Gaps::default()).rows);
        assert_eq!(rows, ["label loc=eu"]);
        // The rulesets: scopes (app, env), rule sides (roles, IP lists,
        // the group), services (PostgreSQL, All Services), deny rules.
        let used = used_by(&rule_sets(), &[], &[], &[]);
        assert_eq!(
            names(&unused(&all(&l, &g, &i, &s), &used, Gaps::default()).rows),
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
        assert!(unused(&objs, &none, Gaps::default()).rows.is_empty());
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
    fn not_searched_names_every_holder_not_read() {
        for holder in [
            "active policy",
            "virtual services",
            "virtual servers",
            "enforcement boundaries",
            "container workloads and their profiles",
            "cloud resources",
            "label mapping rules",
            "RBAC scopes",
            "firewall settings",
        ] {
            assert!(NOT_SEARCHED.contains(holder), "{holder}");
        }
    }

    fn usage_labels() -> Vec<Label> {
        load(include_str!("../../../fixtures/labels_usage.json"))
    }

    #[test]
    fn the_pces_usage_flags_settle_a_label_whatever_was_scanned() {
        // loc=eu: nothing this tool reads names it, but the PCE says a
        // pairing profile and a permission hold it. os=windows: no flag.
        let l = usage_labels();
        let objs = Objects {
            labels: Some(&l),
            ..Objects::default()
        };
        let none = used_by(&[], &[], &[], &[]);
        let u = unused(&objs, &none, Gaps::default());
        assert_eq!(names(&u.rows), ["label os=windows"]);
        assert!(u.rows[0].certain);
        // Every holder list cut short or unread: the flags still decide.
        let all_gaps = Gaps {
            rule_sets: true,
            groups: true,
            profiles: true,
            workloads: true,
        };
        let u = unused(&objs, &none, all_gaps);
        assert_eq!(names(&u.rows), ["label os=windows"]);
        assert!(u.withheld.is_empty() && u.uncertain.is_empty());
        // A label only container workloads (or only a permission) hold.
        let mut l = usage_labels();
        let flags = l[6].usage.as_mut().unwrap();
        flags.0.insert("container_workload".into(), json!(true));
        let objs = Objects {
            labels: Some(&l),
            ..Objects::default()
        };
        assert!(unused(&objs, &none, Gaps::default()).rows.is_empty());
        // Flags say unused, but a rule this tool reads names it: used.
        let l = usage_labels();
        let mut used = none.clone();
        used.hrefs.insert("/orgs/5/labels/7".into());
        let objs = Objects {
            labels: Some(&l),
            ..Objects::default()
        };
        assert!(unused(&objs, &used, Gaps::default()).rows.is_empty());
    }

    #[test]
    fn a_kind_a_cut_short_list_may_hold_is_withheld() {
        let (l, g, i, s) = (labels(), groups(), ip_lists(), services());
        let used = used_by(&rule_sets(), &g, &profiles(), &[]);
        // The workload list cut short: labels (no flags) are withheld, the
        // other kinds listed.
        let gaps = Gaps {
            workloads: true,
            ..Gaps::default()
        };
        let u = unused(&all(&l, &g, &i, &s), &used, gaps);
        assert_eq!(u.withheld, [Kind::Label]);
        assert_eq!(names(&u.rows), ["service Web"]);
        assert_eq!(gaps.text(), "workloads");
        // The rulesets cut short: every object kind.
        let gaps = Gaps {
            rule_sets: true,
            ..Gaps::default()
        };
        let u = unused(&all(&l, &g, &i, &s), &used, gaps);
        assert_eq!(
            u.withheld,
            [Kind::Label, Kind::LabelGroup, Kind::IpList, Kind::Service]
        );
        assert!(u.rows.is_empty());
        // The label groups: labels and groups.
        assert!(
            Gaps {
                groups: true,
                ..Gaps::default()
            }
            .hit(Kind::LabelGroup)
        );
        assert!(
            !Gaps {
                groups: true,
                ..Gaps::default()
            }
            .hit(Kind::IpList)
        );
        assert!(
            Gaps {
                profiles: true,
                ..Gaps::default()
            }
            .hit(Kind::Label)
        );
    }

    #[test]
    fn unread_deny_rules_make_scanned_rows_uncertain() {
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
        let (i, s) = (ip_lists(), services());
        let objs = Objects {
            ip_lists: Some(&i),
            services: Some(&s),
            ..Objects::default()
        };
        let u = unused(&objs, &used, Gaps::default());
        // Still listed, marked: the unread deny rules may name them.
        assert_eq!(names(&u.rows), ["service Web"]);
        assert!(!u.rows[0].certain);
        assert_eq!(u.uncertain, [Kind::IpList, Kind::Service]);
        // Labels the PCE's flags settle stay certain.
        let l = usage_labels();
        let objs = Objects {
            labels: Some(&l),
            ..Objects::default()
        };
        let u = unused(&objs, &used, Gaps::default());
        assert!(u.rows[0].certain);
        assert!(u.uncertain.is_empty());
    }

    #[test]
    fn fields_not_modelled_still_make_an_object_used() {
        // A label group only a custom iptables rule names; a service only
        // a rule's egress services name.
        let mut sets: Vec<serde_json::Value> =
            load(include_str!("../../../fixtures/rule_sets.json"));
        sets[0]["ip_tables_rules"] = json!([{
            "href": "/orgs/5/sec_policy/draft/rule_sets/100/ip_tables_rules/1",
            "actors": [{"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/11"}}],
            "statements": []
        }]);
        sets[0]["rules"][0]["egress_services"] =
            json!([{"href": "/orgs/5/sec_policy/draft/services/32"}]);
        let sets: Vec<RuleSet> = serde_json::from_value(json!(sets)).unwrap();
        let used = used_by(&sets, &[], &[], &[]);
        assert!(used.contains("/orgs/5/sec_policy/active/label_groups/11"));
        assert!(used.contains("/orgs/5/sec_policy/draft/services/32"));
    }

    #[test]
    fn objects_pending_deletion_are_left_out() {
        let mut i = ip_lists();
        i[1].update_type = Some("delete".into());
        let mut s = services();
        s[2].update_type = Some("delete".into());
        let mut g = groups();
        g[0].update_type = Some("delete".into());
        let none = used_by(&[], &[], &[], &[]);
        let objs = Objects {
            groups: Some(&g),
            ip_lists: Some(&i),
            services: Some(&s),
            ..Objects::default()
        };
        assert_eq!(
            names(&unused(&objs, &none, Gaps::default()).rows),
            ["service PostgreSQL"]
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
