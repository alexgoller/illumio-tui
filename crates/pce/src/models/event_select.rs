//! What of the loaded events is shown: the event-type terms
//! (`sec_policy, !=user.login, *.delete`), the actor terms in the same
//! syntax (`!system, user:*`) and a severity floor (`warning+`). All are
//! local, over events already fetched: the PCE events API takes one
//! `event_type` and one `severity`, so neither a list nor a range can be
//! sent, and it is not asked to filter by actor. Shared by `events` (CLI)
//! and the Events screen (TUI).
use super::events::Event;

/// One term: a substring, or a glob when it holds a `*` or starts with
/// `=` (the whole type must match it: `sec_policy.*`, `*.delete`, `*`,
/// `=user.login`). Lowercase.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Term {
    Substring(String),
    Glob(String),
}

impl Term {
    fn new(text: &str, exact: bool) -> Self {
        let t = text.to_lowercase();
        if exact || t.contains('*') {
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
pub(crate) fn glob(pattern: &str, text: &str) -> bool {
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

/// What a filter's errors call a term, with examples.
struct Words {
    /// ` type`, ` actor`: after `a`, so `a type`, `an actor`.
    noun: &'static str,
    eg: &'static str,
    list_eg: &'static str,
}

const TYPE_WORDS: Words = Words {
    noun: " type",
    eg: "user.login",
    list_eg: "agent, !agent.update",
};

const ACTOR_WORDS: Words = Words {
    noun: "n actor",
    eg: "system",
    list_eg: "user:*, !system",
};

/// One term of the type filter's text, taken apart: `!` (excludes), `=`
/// (exact: the whole type) and the type, substring or glob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermText {
    pub exclude: bool,
    pub exact: bool,
    /// Trimmed, case kept.
    pub body: String,
}

impl TermText {
    /// `t`, a non-empty trimmed term: `[!][=]type`, blanks allowed after
    /// `!` and `=`. A `!` starts its term and needs a type after it, as in
    /// the traffic filter (`!`, `!!a`, `a !b`, `!a !b` are errors); so does
    /// `=` (`=`, `!=`, `==a`, `a=b`).
    pub fn parse(t: &str) -> Result<Self, String> {
        Self::parse_as(t, &TYPE_WORDS)
    }

    /// As [`parse`](Self::parse), for an actor term (`!system`,
    /// `=alex@ryte.de`): the errors name an actor.
    pub fn parse_actor(t: &str) -> Result<Self, String> {
        Self::parse_as(t, &ACTOR_WORDS)
    }

    fn parse_as(t: &str, w: &Words) -> Result<Self, String> {
        let Words { noun, eg, list_eg } = w;
        let (exclude, rest) = match t.strip_prefix('!') {
            Some(r) => (true, r.trim_start()),
            None => (false, t),
        };
        if exclude && rest.is_empty() {
            return Err(format!("! needs a{noun} after it, e.g. !={eg}"));
        }
        if rest.starts_with('!') {
            return Err(format!("{t:?}: one ! per term (no double negation)"));
        }
        let (exact, body) = match rest.strip_prefix('=') {
            Some(r) => (true, r.trim_start()),
            None => (false, rest),
        };
        if exact && body.is_empty() {
            return Err(format!("= needs a{noun} after it, e.g. ={eg}"));
        }
        if body.contains('!') {
            return Err(format!("{t:?}: use , before ! ({list_eg})"));
        }
        if body.contains('=') {
            return Err(format!("{t:?}: = starts its term (={eg}, !={eg})"));
        }
        Ok(Self {
            exclude,
            exact,
            body: body.to_string(),
        })
    }
}

/// The event-type filter: comma-separated terms, case-insensitive; `!term`
/// excludes, `=type` is exact. Includes OR together (none: every type), then the excludes
/// remove. Empty terms are ignored, so blank text keeps everything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypeFilter {
    include: Vec<Term>,
    exclude: Vec<Term>,
}

impl TypeFilter {
    /// `sec_policy, rule_set` (either), `!user.login` (not, substring),
    /// `!=user.login` (not this exact type), `*.delete`. See
    /// [`TermText::parse`] for what is refused.
    pub fn parse(text: &str) -> Result<Self, String> {
        Self::parse_as(text, &TYPE_WORDS)
    }

    fn parse_as(text: &str, w: &Words) -> Result<Self, String> {
        let mut f = Self::default();
        for t in text.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            let p = TermText::parse_as(t, w)?;
            let list = if p.exclude {
                &mut f.exclude
            } else {
                &mut f.include
            };
            let term = Term::new(&p.body, p.exact);
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
        self.keeps_any(&[ty])
    }

    /// A term matches when it matches any of `names` (one value under
    /// several names, as an actor is): kept when an include matches (or
    /// there is none) and no exclude does.
    fn keeps_any(&self, names: &[&str]) -> bool {
        let names: Vec<String> = names.iter().map(|n| n.to_lowercase()).collect();
        let hit = |terms: &[Term]| terms.iter().any(|t| names.iter().any(|n| t.matches(n)));
        (self.include.is_empty() || hit(&self.include)) && !hit(&self.exclude)
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

/// The names an actor (as [`Event::actor`] writes it) answers to: the
/// whole string and, for `kind:name`, the name alone (`user:alex@ryte.de`
/// and `alex@ryte.de`; `system` only itself). The placeholder name `?`
/// of an unnamed user or agent (`agent:?`) is not a name: `=?` is the
/// actor-less event only.
pub fn actor_names(actor: &str) -> Vec<&str> {
    let mut names = vec![actor];
    if let Some((_, name)) = actor.split_once(':')
        && name != "?"
    {
        names.push(name);
    }
    names
}

/// `actor` can be written as one exact term (`=actor`, `!=actor`): it
/// holds none of `,` (a term break), `!`, `=` (refused inside a term) or
/// `*` (a glob).
pub fn is_one_term(actor: &str) -> bool {
    !actor.contains([',', '!', '=', '*'])
}

/// The actor filter: the type filter's terms (`,` lists, `!` excludes,
/// `=` exact, `*` globs, any case), each matched against the whole actor
/// string and the part after `kind:` (see [`actor_names`]), so
/// `=alex@ryte.de`, `user:*`, `!system` and `!agent:*` all work.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventActorFilter(TypeFilter);

impl EventActorFilter {
    /// See [`TypeFilter::parse`]; the errors name an actor.
    pub fn parse(text: &str) -> Result<Self, String> {
        TypeFilter::parse_as(text, &ACTOR_WORDS).map(Self)
    }

    /// No term: every actor is kept.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The actor `actor` (`user:alex@ryte.de`, `system`, `?`) is kept.
    pub fn keeps_actor(&self, actor: &str) -> bool {
        self.0.keeps_any(&actor_names(actor))
    }

    pub fn keeps(&self, e: &Event) -> bool {
        self.keeps_actor(&e.actor())
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

/// A severity's syslog level, 0 (`emerg`) to 7 (`debug`); `emergency`,
/// `critical`, `error` and `warn` are aliases. Case-insensitive; `None`
/// for another word.
pub fn severity_level(s: &str) -> Option<u8> {
    let s = s.trim().to_ascii_lowercase();
    let s = match s.as_str() {
        "emergency" => "emerg",
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
/// Any other word is refused here, before a request, naming the values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SeverityFilter {
    #[default]
    Any,
    /// One of [`SEVERITIES`] (an alias already the PCE's word).
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
            None => severity_level(t)
                .map(|l| Self::Exact(SEVERITIES[usize::from(l)].to_string()))
                .ok_or_else(|| {
                    format!(
                        "severity {t:?}: use one of {}, or X+ for X and worse",
                        SEVERITIES.join(", ")
                    )
                }),
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
        for bad in ["!a !b", "!a!b", "!=a!b"] {
            let err = TypeFilter::parse(bad).unwrap_err();
            assert!(err.contains("use , before !"), "{bad:?}: {err}");
        }
        assert!(TypeFilter::parse("a, !b").is_ok());
    }

    #[test]
    fn a_leading_equals_is_exact() {
        assert!(keeps("=user.login", "user.login"));
        assert!(!keeps("=user.login", "user.login_session_terminated"));
        assert!(keeps("user.login", "user.login_session_terminated"));
        assert!(!keeps("!=user.login", "user.login"));
        assert!(
            keeps("!=user.login", "user.login_session_terminated"),
            "an exact exclude keeps the longer type"
        );
        assert!(!keeps("!user.login", "user.login_session_terminated"));
        assert!(
            keeps("= User.Login", "user.login"),
            "blank after =, any case"
        );
        assert!(!keeps("! = user.login", "user.login"));
        assert!(keeps("=sec_policy.*", "sec_policy.create"), "still a glob");
        assert_eq!(
            TypeFilter::parse("=a, =A").unwrap(),
            TypeFilter::parse("=a").unwrap()
        );
        for bad in ["=", " = ", "!=", "! =", "==a", "!==a", "a=b", "=a=b"] {
            assert!(TypeFilter::parse(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            TermText::parse("! = Ab").unwrap(),
            TermText {
                exclude: true,
                exact: true,
                body: "Ab".into()
            }
        );
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

    fn keeps_actor(f: &str, actor: &str) -> bool {
        EventActorFilter::parse(f).unwrap().keeps_actor(actor)
    }

    #[test]
    fn actor_terms_match_the_whole_actor_or_the_name_after_its_kind() {
        let alex = "user:alex@ryte.de";
        assert!(
            keeps_actor("", alex) && keeps_actor("", "?"),
            "blank keeps all"
        );
        assert!(keeps_actor("=alex@ryte.de", alex), "exact, after the kind");
        assert!(keeps_actor("=user:alex@ryte.de", alex), "exact, whole");
        assert!(!keeps_actor("=alex", alex), "exact is not a substring");
        assert!(keeps_actor("alex", alex), "substring");
        assert!(keeps_actor("user:*", alex) && !keeps_actor("user:*", "agent:web01"));
        assert!(keeps_actor("alex*", alex), "a glob anchors on the name too");
        assert!(!keeps_actor("ryte*", alex), "anchored");
        assert!(keeps_actor("*@ryte.de", alex));
        assert!(!keeps_actor("!system", "system") && keeps_actor("!system", alex));
        assert!(!keeps_actor("!agent:*", "agent:web01") && keeps_actor("!agent:*", alex));
        assert!(
            !keeps_actor("!=web01", "agent:web01"),
            "an exclude by name alone"
        );
        assert!(keeps_actor("=USER:Alex@Ryte.DE", alex), "case-insensitive");
        assert!(keeps_actor("system, user:*", "system") && keeps_actor("system, user:*", alex));
        assert!(!keeps_actor("system, user:*", "agent:web01"), "includes OR");
        assert!(!keeps_actor("user:*, !=alex@ryte.de", alex), "excludes win");
        assert!(keeps_actor("user:*, !=alex@ryte.de", "user:bob@ryte.de"));
        assert!(keeps_actor("=?", "?") && !keeps_actor("!=?", "?"));
        // `?` is a placeholder, not a name: `agent:?` is another actor.
        assert!(keeps_actor("!=?", "agent:?") && !keeps_actor("=?", "agent:?"));
        assert!(!keeps_actor("=?", "user:?") && keeps_actor("=agent:?", "agent:?"));
        assert_eq!(actor_names("agent:?"), ["agent:?"]);
        assert!(is_one_term(alex) && is_one_term("agent:?"));
        for bad in ["service_account:ci, nightly", "svc!deploy", "a=b", "web*"] {
            assert!(!is_one_term(bad), "{bad}");
        }
        assert!(keeps_actor("=ci", "service_account:ci"));
        assert_eq!(actor_names("system"), ["system"]);
        assert_eq!(actor_names(alex), [alex, "alex@ryte.de"]);
    }

    #[test]
    fn actor_errors_name_an_actor() {
        let e = EventActorFilter::parse("!").unwrap_err();
        assert!(e.contains("an actor") && e.contains("!=system"), "{e}");
        let e = EventActorFilter::parse("a !b").unwrap_err();
        assert!(e.contains("user:*, !system"), "{e}");
        let e = TypeFilter::parse("!").unwrap_err();
        assert_eq!(e, "! needs a type after it, e.g. !=user.login");
        assert!(EventActorFilter::parse(" , ").unwrap().is_empty());
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
    fn an_unknown_severity_is_refused_and_aliases_become_the_pce_word() {
        let err = SeverityFilter::parse("Ordering").unwrap_err();
        assert!(err.contains("\"Ordering\""), "{err}");
        for v in SEVERITIES {
            assert!(err.contains(v), "{err} lists {v}");
        }
        assert!(err.contains("X+"), "{err}");
        for (typed, sent) in [
            ("error", "err"),
            ("Warn", "warning"),
            ("critical", "crit"),
            ("emergency", "emerg"),
            ("ERR", "err"),
            ("notice", "notice"),
        ] {
            assert_eq!(
                SeverityFilter::parse(typed).unwrap().pce_value().as_deref(),
                Some(sent),
                "{typed}"
            );
        }
        assert_eq!(
            SeverityFilter::parse("emergency+").unwrap(),
            SeverityFilter::AtLeast(0)
        );
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
