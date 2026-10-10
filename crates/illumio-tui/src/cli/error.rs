//! CLI errors and their exit codes: 1 generic, 2 arguments/config, 3 auth,
//! 4 PCE error, 5 rate limited.
use crate::config::ConfigError;
use pce::PceError;

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("{0}")]
    Args(String),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Pce(#[from] PceError),
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
            CliError::Io(_) | CliError::Other(_) => 1,
        }
    }

    pub fn kind(&self) -> String {
        match self {
            CliError::Args(_) => "args".into(),
            CliError::Config(_) => "config".into(),
            CliError::Pce(e) => e.kind().into(),
            CliError::Io(_) => "io".into(),
            CliError::Other(_) => "error".into(),
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({"error": {"kind": self.kind(), "message": self.to_string(), "exit_code": self.exit_code()}})
    }
}
