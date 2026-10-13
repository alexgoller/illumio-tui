//! The app picker: `F` on a row without an app, or `:focus` alone. Pure:
//! a text line narrows the apps of the loaded result; `Enter` picks.
use crossterm::event::{KeyCode, KeyEvent};

/// What the picked app is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppPickerPurpose {
    /// `F` / `:focus`: narrow every traffic view to it.
    Focus,
    /// `A` / `:app`: open the app overlay for it.
    Overlay,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPicker {
    pub input: String,
    /// Every app of the loaded result (`TrafficState::loaded_apps`).
    pub apps: Vec<String>,
    /// Index into `matches()`.
    pub selected: usize,
    pub purpose: AppPickerPurpose,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppPickerOutcome {
    Open(AppPicker),
    Close,
    /// The selected match, or the typed text when nothing matches (the
    /// caller reports an unknown app).
    Pick {
        app: String,
        purpose: AppPickerPurpose,
    },
}

impl AppPicker {
    pub fn new(apps: Vec<String>, purpose: AppPickerPurpose) -> Self {
        Self {
            input: String::new(),
            apps,
            selected: 0,
            purpose,
        }
    }

    pub fn with_input(mut self, input: &str) -> Self {
        self.input = input.to_string();
        self.selected = 0;
        self
    }

    /// The apps containing the typed text, case-insensitively, in order.
    pub fn matches(&self) -> Vec<&str> {
        let needle = self.input.trim().to_lowercase();
        self.apps
            .iter()
            .filter(|a| a.to_lowercase().contains(&needle))
            .map(String::as_str)
            .collect()
    }

    fn selected_match(&self) -> Option<String> {
        self.matches().get(self.selected).map(|s| s.to_string())
    }

    pub fn key(mut self, k: KeyEvent) -> AppPickerOutcome {
        match k.code {
            KeyCode::Esc => AppPickerOutcome::Close,
            KeyCode::Enter => {
                let app = self
                    .selected_match()
                    .unwrap_or_else(|| self.input.trim().to_string());
                if app.is_empty() {
                    return AppPickerOutcome::Open(self);
                }
                AppPickerOutcome::Pick {
                    app,
                    purpose: self.purpose,
                }
            }
            KeyCode::Down | KeyCode::Up => {
                let n = self.matches().len();
                if n > 0 {
                    self.selected = if k.code == KeyCode::Down {
                        (self.selected + 1) % n
                    } else {
                        (self.selected + n - 1) % n
                    };
                }
                AppPickerOutcome::Open(self)
            }
            KeyCode::Tab => {
                if let Some(m) = self.selected_match() {
                    self.input = m;
                    self.selected = 0;
                }
                AppPickerOutcome::Open(self)
            }
            KeyCode::Backspace => {
                self.input.pop();
                self.selected = 0;
                AppPickerOutcome::Open(self)
            }
            KeyCode::Char(c) => {
                self.input.push(c);
                self.selected = 0;
                AppPickerOutcome::Open(self)
            }
            _ => AppPickerOutcome::Open(self),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn picker() -> AppPicker {
        AppPicker::new(
            vec!["crm/dev".into(), "crm/prod".into(), "shop/prod".into()],
            AppPickerPurpose::Focus,
        )
    }

    fn open(o: AppPickerOutcome) -> AppPicker {
        match o {
            AppPickerOutcome::Open(p) => p,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn typing_narrows_and_enter_picks_the_selected_match() {
        let p = open(picker().key(k(KeyCode::Char('C'))));
        assert_eq!(p.matches(), vec!["crm/dev", "crm/prod"], "case-insensitive");
        let p = open(p.key(k(KeyCode::Down)));
        assert_eq!(
            p.key(k(KeyCode::Enter)),
            AppPickerOutcome::Pick {
                app: "crm/prod".into(),
                purpose: AppPickerPurpose::Focus
            }
        );
    }

    #[test]
    fn tab_completes_up_wraps_backspace_widens_and_esc_closes() {
        let p = open(picker().with_input("prod").key(k(KeyCode::Up)));
        assert_eq!(p.selected, 1, "Up from the first wraps to the last");
        let p = open(p.key(k(KeyCode::Tab)));
        assert_eq!(p.input, "shop/prod");
        let p = open(p.key(k(KeyCode::Backspace)));
        assert_eq!(p.input, "shop/pro");
        assert_eq!(picker().key(k(KeyCode::Esc)), AppPickerOutcome::Close);
    }

    #[test]
    fn enter_without_a_match_picks_the_text_and_empty_text_stays_open() {
        let p = picker().with_input("nope");
        assert!(p.matches().is_empty());
        assert_eq!(
            p.key(k(KeyCode::Enter)),
            AppPickerOutcome::Pick {
                app: "nope".into(),
                purpose: AppPickerPurpose::Focus
            }
        );
        let p = AppPicker::new(Vec::new(), AppPickerPurpose::Focus);
        assert!(matches!(
            p.key(k(KeyCode::Enter)),
            AppPickerOutcome::Open(_)
        ));
    }
}
