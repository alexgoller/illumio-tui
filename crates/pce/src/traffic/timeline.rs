//! Blocked traffic over time: a window cut into buckets, and the
//! approximate timeline the loaded flows give. The approximate one counts
//! flow records by their last-detected time, not connections over time;
//! the exact one fills the same shape from one blocked-only query per
//! bucket (and side). Both name their worst spike.
use super::focus::Focus;
use super::query::format_timestamp;
use super::resolved::ResolvedFlow;
use crate::models::{PolicyDecision, TrafficQuery};
use jiff::{SignedDuration, Timestamp};
use serde::Serialize;
use std::time::Duration;

/// The approximate timeline's bucket count.
pub const APPROX_BUCKETS: usize = 24;
/// Above this many bucket queries `illumio-tui app --timeline` needs `--yes`.
pub const TIMELINE_CONFIRM_QUERIES: u32 = 24;
/// A spike is a bucket at least this many times the median of the non-zero ones.
pub const SPIKE_MULTIPLE: f64 = 3.0;
/// With fewer non-zero buckets than this, no spike is named.
pub const SPIKE_MIN_BUCKETS: usize = 4;

/// The exact timeline's shortest bucket: a shorter window gets fewer
/// buckets (1h: 12 of 5 minutes).
pub const MIN_BUCKET: Duration = Duration::from_secs(300);

/// The exact timeline's bucket count for a window: 24 (of 1h) for 1d, 28
/// (of 6h) for 1w, 28 (of 1d) for 4w, 30 (of 1d) for 30d, else about 24
/// equal buckets, none shorter than `MIN_BUCKET` (at least one).
pub fn bucket_count(window: Duration) -> usize {
    match window.as_secs() {
        86_400 => 24,
        604_800 => 28,
        2_419_200 => 28,
        2_592_000 => 30,
        s => {
            let fit = s / MIN_BUCKET.as_secs();
            usize::try_from(fit.clamp(1, APPROX_BUCKETS as u64)).unwrap_or(APPROX_BUCKETS)
        }
    }
}

/// `start` to `end` cut by `bucket_count`, oldest first, the last bucket
/// ending at `end`; none for an empty range.
pub fn bucket_plan(start: Timestamp, end: Timestamp) -> Vec<Bucket> {
    if end <= start {
        return Vec::new();
    }
    let window = end.duration_since(start).unsigned_abs();
    equal_buckets(start, end, bucket_count(window))
}

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

/// The worst bucket, when it is a spike.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Spike {
    pub index: usize,
    pub start: Timestamp,
    pub end: Timestamp,
    /// The bucket over the median of the non-zero buckets.
    pub multiple: f64,
    /// The bucket's query hit the cap: its count, so the multiple, is a
    /// lower bound (`x6+`).
    pub truncated: bool,
    /// The timeline spans more than a day: a bucket shorter than a day is
    /// named by its date and time.
    #[serde(skip)]
    pub multi_day: bool,
}

impl Spike {
    /// `spike 09:00 (x6 median)` in a timeline of a day or less; `spike
    /// 10-04 18:00` for a shorter bucket in a longer one; a bucket of a day
    /// or more by its date (`spike 10-04 (x3 median)`); a capped bucket
    /// `x6+`. UTC, like every timestamp here.
    pub fn text(&self) -> String {
        let day = self.end.duration_since(self.start) >= SignedDuration::from_hours(24);
        let format = match (day, self.multi_day) {
            (true, _) => "%m-%d",
            (false, true) => "%m-%d %H:%M",
            (false, false) => "%H:%M",
        };
        let when = self.start.strftime(format).to_string();
        let plus = if self.truncated { "+" } else { "" };
        format!("spike {when} (x{:.0}{plus} median)", self.multiple)
    }
}

/// The worst bucket when it is at least `SPIKE_MULTIPLE` times the median
/// of the non-zero buckets: `(index, multiple)`. None with fewer than
/// `SPIKE_MIN_BUCKETS` non-zero buckets; the earlier of equal maxima.
pub fn spike(counts: &[u64]) -> Option<(usize, f64)> {
    let mut nz: Vec<u64> = counts.iter().copied().filter(|&c| c > 0).collect();
    if nz.len() < SPIKE_MIN_BUCKETS {
        return None;
    }
    nz.sort_unstable();
    let mid = nz.len() / 2;
    let median = if nz.len().is_multiple_of(2) {
        (nz[mid - 1] as f64 + nz[mid] as f64) / 2.0
    } else {
        nz[mid] as f64
    };
    let (index, &max) = counts
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(&a.0)))?;
    let multiple = max as f64 / median;
    (multiple >= SPIKE_MULTIPLE).then_some((index, multiple))
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

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Timeline {
    /// `false`: counted from the loaded flows' last-detected times.
    pub exact: bool,
    /// Some buckets are not loaded (an exact timeline stopped part-way).
    pub partial: bool,
    pub buckets: Vec<TimelineBucket>,
    /// Some loaded bucket's query hit the cap: its count is a lower bound
    /// (the bucket says which). The timeline's own, not the traffic's.
    pub truncated: bool,
    /// The worst bucket when it is a spike (over the loaded buckets).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spike: Option<Spike>,
}

impl Timeline {
    /// `spike` from the loaded counts.
    pub fn with_spike(mut self) -> Self {
        let multi_day = self
            .buckets
            .first()
            .zip(self.buckets.last())
            .is_some_and(|(a, b)| b.end.duration_since(a.start) > SignedDuration::from_hours(24));
        self.spike = spike(&self.counts()).map(|(i, multiple)| Spike {
            index: i,
            start: self.buckets[i].start,
            end: self.buckets[i].end,
            multiple,
            truncated: self.buckets[i].truncated,
            multi_day,
        });
        self
    }

    /// The buckets with a count.
    pub fn loaded(&self) -> usize {
        self.buckets.iter().filter(|b| b.count.is_some()).count()
    }

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
        truncated: false,
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
        spike: None,
    }
    .with_spike()
}

/// One bucket's query: `base` for the bucket's range, blocked flows only.
/// The focus's labels are put in by `Pce::focused_traffic`.
pub fn bucket_query(base: &TrafficQuery, b: &Bucket) -> TrafficQuery {
    let mut q = base.clone();
    q.start_date = format_timestamp(b.start);
    q.end_date = format_timestamp(b.end);
    q.policy_decisions = vec![PolicyDecision::Blocked];
    q
}

/// The connections of the blocked flows: one exact bucket's count.
pub fn blocked_connections(flows: &[ResolvedFlow]) -> u64 {
    flows
        .iter()
        .filter(|f| f.policy_decision == PolicyDecision::Blocked)
        .map(|f| f.num_connections)
        .sum()
}

/// One exact-timeline bucket's answer (`Pce::timeline_bucket`): the
/// blocked connections the focus keeps, whether a query hit the cap (the
/// count is then a lower bound), whether every query came from the cache,
/// and how many queries it took (one per side).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BucketCount {
    pub connections: u64,
    pub truncated: bool,
    pub hit: bool,
    pub queries: u32,
}

/// The exact timeline from per-bucket `(connections, truncated)`, `None`
/// for a bucket not loaded yet (then `partial`); the spike over the loaded
/// ones.
pub fn exact_timeline(buckets: &[Bucket], counts: &[Option<(u64, bool)>]) -> Timeline {
    let rows: Vec<TimelineBucket> = buckets
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let c = counts.get(i).copied().flatten();
            TimelineBucket {
                start: b.start,
                end: b.end,
                count: c.map(|(n, _)| n),
                truncated: c.is_some_and(|(_, t)| t),
            }
        })
        .collect();
    Timeline {
        exact: true,
        partial: rows.iter().any(|b| b.count.is_none()),
        truncated: rows.iter().any(|b| b.truncated),
        buckets: rows,
        spike: None,
    }
    .with_spike()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::focus::{FocusDirection, parse_app};
    use crate::traffic::resolved::tests::{flows, rich_flows};
    use std::time::Duration;

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

    #[test]
    fn bucket_plans_follow_the_window() {
        let end = ts("2026-10-10T08:00:00Z");
        let plan = |secs: i64| {
            bucket_plan(
                end.checked_sub(SignedDuration::from_secs(secs)).unwrap(),
                end,
            )
        };
        let day = plan(86_400);
        assert_eq!(day.len(), 24);
        assert_eq!(
            day[0].end.duration_since(day[0].start),
            SignedDuration::from_hours(1)
        );
        let week = plan(7 * 86_400);
        assert_eq!(week.len(), 28);
        assert_eq!(
            week[0].end.duration_since(week[0].start),
            SignedDuration::from_hours(6)
        );
        let month = plan(30 * 86_400);
        assert_eq!(month.len(), 30);
        assert_eq!(
            month[0].end.duration_since(month[0].start),
            SignedDuration::from_hours(24)
        );
        let four_weeks = plan(28 * 86_400);
        assert_eq!(four_weeks.len(), 28);
        assert_eq!(
            four_weeks[0].end.duration_since(four_weeks[0].start),
            SignedDuration::from_hours(24),
            "4w: 28 days"
        );
        for (odd, n) in [
            (2 * 3_600, 24),
            (3 * 86_400, 24),
            (90 * 86_400, 24),
            (5_400, 18),
        ] {
            let p = plan(odd);
            assert_eq!(p.len(), n, "{odd}s");
            assert_eq!(p.last().unwrap().end, end);
            assert!(
                p[0].end.duration_since(p[0].start) >= SignedDuration::from_mins(5),
                "{odd}s: no bucket under 5 minutes"
            );
        }
        let hour = plan(3_600);
        assert_eq!(hour.len(), 12, "1h: 12 buckets of 5 minutes");
        assert_eq!(
            hour[0].end.duration_since(hour[0].start),
            SignedDuration::from_mins(5)
        );
        assert_eq!(plan(120).len(), 1, "a window under 5 minutes is one bucket");
        assert_eq!(bucket_count(Duration::from_secs(86_400)), 24);
        assert_eq!(bucket_count(Duration::from_secs(7 * 86_400)), 28);
        assert_eq!(bucket_count(Duration::from_secs(28 * 86_400)), 28);
        assert_eq!(bucket_count(Duration::from_secs(30 * 86_400)), 30);
        assert_eq!(bucket_count(Duration::from_secs(3_600)), 12);
        assert_eq!(bucket_count(Duration::from_secs(1_800)), 6);
        assert!(bucket_plan(end, end).is_empty());
    }

    #[test]
    fn spikes_need_four_non_zero_buckets_and_three_times_the_median() {
        assert_eq!(spike(&[1, 1, 2, 1, 1, 1, 12, 2, 1]), Some((6, 12.0)));
        assert_eq!(
            spike(&[0, 5, 0, 30, 0, 0]),
            None,
            "two non-zero buckets: never a spike"
        );
        assert_eq!(spike(&[2, 2, 2, 5]), None, "2.5x the median is not a spike");
        assert_eq!(spike(&[2, 2, 2, 6]), Some((3, 3.0)), "exactly 3x is");
        assert_eq!(
            spike(&[1, 3, 0, 4, 20]),
            Some((4, 20.0 / 3.5)),
            "even count: the median of 1 3 4 20 is 3.5"
        );
        assert_eq!(
            spike(&[9, 1, 1, 9, 1, 1]),
            Some((0, 9.0)),
            "a tie names the earlier"
        );
        assert_eq!(spike(&[]), None);
        assert_eq!(spike(&[0, 0, 0, 0, 0]), None);
    }

    #[test]
    fn spike_text_names_the_time_and_the_multiple() {
        let s = Spike {
            index: 6,
            start: ts("2026-10-09T14:00:00Z"),
            end: ts("2026-10-09T15:00:00Z"),
            multiple: 6.2,
            truncated: false,
            multi_day: false,
        };
        assert_eq!(s.text(), "spike 14:00 (x6 median)", "a timeline of a day");
        let week = Spike {
            multi_day: true,
            ..s
        };
        assert_eq!(
            week.text(),
            "spike 10-09 14:00 (x6 median)",
            "an hour bucket in a longer timeline names its date"
        );
        let d = Spike {
            index: 3,
            start: ts("2026-10-04T08:00:00Z"),
            end: ts("2026-10-05T08:00:00Z"),
            multiple: 3.0,
            truncated: true,
            multi_day: true,
        };
        assert_eq!(
            d.text(),
            "spike 10-04 (x3+ median)",
            "a capped bucket: a lower bound"
        );
    }

    #[test]
    fn a_bucket_query_is_the_base_narrowed_to_blocked_and_the_bucket() {
        use crate::traffic::query::TrafficQueryBuilder;
        let base =
            TrafficQueryBuilder::between(ts("2026-10-09T08:00:00Z"), ts("2026-10-10T08:00:00Z"))
                .max_results(500)
                .build()
                .unwrap();
        let b = Bucket {
            start: ts("2026-10-09T09:00:00Z"),
            end: ts("2026-10-09T10:00:00Z"),
        };
        let q = bucket_query(&base, &b);
        assert_eq!(q.start_date, "2026-10-09T09:00:00Z");
        assert_eq!(q.end_date, "2026-10-09T10:00:00Z");
        assert_eq!(q.policy_decisions, vec![PolicyDecision::Blocked]);
        assert_eq!(q.max_results, 500);
        assert_eq!(q.sources, base.sources);
        assert_eq!(q.destinations, base.destinations);
        assert_eq!(q.services, base.services);
    }

    #[test]
    fn the_exact_timeline_counts_blocked_connections_and_marks_what_is_missing() {
        assert_eq!(
            blocked_connections(&flows()),
            3,
            "only the NAS flow is blocked"
        );
        let buckets = equal_buckets(ts("2026-10-10T00:00:00Z"), ts("2026-10-10T06:00:00Z"), 6);
        let full = exact_timeline(
            &buckets,
            &[
                Some((1, false)),
                Some((1, false)),
                Some((2, false)),
                Some((1, false)),
                Some((9, true)),
                Some((1, false)),
            ],
        );
        assert!(full.exact && !full.partial);
        assert_eq!(full.loaded(), 6);
        assert!(full.buckets[4].truncated && full.truncated);
        let s = full.spike.as_ref().expect("9 is 9x the median of 1");
        assert_eq!((s.index, s.start), (4, ts("2026-10-10T04:00:00Z")));
        assert!(s.truncated, "the spike's bucket was capped");
        let part = exact_timeline(&buckets, &[Some((1, false)), None, None, None, None, None]);
        assert!(part.partial && !part.truncated);
        assert_eq!(part.loaded(), 1);
        assert_eq!(part.buckets[1].count, None);
        assert!(part.spike.is_none());
        let json = serde_json::to_value(&full).unwrap();
        assert_eq!(json["spike"]["index"], 4);
        assert_eq!(json["buckets"][4]["truncated"], true);
        assert_eq!(json["truncated"], true);
        assert_eq!(json["spike"]["truncated"], true);
        assert!(serde_json::to_value(&part).unwrap().get("spike").is_none());
    }

    #[test]
    fn the_approx_timeline_names_its_spike_too() {
        let buckets = equal_buckets(ts("2026-10-10T07:00:00Z"), ts("2026-10-10T08:00:00Z"), 4);
        let shop = Focus::new(parse_app("shop/prod"), vec!["app".into(), "env".into()]);
        let t = approx_timeline(&rich_flows(), &shop, &buckets);
        assert!(t.spike.is_none(), "one non-zero bucket");
        let spiky = Timeline {
            exact: false,
            partial: false,
            truncated: false,
            buckets: buckets
                .iter()
                .chain(buckets.iter())
                .zip([1, 1, 1, 1, 8, 1, 1, 1])
                .map(|(b, c)| TimelineBucket {
                    start: b.start,
                    end: b.end,
                    count: Some(c),
                    truncated: false,
                })
                .collect(),
            spike: None,
        }
        .with_spike();
        assert_eq!(spiky.spike.map(|s| s.index), Some(4));
    }

    #[test]
    fn the_spike_of_a_longer_timeline_carries_its_date() {
        let start = ts("2026-10-03T08:00:00Z");
        let week = bucket_plan(start, ts("2026-10-10T08:00:00Z"));
        let mut counts = vec![Some((1, false)); week.len()];
        counts[5] = Some((20, false));
        let t = exact_timeline(&week, &counts);
        assert_eq!(
            t.spike.unwrap().text(),
            "spike 10-04 14:00 (x20 median)",
            "a 6h bucket of a week"
        );
        let day = bucket_plan(ts("2026-10-09T08:00:00Z"), ts("2026-10-10T08:00:00Z"));
        let mut counts = vec![Some((1, false)); day.len()];
        counts[6] = Some((20, false));
        assert_eq!(
            exact_timeline(&day, &counts).spike.unwrap().text(),
            "spike 14:00 (x20 median)",
            "a day: the time only"
        );
    }
}
