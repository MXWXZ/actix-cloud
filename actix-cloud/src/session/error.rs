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
