//! What of the loaded events is shown: the event-type terms
//! (`sec_policy, !user.login, *.delete`) and a severity floor (`warning+`).
//! Both are local, over events already fetched: the PCE events API takes
//! one `event_type` and one `severity`, so neither a list nor a range can
//! be sent. Shared by `events` (CLI) and the Events screen (TUI).
use super::events::Event;

/// One term: a substring, or a glob when it holds a `*` (the whole type
/// must match it: `sec_policy.*`, `*.delete`, `*`). Lowercase.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Term {
    Substring(String),
    Glob(String),
}

impl Term {
    fn new(text: &str) -> Self {
        let t = text.to_lowercase();
        if t.contains('*') {
            Term::Glob(t)
        } else {
            Term::Substring(t)
        }
    }

    /// `ty` is lowercase.
    fn matches(&self, ty: &str) -> bool {
        match self {
            Term::Substring(s) => ty.contains(s.as_str()),
            Term::Glob(g) => glob(g, ty),
        }
    }
}

/// `*` matches any run of characters (none included); everything else
/// itself. The whole of `text` must match.
fn glob(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    // `split` always yields a first part; without a `*` it is the pattern.
    let first = parts.next().unwrap_or("");
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        return rest.is_empty();
    };
    for p in middle {
        match rest.find(p) {
            Some(i) => rest = &rest[i + p.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

/// The event-type filter: comma-separated terms, case-insensitive; `!term`
/// excludes. Includes OR together (none: every type), then the excludes
/// remove. Empty terms are ignored, so blank text keeps everything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypeFilter {
    include: Vec<Term>,
    exclude: Vec<Term>,
}

impl TypeFilter {
    /// `sec_policy, rule_set` (either), `!user.login` (not), `*.delete`.
    /// A `!` starts its term and needs one after it, as in the traffic
    /// filter: `!`, `!!a` and `a !b` are errors.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut f = Self::default();
        for t in text.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            let (list, term) = match t.strip_prefix('!').map(str::trim_start) {
                Some("") => {
                    return Err("! needs a type after it, e.g. !user.login".into());
                }
                Some(rest) if rest.starts_with('!') => {
                    return Err(format!("{t:?}: one ! per term (no double negation)"));
                }
                Some(rest) => (&mut f.exclude, rest),
                None if t.contains('!') => {
                    return Err(format!("{t:?}: use , before ! (agent, !agent.update)"));
                }
                None => (&mut f.include, t),
            };
            let term = Term::new(term);
            if !list.contains(&term) {
                list.push(term);
            }
        }
        Ok(f)
    }

    /// The filter of the excludes alone: each entry a type, substring or
    /// glob (a leading `!` is allowed and changes nothing).
    pub fn excluding<S: AsRef<str>>(types: &[S]) -> Result<Self, String> {
        Self::parse(&exclude_text(types))
    }

    /// No term: every type is kept.
    pub fn is_empty(&self) -> bool {
        self.include.is_empty() && self.exclude.is_empty()
    }

    /// An event of type `ty` (missing: empty) is kept.
    pub fn keeps_type(&self, ty: &str) -> bool {
        let ty = ty.to_lowercase();
        (self.include.is_empty() || self.include.iter().any(|t| t.matches(&ty)))
            && !self.exclude.iter().any(|t| t.matches(&ty))
    }

    pub fn keeps(&self, e: &Event) -> bool {
        self.keeps_type(e.event_type.as_deref().unwrap_or(""))
    }

    /// Both filters' terms: `self`'s, then `other`'s not already there.
    pub fn and(mut self, other: &Self) -> Self {
        for t in &other.include {
            if !self.include.contains(t) {
                self.include.push(t.clone());
            }
        }
        for t in &other.exclude {
            if !self.exclude.contains(t) {
                self.exclude.push(t.clone());
            }
        }
        self
    }
}

/// `!a, !b`: `types` as the excludes of a type filter's text; blank
/// entries dropped, a leading `!` not doubled.
pub fn exclude_text<S: AsRef<str>>(types: &[S]) -> String {
    types
        .iter()
        .map(|t| t.as_ref().trim())
        .map(|t| t.strip_prefix('!').unwrap_or(t).trim_start())
        .filter(|t| !t.is_empty())
        .map(|t| format!("!{t}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The syslog severities the PCE uses, worst first.
pub const SEVERITIES: [&str; 8] = [
    "emerg", "alert", "crit", "err", "warning", "notice", "info", "debug",
];

/// A severity's syslog level, 0 (`emerg`) to 7 (`debug`); `critical`,
/// `error` and `warn` are aliases. Case-insensitive; `None` for another word.
pub fn severity_level(s: &str) -> Option<u8> {
    let s = s.trim().to_ascii_lowercase();
    let s = match s.as_str() {
        "critical" => "crit",
        "error" => "err",
        "warn" => "warning",
        other => other,
    };
    SEVERITIES.iter().position(|v| *v == s).map(|i| i as u8)
}

/// The severity field: empty (any), one severity (sent to the PCE, which
/// takes one value), or `X+`: `X` and worse, which the PCE cannot be
/// asked for, so nothing is sent and the loaded events are filtered here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SeverityFilter {
    #[default]
    Any,
    /// Sent as is (the PCE decides what it accepts, as before `X+`).
    Exact(String),
    /// The worst level kept, inclusive (`warning+`: 4).
    AtLeast(u8),
}

impl SeverityFilter {
    pub fn parse(text: &str) -> Result<Self, String> {
        let t = text.trim();
        if t.is_empty() {
            return Ok(Self::Any);
        }
        match t.strip_suffix('+') {
            Some(name) => severity_level(name).map(Self::AtLeast).ok_or_else(|| {
                format!("severity {t:?}: X+ takes one of {}", SEVERITIES.join(", "))
            }),
            None => Ok(Self::Exact(t.to_string())),
        }
    }

    /// The PCE's `severity` parameter: only an exact severity.
    pub fn pce_value(&self) -> Option<String> {
        match self {
            Self::Exact(s) => Some(s.clone()),
            _ => None,
        }
    }

    /// Whether the floor keeps `e` (an event without a known severity is
    /// not "that severity or worse"). `Any` and `Exact` keep every event:
    /// the PCE did the filtering.
    pub fn keeps(&self, e: &Event) -> bool {
        match self {
            Self::AtLeast(floor) => e
                .severity
                .as_deref()
                .and_then(severity_level)
                .is_some_and(|l| l <= *floor),
            _ => true,
        }
    }

    /// The filter is applied here, not by the PCE.
    pub fn is_local(&self) -> bool {
        matches!(self, Self::AtLeast(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keeps(f: &str, ty: &str) -> bool {
        TypeFilter::parse(f).unwrap().keeps_type(ty)
    }

    #[test]
    fn terms_include_or_and_excludes_remove() {
        assert!(keeps("", "user.login"), "blank keeps all");
        assert!(keeps("  , ,", "user.login"), "empty terms ignored");
        assert!(keeps("sec_policy, rule_set", "sec_policy.create"));
        assert!(keeps("sec_policy, rule_set", "rule_set.update"));
        assert!(!keeps("sec_policy, rule_set", "user.login"));
        assert!(!keeps("!user.login", "user.login"));
        assert!(keeps("!user.login", "agent.update"));
        assert!(
            !keeps("user, !user.login", "user.login"),
            "excludes apply after includes"
        );
        assert!(keeps("user, !user.login", "user.logout"));
        assert!(keeps("USER.Login", "user.LOGIN"), "case-insensitive");
        assert!(keeps("  user.login  ,", "user.login"), "trimmed");
        assert!(
            keeps("! user.login", "agent.update"),
            "blank after ! allowed"
        );
        assert!(!keeps("! user.login", "user.login"));
        assert!(!keeps("agent", ""), "no type: an include term misses it");
        assert!(keeps("!agent", ""), "and an exclude term keeps it");
    }

    #[test]
    fn globs_match_the_whole_type() {
        assert!(keeps("sec_policy.*", "sec_policy.create"));
        assert!(!keeps("sec_policy.*", "x.sec_policy.create"));
        assert!(keeps("*.delete", "label.delete"));
        assert!(!keeps("*.delete", "label.delete_all"));
        assert!(keeps("*", "anything") && keeps("*", ""));
        assert!(!keeps("!*", "anything") && !keeps("!*", ""));
        assert!(keeps("a*b*c", "aXbYc") && keeps("a*b*c", "abc"));
        assert!(!keeps("a*b*c", "acb"));
        assert!(keeps("a**c", "ac"));
        assert!(!keeps("ab*ba", "aba"), "the parts may not overlap");
        assert!(keeps("user.*,!*logout", "user.login"));
        assert!(!keeps("user.*,!*logout", "user.logout"));
    }

    #[test]
    fn a_bang_starts_its_term_and_needs_one() {
        for bad in ["!", " ! ", "a, !", "!!a", "! !a", "a !b", "a!"] {
            assert!(TypeFilter::parse(bad).is_err(), "{bad:?}");
        }
        assert!(TypeFilter::parse("a, !b").is_ok());
    }

    #[test]
    fn duplicate_terms_count_once() {
        let f = TypeFilter::parse("a, A, a, !b, !B").unwrap();
        assert_eq!(f, TypeFilter::parse("a, !b").unwrap());
        assert_eq!(
            TypeFilter::parse("a")
                .unwrap()
                .and(&TypeFilter::parse("a, !c").unwrap()),
            TypeFilter::parse("a, !c").unwrap()
        );
        assert!(TypeFilter::parse(" , ").unwrap().is_empty());
    }

    #[test]
    fn excludes_from_a_list() {
        assert_eq!(
            exclude_text(&["user.login", " !agent.update ", "", "!"]),
            "!user.login, !agent.update"
        );
        let f = TypeFilter::excluding(&["user.login"]).unwrap();
        assert!(!f.keeps_type("user.login") && f.keeps_type("agent.update"));
        assert!(TypeFilter::excluding::<&str>(&[]).unwrap().is_empty());
    }

    fn event(sev: Option<&str>) -> Event {
        serde_json::from_value(serde_json::json!({"href": "/x", "severity": sev})).unwrap()
    }

    #[test]
    fn severity_plus_is_local_and_a_plain_one_goes_to_the_pce() {
        assert_eq!(SeverityFilter::parse(" ").unwrap(), SeverityFilter::Any);
        let exact = SeverityFilter::parse(" err ").unwrap();
        assert_eq!(exact.pce_value().as_deref(), Some("err"));
        assert!(!exact.is_local() && exact.keeps(&event(Some("info"))));
        let w = SeverityFilter::parse("Warning+").unwrap();
        assert_eq!(w, SeverityFilter::AtLeast(4));
        assert_eq!(w.pce_value(), None);
        assert!(w.is_local());
        for s in [
            "emerg", "alert", "crit", "critical", "err", "error", "warning", "warn",
        ] {
            assert!(w.keeps(&event(Some(s))), "{s}");
        }
        for s in ["notice", "info", "debug", "other"] {
            assert!(!w.keeps(&event(Some(s))), "{s}");
        }
        assert!(!w.keeps(&event(None)));
        let info = SeverityFilter::parse("info+").unwrap();
        assert!(info.keeps(&event(Some("notice"))) && !info.keeps(&event(Some("debug"))));
        assert!(
            SeverityFilter::parse("debug+")
                .unwrap()
                .keeps(&event(Some("debug")))
        );
        assert!(SeverityFilter::parse("loud+").is_err());
        assert!(SeverityFilter::parse("+").is_err());
    }

    #[test]
    fn levels_follow_syslog() {
        assert_eq!(severity_level("emerg"), Some(0));
        assert_eq!(severity_level("CRITICAL"), Some(2));
        assert_eq!(severity_level("warn"), Some(4));
        assert_eq!(severity_level("debug"), Some(7));
        assert_eq!(severity_level("x"), None);
    }
}
