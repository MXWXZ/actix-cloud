use actix_web::dev;
use actix_web::web::Data;
use chrono::{DateTime, Utc};
use parking_lot::{Mutex, RwLock};

use crate::Result;

pub struct GlobalState {
    #[cfg(feature = "memorydb")]
    pub memorydb: std::sync::Arc<dyn crate::memorydb::interface::MemoryDB>,

    #[cfg(feature = "config")]
    pub config: config::Config,

    #[cfg(feature = "logger")]
    /// Global logger.
    pub logger: Option<crate::logger::Logger>,

    #[cfg(feature = "i18n")]
    pub locale: crate::i18n::Locale,

    /// Handle server state.
    pub server: ServerHandle,
}

impl GlobalState {
    pub fn build(self) -> Data<Self> {
        Data::new(self)
    }
}

#[derive(Default)]
pub struct ServerHandle {
    inner: Mutex<Option<dev::ServerHandle>>,

    pub running: RwLock<bool>,
    pub start_time: RwLock<DateTime<Utc>>,
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
    pub fn stop(&self, graceful: bool) {
        *self.running.write() = false;
        *self.stop_time.write() = Some(Utc::now());
        #[allow(clippy::let_underscore_future)]
        let _ = self.inner.lock().as_ref().unwrap().stop(graceful);
    }
}
