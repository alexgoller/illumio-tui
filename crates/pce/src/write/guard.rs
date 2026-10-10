//! The write guard: writes are impossible without one, and one cannot be
//! built unless the profile allows writes, the caller asked for them, and
//! the audit log is writable.
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
    #[error("writes need --allow-writes")]
    FlagMissing,
    #[error("audit log is not writable: {0}")]
    AuditUnwritable(String),
}

pub struct WriteGuard {
    audit: Box<dyn AuditSink>,
    ctx: WriteContext,
}

impl WriteGuard {
    pub fn new(
        profile_allows: bool,
        flag_allows: bool,
        audit: Box<dyn AuditSink>,
        ctx: WriteContext,
    ) -> Result<Self, WriteRefused> {
        if !profile_allows {
            return Err(WriteRefused::ProfileDenies);
        }
        if !flag_allows {
            return Err(WriteRefused::FlagMissing);
        }
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
