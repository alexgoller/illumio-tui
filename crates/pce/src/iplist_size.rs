//! How many addresses an IP list covers, per family, with its exclusions
//! taken out. Pure std arithmetic (IPv4 as `u32`, IPv6 as `u128`); never a
//! request.
use crate::models::IpList;
use serde::{Serialize, Serializer};
use std::net::IpAddr;

/// The size of one IP list. `v4` and `v6` count addresses after exclusions;
/// `unbounded` names the included `0.0.0.0/0` or `::/0` that makes the list
/// "any"; every other list is bounded, however large. JSON carries `v4` and `v6` as strings
/// (an IPv6 count does not fit a JSON number).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct IpListSize {
    #[serde(serialize_with = "as_string")]
    pub v4: u128,
    #[serde(serialize_with = "as_string")]
    pub v6: u128,
    pub unbounded: Option<String>,
    /// Range entries, parsable or not.
    pub ranges: usize,
    pub fqdns: usize,
}

fn as_string<S: Serializer>(n: &u128, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&n.to_string())
}

/// The last IPv4 address as an integer.
const V4_LAST: u128 = u32::MAX as u128;

/// `[first, last]`, inclusive, in one family's integer space.
type Span = (u128, u128);

/// An address as `(is IPv4, integer)`.
fn int(ip: IpAddr) -> (bool, u128) {
    match ip {
        IpAddr::V4(a) => (true, u128::from(u32::from(a))),
        IpAddr::V6(a) => (false, u128::from(a)),
    }
}

/// One range entry (`A`, `A/N`, or `A` to `B`) as a span of its family;
/// `None` when it does not parse. A CIDR's host bits are ignored, as the PCE
/// does (`10.0.0.1/8` is `10.0.0.0/8`).
fn span(from: &str, to: Option<&str>) -> Option<(bool, Span)> {
    let from = from.trim();
    if let Some((addr, bits)) = from.split_once('/') {
        let (v4, start) = int(addr.trim().parse().ok()?);
        let width: u32 = if v4 { 32 } else { 128 };
        let bits: u32 = bits.trim().parse().ok()?;
        if bits > width {
            return None;
        }
        let host = width - bits;
        let mask = if host == 128 {
            u128::MAX
        } else {
            (1u128 << host) - 1
        };
        let first = start & !mask;
        return Some((v4, (first, first | mask)));
    }
    let (v4, first) = int(from.parse().ok()?);
    let last = match to {
        Some(t) => {
            let (to_v4, last) = int(t.trim().parse().ok()?);
            if to_v4 != v4 || last < first {
                return None;
            }
            last
        }
        None => first,
    };
    Some((v4, (first, last)))
}

/// Sorted, with overlapping and adjacent spans joined.
fn merge(mut spans: Vec<Span>) -> Vec<Span> {
    spans.sort_unstable();
    let mut out: Vec<Span> = Vec::with_capacity(spans.len());
    for (first, last) in spans {
        match out.last_mut() {
            Some(prev) if first <= prev.1.saturating_add(1) => prev.1 = prev.1.max(last),
            _ => out.push((first, last)),
        }
    }
    out
}

/// `included` minus every excluded span; only overlaps are taken out.
fn subtract(included: Vec<Span>, excluded: &[Span]) -> Vec<Span> {
    let mut left = included;
    for &(ex_first, ex_last) in excluded {
        let mut next = Vec::with_capacity(left.len() + 1);
        for (first, last) in left {
            if ex_last < first || ex_first > last {
                next.push((first, last));
                continue;
            }
            if ex_first > first {
                next.push((first, ex_first - 1));
            }
            if ex_last < last {
                next.push((ex_last + 1, last));
            }
        }
        left = next;
    }
    left
}

/// Addresses in `spans`, saturating at `u128::MAX` (all of IPv6).
fn count(spans: &[Span]) -> u128 {
    spans.iter().fold(0u128, |n, (first, last)| {
        n.saturating_add((last - first).saturating_add(1))
    })
}

/// `999`, `65.5K`, `1.5M`, `2.0G`. The value is rounded before the unit is
/// picked, so `999_999` is `1.0M`, never `1000.0K`.
pub(crate) fn human(n: u128) -> String {
    if n < 1_000 {
        return n.to_string();
    }
    let f = n as f64;
    for (unit, scale) in [('K', 1e3), ('M', 1e6)] {
        let tenths = (f / scale * 10.0).round();
        if tenths < 10_000.0 {
            return format!("{:.1}{unit}", tenths / 10.0);
        }
    }
    format!("{:.1}G", f / 1e9)
}

pub fn ip_list_size(list: &IpList) -> IpListSize {
    // Index 0 is IPv4, 1 is IPv6.
    let mut include: [Vec<Span>; 2] = [Vec::new(), Vec::new()];
    let mut exclude: [Vec<Span>; 2] = [Vec::new(), Vec::new()];
    let mut unbounded = None;
    for r in &list.ip_ranges {
        let Some((v4, s)) = span(&r.from_ip, r.to_ip.as_deref()) else {
            continue;
        };
        let family = usize::from(!v4);
        if r.exclusion.unwrap_or(false) {
            exclude[family].push(s);
            continue;
        }
        if unbounded.is_none() && s == (0, if v4 { V4_LAST } else { u128::MAX }) {
            unbounded = Some(if v4 { "0.0.0.0/0" } else { "::/0" }.to_string());
        }
        include[family].push(s);
    }
    let [inc_v4, inc_v6] = include;
    let v4 = count(&subtract(merge(inc_v4), &exclude[0]));
    let v6 = count(&subtract(merge(inc_v6), &exclude[1]));
    IpListSize {
        v4,
        v6,
        unbounded,
        ranges: list.ip_ranges.len(),
        fqdns: list.fqdns.len(),
    }
}

impl IpListSize {
    /// IPv4 plus IPv6 addresses, saturating.
    pub fn total(&self) -> u128 {
        self.v4.saturating_add(self.v6)
    }

    /// A size column's sort key: the address count, unbounded lists last.
    pub fn sort_key(&self) -> u64 {
        if self.unbounded.is_some() {
            return u64::MAX;
        }
        u64::try_from(self.total()).map_or(u64::MAX - 1, |n| n.min(u64::MAX - 1))
    }

    /// `254`, `65.5K`, `1.0M`, `any (0.0.0.0/0)`, `3 FQDNs` when it has no ranges.
    pub fn display(&self) -> String {
        if let Some(why) = &self.unbounded {
            return format!("any ({why})");
        }
        if self.ranges == 0 && self.fqdns > 0 {
            return if self.fqdns == 1 {
                "1 FQDN".to_string()
            } else {
                format!("{} FQDNs", self.fqdns)
            };
        }
        human(self.total())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::IpList;
    use serde_json::{Value, json};

    fn list(ranges: Value, fqdns: Value) -> IpList {
        serde_json::from_value(json!({
            "href": "/orgs/5/sec_policy/draft/ip_lists/9", "name": "T",
            "ip_ranges": ranges, "fqdns": fqdns
        }))
        .unwrap()
    }

    fn size(ranges: Value) -> IpListSize {
        ip_list_size(&list(ranges, json!([])))
    }

    fn fixture() -> Vec<IpList> {
        serde_json::from_str(include_str!("../../../fixtures/ip_lists.json")).unwrap()
    }

    #[test]
    fn cidrs_count_per_family_and_ignore_host_bits() {
        let s = size(json!([{"from_ip": "10.0.0.0/24"}, {"from_ip": "2001:db8::/120"}]));
        assert_eq!((s.v4, s.v6), (256, 256));
        assert_eq!(s.display(), "512");
        assert_eq!(
            size(json!([{"from_ip": "10.0.0.1/8"}])).v4,
            1 << 24,
            "host bits are ignored, as the PCE does"
        );
        assert_eq!(size(json!([{"from_ip": "10.0.0.7/32"}])).v4, 1);
    }

    #[test]
    fn ranges_are_inclusive_and_single_addresses_count_one() {
        let s = size(json!([{"from_ip": "10.1.0.0", "to_ip": "10.1.255.255"}]));
        assert_eq!(s.v4, 65_536);
        assert_eq!(s.display(), "65.5K");
        let s = size(json!([
            {"from_ip": "10.0.0.1", "to_ip": "10.0.0.254"},
            {"from_ip": "fe80::1"}
        ]));
        assert_eq!((s.v4, s.v6), (254, 1));
        assert_eq!(size(json!([{"from_ip": "10.0.0.9"}])).display(), "1");
    }

    #[test]
    fn exclusions_subtract_only_their_overlap_after_merging() {
        // Two overlapping includes count once; the exclusion takes its half.
        let s = size(json!([
            {"from_ip": "10.0.0.0/24"},
            {"from_ip": "10.0.0.128/25"},
            {"from_ip": "10.0.0.0/25", "exclusion": true}
        ]));
        assert_eq!(s.v4, 128);
        // An exclusion elsewhere, or in the other family, takes nothing.
        let s = size(json!([
            {"from_ip": "10.0.0.0/24"},
            {"from_ip": "10.1.0.0/24", "exclusion": true},
            {"from_ip": "::/0", "exclusion": true}
        ]));
        assert_eq!((s.v4, s.v6), (256, 0));
        // Across the edge of a range, and inside one (it splits in two).
        let s = size(json!([
            {"from_ip": "10.0.0.10", "to_ip": "10.0.0.19"},
            {"from_ip": "10.0.0.15", "to_ip": "10.0.0.30", "exclusion": true}
        ]));
        assert_eq!(s.v4, 5);
        let s = size(json!([
            {"from_ip": "10.0.0.0/24"},
            {"from_ip": "10.0.0.100", "to_ip": "10.0.0.109", "exclusion": true}
        ]));
        assert_eq!(s.v4, 246);
        // More excluded than included clamps at zero.
        let s = size(json!([
            {"from_ip": "10.0.0.5"},
            {"from_ip": "10.0.0.0/8", "exclusion": true}
        ]));
        assert_eq!(s.v4, 0);
        assert_eq!(s.display(), "0");
    }

    #[test]
    fn only_everything_is_unbounded_and_big_lists_keep_their_size() {
        let any4 = size(json!([{"from_ip": "0.0.0.0/0"}]));
        assert_eq!(any4.unbounded.as_deref(), Some("0.0.0.0/0"));
        assert_eq!(any4.v4, 1 << 32);
        assert_eq!(any4.display(), "any (0.0.0.0/0)");
        assert_eq!(any4.sort_key(), u64::MAX);
        let any6 = size(json!([{"from_ip": "::/0"}]));
        assert_eq!(any6.unbounded.as_deref(), Some("::/0"));
        assert_eq!(any6.v6, u128::MAX, "2^128 saturates");
        // The recorded "internet" shape: everything minus a private range is still any.
        let internet = size(json!([
            {"from_ip": "0.0.0.0/0", "exclusion": false},
            {"from_ip": "10.0.0.1/8", "exclusion": true}
        ]));
        assert_eq!(internet.unbounded.as_deref(), Some("0.0.0.0/0"));
        assert_eq!(internet.v4, (1 << 32) - (1 << 24));
        // Only /0 is any: every bounded list shows its real size.
        let a8 = size(json!([{"from_ip": "10.0.0.0/8"}]));
        assert_eq!(a8.unbounded, None);
        assert_eq!(a8.display(), "16.8M");
        let a1 = size(json!([{"from_ip": "0.0.0.0/1"}]));
        assert_eq!(a1.unbounded, None);
        assert_eq!(a1.display(), "2.1G");
        let a9 = size(json!([{"from_ip": "10.0.0.0/9"}]));
        assert_eq!(a9.unbounded, None);
        assert_eq!(a9.display(), "8.4M");
        assert!(a8.sort_key() > a9.sort_key());
        assert!(any4.sort_key() > a1.sort_key(), "any sorts last");
        assert_eq!(a9.sort_key(), 1 << 23);
        let rfc1918 = size(json!([
            {"from_ip": "10.0.0.0/8"},
            {"from_ip": "172.16.0.0/12"},
            {"from_ip": "192.168.0.0/16"}
        ]));
        assert_eq!(rfc1918.unbounded, None);
        assert_eq!(rfc1918.v4, 17_891_328);
        assert_eq!(rfc1918.display(), "17.9M");
    }

    #[test]
    fn fqdn_only_lists_and_parse_errors() {
        let three = ip_list_size(&list(
            json!([]),
            json!([{"fqdn": "a.example"}, {"fqdn": "b.example"}, {"fqdn": "c.example"}]),
        ));
        assert_eq!(
            (three.v4, three.v6, three.ranges, three.fqdns),
            (0, 0, 0, 3)
        );
        assert_eq!(three.display(), "3 FQDNs");
        let one = ip_list_size(&list(json!([]), json!([{"fqdn": "a.example"}])));
        assert_eq!(one.display(), "1 FQDN");
        let bad = size(json!([
            {"from_ip": "not-an-ip"},
            {"from_ip": "10.0.0.0/33"},
            {"from_ip": "10.0.0.9", "to_ip": "10.0.0.1"},
            {"from_ip": "10.0.0.1", "to_ip": "::1"},
            {"from_ip": "10.0.0.1"}
        ]));
        assert_eq!(
            (bad.v4, bad.v6, bad.ranges),
            (1, 0, 5),
            "unparsable entries count 0 but are ranges"
        );
        assert_eq!(size(json!([])).display(), "0");
    }

    #[test]
    fn the_fixture_lists_and_human_units() {
        let lists = fixture();
        assert_eq!(ip_list_size(&lists[0]).display(), "any (0.0.0.0/0)");
        let corp = ip_list_size(&lists[1]);
        assert_eq!(
            (corp.v4, corp.v6, corp.ranges, corp.fqdns),
            (65_536, 0, 1, 1)
        );
        assert_eq!(corp.total(), 65_536);
        assert_eq!(corp.display(), "65.5K");
        assert_eq!(human(999), "999");
        assert_eq!(human(1_000), "1.0K");
        assert_eq!(human(1_500_000), "1.5M");
        assert_eq!(human(2_000_000_000), "2.0G");
    }

    #[test]
    fn human_rounds_before_it_picks_the_unit() {
        assert_eq!(human(999_999), "1.0M");
        assert_eq!(human(999_950), "1.0M");
        assert_eq!(human(999_949), "999.9K");
        assert_eq!(human(999_960), "1.0M");
        assert_eq!(human(999_999_999), "1.0G");
        assert_eq!(human(1_000_000), "1.0M");
        assert!(human(u128::MAX).ends_with('G'));
    }

    #[test]
    fn json_carries_the_counts_as_strings() {
        let lists = fixture();
        assert_eq!(
            serde_json::to_value(ip_list_size(&lists[1])).unwrap(),
            json!({"v4": "65536", "v6": "0", "unbounded": null, "ranges": 1, "fqdns": 1})
        );
        let any = serde_json::to_value(ip_list_size(&lists[0])).unwrap();
        assert_eq!(any["v6"], json!(u128::MAX.to_string()));
        assert_eq!(any["unbounded"], "0.0.0.0/0");
    }
}
