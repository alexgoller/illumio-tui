//! Durations as the traffic views print them. Pure.
use std::time::Duration;

/// `4m`, `23h 56m`, `1d 2h`: the two largest non-zero units of d, h, m
/// (`<1m` under a minute, `0m` for none).
pub fn compact_duration(d: Duration) -> String {
    if d.is_zero() {
        return "0m".into();
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::parse_window;

    fn w(s: &str) -> Duration {
        parse_window(s).unwrap()
    }

    #[test]
    fn durations_read_compactly() {
        assert_eq!(compact_duration(w("4m")), "4m");
        assert_eq!(compact_duration(w("23h 56m")), "23h 56m");
        assert_eq!(compact_duration(w("24h")), "1d");
        assert_eq!(compact_duration(w("26h 5m")), "1d 2h");
        assert_eq!(compact_duration(w("2d 5m")), "2d");
        assert_eq!(compact_duration(w("2h")), "2h");
        assert_eq!(compact_duration(w("30s")), "<1m");
        assert_eq!(compact_duration(Duration::ZERO), "0m");
    }
}
