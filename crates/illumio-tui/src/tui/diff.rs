//! What a pending IP list or service changes against its active version.
//! Pure: the runtime loads both lists, these functions compare them. Every
//! entry is listed, unchanged ones with `Sign::Same`, so the reader sees the
//! whole object.
use pce::models::{IpList, PendingChange, Service};
use pce::write::IpRangeSpec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    Created,
    Updated,
    Deleted,
}

impl DiffKind {
    pub fn from_change_type(change_type: &str) -> Self {
        match change_type {
            "create" => DiffKind::Created,
            "delete" => DiffKind::Deleted,
            _ => DiffKind::Updated,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            DiffKind::Created => "created",
            DiffKind::Updated => "updated",
            DiffKind::Deleted => "deleted",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sign {
    Same,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub sign: Sign,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectDiff {
    pub name: String,
    pub href: String,
    pub change: DiffKind,
    pub lines: Vec<DiffLine>,
}

/// One `label: value` line, or a removed and an added line when it changed.
fn field(out: &mut Vec<DiffLine>, label: &str, active: Option<&str>, draft: Option<&str>) {
    let line = |sign, v: &str| DiffLine {
        sign,
        text: format!("{label}: {v}"),
    };
    match (active, draft) {
        (Some(a), Some(d)) if a == d => out.push(line(Sign::Same, d)),
        (a, d) => {
            if let Some(a) = a {
                out.push(line(Sign::Removed, a));
            }
            if let Some(d) = d {
                out.push(line(Sign::Added, d));
            }
        }
    }
}

/// Draft entries in order (same or added), then entries only the active side has.
fn entries(out: &mut Vec<DiffLine>, label: &str, active: &[String], draft: &[String]) {
    for d in draft {
        let sign = if active.contains(d) {
            Sign::Same
        } else {
            Sign::Added
        };
        out.push(DiffLine {
            sign,
            text: format!("{label} {d}"),
        });
    }
    for a in active.iter().filter(|a| !draft.contains(a)) {
        out.push(DiffLine {
            sign: Sign::Removed,
            text: format!("{label} {a}"),
        });
    }
}

fn non_empty(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|d| !d.is_empty())
}

fn ranges(l: &IpList) -> Vec<String> {
    l.ip_ranges
        .iter()
        .map(|r| IpRangeSpec::from_range(r).display())
        .collect()
}

fn fqdns(l: &IpList) -> Vec<String> {
    l.fqdns.iter().map(|f| f.fqdn.clone()).collect()
}

fn ports(s: &Service) -> Vec<String> {
    s.service_ports.iter().map(|p| p.display()).collect()
}

/// `None` on one side: a created (no active) or deleted (no draft) object.
pub fn diff_ip_list(active: Option<&IpList>, draft: Option<&IpList>) -> Vec<DiffLine> {
    let mut out = Vec::new();
    field(
        &mut out,
        "name",
        active.map(|l| l.name.as_str()),
        draft.map(|l| l.name.as_str()),
    );
    field(
        &mut out,
        "description",
        active.and_then(|l| non_empty(&l.description)),
        draft.and_then(|l| non_empty(&l.description)),
    );
    entries(
        &mut out,
        "range",
        &active.map(ranges).unwrap_or_default(),
        &draft.map(ranges).unwrap_or_default(),
    );
    entries(
        &mut out,
        "fqdn",
        &active.map(fqdns).unwrap_or_default(),
        &draft.map(fqdns).unwrap_or_default(),
    );
    out
}

pub fn diff_service(active: Option<&Service>, draft: Option<&Service>) -> Vec<DiffLine> {
    let mut out = Vec::new();
    field(
        &mut out,
        "name",
        active.map(|s| s.name.as_str()),
        draft.map(|s| s.name.as_str()),
    );
    field(
        &mut out,
        "description",
        active.and_then(|s| non_empty(&s.description)),
        draft.and_then(|s| non_empty(&s.description)),
    );
    entries(
        &mut out,
        "port",
        &active.map(ports).unwrap_or_default(),
        &draft.map(ports).unwrap_or_default(),
    );
    out
}

/// The same object in both scopes: active hrefs differ from draft hrefs only
/// by the `sec_policy/<scope>` segment.
fn draft_href(href: &str) -> String {
    href.replacen("/sec_policy/active/", "/sec_policy/draft/", 1)
}

trait Diffable {
    fn href(&self) -> &str;
    fn name(&self) -> &str;
}

impl Diffable for IpList {
    fn href(&self) -> &str {
        &self.href
    }
    fn name(&self) -> &str {
        &self.name
    }
}

impl Diffable for Service {
    fn href(&self) -> &str {
        &self.href
    }
    fn name(&self) -> &str {
        &self.name
    }
}

fn diff_for<T: Diffable>(
    change: &PendingChange,
    draft: &[T],
    active: &[T],
    diff: fn(Option<&T>, Option<&T>) -> Vec<DiffLine>,
) -> ObjectDiff {
    let key = draft_href(&change.href);
    let d = draft.iter().find(|x| draft_href(x.href()) == key);
    let a = active.iter().find(|x| draft_href(x.href()) == key);
    let kind = DiffKind::from_change_type(&change.change_type);
    let name = change
        .name
        .clone()
        .or_else(|| d.map(|x| x.name().to_string()))
        .or_else(|| a.map(|x| x.name().to_string()))
        .unwrap_or_else(|| change.href.clone());
    let missing = match kind {
        DiffKind::Created | DiffKind::Updated if d.is_none() => Some("draft"),
        DiffKind::Deleted | DiffKind::Updated if a.is_none() => Some("active"),
        _ => None,
    };
    let lines = match (missing, kind) {
        (Some(side), _) => vec![DiffLine {
            sign: Sign::Same,
            text: format!("{name} is not in the {side} list"),
        }],
        (None, DiffKind::Created) => diff(None, d),
        (None, DiffKind::Deleted) => diff(a, None),
        (None, DiffKind::Updated) => diff(a, d),
    };
    ObjectDiff {
        name,
        href: change.href.clone(),
        change: kind,
        lines,
    }
}

pub fn ip_list_diff_for(change: &PendingChange, draft: &[IpList], active: &[IpList]) -> ObjectDiff {
    diff_for(change, draft, active, diff_ip_list)
}

pub fn service_diff_for(
    change: &PendingChange,
    draft: &[Service],
    active: &[Service],
) -> ObjectDiff {
    diff_for(change, draft, active, diff_service)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn corp() -> IpList {
        serde_json::from_value(json!({
            "href": "/orgs/5/sec_policy/draft/ip_lists/21", "name": "Corp", "description": "Office ranges",
            "ip_ranges": [{"from_ip": "10.1.0.0", "to_ip": "10.1.255.255"}],
            "fqdns": [{"fqdn": "*.corp.example"}]
        }))
        .unwrap()
    }

    fn line(sign: Sign, text: &str) -> DiffLine {
        DiffLine {
            sign,
            text: text.into(),
        }
    }

    fn change(href: &str, change_type: &str) -> PendingChange {
        PendingChange {
            href: href.into(),
            change_type: change_type.into(),
            name: Some("Corp".into()),
            object_type: "ip_lists".into(),
        }
    }

    #[test]
    fn an_updated_ip_list_shows_added_removed_and_unchanged_entries() {
        let mut active = corp();
        active.href = "/orgs/5/sec_policy/active/ip_lists/21".into();
        let mut draft = corp();
        draft.description = Some("Office".into());
        draft
            .ip_ranges
            .push(serde_json::from_value(json!({"from_ip": "10.2.0.0/16"})).unwrap());
        draft.fqdns.clear();
        assert_eq!(
            diff_ip_list(Some(&active), Some(&draft)),
            vec![
                line(Sign::Same, "name: Corp"),
                line(Sign::Removed, "description: Office ranges"),
                line(Sign::Added, "description: Office"),
                line(Sign::Same, "range 10.1.0.0-10.1.255.255"),
                line(Sign::Added, "range 10.2.0.0/16"),
                line(Sign::Removed, "fqdn *.corp.example"),
            ]
        );
        let d = ip_list_diff_for(
            &change("/orgs/5/sec_policy/draft/ip_lists/21", "update"),
            &[draft],
            &[active],
        );
        assert_eq!(d.change, DiffKind::Updated);
        assert_eq!(d.name, "Corp");
        assert_eq!(d.lines.len(), 6, "the active href matches its draft href");
    }

    #[test]
    fn created_is_all_added_and_deleted_all_removed() {
        let c = corp();
        let created = diff_ip_list(None, Some(&c));
        assert_eq!(created.len(), 4, "name, description, range, fqdn");
        assert!(created.iter().all(|l| l.sign == Sign::Added));
        assert!(
            diff_ip_list(Some(&c), None)
                .iter()
                .all(|l| l.sign == Sign::Removed)
        );
        let d = ip_list_diff_for(
            &change(&c.href, "delete"),
            std::slice::from_ref(&c),
            std::slice::from_ref(&c),
        );
        assert_eq!(d.change, DiffKind::Deleted);
        assert!(
            d.lines.iter().all(|l| l.sign == Sign::Removed),
            "a delete has no draft side"
        );
    }

    #[test]
    fn a_missing_side_is_said_not_shown_as_an_empty_diff() {
        let c = corp();
        let d = ip_list_diff_for(&change(&c.href, "update"), &[], std::slice::from_ref(&c));
        assert_eq!(
            d.lines,
            vec![line(Sign::Same, "Corp is not in the draft list")]
        );
        let d = ip_list_diff_for(&change(&c.href, "update"), std::slice::from_ref(&c), &[]);
        assert_eq!(
            d.lines,
            vec![line(Sign::Same, "Corp is not in the active list")]
        );
        let d = ip_list_diff_for(&change(&c.href, "create"), std::slice::from_ref(&c), &[]);
        assert_eq!(d.change, DiffKind::Created);
        assert!(d.lines.iter().all(|l| l.sign == Sign::Added));
        let d = ip_list_diff_for(&change(&c.href, "create"), &[], &[]);
        assert_eq!(
            d.lines,
            vec![line(Sign::Same, "Corp is not in the draft list")]
        );
    }

    #[test]
    fn services_diff_by_port_and_fall_back_to_the_object_name() {
        let active: Service = serde_json::from_value(json!({
            "href": "/orgs/5/sec_policy/active/services/32", "name": "Web",
            "service_ports": [{"port": 80, "proto": 6}, {"port": 443, "proto": 6}]
        }))
        .unwrap();
        let draft: Service = serde_json::from_value(json!({
            "href": "/orgs/5/sec_policy/draft/services/32", "name": "Web",
            "service_ports": [{"port": 443, "proto": 6}, {"port": 8443, "proto": 6}]
        }))
        .unwrap();
        assert_eq!(
            diff_service(Some(&active), Some(&draft)),
            vec![
                line(Sign::Same, "name: Web"),
                line(Sign::Same, "port 443/tcp"),
                line(Sign::Added, "port 8443/tcp"),
                line(Sign::Removed, "port 80/tcp"),
            ]
        );
        let unnamed = PendingChange {
            href: draft.href.clone(),
            change_type: "update".into(),
            name: None,
            object_type: "services".into(),
        };
        let d = service_diff_for(&unnamed, &[draft], &[active]);
        assert_eq!(d.name, "Web");
        assert_eq!(d.change, DiffKind::Updated);
    }
}
