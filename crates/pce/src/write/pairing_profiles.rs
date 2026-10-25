//! Pairing profiles under `/orgs/N/pairing_profiles`: create and delete.
//! A create sends `name` plus only the settings given, so the PCE applies
//! its own defaults to the rest. Every PCE field name of the request is
//! set in [`PairingProfileDraft::to_body`] and nowhere else.
use super::bulk::{EnforcementMode, VisibilityLevel};
use super::{ObjectKind, Plan, WriteError, WriteRequest, Writer, checked_href};
use crate::client::WriteMethod;
use crate::models::PairingProfile;
use crate::models::pairing_profiles::LOCKS;
use serde_json::{Map, Value, json};

/// The VEN type a profile pairs: `ven_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VenType {
    Server,
    Endpoint,
    SpecifiedDuringActivation,
}

impl VenType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Endpoint => "endpoint",
            Self::SpecifiedDuringActivation => "specified_during_activation",
        }
    }

    /// `server`, `endpoint`, `specified-during-activation` (or with `_`).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().replace('-', "_").as_str() {
            "server" => Some(Self::Server),
            "endpoint" => Some(Self::Endpoint),
            "specified_during_activation" => Some(Self::SpecifiedDuringActivation),
            _ => None,
        }
    }
}

/// `allowed_uses_per_key` or `key_lifespan`: a positive number or
/// `"unlimited"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    Count(u64),
    Unlimited,
}

impl Limit {
    fn to_json(self) -> Value {
        match self {
            Limit::Count(n) => json!(n),
            Limit::Unlimited => json!("unlimited"),
        }
    }

    /// `--uses`: an integer of at least 1, or `unlimited`.
    pub fn parse_uses(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("unlimited") {
            return Ok(Limit::Unlimited);
        }
        match s.parse::<u64>() {
            Ok(n) if n >= 1 => Ok(Limit::Count(n)),
            _ => Err(format!(
                "--uses {s:?}: a whole number of at least 1, or unlimited"
            )),
        }
    }

    /// `--lifespan`: seconds (`3600`), a duration (`1h`, `7d`, `1h 30m`),
    /// or `unlimited`; at least one second, in whole seconds.
    pub fn parse_lifespan(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("unlimited") {
            return Ok(Limit::Unlimited);
        }
        let bad = || format!("--lifespan {s:?}: seconds, a duration (1h, 7d) or unlimited");
        let secs = if s.bytes().all(|b| b.is_ascii_digit()) && !s.is_empty() {
            s.parse::<u64>().map_err(|_| bad())?
        } else {
            let d = humantime::parse_duration(s).map_err(|_| bad())?;
            if d.subsec_nanos() != 0 {
                return Err(format!("--lifespan {s:?}: whole seconds only"));
            }
            d.as_secs()
        };
        if secs == 0 {
            return Err(format!("--lifespan {s:?}: at least one second"));
        }
        Ok(Limit::Count(secs))
    }
}

/// `--enforcement`: the PCE's names, `-` for `_` too (`visibility-only`).
pub fn parse_enforcement(s: &str) -> Option<EnforcementMode> {
    EnforcementMode::parse(&s.trim().replace('-', "_"))
}

/// `--visibility`: the PCE's names (`flow_summary`, `flow_drops`,
/// `flow_off`, `enhanced_data_collection`), as the rest of the CLI takes
/// them, or the UI's (`blocked-allowed`, `blocked`, `off`, `enhanced`).
pub fn parse_visibility(s: &str) -> Option<VisibilityLevel> {
    let s = s.trim();
    VisibilityLevel::parse(s).or(match s.replace('_', "-").as_str() {
        "blocked-allowed" => Some(VisibilityLevel::FlowSummary),
        "blocked" => Some(VisibilityLevel::FlowDrops),
        "off" => Some(VisibilityLevel::FlowOff),
        "enhanced" => Some(VisibilityLevel::EnhancedDataCollection),
        _ => None,
    })
}

/// `--lock role,app,...`: the PCE fields the names set, in [`LOCKS`]
/// order, each once. An unknown name is an error naming the known ones.
pub fn parse_locks(specs: &[String]) -> Result<Vec<&'static str>, String> {
    let mut on = Vec::new();
    for name in specs.iter().flat_map(|s| s.split(',')).map(str::trim) {
        if name.is_empty() {
            continue;
        }
        let Some((_, field)) = LOCKS.iter().find(|(n, _)| *n == name) else {
            let known: Vec<&str> = LOCKS.iter().map(|(n, _)| *n).collect();
            return Err(format!(
                "--lock: unknown lock {name:?}; known: {}",
                known.join(", ")
            ));
        };
        if !on.contains(field) {
            on.push(*field);
        }
    }
    Ok(LOCKS
        .iter()
        .map(|(_, f)| *f)
        .filter(|f| on.contains(f))
        .collect())
}

/// A new pairing profile: `None` (or empty) is not sent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PairingProfileDraft {
    pub name: String,
    pub description: Option<String>,
    /// Label hrefs, at most one per key (the caller resolves them).
    pub labels: Vec<String>,
    pub enforcement: Option<EnforcementMode>,
    pub visibility: Option<VisibilityLevel>,
    pub ven_type: Option<VenType>,
    pub uses: Option<Limit>,
    pub lifespan: Option<Limit>,
    /// Sends `enabled: false`; the PCE's default is enabled.
    pub disabled: bool,
    /// The `*_lock` fields to set to `true` ([`parse_locks`]).
    pub locks: Vec<&'static str>,
}

impl PairingProfileDraft {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// The `POST` body: `name` and what was set.
    pub fn to_body(&self) -> Value {
        let mut v = Map::new();
        v.insert("name".into(), json!(self.name.trim()));
        if let Some(d) = &self.description {
            v.insert("description".into(), json!(d));
        }
        if !self.labels.is_empty() {
            let refs: Vec<Value> = self.labels.iter().map(|h| json!({"href": h})).collect();
            v.insert("labels".into(), Value::Array(refs));
        }
        if let Some(m) = self.enforcement {
            v.insert("enforcement_mode".into(), json!(m.as_str()));
        }
        if let Some(l) = self.visibility {
            v.insert("visibility_level".into(), json!(l.as_str()));
        }
        if let Some(t) = self.ven_type {
            v.insert("ven_type".into(), json!(t.as_str()));
        }
        if let Some(u) = self.uses {
            v.insert("allowed_uses_per_key".into(), u.to_json());
        }
        if let Some(l) = self.lifespan {
            v.insert("key_lifespan".into(), l.to_json());
        }
        if self.disabled {
            v.insert("enabled".into(), json!(false));
        }
        for field in &self.locks {
            v.insert((*field).into(), json!(true));
        }
        Value::Object(v)
    }
}

/// Of `profiles`, the one named exactly `name`, if any.
pub fn find_by_name<'a>(profiles: &'a [PairingProfile], name: &str) -> Option<&'a PairingProfile> {
    profiles.iter().find(|p| p.name == name.trim())
}

impl Writer<'_> {
    /// The create plan, no request: `existing` is the pairing profile list
    /// the caller read; a name already in it is refused.
    pub fn plan_create_pairing_profile(
        &self,
        draft: &PairingProfileDraft,
        existing: &[PairingProfile],
    ) -> Result<Plan, WriteError> {
        let name = draft.name.trim();
        if name.is_empty() {
            return Err(WriteError::invalid(
                "pairing profile name must not be empty",
            ));
        }
        if let Some(p) = find_by_name(existing, name) {
            return Err(WriteError::invalid(format!(
                "a pairing profile named {name:?} exists: {}",
                p.href
            )));
        }
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Post,
                path: self.pce().client().org_path("pairing_profiles"),
                body: Some(draft.to_body()),
                summary: format!("create pairing_profile {name}"),
                objects: Vec::new(),
            },
            kind: ObjectKind::PairingProfile,
        })
    }

    /// `DELETE` on a pairing profile href (`/orgs/N/pairing_profiles/ID`);
    /// `label` names it in the summary.
    pub fn plan_delete_pairing_profile(&self, href: &str, label: &str) -> Result<Plan, WriteError> {
        let href = checked_pairing_href(href, self.pce().client().org_id())?;
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Delete,
                path: href,
                body: None,
                summary: format!("delete pairing_profile {label}"),
                objects: Vec::new(),
            },
            kind: ObjectKind::PairingProfile,
        })
    }
}

/// [`checked_href`] without the `sec_policy` form: pairing profiles are
/// not policy objects.
pub fn checked_pairing_href(href: &str, org_id: u64) -> Result<String, WriteError> {
    let out = checked_href(href, "pairing_profiles", org_id)?;
    if out.contains("/sec_policy/") {
        return Err(WriteError::invalid(format!(
            "{:?} is not a pairing_profiles href (expected /orgs/<org>/pairing_profiles/<id>)",
            href.trim()
        )));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full() -> PairingProfileDraft {
        PairingProfileDraft {
            name: " shop-prod ".into(),
            description: Some("shop web tier".into()),
            labels: vec!["/orgs/5/labels/1".into(), "/orgs/5/labels/3".into()],
            enforcement: Some(EnforcementMode::VisibilityOnly),
            visibility: Some(VisibilityLevel::FlowDrops),
            ven_type: Some(VenType::SpecifiedDuringActivation),
            uses: Some(Limit::Count(5)),
            lifespan: Some(Limit::Count(604_800)),
            disabled: true,
            locks: vec!["role_label_lock", "log_traffic_lock"],
        }
    }

    #[test]
    fn a_name_alone_sends_the_name_alone() {
        assert_eq!(
            PairingProfileDraft::new("p").to_body(),
            json!({"name": "p"}),
            "the PCE's defaults for everything else"
        );
    }

    #[test]
    fn every_setting_maps_to_its_pce_field() {
        assert_eq!(
            full().to_body(),
            json!({
                "name": "shop-prod",
                "description": "shop web tier",
                "labels": [{"href": "/orgs/5/labels/1"}, {"href": "/orgs/5/labels/3"}],
                "enforcement_mode": "visibility_only",
                "visibility_level": "flow_drops",
                "ven_type": "specified_during_activation",
                "allowed_uses_per_key": 5,
                "key_lifespan": 604_800,
                "enabled": false,
                "role_label_lock": true,
                "log_traffic_lock": true,
            })
        );
        let d = PairingProfileDraft {
            uses: Some(Limit::Unlimited),
            lifespan: Some(Limit::Unlimited),
            ..PairingProfileDraft::new("p")
        };
        assert_eq!(
            d.to_body(),
            json!({"name": "p", "allowed_uses_per_key": "unlimited", "key_lifespan": "unlimited"})
        );
    }

    #[test]
    fn flag_values_parse() {
        assert_eq!(
            parse_enforcement("visibility-only"),
            Some(EnforcementMode::VisibilityOnly)
        );
        assert_eq!(
            parse_enforcement("visibility_only"),
            Some(EnforcementMode::VisibilityOnly)
        );
        assert_eq!(parse_enforcement("idle"), Some(EnforcementMode::Idle));
        assert_eq!(parse_enforcement("full"), Some(EnforcementMode::Full));
        assert_eq!(parse_enforcement("enforced"), None);
        for (given, level) in [
            ("blocked-allowed", VisibilityLevel::FlowSummary),
            ("flow_summary", VisibilityLevel::FlowSummary),
            ("blocked", VisibilityLevel::FlowDrops),
            ("flow_drops", VisibilityLevel::FlowDrops),
            ("off", VisibilityLevel::FlowOff),
            ("flow_off", VisibilityLevel::FlowOff),
            ("enhanced", VisibilityLevel::EnhancedDataCollection),
            (
                "enhanced_data_collection",
                VisibilityLevel::EnhancedDataCollection,
            ),
        ] {
            assert_eq!(parse_visibility(given), Some(level), "{given}");
        }
        assert_eq!(parse_visibility("all"), None);
        assert_eq!(
            VenType::parse("specified-during-activation"),
            Some(VenType::SpecifiedDuringActivation)
        );
        assert_eq!(VenType::parse("server"), Some(VenType::Server));
        assert_eq!(VenType::parse("endpoint"), Some(VenType::Endpoint));
        assert_eq!(VenType::parse("laptop"), None);
    }

    #[test]
    fn uses_are_positive_or_unlimited() {
        assert_eq!(Limit::parse_uses("1"), Ok(Limit::Count(1)));
        assert_eq!(Limit::parse_uses("Unlimited"), Ok(Limit::Unlimited));
        for bad in ["0", "-1", "x", "", "1.5"] {
            assert!(Limit::parse_uses(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn lifespans_are_seconds_or_durations() {
        assert_eq!(Limit::parse_lifespan("3600"), Ok(Limit::Count(3_600)));
        assert_eq!(Limit::parse_lifespan("1h"), Ok(Limit::Count(3_600)));
        assert_eq!(Limit::parse_lifespan("7d"), Ok(Limit::Count(604_800)));
        assert_eq!(Limit::parse_lifespan("1h 30m"), Ok(Limit::Count(5_400)));
        assert_eq!(Limit::parse_lifespan("unlimited"), Ok(Limit::Unlimited));
        for bad in ["0", "0s", "soon", "", "1500ms", "-5"] {
            assert!(Limit::parse_lifespan(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn locks_map_to_fields_in_order_and_unknown_ones_fail() {
        assert_eq!(
            parse_locks(&[
                "visibility,role".into(),
                "log-traffic".into(),
                "role".into()
            ]),
            Ok(vec![
                "role_label_lock",
                "visibility_level_lock",
                "log_traffic_lock"
            ])
        );
        assert_eq!(
            parse_locks(&["role,app,env,loc,enforcement,visibility,log-traffic".into()])
                .unwrap()
                .len(),
            7
        );
        let e = parse_locks(&["role,colour".into()]).unwrap_err();
        assert!(e.contains("\"colour\"") && e.contains("log-traffic"), "{e}");
    }

    #[test]
    fn hrefs_are_checked() {
        assert_eq!(
            checked_pairing_href("/orgs/5/pairing_profiles/7/", 5).unwrap(),
            "/orgs/5/pairing_profiles/7"
        );
        assert!(checked_pairing_href("/orgs/6/pairing_profiles/7", 5).is_err());
        assert!(checked_pairing_href("/orgs/5/labels/7", 5).is_err());
        assert!(checked_pairing_href("/orgs/5/sec_policy/draft/pairing_profiles/7", 5).is_err());
        assert!(checked_pairing_href("/orgs/5/pairing_profiles/../labels", 5).is_err());
    }
}
