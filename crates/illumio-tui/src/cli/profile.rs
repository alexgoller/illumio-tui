//! Profile management. Works without credentials and sends no request: it
//! only reads and rewrites the config file. `profile add` writes a secret
//! only when given one, and never prints it.
use super::GlobalArgs;
use super::error::CliError;
use super::output::{
    Meta, OutputFormat, TableRows, choose_format, meta_line, write_json, write_table,
    write_table_csv,
};
use crate::config::{self, ConfigFile};
use clap::{Args, Subcommand, ValueHint};
use serde::Serialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Subcommand, Debug)]
pub enum ProfileCmd {
    /// List profiles in the config file
    List,
    /// Set the default profile
    Use {
        /// Profile name from the config file
        name: String,
    },
    /// Add a profile to the config file (local, no request)
    ///
    /// e.g. `illumio-tui profile add demo --from-env-file path/.env` imports PCE_HOST,
    /// PCE_PORT, PCE_ORG_ID, API_KEY and API_SECRET (also PCE_SCHEME, PCE_TLS_VERIFY); flags
    /// override imported values. Host and org id from --host and --org-id. The secret comes
    /// from the env file, --api-secret-stdin, --api-secret or --api-secret-cmd; a plaintext
    /// secret is stored as api_secret in the config file, which is written mode 0600. The first
    /// profile in a file becomes the default. --force replaces an existing profile's connection
    /// settings given here and keeps the rest (limits, labels, generator, other keys).
    ///
    /// The real environment (PCE_HOST, API_SECRET, ILLUMIO_PROFILE, ...) is ignored: only flags
    /// and --from-env-file count. dotenvy expands $VAR in values that are not single-quoted.
    Add(AddArgs),
}

#[derive(Args, Clone)]
pub struct AddArgs {
    /// Name of the new profile
    pub name: String,
    /// PCE port (default 443; --pce-port works too)
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..))]
    pub port: Option<u16>,
    /// http or https (default https)
    #[arg(long, value_parser = ["http", "https"])]
    pub scheme: Option<String>,
    /// API key (the user name of the API key pair)
    #[arg(long)]
    pub api_key: Option<String>,
    /// API secret, stored as plaintext api_secret. A flag value can land in shell history:
    /// prefer --api-secret-stdin or --from-env-file
    #[arg(long, allow_hyphen_values = true, conflicts_with_all = ["api_secret_stdin", "api_secret_cmd"])]
    pub api_secret: Option<String>,
    /// Read the API secret from stdin (one line), stored as plaintext api_secret
    #[arg(long, conflicts_with = "api_secret_cmd")]
    pub api_secret_stdin: bool,
    /// Shell command whose stdout is the API secret (e.g. `op read ...`); no plaintext
    /// secret is written, even when the env file has API_SECRET
    #[arg(long)]
    pub api_secret_cmd: Option<String>,
    /// dotenv file to import (- reads it from stdin, for pasting; end with Ctrl-D). $VAR in a
    /// value is expanded unless the value is single-quoted
    #[arg(long, value_name = "PATH", value_hint = ValueHint::FilePath)]
    pub from_env_file: Option<PathBuf>,
    /// Allow write commands against this PCE (writes = true)
    #[arg(long)]
    pub writes: bool,
    /// Make it the default profile
    #[arg(long)]
    pub default: bool,
    /// Replace a profile of the same name (keeps its limits, labels, generator, other keys)
    #[arg(long)]
    pub force: bool,
}

/// Manual `Debug` so `--api-secret` never prints.
impl std::fmt::Debug for AddArgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AddArgs")
            .field("name", &self.name)
            .field("port", &self.port)
            .field("scheme", &self.scheme)
            .field("api_key", &self.api_key)
            .field(
                "api_secret",
                &self.api_secret.as_ref().map(|_| "<redacted>"),
            )
            .field("api_secret_stdin", &self.api_secret_stdin)
            .field("api_secret_cmd", &self.api_secret_cmd)
            .field("from_env_file", &self.from_env_file)
            .field("writes", &self.writes)
            .field("default", &self.default)
            .field("force", &self.force)
            .finish()
    }
}

#[derive(Debug, Serialize)]
pub struct ProfileRow {
    pub name: String,
    pub host: String,
    pub org_id: Option<u64>,
    pub is_default: bool,
}

impl TableRows for Vec<ProfileRow> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["name", "host", "org_id", "default"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .map(|p| {
                vec![
                    p.name.clone(),
                    p.host.clone(),
                    p.org_id.map(|o| o.to_string()).unwrap_or_default(),
                    if p.is_default {
                        "*".into()
                    } else {
                        String::new()
                    },
                ]
            })
            .collect()
    }
    /// `default` as `true`/`false`.
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .zip(self.rows())
            .map(|(p, mut row)| {
                row[3] = p.is_default.to_string();
                row
            })
            .collect()
    }
}

#[derive(Debug, Serialize)]
pub struct UseResult {
    pub default_profile: String,
    pub config_path: String,
}

impl TableRows for UseResult {
    fn headers(&self) -> Vec<&'static str> {
        vec!["default_profile", "config_path"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        vec![vec![self.default_profile.clone(), self.config_path.clone()]]
    }
}

/// The profile `profile add` wrote, without its secret: `api_key` and
/// `api_secret` say only whether there is one (`set`, `cmd`, `none`).
#[derive(Debug, Serialize)]
pub struct AddResult {
    pub name: String,
    pub host: String,
    pub port: Option<u16>,
    pub org_id: u64,
    pub api_key: &'static str,
    pub api_secret: &'static str,
    pub is_default: bool,
    pub replaced: bool,
    pub config_path: String,
}

impl TableRows for AddResult {
    fn headers(&self) -> Vec<&'static str> {
        vec![
            "name",
            "host",
            "port",
            "org_id",
            "api_key",
            "api_secret",
            "default",
            "config_path",
        ]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        vec![vec![
            self.name.clone(),
            self.host.clone(),
            self.port.map(|p| p.to_string()).unwrap_or_default(),
            self.org_id.to_string(),
            self.api_key.into(),
            self.api_secret.into(),
            if self.is_default {
                "*".into()
            } else {
                String::new()
            },
            self.config_path.clone(),
        ]]
    }
}

pub(super) fn emit<T: Serialize + TableRows>(
    global: &GlobalArgs,
    data: &T,
) -> Result<(), CliError> {
    let meta = Meta {
        profile: String::new(),
        elapsed_ms: 0,
        api_calls: 0,
        bytes: 0,
        cache: "none",
        truncated: false,
        window: None,
        covered: None,
        cap: None,
        cap_steps: None,
        incomplete: None,
        queries: None,
        draft: None,
        write: None,
        usage: None,
        local_filter: None,
        hidden: None,
        warnings: None,
        marks: None,
        squelched: None,
        cost: None,
        compare: None,
        matched: None,
        where_used: None,
    };
    let mut out = std::io::stdout().lock();
    match choose_format(global.json, global.table, global.csv) {
        OutputFormat::Json => write_json(&mut out, data, &meta)?,
        OutputFormat::Table => {
            write_table(&mut out, data)?;
            eprintln!("{}", meta_line(&meta));
        }
        OutputFormat::Csv => {
            write_table_csv(&mut out, data)?;
            eprintln!("{}", meta_line(&meta));
        }
    }
    Ok(())
}

pub fn run(global: &GlobalArgs, cmd: &ProfileCmd) -> Result<(), CliError> {
    let path = global
        .config
        .clone()
        .unwrap_or_else(config::default_config_path);
    let mut file: ConfigFile = config::load_config_file(&path)?;
    match cmd {
        ProfileCmd::List => {
            let rows: Vec<ProfileRow> = file
                .profiles
                .iter()
                .map(|(name, p)| ProfileRow {
                    name: name.clone(),
                    host: p.host.clone().unwrap_or_default(),
                    org_id: p.org_id,
                    is_default: file.default_profile.as_deref() == Some(name.as_str()),
                })
                .collect();
            emit(global, &rows)
        }
        ProfileCmd::Add(a) => add(global, &path, file, a),
        ProfileCmd::Use { name } => {
            if !file.profiles.contains_key(name) {
                return Err(CliError::Args(format!(
                    "profile {name:?} not found in {}; known: {}",
                    path.display(),
                    file.profiles.keys().cloned().collect::<Vec<_>>().join(", ")
                )));
            }
            file.default_profile = Some(name.clone());
            config::write_config_file(&path, &file)?;
            emit(
                global,
                &UseResult {
                    default_profile: name.clone(),
                    config_path: path.display().to_string(),
                },
            )
        }
    }
}

/// Imports PCE_HOST, PCE_PORT, PCE_ORG_ID, PCE_SCHEME, PCE_TLS_VERIFY,
/// API_KEY and API_SECRET from the env file; other keys are ignored.
fn add(
    global: &GlobalArgs,
    path: &Path,
    mut file: ConfigFile,
    a: &AddArgs,
) -> Result<(), CliError> {
    if global.env_file.is_some() {
        return Err(CliError::Args(
            "profile add imports with --from-env-file, not --env-file".into(),
        ));
    }
    let stdin_env = a.from_env_file.as_deref() == Some(Path::new("-"));
    if stdin_env && a.api_secret_stdin {
        return Err(CliError::Args(
            "--api-secret-stdin and --from-env-file - both read stdin; use one".into(),
        ));
    }
    let port_flag = match (a.port, global.pce_port) {
        (Some(p), Some(q)) if p != q => {
            return Err(CliError::Args(
                "--port and --pce-port disagree; give one".into(),
            ));
        }
        (p, q) => p.or(q),
    };
    if global.pce_port == Some(0) {
        return Err(CliError::Args("--pce-port must be 1-65535".into()));
    }
    let name = a.name.trim();
    if name.is_empty() {
        return Err(CliError::Args("profile name is empty".into()));
    }
    if file.profiles.contains_key(name) && !a.force {
        return Err(CliError::Args(format!(
            "profile {name:?} already exists in {}; --force replaces it",
            path.display()
        )));
    }

    let (env_text, env_name) = match &a.from_env_file {
        None => (String::new(), String::new()),
        Some(_) if stdin_env => {
            if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                eprintln!("paste the env file, then Ctrl-D");
            }
            let mut text = String::new();
            std::io::stdin().read_to_string(&mut text)?;
            (text, "stdin".to_string())
        }
        Some(p) => (config::read_env_file(p)?, p.display().to_string()),
    };
    let env: HashMap<String, String> = config::parse_env_text(&env_text, &env_name)?;
    // Values from the env file are never quoted in an error: it holds a secret.
    let imported = |k: &str| {
        env.get(k)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };

    let host = match global.host.clone().or_else(|| imported("PCE_HOST")) {
        Some(h) if !h.trim().is_empty() => h.trim().to_string(),
        _ => {
            return Err(CliError::Args(
                "no PCE host: pass --host or import PCE_HOST with --from-env-file".into(),
            ));
        }
    };
    if host.contains([':', '/']) || host.contains(char::is_whitespace) {
        return Err(CliError::Args(
            "the host is a hostname only, without scheme, port or path (use --scheme, --port)"
                .into(),
        ));
    }
    let org_id = match global.org_id {
        Some(o) => o,
        None => match imported("PCE_ORG_ID") {
            Some(raw) => raw
                .parse::<u64>()
                .map_err(|_| CliError::Args("PCE_ORG_ID in the env file is not a number".into()))?,
            None => {
                return Err(CliError::Args(
                    "no org id: pass --org-id or import PCE_ORG_ID with --from-env-file".into(),
                ));
            }
        },
    };
    let port = match port_flag {
        Some(p) => Some(p),
        None => match imported("PCE_PORT") {
            Some(raw) => match raw.parse::<u16>() {
                Ok(p) if p > 0 => Some(p),
                _ => {
                    return Err(CliError::Args(
                        "PCE_PORT in the env file is not a port (1-65535)".into(),
                    ));
                }
            },
            None => None,
        },
    };
    let scheme = match a.scheme.clone() {
        Some(s) => Some(s),
        None => match imported("PCE_SCHEME") {
            Some(s) if s == "http" || s == "https" => Some(s),
            Some(_) => {
                return Err(CliError::Args(
                    "PCE_SCHEME in the env file is not http or https".into(),
                ));
            }
            None => None,
        },
    };
    let tls_verify = if global.insecure {
        Some(false)
    } else {
        match imported("PCE_TLS_VERIFY") {
            Some(raw) => Some(config::parse_bool(&raw).ok_or_else(|| {
                CliError::Args("PCE_TLS_VERIFY in the env file is not a boolean".into())
            })?),
            None => None,
        }
    };
    let api_key = a
        .api_key
        .clone()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .or_else(|| imported("API_KEY"));
    // A secret flag beats the env file; `--api-secret-cmd` means no plaintext.
    let (api_secret, api_secret_cmd) = if let Some(cmd) = &a.api_secret_cmd {
        if cmd.trim().is_empty() {
            return Err(CliError::Args("--api-secret-cmd is empty".into()));
        }
        (None, Some(cmd.clone()))
    } else if a.api_secret_stdin {
        if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            eprintln!("type or paste the API secret, then Enter (input is not hidden)");
        }
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        let secret = line.trim().to_string();
        if secret.is_empty() {
            return Err(CliError::Args(
                "--api-secret-stdin: no secret on stdin".into(),
            ));
        }
        (Some(secret), None)
    } else if let Some(s) = &a.api_secret {
        if s.trim().is_empty() {
            return Err(CliError::Args("--api-secret is empty".into()));
        }
        (Some(s.trim().to_string()), None)
    } else {
        (imported("API_SECRET"), None)
    };

    // dotenvy expands `$VAR` in a value that is not single-quoted.
    for key in ["API_KEY", "API_SECRET"] {
        let from_file = match key {
            "API_KEY" => a.api_key.is_none(),
            _ => a.api_secret_cmd.is_none() && !a.api_secret_stdin && a.api_secret.is_none(),
        };
        if from_file && config::env_value_expands(&env_text, key) {
            eprintln!(
                "warning: {key} in {env_name} has a $, which is expanded as a variable; \
                 single-quote the value to keep it literal"
            );
        }
    }

    // A replaced profile keeps everything but the connection settings given
    // here (limits, labels, generator, unknown keys, and what is not given).
    let is_default = a.default || file.profiles.is_empty();
    let replaced = file.profiles.contains_key(name);
    let profile = file.profiles.entry(name.to_string()).or_default();
    profile.host = Some(host.clone());
    profile.org_id = Some(org_id);
    if port.is_some() {
        profile.port = port;
    }
    if scheme.is_some() {
        profile.scheme = scheme;
    }
    if tls_verify.is_some() {
        profile.tls_verify = tls_verify;
    }
    if api_key.is_some() {
        profile.api_key = api_key;
    }
    // One secret source given clears the other.
    if api_secret.is_some() || api_secret_cmd.is_some() {
        profile.api_secret = api_secret;
        profile.api_secret_cmd = api_secret_cmd;
    }
    if a.writes {
        profile.writes = Some(true);
    }
    let port = profile.port;
    let api_key = if profile.api_key.is_some() {
        "set"
    } else {
        "none"
    };
    let secret_state = if profile.api_secret_cmd.is_some() {
        "cmd"
    } else if profile.api_secret.is_some() {
        "set"
    } else {
        "none"
    };
    if is_default {
        file.default_profile = Some(name.to_string());
    }
    config::write_config_file(path, &file)?;
    emit(
        global,
        &AddResult {
            name: name.to_string(),
            host,
            port,
            org_id,
            api_key,
            api_secret: secret_state,
            is_default: file.default_profile.as_deref() == Some(name),
            replaced,
            config_path: path.display().to_string(),
        },
    )
}
