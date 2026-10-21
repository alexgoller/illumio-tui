//! Intra- and extra-scope allow rules (`unscoped_consumers` on the wire):
//! what a rule's consumers mean against its ruleset's scopes, the classic
//! mistakes worth a warning, and how switching between the two rewrites
//! the consumers. Pure: actor tokens in (the grammar of [`crate::tokens`]),
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

/// What a consumer token is to the scope.
#[derive(Debug, PartialEq, Eq)]
enum Consumer {
    /// A label (not excluded): it pins its key.
    Label(String, String),
    /// An IP list or a workload: no scope narrows it.
    Unscoped,
    /// Anything else: `all`, a label group, an exclusion, a label href.
    Other,
}

fn classify(token: &str) -> Consumer {
    match ActorToken::parse(token) {
        Ok(ActorToken::Label {
            spec,
            exclusion: false,
        }) if !is_href(&spec) => {
            split_kv(&spec).map_or(Consumer::Other, |(k, v)| Consumer::Label(k, v))
        }
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

/// A comma list as its tokens, trimmed, blanks dropped.
fn tokens(text: &str) -> Vec<&str> {
    text.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect()
}

/// The labels of `tokens` as `(key, value)`.
fn labels(tokens: &[&str]) -> Vec<(String, String)> {
    tokens
        .iter()
        .filter_map(|t| match classify(t) {
            Consumer::Label(k, v) => Some((k, v)),
            _ => None,
        })
        .collect()
}

/// A ruleset that applies everywhere: no scope, or an empty one.
fn global(scopes: &[Vec<String>]) -> bool {
    scopes.is_empty() || scopes.iter().any(Vec::is_empty)
}

/// Consumers made only of IP lists and workloads: the scope does not apply.
fn only_unscoped(tokens: &[&str]) -> bool {
    !tokens.is_empty() && tokens.iter().all(|t| classify(t) == Consumer::Unscoped)
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
/// app=shop env=prod`; extra → `role=web in any app, env`. `scopes` are the
/// ruleset's scopes as actor tokens (`None` while not loaded; empty: a
/// global ruleset); `keys` the label model's scope keys.
pub fn meaning(
    consumers: &str,
    scope: RuleScope,
    keys: &[String],
    scopes: Option<&[Vec<String>]>,
) -> String {
    let toks = tokens(consumers);
    if toks.is_empty() {
        return format!("{}: no consumers yet", scope.wording());
    }
    let c = toks.join(", ");
    match scope {
        RuleScope::Intra => match scopes {
            None => format!("{c} inside the ruleset scope"),
            Some(s) if global(s) => format!("{c} anywhere: global ruleset"),
            Some(s) => format!("{c} inside {}", scopes_phrase(s)),
        },
        RuleScope::Extra if only_unscoped(&toks) => format!("{c} wherever they are"),
        RuleScope::Extra => {
            let pinned = labels(&toks);
            let free: Vec<&str> = keys
                .iter()
                .filter(|k| !pinned.iter().any(|(p, _)| p == *k))
                .map(String::as_str)
                .collect();
            if free.is_empty() {
                format!("{c} only: the consumers pin every scope key")
            } else {
                format!("{c} in any {}", free.join(", "))
            }
        }
    }
}

/// The mistakes worth a word before the rule is written: an extra-scope
/// rule whose consumers pin no scope key (`role=web` in every app), an
/// intra-scope consumer label no ruleset scope can hold, and intra-scope
/// in a global ruleset. Never blocking.
pub fn warnings(
    consumers: &str,
    scope: RuleScope,
    keys: &[String],
    scopes: Option<&[Vec<String>]>,
) -> Vec<String> {
    let toks = tokens(consumers);
    if toks.is_empty() {
        return Vec::new();
    }
    let pinned = labels(&toks);
    match scope {
        RuleScope::Extra => {
            let pins_one = pinned.iter().any(|(k, _)| keys.contains(k));
            if pins_one || only_unscoped(&toks) || keys.is_empty() {
                return Vec::new();
            }
            vec![format!(
                "matches {} in every {}",
                toks.join(", "),
                keys.join(", ")
            )]
        }
        RuleScope::Intra => match scopes {
            None => Vec::new(),
            Some(s) if global(s) => {
                vec!["intra-scope in a global ruleset covers every workload".to_string()]
            }
            Some(s) => {
                let scope_labels: Vec<Vec<(String, String)>> = s
                    .iter()
                    .map(|scope| labels(&scope.iter().map(String::as_str).collect::<Vec<_>>()))
                    .collect();
                // Outside every scope: each one holds another value of the key.
                pinned
                    .iter()
                    .filter(|(k, v)| {
                        scope_labels
                            .iter()
                            .all(|sl| sl.iter().any(|(sk, sv)| sk == k && sv != v))
                    })
                    .map(|(k, v)| format!("never matches: {k}={v} is outside the ruleset scope"))
                    .collect()
            }
        },
    }
}

/// The Consumers text after the Scope switched to `to`. To extra: the
/// source's scope labels (`source`, tokens; a `w` form's flow source) in
/// front, for each key the consumers do not pin, so the rule does not open
/// to every scope; only when the consumers hold a label. To intra: the
/// labels on a scope key go (an intra-scope rule takes its scope from the
/// ruleset). Other tokens (IP lists, workloads, groups, exclusions, `all`)
/// are never touched; text with nothing to change comes back as it was.
pub fn rewrite_consumers(
    consumers: &str,
    to: RuleScope,
    keys: &[String],
    source: &[String],
) -> String {
    let toks = tokens(consumers);
    let pinned = labels(&toks);
    let out: Vec<String> = match to {
        RuleScope::Extra => {
            if pinned.is_empty() {
                return consumers.to_string();
            }
            let add: Vec<&String> = source
                .iter()
                .filter(|s| match classify(s) {
                    Consumer::Label(k, _) => {
                        keys.contains(&k) && !pinned.iter().any(|(p, _)| *p == k)
                    }
                    _ => false,
                })
                .collect();
            if add.is_empty() {
                return consumers.to_string();
            }
            add.into_iter()
                .cloned()
                .chain(toks.iter().map(|t| t.to_string()))
                .collect()
        }
        RuleScope::Intra => {
            let kept: Vec<String> = toks
                .iter()
                .filter(|t| !matches!(classify(t), Consumer::Label(k, _) if keys.contains(&k)))
                .map(|t| t.to_string())
                .collect();
            if kept.len() == toks.len() {
                return consumers.to_string();
            }
            kept
        }
    };
    out.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Vec<String> {
        ["app", "env", "loc"].map(String::from).to_vec()
    }

    fn scopes(s: &[&[&str]]) -> Vec<Vec<String>> {
        s.iter()
            .map(|s| s.iter().map(|t| t.to_string()).collect())
            .collect()
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
    fn intra_reads_inside_the_rulesets_scopes() {
        let shop = scopes(&[&["app=shop", "env=prod"]]);
        let m = |s: Option<&[Vec<String>]>| meaning("role=web", RuleScope::Intra, &keys(), s);
        assert_eq!(m(Some(&shop)), "role=web inside app=shop env=prod");
        assert_eq!(m(None), "role=web inside the ruleset scope");
        assert_eq!(m(Some(&[])), "role=web anywhere: global ruleset");
        assert_eq!(
            m(Some(&scopes(&[&[]]))),
            "role=web anywhere: global ruleset"
        );
        let many = scopes(&[
            &["app=shop", "env=prod"],
            &["app=crm"],
            &["group:G"],
            &["app=erp"],
        ]);
        assert_eq!(
            m(Some(&many)),
            "role=web inside app=shop env=prod | app=crm | group:G | +1 more"
        );
        assert_eq!(
            meaning(" ", RuleScope::Intra, &keys(), None),
            "intra-scope: no consumers yet"
        );
    }

    #[test]
    fn extra_reads_in_any_scope_key_the_consumers_leave_open() {
        let m = |c: &str| meaning(c, RuleScope::Extra, &keys(), None);
        assert_eq!(m("role=web"), "role=web in any app, env, loc");
        assert_eq!(
            m("app=shop, role=web"),
            "app=shop, role=web in any env, loc"
        );
        assert_eq!(
            m("app=shop,env=prod,loc=fra,role=web"),
            "app=shop, env=prod, loc=fra, role=web only: the consumers pin every scope key"
        );
        assert_eq!(
            m("iplist:Corp, workload:db1"),
            "iplist:Corp, workload:db1 wherever they are"
        );
        assert_eq!(
            m("!app=dev, role=web"),
            "!app=dev, role=web in any app, env, loc"
        );
    }

    #[test]
    fn extra_without_a_scope_label_warns_that_it_matches_everywhere() {
        let w = |c: &str| warnings(c, RuleScope::Extra, &keys(), None);
        assert_eq!(
            w("role=web"),
            vec!["matches role=web in every app, env, loc"]
        );
        assert_eq!(w("all"), vec!["matches all in every app, env, loc"]);
        assert_eq!(
            w("group:Web"),
            vec!["matches group:Web in every app, env, loc"]
        );
        assert!(w("app=shop, role=web").is_empty(), "one scope key pinned");
        assert!(w("iplist:Corp").is_empty(), "IP lists know no scope");
        assert!(w("iplist:Corp, workload:db1").is_empty());
        assert_eq!(
            w("iplist:Corp, role=web"),
            vec!["matches iplist:Corp, role=web in every app, env, loc"]
        );
        assert!(w("").is_empty());
    }

    #[test]
    fn intra_warns_of_labels_outside_every_scope_and_of_a_global_ruleset() {
        let shop = scopes(&[&["app=shop", "env=prod"], &["app=shop", "env=dev"]]);
        let w = |c: &str, s: Option<&[Vec<String>]>| warnings(c, RuleScope::Intra, &keys(), s);
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
            w("app=crm", Some(&scopes(&[&["group:Apps"]]))).is_empty(),
            "a group may hold it"
        );
        assert_eq!(
            w("role=web", Some(&[])),
            vec!["intra-scope in a global ruleset covers every workload"]
        );
        assert!(w("app=crm", None).is_empty(), "scopes not loaded");
    }

    #[test]
    fn switching_to_extra_adds_the_sources_scope_labels() {
        let src: Vec<String> = ["app=shop", "env=prod"].map(String::from).to_vec();
        let to = |c: &str| rewrite_consumers(c, RuleScope::Extra, &keys(), &src);
        assert_eq!(to("role=web"), "app=shop, env=prod, role=web");
        assert_eq!(
            to("env=dev, role=web"),
            "app=shop, env=dev, role=web",
            "pinned keys stay"
        );
        assert_eq!(to("iplist:Corp"), "iplist:Corp", "no label: nothing added");
        assert_eq!(to("all"), "all");
        assert_eq!(
            rewrite_consumers("role=web", RuleScope::Extra, &keys(), &[]),
            "role=web",
            "no flow source: nothing to add"
        );
    }

    #[test]
    fn switching_to_intra_drops_scope_key_labels_only() {
        let to = |c: &str| rewrite_consumers(c, RuleScope::Intra, &keys(), &[]);
        assert_eq!(to("app=shop, env=prod, role=web"), "role=web");
        assert_eq!(
            to("app=shop, iplist:Corp, !env=dev, group:G, workload:db1"),
            "iplist:Corp, !env=dev, group:G, workload:db1"
        );
        assert_eq!(
            to("role=web,iplist:X"),
            "role=web,iplist:X",
            "unchanged text kept"
        );
        assert_eq!(to("app=shop"), "", "the user chooses what is left");
        // The two directions undo each other on a flow's consumers.
        let src: Vec<String> = ["app=shop", "env=prod"].map(String::from).to_vec();
        let extra = rewrite_consumers("role=web", RuleScope::Extra, &keys(), &src);
        assert_eq!(to(&extra), "role=web");
    }
}
