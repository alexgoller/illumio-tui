//! The PCE names shell completion offers (labels, apps, hostnames,
//! services, IP lists, label groups): one JSON file per profile,
//! `$XDG_CACHE_HOME/illumio-tui/<profile>/names.json` (else
//! `~/.cache/illumio-tui/...`; on Windows without `HOME`, the local app
//! data folder), mode 0600 in a 0700 directory. It holds PCE object names
//! in plain text, never a secret.
//!
//! Written only as a side effect of lists the CLI and TUI read anyway
//! ([`record::Recorder`]) and by `completions refresh`; nothing refreshes
//! it on its own. Read by `illumio-tui __complete`, which never builds a
//! client: this module reads and writes the file and nothing else (no
//! request, no secret command). `completion_cache = false` turns both off.
//!
//! The file belongs to one PCE, the [`Origin`] it was written for: only a
//! profile of the config file whose host and org id nothing overrides
//! writes it (see [`config::names_origin`]). A write for another origin
//! starts the file over; a read for another origin (the profile's host or
//! org id changed since) finds nothing. Names of a filtered or truncated
//! read are only added: they leave with a whole read or `--clear`.
use crate::config::{self, ConfigFile};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub mod record;

/// The file's format; a file of another version reads as no file.
pub const VERSION: u32 = 1;
pub const FILE_NAME: &str = "names.json";

/// The PCE a file holds the names of: the profile's host and org id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub host: String,
    pub org_id: u64,
}

/// The file: each set sorted and deduplicated.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Names {
    pub version: u32,
    #[serde(default)]
    pub written_at: Option<Timestamp>,
    /// The [`Origin`]'s host; a file without one is no one's.
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub org_id: Option<u64>,
    /// `key=value`, live labels only.
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub label_keys: Vec<String>,
    /// Workloads' scope label values, `shop/prod`, as `readiness --app`
    /// and `enforce` take them.
    #[serde(default)]
    pub apps: Vec<String>,
    /// Workload hostnames (the name when there is none).
    #[serde(default)]
    pub hostnames: Vec<String>,
    #[serde(default)]
    pub services: Vec<String>,
    #[serde(default)]
    pub ip_lists: Vec<String>,
    #[serde(default)]
    pub label_groups: Vec<String>,
}

/// One list as read from the PCE: the sets it decides. Each kind of list
/// touches its own sets only.
#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    Labels {
        labels: Vec<String>,
        keys: Vec<String>,
    },
    LabelGroups(Vec<String>),
    IpLists(Vec<String>),
    Services(Vec<String>),
    Workloads {
        hostnames: Vec<String>,
        apps: Vec<String>,
    },
}

impl Names {
    /// Whether the file holds `origin`'s names.
    pub fn is_of(&self, origin: &Origin) -> bool {
        self.host.as_deref() == Some(origin.host.as_str()) && self.org_id == Some(origin.org_id)
    }

    /// `update`'s sets replaced (`complete`: the read was the whole
    /// collection) or added to (a filtered or truncated read).
    pub fn apply(&mut self, update: Update, complete: bool) {
        let set = |field: &mut Vec<String>, mut values: Vec<String>| {
            if !complete {
                values.append(field);
            }
            values.sort();
            values.dedup();
            *field = values;
        };
        match update {
            Update::Labels { labels, keys } => {
                set(&mut self.labels, labels);
                set(&mut self.label_keys, keys);
            }
            Update::LabelGroups(v) => set(&mut self.label_groups, v),
            Update::IpLists(v) => set(&mut self.ip_lists, v),
            Update::Services(v) => set(&mut self.services, v),
            Update::Workloads { hostnames, apps } => {
                set(&mut self.hostnames, hostnames);
                set(&mut self.apps, apps);
            }
        }
    }
}

/// `$XDG_CACHE_HOME/illumio-tui/<profile>/names.json`, else
/// `$HOME/.cache/illumio-tui/...`; `None` when neither is set (never a
/// path relative to the working directory). The profile name is made
/// safe for a directory name.
pub fn path_from(
    xdg_cache_home: Option<&Path>,
    home: Option<&Path>,
    profile: &str,
) -> Option<PathBuf> {
    let base = match (xdg_cache_home, home) {
        (Some(x), _) => x.to_path_buf(),
        (None, Some(h)) => h.join(".cache"),
        (None, None) => return None,
    };
    Some(
        base.join("illumio-tui")
            .join(dir_name(profile))
            .join(FILE_NAME),
    )
}

/// [`path_from`] with the environment; on Windows without `HOME`, the
/// local app data folder.
pub fn default_path(profile: &str) -> Option<PathBuf> {
    let var = |name| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let home = var("HOME");
    #[cfg(windows)]
    if var("XDG_CACHE_HOME").is_none() && home.is_none() {
        return dirs::cache_dir().map(|d| {
            d.join("illumio-tui")
                .join(dir_name(profile))
                .join(FILE_NAME)
        });
    }
    path_from(var("XDG_CACHE_HOME").as_deref(), home.as_deref(), profile)
}

/// The profile's file, or `None` when `completion_cache = false` keeps
/// names off disk for it.
pub fn path_for(file: &ConfigFile, profile: &str) -> Option<PathBuf> {
    config::completion_cache(file, profile)
        .then(|| default_path(profile))
        .flatten()
}

/// The profile name as a directory name, losslessly: ASCII letters,
/// digits, `-` and `_` kept, every other byte `%XX` (so `.`, `..`, `/`
/// and a space never reach the path, and `a b` and `a_b` stay apart).
pub(crate) fn dir_name(profile: &str) -> String {
    if profile.is_empty() {
        return "%".into();
    }
    profile
        .bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The file at `path`; `None` when it is missing, unreadable, not JSON
/// or of another version.
pub fn load(path: &Path) -> Option<Names> {
    let text = std::fs::read(path).ok()?;
    serde_json::from_slice::<Names>(&text)
        .ok()
        .filter(|n| n.version == VERSION)
}

/// `update` merged into `origin`'s file at `path` (a missing or
/// unreadable one, or one of another origin, starts empty), stamped
/// `now`, written atomically.
pub fn record(
    path: &Path,
    origin: &Origin,
    update: Update,
    complete: bool,
    now: Timestamp,
) -> io::Result<()> {
    let mut names = load(path).filter(|n| n.is_of(origin)).unwrap_or_default();
    names.version = VERSION;
    names.host = Some(origin.host.clone());
    names.org_id = Some(origin.org_id);
    names.apply(update, complete);
    names.written_at = Some(now);
    let body = serde_json::to_vec_pretty(&names).map_err(io::Error::other)?;
    write_atomic(path, &body)
}

/// Deletes the file; `false` when there was none.
pub fn clear(path: &Path) -> io::Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// `body` to `path` mode 0600 (its directory created 0700): a synced
/// temp file in the same directory, renamed over it.
fn write_atomic(path: &Path, body: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(dir)?;
    // Unique within the process too: two recorders may write at once.
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".{FILE_NAME}.tmp-{}-{seq}", std::process::id()));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let result = (|| {
        let mut f = opts.open(&tmp)?;
        f.write_all(body)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// What a completed value is: the helper's `<kind>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `key=value`, comma-separated.
    Labels,
    /// `[!]key=value`, then (after a comma) more values of that key.
    LabelFilter,
    /// Label keys, comma-separated.
    LabelKeys,
    Apps,
    Hostnames,
    Services,
    IpLists,
    LabelGroups,
    /// A traffic filter term: `key=value`, `wl:HOST`, `ipl:NAME`; comma = AND.
    Traffic,
    /// `svc:NAME`.
    ServiceTerms,
    /// A ruleset scope: `key=value` or `group:NAME`, comma-separated.
    Scope,
    /// A rule side: `[!]key=value`, `group:`, `iplist:`, `workload:`.
    Actors,
    /// A `policy search` term: `key=value`, `ipl:`, `svc:`, `wl:`; after
    /// `scope:` labels and `global`, after `src:`/`dst:` (`consumer:`,
    /// `provider:`) labels, `ipl:`, `wl:` and `all`; a leading `-` and a
    /// `!` before a label kept.
    Search,
    /// Saved filter names of a screen, from the config file (no cache).
    TrafficFilters,
    PolicyFilters,
    EventsFilters,
    WorkloadsFilters,
}

impl Kind {
    pub const ALL: [Kind; 17] = [
        Kind::Labels,
        Kind::LabelFilter,
        Kind::LabelKeys,
        Kind::Apps,
        Kind::Hostnames,
        Kind::Services,
        Kind::IpLists,
        Kind::LabelGroups,
        Kind::Traffic,
        Kind::ServiceTerms,
        Kind::Scope,
        Kind::Actors,
        Kind::Search,
        Kind::TrafficFilters,
        Kind::PolicyFilters,
        Kind::EventsFilters,
        Kind::WorkloadsFilters,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Labels => "labels",
            Kind::LabelFilter => "label-filter",
            Kind::LabelKeys => "label-keys",
            Kind::Apps => "apps",
            Kind::Hostnames => "hostnames",
            Kind::Services => "services",
            Kind::IpLists => "iplists",
            Kind::LabelGroups => "label-groups",
            Kind::Traffic => "traffic",
            Kind::ServiceTerms => "service-terms",
            Kind::Scope => "scope",
            Kind::Actors => "actors",
            Kind::Search => "search",
            Kind::TrafficFilters => "filters-traffic",
            Kind::PolicyFilters => "filters-policy",
            Kind::EventsFilters => "filters-events",
            Kind::WorkloadsFilters => "filters-workloads",
        }
    }

    /// The screen whose saved filter names this kind completes; those come
    /// from the config file, not the names cache.
    pub fn filters(self) -> Option<crate::filters::Screen> {
        use crate::filters::Screen;
        match self {
            Kind::TrafficFilters => Some(Screen::Traffic),
            Kind::PolicyFilters => Some(Screen::Policy),
            Kind::EventsFilters => Some(Screen::Events),
            Kind::WorkloadsFilters => Some(Screen::Workloads),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.name() == s)
    }

    /// Comma-separated terms: the word's last one completes.
    fn lists(self) -> bool {
        matches!(
            self,
            Kind::Labels | Kind::LabelFilter | Kind::LabelKeys | Kind::Traffic | Kind::Scope
        )
    }

    /// The (first) term may start with `!`.
    fn negates(self) -> bool {
        matches!(self, Kind::LabelFilter | Kind::Actors)
    }

    /// Every whole term of this kind; `head` is the word's text before
    /// the term (`app=shop,` for a label filter's next value).
    fn terms(self, n: &Names, head: &str) -> Vec<String> {
        let prefixed =
            |p: &str, v: &[String]| v.iter().map(|x| format!("{p}{x}")).collect::<Vec<_>>();
        let mut out = match self {
            Kind::Labels | Kind::Scope | Kind::Traffic | Kind::Actors | Kind::Search => {
                n.labels.clone()
            }
            Kind::LabelFilter if head.is_empty() => n.labels.clone(),
            Kind::LabelFilter => {
                let key = head.trim_start_matches('!').split('=').next().unwrap_or("");
                let at = format!("{key}=");
                n.labels
                    .iter()
                    .filter_map(|l| l.strip_prefix(&at).map(String::from))
                    .collect()
            }
            Kind::LabelKeys => n.label_keys.clone(),
            Kind::Apps => n.apps.clone(),
            Kind::Hostnames => n.hostnames.clone(),
            Kind::Services => n.services.clone(),
            Kind::IpLists => n.ip_lists.clone(),
            Kind::LabelGroups => n.label_groups.clone(),
            Kind::ServiceTerms => prefixed("svc:", &n.services),
            Kind::TrafficFilters
            | Kind::PolicyFilters
            | Kind::EventsFilters
            | Kind::WorkloadsFilters => Vec::new(),
        };
        match self {
            Kind::Traffic => {
                out.extend(prefixed("wl:", &n.hostnames));
                out.extend(prefixed("ipl:", &n.ip_lists));
            }
            Kind::Scope => out.extend(prefixed("group:", &n.label_groups)),
            Kind::Actors => {
                out.extend(prefixed("group:", &n.label_groups));
                out.extend(prefixed("iplist:", &n.ip_lists));
                out.extend(prefixed("workload:", &n.hostnames));
            }
            Kind::Search => {
                out.extend(prefixed("ipl:", &n.ip_lists));
                out.extend(prefixed("svc:", &n.services));
                out.extend(prefixed("wl:", &n.hostnames));
            }
            _ => {}
        }
        out
    }
}

/// The values of `kind` that complete `word` (the text typed so far, a
/// `--flag=` in front of it dropped), whole: `app=shop,env=p` completes
/// to `app=shop,env=prod`.
pub fn complete(kind: Kind, names: &Names, word: &str) -> Vec<String> {
    let word = match word.strip_prefix("--") {
        Some(rest) => rest.split_once('=').map_or("", |(_, v)| v),
        None => word,
    };
    if kind == Kind::Search {
        return search_terms(names, word);
    }
    let (head, term) = match word.rfind(',') {
        Some(i) if kind.lists() => word.split_at(i + 1),
        _ => ("", word),
    };
    let (bang, term) = match term.strip_prefix('!') {
        Some(t) if kind.negates() && head.is_empty() => ("!", t),
        _ => ("", term),
    };
    kind.terms(names, head)
        .into_iter()
        .filter(|t| t.starts_with(term))
        .map(|t| format!("{head}{bang}{t}"))
        .collect()
}

/// The words a bare search term may start as, as the TUI offers them.
const SEARCH_WORDS: [&str; 9] = [
    "scope:", "src:", "dst:", "disabled", "enabled", "deny", "allow", "intra", "extra",
];

/// [`Kind::Search`]: `-` (a negated term), then a side prefix, then `!`
/// (an exclusion: labels only) peeled off the word and put back on each
/// candidate.
fn search_terms(n: &Names, word: &str) -> Vec<String> {
    let prefixed = |p: &str, v: &[String]| v.iter().map(|x| format!("{p}{x}")).collect::<Vec<_>>();
    let (dash, rest) = word.split_at(usize::from(word.starts_with('-')));
    let side = ["scope:", "src:", "consumer:", "dst:", "provider:"]
        .into_iter()
        .find(|p| rest.starts_with(p));
    let (side, rest) = rest.split_at(side.map_or(0, str::len));
    let (bang, term) = rest.split_at(usize::from(rest.starts_with('!')));
    let terms = match (side, bang) {
        (_, "!") => n.labels.clone(),
        ("", _) => [
            SEARCH_WORDS.map(String::from).to_vec(),
            Kind::Search.terms(n, ""),
        ]
        .concat(),
        ("scope:", _) => [n.labels.clone(), vec!["global".into()]].concat(),
        _ => [
            n.labels.clone(),
            vec!["all".into()],
            prefixed("ipl:", &n.ip_lists),
            prefixed("wl:", &n.hostnames),
        ]
        .concat(),
    };
    terms
        .into_iter()
        .filter(|t| t.starts_with(term))
        .map(|t| format!("{dash}{side}{bang}{t}"))
        .collect()
}

/// The names of the profile `named` (else `ILLUMIO_PROFILE` of the
/// process environment, not of an `--env-file`; else the config's
/// default) from the config file at `config_path`: `None` when the cache
/// is off for it, the profile is not in the file with a host and an org
/// id, the file holds another PCE's names, or any file is missing or
/// unreadable.
pub fn lookup(config_path: &Path, named: Option<String>) -> Option<Names> {
    let file = config::load_config_file(config_path).ok()?;
    let named = named.or_else(|| {
        std::env::var("ILLUMIO_PROFILE")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    });
    let (profile, _) = config::profile_name(&file, named);
    let origin = config::profile_origin(&file, &profile)?;
    load(&path_for(&file, &profile)?).filter(|n| n.is_of(&origin))
}

#[cfg(test)]
mod tests;
