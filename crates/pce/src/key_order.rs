//! The order label keys are shown in, everywhere a workload's or an
//! endpoint's labels are printed: the profile's scope keys, then its tier
//! key, then every other key alphabetically. Pure data; never makes requests.
use crate::resolve::ResolvedLabel;
use std::cmp::Ordering;

/// The key [`crate::NameIndex::resolve_labels`] gives a label href it cannot
/// resolve.
const UNRESOLVED: &str = "?";

/// The order label keys are shown in: the profile's scope keys, then its
/// tier key, then every other key alphabetically; `?` (unresolved) last.
/// Also carries the keys hidden from label text ([`KeyOrder::shows`]); they
/// still sort like any other key.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyOrder {
    first: Vec<String>,
    ignored: Vec<String>,
}

impl KeyOrder {
    /// `scope_keys` in order, then `tier_key`; blanks (after trim) and
    /// repeats dropped.
    pub fn new(scope_keys: &[String], tier_key: &str) -> Self {
        let mut first: Vec<String> = Vec::new();
        for k in scope_keys.iter().map(String::as_str).chain([tier_key]) {
            let k = k.trim();
            if !k.is_empty() && k != UNRESOLVED && !first.iter().any(|f| f == k) {
                first.push(k.to_string());
            }
        }
        Self {
            first,
            ignored: Vec::new(),
        }
    }

    /// Hides `keys` from label text. Blanks (after trim), `?`, repeats and
    /// keys in [`KeyOrder::first`] (the label model) are dropped.
    pub fn with_ignored(mut self, keys: &[String]) -> Self {
        let mut ignored: Vec<String> = Vec::new();
        for k in keys {
            let k = k.trim();
            if !k.is_empty()
                && k != UNRESOLVED
                && !self.first.iter().any(|f| f == k)
                && !ignored.iter().any(|i| i == k)
            {
                ignored.push(k.to_string());
            }
        }
        self.ignored = ignored;
        self
    }

    /// The keys hidden from label text, in the order given.
    pub fn ignored(&self) -> &[String] {
        &self.ignored
    }

    /// Whether label text shows `key`: `true` unless it is ignored (exact,
    /// case-sensitive match); `?` is always shown.
    pub fn shows(&self, key: &str) -> bool {
        !self.ignored.iter().any(|i| i == key)
    }

    /// `keys` first, in order, and nothing else (a tier key is just the
    /// last of them).
    pub fn from_keys(keys: &[String]) -> Self {
        Self::new(keys, "")
    }

    /// The keys that come first, in order.
    pub fn first(&self) -> &[String] {
        &self.first
    }

    /// Position in `first`, else `first.len()`; `?` ranks after everything.
    /// Keys with the same rank sort alphabetically.
    pub fn rank(&self, key: &str) -> usize {
        if key == UNRESOLVED {
            return self.first.len() + 1;
        }
        self.first
            .iter()
            .position(|f| f == key)
            .unwrap_or(self.first.len())
    }

    /// By rank, then alphabetically ignoring case (`bu` before `DFIRBubble`),
    /// with the exact bytes as the last tiebreak so the order is total.
    pub fn cmp_keys(&self, a: &str, b: &str) -> Ordering {
        self.rank(a)
            .cmp(&self.rank(b))
            .then_with(|| a.to_lowercase().cmp(&b.to_lowercase()))
            .then_with(|| a.cmp(b))
    }

    /// By [`KeyOrder::cmp_keys`] on the key, then by value.
    pub fn cmp_kv(&self, a: (&str, &str), b: (&str, &str)) -> Ordering {
        self.cmp_keys(a.0, b.0).then_with(|| a.1.cmp(b.1))
    }

    /// Sorts by [`KeyOrder::cmp_kv`].
    pub fn sort_labels(&self, labels: &mut [ResolvedLabel]) {
        labels.sort_by(|a, b| self.cmp_kv((&a.key, &a.value), (&b.key, &b.value)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(k: &[&str]) -> Vec<String> {
        k.iter().map(|s| s.to_string()).collect()
    }

    fn sorted(order: &KeyOrder, k: &[&str]) -> Vec<String> {
        let mut out = keys(k);
        out.sort_by(|a, b| order.cmp_keys(a, b));
        out
    }

    #[test]
    fn the_rest_is_alphabetical_ignoring_case() {
        let order = KeyOrder::new(&keys(&["app"]), "role");
        assert_eq!(
            sorted(&order, &["DFIRBubble", "role", "bu", "Zone", "app", "loc"]),
            keys(&["app", "role", "bu", "DFIRBubble", "loc", "Zone"])
        );
        assert_eq!(
            order.cmp_keys("bu", "BU"),
            Ordering::Greater,
            "a total order"
        );
    }

    #[test]
    fn scope_keys_then_tier_key_then_alphabetical() {
        let order = KeyOrder::new(&keys(&["env", "app"]), "role");
        assert_eq!(order.first(), keys(&["env", "app", "role"]));
        assert_eq!(
            sorted(&order, &["os", "role", "bu", "app", "loc", "env"]),
            keys(&["env", "app", "role", "bu", "loc", "os"])
        );
    }

    #[test]
    fn a_tier_key_that_is_already_a_scope_key_is_not_repeated() {
        let order = KeyOrder::new(&keys(&["app", "role", "app"]), "role");
        assert_eq!(order.first(), keys(&["app", "role"]));
    }

    #[test]
    fn blank_keys_are_dropped() {
        let order = KeyOrder::new(&keys(&["", " app ", "  "]), " ");
        assert_eq!(order.first(), keys(&["app"]));
        assert_eq!(KeyOrder::new(&[], "").first(), Vec::<String>::new());
    }

    #[test]
    fn unresolved_key_sorts_last() {
        let order = KeyOrder::new(&keys(&["app"]), "role");
        assert_eq!(
            sorted(&order, &["?", "zone", "role", "app"]),
            keys(&["app", "role", "zone", "?"])
        );
        assert_eq!(
            sorted(&KeyOrder::default(), &["?", "app"]),
            keys(&["app", "?"])
        );
    }

    #[test]
    fn default_order_is_alphabetical() {
        let order = KeyOrder::default();
        assert!(order.first().is_empty());
        assert_eq!(
            sorted(&order, &["role", "app", "loc", "env"]),
            keys(&["app", "env", "loc", "role"])
        );
    }

    #[test]
    fn shows_hides_only_the_ignored_keys_exactly() {
        let order =
            KeyOrder::new(&keys(&["app"]), "role").with_ignored(&keys(&["DFIRBubble", "os"]));
        assert_eq!(order.ignored(), keys(&["DFIRBubble", "os"]));
        assert!(!order.shows("DFIRBubble"));
        assert!(!order.shows("os"));
        assert!(order.shows("dfirbubble"), "case-sensitive");
        assert!(order.shows("app"));
        assert!(order.shows("loc"));
        assert!(order.shows("?"));
        assert_eq!(
            sorted(&order, &["os", "app", "bu"]),
            keys(&["app", "bu", "os"]),
            "ignored keys still sort"
        );
    }

    #[test]
    fn with_ignored_drops_first_members_unresolved_blanks_and_repeats() {
        let order = KeyOrder::new(&keys(&["app", "env"]), "role")
            .with_ignored(&keys(&["app", "?", " ", " os ", "role", "os", "bu"]));
        assert_eq!(order.ignored(), keys(&["os", "bu"]));
        assert!(order.shows("app"));
        assert!(order.shows("role"));
        assert!(order.shows("?"));
    }

    #[test]
    fn default_shows_every_key() {
        let order = KeyOrder::default();
        assert!(order.ignored().is_empty());
        for k in ["app", "os", "DFIRBubble", "?", ""] {
            assert!(order.shows(k), "{k}");
        }
    }

    #[test]
    fn sort_labels_orders_by_key_then_value() {
        let l = |k: &str, v: &str| ResolvedLabel {
            href: format!("/orgs/1/labels/{k}{v}"),
            key: k.into(),
            value: v.into(),
        };
        let mut labels = vec![
            l("bu", "x"),
            l("role", "web"),
            l("role", "db"),
            l("?", "/h"),
        ];
        KeyOrder::new(&keys(&["role"]), "").sort_labels(&mut labels);
        let text: Vec<String> = labels
            .iter()
            .map(|l| format!("{}={}", l.key, l.value))
            .collect();
        assert_eq!(text, keys(&["role=db", "role=web", "bu=x", "?=/h"]));
    }
}
