//! Unmanaged workloads: `POST /orgs/{org}/workloads`, `PUT|DELETE
//! /orgs/{org}/workloads/{id}`. A VEN-managed workload is refused locally:
//! the VEN owns its hostname and interfaces, and label changes on managed
//! workloads are a later plan. `labels` and `interfaces` are replaced
//! wholesale by the PCE, so a patch sends the full list it computed.
use super::specs::InterfaceSpec;
use super::{IfMissing, ObjectKind, Plan, WriteError, WriteRequest, Writer, checked_href};
use crate::client::WriteMethod;
use crate::models::{Workload, WorkloadFilter};
use crate::resolve::NameIndex;
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadDraft {
    pub name: String,
    pub hostname: Option<String>,
    pub description: Option<String>,
    pub interfaces: Vec<InterfaceSpec>,
    /// Label hrefs.
    pub labels: Vec<String>,
}

impl WorkloadDraft {
    pub fn hostname(&self) -> String {
        self.hostname
            .as_deref()
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .unwrap_or(self.name.trim())
            .to_string()
    }

    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        if self.name.trim().is_empty() {
            errs.push("workload name must not be empty".into());
        }
        if self.hostname().is_empty() && self.interfaces.is_empty() {
            errs.push("a workload needs a hostname or at least one interface".into());
        }
        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }

    pub(crate) fn to_body(&self) -> Value {
        let mut v = json!({
            "name": self.name.trim(),
            "hostname": self.hostname(),
            "interfaces": self.interfaces.iter().map(InterfaceSpec::to_json).collect::<Vec<_>>(),
            "labels": self.labels.iter().map(|h| json!({"href": h})).collect::<Vec<_>>(),
        });
        if let Some(d) = &self.description {
            v["description"] = json!(d);
        }
        v
    }
}

/// `--remove-label app` (any value) or `--remove-label app=web`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelRemoval {
    pub key: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkloadPatch {
    pub name: Option<String>,
    pub hostname: Option<String>,
    pub description: Option<String>,
    /// `ADDR[/N]` strings; interface names are assigned on apply.
    pub add_addresses: Vec<String>,
    pub remove_addresses: Vec<String>,
    /// Replace the whole label set (hrefs). Conflicts with add/remove.
    pub set_labels: Option<Vec<String>>,
    pub add_labels: Vec<String>,
    pub remove_labels: Vec<LabelRemoval>,
}

fn next_interface_name(existing: &[InterfaceSpec]) -> String {
    let mut n = 0;
    loop {
        let name = format!("eth{n}");
        if !existing.iter().any(|i| i.name == name) {
            return name;
        }
        n += 1;
    }
}

impl WorkloadPatch {
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.hostname.is_none()
            && self.description.is_none()
            && self.add_addresses.is_empty()
            && self.remove_addresses.is_empty()
            && self.set_labels.is_none()
            && self.add_labels.is_empty()
            && self.remove_labels.is_empty()
    }

    /// The `PUT` body: only the keys this patch changes. `idx` supplies a
    /// label's key when the workload's own label entry lacks it.
    pub fn apply(&self, current: &Workload, idx: &NameIndex) -> Result<Value, Vec<String>> {
        let mut body = Map::new();
        let mut errs = Vec::new();
        let who = current
            .hostname
            .clone()
            .or_else(|| current.name.clone())
            .unwrap_or_else(|| current.href.clone());

        if let Some(n) = &self.name {
            body.insert("name".into(), json!(n));
        }
        if let Some(h) = &self.hostname {
            body.insert("hostname".into(), json!(h));
        }
        if let Some(d) = &self.description {
            body.insert("description".into(), json!(d));
        }

        if !self.add_addresses.is_empty() || !self.remove_addresses.is_empty() {
            let mut ifaces: Vec<InterfaceSpec> = current
                .interfaces
                .iter()
                .map(InterfaceSpec::from_interface)
                .collect();
            for a in &self.remove_addresses {
                let addr = a.split('/').next().unwrap_or(a).trim();
                let before = ifaces.len();
                ifaces.retain(|i| i.address != addr);
                if ifaces.len() == before {
                    errs.push(format!("address {addr} is not an interface of {who}"));
                }
            }
            for a in &self.add_addresses {
                match InterfaceSpec::parse(a, next_interface_name(&ifaces)) {
                    Ok(spec) => {
                        if !ifaces.iter().any(|i| i.address == spec.address) {
                            ifaces.push(spec);
                        }
                    }
                    Err(e) => errs.push(e),
                }
            }
            let hostname = self
                .hostname
                .clone()
                .or_else(|| current.hostname.clone())
                .unwrap_or_default();
            if ifaces.is_empty() && hostname.trim().is_empty() {
                errs.push(format!(
                    "{who} would have no interface and no hostname; keep one interface or set --hostname"
                ));
            }
            body.insert(
                "interfaces".into(),
                Value::Array(ifaces.iter().map(InterfaceSpec::to_json).collect()),
            );
        }

        if let Some(set) = &self.set_labels {
            body.insert(
                "labels".into(),
                Value::Array(set.iter().map(|h| json!({"href": h})).collect()),
            );
        } else if !self.add_labels.is_empty() || !self.remove_labels.is_empty() {
            // (href, key, value) for every current label.
            let mut labels: Vec<(String, String, String)> = current
                .labels
                .iter()
                .map(|l| {
                    let from_idx = idx.label(&l.href);
                    (
                        l.href.clone(),
                        l.key
                            .clone()
                            .or_else(|| from_idx.map(|x| x.key.clone()))
                            .unwrap_or_default(),
                        l.value
                            .clone()
                            .or_else(|| from_idx.map(|x| x.value.clone()))
                            .unwrap_or_default(),
                    )
                })
                .collect();
            for r in &self.remove_labels {
                let before = labels.len();
                labels.retain(|(_, k, v)| {
                    !(k == &r.key && r.value.as_deref().is_none_or(|rv| rv == v))
                });
                if labels.len() == before {
                    let what = match &r.value {
                        Some(v) => format!("{}={v}", r.key),
                        None => r.key.clone(),
                    };
                    errs.push(format!("{who} has no label {what}"));
                }
            }
            for h in &self.add_labels {
                if !labels.iter().any(|(x, _, _)| x == h) {
                    let (k, v) = idx
                        .label(h)
                        .map(|l| (l.key.clone(), l.value.clone()))
                        .unwrap_or_default();
                    labels.push((h.clone(), k, v));
                }
            }
            body.insert(
                "labels".into(),
                Value::Array(labels.iter().map(|(h, _, _)| json!({"href": h})).collect()),
            );
        }

        if errs.is_empty() {
            Ok(Value::Object(body))
        } else {
            Err(errs)
        }
    }
}

fn require_unmanaged(w: &Workload) -> Result<(), WriteError> {
    if w.managed == Some(true) {
        let who = w
            .hostname
            .clone()
            .or_else(|| w.name.clone())
            .unwrap_or_else(|| w.href.clone());
        return Err(WriteError::invalid(format!(
            "{who} is VEN-managed; this version writes unmanaged workloads only"
        )));
    }
    Ok(())
}

impl Writer<'_> {
    /// Exact match on hostname (case-insensitive), then on name. The PCE's
    /// `?hostname=` and `?name=` are substring filters, so the result is
    /// narrowed here; none or several matches is an error listing them.
    pub async fn find_workload(&self, name_or_hostname: &str) -> Result<Workload, WriteError> {
        let needle = name_or_hostname.trim();
        let by_host = self
            .pce()
            .workloads(
                &WorkloadFilter {
                    hostname: Some(needle.to_string()),
                    ..WorkloadFilter::default()
                },
                self.bypass(),
            )
            .await?
            .value;
        let mut hits: Vec<Workload> = by_host
            .into_iter()
            .filter(|w| {
                w.hostname
                    .as_deref()
                    .is_some_and(|h| h.eq_ignore_ascii_case(needle))
            })
            .collect();
        if hits.is_empty() {
            let by_name = self
                .pce()
                .workloads(
                    &WorkloadFilter {
                        name: Some(needle.to_string()),
                        ..WorkloadFilter::default()
                    },
                    self.bypass(),
                )
                .await?
                .value;
            hits = by_name
                .into_iter()
                .filter(|w| w.name.as_deref() == Some(needle))
                .collect();
        }
        match hits.len() {
            0 => Err(WriteError::invalid(format!(
                "no workload with hostname or name {needle:?}"
            ))),
            1 => Ok(hits.remove(0)),
            _ => Err(WriteError::invalid(format!(
                "{needle:?} matches several workloads: {}; use the href",
                hits.iter()
                    .map(|w| w.href.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))),
        }
    }

    pub async fn plan_create_workload(
        &self,
        draft: &WorkloadDraft,
        if_missing: IfMissing,
    ) -> Result<Plan, WriteError> {
        draft.validate().map_err(WriteError::Invalid)?;
        if if_missing == IfMissing::SkipExisting
            && let Ok(existing) = self.find_workload(&draft.hostname()).await
        {
            return Ok(Plan::Noop {
                existing_href: existing.href.clone(),
                reason: format!("workload {} exists", draft.hostname()),
            });
        }
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Post,
                path: self.pce().client().org_path("workloads"),
                body: Some(draft.to_body()),
                summary: format!("create workload {}", draft.name.trim()),
            },
            kind: ObjectKind::Workload,
        })
    }

    async fn fetch_workload(&self, href: &str) -> Result<Workload, WriteError> {
        serde_json::from_value(self.fetch(href).await?)
            .map_err(|e| WriteError::Pce(crate::PceError::Decode(format!("{href}: {e}"))))
    }

    pub async fn plan_update_workload(
        &self,
        href: &str,
        patch: &WorkloadPatch,
        idx: &NameIndex,
    ) -> Result<Plan, WriteError> {
        let href = checked_href(href, "workloads")?;
        if patch.is_empty() {
            return Err(WriteError::invalid("nothing to change"));
        }
        let current = self.fetch_workload(&href).await?;
        require_unmanaged(&current)?;
        let body = patch.apply(&current, idx).map_err(WriteError::Invalid)?;
        let who = current
            .hostname
            .clone()
            .or(current.name.clone())
            .unwrap_or(href.clone());
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Put,
                path: href,
                body: Some(body),
                summary: format!("update workload {who}"),
            },
            kind: ObjectKind::Workload,
        })
    }

    pub async fn plan_delete_workload(&self, href: &str) -> Result<Plan, WriteError> {
        let href = checked_href(href, "workloads")?;
        let current = self.fetch_workload(&href).await?;
        require_unmanaged(&current)?;
        let who = current
            .hostname
            .clone()
            .or(current.name.clone())
            .unwrap_or(href.clone());
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Delete,
                path: href,
                body: None,
                summary: format!("delete workload {who}"),
            },
            kind: ObjectKind::Workload,
        })
    }
}
