//! Services under `/sec_policy/draft/services`. The body carries exactly
//! one port list, `service_ports`; the PCE silently drops a list it does
//! not accept and answers 201, which `Written::dropped_fields` reports. An
//! update `PUT`s only the keys it changes; `service_ports` only when ports
//! are added or removed.
use super::specs::PortSpec;
use super::{IfMissing, ObjectKind, Plan, WriteError, WriteRequest, Writer, checked_href};
use crate::client::WriteMethod;
use crate::models::Service;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDraft {
    pub name: String,
    pub description: Option<String>,
    pub ports: Vec<PortSpec>,
}

impl ServiceDraft {
    pub fn from_service(s: &Service) -> Self {
        Self {
            name: s.name.clone(),
            description: s.description.clone(),
            ports: s
                .service_ports
                .iter()
                .filter_map(PortSpec::from_service_port)
                .collect(),
        }
    }

    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        if self.name.trim().is_empty() {
            errs.push("service name must not be empty".into());
        }
        if let Err(e) = self.validate_ports() {
            errs.push(e);
        }
        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }

    fn validate_ports(&self) -> Result<(), String> {
        if self.ports.is_empty() {
            return Err("a service needs at least one port or protocol".into());
        }
        Ok(())
    }

    fn ports_json(&self) -> Value {
        Value::Array(self.ports.iter().map(PortSpec::to_json).collect())
    }

    pub(crate) fn to_body(&self) -> Value {
        let mut v = json!({
            "name": self.name.trim(),
            "service_ports": self.ports_json(),
        });
        if let Some(d) = &self.description {
            v["description"] = json!(d);
        }
        v
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServicePatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub add_ports: Vec<PortSpec>,
    pub remove_ports: Vec<PortSpec>,
}

impl ServicePatch {
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.description.is_none()
            && self.add_ports.is_empty()
            && self.remove_ports.is_empty()
    }

    /// True when `service_ports` will be sent.
    pub fn touches_ports(&self) -> bool {
        !self.add_ports.is_empty() || !self.remove_ports.is_empty()
    }

    /// "At least one port" is checked only when the ports change.
    pub fn apply(&self, current: &Service) -> Result<ServiceDraft, Vec<String>> {
        let mut d = ServiceDraft::from_service(current);
        let mut errs = Vec::new();
        for p in &self.remove_ports {
            let before = d.ports.len();
            d.ports.retain(|x| x != p);
            if d.ports.len() == before {
                errs.push(format!("port {} is not in {}", p.display(), current.name));
            }
        }
        for p in &self.add_ports {
            if !d.ports.contains(p) {
                d.ports.push(p.clone());
            }
        }
        if let Some(n) = &self.name {
            if n.trim().is_empty() {
                errs.push("service name must not be empty".into());
            }
            d.name = n.clone();
        }
        if let Some(desc) = &self.description {
            d.description = Some(desc.clone());
        }
        if self.touches_ports()
            && let Err(e) = d.validate_ports()
        {
            errs.push(e);
        }
        if errs.is_empty() { Ok(d) } else { Err(errs) }
    }

    /// The `PUT` body: only the keys this patch changes.
    pub(crate) fn body(&self, d: &ServiceDraft) -> Value {
        let mut v = serde_json::Map::new();
        if self.name.is_some() {
            v.insert("name".into(), json!(d.name.trim()));
        }
        if let Some(desc) = &self.description {
            v.insert("description".into(), json!(desc));
        }
        if self.touches_ports() {
            v.insert("service_ports".into(), d.ports_json());
        }
        Value::Object(v)
    }
}

impl Writer<'_> {
    pub async fn plan_create_service(
        &self,
        draft: &ServiceDraft,
        if_missing: IfMissing,
    ) -> Result<Plan, WriteError> {
        draft.validate().map_err(WriteError::Invalid)?;
        if if_missing == IfMissing::SkipExisting {
            let services = self.pce().services(self.bypass()).await?.value;
            if let Some(existing) = services
                .iter()
                .find(|s| s.update_type.as_deref() != Some("delete") && s.name == draft.name.trim())
            {
                return Ok(Plan::Noop {
                    existing_href: existing.href.clone(),
                    reason: format!("service {} exists", draft.name.trim()),
                });
            }
        }
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Post,
                path: self.pce().client().org_path("sec_policy/draft/services"),
                body: Some(draft.to_body()),
                summary: format!("create service {}", draft.name.trim()),
            },
            kind: ObjectKind::Service,
        })
    }

    pub async fn plan_update_service(
        &self,
        href: &str,
        patch: &ServicePatch,
    ) -> Result<Plan, WriteError> {
        let href = checked_href(href, "services", self.pce().client().org_id())?;
        if patch.is_empty() {
            return Err(WriteError::invalid("nothing to change"));
        }
        let raw = self.fetch(&href).await?;
        let has_windows = raw
            .get("windows_services")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty());
        let has_icmp = raw
            .get("service_ports")
            .and_then(Value::as_array)
            .is_some_and(|a| {
                let set = |p: &Value, k: &str| p.get(k).is_some_and(|v| !v.is_null());
                a.iter()
                    .any(|p| set(p, "icmp_type") || set(p, "icmp_code") || !set(p, "proto"))
            });
        // Only a changed port list can lose these fields.
        if patch.touches_ports() && (has_windows || has_icmp) {
            let name = raw.get("name").and_then(Value::as_str).unwrap_or(&href);
            return Err(WriteError::invalid(format!(
                "{name} has fields this tool cannot preserve (windows_services, icmp_type/icmp_code or a port without a protocol); edit it in the PCE UI"
            )));
        }
        let current: Service = serde_json::from_value(raw)
            .map_err(|e| WriteError::Pce(crate::PceError::Decode(format!("{href}: {e}"))))?;
        let draft = patch.apply(&current).map_err(WriteError::Invalid)?;
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Put,
                path: href.clone(),
                body: Some(patch.body(&draft)),
                summary: format!("update service {}", current.name),
            },
            kind: ObjectKind::Service,
        })
    }

    pub fn plan_delete_service(&self, href: &str) -> Result<Plan, WriteError> {
        let href = checked_href(href, "services", self.pce().client().org_id())?;
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Delete,
                path: href.clone(),
                body: None,
                summary: format!("delete service {href}"),
            },
            kind: ObjectKind::Service,
        })
    }
}
