//! Intra- and extra-scope allow rules (`unscoped_consumers` on the wire):
//! what a rule's consumers mean against its ruleset's scopes, the classic
//! mistakes worth a warning, and how switching between the two rewrites
//! the consumers. Pure: actor tokens in (the grammar of [`crate::tokens`];
//! a ruleset's scopes as `crate::tui::edit::actor_tokens` renders them),
//! text out; the TUI form and the CLI share it.
use crate::tokens::{ActorToken, is_href, split_kv};

/// Where a rule's consumers may be: inside the ruleset's scope (intra) or
/// anywhere (extra, `unscoped_consumers: true`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleScope {
    Intra,
    Extra,
}

impl RuleScope {
    pub fn from_unscoped(unscoped: bool) -> Self {
        if unscoped { Self::Extra } else { Self::Intra }
    }

    /// The wire flag.
    pub fn unscoped(self) -> bool {
        self == Self::Extra
    }

    /// The form's value: `intra` or `extra`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Intra => "intra",
            Self::Extra => "extra",
        }
    }

    /// As views name it: `intra-scope`, `extra-scope`.
    pub fn wording(self) -> &'static str {
        match self {
            Self::Intra => "intra-scope",
            Self::Extra => "extra-scope",
        }
    }

    pub fn other(self) -> Self {
        match self {
            Self::Intra => Self::Extra,
            Self::Extra => Self::Intra,
        }
    }

    /// `intra`/`i` or `extra`/`e`, any case.
    pub fn parse(text: &str) -> Result<Self, String> {
        match text.trim().to_ascii_lowercase().as_str() {
            "intra" | "i" => Ok(Self::Intra),
            "extra" | "e" => Ok(Self::Extra),
            other => Err(format!("{other:?} is not intra or extra")),
        }
    }
}

/// What the scope checks know of the label model: the scope keys and the
/// key of each label group they may meet (`(name, key)`; a group on a scope
/// key pins that key as a label does).
#[derive(Debug, Clone, Copy)]
pub struct ScopeKeys<'a> {
    pub keys: &'a [String],
    pub groups: &'a [(String, String)],
}

impl ScopeKeys<'_> {
    fn group_key(&self, name: &str) -> Option<&str> {
        self.groups
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, k)| k.as_str())
    }
}

/// What a consumer (or scope) token is to the scope.
#[derive(Debug, PartialEq, Eq)]
enum Consumer {
    /// A label (not excluded): it pins its key to its value.
    Label(String, String),
    /// A label group (not excluded) of a known key: it pins that key to
    /// some of its values.
    Group(String),
    /// An IP list or a workload: no scope narrows it.
    Unscoped,
    /// Anything else: `all`, a group of unknown key, an exclusion, a label href.
    Other,
}

fn classify(token: &str, m: ScopeKeys) -> Consumer {
    match ActorToken::parse(token) {
        Ok(ActorToken::Label {
            spec,
            exclusion: false,
        }) if !is_href(&spec) => {
            split_kv(&spec).map_or(Consumer::Other, |(k, v)| Consumer::Label(k, v))
        }
        Ok(ActorToken::LabelGroup {
            name,
            exclusion: false,
        }) => m
            .group_key(&name)
            .map_or(Consumer::Other, |k| Consumer::Group(k.to_string())),
        Ok(ActorToken::IpList {
            exclusion: false, ..
        })
        | Ok(ActorToken::Workload { .. }) => Consumer::Unscoped,
        Ok(ActorToken::Href {
            href,
            exclusion: false,
        }) if href.contains("/ip_lists/") || href.contains("/workloads/") => Consumer::Unscoped,
        _ => Consumer::Other,
    }
}

/// The key a token pins: a label's or a known group's.
fn pin(token: &str, m: ScopeKeys) -> Option<String> {
    match classify(token, m) {
        Consumer::Label(k, _) | Consumer::Group(k) => Some(k),
        Consumer::Unscoped | Consumer::Other => None,
    }
}

/// A comma list as its tokens, trimmed, blanks dropped.
fn tokens(text: &str) -> Vec<&str> {
    text.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect()
}

/// The tokens the scope narrows and those it does not (IP lists, workloads).
fn split_unscoped<'t>(toks: &[&'t str], m: ScopeKeys) -> (Vec<&'t str>, Vec<&'t str>) {
    toks.iter()
        .partition(|t| classify(t, m) != Consumer::Unscoped)
}

/// A ruleset that applies everywhere: no scope, or an empty one.
fn global(scopes: &[Vec<String>]) -> bool {
    scopes.is_empty() || scopes.iter().any(Vec::is_empty)
}

/// Scopes shown in this many at most; the rest as `+N more`.
const SHOWN_SCOPES: usize = 3;

/// Scopes compactly: `app=shop env=prod | app=crm env=prod`.
fn scopes_phrase(scopes: &[Vec<String>]) -> String {
    let mut parts: Vec<String> = scopes
        .iter()
        .take(SHOWN_SCOPES)
        .map(|s| s.join(" "))
        .collect();
    if scopes.len() > SHOWN_SCOPES {
        parts.push(format!("+{} more", scopes.len() - SHOWN_SCOPES));
    }
    parts.join(" | ")
}

/// What the consumers match, one line: intra `role=web` → `role=web inside
/// app=shop env=prod`; extra → `role=web in any app, env`; IP lists and
/// workloads, which no scope narrows, `wherever they are` either way.
/// `scopes` are the ruleset's scopes as actor tokens (`None` while not
/// loaded; empty: a global ruleset).
pub fn meaning(
    consumers: &str,
    scope: RuleScope,
    m: ScopeKeys,
    scopes: Option<&[Vec<String>]>,
) -> String {
    let toks = tokens(consumers);
    if toks.is_empty() {
        return format!("{}: no consumers yet", scope.wording());
    }
    let (scoped, unscoped) = split_unscoped(&toks, m);
    let mut parts = Vec::new();
    if !scoped.is_empty() {
        let c = scoped.join(", ");
        parts.push(match scope {
            RuleScope::Intra => match scopes {
                None => format!("{c} inside the ruleset scope"),
                Some(s) if global(s) => format!("{c} anywhere: global ruleset"),
                Some(s) => format!("{c} inside {}", scopes_phrase(s)),
            },
            RuleScope::Extra => {
                let pinned: Vec<String> = scoped.iter().filter_map(|t| pin(t, m)).collect();
                let free: Vec<&str> = m
                    .keys
                    .iter()
                    .filter(|k| !pinned.contains(k))
                    .map(String::as_str)
                    .collect();
                if free.is_empty() {
                    format!("{c} only: the consumers pin every scope key")
                } else {
                    format!("{c} in any {}", free.join(", "))
                }
            }
        });
    }
    if !unscoped.is_empty() {
        parts.push(format!("{} wherever they are", unscoped.join(", ")));
    }
    parts.join(" · ")
}

/// Whether a scope (its tokens) rules out `key=value`: it holds another
/// value of the key and no group on it that might hold this one.
fn rules_out(scope: &[String], key: &str, value: &str, m: ScopeKeys) -> bool {
    let mut other = false;
    for t in scope {
        match classify(t, m) {
            Consumer::Label(k, v) if k == key => {
                if v == value {
                    return false;
                }
                other = true;
            }
            Consumer::Group(k) if k == key => return false,
            _ => {}
        }
    }
    other
}

/// The mistakes worth a word before the rule is written: an extra-scope
/// rule whose consumers pin no scope key (`role=web` in every app), an
/// intra-scope consumer label every ruleset scope rules out, and
/// intra-scope in a global ruleset. IP lists and workloads, which no scope
/// narrows, never warn. Never blocking.
pub fn warnings(
    consumers: &str,
    scope: RuleScope,
    m: ScopeKeys,
    scopes: Option<&[Vec<String>]>,
) -> Vec<String> {
    let toks = tokens(consumers);
    let (scoped, _) = split_unscoped(&toks, m);
    if scoped.is_empty() {
        return Vec::new();
    }
    match scope {
        RuleScope::Extra => {
            let pins_one = scoped
                .iter()
                .filter_map(|t| pin(t, m))
                .any(|k| m.keys.contains(&k));
            if pins_one || m.keys.is_empty() {
                return Vec::new();
            }
            vec![format!(
                "matches {} in every {}",
                scoped.join(", "),
                m.keys.join(", ")
            )]
        }
        RuleScope::Intra => match scopes {
            None => Vec::new(),
            Some(s) if global(s) => {
                vec![
                    "global ruleset: intra-scope is the same as extra-scope here (anywhere)".into(),
                ]
            }
            Some(s) => scoped
                .iter()
                .filter_map(|t| match classify(t, m) {
                    Consumer::Label(k, v) => Some((k, v)),
                    _ => None,
                })
                .filter(|(k, v)| s.iter().all(|scope| rules_out(scope, k, v, m)))
                .map(|(k, v)| format!("never matches: {k}={v} is outside the ruleset scope"))
                .collect(),
        },
    }
}

/// What the last Scope switch did to the consumers, so that switching back
/// restores them instead of losing what it dropped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Switch {
    /// The Consumers text before and after the switch: switched back with
    /// the text untouched, it is `before` again, exactly.
    pub before: String,
    pub after: String,
    /// Tokens the switch to intra dropped (back on the switch to extra).
    pub dropped: Vec<String>,
    /// Tokens the switch to extra added (gone on the switch to intra), and
    /// whether they replaced `all` (back on the switch to intra).
    pub added: Vec<String>,
    pub replaced_all: bool,
}

/// The Consumers text after the Scope switched to `to`, and what to
/// remember for the way back (`None` after switching straight back).
///
/// Switched straight back (the text as the last switch left it, changed or
/// not), the text is what it was before it, so a switch there and back
/// never changes the rule. Otherwise, to extra: the tokens the last
/// switch to intra dropped come back, then the source's scope labels
/// (`source`, a `w` form's flow source) go in front for each scope key the
/// consumers do not pin (only when they pin one), or in place of `all`, so
/// the rule does not open to every scope. To intra: what the last switch to
/// extra added goes (`all` comes back), then the labels every ruleset
/// scope pins to the same value are dropped, being redundant there; any
/// other label stays (one the scope rules out then warns). IP lists,
/// workloads, groups, exclusions and `all` are never touched otherwise.
pub fn switch_consumers(
    consumers: &str,
    to: RuleScope,
    m: ScopeKeys,
    scopes: Option<&[Vec<String>]>,
    source: &[String],
    last: Option<&Switch>,
) -> (String, Option<Switch>) {
    if let Some(l) = last
        && l.after == consumers
    {
        return (l.before.clone(), None);
    }
    let original = tokens(consumers);
    let mut toks: Vec<String> = original.iter().map(|t| t.to_string()).collect();
    let mut sw = Switch::default();
    match to {
        RuleScope::Extra => {
            if let Some(l) = last {
                let back: Vec<String> = l
                    .dropped
                    .iter()
                    .filter(|d| !toks.contains(d))
                    .cloned()
                    .collect();
                toks.splice(0..0, back);
            }
            let pinned: Vec<String> = toks.iter().filter_map(|t| pin(t, m)).collect();
            let from_source = |all: bool| -> Vec<String> {
                source
                    .iter()
                    .filter(|s| match classify(s, m) {
                        Consumer::Label(k, _) => {
                            m.keys.contains(&k) && (all || !pinned.contains(&k))
                        }
                        _ => false,
                    })
                    .filter(|s| !toks.contains(s))
                    .cloned()
                    .collect()
            };
            let all_at = toks.iter().position(|t| t.eq_ignore_ascii_case("all"));
            match all_at {
                Some(i) if !from_source(true).is_empty() => {
                    sw.added = from_source(true);
                    sw.replaced_all = true;
                    toks.splice(i..=i, sw.added.clone());
                }
                None if !pinned.is_empty() => {
                    sw.added = from_source(false);
                    toks.splice(0..0, sw.added.clone());
                }
                _ => {}
            }
        }
        RuleScope::Intra => {
            if let Some(l) = last {
                toks.retain(|t| !l.added.contains(t));
                if l.replaced_all && !toks.iter().any(|t| t.eq_ignore_ascii_case("all")) {
                    toks.insert(0, "all".into());
                }
            }
            if let Some(s) = scopes.filter(|s| !global(s)) {
                let redundant = |t: &String| match classify(t, m) {
                    Consumer::Label(k, v) => s
                        .iter()
                        .all(|scope| scope.iter().any(|x| x == &format!("{k}={v}"))),
                    _ => false,
                };
                sw.dropped = toks.iter().filter(|t| redundant(t)).cloned().collect();
                toks.retain(|t| !redundant(t));
            }
        }
    }
    sw.before = consumers.to_string();
    sw.after = if toks.iter().map(String::as_str).eq(original.iter().copied()) {
        consumers.to_string()
    } else {
        toks.join(", ")
    };
    (sw.after.clone(), Some(sw))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Vec<String> {
        ["app", "env", "loc"].map(String::from).to_vec()
    }

    /// `All prod envs` is a group of `env` labels.
    fn groups() -> Vec<(String, String)> {
        vec![("All prod envs".into(), "env".into())]
    }

    fn scopes(s: &[&[&str]]) -> Vec<Vec<String>> {
        s.iter()
            .map(|s| s.iter().map(|t| t.to_string()).collect())
            .collect()
    }

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_scope_parses_both_words_and_their_initials() {
        assert_eq!(RuleScope::parse(" Intra "), Ok(RuleScope::Intra));
        assert_eq!(RuleScope::parse("e"), Ok(RuleScope::Extra));
        assert_eq!(RuleScope::parse("I"), Ok(RuleScope::Intra));
        assert!(RuleScope::parse("y").is_err());
        assert!(RuleScope::from_unscoped(true).unscoped());
        assert_eq!(RuleScope::Extra.other(), RuleScope::Intra);
    }

    #[test]
    fn intra_reads_inside_the_rulesets_scopes_and_ip_lists_anywhere() {
        let (k, g) = (keys(), groups());
        let m = ScopeKeys {
            keys: &k,
            groups: &g,
        };
        let shop = scopes(&[&["app=shop", "env=prod"]]);
        let at = |c: &str, s: Option<&[Vec<String>]>| meaning(c, RuleScope::Intra, m, s);
        assert_eq!(
            at("role=web", Some(&shop)),
            "role=web inside app=shop env=prod"
        );
        assert_eq!(at("role=web", None), "role=web inside the ruleset scope");
        assert_eq!(
            at("role=web", Some(&[])),
            "role=web anywhere: global ruleset"
        );
        assert_eq!(
            at("role=web", Some(&scopes(&[&[]]))),
            "role=web anywhere: global ruleset"
        );
        let many = scopes(&[
            &["app=shop", "env=prod"],
            &["app=crm"],
            &["group:G"],
            &["app=erp"],
        ]);
        assert_eq!(
            at("role=web", Some(&many)),
            "role=web inside app=shop env=prod | app=crm | group:G | +1 more"
        );
        assert_eq!(
            at("iplist:Corp", Some(&shop)),
            "iplist:Corp wherever they are"
        );
        assert_eq!(
            at("role=web, iplist:Corp", Some(&shop)),
            "role=web inside app=shop env=prod · iplist:Corp wherever they are"
        );
        assert_eq!(
            meaning(" ", RuleScope::Intra, m, None),
            "intra-scope: no consumers yet"
        );
    }

    #[test]
    fn extra_reads_in_any_scope_key_the_consumers_leave_open() {
        let (k, g) = (keys(), groups());
        let m = ScopeKeys {
            keys: &k,
            groups: &g,
        };
        let at = |c: &str| meaning(c, RuleScope::Extra, m, None);
        assert_eq!(at("role=web"), "role=web in any app, env, loc");
        assert_eq!(
            at("app=shop, role=web"),
            "app=shop, role=web in any env, loc"
        );
        assert_eq!(
            at("group:All prod envs, role=web"),
            "group:All prod envs, role=web in any app, loc",
            "a group pins its key"
        );
        assert_eq!(
            at("app=shop,env=prod,loc=fra,role=web"),
            "app=shop, env=prod, loc=fra, role=web only: the consumers pin every scope key"
        );
        assert_eq!(
            at("iplist:Corp, workload:db1"),
            "iplist:Corp, workload:db1 wherever they are"
        );
        assert_eq!(
            at("!app=dev, role=web, iplist:Corp"),
            "!app=dev, role=web in any app, env, loc · iplist:Corp wherever they are"
        );
    }

    #[test]
    fn extra_without_a_scope_key_pinned_warns_that_it_matches_everywhere() {
        let (k, g) = (keys(), groups());
        let m = ScopeKeys {
            keys: &k,
            groups: &g,
        };
        let w = |c: &str| warnings(c, RuleScope::Extra, m, None);
        assert_eq!(
            w("role=web"),
            vec!["matches role=web in every app, env, loc"]
        );
        assert_eq!(w("all"), vec!["matches all in every app, env, loc"]);
        assert_eq!(
            w("group:Web"),
            vec!["matches group:Web in every app, env, loc"],
            "a group of unknown key pins nothing"
        );
        assert!(
            w("group:All prod envs").is_empty(),
            "a group on env pins it"
        );
        assert!(w("app=shop, role=web").is_empty(), "one scope key pinned");
        assert!(w("iplist:Corp").is_empty(), "IP lists know no scope");
        assert!(w("iplist:Corp, workload:db1").is_empty());
        assert_eq!(
            w("iplist:Corp, role=web"),
            vec!["matches role=web in every app, env, loc"],
            "the IP list is not the problem"
        );
        assert!(w("").is_empty());
    }

    #[test]
    fn intra_warns_of_labels_every_scope_rules_out_and_of_a_global_ruleset() {
        let (k, g) = (keys(), groups());
        let m = ScopeKeys {
            keys: &k,
            groups: &g,
        };
        let shop = scopes(&[&["app=shop", "env=prod"], &["app=shop", "env=dev"]]);
        let w = |c: &str, s: Option<&[Vec<String>]>| warnings(c, RuleScope::Intra, m, s);
        assert_eq!(
            w("app=crm, role=web", Some(&shop)),
            vec!["never matches: app=crm is outside the ruleset scope"]
        );
        assert!(
            w("env=prod, role=web", Some(&shop)).is_empty(),
            "one scope holds it"
        );
        assert!(w("app=shop, role=web", Some(&shop)).is_empty());
        assert!(w("loc=fra", Some(&shop)).is_empty(), "no scope pins loc");
        assert!(
            w(
                "env=dev",
                Some(&scopes(&[&["app=shop", "group:All prod envs"]]))
            )
            .is_empty(),
            "a group on the key may hold it"
        );
        assert_eq!(
            w("env=dev", Some(&scopes(&[&["app=shop", "!env=prod"]]))),
            Vec::<String>::new(),
            "an exclusion pins nothing"
        );
        assert_eq!(
            w("role=web", Some(&[])),
            vec!["global ruleset: intra-scope is the same as extra-scope here (anywhere)"]
        );
        assert!(
            w("iplist:Corp", Some(&[])).is_empty(),
            "IP lists are anywhere anyway"
        );
        assert!(w("app=crm", None).is_empty(), "scopes not loaded");
    }

    #[test]
    fn switching_to_extra_adds_the_sources_scope_labels() {
        let (k, g) = (keys(), groups());
        let m = ScopeKeys {
            keys: &k,
            groups: &g,
        };
        let src = strs(&["app=shop", "env=prod"]);
        let to = |c: &str| switch_consumers(c, RuleScope::Extra, m, None, &src, None).0;
        assert_eq!(to("role=web"), "app=shop, env=prod, role=web");
        assert_eq!(
            to("env=dev, role=web"),
            "app=shop, env=dev, role=web",
            "pinned keys stay"
        );
        assert_eq!(
            to("group:All prod envs, role=web"),
            "app=shop, group:All prod envs, role=web"
        );
        assert_eq!(to("iplist:Corp"), "iplist:Corp", "no label: nothing added");
        assert_eq!(
            to("all, iplist:Corp"),
            "app=shop, env=prod, iplist:Corp",
            "all narrowed"
        );
        assert_eq!(
            switch_consumers("all", RuleScope::Extra, m, None, &[], None).0,
            "all",
            "no flow source: nothing to add"
        );
    }

    #[test]
    fn switching_to_intra_drops_only_what_every_scope_pins_alike() {
        let (k, g) = (keys(), groups());
        let m = ScopeKeys {
            keys: &k,
            groups: &g,
        };
        let shop = scopes(&[&["app=shop"]]);
        let to = |c: &str, s: Option<&[Vec<String>]>| {
            switch_consumers(c, RuleScope::Intra, m, s, &[], None).0
        };
        assert_eq!(
            to("app=shop, env=prod, role=web", Some(&shop)),
            "env=prod, role=web",
            "env=prod still narrows a scope of app=shop alone"
        );
        let both = scopes(&[&["app=shop", "env=prod"], &["app=shop", "env=dev"]]);
        assert_eq!(
            to("app=shop, env=prod, role=web", Some(&both)),
            "env=prod, role=web"
        );
        assert_eq!(
            to("app=crm, role=web", Some(&shop)),
            "app=crm, role=web",
            "kept: it warns"
        );
        assert_eq!(
            to(
                "app=shop, iplist:Corp, !env=dev, group:G, workload:db1",
                Some(&shop)
            ),
            "iplist:Corp, !env=dev, group:G, workload:db1"
        );
        assert_eq!(
            to("app=shop, role=web", None),
            "app=shop, role=web",
            "scopes unknown"
        );
        assert_eq!(
            to("app=shop, role=web", Some(&[])),
            "app=shop, role=web",
            "global"
        );
        assert_eq!(
            to("role=web,iplist:X", Some(&shop)),
            "role=web,iplist:X",
            "text kept"
        );
    }

    #[test]
    fn switching_back_restores_the_consumers() {
        let (k, g) = (keys(), groups());
        let m = ScopeKeys {
            keys: &k,
            groups: &g,
        };
        let shop = scopes(&[&["app=shop", "env=prod"]]);
        let src = strs(&["app=shop", "env=prod"]);
        let round = |c: &str, first: RuleScope, source: &[String]| {
            let (mid, sw) = switch_consumers(c, first, m, Some(&shop), source, None);
            switch_consumers(&mid, first.other(), m, Some(&shop), source, sw.as_ref()).0
        };
        for c in [
            "app=shop, env=prod, role=web",
            "role=web, env=prod, iplist:Corp",
            "all",
            "role=web",
        ] {
            assert_eq!(round(c, RuleScope::Intra, &[]), c, "extra→intra→extra {c}");
            assert_eq!(round(c, RuleScope::Intra, &src), c, "w form {c}");
            assert_eq!(round(c, RuleScope::Extra, &src), c, "intra→extra→intra {c}");
        }
        // Edited in between: the dropped labels come back all the same.
        let (mid, sw) = switch_consumers(
            "app=shop, env=prod, role=web",
            RuleScope::Intra,
            m,
            Some(&shop),
            &[],
            None,
        );
        assert_eq!(mid, "role=web");
        let back = switch_consumers(
            "role=web, iplist:X",
            RuleScope::Extra,
            m,
            Some(&shop),
            &[],
            sw.as_ref(),
        );
        assert_eq!(back.0, "app=shop, env=prod, role=web, iplist:X");
        // `all` the switch to extra replaced comes back on the switch to intra.
        let (mid, sw) = switch_consumers("all", RuleScope::Extra, m, None, &src, None);
        assert_eq!(mid, "app=shop, env=prod");
        let edited = format!("{mid}, iplist:X");
        let back = switch_consumers(&edited, RuleScope::Intra, m, None, &src, sw.as_ref());
        assert_eq!(back.0, "all, iplist:X");
    }
}
