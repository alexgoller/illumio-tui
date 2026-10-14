//! Profile management. Works without credentials: it only reads and
//! rewrites the config file.
use super::GlobalArgs;
use super::error::CliError;
use super::output::{
    Meta, OutputFormat, TableRows, choose_format, meta_line, write_json, write_table,
    write_table_csv,
};
use crate::config::{self, ConfigFile};
use clap::Subcommand;
use serde::Serialize;

#[derive(Subcommand, Debug)]
pub enum ProfileCmd {
    /// List profiles in the config file
    List,
    /// Set the default profile
    Use { name: String },
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

fn emit<T: Serialize + TableRows>(global: &GlobalArgs, data: &T) -> Result<(), CliError> {
    let meta = Meta {
        profile: String::new(),
        elapsed_ms: 0,
        api_calls: 0,
        bytes: 0,
        cache: "none",
        truncated: false,
        window: None,
        covered: None,
        slices: None,
        queries: None,
        write: None,
        usage: None,
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
        ProfileCmd::Use { name } => {
            if !file.profiles.contains_key(name) {
                return Err(CliError::Args(format!(
                    "profile {name:?} not found in {}; known: {}",
                    path.display(),
                    file.profiles.keys().cloned().collect::<Vec<_>>().join(", ")
                )));
            }
            file.default_profile = Some(name.clone());
            let text = toml::to_string_pretty(&file)
                .map_err(|e| CliError::Other(format!("cannot serialise config: {e}")))?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, text)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            }
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
