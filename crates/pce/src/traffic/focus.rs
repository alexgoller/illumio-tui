//! App focus: one app (a group under the `g` keys) and the direction of its
//! traffic. A pure predicate over the loaded flows, the focused Explorer
//! queries that spend the cap on the app (one per side, because Explorer
//! ANDs sources and destinations), and their merge. No requests here;
//! `Pce::focused_traffic` runs the queries.
use super::aggregate::{group_key, keys_or_default};
use super::merge::merge_flows;
use super::resolved::ResolvedFlow;
use super::sections::Direction;
use crate::models::{ActorMatch, Ref, TrafficQuery};
use crate::resolve::NameIndex;
use serde::Serialize;
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::time::Duration;

/// The values of one group under the keys in force, in key order:
/// `["shop", "prod"]`. What `group_key` returns.
pub type GroupKey = Vec<String>;

/// Which of the app's flows the focus keeps: `In` the flows whose
/// destination is the app, `Out` whose source is, `Both` either.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FocusDirection {
    In,
    Out,
    #[default]
    Both,
}

impl FocusDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            FocusDirection::In => "in",
            FocusDirection::Out => "out",
            FocusDirection::Both => "both",
        }
    }

    /// `in+out` for both: the app overlay's header.
    pub fn header_text(self) -> &'static str {
        match self {
            FocusDirection::In => "in",
            FocusDirection::Out => "out",
            FocusDirection::Both => "in+out",
        }
    }

    /// `X`: in → out → both → in.
    pub fn next(self) -> Self {
        match self {
            FocusDirection::In => FocusDirection::Out,
            FocusDirection::Out => FocusDirection::Both,
            FocusDirection::Both => FocusDirection::In,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "in" => Some(FocusDirection::In),
            "out" => Some(FocusDirection::Out),
            "both" => Some(FocusDirection::Both),
            _ => None,
        }
    }

    /// The Explorer queries a focused re-query takes: 2 for `both`.
    pub fn query_count(self) -> u32 {
        u32::try_from(self.sides().len()).unwrap_or(2)
    }

    /// The sides a focused query puts the app on, in the order they run.
    pub fn sides(self) -> &'static [Direction] {
        match self {
            FocusDirection::In => &[Direction::In],
            FocusDirection::Out => &[Direction::Out],
            FocusDirection::Both => &[Direction::In, Direction::Out],
        }
    }
}

/// An app plus a direction: what every traffic view is narrowed to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Focus {
    pub app: GroupKey,
    /// The keys `app` is a group of (never empty: `app,env` by default).
    pub keys: Vec<String>,
    pub direction: FocusDirection,
}

impl Focus {
    /// A new focus starts at `both`.
    pub fn new(app: GroupKey, keys: Vec<String>) -> Self {
        Self {
            app,
            keys: keys_or_default(&keys),
            direction: FocusDirection::Both,
        }
    }

    pub fn with_direction(mut self, direction: FocusDirection) -> Self {
        self.direction = direction;
        self
    }

    /// Whether `f` is the app's traffic in this direction. A flow from the
    /// app to itself is kept by `in`, by `out` and (once) by `both`. A side
    /// missing one of the keys is not in the app (its slot is `-`), as in
    /// `group`.
    pub fn keeps(&self, f: &ResolvedFlow) -> bool {
        let dst = || group_key(&f.dst, &self.keys) == self.app;
        let src = || group_key(&f.src, &self.keys) == self.app;
        match self.direction {
            FocusDirection::In => dst(),
            FocusDirection::Out => src(),
            FocusDirection::Both => dst() || src(),
        }
    }

    /// `shop/prod`.
    pub fn app_text(&self) -> String {
        self.app.join("/")
    }

    /// `focus shop/prod · in`: the traffic views' header badge.
    pub fn badge(&self) -> String {
        format!("focus {} · {}", self.app_text(), self.direction.as_str())
    }

    /// The same app under the same keys, whatever the direction.
    pub fn same_app(&self, other: &Focus) -> bool {
        self.app == other.app && self.keys == other.keys
    }

    /// One real label value per key: no `-` slot, no `ipl:` / host name
    /// group. Only such an app can be put into a query.
    pub fn is_label_group(&self) -> bool {
        self.app.len() == self.keys.len()
            && self
                .app
                .iter()
                .all(|v| !v.is_empty() && v != "-" && !v.starts_with("ipl:"))
    }

    /// The app's labels as one AND group, in key order.
    pub fn label_matches(&self, idx: &NameIndex) -> Result<Vec<ActorMatch>, String> {
        app_labels(&self.app, &self.keys, idx)
    }

    /// One query per side of `direction` (`in` first): `base` with the app's
    /// labels on that side (`focus_query`). `Err` names the labels the PCE
    /// does not have, before any request.
    pub fn queries(
        &self,
        base: &TrafficQuery,
        idx: &NameIndex,
    ) -> Result<Vec<(Direction, TrafficQuery)>, String> {
        let labels = self.label_matches(idx)?;
        Ok(self
            .direction
            .sides()
            .iter()
            .map(|&side| (side, focus_query(base, side, &labels)))
            .collect())
    }
}

/// `shop/prod` → `["shop", "prod"]`, each part trimmed. A label value that
/// itself holds `/` cannot be written this way.
pub fn parse_app(text: &str) -> GroupKey {
    text.split('/').map(|s| s.trim().to_string()).collect()
}

/// `1 query`, `2 queries`: the TUI status and header and the CLI meta line.
pub fn queries_text(n: u32) -> String {
    format!("{n} quer{}", if n == 1 { "y" } else { "ies" })
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The label of each of `app`'s values under `keys`, as actor matches. An
/// unknown value is named with up to three of the key's values closest to
/// it; a `-` slot (a missing label) cannot be queried.
pub fn app_labels(
    app: &[String],
    keys: &[String],
    idx: &NameIndex,
) -> Result<Vec<ActorMatch>, String> {
    let text = app.join("/");
    if app.len() != keys.len() {
        let shape: Vec<String> = keys.iter().map(|k| format!("<{k}>")).collect();
        return Err(format!(
            "app {text:?} has {} for the {} {}: write it as {}",
            plural(app.len(), "value", "values"),
            plural(keys.len(), "key", "keys"),
            keys.join(","),
            shape.join("/")
        ));
    }
    let mut labels = Vec::new();
    let mut misses = Vec::new();
    for (key, value) in keys.iter().zip(app) {
        if value == "-" {
            misses.push(format!("no {key} label: a - slot cannot be queried"));
            continue;
        }
        match idx.label_href(key, value) {
            Some(href) => labels.push(ActorMatch::Label {
                label: Ref::new(href),
            }),
            None => {
                let near = closest(value, &idx.values_for_key(key), 3);
                misses.push(if near.is_empty() {
                    format!("no {key}={value} label")
                } else {
                    format!("no {key}={value} label (closest: {})", near.join(", "))
                });
            }
        }
    }
    if misses.is_empty() {
        Ok(labels)
    } else {
        Err(format!("unknown app {text:?}: {}", misses.join("; ")))
    }
}

/// `base` with `labels` ANDed into every include group of `side` (`In`:
/// destinations, `Out`: sources); an empty include becomes `labels` alone.
/// Window, the other side, services, decisions, excludes and the cap stay.
pub fn focus_query(base: &TrafficQuery, side: Direction, labels: &[ActorMatch]) -> TrafficQuery {
    let mut q = base.clone();
    let filter = match side {
        Direction::In => &mut q.destinations,
        Direction::Out => &mut q.sources,
    };
    filter.include.retain(|g| !g.is_empty());
    if filter.include.is_empty() {
        filter.include.push(labels.to_vec());
    } else {
        for group in &mut filter.include {
            for l in labels {
                if !group.contains(l) {
                    group.push(l.clone());
                }
            }
        }
    }
    q
}

/// The flows `focus` keeps, in order; all of them, borrowed, without a focus.
pub fn focus_flows<'a>(
    focus: Option<&Focus>,
    flows: &'a [ResolvedFlow],
) -> Cow<'a, [ResolvedFlow]> {
    match focus {
        None => Cow::Borrowed(flows),
        Some(f) => Cow::Owned(flows.iter().filter(|x| f.keeps(x)).cloned().collect()),
    }
}

/// Two focused queries' flows as one result, by the one merge rule
/// (`merge_flows`): a record `first` already holds is not added again, so
/// a flow from the app to itself, which the `in` and the `out` query both
/// return, counts once, and a later version of it replaces the earlier.
/// Order: `first`, then the rest of `second`.
pub fn union_flows(mut first: Vec<ResolvedFlow>, second: Vec<ResolvedFlow>) -> Vec<ResolvedFlow> {
    merge_flows(&mut first, second);
    first
}

/// Every labelled workload group in `flows` under `keys` (`shop/prod`),
/// either side, sorted: what `:focus` and the app picker complete over.
pub fn apps_in(flows: &[ResolvedFlow], keys: &[String]) -> Vec<String> {
    let keys = keys_or_default(keys);
    let mut apps = BTreeSet::new();
    for f in flows {
        for ep in [&f.src, &f.dst] {
            if ep.is_workload() && !ep.labels.is_empty() {
                apps.insert(group_key(ep, &keys).join("/"));
            }
        }
    }
    apps.into_iter().collect()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = Vec::with_capacity(b.len() + 1);
        cur.push(i + 1);
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Up to `n` of `candidates` closest to `name`, case-insensitively: those
/// containing it first, then by edit distance (at most a quarter of the
/// name's length, at least 2); ties alphabetical. Empty for an empty name.
pub fn closest(name: &str, candidates: &[String], n: usize) -> Vec<String> {
    let needle = name.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let limit = (needle.chars().count() / 4).max(2);
    let mut scored: Vec<(usize, &String)> = candidates
        .iter()
        .filter_map(|c| {
            let lc = c.to_lowercase();
            let score = if lc.contains(&needle) {
                0
            } else {
                edit_distance(&lc, &needle)
            };
            (score <= limit).then_some((score, c))
        })
        .collect();
    scored.sort();
    scored.into_iter().take(n).map(|(_, c)| c.clone()).collect()
}

/// What `Pce::focused_traffic` returns: the merged, narrowed flows of one
/// query per side.
#[derive(Debug, Clone)]
pub struct FocusedTraffic {
    pub flows: Vec<ResolvedFlow>,
    /// Any query hit the cap.
    pub truncated: bool,
    /// A supercluster region did not respond to a query.
    pub incomplete: bool,
    pub elapsed: Duration,
    /// Every query was answered from the cache.
    pub hit: bool,
    /// The bodies sent (or reused), `in` first.
    pub queries: Vec<TrafficQuery>,
}

impl FocusedTraffic {
    pub fn query_count(&self) -> u32 {
        u32::try_from(self.queries.len()).unwrap_or(u32::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::query::TrafficQueryBuilder;
    use crate::traffic::resolved::tests::{flows, index, rich_flows};
    use FocusDirection::{Both, In, Out};
    use serde_json::json;

    fn k() -> Vec<String> {
        vec!["app".into(), "env".into()]
    }

    fn focus(s: &str, d: FocusDirection) -> Focus {
        Focus::new(parse_app(s), k()).with_direction(d)
    }

    #[test]
    fn keeps_follows_the_direction_and_counts_self_traffic_once() {
        let f = flows();
        let kept = |d| f.iter().filter(|x| focus("shop/prod", d).keeps(x)).count();
        assert_eq!(kept(In), 3, "flow 0 (to itself), the IP list, the NAS");
        assert_eq!(kept(Out), 2, "flow 0 (to itself) and the FQDN");
        assert_eq!(kept(Both), 4, "the union: flow 0 once");
        let rich = rich_flows();
        let n = |d| {
            rich.iter()
                .filter(|x| focus("crm/prod", d).keeps(x))
                .count()
        };
        assert_eq!((n(In), n(Out), n(Both)), (7, 4, 9));
    }

    #[test]
    fn a_missing_label_key_is_not_the_app() {
        let f = flows();
        let three: Vec<String> = vec!["app".into(), "env".into(), "loc".into()];
        let eu = Focus::new(parse_app("shop/prod/eu"), three.clone());
        assert_eq!(
            f.iter().filter(|x| eu.keeps(x)).count(),
            0,
            "no workload in the flows carries loc: shop/prod/- is not shop/prod/eu"
        );
        let dashed = Focus::new(parse_app("shop/prod/-"), three);
        assert_eq!(f.iter().filter(|x| dashed.keeps(x)).count(), 4);
        assert!(
            !dashed.is_label_group(),
            "a - slot has no label to query by"
        );
        assert!(focus("shop/prod", Both).is_label_group());
        assert!(!focus("ipl:Corp/-", Both).is_label_group());
    }

    #[test]
    fn x_cycles_in_out_both_and_a_new_focus_starts_at_both() {
        assert_eq!(Focus::new(parse_app("shop/prod"), k()).direction, Both);
        let mut d = Both;
        let mut seen = Vec::new();
        for _ in 0..3 {
            d = d.next();
            seen.push(d);
        }
        assert_eq!(seen, vec![In, Out, Both]);
        assert_eq!(FocusDirection::parse(" IN "), Some(In));
        assert_eq!(FocusDirection::parse("sideways"), None);
        assert_eq!(Both.sides(), &[Direction::In, Direction::Out]);
        assert_eq!(Out.sides(), &[Direction::Out]);
        assert_eq!(focus("shop/prod", In).badge(), "focus shop/prod · in");
        assert_eq!(Both.header_text(), "in+out");
        assert_eq!(
            Focus::new(parse_app("shop/prod"), Vec::new()).keys,
            k(),
            "no keys: app,env"
        );
        assert!(focus("shop/prod", In).same_app(&focus("shop/prod", Out)));
    }

    #[test]
    fn parse_app_splits_on_slash_and_an_extra_slot_is_an_error() {
        assert_eq!(parse_app(" shop / prod "), vec!["shop", "prod"]);
        let e = app_labels(&parse_app("a/b/prod"), &k(), &index()).unwrap_err();
        assert!(
            e.contains("has 3 values for the 2 keys app,env: write it as <app>/<env>"),
            "{e}"
        );
    }

    #[test]
    fn queries_put_the_app_labels_on_its_side_and_both_needs_two() {
        let idx = index();
        let base = TrafficQueryBuilder::between(
            "2026-10-09T00:00:00Z".parse().unwrap(),
            "2026-10-10T00:00:00Z".parse().unwrap(),
        )
        .source(vec![ActorMatch::IpAddress {
            ip_address: "10.0.0.1".into(),
        }])
        .max_results(500)
        .build()
        .unwrap();
        let labels = json!([
            {"label": {"href": "/orgs/5/labels/3"}},
            {"label": {"href": "/orgs/5/labels/4"}}
        ]);
        let qs = focus("shop/prod", Both).queries(&base, &idx).unwrap();
        assert_eq!(qs.len(), 2);
        assert_eq!(qs[0].0, Direction::In);
        assert_eq!(qs[1].0, Direction::Out);
        let v0 = serde_json::to_value(&qs[0].1).unwrap();
        assert_eq!(v0["destinations"]["include"], json!([labels.clone()]));
        assert_eq!(
            v0["sources"]["include"],
            json!([[{"ip_address": "10.0.0.1"}]]),
            "the user's source filter stays"
        );
        let v1 = serde_json::to_value(&qs[1].1).unwrap();
        assert_eq!(
            v1["sources"]["include"],
            json!([[
                {"ip_address": "10.0.0.1"},
                {"label": {"href": "/orgs/5/labels/3"}},
                {"label": {"href": "/orgs/5/labels/4"}}
            ]]),
            "ANDed into the user's group"
        );
        assert_eq!(v1["destinations"]["include"], json!([[]]));
        for v in [&v0, &v1] {
            assert_eq!(v["start_date"], "2026-10-09T00:00:00Z");
            assert_eq!(v["end_date"], "2026-10-10T00:00:00Z");
            assert_eq!(v["max_results"], 500);
            assert_eq!(
                v["policy_decisions"],
                json!(["allowed", "potentially_blocked", "blocked"])
            );
        }
        assert_eq!(
            focus("shop/prod", In).queries(&base, &idx).unwrap().len(),
            1
        );
        assert_eq!(
            focus("shop/prod", Out).queries(&base, &idx).unwrap()[0].0,
            Direction::Out
        );
    }

    #[test]
    fn an_unknown_app_names_the_closest_labels() {
        let idx = index();
        let e = focus("shp/prod", Both).label_matches(&idx).unwrap_err();
        assert_eq!(
            e,
            "unknown app \"shp/prod\": no app=shp label (closest: shop)"
        );
        let e = focus("shop", Both).label_matches(&idx).unwrap_err();
        assert!(
            e.contains("has 1 value for the 2 keys app,env: write it as <app>/<env>"),
            "{e}"
        );
        let e = focus("unmanaged-nas/-", Both)
            .label_matches(&idx)
            .unwrap_err();
        assert!(
            e.contains("no app=unmanaged-nas label")
                && e.contains("no env label: a - slot cannot be queried"),
            "{e}"
        );
    }

    #[test]
    fn union_drops_the_rows_both_queries_returned() {
        let f = flows();
        let side = |d| -> Vec<ResolvedFlow> {
            f.iter()
                .filter(|x| focus("shop/prod", d).keeps(x))
                .cloned()
                .collect()
        };
        let inbound = side(In);
        let both = union_flows(inbound.clone(), side(Out));
        assert_eq!(both.len(), 4, "flow 0 is in both and counts once");
        assert_eq!(both.iter().map(|x| x.num_connections).sum::<u64>(), 330);
        assert_eq!(
            &both[..3],
            &inbound[..],
            "the first query's rows keep their order"
        );
    }

    #[test]
    fn apps_in_lists_the_workload_groups_and_closest_ranks_them() {
        assert_eq!(apps_in(&flows(), &k()), vec!["shop/prod"]);
        let rich = apps_in(&rich_flows(), &k());
        assert_eq!(
            rich,
            vec![
                "crm/dev",
                "crm/prod",
                "erp/dev",
                "erp/prod",
                "hr/dev",
                "hr/prod",
                "shop/dev",
                "shop/prod"
            ]
        );
        let c = |n: &str| closest(n, &rich, 3);
        assert_eq!(c("crm"), vec!["crm/dev", "crm/prod"]);
        assert_eq!(c("shop/prd"), vec!["shop/prod"]);
        assert_eq!(c("crm/prd"), vec!["crm/prod"]);
        assert!(c("zzzzzz").is_empty());
        assert!(c("").is_empty());
    }

    #[test]
    fn query_counts_and_their_text() {
        assert_eq!(
            (In.query_count(), Out.query_count(), Both.query_count()),
            (1, 1, 2)
        );
        assert_eq!(
            (queries_text(1), queries_text(2)),
            ("1 query".into(), "2 queries".into())
        );
    }

    #[test]
    fn focus_flows_borrows_without_a_focus() {
        let f = flows();
        assert!(matches!(focus_flows(None, &f), Cow::Borrowed(_)));
        assert_eq!(focus_flows(Some(&focus("shop/prod", In)), &f).len(), 3);
    }
}
