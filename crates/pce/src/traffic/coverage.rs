//! How much of the requested window a (truncated) result covers, and the
//! backfill that fills the rest: slices sized to fit under the cap, walked
//! newest first. Explorer results are not time-ordered within the cap, so
//! slicing by time is the only reliable way to page. Pure: no clock, no
//! network.
use super::resolved::ResolvedFlow;
use jiff::{Span, Timestamp};
use std::time::Duration;

/// Slice lengths a plan rounds down to, shortest first.
pub const SLICE_STEPS: [Duration; 9] = [
    Duration::from_secs(60),
    Duration::from_secs(5 * 60),
    Duration::from_secs(15 * 60),
    Duration::from_secs(30 * 60),
    Duration::from_secs(3600),
    Duration::from_secs(2 * 3600),
    Duration::from_secs(6 * 3600),
    Duration::from_secs(12 * 3600),
    Duration::from_secs(86_400),
];

/// The shortest slice; one that still truncates stops the backfill.
pub const MIN_SLICE: Duration = SLICE_STEPS[0];

/// At most this many slice requests per backfill.
pub const MAX_SLICES: u32 = 50;

/// A slice is sized to fill this share of the cap at the measured rate.
const FILL: f64 = 0.8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coverage {
    /// `newest − oldest` over the flows' `last_detected`.
    pub covered: Duration,
    pub newest: Option<Timestamp>,
    pub oldest: Option<Timestamp>,
    /// `covered` as a share of the window, at most 1.
    pub share: f64,
    /// Flows that carry a `last_detected` (the others are ignored).
    pub flows: usize,
}

/// The span the flows' `last_detected` cover and its share of `window`.
pub fn coverage(flows: &[ResolvedFlow], window: Duration) -> Coverage {
    let times: Vec<Timestamp> = flows
        .iter()
        .filter_map(|f| f.last_detected.as_deref()?.parse().ok())
        .collect();
    let newest = times.iter().max().copied();
    let oldest = times.iter().min().copied();
    let covered = match (newest, oldest) {
        (Some(n), Some(o)) => n.duration_since(o).unsigned_abs(),
        _ => Duration::ZERO,
    };
    Coverage {
        covered,
        newest,
        oldest,
        share: share(covered, window),
        flows: times.len(),
    }
}

/// `covered / window`, at most 1; 1 for an empty window.
pub fn share(covered: Duration, window: Duration) -> f64 {
    if window.is_zero() {
        1.0
    } else {
        (covered.as_secs_f64() / window.as_secs_f64()).min(1.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackfillPlan {
    /// The time the slices fill: from the oldest covered instant back to
    /// the window start.
    pub uncovered: Duration,
    pub slice: Duration,
    /// Slices needed; may exceed `MAX_SLICES` (the caller refuses then).
    pub count: u32,
}

impl BackfillPlan {
    /// `uncovered` in slices of `slice`.
    pub fn with_slice(uncovered: Duration, slice: Duration) -> Self {
        Self {
            uncovered,
            slice,
            count: slices_needed(uncovered, slice),
        }
    }

    pub fn too_many(&self) -> bool {
        self.count > MAX_SLICES
    }
}

/// The slice that holds `FILL × cap` flows at the rate `flows` per
/// `covered` (a span under a minute counts as one), rounded down to a
/// `SLICE_STEPS` step, never under `MIN_SLICE`.
pub fn slice_for(flows: usize, covered: Duration, cap: u32) -> Duration {
    if flows == 0 {
        return SLICE_STEPS[SLICE_STEPS.len() - 1];
    }
    let minutes = (covered.as_secs_f64() / 60.0).max(1.0);
    let rate = flows as f64 / minutes;
    let raw = Duration::from_secs_f64(FILL * cap as f64 / rate * 60.0);
    SLICE_STEPS
        .into_iter()
        .rev()
        .find(|&s| s <= raw)
        .unwrap_or(MIN_SLICE)
}

/// `ceil(uncovered / slice)`.
pub fn slices_needed(uncovered: Duration, slice: Duration) -> u32 {
    if slice.is_zero() {
        return 0;
    }
    let n = uncovered.as_secs().div_ceil(slice.as_secs().max(1));
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The backfill a truncated result needs: from its oldest flow back to
/// `window_start`. `None` when the result is complete or nothing is left.
pub fn plan_backfill(
    cov: &Coverage,
    truncated: bool,
    cap: u32,
    window_start: Timestamp,
) -> Option<BackfillPlan> {
    if !truncated {
        return None;
    }
    let oldest = cov.oldest?;
    let uncovered = uncovered(window_start, oldest)?;
    Some(BackfillPlan::with_slice(
        uncovered,
        slice_for(cov.flows, cov.covered, cap),
    ))
}

/// `oldest − window_start` when positive.
pub fn uncovered(window_start: Timestamp, oldest: Timestamp) -> Option<Duration> {
    (oldest > window_start).then(|| oldest.duration_since(window_start).unsigned_abs())
}

fn span(d: Duration) -> Span {
    Span::new().seconds(i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// What to do with the answer to the slice just fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Merge it; the walk moved on.
    Merge,
    /// It truncated: drop it and fetch the same range again in halves.
    Retry { slice: Duration },
    /// It truncated at `MIN_SLICE`: merge it and stop, the cap is too small.
    TooSmall,
}

/// Where the next slice comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    Slice(Timestamp, Timestamp),
    /// The window start is reached.
    Done,
    /// `MAX_SLICES` (or the given limit) requests made; `remaining` slices left.
    OverBudget {
        remaining: u32,
    },
}

/// A backfill in progress: slices `[end − slice, end)` walking back from
/// the oldest covered instant to the window start, one at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackfillRun {
    pub slice: Duration,
    pub window_start: Timestamp,
    /// The end of the next (or running) slice.
    pub next_end: Timestamp,
    /// Slices merged.
    pub done: u32,
    /// Requests sent, retries included.
    pub requests: u32,
    pub max_requests: u32,
}

impl BackfillRun {
    pub fn new(slice: Duration, window_start: Timestamp, from: Timestamp, max: u32) -> Self {
        Self {
            slice: slice.max(MIN_SLICE),
            window_start,
            next_end: from,
            done: 0,
            requests: 0,
            max_requests: max,
        }
    }

    /// Slices still to fetch at the current length.
    pub fn remaining(&self) -> u32 {
        uncovered(self.window_start, self.next_end).map_or(0, |u| slices_needed(u, self.slice))
    }

    /// `m` in `slice n of m`: merged plus remaining.
    pub fn total(&self) -> u32 {
        self.done + self.remaining()
    }

    /// The range the next request fetches, clipped at the window start.
    pub fn range(&self) -> Option<(Timestamp, Timestamp)> {
        if self.next_end <= self.window_start {
            return None;
        }
        let start = self
            .next_end
            .checked_sub(span(self.slice))
            .map_or(self.window_start, |s| s.max(self.window_start));
        Some((start, self.next_end))
    }

    /// Counts the request when there is one to make.
    pub fn next_slice(&mut self) -> Next {
        match self.range() {
            None => Next::Done,
            Some(_) if self.requests >= self.max_requests => Next::OverBudget {
                remaining: self.remaining(),
            },
            Some((s, e)) => {
                self.requests += 1;
                Next::Slice(s, e)
            }
        }
    }

    /// The answer to the range `next_slice` returned arrived.
    pub fn answered(&mut self, truncated: bool) -> Step {
        let half = self.slice / 2;
        if truncated && half < MIN_SLICE {
            self.advance();
            return Step::TooSmall;
        }
        if truncated {
            self.slice = half;
            return Step::Retry { slice: half };
        }
        self.advance();
        Step::Merge
    }

    fn advance(&mut self) {
        if let Some((start, _)) = self.range() {
            self.next_end = start;
            self.done += 1;
        }
    }
}

/// `4m`, `23h 56m`, `1d 2h`: the two largest non-zero units of d, h, m
/// (`<1m` under a minute).
pub fn compact_duration(d: Duration) -> String {
    let mins = d.as_secs() / 60;
    if mins == 0 {
        return "<1m".into();
    }
    let parts = [(mins / 1440, "d"), (mins / 60 % 24, "h"), (mins % 60, "m")];
    let first = parts.iter().position(|(n, _)| *n > 0).unwrap_or(2);
    parts[first..]
        .iter()
        .take(2)
        .filter(|(n, _)| *n > 0)
        .map(|(n, u)| format!("{n}{u}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `~4 minutes`, `~2 hours`, `~3 days`: the largest whole unit.
pub fn approx_duration(d: Duration) -> String {
    let s = d.as_secs();
    let (n, unit) = if s < 60 {
        return "under a minute".into();
    } else if s < 3600 {
        (s / 60, "minute")
    } else if s < 86_400 {
        (s / 3600, "hour")
    } else {
        (s / 86_400, "day")
    };
    format!("~{n} {unit}{}", if n == 1 { "" } else { "s" })
}

/// A window `parse_window` reads back: `26h`, `2d`, `90m`, `45s`.
pub fn window_text(d: Duration) -> String {
    let s = d.as_secs();
    if s > 0 && s.is_multiple_of(86_400) {
        format!("{}d", s / 86_400)
    } else if s > 0 && s.is_multiple_of(3600) {
        format!("{}h", s / 3600)
    } else if s > 0 && s.is_multiple_of(60) {
        format!("{}m", s / 60)
    } else {
        format!("{s}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::PolicyDecision;
    use crate::traffic::parse_window;

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    fn w(s: &str) -> Duration {
        parse_window(s).unwrap()
    }

    fn flow(last: Option<&str>) -> ResolvedFlow {
        ResolvedFlow {
            src: Default::default(),
            dst: Default::default(),
            port: Some(443),
            proto: "tcp".into(),
            process_name: None,
            user_name: None,
            num_connections: 1,
            policy_decision: PolicyDecision::Allowed,
            draft_policy_decision: None,
            flow_direction: None,
            first_detected: None,
            last_detected: last.map(str::to_string),
            rules: vec![],
            bytes_in: None,
            bytes_out: None,
        }
    }

    #[test]
    fn coverage_from_the_fixture_flows() {
        let raw: Vec<crate::models::Flow> =
            serde_json::from_str(include_str!("../../../../fixtures/traffic_flows.json")).unwrap();
        let flows = crate::traffic::resolve_flows(&raw, &crate::NameIndex::default());
        let c = coverage(&flows, w("24h"));
        assert_eq!(c.newest, Some(ts("2026-10-10T07:31:02Z")));
        assert_eq!(c.oldest, Some(ts("2026-10-10T06:05:00Z")));
        assert_eq!(c.covered, Duration::from_secs(3600 + 26 * 60 + 2));
        assert_eq!(c.flows, 4);
        assert!((c.share - c.covered.as_secs_f64() / 86_400.0).abs() < 1e-9);
    }

    #[test]
    fn coverage_ignores_flows_without_a_time_and_caps_the_share() {
        let flows = vec![
            flow(Some("2026-10-10T07:00:00Z")),
            flow(None),
            flow(Some("junk")),
            flow(Some("2026-10-10T05:00:00Z")),
        ];
        let c = coverage(&flows, w("1h"));
        assert_eq!(c.covered, w("2h"));
        assert_eq!(c.flows, 2);
        assert_eq!(c.share, 1.0, "never above the window");
        let none = coverage(&[flow(None)], w("1h"));
        assert_eq!(
            (none.covered, none.oldest, none.share),
            (Duration::ZERO, None, 0.0)
        );
        assert_eq!(coverage(&[], Duration::ZERO).share, 1.0);
    }

    #[test]
    fn slice_length_rounds_down_to_a_clean_step_with_a_1m_floor() {
        // 1000 flows in 4m at cap 1000: 3.2m fits, rounded down to 1m.
        assert_eq!(slice_for(1000, w("4m"), 1000), w("1m"));
        // 1000 flows in 2h30m: 0.8 × 150m = 120m.
        assert_eq!(slice_for(1000, w("150m"), 1000), w("2h"));
        // 119m rounds down to 1h.
        assert_eq!(
            slice_for(1000, Duration::from_secs(149 * 60), 1000),
            w("1h")
        );
        assert_eq!(slice_for(1000, w("25m"), 1000), w("15m"));
        assert_eq!(slice_for(1000, w("10m"), 1000), w("5m"));
        assert_eq!(slice_for(500, w("1h"), 1000), w("1h"), "0.8 × 2h = 96m");
        // Very sparse traffic: the largest step.
        assert_eq!(slice_for(10, w("1d"), 1000), w("1d"));
        // Everything within one second: the floor.
        assert_eq!(slice_for(1000, Duration::ZERO, 1000), w("1m"));
        assert_eq!(slice_for(0, Duration::ZERO, 1000), w("1d"));
    }

    #[test]
    fn plan_counts_slices_and_flags_the_limit() {
        let end = ts("2026-10-10T12:00:00Z");
        let start = ts("2026-10-09T12:00:00Z");
        let mut flows: Vec<ResolvedFlow> = (0..1000)
            .map(|_| flow(Some("2026-10-10T12:00:00Z")))
            .collect();
        flows[999] = flow(Some("2026-10-10T09:30:00Z"));
        let c = coverage(&flows, end.duration_since(start).unsigned_abs());
        let p = plan_backfill(&c, true, 1000, start).unwrap();
        assert_eq!(p.slice, w("2h"));
        assert_eq!(p.uncovered, w("21h 30m"));
        assert_eq!(p.count, 11, "ceil(21.5 / 2)");
        assert!(!p.too_many());
        assert_eq!(plan_backfill(&c, false, 1000, start), None, "not truncated");
        assert_eq!(
            plan_backfill(&c, true, 1000, ts("2026-10-10T10:00:00Z")),
            None,
            "nothing older left"
        );
        // 1000 flows in 4 minutes of 24h: 1436 one-minute slices.
        flows[999] = flow(Some("2026-10-10T11:56:00Z"));
        let c = coverage(&flows, w("24h"));
        let p = plan_backfill(&c, true, 1000, start).unwrap();
        assert_eq!((p.slice, p.count), (w("1m"), 1436));
        assert!(p.too_many());
        assert_eq!(
            BackfillPlan::with_slice(w("50m"), w("1m")).count,
            MAX_SLICES
        );
        assert!(!BackfillPlan::with_slice(w("50m"), w("1m")).too_many());
    }

    #[test]
    fn a_run_walks_back_newest_first_and_clips_at_the_window_start() {
        let start = ts("2026-10-10T00:00:00Z");
        let mut r = BackfillRun::new(w("2h"), start, ts("2026-10-10T05:00:00Z"), MAX_SLICES);
        assert_eq!(r.total(), 3);
        let mut ranges = Vec::new();
        while let Next::Slice(s, e) = r.next_slice() {
            ranges.push((s, e));
            assert_eq!(r.answered(false), Step::Merge);
        }
        assert_eq!(
            ranges,
            vec![
                (ts("2026-10-10T03:00:00Z"), ts("2026-10-10T05:00:00Z")),
                (ts("2026-10-10T01:00:00Z"), ts("2026-10-10T03:00:00Z")),
                (start, ts("2026-10-10T01:00:00Z")),
            ]
        );
        assert_eq!((r.done, r.requests, r.total()), (3, 3, 3));
        assert_eq!(r.next_slice(), Next::Done);
    }

    #[test]
    fn a_truncated_slice_halves_and_retries_its_range() {
        let start = ts("2026-10-10T00:00:00Z");
        let mut r = BackfillRun::new(w("2h"), start, ts("2026-10-10T04:00:00Z"), MAX_SLICES);
        assert_eq!(
            r.next_slice(),
            Next::Slice(ts("2026-10-10T02:00:00Z"), ts("2026-10-10T04:00:00Z"))
        );
        assert_eq!(r.answered(true), Step::Retry { slice: w("1h") });
        assert_eq!((r.done, r.total()), (0, 4));
        assert_eq!(
            r.next_slice(),
            Next::Slice(ts("2026-10-10T03:00:00Z"), ts("2026-10-10T04:00:00Z")),
            "the same range, its newer half first"
        );
        assert_eq!(r.answered(false), Step::Merge);
        assert_eq!((r.done, r.requests, r.total()), (1, 2, 4));
    }

    #[test]
    fn a_truncated_1m_slice_is_too_small() {
        let start = ts("2026-10-10T00:00:00Z");
        let mut r = BackfillRun::new(w("2m"), start, ts("2026-10-10T00:10:00Z"), MAX_SLICES);
        r.next_slice();
        assert_eq!(r.answered(true), Step::Retry { slice: w("1m") });
        r.next_slice();
        assert_eq!(r.answered(true), Step::TooSmall);
        assert_eq!(r.done, 1, "merged");
        assert_eq!(r.slice, w("1m"));
        // Never below the floor, even when asked.
        assert_eq!(BackfillRun::new(w("10s"), start, start, 1).slice, w("1m"));
    }

    #[test]
    fn the_request_budget_stops_the_run() {
        let start = ts("2026-10-10T00:00:00Z");
        let mut r = BackfillRun::new(w("1h"), start, ts("2026-10-10T05:00:00Z"), 2);
        for _ in 0..2 {
            assert!(matches!(r.next_slice(), Next::Slice(..)));
            r.answered(false);
        }
        assert_eq!(r.next_slice(), Next::OverBudget { remaining: 3 });
        assert_eq!(r.requests, 2);
    }

    #[test]
    fn durations_read_compactly_and_roughly() {
        assert_eq!(compact_duration(w("4m")), "4m");
        assert_eq!(compact_duration(w("23h 56m")), "23h 56m");
        assert_eq!(compact_duration(w("24h")), "1d");
        assert_eq!(compact_duration(w("26h 5m")), "1d 2h");
        assert_eq!(compact_duration(w("2d 5m")), "2d");
        assert_eq!(compact_duration(w("2h")), "2h");
        assert_eq!(compact_duration(w("30s")), "<1m");
        assert_eq!(approx_duration(w("4m 30s")), "~4 minutes");
        assert_eq!(approx_duration(w("1m")), "~1 minute");
        assert_eq!(approx_duration(w("23h 56m")), "~23 hours");
        assert_eq!(approx_duration(w("3d")), "~3 days");
        assert_eq!(approx_duration(w("10s")), "under a minute");
        for (d, s) in [("26h", "26h"), ("2d", "2d"), ("90m", "90m"), ("45s", "45s")] {
            assert_eq!(window_text(w(d)), s);
            assert_eq!(parse_window(&window_text(w(d))).unwrap(), w(d));
        }
    }
}
