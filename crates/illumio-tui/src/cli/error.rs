//! CLI errors and their exit codes: 1 generic, 2 arguments/config, 3 auth,
//! 4 PCE error, 5 rate limited, 6 write refused, 7 blocked since an
//! enforcement (`enforce --watch`).
use crate::config::ConfigError;
use crate::marks::MarksError;
use pce::PceError;
use pce::write::WriteError;

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("{0}")]
    Args(String),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Pce(#[from] PceError),
    #[error(transparent)]
    Write(#[from] WriteError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
    /// The data is missing rows (a PCE region did not answer): exit 4.
    #[error("{0}")]
    Incomplete(String),
    /// A bulk write that did not change everything it planned, after its
    /// report: the exit code and kind follow the first reason (2 invalid,
    /// 3 auth, 5 rate limited, 6 write refused, else 4).
    #[error("bulk write incomplete: {message}")]
    Bulk {
        message: String,
        code: i32,
        kind: &'static str,
    },
    /// A write refused before anything was sent, with why (`enforce` on
    /// an app that is not ready, a record from another PCE): exit 6.
    #[error("{0}")]
    Refused(String),
    /// `enforce --watch` found flows blocked since the change: exit 7,
    /// after the report.
    #[error("{0}")]
    Blocked(String),
    /// `marks.toml`: invalid (exit 2, every problem), locked by another
    /// writer (exit 3), a mark gone (exit 2), unreadable (exit 1).
    #[error("{}", .0.lines().join("; "))]
    Marks(crate::marks::MarksError),
}

impl CliError {
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::Args(_) | CliError::Config(_) => 2,
            CliError::Pce(PceError::Auth { .. }) => 3,
            CliError::Pce(PceError::RateLimited { .. }) => 5,
            CliError::Pce(PceError::Invalid(_)) => 2,
            CliError::Pce(_) | CliError::Incomplete(_) => 4,
            CliError::Bulk { code, .. } => *code,
            CliError::Refused(_) => 6,
            CliError::Blocked(_) => 7,
            CliError::Write(e) => write_exit_code(e),
            CliError::Io(_) | CliError::Other(_) => 1,
            CliError::Marks(e) => match e {
                MarksError::Parse(_) | MarksError::Gone(_) => 2,
                MarksError::Locked => 3,
                MarksError::Io(_) => 1,
            },
        }
    }

    pub fn kind(&self) -> String {
        match self {
            CliError::Args(_) => "args".into(),
            CliError::Config(_) => "config".into(),
            CliError::Pce(e) => e.kind().into(),
            CliError::Write(e) => e.kind().into(),
            CliError::Io(_) => "io".into(),
            CliError::Other(_) => "error".into(),
            CliError::Incomplete(_) => "incomplete".into(),
            CliError::Bulk { kind, .. } => (*kind).into(),
            CliError::Refused(_) => "write_refused".into(),
            CliError::Blocked(_) => "blocked_since_enforcement".into(),
            CliError::Marks(e) => match e {
                MarksError::Parse(_) => "marks_invalid".into(),
                MarksError::Locked => "marks_locked".into(),
                MarksError::Gone(_) => "marks_gone".into(),
                MarksError::Io(_) => "io".into(),
            },
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({"error": {"kind": self.kind(), "message": self.to_string(), "exit_code": self.exit_code()}})
    }
}

/// Exit code of a write error; `AuditFailed` reports as the write error it wraps.
fn write_exit_code(e: &WriteError) -> i32 {
    match e {
        WriteError::Refused(_) => 6,
        WriteError::Invalid(_) => 2,
        WriteError::Unanswered(_) => 4,
        WriteError::AuditFailed { write, .. } => write_exit_code(write),
        WriteError::Pce(e) => match e {
            PceError::Auth { .. } => 3,
            PceError::RateLimited { .. } => 5,
            PceError::Invalid(_) => 2,
            _ => 4,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pce::write::WriteRefused;

    #[test]
    fn write_errors_map_to_exit_codes() {
        assert_eq!(
            CliError::Write(WriteError::Refused(WriteRefused::ProfileDenies)).exit_code(),
            6
        );
        assert_eq!(
            CliError::Write(WriteError::Refused(WriteRefused::ProfileDenies)).kind(),
            "write_refused"
        );
        assert_eq!(
            CliError::Write(WriteError::Invalid(vec!["x".into()])).exit_code(),
            2
        );
        assert_eq!(
            CliError::Write(WriteError::Unanswered(PceError::Cancelled)).exit_code(),
            4
        );
        assert_eq!(
            CliError::Write(WriteError::Pce(PceError::Auth { status: 401 })).exit_code(),
            3
        );
        assert_eq!(
            CliError::Write(WriteError::Pce(PceError::Rejected {
                status: 406,
                body: "x".into()
            }))
            .exit_code(),
            4
        );
        let wrapped = CliError::Write(WriteError::AuditFailed {
            write: Box::new(WriteError::Refused(WriteRefused::ProfileDenies)),
            error: "disk full".into(),
        });
        assert_eq!(wrapped.exit_code(), 6);
        assert_eq!(wrapped.kind(), "write_refused");
        let r = CliError::Refused("not ready".into());
        assert_eq!((r.exit_code(), r.kind().as_str()), (6, "write_refused"));
        let b = CliError::Blocked("2 flows".into());
        assert_eq!(
            (b.exit_code(), b.kind().as_str()),
            (7, "blocked_since_enforcement")
        );
        assert!(wrapped.to_string().contains("audit"));
    }
}
