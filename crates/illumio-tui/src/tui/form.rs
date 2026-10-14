//! One generic form for every editable kind, held in `Overlay::Form`. Pure:
//! keys change the form; `update` decides what a submit means. Fields are
//! single-line and the cursor is always at the end of the active field.
//! `Ctrl-N`/`Ctrl-P` complete the active field's current token; the form
//! cannot see the name index, so `update` asks [`Completing::completion_request`],
//! computes the candidates ([`crate::tui::complete`]) and hands them to
//! [`Completing::start_completion`].
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
    /// What `Ctrl-N` completes in this field.
    pub kind: FieldKind,
}

/// What a field holds, for completion: which candidates `Ctrl-N` offers and
/// how the field splits into tokens.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FieldKind {
    /// Free text: no candidates.
    #[default]
    Text,
    /// A label key (the whole field).
    LabelKey,
    /// A label value (the whole field) of the form's key: the sibling `Key`
    /// field, or the key of the label being edited.
    LabelValue,
    /// A workload's labels, `key=value` comma list, one label per key.
    Labels,
    /// Ruleset scopes: labels and label groups, `,` within a scope and `|`
    /// between scopes, one label per key within a scope.
    Scope,
    /// Rule providers or consumers: labels, `group:`, `iplist:`, `all`.
    Actors,
    /// Rule services: service names (`port/proto` is left as typed).
    Services,
    /// A ruleset name (the whole field), from the loaded draft rulesets.
    Ruleset,
    /// A traffic filter's source or destination, in `FilterExpr`'s grammar:
    /// labels, `ipl:`, `wl:` (the loaded workloads), `ip:`, `fqdn:`; `,`
    /// within a group and `|` between groups.
    TrafficActors,
    /// A traffic filter's service (the whole field): protocols, and the
    /// index's services as the `port/proto` the filter takes.
    TrafficService,
    /// A traffic filter's decisions, comma list.
    Decisions,
    /// A traffic filter's transmission kinds, comma list.
    Transmission,
    /// A traffic window (the whole field): the `+`/`-` step ladder.
    Window,
}

impl FieldKind {
    /// The characters that end a token; none: the whole field is one token.
    fn separators(self) -> &'static [char] {
        match self {
            Self::Labels | Self::Actors | Self::Services | Self::Decisions | Self::Transmission => {
                &[',']
            }
            Self::Scope | Self::TrafficActors => &[',', '|'],
            Self::Text
            | Self::LabelKey
            | Self::LabelValue
            | Self::Ruleset
            | Self::TrafficService
            | Self::Window => &[],
        }
    }

    /// The token being typed in `value` and the earlier tokens that matter
    /// (only the current group's where `|` separates groups). `None` for a
    /// text field. A `LabelValue`'s key is not in the field; the form adds it.
    pub fn request(self, value: &str) -> Option<(String, Vec<String>)> {
        let (head, token) = self.split(value);
        let tokens = |text: &str| -> Vec<String> {
            text.split(',')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(String::from)
                .collect()
        };
        let earlier = match self {
            Self::Text => return None,
            Self::LabelKey
            | Self::LabelValue
            | Self::Ruleset
            | Self::TrafficService
            | Self::Window => Vec::new(),
            Self::Labels | Self::Actors | Self::Services | Self::Decisions | Self::Transmission => {
                tokens(head)
            }
            Self::Scope | Self::TrafficActors => tokens(head.rsplit('|').next().unwrap_or(head)),
        };
        Some((token.to_string(), earlier))
    }

    /// A field of token lists where a key or a prefix (`app=`, `group:`)
    /// can be completed further; never a name field (a ruleset named `a:`).
    fn dives(self) -> bool {
        matches!(
            self,
            Self::Labels | Self::Scope | Self::Actors | Self::TrafficActors
        )
    }

    /// The field text split into what stays (`head`, separators and the
    /// blanks after the last one included) and the token being typed.
    fn split(self, value: &str) -> (&str, &str) {
        let start = value.rfind(self.separators()).map_or(0, |i| {
            i + value[i..].chars().next().map_or(1, char::len_utf8)
        });
        let rest = &value[start..];
        let token = rest.trim_start();
        (&value[..value.len() - token.len()], token)
    }
}

/// An open completion: `Ctrl-N`/`Ctrl-P` cycle `candidates` in the token
/// of field `field`; `Esc` puts `prefix` back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub field: usize,
    /// The token as typed.
    pub prefix: String,
    /// The field text before the token, kept as typed.
    pub head: String,
    pub candidates: Vec<String>,
    /// The candidate shown in the field.
    pub index: usize,
}

/// Candidates the strip under a field shows at once.
pub const STRIP_LEN: usize = 5;

impl Completion {
    /// A completion on field `field` (of `kind`, holding `value`) showing
    /// the first of `candidates` in `value`. `prev`, open on the same field
    /// (a key completed to `app=`, now its values), keeps the prefix `Esc`
    /// restores. `None`, and `value` unchanged, without candidates.
    pub fn open(
        prev: Option<&Completion>,
        field: usize,
        kind: FieldKind,
        value: &mut String,
        candidates: Vec<String>,
    ) -> Option<Self> {
        let first = candidates.first()?;
        let (head, prefix) = match prev {
            Some(c) if c.field == field => (c.head.clone(), c.prefix.clone()),
            _ => {
                let (head, token) = kind.split(value);
                (head.to_string(), token.to_string())
            }
        };
        *value = joined(&head, first);
        Some(Self {
            field,
            prefix,
            head,
            candidates,
            index: 0,
        })
    }

    /// The field text with the shown candidate.
    pub fn shown(&self) -> String {
        joined(
            &self.head,
            self.candidates.get(self.index).map_or("", String::as_str),
        )
    }

    /// The field text as typed.
    pub fn typed(&self) -> String {
        format!("{}{}", self.head, self.prefix)
    }

    /// The next (or previous) candidate, wrapping.
    pub fn step(&mut self, forward: bool) {
        let n = self.candidates.len();
        if n == 0 {
            return;
        }
        self.index = if forward {
            (self.index + 1) % n
        } else {
            (self.index + n - 1) % n
        };
    }

    /// The shown candidate in a token list of `kind` is a key or a prefix:
    /// the next `Ctrl-N` completes what follows it instead of cycling.
    pub fn dives(&self, kind: FieldKind) -> bool {
        kind.dives()
            && self
                .candidates
                .get(self.index)
                .is_some_and(|s| dives_into(s))
    }

    /// Up to [`STRIP_LEN`] candidates around the shown one (`true`), then
    /// `+N more` (`false`) for the ones not in the strip.
    pub fn strip(&self) -> Vec<(String, bool)> {
        let start = self.index / STRIP_LEN * STRIP_LEN;
        let end = (start + STRIP_LEN).min(self.candidates.len());
        let mut strip: Vec<(String, bool)> = self.candidates[start..end]
            .iter()
            .enumerate()
            .map(|(i, s)| (s.clone(), start + i == self.index))
            .collect();
        let more = self.candidates.len() - (end - start);
        if more > 0 {
            strip.push((format!("+{more} more"), false));
        }
        strip
    }
}

/// An editor whose fields `Ctrl-N`/`Ctrl-P` complete: a form or the
/// traffic filter editor. `update` computes the candidates.
pub trait Completing {
    /// The field kind, the token being typed and the earlier tokens that
    /// matter; `None` for a text field.
    fn completion_request(&self) -> Option<(FieldKind, String, Vec<String>)>;
    /// Opens a completion on the active field with `candidates` (the first
    /// one shown); `false`, and nothing changes, when there are none.
    fn start_completion(&mut self, candidates: Vec<String>) -> bool;
    /// The next (or previous) candidate, wrapping; nothing without a
    /// completion.
    fn cycle(&mut self, forward: bool);
    /// A completion is open.
    fn completing(&self) -> bool;
    /// The shown candidate is a key or a prefix (`app=`, `group:`): the
    /// next `Ctrl-N` completes what follows it instead of cycling.
    fn completion_dives(&self) -> bool;
    /// Candidates offered before the others, in this order, where they
    /// match (a `w` form's rulesets that cover the destination).
    fn preferred(&self) -> &[String] {
        &[]
    }
}

/// A candidate that only names what comes next: a key (`app=`, `!app=`,
/// `label:app=`; one `=`, at the end) or a bare prefix that has names
/// after it (`group:`, `iplist:`, and the filter's `ipl:`, `wl:`).
/// A value that happens to end in `=` or `:` (`app=foo:`) is not one.
fn dives_into(candidate: &str) -> bool {
    let c = candidate.trim_start_matches('!');
    (c.ends_with('=') && c.find('=') == Some(c.len() - 1))
        || matches!(c, "group:" | "iplist:" | "ipl:" | "wl:")
}

/// `head` with `token` after it; a space after a separator for readability.
fn joined(head: &str, token: &str) -> String {
    if head.is_empty() || head.ends_with(char::is_whitespace) {
        format!("{head}{token}")
    } else {
        format!("{head} {token}")
    }
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
            kind: FieldKind::Text,
        }
    }

    /// This field, completed as `kind`.
    pub fn kind(mut self, kind: FieldKind) -> Self {
        self.kind = kind;
        self
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
    /// `Ctrl-R` creates this ruleset (a `w` form whose name the planner did
    /// not know); armed while the Ruleset field still holds that name.
    /// Boxed: `Overlay::Form` stays within clippy's variant-size budget.
    pub offers_ruleset: Option<Box<RulesetOffer>>,
    /// The completion being cycled; any key but `Ctrl-N`/`Ctrl-P` ends it.
    /// Boxed like `offers_ruleset`.
    pub completion: Option<Box<Completion>>,
    /// A `w` form's ruleset choice; boxed like `offers_ruleset`.
    pub flow: Option<Box<FlowRuleset>>,
}

/// What a `w` form keeps to choose its ruleset again when the draft
/// rulesets arrive, and what completion offers first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowRuleset {
    /// Drafted from a grouped row: the keys its groups are made of.
    pub group_keys: Option<Vec<String>>,
    /// The rulesets whose scope covers the destination, best first.
    pub covering: Vec<String>,
}

/// The ruleset `Ctrl-R` would create: the name the planner checked and the
/// destination scope as tokens (`app=shop, env=prod`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulesetOffer {
    pub name: String,
    pub scope: String,
}

const NAME_CHANGED: &str = "the ruleset name changed; press Enter to check it first";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormEvent {
    Continue,
    /// Enter with every required field filled.
    Submit,
    /// Esc: close without a request.
    Cancel,
    /// Ctrl-R where the form offers it: create the ruleset it names.
    NewRuleset,
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
            offers_ruleset: None,
            completion: None,
            flow: None,
        }
    }

    /// Up to [`STRIP_LEN`] candidates around the shown one (`true`), then
    /// `+N more` (`false`) for the ones not in the strip.
    pub fn completion_strip(&self) -> Option<Vec<(String, bool)>> {
        self.completion.as_ref().map(|c| c.strip())
    }

    /// Ends the completion; `restore` puts the typed token back.
    fn end_completion(&mut self, restore: bool) {
        if let Some(c) = self.completion.take()
            && restore
            && let Some(f) = self.fields.get_mut(c.field)
        {
            f.value = c.typed();
        }
    }

    /// The offer, while the Ruleset field holds the offered name (trimmed,
    /// any case); an edit to another name disarms it.
    pub fn ruleset_offer_armed(&self) -> Option<&RulesetOffer> {
        self.offers_ruleset.as_deref().filter(|o| {
            self.value("Ruleset")
                .trim()
                .eq_ignore_ascii_case(o.name.trim())
        })
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
        // `update` normally intercepts Ctrl-N/Ctrl-P before the form sees
        // them (it opens a completion from the index); here they only cycle.
        if ctrl && matches!(k.code, KeyCode::Char('n' | 'p')) {
            self.cycle(k.code == KeyCode::Char('n'));
            return FormEvent::Continue;
        }
        if self.completion.is_some() {
            // Esc puts the typed token back; any other key keeps the shown
            // candidate and then does what it always does.
            let esc = k.code == KeyCode::Esc;
            self.end_completion(esc);
            if esc {
                return FormEvent::Continue;
            }
        }
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
            KeyCode::Char('r') if ctrl && self.offers_ruleset.is_some() => {
                if self.ruleset_offer_armed().is_some() {
                    return FormEvent::NewRuleset;
                }
                self.error = Some(NAME_CHANGED.into());
            }
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

impl Completing for Form {
    /// The earlier tokens are those of the current scope for `Scope`; the
    /// key for `LabelValue`.
    fn completion_request(&self) -> Option<(FieldKind, String, Vec<String>)> {
        let field = self.fields.get(self.active)?;
        let (token, mut earlier) = field.kind.request(&field.value)?;
        if field.kind == FieldKind::LabelValue {
            let key = match (&self.intent, self.fields.iter().find(|f| f.label == "Key")) {
                (_, Some(f)) => f.value.trim().to_string(),
                (EditIntent::UpdateLabel(l), None) => l.key.clone(),
                _ => String::new(),
            };
            earlier = vec![key];
        }
        Some((field.kind, token, earlier))
    }

    fn start_completion(&mut self, candidates: Vec<String>) -> bool {
        let active = self.active;
        let Some(field) = self.fields.get_mut(active) else {
            return false;
        };
        let prev = self.completion.as_deref();
        match Completion::open(prev, active, field.kind, &mut field.value, candidates) {
            Some(c) => {
                self.completion = Some(Box::new(c));
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
        if let Some(f) = self.fields.get_mut(c.field) {
            f.value = c.shown();
        }
    }

    fn completing(&self) -> bool {
        self.completion.is_some()
    }

    fn completion_dives(&self) -> bool {
        self.completion
            .as_ref()
            .is_some_and(|c| self.fields.get(c.field).is_some_and(|f| c.dives(f.kind)))
    }

    fn preferred(&self) -> &[String] {
        self.flow.as_deref().map_or(&[], |f| f.covering.as_slice())
    }
}

/// The start of `text` that fits in `width` cells, with `…` at the end
/// when the rest is cut.
pub fn clip(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let head: String = text.chars().take(width - 1).collect();
    format!("{head}…")
}

/// `text` in up to `rows` lines of at most `width` characters, broken at
/// spaces (a word longer than a line is broken); what does not fit ends in `…`.
pub fn wrap_rows(text: &str, width: usize, rows: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() && lines.len() < rows {
        if lines.len() + 1 == rows || rest.chars().count() <= width {
            lines.push(clip(rest, width));
            break;
        }
        let cut: String = rest.chars().take(width + 1).collect();
        let at = cut.rfind(' ').filter(|&i| i > 0).unwrap_or_else(|| {
            cut.char_indices()
                .nth(width.max(1))
                .map_or(cut.len(), |(i, _)| i)
        });
        let (line, tail) = rest.split_at(at);
        lines.push(line.trim_end().to_string());
        rest = tail.trim_start();
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
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

    #[test]
    fn wrap_rows_breaks_at_spaces_and_cuts_the_last_row() {
        assert_eq!(wrap_rows("a short hint", 20, 2), vec!["a short hint"]);
        assert_eq!(wrap_rows("", 20, 2), vec![""]);
        assert_eq!(
            wrap_rows("no ruleset covers app=shop: type a new name", 20, 2),
            vec!["no ruleset covers", "app=shop: type a ne…"]
        );
        assert_eq!(
            wrap_rows("abcdefghij klm", 5, 3),
            vec!["abcde", "fghij", "klm"],
            "a long word is broken"
        );

        assert_eq!(wrap_rows("one two three", 7, 1), vec!["one tw…"]);
    }

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
    fn ctrl_r_fires_only_for_the_offered_name() {
        let mut f = flow_like();
        assert_eq!(f.on_key(ctrl('r')), FormEvent::Continue, "no offer");
        assert_eq!(f.error, None);
        f.fields[0].value = " new-app ".into();
        f.offers_ruleset = Some(Box::new(RulesetOffer {
            name: "New-App".into(),
            scope: "app=shop, env=prod".into(),
        }));
        assert!(f.ruleset_offer_armed().is_some(), "trimmed, any case");
        assert_eq!(f.on_key(ctrl('r')), FormEvent::NewRuleset);
        f.active = 0;
        f.on_key(k(KeyCode::Char('2')));
        assert!(f.ruleset_offer_armed().is_none(), "an edit disarms it");
        assert_eq!(f.on_key(ctrl('r')), FormEvent::Continue);
        assert_eq!(
            f.error.as_deref(),
            Some("the ruleset name changed; press Enter to check it first")
        );
    }

    fn actors_form(value: &str) -> Form {
        Form::new(
            "New allow rule",
            vec![
                Field::new("Providers", value, "actors", true).kind(FieldKind::Actors),
                Field::new("Note", "", "optional", false),
            ],
            EditIntent::CreateLabel,
        )
    }

    fn cands(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn completion_request_gives_the_token_and_the_earlier_ones() {
        let f = actors_form("app=shop,  !role=w");
        assert_eq!(
            f.completion_request(),
            Some((
                FieldKind::Actors,
                "!role=w".to_string(),
                cands(&["app=shop"])
            ))
        );
        let mut scope = actors_form("app=a, env=b | role=");
        scope.fields[0].kind = FieldKind::Scope;
        let (_, token, earlier) = scope.completion_request().unwrap();
        assert_eq!(
            (token.as_str(), earlier),
            ("role=", Vec::new()),
            "this scope only"
        );
        let mut label = label_form();
        label.fields[0].kind = FieldKind::LabelKey;
        label.fields[1].kind = FieldKind::LabelValue;
        label.fields[0].value = "role".into();
        label.active = 1;
        assert_eq!(
            label.completion_request(),
            Some((FieldKind::LabelValue, String::new(), cands(&["role"])))
        );
        label.active = 2;
        assert_eq!(label.completion_request(), None, "a text field");
    }

    #[test]
    fn ctrl_n_cycles_ctrl_p_goes_back_and_earlier_tokens_stay() {
        let mut f = actors_form("app=shop,ro");
        assert!(
            !f.start_completion(Vec::new()),
            "no candidates, no completion"
        );
        assert_eq!(f.completion, None);
        assert!(f.start_completion(cands(&["role=db", "role=web", "role="])));
        assert_eq!(f.value("Providers"), "app=shop, role=db");
        assert_eq!(f.on_key(ctrl('n')), FormEvent::Continue);
        assert_eq!(f.value("Providers"), "app=shop, role=web");
        f.on_key(ctrl('n'));
        assert_eq!(f.value("Providers"), "app=shop, role=");
        assert!(
            f.completion_dives(),
            "a key: the next Ctrl-N completes its values"
        );
        f.on_key(ctrl('n'));
        assert_eq!(f.value("Providers"), "app=shop, role=db", "wraps");
        f.on_key(ctrl('p'));
        assert_eq!(f.value("Providers"), "app=shop, role=");
        f.on_key(ctrl('p'));
        assert_eq!(f.value("Providers"), "app=shop, role=web");
    }

    #[test]
    fn a_typed_key_accepts_and_esc_restores_the_prefix() {
        let mut f = actors_form("app=shop, ro");
        f.start_completion(cands(&["role=db", "role=web"]));
        f.on_key(ctrl('n'));
        assert_eq!(f.on_key(k(KeyCode::Char(','))), FormEvent::Continue);
        assert_eq!(
            f.value("Providers"),
            "app=shop, role=web,",
            "accepted, then typed"
        );
        assert_eq!(f.completion, None);
        let mut f = actors_form("app=shop, ro");
        f.start_completion(cands(&["role=db", "role=web"]));
        f.on_key(ctrl('n'));
        assert_eq!(
            f.on_key(k(KeyCode::Esc)),
            FormEvent::Continue,
            "not a cancel"
        );
        assert_eq!(
            f.value("Providers"),
            "app=shop, ro",
            "the typed prefix is back"
        );
        assert_eq!(f.completion, None);
        assert_eq!(
            f.on_key(k(KeyCode::Esc)),
            FormEvent::Cancel,
            "a second Esc cancels"
        );
        // Tab accepts and moves on; Ctrl-N without a completion does nothing.
        let mut f = actors_form("ro");
        f.start_completion(cands(&["role="]));
        f.on_key(k(KeyCode::Tab));
        assert_eq!((f.value("Providers"), f.active), ("role=", 1));
        let before = f.clone();
        f.on_key(ctrl('n'));
        assert_eq!(f, before);
    }

    #[test]
    fn only_a_key_or_a_bare_prefix_dives() {
        let mut f = actors_form("ap");
        f.start_completion(cands(&[
            "app=foo:", "app=x=", "app=", "group:", "!iplist:", "group:a:",
        ]));
        let mut dives = Vec::new();
        for _ in 0..6 {
            dives.push(f.completion_dives());
            f.on_key(ctrl('n'));
        }
        assert_eq!(dives, [false, false, true, true, true, false]);
    }

    #[test]
    fn a_second_start_on_the_field_keeps_the_typed_prefix() {
        let mut f = actors_form("ro");
        f.start_completion(cands(&["role="]));
        // `update` asks again for what follows `role=`.
        assert_eq!(f.completion_request().unwrap().1, "role=");
        f.start_completion(cands(&["role=db", "role=web"]));
        assert_eq!(f.value("Providers"), "role=db");
        f.on_key(k(KeyCode::Esc));
        assert_eq!(f.value("Providers"), "ro");
    }

    #[test]
    fn the_strip_shows_five_around_the_current_one_and_counts_the_rest() {
        let mut f = actors_form("");
        assert_eq!(f.completion_strip(), None);
        let all: Vec<String> = (1..=7).map(|i| format!("c{i}")).collect();
        f.start_completion(all);
        let strip = f.completion_strip().unwrap();
        assert_eq!(strip.len(), 6);
        assert_eq!(strip[0], ("c1".to_string(), true));
        assert_eq!(strip[5], ("+2 more".to_string(), false));
        for _ in 0..5 {
            f.on_key(ctrl('n'));
        }
        let strip = f.completion_strip().unwrap();
        assert_eq!(strip[0], ("c6".to_string(), true), "the next page");
        assert_eq!(strip.last().unwrap().0, "+5 more");
        f.start_completion(cands(&["only"]));
        assert_eq!(
            f.completion_strip().unwrap(),
            vec![("only".to_string(), true)]
        );
    }

    #[test]
    fn clip_cuts_the_end_with_an_ellipsis() {
        assert_eq!(clip("scope app=shop", 20), "scope app=shop");
        assert_eq!(clip("abcdefghij", 5), "abcd…");
        assert_eq!(clip("ab", 0), "");
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
