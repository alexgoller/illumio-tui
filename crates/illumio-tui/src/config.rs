//! Where connection settings come from, in order: command-line flag, then
//! environment variable (same names as illumio-mcp-server), then the
//! selected profile in `~/.config/illumio-tui/config.toml`, then defaults.
//! `writes` (profile) or `ILLUMIO_WRITES` (env) enables write commands; the
//! CLI still sends nothing without `--yes`, the TUI nothing before `:writes on`
//! and a confirmed request.
//! `[profiles.<name>.labels]` (or `ILLUMIO_SCOPE_KEYS`, `ILLUMIO_TIER_KEY`)
//! names the label model the TUI writes rules in; its `ignore_keys` (or
//! `ILLUMIO_IGNORE_KEYS`) hides housekeeping keys from label columns.
//! The tool never writes secrets anywhere.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ConfigFile {
    pub default_profile: Option<String>,
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
    /// Keys this version does not know, kept so `profile use` can rewrite
    /// the file without dropping them.
    #[serde(default, flatten)]
    pub extra: toml::Table,
}

#[derive(Default, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub org_id: Option<u64>,
    pub scheme: Option<String>,
    pub tls_verify: Option<bool>,
    pub api_key: Option<String>,
    pub api_secret: Option<String>,
    /// Shell command whose stdout is the API secret (e.g. `op read ...`).
    pub api_secret_cmd: Option<String>,
    /// Allow write commands against this PCE (the CLI still needs --yes to send).
    pub writes: Option<bool>,
    #[serde(default, skip_serializing_if = "Limits::is_empty")]
    pub limits: Limits,
    /// The label model rules are written in (`[profiles.<name>.labels]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labels: Option<LabelsConfig>,
    /// Unknown keys, round-tripped verbatim.
    #[serde(default, flatten)]
    pub extra: toml::Table,
}

/// Manual `Debug` so a profile never prints its plaintext secret, even
/// through `{:?}` in a log line or an error message.
impl std::fmt::Debug for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Profile")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("org_id", &self.org_id)
            .field("scheme", &self.scheme)
            .field("tls_verify", &self.tls_verify)
            .field("api_key", &self.api_key)
            .field(
                "api_secret",
                &self.api_secret.as_ref().map(|_| "<redacted>"),
            )
            .field("api_secret_cmd", &self.api_secret_cmd)
            .field("writes", &self.writes)
            .field("limits", &self.limits)
            .field("labels", &self.labels)
            // Unknown keys could be anything; print their names only.
            .field("extra", &self.extra.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Limits {
    pub requests_per_second: Option<f64>,
    pub auto_refresh_min_seconds: Option<u64>,
    pub traffic_max_results: Option<u32>,
    pub traffic_default_window: Option<String>,
}

impl Limits {
    pub fn is_empty(&self) -> bool {
        self.requests_per_second.is_none()
            && self.auto_refresh_min_seconds.is_none()
            && self.traffic_max_results.is_none()
            && self.traffic_default_window.is_none()
    }
}

/// Rulesets are scoped by `scope_keys` (`app`, `env`); rules select tiers
/// by `tier_key` (`role`). Used by the TUI's `w` pre-fill and the label key
/// order. `ignore_keys` are hidden from label text (columns, the `g` picker);
/// JSON output, the edit form and the Labels tab keep them.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct LabelsConfig {
    pub scope_keys: Option<Vec<String>>,
    pub tier_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignore_keys: Option<Vec<String>>,
}

pub const DEFAULT_SCOPE_KEYS: [&str; 2] = ["app", "env"];
pub const DEFAULT_TIER_KEY: &str = "role";

/// Trimmed, empty entries dropped; `None` when nothing is left.
fn key_list<'a>(items: impl Iterator<Item = &'a str>) -> Option<Vec<String>> {
    let keys: Vec<String> = items
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(String::from)
        .collect();
    (!keys.is_empty()).then_some(keys)
}

/// Values given on the command line.
#[derive(Debug, Default, Clone)]
pub struct Overrides {
    pub profile: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub org_id: Option<u64>,
    pub tls_verify: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub profile_name: String,
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub org_id: u64,
    pub tls_verify: bool,
    pub credentials: pce::Credentials,
    pub requests_per_second: f64,
    pub auto_refresh_min_seconds: u64,
    pub traffic_max_results: u32,
    pub traffic_default_window: String,
    pub writes: bool,
    /// Label keys a ruleset scope is made of, in order.
    pub scope_keys: Vec<String>,
    /// The label key that names a tier inside a scope.
    pub tier_key: String,
    /// Label keys hidden from label text; never a scope key or the tier key.
    pub ignore_keys: Vec<String>,
}

impl ResolvedConfig {
    /// The order label keys are printed in: `scope_keys`, `tier_key`, then
    /// the rest alphabetically; `ignore_keys` hidden from label text.
    pub fn key_order(&self) -> pce::KeyOrder {
        pce::KeyOrder::new(&self.scope_keys, &self.tier_key).with_ignored(&self.ignore_keys)
    }

    pub fn client_config(&self) -> pce::ClientConfig {
        let mut c =
            pce::ClientConfig::new(self.host.clone(), self.org_id, self.credentials.clone());
        c.scheme = self.scheme.clone();
        c.port = self.port;
        c.tls_verify = self.tls_verify;
        c.requests_per_second = self.requests_per_second;
        c
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{0}")]
    Message(String),
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Reports the parser's message and byte offset only: the `Display` of
    /// `toml::de::Error` quotes the offending line, which may hold a secret.
    #[error("cannot parse {}: {}", path.display(), parse_summary(source))]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
}

fn parse_summary(e: &toml::de::Error) -> String {
    match e.span() {
        Some(span) => format!("{} at byte {}", e.message(), span.start),
        None => e.message().to_string(),
    }
}

pub fn default_config_path() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("illumio-tui").join("config.toml");
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("illumio-tui")
        .join("config.toml")
}

/// A missing file is an empty config; a present but broken file is an error.
pub fn load_config_file(path: &Path) -> Result<ConfigFile, ConfigError> {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ConfigFile::default()),
        Err(source) => Err(ConfigError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Reads a dotenv file into a map without touching the process environment.
pub fn load_env_file(path: &Path) -> Result<HashMap<String, String>, ConfigError> {
    let iter = dotenvy::from_path_iter(path).map_err(|e| {
        ConfigError::Message(format!("cannot read env file {}: {e}", path.display()))
    })?;
    let mut map = HashMap::new();
    for item in iter {
        let (k, v) = item.map_err(|e| {
            // `LineParse` displays the raw line, which may be a secret.
            let what = match e {
                dotenvy::Error::LineParse(_, idx) => format!("parse error at index {idx}"),
                other => other.to_string(),
            };
            ConfigError::Message(format!("bad line in {} ({what})", path.display()))
        })?;
        map.insert(k, v);
    }
    Ok(map)
}

/// Real environment variables win over the env file.
pub fn merge_env(
    file: HashMap<String, String>,
    real: HashMap<String, String>,
) -> HashMap<String, String> {
    let mut merged = file;
    merged.extend(real);
    merged
}

pub fn resolve(
    file: &ConfigFile,
    overrides: &Overrides,
    env: &HashMap<String, String>,
) -> Result<ResolvedConfig, ConfigError> {
    let env_str = |k: &str| {
        env.get(k)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };

    // `default_profile` in the file is as much an explicit request as
    // `--profile` or `ILLUMIO_PROFILE`: naming an unknown profile through any
    // of the three is always an error. Only the implicit fallback name
    // "default" (nothing named a profile at all) may resolve to
    // `Profile::default()` when no such profile is configured.
    let explicit_profile_name = overrides
        .profile
        .clone()
        .or_else(|| env_str("ILLUMIO_PROFILE"))
        .or_else(|| file.default_profile.clone());
    let (profile_name, is_explicit) = match explicit_profile_name {
        Some(name) => (name, true),
        None => ("default".to_string(), false),
    };
    let profile = match file.profiles.get(&profile_name) {
        Some(p) => p.clone(),
        None if is_explicit => {
            return Err(ConfigError::Message(format!(
                "profile {profile_name:?} not found in config; known: {}",
                file.profiles.keys().cloned().collect::<Vec<_>>().join(", ")
            )));
        }
        None => Profile::default(),
    };

    let host = overrides
        .host
        .clone()
        .or_else(|| env_str("PCE_HOST"))
        .or_else(|| profile.host.clone())
        .ok_or_else(|| {
            ConfigError::Message(
                "no PCE host: pass --host, set PCE_HOST, or add host to the profile".into(),
            )
        })?;
    let port = if let Some(p) = overrides.port {
        p
    } else if let Some(raw) = env_str("PCE_PORT") {
        raw.parse::<u16>()
            .map_err(|_| ConfigError::Message(format!("PCE_PORT is not a number: {raw:?}")))?
    } else {
        profile.port.unwrap_or(443)
    };
    let org_id = if let Some(o) = overrides.org_id {
        o
    } else if let Some(raw) = env_str("PCE_ORG_ID") {
        raw.parse::<u64>()
            .map_err(|_| ConfigError::Message(format!("PCE_ORG_ID is not a number: {raw:?}")))?
    } else {
        profile.org_id.ok_or_else(|| {
            ConfigError::Message(
                "no org id: pass --org-id, set PCE_ORG_ID, or add org_id to the profile".into(),
            )
        })?
    };
    let scheme = env_str("PCE_SCHEME")
        .or_else(|| profile.scheme.clone())
        .unwrap_or_else(|| "https".into());
    let tls_verify = overrides
        .tls_verify
        .or_else(|| env_str("PCE_TLS_VERIFY").and_then(|v| parse_bool(&v)))
        .or(profile.tls_verify)
        .unwrap_or(true);

    let api_key = env_str("API_KEY")
        .or_else(|| profile.api_key.clone())
        .ok_or_else(|| {
            ConfigError::Message("no API key: set API_KEY or add api_key to the profile".into())
        })?;
    let api_secret = match env_str("API_SECRET").or_else(|| profile.api_secret.clone()) {
        Some(s) => s,
        None => match &profile.api_secret_cmd {
            Some(cmd) => run_secret_cmd(cmd)?,
            None => return Err(ConfigError::Message(
                "no API secret: set API_SECRET, or add api_secret or api_secret_cmd to the profile"
                    .into(),
            )),
        },
    };

    let labels = profile.labels.clone().unwrap_or_default();
    let scope_keys = match env_str("ILLUMIO_SCOPE_KEYS") {
        Some(raw) => key_list(raw.split(',')).ok_or_else(|| {
            ConfigError::Message("ILLUMIO_SCOPE_KEYS must name at least one key".into())
        })?,
        None => match &labels.scope_keys {
            Some(list) => key_list(list.iter().map(String::as_str)).ok_or_else(|| {
                ConfigError::Message("labels.scope_keys must name at least one key".into())
            })?,
            None => DEFAULT_SCOPE_KEYS.map(String::from).to_vec(),
        },
    };
    let tier_key = env_str("ILLUMIO_TIER_KEY")
        .or_else(|| {
            labels
                .tier_key
                .as_deref()
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .map(String::from)
        })
        .unwrap_or_else(|| DEFAULT_TIER_KEY.into());
    // An empty list is fine (ignore nothing), so a blank env falls through
    // to the profile. Scope and tier keys are the model: never ignored.
    let ignore_keys: Vec<String> =
        match env_str("ILLUMIO_IGNORE_KEYS").and_then(|raw| key_list(raw.split(','))) {
            Some(list) => list,
            None => labels
                .ignore_keys
                .as_deref()
                .and_then(|list| key_list(list.iter().map(String::as_str)))
                .unwrap_or_default(),
        };
    let mut ignored: Vec<String> = Vec::new();
    for k in ignore_keys {
        if k != tier_key && !scope_keys.contains(&k) && !ignored.contains(&k) {
            ignored.push(k);
        }
    }

    Ok(ResolvedConfig {
        profile_name,
        scheme,
        host,
        port,
        org_id,
        tls_verify,
        credentials: pce::Credentials::new(api_key, api_secret),
        requests_per_second: profile.limits.requests_per_second.unwrap_or(2.0),
        auto_refresh_min_seconds: profile
            .limits
            .auto_refresh_min_seconds
            .unwrap_or(60)
            .max(60),
        traffic_max_results: profile.limits.traffic_max_results.unwrap_or(1000),
        traffic_default_window: profile
            .limits
            .traffic_default_window
            .clone()
            .unwrap_or_else(|| "24h".into()),
        writes: match env_str("ILLUMIO_WRITES") {
            Some(raw) => parse_bool(&raw).ok_or_else(|| {
                ConfigError::Message(format!("ILLUMIO_WRITES is not a boolean: {raw:?}"))
            })?,
            None => profile.writes.unwrap_or(false),
        },
        scope_keys,
        tier_key,
        ignore_keys: ignored,
    })
}

pub fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn run_secret_cmd(cmd: &str) -> Result<String, ConfigError> {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .map_err(|e| ConfigError::Message(format!("api_secret_cmd could not start: {e}")))?;
    if !out.status.success() {
        return Err(ConfigError::Message(format!(
            "api_secret_cmd exited with {}",
            out.status
        )));
    }
    let secret = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if secret.is_empty() {
        return Err(ConfigError::Message(
            "api_secret_cmd printed an empty secret".into(),
        ));
    }
    Ok(secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn file() -> ConfigFile {
        toml::from_str(
            r#"
default_profile = "demo"

[profiles.demo]
host = "demo.example"
port = 8443
org_id = 7
tls_verify = false
api_key = "file-key"
api_secret = "file-secret"

[profiles.demo.limits]
requests_per_second = 1.5
traffic_max_results = 250
traffic_default_window = "6h"

[profiles.cmd]
host = "cmd.example"
org_id = 9
api_key = "k"
api_secret_cmd = "echo   from-command  "
"#,
        )
        .unwrap()
    }

    #[test]
    fn profile_values_and_limit_defaults() {
        let c = resolve(&file(), &Overrides::default(), &env(&[])).unwrap();
        assert_eq!(c.profile_name, "demo");
        assert_eq!(c.host, "demo.example");
        assert_eq!(c.port, 8443);
        assert_eq!(c.org_id, 7);
        assert!(!c.tls_verify);
        assert_eq!(c.scheme, "https");
        assert_eq!(c.credentials.api_key(), "file-key");
        assert_eq!(c.credentials.expose_secret(), "file-secret");
        assert_eq!(c.requests_per_second, 1.5);
        assert_eq!(c.auto_refresh_min_seconds, 60);
        assert_eq!(c.traffic_max_results, 250);
        assert_eq!(c.traffic_default_window, "6h");
    }

    #[test]
    fn flag_beats_env_beats_profile() {
        let e = env(&[
            ("PCE_HOST", "env.example"),
            ("PCE_PORT", "9443"),
            ("API_KEY", "env-key"),
            ("API_SECRET", "env-secret"),
            ("PCE_TLS_VERIFY", "true"),
        ]);
        let o = Overrides {
            host: Some("flag.example".into()),
            ..Default::default()
        };
        let c = resolve(&file(), &o, &e).unwrap();
        assert_eq!(c.host, "flag.example");
        assert_eq!(c.port, 9443);
        assert_eq!(c.org_id, 7, "org id falls through to the profile");
        assert!(c.tls_verify);
        assert_eq!(c.credentials.api_key(), "env-key");
        assert_eq!(c.credentials.expose_secret(), "env-secret");
    }

    #[test]
    fn env_only_works_without_a_config_file() {
        let e = env(&[
            ("PCE_HOST", "h"),
            ("PCE_ORG_ID", "3"),
            ("API_KEY", "k"),
            ("API_SECRET", "s"),
            ("PCE_SCHEME", "http"),
        ]);
        let c = resolve(&ConfigFile::default(), &Overrides::default(), &e).unwrap();
        assert_eq!(c.profile_name, "default");
        assert_eq!(c.port, 443);
        assert_eq!(c.scheme, "http");
        assert!(c.tls_verify);
        assert_eq!(c.requests_per_second, 2.0);
        assert_eq!(c.traffic_max_results, 1000);
        assert_eq!(c.traffic_default_window, "24h");
    }

    #[test]
    fn missing_values_name_the_env_var() {
        let err = resolve(&ConfigFile::default(), &Overrides::default(), &env(&[]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("PCE_HOST"), "{err}");
        let e = env(&[("PCE_HOST", "h"), ("PCE_ORG_ID", "3"), ("API_KEY", "k")]);
        let err = resolve(&ConfigFile::default(), &Overrides::default(), &e)
            .unwrap_err()
            .to_string();
        assert!(err.contains("API_SECRET"), "{err}");
        let err = resolve(
            &file(),
            &Overrides {
                profile: Some("nope".into()),
                ..Default::default()
            },
            &env(&[]),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("profile \"nope\""), "{err}");
    }

    #[test]
    fn secret_command_is_run_and_trimmed() {
        let o = Overrides {
            profile: Some("cmd".into()),
            ..Default::default()
        };
        let c = resolve(&file(), &o, &env(&[])).unwrap();
        assert_eq!(c.credentials.expose_secret(), "from-command");
    }

    #[test]
    fn failing_or_empty_secret_command_is_an_error() {
        let mut f = file();
        f.profiles.get_mut("cmd").unwrap().api_secret_cmd = Some("exit 3".into());
        let o = Overrides {
            profile: Some("cmd".into()),
            ..Default::default()
        };
        let err = resolve(&f, &o, &env(&[])).unwrap_err().to_string();
        assert!(err.contains("api_secret_cmd"), "{err}");
        f.profiles.get_mut("cmd").unwrap().api_secret_cmd = Some("true".into());
        let err = resolve(&f, &o, &env(&[])).unwrap_err().to_string();
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn default_profile_pointing_at_missing_profile_is_an_error() {
        let f = ConfigFile {
            default_profile: Some("ghost".into()),
            ..Default::default()
        };
        let err = resolve(&f, &Overrides::default(), &env(&[]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("profile \"ghost\""), "{err}");
    }

    #[test]
    fn writes_default_off_profile_enables_env_outranks() {
        let f = file();
        let r = resolve(&f, &Overrides::default(), &env(&[])).unwrap();
        assert!(!r.writes);
        let mut f2 = f.clone();
        f2.profiles.get_mut("demo").unwrap().writes = Some(true);
        assert!(
            resolve(&f2, &Overrides::default(), &env(&[]))
                .unwrap()
                .writes
        );
        let r = resolve(
            &f2,
            &Overrides::default(),
            &env(&[("ILLUMIO_WRITES", "false")]),
        )
        .unwrap();
        assert!(!r.writes, "env outranks the profile");
        let r = resolve(&f, &Overrides::default(), &env(&[("ILLUMIO_WRITES", "1")])).unwrap();
        assert!(r.writes);
        let shown = format!("{:?}", f2.profiles["demo"]);
        assert!(shown.contains("writes: Some(true)"), "{shown}");
        assert!(!shown.contains("file-secret"));
    }

    #[test]
    fn label_model_defaults_profile_values_and_env_outranks() {
        let r = resolve(&file(), &Overrides::default(), &env(&[])).unwrap();
        assert_eq!(r.scope_keys, vec!["app", "env"]);
        assert_eq!(r.tier_key, "role");
        let f: ConfigFile = toml::from_str(
            r#"
[profiles.default]
host = "h"
org_id = 1
api_key = "k"
api_secret = "s"

[profiles.default.labels]
scope_keys = ["app", " site ", ""]
tier_key = "tier"
"#,
        )
        .unwrap();
        let r = resolve(&f, &Overrides::default(), &env(&[])).unwrap();
        assert_eq!(
            r.scope_keys,
            vec!["app", "site"],
            "trimmed, empty entries dropped"
        );
        assert_eq!(r.tier_key, "tier");
        let r = resolve(
            &f,
            &Overrides::default(),
            &env(&[
                ("ILLUMIO_SCOPE_KEYS", " env , ,loc "),
                ("ILLUMIO_TIER_KEY", " role "),
            ]),
        )
        .unwrap();
        assert_eq!(r.scope_keys, vec!["env", "loc"], "env outranks the profile");
        assert_eq!(r.tier_key, "role");
        let shown = format!("{:?}", f.profiles["default"]);
        assert!(shown.contains("tier_key: Some(\"tier\")"), "{shown}");
    }

    #[test]
    fn ignore_keys_profile_env_and_label_model_keys() {
        let r = resolve(&file(), &Overrides::default(), &env(&[])).unwrap();
        assert!(r.ignore_keys.is_empty(), "nothing ignored by default");
        let f: ConfigFile = toml::from_str(
            r#"
[profiles.default]
host = "h"
org_id = 1
api_key = "k"
api_secret = "s"

[profiles.default.labels]
scope_keys = ["app", "env"]
tier_key = "role"
ignore_keys = [" DFIRBubble ", "", "os", "DFIRBubble", "app", "role", "bu"]
"#,
        )
        .unwrap();
        let r = resolve(&f, &Overrides::default(), &env(&[])).unwrap();
        assert_eq!(
            r.ignore_keys,
            vec!["DFIRBubble", "os", "bu"],
            "trimmed, blanks and repeats dropped, scope and tier keys dropped, order kept"
        );
        let r = resolve(
            &f,
            &Overrides::default(),
            &env(&[("ILLUMIO_IGNORE_KEYS", " os , ,zone,env")]),
        )
        .unwrap();
        assert_eq!(
            r.ignore_keys,
            vec!["os", "zone"],
            "env outranks the profile"
        );
        for blank in ["", " , ,"] {
            let r = resolve(
                &f,
                &Overrides::default(),
                &env(&[("ILLUMIO_IGNORE_KEYS", blank)]),
            )
            .unwrap();
            assert_eq!(
                r.ignore_keys,
                vec!["DFIRBubble", "os", "bu"],
                "an empty env is unset: {blank:?}"
            );
        }
        let r = resolve(
            &f,
            &Overrides::default(),
            &env(&[("ILLUMIO_SCOPE_KEYS", "os"), ("ILLUMIO_TIER_KEY", "bu")]),
        )
        .unwrap();
        assert_eq!(
            r.ignore_keys,
            vec!["DFIRBubble", "app", "role"],
            "the resolved label model decides what is dropped"
        );
    }

    #[test]
    fn an_empty_scope_key_list_is_refused() {
        let err = resolve(
            &file(),
            &Overrides::default(),
            &env(&[("ILLUMIO_SCOPE_KEYS", " , ,")]),
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "ILLUMIO_SCOPE_KEYS must name at least one key"
        );
        let mut f = file();
        f.profiles.get_mut("demo").unwrap().labels = Some(LabelsConfig {
            scope_keys: Some(vec![" ".into()]),
            tier_key: None,
            ignore_keys: None,
        });
        let err = resolve(&f, &Overrides::default(), &env(&[])).unwrap_err();
        assert_eq!(
            err.to_string(),
            "labels.scope_keys must name at least one key"
        );
    }

    #[test]
    fn unparsable_illumio_writes_is_a_config_error() {
        let mut f = file();
        f.profiles.get_mut("demo").unwrap().writes = Some(true);
        let err = resolve(
            &f,
            &Overrides::default(),
            &env(&[("ILLUMIO_WRITES", "maybe")]),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("ILLUMIO_WRITES is not a boolean"),
            "{err}"
        );
    }

    #[test]
    fn profile_debug_redacts_secret() {
        let mut p = Profile {
            api_key: Some("k".into()),
            api_secret: Some("very-secret".into()),
            ..Default::default()
        };
        p.extra
            .insert("mystery".into(), toml::Value::String("hidden-value".into()));
        let out = format!("{p:?}");
        assert!(out.contains('k'), "{out}");
        assert!(out.contains("<redacted>"), "{out}");
        assert!(!out.contains("very-secret"), "{out}");
        assert!(out.contains("mystery"), "{out}");
        assert!(!out.contains("hidden-value"), "{out}");
    }

    #[test]
    fn unknown_keys_round_trip_and_empty_limits_are_omitted() {
        let f: ConfigFile = toml::from_str(
            "top_level_unknown = 1\n[profiles.a]\nhost = \"h\"\nunknown_key = \"kept\"\n",
        )
        .unwrap();
        assert_eq!(f.extra["top_level_unknown"].as_integer(), Some(1));
        assert_eq!(f.profiles["a"].extra["unknown_key"].as_str(), Some("kept"));
        assert!(f.profiles["a"].limits.is_empty());
        let text = toml::to_string_pretty(&f).unwrap();
        assert!(text.contains("unknown_key = \"kept\""), "{text}");
        assert!(text.contains("top_level_unknown = 1"), "{text}");
        assert!(!text.contains("limits"), "{text}");
        let again: ConfigFile = toml::from_str(&text).unwrap();
        assert_eq!(again.profiles["a"].host.as_deref(), Some("h"));
    }

    #[test]
    fn empty_illumio_profile_env_falls_through_to_default() {
        let e = env(&[
            ("ILLUMIO_PROFILE", ""),
            ("PCE_HOST", "h"),
            ("PCE_ORG_ID", "3"),
            ("API_KEY", "k"),
            ("API_SECRET", "s"),
        ]);
        let c = resolve(&ConfigFile::default(), &Overrides::default(), &e).unwrap();
        assert_eq!(c.profile_name, "default");
    }

    #[test]
    fn non_numeric_port_and_org_id_name_the_env_var() {
        let e = env(&[
            ("PCE_HOST", "h"),
            ("PCE_ORG_ID", "abc"),
            ("API_KEY", "k"),
            ("API_SECRET", "s"),
        ]);
        let err = resolve(&ConfigFile::default(), &Overrides::default(), &e)
            .unwrap_err()
            .to_string();
        assert!(err.contains("PCE_ORG_ID"), "{err}");

        let e = env(&[
            ("PCE_HOST", "h"),
            ("PCE_ORG_ID", "3"),
            ("PCE_PORT", "nope"),
            ("API_KEY", "k"),
            ("API_SECRET", "s"),
        ]);
        let err = resolve(&ConfigFile::default(), &Overrides::default(), &e)
            .unwrap_err()
            .to_string();
        assert!(err.contains("PCE_PORT"), "{err}");
    }

    #[test]
    fn broken_config_error_does_not_echo_the_line() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.toml");
        std::fs::write(
            &p,
            "[profiles.a]\nhost = \"h\"\napi_secret = \"SUPERSECRET123\n",
        )
        .unwrap();
        let err = load_config_file(&p).unwrap_err().to_string();
        assert!(err.contains("cannot parse"), "{err}");
        assert!(err.contains("config.toml"), "{err}");
        assert!(!err.contains("SUPERSECRET123"), "{err}");
    }

    #[test]
    fn broken_env_file_error_does_not_echo_the_line() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bad.env");
        std::fs::write(&p, "PCE_HOST=h\nAPI_SECRET=abc def!\n").unwrap();
        let err = load_env_file(&p).unwrap_err().to_string();
        assert!(err.contains("bad line"), "{err}");
        assert!(err.contains("bad.env"), "{err}");
        assert!(!err.contains("abc def"), "{err}");
        assert!(!err.contains("API_SECRET"), "{err}");
    }

    #[test]
    fn env_file_merge_and_missing_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(".env");
        std::fs::write(&p, "PCE_HOST=file.example\nAPI_KEY=fk\n# comment\n").unwrap();
        let from_file = load_env_file(&p).unwrap();
        let merged = merge_env(from_file, env(&[("PCE_HOST", "real.example")]));
        assert_eq!(merged["PCE_HOST"], "real.example");
        assert_eq!(merged["API_KEY"], "fk");
        assert!(
            load_config_file(&dir.path().join("none.toml"))
                .unwrap()
                .profiles
                .is_empty()
        );
        assert!(load_env_file(&dir.path().join("none.env")).is_err());
    }
}
