//! The local audit log of writes: one JSON line per sent request, written
//! after the response (or with `phase: "sent"` when none arrived). Dry runs
//! and no-ops never reach the log. Request bodies carry no credentials.
use crate::client::WriteMethod;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditEntry {
    pub ts: String,
    pub profile: String,
    pub host: String,
    pub org_id: u64,
    pub method: WriteMethod,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// `"sent"` when the request left but no response arrived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// `cli` or `tui`.
    pub source: String,
}

pub trait AuditSink: Send + Sync {
    /// Called before a request is sent; a failure refuses the write.
    fn check_writable(&self) -> std::io::Result<()>;
    fn append(&self, entry: &AuditEntry) -> std::io::Result<()>;
    /// Where entries go, for `meta.write.audit`.
    fn location(&self) -> String;
}

/// Append-only JSON lines file, private to the user.
#[derive(Debug, Clone)]
pub struct JsonlAudit {
    path: PathBuf,
}

impl JsonlAudit {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// `$XDG_STATE_HOME/illumio-tui/writes.jsonl`, else
    /// `~/.local/state/illumio-tui/writes.jsonl`.
    pub fn default_path() -> PathBuf {
        let xdg = std::env::var_os("XDG_STATE_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        let home = std::env::var_os("HOME").map(PathBuf::from);
        Self::path_from(xdg.as_deref(), home.as_deref())
    }

    pub fn path_from(xdg_state_home: Option<&Path>, home: Option<&Path>) -> PathBuf {
        let base = match (xdg_state_home, home) {
            (Some(x), _) => x.to_path_buf(),
            (None, Some(h)) => h.join(".local").join("state"),
            (None, None) => PathBuf::from(".local").join("state"),
        };
        base.join("illumio-tui").join("writes.jsonl")
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn open(&self) -> std::io::Result<File> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut opts = OpenOptions::new();
        opts.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        opts.open(&self.path)
    }
}

impl AuditSink for JsonlAudit {
    fn check_writable(&self) -> std::io::Result<()> {
        self.open().map(|_| ())
    }

    fn append(&self, entry: &AuditEntry) -> std::io::Result<()> {
        let mut f = self.open()?;
        // One write call: O_APPEND only makes a single write atomic.
        let mut line = serde_json::to_string(entry).map_err(std::io::Error::other)?;
        line.push('\n');
        f.write_all(line.as_bytes())?;
        f.flush()
    }

    fn location(&self) -> String {
        self.path.display().to_string()
    }
}

/// Discards entries. For tests and for the TUI until it writes.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullAudit;

impl AuditSink for NullAudit {
    fn check_writable(&self) -> std::io::Result<()> {
        Ok(())
    }
    fn append(&self, _entry: &AuditEntry) -> std::io::Result<()> {
        Ok(())
    }
    fn location(&self) -> String {
        "none".into()
    }
}

/// Keeps entries in memory so tests can assert on them.
#[derive(Debug, Default)]
pub struct MemoryAudit {
    entries: Mutex<Vec<AuditEntry>>,
}

impl MemoryAudit {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn entries(&self) -> Vec<AuditEntry> {
        self.entries.lock().expect("audit mutex poisoned").clone()
    }
}

impl AuditSink for MemoryAudit {
    fn check_writable(&self) -> std::io::Result<()> {
        Ok(())
    }
    fn append(&self, entry: &AuditEntry) -> std::io::Result<()> {
        self.entries
            .lock()
            .expect("audit mutex poisoned")
            .push(entry.clone());
        Ok(())
    }
    fn location(&self) -> String {
        "memory".into()
    }
}

impl<T: AuditSink + ?Sized> AuditSink for Arc<T> {
    fn check_writable(&self) -> std::io::Result<()> {
        (**self).check_writable()
    }
    fn append(&self, entry: &AuditEntry) -> std::io::Result<()> {
        (**self).append(entry)
    }
    fn location(&self) -> String {
        (**self).location()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry() -> AuditEntry {
        AuditEntry {
            ts: "2026-10-10T10:00:00Z".into(),
            profile: "lab".into(),
            host: "pce.example".into(),
            org_id: 5,
            method: WriteMethod::Post,
            path: "/orgs/5/labels".into(),
            body: Some(json!({"key": "app", "value": "web"})),
            status: Some(201),
            href: Some("/orgs/5/labels/9".into()),
            error: None,
            phase: None,
            source: "cli".into(),
        }
    }

    #[test]
    fn jsonl_appends_one_line_per_entry_and_creates_parents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join("state")
            .join("illumio-tui")
            .join("writes.jsonl");
        let audit = JsonlAudit::new(path.clone());
        audit.check_writable().unwrap();
        audit.append(&entry()).unwrap();
        audit.append(&entry()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        let v: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(v["method"], "POST");
        assert_eq!(v["status"], 201);
        assert_eq!(v["body"]["key"], "app");
        assert!(v.get("error").is_none(), "absent fields are omitted");
        assert_eq!(audit.location(), path.display().to_string());
    }

    #[cfg(unix)]
    #[test]
    fn jsonl_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("writes.jsonl");
        JsonlAudit::new(path.clone()).append(&entry()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn unwritable_path_fails_check() {
        let dir = tempfile::tempdir().unwrap();
        // A file where the parent directory should be.
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        let audit = JsonlAudit::new(blocker.join("writes.jsonl"));
        assert!(audit.check_writable().is_err());
    }

    #[test]
    fn default_path_honours_xdg_state_home() {
        let p = JsonlAudit::path_from(Some(std::path::Path::new("/tmp/xdg-state")), None);
        assert_eq!(
            p,
            std::path::PathBuf::from("/tmp/xdg-state/illumio-tui/writes.jsonl")
        );
        let p = JsonlAudit::path_from(None, Some(std::path::Path::new("/home/u")));
        assert_eq!(
            p,
            std::path::PathBuf::from("/home/u/.local/state/illumio-tui/writes.jsonl")
        );
    }

    #[test]
    fn memory_audit_records_entries() {
        let m = MemoryAudit::new();
        m.append(&entry()).unwrap();
        assert_eq!(m.entries().len(), 1);
        assert_eq!(NullAudit.location(), "none");
        assert!(NullAudit.append(&entry()).is_ok());
    }
}
