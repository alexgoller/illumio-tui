use serde::{Deserialize, Serialize};

/// A PCE object reference: `{"href": "/orgs/5/labels/7"}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Ref {
    pub href: String,
}

impl Ref {
    pub fn new(href: impl Into<String>) -> Self {
        Self { href: href.into() }
    }

    /// Last path segment.
    pub fn id(&self) -> &str {
        self.href.rsplit('/').next().unwrap_or("")
    }

    /// Segment before the id, e.g. `labels`, `rule_sets`, `ip_lists`.
    pub fn object_type(&self) -> &str {
        let mut parts = self.href.rsplit('/');
        let _id = parts.next();
        match parts.next() {
            Some(t) if !t.is_empty() && self.href.contains('/') => t,
            _ => "",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyDecision {
    Allowed,
    PotentiallyBlocked,
    Blocked,
    #[serde(other)]
    Unknown,
}

impl PolicyDecision {
    pub const ALL: [PolicyDecision; 3] = [
        PolicyDecision::Allowed,
        PolicyDecision::PotentiallyBlocked,
        PolicyDecision::Blocked,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            PolicyDecision::Allowed => "allowed",
            PolicyDecision::PotentiallyBlocked => "potentially_blocked",
            PolicyDecision::Blocked => "blocked",
            PolicyDecision::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "allowed" => Some(PolicyDecision::Allowed),
            "potentially_blocked" | "potentially-blocked" | "pb" => {
                Some(PolicyDecision::PotentiallyBlocked)
            }
            "blocked" => Some(PolicyDecision::Blocked),
            _ => None,
        }
    }
}

pub fn proto_name(proto: i64) -> String {
    match proto {
        6 => "tcp".into(),
        17 => "udp".into(),
        1 => "icmp".into(),
        58 => "icmpv6".into(),
        -1 => "any".into(),
        n => n.to_string(),
    }
}

pub fn proto_number(name: &str) -> Option<u8> {
    match name.trim().to_ascii_lowercase().as_str() {
        "tcp" => Some(6),
        "udp" => Some(17),
        "icmp" => Some(1),
        "icmpv6" => Some(58),
        n => n.parse().ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_parses_type_and_id() {
        let r = Ref::new("/orgs/5/sec_policy/draft/rule_sets/100");
        assert_eq!(r.id(), "100");
        assert_eq!(r.object_type(), "rule_sets");
        assert_eq!(Ref::new("/orgs/5/labels/7").object_type(), "labels");
        assert_eq!(Ref::new("weird").object_type(), "");
    }

    #[test]
    fn policy_decision_round_trips_and_tolerates_unknown() {
        let d: Vec<PolicyDecision> =
            serde_json::from_str(r#"["allowed","potentially_blocked","blocked","something_new"]"#)
                .unwrap();
        assert_eq!(
            d,
            vec![
                PolicyDecision::Allowed,
                PolicyDecision::PotentiallyBlocked,
                PolicyDecision::Blocked,
                PolicyDecision::Unknown
            ]
        );
        assert_eq!(
            serde_json::to_string(&PolicyDecision::PotentiallyBlocked).unwrap(),
            "\"potentially_blocked\""
        );
        assert_eq!(
            PolicyDecision::parse("blocked"),
            Some(PolicyDecision::Blocked)
        );
        assert_eq!(PolicyDecision::parse("nope"), None);
    }

    #[test]
    fn proto_names() {
        assert_eq!(proto_name(6), "tcp");
        assert_eq!(proto_name(17), "udp");
        assert_eq!(proto_name(1), "icmp");
        assert_eq!(proto_name(-1), "any");
        assert_eq!(proto_name(47), "47");
        assert_eq!(proto_number("TCP"), Some(6));
        assert_eq!(proto_number("17"), Some(17));
        assert_eq!(proto_number("x"), None);
    }
}
