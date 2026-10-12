//! The ladders the TUI steps the window (`+`/`-`) and the result cap
//! (`]`/`[`) along. Pure: no clock, no network.
use super::query::parse_window;
use std::time::Duration;

/// Window steps, shortest first. A window between two steps snaps to the
/// next larger (`next_window`) or next smaller (`prev_window`) one.
pub const WINDOW_STEPS: [&str; 10] = ["1h", "6h", "12h", "1d", "2d", "3d", "1w", "2w", "4w", "90d"];

/// Result cap steps, smallest first. The profile's `traffic_max_results`
/// tops the ladder: steps at or above it are left out and it is the last rung.
pub const CAP_STEPS: [u32; 5] = [500, 1000, 2000, 5000, 10_000];

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

fn cap_ladder(profile_max: u32) -> impl DoubleEndedIterator<Item = u32> {
    CAP_STEPS
        .into_iter()
        .filter(move |&c| c < profile_max)
        .chain(std::iter::once(profile_max))
}

/// The first rung above `current`; `None` when `current` is at (or above)
/// the profile maximum.
pub fn next_cap(current: u32, profile_max: u32) -> Option<u32> {
    cap_ladder(profile_max).find(|&c| c > current)
}

/// The last rung below `current`; `None` at the bottom of the ladder.
pub fn prev_cap(current: u32, profile_max: u32) -> Option<u32> {
    cap_ladder(profile_max).rev().find(|&c| c < current)
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
    fn caps_step_up_to_the_profile_maximum() {
        assert_eq!(next_cap(500, 10_000), Some(1000));
        assert_eq!(next_cap(1000, 10_000), Some(2000));
        assert_eq!(next_cap(5000, 10_000), Some(10_000));
        assert_eq!(next_cap(10_000, 10_000), None);
        assert_eq!(
            next_cap(10_000, 50_000),
            Some(50_000),
            "the profile tops the ladder"
        );
        assert_eq!(next_cap(50_000, 50_000), None);
        assert_eq!(next_cap(2000, 3000), Some(3000), "never above the profile");
        assert_eq!(next_cap(1000, 1000), None);
        assert_eq!(next_cap(50, 200), Some(200));
        assert_eq!(prev_cap(1000, 10_000), Some(500));
        assert_eq!(prev_cap(500, 10_000), None);
        assert_eq!(prev_cap(50_000, 50_000), Some(10_000));
        assert_eq!(prev_cap(3000, 3000), Some(2000));
    }

    #[test]
    fn off_ladder_caps_snap() {
        assert_eq!(next_cap(750, 10_000), Some(1000));
        assert_eq!(prev_cap(750, 10_000), Some(500));
        assert_eq!(
            prev_cap(20_000, 3000),
            Some(3000),
            "above the profile: back to it"
        );
        assert_eq!(next_cap(20_000, 3000), None);
    }
}
