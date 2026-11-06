//! In-process [`MemoryDB`] backend.
//!
//! Entries live in a `HashMap` behind an `RwLock`. Expiration timestamps are absolute
//! milliseconds; expired entries are dropped lazily when accessed and eagerly during
//! capacity-driven garbage collection.
//!
//! See [`DefaultBackend`] for the eviction policy and its production caveats.
use std::{
    cmp::{max, Reverse},
    collections::HashMap,
    sync::Arc,
    time::Duration,
};

use anyhow::bail;
use async_trait::async_trait;
use chrono::Utc;
use glob::Pattern;
use parking_lot::{RwLock, RwLockWriteGuard};
use priority_queue::PriorityQueue;

use super::interface::MemoryDB;
use crate::Result;

/// Internal entry: `(value, absolute expiration timestamp in ms)`. `None` means no TTL.
struct Data(String, Option<i64>);

impl Data {
    /// Current Unix timestamp in milliseconds.
    fn now() -> i64 {
        Utc::now().timestamp_millis()
    }

    /// Convert a relative TTL in milliseconds to an absolute expiration timestamp.
    fn parse_ttl(ttl: Option<i64>) -> Option<i64> {
        ttl.map(|x| Self::now().saturating_add(x))
    }

    /// Create an entry from a value and a relative TTL in milliseconds.
    fn new<S>(value: S, ttl: Option<i64>) -> Self
    where
        S: Into<String>,
    {
        Self(value.into(), Self::parse_ttl(ttl))
    }

    /// Reset the expiration from a relative TTL in milliseconds.
    fn set_ttl(&mut self, ttl: Option<i64>) {
        self.1 = Self::parse_ttl(ttl);
    }

    /// Remaining TTL in milliseconds; may be negative once the entry has expired.
    fn get_ttl(&self) -> Option<i64> {
        self.1.map(|x| x.saturating_sub(Self::now()))
    }

    /// Whether the entry is still alive. Entries without a TTL never expire.
    fn valid(&self) -> bool {
        if let Some(x) = self.1 {
            x > Self::now()
        } else {
            true
        }
    }
}

/// Built-in in-process memory database backend.
///
/// Keys and values live in a `HashMap` guarded by an `RwLock`; expired entries are
/// removed lazily on access, plus eagerly during capacity-driven GC.
///
/// # Warning
/// The default backend is only for demo purpose. The performance is not guaranteed.
/// Use [`RedisBackend`](super::redis::RedisBackend) or your own [`MemoryDB`]
/// implementation for production workloads.
///
/// When `capacity` is set and the database is full, the eviction policy is:
/// - Evict 10% keys at a time (at least 1).
/// - Expired keys go first, then keys having TTL attribute, sorted by TTL from shortest to longest.
/// - If no keys are evicted, an error will be returned.
#[derive(Clone)]
pub struct DefaultBackend {
    data: Arc<RwLock<HashMap<String, Data>>>,
    capacity: Option<usize>,
}

impl DefaultBackend {
    /// Create a new backend.
    ///
    /// `capacity` limits the maximum number of entries (`None` means unbounded —
    /// be aware that this allows memory exhaustion).
    pub fn new(capacity: Option<usize>) -> Self {
        Self {
            data: Default::default(),
            capacity,
        }
    }

    /// Remove up to `num` entries, returning how many were removed.
    ///
    /// Every expired entry is removed first; if that freed fewer than `num` entries,
    /// live entries with a TTL are removed shortest-TTL-first until the quota is met.
    /// Entries without a TTL are never evicted.
    fn gc(&self, wlock: &mut RwLockWriteGuard<HashMap<String, Data>>, num: usize) -> usize {
        let mut queue = PriorityQueue::new();
        let mut delete = Vec::new();
        for (k, v) in wlock.iter() {
            if !v.valid() {
                delete.push(k.to_owned());
            } else if let Some(x) = v.1 {
                queue.push(k.to_owned(), Reverse(x));
            }
        }
        for i in &delete {
            wlock.remove(i);
        }
        let mut ret = delete.len();
        if ret < num {
            let remain = num - ret;
            for _ in 0..remain {
                if let Some(k) = queue.pop() {
                    wlock.remove(&k.0);
                    ret += 1;
                } else {
                    return ret;
                }
            }
        }
        ret
    }
}

impl Default for DefaultBackend {
    fn default() -> Self {
        Self::new(None)
    }
}

#[async_trait]
impl MemoryDB for DefaultBackend {
    async fn set(&self, key: &str, value: &str) -> Result<()> {
        let mut wlock = self.data.write();
        // When full and inserting a new key, try to make room; fail if nothing is
        // evictable (all remaining entries are live and have no TTL).
        if let Some(x) = self.capacity {
            if x <= wlock.len()
                && wlock.get(key).is_none()
                && self.gc(&mut wlock, max(x / 10, 1)) == 0
            {
                bail!("capacity is full");
            }
        }
        wlock.insert(key.to_owned(), Data::new(value, None));
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<String>> {
        let rlock = self.data.read();
        if let Some(v) = rlock.get(key) {
            if v.valid() {
                Ok(Some(v.0.to_owned()))
            } else {
                drop(rlock);
                self.data.write().remove(key);
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    async fn get_del(&self, key: &str) -> Result<Option<String>> {
        let v = self.data.write().remove(key);
        if let Some(v) = v {
            if v.valid() {
                return Ok(Some(v.0));
            }
        }
        Ok(None)
    }

    async fn get_ex(&self, key: &str, ttl: &Duration) -> Result<Option<String>> {
        let mut wlock = self.data.write();
        if let Some(v) = wlock.get_mut(key) {
            if v.valid() {
                let ret = v.0.to_owned();
                let ms = ttl.as_millis();
                if ms == 0 {
                    wlock.remove(key);
                } else {
                    v.set_ttl(Some(ms.try_into()?));
                }
                Ok(Some(ret))
            } else {
                wlock.remove(key);
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    async fn set_ex(&self, key: &str, value: &str, ttl: &Duration) -> Result<()> {
        let ms = ttl.as_millis();
        if ms == 0 {
            return self.del(key).await.map(|_| ());
        }
        let mut wlock = self.data.write();
        // When full and inserting a new key, try to make room; fail if nothing is
        // evictable (all remaining entries are live and have no TTL).
        if let Some(x) = self.capacity {
            if x <= wlock.len()
                && wlock.get(key).is_none()
                && self.gc(&mut wlock, max(x / 10, 1)) == 0
            {
                bail!("capacity is full");
            }
        }
        wlock.insert(key.to_owned(), Data::new(value, Some(ms.try_into()?)));
        Ok(())
    }

    async fn del(&self, key: &str) -> Result<bool> {
        // Expired entries are logically absent even if they have not been reaped yet.
        Ok(match self.data.write().remove(key) {
            Some(v) => v.valid(),
            None => false,
        })
    }

    async fn expire(&self, key: &str, ttl: &Duration) -> Result<bool> {
        let ms = ttl.as_millis();
        if ms == 0 {
            self.del(key).await
        } else {
            let mut wlock = self.data.write();
            if let Some(v) = wlock.get_mut(key) {
                if v.valid() {
                    v.set_ttl(Some(ms.try_into()?));
                    Ok(true)
                } else {
                    wlock.remove(key);
                    Ok(false)
                }
            } else {
                Ok(false)
            }
        }
    }

    async fn flush(&self) -> Result<()> {
        self.data.write().clear();
        Ok(())
    }

    async fn keys(&self, key: &str) -> Result<Vec<String>> {
        let mut ret = Vec::new();
        let p = Pattern::new(key)?;
        for (k, v) in self.data.read().iter() {
            if v.valid() && p.matches(k) {
                ret.push(k.to_owned());
            }
        }
        Ok(ret)
    }

    async fn dels(&self, keys: &[String]) -> Result<u64> {
        let mut wlock = self.data.write();
        let mut sum = 0;
        for i in keys {
            // Expired entries are removed but not counted as deleted.
            if wlock.remove(i).is_some_and(|v| v.valid()) {
                sum += 1;
            }
        }
        Ok(sum)
    }

    async fn ttl(&self, key: &str) -> Result<Option<Duration>> {
        let rlock = self.data.read();
        if let Some(v) = rlock.get(key) {
            if v.valid() {
                v.get_ttl()
                    .map(|x| Ok(Duration::from_millis(x.try_into()?)))
                    .transpose()
            } else {
                drop(rlock);
                self.data.write().remove(key);
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend() -> DefaultBackend {
        DefaultBackend::new(None)
    }

    #[test]
    fn data_expiration_helpers() {
        let mut data = Data::new("value", None);
        assert!(data.valid());
        assert_eq!(data.get_ttl(), None);

        data.set_ttl(Some(60_000));
        assert!(data.valid());
        assert!((59_000..=60_000).contains(&data.get_ttl().unwrap()));

        data.set_ttl(Some(0));
        assert!(!data.valid());
        assert!(data.get_ttl().unwrap() <= 0);
    }

    #[tokio::test]
    async fn get_set_del_roundtrip() {
        let db = backend();

        assert_eq!(db.get("key").await.unwrap(), None);
        assert_eq!(db.get_del("key").await.unwrap(), None);

        db.set("key", "value1").await.unwrap();
        assert_eq!(db.get("key").await.unwrap().as_deref(), Some("value1"));
        assert_eq!(db.ttl("key").await.unwrap(), None);

        db.set("key", "value2").await.unwrap();
        assert_eq!(db.get("key").await.unwrap().as_deref(), Some("value2"));

        assert!(db.del("key").await.unwrap());
        assert!(!db.del("key").await.unwrap());
        assert_eq!(db.get("key").await.unwrap(), None);
        assert_eq!(db.ttl("key").await.unwrap(), None);
    }

    #[tokio::test]
    async fn get_del_removes_key() {
        let db = backend();

        db.set("key", "value").await.unwrap();
        assert_eq!(db.get_del("key").await.unwrap().as_deref(), Some("value"));
        assert_eq!(db.get("key").await.unwrap(), None);
    }

    #[tokio::test]
    async fn zero_ttl_deletes_key() {
        let db = backend();

        db.set("set_ex", "value").await.unwrap();
        db.set_ex("set_ex", "value", &Duration::ZERO).await.unwrap();
        assert_eq!(db.get("set_ex").await.unwrap(), None);

        db.set_ex("get_ex", "value", &Duration::from_secs(60))
            .await
            .unwrap();
        assert_eq!(
            db.get_ex("get_ex", &Duration::ZERO)
                .await
                .unwrap()
                .as_deref(),
            Some("value")
        );
        assert_eq!(db.get("get_ex").await.unwrap(), None);

        db.set("expire", "value").await.unwrap();
        assert!(db.expire("expire", &Duration::ZERO).await.unwrap());
        assert_eq!(db.get("expire").await.unwrap(), None);
    }

    #[tokio::test]
    async fn ttl_reports_remaining_time() {
        let db = backend();

        assert_eq!(db.ttl("missing").await.unwrap(), None);

        db.set("persistent", "value").await.unwrap();
        assert_eq!(db.ttl("persistent").await.unwrap(), None);

        db.set_ex("expiring", "value", &Duration::from_secs(60))
            .await
            .unwrap();
        let ttl = db.ttl("expiring").await.unwrap().unwrap();
        assert!(ttl > Duration::from_secs(59));
        assert!(ttl <= Duration::from_secs(60));
    }

    #[tokio::test]
    async fn keys_supports_globs_and_hides_expired_entries() {
        let db = backend();

        db.set("user:1", "a").await.unwrap();
        db.set("user:2", "b").await.unwrap();
        db.set("other:1", "c").await.unwrap();
        // Simulate an expired entry that has not been reaped yet.
        {
            let mut data = db.data.write();
            data.insert(
                "user:expired".to_owned(),
                Data("x".to_owned(), Some(Data::now() - 1)),
            );
        }

        let mut keys = db.keys("user:*").await.unwrap();
        keys.sort();
        assert_eq!(keys, ["user:1", "user:2"]);
    }

    #[tokio::test]
    async fn dels_and_flush() {
        let db = backend();

        db.set("key1", "value").await.unwrap();
        db.set("key2", "value").await.unwrap();
        assert_eq!(
            db.dels(&["key1".to_owned(), "key2".to_owned(), "missing".to_owned()])
                .await
                .unwrap(),
            2
        );
        assert!(db.keys("*").await.unwrap().is_empty());

        db.set("key3", "value").await.unwrap();
        db.flush().await.unwrap();
        assert!(db.keys("*").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn del_ignores_expired_entries() {
        let db = backend();

        db.set("live", "value").await.unwrap();
        // Simulate an expired entry that has not been reaped yet.
        {
            let mut data = db.data.write();
            data.insert(
                "expired".to_owned(),
                Data("value".to_owned(), Some(Data::now() - 1)),
            );
        }

        assert!(!db.del("expired").await.unwrap());
        assert!(!db.data.read().contains_key("expired"));

        {
            let mut data = db.data.write();
            data.insert(
                "expired".to_owned(),
                Data("value".to_owned(), Some(Data::now() - 1)),
            );
        }
        assert_eq!(
            db.dels(&["live".to_owned(), "expired".to_owned()])
                .await
                .unwrap(),
            1
        );
        assert!(db.keys("*").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn capacity_rejects_when_nothing_is_evictable() {
        let db = DefaultBackend::new(Some(2));

        db.set("key1", "value").await.unwrap();
        db.set("key2", "value").await.unwrap();

        // Full, and no entry has a TTL, so GC cannot free anything.
        assert!(db.set("key3", "value").await.is_err());
        // Overwriting an existing key does not need free space.
        assert!(db.set("key1", "new").await.is_ok());
        assert_eq!(db.get("key1").await.unwrap().as_deref(), Some("new"));
    }

    #[tokio::test]
    async fn capacity_evicts_shortest_ttl() {
        let db = DefaultBackend::new(Some(2));

        db.set_ex("long", "value", &Duration::from_secs(60))
            .await
            .unwrap();
        db.set_ex("short", "value", &Duration::from_secs(10))
            .await
            .unwrap();
        db.set("new", "value").await.unwrap();

        assert_eq!(db.get("short").await.unwrap(), None);
        assert!(db.get("long").await.unwrap().is_some());
        assert_eq!(db.get("new").await.unwrap().as_deref(), Some("value"));
    }

    #[tokio::test]
    async fn capacity_evicts_expired_entries_first() {
        let db = DefaultBackend::new(Some(2));

        db.set_ex("live", "value", &Duration::from_secs(60))
            .await
            .unwrap();
        // Simulate an expired entry that has not been reaped yet.
        {
            let mut data = db.data.write();
            data.insert(
                "expired".to_owned(),
                Data("value".to_owned(), Some(Data::now() - 1)),
            );
        }

        db.set("new", "value").await.unwrap();

        assert!(!db.data.read().contains_key("expired"));
        assert!(db.get("live").await.unwrap().is_some());
        assert_eq!(db.get("new").await.unwrap().as_deref(), Some("value"));
    }
}
