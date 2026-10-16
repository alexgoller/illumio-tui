//! A batch of writes whose later steps name what earlier ones create. A
//! step that needs such an href carries the placeholder `{step:N}`; the
//! executor (a later plan) puts the href in just before sending. A
//! placeholder never passes `checked_href`, so it cannot reach the wire.

/// `{step:3}`: the href step 3 creates (or finds).
pub fn step_placeholder(step: usize) -> String {
    format!("{{step:{step}}}")
}

/// The step a whole placeholder names: `{step:3}` → 3.
pub fn parse_placeholder(s: &str) -> Option<usize> {
    s.strip_prefix("{step:")?.strip_suffix('}')?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_round_trip_and_never_pass_as_hrefs() {
        assert_eq!(step_placeholder(3), "{step:3}");
        assert_eq!(parse_placeholder("{step:3}"), Some(3));
        assert_eq!(parse_placeholder("{step:x}"), None);
        assert_eq!(
            parse_placeholder("/orgs/5/sec_policy/draft/rule_sets/1"),
            None
        );
        assert!(super::super::checked_policy_href(&step_placeholder(1), "rule_sets", 5).is_err());
        assert!(super::super::checked_href(&step_placeholder(0), "ip_lists", 5).is_err());
    }
}
