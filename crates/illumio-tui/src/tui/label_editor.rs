//! `f` on Workloads: the one-line label filter editor
//! (`app=shop,crm env=prod !role=db app=-`). Pure: keys change the text;
//! `update` parses it on `Enter` and completes it on `Ctrl-N`/`Ctrl-P`
//! from the name index (never a request).
use crate::tui::form::{Completing, Completion, FieldKind};
use pce::LabelFilter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelFilterEditor {
    pub value: String,
    /// The last `Enter`'s parse error; the filter in force is kept.
    pub error: Option<String>,
    /// The completion being cycled; any key but `Ctrl-N`/`Ctrl-P` ends it.
    pub completion: Option<Box<Completion>>,
    /// The text's keys and values no label has, worked out once per edit
    /// (`update` sets it), not per frame.
    pub unknown: Vec<String>,
}

impl LabelFilterEditor {
    pub fn new(value: &str) -> Self {
        Self {
            value: value.to_string(),
            error: None,
            completion: None,
            unknown: Vec::new(),
        }
    }

    pub fn push(&mut self, c: char) {
        self.value.push(c);
        self.error = None;
    }

    pub fn pop(&mut self) {
        self.value.pop();
        self.error = None;
    }

    /// The text as a filter, or why it is not one.
    pub fn parse(&self) -> Result<LabelFilter, String> {
        LabelFilter::parse(&self.value)
    }

    /// Up to five candidates around the shown one, then `+N more`.
    pub fn completion_strip(&self) -> Option<Vec<(String, bool)>> {
        self.completion.as_ref().map(|c| c.strip())
    }

    /// Ends an open completion (`true`); `restore` puts the typed token back.
    pub fn end_completion(&mut self, restore: bool) -> bool {
        let Some(c) = self.completion.take() else {
            return false;
        };
        if restore {
            self.value = c.typed();
        }
        true
    }
}

impl Completing for LabelFilterEditor {
    fn completion_request(&self) -> Option<(FieldKind, String, Vec<String>)> {
        let kind = FieldKind::WorkloadLabels;
        let (token, earlier) = kind.request(&self.value)?;
        Some((kind, token, earlier))
    }

    fn start_completion(&mut self, candidates: Vec<String>) -> bool {
        let kind = FieldKind::WorkloadLabels;
        match Completion::open(
            self.completion.as_deref(),
            0,
            kind,
            &mut self.value,
            candidates,
        ) {
            Some(c) => {
                self.completion = Some(Box::new(c));
                self.error = None;
                true
            }
            None => false,
        }
    }

    fn cycle(&mut self, forward: bool) {
        let Some(c) = self.completion.as_mut() else {
            return;
        };
        c.step(forward);
        self.value = c.shown();
    }

    fn completing(&self) -> bool {
        self.completion.is_some()
    }

    fn completion_dives(&self) -> bool {
        self.completion
            .as_ref()
            .is_some_and(|c| c.dives(FieldKind::WorkloadLabels))
    }
}
