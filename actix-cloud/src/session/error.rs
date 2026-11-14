//! Errors returned by session operations.
use crate::memorydb::MemoryDBError;

/// Errors returned by [`Session`](super::Session) operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SessionError {
    /// The session key is too long to be stored in a cookie.
    #[error("the session key is bigger than 4064 bytes, the upper limit on cookie content")]
    KeyTooLong,
    /// Session state could not be serialized or deserialized.
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    /// The session storage backend failed.
    #[error(transparent)]
    Storage(#[from] MemoryDBError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_too_long_message() {
        assert_eq!(
            SessionError::KeyTooLong.to_string(),
            "the session key is bigger than 4064 bytes, the upper limit on cookie content"
        );
    }

    #[test]
    fn serde_and_storage_errors_are_transparent() {
        let serde: SessionError = serde_json::from_str::<serde_json::Value>("{")
            .unwrap_err()
            .into();
        assert!(!serde.to_string().is_empty());

        let storage: SessionError = MemoryDBError::MaxEntriesExceeded.into();
        assert_eq!(storage.to_string(), "max entries exceeded");
    }
}
