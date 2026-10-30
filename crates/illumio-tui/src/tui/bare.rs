//! A label value typed without its key (`ordering`) in a filter that
//! takes `key=value` (#88): the Traffic source and destination, the
//! Workloads label filter and the Policy rule search. Only a term the
//! grammar rejects is looked up, case-insensitively, among the label
//! values already loaded (the name index); never a request. One label
//! with that value: the term becomes its `key=value`, so the text shows
//! the format. Several: which one is asked. None: the grammar's own
//! error stays, with similar labels after it.
use crate::tokens::with_similar;
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
        let value = term
            .strip_prefix('"')
            .and_then(|t| t.strip_suffix('"'))
            .unwrap_or(term);
        let like = similar_labels(value, pairs);
        match err.split_once("; ") {
            Some((head, tail)) if !like.is_empty() => {
                format!("{}; {tail}", with_similar(head.to_string(), &like))
            }
            _ => with_similar(err, &like),
        }
    }
}

/// Up to three labels whose value is like `typed`, any case: those it
/// starts, then those holding it, then those a typo or two away (one for
/// a short word), each group by `key=value`.
fn similar_labels(typed: &str, pairs: &[(String, String)]) -> Vec<String> {
    let typed = typed.to_lowercase();
    let typos = if typed.chars().count() <= 4 { 1 } else { 2 };
    let mut ranked: Vec<(u8, String)> = pairs
        .iter()
        .filter_map(|(k, v)| {
            let value = v.to_lowercase();
            let rank = if value.starts_with(&typed) {
                0
            } else if value.contains(&typed) {
                1
            } else if distance(&value, &typed) <= typos {
                2
            } else {
                return None;
            };
            Some((rank, format!("{k}={v}")))
        })
        .collect();
    ranked.sort();
    ranked.dedup();
    ranked.into_iter().take(3).map(|(_, kv)| kv).collect()
}

/// The edit distance (insert, delete, replace one character) of `a` and `b`.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diag = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let next = (diag + usize::from(ca != *cb))
                .min(row[j] + 1)
                .min(row[j + 1] + 1);
            diag = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
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
        let quoted = quoted_value(body, grammar);
        if body.is_empty() || (quoted.is_none() && !is_bare(body, grammar)) {
            continue;
        }
        let found = match quoted {
            Some(inner) => lookup(inner, pairs)?
                .map(|(k, v)| format!("{k}=\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))),
            None => lookup(body, pairs)?.map(|(k, v)| format!("{k}={v}")),
        };
        match found {
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

/// The value of a wholly quoted term (`"New York"`) where quotes are
/// grammar (not Traffic), with no `=`, `,`, quote or `\\` inside: such a
/// term is a bare value too, written back quoted.
fn quoted_value(body: &str, grammar: Grammar) -> Option<&str> {
    if grammar == Grammar::Traffic {
        return None;
    }
    let inner = body.strip_prefix('"')?.strip_suffix('"')?;
    (!inner.trim().is_empty() && !inner.contains(['=', ',', '"', '\\'])).then_some(inner)
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

/// The one label whose value is `value` (any case; of several under one
/// key that differ only in case, the one spelt the same); `None` for no
/// label; an error naming the keys when several keys have it.
fn lookup(value: &str, pairs: &[(String, String)]) -> Result<Option<(String, String)>, String> {
    let hits: Vec<&(String, String)> = pairs
        .iter()
        .filter(|(_, v)| v.to_lowercase() == value.to_lowercase())
        .collect();
    let mut keys: Vec<&str> = Vec::new();
    for (k, _) in &hits {
        if !keys.contains(&k.as_str()) {
            keys.push(k);
        }
    }
    let exact: Vec<&(String, String)> = hits.iter().copied().filter(|(_, v)| v == value).collect();
    let one = match (hits.as_slice(), exact.as_slice()) {
        ([], _) => return Ok(None),
        ([one], _) => *one,
        (_, [one]) if keys.len() == 1 => *one,
        _ => {
            let options: Vec<String> = hits.iter().map(|(k, v)| format!("{k}={v}")).collect();
            return Err(format!(
                "{value:?} is a value of {}: write {}",
                listed(&keys, "and"),
                listed(&options, "or")
            ));
        }
    };
    Ok(Some(one.clone()))
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
    fn similar_matches_the_value_by_prefix_part_or_typo_any_case() {
        let r = |t: &str| Rewritten {
            text: String::new(),
            unmatched: vec![t.to_string()],
        };
        let similar = |t: &str| r(t).explain(format!("{t:?}: bad; use key=value"), &pairs());
        assert_eq!(
            similar("Orderng"),
            "\"Orderng\": bad; similar: app=ordering; use key=value"
        );
        assert_eq!(
            similar("Order"),
            "\"Order\": bad; similar: app=orderbook, app=ordering; use key=value",
            "prefixes first, by name"
        );
        assert_eq!(
            similar("STAGNG"),
            "\"STAGNG\": bad; similar: env=staging, loc=staging; use key=value"
        );
        assert_eq!(
            similar("app"),
            "\"app\": bad; use key=value",
            "keys are not values"
        );
        assert_eq!(similar("xyz"), "\"xyz\": bad; use key=value");
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
    fn a_value_spelt_alike_under_another_key_is_still_asked_about() {
        let p: Vec<(String, String)> = vec![
            ("app".into(), "Ordering".into()),
            ("env".into(), "ordering".into()),
        ];
        for typed in ["ordering", "Ordering"] {
            let err = rewrite(typed, Grammar::Traffic, &p).unwrap_err();
            assert_eq!(
                err,
                format!("{typed:?} is a value of app and env: write app=Ordering or env=ordering")
            );
        }
    }

    #[test]
    fn a_wholly_quoted_value_resolves_and_stays_quoted() {
        assert_eq!(
            text("env=prod \"new york\" !\"New York\"", Grammar::Labels),
            "env=prod loc=\"New York\" !loc=\"New York\""
        );
        assert_eq!(text("-\"new york\"", Grammar::Rules), "-loc=\"New York\"");
        let r = rewrite("\"new yrk\"", Grammar::Labels, &pairs()).unwrap();
        assert_eq!(r.text, "\"new yrk\"");
        assert_eq!(
            r.explain(
                "\"\\\"new yrk\\\"\": expected key=value (key=- for no label)".into(),
                &pairs()
            ),
            "\"\\\"new yrk\\\"\": expected key=value (key=- for no label); similar: loc=New York"
        );
        assert_eq!(
            text("\"a=b\" \"x,y\"", Grammar::Labels),
            "\"a=b\" \"x,y\"",
            "a quoted term with = or , is the grammar's"
        );
    }

    #[test]
    fn lists_read_as_prose() {
        assert_eq!(listed(&["a"], "and"), "a");
        assert_eq!(listed(&["a", "b", "c"], "or"), "a, b or c");
    }
}
