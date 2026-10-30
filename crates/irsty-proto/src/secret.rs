//! A string that never appears in `Debug` or `Display` output (ADR 0004).

use std::fmt;

/// A credential such as a password. Formatting prints `<redacted>`; use
/// [`Secret::expose`] at the single point where the value is needed.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Wraps a value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the underlying value. Keep the result out of logs.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[cfg(test)]
mod tests {
    use super::Secret;

    #[test]
    fn never_formats_the_value() {
        let secret = Secret::new("hunter2");
        assert_eq!(
            format!("{secret} {secret:?}"),
            "<redacted> Secret(<redacted>)"
        );
        assert_eq!(secret.expose(), "hunter2");
    }
}
