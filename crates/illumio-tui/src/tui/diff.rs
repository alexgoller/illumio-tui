//! What a pending IP list, label group, service or ruleset changes against its
//! active version. The `*_diff_for` and `diff_*` functions are pure: they
//! compare two loaded lists, listing every entry (unchanged ones with
//! `Sign::Same`) so the reader sees the whole object. [`load_diff`] is the one
//! cached I/O entry point, shared by the TUI runtime and `policy pending --diff`.
use crate::tui::edit::{actor_tokens, service_tokens};
use crate::tui::model::rule_kind_name;
use pce::models::{
    Actor, DenyRule, IngressService, IpList, LabelGroup, PendingChange, PolicyScope, RuleSet,
    Service,
};
use pce::write::{IpRangeSpec, ObjectKind};
use pce::{CacheMode, NameIndex, Pce, PceError, RuleKind, RuleSetView};

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
pub(crate) fn field(
    out: &mut Vec<DiffLine>,
    label: &str,
    active: Option<&str>,
    draft: Option<&str>,
) {
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
pub(crate) fn entries(out: &mut Vec<DiffLine>, label: &str, active: &[String], draft: &[String]) {
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

/// Member labels as `key=value` and sub-groups by name, both read through the
/// index (an unknown href stays an href). The model has no description.
pub fn diff_label_group(
    active: Option<&LabelGroup>,
    draft: Option<&LabelGroup>,
    idx: &NameIndex,
) -> Vec<DiffLine> {
    let labels = |g: &LabelGroup| -> Vec<String> {
        g.labels.iter().map(|r| idx.label_text(&r.href)).collect()
    };
    let subs = |g: &LabelGroup| -> Vec<String> {
        g.sub_groups
            .iter()
            .map(|r| idx.label_group_name(&r.href).unwrap_or(&r.href).to_string())
            .collect()
    };
    let mut out = Vec::new();
    field(
        &mut out,
        "name",
        active.map(|g| g.name.as_str()),
        draft.map(|g| g.name.as_str()),
    );
    field(
        &mut out,
        "key",
        active.map(|g| g.key.as_str()).filter(|k| !k.is_empty()),
        draft.map(|g| g.key.as_str()).filter(|k| !k.is_empty()),
    );
    entries(
        &mut out,
        "label",
        &active.map(labels).unwrap_or_default(),
        &draft.map(labels).unwrap_or_default(),
    );
    entries(
        &mut out,
        "sub-group",
        &active.map(subs).unwrap_or_default(),
        &draft.map(subs).unwrap_or_default(),
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
pub(crate) fn draft_href(href: &str) -> String {
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

impl Diffable for LabelGroup {
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

impl Diffable for RuleSet {
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
    diff: impl Fn(Option<&T>, Option<&T>) -> Vec<DiffLine>,
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

pub fn label_group_diff_for(
    change: &PendingChange,
    draft: &[LabelGroup],
    active: &[LabelGroup],
    idx: &NameIndex,
) -> ObjectDiff {
    diff_for(change, draft, active, |a, d| diff_label_group(a, d, idx))
}

pub fn service_diff_for(
    change: &PendingChange,
    draft: &[Service],
    active: &[Service],
) -> ObjectDiff {
    diff_for(change, draft, active, diff_service)
}

fn yes_no(on: bool) -> &'static str {
    if on { "yes" } else { "no" }
}

/// `allow role=web ← iplist:Corp 443/tcp`: kind, providers ← consumers,
/// services, in the form's token grammar; ` (disabled)` when it is off.
/// Unknown objects show their href, compared in draft form.
pub(crate) fn rule_summary(
    kind: RuleKind,
    providers: &[Actor],
    consumers: &[Actor],
    services: &[IngressService],
    enabled: bool,
    idx: &NameIndex,
) -> String {
    let mut s = format!(
        "{} {} ← {} {}",
        rule_kind_name(kind),
        actor_tokens(providers, idx).join(", "),
        actor_tokens(consumers, idx).join(", "),
        service_tokens(services, idx).join(", ")
    );
    if !enabled {
        s.push_str(" (disabled)");
    }
    s.replace("/sec_policy/active/", "/sec_policy/draft/")
}

/// The allow rules of `rs` as (draft href, summary).
fn allow_lines(rs: &RuleSet, idx: &NameIndex) -> Vec<(String, String)> {
    rs.rules
        .iter()
        .map(|r| {
            (
                draft_href(&r.href),
                rule_summary(
                    RuleKind::Allow,
                    &r.providers,
                    &r.consumers,
                    &r.ingress_services,
                    r.enabled,
                    idx,
                ),
            )
        })
        .collect()
}

fn deny_lines(deny: &[DenyRule], idx: &NameIndex) -> Vec<(String, String)> {
    deny.iter()
        .map(|d| {
            let kind = if d.override_ {
                RuleKind::OverrideDeny
            } else {
                RuleKind::Deny
            };
            (
                draft_href(&d.href),
                rule_summary(
                    kind,
                    &d.providers,
                    &d.consumers,
                    &d.ingress_services,
                    d.enabled,
                    idx,
                ),
            )
        })
        .collect()
}

/// Draft rules in order (same, changed as `-` old `+` new, or added), then
/// rules only the active side has.
fn rules(out: &mut Vec<DiffLine>, active: &[(String, String)], draft: &[(String, String)]) {
    let line = |sign, text: &str| DiffLine {
        sign,
        text: text.to_string(),
    };
    for (href, d) in draft {
        match active.iter().find(|(h, _)| h == href) {
            Some((_, a)) if a == d => out.push(line(Sign::Same, d)),
            Some((_, a)) => {
                out.push(line(Sign::Removed, a));
                out.push(line(Sign::Added, d));
            }
            None => out.push(line(Sign::Added, d)),
        }
    }
    for (href, a) in active {
        if !draft.iter().any(|(h, _)| h == href) {
            out.push(line(Sign::Removed, a));
        }
    }
}

/// The deny rules one side of a ruleset diff has.
#[derive(Debug, Clone, Copy)]
pub enum DenySide<'a> {
    /// These, possibly none.
    Known(&'a [DenyRule]),
    /// Not in the payload and not fetched: nothing to compare.
    Unfetched,
}

impl<'a> DenySide<'a> {
    /// `rs`'s inline deny rules when it has some; else `fallback` (fetched
    /// separately); else none when its list inlines deny rules elsewhere (the
    /// PCE then inlines them for every ruleset, so empty means zero); else
    /// unfetched.
    pub fn of(rs: &'a RuleSet, list_inlines: bool, fallback: Option<&'a [DenyRule]>) -> Self {
        if !rs.deny_rules.is_empty() {
            DenySide::Known(&rs.deny_rules)
        } else if let Some(f) = fallback {
            DenySide::Known(f)
        } else if list_inlines {
            DenySide::Known(&[])
        } else {
            DenySide::Unfetched
        }
    }
}

/// Whether any ruleset of a list payload carries inline deny rules.
pub fn inlines_deny(list: &[RuleSet]) -> bool {
    list.iter().any(|r| !r.deny_rules.is_empty())
}

/// A ruleset's name, description, state and scopes (`-`/`+` when they
/// differ), then one line per rule, matched by href. Each side's deny rules
/// are its inline list, or unfetched when that list is empty.
pub fn diff_rule_set(
    active: Option<&RuleSet>,
    draft: Option<&RuleSet>,
    idx: &NameIndex,
) -> Vec<DiffLine> {
    diff_rule_set_with(active, draft, idx, inline_side(active), inline_side(draft))
}

fn inline_side(r: Option<&RuleSet>) -> DenySide<'_> {
    r.map_or(DenySide::Unfetched, |r| DenySide::of(r, false, None))
}

/// [`diff_rule_set`] with each side's deny rules given. When a present side's
/// deny rules are unfetched they are not compared: the other side's show as
/// unchanged under one note, never as added or removed.
pub fn diff_rule_set_with(
    active: Option<&RuleSet>,
    draft: Option<&RuleSet>,
    idx: &NameIndex,
    active_deny: DenySide,
    draft_deny: DenySide,
) -> Vec<DiffLine> {
    let mut out = Vec::new();
    field(
        &mut out,
        "name",
        active.map(|r| r.name.as_str()),
        draft.map(|r| r.name.as_str()),
    );
    field(
        &mut out,
        "description",
        active.and_then(|r| non_empty(&r.description)),
        draft.and_then(|r| non_empty(&r.description)),
    );
    field(
        &mut out,
        "enabled",
        active.map(|r| yes_no(r.enabled)),
        draft.map(|r| yes_no(r.enabled)),
    );
    let scopes = |r: &RuleSet| RuleSetView::from_rule_set(r, idx).scopes.join(" | ");
    let (a, d) = (active.map(scopes), draft.map(scopes));
    field(&mut out, "scopes", a.as_deref(), d.as_deref());

    let a_rules = active.map(|r| allow_lines(r, idx)).unwrap_or_default();
    let d_rules = draft.map(|r| allow_lines(r, idx)).unwrap_or_default();
    rules(&mut out, &a_rules, &d_rules);

    // A missing side (create, delete) has no deny rules; a present side's
    // are known or unfetched.
    let known = |present: bool, side: DenySide| match (present, side) {
        (false, _) => Some(Vec::new()),
        (true, DenySide::Known(d)) => Some(deny_lines(d, idx)),
        (true, DenySide::Unfetched) => None,
    };
    let a_deny = known(active.is_some(), active_deny);
    let d_deny = known(draft.is_some(), draft_deny);
    match (a_deny, d_deny) {
        (Some(a), Some(d)) => rules(&mut out, &a, &d),
        (a, d) => {
            let unfetched: Vec<&str> = [("active", a.is_none()), ("draft", d.is_none())]
                .into_iter()
                .filter_map(|(side, missing)| missing.then_some(side))
                .collect();
            // Only the draft deny rules can be loaded from the Policy screen.
            let advice = if d.is_none() {
                "; Enter on the ruleset in the Policy screen loads them"
            } else {
                ""
            };
            let shown = d.or(a).unwrap_or_default();
            out.extend(shown.into_iter().map(|(_, text)| DiffLine {
                sign: Sign::Same,
                text,
            }));
            out.push(DiffLine {
                sign: Sign::Same,
                text: format!(
                    "deny rules not compared ({} not fetched{advice})",
                    unfetched.join(" and ")
                ),
            });
        }
    }
    out
}

/// The diff of one pending ruleset. Whether each list payload inlines deny
/// rules decides what an empty inline list means (see [`DenySide::of`]);
/// `draft_fallback_deny` is the draft ruleset's deny rules fetched separately.
pub fn rule_set_diff_for(
    change: &PendingChange,
    draft: &[RuleSet],
    active: &[RuleSet],
    idx: &NameIndex,
    draft_fallback_deny: Option<&[DenyRule]>,
) -> ObjectDiff {
    let (a_inl, d_inl) = (inlines_deny(active), inlines_deny(draft));
    diff_for(change, draft, active, |a, d| {
        let a_deny = a.map_or(DenySide::Unfetched, |r| DenySide::of(r, a_inl, None));
        let d_deny = d.map_or(DenySide::Unfetched, |r| {
            DenySide::of(r, d_inl, draft_fallback_deny)
        });
        diff_rule_set_with(a, d, idx, a_deny, d_deny)
    })
}

/// The kind of a pending change's `object_type` that has a diff. A rule is
/// diffed through its ruleset, so rules have none of their own.
pub fn diff_object_kind(object_type: &str) -> Option<ObjectKind> {
    match object_type {
        "label_groups" => Some(ObjectKind::LabelGroup),
        "ip_lists" => Some(ObjectKind::IpList),
        "services" => Some(ObjectKind::Service),
        "rule_sets" => Some(ObjectKind::RuleSet),
        _ => None,
    }
}

/// The draft and active list of one kind through the cache, compared
/// locally. The draft list is usually cached by the name index, so the
/// common cost is one GET for the active list, and none when both are fresh.
pub async fn load_diff(
    pce: &Pce,
    change: &PendingChange,
    kind: ObjectKind,
    index: Option<&NameIndex>,
    fallback_deny: Option<&[DenyRule]>,
) -> pce::Result<ObjectDiff> {
    match kind {
        ObjectKind::LabelGroup => {
            let draft = pce.label_groups(CacheMode::Use).await?.value;
            let active = pce.active_label_groups(CacheMode::Use).await?.value;
            let empty = NameIndex::default();
            Ok(label_group_diff_for(
                change,
                &draft,
                &active,
                index.unwrap_or(&empty),
            ))
        }
        ObjectKind::IpList => {
            let draft = pce.ip_lists(CacheMode::Use).await?.value;
            let active = pce.active_ip_lists(CacheMode::Use).await?.value;
            Ok(ip_list_diff_for(change, &draft, &active))
        }
        ObjectKind::Service => {
            let draft = pce.services(CacheMode::Use).await?.value;
            let active = pce.active_services(CacheMode::Use).await?.value;
            Ok(service_diff_for(change, &draft, &active))
        }
        ObjectKind::RuleSet => {
            let draft = pce
                .rule_sets(PolicyScope::Draft, CacheMode::Use)
                .await?
                .value;
            let active = pce
                .rule_sets(PolicyScope::Active, CacheMode::Use)
                .await?
                .value;
            let empty = NameIndex::default();
            Ok(rule_set_diff_for(
                change,
                &draft,
                &active,
                index.unwrap_or(&empty),
                fallback_deny,
            ))
        }
        other => Err(PceError::Invalid(format!("no diff for {}", other.as_str()))),
    }
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

    #[test]
    fn a_ruleset_diff_lists_fields_and_one_line_per_rule() {
        use crate::tui::test_support::{fixture_index, rule_set_diff_fixture};
        let (active, draft) = rule_set_diff_fixture();
        assert_eq!(
            diff_rule_set(Some(&active), Some(&draft), &fixture_index()),
            vec![
                line(Sign::Same, "name: Shop-Prod"),
                line(Sign::Removed, "description: Ringfence"),
                line(Sign::Added, "description: Ringfence shop"),
                line(Sign::Same, "enabled: yes"),
                line(Sign::Same, "scopes: app=shop & env=prod"),
                line(Sign::Same, "allow role=db ← role=web PostgreSQL"),
                line(
                    Sign::Removed,
                    "allow role=web ← iplist:Corp 443/tcp, 8000-8099/tcp"
                ),
                line(Sign::Added, "allow role=web ← iplist:Corp 443/tcp"),
                line(
                    Sign::Added,
                    "allow role=web ← iplist:Any (0.0.0.0/0 and ::/0) 22/tcp"
                ),
                line(
                    Sign::Same,
                    "deny all ← iplist:Any (0.0.0.0/0 and ::/0) 23/tcp"
                ),
                line(Sign::Same, "override_deny role=db ← all 22/tcp"),
            ]
        );
    }

    #[test]
    fn a_created_ruleset_is_all_added() {
        use crate::tui::test_support::fixture_index;
        let sets: Vec<RuleSet> =
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
        let change = PendingChange {
            href: "/orgs/5/sec_policy/draft/rule_sets/101".into(),
            change_type: "create".into(),
            name: Some("Global-Infra".into()),
            object_type: "rule_sets".into(),
        };
        let d = rule_set_diff_for(&change, &sets, &[], &fixture_index(), None);
        assert_eq!(d.name, "Global-Infra");
        assert_eq!(d.change, DiffKind::Created);
        assert_eq!(
            d.lines,
            vec![
                line(Sign::Added, "name: Global-Infra"),
                line(Sign::Added, "enabled: no"),
                line(Sign::Added, "scopes: All"),
                line(Sign::Added, "allow all ← group:All prod envs All Services"),
            ]
        );
        // Shop-Prod inlines its deny rules, so Global-Infra's empty list
        // means it has none: nothing to note.
    }

    const NOT_COMPARED: &str = "deny rules not compared";

    fn deny(href: &str, port: u16) -> DenyRule {
        serde_json::from_value(json!({
            "href": href, "enabled": true,
            "providers": [{"actors": "ams"}], "consumers": [{"actors": "ams"}],
            "ingress_services": [{"port": port, "proto": 6}]
        }))
        .unwrap()
    }

    #[test]
    fn fallback_deny_rules_stand_in_for_an_empty_inline_list() {
        use crate::tui::test_support::{fixture_index, rule_set_diff_fixture};
        let (active, mut draft) = rule_set_diff_fixture();
        draft.deny_rules.clear();
        let fetched = vec![deny(
            "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/9",
            23,
        )];
        // The draft list inlines no deny rules: only the fallback knows them.
        let lines = rule_set_diff_for(
            &update_of(&draft.href),
            std::slice::from_ref(&draft),
            std::slice::from_ref(&active),
            &fixture_index(),
            Some(&fetched),
        )
        .lines;
        assert!(
            lines.contains(&line(Sign::Added, "deny all ← all 23/tcp")),
            "{lines:?}"
        );
        assert!(
            !lines.iter().any(|l| l.text.starts_with(NOT_COMPARED)),
            "{lines:?}"
        );
        // The active deny rules the draft no longer has still show as removed.
        assert!(
            lines
                .iter()
                .any(|l| l.sign == Sign::Removed && l.text.starts_with("deny"))
        );
    }

    fn update_of(href: &str) -> PendingChange {
        PendingChange {
            href: href.into(),
            change_type: "update".into(),
            name: Some("Shop-Prod".into()),
            object_type: "rule_sets".into(),
        }
    }

    /// Another ruleset of the same list with one inline deny rule: the list
    /// payload inlines deny rules, so an empty inline list means none.
    fn inlining(scope: &str) -> RuleSet {
        let mut o = crate::tui::test_support::rule_set_diff_fixture().0;
        o.href = format!("/orgs/5/sec_policy/{scope}/rule_sets/999");
        o.deny_rules = vec![deny(
            &format!("/orgs/5/sec_policy/{scope}/rule_sets/999/deny_rules/1"),
            25,
        )];
        o
    }

    #[test]
    fn a_first_deny_rule_is_added_when_the_active_list_inlines_deny_rules() {
        use crate::tui::test_support::{fixture_index, rule_set_diff_fixture};
        let (mut active, mut draft) = rule_set_diff_fixture();
        active.href = "/orgs/5/sec_policy/active/rule_sets/100".into();
        active.deny_rules.clear();
        draft.deny_rules = vec![deny(
            "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/9",
            23,
        )];
        let d = rule_set_diff_for(
            &update_of(&draft.href),
            std::slice::from_ref(&draft),
            &[active, inlining("active")],
            &fixture_index(),
            None,
        );
        assert!(
            d.lines
                .contains(&line(Sign::Added, "deny all ← all 23/tcp")),
            "{:?}",
            d.lines
        );
        assert!(
            !d.lines.iter().any(|l| l.text.starts_with("deny rules")),
            "{:?}",
            d.lines
        );
    }

    #[test]
    fn deleting_the_last_deny_rule_is_removed_when_the_draft_list_inlines_deny_rules() {
        use crate::tui::test_support::{fixture_index, rule_set_diff_fixture};
        let (mut active, mut draft) = rule_set_diff_fixture();
        active.href = "/orgs/5/sec_policy/active/rule_sets/100".into();
        draft.deny_rules.clear();
        let d = rule_set_diff_for(
            &update_of(&draft.href),
            &[draft.clone(), inlining("draft")],
            &[active],
            &fixture_index(),
            None,
        );
        for text in [
            "deny all ← iplist:Any (0.0.0.0/0 and ::/0) 23/tcp",
            "override_deny role=db ← all 22/tcp",
        ] {
            assert!(
                d.lines.contains(&line(Sign::Removed, text)),
                "{text}: {:?}",
                d.lines
            );
        }
        assert!(
            !d.lines.iter().any(|l| l.text.starts_with("deny rules")),
            "{:?}",
            d.lines
        );
    }

    #[test]
    fn a_side_that_inlines_no_deny_rules_is_not_compared() {
        use crate::tui::test_support::{fixture_index, rule_set_diff_fixture};
        let (mut active, mut draft) = rule_set_diff_fixture();
        active.href = "/orgs/5/sec_policy/active/rule_sets/100".into();
        draft.deny_rules.clear();
        // The draft list inlines no deny rules anywhere and none were fetched:
        // the active deny rules show unchanged, under one note.
        let d = rule_set_diff_for(
            &update_of(&draft.href),
            std::slice::from_ref(&draft),
            std::slice::from_ref(&active),
            &fixture_index(),
            None,
        );
        assert!(
            d.lines
                .contains(&line(Sign::Same, "override_deny role=db ← all 22/tcp")),
            "{:?}",
            d.lines
        );
        assert!(
            !d.lines
                .iter()
                .any(|l| l.sign != Sign::Same && l.text.starts_with("deny")
                    || l.sign != Sign::Same && l.text.starts_with("override_deny")),
            "{:?}",
            d.lines
        );
        let notes: Vec<_> = d
            .lines
            .iter()
            .filter(|l| l.text.starts_with("deny rules"))
            .collect();
        assert_eq!(
            notes,
            vec![&line(
                Sign::Same,
                "deny rules not compared (draft not fetched; Enter on the ruleset in the Policy screen loads them)"
            )]
        );
        // The active side unfetched: no advice, the Policy screen loads draft only.
        let (mut active, draft) = rule_set_diff_fixture();
        active.href = "/orgs/5/sec_policy/active/rule_sets/100".into();
        active.deny_rules.clear();
        let d = rule_set_diff_for(
            &update_of(&draft.href),
            std::slice::from_ref(&draft),
            std::slice::from_ref(&active),
            &fixture_index(),
            None,
        );
        assert!(
            d.lines
                .contains(&line(Sign::Same, "override_deny role=db ← all 22/tcp")),
            "{:?}",
            d.lines
        );
        assert!(
            d.lines.contains(&line(
                Sign::Same,
                "deny rules not compared (active not fetched)"
            )),
            "{:?}",
            d.lines
        );
    }

    #[test]
    fn unfetched_deny_rules_are_said_and_never_shown_as_changes() {
        use crate::tui::test_support::{fixture_index, rule_set_diff_fixture};
        let (mut active, mut draft) = rule_set_diff_fixture();
        active.deny_rules.clear();
        draft.deny_rules.clear();
        let lines = diff_rule_set(Some(&active), Some(&draft), &fixture_index());
        let notes: Vec<_> = lines
            .iter()
            .filter(|l| l.text.starts_with(NOT_COMPARED))
            .collect();
        assert_eq!(
            notes,
            vec![&line(
                Sign::Same,
                "deny rules not compared (active and draft not fetched; Enter on the ruleset in the Policy screen loads them)"
            )]
        );
        assert!(
            !lines
                .iter()
                .any(|l| l.sign != Sign::Same && l.text.starts_with("deny")),
            "{lines:?}"
        );
        // Only the draft side fetched: the active side is noted, draft denies are unchanged.
        let fetched = vec![deny(
            "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/9",
            23,
        )];
        let lines = diff_rule_set_with(
            Some(&active),
            Some(&draft),
            &fixture_index(),
            DenySide::Unfetched,
            DenySide::Known(&fetched),
        );
        assert!(lines.contains(&line(Sign::Same, "deny all ← all 23/tcp")));
        assert!(
            lines.contains(&line(
                Sign::Same,
                "deny rules not compared (active not fetched)"
            )),
            "{lines:?}"
        );
    }

    fn group_idx() -> NameIndex {
        let labels: Vec<pce::models::Label> = serde_json::from_value(json!([
            {"href": "/orgs/5/labels/4", "key": "env", "value": "prod"},
            {"href": "/orgs/5/labels/5", "key": "env", "value": "staging"},
            {"href": "/orgs/5/labels/6", "key": "env", "value": "dev"}
        ]))
        .unwrap();
        let groups: Vec<LabelGroup> = serde_json::from_value(json!([
            {"href": "/orgs/5/sec_policy/draft/label_groups/11", "name": "Old sub", "key": "env"},
            {"href": "/orgs/5/sec_policy/draft/label_groups/12", "name": "New sub", "key": "env"}
        ]))
        .unwrap();
        NameIndex::new(&labels, &groups, &[], &[])
    }

    fn group(labels: &[&str], subs: &[&str]) -> LabelGroup {
        let refs = |v: &[&str]| json!(v.iter().map(|h| json!({"href": h})).collect::<Vec<_>>());
        serde_json::from_value(json!({
            "href": "/orgs/5/sec_policy/draft/label_groups/10", "name": "All envs", "key": "env",
            "labels": refs(labels), "sub_groups": refs(subs)
        }))
        .unwrap()
    }

    fn group_change(change_type: &str) -> PendingChange {
        PendingChange {
            href: "/orgs/5/sec_policy/draft/label_groups/10".into(),
            change_type: change_type.into(),
            name: Some("All envs".into()),
            object_type: "label_groups".into(),
        }
    }

    #[test]
    fn a_created_or_deleted_label_group_is_all_added_or_all_removed() {
        let idx = group_idx();
        let g = group(
            &["/orgs/5/labels/4"],
            &["/orgs/5/sec_policy/draft/label_groups/11"],
        );
        let want = |sign| {
            vec![
                line(sign, "name: All envs"),
                line(sign, "key: env"),
                line(sign, "label env=prod"),
                line(sign, "sub-group Old sub"),
            ]
        };
        assert_eq!(diff_label_group(None, Some(&g), &idx), want(Sign::Added));
        assert_eq!(diff_label_group(Some(&g), None, &idx), want(Sign::Removed));
    }

    #[test]
    fn an_updated_label_group_marks_one_member_added_and_one_removed() {
        let idx = group_idx();
        let mut active = group(&["/orgs/5/labels/4", "/orgs/5/labels/5"], &[]);
        active.href = "/orgs/5/sec_policy/active/label_groups/10".into();
        let draft = group(&["/orgs/5/labels/4", "/orgs/5/labels/6"], &[]);
        let d = label_group_diff_for(&group_change("update"), &[draft], &[active], &idx);
        assert_eq!(d.change, DiffKind::Updated);
        assert_eq!(
            d.lines,
            vec![
                line(Sign::Same, "name: All envs"),
                line(Sign::Same, "key: env"),
                line(Sign::Same, "label env=prod"),
                line(Sign::Added, "label env=dev"),
                line(Sign::Removed, "label env=staging"),
            ]
        );
    }

    #[test]
    fn a_replaced_sub_group_and_a_renamed_group_show_both_sides() {
        let idx = group_idx();
        let active = group(&[], &["/orgs/5/sec_policy/draft/label_groups/11"]);
        let mut draft = group(&[], &["/orgs/5/sec_policy/draft/label_groups/12"]);
        draft.name = "All envs v2".into();
        let lines = diff_label_group(Some(&active), Some(&draft), &idx);
        assert_eq!(
            lines,
            vec![
                line(Sign::Removed, "name: All envs"),
                line(Sign::Added, "name: All envs v2"),
                line(Sign::Same, "key: env"),
                line(Sign::Added, "sub-group New sub"),
                line(Sign::Removed, "sub-group Old sub"),
            ]
        );
    }

    #[test]
    fn a_label_group_missing_from_a_list_says_so() {
        let d = label_group_diff_for(&group_change("update"), &[], &[], &group_idx());
        assert_eq!(d.lines.len(), 1);
        assert!(d.lines[0].text.contains("not in the draft list"));
    }
}
