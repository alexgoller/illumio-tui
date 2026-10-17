//! Workload label filters, one grammar for the TUI's `f` on Workloads and
//! the CLI's `workloads --label`:
//!
//! ```text
//! app=shop,crm env=prod !role=db,web loc=- "dc=Frankfurt Main"
//! ```
//!
//! A term is `key=value[,value…]`: a comma lists values any of which
//! matches; terms apart by spaces must all match (AND); a leading `!`
//! excludes what the term matches; the value `-` means "no label for the
//! key" (`app=-`: workloads without an app label). A value with spaces or
//! commas is quoted (`loc="New York"`, or the whole term quoted). Pure: the
//! filter runs over workloads already loaded; [`LabelFilter::query_sets`]
//! is the part the PCE's `labels` query parameter can carry.
use crate::models::Workload;
use std::fmt;

/// The value that stands for "no label for this key".
pub const NO_LABEL: &str = "-";

/// The most label sets one `labels` query may carry (the cross product
/// of the any-of terms); more is refused, never split into requests.
pub const MAX_COMBINATIONS: usize = 50;

/// One term: `key=v1,v2`, `!key=v`, `key=-`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelTerm {
    pub key: String,
    /// The label values, any of which matches (no `-`).
    pub values: Vec<String>,
    /// `-` was listed: a workload without a label for `key` matches too.
    pub none: bool,
    /// `!`: the term keeps what it does not match.
    pub negate: bool,
}

impl LabelTerm {
    /// One term (a CLI `--label` value, or one token of an expression).
    /// Spaces inside the value are kept (`loc=New York`); `"…"` quotes a
    /// value with a comma, or a literal `-`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let t = text.trim();
        let (negate, rest) = match t.strip_prefix('!') {
            Some(r) => (true, r.trim_start()),
            None => (false, t),
        };
        let rest = unquote_whole(rest);
        let Some((key, values)) = rest.split_once('=') else {
            return Err(if rest.is_empty() {
                format!("{t:?}: ! needs key=value")
            } else {
                format!("{t:?}: expected key=value (key=- for no label)")
            });
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(format!("{t:?}: missing key before ="));
        }
        if key.contains(['"', ',', '!']) || key.contains(char::is_whitespace) {
            return Err(format!("{t:?}: bad key {key:?}"));
        }
        let mut term = LabelTerm {
            key: key.to_string(),
            values: Vec::new(),
            none: false,
            negate,
        };
        if values.trim().is_empty() {
            return Err(format!(
                "{t:?}: no value for {key} (use {key}=- for no label)"
            ));
        }
        for piece in split_outside_quotes(values, |c| c == ',')? {
            let raw = piece.trim();
            if raw.is_empty() {
                return Err(format!("{t:?}: empty value in the list"));
            }
            if raw == NO_LABEL {
                term.none = true;
                continue;
            }
            let v = strip_quotes(raw)?;
            if v.is_empty() {
                return Err(format!("{t:?}: empty value in the list"));
            }
            if !term.values.iter().any(|x| x == v) {
                term.values.push(v.to_string());
            }
        }
        Ok(term)
    }

    /// Whether a workload whose label for the key is `value` (`None`: no
    /// label) passes the term.
    pub fn keeps_value(&self, value: Option<&str>) -> bool {
        let hit = match value {
            Some(v) => self.values.iter().any(|x| x == v),
            None => self.none,
        };
        hit != self.negate
    }

    pub fn keeps(&self, w: &Workload) -> bool {
        self.keeps_value(value_of(w, &self.key))
    }

    /// The PCE can carry the term: a positive list of values, no `-`.
    fn for_query(&self) -> bool {
        !self.negate && !self.none
    }
}

/// The workload's value for `key`, if it has a label with that key.
pub fn value_of<'a>(w: &'a Workload, key: &str) -> Option<&'a str> {
    w.labels
        .iter()
        .find(|l| l.key.as_deref() == Some(key))
        .map(|l| l.value.as_deref().unwrap_or(""))
}

/// A workload's group under `keys`: its value for each key, `None` where
/// it has no label for it.
pub fn group_key(w: &Workload, keys: &[String]) -> Vec<Option<String>> {
    keys.iter()
        .map(|k| value_of(w, k).map(str::to_string))
        .collect()
}

/// A group's name: `shop | prod`, `(no app)` for a missing label.
pub fn group_title(key: &[Option<String>], keys: &[String]) -> String {
    key.iter()
        .zip(keys)
        .map(|(v, k)| match v {
            Some(v) => v.clone(),
            None => format!("(no {k})"),
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

/// A group as filter text: `app=shop env=-` (what narrows to it).
pub fn group_filter(key: &[Option<String>], keys: &[String]) -> String {
    LabelFilter::default()
        .narrowed(&group_pairs(key, keys))
        .to_string()
}

/// `(key, value)` per group key, for [`LabelFilter::narrowed`].
pub fn group_pairs(key: &[Option<String>], keys: &[String]) -> Vec<(String, Option<String>)> {
    keys.iter().cloned().zip(key.iter().cloned()).collect()
}

/// Terms that must all match; empty matches every workload.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LabelFilter {
    pub terms: Vec<LabelTerm>,
}

/// Why [`LabelFilter::query_sets`] gave up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryError<E> {
    /// The any-of terms make more label sets than [`MAX_COMBINATIONS`].
    TooMany(usize),
    /// A label did not resolve to an href.
    Unresolved(E),
}

impl LabelFilter {
    /// Terms apart by whitespace (outside quotes).
    pub fn parse(expr: &str) -> Result<Self, String> {
        let terms = split_outside_quotes(expr, char::is_whitespace)?
            .into_iter()
            .filter(|t| !t.trim().is_empty())
            .map(LabelTerm::parse)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { terms })
    }

    /// One term per item (the CLI's repeated `--label`).
    pub fn from_terms<S: AsRef<str>>(items: &[S]) -> Result<Self, String> {
        let terms = items
            .iter()
            .map(|s| LabelTerm::parse(s.as_ref()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { terms })
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    pub fn keeps(&self, w: &Workload) -> bool {
        self.terms.iter().all(|t| t.keeps(w))
    }

    /// The filter with exactly `pairs` for their keys: earlier terms on
    /// those keys are dropped, the others kept (`None`: no label, `key=-`).
    pub fn narrowed(&self, pairs: &[(String, Option<String>)]) -> Self {
        let mut terms: Vec<LabelTerm> = self
            .terms
            .iter()
            .filter(|t| !pairs.iter().any(|(k, _)| *k == t.key))
            .cloned()
            .collect();
        terms.extend(pairs.iter().map(|(k, v)| LabelTerm {
            key: k.clone(),
            values: v.iter().cloned().collect(),
            none: v.is_none(),
            negate: false,
        }));
        Self { terms }
    }

    /// Some term the PCE cannot carry (`!`, `-`): the whole filter must
    /// run over what comes back.
    pub fn needs_local(&self) -> bool {
        self.terms.iter().any(|t| !t.for_query())
    }

    /// The PCE's `labels` parameter for the positive terms without `-`:
    /// the outer list is OR, each inner list AND, so `app=shop,crm
    /// env=prod` is `[[shop,prod],[crm,prod]]` (each label resolved to its
    /// href by `resolve`). `None` when no term goes into the query. More
    /// than [`MAX_COMBINATIONS`] sets is an error, checked before resolving.
    pub fn query_sets<E>(
        &self,
        mut resolve: impl FnMut(&str, &str) -> Result<String, E>,
    ) -> Result<Option<Vec<Vec<String>>>, QueryError<E>> {
        let dims: Vec<&LabelTerm> = self.terms.iter().filter(|t| t.for_query()).collect();
        if dims.is_empty() {
            return Ok(None);
        }
        let total = dims
            .iter()
            .try_fold(1usize, |n, t| n.checked_mul(t.values.len()))
            .unwrap_or(usize::MAX);
        if total > MAX_COMBINATIONS {
            return Err(QueryError::TooMany(total));
        }
        let mut sets: Vec<Vec<String>> = vec![Vec::new()];
        for t in dims {
            let hrefs = t
                .values
                .iter()
                .map(|v| resolve(&t.key, v))
                .collect::<Result<Vec<_>, _>>()
                .map_err(QueryError::Unresolved)?;
            sets = sets
                .into_iter()
                .flat_map(|s| {
                    hrefs.iter().map(move |h| {
                        let mut s = s.clone();
                        s.push(h.clone());
                        s
                    })
                })
                .collect();
        }
        Ok(Some(sets))
    }

    /// The keys and `key=value` pairs no label in `known` has, as warnings
    /// (`unknown key os`, `unknown value app=shpo`): such a term matches
    /// nothing (or, with `!`, everything). `known(key)` lists a key's
    /// values, `None` for a key no label has.
    pub fn unknown(&self, known: impl Fn(&str) -> Option<Vec<String>>) -> Vec<String> {
        let mut out = Vec::new();
        for t in &self.terms {
            match known(&t.key) {
                None => out.push(format!("unknown key {}", t.key)),
                Some(vals) => {
                    for v in t.values.iter().filter(|v| !vals.contains(v)) {
                        out.push(format!("unknown value {}={}", t.key, quote(v)));
                    }
                }
            }
        }
        out.dedup();
        out
    }
}

impl fmt::Display for LabelTerm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.negate {
            f.write_str("!")?;
        }
        write!(f, "{}=", self.key)?;
        let mut parts: Vec<String> = self.values.iter().map(|v| quote(v)).collect();
        if self.none {
            parts.push(NO_LABEL.to_string());
        }
        f.write_str(&parts.join(","))
    }
}

impl fmt::Display for LabelFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let terms: Vec<String> = self.terms.iter().map(ToString::to_string).collect();
        f.write_str(&terms.join(" "))
    }
}

/// A value as the grammar reads it back: quoted when it has a space, a
/// comma, a quote or is the literal `-`.
pub fn quote(v: &str) -> String {
    if v == NO_LABEL || v.contains([',', '"']) || v.contains(char::is_whitespace) {
        format!("\"{}\"", v.replace('"', ""))
    } else {
        v.to_string()
    }
}

/// `"app=New York"` (a whole quoted term) without its quotes.
fn unquote_whole(s: &str) -> &str {
    match s.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        Some(inner) if !inner.contains('"') && inner.contains('=') => inner,
        _ => s,
    }
}

/// `"x"` → `x`; a stray quote is an error.
fn strip_quotes(s: &str) -> Result<&str, String> {
    match s.strip_prefix('"') {
        Some(r) => r
            .strip_suffix('"')
            .filter(|i| !i.contains('"'))
            .ok_or_else(|| format!("unbalanced quote in {s:?}")),
        None if s.contains('"') => Err(format!("unbalanced quote in {s:?}")),
        None => Ok(s),
    }
}

/// `s` split where `sep` matches outside double quotes; an unclosed quote
/// is an error.
fn split_outside_quotes(s: &str, sep: impl Fn(char) -> bool) -> Result<Vec<&str>, String> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    for (i, c) in s.char_indices() {
        if c == '"' {
            quoted = !quoted;
        } else if !quoted && sep(c) {
            out.push(&s[start..i]);
            start = i + c.len_utf8();
        }
    }
    if quoted {
        return Err(format!("unclosed quote in {s:?}"));
    }
    out.push(&s[start..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wl(labels: &[(&str, &str)]) -> Workload {
        let labels: Vec<serde_json::Value> = labels
            .iter()
            .map(|(k, v)| serde_json::json!({"href": format!("/l/{k}/{v}"), "key": k, "value": v}))
            .collect();
        serde_json::from_value(serde_json::json!({"href": "/w/1", "labels": labels})).unwrap()
    }

    #[test]
    fn parses_any_of_and_negation_and_no_label() {
        let f = LabelFilter::parse("app=shop,crm  env=prod !role=db,web loc=-").unwrap();
        assert_eq!(f.terms.len(), 4);
        assert_eq!(f.terms[0].values, ["shop", "crm"]);
        assert!(!f.terms[0].negate && !f.terms[0].none);
        assert!(f.terms[2].negate);
        assert_eq!(f.terms[2].values, ["db", "web"]);
        assert!(f.terms[3].none && f.terms[3].values.is_empty());
        assert_eq!(
            f.to_string(),
            "app=shop,crm env=prod !role=db,web loc=-",
            "round trip"
        );
        assert_eq!(LabelFilter::parse(&f.to_string()).unwrap(), f);
        assert!(LabelFilter::parse("  ").unwrap().is_empty());
        // Repeats once; `-` mixes with values.
        let t = LabelTerm::parse("app=shop,shop,-").unwrap();
        assert_eq!((t.values.len(), t.none), (1, true));
    }

    #[test]
    fn quotes_keep_spaces_commas_and_a_literal_dash() {
        let f = LabelFilter::parse(r#"loc="New York","a,b" "dc=Frankfurt Main" x="-""#).unwrap();
        assert_eq!(f.terms[0].values, ["New York", "a,b"]);
        assert_eq!(f.terms[1].values, ["Frankfurt Main"]);
        assert_eq!(f.terms[2].values, ["-"]);
        assert!(!f.terms[2].none);
        assert_eq!(LabelFilter::parse(&f.to_string()).unwrap(), f);
        // A CLI `--label` keeps an unquoted space.
        assert_eq!(
            LabelTerm::parse("loc=New York").unwrap().values,
            ["New York"]
        );
    }

    #[test]
    fn malformed_terms_are_errors() {
        for bad in [
            "app",
            "=shop",
            "app=",
            "app=shop,,crm",
            "app=shop,",
            "!",
            "! ",
            "app=\"shop",
            "a pp=x",
            "app=sh\"op",
        ] {
            assert!(LabelFilter::parse(bad).is_err(), "{bad:?}");
        }
        assert!(LabelTerm::parse("app").unwrap_err().contains("key=value"));
        assert!(LabelTerm::parse("app=").unwrap_err().contains("app=-"));
    }

    #[test]
    fn keeps_follows_any_of_and_not_and_absence() {
        let shop = wl(&[("app", "shop"), ("env", "prod"), ("role", "web")]);
        let crm = wl(&[("app", "crm"), ("env", "dev"), ("role", "db")]);
        let bare = wl(&[("env", "prod")]);
        let f = |e: &str| LabelFilter::parse(e).unwrap();
        let kept = |e: &str| {
            [&shop, &crm, &bare]
                .iter()
                .map(|w| f(e).keeps(w))
                .collect::<Vec<_>>()
        };
        assert_eq!(kept("app=shop,crm"), [true, true, false]);
        assert_eq!(kept("app=shop,crm env=prod"), [true, false, false]);
        assert_eq!(kept("!role=db"), [true, false, true]);
        assert_eq!(kept("!role=db,web"), [false, false, true]);
        assert_eq!(kept("app=-"), [false, false, true]);
        assert_eq!(kept("!app=-"), [true, true, false]);
        assert_eq!(kept("app=shop,-"), [true, false, true]);
        assert_eq!(kept("os=linux"), [false, false, false]);
        assert_eq!(kept(""), [true, true, true]);
    }

    #[test]
    fn query_sets_are_the_cross_product_of_the_positive_terms() {
        let r = |k: &str, v: &str| -> Result<String, String> { Ok(format!("{k}:{v}")) };
        let f = LabelFilter::parse("app=shop,crm env=prod !role=db loc=-").unwrap();
        assert_eq!(
            f.query_sets(r).unwrap().unwrap(),
            vec![vec!["app:shop", "env:prod"], vec!["app:crm", "env:prod"]]
        );
        assert!(f.needs_local());
        // Plain terms: one AND set, as `--label a --label b` always sent.
        let plain = LabelFilter::from_terms(&["app=shop", "env=prod"]).unwrap();
        assert_eq!(
            plain.query_sets(r).unwrap().unwrap(),
            vec![vec!["app:shop", "env:prod"]]
        );
        assert!(!plain.needs_local());
        assert_eq!(
            LabelFilter::parse("!app=x").unwrap().query_sets(r).unwrap(),
            None
        );
        // 8 x 7 = 56 sets: refused before any lookup.
        let many = LabelFilter::parse("a=1,2,3,4,5,6,7,8 b=1,2,3,4,5,6,7").unwrap();
        let mut calls = 0;
        let res = many.query_sets(|_, _| -> Result<String, String> {
            calls += 1;
            Ok(String::new())
        });
        assert_eq!(res, Err(QueryError::TooMany(56)));
        assert_eq!(calls, 0);
        let ok = LabelFilter::parse("a=1,2,3,4,5 b=1,2,3,4,5,6,7,8,9,10").unwrap();
        assert_eq!(ok.query_sets(r).unwrap().unwrap().len(), 50);
        let err = plain.query_sets(|k, _| -> Result<String, String> { Err(k.to_string()) });
        assert_eq!(err, Err(QueryError::Unresolved("app".into())));
    }

    #[test]
    fn narrowing_replaces_the_terms_of_its_keys() {
        let f = LabelFilter::parse("app=shop,crm !role=db").unwrap();
        let n = f.narrowed(&[("app".into(), Some("crm".into())), ("env".into(), None)]);
        assert_eq!(n.to_string(), "!role=db app=crm env=-");
    }

    #[test]
    fn groups_name_missing_labels() {
        let keys = vec!["app".to_string(), "env".to_string()];
        let k = group_key(&wl(&[("env", "prod"), ("role", "db")]), &keys);
        assert_eq!(k, [None, Some("prod".to_string())]);
        assert_eq!(group_title(&k, &keys), "(no app) | prod");
        assert_eq!(group_filter(&k, &keys), "app=- env=prod");
        let f = LabelFilter::parse(&group_filter(&k, &keys)).unwrap();
        assert!(f.keeps(&wl(&[("env", "prod")])));
        assert!(!f.keeps(&wl(&[("env", "prod"), ("app", "x")])));
    }

    #[test]
    fn unknown_keys_and_values_are_named() {
        let f = LabelFilter::parse("app=shop,shpo os=linux !role=db").unwrap();
        let known = |k: &str| match k {
            "app" => Some(vec!["shop".to_string()]),
            "role" => Some(vec!["db".to_string()]),
            _ => None,
        };
        assert_eq!(
            f.unknown(known),
            ["unknown value app=shpo", "unknown key os"]
        );
    }
}
