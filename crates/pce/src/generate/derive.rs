//! Rows and a stop to rules: providers, consumers and `unscoped_consumers`
//! by the derivation tables, one rule per (target, providers, consumers,
//! flag) with the services of every flow behind it, the coverage of the
//! flows, the IP lists to create. Pure.
use super::rows::{excluded, ip_list_name, row_key, side_of};
use super::services::{Expr, express, flow_spec, fold_ports};
use super::{
    Coverage, GenActor, GenRow, GenRule, PeerKey, PeerKind, Proposal, RowKey, RuleAction, Side,
    Stop, TargetScope,
};
use crate::models::RuleSet;
use crate::resolve::{NameIndex, ResolvedLabel};
use crate::traffic::{Focus, ResolvedEndpoint, ResolvedFlow};
use crate::write::{ActorKind, ActorRef, IpListDraft, IpRangeSpec, PortSpec, ServiceRef};
use std::collections::{BTreeMap, BTreeSet};

/// A rule description's longest form; the service list is cut with `+N`.
pub const DESCRIPTION_MAX: usize = 255;

/// What `propose` reads besides the rows and the stop. Nothing here is
/// fetched: the caller passes what is loaded.
#[derive(Debug, Clone, Copy)]
pub struct GenInput<'a> {
    /// The app's flows, one per record (`TrafficState::flows_for`).
    pub flows: &'a [ResolvedFlow],
    pub focus: &'a Focus,
    /// The profile's tier key (`role`).
    pub tier_key: &'a str,
    /// The draft rulesets the rules are deduplicated against.
    pub drafts: &'a [RuleSet],
    pub idx: &'a NameIndex,
    /// `generator.ruleset_name`, for a ruleset to create.
    pub ruleset_name: &'a str,
    /// The traffic window (`24h`), for the descriptions.
    pub window: &'a str,
    /// The outbound section is on (`o`, `--outbound`).
    pub outbound: bool,
    /// The app ruleset `R` picked among several.
    pub picked: Option<&'a str>,
}

/// Row index by key; IP flows of folded peers find the folded row.
fn row_index(rows: &[GenRow]) -> BTreeMap<RowKey, usize> {
    rows.iter().enumerate().map(|(i, r)| (r.key(), i)).collect()
}

/// The rules `rows` (checkboxes and overrides as they are) need at the
/// slider's `global` stop, classified against `input.drafts`.
pub fn propose(rows: &[GenRow], global: Stop, input: &GenInput) -> Proposal {
    let mut p = derive(rows, global, input);
    p.refresh(input.drafts, input.idx, input.ruleset_name, input.picked);
    p
}

/// One rule being built.
struct Group {
    target: usize,
    providers: Vec<GenActor>,
    consumers: Vec<GenActor>,
    unscoped: bool,
    inline: BTreeSet<PortSpec>,
    objects: BTreeSet<String>,
    rows: BTreeSet<usize>,
    from: BTreeSet<String>,
    to: BTreeSet<String>,
}

fn keys_of(a: &[GenActor]) -> Vec<String> {
    let mut k: Vec<String> = a.iter().map(GenActor::sort_key).collect();
    k.sort();
    k
}

fn label_actor(l: &ResolvedLabel) -> GenActor {
    GenActor::label(&l.href)
}

/// The endpoint's labels under `keys`, in key order.
fn scope_labels(ep: &ResolvedEndpoint, keys: &[String]) -> Vec<ResolvedLabel> {
    keys.iter()
        .filter_map(|k| ep.labels.iter().find(|l| &l.key == k).cloned())
        .collect()
}

fn role(ep: &ResolvedEndpoint, tier_key: &str) -> Option<GenActor> {
    ep.labels
        .iter()
        .find(|l| l.key == tier_key)
        .map(label_actor)
}

/// The app's scope labels from the index, else from one of its flows.
fn app_scope(input: &GenInput) -> TargetScope {
    let f = input.focus;
    let from_index: Option<Vec<ResolvedLabel>> = f
        .keys
        .iter()
        .zip(&f.app)
        .map(|(k, v)| {
            input.idx.label_href(k, v).map(|h| ResolvedLabel {
                href: h.to_string(),
                key: k.clone(),
                value: v.clone(),
            })
        })
        .collect();
    let labels = from_index.unwrap_or_else(|| {
        input
            .flows
            .iter()
            .find_map(|x| match side_of(x, f)? {
                Side::Inbound => Some(scope_labels(&x.dst, &f.keys)),
                Side::Outbound => Some(scope_labels(&x.src, &f.keys)),
            })
            .unwrap_or_default()
    });
    TargetScope {
        values: f.app.clone(),
        labels,
    }
}

struct Ctx<'a> {
    input: &'a GenInput<'a>,
    scopes: Vec<TargetScope>,
    ip_lists: Vec<IpListDraft>,
}

impl Ctx<'_> {
    /// The target index of a peer's scope, added on first use.
    fn peer_target(&mut self, values: &[String], ep: &ResolvedEndpoint) -> usize {
        if let Some(i) = self.scopes.iter().position(|s| s.values == values) {
            return i;
        }
        self.scopes.push(TargetScope {
            values: values.to_vec(),
            labels: scope_labels(ep, &self.input.focus.keys),
        });
        self.scopes.len() - 1
    }

    /// The IP list actor of an IP row: the existing list, else the new one.
    fn ip_actor(&mut self, ip: &str, list: Option<&str>) -> GenActor {
        if let Some(h) = list.and_then(|n| self.input.idx.ip_list_href(n)) {
            return GenActor::Ref(ActorRef::new(ActorKind::IpList, h));
        }
        let name = ip_list_name(&self.input.focus.app, ip);
        if let Some(i) = self.ip_lists.iter().position(|d| d.name == name) {
            return GenActor::NewIpList(i);
        }
        let Ok(range) = IpRangeSpec::parse(ip, false) else {
            // `rows` makes such a row uncheckable; never reached.
            return GenActor::NewIpList(usize::MAX);
        };
        self.ip_lists.push(IpListDraft {
            name,
            description: None,
            ranges: vec![range],
            fqdns: Vec::new(),
        });
        GenActor::NewIpList(self.ip_lists.len() - 1)
    }

    /// Target, providers, consumers and `unscoped_consumers` of one flow of
    /// `row` at `stop`, by the derivation tables; `None` for a row that
    /// proposes nothing.
    fn actors(
        &mut self,
        f: &ResolvedFlow,
        row: &GenRow,
        stop: Stop,
    ) -> Option<(usize, Vec<GenActor>, Vec<GenActor>, bool)> {
        let keys = self.input.focus.keys.clone();
        let tier = self.input.tier_key;
        let by_role = stop.is_role();
        // Providers sit in the target's scope: `ams` there, or the role.
        let provider = |ep: &ResolvedEndpoint| {
            vec![
                by_role
                    .then(|| role(ep, tier))
                    .flatten()
                    .unwrap_or_else(GenActor::all),
            ]
        };
        // A labelled side: its scope labels, plus its role at role stops.
        let side = |ep: &ResolvedEndpoint| {
            let mut v: Vec<GenActor> = scope_labels(ep, &keys).iter().map(label_actor).collect();
            if let Some(r) = by_role.then(|| role(ep, tier)).flatten() {
                v.push(r);
            }
            v
        };
        Some(match (row.side, &row.peer) {
            (Side::Inbound, PeerKind::Intra) => {
                let cons = by_role
                    .then(|| role(&f.src, tier))
                    .flatten()
                    .unwrap_or_else(GenActor::all);
                (0, provider(&f.dst), vec![cons], false)
            }
            (Side::Inbound, PeerKind::App(_)) => (0, provider(&f.dst), side(&f.src), true),
            (Side::Inbound, PeerKind::Ip { ip, list }) => {
                let cons = self.ip_actor(ip, list.as_deref());
                (0, provider(&f.dst), vec![cons], false)
            }
            (Side::Inbound, PeerKind::Workload { href, .. }) => (
                0,
                provider(&f.dst),
                vec![GenActor::Ref(ActorRef::new(ActorKind::Workload, href))],
                true,
            ),
            (Side::Outbound, PeerKind::App(values)) => {
                let t = self.peer_target(values, &f.dst);
                (t, provider(&f.dst), side(&f.src), true)
            }
            (Side::Outbound, PeerKind::Ip { ip, list }) => {
                let prov = self.ip_actor(ip, list.as_deref());
                let cons = by_role
                    .then(|| role(&f.src, tier))
                    .flatten()
                    .unwrap_or_else(GenActor::all);
                (0, vec![prov], vec![cons], false)
            }
            _ => return None,
        })
    }
}

/// The names a description gives a row's two ends: `(from, to)`.
fn ends(row: &GenRow, app: &str) -> (String, String) {
    let peer = match &row.peer {
        PeerKind::Intra => app.to_string(),
        _ => row.peer_text(),
    };
    match row.side {
        Side::Inbound => (peer, app.to_string()),
        Side::Outbound => (app.to_string(), peer),
    }
}

/// `items` joined with `, `, cut with `+N` once it would pass `budget`
/// characters (at least one item stays).
fn capped(items: &[String], budget: usize) -> String {
    for keep in (1..=items.len()).rev() {
        let mut s = items[..keep].join(", ");
        if keep < items.len() {
            s.push_str(&format!(" +{}", items.len() - keep));
        }
        if s.chars().count() <= budget || keep == 1 {
            return s;
        }
    }
    String::new()
}

fn service_text(s: &ServiceRef, all: Option<&str>, idx: &NameIndex) -> String {
    match s {
        ServiceRef::Port(p) => p.display(),
        ServiceRef::Href(h) if Some(h.as_str()) == all => "all services".into(),
        ServiceRef::Href(h) => idx.service_name(h).unwrap_or(h).to_string(),
    }
}

/// `generated: web/prod → shop/prod on 443/tcp, from 24h of traffic
/// (illumio-tui)`, at most [`DESCRIPTION_MAX`] characters.
fn description(from: &[String], to: &[String], services: &[String], window: &str) -> String {
    let head = |f: &str, t: &str, s: &str| {
        format!("generated: {f} → {t} on {s}, from {window} of traffic (illumio-tui)")
    };
    let f = capped(from, 60);
    let t = capped(to, 60);
    let fixed = head(&f, &t, "").chars().count();
    let s = capped(services, DESCRIPTION_MAX.saturating_sub(fixed));
    head(&f, &t, &s)
}

fn derive(rows: &[GenRow], global: Stop, input: &GenInput) -> Proposal {
    let focus = input.focus;
    let app = focus.app_text();
    let index = row_index(rows);
    let all_services = input.idx.all_services_href().map(str::to_string);
    let mut ctx = Ctx {
        input,
        scopes: vec![app_scope(input)],
        ip_lists: Vec::new(),
    };
    let mut groups: BTreeMap<(usize, Vec<String>, Vec<String>, bool), Group> = BTreeMap::new();
    let mut cov = Coverage::default();
    let in_force = |s: Side| s == Side::Inbound || input.outbound;
    for f in input.flows {
        if excluded(f) {
            if side_of(f, focus).is_some_and(in_force) {
                cov.broadcast_multicast += 1;
            }
            continue;
        }
        let Some(key) = row_key(f, focus) else {
            continue;
        };
        if !in_force(key.side) {
            continue;
        }
        let folded = RowKey {
            side: key.side,
            peer: PeerKey::Folded,
        };
        let Some(&ri) = index.get(&key).or_else(|| index.get(&folded)) else {
            continue;
        };
        let row = &rows[ri];
        if matches!(row.peer, PeerKind::FoldedIps(_)) {
            cov.folded_ips += 1;
            continue;
        }
        let mut stop = row.effective(global);
        if stop.is_all() && all_services.is_none() {
            stop = stop.ports();
        }
        let svc = match (&all_services, stop.is_all()) {
            (Some(h), true) if row.proposed() => Expr::Object(h.clone()),
            _ => express(&flow_spec(f), input.idx),
        };
        // A service nothing expresses is that, checked or not.
        if let Expr::Unsupported(_) = svc {
            cov.unsupported_proto += 1;
            continue;
        }
        if !row.proposed() {
            cov.unchecked += 1;
            continue;
        }
        let Some((target, providers, consumers, unscoped)) = ctx.actors(f, row, stop) else {
            cov.unchecked += 1;
            continue;
        };
        let k = (target, keys_of(&providers), keys_of(&consumers), unscoped);
        let g = groups.entry(k).or_insert_with(|| Group {
            target,
            providers,
            consumers,
            unscoped,
            inline: BTreeSet::new(),
            objects: BTreeSet::new(),
            rows: BTreeSet::new(),
            from: BTreeSet::new(),
            to: BTreeSet::new(),
        });
        match svc {
            Expr::Inline(p) => {
                g.inline.insert(p);
            }
            Expr::Object(h) => {
                g.objects.insert(h);
            }
            Expr::Unsupported(_) => {}
        }
        g.rows.insert(ri);
        let (from, to) = ends(row, &app);
        g.from.insert(from);
        g.to.insert(to);
        cov.covered += 1;
    }
    cov.uncovered =
        cov.unchecked + cov.folded_ips + cov.unsupported_proto + cov.broadcast_multicast;
    let mut groups: Vec<Group> = groups.into_values().collect();
    // The app's ruleset first, then by the first row behind the rule.
    groups.sort_by_key(|g| (g.target, g.rows.first().copied()));
    let rules = groups
        .into_iter()
        .map(|g| {
            let services: Vec<ServiceRef> = match &all_services {
                Some(h) if g.objects.contains(h) => vec![ServiceRef::Href(h.clone())],
                _ => fold_ports(&g.inline)
                    .into_iter()
                    .map(ServiceRef::Port)
                    .chain(g.objects.iter().cloned().map(ServiceRef::Href))
                    .collect(),
            };
            let names: Vec<String> = services
                .iter()
                .map(|s| service_text(s, all_services.as_deref(), input.idx))
                .collect();
            let from: Vec<String> = g.from.into_iter().collect();
            let to: Vec<String> = g.to.into_iter().collect();
            GenRule {
                target: g.target,
                providers: g.providers,
                consumers: g.consumers,
                unscoped_consumers: g.unscoped,
                description: description(&from, &to, &names, input.window),
                services,
                rows: g.rows.into_iter().collect(),
                action: RuleAction::Create,
            }
        })
        .collect();
    Proposal {
        scopes: ctx.scopes,
        targets: Vec::new(),
        ip_lists: ctx.ip_lists,
        rules,
        coverage: cov,
    }
}

#[cfg(test)]
mod tests {
    use super::super::rows::rows;
    use super::super::rows::tests::{intra_on, shop, to_ip};
    use super::super::{DEFAULT_RULESET_NAME, Target};
    use super::*;
    use crate::traffic::resolved::tests::{rich_flows, rich_index};

    fn sets() -> Vec<RuleSet> {
        serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap()
    }

    /// `propose` on the rich fixture for shop/prod, the outbound section
    /// on or off, against the fixture rulesets.
    fn run(
        flows: &[ResolvedFlow],
        stop: Stop,
        outbound: bool,
        edit: impl Fn(&mut Vec<GenRow>),
    ) -> (Vec<GenRow>, Proposal) {
        let idx = rich_index();
        let focus = shop();
        let mut r = rows(flows, &focus, "role", &idx);
        edit(&mut r);
        let drafts = sets();
        let input = GenInput {
            flows,
            focus: &focus,
            tier_key: "role",
            drafts: &drafts,
            idx: &idx,
            ruleset_name: DEFAULT_RULESET_NAME,
            window: "24h",
            outbound,
            picked: None,
        };
        let p = propose(&r, stop, &input);
        (r, p)
    }

    /// Each rule as `providers -> consumers unscoped services`, by href.
    fn shape(p: &Proposal) -> Vec<String> {
        let idx = rich_index();
        p.rules
            .iter()
            .map(|r| {
                let side = |a: &[GenActor]| {
                    a.iter()
                        .map(|x| x.text(p, &idx))
                        .collect::<Vec<_>>()
                        .join("+")
                };
                let svc: Vec<String> = r.services.iter().map(ServiceRef::display).collect();
                format!(
                    "t{} {} <- {}{} {}",
                    r.target,
                    side(&r.providers),
                    side(&r.consumers),
                    if r.unscoped_consumers {
                        " unscoped"
                    } else {
                        ""
                    },
                    svc.join(",")
                )
            })
            .collect()
    }

    #[test]
    fn app_ports_proposes_intra_and_the_same_env_peer_into_the_app_ruleset() {
        let (_, p) = run(&rich_flows(), Stop::AppPorts, false, |_| {});
        assert_eq!(
            shape(&p),
            vec![
                "t0 all workloads <- all workloads 5432/tcp",
                "t0 all workloads <- app=hr+env=prod unscoped 5432/tcp",
            ]
        );
        assert_eq!(
            p.coverage,
            Coverage {
                covered: 5,
                uncovered: 3,
                unchecked: 3,
                ..Coverage::default()
            }
        );
        assert_eq!(p.rules[0].rows, vec![0]);
        assert_eq!(
            p.rules[1].description,
            "generated: hr/prod → shop/prod on 5432/tcp, from 24h of traffic (illumio-tui)"
        );
        assert!(p.ip_lists.is_empty());
    }

    #[test]
    fn role_ports_splits_rows_by_role_pair() {
        let (_, p) = run(&rich_flows(), Stop::RolePorts, false, |r| {
            r[3].checked = true
        });
        assert_eq!(
            shape(&p),
            vec![
                "t0 role=db <- role=web 5432/tcp",
                "t0 role=db <- app=hr+env=prod+role=web unscoped 5432/tcp",
                "t0 role=web <- ipl:Corp 443/tcp",
                "t0 role=db <- ipl:Corp 5432/tcp",
            ]
        );
    }

    #[test]
    fn all_stops_take_the_all_services_object() {
        let (_, p) = run(&rich_flows(), Stop::AppAll, false, |_| {});
        assert!(
            p.rules.iter().all(|r| r.services
                == vec![ServiceRef::Href(
                    "/orgs/5/sec_policy/draft/services/30".into()
                )]),
            "{:?}",
            shape(&p)
        );
        assert!(p.rules[0].description.contains("on all services"));
    }

    #[test]
    fn a_row_override_beats_the_slider() {
        let (_, p) = run(&rich_flows(), Stop::AppPorts, false, |r| {
            r[0].stop = Some(Stop::RolePorts)
        });
        assert_eq!(shape(&p)[0], "t0 role=db <- role=web 5432/tcp");
        assert_eq!(
            shape(&p)[1],
            "t0 all workloads <- app=hr+env=prod unscoped 5432/tcp"
        );
    }

    #[test]
    fn rows_with_the_same_actors_merge_into_one_rule() {
        let mut f = rich_flows();
        let mut redis = f[12].clone(); // hr/prod -> shop/prod, another port
        redis.port = Some(6379);
        f.push(redis);
        let mut other = f[17].clone(); // a second Corp address -> shop web
        other.src.ip = Some("10.1.4.10".into());
        f.push(other);
        let (r, p) = run(&f, Stop::AppPorts, false, |r| {
            for row in r.iter_mut() {
                if matches!(row.peer, PeerKind::Ip { .. }) {
                    row.checked = true;
                }
            }
        });
        assert_eq!(r[3].peer_text(), "10.1.4.9");
        assert_eq!(r[4].peer_text(), "10.1.4.10");
        assert_eq!(
            shape(&p),
            vec![
                "t0 all workloads <- all workloads 5432/tcp",
                "t0 all workloads <- app=hr+env=prod unscoped 5432/tcp,6379/tcp",
                "t0 all workloads <- ipl:Corp 443/tcp,5432/tcp",
            ]
        );
        assert_eq!(p.rules[2].rows, vec![3, 4], "two IP peers, one rule");
        assert_eq!(
            p.rules[2].description,
            "generated: 10.1.4.10, 10.1.4.9 → shop/prod on 443/tcp, 5432/tcp, from 24h of traffic (illumio-tui)"
        );
    }

    #[test]
    fn outbound_rules_go_into_the_peer_ruleset_or_name_a_new_ip_list() {
        let (_, p) = run(&rich_flows(), Stop::AppPorts, true, |r| r[5].checked = true);
        assert_eq!(
            shape(&p)[2..],
            [
                "t0 new ipl:shop-prod peer 203.0.113.7 <- all workloads 443/tcp".to_string(),
                "t1 all workloads <- app=shop+env=prod unscoped 443/tcp".to_string(),
            ]
        );
        assert_eq!(p.scopes[1].text(), "crm/prod");
        let Target::Create(d) = &p.targets[1] else {
            panic!("no ruleset covers crm/prod: {:?}", p.targets[1])
        };
        assert_eq!(d.name, "RF-crm-prod");
        assert_eq!(p.ip_lists.len(), 1);
        assert_eq!(p.ip_lists[0].name, "shop-prod peer 203.0.113.7");
        assert_eq!(p.ip_lists[0].ranges[0].from_ip, "203.0.113.7");
        assert_eq!(p.coverage.covered, 7, "5 inbound, 2 outbound");
        // The batch creates the IP list (step 0), then the ruleset (step 1).
        assert_eq!(p.ruleset_step(1), Some(1));
        assert_eq!(p.ruleset_step(0), None, "the app's ruleset exists");
        let new_list = p
            .rules
            .iter()
            .find(|r| r.providers[0] == GenActor::NewIpList(0))
            .unwrap();
        assert_eq!(
            new_list.to_draft(&p).providers[0].href.as_deref(),
            Some("{step:0}")
        );
        let crm = p.rules.iter().find(|r| r.target == 1).unwrap();
        assert_eq!(crm.ruleset_ref(&p).as_deref(), Some("{step:1}"));
        assert_eq!(
            crm.body(&p)["consumers"],
            serde_json::json!([{"label": {"href": "/orgs/5/labels/3"}}, {"label": {"href": "/orgs/5/labels/4"}}])
        );
    }

    #[test]
    fn coverage_counts_every_kept_flow_once_by_reason() {
        let mut f = rich_flows();
        f.push(to_ip(22, "224.0.0.251")); // outbound multicast
        let mut icmp = intra_on(0);
        icmp.port = None;
        icmp.proto = "icmp".into();
        f.push(icmp); // intra, no ICMP service in the fixture
        let (_, p) = run(&f, Stop::AppPorts, true, |_| {});
        assert_eq!(
            p.coverage,
            Coverage {
                covered: 6,
                uncovered: 6,
                unchecked: 4,
                folded_ips: 0,
                unsupported_proto: 1,
                broadcast_multicast: 1,
            }
        );
        let (_, p) = run(&f, Stop::AppPorts, false, |_| {});
        assert_eq!(
            p.coverage.broadcast_multicast, 0,
            "outbound off: not counted"
        );
    }

    #[test]
    fn a_long_service_list_is_cut_in_the_description() {
        let mut f = rich_flows();
        f.extend((0..60u16).map(|i| intra_on(10_000 + 2 * i)));
        let (_, p) = run(&f, Stop::AppPorts, false, |_| {});
        let d = &p.rules[0].description;
        assert!(d.chars().count() <= DESCRIPTION_MAX, "{d}");
        assert!(d.contains(" +"), "{d}");
        assert!(d.ends_with("from 24h of traffic (illumio-tui)"), "{d}");
    }
}
