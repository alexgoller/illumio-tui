//! Which ruleset a scope's rules go into: the draft rulesets whose scope
//! covers it (the logic the `w` form's pre-fill uses, moved here so it
//! exists once), the name of a ruleset to create, and the keys check.
use super::{Target, TargetScope};
use crate::models::{Actor, RuleSet};
use crate::resolve::NameIndex;
use crate::write::{ActorKind, ActorRef, RuleSetDraft};
use serde::Serialize;
use std::collections::BTreeSet;

/// `generator.ruleset_name` when the profile names none and the scope keys
/// are the default ones: `RF-shop-prod`, the ringfence convention.
pub const DEFAULT_RULESET_NAME: &str = "RF-{app}-{env}";

/// The default template for any scope keys: `RF-` and every key, `-`
/// apart (`RF-{app}-{env}` for the default ones).
pub fn default_ruleset_name(scope_keys: &[String]) -> String {
    let keys: Vec<String> = scope_keys.iter().map(|k| format!("{{{k}}}")).collect();
    format!("RF-{}", keys.join("-"))
}

/// Every `{key}` in `template` must be a scope key and every scope key
/// must be named (so two scopes never share a name); braces must close;
/// the template must not be blank. The error names the key.
pub fn check_ruleset_name(template: &str, scope_keys: &[String]) -> Result<(), String> {
    if template.trim().is_empty() {
        return Err("generator.ruleset_name must not be empty".into());
    }
    let mut named = BTreeSet::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            return Err(format!(
                "generator.ruleset_name {template:?}: a {{ is not closed"
            ));
        };
        let key = &after[..close];
        if !scope_keys.iter().any(|k| k == key) {
            return Err(format!(
                "generator.ruleset_name {template:?}: {{{key}}} is not a scope key ({})",
                scope_keys.join(", ")
            ));
        }
        named.insert(key);
        rest = &after[close + 1..];
    }
    if let Some(k) = scope_keys.iter().find(|k| !named.contains(k.as_str())) {
        return Err(format!(
            "generator.ruleset_name {template:?}: {{{k}}} is missing; every scope key must be named, \
             so each new ruleset gets its own name"
        ));
    }
    Ok(())
}

/// `template` with each `{key}` replaced by the scope's value for it.
pub fn render_ruleset_name(template: &str, keys: &[String], values: &[String]) -> String {
    let mut out = template.to_string();
    for (k, v) in keys.iter().zip(values) {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out.trim().to_string()
}

/// The generator works on apps of the scope keys only: their rules go
/// into scoped rulesets, and only the scope keys say which.
pub fn keys_refusal(keys: &[String], scope_keys: &[String]) -> Option<String> {
    (keys != scope_keys).then(|| {
        format!(
            "the generator needs g = {} (the scope keys)",
            scope_keys.join(",")
        )
    })
}

/// The live draft rulesets with a scope that covers an endpoint holding
/// the labels `has`: each actor of one of its scopes is a label in `has`
/// (under any key) or a label group with such a label among its members.
/// The empty, global scope and other actors do not count. Those scoped
/// exactly like `want` (in one of their scopes, no group) come first
/// (`true`), then the others, the more actors the earlier, each in list
/// order. A ruleset with a second scope is wider than `want` even so: the
/// generator checks [`narrow`] before writing `ams` into it.
pub fn rulesets_covering<'r>(
    sets: impl IntoIterator<Item = &'r RuleSet>,
    has: &BTreeSet<&str>,
    want: &BTreeSet<&str>,
    idx: &NameIndex,
) -> Vec<(&'r RuleSet, bool)> {
    // `Some((exact, actors))` when the scope covers the endpoint.
    let fit = |s: &Vec<Actor>| -> Option<(bool, usize)> {
        if s.is_empty() {
            return None;
        }
        let mut labels = BTreeSet::new();
        let mut grouped = false;
        for a in s {
            if let Some(l) = &a.label {
                labels.insert(*has.get(l.href.as_str())?);
            } else if let Some(g) = &a.label_group {
                grouped = true;
                crate::impact::group_members(&g.href, idx)
                    .iter()
                    .any(|m| has.contains(m.as_str()))
                    .then_some(())?;
            } else {
                return None;
            }
        }
        let exact = !grouped && labels.into_iter().eq(want.iter().copied());
        Some((exact, s.len()))
    };
    let mut hits: Vec<(&RuleSet, (bool, usize))> = sets
        .into_iter()
        .filter(|rs| rs.update_type.as_deref() != Some("delete"))
        .filter_map(|rs| rs.scopes.iter().filter_map(fit).max().map(|f| (rs, f)))
        .collect();
    hits.sort_by_key(|(_, (exact, n))| (!exact, std::cmp::Reverse(*n)));
    hits.into_iter()
        .map(|(rs, (exact, _))| (rs, exact))
        .collect()
}

/// One draft ruleset whose scope covers the scope asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Covering {
    pub href: String,
    pub name: String,
    /// Scoped exactly by the scope's labels.
    pub exact: bool,
}

/// The covering rulesets, best first, and the one to use: the only exact
/// one, else the only one; `None` leaves the choice to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulesetChoice {
    pub covering: Vec<Covering>,
    pub picked: Option<usize>,
}

/// [`rulesets_covering`] and the pick, by label hrefs: `scope` the labels
/// a ruleset would be scoped by, `has` every label the endpoint carries
/// (for the generator's app, the scope labels themselves).
pub fn pick_ruleset<'r>(
    scope: &[&str],
    has: &[&str],
    sets: impl IntoIterator<Item = &'r RuleSet>,
    idx: &NameIndex,
) -> RulesetChoice {
    let want: BTreeSet<&str> = scope.iter().copied().collect();
    let has: BTreeSet<&str> = has.iter().copied().collect();
    let covering: Vec<Covering> = rulesets_covering(sets, &has, &want, idx)
        .into_iter()
        .map(|(rs, exact)| Covering {
            href: rs.href.clone(),
            name: rs.name.clone(),
            exact,
        })
        .collect();
    let exact: Vec<usize> = covering
        .iter()
        .enumerate()
        .filter(|(_, c)| c.exact)
        .map(|(i, _)| i)
        .collect();
    let picked = match (exact.as_slice(), covering.len()) {
        ([one], _) => Some(*one),
        ([], 1) => Some(0),
        _ => None,
    };
    RulesetChoice { covering, picked }
}

/// The target of one scope against `drafts`: the picked covering enabled
/// ruleset (`picked` overrides among several; a disabled one enforces
/// nothing, so it is no target), a ruleset to create when none covers it,
/// else ambiguous. A ruleset to create whose name a draft ruleset already
/// has is no target either.
pub fn resolve(
    scope: &TargetScope,
    drafts: &[RuleSet],
    idx: &NameIndex,
    template: &str,
    picked: Option<&str>,
) -> Target {
    if scope.labels.is_empty() || scope.labels.len() != scope.values.len() {
        return Target::None(format!("{} has no scope labels", scope.text()));
    }
    let hrefs: Vec<&str> = scope.labels.iter().map(|l| l.href.as_str()).collect();
    let enabled = drafts.iter().filter(|rs| rs.enabled);
    let choice = pick_ruleset(&hrefs, &hrefs, enabled, idx);
    let chosen = picked
        .and_then(|h| choice.covering.iter().find(|c| c.href == h))
        .or_else(|| choice.picked.map(|i| &choice.covering[i]));
    if let Some(c) = chosen {
        return Target::Existing {
            href: c.href.clone(),
            name: c.name.clone(),
        };
    }
    if !choice.covering.is_empty() {
        return Target::Ambiguous(choice.covering);
    }
    let keys: Vec<String> = scope.labels.iter().map(|l| l.key.clone()).collect();
    let name = render_ruleset_name(template, &keys, &scope.values);
    if let Some(rs) = drafts.iter().find(|rs| rs.name.eq_ignore_ascii_case(&name)) {
        let how = if rs.enabled {
            "with another scope"
        } else {
            "disabled"
        };
        return Target::None(format!(
            "ruleset {} exists {how}: no ruleset for {} to create",
            rs.name,
            scope.text()
        ));
    }
    Target::Create(RuleSetDraft {
        name,
        description: None,
        enabled: true,
        scopes: vec![
            scope
                .labels
                .iter()
                .map(|l| ActorRef::new(ActorKind::Label, l.href.clone()))
                .collect(),
        ],
    })
}

/// Whether `target`'s ruleset is scoped by exactly `scope`'s labels: one
/// scope of those labels and nothing else (a ruleset to create is). Only
/// then do `ams` and a bare role in it stay within the scope's workloads.
pub fn narrow(scope: &TargetScope, target: &Target, drafts: &[RuleSet]) -> bool {
    match target {
        Target::Create(_) => true,
        Target::Existing { href, .. } => {
            drafts.iter().find(|rs| &rs.href == href).is_some_and(|rs| {
                let want: BTreeSet<&str> = scope.labels.iter().map(|l| l.href.as_str()).collect();
                match rs.scopes.as_slice() {
                    [one] => {
                        one.iter()
                            .all(|a| a.label.is_some() && a.exclusion != Some(true))
                            && one.len() == want.len()
                            && one
                                .iter()
                                .filter_map(|a| a.label.as_ref())
                                .all(|l| want.contains(l.href.as_str()))
                    }
                    _ => false,
                }
            })
        }
        Target::Ambiguous(_) | Target::None(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::ResolvedLabel;
    use crate::traffic::resolved::tests::rich_index;

    fn keys() -> Vec<String> {
        vec!["app".into(), "env".into()]
    }

    fn rule_sets() -> Vec<RuleSet> {
        serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap()
    }

    fn label(href: &str, key: &str, value: &str) -> ResolvedLabel {
        ResolvedLabel {
            href: href.into(),
            key: key.into(),
            value: value.into(),
        }
    }

    pub(crate) fn shop_scope() -> TargetScope {
        TargetScope {
            values: vec!["shop".into(), "prod".into()],
            labels: vec![
                label("/orgs/5/labels/3", "app", "shop"),
                label("/orgs/5/labels/4", "env", "prod"),
            ],
        }
    }

    #[test]
    fn the_default_template_is_rf_and_the_scope_keys() {
        assert_eq!(default_ruleset_name(&keys()), DEFAULT_RULESET_NAME);
        assert_eq!(default_ruleset_name(&["site".into()]), "RF-{site}");
        assert_eq!(
            render_ruleset_name(
                DEFAULT_RULESET_NAME,
                &keys(),
                &["shop".into(), "prod".into()]
            ),
            "RF-shop-prod"
        );
    }

    #[test]
    fn a_template_names_scope_keys_only_and_closes_its_braces() {
        assert!(check_ruleset_name("RS-{app}-{env}", &keys()).is_ok());
        assert_eq!(
            check_ruleset_name("fixed name", &keys()).unwrap_err(),
            "generator.ruleset_name \"fixed name\": {app} is missing; every scope key must be \
             named, so each new ruleset gets its own name"
        );
        assert!(
            check_ruleset_name("RF-{app}", &keys())
                .unwrap_err()
                .contains("{env} is missing")
        );
        assert_eq!(
            check_ruleset_name("RF-{app}-{role}", &keys()).unwrap_err(),
            "generator.ruleset_name \"RF-{app}-{role}\": {role} is not a scope key (app, env)"
        );
        assert!(
            check_ruleset_name("RF-{app", &keys())
                .unwrap_err()
                .contains("not closed")
        );
        assert!(check_ruleset_name("  ", &keys()).is_err());
    }

    #[test]
    fn only_the_scope_keys_are_accepted() {
        assert_eq!(keys_refusal(&keys(), &keys()), None);
        assert_eq!(
            keys_refusal(&["app".into(), "env".into(), "role".into()], &keys()).as_deref(),
            Some("the generator needs g = app,env (the scope keys)")
        );
    }

    #[test]
    fn the_exactly_scoped_ruleset_is_picked_and_pending_deletes_are_not_targets() {
        let idx = rich_index();
        let sets = rule_sets();
        let t = resolve(&shop_scope(), &sets, &idx, DEFAULT_RULESET_NAME, None);
        assert_eq!(
            t,
            Target::Existing {
                href: "/orgs/5/sec_policy/draft/rule_sets/100".into(),
                name: "Shop-Prod".into()
            }
        );
        let mut gone = sets.clone();
        gone[0].update_type = Some("delete".into());
        let Target::Create(d) = resolve(&shop_scope(), &gone, &idx, DEFAULT_RULESET_NAME, None)
        else {
            panic!("a ruleset pending deletion is no target")
        };
        assert_eq!(d.name, "RF-shop-prod");
        assert!(d.enabled);
        assert_eq!(
            d.scopes,
            vec![vec![
                ActorRef::new(ActorKind::Label, "/orgs/5/labels/3"),
                ActorRef::new(ActorKind::Label, "/orgs/5/labels/4"),
            ]]
        );
    }

    #[test]
    fn several_covering_rulesets_are_ambiguous_until_one_is_picked() {
        let idx = rich_index();
        let mut sets = rule_sets();
        let mut twin = sets[0].clone();
        twin.href = "/orgs/5/sec_policy/draft/rule_sets/102".into();
        twin.name = "Shop-Prod-2".into();
        sets.push(twin);
        let Target::Ambiguous(c) = resolve(&shop_scope(), &sets, &idx, DEFAULT_RULESET_NAME, None)
        else {
            panic!("two exact rulesets")
        };
        assert_eq!(c.len(), 2);
        let t = resolve(
            &shop_scope(),
            &sets,
            &idx,
            DEFAULT_RULESET_NAME,
            Some("/orgs/5/sec_policy/draft/rule_sets/102"),
        );
        assert_eq!(
            t,
            Target::Existing {
                href: "/orgs/5/sec_policy/draft/rule_sets/102".into(),
                name: "Shop-Prod-2".into()
            }
        );
    }

    #[test]
    fn a_disabled_ruleset_is_no_target() {
        let idx = rich_index();
        let mut sets = rule_sets();
        sets[0].enabled = false;
        let t = resolve(&shop_scope(), &sets, &idx, DEFAULT_RULESET_NAME, None);
        let Target::Create(d) = t else {
            panic!("a disabled ruleset enforces nothing: {t:?}")
        };
        assert_eq!(d.name, "RF-shop-prod");
        // Named like the ruleset to create: no target, and why.
        sets[0].name = "RF-shop-prod".into();
        assert_eq!(
            resolve(&shop_scope(), &sets, &idx, DEFAULT_RULESET_NAME, None),
            Target::None(
                "ruleset RF-shop-prod exists disabled: no ruleset for shop/prod to create".into()
            )
        );
    }

    #[test]
    fn a_ruleset_named_like_the_one_to_create_but_scoped_otherwise_is_no_target() {
        let idx = rich_index();
        let mut sets = rule_sets();
        // Scoped by loc=eu too: it does not cover shop/prod.
        sets[0].scopes[0].push(
            serde_json::from_value(serde_json::json!({"label": {"href": "/orgs/5/labels/5"}}))
                .unwrap(),
        );
        sets[0].name = "rf-shop-prod".into();
        assert_eq!(
            resolve(&shop_scope(), &sets, &idx, DEFAULT_RULESET_NAME, None),
            Target::None(
                "ruleset rf-shop-prod exists with another scope: no ruleset for shop/prod to create"
                    .into()
            )
        );
    }

    #[test]
    fn a_second_scope_or_a_wider_one_is_not_exact_and_not_narrow() {
        let idx = rich_index();
        let mut sets = rule_sets();
        let t = resolve(&shop_scope(), &sets, &idx, DEFAULT_RULESET_NAME, None);
        assert!(narrow(&shop_scope(), &t, &sets), "exactly shop/prod");
        // A second scope (shop and loc=eu).
        sets[0].scopes.push(vec![
            serde_json::from_value(serde_json::json!({"label": {"href": "/orgs/5/labels/3"}}))
                .unwrap(),
            serde_json::from_value(serde_json::json!({"label": {"href": "/orgs/5/labels/5"}}))
                .unwrap(),
        ]);
        let hrefs = ["/orgs/5/labels/3", "/orgs/5/labels/4"];
        let c = pick_ruleset(&hrefs, &hrefs, &sets, &idx);
        assert_eq!(c.picked, Some(0), "still the one");
        assert!(
            !narrow(&shop_scope(), &t, &sets),
            "but wider than shop/prod"
        );
        // Scoped by app=shop only.
        sets[0].scopes = vec![vec![
            serde_json::from_value(serde_json::json!({"label": {"href": "/orgs/5/labels/3"}}))
                .unwrap(),
        ]];
        let t = resolve(&shop_scope(), &sets, &idx, DEFAULT_RULESET_NAME, None);
        assert_eq!(t.name(), "Shop-Prod", "it covers shop/prod");
        assert!(!narrow(&shop_scope(), &t, &sets));
        let Target::Create(_) = resolve(&shop_scope(), &[], &idx, DEFAULT_RULESET_NAME, None)
        else {
            panic!()
        };
        assert!(narrow(
            &shop_scope(),
            &resolve(&shop_scope(), &[], &idx, DEFAULT_RULESET_NAME, None),
            &[]
        ));
    }

    #[test]
    fn a_label_group_scope_covers_through_its_members() {
        let idx = rich_index();
        let mut sets = rule_sets();
        // app=shop and the group "All prod envs" (env=prod).
        sets[0].scopes = vec![vec![
            serde_json::from_value(serde_json::json!({"label": {"href": "/orgs/5/labels/3"}}))
                .unwrap(),
            serde_json::from_value(serde_json::json!(
                {"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/10"}}
            ))
            .unwrap(),
        ]];
        let hrefs = ["/orgs/5/labels/3", "/orgs/5/labels/4"];
        let c = pick_ruleset(&hrefs, &hrefs, &sets, &idx);
        assert_eq!(c.covering.len(), 1);
        assert!(!c.covering[0].exact);
        assert_eq!(c.picked, Some(0), "the only one covering it");
    }
}
