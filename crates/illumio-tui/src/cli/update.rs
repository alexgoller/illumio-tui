//! `illumio-tui update check` and `illumio-tui news` (#91): the only
//! commands that read GitHub's release list (`crate::updates`), never the
//! PCE. `update check` always asks; `news` reads the cache while it is
//! fresh, else asks once, and takes this version's notes and older ones
//! from the built-in CHANGELOG.
use super::GlobalArgs;
use super::error::CliError;
use super::output::{Meta, OutputFormat, choose_format, write_json};
use crate::updates::{self, Install, Range, Version};
use clap::{Args, Subcommand};
use serde::Serialize;
use std::io::Write;

#[derive(Subcommand, Debug)]
pub enum UpdateCmd {
    /// Is a newer release out? One request to GitHub's release list (never the PCE)
    ///
    /// Prints `illumio-tui X is available (you have Y)` and the upgrade command for this
    /// install (Homebrew, Scoop, winget, MSI, .deb/.rpm, or the release page), or `illumio-tui
    /// Y is the latest`. Always asks, and refreshes the cache the TUI's once-a-day check reads
    /// ($XDG_CACHE_HOME/illumio-tui/update.json). GitHub unreachable: exit 5.
    /// --fail-if-outdated: exit 10 when a newer release exists (after the report).
    Check(CheckArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct CheckArgs {
    /// Exit 10 (after the report) when a newer release exists: for cron
    #[arg(long)]
    pub fail_if_outdated: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct NewsArgs {
    /// Releases after A (A..), or after A up to B included (A..B): `0.5.6..`, `0.5.6..0.5.8`.
    /// Default: the releases newer than this binary, else this version's notes
    #[arg(value_name = "RANGE")]
    pub range: Option<String>,
    /// Only the notes built into the binary (this version and older): no request
    #[arg(long)]
    pub offline: bool,
}

/// `update check`'s `data`.
#[derive(Debug, Serialize)]
pub struct Checked {
    pub current: String,
    /// The newest published release; `null` when GitHub lists none.
    pub latest: Option<String>,
    pub outdated: bool,
    pub install: Install,
    /// How to upgrade this install; `null` when up to date.
    pub upgrade: Option<String>,
    /// The newest release's page; `null` when GitHub lists none.
    pub release_url: Option<String>,
    pub checked_at: jiff::Timestamp,
}

impl Checked {
    /// The line (and the upgrade command) `--table` prints.
    pub fn text(&self) -> String {
        match (&self.latest, &self.upgrade) {
            (Some(latest), Some(upgrade)) if self.outdated => format!(
                "illumio-tui {latest} is available (you have {})\nupgrade: {upgrade}\n",
                self.current
            ),
            _ => format!("illumio-tui {} is the latest\n", self.current),
        }
    }
}

fn local_meta(started: std::time::Instant, cache: &'static str, warnings: Vec<String>) -> Meta {
    Meta {
        elapsed_ms: started.elapsed().as_millis(),
        cache,
        warnings: (!warnings.is_empty()).then_some(warnings),
        ..Meta::default()
    }
}

pub async fn run_update(global: &GlobalArgs, cmd: &UpdateCmd) -> Result<(), CliError> {
    let UpdateCmd::Check(args) = cmd;
    let started = std::time::Instant::now();
    let format = choose_format(global.json, global.table, global.csv);
    let path = updates::cache_path();
    let (releases, cache) = updates::fetch_and_record(&updates::base_url(), path.as_deref())
        .await
        .map_err(CliError::Unreachable)?;
    let current = Version::current();
    let latest = releases.first().map(|r| r.version.clone());
    let newer = cache.update_for(current);
    let install = Install::detect();
    let data = Checked {
        current: current.to_string(),
        release_url: latest.as_deref().map(updates::release_url),
        latest,
        outdated: newer.is_some(),
        install,
        upgrade: newer.map(|v| install.upgrade_hint(&v.to_string())),
        checked_at: cache.attempted_at,
    };
    let mut out = std::io::stdout().lock();
    match format {
        OutputFormat::Json => write_json(&mut out, &data, &local_meta(started, "bypass", vec![]))?,
        // No meta line: nothing was asked of a PCE.
        OutputFormat::Table | OutputFormat::Csv => out.write_all(data.text().as_bytes())?,
    }
    drop(out);
    match newer {
        Some(v) if args.fail_if_outdated => Err(CliError::Outdated(format!(
            "illumio-tui {v} is available (you have {current})"
        ))),
        _ => Ok(()),
    }
}

pub async fn run_news(global: &GlobalArgs, args: &NewsArgs) -> Result<(), CliError> {
    let started = std::time::Instant::now();
    if !global.json && !global.fields.is_empty() {
        return Err(CliError::Args("--fields needs --json".into()));
    }
    let range = args
        .range
        .as_deref()
        .map(Range::parse)
        .transpose()
        .map_err(CliError::Args)?;
    let current = Version::current();
    let wants_newer = range.as_ref().is_none_or(|r| r.reaches_past(current));
    let mut warnings = Vec::new();
    let (newer, cache) = if args.offline || !wants_newer {
        if args.offline && range.as_ref().is_some_and(|r| r.reaches_past(current)) {
            warnings.push(format!(
                "--offline: the built-in notes only, up to {current}"
            ));
        }
        (Vec::new(), "none")
    } else {
        let newer =
            updates::newer_releases(Some(&updates::base_url()), updates::cache_path().as_deref())
                .await;
        warnings.extend(newer.warning);
        (newer.releases, newer.cache)
    };
    let releases = updates::select(range.as_ref(), current, &updates::embedded(), &newer);
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    let mut out = std::io::stdout().lock();
    if global.json {
        write_json(&mut out, &releases, &local_meta(started, cache, warnings))?;
    } else if releases.is_empty() {
        eprintln!(
            "no release notes in {}",
            args.range.as_deref().unwrap_or("this version")
        );
    } else {
        out.write_all(updates::markdown(&releases).as_bytes())?;
    }
    Ok(())
}
