//! Allow rules (`sec_rules`) and deny rules inside a ruleset.
//!
//! Create is a `POST` under the ruleset's href; update and delete address
//! the nested href. An update `PUT`s only the keys it changes: a text or
//! bool field equal to the current value contributes nothing, and a
//! changed provider, consumer or service list is sent whole with the
//! untouched entries exactly as the PCE returned them.
use super::actors::{ActorRef, ServiceRef};
use super::{
    ObjectKind, Plan, WriteError, WriteRequest, Writer, checked_nested_href, checked_policy_href,
};
use crate::client::WriteMethod;
use crate::models::{Actor, IngressService, ResolveLabelsAs};
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, PartialEq)]
pub struct RuleDraft {
    pub description: Option<String>,
    pub enabled: bool,
    pub providers: Vec<ActorRef>,
    pub consumers: Vec<ActorRef>,
    pub services: Vec<ServiceRef>,
    pub unscoped_consumers: bool,
    /// Absent: omitted from the body, so the PCE applies its default.
    pub resolve_labels_as: Option<ResolveLabelsAs>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RulePatch {
    pub description: Option<String>,
    pub enabled: Option<bool>,
    pub add_providers: Vec<ActorRef>,
    pub remove_providers: Vec<ActorRef>,
    pub add_consumers: Vec<ActorRef>,
    pub remove_consumers: Vec<ActorRef>,
    pub add_services: Vec<ServiceRef>,
    pub remove_services: Vec<ServiceRef>,
    pub unscoped_consumers: Option<bool>,
    pub resolve_labels_as: Option<ResolveLabelsAs>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenyRuleDraft {
    pub description: Option<String>,
    pub enabled: bool,
    pub providers: Vec<ActorRef>,
    pub consumers: Vec<ActorRef>,
    pub services: Vec<ServiceRef>,
    /// Sent as `override`: an override deny is evaluated before allow rules.
    pub override_: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DenyRulePatch {
    pub description: Option<String>,
    pub enabled: Option<bool>,
    pub add_providers: Vec<ActorRef>,
    pub remove_providers: Vec<ActorRef>,
    pub add_consumers: Vec<ActorRef>,
    pub remove_consumers: Vec<ActorRef>,
    pub add_services: Vec<ServiceRef>,
    pub remove_services: Vec<ServiceRef>,
    pub override_: Option<bool>,
}

fn done(errs: Vec<String>) -> Result<(), Vec<String>> {
    if errs.is_empty() { Ok(()) } else { Err(errs) }
}

fn check_actors<'a>(actors: impl Iterator<Item = &'a ActorRef>, errs: &mut Vec<String>) {
    for a in actors {
        if let Err(e) = a.validate()
            && !errs.contains(&e)
        {
            errs.push(e);
        }
    }
}

fn check_sides(
    providers: &[ActorRef],
    consumers: &[ActorRef],
    services: &[ServiceRef],
    errs: &mut Vec<String>,
) {
    if providers.is_empty() {
        errs.push("a rule needs at least one provider".into());
    }
    if consumers.is_empty() {
        errs.push("a rule needs at least one consumer".into());
    }
    if services.is_empty() {
        errs.push("a rule needs at least one service".into());
    }
    check_actors(providers.iter().chain(consumers), errs);
}

fn check_resolve(r: &ResolveLabelsAs, errs: &mut Vec<String>) {
    for (side, values) in [("providers", &r.providers), ("consumers", &r.consumers)] {
        if values.is_empty() {
            errs.push(format!(
                "resolve_labels_as {side} must name workloads or virtual_services"
            ));
        }
        for v in values {
            if !matches!(v.as_str(), "workloads" | "virtual_services") {
                errs.push(format!(
                    "resolve_labels_as {side}: {v:?} is not workloads or virtual_services"
                ));
            }
        }
    }
}

fn actors_json(a: &[ActorRef]) -> Value {
    Value::Array(a.iter().map(ActorRef::to_json).collect())
}

fn services_json(s: &[ServiceRef]) -> Value {
    Value::Array(s.iter().map(ServiceRef::to_json).collect())
}

impl RuleDraft {
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        check_sides(&self.providers, &self.consumers, &self.services, &mut errs);
        if let Some(r) = &self.resolve_labels_as {
            check_resolve(r, &mut errs);
        }
        done(errs)
    }

    pub(crate) fn to_body(&self) -> Value {
        let mut v = json!({
            "enabled": self.enabled,
            "providers": actors_json(&self.providers),
            "consumers": actors_json(&self.consumers),
            "ingress_services": services_json(&self.services),
            "unscoped_consumers": self.unscoped_consumers,
        });
        if let Some(d) = &self.description {
            v["description"] = json!(d);
        }
        if let Some(r) = &self.resolve_labels_as {
            v["resolve_labels_as"] = json!({"providers": r.providers, "consumers": r.consumers});
        }
        v
    }
}

impl DenyRuleDraft {
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        check_sides(&self.providers, &self.consumers, &self.services, &mut errs);
        done(errs)
    }

    pub(crate) fn to_body(&self) -> Value {
        let mut v = json!({
            "enabled": self.enabled,
            "providers": actors_json(&self.providers),
            "consumers": actors_json(&self.consumers),
            "ingress_services": services_json(&self.services),
            "override": self.override_,
        });
        if let Some(d) = &self.description {
            v["description"] = json!(d);
        }
        v
    }
}

/// One entry of a provider, consumer or service list.
trait Entry: Clone {
    /// The entry as the PCE returned it; `None` when it cannot be matched
    /// (it is then carried untouched and no remove can hit it).
    fn parse(v: &Value) -> Option<Self>;
    fn same(&self, other: &Self) -> bool;
    fn json(&self) -> Value;
    fn label(&self) -> String;
}

impl Entry for ActorRef {
    fn parse(v: &Value) -> Option<Self> {
        let a: Actor = serde_json::from_value(v.clone()).ok()?;
        ActorRef::from_actor(&a).ok()
    }
    fn same(&self, other: &Self) -> bool {
        self.same_actor(other)
    }
    fn json(&self) -> Value {
        self.to_json()
    }
    fn label(&self) -> String {
        self.display()
    }
}

impl Entry for ServiceRef {
    fn parse(v: &Value) -> Option<Self> {
        let known = v
            .as_object()?
            .keys()
            .all(|k| matches!(k.as_str(), "href" | "port" | "to_port" | "proto"));
        if !known {
            return None;
        }
        let s: IngressService = serde_json::from_value(v.clone()).ok()?;
        Some(ServiceRef::from_ingress(&s))
    }
    fn same(&self, other: &Self) -> bool {
        self.same_service(other)
    }
    fn json(&self) -> Value {
        self.to_json()
    }
    fn label(&self) -> String {
        self.display()
    }
}

/// Removes, then adds. Returns the new list only when it changed; every
/// entry not removed is the PCE's JSON, unchanged.
fn patch_list<T: Entry>(
    what: &str,
    current: &[Value],
    add: &[T],
    remove: &[T],
    errs: &mut Vec<String>,
) -> Option<Vec<Value>> {
    let mut entries: Vec<(Option<T>, Value)> =
        current.iter().map(|v| (T::parse(v), v.clone())).collect();
    let mut changed = false;
    for r in remove {
        let before = entries.len();
        entries.retain(|(e, _)| !e.as_ref().is_some_and(|e| e.same(r)));
        if entries.len() == before {
            errs.push(format!("{} is not among the {what}", r.label()));
        } else {
            changed = true;
        }
    }
    for a in add {
        if !entries
            .iter()
            .any(|(e, _)| e.as_ref().is_some_and(|e| e.same(a)))
        {
            entries.push((Some(a.clone()), a.json()));
            changed = true;
        }
    }
    changed.then(|| entries.into_iter().map(|(_, v)| v).collect())
}

fn list_of<'v>(current: &'v Value, key: &str) -> &'v [Value] {
    current
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn text_change(body: &mut Map<String, Value>, current: &Value, key: &str, new: Option<&str>) {
    if let Some(new) = new
        && current.get(key).and_then(Value::as_str).unwrap_or("") != new
    {
        body.insert(key.into(), json!(new));
    }
}

fn bool_change(body: &mut Map<String, Value>, current: &Value, key: &str, new: Option<bool>) {
    if let Some(new) = new
        && current.get(key).and_then(Value::as_bool).unwrap_or(false) != new
    {
        body.insert(key.into(), json!(new));
    }
}

/// The list edits a rule patch and a deny-rule patch share.
struct Sides<'a> {
    add_providers: &'a [ActorRef],
    remove_providers: &'a [ActorRef],
    add_consumers: &'a [ActorRef],
    remove_consumers: &'a [ActorRef],
    add_services: &'a [ServiceRef],
    remove_services: &'a [ServiceRef],
}

impl Sides<'_> {
    fn is_empty(&self) -> bool {
        self.add_providers.is_empty()
            && self.remove_providers.is_empty()
            && self.add_consumers.is_empty()
            && self.remove_consumers.is_empty()
            && self.add_services.is_empty()
            && self.remove_services.is_empty()
    }

    fn check(&self, errs: &mut Vec<String>) {
        check_actors(self.add_providers.iter().chain(self.add_consumers), errs);
    }

    fn apply(&self, current: &Value, body: &mut Map<String, Value>, errs: &mut Vec<String>) {
        for (key, add, remove, empty) in [
            (
                "providers",
                self.add_providers,
                self.remove_providers,
                "a rule needs at least one provider",
            ),
            (
                "consumers",
                self.add_consumers,
                self.remove_consumers,
                "a rule needs at least one consumer",
            ),
        ] {
            if let Some(list) = patch_list(key, list_of(current, key), add, remove, errs) {
                if list.is_empty() {
                    errs.push(empty.into());
                }
                body.insert(key.into(), Value::Array(list));
            }
        }
        if let Some(list) = patch_list(
            "services",
            list_of(current, "ingress_services"),
            self.add_services,
            self.remove_services,
            errs,
        ) {
            if list.is_empty() {
                errs.push("a rule needs at least one service".into());
            }
            body.insert("ingress_services".into(), Value::Array(list));
        }
    }
}

impl RulePatch {
    fn sides(&self) -> Sides<'_> {
        Sides {
            add_providers: &self.add_providers,
            remove_providers: &self.remove_providers,
            add_consumers: &self.add_consumers,
            remove_consumers: &self.remove_consumers,
            add_services: &self.add_services,
            remove_services: &self.remove_services,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.description.is_none()
            && self.enabled.is_none()
            && self.unscoped_consumers.is_none()
            && self.resolve_labels_as.is_none()
            && self.sides().is_empty()
    }

    /// The errors that need no current rule.
    pub fn check(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        self.sides().check(&mut errs);
        if let Some(r) = &self.resolve_labels_as {
            check_resolve(r, &mut errs);
        }
        done(errs)
    }
}

impl DenyRulePatch {
    fn sides(&self) -> Sides<'_> {
        Sides {
            add_providers: &self.add_providers,
            remove_providers: &self.remove_providers,
            add_consumers: &self.add_consumers,
            remove_consumers: &self.remove_consumers,
            add_services: &self.add_services,
            remove_services: &self.remove_services,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.description.is_none()
            && self.enabled.is_none()
            && self.override_.is_none()
            && self.sides().is_empty()
    }

    pub fn check(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        self.sides().check(&mut errs);
        done(errs)
    }
}

/// The `PUT` body for `patch` against the rule as the PCE returned it:
/// only the keys that change.
pub fn apply_rule_patch(current: &Value, patch: &RulePatch) -> Result<Value, Vec<String>> {
    let mut errs = patch.check().err().unwrap_or_default();
    let mut body = Map::new();
    text_change(
        &mut body,
        current,
        "description",
        patch.description.as_deref(),
    );
    bool_change(&mut body, current, "enabled", patch.enabled);
    bool_change(
        &mut body,
        current,
        "unscoped_consumers",
        patch.unscoped_consumers,
    );
    if let Some(r) = &patch.resolve_labels_as {
        let new = json!({"providers": r.providers, "consumers": r.consumers});
        if current.get("resolve_labels_as") != Some(&new) {
            body.insert("resolve_labels_as".into(), new);
        }
    }
    patch.sides().apply(current, &mut body, &mut errs);
    if errs.is_empty() {
        Ok(Value::Object(body))
    } else {
        Err(errs)
    }
}

/// [`apply_rule_patch`] for a deny rule (`override` instead of
/// `unscoped_consumers` and `resolve_labels_as`).
pub fn apply_deny_rule_patch(current: &Value, patch: &DenyRulePatch) -> Result<Value, Vec<String>> {
    let mut errs = patch.check().err().unwrap_or_default();
    let mut body = Map::new();
    text_change(
        &mut body,
        current,
        "description",
        patch.description.as_deref(),
    );
    bool_change(&mut body, current, "enabled", patch.enabled);
    bool_change(&mut body, current, "override", patch.override_);
    patch.sides().apply(current, &mut body, &mut errs);
    if errs.is_empty() {
        Ok(Value::Object(body))
    } else {
        Err(errs)
    }
}

fn create(ruleset: &str, child: &str, what: &str, body: Value, kind: ObjectKind) -> Plan {
    Plan::Send {
        request: WriteRequest {
            method: WriteMethod::Post,
            path: format!("{ruleset}/{child}"),
            body: Some(body),
            summary: format!("create {what} in rule_set {ruleset}"),
        },
        kind,
    }
}

fn update(href: String, what: &str, body: Value, kind: ObjectKind) -> Plan {
    if body.as_object().is_some_and(Map::is_empty) {
        return Plan::Noop {
            existing_href: href,
            reason: format!("{what} already matches"),
        };
    }
    Plan::Send {
        request: WriteRequest {
            method: WriteMethod::Put,
            path: href.clone(),
            body: Some(body),
            summary: format!("update {what} {href}"),
        },
        kind,
    }
}

fn delete(href: String, what: &str, kind: ObjectKind) -> Plan {
    Plan::Send {
        request: WriteRequest {
            method: WriteMethod::Delete,
            path: href.clone(),
            body: None,
            summary: format!("delete {what} {href}"),
        },
        kind,
    }
}

impl Writer<'_> {
    /// One GET of a rule or deny rule, kept as JSON (see `apply_rule_patch`).
    async fn fetch_rule(&self, href: &str) -> Result<Value, WriteError> {
        let v = self.fetch(href).await?;
        if v.is_object() {
            Ok(v)
        } else {
            Err(WriteError::Pce(crate::PceError::Decode(format!(
                "{href}: expected a rule object"
            ))))
        }
    }

    pub fn plan_create_rule(
        &self,
        ruleset_href: &str,
        draft: &RuleDraft,
    ) -> Result<Plan, WriteError> {
        let rs = checked_policy_href(ruleset_href, "rule_sets", self.pce().client().org_id())?;
        draft.validate().map_err(WriteError::Invalid)?;
        Ok(create(
            &rs,
            "sec_rules",
            "rule",
            draft.to_body(),
            ObjectKind::Rule,
        ))
    }

    pub async fn plan_update_rule(
        &self,
        href: &str,
        patch: &RulePatch,
    ) -> Result<Plan, WriteError> {
        let href =
            checked_nested_href(href, "rule_sets", "sec_rules", self.pce().client().org_id())?;
        if patch.is_empty() {
            return Err(WriteError::invalid("nothing to change"));
        }
        patch.check().map_err(WriteError::Invalid)?;
        let current = self.fetch_rule(&href).await?;
        let body = apply_rule_patch(&current, patch).map_err(WriteError::Invalid)?;
        Ok(update(href, "rule", body, ObjectKind::Rule))
    }

    pub fn plan_delete_rule(&self, href: &str) -> Result<Plan, WriteError> {
        let href =
            checked_nested_href(href, "rule_sets", "sec_rules", self.pce().client().org_id())?;
        Ok(delete(href, "rule", ObjectKind::Rule))
    }

    pub fn plan_create_deny_rule(
        &self,
        ruleset_href: &str,
        draft: &DenyRuleDraft,
    ) -> Result<Plan, WriteError> {
        let rs = checked_policy_href(ruleset_href, "rule_sets", self.pce().client().org_id())?;
        draft.validate().map_err(WriteError::Invalid)?;
        Ok(create(
            &rs,
            "deny_rules",
            "deny_rule",
            draft.to_body(),
            ObjectKind::DenyRule,
        ))
    }

    pub async fn plan_update_deny_rule(
        &self,
        href: &str,
        patch: &DenyRulePatch,
    ) -> Result<Plan, WriteError> {
        let href = checked_nested_href(
            href,
            "rule_sets",
            "deny_rules",
            self.pce().client().org_id(),
        )?;
        if patch.is_empty() {
            return Err(WriteError::invalid("nothing to change"));
        }
        patch.check().map_err(WriteError::Invalid)?;
        let current = self.fetch_rule(&href).await?;
        let body = apply_deny_rule_patch(&current, patch).map_err(WriteError::Invalid)?;
        Ok(update(href, "deny_rule", body, ObjectKind::DenyRule))
    }

    pub fn plan_delete_deny_rule(&self, href: &str) -> Result<Plan, WriteError> {
        let href = checked_nested_href(
            href,
            "rule_sets",
            "deny_rules",
            self.pce().client().org_id(),
        )?;
        Ok(delete(href, "deny_rule", ObjectKind::DenyRule))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write::actors::ActorKind;
    use crate::write::specs::PortSpec;

    fn rule2() -> Value {
        json!({
            "href": "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2", "enabled": true,
            "description": "corp to web",
            "providers": [{"label": {"href": "/orgs/5/labels/1"}}],
            "consumers": [{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}],
            "ingress_services": [
                {"port": 443, "proto": 6},
                {"port": 8000, "to_port": 8099, "proto": 6},
                {"proto": 1, "icmp_type": 8}
            ],
            "resolve_labels_as": {"providers": ["workloads"], "consumers": ["workloads"]},
            "unscoped_consumers": true
        })
    }

    fn port(s: &str) -> ServiceRef {
        ServiceRef::Port(PortSpec::parse(s).unwrap())
    }

    fn label(id: u32) -> ActorRef {
        ActorRef::new(ActorKind::Label, format!("/orgs/5/labels/{id}"))
    }

    #[test]
    fn removing_a_range_by_its_spec_keeps_the_other_entries_verbatim() {
        let patch = RulePatch {
            remove_services: vec![port("8000-8099/tcp")],
            ..RulePatch::default()
        };
        assert_eq!(
            apply_rule_patch(&rule2(), &patch).unwrap(),
            json!({"ingress_services": [{"port": 443, "proto": 6}, {"proto": 1, "icmp_type": 8}]})
        );
    }

    #[test]
    fn an_entry_with_unknown_keys_is_never_matched_by_a_remove() {
        let patch = RulePatch {
            remove_services: vec![port("icmp")],
            ..RulePatch::default()
        };
        assert_eq!(
            apply_rule_patch(&rule2(), &patch).unwrap_err(),
            vec!["icmp is not among the services"]
        );
    }

    #[test]
    fn adding_an_existing_entry_changes_nothing_and_a_swap_is_one_list() {
        let same = RulePatch {
            add_services: vec![port("443/tcp")],
            add_consumers: vec![ActorRef::new(
                ActorKind::IpList,
                "/orgs/5/sec_policy/draft/ip_lists/21",
            )],
            ..RulePatch::default()
        };
        assert_eq!(apply_rule_patch(&rule2(), &same).unwrap(), json!({}));
        let swap = RulePatch {
            remove_providers: vec![label(1)],
            add_providers: vec![label(2)],
            ..RulePatch::default()
        };
        assert_eq!(
            apply_rule_patch(&rule2(), &swap).unwrap(),
            json!({"providers": [{"label": {"href": "/orgs/5/labels/2"}}]})
        );
    }

    #[test]
    fn removing_the_last_provider_or_an_absent_consumer_is_refused() {
        let last = RulePatch {
            remove_providers: vec![label(1)],
            ..RulePatch::default()
        };
        assert_eq!(
            apply_rule_patch(&rule2(), &last).unwrap_err(),
            vec!["a rule needs at least one provider"]
        );
        let absent = RulePatch {
            remove_consumers: vec![ActorRef::all()],
            ..RulePatch::default()
        };
        assert_eq!(
            apply_rule_patch(&rule2(), &absent).unwrap_err(),
            vec!["all is not among the consumers"]
        );
    }

    #[test]
    fn text_and_bool_fields_equal_to_the_current_value_send_nothing() {
        let patch = RulePatch {
            description: Some("corp to web".into()),
            enabled: Some(true),
            unscoped_consumers: Some(false),
            resolve_labels_as: Some(ResolveLabelsAs {
                providers: vec!["workloads".into()],
                consumers: vec!["workloads".into()],
            }),
            ..RulePatch::default()
        };
        assert_eq!(
            apply_rule_patch(&rule2(), &patch).unwrap(),
            json!({"unscoped_consumers": false})
        );
    }

    #[test]
    fn deny_patch_toggles_override_and_changes_only_what_differs() {
        let current = json!({
            "href": "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8", "enabled": true,
            "providers": [{"label": {"href": "/orgs/5/labels/2"}}], "consumers": [{"actors": "ams"}],
            "ingress_services": [{"port": 22, "proto": 6}], "override": true
        });
        let off = DenyRulePatch {
            override_: Some(false),
            ..DenyRulePatch::default()
        };
        assert_eq!(
            apply_deny_rule_patch(&current, &off).unwrap(),
            json!({"override": false})
        );
        let same = DenyRulePatch {
            override_: Some(true),
            description: Some("ssh".into()),
            ..DenyRulePatch::default()
        };
        assert_eq!(
            apply_deny_rule_patch(&current, &same).unwrap(),
            json!({"description": "ssh"})
        );
    }

    #[test]
    fn excluded_all_and_bad_resolve_labels_as_are_refused() {
        let p = RulePatch {
            add_consumers: vec![ActorRef::all().excluded()],
            ..RulePatch::default()
        };
        assert_eq!(p.check().unwrap_err(), vec!["all cannot be excluded"]);
        let d = RuleDraft {
            description: None,
            enabled: true,
            providers: vec![ActorRef::all()],
            consumers: vec![ActorRef::all()],
            services: vec![port("443/tcp")],
            unscoped_consumers: false,
            resolve_labels_as: Some(ResolveLabelsAs {
                providers: vec![],
                consumers: vec!["vms".into()],
            }),
        };
        assert_eq!(
            d.validate().unwrap_err(),
            vec![
                "resolve_labels_as providers must name workloads or virtual_services",
                "resolve_labels_as consumers: \"vms\" is not workloads or virtual_services",
            ]
        );
        assert!(RulePatch::default().is_empty());
        assert!(DenyRulePatch::default().is_empty());
    }
}
