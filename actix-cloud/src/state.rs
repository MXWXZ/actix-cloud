//! Global application state and server handle.
//!
//! Populate [`GlobalState`] (fields depend on enabled features), attach it with
//! `.build()` as app data, and let [`ServerHandle`] own the actix server lifecycle.
use std::io;

#[cfg(feature = "memorydb")]
use std::sync::Arc;

use actix_web::dev;
use actix_web::web::Data;
use chrono::{DateTime, Utc};
use parking_lot::{Mutex, RwLock};

/// Shared application state, stored as actix-web `Data<GlobalState>`.
pub struct GlobalState {
    #[cfg(feature = "memorydb")]
    /// Shared key-value database (sessions, CSRF tokens, ...).
    pub memorydb: Arc<dyn crate::memorydb::interface::MemoryDB>,

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
    pub async fn start(&self, server: dev::Server) -> io::Result<()> {
        *self.inner.lock() = Some(server.handle());
        *self.running.write() = true;
        *self.start_time.write() = Utc::now();

        server.await
    }

    /// Sends stop signal through contained server handle.
    ///
    /// `running`/`stop_time` are always updated; the server signal is only sent when
    /// [`Self::start`] has been called.
    pub fn stop(&self, graceful: bool) {
        *self.running.write() = false;
        *self.stop_time.write() = Some(Utc::now());
        if let Some(handle) = self.inner.lock().as_ref() {
            #[allow(clippy::let_underscore_future)]
            let _ = handle.stop(graceful);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(server: ServerHandle) -> GlobalState {
        GlobalState {
            #[cfg(feature = "memorydb")]
            memorydb: Arc::new(crate::memorydb::default::DefaultBackend::new()),
            #[cfg(feature = "config")]
            config: config::Config::default(),
            #[cfg(feature = "logger")]
            logger: None,
            #[cfg(feature = "i18n")]
            locale: crate::i18n::Locale::new("en-US"),
            server,
        }
    }

    #[test]
    fn server_handle_defaults_and_stop() {
        let handle = ServerHandle::default();
        assert!(!*handle.running.read());
        assert_eq!(*handle.start_time.read(), DateTime::<Utc>::default());
        assert!(handle.stop_time.read().is_none());

        // Stopping without a started server only updates the state.
        handle.stop(true);
        assert!(!*handle.running.read());
        assert!(handle.stop_time.read().is_some());
    }

    #[test]
    fn build_wraps_state_in_data() {
        let data = state(ServerHandle::default()).build();
        assert!(!*data.server.running.read());
    }

    #[test]
    fn stop_updates_the_stop_time_on_every_call() {
        let handle = ServerHandle::default();

        handle.stop(false);
        let first = *handle.stop_time.read();
        assert!(first.is_some());

        handle.stop(true);
        let second = *handle.stop_time.read();
        assert!(second >= first);
    }

    #[test]
    fn global_state_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send_sync::<ServerHandle>();
        assert_send_sync::<GlobalState>();
    }

    #[actix_web::test]
    async fn start_and_stop_update_the_server_state() {
        use actix_web::{web, App, HttpServer};

        let server = HttpServer::new(|| App::new().route("/", web::get().to(|| async { "ok" })))
            .workers(1)
            .bind(("127.0.0.1", 0))
            .expect("bind")
            .run();

        let handle = Arc::new(ServerHandle::default());
        let runner = handle.clone();
        let task = actix_web::rt::spawn(async move { runner.start(server).await });

        // Wait for `start` to record the handle and timestamp.
        for _ in 0..10_000 {
            if *handle.running.read() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(*handle.running.read());
        let start_time = *handle.start_time.read();
        assert!(start_time > DateTime::<Utc>::default());

        handle.stop(true);
        task.await.unwrap().unwrap();

        assert!(!*handle.running.read());
        assert!(*handle.start_time.read() == start_time);
        assert!(handle.stop_time.read().is_some());
    }
}
