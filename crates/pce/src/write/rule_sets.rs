//! Rulesets under `/sec_policy/draft/rule_sets`. A ruleset `PUT` carries
//! only the keys that change and never `rules` or `deny_rules`: rules have
//! their own nested hrefs (`write/rules.rs`).
use super::actors::{ActorKind, ActorRef};
use super::{
    IfMissing, ObjectKind, Plan, WriteError, WriteRequest, Writer, checked_policy_href, decode,
};
use crate::client::WriteMethod;
use crate::models::{PolicyScope, RuleSet};
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleSetDraft {
    pub name: String,
    pub description: Option<String>,
    pub enabled: bool,
    /// Alternatives; each inner list is one scope of labels or label
    /// groups. Empty: the ruleset applies everywhere.
    pub scopes: Vec<Vec<ActorRef>>,
}

const SCOPE_KINDS: &str = "scopes take labels and label groups only";

/// At most one label per dimension in a scope is not checked here (the
/// library has no label index); the PCE refuses it.
fn check_scopes(scopes: &[Vec<ActorRef>], errs: &mut Vec<String>) {
    for a in scopes.iter().flatten() {
        if !matches!(a.kind, ActorKind::Label | ActorKind::LabelGroup) {
            if !errs.iter().any(|e| e == SCOPE_KINDS) {
                errs.push(SCOPE_KINDS.into());
            }
        } else if let Err(e) = a.validate() {
            errs.push(e);
        }
    }
}

fn check_name(name: &str, errs: &mut Vec<String>) {
    if name.trim().is_empty() {
        errs.push("ruleset name must not be empty".into());
    }
}

fn done(errs: Vec<String>) -> Result<(), Vec<String>> {
    if errs.is_empty() { Ok(()) } else { Err(errs) }
}

/// The PCE represents "applies everywhere" as one empty scope (`[[]]`),
/// which is what every unscoped ruleset on a real PCE returns.
fn scopes_json(scopes: &[Vec<ActorRef>]) -> Value {
    if scopes.is_empty() {
        return json!([[]]);
    }
    Value::Array(
        scopes
            .iter()
            .map(|s| Value::Array(s.iter().map(ActorRef::to_json).collect()))
            .collect(),
    )
}

impl RuleSetDraft {
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        check_name(&self.name, &mut errs);
        check_scopes(&self.scopes, &mut errs);
        done(errs)
    }

    pub(crate) fn to_body(&self) -> Value {
        let mut v = json!({
            "name": self.name.trim(),
            "enabled": self.enabled,
            "scopes": scopes_json(&self.scopes),
        });
        if let Some(d) = &self.description {
            v["description"] = json!(d);
        }
        v
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleSetPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub enabled: Option<bool>,
    /// Replaces every scope; `Some(vec![])` makes the ruleset apply everywhere.
    pub set_scopes: Option<Vec<Vec<ActorRef>>>,
}

impl RuleSetPatch {
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.description.is_none()
            && self.enabled.is_none()
            && self.set_scopes.is_none()
    }

    /// The errors that need no current ruleset.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        if let Some(n) = &self.name {
            check_name(n, &mut errs);
        }
        if let Some(s) = &self.set_scopes {
            check_scopes(s, &mut errs);
        }
        done(errs)
    }

    /// The `PUT` body against `current`: a name, description or state equal
    /// to the current one contributes nothing; `set_scopes` is always sent.
    pub fn body(&self, current: &RuleSet) -> Value {
        let mut v = Map::new();
        if let Some(n) = &self.name
            && n.trim() != current.name
        {
            v.insert("name".into(), json!(n.trim()));
        }
        if let Some(d) = &self.description
            && current.description.as_deref().unwrap_or("") != d.as_str()
        {
            v.insert("description".into(), json!(d));
        }
        if let Some(e) = self.enabled
            && e != current.enabled
        {
            v.insert("enabled".into(), json!(e));
        }
        if let Some(s) = &self.set_scopes {
            v.insert("scopes".into(), scopes_json(s));
        }
        Value::Object(v)
    }
}

impl Writer<'_> {
    pub async fn plan_create_rule_set(
        &self,
        draft: &RuleSetDraft,
        if_missing: IfMissing,
    ) -> Result<Plan, WriteError> {
        draft.validate().map_err(WriteError::Invalid)?;
        if if_missing == IfMissing::Create {
            return self.rule_set_create(draft, None);
        }
        let sets = self
            .pce()
            .rule_sets(PolicyScope::Draft, self.bypass())
            .await?;
        if sets.truncated {
            return Err(WriteError::invalid(
                "too many rulesets to check --if-missing; create without it",
            ));
        }
        self.rule_set_create(draft, Some(&sets.value))
    }

    /// The create plan; with `existing` (a fresh read), a live ruleset of
    /// the same name (any case) is a `Noop` with its href, several are an
    /// error. No request.
    pub(crate) fn rule_set_create(
        &self,
        draft: &RuleSetDraft,
        existing: Option<&[RuleSet]>,
    ) -> Result<Plan, WriteError> {
        draft.validate().map_err(WriteError::Invalid)?;
        let name = draft.name.trim();
        if let Some(sets) = existing {
            let hits: Vec<&RuleSet> = sets
                .iter()
                .filter(|rs| {
                    rs.update_type.as_deref() != Some("delete")
                        && rs.name.eq_ignore_ascii_case(name)
                })
                .collect();
            match hits.as_slice() {
                [] => {}
                [one] => {
                    return Ok(Plan::Noop {
                        existing_href: one.href.clone(),
                        reason: format!("rule_set {name} exists"),
                    });
                }
                many => {
                    return Err(WriteError::invalid(format!(
                        "{name:?} matches several rulesets: {}",
                        many.iter()
                            .map(|rs| rs.href.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )));
                }
            }
        }
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Post,
                path: self.pce().client().org_path("sec_policy/draft/rule_sets"),
                body: Some(draft.to_body()),
                summary: format!("create rule_set {name}"),
                objects: Vec::new(),
            },
            kind: ObjectKind::RuleSet,
        })
    }

    pub async fn plan_update_rule_set(
        &self,
        href: &str,
        patch: &RuleSetPatch,
    ) -> Result<Plan, WriteError> {
        let href = checked_policy_href(href, "rule_sets", self.pce().client().org_id())?;
        if patch.is_empty() {
            return Err(WriteError::invalid("nothing to change"));
        }
        patch.validate().map_err(WriteError::Invalid)?;
        let current: RuleSet = decode(&href, self.fetch(&href).await?)?;
        let body = patch.body(&current);
        if body.as_object().is_some_and(Map::is_empty) {
            return Ok(Plan::Noop {
                existing_href: href,
                reason: format!("rule_set {} already matches", current.name),
            });
        }
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Put,
                path: href,
                body: Some(body),
                summary: format!("update rule_set {}", current.name),
                objects: Vec::new(),
            },
            kind: ObjectKind::RuleSet,
        })
    }

    pub fn plan_delete_rule_set(&self, href: &str) -> Result<Plan, WriteError> {
        let href = checked_policy_href(href, "rule_sets", self.pce().client().org_id())?;
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Delete,
                path: href.clone(),
                body: None,
                summary: format!("delete rule_set {href}"),
                objects: Vec::new(),
            },
            kind: ObjectKind::RuleSet,
        })
    }
}
