//! A label value typed without its key (`ordering`) in a filter that
//! takes `key=value` (#88): the Traffic source and destination, the
//! Workloads label filter and the Policy rule search. Only a term the
//! grammar rejects is looked up, case-insensitively, among the label
//! values already loaded (the name index); never a request. One label
//! with that value: the term becomes its `key=value`, so the text shows
//! the format. Several: which one is asked. None: the grammar's own
//! error stays, with similar labels after it.
use crate::tokens::{similar, with_similar};
use pce::NameIndex;
use std::ops::Range;

/// The filter text a term is part of, which decides how terms are cut
/// and which of them the grammar already takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grammar {
    /// Traffic source or destination: `a,b | c, !d`.
    Traffic,
    /// The Workloads label filter: terms apart by spaces, `!` negates.
    Labels,
    /// The Policy rule search: words apart by spaces, `-` negates (and
    /// `!` before a label).
    Rules,
}

/// Every live label as `(key, value)`, keys in the index's order.
pub fn label_pairs(idx: Option<&NameIndex>) -> Vec<(String, String)> {
    let Some(idx) = idx else {
        return Vec::new();
    };
    idx.label_keys()
        .into_iter()
        .flat_map(|k| {
            idx.values_for_key(&k)
                .into_iter()
                .map(move |v| (k.clone(), v))
        })
        .collect()
}

/// The text with its bare values rewritten, and those no label has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewritten {
    pub text: String,
    pub unmatched: Vec<String>,
}

impl Rewritten {
    /// The grammar's error `err`, with labels similar to the bare value it
    /// names (up to three) after its first clause, where a cut line keeps
    /// them.
    pub fn explain(&self, err: String, pairs: &[(String, String)]) -> String {
        let Some(term) = self
            .unmatched
            .iter()
            .find(|t| err.contains(&format!("{t:?}")))
        else {
            return err;
        };
        let names: Vec<String> = pairs.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let mut like = similar(term, names.iter().map(String::as_str));
        like.truncate(3);
        match err.split_once("; ") {
            Some((head, tail)) if !like.is_empty() => {
                format!("{}; {tail}", with_similar(head.to_string(), &like))
            }
            _ => with_similar(err, &like),
        }
    }
}

/// `text` with its bare values rewritten in place (unless one is asked
/// about), then `apply`d; its error gets the similar labels.
pub fn apply<T>(
    text: &mut String,
    grammar: Grammar,
    pairs: &[(String, String)],
    apply: impl FnOnce(&str) -> Result<T, String>,
) -> Result<T, String> {
    let r = rewrite(text, grammar, pairs)?;
    text.clone_from(&r.text);
    apply(text).map_err(|e| r.explain(e, pairs))
}

/// `text` with each bare value that one loaded label has as `key=value`;
/// an error for a value several keys have.
pub fn rewrite(
    text: &str,
    grammar: Grammar,
    pairs: &[(String, String)],
) -> Result<Rewritten, String> {
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let mut unmatched = Vec::new();
    for span in spans(text, grammar) {
        let term = &text[span.clone()];
        let rest = term.trim_start_matches(|c: char| negation(grammar, c) || c.is_whitespace());
        let body = rest.trim_end();
        if body.is_empty() || !is_bare(body, grammar) {
            continue;
        }
        match lookup(body, pairs)? {
            Some(kv) => {
                let start = span.start + (term.len() - rest.len());
                edits.push((start..start + body.len(), kv));
            }
            None => unmatched.push(body.to_string()),
        }
    }
    // From the end, so the earlier ranges stay where they were.
    let mut out = text.to_string();
    for (range, kv) in edits.into_iter().rev() {
        out.replace_range(range, &kv);
    }
    Ok(Rewritten {
        text: out,
        unmatched,
    })
}

fn negation(grammar: Grammar, c: char) -> bool {
    match grammar {
        Grammar::Traffic | Grammar::Labels => c == '!',
        Grammar::Rules => c == '-' || c == '!',
    }
}

/// A term the grammar rejects, with nothing in it that names its kind
/// (`=`, a `prefix:`, a quote or a list).
fn is_bare(body: &str, grammar: Grammar) -> bool {
    if body.contains(['=', '"', ',']) {
        return false;
    }
    match grammar {
        Grammar::Traffic => !body.contains(':') && pce::traffic::FilterExpr::parse(body).is_err(),
        Grammar::Labels => true,
        Grammar::Rules => !body.contains(':') && pce::RuleFilter::parse(body).is_err(),
    }
}

/// `key=value` of the one label whose value is `value` (any case; of
/// several that differ only in case, the one spelt the same); `None` for
/// no label; an error naming the keys when several have it.
fn lookup(value: &str, pairs: &[(String, String)]) -> Result<Option<String>, String> {
    let hits: Vec<&(String, String)> = pairs
        .iter()
        .filter(|(_, v)| v.to_lowercase() == value.to_lowercase())
        .collect();
    let exact: Vec<&(String, String)> = hits.iter().copied().filter(|(_, v)| v == value).collect();
    let one = match (hits.as_slice(), exact.as_slice()) {
        ([], _) => return Ok(None),
        ([one], _) | (_, [one]) => *one,
        _ => {
            let mut keys: Vec<&str> = Vec::new();
            for (k, _) in &hits {
                if !keys.contains(&k.as_str()) {
                    keys.push(k);
                }
            }
            let options: Vec<String> = hits.iter().map(|(k, v)| format!("{k}={v}")).collect();
            return Err(format!(
                "{value:?} is a value of {}: write {}",
                listed(&keys, "and"),
                listed(&options, "or")
            ));
        }
    };
    Ok(Some(format!("{}={}", one.0, one.1)))
}

/// `a`, `a and b`, `a, b and c`.
fn listed<S: AsRef<str>>(items: &[S], last: &str) -> String {
    match items {
        [] => String::new(),
        [one] => one.as_ref().to_string(),
        [head @ .., tail] => format!(
            "{} {last} {}",
            head.iter()
                .map(AsRef::as_ref)
                .collect::<Vec<_>>()
                .join(", "),
            tail.as_ref()
        ),
    }
}

/// Where the terms of `text` are: between `|` and `,` (Traffic), else
/// between spaces outside `"…"` (a `\` inside quotes escapes).
fn spans(text: &str, grammar: Grammar) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    if grammar == Grammar::Traffic {
        for (i, c) in text.char_indices() {
            if c == '|' || c == ',' {
                out.push(start..i);
                start = i + c.len_utf8();
            }
        }
        out.push(start..text.len());
        return out;
    }
    let (mut quoted, mut escaped) = (false, false);
    let mut begun: Option<usize> = None;
    for (i, c) in text.char_indices() {
        if quoted {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => quoted = false,
                _ => {}
            }
            continue;
        }
        if c.is_whitespace() {
            if let Some(s) = begun.take() {
                out.push(s..i);
            }
            continue;
        }
        begun.get_or_insert(i);
        if c == '"' {
            quoted = true;
        }
    }
    if let Some(s) = begun {
        out.push(s..text.len());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs() -> Vec<(String, String)> {
        [
            ("app", "ordering"),
            ("app", "orderbook"),
            ("env", "staging"),
            ("loc", "staging"),
            ("loc", "New York"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    fn text(t: &str, g: Grammar) -> String {
        rewrite(t, g, &pairs()).unwrap().text
    }

    #[test]
    fn one_label_rewrites_the_value_keeping_the_rest_of_the_text() {
        assert_eq!(text("Ordering", Grammar::Traffic), "app=ordering");
        assert_eq!(
            text(" ordering ,env=prod| ! ORDERING", Grammar::Traffic),
            " app=ordering ,env=prod| ! app=ordering"
        );
        assert_eq!(text("new york", Grammar::Traffic), "loc=New York");
        assert_eq!(
            text("10.0.0.1, ipl:Any, wl:ordering", Grammar::Traffic),
            "10.0.0.1, ipl:Any, wl:ordering",
            "terms the grammar takes stay"
        );
        assert_eq!(
            text("env=prod  !ordering \"x y\"", Grammar::Labels),
            "env=prod  !app=ordering \"x y\""
        );
        assert_eq!(
            text("deny -ordering !ordering src:x", Grammar::Rules),
            "deny -app=ordering !app=ordering src:x"
        );
    }

    #[test]
    fn several_keys_are_asked_and_none_are_left_for_the_grammar() {
        assert_eq!(
            rewrite("staging", Grammar::Labels, &pairs()).unwrap_err(),
            "\"staging\" is a value of env and loc: write env=staging or loc=staging"
        );
        let r = rewrite("order, nope", Grammar::Traffic, &pairs()).unwrap();
        assert_eq!(r.text, "order, nope");
        assert_eq!(r.unmatched, ["order", "nope"]);
        assert_eq!(
            r.explain(
                "cannot parse filter \"order\"; use key=value".into(),
                &pairs()
            ),
            "cannot parse filter \"order\"; similar: app=orderbook, app=ordering; use key=value"
        );
        assert_eq!(
            r.explain("other error".into(), &pairs()),
            "other error",
            "only an error naming the value"
        );
    }

    #[test]
    fn the_same_value_in_two_cases_takes_the_one_spelt_alike() {
        let p: Vec<(String, String)> =
            vec![("app".into(), "Shop".into()), ("app".into(), "shop".into())];
        assert_eq!(
            rewrite("Shop", Grammar::Labels, &p).unwrap().text,
            "app=Shop"
        );
        assert!(rewrite("SHOP", Grammar::Labels, &p).is_err());
    }

    #[test]
    fn lists_read_as_prose() {
        assert_eq!(listed(&["a"], "and"), "a");
        assert_eq!(listed(&["a", "b", "c"], "or"), "a, b or c");
    }
}
