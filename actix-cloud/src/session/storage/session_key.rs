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
