//! IP lists under `/sec_policy/draft/ip_lists`. An update `PUT`s only the
//! keys it changes; a changed `ip_ranges` or `fqdns` is sent as the full
//! list computed locally (the PCE replaces those wholesale), with every
//! existing entry carried through with its unknown fields.
use super::specs::IpRangeSpec;
use super::{IfMissing, ObjectKind, Plan, WriteError, WriteRequest, Writer, checked_href};
use crate::client::WriteMethod;
use crate::models::{Fqdn, IpList};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpListDraft {
    pub name: String,
    pub description: Option<String>,
    pub ranges: Vec<IpRangeSpec>,
    pub fqdns: Vec<Fqdn>,
}

impl IpListDraft {
    pub fn from_ip_list(l: &IpList) -> Self {
        Self {
            name: l.name.clone(),
            description: l.description.clone(),
            ranges: l.ip_ranges.iter().map(IpRangeSpec::from_range).collect(),
            fqdns: l.fqdns.clone(),
        }
    }

    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        if self.name.trim().is_empty() {
            errs.push("IP list name must not be empty".into());
        }
        if let Err(e) = self.validate_entries() {
            errs.push(e);
        }
        if self.fqdns.iter().any(|f| f.fqdn.trim().is_empty()) {
            errs.push("FQDN must not be empty".into());
        }
        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }

    fn validate_entries(&self) -> Result<(), String> {
        if self.ranges.is_empty() && self.fqdns.is_empty() {
            return Err("an IP list needs at least one range or FQDN".into());
        }
        Ok(())
    }

    fn ranges_json(&self) -> Value {
        Value::Array(self.ranges.iter().map(IpRangeSpec::to_json).collect())
    }

    fn fqdns_json(&self) -> Value {
        Value::Array(
            self.fqdns
                .iter()
                .map(|f| serde_json::to_value(f).unwrap_or(Value::Null))
                .collect(),
        )
    }

    pub(crate) fn to_body(&self) -> Value {
        let mut v = json!({
            "name": self.name.trim(),
            "ip_ranges": self.ranges_json(),
            "fqdns": self.fqdns_json(),
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

    fn touches_ranges(&self) -> bool {
        !self.add_ranges.is_empty() || !self.remove_ranges.is_empty()
    }

    fn touches_fqdns(&self) -> bool {
        !self.add_fqdns.is_empty() || !self.remove_fqdns.is_empty()
    }

    /// The full list after this patch, validated. A remove that matches
    /// nothing is an error so a typo never silently leaves a range behind.
    /// "At least one range or FQDN" is checked only when a list changes.
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
            d.fqdns.retain(|x| x.fqdn != *f);
            if d.fqdns.len() == before {
                errs.push(format!("fqdn {f} is not in {}", current.name));
            }
        }
        for f in &self.add_fqdns {
            let f = Fqdn::new(f);
            if f.fqdn.is_empty() {
                errs.push("FQDN must not be empty".into());
            } else if !d.fqdns.iter().any(|x| x.fqdn == f.fqdn) {
                d.fqdns.push(f);
            }
        }
        if let Some(n) = &self.name {
            if n.trim().is_empty() {
                errs.push("IP list name must not be empty".into());
            }
            d.name = n.clone();
        }
        if let Some(desc) = &self.description {
            d.description = Some(desc.clone());
        }
        if (self.touches_ranges() || self.touches_fqdns())
            && let Err(e) = d.validate_entries()
        {
            errs.push(e);
        }
        if errs.is_empty() { Ok(d) } else { Err(errs) }
    }

    /// The `PUT` body: only the keys this patch changes; a changed list is
    /// sent whole, as computed by [`IpListPatch::apply`].
    pub(crate) fn body(&self, d: &IpListDraft) -> Value {
        let mut v = serde_json::Map::new();
        if self.name.is_some() {
            v.insert("name".into(), json!(d.name.trim()));
        }
        if let Some(desc) = &self.description {
            v.insert("description".into(), json!(desc));
        }
        if self.touches_ranges() {
            v.insert("ip_ranges".into(), d.ranges_json());
        }
        if self.touches_fqdns() {
            v.insert("fqdns".into(), d.fqdns_json());
        }
        Value::Object(v)
    }
}

impl Writer<'_> {
    pub async fn plan_create_ip_list(
        &self,
        draft: &IpListDraft,
        if_missing: IfMissing,
    ) -> Result<Plan, WriteError> {
        draft.validate().map_err(WriteError::Invalid)?;
        if if_missing == IfMissing::Create {
            return self.ip_list_create(draft, None);
        }
        let lists = self.pce().ip_lists(self.bypass()).await?.value;
        self.ip_list_create(draft, Some(&lists))
    }

    /// The create plan; with `existing` (a fresh read), a live list of
    /// exactly that name is a `Noop` with its href. No request.
    pub(crate) fn ip_list_create(
        &self,
        draft: &IpListDraft,
        existing: Option<&[IpList]>,
    ) -> Result<Plan, WriteError> {
        draft.validate().map_err(WriteError::Invalid)?;
        if let Some(found) = existing
            .into_iter()
            .flatten()
            .find(|l| l.update_type.as_deref() != Some("delete") && l.name == draft.name.trim())
        {
            return Ok(Plan::Noop {
                existing_href: found.href.clone(),
                reason: format!("ip_list {} exists", draft.name.trim()),
            });
        }
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Post,
                path: self.pce().client().org_path("sec_policy/draft/ip_lists"),
                body: Some(draft.to_body()),
                summary: format!("create ip_list {}", draft.name.trim()),
                objects: Vec::new(),
            },
            kind: ObjectKind::IpList,
        })
    }

    pub async fn plan_update_ip_list(
        &self,
        href: &str,
        patch: &IpListPatch,
    ) -> Result<Plan, WriteError> {
        let href = checked_href(href, "ip_lists", self.pce().client().org_id())?;
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
                body: Some(patch.body(&draft)),
                summary: format!("update ip_list {}", current.name),
                objects: Vec::new(),
            },
            kind: ObjectKind::IpList,
        })
    }

    pub fn plan_delete_ip_list(&self, href: &str) -> Result<Plan, WriteError> {
        let href = checked_href(href, "ip_lists", self.pce().client().org_id())?;
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Delete,
                path: href.clone(),
                body: None,
                summary: format!("delete ip_list {href}"),
                objects: Vec::new(),
            },
            kind: ObjectKind::IpList,
        })
    }
}
