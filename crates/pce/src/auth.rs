//! API credentials. The secret is wrapped so it cannot leak through `Debug`
//! or accidental logging.
use secrecy::{ExposeSecret, SecretString};

#[derive(Clone)]
pub struct Credentials {
    api_key: String,
    api_secret: SecretString,
}

impl Credentials {
    pub fn new(api_key: impl Into<String>, api_secret: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            api_secret: SecretString::from(api_secret.into()),
        }
    }

    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    /// Returns the secret for building the Authorization header. Call sites
    /// should be few and obvious.
    pub fn expose_secret(&self) -> &str {
        self.api_secret.expose_secret()
    }
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("api_key", &self.api_key)
            .field("api_secret", &"<redacted>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_never_contains_the_secret() {
        let c = Credentials::new("api_key_123", "very-secret");
        let shown = format!("{c:?}");
        assert!(shown.contains("api_key_123"));
        assert!(!shown.contains("very-secret"));
    }

    #[test]
    fn secret_is_available_on_request() {
        let c = Credentials::new("k", "s");
        assert_eq!(c.expose_secret(), "s");
        assert_eq!(c.api_key(), "k");
    }
}
