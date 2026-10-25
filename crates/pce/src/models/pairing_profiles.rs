//! Pairing profiles (`/orgs/N/pairing_profiles`): what a VEN paired with
//! one of them starts with. Read leniently: every field but `href` is
//! optional, and fields this model does not name are kept in `extra`.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A label of a pairing profile: the PCE sends the href; key and value
/// only if a version adds them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairingLabel {
    pub href: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairingProfile {
    pub href: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub labels: Vec<PairingLabel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enforcement_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility_level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ven_type: Option<String>,
    /// An integer or `"unlimited"`, as the PCE sends it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_uses_per_key: Option<Value>,
    /// Seconds or `"unlimited"`, as the PCE sends it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_lifespan: Option<Value>,
    /// The `*_lock` flags and everything else (`total_use_count`,
    /// `is_default`, `log_traffic`, timestamps, ...), carried through.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The settings a pairing profile can lock against change at pairing: the
/// CLI name and the PCE field.
pub const LOCKS: [(&str, &str); 7] = [
    ("role", "role_label_lock"),
    ("app", "app_label_lock"),
    ("env", "env_label_lock"),
    ("loc", "loc_label_lock"),
    ("enforcement", "enforcement_mode_lock"),
    ("visibility", "visibility_level_lock"),
    ("log-traffic", "log_traffic_lock"),
];

impl PairingProfile {
    /// The CLI names of the locks set to `true`, in [`LOCKS`] order.
    pub fn locks(&self) -> Vec<&'static str> {
        LOCKS
            .iter()
            .filter(|(_, field)| self.extra.get(*field) == Some(&Value::Bool(true)))
            .map(|(name, _)| *name)
            .collect()
    }
}

/// `unlimited`, a number as text, or empty when absent.
pub fn value_text(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// A key lifespan for people: `unlimited`, else seconds in the largest
/// whole unit (`7d`, `1h`, `90s`).
pub fn lifespan_text(v: Option<&Value>) -> String {
    let Some(secs) = v.and_then(Value::as_u64) else {
        return value_text(v);
    };
    match secs {
        0 => "0s".into(),
        s if s % 86_400 == 0 => format!("{}d", s / 86_400),
        s if s % 3_600 == 0 => format!("{}h", s / 3_600),
        s if s % 60 == 0 => format!("{}m", s / 60),
        s => format!("{s}s"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> Vec<PairingProfile> {
        serde_json::from_str(include_str!("../../../../fixtures/pairing_profiles.json")).unwrap()
    }

    #[test]
    fn a_pce_list_parses_and_keeps_unknown_fields() {
        let p = fixture();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].name, "Default (Servers)");
        assert_eq!(p[0].allowed_uses_per_key, Some(json!("unlimited")));
        assert_eq!(p[0].locks(), ["enforcement", "visibility", "log-traffic"]);
        assert_eq!(p[0].extra["is_default"], json!(true));
        let shop = &p[1];
        assert_eq!(shop.enabled, Some(false));
        assert_eq!(shop.labels.len(), 2);
        assert_eq!(shop.labels[0].href, "/orgs/5/labels/1");
        assert_eq!(
            shop.ven_type.as_deref(),
            Some("specified_during_activation")
        );
        assert_eq!(shop.locks(), ["role", "app", "env", "enforcement"]);
        // A round trip gives back what the PCE sent, unknown fields and all.
        let raw: Value =
            serde_json::from_str(include_str!("../../../../fixtures/pairing_profiles.json"))
                .unwrap();
        let back = serde_json::to_value(&p).unwrap();
        for (got, want) in back.as_array().unwrap().iter().zip(raw.as_array().unwrap()) {
            for (k, v) in want.as_object().unwrap() {
                if !v.is_null() {
                    assert_eq!(&got[k], v, "{k}");
                }
            }
        }
    }

    #[test]
    fn a_bare_profile_parses() {
        let p: PairingProfile =
            serde_json::from_value(json!({"href": "/orgs/5/pairing_profiles/2"})).unwrap();
        assert_eq!(p.name, "");
        assert!(p.labels.is_empty() && p.locks().is_empty() && p.enabled.is_none());
    }

    #[test]
    fn uses_and_lifespans_read_for_people() {
        assert_eq!(value_text(Some(&json!("unlimited"))), "unlimited");
        assert_eq!(value_text(Some(&json!(3))), "3");
        assert_eq!(value_text(None), "");
        assert_eq!(lifespan_text(Some(&json!(604_800))), "7d");
        assert_eq!(lifespan_text(Some(&json!(3_600))), "1h");
        assert_eq!(lifespan_text(Some(&json!(5_400))), "90m");
        assert_eq!(lifespan_text(Some(&json!(61))), "61s");
        assert_eq!(lifespan_text(Some(&json!("unlimited"))), "unlimited");
    }
}
