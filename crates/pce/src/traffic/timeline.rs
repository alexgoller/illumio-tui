//! Blocked traffic over time: a window cut into buckets, and the
//! approximate timeline the loaded flows give. The approximate one counts
//! flow records by their last-detected time, not connections over time;
//! Plan 3's exact timeline fills the same shape from one blocked-only
//! query per bucket.
use super::focus::Focus;
use super::resolved::ResolvedFlow;
use crate::models::PolicyDecision;
use jiff::{SignedDuration, Timestamp};
use serde::Serialize;

/// The approximate timeline's bucket count.
pub const APPROX_BUCKETS: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Bucket {
    pub start: Timestamp,
    pub end: Timestamp,
}

/// `start` to `end` in `n` equal buckets, oldest first, the last one ending
/// exactly at `end`; none for an empty range or `n == 0`.
pub fn equal_buckets(start: Timestamp, end: Timestamp, n: usize) -> Vec<Bucket> {
    if end <= start || n == 0 {
        return Vec::new();
    }
    let total = end.duration_since(start).as_nanos();
    let at = |i: usize| -> Timestamp {
        if i >= n {
            return end;
        }
        let off = i64::try_from(total * i as i128 / n as i128).unwrap_or(i64::MAX);
        start
            .checked_add(SignedDuration::from_nanos(off))
            .unwrap_or(end)
    };
    (0..n)
        .map(|i| Bucket {
            start: at(i),
            end: at(i + 1),
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimelineBucket {
    pub start: Timestamp,
    pub end: Timestamp,
    /// Approximate: blocked flow records. Exact: blocked connections.
    /// `None` until the bucket is loaded (exact only).
    pub count: Option<u64>,
    /// The bucket's query hit the cap: `count` is a lower bound.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Timeline {
    /// `false`: counted from the loaded flows' last-detected times.
    pub exact: bool,
    /// Some buckets are not loaded (an exact timeline stopped part-way).
    pub partial: bool,
    pub buckets: Vec<TimelineBucket>,
}

impl Timeline {
    /// Every bucket's count, oldest first; an unloaded one as 0.
    pub fn counts(&self) -> Vec<u64> {
        self.buckets.iter().map(|b| b.count.unwrap_or(0)).collect()
    }

    /// No bucket counts anything.
    pub fn is_quiet(&self) -> bool {
        self.buckets.iter().all(|b| b.count.unwrap_or(0) == 0)
    }
}

/// The approximate timeline: the blocked flows `focus` keeps, counted as
/// flow records in the bucket their `last_detected` falls into (a bucket
/// holds its start, not its end; the last one holds its end too). A flow
/// without a parseable `last_detected`, or outside every bucket, is not
/// counted.
pub fn approx_timeline(flows: &[ResolvedFlow], focus: &Focus, buckets: &[Bucket]) -> Timeline {
    let mut counts = vec![0u64; buckets.len()];
    let last = buckets.len().checked_sub(1);
    for f in flows
        .iter()
        .filter(|f| f.policy_decision == PolicyDecision::Blocked && focus.keeps(f))
    {
        let Some(t) = f
            .last_detected
            .as_deref()
            .and_then(|s| s.parse::<Timestamp>().ok())
        else {
            continue;
        };
        let i = buckets
            .iter()
            .position(|b| b.start <= t && t < b.end)
            .or_else(|| last.filter(|&l| buckets[l].end == t));
        if let Some(i) = i {
            counts[i] += 1;
        }
    }
    Timeline {
        exact: false,
        partial: false,
        buckets: buckets
            .iter()
            .zip(counts)
            .map(|(b, c)| TimelineBucket {
                start: b.start,
                end: b.end,
                count: Some(c),
                truncated: false,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::focus::{FocusDirection, parse_app};
    use crate::traffic::resolved::tests::rich_flows;

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn equal_buckets_cut_the_range_and_end_exactly_at_its_end() {
        let b = equal_buckets(ts("2026-10-09T08:00:00Z"), ts("2026-10-10T08:00:00Z"), 24);
        assert_eq!(b.len(), 24);
        assert_eq!(b[0].start, ts("2026-10-09T08:00:00Z"));
        assert_eq!(b[0].end, ts("2026-10-09T09:00:00Z"));
        assert_eq!(b[23].end, ts("2026-10-10T08:00:00Z"));
        assert!(
            b.windows(2).all(|w| w[0].end == w[1].start),
            "no gap, no overlap"
        );
        let odd = equal_buckets(ts("2026-10-10T00:00:00Z"), ts("2026-10-10T00:00:10Z"), 3);
        assert_eq!(odd.last().unwrap().end, ts("2026-10-10T00:00:10Z"));
        assert!(
            equal_buckets(ts("2026-10-10T00:00:00Z"), ts("2026-10-10T00:00:00Z"), 4).is_empty()
        );
        assert!(
            equal_buckets(ts("2026-10-10T00:00:00Z"), ts("2026-10-10T01:00:00Z"), 0).is_empty()
        );
    }

    #[test]
    fn approx_counts_blocked_flow_records_by_last_detected() {
        let shop = Focus::new(parse_app("shop/prod"), vec!["app".into(), "env".into()]);
        let buckets = equal_buckets(ts("2026-10-10T07:00:00Z"), ts("2026-10-10T08:00:00Z"), 4);
        let t = approx_timeline(&rich_flows(), &shop, &buckets);
        assert!(!t.exact && !t.partial);
        assert_eq!(
            t.counts(),
            vec![2, 0, 0, 0],
            "flows 15 (07:12) and 21 (07:01) are the blocked ones into shop/prod"
        );
        assert_eq!(t.buckets[0].count, Some(2));
        assert!(!t.is_quiet());
        let out = approx_timeline(
            &rich_flows(),
            &shop.clone().with_direction(FocusDirection::Out),
            &buckets,
        );
        assert!(out.is_quiet(), "shop/prod sends nothing blocked");
        let early = equal_buckets(ts("2026-10-01T00:00:00Z"), ts("2026-10-02T00:00:00Z"), 24);
        assert!(
            approx_timeline(&rich_flows(), &shop, &early).is_quiet(),
            "outside every bucket"
        );
    }

    #[test]
    fn a_flow_on_the_last_instant_counts_in_the_last_bucket() {
        let crm = Focus::new(parse_app("crm/prod"), vec!["app".into(), "env".into()]);
        // flow 6, crm/dev -> crm/prod, blocked, last seen 07:20:00
        let buckets = equal_buckets(ts("2026-10-10T07:00:00Z"), ts("2026-10-10T07:20:00Z"), 2);
        assert_eq!(
            approx_timeline(&rich_flows(), &crm, &buckets).counts(),
            vec![0, 1]
        );
    }
}
