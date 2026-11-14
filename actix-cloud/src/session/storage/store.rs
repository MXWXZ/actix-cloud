use core::time;
use std::sync::Arc;

use actix_web::cookie::time::Duration;
use serde_json::{Map, Value};

use super::{utils::generate_session_key, SessionKey};
use crate::{memorydb::MemoryDB, session::error::SessionError};

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
    pub async fn load(
        &self,
        session_key: &SessionKey,
    ) -> Result<Option<SessionState>, SessionError> {
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
    ) -> Result<(), SessionError> {
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
    ) -> Result<SessionKey, SessionError> {
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
    ) -> Result<SessionKey, SessionError> {
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
    ) -> Result<(), SessionError> {
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
    pub async fn delete(
        &self,
        session_key: &SessionKey,
        id: &Option<String>,
    ) -> Result<(), SessionError> {
        let cache_key = (self.configuration.cache_keygen)(session_key.as_ref());

        self.client.del(&cache_key).await?;
        if let Some(id) = id {
            let key =
                (self.configuration.cache_keygen)(&format!("{}_{}", id, session_key.as_ref()));
            self.client.del(&key).await?;
        }
        Ok(())
    }

    /// Renew the session: move the state to a new key, overwrite it with
    /// `session_state`, refresh the TTL and move the reverse-lookup binding.
    ///
    /// The move uses [`MemoryDB::rename`] so the old key disappears atomically; a
    /// concurrent reader never sees neither key. When the old key is gone (expired
    /// or lost) the state is saved under the new key instead.
    pub async fn renew(
        &self,
        session_key: SessionKey,
        session_state: SessionState,
        id: &Option<String>,
        ttl: &Duration,
    ) -> Result<SessionKey, SessionError> {
        let body = serde_json::to_string(&session_state)?;
        let new_key = generate_session_key();
        let cache_key = (self.configuration.cache_keygen)(session_key.as_ref());
        let new_cache_key = (self.configuration.cache_keygen)(new_key.as_ref());

        if self.client.rename(&cache_key, &new_cache_key).await? {
            // The rename carried the old state over; replace it with this request's
            // state and refresh the TTL.
            self.client
                .set_ex(&new_cache_key, &body, &Self::parse_ttl(ttl))
                .await?;
            if let Some(id) = id {
                let old_id_key =
                    (self.configuration.cache_keygen)(&format!("{}_{}", id, session_key.as_ref()));
                let new_id_key =
                    (self.configuration.cache_keygen)(&format!("{}_{}", id, new_key.as_ref()));
                let _ = self.client.rename(&old_id_key, &new_id_key).await?;
                self.client
                    .set_ex(&new_id_key, "1", &Self::parse_ttl(ttl))
                    .await?;
            }
            Ok(new_key)
        } else {
            // The old entry is gone; drop any leftover reverse lookup and save.
            self.delete(&session_key, id).await?;
            self.persist(new_key.as_ref(), &body, id, ttl).await?;
            Ok(new_key)
        }
    }

    fn parse_ttl(t: &Duration) -> time::Duration {
        let t = t.whole_milliseconds();
        let t = if t < 0 { 0 } else { t as u64 };
        time::Duration::from_millis(t)
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration as StdDuration};

    use actix_web::cookie::time::Duration as CookieDuration;
    use serde_json::json;

    use super::*;
    use crate::memorydb::{default::DefaultBackend, MemoryDB};

    fn backend() -> Arc<dyn MemoryDB> {
        Arc::new(DefaultBackend::new())
    }

    fn state(user: i32) -> SessionState {
        let mut state = Map::new();
        state.insert("user".to_owned(), json!(user));
        state
    }

    fn key(value: &str) -> SessionKey {
        SessionKey::try_from(value.to_owned()).unwrap()
    }

    #[tokio::test]
    async fn save_and_load_roundtrip() {
        let backend = backend();
        let store = SessionStore::new(backend.clone());
        let key = store
            .save(state(1), &None, &CookieDuration::seconds(60))
            .await
            .unwrap();

        assert_eq!(key.as_ref().len(), 64);
        let loaded = store.load(&key).await.unwrap().unwrap();
        assert_eq!(loaded.get("user"), Some(&json!(1)));

        let ttl = backend.ttl(key.as_ref()).await.unwrap().unwrap();
        assert!(ttl > StdDuration::from_secs(59));
        assert!(ttl <= StdDuration::from_secs(60));
    }

    #[tokio::test]
    async fn save_generates_a_new_key_each_time() {
        let backend = backend();
        let store = SessionStore::new(backend.clone());
        let ttl = CookieDuration::seconds(60);

        let first = store.save(state(1), &None, &ttl).await.unwrap();
        let second = store.save(state(2), &None, &ttl).await.unwrap();

        assert_ne!(first, second);
        assert_eq!(backend.keys("*").await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn load_returns_none_for_unknown_or_invalid_state() {
        let backend = backend();
        let store = SessionStore::new(backend.clone());

        assert!(store.load(&key("missing")).await.unwrap().is_none());

        // Corrupt state is treated as data loss, not an error.
        backend.set("broken", "not json").await.unwrap();
        assert!(store.load(&key("broken")).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn update_replaces_state_and_keeps_the_key() {
        let backend = backend();
        let store = SessionStore::new(backend.clone());
        let session_key = store
            .save(state(1), &None, &CookieDuration::seconds(60))
            .await
            .unwrap();
        let key_str = session_key.as_ref().to_owned();

        let updated = store
            .update(session_key, state(2), &None, &CookieDuration::seconds(120))
            .await
            .unwrap();

        assert_eq!(updated.as_ref(), key_str);
        let loaded = store.load(&updated).await.unwrap().unwrap();
        assert_eq!(loaded.get("user"), Some(&json!(2)));
        let ttl = backend.ttl(&key_str).await.unwrap().unwrap();
        assert!(ttl > StdDuration::from_secs(119));
    }

    #[tokio::test]
    async fn update_ttl_keeps_the_state() {
        let backend = backend();
        let store = SessionStore::new(backend.clone());
        let key = store
            .save(state(1), &None, &CookieDuration::seconds(60))
            .await
            .unwrap();

        store
            .update_ttl(&key, &None, &CookieDuration::seconds(300))
            .await
            .unwrap();

        assert!(backend.ttl(key.as_ref()).await.unwrap().unwrap() > StdDuration::from_secs(299));
        let loaded = store.load(&key).await.unwrap().unwrap();
        assert_eq!(loaded.get("user"), Some(&json!(1)));
    }

    #[tokio::test]
    async fn delete_removes_the_state() {
        let backend = backend();
        let store = SessionStore::new(backend.clone());
        let key = store
            .save(state(1), &None, &CookieDuration::seconds(60))
            .await
            .unwrap();

        store.delete(&key, &None).await.unwrap();

        assert!(backend.get(key.as_ref()).await.unwrap().is_none());
        assert!(store.load(&key).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn renew_moves_the_state_and_reverse_lookup() {
        let backend = backend();
        let store = SessionStore::new(backend.clone());
        let id = Some("user1".to_owned());

        let old = store
            .save(state(1), &id, &CookieDuration::seconds(60))
            .await
            .unwrap();
        let old_key = old.as_ref().to_owned();

        let new = store
            .renew(old, state(2), &id, &CookieDuration::seconds(120))
            .await
            .unwrap();
        let new_key = new.as_ref().to_owned();
        assert_ne!(new_key, old_key);

        // The old key is gone, the new one carries the current state and TTL.
        assert!(backend.get(&old_key).await.unwrap().is_none());
        assert_eq!(
            store.load(&new).await.unwrap().unwrap().get("user"),
            Some(&json!(2))
        );
        assert!(backend.ttl(&new_key).await.unwrap().unwrap() > StdDuration::from_secs(119));

        // The reverse-lookup binding moved with the state.
        let mut keys = backend.keys("*").await.unwrap();
        keys.sort();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&new_key));
        assert!(keys.contains(&format!("user1_{new_key}")));
        assert!(!keys.contains(&format!("user1_{old_key}")));
    }

    #[tokio::test]
    async fn renew_saves_the_state_when_the_old_key_is_missing() {
        let backend = backend();
        let store = SessionStore::new(backend.clone());
        let id = Some("user1".to_owned());
        // A stale reverse lookup from an earlier session must not survive renewal.
        backend.set("user1_missing", "1").await.unwrap();

        let new = store
            .renew(key("missing"), state(3), &id, &CookieDuration::seconds(60))
            .await
            .unwrap();

        assert_eq!(
            store.load(&new).await.unwrap().unwrap().get("user"),
            Some(&json!(3))
        );
        let keys = backend.keys("*").await.unwrap();
        assert_eq!(keys.len(), 2);
        assert!(!keys.contains(&"missing".to_owned()));
        assert!(!keys.contains(&"user1_missing".to_owned()));
        assert!(keys.contains(&format!("user1_{}", new.as_ref())));
    }

    #[tokio::test]
    async fn id_records_a_reverse_lookup_entry() {
        let backend = backend();
        let store = SessionStore::new(backend.clone());
        let id = Some("user1".to_owned());
        let key = store
            .save(state(1), &id, &CookieDuration::seconds(60))
            .await
            .unwrap();

        let mut keys = backend.keys("*").await.unwrap();
        keys.sort();
        assert_eq!(keys.len(), 2);
        let id_key = keys
            .iter()
            .find(|k| k.starts_with("user1_"))
            .expect("reverse lookup key")
            .clone();
        assert_eq!(id_key, format!("user1_{}", key.as_ref()));
        assert_eq!(backend.get(&id_key).await.unwrap().as_deref(), Some("1"));

        store
            .update_ttl(&key, &id, &CookieDuration::seconds(300))
            .await
            .unwrap();
        assert!(backend.ttl(&id_key).await.unwrap().unwrap() > StdDuration::from_secs(299));

        store.delete(&key, &id).await.unwrap();
        assert!(backend.keys("*").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn custom_cache_keygen_is_used_for_state_and_id() {
        let backend = backend();
        let mut store = SessionStore::new(backend.clone());
        store.cache_keygen(|key| format!("prefix:{key}"));

        let id = Some("user1".to_owned());
        let key = store
            .save(state(1), &id, &CookieDuration::seconds(60))
            .await
            .unwrap();

        assert!(backend.get(key.as_ref()).await.unwrap().is_none());
        assert!(backend
            .get(&format!("prefix:{}", key.as_ref()))
            .await
            .unwrap()
            .is_some());
        assert_eq!(
            backend
                .get(&format!("prefix:user1_{}", key.as_ref()))
                .await
                .unwrap()
                .as_deref(),
            Some("1")
        );
        assert_eq!(
            store.load(&key).await.unwrap().unwrap().get("user"),
            Some(&json!(1))
        );

        store.delete(&key, &id).await.unwrap();
        assert!(backend.keys("*").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn non_positive_ttl_stores_nothing() {
        let backend = backend();
        let store = SessionStore::new(backend.clone());

        for ttl in [CookieDuration::ZERO, CookieDuration::milliseconds(-5)] {
            let key = store.save(state(1), &None, &ttl).await.unwrap();
            assert!(store.load(&key).await.unwrap().is_none());
        }
        assert!(backend.keys("*").await.unwrap().is_empty());
    }
}
