//! `completions refresh [--clear]`: the profile's PCE names cache (see
//! [`crate::names`]) filled through the normal client, or deleted. The
//! fill is the reads any list command makes, throttled and counted the
//! same; the cache is written by the recorder the [`Context`] installs, as
//! for every other read. `--clear` reads the config file and sends nothing.
use super::GlobalArgs;
use super::context::Context;
use super::error::CliError;
use super::output::TableRows;
use crate::config;
use crate::names::{self, Names};
use jiff::Timestamp;
use pce::models::WorkloadFilter;
use serde::Serialize;
use std::path::Path;

/// What `refresh` prints: the file, and its counts and age (`refresh`)
/// or whether there was one to delete (`--clear`).
#[derive(Debug, Serialize)]
pub struct NamesReport {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleared: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub written_at: Option<Timestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_seconds: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counts: Option<Counts>,
}

#[derive(Debug, Serialize)]
pub struct Counts {
    pub labels: usize,
    pub label_keys: usize,
    pub apps: usize,
    pub hostnames: usize,
    pub services: usize,
    pub ip_lists: usize,
    pub label_groups: usize,
}

impl Counts {
    fn of(n: &Names) -> Self {
        Self {
            labels: n.labels.len(),
            label_keys: n.label_keys.len(),
            apps: n.apps.len(),
            hostnames: n.hostnames.len(),
            services: n.services.len(),
            ip_lists: n.ip_lists.len(),
            label_groups: n.label_groups.len(),
        }
    }
}

impl NamesReport {
    fn written(path: &Path, names: &Names, now: Timestamp) -> Self {
        Self {
            path: path.display().to_string(),
            cleared: None,
            written_at: names.written_at,
            age_seconds: names.written_at.map(|t| (now - t).get_seconds()),
            counts: Some(Counts::of(names)),
        }
    }
}

impl TableRows for NamesReport {
    fn headers(&self) -> Vec<&'static str> {
        vec!["field", "value"]
    }

    fn rows(&self) -> Vec<Vec<String>> {
        let mut rows = vec![vec!["path".into(), self.path.clone()]];
        if let Some(c) = self.cleared {
            rows.push(vec!["cleared".into(), c.to_string()]);
        }
        if let Some(t) = self.written_at {
            rows.push(vec!["written_at".into(), t.to_string()]);
        }
        if let Some(s) = self.age_seconds {
            rows.push(vec!["age".into(), age(s)]);
        }
        if let Some(c) = &self.counts {
            for (name, n) in [
                ("labels", c.labels),
                ("label_keys", c.label_keys),
                ("apps", c.apps),
                ("hostnames", c.hostnames),
                ("services", c.services),
                ("ip_lists", c.ip_lists),
                ("label_groups", c.label_groups),
            ] {
                rows.push(vec![name.into(), n.to_string()]);
            }
        }
        rows
    }
}

/// `90` → `1m`: the largest whole unit, as the cache's age.
fn age(seconds: i64) -> String {
    match seconds.max(0) {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}

/// `completions refresh`: one read of each list the cache holds, then the
/// file's counts and age. Refused before any request when
/// `completion_cache = false` (nothing would be kept).
pub async fn refresh(ctx: &Context) -> Result<(), CliError> {
    let path = ctx.names_path.clone().ok_or_else(|| {
        CliError::Args(
            "completions refresh: completion_cache = false for this profile (or no cache \
             directory: set HOME or XDG_CACHE_HOME); nothing is kept, so nothing is read"
                .into(),
        )
    })?;
    let started = Timestamp::now();
    let mode = ctx.cache_mode;
    let truncated = [
        ctx.pce.labels(mode).await?.truncated,
        ctx.pce.label_groups(mode).await?.truncated,
        ctx.pce.ip_lists(mode).await?.truncated,
        ctx.pce.services(mode).await?.truncated,
        ctx.pce
            .workloads(&WorkloadFilter::default(), mode)
            .await?
            .truncated,
    ]
    .contains(&true);
    // The recorder swallows a failed write (it must never fail a list
    // command); here the write is the point, so say it did not happen.
    let names = names::load(&path)
        .filter(|n| n.written_at.is_some_and(|t| t >= started))
        .ok_or_else(|| {
            CliError::Other(format!(
                "completions refresh: the names cache could not be written to {}",
                path.display()
            ))
        })?;
    let report = NamesReport::written(&path, &names, Timestamp::now());
    ctx.emit(&report, "none", truncated, None)
}

/// `completions refresh --clear`: deletes the profile's file, whatever
/// `completion_cache` says (it may predate the switch). No request.
pub fn clear(global: &GlobalArgs) -> Result<(), CliError> {
    let path = global
        .config
        .clone()
        .unwrap_or_else(config::default_config_path);
    let file = config::load_config_file(&path)?;
    let (profile, _) = config::profile_name(&file, global.profile.clone());
    let path = names::default_path(&profile).ok_or_else(|| {
        CliError::Args("completions refresh --clear: no cache directory (HOME unset)".into())
    })?;
    let cleared = names::clear(&path)?;
    let report = NamesReport {
        path: path.display().to_string(),
        cleared: Some(cleared),
        written_at: None,
        age_seconds: None,
        counts: None,
    };
    super::profile::emit(global, &report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_age_is_its_largest_whole_unit() {
        assert_eq!(age(-3), "0s");
        assert_eq!(age(59), "59s");
        assert_eq!(age(90), "1m");
        assert_eq!(age(7200), "2h");
        assert_eq!(age(3 * 86_400 + 5), "3d");
    }
}
