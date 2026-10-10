//! Human spellings of IP ranges, service ports and workload interfaces, and
//! their PCE JSON forms. Parsers never touch the network.
use crate::models::{Interface, IpRange, ServicePort, proto_name, proto_number};
use serde_json::{Value, json};
use std::net::IpAddr;

fn parse_ip(s: &str) -> Result<IpAddr, String> {
    s.parse::<IpAddr>()
        .map_err(|_| format!("{s:?} is not an IP address"))
}

/// `addr/N` → (addr, N) with N checked against the address family.
fn parse_cidr(s: &str) -> Result<(IpAddr, u8), String> {
    let (a, n) = s
        .split_once('/')
        .ok_or_else(|| format!("{s:?} is not a CIDR"))?;
    let ip = parse_ip(a)?;
    let bits: u8 = n
        .parse()
        .map_err(|_| format!("{s:?}: prefix length is not a number"))?;
    let max = if ip.is_ipv4() { 32 } else { 128 };
    if bits > max {
        return Err(format!("{s:?}: prefix length must be at most {max}"));
    }
    Ok((ip, bits))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpRangeSpec {
    /// A single address or a CIDR (`10.0.0.0/8`), as the PCE stores it.
    pub from_ip: String,
    pub to_ip: Option<String>,
    pub exclusion: bool,
    /// Per-range comment; kept so a full PUT does not erase it.
    pub description: Option<String>,
}

impl IpRangeSpec {
    /// `A`, `A-B` or `A/N`, IPv4 or IPv6.
    pub fn parse(s: &str, exclusion: bool) -> Result<Self, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("empty IP range".into());
        }
        if s.contains('/') {
            if s.contains('-') {
                return Err(format!("{s:?}: a CIDR cannot also be a range"));
            }
            let (ip, bits) = parse_cidr(s)?;
            return Ok(Self {
                from_ip: format!("{ip}/{bits}"),
                to_ip: None,
                exclusion,
                description: None,
            });
        }
        if let Some((a, b)) = s.split_once('-') {
            let from = parse_ip(a.trim()).map_err(|e| format!("{s:?}: {e}"))?;
            let to = parse_ip(b.trim()).map_err(|e| format!("{s:?}: {e}"))?;
            if from.is_ipv4() != to.is_ipv4() {
                return Err(format!("{s:?}: mixed address families"));
            }
            if to < from {
                return Err(format!("{s:?}: range end is before its start"));
            }
            return Ok(Self {
                from_ip: from.to_string(),
                to_ip: Some(to.to_string()),
                exclusion,
                description: None,
            });
        }
        let ip = parse_ip(s)?;
        Ok(Self {
            from_ip: ip.to_string(),
            to_ip: None,
            exclusion,
            description: None,
        })
    }

    pub fn from_range(r: &IpRange) -> Self {
        Self {
            from_ip: r.from_ip.clone(),
            to_ip: r.to_ip.clone(),
            exclusion: r.exclusion.unwrap_or(false),
            description: r.description.clone(),
        }
    }

    pub fn to_json(&self) -> Value {
        let mut v = json!({"from_ip": self.from_ip});
        if let Some(to) = &self.to_ip {
            v["to_ip"] = json!(to);
        }
        if self.exclusion {
            v["exclusion"] = json!(true);
        }
        if let Some(d) = &self.description {
            v["description"] = json!(d);
        }
        v
    }

    pub fn display(&self) -> String {
        let excl = if self.exclusion { "!" } else { "" };
        match &self.to_ip {
            Some(to) => format!("{excl}{}-{to}", self.from_ip),
            None => format!("{excl}{}", self.from_ip),
        }
    }

    /// Identity ignores the description.
    pub fn same_range(&self, other: &Self) -> bool {
        self.from_ip == other.from_ip
            && self.to_ip == other.to_ip
            && self.exclusion == other.exclusion
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortSpec {
    pub port: Option<u16>,
    pub to_port: Option<u16>,
    pub proto: i64,
}

impl PortSpec {
    /// `P/PROTO`, `P-Q/PROTO`, or a bare protocol (`icmp`, `udp`, `47`).
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("empty port spec".into());
        }
        let (ports, proto) = match s.split_once('/') {
            Some((p, pr)) => (Some(p.trim()), pr.trim()),
            None => (None, s),
        };
        if proto.is_empty() {
            return Err(format!("{s:?}: missing protocol"));
        }
        let proto = i64::from(
            proto_number(proto).ok_or_else(|| format!("{s:?}: unknown protocol {proto:?}"))?,
        );
        let Some(ports) = ports else {
            return Ok(Self {
                port: None,
                to_port: None,
                proto,
            });
        };
        if ports.is_empty() {
            return Err(format!("{s:?}: missing port"));
        }
        if matches!(proto, 1 | 58) {
            return Err(format!("{s:?}: ICMP has no ports"));
        }
        let parse_port = |p: &str| {
            p.trim()
                .parse::<u16>()
                .map_err(|_| format!("{s:?}: {p:?} is not a port (0-65535)"))
        };
        let (port, to_port) = match ports.split_once('-') {
            Some((a, b)) => {
                let a = parse_port(a)?;
                let b = parse_port(b)?;
                if b < a {
                    return Err(format!("{s:?}: port range end is before its start"));
                }
                (a, if b == a { None } else { Some(b) })
            }
            None => (parse_port(ports)?, None),
        };
        Ok(Self {
            port: Some(port),
            to_port,
            proto,
        })
    }

    pub fn from_service_port(p: &ServicePort) -> Option<Self> {
        Some(Self {
            port: p.port,
            to_port: p.to_port.filter(|t| Some(*t) != p.port),
            proto: p.proto?,
        })
    }

    pub fn to_json(&self) -> Value {
        let mut v = json!({"proto": self.proto});
        if let Some(p) = self.port {
            v["port"] = json!(p);
        }
        if let Some(t) = self.to_port {
            v["to_port"] = json!(t);
        }
        v
    }

    pub fn display(&self) -> String {
        let proto = proto_name(self.proto);
        match (self.port, self.to_port) {
            (Some(p), Some(t)) => format!("{p}-{t}/{proto}"),
            (Some(p), None) => format!("{p}/{proto}"),
            (None, _) => proto,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceSpec {
    pub name: String,
    pub address: String,
    pub cidr_block: Option<u8>,
}

impl InterfaceSpec {
    /// `ADDR` or `ADDR/N`.
    pub fn parse(s: &str, name: String) -> Result<Self, String> {
        let s = s.trim();
        if s.contains('/') {
            let (ip, bits) = parse_cidr(s)?;
            return Ok(Self {
                name,
                address: ip.to_string(),
                cidr_block: Some(bits),
            });
        }
        let ip = parse_ip(s)?;
        Ok(Self {
            name,
            address: ip.to_string(),
            cidr_block: None,
        })
    }

    pub fn from_interface(i: &Interface) -> Self {
        Self {
            name: i.name.clone(),
            address: i.address.clone(),
            cidr_block: i.cidr_block,
        }
    }

    pub fn to_json(&self) -> Value {
        let mut v = json!({"name": self.name, "address": self.address});
        if let Some(c) = self.cidr_block {
            v["cidr_block"] = json!(c);
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ip_ranges_parse_single_range_and_cidr() {
        let r = IpRangeSpec::parse("10.1.2.3", false).unwrap();
        assert_eq!(r.to_json(), json!({"from_ip": "10.1.2.3"}));
        let r = IpRangeSpec::parse("10.1.0.0-10.1.255.255", false).unwrap();
        assert_eq!(
            r.to_json(),
            json!({"from_ip": "10.1.0.0", "to_ip": "10.1.255.255"})
        );
        let r = IpRangeSpec::parse(" 2001:db8::1 ", true).unwrap();
        assert_eq!(
            r.to_json(),
            json!({"from_ip": "2001:db8::1", "exclusion": true})
        );
        assert_eq!(r.display(), "!2001:db8::1");
    }

    #[test]
    fn cidr_stays_in_from_ip() {
        let r = IpRangeSpec::parse("10.0.0.0/8", false).unwrap();
        assert_eq!(r.to_json(), json!({"from_ip": "10.0.0.0/8"}));
        assert_eq!(r.display(), "10.0.0.0/8");
        assert!(IpRangeSpec::parse("10.0.0.0/33", false).is_err());
        assert!(IpRangeSpec::parse("2001:db8::/129", false).is_err());
    }

    #[test]
    fn bad_ip_ranges_are_rejected_with_the_input_in_the_message() {
        for bad in [
            "",
            "10.1",
            "10.1.0.0-",
            "a-b",
            "10.0.0.2-10.0.0.1",
            "10.0.0.0/8-10.0.0.9",
        ] {
            let err = IpRangeSpec::parse(bad, false).unwrap_err();
            assert!(err.contains(bad.trim()) || bad.is_empty(), "{bad:?}: {err}");
        }
    }

    #[test]
    fn same_range_ignores_nothing() {
        let a = IpRangeSpec::parse("10.0.0.0/8", false).unwrap();
        let b = IpRangeSpec::parse("10.0.0.0/8", true).unwrap();
        assert!(!a.same_range(&b), "exclusion is part of identity");
        assert!(a.same_range(&IpRangeSpec::parse("10.0.0.0/8", false).unwrap()));
        let from = IpRangeSpec::from_range(&crate::models::IpRange {
            from_ip: "10.0.0.0/8".into(),
            to_ip: None,
            exclusion: Some(false),
            description: None,
        });
        assert!(a.same_range(&from));
    }

    #[test]
    fn ports_parse_every_form() {
        assert_eq!(
            PortSpec::parse("443/tcp").unwrap().to_json(),
            json!({"port": 443, "proto": 6})
        );
        assert_eq!(
            PortSpec::parse("8000-8100/udp").unwrap().to_json(),
            json!({"port": 8000, "to_port": 8100, "proto": 17})
        );
        assert_eq!(
            PortSpec::parse("icmp").unwrap().to_json(),
            json!({"proto": 1})
        );
        assert_eq!(
            PortSpec::parse("ICMPv6").unwrap().to_json(),
            json!({"proto": 58})
        );
        assert_eq!(
            PortSpec::parse("tcp").unwrap().to_json(),
            json!({"proto": 6})
        );
        assert_eq!(
            PortSpec::parse("47").unwrap().to_json(),
            json!({"proto": 47})
        );
        assert_eq!(PortSpec::parse("53/17").unwrap().display(), "53/udp");
        assert_eq!(
            PortSpec::parse("8000-8100/tcp").unwrap().display(),
            "8000-8100/tcp"
        );
    }

    #[test]
    fn bad_ports_are_rejected() {
        for bad in [
            "",
            "443",
            "443/",
            "/tcp",
            "443/sctpx",
            "9-1/tcp",
            "70000/tcp",
            "443/icmp",
        ] {
            assert!(PortSpec::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn port_round_trips_from_service_port() {
        let sp = crate::models::ServicePort {
            port: Some(5432),
            to_port: None,
            proto: Some(6),
            icmp_type: None,
            icmp_code: None,
        };
        let p = PortSpec::from_service_port(&sp).unwrap();
        assert_eq!(p.display(), "5432/tcp");
        assert_eq!(p, PortSpec::parse("5432/tcp").unwrap());
        let none = crate::models::ServicePort { proto: None, ..sp };
        assert!(PortSpec::from_service_port(&none).is_none());
    }

    #[test]
    fn interfaces_parse_address_and_prefix() {
        let i = InterfaceSpec::parse("10.0.0.5/24", "eth0".into()).unwrap();
        assert_eq!(
            i.to_json(),
            json!({"name": "eth0", "address": "10.0.0.5", "cidr_block": 24})
        );
        let i = InterfaceSpec::parse("fe80::1", "eth1".into()).unwrap();
        assert_eq!(i.to_json(), json!({"name": "eth1", "address": "fe80::1"}));
        assert!(InterfaceSpec::parse("10.0.0.5/33", "eth0".into()).is_err());
        assert!(InterfaceSpec::parse("host", "eth0".into()).is_err());
        let from = InterfaceSpec::from_interface(&crate::models::Interface {
            name: "eth0".into(),
            address: "10.0.0.5".into(),
            cidr_block: Some(24),
            link_state: Some("up".into()),
        });
        assert_eq!(from.to_json()["cidr_block"], 24);
    }
}
