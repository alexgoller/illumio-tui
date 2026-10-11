//! Provisioning: `POST /orgs/N/sec_policy` with an explicit
//! `change_subset`, built from one fresh read of the pending list. A rule
//! is provisioned through its ruleset, so rule hrefs never appear.
use super::{
    ObjectKind, Plan, WriteError, WriteRequest, Writer, checked_nested_href, checked_policy_href,
    parent_href,
};
use crate::client::WriteMethod;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvisionSubset {
    /// Every pending change, listed explicitly.
    All,
    /// These hrefs; each must be pending. A rule href means its ruleset.
    Hrefs(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionRequest {
    pub description: Option<String>,
    pub subset: ProvisionSubset,
}

/// Children of a ruleset: provisioned through the ruleset itself.
const RULE_CHILDREN: [&str; 2] = ["sec_rules", "deny_rules"];

/// Object type → number of hrefs in the body's `change_subset`.
pub fn provision_counts(body: &Value) -> BTreeMap<String, usize> {
    body.get("change_subset")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_array().map(|a| (k.clone(), a.len())))
                .filter(|(_, n)| *n > 0)
                .collect()
        })
        .unwrap_or_default()
}

/// `provision 1 ip_list, 3 rule_sets`.
fn summary(counts: &BTreeMap<String, usize>) -> String {
    let parts: Vec<String> = counts
        .iter()
        .map(|(k, n)| {
            if *n == 1 {
                format!("1 {}", k.strip_suffix('s').unwrap_or(k))
            } else {
                format!("{n} {k}")
            }
        })
        .collect();
    format!("provision {}", parts.join(", "))
}

/// `(object type, draft href)` for a pending or requested href. A rule or
/// deny rule maps to its ruleset; anything else is a draft policy object
/// grouped under its own type segment. Parsed by component, checked strictly.
fn provisionable(href: &str, org_id: u64) -> Result<(String, String), WriteError> {
    let trimmed = href.trim().trim_end_matches('/');
    let parts: Vec<&str> = trimmed.split('/').collect();
    if let [_, _, _, _, _, _, _, child, _] = parts.as_slice()
        && RULE_CHILDREN.contains(child)
    {
        let nested = checked_nested_href(trimmed, "rule_sets", child, org_id)?;
        return Ok(("rule_sets".into(), parent_href(&nested)));
    }
    let kind = parts.get(5).copied().unwrap_or_default();
    if kind.is_empty()
        || NOT_POLICY.contains(&kind)
        || !kind.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
    {
        return Err(WriteError::invalid(format!(
            "{trimmed} is not a draft policy object href"
        )));
    }
    let checked = checked_policy_href(trimmed, kind, org_id)?;
    Ok((kind.to_string(), checked))
}

/// Objects that live outside the draft policy and are never provisioned.
const NOT_POLICY: [&str; 2] = ["workloads", "labels"];

impl Writer<'_> {
    pub async fn plan_provision(&self, req: &ProvisionRequest) -> Result<Plan, WriteError> {
        let org = self.pce().client().org_id();
        // Requested hrefs are checked before any request.
        let requested: Option<Vec<(String, String)>> = match &req.subset {
            ProvisionSubset::All => None,
            ProvisionSubset::Hrefs(h) if h.is_empty() => {
                return Err(WriteError::invalid(
                    "nothing to provision: name at least one href, or provision all",
                ));
            }
            ProvisionSubset::Hrefs(h) => Some(
                h.iter()
                    .map(|x| provisionable(x, org))
                    .collect::<Result<_, _>>()?,
            ),
        };
        let pending = self.pce().pending_changes(self.bypass()).await?.value;
        let mut pending_set: BTreeSet<(String, String)> = BTreeSet::new();
        for p in &pending {
            match provisionable(&p.href, org) {
                Ok(entry) => {
                    pending_set.insert(entry);
                }
                // `All` must mean all: an entry this version cannot
                // provision refuses the whole plan.
                Err(e) if requested.is_none() => return Err(e),
                Err(_) => {}
            }
        }
        let chosen: BTreeSet<(String, String)> = match requested {
            None => pending_set,
            Some(list) => {
                if let Some((_, h)) = list.iter().find(|e| !pending_set.contains(*e)) {
                    return Err(WriteError::invalid(format!("{h} has no pending changes")));
                }
                list.into_iter().collect()
            }
        };
        if chosen.is_empty() {
            return Ok(Plan::Noop {
                existing_href: self.pce().client().org_path("sec_policy/pending"),
                reason: "nothing pending".into(),
            });
        }
        let mut subset: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for (kind, href) in chosen {
            subset
                .entry(kind)
                .or_default()
                .push(json!({ "href": href }));
        }
        let description = req
            .description
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .unwrap_or("illumio-tui provision");
        let body = json!({
            "update_description": description,
            "change_subset": subset,
        });
        let counts = provision_counts(&body);
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Post,
                path: self.pce().client().org_path("sec_policy"),
                body: Some(body),
                summary: summary(&counts),
            },
            kind: ObjectKind::Provision,
        })
    }
}
