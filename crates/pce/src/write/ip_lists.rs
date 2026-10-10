//! IP lists under `/sec_policy/draft/ip_lists`. Updates are a full `PUT`
//! computed locally: the PCE replaces `ip_ranges` and `fqdns` wholesale.
use super::specs::IpRangeSpec;
use super::{IfMissing, ObjectKind, Plan, WriteError, WriteRequest, Writer, checked_href};
use crate::client::WriteMethod;
use crate::models::IpList;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpListDraft {
    pub name: String,
    pub description: Option<String>,
    pub ranges: Vec<IpRangeSpec>,
    pub fqdns: Vec<String>,
}

impl IpListDraft {
    pub fn from_ip_list(l: &IpList) -> Self {
        Self {
            name: l.name.clone(),
            description: l.description.clone(),
            ranges: l.ip_ranges.iter().map(IpRangeSpec::from_range).collect(),
            fqdns: l.fqdns.iter().map(|f| f.fqdn.clone()).collect(),
        }
    }

    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        if self.name.trim().is_empty() {
            errs.push("IP list name must not be empty".into());
        }
        if self.ranges.is_empty() && self.fqdns.is_empty() {
            errs.push("an IP list needs at least one range or FQDN".into());
        }
        if self.fqdns.iter().any(|f| f.trim().is_empty()) {
            errs.push("FQDN must not be empty".into());
        }
        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }

    pub(crate) fn to_body(&self) -> Value {
        let mut v = json!({
            "name": self.name.trim(),
            "ip_ranges": self.ranges.iter().map(IpRangeSpec::to_json).collect::<Vec<_>>(),
            "fqdns": self.fqdns.iter().map(|f| json!({"fqdn": f.trim()})).collect::<Vec<_>>(),
        });
        if let Some(d) = &self.description {
            v["description"] = json!(d);
        }
        v
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IpListPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub add_ranges: Vec<IpRangeSpec>,
    pub remove_ranges: Vec<IpRangeSpec>,
    pub add_fqdns: Vec<String>,
    pub remove_fqdns: Vec<String>,
}

impl IpListPatch {
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.description.is_none()
            && self.add_ranges.is_empty()
            && self.remove_ranges.is_empty()
            && self.add_fqdns.is_empty()
            && self.remove_fqdns.is_empty()
    }

    /// The full list after this patch, validated. A remove that matches
    /// nothing is an error so a typo never silently leaves a range behind.
    pub fn apply(&self, current: &IpList) -> Result<IpListDraft, Vec<String>> {
        let mut d = IpListDraft::from_ip_list(current);
        let mut errs = Vec::new();
        for r in &self.remove_ranges {
            let before = d.ranges.len();
            d.ranges.retain(|x| !x.same_range(r));
            if d.ranges.len() == before {
                errs.push(format!("range {} is not in {}", r.display(), current.name));
            }
        }
        for r in &self.add_ranges {
            if !d.ranges.iter().any(|x| x.same_range(r)) {
                d.ranges.push(r.clone());
            }
        }
        for f in &self.remove_fqdns {
            let before = d.fqdns.len();
            d.fqdns.retain(|x| x != f);
            if d.fqdns.len() == before {
                errs.push(format!("fqdn {f} is not in {}", current.name));
            }
        }
        for f in &self.add_fqdns {
            if !d.fqdns.contains(f) {
                d.fqdns.push(f.clone());
            }
        }
        if let Some(n) = &self.name {
            d.name = n.clone();
        }
        if let Some(desc) = &self.description {
            d.description = Some(desc.clone());
        }
        if let Err(mut e) = d.validate() {
            errs.append(&mut e);
        }
        if errs.is_empty() { Ok(d) } else { Err(errs) }
    }
}

impl Writer<'_> {
    pub async fn plan_create_ip_list(
        &self,
        draft: &IpListDraft,
        if_missing: IfMissing,
    ) -> Result<Plan, WriteError> {
        draft.validate().map_err(WriteError::Invalid)?;
        if if_missing == IfMissing::SkipExisting {
            let lists = self.pce().ip_lists(self.bypass()).await?.value;
            if let Some(existing) = lists.iter().find(|l| l.name == draft.name.trim()) {
                return Ok(Plan::Noop {
                    existing_href: existing.href.clone(),
                    reason: format!("ip_list {} exists", draft.name.trim()),
                });
            }
        }
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Post,
                path: self.pce().client().org_path("sec_policy/draft/ip_lists"),
                body: Some(draft.to_body()),
                summary: format!("create ip_list {}", draft.name.trim()),
            },
            kind: ObjectKind::IpList,
        })
    }

    pub async fn plan_update_ip_list(
        &self,
        href: &str,
        patch: &IpListPatch,
    ) -> Result<Plan, WriteError> {
        let href = checked_href(href, "ip_lists")?;
        if patch.is_empty() {
            return Err(WriteError::invalid("nothing to change"));
        }
        let current: IpList = serde_json::from_value(self.fetch(&href).await?)
            .map_err(|e| WriteError::Pce(crate::PceError::Decode(format!("{href}: {e}"))))?;
        let draft = patch.apply(&current).map_err(WriteError::Invalid)?;
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Put,
                path: href.clone(),
                body: Some(draft.to_body()),
                summary: format!("update ip_list {}", current.name),
            },
            kind: ObjectKind::IpList,
        })
    }

    pub fn plan_delete_ip_list(&self, href: &str) -> Result<Plan, WriteError> {
        let href = checked_href(href, "ip_lists")?;
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Delete,
                path: href.clone(),
                body: None,
                summary: format!("delete ip_list {href}"),
            },
            kind: ObjectKind::IpList,
        })
    }
}
