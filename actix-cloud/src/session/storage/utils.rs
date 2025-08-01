use rand::distr::{Alphanumeric, SampleString as _};

use super::SessionKey;

/// Session key generation routine that follows [OWASP recommendations].
///
/// [OWASP recommendations]: https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html#session-id-entropy
pub fn generate_session_key() -> SessionKey {
    Alphanumeric
        .sample_string(&mut rand::rng(), 64)
        .try_into()
        .expect("generated string should be within size range for a session key")
}
