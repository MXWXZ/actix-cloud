use core::time;
use std::sync::Arc;

use actix_web::cookie::time::Duration;
use serde_json::{Map, Value};

use super::{utils::generate_session_key, SessionKey};
use crate::{memorydb::MemoryDB, Result};

pub(crate) type SessionState = Map<String, Value>;

/// Session storage backed by [`MemoryDB`].
///
/// The session state is stored as JSON under the (optionally remapped) session key.
/// When a reverse-lookup `id` is set in the session, an extra `{id}_{session_key}`
/// entry is kept so all session keys of one id can be found via `keys`.
#[derive(Clone)]
pub struct SessionStore {
    configuration: CacheConfiguration,
    client: Arc<dyn MemoryDB>,
}

#[derive(Clone)]
struct CacheConfiguration {
    cache_keygen: Arc<dyn Fn(&str) -> String + Send + Sync>,
}

impl Default for CacheConfiguration {
    fn default() -> Self {
        Self {
            cache_keygen: Arc::new(str::to_owned),
        }
    }
}

impl SessionStore {
    pub fn new(client: Arc<dyn MemoryDB>) -> Self {
        Self {
            client,
            configuration: CacheConfiguration::default(),
        }
    }

    /// Set a custom cache key generation strategy, expecting a session key as input.
    pub fn cache_keygen<F>(&mut self, keygen: F)
    where
        F: Fn(&str) -> String + 'static + Send + Sync,
    {
        self.configuration.cache_keygen = Arc::new(keygen);
    }

    /// Load the session state of `session_key`. Returns `None` when the key is unknown,
    /// has expired, or its state fails to deserialize (treated as data loss).
    pub async fn load(&self, session_key: &SessionKey) -> Result<Option<SessionState>> {
        let cache_key = (self.configuration.cache_keygen)(session_key.as_ref());
        let value = self.client.get(&cache_key).await?;

        match value {
            None => Ok(None),
            Some(value) => Ok(serde_json::from_str(&value).ok()),
        }
    }

    /// Write the session body under `session_key` with the given `ttl`, recording the
    /// reverse-lookup binding for `id` when set.
    async fn persist(
        &self,
        session_key: &str,
        body: &str,
        id: &Option<String>,
        ttl: &Duration,
    ) -> Result<()> {
        let cache_key = (self.configuration.cache_keygen)(session_key);
        self.client
            .set_ex(&cache_key, body, &Self::parse_ttl(ttl))
            .await?;
        if let Some(id) = id {
            let key = (self.configuration.cache_keygen)(&format!("{id}_{session_key}"));
            self.client.set_ex(&key, "1", &Self::parse_ttl(ttl)).await?;
        }
        Ok(())
    }

    /// Persist a fresh session under a newly generated key with the given `ttl`,
    /// recording the reverse-lookup binding for `id` when set.
    pub async fn save(
        &self,
        session_state: SessionState,
        id: &Option<String>,
        ttl: &Duration,
    ) -> Result<SessionKey> {
        let body = serde_json::to_string(&session_state)?;
        let session_key = generate_session_key();
        self.persist(session_key.as_ref(), &body, id, ttl).await?;
        Ok(session_key)
    }

    /// Overwrite the state of an existing `session_key` (key is kept), refreshing its
    /// TTL and the reverse-lookup binding for `id` when set.
    pub async fn update(
        &self,
        session_key: SessionKey,
        session_state: SessionState,
        id: &Option<String>,
        ttl: &Duration,
    ) -> Result<SessionKey> {
        let body = serde_json::to_string(&session_state)?;
        self.persist(session_key.as_ref(), &body, id, ttl).await?;
        Ok(session_key)
    }

    /// Refresh the TTL of the session (and its reverse-lookup binding) without
    /// touching the state.
    pub async fn update_ttl(
        &self,
        session_key: &SessionKey,
        id: &Option<String>,
        ttl: &Duration,
    ) -> Result<()> {
        let cache_key = (self.configuration.cache_keygen)(session_key.as_ref());

        self.client
            .expire(&cache_key, &Self::parse_ttl(ttl))
            .await?;
        if let Some(id) = id {
            let key =
                (self.configuration.cache_keygen)(&format!("{}_{}", id, session_key.as_ref()));
            self.client.expire(&key, &Self::parse_ttl(ttl)).await?;
        }
        Ok(())
    }

    /// Delete the session state and its reverse-lookup binding.
    pub async fn delete(&self, session_key: &SessionKey, id: &Option<String>) -> Result<()> {
        let cache_key = (self.configuration.cache_keygen)(session_key.as_ref());

        self.client.del(&cache_key).await?;
        if let Some(id) = id {
            let key =
                (self.configuration.cache_keygen)(&format!("{}_{}", id, session_key.as_ref()));
            self.client.del(&key).await?;
        }
        Ok(())
    }

    fn parse_ttl(t: &Duration) -> time::Duration {
        let t = t.whole_milliseconds();
        let t = if t < 0 { 0 } else { t as u64 };
        time::Duration::from_millis(t)
    }
}
