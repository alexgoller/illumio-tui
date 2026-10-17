//! Bulk changes from the Workloads screen, the pure parts: the labels form
//! (`L`) and what its submit means, the enforcement picker (`M`), the
//! confirm's state (typed count, the loaded traffic's counts, the draft
//! check) and the check's result. The transitions are in
//! `update::bulk`, the drawing in `ui::widgets::bulk`. Nothing here sends
//! a request: planning is local (`pce::write::plan_bulk`).
use crate::tui::edit::EditIntent;
use crate::tui::form::{Field, FieldKind, Form};
use crossterm::event::{KeyCode, KeyEvent};
use pce::models::PolicyDecision;
use pce::traffic::ResolvedFlow;
use pce::write::bulk::would_block;
use pce::write::{
    BatchOrigin, BulkChange, BulkLabel, BulkPlan, BulkTarget, EnforcementMode, LabelDraft,
    LabelEdit, LabelRemoval, LabelSet, VisibilityLevel, plan_bulk,
};
use pce::{KeyOrder, NameIndex};
use std::collections::{BTreeSet, HashSet};
use tokio_util::sync::CancellationToken;

/// The bulk labels form's own state, kept in its intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkLabelsCtx {
    pub targets: Vec<BulkTarget>,
    /// `3 marked`, `group shop | prod`, `host-1`.
    pub what: String,
    /// The keys whose typed value `Ctrl-A` asked to create.
    pub creates: BTreeSet<String>,
}

/// `now shop`, `now (mixed: 3)`, `now none`: what the targets hold for `key`.
fn now_text(targets: &[BulkTarget], key: &str) -> String {
    let values: BTreeSet<Option<&str>> = targets
        .iter()
        .map(|t| {
            t.labels
                .iter()
                .find(|l| l.key == key)
                .map(|l| l.value.as_str())
        })
        .collect();
    match values.len() {
        1 => format!(
            "now {}",
            values.into_iter().next().flatten().unwrap_or("none")
        ),
        n => format!("now (mixed: {n})"),
    }
}

/// The hint under a key's field: what the targets hold, and `Ctrl-A`'s
/// offer while the typed value is no label.
pub fn label_hint(ctx: &BulkLabelsCtx, key: &str, value: &str, idx: &NameIndex) -> String {
    let now = now_text(&ctx.targets, key);
    let v = value.trim();
    if v.is_empty() || v == "-" || idx.label_href(key, v).is_some() {
        return now;
    }
    if ctx.creates.contains(key) {
        format!("{now} · new label {key}={v}: created first (Ctrl-A: don't)")
    } else {
        format!("{now} · {key}={v} is no label: Ctrl-A creates it")
    }
}

/// `L`: one field per label key (the label model's keys, then the other
/// keys the targets have, in the key order), each empty (keep).
pub fn labels_form(
    targets: Vec<BulkTarget>,
    what: String,
    order: &KeyOrder,
    idx: &NameIndex,
) -> Form {
    let mut keys: Vec<String> = order.first().to_vec();
    let mut extra: Vec<String> = targets
        .iter()
        .flat_map(|t| t.labels.iter().map(|l| l.key.clone()))
        .filter(|k| !k.is_empty() && !keys.contains(k))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    extra.sort_by(|a, b| order.cmp_keys(a, b));
    keys.extend(extra);
    let ctx = BulkLabelsCtx {
        targets,
        what,
        creates: BTreeSet::new(),
    };
    let fields = keys
        .iter()
        .map(|k| {
            Field::new(k.clone(), "", label_hint(&ctx, k, "", idx), false)
                .kind(FieldKind::LabelValue)
        })
        .collect();
    let n = ctx.targets.len();
    let title = format!(
        "Labels · {} ({n} workload{})",
        ctx.what,
        if n == 1 { "" } else { "s" }
    );
    Form::new(title, fields, EditIntent::BulkLabels(Box::new(ctx)))
}

/// The hints again from the fields' values (after a key or `Ctrl-A`).
pub fn refresh_hints(form: &mut Form, idx: &NameIndex) {
    let EditIntent::BulkLabels(ctx) = &form.intent else {
        return;
    };
    let ctx = ctx.clone();
    for f in &mut form.fields {
        f.hint = label_hint(&ctx, &f.label, &f.value, idx).into();
    }
}

/// `Ctrl-A` on the active field: create its typed value (or no longer).
/// `Err` says why there is nothing to create.
pub fn toggle_create(form: &mut Form, idx: &NameIndex) -> Result<String, String> {
    let Some(field) = form.fields.get(form.active) else {
        return Err("no field".into());
    };
    let key = field.label.to_string();
    let v = field.value.trim().to_string();
    let EditIntent::BulkLabels(ctx) = &mut form.intent else {
        return Err("not a labels form".into());
    };
    if ctx.creates.remove(&key) {
        refresh_hints(form, idx);
        return Ok(format!("{key}={v} will not be created"));
    }
    if v.is_empty() || v == "-" {
        return Err(format!("type a value for {key} first"));
    }
    if idx.label_href(&key, &v).is_some() {
        return Err(format!("{key}={v} exists: nothing to create"));
    }
    ctx.creates.insert(key.clone());
    refresh_hints(form, idx);
    Ok(format!("{key}={v} is created first, with the change"))
}

/// The submitted form as a change: per key, empty keeps, `-` removes, a
/// value sets (an existing label, or one `Ctrl-A` creates). `Ok(None)`:
/// nothing typed.
pub fn labels_change(form: &Form, idx: &NameIndex) -> Result<Option<BulkChange>, String> {
    let EditIntent::BulkLabels(ctx) = &form.intent else {
        return Err("not a labels form".into());
    };
    let mut edit = LabelEdit::default();
    for f in &form.fields {
        let key = f.label.to_string();
        let v = f.value.trim();
        if v.is_empty() {
            continue;
        }
        if v == "-" {
            edit.remove.push(LabelRemoval { key, value: None });
            continue;
        }
        match idx.label_href(&key, v) {
            Some(href) => edit.set.push(LabelSet::Existing(BulkLabel {
                href: href.to_string(),
                key,
                value: v.to_string(),
            })),
            None if ctx.creates.contains(&key) => {
                edit.set.push(LabelSet::Create(LabelDraft::new(&key, v)));
            }
            None => {
                return Err(format!(
                    "{key}={v} is no label: Ctrl-A creates it with the change, - removes {key}"
                ));
            }
        }
    }
    if edit.is_empty() {
        return Ok(None);
    }
    Ok(Some(BulkChange::relabel(edit)))
}

/// What a picker key did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKey {
    Continue,
    Cancel,
    Review,
}

/// `M`: the enforcement mode and visibility level for the targets, with
/// the change it makes (planned again on every move, locally).
#[derive(Debug, Clone, PartialEq)]
pub struct EnforcePicker {
    pub targets: Vec<BulkTarget>,
    pub what: String,
    /// Index into [`EnforcementMode::ALL`].
    pub mode: usize,
    /// 0: keep each workload's level; else [`VisibilityLevel::ALL`]`[i - 1]`.
    pub visibility: usize,
    /// The column `j`/`k` move in: 0 mode, 1 visibility.
    pub column: usize,
    /// `14 workloads: 9 visibility_only → full · …`.
    pub summary: String,
}

impl EnforcePicker {
    /// The cursor starts on the most common mode of the managed targets
    /// (`visibility_only` when none).
    pub fn new(targets: Vec<BulkTarget>, what: String) -> Self {
        let counts = EnforcementMode::ALL.map(|m| {
            targets
                .iter()
                .filter(|t| t.managed && t.enforcement.as_deref() == Some(m.as_str()))
                .count()
        });
        let mode = if counts.iter().all(|&c| c == 0) {
            1
        } else {
            (0..4)
                .max_by_key(|&i| (counts[i], std::cmp::Reverse(i)))
                .unwrap_or(1)
        };
        let mut p = Self {
            targets,
            what,
            mode,
            visibility: 0,
            column: 0,
            summary: String::new(),
        };
        p.refresh();
        p
    }

    pub fn change(&self) -> BulkChange {
        let mode = EnforcementMode::ALL[self.mode.min(3)];
        let vis = self
            .visibility
            .checked_sub(1)
            .and_then(|i| VisibilityLevel::ALL.get(i).copied());
        BulkChange::enforce(mode, vis)
    }

    fn refresh(&mut self) {
        self.summary = plan_bulk(&self.targets, &self.change(), Default::default(), None)
            .map_or_else(|e| e.to_string(), |p| p.summary());
    }

    pub fn key(&mut self, k: KeyEvent) -> PickerKey {
        let rows = |column: usize| if column == 0 { 4 } else { 5 };
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => return PickerKey::Cancel,
            KeyCode::Enter => return PickerKey::Review,
            KeyCode::Tab
            | KeyCode::BackTab
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Char('h')
            | KeyCode::Char('l') => self.column = 1 - self.column.min(1),
            KeyCode::Down | KeyCode::Char('j') => {
                let n = rows(self.column);
                let v = self.cursor_mut();
                *v = (*v + 1) % n;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let n = rows(self.column);
                let v = self.cursor_mut();
                *v = (*v + n - 1) % n;
            }
            _ => return PickerKey::Continue,
        }
        self.refresh();
        PickerKey::Continue
    }

    fn cursor_mut(&mut self) -> &mut usize {
        if self.column == 0 {
            &mut self.mode
        } else {
            &mut self.visibility
        }
    }

    /// The visibility choices as shown: `keep each workload's level`, then the levels.
    pub fn visibility_names() -> [&'static str; 5] {
        [
            "keep each workload's level",
            VisibilityLevel::FlowSummary.as_str(),
            VisibilityLevel::FlowDrops.as_str(),
            VisibilityLevel::FlowOff.as_str(),
            VisibilityLevel::EnhancedDataCollection.as_str(),
        ]
    }
}

/// What the draft check (`d` on the confirm) found.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckResult {
    /// Flows the query returned.
    pub flows: usize,
    /// The ones the draft policy does not allow, most connections first.
    pub blocked: Vec<ResolvedFlow>,
    /// Requests the check made (POST, polls, the draft PUT, download).
    pub requests: u64,
    pub truncated: bool,
    /// The window it covered (`24h`).
    pub window: String,
}

impl CheckResult {
    pub fn new(flows: &[ResolvedFlow], requests: u64, truncated: bool, window: String) -> Self {
        let mut blocked: Vec<ResolvedFlow> =
            flows.iter().filter(|f| would_block(f)).cloned().collect();
        blocked.sort_by_key(|f| std::cmp::Reverse(f.num_connections));
        Self {
            flows: flows.len(),
            blocked,
            requests,
            truncated,
            window,
        }
    }

    /// `12 of 340 flows (24h) would be blocked · 6 requests`.
    pub fn text(&self) -> String {
        format!(
            "{} of {}{} flows ({}) would be blocked · {} request{}",
            self.blocked.len(),
            if self.truncated { "≥" } else { "" },
            self.flows,
            self.window,
            self.requests,
            if self.requests == 1 { "" } else { "s" }
        )
    }

    /// The top `n` rows: `src → dst 443/tcp · 12 conns · potentially_blocked`.
    pub fn top(&self, n: usize) -> Vec<String> {
        self.blocked
            .iter()
            .take(n)
            .map(|f| {
                format!(
                    "{} → {} {} · {} conns · {}",
                    f.src.display_name(),
                    f.dst.display_name(),
                    f.service_text(),
                    f.num_connections,
                    f.draft_policy_decision.as_ref().map_or("?", |d| d.as_str())
                )
            })
            .collect()
    }
}

/// The draft check's state on the confirm.
#[derive(Debug, Clone)]
pub enum DraftCheck {
    /// Not asked (`d` runs it).
    Idle,
    Running {
        seq: u64,
        cancel: CancellationToken,
    },
    Done(Box<CheckResult>),
    Failed(String),
}

impl PartialEq for DraftCheck {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Idle, Self::Idle) => true,
            (Self::Running { seq: a, .. }, Self::Running { seq: b, .. }) => a == b,
            (Self::Done(a), Self::Done(b)) => a == b,
            (Self::Failed(a), Self::Failed(b)) => a == b,
            _ => false,
        }
    }
}

/// The confirm of a bulk change: every change (scrollable), the typed
/// count when it enforces several workloads, what the loaded traffic says
/// and the draft check.
#[derive(Debug, Clone)]
pub struct BulkConfirm {
    pub plan: BulkPlan,
    pub scroll: u16,
    /// The count typed so far.
    pub typed: String,
    /// `(blocked, potentially blocked)` loaded flows to the targets; `None`
    /// without loaded traffic (or for a label change).
    pub loaded: Option<(usize, usize)>,
    pub check: DraftCheck,
}

impl PartialEq for BulkConfirm {
    fn eq(&self, other: &Self) -> bool {
        self.plan.hrefs() == other.plan.hrefs()
            && self.plan.change == other.plan.change
            && self.scroll == other.scroll
            && self.typed == other.typed
            && self.loaded == other.loaded
            && self.check == other.check
    }
}

impl BulkConfirm {
    pub fn new(plan: BulkPlan, loaded: Option<(usize, usize)>) -> Self {
        Self {
            plan,
            scroll: 0,
            typed: String::new(),
            loaded,
            check: DraftCheck::Idle,
        }
    }

    /// The count is typed (or not needed): `y` sends.
    pub fn count_ok(&self) -> bool {
        !self.plan.needs_count() || self.typed.parse::<usize>().ok() == Some(self.plan.items.len())
    }

    /// The check answers an enforcement change only.
    pub fn checkable(&self) -> bool {
        self.plan.change.enforcement.is_some() && !self.plan.items.is_empty()
    }
}

/// `(blocked, potentially blocked)` flows of `flows` whose destination is
/// one of `hrefs`: the loaded traffic's view, no request.
pub fn loaded_counts(flows: &[ResolvedFlow], hrefs: &[String]) -> (usize, usize) {
    let set: HashSet<&str> = hrefs.iter().map(String::as_str).collect();
    let to = |f: &&ResolvedFlow| {
        f.dst
            .workload_href
            .as_deref()
            .is_some_and(|h| set.contains(h))
    };
    let n = |d: PolicyDecision| {
        flows
            .iter()
            .filter(to)
            .filter(|f| f.policy_decision == d)
            .count()
    };
    (
        n(PolicyDecision::Blocked),
        n(PolicyDecision::PotentiallyBlocked),
    )
}

/// The profile a plan is made on: it is sent only there.
pub fn origin(p: &crate::tui::model::ProfileInfo) -> BatchOrigin {
    BatchOrigin {
        profile: p.name.clone(),
        host: p.host.clone(),
        org_id: p.org_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::fixture_index;
    use crossterm::event::KeyModifiers;

    fn target(n: usize, mode: &str, managed: bool, labels: &[(&str, &str, &str)]) -> BulkTarget {
        BulkTarget {
            href: format!("/orgs/5/workloads/w{n}"),
            name: format!("host-{n}"),
            managed,
            enforcement: Some(mode.into()),
            visibility: Some("flow_summary".into()),
            labels: labels
                .iter()
                .map(|(h, k, v)| BulkLabel {
                    href: (*h).into(),
                    key: (*k).into(),
                    value: (*v).into(),
                })
                .collect(),
        }
    }

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    #[test]
    fn the_picker_starts_on_the_common_mode_and_plans_on_every_move() {
        let ts = vec![
            target(1, "visibility_only", true, &[]),
            target(2, "visibility_only", true, &[]),
            target(3, "full", true, &[]),
            target(4, "idle", false, &[]),
        ];
        let mut p = EnforcePicker::new(ts, "4 marked".into());
        assert_eq!(
            EnforcementMode::ALL[p.mode],
            EnforcementMode::VisibilityOnly
        );
        assert_eq!(
            p.summary,
            "4 workloads: 1 full → visibility_only · 2 already visibility_only (skipped) · 1 unmanaged (skipped)"
        );
        p.key(key(KeyCode::Char('j')));
        p.key(key(KeyCode::Char('j')));
        assert_eq!(p.change(), BulkChange::enforce(EnforcementMode::Full, None));
        assert_eq!(
            p.summary,
            "4 workloads: 2 visibility_only → full · 1 already full (skipped) · 1 unmanaged (skipped)"
        );
        p.key(key(KeyCode::Tab));
        p.key(key(KeyCode::Char('j')));
        assert_eq!(
            p.change(),
            BulkChange::enforce(EnforcementMode::Full, Some(VisibilityLevel::FlowSummary))
        );
        p.key(key(KeyCode::Char('k')));
        p.key(key(KeyCode::Char('k')));
        assert_eq!(
            p.change().visibility,
            Some(VisibilityLevel::EnhancedDataCollection),
            "wraps"
        );
        assert_eq!(p.key(key(KeyCode::Enter)), PickerKey::Review);
        assert_eq!(p.key(key(KeyCode::Esc)), PickerKey::Cancel);
    }

    #[test]
    fn the_labels_form_has_a_field_per_key_and_parses_set_remove_keep() {
        let idx = fixture_index();
        let shop = idx.label_href("app", "shop").unwrap().to_string();
        let ts = vec![
            target(
                1,
                "full",
                true,
                &[(&shop, "app", "shop"), ("/x/9", "zone", "a")],
            ),
            target(2, "full", true, &[]),
        ];
        let order = KeyOrder::new(&["app".into(), "env".into()], "role");
        let mut form = labels_form(ts, "2 marked".into(), &order, &idx);
        let keys: Vec<&str> = form.fields.iter().map(|f| f.label.as_ref()).collect();
        assert_eq!(keys, vec!["app", "env", "role", "zone"]);
        assert_eq!(form.fields[0].hint, "now (mixed: 2)");
        assert_eq!(form.fields[1].hint, "now none");
        assert_eq!(labels_change(&form, &idx), Ok(None), "empty keeps");
        form.fields[1].value = "-".into();
        form.fields[0].value = "shop".into();
        let change = labels_change(&form, &idx).unwrap().unwrap();
        assert_eq!(change.labels.remove[0].key, "env");
        assert_eq!(
            change.labels.set,
            vec![LabelSet::Existing(BulkLabel {
                href: shop.clone(),
                key: "app".into(),
                value: "shop".into()
            })]
        );
        // An unknown value: refused until Ctrl-A asks to create it.
        form.fields[0].value = "brandnew".into();
        form.active = 0;
        assert!(
            labels_change(&form, &idx)
                .unwrap_err()
                .contains("Ctrl-A creates")
        );
        refresh_hints(&mut form, &idx);
        assert!(
            form.fields[0]
                .hint
                .contains("is no label: Ctrl-A creates it")
        );
        toggle_create(&mut form, &idx).unwrap();
        assert!(form.fields[0].hint.contains("created first"));
        let change = labels_change(&form, &idx).unwrap().unwrap();
        assert_eq!(
            change.labels.set,
            vec![LabelSet::Create(LabelDraft::new("app", "brandnew"))]
        );
        toggle_create(&mut form, &idx).unwrap();
        assert!(
            labels_change(&form, &idx).is_err(),
            "Ctrl-A again: not created"
        );
        form.fields[0].value = "shop".into();
        assert!(
            toggle_create(&mut form, &idx).is_err(),
            "exists: nothing to create"
        );
    }

    #[test]
    fn loaded_counts_take_flows_to_the_targets() {
        use crate::tui::test_support::model_with_flows;
        let m = model_with_flows();
        let flows = m.traffic.loaded_flows();
        let hrefs: Vec<String> = flows
            .iter()
            .filter_map(|f| f.dst.workload_href.clone())
            .collect();
        let (b, p) = loaded_counts(flows, &hrefs);
        let expect = |d| {
            flows
                .iter()
                .filter(|f| f.dst.workload_href.is_some() && f.policy_decision == d)
                .count()
        };
        assert_eq!(
            (b, p),
            (
                expect(PolicyDecision::Blocked),
                expect(PolicyDecision::PotentiallyBlocked)
            )
        );
        assert_eq!(loaded_counts(flows, &[]), (0, 0));
    }
}
