use core::time;
use std::{collections::HashMap, sync::Arc};

use actix_web::cookie::time::Duration;

use super::{utils::generate_session_key, SessionKey};
use crate::{memorydb::MemoryDB, Result};

pub(crate) type SessionState = HashMap<String, String>;

#[derive(Clone)]
pub struct SessionStore<M>
where
    M: MemoryDB,
{
    configuration: CacheConfiguration,
    client: M,
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

impl<M> SessionStore<M>
where
    M: MemoryDB,
{
    pub fn new(client: M) -> Self {
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

    pub async fn load(&self, session_key: &SessionKey) -> Result<Option<SessionState>> {
        let cache_key = (self.configuration.cache_keygen)(session_key.as_ref());
        let value = self.client.get(cache_key).await?;

        match value {
            None => Ok(None),
            Some(value) => Ok(serde_json::from_str(&value).ok()),
        }
    }

    pub async fn save(&self, session_state: SessionState, ttl: &Duration) -> Result<SessionKey> {
        let body = serde_json::to_string(&session_state)?;
        let session_key = generate_session_key();
        let cache_key = (self.configuration.cache_keygen)(session_key.as_ref());

        self.client
            .set_ex(cache_key, body, &Self::parse_ttl(ttl))
            .await?;

        Ok(session_key)
    }

    pub async fn update(
        &self,
        session_key: SessionKey,
        session_state: SessionState,
        ttl: &Duration,
    ) -> Result<SessionKey> {
        let body = serde_json::to_string(&session_state)?;
        let cache_key = (self.configuration.cache_keygen)(session_key.as_ref());

        self.client
            .set_ex(cache_key, body, &Self::parse_ttl(ttl))
            .await?;
        Ok(session_key)
    }

    pub async fn update_ttl(&self, session_key: &SessionKey, ttl: &Duration) -> Result<()> {
        let cache_key = (self.configuration.cache_keygen)(session_key.as_ref());

        self.client.expire(cache_key, ttl.whole_seconds()).await?;
        Ok(())
    }

    pub async fn delete(&self, session_key: &SessionKey) -> Result<()> {
        let cache_key = (self.configuration.cache_keygen)(session_key.as_ref());

        self.client.del(cache_key).await?;
        Ok(())
    }

    fn parse_ttl(t: &Duration) -> time::Duration {
        let t = t.whole_seconds();
        let t = if t < 0 { 0 } else { t as u64 };
        time::Duration::from_secs(t)
    }
}
