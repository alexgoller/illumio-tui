//! Reverting: `PUT /orgs/N/sec_policy/delete` with an explicit
//! `change_subset`, built like a provision's from one fresh read of the
//! pending list. Draft only: every object named goes back to its active
//! version, or disappears when it only exists in draft. A rule is reverted
//! through its ruleset. The empty body (revert everything) is never sent:
//! "all" lists every pending change it read.
use super::provision::{
    PendingSubset, is_rule_child, nothing_pending, provision_counts, provisionable, singular,
};
use super::{ObjectKind, Plan, ProvisionSubset, WriteError, WriteRequest, Writer};
use crate::client::WriteMethod;
use serde_json::json;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RevertRequest {
    /// Every pending change, listed explicitly.
    pub all: bool,
    /// Hrefs of pending objects (a rule href means its ruleset); used when
    /// `all` is false, and then at least one is required.
    pub targets: Vec<String>,
}

/// What a revert does to one object.
pub const BACK_TO_ACTIVE: &str = "back to active";
/// An object that only exists in draft: the revert deletes it.
pub const REMOVED: &str = "removed";

/// `ip_list "Corp" (removed)` for each chosen object: its own pending entry
/// decides the fate (`create` → removed); a ruleset pending only through its
/// rules goes back to active.
fn fates(picked: &PendingSubset, org: u64) -> Vec<String> {
    picked
        .chosen
        .iter()
        .map(|(kind, href)| {
            let own = picked.pending.iter().find(|p| {
                !is_rule_child(&p.href)
                    && provisionable(&p.href, org).is_ok_and(|(k, h)| &k == kind && &h == href)
            });
            let fate = if own.is_some_and(|p| p.change_type == "create") {
                REMOVED
            } else {
                BACK_TO_ACTIVE
            };
            let name = own
                .and_then(|p| p.name.as_deref())
                .map(|n| format!("{n:?}"))
                .unwrap_or_else(|| href.clone());
            format!("{} {name} ({fate})", singular(kind))
        })
        .collect()
}

impl Writer<'_> {
    /// Plans a revert of pending draft changes. Requested hrefs are checked
    /// before any request; the pending list is read once, past the cache.
    /// The summary names every object and its fate.
    pub async fn plan_revert(&self, req: &RevertRequest) -> Result<Plan, WriteError> {
        let subset = match (req.all, req.targets.is_empty()) {
            (true, true) => ProvisionSubset::All,
            (true, false) => {
                return Err(WriteError::invalid(
                    "revert all takes no hrefs: name hrefs, or revert all",
                ));
            }
            (false, _) => ProvisionSubset::Hrefs(req.targets.clone()),
        };
        let picked = self.pending_subset(&subset, "revert").await?;
        if picked.chosen.is_empty() {
            return Ok(nothing_pending(self));
        }
        let org = self.pce().client().org_id();
        let body = json!({ "change_subset": picked.change_subset() });
        let n = provision_counts(&body).values().sum::<usize>();
        let what = match (req.all, n) {
            (true, 1) => "all 1 pending change".to_string(),
            (true, _) => format!("all {n} pending changes"),
            (false, 1) => "1 change".to_string(),
            (false, _) => format!("{n} changes"),
        };
        let summary = format!("revert {what}: {}", fates(&picked, org).join(", "));
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Put,
                path: self.pce().client().org_path("sec_policy/delete"),
                body: Some(body),
                summary,
            },
            kind: ObjectKind::Revert,
        })
    }
}
