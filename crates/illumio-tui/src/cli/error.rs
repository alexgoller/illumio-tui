//! CLI errors and their exit codes: 1 generic, 2 arguments/config, 3 auth,
//! 4 PCE error, 5 rate limited, 6 write refused.
use crate::config::ConfigError;
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
}

impl CliError {
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::Args(_) | CliError::Config(_) => 2,
            CliError::Pce(PceError::Auth { .. }) => 3,
            CliError::Pce(PceError::RateLimited { .. }) => 5,
            CliError::Pce(PceError::Invalid(_)) => 2,
            CliError::Pce(_) => 4,
            CliError::Write(e) => write_exit_code(e),
            CliError::Io(_) | CliError::Other(_) => 1,
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
            CliError::Write(WriteError::Refused(WriteRefused::FlagMissing)).exit_code(),
            6
        );
        assert_eq!(
            CliError::Write(WriteError::Refused(WriteRefused::FlagMissing)).kind(),
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
            write: Box::new(WriteError::Refused(WriteRefused::FlagMissing)),
            error: "disk full".into(),
        });
        assert_eq!(wrapped.exit_code(), 6);
        assert_eq!(wrapped.kind(), "write_refused");
        assert!(wrapped.to_string().contains("audit"));
    }
}
