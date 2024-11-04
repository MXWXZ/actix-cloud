//! Actix Cloud is an all-in-one web framework based on [Actix Web](https://crates.io/crates/actix-web).
//!
//! Please refer to our [crate.io](https://crates.io/crates/actix-cloud) and [Github](https://github.com/MXWXZ/actix-cloud) for more documents.
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

#[cfg(feature = "macros")]
pub mod macros {
    pub use actix_cloud_codegen::*;
}
#[cfg(feature = "actix-web")]
pub use actix_web;
#[cfg(feature = "anyhow")]
pub use anyhow;
#[cfg(feature = "anyhow")]
pub use anyhow::bail;
#[cfg(feature = "anyhow")]
pub use anyhow::Error;
#[cfg(feature = "anyhow")]
pub use anyhow::Result;
#[cfg(feature = "async-trait")]
pub use async_trait::async_trait;
#[cfg(feature = "chrono")]
pub use chrono;
#[cfg(feature = "config")]
pub use config;
#[cfg(feature = "macros")]
pub use macros::main;
#[cfg(feature = "router")]
pub use router::build_router;
#[cfg(feature = "tokio")]
pub use tokio;
#[cfg(feature = "logger")]
pub use tracing;

#[cfg(feature = "csrf")]
pub mod csrf;
#[cfg(feature = "i18n")]
pub mod i18n;
#[cfg(feature = "logger")]
pub mod logger;
#[cfg(feature = "memorydb")]
pub mod memorydb;
#[cfg(feature = "request")]
pub mod request;
#[cfg(feature = "response")]
pub mod response;
#[cfg(feature = "router")]
pub mod router;
#[cfg(feature = "security")]
pub mod security;
#[cfg(feature = "session")]
pub mod session;
#[cfg(feature = "state")]
pub mod state;
#[cfg(feature = "traceid")]
pub use tracing_actix_web;
#[cfg(feature = "response-build")]
pub mod response_build;
#[cfg(feature = "utils")]
pub mod utils;
