//! Actix Cloud is an all-in-one web framework based on [Actix Web](https://crates.io/crates/actix-web).
//!
//! Please refer to our [crate.io](https://crates.io/crates/actix-cloud) and [Github](https://github.com/MXWXZ/actix-cloud) for more documents.
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

pub mod macros {
    pub use actix_cloud_codegen::*;
}
pub use actix_web;
pub use anyhow;
pub use anyhow::bail;
pub use anyhow::Error;
pub use anyhow::Result;
pub use async_trait::async_trait;
#[cfg(feature = "config")]
pub use config;
pub use macros::main;
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
pub mod request;
#[cfg(feature = "response")]
pub mod response;
pub mod router;
pub mod security;

#[cfg(feature = "session")]
pub mod session;
pub mod state;
#[cfg(feature = "traceid")]
pub use tracing_actix_web;
pub mod utils;

pub use router::build_router;
