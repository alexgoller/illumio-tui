//! Marks applied to traffic output (`traffic query|groups|mesh`, `app`;
//! `gen` only counts them): the file read once (a local file, no
//! request), its active marks for the profile's PCE, and what the
//! envelope says: `meta.squelched`, `meta.marks`, a warning for an invalid
//! file (the command runs without marks).
use super::context::Context;
use super::output::{MarksMeta, Meta, SquelchedMeta};
use crate::marks::MarksStore;
use jiff::Timestamp;
use pce::marks::{Action, ActiveMarks, Hit, MarkSet, Marked, Marker, Partition, PceId, RowMarks};
use pce::traffic::ResolvedFlow;
use serde::Serialize;

/// The marks file as a traffic command sees it.
pub struct CliMarks {
    file: String,
    set: Option<MarkSet>,
    warning: Option<String>,
    /// `--show-squelched`: squelched flows stay in, marked `squelch`.
    pub show: bool,
    pce: PceId,
    now: Timestamp,
    /// `draft_decision` marks the last `active` left out (no `--draft`).
    waiting: std::cell::Cell<usize>,
}

impl CliMarks {
    /// Reads `marks.toml` unless `no_marks`. An invalid or unreadable file
    /// is a warning on stderr (and in `meta.warnings`): marks off.
    pub fn load(ctx: &Context, no_marks: bool, show: bool) -> Self {
        let mut cm = CliMarks {
            file: ctx.marks_path.display().to_string(),
            set: None,
            warning: None,
            show,
            pce: PceId {
                host: ctx.config.host.clone(),
                org: ctx.config.org_id,
            },
            now: Timestamp::now(),
            waiting: std::cell::Cell::new(0),
        };
        if no_marks {
            return cm;
        }
        let store = MarksStore::open(ctx.marks_path.clone());
        match (store.set(), store.error()) {
            (Some(set), _) => cm.set = Some(set.clone()),
            (None, Some(crate::marks::MarksError::Parse(p))) => {
                cm.warning = Some(format!(
                    "marks.toml invalid: {} problem{}, marks off (illumio-tui marks list shows them)",
                    p.len(),
                    if p.len() == 1 { "" } else { "s" }
                ));
            }
            (None, e) => {
                cm.warning = Some(format!(
                    "marks.toml unreadable, marks off: {}",
                    e.map(|e| e.to_string()).unwrap_or_default()
                ));
            }
        }
        if let Some(w) = &cm.warning {
            eprintln!("warning: {w}");
        }
        cm
    }

    /// The marks active for this result (`draft`: draft decisions were
    /// loaded, `--draft`); `None` without a valid file or an active mark.
    pub fn active(&self, draft: bool) -> Option<ActiveMarks<'_>> {
        let a = self
            .set
            .as_ref()
            .map(|s| s.active(Some(&self.pce), self.now, draft))?;
        self.waiting.set(a.waiting_for_draft());
        (!a.is_empty()).then_some(a)
    }

    /// `meta.marks`, `meta.squelched` (with an active mark) and the
    /// warning of an invalid file.
    pub fn fill(&self, meta: &mut Meta, active: Option<&ActiveMarks>, sq: Option<SquelchedMeta>) {
        if let Some(w) = &self.warning {
            meta.warnings.get_or_insert_with(Vec::new).push(w.clone());
        }
        let waiting = self.waiting.get();
        if let Some(a) = active {
            meta.marks = Some(MarksMeta {
                active: a.len(),
                file: self.file.clone(),
                waiting_draft: (waiting > 0).then_some(waiting),
            });
            meta.squelched = sq;
        } else if waiting > 0 {
            meta.marks = Some(MarksMeta {
                active: 0,
                file: self.file.clone(),
                waiting_draft: Some(waiting),
            });
        }
    }
}

/// `meta.squelched` of one view: `annotate` joins the marks to its rows
/// and returns the rows squelch removed; `pred` keeps the squelched
/// records of the view's own input (its decision filter).
pub fn squelched(
    part: Option<&Partition>,
    active: Option<&ActiveMarks>,
    annotate: impl FnOnce(&Marked, &ActiveMarks) -> u64,
    pred: impl Fn(&ResolvedFlow) -> bool,
) -> Option<SquelchedMeta> {
    let (p, a) = (part?, active?);
    Some(SquelchedMeta {
        rows: Some(annotate(&p.marked, a)),
        flows: p.marked.squelched_where(pred),
    })
}

/// A flow's `mark`: its winning action and mark.
#[derive(Debug, Clone, Serialize)]
pub struct FlowMark {
    pub action: Action,
    pub id: String,
}

impl FlowMark {
    pub fn of(h: Option<&Hit>, a: &ActiveMarks) -> Option<FlowMark> {
        h.map(|h| FlowMark {
            action: h.action,
            id: a.mark(h.mark).id.clone(),
        })
    }
}

/// A flow record with its mark (`traffic query`) and its change (compare
/// mode): `ResolvedFlow` itself is unchanged.
#[derive(Debug, Serialize)]
pub struct MarkedFlow<'a> {
    #[serde(flatten)]
    pub flow: &'a ResolvedFlow,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mark: Option<&'a FlowMark>,
    /// Compare mode: `change` and `baseline_decision`.
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub change: Option<&'a pce::traffic::RowChange>,
}

/// The `mark` cell of tables and CSV: the row marker's action, `~12` for
/// a partly squelched row, empty without one.
pub fn mark_cell(m: Option<&RowMarks>) -> String {
    match m.and_then(RowMarks::marker) {
        Some(Marker::PartlySquelched(n)) => format!("~{n}"),
        Some(k) => k
            .action()
            .map(Action::as_str)
            .unwrap_or_default()
            .to_string(),
        None => String::new(),
    }
}
