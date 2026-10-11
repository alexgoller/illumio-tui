//! The write guard: writes are impossible without one, and one cannot be
//! built unless the profile allows writes (`writes = true` or
//! `ILLUMIO_WRITES=1`) and the audit log is writable. Whether the caller
//! wants a guard at all (the TUI's `:writes on`, the CLI's `--yes`) is the
//! caller's business.
use super::audit::AuditSink;

/// Who is writing, for audit entries.
#[derive(Debug, Clone)]
pub struct WriteContext {
    pub profile: String,
    pub host: String,
    pub org_id: u64,
    /// `cli` or `tui`.
    pub source: String,
}

#[derive(Debug, thiserror::Error)]
pub enum WriteRefused {
    #[error(
        "writes are not enabled for this profile; set writes = true in the profile or ILLUMIO_WRITES=1"
    )]
    ProfileDenies,
    #[error("audit log is not writable: {0}")]
    AuditUnwritable(String),
}

pub struct WriteGuard {
    audit: Box<dyn AuditSink>,
    ctx: WriteContext,
}

impl WriteGuard {
    /// The profile refusal `new` reports first; for callers that must refuse
    /// before they can build an audit sink.
    pub fn check(profile_allows: bool) -> Result<(), WriteRefused> {
        if !profile_allows {
            return Err(WriteRefused::ProfileDenies);
        }
        Ok(())
    }

    pub fn new(
        profile_allows: bool,
        audit: Box<dyn AuditSink>,
        ctx: WriteContext,
    ) -> Result<Self, WriteRefused> {
        Self::check(profile_allows)?;
        audit
            .check_writable()
            .map_err(|e| WriteRefused::AuditUnwritable(format!("{}: {e}", audit.location())))?;
        Ok(Self { audit, ctx })
    }

    pub fn audit(&self) -> &dyn AuditSink {
        self.audit.as_ref()
    }

    pub fn audit_location(&self) -> String {
        self.audit.location()
    }

    pub fn context(&self) -> &WriteContext {
        &self.ctx
    }
}

impl std::fmt::Debug for WriteGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriteGuard")
            .field("audit", &self.audit.location())
            .field("ctx", &self.ctx)
            .finish()
    }
}
