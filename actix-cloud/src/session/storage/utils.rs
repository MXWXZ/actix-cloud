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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn generates_distinct_alphanumeric_keys() {
        let mut seen = HashSet::new();
        for _ in 0..100 {
            let key = generate_session_key();
            assert_eq!(key.as_ref().len(), 64);
            assert!(key.as_ref().chars().all(|c| c.is_ascii_alphanumeric()));
            seen.insert(key.as_ref().to_owned());
        }
        // 62^64 possible keys: collisions are practically impossible.
        assert_eq!(seen.len(), 100);
    }
}
