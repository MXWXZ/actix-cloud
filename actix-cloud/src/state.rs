use actix_web::dev::{Server, ServerHandle};
use actix_web::web::Data;
use parking_lot::Mutex;

#[cfg(feature = "i18n")]
use crate::i18n::Locale;
#[cfg(feature = "logger")]
use crate::logger::Logger;
use crate::memorydb::interface::MemoryDB;
use crate::Result;

pub struct GlobalState<M>
where
    M: MemoryDB,
{
    pub memorydb: M,

    #[cfg(feature = "logger")]
    /// Global logger.
    pub logger: Logger,

    #[cfg(feature = "i18n")]
    pub locale: Locale,

    /// Handle stop state.
    pub stop_handle: StopHandle,
}

impl<M> GlobalState<M>
where
    M: MemoryDB,
{
    pub fn build(self) -> Data<Self> {
        Data::new(self)
    }
}

#[derive(Default)]
pub struct StopHandle {
    inner: Mutex<Option<ServerHandle>>,
}

impl StopHandle {
    /// Sets the server handle and start blocking.
    pub async fn start(&self, server: Server) -> Result<()> {
        *self.inner.lock() = Some(server.handle());
        server.await.map_err(Into::into)
    }

    /// Sends stop signal through contained server handle.
    pub fn stop(&self, graceful: bool) {
        #[allow(clippy::let_underscore_future)]
        let _ = self.inner.lock().as_ref().unwrap().stop(graceful);
    }
}
