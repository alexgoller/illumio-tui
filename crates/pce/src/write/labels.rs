//! Labels: `POST /orgs/{org}/labels`, `PUT|DELETE /orgs/{org}/labels/{id}`.
//! Not provisionable, so a written label is live at once.
use super::{IfMissing, ObjectKind, Plan, WriteError, WriteRequest, Writer, checked_href};
use crate::client::WriteMethod;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelDraft {
    pub key: String,
    pub value: String,
}

impl LabelDraft {
    pub fn new(key: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        Self {
            key: key.as_ref().trim().to_string(),
            value: value.as_ref().trim().to_string(),
        }
    }

    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        if self.key.is_empty() {
            errs.push("label key must not be empty".into());
        }
        if self.value.is_empty() {
            errs.push("label value must not be empty".into());
        }
        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }

    pub fn display(&self) -> String {
        format!("{}={}", self.key, self.value)
    }

    pub(crate) fn to_body(&self) -> Value {
        json!({"key": self.key, "value": self.value})
    }
}

impl Writer<'_> {
    pub async fn plan_create_label(
        &self,
        draft: &LabelDraft,
        if_missing: IfMissing,
    ) -> Result<Plan, WriteError> {
        draft.validate().map_err(WriteError::Invalid)?;
        if if_missing == IfMissing::SkipExisting {
            let labels = self.pce().labels(self.bypass()).await?.value;
            if let Some(existing) = labels
                .iter()
                .find(|l| !l.is_deleted() && l.key == draft.key && l.value == draft.value)
            {
                return Ok(Plan::Noop {
                    existing_href: existing.href.clone(),
                    reason: format!("label {} exists", draft.display()),
                });
            }
        }
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Post,
                path: self.pce().client().org_path("labels"),
                body: Some(draft.to_body()),
                summary: format!("create label {}", draft.display()),
            },
            kind: ObjectKind::Label,
        })
    }

    pub fn plan_update_label(&self, href: &str, value: &str) -> Result<Plan, WriteError> {
        let href = checked_href(href, "labels", self.pce().client().org_id())?;
        let value = value.trim();
        if value.is_empty() {
            return Err(WriteError::invalid("label value must not be empty"));
        }
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Put,
                path: href.clone(),
                body: Some(json!({"value": value})),
                summary: format!("update label {href} value to {value}"),
            },
            kind: ObjectKind::Label,
        })
    }

    pub fn plan_delete_label(&self, href: &str) -> Result<Plan, WriteError> {
        let href = checked_href(href, "labels", self.pce().client().org_id())?;
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Delete,
                path: href.clone(),
                body: None,
                summary: format!("delete label {href}"),
            },
            kind: ObjectKind::Label,
        })
    }
}
