//! Everything a command needs: the resolved config, the Pce facade, the
//! output format, and the cache mode.
use super::GlobalArgs;
use super::error::CliError;
use super::output::{
    Meta, OutputFormat, TableRows, Window, WriteMeta, meta_line, write_json, write_table,
    write_table_csv,
};
use crate::config::{self, Overrides};
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
        let (client, poll) = test_speed(resolved.client_config());
        let mut pce = Pce::new(PceClient::new(client)?).with_key_order(resolved.key_order());
        if let Some(poll) = poll {
            pce.set_poll_config(poll);
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
            write: None,
            usage: None,
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
                    writeln!(out, "{title}")?;
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

/// The env var that speeds the CLI up in debug builds, for the CLI tests:
/// the first traffic poll in ms (the default is 1 s) and no rate limit (the
/// default is 2 requests a second).
pub const TEST_FAST_ENV: &str = "ILLUMIO_TUI_TEST_FAST_MS";

/// Debug builds only: with `ILLUMIO_TUI_TEST_FAST_MS` set and a loopback
/// host (the tests' mock PCE), the client is not rate limited and the traffic
/// poll starts at that many ms (its backoff capped at 5 times it). A real PCE
/// is never sped up; release builds ignore it.
#[cfg(debug_assertions)]
fn test_speed(mut cfg: pce::ClientConfig) -> (pce::ClientConfig, Option<pce::traffic::PollConfig>) {
    let ms = std::env::var(TEST_FAST_ENV)
        .ok()
        .and_then(|v| v.parse::<u64>().ok());
    let Some(ms) = ms.filter(|_| is_loopback(&cfg.host)) else {
        return (cfg, None);
    };
    cfg.requests_per_second = 1000.0;
    cfg.burst = 1000;
    let poll = pce::traffic::PollConfig {
        initial: std::time::Duration::from_millis(ms),
        cap: std::time::Duration::from_millis(ms.saturating_mul(5)),
        ..pce::traffic::PollConfig::default()
    };
    (cfg, Some(poll))
}

/// `localhost`, `127.0.0.0/8` or `::1`, with or without a scheme or port.
#[cfg(debug_assertions)]
fn is_loopback(host: &str) -> bool {
    let h = host
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let h = h.split('/').next().unwrap_or(h);
    let h = match h.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or(rest),
        None if h.matches(':').count() == 1 => h.split(':').next().unwrap_or(h),
        None => h,
    };
    h.eq_ignore_ascii_case("localhost")
        || h.parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

#[cfg(not(debug_assertions))]
fn test_speed(cfg: pce::ClientConfig) -> (pce::ClientConfig, Option<pce::traffic::PollConfig>) {
    (cfg, None)
}

#[cfg(all(test, debug_assertions))]
mod test_speed_tests {
    use super::is_loopback;

    #[test]
    fn only_a_loopback_host_is_sped_up() {
        for h in [
            "127.0.0.1",
            "localhost",
            "::1",
            "[::1]:8443",
            "http://127.0.0.1:9000",
            "127.0.0.1:443",
        ] {
            assert!(is_loopback(h), "{h}");
        }
        for h in [
            "demo100.illum.io",
            "10.0.0.1",
            "https://pce.example.com:8443",
            "localhost.evil.com",
            "1.2.3.4:443",
        ] {
            assert!(!is_loopback(h), "{h}");
        }
    }
}
