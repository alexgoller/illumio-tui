//! Everything a command needs: the resolved config, the Pce facade, the
//! output format, and the cache mode.
use super::GlobalArgs;
use super::error::CliError;
use super::output::{
    Meta, OutputFormat, TableRows, Window, WriteMeta, meta_line, write_json, write_table,
    write_table_csv,
};
use crate::config::{self, Overrides};
use crate::names::record::Recorder;
use pce::{CacheMode, Pce, PceClient};
use serde::Serialize;
use std::collections::HashMap;
use std::io::Write;
use std::time::Instant;

pub struct Context {
    pub config: config::ResolvedConfig,
    pub pce: Pce,
    pub format: OutputFormat,
    pub cache_mode: CacheMode,
    /// `marks.toml` next to the config file in use.
    pub marks_path: std::path::PathBuf,
    /// The profile's names cache (shell completion), filled from the
    /// lists read; `None` when `completion_cache = false` or no cache
    /// directory.
    pub names_path: Option<std::path::PathBuf>,
    started: Instant,
}

/// What a profile is resolved from: the config file, the flags and the
/// environment (with `--env-file`).
pub fn config_inputs(
    global: &GlobalArgs,
) -> Result<(config::ConfigFile, Overrides, HashMap<String, String>), CliError> {
    let path = global
        .config
        .clone()
        .unwrap_or_else(config::default_config_path);
    let file = config::load_config_file(&path)?;
    let real: HashMap<String, String> = std::env::vars().collect();
    let env = match &global.env_file {
        Some(p) => config::merge_env(config::load_env_file(p)?, real),
        None => real,
    };
    let overrides = Overrides {
        profile: global.profile.clone(),
        host: global.host.clone(),
        port: global.pce_port,
        org_id: global.org_id,
        tls_verify: if global.insecure { Some(false) } else { None },
    };
    Ok((file, overrides, env))
}

impl Context {
    pub fn new(global: &GlobalArgs) -> Result<Self, CliError> {
        let (file, overrides, env) = config_inputs(global)?;
        let resolved = config::resolve(&file, &overrides, &env)?;
        let mut pce = Pce::new(PceClient::new(resolved.client_config())?)
            .with_key_order(resolved.key_order());
        let names_path = crate::names::path_for(&file, &resolved.profile_name);
        if let Some(path) = &names_path {
            let recorder = Recorder::new(path.clone(), resolved.scope_keys.clone());
            pce = pce.with_observer(std::sync::Arc::new(recorder));
        }
        let format = super::output::choose_format(global.json, global.table, global.csv);
        let cache_mode = if global.no_cache {
            CacheMode::Bypass
        } else {
            CacheMode::Use
        };
        Ok(Self {
            config: resolved,
            pce,
            format,
            cache_mode,
            marks_path: crate::marks::marks_path(global.config.as_deref()),
            names_path,
            started: Instant::now(),
        })
    }

    pub fn meta(&self, cache: &'static str, truncated: bool, window: Option<Window>) -> Meta {
        let s = self.pce.stats();
        Meta {
            profile: self.config.profile_name.clone(),
            elapsed_ms: self.started.elapsed().as_millis(),
            api_calls: s.calls,
            bytes: s.bytes,
            cache,
            truncated,
            window,
            covered: None,
            cap: None,
            cap_steps: None,
            incomplete: None,
            queries: None,
            draft: None,
            write: None,
            usage: None,
            local_filter: None,
            warnings: None,
            marks: None,
            squelched: None,
            cost: None,
        }
    }

    pub fn emit<T: Serialize + TableRows>(
        &self,
        data: &T,
        cache: &'static str,
        truncated: bool,
        window: Option<Window>,
    ) -> Result<(), CliError> {
        self.emit_meta(data, &self.meta(cache, truncated, window))
    }

    /// `emit` with a meta the caller filled in.
    pub fn emit_meta<T: Serialize + TableRows>(
        &self,
        data: &T,
        meta: &Meta,
    ) -> Result<(), CliError> {
        let mut out = std::io::stdout().lock();
        match self.format {
            OutputFormat::Json => write_json(&mut out, data, meta)?,
            OutputFormat::Table => {
                write_table(&mut out, data)?;
                eprintln!("{}", meta_line(meta));
            }
            OutputFormat::Csv => {
                write_table_csv(&mut out, data)?;
                eprintln!("{}", meta_line(meta));
            }
        }
        Ok(())
    }

    /// A report of several tables. JSON: `data` whole. `--table`: each
    /// table under its title line. `--csv`: the tables' full rows (the
    /// caller passes exactly one).
    pub fn emit_report<T: Serialize>(
        &self,
        data: &T,
        tables: &[(String, &dyn TableRows)],
        meta: &Meta,
    ) -> Result<(), CliError> {
        let mut out = std::io::stdout().lock();
        match self.format {
            OutputFormat::Json => write_json(&mut out, data, meta)?,
            OutputFormat::Table => {
                for (i, (title, table)) in tables.iter().enumerate() {
                    if i > 0 {
                        writeln!(out)?;
                    }
                    if !title.is_empty() {
                        writeln!(out, "{title}")?;
                    }
                    write_table(&mut out, *table)?;
                }
                eprintln!("{}", meta_line(meta));
            }
            OutputFormat::Csv => {
                for (_, table) in tables {
                    write_table_csv(&mut out, *table)?;
                }
                eprintln!("{}", meta_line(meta));
            }
        }
        Ok(())
    }

    /// Envelope for a write command: `cache` is always `none`.
    pub fn emit_write(
        &self,
        data: &super::write::WriteOutput,
        write: Option<WriteMeta>,
    ) -> Result<(), CliError> {
        self.emit_write_with(data, write, false)
    }

    /// [`Context::emit_write`] with `meta.truncated` from a read the output
    /// depends on (the workload list behind `--impact`).
    pub fn emit_write_with(
        &self,
        data: &super::write::WriteOutput,
        write: Option<WriteMeta>,
        truncated: bool,
    ) -> Result<(), CliError> {
        let mut meta = self.meta("none", truncated, None);
        meta.write = write;
        let mut out = std::io::stdout().lock();
        match self.format {
            OutputFormat::Json => write_json(&mut out, data, &meta)?,
            // `run` refuses `--csv` on writes before any request.
            OutputFormat::Csv => {
                return Err(CliError::Args("--csv: a write prints no table".into()));
            }
            OutputFormat::Table => {
                write_table(&mut out, data)?;
                eprintln!("{}", meta_line(&meta));
            }
        }
        Ok(())
    }
}

pub fn cache_label(hit: bool, mode: CacheMode) -> &'static str {
    match (mode, hit) {
        (CacheMode::Bypass, _) => "bypass",
        (CacheMode::Use, true) => "hit",
        (CacheMode::Use, false) => "miss",
    }
}
