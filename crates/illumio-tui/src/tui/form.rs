//! One generic form for every editable kind, held in `Overlay::Form`. Pure:
//! keys change the form; `update` decides what a submit means. Fields are
//! single-line and the cursor is always at the end of the active field.
use crate::tui::edit::EditIntent;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::borrow::Cow;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// "Name", "Ranges".
    pub label: &'static str,
    pub value: String,
    /// Example input shown under the field (or what to type, such as the
    /// scope a `w` form's ruleset needs).
    pub hint: Cow<'static, str>,
    pub required: bool,
}

impl Field {
    pub fn new(
        label: &'static str,
        value: impl Into<String>,
        hint: impl Into<Cow<'static, str>>,
        required: bool,
    ) -> Self {
        Self {
            label,
            value: value.into(),
            hint: hint.into(),
            required,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    /// "New IP list", "Edit service Web".
    pub title: String,
    pub fields: Vec<Field>,
    pub active: usize,
    /// The last validation failure, shown in red.
    pub error: Option<String>,
    /// What to build on Enter.
    pub intent: EditIntent,
    /// Alternate values of some fields, by index: `Ctrl-A` swaps them in
    /// (a `w` form: every attached label instead of the label model).
    pub alt: Option<Vec<(usize, String)>>,
    /// The alternates are shown (`Ctrl-A` was pressed an odd number of times).
    pub expanded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormEvent {
    Continue,
    /// Enter with every required field filled.
    Submit,
    /// Esc: close without a request.
    Cancel,
}

impl Form {
    pub fn new(title: impl Into<String>, fields: Vec<Field>, intent: EditIntent) -> Self {
        Self {
            title: title.into(),
            fields,
            active: 0,
            error: None,
            intent,
            alt: None,
            expanded: false,
        }
    }

    /// `Ctrl-A`: the listed fields and their alternates trade places, so the
    /// text the user left becomes the alternate. Nothing without alternates.
    fn swap_alternates(&mut self) {
        let Some(alt) = self.alt.as_mut() else {
            return;
        };
        for (i, other) in alt.iter_mut() {
            if let Some(f) = self.fields.get_mut(*i) {
                std::mem::swap(&mut f.value, other);
            }
        }
        self.expanded = !self.expanded;
    }

    /// The value of the field with this label ("" when there is none).
    pub fn value(&self, label: &str) -> &str {
        self.fields
            .iter()
            .find(|f| f.label == label)
            .map_or("", |f| f.value.as_str())
    }

    pub fn on_key(&mut self, k: KeyEvent) -> FormEvent {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let n = self.fields.len().max(1);
        match k.code {
            KeyCode::Esc => return FormEvent::Cancel,
            KeyCode::Enter => {
                if let Some(i) = self
                    .fields
                    .iter()
                    .position(|f| f.required && f.value.trim().is_empty())
                {
                    self.active = i;
                    self.error = Some(format!("{} is required", self.fields[i].label));
                    return FormEvent::Continue;
                }
                self.error = None;
                return FormEvent::Submit;
            }
            KeyCode::Tab | KeyCode::Down => self.active = (self.active + 1) % n,
            KeyCode::BackTab | KeyCode::Up => self.active = (self.active + n - 1) % n,
            KeyCode::Backspace => {
                if let Some(f) = self.fields.get_mut(self.active) {
                    f.value.pop();
                }
            }
            KeyCode::Char('a') if ctrl => self.swap_alternates(),
            KeyCode::Char('u') if ctrl => {
                if let Some(f) = self.fields.get_mut(self.active) {
                    f.value.clear();
                }
            }
            KeyCode::Char(c) if !ctrl => {
                if let Some(f) = self.fields.get_mut(self.active) {
                    f.value.push(c);
                }
            }
            _ => {}
        }
        FormEvent::Continue
    }
}

/// The end of `value` that fits in `width` cells, with `…` in front when the
/// start is cut, so the cursor (always at the end) stays visible.
pub fn visible_tail(value: &str, width: usize) -> String {
    let n = value.chars().count();
    if n <= width {
        return value.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let tail: String = value.chars().skip(n - (width - 1)).collect();
    format!("…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::edit::EditIntent;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn label_form() -> Form {
        Form::new(
            "New label",
            vec![
                Field::new("Key", "", "app", true),
                Field::new("Value", "", "web", true),
                Field::new("Note", "", "optional", false),
            ],
            EditIntent::CreateLabel,
        )
    }

    #[test]
    fn tab_and_arrows_move_between_fields_and_wrap() {
        let mut f = label_form();
        assert_eq!(f.on_key(k(KeyCode::Tab)), FormEvent::Continue);
        assert_eq!(f.active, 1);
        f.on_key(k(KeyCode::Down));
        assert_eq!(f.active, 2);
        f.on_key(k(KeyCode::Tab));
        assert_eq!(f.active, 0, "wraps forward");
        f.on_key(k(KeyCode::BackTab));
        assert_eq!(f.active, 2, "wraps backward");
        f.on_key(k(KeyCode::Up));
        assert_eq!(f.active, 1);
    }

    #[test]
    fn typing_backspace_and_ctrl_u_edit_only_the_active_field() {
        let mut f = label_form();
        for c in "apq".chars() {
            f.on_key(k(KeyCode::Char(c)));
        }
        f.on_key(k(KeyCode::Backspace));
        f.on_key(k(KeyCode::Char('p')));
        assert_eq!(f.value("Key"), "app");
        assert_eq!(f.value("Value"), "");
        f.on_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
        assert_eq!(f.value("Key"), "app", "other control keys are not typed");
        f.on_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(f.value("Key"), "", "Ctrl-U clears the field");
        f.on_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT));
        assert_eq!(f.value("Key"), "A", "shifted characters are typed");
        assert_eq!(f.value("Missing"), "");
    }

    #[test]
    fn enter_checks_required_fields_and_esc_cancels() {
        let mut f = label_form();
        f.on_key(k(KeyCode::Char('a')));
        f.on_key(k(KeyCode::Tab));
        f.on_key(k(KeyCode::Tab));
        assert_eq!(f.on_key(k(KeyCode::Enter)), FormEvent::Continue);
        assert_eq!(f.error.as_deref(), Some("Value is required"));
        assert_eq!(f.active, 1, "the empty required field takes the focus");
        f.on_key(k(KeyCode::Char(' ')));
        assert_eq!(
            f.on_key(k(KeyCode::Enter)),
            FormEvent::Continue,
            "blank is empty"
        );
        f.on_key(k(KeyCode::Char('w')));
        assert_eq!(f.on_key(k(KeyCode::Enter)), FormEvent::Submit);
        assert_eq!(f.error, None);
        assert_eq!(f.on_key(k(KeyCode::Esc)), FormEvent::Cancel);
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn flow_like() -> Form {
        let mut f = Form::new(
            "Allow rule from flow",
            vec![
                Field::new("Ruleset", "", "name", true),
                Field::new("Providers", "role=web", "actors", true),
                Field::new("Consumers", "iplist:Corp", "actors", true),
            ],
            EditIntent::CreateLabel,
        );
        f.alt = Some(vec![
            (1, "app=shop, env=prod, role=web".into()),
            (2, "iplist:Corp, loc=eu".into()),
        ]);
        f
    }

    #[test]
    fn ctrl_a_swaps_the_alternates_and_twice_restores_the_model_view() {
        let mut f = flow_like();
        let model_view = f.clone();
        assert_eq!(f.on_key(ctrl('a')), FormEvent::Continue);
        assert!(f.expanded);
        assert_eq!(f.value("Providers"), "app=shop, env=prod, role=web");
        assert_eq!(f.value("Consumers"), "iplist:Corp, loc=eu");
        assert_eq!(f.value("Ruleset"), "", "unlisted fields stay");
        f.on_key(ctrl('a'));
        assert_eq!(f, model_view, "twice is the model view again");
        // Without alternates the key does nothing.
        let mut plain = label_form();
        let before = plain.clone();
        plain.on_key(ctrl('a'));
        assert_eq!(plain, before);
    }

    #[test]
    fn ctrl_a_keeps_what_the_user_typed_as_the_alternate() {
        let mut f = flow_like();
        f.active = 1;
        for _ in 0..3 {
            f.on_key(k(KeyCode::Backspace));
        }
        f.on_key(k(KeyCode::Char('d')));
        f.on_key(k(KeyCode::Char('b')));
        assert_eq!(f.value("Providers"), "role=db");
        f.on_key(ctrl('a'));
        assert_eq!(f.value("Providers"), "app=shop, env=prod, role=web");
        f.on_key(ctrl('a'));
        assert_eq!(f.value("Providers"), "role=db", "the edit comes back");
        assert!(!f.expanded);
    }

    #[test]
    fn visible_tail_keeps_the_end_where_the_cursor_is() {
        assert_eq!(visible_tail("10.0.0.0/8", 20), "10.0.0.0/8");
        assert_eq!(visible_tail("abcdefghij", 5), "…ghij");
        assert_eq!(visible_tail("ab", 0), "");
        assert_eq!(
            visible_tail("äöüß", 3),
            "…üß",
            "counts characters, not bytes"
        );
    }
}
