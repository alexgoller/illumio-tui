//! `f` on Events: severity, type, actor and window. Pure: keys change the
//! text; `update` applies it on `Enter` and completes the Type and Actor
//! fields on `Ctrl-N`/`Ctrl-P` from the event types and actors of the
//! loaded events (never a request).
use crate::tui::form::{Completing, Completion, FieldKind};

/// The fields, in order: severity, type, actor, window.
pub const SEVERITY: usize = 0;
pub const TYPE: usize = 1;
pub const ACTOR: usize = 2;
pub const WINDOW: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventsEditor {
    pub values: [String; 4],
    pub active: usize,
    /// The last `Enter`'s error; the filter in force is kept.
    pub error: Option<String>,
    /// The completion being cycled (the Type and Actor fields only); any key but
    /// `Ctrl-N`/`Ctrl-P` ends it.
    pub completion: Option<Box<Completion>>,
}

impl EventsEditor {
    pub fn new(values: [String; 4]) -> Self {
        Self {
            values,
            active: 0,
            error: None,
            completion: None,
        }
    }

    fn kind(&self) -> FieldKind {
        match self.active {
            TYPE => FieldKind::EventTypes,
            ACTOR => FieldKind::EventActors,
            _ => FieldKind::Text,
        }
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
            self.values[c.field] = c.typed();
        }
        true
    }
}

impl Completing for EventsEditor {
    fn completion_request(&self) -> Option<(FieldKind, String, Vec<String>)> {
        let kind = self.kind();
        let (token, earlier) = kind.request(&self.values[self.active])?;
        Some((kind, token, earlier))
    }

    fn start_completion(&mut self, candidates: Vec<String>) -> bool {
        let active = self.active;
        match Completion::open(
            self.completion.as_deref(),
            active,
            self.kind(),
            &mut self.values[active],
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
        self.values[c.field] = c.shown();
    }

    fn completing(&self) -> bool {
        self.completion.is_some()
    }

    fn completion_dives(&self) -> bool {
        false
    }
}
