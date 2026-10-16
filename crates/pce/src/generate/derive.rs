//! Rows and a stop to rules: providers, consumers and `unscoped_consumers`
//! by the derivation tables, one rule per (target, providers, consumers,
//! flag) with the services of every flow behind it, rules to create that
//! differ in one side only merged when the union means exactly both, the
//! coverage of the flows, the IP lists to create (none for an IP peer in
//! an existing list: its list row names that list). Pure.
use super::rows::{excluded, ip_list_name, row_key, side_of};
use super::services::{Expr, express, flow_spec, fold_ports};
use super::{
    Coverage, GenActor, GenRow, GenRule, PeerKey, PeerKind, Proposal, RowKey, RuleAction, SeenFlow,
    Side, Stop, TargetScope,
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

/// The list row each IP peer went to, by side and IP: a flow follows the
/// rows as `rows` grouped them.
fn list_rows(rows: &[GenRow]) -> BTreeMap<(Side, &str), usize> {
    let mut out = BTreeMap::new();
    for (i, r) in rows.iter().enumerate() {
        if let PeerKind::IpList { ips, .. } = &r.peer {
            for ip in ips {
                out.insert((r.side, ip.as_str()), i);
            }
        }
    }
    out
}

/// The rules `rows` (checkboxes and overrides as they are) need at the
/// slider's `global` stop, classified against `input.drafts`.
pub fn propose(rows: &[GenRow], global: Stop, input: &GenInput) -> Proposal {
    let mut p = derive(rows, global, input);
    p.refresh(input.drafts, input.idx, input.ruleset_name, input.picked);
    merge(&mut p, input.idx);
    for r in &mut p.rules {
        r.description = describe(r, rows, input);
    }
    p.refresh(input.drafts, input.idx, input.ruleset_name, input.picked);
    p
}

/// A side as Illumio reads it: its labels by key (AND across keys, OR
/// within one), ORed with its other actors (IP lists, workloads). `None`
/// for `ams`, an exclusion or a label the index does not know.
#[allow(clippy::type_complexity)]
fn split(
    side: &[GenActor],
    idx: &NameIndex,
) -> Option<(BTreeMap<String, BTreeSet<String>>, BTreeSet<String>)> {
    let mut labels: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut others = BTreeSet::new();
    for a in side {
        match a {
            GenActor::Ref(r) if r.exclusion => return None,
            GenActor::Ref(r) if r.kind == ActorKind::Label => {
                let h = r.href.as_deref()?;
                labels
                    .entry(idx.label(h)?.key.clone())
                    .or_default()
                    .insert(h.to_string());
            }
            GenActor::Ref(r) if matches!(r.kind, ActorKind::IpList | ActorKind::Workload) => {
                others.insert(a.sort_key());
            }
            GenActor::Ref(_) => return None,
            GenActor::NewIpList(_) => {
                others.insert(a.sort_key());
            }
        }
    }
    Some((labels, others))
}

/// Whether one side holding both sides' actors means exactly either: the
/// label parts agree on every key but at most one (`role=web` and
/// `role=db` become `role=web, role=db`), or one side has no labels.
fn union_exact(a: &[GenActor], b: &[GenActor], idx: &NameIndex) -> bool {
    let (Some((la, _)), Some((lb, _))) = (split(a, idx), split(b, idx)) else {
        return false;
    };
    if la.is_empty() || lb.is_empty() {
        return true;
    }
    la.keys().eq(lb.keys()) && la.iter().filter(|(k, v)| lb.get(*k) != Some(v)).count() <= 1
}

fn union_into(into: &mut Vec<GenActor>, from: &[GenActor]) {
    for a in from {
        if !into.iter().any(|x| x.sort_key() == a.sort_key()) {
            into.push(a.clone());
        }
    }
}

/// Rules to create in one ruleset with the same services and flag, the
/// same actors on one side and on the other sides whose union means
/// exactly both (see [`union_exact`]), become one: that union, the rows
/// and the flows of both. Rules that exist or gain services stay apart.
fn merge(p: &mut Proposal, idx: &NameIndex) {
    let mut i = 0;
    while i < p.rules.len() {
        let mut j = i + 1;
        while j < p.rules.len() {
            let (a, b) = (&p.rules[i], &p.rules[j]);
            let alike = a.action == RuleAction::Create
                && b.action == RuleAction::Create
                && a.target == b.target
                && a.unscoped_consumers == b.unscoped_consumers
                && a.services == b.services;
            let same_p = keys_of(&a.providers) == keys_of(&b.providers);
            let same_c = keys_of(&a.consumers) == keys_of(&b.consumers);
            let side = match () {
                _ if !alike => None,
                _ if same_p && union_exact(&a.consumers, &b.consumers, idx) => Some(false),
                _ if same_c && union_exact(&a.providers, &b.providers, idx) => Some(true),
                _ => None,
            };
            let Some(providers) = side else {
                j += 1;
                continue;
            };
            let b = p.rules.remove(j);
            let a = &mut p.rules[i];
            if providers {
                union_into(&mut a.providers, &b.providers);
            } else {
                union_into(&mut a.consumers, &b.consumers);
            }
            a.rows.extend(b.rows);
            a.rows.sort_unstable();
            a.rows.dedup();
            a.flows += b.flows;
            for (f, n) in b.seen {
                *a.seen.entry(f).or_default() += n;
            }
            for (l, ips) in b.ips {
                a.ips.entry(l).or_default().extend(ips);
            }
            // Another rule may merge with the union now: look again.
            j = i + 1;
        }
        i += 1;
    }
}

/// The rule's description from the rows behind it and its services.
fn describe(r: &GenRule, rows: &[GenRow], input: &GenInput) -> String {
    let app = input.focus.app_text();
    let all = input.idx.all_services_href();
    let mut from = BTreeSet::new();
    let mut to = BTreeSet::new();
    for &ri in &r.rows {
        let (f, t) = ends(&rows[ri], &app);
        from.insert(f);
        to.insert(t);
    }
    let names: Vec<String> = r
        .services
        .iter()
        .map(|s| service_text(s, all, input.idx))
        .collect();
    let from: Vec<String> = from.into_iter().collect();
    let to: Vec<String> = to.into_iter().collect();
    description(&from, &to, &names, input.window)
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
    flows: u64,
    seen: BTreeMap<SeenFlow, u64>,
    ips: BTreeMap<String, BTreeSet<String>>,
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
    scope_of(input.flows, input.focus, input.idx)
}

/// The draft ruleset the app's rules would go into (the proposal's target
/// 0) when it exists: by `template` and the `R` pick (`picked`), as
/// `propose` resolves it. `None` for a ruleset to create, several or none.
pub fn app_ruleset<'a>(
    flows: &[ResolvedFlow],
    focus: &Focus,
    idx: &NameIndex,
    drafts: &'a [RuleSet],
    template: &str,
    picked: Option<&str>,
) -> Option<&'a RuleSet> {
    let scope = scope_of(flows, focus, idx);
    match super::target::resolve(&scope, drafts, idx, template, picked) {
        super::Target::Existing { href, .. } => drafts.iter().find(|rs| rs.href == href),
        _ => None,
    }
}

fn scope_of(flows: &[ResolvedFlow], f: &Focus, idx: &NameIndex) -> TargetScope {
    let input = (flows, idx);
    let from_index: Option<Vec<ResolvedLabel>> = f
        .keys
        .iter()
        .zip(&f.app)
        .map(|(k, v)| {
            input.1.label_href(k, v).map(|h| ResolvedLabel {
                href: h.to_string(),
                key: k.clone(),
                value: v.clone(),
            })
        })
        .collect();
    let labels = from_index.unwrap_or_else(|| {
        input
            .0
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

    /// The new IP list of an IP row (in no existing list).
    fn ip_actor(&mut self, ip: &str) -> GenActor {
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
            (Side::Inbound, PeerKind::Ip { ip }) => {
                let cons = self.ip_actor(ip);
                (0, provider(&f.dst), vec![cons], false)
            }
            (Side::Inbound, PeerKind::IpList { href, .. }) => {
                let cons = GenActor::Ref(ActorRef::new(ActorKind::IpList, href));
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
            (Side::Outbound, PeerKind::Ip { .. } | PeerKind::IpList { .. }) => {
                let prov = match &row.peer {
                    PeerKind::IpList { href, .. } => {
                        GenActor::Ref(ActorRef::new(ActorKind::IpList, href))
                    }
                    PeerKind::Ip { ip } => self.ip_actor(ip),
                    _ => return None,
                };
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
        PeerKind::IpList { name, .. } => format!("IP list {name}"),
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
    // One long name alone can still pass the limit.
    let full = head(&f, &t, &s);
    if full.chars().count() <= DESCRIPTION_MAX {
        return full;
    }
    let mut cut: String = full.chars().take(DESCRIPTION_MAX - 1).collect();
    cut.push('…');
    cut
}

/// The rules before merging.
fn derive(rows: &[GenRow], global: Stop, input: &GenInput) -> Proposal {
    let focus = input.focus;
    let index = row_index(rows);
    let by_ip = list_rows(rows);
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
        let listed = match &key.peer {
            PeerKey::Ip(ip) => by_ip.get(&(key.side, ip.as_str())),
            _ => None,
        };
        let Some(&ri) = listed
            .or_else(|| index.get(&key))
            .or_else(|| index.get(&folded))
        else {
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
            flows: 0,
            seen: BTreeMap::new(),
            ips: BTreeMap::new(),
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
        if let PeerKind::IpList { name, .. } = &row.peer {
            let peer = match key.side {
                Side::Inbound => &f.src,
                Side::Outbound => &f.dst,
            };
            let ip = peer.ip.clone().unwrap_or_else(|| "?".into());
            g.ips.entry(name.clone()).or_default().insert(ip);
        }
        g.rows.insert(ri);
        g.flows += 1;
        *g.seen.entry(SeenFlow::of(f)).or_default() += 1;
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
            GenRule {
                target: g.target,
                providers: g.providers,
                consumers: g.consumers,
                unscoped_consumers: g.unscoped,
                description: String::new(),
                services,
                rows: g.rows.into_iter().collect(),
                action: RuleAction::Create,
                flows: g.flows,
                denied_flows: 0,
                denied_by: Vec::new(),
                seen: g.seen,
                ips: g.ips,
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
        run_in(sets(), flows, stop, outbound, edit)
    }

    /// `run` against these draft rulesets.
    fn run_in(
        drafts: Vec<RuleSet>,
        flows: &[ResolvedFlow],
        stop: Stop,
        outbound: bool,
        edit: impl Fn(&mut Vec<GenRow>),
    ) -> (Vec<GenRow>, Proposal) {
        let idx = rich_index();
        let focus = shop();
        let target = app_ruleset(flows, &focus, &idx, &drafts, DEFAULT_RULESET_NAME, None);
        let mut r = rows(flows, &focus, "role", &idx, target);
        edit(&mut r);
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
                if matches!(row.peer, PeerKind::IpList { .. }) {
                    row.checked = true;
                }
            }
        });
        assert_eq!(r[3].peer_text(), "IP list Corp · 2 IPs", "one row per list");
        assert_eq!(
            shape(&p),
            vec![
                "t0 all workloads <- all workloads 5432/tcp",
                "t0 all workloads <- app=hr+env=prod unscoped 5432/tcp,6379/tcp",
                "t0 all workloads <- ipl:Corp 443/tcp,5432/tcp",
            ]
        );
        assert_eq!(p.rules[2].rows, vec![3], "two IP peers, one row, one rule");
        assert!(p.ip_lists.is_empty(), "an existing list: none created");
        assert_eq!(
            p.rules[2].ips["Corp"].iter().collect::<Vec<_>>(),
            ["10.1.4.10", "10.1.4.9"]
        );
        assert_eq!(
            p.rules[2].description,
            "generated: IP list Corp → shop/prod on 443/tcp, 5432/tcp, from 24h of traffic (illumio-tui)"
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
                ..Coverage::default()
            }
        );
        let (_, p) = run(&f, Stop::AppPorts, false, |_| {});
        assert_eq!(
            p.coverage.broadcast_multicast, 0,
            "outbound off: not counted"
        );
    }

    /// Flow 0 (shop web to shop db) on `port`, to a shop web workload.
    fn web_to_web(port: u16) -> ResolvedFlow {
        let mut f = intra_on(port);
        for l in &mut f.dst.labels {
            if l.key == "role" {
                l.href = "/orgs/5/labels/1".into();
                l.value = "web".into();
            }
        }
        f
    }

    #[test]
    fn rules_differing_in_one_role_merge_into_one_and_existing_ones_stay_apart() {
        let mut f = rich_flows();
        f.push(web_to_web(5432));
        let mut empty = sets();
        empty[0].rules.clear();
        let (_, p) = run_in(empty, &f, Stop::RolePorts, false, |_| {});
        assert_eq!(
            shape(&p)[0],
            "t0 role=web+role=db <- role=web 5432/tcp",
            "{:?}",
            shape(&p)
        );
        assert_eq!(p.rules[0].action, RuleAction::Create);
        assert_eq!(p.rules[0].flows, 5, "4 web to db, 1 web to web");
        assert_eq!(p.rules[0].rows, vec![0]);
        // sec_rules/1 (db <- web) covers the db part: it exists, apart.
        let (_, p) = run(&f, Stop::RolePorts, false, |_| {});
        let s = shape(&p);
        assert_eq!(s[0], "t0 role=web <- role=web 5432/tcp", "{s:?}");
        assert_eq!(p.rules[0].action, RuleAction::Create);
        assert_eq!(s[1], "t0 role=db <- role=web 5432/tcp", "{s:?}");
        assert!(matches!(p.rules[1].action, RuleAction::Exists { .. }));
    }

    #[test]
    fn only_unions_that_mean_exactly_both_sides_merge() {
        let idx = rich_index();
        let l = |n: u32| GenActor::label(&format!("/orgs/5/labels/{n}"));
        let ipl = GenActor::Ref(ActorRef::new(
            ActorKind::IpList,
            "/orgs/5/sec_policy/draft/ip_lists/21",
        ));
        // app=shop role=web | app=shop role=db: one key apart.
        assert!(union_exact(&[l(3), l(1)], &[l(3), l(2)], &idx));
        // role=web | app=shop: the union would read role=web AND app=shop.
        assert!(!union_exact(&[l(1)], &[l(3)], &idx));
        assert!(!union_exact(&[l(3), l(1)], &[l(3)], &idx));
        // app=shop env=prod | app=hr env=dev-like: two keys apart.
        assert!(!union_exact(&[l(3), l(1)], &[l(103), l(2)], &idx));
        // An IP list ORs with any label combination; ams never merges.
        assert!(union_exact(std::slice::from_ref(&ipl), &[l(3), l(1)], &idx));
        assert!(!union_exact(&[GenActor::all()], &[l(1)], &idx));
        assert!(!union_exact(&[ipl], &[GenActor::all()], &idx));
    }

    /// The fixture rulesets with Shop-Prod scoped by `app=shop` only.
    fn shop_wide() -> Vec<RuleSet> {
        let mut d = sets();
        d[0].scopes = vec![vec![
            serde_json::from_value(serde_json::json!({"label": {"href": "/orgs/5/labels/3"}}))
                .unwrap(),
        ]];
        d
    }

    #[test]
    fn a_ruleset_wider_than_the_app_gets_the_app_labels_not_ams() {
        let (_, p) = run_in(shop_wide(), &rich_flows(), Stop::AppPorts, false, |_| {});
        assert_eq!(p.targets[0].name(), "Shop-Prod");
        assert_eq!(
            shape(&p),
            vec![
                "t0 app=shop+env=prod <- app=shop+env=prod 5432/tcp",
                "t0 app=shop+env=prod <- app=hr+env=prod unscoped 5432/tcp",
            ]
        );
        let (_, p) = run_in(shop_wide(), &rich_flows(), Stop::RolePorts, false, |_| {});
        assert_eq!(
            shape(&p)[0],
            "t0 app=shop+env=prod+role=db <- app=shop+env=prod+role=web 5432/tcp"
        );
        assert_eq!(
            p.rules[0].action,
            RuleAction::Exists {
                rule_href: "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1".into()
            },
            "role=db <- role=web over all of shop covers shop/prod"
        );
        // Two scopes: as wide.
        let mut two = sets();
        let dev = two[0].scopes[0].clone();
        two[0].scopes.push(dev[..1].to_vec());
        let (_, p) = run_in(two, &rich_flows(), Stop::AppPorts, false, |_| {});
        assert_eq!(
            shape(&p)[0],
            "t0 app=shop+env=prod <- app=shop+env=prod 5432/tcp"
        );
    }

    #[test]
    fn flows_of_a_rule_without_a_ruleset_are_not_covered() {
        let mut d = sets();
        let mut twin = d[0].clone();
        twin.href = "/orgs/5/sec_policy/draft/rule_sets/102".into();
        twin.name = "Shop-Prod-2".into();
        d.push(twin);
        let (_, p) = run_in(d, &rich_flows(), Stop::AppPorts, false, |_| {});
        assert_eq!(p.targets[0].action(), "ambiguous");
        assert_eq!(
            p.coverage,
            Coverage {
                covered: 0,
                uncovered: 8,
                unchecked: 3,
                no_target: 5,
                ..Coverage::default()
            }
        );
    }

    #[test]
    fn an_override_deny_in_the_target_uncovers_its_flows() {
        let mut f = rich_flows();
        f.push(intra_on(22)); // web -> db: deny_rules/8 (db, ams, 22/tcp, override)
        f.push(web_to_web(22)); // web -> web: not a db provider
        let (_, p) = run(&f, Stop::AppPorts, false, |_| {});
        let r = p
            .rules
            .iter()
            .find(|r| r.services.iter().any(|s| s.display() == "22/tcp"))
            .unwrap();
        assert_eq!(r.denied_flows, 1);
        assert_eq!(
            r.denied_by,
            vec!["/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8".to_string()]
        );
        assert_eq!(p.coverage.override_deny, 1);
        assert_eq!(p.coverage.covered, 6);
    }

    fn deny_22(json: serde_json::Value) -> Vec<RuleSet> {
        let mut d = sets();
        let deny = &mut d[0].deny_rules[1];
        for (k, v) in json.as_object().unwrap() {
            match k.as_str() {
                "providers" => deny.providers = serde_json::from_value(v.clone()).unwrap(),
                "consumers" => deny.consumers = serde_json::from_value(v.clone()).unwrap(),
                _ => unreachable!(),
            }
        }
        d
    }

    fn denied(p: &Proposal) -> u64 {
        p.rules.iter().map(|r| r.denied_flows).sum()
    }

    #[test]
    fn a_list_row_keeps_the_override_deny_of_a_deny_naming_its_list() {
        let mut f = rich_flows();
        let mut ssh = f[17].clone(); // Corp 10.1.4.9 -> shop web
        ssh.port = Some(22);
        f.push(ssh);
        let d = deny_22(serde_json::json!({
            "providers": [{"actors": "ams"}],
            "consumers": [{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}]
        }));
        let (r, p) = run_in(d, &f, Stop::AppPorts, false, |r| r[3].checked = true);
        assert!(matches!(r[3].peer, PeerKind::IpList { .. }));
        let corp = p.rules.iter().find(|x| x.rows == vec![3]).unwrap();
        assert_eq!(corp.denied_flows, 1, "the 22/tcp flow from Corp");
        assert_eq!(p.coverage.override_deny, 1);
    }

    #[test]
    fn an_outbound_list_row_names_its_list_as_provider_and_creates_none() {
        let mut f = rich_flows();
        let mut x = to_ip(22, "10.1.9.9");
        x.dst.ip_lists = vec!["Corp".into()];
        f.push(x);
        let (r, p) = run(&f, Stop::AppPorts, true, |r| {
            for row in r.iter_mut() {
                if matches!(row.peer, PeerKind::IpList { .. }) && row.side == Side::Outbound {
                    row.checked = true;
                }
            }
        });
        let out = r
            .iter()
            .position(|x| x.side == Side::Outbound && x.peer_text() == "IP list Corp · 1 IP")
            .unwrap();
        let rule = p.rules.iter().find(|x| x.rows == vec![out]).unwrap();
        assert_eq!(rule.target, 0, "the app's own ruleset");
        assert_eq!(
            rule.providers,
            vec![GenActor::Ref(ActorRef::new(
                ActorKind::IpList,
                "/orgs/5/sec_policy/draft/ip_lists/21"
            ))]
        );
        assert!(p.ip_lists.is_empty());
        assert_eq!(rule.ips["Corp"].iter().collect::<Vec<_>>(), ["10.1.9.9"]);
    }

    #[test]
    fn a_deny_that_cannot_be_evaluated_exactly_counts_as_matching() {
        let mut f = rich_flows();
        f.push(intra_on(22)); // web -> db
        f.push(web_to_web(22));
        // ams minus role=web: exactly, web -> db is not denied; fail safe, it is.
        let excl = deny_22(serde_json::json!({"consumers": [{"actors": "ams"},
            {"label": {"href": "/orgs/5/labels/1"}, "exclusion": true}]}));
        let (_, p) = run_in(excl, &f, Stop::AppPorts, false, |_| {});
        assert_eq!(denied(&p), 1);
        assert_eq!(p.coverage.override_deny, 1);
        // A provider label the index does not know: both 22 flows.
        let unknown =
            deny_22(serde_json::json!({"providers": [{"label": {"href": "/orgs/5/labels/999"}}]}));
        let (_, p) = run_in(unknown, &f, Stop::AppPorts, false, |_| {});
        assert_eq!(denied(&p), 2);
        assert_eq!(p.coverage.override_deny, 2);
    }

    #[test]
    fn override_denies_of_every_enabled_ruleset_whose_scope_holds_the_flow_count() {
        let mut f = rich_flows();
        f.push(intra_on(445));
        let global = |enabled: bool| {
            let mut d = sets();
            d[1].enabled = enabled; // Global-Infra, scopes [[]]
            d[1].deny_rules = serde_json::from_value(serde_json::json!([{
                "href": "/orgs/5/sec_policy/draft/rule_sets/101/deny_rules/1", "enabled": true,
                "providers": [{"actors": "ams"}], "consumers": [{"actors": "ams"}],
                "ingress_services": [{"port": 445, "proto": 6}], "override": true
            }]))
            .unwrap();
            d
        };
        let (_, p) = run_in(global(true), &f, Stop::AppPorts, false, |_| {});
        let r = p.rules.iter().find(|r| r.denied_flows > 0).unwrap();
        assert_eq!(
            r.denied_by,
            vec!["/orgs/5/sec_policy/draft/rule_sets/101/deny_rules/1".to_string()]
        );
        assert_eq!(p.coverage.override_deny, 1);
        let (_, p) = run_in(global(false), &f, Stop::AppPorts, false, |_| {});
        assert_eq!(denied(&p), 0, "a disabled ruleset denies nothing");
    }

    #[test]
    fn refresh_checks_the_override_denies_again() {
        let mut f = rich_flows();
        f.push(intra_on(22));
        let (_, mut p) = run(&f, Stop::AppPorts, false, |_| {});
        assert_eq!(p.coverage.override_deny, 1);
        let idx = rich_index();
        let mut gone = sets();
        gone[0].deny_rules.clear();
        p.refresh(&gone, &idx, DEFAULT_RULESET_NAME, None);
        assert_eq!((denied(&p), p.coverage.override_deny), (0, 0));
        assert_eq!(p.coverage.covered, 6);
        p.refresh(&sets(), &idx, DEFAULT_RULESET_NAME, None);
        assert_eq!((denied(&p), p.coverage.override_deny), (1, 1));
    }

    #[test]
    fn one_long_name_still_keeps_the_description_within_the_limit() {
        let d = description(
            &["w".repeat(300)],
            &["shop/prod".into()],
            &["22/tcp".into()],
            "24h",
        );
        assert_eq!(d.chars().count(), DESCRIPTION_MAX);
        assert!(d.ends_with('…'));
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
