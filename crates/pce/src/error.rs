//! One error type for everything the library can report. The CLI maps
//! `kind()` to exit codes; the TUI shows `Display`.
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum PceError {
    #[error("authentication failed (HTTP {status}); check API key, secret and org id")]
    Auth { status: u16 },
    #[error("not found: {path}")]
    NotFound { path: String },
    #[error("rate limited by the PCE{}", retry_after.map(|d| format!(" (retry after {}s)", d.as_secs())).unwrap_or_default())]
    RateLimited { retry_after: Option<Duration> },
    #[error("request timed out after {0:?}")]
    Timeout(Duration),
    #[error("network error: {0}")]
    Network(String),
    #[error("the PCE rejected the request: {0}")]
    InvalidFilter(String),
    #[error("PCE server error (HTTP {status}): {body}")]
    Server { status: u16, body: String },
    /// A write the PCE refused on validation grounds (HTTP 400, 406, 409, 422).
    #[error("the PCE rejected the write (HTTP {status}): {body}")]
    Rejected { status: u16, body: String },
    #[error("traffic query failed: {0}")]
    QueryFailed(String),
    #[error("cancelled")]
    Cancelled,
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("unexpected response: {0}")]
    Decode(String),
}

impl PceError {
    /// Stable machine-readable name, used in CLI JSON errors.
    pub fn kind(&self) -> &'static str {
        match self {
            PceError::Auth { .. } => "auth",
            PceError::NotFound { .. } => "not_found",
            PceError::RateLimited { .. } => "rate_limited",
            PceError::Timeout(_) => "timeout",
            PceError::Network(_) => "network",
            PceError::InvalidFilter(_) => "invalid_filter",
            PceError::Server { .. } => "server",
            PceError::Rejected { .. } => "rejected",
            PceError::QueryFailed(_) => "query_failed",
            PceError::Cancelled => "cancelled",
            PceError::Invalid(_) => "invalid",
            PceError::Decode(_) => "decode",
        }
    }
}

pub type Result<T> = std::result::Result<T, PceError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_are_stable_snake_case() {
        assert_eq!(PceError::Auth { status: 401 }.kind(), "auth");
        assert_eq!(
            PceError::RateLimited { retry_after: None }.kind(),
            "rate_limited"
        );
        assert_eq!(PceError::Invalid("x".into()).kind(), "invalid");
        assert_eq!(
            PceError::Rejected {
                status: 406,
                body: "x".into()
            }
            .kind(),
            "rejected"
        );
        assert_eq!(PceError::QueryFailed("x".into()).kind(), "query_failed");
    }

    #[test]
    fn display_is_human_readable() {
        let e = PceError::NotFound {
            path: "/orgs/1/labels/9".into(),
        };
        assert_eq!(e.to_string(), "not found: /orgs/1/labels/9");
    }
}
