//! Global application state and server handle.
//!
//! Populate [`GlobalState`] (fields depend on enabled features), attach it with
//! `.build()` as app data, and let [`ServerHandle`] own the actix server lifecycle.
use actix_web::dev;
use actix_web::web::Data;
use chrono::{DateTime, Utc};
use parking_lot::{Mutex, RwLock};

use crate::Result;

/// Shared application state, stored as actix-web `Data<GlobalState>`.
pub struct GlobalState {
    #[cfg(feature = "memorydb")]
    /// Shared key-value database (sessions, CSRF tokens, ...).
    pub memorydb: std::sync::Arc<dyn crate::memorydb::interface::MemoryDB>,

    #[cfg(feature = "config")]
    /// Application configuration loaded through the `config` crate.
    pub config: config::Config,

    #[cfg(feature = "logger")]
    /// Global logger.
    pub logger: Option<crate::logger::Logger>,

    #[cfg(feature = "i18n")]
    /// Default locale; used when the request middleware cannot identify a language.
    pub locale: crate::i18n::Locale,

    /// Handle server state.
    pub server: ServerHandle,
}

impl GlobalState {
    /// Wrap the state into actix-web `Data` for `.app_data(...)`.
    pub fn build(self) -> Data<Self> {
        Data::new(self)
    }
}

/// Owns the running server: start/stop timestamps, running flag and the stop signal.
#[derive(Default)]
pub struct ServerHandle {
    inner: Mutex<Option<dev::ServerHandle>>,

    /// Whether server is running (never received stop signals).
    pub running: RwLock<bool>,
    /// Server start timestamp.
    pub start_time: RwLock<DateTime<Utc>>,
    /// Server stop timestamp.
    pub stop_time: RwLock<Option<DateTime<Utc>>>,
}

impl ServerHandle {
    /// Sets the server handle and start blocking.
    pub async fn start(&self, server: dev::Server) -> Result<()> {
        *self.inner.lock() = Some(server.handle());
        *self.running.write() = true;
        *self.start_time.write() = Utc::now();

        server.await.map_err(Into::into)
    }

    /// Sends stop signal through contained server handle.
    ///
    /// Does nothing if [`Self::start`] has never completed.
    pub fn stop(&self, graceful: bool) {
        *self.running.write() = false;
        *self.stop_time.write() = Some(Utc::now());
        if let Some(handle) = self.inner.lock().as_ref() {
            #[allow(clippy::let_underscore_future)]
            let _ = handle.stop(graceful);
        }
    }
}
