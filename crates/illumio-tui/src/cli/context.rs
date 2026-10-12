//! Everything a command needs: the resolved config, the Pce facade, the
//! output format, and the cache mode.
use super::GlobalArgs;
use super::error::CliError;
use super::output::{
    Meta, OutputFormat, TableRows, Window, WriteMeta, meta_line, write_json, write_table,
};
use crate::config::{self, Overrides};
use pce::{CacheMode, KeyOrder, Pce, PceClient};
use serde::Serialize;
use std::collections::HashMap;
use std::time::Instant;

pub struct Context {
    pub config: config::ResolvedConfig,
    pub pce: Pce,
    pub format: OutputFormat,
    pub cache_mode: CacheMode,
    started: Instant,
}

impl Context {
    pub fn new(global: &GlobalArgs) -> Result<Self, CliError> {
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
        let resolved = config::resolve(&file, &overrides, &env)?;
        let pce = Pce::new(PceClient::new(resolved.client_config())?)
            .with_key_order(KeyOrder::new(&resolved.scope_keys, &resolved.tier_key));
        let format = super::output::choose_format(global.json, global.table);
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
            write: None,
        }
    }

    pub fn emit<T: Serialize + TableRows>(
        &self,
        data: &T,
        cache: &'static str,
        truncated: bool,
        window: Option<Window>,
    ) -> Result<(), CliError> {
        let meta = self.meta(cache, truncated, window);
        let mut out = std::io::stdout().lock();
        match self.format {
            OutputFormat::Json => write_json(&mut out, data, &meta)?,
            OutputFormat::Table => {
                write_table(&mut out, data)?;
                eprintln!("{}", meta_line(&meta));
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
