//! The `g` overlay: which label keys every traffic view groups by. Pure
//! state; `update` applies the outcome and `ui` draws it.
use crossterm::event::{KeyCode, KeyEvent};

/// One key the picker offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerEntry {
    pub key: String,
    /// Flows carrying the key, shown for keys outside the profile's label
    /// model (`None` for the profile keys and `loc`).
    pub seen: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPicker {
    pub entries: Vec<PickerEntry>,
    /// The row under the cursor: an entry, then `all labels`
    /// (`all_row()`), then the `keys:` text line (`text_row()`).
    pub cursor: usize,
    /// The chosen keys as text, `app,env`. The toggles rewrite it; typing
    /// edits it.
    pub text: String,
    /// `all labels` is checked: every key seen in the loaded flows.
    pub all: bool,
}

/// What a key press leaves behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerOutcome {
    Open(KeyPicker),
    Close,
    /// Enter: these keys (possibly empty; the caller falls back to the
    /// profile's scope keys), or every seen key when `all`.
    Apply {
        keys: Vec<String>,
        all: bool,
    },
}

/// `app, env ,,role` -> `["app", "env", "role"]`.
pub fn parse_keys(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

impl KeyPicker {
    /// Entries: the profile's scope keys and tier key, then `loc`, then every
    /// other key in `seen` (with its count), then the current keys not listed
    /// yet (typed earlier, perhaps unknown). The text starts as `current`.
    pub fn new(
        scope_keys: &[String],
        tier_key: &str,
        seen: &[(String, usize)],
        current: &[String],
        all: bool,
    ) -> Self {
        let mut entries: Vec<PickerEntry> = Vec::new();
        let mut add = |key: &str, count: Option<usize>| {
            if !key.is_empty() && !entries.iter().any(|e| e.key == key) {
                entries.push(PickerEntry {
                    key: key.to_string(),
                    seen: count,
                });
            }
        };
        for k in scope_keys {
            add(k, None);
        }
        add(tier_key, None);
        add("loc", None);
        for (k, n) in seen {
            add(k, Some(*n));
        }
        for k in current {
            add(k, None);
        }
        Self {
            entries,
            cursor: 0,
            text: if all {
                String::new()
            } else {
                current.join(",")
            },
            all,
        }
    }

    pub fn all_row(&self) -> usize {
        self.entries.len()
    }

    pub fn text_row(&self) -> usize {
        self.entries.len() + 1
    }

    /// The keys the text names, in its order.
    pub fn chosen(&self) -> Vec<String> {
        parse_keys(&self.text)
    }

    pub fn is_checked(&self, key: &str) -> bool {
        !self.all && self.chosen().iter().any(|k| k == key)
    }

    /// `j`/`k` and arrows move (`j`/`k` are text on the text line), Space
    /// toggles the row, any other printable character or Backspace edits
    /// the text (moving the cursor to it), Enter applies, Esc closes.
    pub fn key(mut self, k: KeyEvent) -> PickerOutcome {
        let on_text = self.cursor == self.text_row();
        match k.code {
            KeyCode::Esc => return PickerOutcome::Close,
            KeyCode::Enter => {
                return PickerOutcome::Apply {
                    keys: if self.all { Vec::new() } else { self.chosen() },
                    all: self.all,
                };
            }
            KeyCode::Down => self.cursor = (self.cursor + 1).min(self.text_row()),
            KeyCode::Up => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Char('j') if !on_text => self.cursor = (self.cursor + 1).min(self.text_row()),
            KeyCode::Char('k') if !on_text => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Char(' ') if !on_text => self.toggle(),
            KeyCode::Char(' ') => {}
            KeyCode::Backspace => {
                self.cursor = self.text_row();
                self.all = false;
                self.text.pop();
            }
            KeyCode::Char(c) => {
                self.cursor = self.text_row();
                self.all = false;
                self.text.push(c);
            }
            _ => {}
        }
        PickerOutcome::Open(self)
    }

    fn toggle(&mut self) {
        if self.cursor == self.all_row() {
            self.all = !self.all;
            if self.all {
                self.text.clear();
            }
            return;
        }
        let Some(key) = self.entries.get(self.cursor).map(|e| e.key.clone()) else {
            return;
        };
        let mut chosen = if self.all { Vec::new() } else { self.chosen() };
        self.all = false;
        match chosen.iter().position(|k| *k == key) {
            Some(i) => {
                chosen.remove(i);
            }
            None => chosen.push(key),
        }
        self.text = chosen.join(",");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn kc(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ch(c: char) -> KeyEvent {
        kc(KeyCode::Char(c))
    }

    fn keys(k: &[&str]) -> Vec<String> {
        k.iter().map(|s| s.to_string()).collect()
    }

    fn picker() -> KeyPicker {
        KeyPicker::new(
            &keys(&["app", "env"]),
            "role",
            &[
                ("app".into(), 30),
                ("env".into(), 30),
                ("role".into(), 30),
                ("loc".into(), 30),
                ("datacenter".into(), 11),
            ],
            &keys(&["app", "env"]),
            false,
        )
    }

    fn press(p: KeyPicker, k: KeyEvent) -> KeyPicker {
        match p.key(k) {
            PickerOutcome::Open(p) => p,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn entries_list_the_profile_keys_then_loc_then_the_seen_keys_with_counts() {
        let p = picker();
        let listed: Vec<(&str, Option<usize>)> =
            p.entries.iter().map(|e| (e.key.as_str(), e.seen)).collect();
        assert_eq!(
            listed,
            vec![
                ("app", None),
                ("env", None),
                ("role", None),
                ("loc", None),
                ("datacenter", Some(11)),
            ]
        );
        assert!(p.is_checked("app") && p.is_checked("env") && !p.is_checked("role"));
        assert_eq!((p.all_row(), p.text_row()), (5, 6));
        // A profile key no flow carries, and a key typed earlier, are listed.
        let p = KeyPicker::new(
            &keys(&["site"]),
            "tier",
            &[],
            &keys(&["site", "zone"]),
            false,
        );
        let listed: Vec<&str> = p.entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(listed, vec!["site", "tier", "loc", "zone"]);
    }

    #[test]
    fn space_toggles_keys_in_the_order_they_were_chosen() {
        let mut p = picker();
        p = press(p, ch('j')); // env
        p = press(p, ch(' '));
        assert_eq!(p.text, "app");
        p = press(p, kc(KeyCode::Down)); // role
        p = press(p, ch(' '));
        p = press(p, kc(KeyCode::Up)); // env
        p = press(p, ch(' '));
        assert_eq!(p.text, "app,role,env");
        assert_eq!(
            p.key(kc(KeyCode::Enter)),
            PickerOutcome::Apply {
                keys: keys(&["app", "role", "env"]),
                all: false
            }
        );
    }

    #[test]
    fn all_labels_unchecks_the_keys_and_a_key_clears_all_labels() {
        let mut p = picker();
        for _ in 0..5 {
            p = press(p, ch('j'));
        }
        assert_eq!(p.cursor, p.all_row());
        p = press(p, ch(' '));
        assert!(p.all && p.text.is_empty() && !p.is_checked("app"));
        assert_eq!(
            p.clone().key(kc(KeyCode::Enter)),
            PickerOutcome::Apply {
                keys: Vec::new(),
                all: true
            }
        );
        p = press(p, ch('k')); // datacenter
        p = press(p, ch(' '));
        assert!(!p.all);
        assert_eq!(p.text, "datacenter");
    }

    #[test]
    fn typing_edits_the_text_line_like_the_old_prompt() {
        let mut p = picker();
        for _ in 0..7 {
            p = press(p, kc(KeyCode::Backspace));
        }
        assert_eq!(p.cursor, p.text_row());
        assert_eq!(p.text, "");
        for c in "role, job ,".chars() {
            p = press(p, ch(c));
        }
        assert_eq!(
            p.text, "role,job,",
            "on the text line j is text and spaces are dropped"
        );
        assert_eq!(
            p.key(kc(KeyCode::Enter)),
            PickerOutcome::Apply {
                keys: keys(&["role", "job"]),
                all: false
            }
        );
        let mut p = picker();
        for c in "zz".chars() {
            p = press(p, ch(c));
        }
        assert_eq!(
            p.text, "app,envzz",
            "a printable key jumps to the text line"
        );
        assert_eq!(picker().key(kc(KeyCode::Esc)), PickerOutcome::Close);
        assert_eq!(parse_keys(" app, ,env ,"), keys(&["app", "env"]));
    }
}
