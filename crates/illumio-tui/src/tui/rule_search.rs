//! `f` on Policy: the one-line rule search editor
//! (`app=shop ipl:Corp svc:443/tcp wl:web01 href:… disabled deny`). Pure:
//! keys change the text; `update` parses it on `Enter` and completes it
//! on `Ctrl-N`/`Ctrl-P` from the name index and the loaded workloads
//! (never a request). The grammar is `pce::RuleFilter`'s.
use crate::tui::form::{Completing, Completion, FieldKind};
use pce::RuleFilter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleFilterEditor {
    pub value: String,
    /// The last `Enter`'s parse error; the search in force is kept.
    pub error: Option<String>,
    /// The completion being cycled; any key but `Ctrl-N`/`Ctrl-P` ends it.
    pub completion: Option<Box<Completion>>,
    /// The text's names the index does not know, worked out once per edit
    /// (`update` sets it), not per frame.
    pub unknown: Vec<String>,
}

const KIND: FieldKind = FieldKind::RuleFilter;

impl RuleFilterEditor {
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

    /// Ctrl-U: empties the line.
    pub fn clear(&mut self) {
        self.value.clear();
        self.error = None;
    }

    /// The text as a search, or why it is not one.
    pub fn parse(&self) -> Result<RuleFilter, String> {
        RuleFilter::parse(&self.value)
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

impl Completing for RuleFilterEditor {
    fn completion_request(&self) -> Option<(FieldKind, String, Vec<String>)> {
        let (token, earlier) = KIND.request(&self.value)?;
        Some((KIND, token, earlier))
    }

    fn start_completion(&mut self, candidates: Vec<String>) -> bool {
        match Completion::open(
            self.completion.as_deref(),
            0,
            KIND,
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
        self.completion.as_ref().is_some_and(|c| c.dives(KIND))
    }
}
