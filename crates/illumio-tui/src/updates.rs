//! Update check and release notes (#91). One request, never to the PCE and
//! never with its credentials: `GET <base>/repos/alexgoller/illumio-tui/releases?per_page=30`
//! (GitHub, unauthenticated, 3 s, the usual proxy variables), drafts and
//! prereleases left out. The release bodies are the CHANGELOG sections, so
//! the one answer feeds both `update check` and `news`. It is kept in the
//! cache dir as `update.json` (fresh for 24 h, the attempt time too, so a
//! failed check is retried only a day later). This version's notes and
//! older ones come from the CHANGELOG built into the binary, offline.
//! `ILLUMIO_TUI_UPDATE_URL` (hidden) replaces the base URL for tests.
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// This binary's version.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
/// Where releases are read, unless [`URL_ENV`] says otherwise.
pub const DEFAULT_BASE_URL: &str = "https://api.github.com";
pub const REPO: &str = "alexgoller/illumio-tui";
/// Replaces [`DEFAULT_BASE_URL`] (tests point it at a mock server).
pub const URL_ENV: &str = "ILLUMIO_TUI_UPDATE_URL";
/// Set (to anything but a false value), the TUI checks nothing at startup.
pub const NO_CHECK_ENV: &str = "ILLUMIO_TUI_NO_UPDATE_CHECK";
/// How long a fetch (or a failed attempt) stands.
pub const FRESH_FOR: Duration = Duration::from_secs(24 * 60 * 60);
pub const TIMEOUT: Duration = Duration::from_secs(3);
/// The CHANGELOG at build time: this version's notes and every older one.
pub const CHANGELOG: &str = include_str!("../../../CHANGELOG.md");

/// `MAJOR.MINOR.PATCH`, compared in that order; a leading `v` is taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(pub u64, pub u64, pub u64);

impl Version {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let s = s.strip_prefix('v').unwrap_or(s);
        let mut parts = s.split('.').map(|p| {
            (!p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
                .then(|| p.parse::<u64>().ok())
                .flatten()
        });
        let v = Version(parts.next()??, parts.next()??, parts.next()??);
        parts.next().is_none().then_some(v)
    }

    pub fn current() -> Self {
        Self::parse(CURRENT).expect("the package version is MAJOR.MINOR.PATCH")
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// One release's notes: `body` in the GitHub release form (the CHANGELOG
/// section without its heading, its `###` headings at `##`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    /// `YYYY-MM-DD`, when known.
    pub date: Option<String>,
    pub body: String,
}

impl Release {
    pub fn parsed(&self) -> Option<Version> {
        Version::parse(&self.version)
    }
}

/// The base URL releases are read from: [`URL_ENV`], else GitHub.
pub fn base_url() -> String {
    std::env::var(URL_ENV)
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
}

/// The release page of `version`.
pub fn release_url(version: &str) -> String {
    format!("https://github.com/{REPO}/releases/tag/v{version}")
}

/// What GitHub's release list holds that we read.
#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    published_at: Option<String>,
}

/// The published releases at `base`, newest first; drafts, prereleases
/// and tags that are no `vMAJOR.MINOR.PATCH` left out. The error is the
/// message for the user.
pub async fn fetch(base: &str) -> Result<Vec<Release>, String> {
    let http = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(concat!("illumio-tui/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("cannot reach GitHub: {e}"))?;
    let url = format!("{base}/repos/{REPO}/releases?per_page=30");
    let resp = http
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("cannot reach GitHub ({url}): {}", error_chain(&e)))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!(
            "GitHub answered HTTP {} for {url}",
            status.as_u16()
        ));
    }
    let list: Vec<GithubRelease> = resp
        .json()
        .await
        .map_err(|e| format!("unexpected answer from GitHub ({url}): {e}"))?;
    Ok(from_github(list))
}

/// An error with its causes (reqwest's top line alone says little).
fn error_chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut cause = e.source();
    while let Some(c) = cause {
        s.push_str(": ");
        s.push_str(&c.to_string());
        cause = c.source();
    }
    s
}

fn from_github(list: Vec<GithubRelease>) -> Vec<Release> {
    let mut out: Vec<Release> = list
        .into_iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter_map(|r| {
            let v = Version::parse(&r.tag_name)?;
            Some(Release {
                version: v.to_string(),
                date: r
                    .published_at
                    .as_deref()
                    .and_then(|d| d.get(..10))
                    .map(str::to_string),
                body: r
                    .body
                    .unwrap_or_default()
                    .replace("\r\n", "\n")
                    .trim()
                    .to_string(),
            })
        })
        .collect();
    sort_newest_first(&mut out);
    out.dedup_by(|a, b| a.version == b.version);
    out
}

fn sort_newest_first(list: &mut [Release]) {
    list.sort_by(|a, b| b.parsed().cmp(&a.parsed()));
}

/// The releases in the built-in CHANGELOG, newest first: every
/// `## vX.Y.Z — DATE` section, in the release-body form.
pub fn embedded() -> Vec<Release> {
    parse_changelog(CHANGELOG)
}

/// [`embedded`] of `text`: sections end at the next version heading (one
/// inside a code fence is text), `###` headings become `##` (as
/// `scripts/release-notes.sh` writes the GitHub body).
pub fn parse_changelog(text: &str) -> Vec<Release> {
    let mut out: Vec<Release> = Vec::new();
    let mut fence = false;
    let mut current: Option<(Release, Vec<String>)> = None;
    let finish = |c: Option<(Release, Vec<String>)>, out: &mut Vec<Release>| {
        if let Some((mut r, lines)) = c {
            r.body = lines.join("\n").trim().to_string();
            out.push(r);
        }
    };
    for line in text.lines() {
        if !fence && line.starts_with("## v") {
            finish(current.take(), &mut out);
            let rest = &line[3..];
            let mut words = rest.split_whitespace();
            let Some(v) = words.next().and_then(Version::parse) else {
                continue;
            };
            let date = words
                .find(|w| w.len() == 10 && w.as_bytes()[4] == b'-')
                .map(str::to_string);
            current = Some((
                Release {
                    version: v.to_string(),
                    date,
                    body: String::new(),
                },
                Vec::new(),
            ));
            continue;
        }
        if line.trim_start().starts_with("```") {
            fence = !fence;
        }
        if let Some((_, lines)) = current.as_mut() {
            if !fence && line.starts_with("###") {
                lines.push(line[1..].to_string());
            } else {
                lines.push(line.to_string());
            }
        }
    }
    finish(current, &mut out);
    sort_newest_first(&mut out);
    out
}

/// `update.json`: what the last fetch found, and when it was tried.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cache {
    /// The last fetch that answered; `None` when none has.
    pub fetched_at: Option<jiff::Timestamp>,
    /// The last attempt, answered or not.
    pub attempted_at: jiff::Timestamp,
    /// The newest published version the fetch saw.
    pub latest: Option<String>,
    /// The releases newer than the binary that fetched, with their notes,
    /// newest first.
    pub newer: Vec<Release>,
}

impl Cache {
    /// The file, or `None` when it is missing or unreadable (a broken
    /// cache is no cache).
    pub fn load(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let body = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        write_atomic(path, &body)
    }

    /// A fetch that answered: the newest version and what is newer than
    /// this binary.
    pub fn from_fetch(releases: &[Release], now: jiff::Timestamp) -> Self {
        let current = Version::current();
        Self {
            fetched_at: Some(now),
            attempted_at: now,
            latest: releases.first().map(|r| r.version.clone()),
            newer: releases
                .iter()
                .filter(|r| r.parsed().is_some_and(|v| v > current))
                .cloned()
                .collect(),
        }
    }

    /// A failed attempt at `now`: what an earlier fetch found stays.
    pub fn failed(previous: Option<Self>, now: jiff::Timestamp) -> Self {
        match previous {
            Some(c) => Self {
                attempted_at: now,
                ..c
            },
            None => Self {
                fetched_at: None,
                attempted_at: now,
                latest: None,
                newer: Vec::new(),
            },
        }
    }

    /// The last attempt is under a day old: no new one yet.
    pub fn attempted_recently(&self, now: jiff::Timestamp) -> bool {
        within_a_day(self.attempted_at, now)
    }

    /// The last answer is under a day old.
    pub fn fetched_recently(&self, now: jiff::Timestamp) -> bool {
        self.fetched_at.is_some_and(|t| within_a_day(t, now))
    }

    /// The releases newer than `current` (an older binary's cache may hold
    /// some that are not).
    pub fn newer_than(&self, current: Version) -> Vec<Release> {
        self.newer
            .iter()
            .filter(|r| r.parsed().is_some_and(|v| v > current))
            .cloned()
            .collect()
    }

    /// The newest version, when it is newer than `current`.
    pub fn update_for(&self, current: Version) -> Option<Version> {
        self.latest
            .as_deref()
            .and_then(Version::parse)
            .filter(|v| *v > current)
    }
}

fn within_a_day(t: jiff::Timestamp, now: jiff::Timestamp) -> bool {
    let age = now.as_second() - t.as_second();
    // A time ahead of the clock (a changed clock) counts as stale.
    (0..FRESH_FOR.as_secs() as i64).contains(&age)
}

/// `$XDG_CACHE_HOME/illumio-tui/update.json`, else
/// `$HOME/.cache/illumio-tui/update.json`; on Windows without `HOME`, the
/// local cache folder. `None` when there is none.
pub fn cache_path() -> Option<PathBuf> {
    let var = |name| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let home = var("HOME");
    #[cfg(windows)]
    if var("XDG_CACHE_HOME").is_none() && home.is_none() {
        return dirs::cache_dir().map(|d| d.join("illumio-tui").join("update.json"));
    }
    let base = match (var("XDG_CACHE_HOME"), home) {
        (Some(x), _) => x,
        (None, Some(h)) => h.join(".cache"),
        (None, None) => return None,
    };
    Some(base.join("illumio-tui").join("update.json"))
}

/// `$XDG_DATA_HOME/illumio-tui/last-version` (else
/// `$HOME/.local/share/…`): the version the TUI last started as.
pub fn last_version_path() -> Option<PathBuf> {
    crate::snapshot::default_root()
        .and_then(|snapshots| snapshots.parent().map(|d| d.join("last-version")))
}

/// The TUI's start as `current`: the version it last started as is
/// replaced, and `Some(current)` returned when that was another one. A
/// first start (no file) says nothing; a file that cannot be written is
/// left as it is.
pub fn note_start(path: &Path, current: &str) -> Option<String> {
    let seen = std::fs::read_to_string(path).ok();
    let changed = seen
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty() && s.trim() != current);
    if seen.as_deref().map(str::trim) != Some(current) {
        let _ = write_atomic(path, format!("{current}\n").as_bytes());
    }
    changed.then(|| current.to_string())
}

fn write_atomic(path: &Path, body: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(body)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// One fetch, recorded in the cache at `path` (when there is one) either
/// way: the releases, or the message of the failure.
pub async fn fetch_and_record(
    base: &str,
    path: Option<&Path>,
) -> Result<(Vec<Release>, Cache), String> {
    let now = jiff::Timestamp::now();
    let result = fetch(base).await;
    let cache = match &result {
        Ok(releases) => Cache::from_fetch(releases, now),
        Err(_) => Cache::failed(path.and_then(Cache::load), now),
    };
    if let Some(p) = path {
        let _ = cache.save(p);
    }
    result.map(|r| (r, cache))
}

/// The TUI's startup check: nothing when the cache was tried within a day;
/// else one fetch, recorded. The newer version it knows of, from the
/// fetch or the cache; `None` on any failure.
pub async fn startup_check(base: &str, path: &Path) -> Option<Version> {
    let now = jiff::Timestamp::now();
    let cached = Cache::load(path);
    if let Some(c) = cached.as_ref().filter(|c| c.attempted_recently(now)) {
        return c.update_for(Version::current());
    }
    let (_, cache) = fetch_and_record(base, Some(path)).await.ok()?;
    cache.update_for(Version::current())
}

/// `news`' range: `A..` or `A..B`, as git's: after A, up to B included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub after: Version,
    pub upto: Option<Version>,
}

impl Range {
    pub fn parse(s: &str) -> Result<Self, String> {
        let bad = || format!("range {s:?}: use A.. or A..B (e.g. 0.5.6.. or 0.5.6..0.5.8)");
        let (a, b) = s.split_once("..").ok_or_else(bad)?;
        let after = Version::parse(a).ok_or_else(bad)?;
        let upto = match b.trim() {
            "" => None,
            b => Some(Version::parse(b).ok_or_else(bad)?),
        };
        if upto.is_some_and(|u| u <= after) {
            return Err(format!(
                "range {s:?}: the end must come after the start (A..B lists the releases after A, up to B)"
            ));
        }
        Ok(Self { after, upto })
    }

    pub fn contains(&self, v: Version) -> bool {
        v > self.after && self.upto.is_none_or(|u| v <= u)
    }

    /// Whether releases newer than `current` fall in it (they come from
    /// GitHub; the rest from the built-in CHANGELOG).
    pub fn reaches_past(&self, current: Version) -> bool {
        self.upto.is_none_or(|u| u > current)
    }
}

/// What `news` shows, newest first. No range: the releases newer than
/// `current`, else `current`'s own notes. A range: its releases, those up
/// to `current` from `embedded`, newer ones from `newer`.
pub fn select(
    range: Option<&Range>,
    current: Version,
    embedded: &[Release],
    newer: &[Release],
) -> Vec<Release> {
    let newer: Vec<Release> = newer
        .iter()
        .filter(|r| r.parsed().is_some_and(|v| v > current))
        .cloned()
        .collect();
    let mut out: Vec<Release> = match range {
        None if !newer.is_empty() => newer,
        None => embedded
            .iter()
            .filter(|r| r.parsed() == Some(current))
            .cloned()
            .collect(),
        Some(range) => newer
            .into_iter()
            .chain(
                embedded
                    .iter()
                    .filter(|r| r.parsed().is_some_and(|v| v <= current))
                    .cloned(),
            )
            .filter(|r| r.parsed().is_some_and(|v| range.contains(v)))
            .collect(),
    };
    sort_newest_first(&mut out);
    out
}

/// `news` as markdown: each release under `# vX.Y.Z — DATE`.
pub fn markdown(releases: &[Release]) -> String {
    let mut s = String::new();
    for (i, r) in releases.iter().enumerate() {
        if i > 0 {
            s.push('\n');
        }
        s.push_str(&format!("# v{}", r.version));
        if let Some(d) = &r.date {
            s.push_str(&format!(" — {d}"));
        }
        s.push_str("\n\n");
        if !r.body.is_empty() {
            s.push_str(&r.body);
            s.push('\n');
        }
    }
    s
}

/// How this binary was installed, from its resolved path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Install {
    Homebrew,
    Scoop,
    Winget,
    Msi,
    Package,
    Other,
}

impl Install {
    pub fn from_path(path: &str) -> Self {
        let lower = path.to_ascii_lowercase();
        if path.contains("/Cellar/") {
            Install::Homebrew
        } else if lower.contains("\\scoop\\apps\\") {
            Install::Scoop
        } else if lower.contains("\\winget\\packages\\") {
            Install::Winget
        } else if lower.contains("program files") {
            Install::Msi
        } else if path.starts_with("/usr/bin/") {
            Install::Package
        } else {
            Install::Other
        }
    }

    /// This binary's install, from its path with links resolved.
    pub fn detect() -> Self {
        std::env::current_exe()
            .and_then(std::fs::canonicalize)
            .map(|p| Self::from_path(&p.to_string_lossy()))
            .unwrap_or(Install::Other)
    }

    /// How to get `version` with this install.
    pub fn upgrade_hint(self, version: &str) -> String {
        let page = release_url(version);
        match self {
            Install::Homebrew => "brew upgrade illumio-tui".into(),
            Install::Scoop => "scoop update illumio-tui".into(),
            Install::Winget => "winget upgrade illumio-tui".into(),
            Install::Msi => format!("winget upgrade illumio-tui, or the MSI from {page}"),
            Install::Package => {
                format!("the illumio-tui .deb or .rpm package for {version} from {page}")
            }
            Install::Other => format!("download it from {page}"),
        }
    }
}

#[cfg(test)]
mod tests;
