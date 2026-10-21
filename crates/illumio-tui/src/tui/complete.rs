//! Candidates for `Ctrl-N` in a form or traffic filter field, from what the
//! TUI already holds: the name index, the loaded draft rulesets and the
//! workloads the Workloads screen loaded. Pure; never a request.
//!
//! Matching is case-insensitive, candidates keep their real case. Prefix
//! matches come before substring matches; within each, index order (label
//! keys in the index's `KeyOrder`). A bare label token also matches values:
//! `pay` offers `app=payment` and `role=payroll`, after any key that matches.
use crate::tokens::is_href;
use crate::tui::form::FieldKind;
use pce::NameIndex;
use pce::models::{Event, RuleSet, Workload};
use pce::traffic::{WINDOW_STEPS, next_window, parse_window};

/// Most candidates one completion cycles through.
pub const MAX_CANDIDATES: usize = 200;

/// The key the index gives a label it cannot resolve; never offered.
const UNRESOLVED: &str = "?";

/// The candidates that replace `token` in a field of `kind`, best first.
/// `earlier` are the field's tokens before it (for `Labels` and `Scope`
/// their keys are not offered again, for `Decisions` their words); for
/// `LabelValue` its first entry is the key whose values are offered.
/// `workloads` are those already loaded (empty when none are).
pub fn complete(
    kind: FieldKind,
    token: &str,
    earlier: &[String],
    idx: &NameIndex,
    rulesets: &[RuleSet],
    workloads: &[Workload],
) -> Vec<String> {
    let mut out = match kind {
        FieldKind::Text | FieldKind::RuleScope => Vec::new(),
        FieldKind::LabelKey => ranked(token, keys(idx, &[]).into_iter().map(|k| (k.clone(), k))),
        FieldKind::LabelValue => {
            let key = earlier.first().map_or("", |k| k.trim());
            ranked(
                token,
                pairs(idx, &matching_keys(idx, key, &[])).map(|(_, v)| (v.clone(), v)),
            )
        }
        FieldKind::Labels => labels(token, &used_keys(earlier), idx, &[], &[]),
        FieldKind::Scope => negatable(token, |t| {
            let groups = [("group:", idx.label_group_names())];
            prefixed(t, &groups).unwrap_or_else(|| {
                labels(
                    t,
                    &used_keys(earlier),
                    idx,
                    &["group:"],
                    &named_all(&groups),
                )
            })
        }),
        FieldKind::Actors => negatable(token, |t| actors(t, idx, token.starts_with('!'))),
        FieldKind::Services => services(token, idx),
        FieldKind::Ruleset => {
            let names = rulesets
                .iter()
                .filter(|r| r.update_type.as_deref() != Some("delete"))
                .map(|r| (r.name.clone(), r.name.clone()));
            ranked(token, names)
        }
        FieldKind::TrafficActors | FieldKind::TrafficSource => {
            let source = kind == FieldKind::TrafficSource;
            // A `!` term excludes from the whole field, not the AND group:
            // it neither hides the group's keys nor is hidden by them.
            let used = if token.starts_with('!') {
                Vec::new()
            } else {
                let included: Vec<String> = earlier
                    .iter()
                    .filter(|t| !t.trim_start().starts_with('!'))
                    .cloned()
                    .collect();
                used_keys(&included)
            };
            negatable(token, |t| filter_actors(t, &used, idx, workloads, source))
        }
        FieldKind::TrafficService => negatable(token, |t| traffic_service(t, idx)),
        FieldKind::Decisions => word_list(token, earlier, &DECISIONS),
        FieldKind::Transmission => word_list(
            token,
            earlier,
            &["all", "unicast", "broadcast", "multicast"],
        ),
        FieldKind::Window => window_steps(token),
        FieldKind::MarkAction => cycle_words(token, &pce::marks::Action::ALL.map(|a| a.as_str())),
        FieldKind::MarkExpiry => cycle_words(token, &["1d", "7d", "30d", "90d", "never"]),
        FieldKind::MarkScope => cycle_words(token, &[THIS_PCE, ANY_PCE]),
        FieldKind::MarkSide => mark_side(token, &used_keys(earlier), idx, workloads),
        FieldKind::WorkloadLabels => {
            negatable(token, |t| workload_label_term(t, &used_keys(earlier), idx))
        }
        FieldKind::RuleFilter => rule_filter_term(token, idx, workloads),
        // From the loaded events, not the index: see `event_types`.
        FieldKind::EventTypes => Vec::new(),
    };
    let mut seen = std::collections::HashSet::new();
    out.retain(|c| seen.insert(c.clone()));
    out.truncate(MAX_CANDIDATES);
    out
}

/// The Events type filter's candidates: the event types of the loaded
/// `events`, most frequent first, those named by `earlier` terms (with or
/// without `!` and `=`) left out. A typed `!`, `=` or `!=` prefix stays on
/// each candidate. Pure.
pub fn event_types(token: &str, earlier: &[String], events: &[Event]) -> Vec<String> {
    let used: Vec<String> = earlier
        .iter()
        .filter_map(|t| pce::models::TermText::parse(t.trim()).ok())
        .map(|p| p.body.to_lowercase())
        .collect();
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for ty in events.iter().filter_map(|e| e.event_type.as_deref()) {
        if ty.is_empty() || used.contains(&ty.to_lowercase()) {
            continue;
        }
        match counts.iter_mut().find(|(t, _)| *t == ty) {
            Some((_, n)) => *n += 1,
            None => counts.push((ty, 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    // A typed `!`, `=` or `!=` (blanks after each allowed) stays on
    // every candidate, written tight.
    let (neg, rest) = match token.strip_prefix('!') {
        Some(r) => ("!", r.trim_start()),
        None => ("", token),
    };
    let (exact, rest) = match rest.strip_prefix('=') {
        Some(r) => ("=", r.trim_start()),
        None => ("", rest),
    };
    let mut out: Vec<String> = ranked(
        rest,
        counts
            .iter()
            .map(|(ty, _)| (ty.to_string(), ty.to_string())),
    )
    .into_iter()
    .map(|c| format!("{neg}{exact}{c}"))
    .collect();
    out.truncate(MAX_CANDIDATES);
    out
}

/// The words of a comma list not listed yet; `all` stands alone: first or
/// not at all.
fn word_list(token: &str, earlier: &[String], words: &[&str]) -> Vec<String> {
    let used: Vec<String> = earlier.iter().map(|e| e.trim().to_lowercase()).collect();
    if used.iter().any(|u| u == "all") {
        return Vec::new();
    }
    let words = words
        .iter()
        .filter(|d| !used.iter().any(|u| u == *d) && (used.is_empty() || **d != "all"))
        .map(|d| (d.to_string(), d.to_string()));
    ranked(token, words)
}

/// `(text matched, candidate)` pairs whose text starts with `needle`, then
/// those that contain it elsewhere; case-insensitive, order kept.
fn ranked(needle: &str, items: impl IntoIterator<Item = (String, String)>) -> Vec<String> {
    let needle = needle.to_lowercase();
    let (mut prefix, mut inner) = (Vec::new(), Vec::new());
    for (text, cand) in items {
        let text = text.to_lowercase();
        if text.starts_with(&needle) {
            prefix.push(cand);
        } else if text.contains(&needle) {
            inner.push(cand);
        }
    }
    prefix.extend(inner);
    prefix
}

/// Index keys in `KeyOrder`, `?` and `used` (lowercase) left out.
fn keys(idx: &NameIndex, used: &[String]) -> Vec<String> {
    idx.label_keys()
        .into_iter()
        .filter(|k| k != UNRESOLVED && !used.contains(&k.to_lowercase()))
        .collect()
}

/// The index keys equal to `key`, ignoring case.
fn matching_keys(idx: &NameIndex, key: &str, used: &[String]) -> Vec<String> {
    keys(idx, used)
        .into_iter()
        .filter(|k| k.to_lowercase() == key.to_lowercase())
        .collect()
}

/// Every `(key, value)` of `keys`, in that order; values with a `,` are
/// left out (the field would split them into two tokens).
fn pairs<'a>(
    idx: &'a NameIndex,
    keys: &'a [String],
) -> impl Iterator<Item = (String, String)> + 'a {
    keys.iter().flat_map(|k| {
        idx.values_for_key(k)
            .into_iter()
            .filter(|v| !v.contains(','))
            .map(|v| (k.clone(), v))
    })
}

/// The lowercase keys of `earlier` label tokens (`!` and `label:` dropped).
fn used_keys(earlier: &[String]) -> Vec<String> {
    earlier
        .iter()
        .filter_map(|t| {
            let t = t.trim().trim_start_matches('!').trim();
            let t = t.strip_prefix("label:").unwrap_or(t);
            t.split_once('=').map(|(k, _)| k.trim().to_lowercase())
        })
        .collect()
}

/// `names` as `(name, prefix + name)`.
fn named(names: Vec<String>, prefix: &str) -> Vec<(String, String)> {
    names
        .into_iter()
        .map(|n| (n.clone(), format!("{prefix}{n}")))
        .collect()
}

/// A label token: `key=value` offers that key's values; a bare token offers
/// the keys (`key=`, with `words`), then every `key=value` whose value
/// matches (with `extra`, such as `group:` names). Keys in `used` are left out.
fn labels(
    token: &str,
    used: &[String],
    idx: &NameIndex,
    words: &[&str],
    extra: &[Vec<(String, String)>],
) -> Vec<String> {
    if let Some((k, v)) = token.split_once('=') {
        let keys = matching_keys(idx, k.trim(), used);
        return ranked(
            v.trim_start(),
            pairs(idx, &keys).map(|(k, v)| (v.clone(), format!("{k}={v}"))),
        );
    }
    let keys = keys(idx, used);
    let mut out = ranked(
        token,
        keys.iter()
            .map(|k| (k.clone(), format!("{k}=")))
            .chain(words.iter().map(|w| (w.to_string(), w.to_string()))),
    );
    out.extend(ranked(
        token,
        pairs(idx, &keys)
            .map(|(k, v)| (v.clone(), format!("{k}={v}")))
            .chain(extra.iter().flatten().cloned()),
    ));
    out
}

/// A token starting with one of the `sources` prefixes (any case): the
/// names of that source, `prefix` + name. `None` for any other token.
fn prefixed(token: &str, sources: &[(&str, Vec<String>)]) -> Option<Vec<String>> {
    sources.iter().find_map(|(prefix, names)| {
        let rest = token
            .get(..prefix.len())
            .filter(|head| head.eq_ignore_ascii_case(prefix))
            .map(|_| &token[prefix.len()..])?;
        Some(ranked(rest, named(names.clone(), prefix)))
    })
}

/// Every name of `sources` as `(name, prefix + name)`, in order.
fn named_all(sources: &[(&str, Vec<String>)]) -> Vec<Vec<(String, String)>> {
    sources
        .iter()
        .map(|(prefix, names)| named(names.clone(), prefix))
        .collect()
}

/// A provider or consumer: `group:` and `iplist:` complete their names,
/// `key=` its values; a bare token offers keys, `all`, `group:`, `iplist:`,
/// then label values, label groups and IP lists. `workload:` and hrefs are
/// left as typed.
fn actors(token: &str, idx: &NameIndex, excluded: bool) -> Vec<String> {
    let workload = token
        .get(..9)
        .is_some_and(|h| h.eq_ignore_ascii_case("workload:"));
    if workload || is_href(token) {
        return Vec::new();
    }
    let sources = [
        ("group:", idx.label_group_names()),
        ("iplist:", idx.ip_list_names()),
    ];
    if let Some(names) = prefixed(token, &sources) {
        return names;
    }
    // `label:key=value` completes like `key=value` and keeps its prefix.
    if let Some(rest) = token
        .get(..6)
        .filter(|h| h.eq_ignore_ascii_case("label:"))
        .map(|_| &token[6..])
    {
        return labels(rest, &[], idx, &[], &[])
            .into_iter()
            .map(|c| format!("label:{c}"))
            .collect();
    }
    // `all` cannot be excluded.
    let words: &[&str] = if excluded {
        &["group:", "iplist:"]
    } else {
        &["all", "group:", "iplist:"]
    };
    labels(token, &[], idx, words, &named_all(&sources))
}

/// Completes the token after a leading `!` and keeps the `!` on each candidate.
fn negatable(token: &str, f: impl Fn(&str) -> Vec<String>) -> Vec<String> {
    match token.strip_prefix('!') {
        Some(rest) => f(rest.trim_start())
            .into_iter()
            .map(|c| format!("!{c}"))
            .collect(),
        None => f(token),
    }
}

/// The window ladder. A whole window (`24h`) offers every step, starting
/// at the one `+` would go to, then the rest in ladder order; a partial
/// token (`1`, `d`) the steps it matches.
fn window_steps(token: &str) -> Vec<String> {
    let steps = WINDOW_STEPS.map(String::from);
    let Ok(current) = parse_window(token.trim()) else {
        return ranked(token, steps.into_iter().map(|w| (w.clone(), w)));
    };
    let start = next_window(current)
        .and_then(|n| WINDOW_STEPS.iter().position(|w| *w == n))
        .unwrap_or(0);
    let mut out = steps.to_vec();
    out.rotate_left(start);
    out
}

/// The words the filter's decision field takes.
const DECISIONS: [&str; 4] = ["all", "allowed", "potentially_blocked", "blocked"];

/// The prefixes `FilterExpr` takes besides `key=value`; a source takes
/// all but the last (`fqdn:`).
const FILTER_WORDS: [&str; 4] = ["ip:", "ipl:", "wl:", "fqdn:"];

/// A traffic filter source or destination, in `FilterExpr`'s grammar:
/// `ipl:` completes IP list names, `wl:` the loaded workloads' hostnames
/// and names, `key=` its values; a bare token offers keys (not those in
/// `used`, the current AND group's), the prefixes, then label values, IP
/// lists and workloads. `ip:` and `fqdn:` are left as typed; a `source`
/// is not offered `fqdn:`.
fn filter_actors(
    token: &str,
    used: &[String],
    idx: &NameIndex,
    workloads: &[Workload],
    source: bool,
) -> Vec<String> {
    let literal = ["ip:", "fqdn:"].iter().any(|p| {
        token
            .get(..p.len())
            .is_some_and(|h| h.eq_ignore_ascii_case(p))
    });
    if literal {
        return Vec::new();
    }
    let sources = [
        ("ipl:", idx.ip_list_names()),
        ("wl:", workload_names(workloads)),
    ];
    if let Some(names) = prefixed(token, &sources) {
        return names;
    }
    let words = if source {
        &FILTER_WORDS[..3]
    } else {
        &FILTER_WORDS[..]
    };
    labels(token, used, idx, words, &named_all(&sources))
}

/// A Workloads label filter term: a bare token offers keys (`app=`) and
/// then `key=value` pairs; `key=v1,` offers the key's other values, each
/// as the whole term (`app=shop,crm`), and `-` (no label) last. Values
/// with spaces or commas come quoted, as the filter reads them.
fn workload_label_term(token: &str, used: &[String], idx: &NameIndex) -> Vec<String> {
    let Some((k, vals)) = token.split_once('=') else {
        return labels(token, used, idx, &[], &[])
            .into_iter()
            .map(|c| match c.split_once('=') {
                Some((k, v)) if !v.is_empty() => {
                    format!("{k}={}", pce::label_filter::quote(v))
                }
                _ => c,
            })
            .collect();
    };
    let (done, partial) = match vals.rfind(',') {
        Some(i) => (&vals[..=i], &vals[i + 1..]),
        None => ("", vals),
    };
    let listed: Vec<String> = done
        .split(',')
        .map(|v| v.trim().trim_matches('"').to_string())
        .filter(|v| !v.is_empty())
        .collect();
    let keys = matching_keys(idx, k.trim(), &[]);
    let key = keys.first().map_or(k.trim(), String::as_str);
    let values = idx
        .values_for_key(key)
        .into_iter()
        .chain(std::iter::once(pce::label_filter::NO_LABEL.to_string()))
        .filter(|v| !listed.contains(v))
        .map(|v| {
            let shown = if v == pce::label_filter::NO_LABEL {
                v.clone()
            } else {
                pce::label_filter::quote(&v)
            };
            (v, format!("{key}={done}{shown}"))
        });
    ranked(partial.trim_start_matches('"'), values)
}

/// A mark form's scope words.
pub const THIS_PCE: &str = "this PCE";
pub const ANY_PCE: &str = "any PCE";

/// A fixed list cycled by `Ctrl-N`: the word after the one in the field
/// first (so the first `Ctrl-N` changes it), else those matching the
/// typed text, else all of them.
fn cycle_words(token: &str, words: &[&str]) -> Vec<String> {
    let t = token.trim();
    if let Some(i) = words.iter().position(|w| w.eq_ignore_ascii_case(t)) {
        return words[i + 1..]
            .iter()
            .chain(&words[..=i])
            .map(|w| w.to_string())
            .collect();
    }
    let hits = ranked(t, words.iter().map(|w| (w.to_string(), w.to_string())));
    if hits.is_empty() {
        words.iter().map(|w| w.to_string()).collect()
    } else {
        hits
    }
}

/// The prefixes a mark side takes besides labels.
const MARK_WORDS: [&str; 4] = ["wl:", "ip:", "ipl:", "any"];

/// A mark side: labels, `ipl:` names from the index, `wl:` names of the
/// loaded workloads, `ip:` (typed), `any`. Never a request.
fn mark_side(token: &str, used: &[String], idx: &NameIndex, workloads: &[Workload]) -> Vec<String> {
    if token
        .get(..3)
        .is_some_and(|h| h.eq_ignore_ascii_case("ip:"))
    {
        return Vec::new();
    }
    let sources = [
        ("ipl:", idx.ip_list_names()),
        ("wl:", workload_names(workloads)),
    ];
    if let Some(names) = prefixed(token, &sources) {
        return names;
    }
    // `app="my` matches values as typed; a value with a separator comes
    // back quoted, as the side grammar reads it.
    let token = token.replacen("=\"", "=", 1);
    labels(&token, used, idx, &MARK_WORDS, &named_all(&sources))
        .into_iter()
        .map(|c| quote_label(&c))
        .collect()
}

/// The words the rule search takes besides `key=value`.
const RULE_WORDS: [&str; 6] = ["ipl:", "svc:", "wl:", "href:", "disabled", "deny"];

/// A rule search term: `ipl:` IP list, `svc:` service and `wl:` loaded
/// workload names (quoted when they hold a space), `key=` its values (a
/// comma lists more, as on Workloads; never `-`), a bare token the words,
/// keys and values. `href:` and ports are left as typed. Never a request.
fn rule_filter_term(token: &str, idx: &NameIndex, workloads: &[Workload]) -> Vec<String> {
    let services = idx.services().iter().map(|s| s.name.clone()).collect();
    let sources = [
        ("ipl:", idx.ip_list_names()),
        ("svc:", services),
        ("wl:", workload_names(workloads)),
    ];
    // `ipl:"Any` matches names as typed.
    let unquoted = token.replacen(":\"", ":", 1);
    if let Some(names) = prefixed(&unquoted, &sources) {
        return names
            .into_iter()
            .map(|c| match c.split_once(':') {
                Some((p, n)) if n.contains(char::is_whitespace) || n.contains(['"', '\\']) => {
                    format!("{p}:\"{}\"", n.replace('\\', "\\\\").replace('"', "\\\""))
                }
                _ => c,
            })
            .collect();
    }
    let no_dash = |c: &String| !(c.ends_with("=-") || c.ends_with(",-"));
    if token.contains('=') {
        return workload_label_term(token, &[], idx)
            .into_iter()
            .filter(no_dash)
            .collect();
    }
    let mut out = ranked(
        token,
        RULE_WORDS.iter().map(|w| (w.to_string(), w.to_string())),
    );
    out.extend(
        workload_label_term(token, &[], idx)
            .into_iter()
            .filter(no_dash),
    );
    out
}

/// `key=value` with the value quoted when it holds a side separator.
fn quote_label(c: &str) -> String {
    match c.split_once('=') {
        Some((k, v)) if v.contains([' ', ',', '|', '"', '\\']) => {
            format!("{k}=\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
        }
        _ => c.to_string(),
    }
}

/// Hostnames and names of `workloads`, any-case sorted, blanks left out.
fn workload_names(workloads: &[Workload]) -> Vec<String> {
    let mut names: Vec<String> = workloads
        .iter()
        .flat_map(|w| [w.hostname.as_deref(), w.name.as_deref()])
        .flatten()
        .filter(|n| !n.trim().is_empty())
        .map(String::from)
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names
}

/// A traffic filter service: the protocols and `svc:`, then each index
/// service whose name matches as its TCP and UDP `port/proto` and
/// `from-to/proto` entries (other protocols have no port to write). `svc:`
/// completes the names the filter can expand (`resolve_service`: not every
/// protocol, no `,` or `|` in the name). A port (`/`, digits, a `-` range)
/// is left as typed.
fn traffic_service(token: &str, idx: &NameIndex) -> Vec<String> {
    let all = idx.services();
    let expandable = all
        .iter()
        .filter(|s| !s.name.contains([',', '|']))
        .filter(|s| {
            let named = pce::traffic::ServiceExpr::Named(s.name.clone());
            pce::traffic::resolve_service(&named, &all).is_ok()
        })
        .map(|s| s.name.clone())
        .collect();
    if let Some(names) = prefixed(token, &[("svc:", expandable)]) {
        return names;
    }
    let is_port = token.contains('/')
        || (!token.is_empty() && token.bytes().all(|b| b.is_ascii_digit() || b == b'-'));
    if is_port {
        return Vec::new();
    }
    let protos = ["tcp", "udp", "icmp", "icmpv6", "svc:"]
        .into_iter()
        .map(|p| (p.to_string(), p.to_string()));
    let mut out = ranked(token, protos);
    let ports = idx.services().into_iter().flat_map(|s| {
        s.service_ports.iter().filter_map(|p| {
            let proto = match p.proto? {
                6 => "tcp",
                17 => "udp",
                _ => return None,
            };
            let port = p.port?;
            let text = match p.to_port.filter(|to| *to > port) {
                Some(to) => format!("{port}-{to}/{proto}"),
                None => format!("{port}/{proto}"),
            };
            Some((s.name.clone(), text))
        })
    });
    out.extend(ranked(token, ports));
    out
}

/// Service names (an optional `service:` prefix kept); a token with a `/`
/// or only digits is a port and left as typed.
fn services(token: &str, idx: &NameIndex) -> Vec<String> {
    let (prefix, name) = match token.strip_prefix("service:") {
        Some(n) => ("service:", n.trim_start()),
        None => ("", token),
    };
    let is_port =
        name.contains('/') || (!name.is_empty() && name.bytes().all(|b| b.is_ascii_digit()));
    if is_port || is_href(token) {
        return Vec::new();
    }
    let names = idx.services().into_iter().map(|s| s.name.clone()).collect();
    ranked(name, named(names, prefix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::fixture_index;
    use pce::KeyOrder;
    use pce::models::{IpList, Label, LabelGroup, Service, Workload};

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    /// The fixture index (app=shop, env=prod, loc=eu, role=web|db, the
    /// `All prod envs` group, the `Any …` and `Corp` IP lists, the `All
    /// Services`, `PostgreSQL` and `Web` services).
    fn fx(kind: FieldKind, token: &str, earlier: &[&str]) -> Vec<String> {
        complete(kind, token, &s(earlier), &fixture_index(), &[], &[])
    }

    /// Labels for the "smart" cases: `pay` in two keys' values, a model
    /// order (role first) and an ignored key.
    fn rich() -> NameIndex {
        let label = |n: usize, k: &str, v: &str| -> Label {
            serde_json::from_value(serde_json::json!({
                "href": format!("/orgs/5/labels/{n}"), "key": k, "value": v, "deleted": false
            }))
            .unwrap()
        };
        let labels = vec![
            label(1, "app", "payment"),
            label(2, "app", "Shop"),
            label(3, "role", "payroll"),
            label(4, "role", "web"),
            label(5, "env", "prod"),
            label(6, "paygrade", "high"),
            label(7, "bu", "repay"),
            label(8, "single", "only"),
        ];
        let groups: Vec<LabelGroup> = Vec::new();
        let lists: Vec<IpList> = Vec::new();
        let services: Vec<Service> = Vec::new();
        NameIndex::new(&labels, &groups, &lists, &services)
            .with_key_order(KeyOrder::new(&s(&["role", "app"]), "").with_ignored(&s(&["bu"])))
    }

    fn rc(kind: FieldKind, token: &str, earlier: &[&str]) -> Vec<String> {
        complete(kind, token, &s(earlier), &rich(), &[], &[])
    }

    #[test]
    fn label_keys_prefix_before_substring_any_case() {
        assert_eq!(
            fx(FieldKind::LabelKey, "", &[]),
            s(&["app", "env", "loc", "role"])
        );
        assert_eq!(fx(FieldKind::LabelKey, "O", &[]), s(&["loc", "role"]));
        assert_eq!(fx(FieldKind::LabelKey, "e", &[]), s(&["env", "role"]));
        assert!(fx(FieldKind::LabelKey, "zzz", &[]).is_empty());
    }

    #[test]
    fn label_values_follow_the_key_field() {
        assert_eq!(fx(FieldKind::LabelValue, "", &["role"]), s(&["db", "web"]));
        assert_eq!(fx(FieldKind::LabelValue, "W", &["ROLE"]), s(&["web"]));
        assert!(fx(FieldKind::LabelValue, "", &["nokey"]).is_empty());
        assert!(fx(FieldKind::LabelValue, "", &[]).is_empty());
    }

    #[test]
    fn key_equals_switches_to_that_keys_values() {
        assert_eq!(
            fx(FieldKind::Labels, "role=", &[]),
            s(&["role=db", "role=web"])
        );
        assert_eq!(fx(FieldKind::Labels, "ROLE=W", &[]), s(&["role=web"]));
        assert_eq!(
            rc(FieldKind::Labels, "app=p", &[]),
            s(&["app=payment", "app=Shop"]),
            "prefix, then substring; real case"
        );
        assert_eq!(
            rc(FieldKind::Actors, "app=", &[]),
            s(&["app=payment", "app=Shop"]),
            "every value of the key, any case in order"
        );
    }

    #[test]
    fn a_bare_token_offers_keys_then_values_in_key_order() {
        assert_eq!(
            rc(FieldKind::Labels, "pay", &[]),
            s(&["paygrade=", "role=payroll", "app=payment", "bu=repay"]),
            "the key first; values by prefix (role before app), then substring"
        );
        assert_eq!(
            rc(FieldKind::Labels, "", &[]).first().map(String::as_str),
            Some("role="),
            "an empty token lists the keys in KeyOrder first"
        );
        assert!(
            rc(FieldKind::Labels, "", &[]).contains(&"bu=repay".to_string()),
            "ignored keys are still offered"
        );
    }

    #[test]
    fn labels_and_scopes_skip_keys_already_used_actors_do_not() {
        let got = rc(FieldKind::Labels, "", &["role=web", "APP=Shop"]);
        assert!(
            !got.iter()
                .any(|c| c.starts_with("role=") || c.starts_with("app=")),
            "{got:?}"
        );
        assert!(rc(FieldKind::Labels, "role=", &["role=web"]).is_empty());
        assert!(rc(FieldKind::Scope, "app=", &["!app=Shop"]).is_empty());
        assert_eq!(
            rc(FieldKind::Actors, "role=w", &["role=payroll"]),
            s(&["role=web"])
        );
    }

    #[test]
    fn actors_offer_groups_ip_lists_and_fixed_words() {
        assert_eq!(
            fx(FieldKind::Actors, "group:", &[]),
            s(&["group:All prod envs"])
        );
        assert_eq!(
            fx(FieldKind::Actors, "group:PROD", &[]),
            s(&["group:All prod envs"])
        );
        assert_eq!(
            fx(FieldKind::Actors, "iplist:", &[]),
            s(&["iplist:Any (0.0.0.0/0 and ::/0)", "iplist:Corp"])
        );
        assert_eq!(
            fx(FieldKind::Actors, "al", &[]),
            s(&["all", "group:All prod envs"])
        );
        assert_eq!(fx(FieldKind::Actors, "cor", &[]), s(&["iplist:Corp"]));
        assert_eq!(fx(FieldKind::Actors, "gr", &[]), s(&["group:"]));
        assert_eq!(
            fx(FieldKind::Actors, "!al", &[]),
            s(&["!group:All prod envs"]),
            "an exclusion keeps its ! and never offers all"
        );
        assert!(fx(FieldKind::Actors, "workload:x", &[]).is_empty());
        assert!(fx(FieldKind::Actors, "/orgs/5/labels/1", &[]).is_empty());
        let all = fx(FieldKind::Actors, "", &[]);
        assert_eq!(
            &all[..7],
            &s(&["app=", "env=", "loc=", "role=", "all", "group:", "iplist:"])
        );
        assert!(all.contains(&"role=db".to_string()));
    }

    #[test]
    fn scopes_offer_labels_and_groups_only() {
        let got = fx(FieldKind::Scope, "", &[]);
        assert!(got.contains(&"group:All prod envs".to_string()));
        assert!(
            !got.iter().any(|c| c == "all" || c.starts_with("iplist:")),
            "{got:?}"
        );
        assert_eq!(
            fx(FieldKind::Scope, "!pro", &[]),
            s(&["!env=prod", "!group:All prod envs"])
        );
        assert_eq!(fx(FieldKind::Scope, "gr", &[]), s(&["group:"]));
        assert_eq!(
            fx(FieldKind::Scope, "Group:all", &[]),
            s(&["group:All prod envs"]),
            "the prefix in any case"
        );
        assert!(fx(FieldKind::Scope, "iplist:", &[]).is_empty());
    }

    #[test]
    fn services_complete_names_and_leave_ports() {
        assert_eq!(
            fx(FieldKind::Services, "", &[]),
            s(&["All Services", "PostgreSQL", "Web"])
        );
        assert_eq!(
            fx(FieldKind::Services, "s", &[]),
            s(&["All Services", "PostgreSQL"])
        );
        assert_eq!(
            fx(FieldKind::Services, "service:w", &[]),
            s(&["service:Web"])
        );
        assert!(fx(FieldKind::Services, "443/", &[]).is_empty());
        assert!(fx(FieldKind::Services, "44", &[]).is_empty());
    }

    #[test]
    fn rulesets_come_from_the_loaded_drafts() {
        let rs = |name: &str, update: Option<&str>| -> RuleSet {
            serde_json::from_value(serde_json::json!({
                "href": format!("/orgs/5/sec_policy/draft/rule_sets/{name}"),
                "name": name, "enabled": true, "update_type": update
            }))
            .unwrap()
        };
        let loaded = [
            rs("Shop-Prod", None),
            rs("Old", Some("delete")),
            rs("Prod-DB", None),
        ];
        let got = |t: &str| complete(FieldKind::Ruleset, t, &[], &fixture_index(), &loaded, &[]);
        assert_eq!(got("prod"), s(&["Prod-DB", "Shop-Prod"]));
        assert_eq!(
            got(""),
            s(&["Shop-Prod", "Prod-DB"]),
            "pending deletions left out"
        );
        assert!(complete(FieldKind::Ruleset, "", &[], &fixture_index(), &[], &[]).is_empty());
    }

    #[test]
    fn text_and_an_empty_index_give_nothing() {
        assert!(fx(FieldKind::Text, "a", &[]).is_empty());
        let empty = NameIndex::default();
        for kind in [
            FieldKind::LabelKey,
            FieldKind::Labels,
            FieldKind::Actors,
            FieldKind::Services,
        ] {
            let got = complete(kind, "", &[], &empty, &[], &[]);
            assert!(
                got.iter()
                    .all(|c| ["all", "group:", "iplist:"].contains(&c.as_str())),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_label_prefix_is_completed_and_kept() {
        assert_eq!(
            rc(FieldKind::Actors, "label:app=", &[]),
            s(&["label:app=payment", "label:app=Shop"])
        );
        assert_eq!(
            rc(FieldKind::Actors, "!label:role=w", &[]),
            s(&["!label:role=web"])
        );
        assert_eq!(
            rc(FieldKind::Actors, "LABEL:sin", &[]),
            s(&["label:single="])
        );
    }

    #[test]
    fn values_with_a_comma_are_not_offered() {
        let label = |n: usize, v: &str| -> Label {
            serde_json::from_value(serde_json::json!({
                "href": format!("/orgs/5/labels/{n}"), "key": "app", "value": v
            }))
            .unwrap()
        };
        let idx = NameIndex::new(&[label(1, "a,b"), label(2, "ab")], &[], &[], &[]);
        assert_eq!(
            complete(FieldKind::Labels, "app=a", &[], &idx, &[], &[]),
            s(&["app=ab"])
        );
        assert_eq!(
            complete(FieldKind::Labels, "b", &[], &idx, &[], &[]),
            s(&["app=ab"])
        );
    }

    #[test]
    fn the_list_is_capped() {
        let label = |n: usize| -> Label {
            serde_json::from_value(serde_json::json!({
                "href": format!("/orgs/5/labels/{n}"), "key": "host", "value": format!("h{n:04}")
            }))
            .unwrap()
        };
        let labels: Vec<Label> = (0..500).map(label).collect();
        let idx = NameIndex::new(&labels, &[], &[], &[]);
        assert_eq!(
            complete(FieldKind::Labels, "", &[], &idx, &[], &[]).len(),
            MAX_CANDIDATES
        );
    }

    fn workloads() -> Vec<Workload> {
        serde_json::from_str(include_str!("../../../../fixtures/workloads.json")).unwrap()
    }

    /// A filter field on the fixture index, with the fixture workloads
    /// loaded or not.
    fn ff(kind: FieldKind, token: &str, earlier: &[&str], loaded: bool) -> Vec<String> {
        let wls = if loaded { workloads() } else { Vec::new() };
        complete(kind, token, &s(earlier), &fixture_index(), &[], &wls)
    }

    #[test]
    fn mark_fields_cycle_their_lists_and_sides_offer_the_side_grammar() {
        assert_eq!(
            fx(FieldKind::MarkAction, "reviewed", &[]),
            s(&[
                "squelch",
                "critical",
                "important",
                "revisit",
                "accept-blocked",
                "reviewed"
            ]),
            "the next action first"
        );
        assert_eq!(
            fx(FieldKind::MarkScope, "this PCE", &[]),
            s(&["any PCE", "this PCE"])
        );
        assert_eq!(fx(FieldKind::MarkExpiry, "9", &[]), s(&["90d"]));
        assert_eq!(
            fx(FieldKind::MarkExpiry, "x", &[]).len(),
            5,
            "no match: all"
        );
        let side = ff(FieldKind::MarkSide, "", &[], false);
        assert_eq!(
            &side[..8],
            &s(&["app=", "env=", "loc=", "role=", "wl:", "ip:", "ipl:", "any"]),
            "keys, then the side grammar's words"
        );
        assert!(ff(FieldKind::MarkSide, "ipl:", &[], false).contains(&"ipl:Corp".to_string()));
        assert!(
            ff(FieldKind::MarkSide, "wl:", &[], true)
                .iter()
                .any(|c| c.starts_with("wl:")),
            "the loaded workloads"
        );
        assert!(
            ff(FieldKind::MarkSide, "ip:10", &[], false).is_empty(),
            "typed"
        );
        assert!(
            !ff(FieldKind::MarkSide, "", &["app=shop"], false).contains(&"app=".to_string()),
            "a key used in the side is not offered again"
        );
        assert_eq!(
            FieldKind::MarkSide.request("app=shop en"),
            Some(("en".to_string(), vec!["app=shop".to_string()]))
        );
    }

    #[test]
    fn mark_side_quotes_label_values_with_separators() {
        let label = |n: usize, k: &str, v: &str| -> Label {
            serde_json::from_value(serde_json::json!({
                "href": format!("/orgs/5/labels/{n}"), "key": k, "value": v, "deleted": false
            }))
            .unwrap()
        };
        let labels = vec![label(1, "app", "my shop"), label(2, "app", "crm")];
        let idx = NameIndex::new(&labels, &[], &[], &[]);
        let c = |t: &str| complete(FieldKind::MarkSide, t, &[], &idx, &[], &[]);
        assert_eq!(c("app="), s(&["app=crm", "app=\"my shop\""]));
        assert_eq!(c("app=\"my"), s(&["app=\"my shop\""]));
        assert!(pce::marks::parse_side("app=\"my shop\"").is_ok());
    }

    #[test]
    fn filter_actors_offer_the_filter_grammar() {
        let all = ff(FieldKind::TrafficActors, "", &[], false);
        assert_eq!(
            &all[..8],
            &s(&[
                "app=", "env=", "loc=", "role=", "ip:", "ipl:", "wl:", "fqdn:"
            ]),
            "keys in KeyOrder, then the grammar's prefixes"
        );
        assert!(all.contains(&"role=db".to_string()));
        assert!(
            !all.iter()
                .any(|c| c.starts_with("group:") || c.starts_with("iplist:") || c == "all"),
            "the filter takes no label groups, `iplist:` or `all`: {all:?}"
        );
        assert_eq!(
            ff(FieldKind::TrafficActors, "role=", &[], false),
            s(&["role=db", "role=web"])
        );
        assert_eq!(
            ff(FieldKind::TrafficActors, "ipl:", &[], false),
            s(&["ipl:Any (0.0.0.0/0 and ::/0)", "ipl:Corp"])
        );
        assert_eq!(
            ff(FieldKind::TrafficActors, "IPL:co", &[], false),
            s(&["ipl:Corp"])
        );
        assert_eq!(
            ff(FieldKind::TrafficActors, "cor", &[], false),
            s(&["ipl:Corp"]),
            "a bare token matches IP list names"
        );
        assert_eq!(
            ff(FieldKind::TrafficActors, "pro", &[], false),
            s(&["env=prod"]),
            "and values across keys"
        );
        assert_eq!(
            ff(FieldKind::TrafficActors, "!ro", &[], false),
            s(&["!role=", "!env=prod"]),
            "after a ! the same candidates, the ! kept"
        );
        assert_eq!(
            ff(FieldKind::TrafficActors, "!ipl:co", &[], false),
            s(&["!ipl:Corp"])
        );
        assert_eq!(
            ff(FieldKind::TrafficActors, "! wl:web", &[], true),
            s(&["!wl:web01"])
        );
        assert!(ff(FieldKind::TrafficActors, "!ip:10.", &[], false).is_empty());
        assert!(
            ff(FieldKind::TrafficActors, "!app=", &["app=shop"], false)
                .contains(&"!app=shop".into()),
            "an exclusion is field-wide: the group's keys stay offered"
        );
        assert!(ff(FieldKind::TrafficActors, "ip:10.", &[], true).is_empty());
        assert!(ff(FieldKind::TrafficActors, "fqdn:x", &[], true).is_empty());
        let got = ff(FieldKind::TrafficActors, "", &["app=shop"], false);
        assert_eq!(
            &got[..7],
            &s(&["env=", "loc=", "role=", "ip:", "ipl:", "wl:", "fqdn:"]),
            "a key already in the AND group is not offered again"
        );
        assert!(!got.iter().any(|c| c.starts_with("app=")), "{got:?}");
        assert!(ff(FieldKind::TrafficActors, "APP=", &["app=shop"], false).is_empty());
    }

    #[test]
    fn the_source_side_offers_no_fqdn_and_bang_terms_hide_no_keys() {
        let src = ff(FieldKind::TrafficSource, "", &[], false);
        assert_eq!(
            &src[..7],
            &s(&["app=", "env=", "loc=", "role=", "ip:", "ipl:", "wl:"])
        );
        assert!(!src.contains(&"fqdn:".to_string()), "{src:?}");
        assert!(ff(FieldKind::TrafficSource, "fq", &[], false).is_empty());
        assert_eq!(
            ff(FieldKind::TrafficActors, "fq", &[], false),
            s(&["fqdn:"])
        );
        let got = ff(FieldKind::TrafficSource, "", &["!app=shop"], false);
        assert!(
            got.contains(&"app=".to_string()),
            "an excluded key is still offered to include: {got:?}"
        );
        let got = ff(FieldKind::TrafficSource, "", &["app=shop"], false);
        assert!(!got.contains(&"app=".to_string()), "{got:?}");
    }

    #[test]
    fn filter_actors_keep_key_order_and_ignored_keys() {
        let got = complete(FieldKind::TrafficActors, "pay", &[], &rich(), &[], &[]);
        assert_eq!(
            got,
            s(&["paygrade=", "role=payroll", "app=payment", "bu=repay"]),
            "the key, then values by prefix in KeyOrder, then substring"
        );
        let all = complete(FieldKind::TrafficActors, "", &[], &rich(), &[], &[]);
        assert_eq!(all.first().map(String::as_str), Some("role="));
        assert!(
            all.contains(&"bu=repay".to_string()),
            "ignored keys still offered"
        );
    }

    #[test]
    fn filter_actors_offer_workloads_only_when_loaded() {
        assert!(ff(FieldKind::TrafficActors, "wl:", &[], false).is_empty());
        assert!(ff(FieldKind::TrafficActors, "host", &[], false).is_empty());
        assert_eq!(
            ff(FieldKind::TrafficActors, "wl:", &[], true),
            s(&[
                "wl:batch04",
                "wl:host-1.example",
                "wl:host-2.example",
                "wl:host-4.example",
                "wl:unmanaged-nas",
                "wl:web01",
            ]),
            "hostnames and names, any-case sorted"
        );
        assert_eq!(
            ff(FieldKind::TrafficActors, "wl:WEB", &[], true),
            s(&["wl:web01"])
        );
        assert_eq!(
            ff(FieldKind::TrafficActors, "nas", &[], true),
            s(&["wl:unmanaged-nas"]),
            "a bare token matches workload names"
        );
    }

    #[test]
    fn filter_services_offer_protocols_ports_and_ranges() {
        assert_eq!(
            ff(FieldKind::TrafficService, "", &[], false),
            s(&[
                "tcp",
                "udp",
                "icmp",
                "icmpv6",
                "svc:",
                "5432/tcp",
                "80/tcp",
                "443/tcp",
                "8000-8099/tcp"
            ]),
            "a range as from-to/proto, no `All Services`"
        );
        assert_eq!(
            ff(FieldKind::TrafficService, "post", &[], false),
            s(&["5432/tcp"]),
            "a service by name, as the port/proto the filter takes"
        );
        assert_eq!(
            ff(FieldKind::TrafficService, "WEB", &[], false),
            s(&["80/tcp", "443/tcp", "8000-8099/tcp"])
        );
        assert_eq!(ff(FieldKind::TrafficService, "u", &[], false), s(&["udp"]));
        assert!(ff(FieldKind::TrafficService, "443", &[], false).is_empty());
        assert!(ff(FieldKind::TrafficService, "443/t", &[], false).is_empty());
        assert!(ff(FieldKind::TrafficService, "8000-", &[], false).is_empty());
        assert_eq!(
            ff(FieldKind::TrafficService, "!post", &[], false),
            s(&["!5432/tcp"]),
            "after a ! the same candidates, the ! kept"
        );
        assert_eq!(
            ff(FieldKind::TrafficService, "!u", &[], false),
            s(&["!udp"])
        );
        assert!(ff(FieldKind::TrafficService, "!22", &[], false).is_empty());
        assert_eq!(
            ff(FieldKind::TrafficService, "svc:", &[], false),
            s(&["svc:PostgreSQL", "svc:Web"]),
            "only names the filter can expand: not every protocol"
        );
        assert_eq!(
            ff(FieldKind::TrafficService, "!SVC:post", &[], false),
            s(&["!svc:PostgreSQL"])
        );
    }

    #[test]
    fn transmission_offers_the_kinds_not_yet_listed() {
        assert_eq!(
            ff(FieldKind::Transmission, "", &[], false),
            s(&["all", "unicast", "broadcast", "multicast"])
        );
        assert_eq!(
            ff(FieldKind::Transmission, "m", &[], false),
            s(&["multicast"])
        );
        assert_eq!(
            ff(FieldKind::Transmission, "", &["unicast"], false),
            s(&["broadcast", "multicast"])
        );
    }

    #[test]
    fn decisions_offer_the_words_not_yet_listed() {
        assert_eq!(
            ff(FieldKind::Decisions, "", &[], false),
            s(&["all", "allowed", "potentially_blocked", "blocked"])
        );
        assert_eq!(
            ff(FieldKind::Decisions, "b", &[], false),
            s(&["blocked", "potentially_blocked"]),
            "prefix before substring"
        );
        assert_eq!(
            ff(FieldKind::Decisions, "AL", &[], false),
            s(&["all", "allowed", "potentially_blocked"])
        );
        assert_eq!(
            ff(FieldKind::Decisions, "", &["Allowed", "blocked"], false),
            s(&["potentially_blocked"]),
            "`all` only as the first word"
        );
        assert!(
            ff(FieldKind::Decisions, "", &["all"], false).is_empty(),
            "nothing after `all`"
        );
    }

    #[test]
    fn windows_follow_the_step_ladder() {
        assert_eq!(
            ff(FieldKind::Window, "", &[], false),
            s(&["1h", "6h", "12h", "1d", "2d", "3d", "1w", "2w", "4w", "90d"])
        );
        assert_eq!(
            ff(FieldKind::Window, "1", &[], false),
            s(&["1h", "12h", "1d", "1w"])
        );
        assert_eq!(
            ff(FieldKind::Window, "D", &[], false),
            s(&["1d", "2d", "3d", "90d"])
        );
        assert_eq!(
            ff(FieldKind::Window, "24h", &[], false),
            s(&["2d", "3d", "1w", "2w", "4w", "90d", "1h", "6h", "12h", "1d"]),
            "a whole window: the ladder from where `+` goes, then the rest"
        );
        assert_eq!(
            ff(FieldKind::Window, "90d", &[], false),
            s(&["1h", "6h", "12h", "1d", "2d", "3d", "1w", "2w", "4w", "90d"]),
            "the top of the ladder: in ladder order"
        );
    }
}
