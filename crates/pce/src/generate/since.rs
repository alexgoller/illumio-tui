//! "New since" (#81): the generator on the traffic a baseline window did
//! not have. Each recent record of the app is judged with the traffic
//! compare engine (`compare_flows`: the key without decision and
//! `first_detected`, decision sets); only the records whose change the
//! caller keeps (`new`, or `new,changed`) go on to the rows, the rules and
//! the fan-in candidates. The others are counted as `seen_before`. Pure;
//! both sides are fetched by the caller.
use super::Side;
use super::rows::{excluded, side_of};
use crate::traffic::{Change, ChangeSet, CompareCounts, Focus, ResolvedFlow, compare_flows};

/// What the generator reads in "new since" mode.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Since {
    /// The recent records the generator sees: the kept ones, and every
    /// record it never proposes from anyway (broadcast and multicast,
    /// another app's, a side not in force), as they were.
    pub flows: Vec<ResolvedFlow>,
    /// The app's records in force left out: their key was in the baseline
    /// (with the same decisions, or any with `new` alone).
    pub seen_before: u64,
    /// The app's records in force by change, and the keys only the
    /// baseline had (`gone`), as `meta.compare` counts them.
    pub counts: CompareCounts,
}

/// The changes the generator can be fed: `new`, or `new` and `changed`;
/// `gone` and `same` never are.
pub fn feedable(keep: ChangeSet) -> bool {
    keep == ChangeSet::of(&[Change::New]) || keep == ChangeSet::of(&[Change::New, Change::Changed])
}

/// `new since 7d before: 40 flows seen before`, or with no new record in
/// force `nothing new since 7d before: …`: the TUI's footer, the CLI's
/// stderr line.
pub fn since_text(length: &str, new: u64, seen_before: u64) -> String {
    format!(
        "{} since {length} before: {seen_before} flows seen before",
        if new == 0 { "nothing new" } else { "new" }
    )
}

/// `recent` without the app's records `keep` does not keep, against
/// `baseline` (the same focused query over the window before). A record
/// is judged when the generator would read it: the app's, not broadcast
/// or multicast, on the inbound side (or outbound with `outbound`); the
/// baseline is judged on the same records.
pub fn new_since(
    recent: &[ResolvedFlow],
    baseline: &[ResolvedFlow],
    keep: ChangeSet,
    focus: &Focus,
    outbound: bool,
) -> Since {
    let judged = |f: &ResolvedFlow| {
        !excluded(f) && side_of(f, focus).is_some_and(|s| s == Side::Inbound || outbound)
    };
    let mine: Vec<ResolvedFlow> = recent.iter().filter(|f| judged(f)).cloned().collect();
    let base: Vec<ResolvedFlow> = baseline.iter().filter(|f| judged(f)).cloned().collect();
    let cmp = compare_flows(&mine, &base);
    let mut changes = cmp.recent.iter();
    let mut out = Since {
        counts: cmp.counts,
        ..Since::default()
    };
    for f in recent {
        if !judged(f) {
            out.flows.push(f.clone());
            continue;
        }
        match changes.next() {
            Some(c) if keep.contains(c.change) => out.flows.push(f.clone()),
            _ => out.seen_before += 1,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::rows::rows;
    use super::super::rows::tests::{shop, texts};
    use super::super::{DEFAULT_RULESET_NAME, GenInput, Stop, candidates, propose};
    use super::*;
    use crate::models::PolicyDecision;
    use crate::traffic::ResolvedEndpoint;
    use crate::traffic::resolved::tests::{rich_flows, rich_index};
    use std::collections::BTreeMap;

    fn new_only() -> ChangeSet {
        ChangeSet::of(&[Change::New])
    }

    fn new_changed() -> ChangeSet {
        ChangeSet::of(&[Change::New, Change::Changed])
    }

    /// The rich fixture's intra-app records (shop web to shop db).
    fn intra() -> Vec<ResolvedFlow> {
        let f = rich_flows();
        [0, 1, 26, 29].iter().map(|i| f[*i].clone()).collect()
    }

    #[test]
    fn only_new_keys_feed_the_rows_and_the_rest_count_as_seen_before() {
        let s = new_since(&rich_flows(), &intra(), new_only(), &shop(), false);
        assert_eq!(s.seen_before, 4);
        assert_eq!(s.flows.len(), rich_flows().len() - 4);
        assert_eq!((s.counts.new, s.counts.same, s.counts.gone), (4, 4, 0));
        let r = rows(&s.flows, &shop(), "role", &rich_index(), None);
        assert!(
            texts(&r).iter().all(|t| t.1 != "intra-app"),
            "a row with no flows left disappears"
        );
        assert!(texts(&r).iter().any(|t| t.1 == "hr/prod"));
    }

    #[test]
    fn an_empty_baseline_keeps_everything_and_gone_keys_are_counted() {
        let s = new_since(&rich_flows(), &[], new_only(), &shop(), false);
        assert_eq!((s.seen_before, s.flows.len()), (0, rich_flows().len()));
        // A baseline key the recent window lacks: gone, never fed.
        let mut old = rich_flows()[0].clone();
        old.port = Some(9999);
        let s = new_since(&rich_flows(), &[old], new_only(), &shop(), false);
        assert_eq!((s.counts.gone, s.seen_before), (1, 0));
    }

    #[test]
    fn a_changed_decision_feeds_only_with_new_changed() {
        let mut base = intra();
        base[0].policy_decision = PolicyDecision::Blocked;
        let only = new_since(&rich_flows(), &base, new_only(), &shop(), false);
        let both = new_since(&rich_flows(), &base, new_changed(), &shop(), false);
        assert!(only.counts.changed > 0);
        assert_eq!(only.seen_before, 4);
        assert_eq!(
            both.seen_before,
            4 - only.counts.changed,
            "the changed key's records feed the generator"
        );
    }

    #[test]
    fn the_outbound_side_is_judged_only_when_in_force() {
        let all = rich_flows();
        let s = new_since(&all, &all, new_only(), &shop(), false);
        let o = new_since(&all, &all, new_only(), &shop(), true);
        assert!(o.seen_before > s.seen_before, "outbound records too");
        let r = rows(&s.flows, &shop(), "role", &rich_index(), None);
        assert!(
            r.iter().all(|r| r.side == Side::Outbound),
            "inbound all seen before; outbound passes untouched"
        );
        let r = rows(&o.flows, &shop(), "role", &rich_index(), None);
        assert!(r.is_empty());
    }

    #[test]
    fn the_text_says_when_nothing_is_new() {
        assert_eq!(
            since_text("7d", 3, 40),
            "new since 7d before: 40 flows seen before"
        );
        assert_eq!(
            since_text("7d", 0, 40),
            "nothing new since 7d before: 40 flows seen before"
        );
    }

    #[test]
    fn feedable_is_new_or_new_changed_only() {
        assert!(feedable(new_only()));
        assert!(feedable(new_changed()));
        assert!(feedable(ChangeSet::parse("changed,new").unwrap()));
        for s in ["changed", "new,gone", "same", "new,changed,same"] {
            assert!(!feedable(ChangeSet::parse(s).unwrap()), "{s}");
        }
    }

    #[test]
    fn coverage_counts_seen_before_apart() {
        let s = new_since(&rich_flows(), &intra(), new_only(), &shop(), false);
        let idx = rich_index();
        let focus = shop();
        let r = rows(&s.flows, &focus, "role", &idx, None);
        let input = GenInput {
            flows: &s.flows,
            focus: &focus,
            tier_key: "role",
            drafts: &[],
            idx: &idx,
            ruleset_name: DEFAULT_RULESET_NAME,
            window: "24h",
            outbound: false,
            picked: None,
            consolidate: &BTreeMap::new(),
            seen_before: s.seen_before,
        };
        let p = propose(&r, Stop::AppPorts, &input);
        assert_eq!(p.coverage.seen_before, 4);
        assert_eq!(
            p.coverage.covered + p.coverage.uncovered + p.coverage.seen_before,
            8,
            "every inbound record of the app, the seen before ones apart"
        );
        assert_eq!(p.coverage.uncovered, 3, "the unchecked ones only");
    }

    /// An IP peer `ip` (no workload) to shop db on 5666/tcp.
    fn nrpe_from(ip: &str) -> ResolvedFlow {
        let mut f = rich_flows()[0].clone();
        f.src = ResolvedEndpoint {
            ip: Some(ip.into()),
            ..Default::default()
        };
        f.port = Some(5666);
        f
    }

    #[test]
    fn fan_in_is_judged_on_the_remaining_flows_only() {
        let recent: Vec<ResolvedFlow> = ["10.9.0.1", "10.9.0.2", "10.9.0.3"]
            .into_iter()
            .map(nrpe_from)
            .collect();
        let idx = rich_index();
        let all = rows(&recent, &shop(), "role", &idx, None);
        assert_eq!(candidates(&all, 5).len(), 1, "three peers on 5666/tcp");
        let s = new_since(&recent, &recent[..1], new_only(), &shop(), false);
        assert_eq!(s.seen_before, 1);
        let left = rows(&s.flows, &shop(), "role", &idx, None);
        assert!(candidates(&left, 5).is_empty(), "two new peers only");
    }
}
