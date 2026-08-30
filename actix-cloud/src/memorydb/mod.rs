//! In-memory key-value database used for sessions, CSRF tokens, etc.
//!
//! Implement the [`MemoryDB`] trait to provide your own backend, or use the
//! built-in [`default`] (demo-grade) / [`redis`] backends.
pub mod interface;
pub use interface::MemoryDB;

pub mod default;
#[cfg(feature = "redis")]
pub mod redis;
