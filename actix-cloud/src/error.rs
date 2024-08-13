use std::{fmt, io};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("IO error: {0}")]
    IO(#[from] io::Error),

    #[error("fmt error: {0}")]
    Fmt(#[from] fmt::Error),

    #[cfg(feature = "session")]
    #[error("session error: {0}")]
    Session(String),

    #[cfg(feature = "serde")]
    #[error("json error: {0}")]
    JSON(#[from] serde_json::Error),

    #[cfg(feature = "redis")]
    #[error("redis error: {0}")]
    Redis(#[from] redis::RedisError),

    #[error("timestamp error: {0}")]
    Timestamp(&'static str),
}

pub type Result<T, E = Error> = core::result::Result<T, E>;
