//! The mesh: flows grouped by who talked to whom on which service, with
//! the user and the processes on each side, like the Illumio UI's mesh.
//! Pure; no requests.
use super::aggregate::{Dominant, dominant, group_key, keys_or_default, tally};
use super::resolved::{ResolvedFlow, service_text};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MeshRow {
    /// The source's user: None when the column is hidden, the flow carried
    /// no user, or the flow is inbound (the user, like the process, is the
    /// one the reporting VEN saw, so on inbound flows it is the destination's).
    pub user: Option<String>,
    /// The source's process (outbound flows), None when hidden or absent.
    pub process: Option<String>,
    /// Group values (`group_key`) of the source and the destination.
    pub src: Vec<String>,
    pub dst: Vec<String>,
    pub port: Option<u16>,
    pub proto: String,
    /// The destination's process (inbound flows).
    pub dst_process: Option<String>,
    pub flows: u64,
    pub connections: u64,
    pub bytes: u64,
    pub allowed: u64,
    pub potentially_blocked: u64,
    pub blocked: u64,
}

/// Which optional columns take part in the grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MeshColumns {
    pub user: bool,
    pub process: bool,
}

/// What a mesh row groups on; `MeshRow::key` gives it back, so a row can
/// find its flows again (the TUI's drill-down and `w`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MeshKey {
    pub user: Option<String>,
    pub process: Option<String>,
    pub src: Vec<String>,
    pub dst: Vec<String>,
    pub port: Option<u16>,
    pub proto: String,
    pub dst_process: Option<String>,
}

impl MeshKey {
    /// `5432/tcp`, or just `icmp` when there is no port.
    pub fn service_text(&self) -> String {
        service_text(self.port, &self.proto)
    }
}

/// `(source process, destination process)`: the PCE reports one
/// `process_name`, observed on the reporting VEN. Inbound flows were seen on
/// the destination, outbound ones (and flows without a direction) on the
/// source.
pub fn process_sides(f: &ResolvedFlow) -> (Option<String>, Option<String>) {
    if f.flow_direction.as_deref() == Some("inbound") {
        (None, f.process_name.clone())
    } else {
        (f.process_name.clone(), None)
    }
}

/// The source's user: the PCE reports `user_name` from the same VEN as
/// `process_name`, so it sits on the side `process_sides` gives the process.
/// Inbound flows were seen on the destination and have no source user.
pub fn source_user(f: &ResolvedFlow) -> Option<String> {
    if f.flow_direction.as_deref() == Some("inbound") {
        None
    } else {
        f.user_name.clone()
    }
}

/// The mesh key of one flow under `keys` (empty means `app,env`) and the
/// shown columns.
pub fn mesh_key(f: &ResolvedFlow, keys: &[String], cols: MeshColumns) -> MeshKey {
    let keys = keys_or_default(keys);
    let (process, dst_process) = process_sides(f);
    MeshKey {
        user: source_user(f).filter(|_| cols.user),
        process: process.filter(|_| cols.process),
        src: group_key(&f.src, &keys),
        dst: group_key(&f.dst, &keys),
        port: f.port,
        proto: f.proto.clone(),
        dst_process,
    }
}

impl MeshRow {
    pub fn key(&self) -> MeshKey {
        MeshKey {
            user: self.user.clone(),
            process: self.process.clone(),
            src: self.src.clone(),
            dst: self.dst.clone(),
            port: self.port,
            proto: self.proto.clone(),
            dst_process: self.dst_process.clone(),
        }
    }

    pub fn service_text(&self) -> String {
        service_text(self.port, &self.proto)
    }

    pub fn dominant(&self) -> Dominant {
        dominant(self.allowed, self.potentially_blocked, self.blocked)
    }

    /// Whether `f` is one of this row's flows under `keys` and `cols`.
    pub fn contains(&self, f: &ResolvedFlow, keys: &[String], cols: MeshColumns) -> bool {
        mesh_key(f, keys, cols) == self.key()
    }
}

/// Rows grouped on `(user?, process?, src, dst, port, proto, dst_process)`,
/// user and process only when their column is shown. Sorted by connections
/// descending, then src, dst, service. Unknown decisions count in flows and
/// connections only.
pub fn mesh(flows: &[ResolvedFlow], keys: &[String], cols: MeshColumns) -> Vec<MeshRow> {
    let keys = keys_or_default(keys);
    let mut map: BTreeMap<MeshKey, MeshRow> = BTreeMap::new();
    for f in flows {
        let k = mesh_key(f, &keys, cols);
        let row = map.entry(k.clone()).or_insert_with(|| MeshRow {
            user: k.user,
            process: k.process,
            src: k.src,
            dst: k.dst,
            port: k.port,
            proto: k.proto,
            dst_process: k.dst_process,
            flows: 0,
            connections: 0,
            bytes: 0,
            allowed: 0,
            potentially_blocked: 0,
            blocked: 0,
        });
        row.flows += 1;
        row.connections += f.num_connections;
        row.bytes += f.bytes();
        tally(
            f.policy_decision,
            &mut row.allowed,
            &mut row.potentially_blocked,
            &mut row.blocked,
        );
    }
    let mut out: Vec<MeshRow> = map.into_values().collect();
    out.sort_by(|a, b| {
        b.connections
            .cmp(&a.connections)
            .then_with(|| a.src.cmp(&b.src))
            .then_with(|| a.dst.cmp(&b.dst))
            .then_with(|| a.port.cmp(&b.port))
            .then_with(|| a.proto.cmp(&b.proto))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::PolicyDecision;
    use crate::traffic::resolved::tests::{flows, rich_flows};

    fn keys(k: &[&str]) -> Vec<String> {
        k.iter().map(|s| s.to_string()).collect()
    }

    const BOTH: MeshColumns = MeshColumns {
        user: true,
        process: true,
    };

    #[test]
    fn the_direction_places_the_process_on_its_side() {
        let rows = mesh(&flows(), &keys(&["app", "env"]), BOTH);
        assert_eq!(rows.len(), 4);
        let out = &rows[0]; // shop/prod -> shop/prod 5432/tcp, outbound postgres
        assert_eq!(out.connections, 310);
        assert_eq!(out.user.as_deref(), Some("postgres"));
        assert_eq!(out.process.as_deref(), Some("postgres"));
        assert_eq!(out.dst_process, None);
        assert_eq!(out.service_text(), "5432/tcp");
        assert_eq!(out.bytes, 124_000);
        let rich = mesh(&rich_flows(), &keys(&["app", "env"]), BOTH);
        let inbound = rich
            .iter()
            .find(|r| r.dst_process.as_deref() == Some("nginx"))
            .expect("the inbound Corp flow");
        assert_eq!(inbound.process, None, "seen on the destination");
        assert_eq!(inbound.src, vec!["ipl:Corp", "-"]);
        assert_eq!(inbound.user, None, "www-data is the destination's user");
        let icmp = rich.iter().find(|r| r.proto == "icmp").unwrap();
        assert_eq!(
            (icmp.process.as_deref(), icmp.dst_process.as_deref()),
            (None, None),
            "no direction and no process"
        );
        assert_eq!(icmp.service_text(), "icmp");
    }

    #[test]
    fn an_inbound_flows_user_belongs_to_the_destination_and_is_not_shown_as_the_source_user() {
        let f = rich_flows();
        let k = keys(&["app", "env"]);
        let inbound: Vec<&ResolvedFlow> = f
            .iter()
            .filter(|fl| fl.flow_direction.as_deref() == Some("inbound"))
            .collect();
        assert!(
            inbound.iter().any(|fl| fl.user_name.is_some()),
            "the fixture reports users on inbound flows"
        );
        for fl in &inbound {
            assert_eq!(source_user(fl), None);
            assert_eq!(mesh_key(fl, &k, BOTH).user, None);
        }
        let outbound = f
            .iter()
            .find(|fl| fl.flow_direction.as_deref() != Some("inbound") && fl.user_name.is_some())
            .expect("an outbound or undirected flow with a user");
        assert_eq!(source_user(outbound), outbound.user_name);
        let rows = mesh(&f, &k, BOTH);
        assert!(
            rows.iter()
                .filter(|r| r.dst_process.is_some())
                .all(|r| r.user.is_none()),
            "a row with a destination process came from an inbound flow"
        );
        assert!(
            rows.iter()
                .filter(|r| r.src.first().is_some_and(|s| s.starts_with("ipl:")))
                .all(|r| r.user.is_none()),
            "an IP list has no user"
        );
    }

    #[test]
    fn hiding_the_user_or_the_process_merges_rows() {
        let f = rich_flows();
        let k = keys(&["app", "env"]);
        let none = MeshColumns::default();
        let no_user = MeshColumns {
            user: false,
            process: true,
        };
        let no_process = MeshColumns {
            user: true,
            process: false,
        };
        assert_eq!(mesh(&f, &k, BOTH).len(), 30);
        assert_eq!(mesh(&f, &k, no_user).len(), 29, "java as app and as batch");
        assert_eq!(
            mesh(&f, &k, no_process).len(),
            29,
            "java and python3 as app"
        );
        let merged = mesh(&f, &k, none);
        assert_eq!(merged.len(), 28);
        let shop = merged
            .iter()
            .find(|r| {
                r.src == vec!["shop", "prod"] && r.port == Some(5432) && r.dst_process.is_none()
            })
            .unwrap();
        assert_eq!((shop.flows, shop.connections), (3, 401));
        assert_eq!(
            (shop.user.as_deref(), shop.process.as_deref()),
            (None, None)
        );
        assert!(
            merged
                .iter()
                .all(|r| r.user.is_none() && r.process.is_none())
        );
        assert!(
            merged.iter().any(|r| r.dst_process.is_some()),
            "the destination process stays"
        );
    }

    #[test]
    fn rows_sort_by_connections_then_src_dst_and_service() {
        let rows = mesh(&rich_flows(), &keys(&["app", "env"]), BOTH);
        let top: Vec<u64> = rows.iter().take(4).map(|r| r.connections).collect();
        assert_eq!(top, vec![700, 610, 520, 420]);
        assert!(
            rows.windows(2)
                .all(|w| w[0].connections >= w[1].connections)
        );
        assert_eq!(
            rows.iter().map(|r| r.connections).sum::<u64>(),
            4_109,
            "every flow in exactly one row"
        );
        let unknown = rows.iter().find(|r| r.proto == "udp").unwrap();
        assert_eq!(
            (unknown.flows, unknown.allowed, unknown.dominant()),
            (1, 0, PolicyDecision::Unknown)
        );
        let by_role = mesh(&rich_flows(), &keys(&["role"]), BOTH);
        assert_eq!(by_role[0].src, vec!["ipl:Corp"]);
        assert_eq!(by_role[0].dst, vec!["web"]);
        assert_eq!(
            mesh(&flows(), &[], BOTH),
            mesh(&flows(), &keys(&["app", "env"]), BOTH)
        );
    }

    #[test]
    fn a_row_finds_its_flows_again() {
        let f = rich_flows();
        let k = keys(&["app", "env"]);
        let rows = mesh(&f, &k, MeshColumns::default());
        let shop = rows
            .iter()
            .find(|r| {
                r.src == vec!["shop", "prod"] && r.port == Some(5432) && r.dst_process.is_none()
            })
            .unwrap();
        let behind: Vec<usize> = f
            .iter()
            .enumerate()
            .filter(|(_, fl)| shop.contains(fl, &k, MeshColumns::default()))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(behind, vec![1, 26, 29]);
        assert_eq!(shop.key(), mesh_key(&f[1], &k, MeshColumns::default()));
        assert_eq!(shop.key().service_text(), "5432/tcp");
        assert!(!shop.contains(&f[1], &k, BOTH), "other columns, other key");
    }
}
