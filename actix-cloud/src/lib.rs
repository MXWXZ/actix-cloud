//! Actix Cloud is an all-in-one web framework built on
//! [Actix Web](https://crates.io/crates/actix-web).
//!
//! Every module is behind a cargo feature, so a binary only compiles what it uses (the
//! `all` feature enables everything). The main pieces:
//!
//! - `memorydb` — object-safe key-value abstraction with in-process and Redis backends.
//! - `router` / `csrf` — declarative routes with per-route auth checkers and CSRF.
//! - `session` — session storage backed by `memorydb`.
//! - `logger` — tracing-based structured logger with JSON/color writers.
//! - `i18n` — compile-time locale loading (`i18n!`) and the `t!`/`map!` helpers.
//! - `request` — per-request extension data (language, real IP, trace id).
//! - `response` / `response-build` — uniform responses and build-time response enums.
//! - `state` — `GlobalState` and the server handle.
//!
//! See [crates.io](https://crates.io/crates/actix-cloud) and
//! [GitHub](https://github.com/MXWXZ/actix-cloud) for more documents.
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "macros")]
pub mod macros {
    pub use actix_cloud_codegen::*;
}
#[cfg(feature = "actix-web")]
pub use actix_web;
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

#[cfg(any(feature = "csrf", feature = "session"))]
mod cookie;
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
