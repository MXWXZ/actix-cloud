use crate::session::error::SessionError;

/// A session key, the string stored in a client-side cookie to associate a user with its session
/// state on the backend.
///
/// # Validation
/// Session keys are stored as cookies, therefore they cannot be arbitrary long. Session keys are
/// required to be smaller than 4064 bytes.
#[derive(Debug, PartialEq, Eq)]
pub struct SessionKey(String);

impl TryFrom<String> for SessionKey {
    type Error = SessionError;

    fn try_from(val: String) -> Result<Self, Self::Error> {
        if val.len() > 4064 {
            return Err(SessionError::KeyTooLong);
        }

        Ok(SessionKey(val))
    }
}

impl AsRef<str> for SessionKey {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<SessionKey> for String {
    fn from(key: SessionKey) -> Self {
        key.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_keys_up_to_the_cookie_limit() {
        let ok = "a".repeat(4064);
        let key = SessionKey::try_from(ok.clone()).unwrap();
        assert_eq!(key.as_ref(), ok);
        assert_eq!(String::from(key), ok);

        let too_long = "a".repeat(4065);
        assert!(matches!(
            SessionKey::try_from(too_long),
            Err(SessionError::KeyTooLong)
        ));
    }

    #[test]
    fn limit_counts_bytes_not_characters() {
        // Each `é` is two UTF-8 bytes.
        assert!(SessionKey::try_from("é".repeat(2032)).is_ok());
        assert!(matches!(
            SessionKey::try_from("é".repeat(2033)),
            Err(SessionError::KeyTooLong)
        ));
    }

    #[test]
    fn empty_key_is_valid() {
        let key = SessionKey::try_from(String::new()).unwrap();
        assert_eq!(key.as_ref(), "");
    }
}
