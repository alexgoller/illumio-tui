//! The ladders the TUI steps the window (`+`/`-`) and the result cap
//! (`]`/`[`) along. Pure: no clock, no network.
use super::query::{CONFIRM_MAX_RESULTS, MAX_TRAFFIC_MAX_RESULTS, parse_window};
use std::time::Duration;

/// Window steps, shortest first. A window between two steps snaps to the
/// next larger (`next_window`) or next smaller (`prev_window`) one.
pub const WINDOW_STEPS: [&str; 10] = ["1h", "6h", "12h", "1d", "2d", "3d", "1w", "2w", "4w", "90d"];

/// Result cap steps, smallest first, up to the hard maximum. The profile's
/// `traffic_max_results` (the starting cap) is a rung of its own.
pub const CAP_STEPS: [u32; 9] = [
    500,
    1000,
    2000,
    5000,
    10_000,
    25_000,
    50_000,
    100_000,
    MAX_TRAFFIC_MAX_RESULTS,
];

fn step(s: &str) -> Duration {
    parse_window(s).expect("every window step parses")
}

/// The first step longer than `current`; `None` at the end of the ladder.
pub fn next_window(current: Duration) -> Option<&'static str> {
    WINDOW_STEPS.into_iter().find(|s| step(s) > current)
}

/// The last step shorter than `current`; `None` at the start of the ladder.
pub fn prev_window(current: Duration) -> Option<&'static str> {
    WINDOW_STEPS.into_iter().rev().find(|s| step(s) < current)
}

/// `CAP_STEPS` with `start` (the profile's starting cap) as a rung of its
/// own when it lies between two steps, never above the hard maximum.
fn cap_ladder(start: u32) -> impl DoubleEndedIterator<Item = u32> {
    let extra = (start > 0 && start < MAX_TRAFFIC_MAX_RESULTS && !CAP_STEPS.contains(&start))
        .then_some(start);
    let mut ladder: Vec<u32> = CAP_STEPS.into_iter().chain(extra).collect();
    ladder.sort_unstable();
    ladder.into_iter()
}

/// The first rung above `current`; `None` at the hard maximum.
pub fn next_cap(current: u32, start: u32) -> Option<u32> {
    cap_ladder(start).find(|&c| c > current)
}

/// The last rung below `current`; `None` at the bottom of the ladder.
pub fn prev_cap(current: u32, start: u32) -> Option<u32> {
    cap_ladder(start).rev().find(|&c| c < current)
}

/// The cap a backfill re-runs a truncated window at: the confirmation
/// threshold (10000) from below it, since every rung under it would be a
/// query that asks nothing, then the next `CAP_STEPS` rung (25000, 50000,
/// 100000, 200000). `None` at the hard maximum. Explorer cannot page a
/// result, so a larger cap is the only way to more of the same window.
pub fn backfill_cap(current: u32) -> Option<u32> {
    if current < CONFIRM_MAX_RESULTS {
        return Some(CONFIRM_MAX_RESULTS);
    }
    CAP_STEPS.into_iter().find(|&c| c > current)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Duration {
        parse_window(s).unwrap()
    }

    #[test]
    fn window_steps_parse_and_ascend() {
        let d: Vec<Duration> = WINDOW_STEPS.iter().map(|s| w(s)).collect();
        assert!(d.windows(2).all(|p| p[0] < p[1]), "{d:?}");
    }

    #[test]
    fn window_steps_up_and_down() {
        assert_eq!(next_window(w("1h")), Some("6h"));
        assert_eq!(next_window(w("1d")), Some("2d"));
        assert_eq!(next_window(w("24h")), Some("2d"), "24h is the 1d step");
        assert_eq!(next_window(w("4w")), Some("90d"));
        assert_eq!(next_window(w("90d")), None);
        assert_eq!(next_window(w("100d")), None);
        assert_eq!(prev_window(w("2d")), Some("1d"));
        assert_eq!(prev_window(w("6h")), Some("1h"));
        assert_eq!(prev_window(w("1h")), None);
        assert_eq!(prev_window(w("30m")), None);
        assert_eq!(prev_window(w("100d")), Some("90d"));
    }

    #[test]
    fn off_ladder_windows_snap() {
        assert_eq!(next_window(w("36h")), Some("2d"));
        assert_eq!(prev_window(w("36h")), Some("1d"));
        assert_eq!(next_window(w("5d")), Some("1w"));
        assert_eq!(prev_window(w("5d")), Some("3d"));
        assert_eq!(next_window(w("30m")), Some("1h"));
    }

    #[test]
    fn caps_step_up_to_the_hard_maximum() {
        assert!(CAP_STEPS.windows(2).all(|p| p[0] < p[1]));
        assert_eq!(next_cap(500, 10_000), Some(1000));
        assert_eq!(next_cap(1000, 10_000), Some(2000));
        assert_eq!(next_cap(5000, 10_000), Some(10_000));
        assert_eq!(
            next_cap(10_000, 10_000),
            Some(25_000),
            "the profile no longer tops the ladder"
        );
        assert_eq!(next_cap(1000, 1000), Some(2000));
        assert_eq!(next_cap(100_000, 10_000), Some(200_000));
        assert_eq!(next_cap(200_000, 10_000), None, "the hard maximum");
        assert_eq!(next_cap(200_000, 300_000), None, "never above it");
        assert_eq!(prev_cap(1000, 10_000), Some(500));
        assert_eq!(prev_cap(500, 10_000), None);
        assert_eq!(prev_cap(200_000, 10_000), Some(100_000));
        assert_eq!(prev_cap(25_000, 10_000), Some(10_000));
    }

    #[test]
    fn a_profile_cap_between_steps_is_its_own_rung() {
        assert_eq!(next_cap(2000, 3000), Some(3000));
        assert_eq!(next_cap(3000, 3000), Some(5000));
        assert_eq!(prev_cap(5000, 3000), Some(3000));
        assert_eq!(prev_cap(3000, 3000), Some(2000));
        assert_eq!(next_cap(50, 200), Some(200));
        assert_eq!(prev_cap(500, 200), Some(200));
        assert_eq!(next_cap(10_000, 30_000), Some(25_000));
        assert_eq!(next_cap(25_000, 30_000), Some(30_000));
        assert_eq!(prev_cap(50_000, 30_000), Some(30_000));
    }

    #[test]
    fn off_ladder_caps_snap() {
        assert_eq!(next_cap(750, 10_000), Some(1000));
        assert_eq!(prev_cap(750, 10_000), Some(500));
        assert_eq!(next_cap(20_000, 10_000), Some(25_000));
        assert_eq!(prev_cap(20_000, 10_000), Some(10_000));
        assert_eq!(prev_cap(300_000, 10_000), Some(200_000));
        assert_eq!(next_cap(300_000, 10_000), None);
    }

    #[test]
    fn backfill_jumps_to_10000_then_climbs_the_rungs() {
        assert_eq!(backfill_cap(200), Some(10_000));
        assert_eq!(backfill_cap(9_999), Some(10_000));
        assert_eq!(backfill_cap(10_000), Some(25_000));
        assert_eq!(backfill_cap(15_000), Some(25_000));
        assert_eq!(backfill_cap(25_000), Some(50_000));
        assert_eq!(backfill_cap(50_000), Some(100_000));
        assert_eq!(backfill_cap(100_000), Some(200_000));
        assert_eq!(backfill_cap(200_000), None);
    }
}
