// A header value that must never be printed, logged or serialized. The only way out
// is `expose`, which the outgoing HTTP request uses.
#[derive(Clone)]
pub struct SecretValue(String);

impl SecretValue {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("<redacted>")
    }
}
