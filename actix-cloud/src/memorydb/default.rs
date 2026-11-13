//! In-process [`MemoryDB`] backend.
//!
//! Entries live in a `HashMap` behind an `RwLock`. Expiration timestamps are absolute
//! milliseconds; expired entries are dropped lazily when accessed and eagerly during
//! entry/byte-limit-driven garbage collection.
//!
//! See [`DefaultBackend`] for the eviction policy and its production caveats.
use std::{cmp::max, collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use chrono::Utc;
use glob::Pattern;
use parking_lot::RwLock;

use super::interface::{MemoryDB, MemoryDBError, MemoryDBResult};

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

/// Convert a TTL to milliseconds, rejecting values that do not fit in `i64`.
fn ttl_millis(ttl: &Duration) -> MemoryDBResult<i64> {
    i64::try_from(ttl.as_millis()).map_err(|_| MemoryDBError::TtlOutOfRange)
}

/// Storage plus the running total of stored payload bytes.
#[derive(Default)]
struct Store {
    map: HashMap<String, Data>,
    /// Sum of `key.len() + value.len()` for every entry in `map`, including entries
    /// that have expired but have not been reaped yet.
    used_bytes: usize,
}

impl Store {
    /// Payload bytes of an entry: UTF-8 key bytes plus UTF-8 value bytes.
    fn entry_bytes(key: &str, data: &Data) -> usize {
        key.len().saturating_add(data.0.len())
    }

    /// Insert or overwrite an entry, keeping `used_bytes` in sync.
    fn insert(&mut self, key: String, data: Data) {
        let key_bytes = key.len();
        let new_bytes = Self::entry_bytes(&key, &data);
        match self.map.insert(key, data) {
            Some(old) => {
                // A replaced key is byte-for-byte equal, so it has the same length.
                self.used_bytes = self
                    .used_bytes
                    .saturating_sub(key_bytes.saturating_add(old.0.len()))
                    .saturating_add(new_bytes);
            }
            None => {
                self.used_bytes = self.used_bytes.saturating_add(new_bytes);
            }
        }
    }

    /// Remove an entry, returning it and its payload size.
    fn take(&mut self, key: &str) -> Option<(Data, usize)> {
        let data = self.map.remove(key)?;
        let bytes = Self::entry_bytes(key, &data);
        self.used_bytes = self.used_bytes.saturating_sub(bytes);
        Some((data, bytes))
    }

    /// Remove an entry, keeping `used_bytes` in sync.
    fn remove(&mut self, key: &str) -> Option<Data> {
        self.take(key).map(|(data, _)| data)
    }

    /// Remove every entry.
    fn clear(&mut self) {
        self.map.clear();
        self.used_bytes = 0;
    }

    /// Evict expired entries, then live TTL entries shortest-TTL-first, until both
    /// quotas are met or no candidates remain.
    ///
    /// `exclude` is never a candidate. Expired entries are always reaped; live
    /// entries are only evicted when the candidate pool can satisfy the hard needs.
    fn evict(&mut self, exclude: &str, entries: Quota, bytes: Quota) -> MemoryDBResult<()> {
        let mut freed_entries: usize = 0;
        let mut freed_bytes: usize = 0;
        let mut candidates = Vec::new();
        let mut candidate_bytes: usize = 0;

        self.map.retain(|k, v| {
            if k == exclude {
                return true;
            }
            if !v.valid() {
                freed_entries += 1;
                freed_bytes = freed_bytes.saturating_add(Self::entry_bytes(k, v));
                return false;
            }
            if let Some(expiry) = v.1 {
                if bytes.need > 0 {
                    candidate_bytes = candidate_bytes.saturating_add(Self::entry_bytes(k, v));
                }
                candidates.push((k.to_owned(), expiry));
            }
            true
        });
        self.used_bytes = self.used_bytes.saturating_sub(freed_bytes);

        if freed_entries.saturating_add(candidates.len()) < entries.need {
            return Err(MemoryDBError::MaxEntriesExceeded);
        }
        if bytes.need > 0 && freed_bytes.saturating_add(candidate_bytes) < bytes.need {
            return Err(MemoryDBError::MaxBytesExceeded);
        }

        candidates.sort_unstable_by_key(|(_, expiry)| *expiry);
        for (k, _) in candidates {
            if freed_entries >= entries.want && freed_bytes >= bytes.want {
                break;
            }
            if let Some((_, evicted_bytes)) = self.take(&k) {
                freed_entries += 1;
                freed_bytes = freed_bytes.saturating_add(evicted_bytes);
            }
        }
        Ok(())
    }

    /// Verify the byte counter matches the map contents (tests only).
    #[cfg(test)]
    fn assert_accounting(&self) {
        let sum = self
            .map
            .iter()
            .map(|(k, v)| Self::entry_bytes(k, v))
            .sum::<usize>();
        assert_eq!(self.used_bytes, sum, "used_bytes drifted from map contents");
    }
}

/// How much one limit requires a write to free, plus its 10% batch target.
#[derive(Default, Clone, Copy)]
struct Quota {
    need: usize,
    want: usize,
}

impl Quota {
    /// Compute the quota for one limit against the projected usage after the write.
    fn new(limit: Option<usize>, used_after_write: usize) -> Self {
        let Some(limit) = limit else {
            return Self::default();
        };
        let need = used_after_write.saturating_sub(limit);
        let want = if need > 0 {
            need.max(max(limit / 10, 1))
        } else {
            0
        };
        Self { need, want }
    }
}

/// Built-in in-process memory database backend.
///
/// Keys and values live in a `HashMap` guarded by an `RwLock`; expired entries are
/// removed lazily on access, plus eagerly during limit-driven GC.
///
/// Limits are optional and can be combined. [`max_entries`](Self::max_entries) bounds
/// the number of entries and applies when a write adds a new key; overwriting an
/// existing key does not change the count, so it is not subject to this limit.
/// [`max_bytes`](Self::max_bytes) bounds payload bytes as `key.len() + value.len()`
/// in UTF-8 (internal allocations and TTL metadata are not counted) and applies to
/// every write. Expired entries keep counting toward both limits until they are
/// reaped, either lazily or during GC.
///
/// When a write would exceed a limit, the eviction policy is:
/// - Evict a batch of 10% of the exceeded limit (at least 1 entry or 1 byte).
/// - Expired entries go first, then entries having a TTL attribute, sorted by TTL
///   from shortest to longest. Entries without a TTL are never evicted.
/// - A single key+value larger than `max_bytes` is rejected without evicting anything.
/// - If the write still cannot be accommodated, an error is returned.
///
/// # Warning
/// The default backend is only for demo purpose. The performance is not guaranteed.
/// Use [`RedisBackend`](super::redis::RedisBackend) or your own [`MemoryDB`]
/// implementation for production workloads.
#[derive(Clone, Default)]
pub struct DefaultBackend {
    data: Arc<RwLock<Store>>,
    max_entries: Option<usize>,
    max_bytes: Option<usize>,
}

impl DefaultBackend {
    /// Create a new backend with no resource limits.
    ///
    /// Use [`max_entries`](Self::max_entries) and [`max_bytes`](Self::max_bytes) to
    /// bound the database. Without limits it can grow until memory is exhausted.
    pub fn new() -> Self {
        Self::default()
    }

    /// Limit the maximum number of entries; `None` means unbounded.
    ///
    /// Overwriting an existing key does not change the entry count and is not subject
    /// to this limit.
    #[must_use]
    pub fn max_entries(mut self, max_entries: Option<usize>) -> Self {
        self.max_entries = max_entries;
        self
    }

    /// Limit the total payload bytes (`key.len() + value.len()`, UTF-8);
    /// `None` means unbounded.
    #[must_use]
    pub fn max_bytes(mut self, max_bytes: Option<usize>) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    /// Enforce both limits before writing `key`.
    ///
    /// Returns an error if the entry alone exceeds `max_bytes`, or if eviction cannot
    /// free enough entries/bytes. On error, live TTL entries are only evicted when the
    /// write is feasible, so a doomed write does not evict survivors.
    fn prepare_write(&self, store: &mut Store, key: &str, data: &Data) -> MemoryDBResult<()> {
        if self.max_entries.is_none() && self.max_bytes.is_none() {
            return Ok(());
        }

        let incoming = Store::entry_bytes(key, data);
        if self.max_bytes.is_some_and(|limit| incoming > limit) {
            return Err(MemoryDBError::EntryTooLarge);
        }

        let old = store.map.get(key);
        let is_new = old.is_none();
        let old_bytes = old.map_or(0, |v| Store::entry_bytes(key, v));

        // Only a new key grows the entry count; overwrites never need entry room.
        let entries = if is_new {
            Quota::new(self.max_entries, store.map.len().saturating_add(1))
        } else {
            Quota::default()
        };
        // Overwriting an existing key reclaims its old bytes automatically.
        let bytes = Quota::new(
            self.max_bytes,
            store
                .used_bytes
                .saturating_sub(old_bytes)
                .saturating_add(incoming),
        );

        if entries.need == 0 && bytes.need == 0 {
            return Ok(());
        }
        store.evict(key, entries, bytes)
    }

    /// Shared implementation of [`MemoryDB::set`] and [`MemoryDB::set_ex`].
    fn set_inner(&self, key: &str, value: &str, ttl: Option<i64>) -> MemoryDBResult<()> {
        let data = Data::new(value, ttl);
        let mut store = self.data.write();
        self.prepare_write(&mut store, key, &data)?;
        store.insert(key.to_owned(), data);
        Ok(())
    }
}

#[async_trait]
impl MemoryDB for DefaultBackend {
    async fn set(&self, key: &str, value: &str) -> MemoryDBResult<()> {
        self.set_inner(key, value, None)
    }

    async fn get(&self, key: &str) -> MemoryDBResult<Option<String>> {
        let rlock = self.data.read();
        if let Some(v) = rlock.map.get(key) {
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

    async fn get_del(&self, key: &str) -> MemoryDBResult<Option<String>> {
        let v = self.data.write().remove(key);
        if let Some(v) = v {
            if v.valid() {
                return Ok(Some(v.0));
            }
        }
        Ok(None)
    }

    async fn get_ex(&self, key: &str, ttl: &Duration) -> MemoryDBResult<Option<String>> {
        let mut wlock = self.data.write();
        if let Some(v) = wlock.map.get_mut(key) {
            if v.valid() {
                let ret = v.0.to_owned();
                let ms = ttl_millis(ttl)?;
                if ms == 0 {
                    wlock.remove(key);
                } else {
                    v.set_ttl(Some(ms));
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

    async fn set_ex(&self, key: &str, value: &str, ttl: &Duration) -> MemoryDBResult<()> {
        let ms = ttl_millis(ttl)?;
        if ms == 0 {
            return self.del(key).await.map(|_| ());
        }
        self.set_inner(key, value, Some(ms))
    }

    async fn del(&self, key: &str) -> MemoryDBResult<bool> {
        // Expired entries are logically absent even if they have not been reaped yet.
        Ok(match self.data.write().remove(key) {
            Some(v) => v.valid(),
            None => false,
        })
    }

    async fn expire(&self, key: &str, ttl: &Duration) -> MemoryDBResult<bool> {
        let ms = ttl_millis(ttl)?;
        if ms == 0 {
            self.del(key).await
        } else {
            let mut wlock = self.data.write();
            if let Some(v) = wlock.map.get_mut(key) {
                if v.valid() {
                    v.set_ttl(Some(ms));
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

    async fn flush(&self) -> MemoryDBResult<()> {
        self.data.write().clear();
        Ok(())
    }

    async fn keys(&self, key: &str) -> MemoryDBResult<Vec<String>> {
        let mut ret = Vec::new();
        let p = Pattern::new(key).map_err(|e| MemoryDBError::InvalidPattern(e.to_string()))?;
        for (k, v) in self.data.read().map.iter() {
            if v.valid() && p.matches(k) {
                ret.push(k.to_owned());
            }
        }
        Ok(ret)
    }

    async fn dels(&self, keys: &[String]) -> MemoryDBResult<u64> {
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

    async fn ttl(&self, key: &str) -> MemoryDBResult<Option<Duration>> {
        let rlock = self.data.read();
        if let Some(v) = rlock.map.get(key) {
            if v.valid() {
                v.get_ttl()
                    .map(|x| {
                        u64::try_from(x)
                            .map(Duration::from_millis)
                            .map_err(|_| MemoryDBError::TtlOutOfRange)
                    })
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
        DefaultBackend::new()
    }

    fn used_bytes(db: &DefaultBackend) -> usize {
        let store = db.data.read();
        store.assert_accounting();
        store.used_bytes
    }

    fn insert_expired(db: &DefaultBackend, key: &str, value: &str) {
        db.data
            .write()
            .insert(key.to_owned(), Data::new(value, Some(-1)));
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
        insert_expired(&db, "user:expired", "x");

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
        insert_expired(&db, "expired", "value");

        assert!(!db.del("expired").await.unwrap());
        assert!(!db.data.read().map.contains_key("expired"));

        insert_expired(&db, "expired", "value");
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
        let db = DefaultBackend::new().max_entries(Some(2));

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
        let db = DefaultBackend::new().max_entries(Some(2));

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
        let db = DefaultBackend::new().max_entries(Some(2));

        db.set_ex("live", "value", &Duration::from_secs(60))
            .await
            .unwrap();
        // Simulate an expired entry that has not been reaped yet.
        insert_expired(&db, "expired", "value");

        db.set("new", "value").await.unwrap();

        assert!(!db.data.read().map.contains_key("expired"));
        assert!(db.get("live").await.unwrap().is_some());
        assert_eq!(db.get("new").await.unwrap().as_deref(), Some("value"));
    }

    #[tokio::test]
    async fn max_bytes_rejects_when_nothing_is_evictable() {
        let db = DefaultBackend::new().max_bytes(Some(10));

        db.set("a", "aaaa").await.unwrap(); // 1 + 4 = 5 bytes
        db.set("b", "aaaa").await.unwrap(); // 1 + 4 = 5 bytes
        assert_eq!(used_bytes(&db), 10);

        // Full, and no entry has a TTL, so GC cannot free anything.
        assert!(db.set("c", "aa").await.is_err());
        assert_eq!(used_bytes(&db), 10);
        // Shrinking overwrites always fit.
        assert!(db.set("a", "aa").await.is_ok()); // 1 + 2 = 3 bytes
        assert_eq!(used_bytes(&db), 8);
        // Growing overwrites that no longer fit are rejected and keep the old value.
        assert!(db.set("a", "aaaaaaaa").await.is_err()); // 1 + 8 = 9 bytes
        assert_eq!(used_bytes(&db), 8);
        assert_eq!(db.get("a").await.unwrap().as_deref(), Some("aa"));
    }

    #[tokio::test]
    async fn max_bytes_evicts_shortest_ttl() {
        let db = DefaultBackend::new().max_bytes(Some(20));

        db.set_ex("long", "123456", &Duration::from_secs(60))
            .await
            .unwrap(); // 4 + 6 = 10 bytes
        db.set_ex("short", "12345", &Duration::from_secs(10))
            .await
            .unwrap(); // 5 + 5 = 10 bytes
        db.set("new", "1234").await.unwrap(); // 3 + 4 = 7 bytes

        assert_eq!(db.get("short").await.unwrap(), None);
        assert!(db.get("long").await.unwrap().is_some());
        assert_eq!(db.get("new").await.unwrap().as_deref(), Some("1234"));
        assert_eq!(used_bytes(&db), 17);
    }

    #[tokio::test]
    async fn max_bytes_evicts_expired_entries_first() {
        let db = DefaultBackend::new().max_bytes(Some(20));

        db.set_ex("live", "123456", &Duration::from_secs(60))
            .await
            .unwrap(); // 4 + 6 = 10 bytes
        insert_expired(&db, "exp", "123456"); // 3 + 6 = 9 bytes
        assert_eq!(used_bytes(&db), 19);

        db.set("new", "1234").await.unwrap(); // 3 + 4 = 7 bytes

        assert!(!db.data.read().map.contains_key("exp"));
        assert!(db.get("live").await.unwrap().is_some());
        assert_eq!(db.get("new").await.unwrap().as_deref(), Some("1234"));
        assert_eq!(used_bytes(&db), 17);
    }

    #[tokio::test]
    async fn max_bytes_evicts_ten_percent_batch() {
        let db = DefaultBackend::new().max_bytes(Some(100));

        for i in 0..20u64 {
            let key = format!("k{:02}", i); // 3 bytes
            db.set_ex(&key, "vv", &Duration::from_secs(i + 1))
                .await
                .unwrap(); // 3 + 2 = 5 bytes
        }
        assert_eq!(used_bytes(&db), 100);

        db.set("n", "").await.unwrap(); // 1 byte

        // The write needs 1 byte, but the 10% batch target is 10 bytes: the two
        // shortest-TTL entries are evicted.
        assert_eq!(db.get("k00").await.unwrap(), None);
        assert_eq!(db.get("k01").await.unwrap(), None);
        assert!(db.get("k02").await.unwrap().is_some());
        assert_eq!(db.get("n").await.unwrap().as_deref(), Some(""));
        assert_eq!(used_bytes(&db), 91);
    }

    #[tokio::test]
    async fn max_bytes_overwrite_accounting() {
        let db = DefaultBackend::new().max_bytes(Some(100));

        db.set("key", "aa").await.unwrap(); // 5 bytes
        assert_eq!(used_bytes(&db), 5);
        db.set("key", "aaaaaaaa").await.unwrap(); // 11 bytes
        assert_eq!(used_bytes(&db), 11);
        db.set("key", "a").await.unwrap(); // 4 bytes
        assert_eq!(used_bytes(&db), 4);
        assert_eq!(db.get("key").await.unwrap().as_deref(), Some("a"));
    }

    #[tokio::test]
    async fn max_entries_and_max_bytes_both_binding() {
        let db = DefaultBackend::new()
            .max_entries(Some(3))
            .max_bytes(Some(30));

        db.set_ex("a", "111111111", &Duration::from_secs(30))
            .await
            .unwrap(); // 10 bytes
        db.set_ex("b", "222222222", &Duration::from_secs(20))
            .await
            .unwrap(); // 10 bytes
        db.set_ex("c", "333333333", &Duration::from_secs(10))
            .await
            .unwrap(); // 10 bytes
        assert_eq!(used_bytes(&db), 30);

        db.set("d", "444444444").await.unwrap(); // 10 bytes

        // Both limits were exceeded; one GC pass frees one entry and 10 bytes.
        assert_eq!(db.get("c").await.unwrap(), None);
        assert!(db.get("a").await.unwrap().is_some());
        assert!(db.get("b").await.unwrap().is_some());
        assert_eq!(db.get("d").await.unwrap().as_deref(), Some("444444444"));
        assert_eq!(db.keys("*").await.unwrap().len(), 3);
        assert_eq!(used_bytes(&db), 30);
    }

    #[tokio::test]
    async fn max_bytes_rejects_oversized_entry_without_eviction() {
        let db = DefaultBackend::new().max_bytes(Some(10));

        db.set_ex("a", "bbbb", &Duration::from_secs(60))
            .await
            .unwrap(); // 5 bytes
        db.set_ex("b", "cccc", &Duration::from_secs(60))
            .await
            .unwrap(); // 5 bytes

        // 1 + 10 = 11 bytes: larger than the whole limit, so reject immediately.
        assert!(db.set("x", "yyyyyyyyyy").await.is_err());

        assert!(db.get("a").await.unwrap().is_some());
        assert!(db.get("b").await.unwrap().is_some());
        assert_eq!(db.keys("*").await.unwrap().len(), 2);
        assert_eq!(used_bytes(&db), 10);
    }

    #[tokio::test]
    async fn max_bytes_failure_does_not_evict_live_ttl_entries() {
        let db = DefaultBackend::new()
            .max_entries(Some(2))
            .max_bytes(Some(30));

        db.set("big", "12345678901234567").await.unwrap(); // 3 + 17 = 20 bytes
        db.set_ex("ttl", "1234", &Duration::from_secs(60))
            .await
            .unwrap(); // 3 + 4 = 7 bytes
        assert_eq!(used_bytes(&db), 27);

        // Needs one entry (feasible) and 9 bytes (only 7 are evictable).
        assert!(db.set("new", "123456789").await.is_err());

        assert!(db.get("big").await.unwrap().is_some());
        assert!(db.get("ttl").await.unwrap().is_some());
        assert_eq!(db.keys("*").await.unwrap().len(), 2);
        assert_eq!(used_bytes(&db), 27);
    }

    #[tokio::test]
    async fn max_bytes_overwrite_excludes_target_from_gc() {
        let db = DefaultBackend::new().max_bytes(Some(20));

        db.set_ex("a", "11111", &Duration::from_secs(10))
            .await
            .unwrap(); // 6 bytes, shortest TTL
        db.set_ex("b", "22222", &Duration::from_secs(20))
            .await
            .unwrap(); // 6 bytes
        db.set_ex("c", "33333", &Duration::from_secs(30))
            .await
            .unwrap(); // 6 bytes
        assert_eq!(used_bytes(&db), 18);

        // Overwriting `a` needs 2 more bytes; `a` itself must not be a GC candidate.
        db.set("a", "111111111").await.unwrap(); // 1 + 9 = 10 bytes

        assert_eq!(db.get("a").await.unwrap().as_deref(), Some("111111111"));
        assert_eq!(db.get("b").await.unwrap(), None);
        assert!(db.get("c").await.unwrap().is_some());
        assert_eq!(used_bytes(&db), 16);
    }

    #[tokio::test]
    async fn accounting_tracks_lazy_expiry() {
        let db = backend();

        db.set("live", "value").await.unwrap(); // 9 bytes
        insert_expired(&db, "expired", "value"); // 12 bytes
        assert_eq!(used_bytes(&db), 21);

        assert_eq!(db.get("expired").await.unwrap(), None);
        assert!(!db.data.read().map.contains_key("expired"));
        assert_eq!(used_bytes(&db), 9);

        insert_expired(&db, "ttl", "value"); // 8 bytes
        assert_eq!(used_bytes(&db), 17);
        assert_eq!(db.ttl("ttl").await.unwrap(), None);
        assert!(!db.data.read().map.contains_key("ttl"));
        assert_eq!(used_bytes(&db), 9);
    }

    #[tokio::test]
    async fn accounting_tracks_deletes_and_flush() {
        let db = backend();

        db.set("a", "1234").await.unwrap(); // 5 bytes
        db.set("b", "1234").await.unwrap(); // 5 bytes
        assert_eq!(used_bytes(&db), 10);

        assert_eq!(db.get_del("a").await.unwrap().as_deref(), Some("1234"));
        assert_eq!(used_bytes(&db), 5);
        assert!(db.del("b").await.unwrap());
        assert_eq!(used_bytes(&db), 0);

        db.set_ex("c", "1234", &Duration::from_secs(60))
            .await
            .unwrap();
        db.set_ex("d", "1234", &Duration::from_secs(60))
            .await
            .unwrap();
        assert_eq!(used_bytes(&db), 10);
        assert_eq!(
            db.dels(&["c".to_owned(), "d".to_owned(), "missing".to_owned()])
                .await
                .unwrap(),
            2
        );
        assert_eq!(used_bytes(&db), 0);

        db.set("e", "1234").await.unwrap();
        db.set_ex("f", "1234", &Duration::from_secs(60))
            .await
            .unwrap();
        assert_eq!(used_bytes(&db), 10);
        assert!(db.expire("e", &Duration::ZERO).await.unwrap());
        assert_eq!(used_bytes(&db), 5);
        assert_eq!(
            db.get_ex("f", &Duration::ZERO).await.unwrap().as_deref(),
            Some("1234")
        );
        assert_eq!(used_bytes(&db), 0);

        db.set("g", "1234").await.unwrap();
        db.flush().await.unwrap();
        assert_eq!(used_bytes(&db), 0);
        assert!(db.keys("*").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn accounting_tracks_get_ex_and_expire() {
        let db = backend();

        db.set_ex("key", "value", &Duration::from_secs(60))
            .await
            .unwrap(); // 8 bytes
        assert_eq!(used_bytes(&db), 8);

        // Refreshing a TTL does not change the payload size.
        assert_eq!(
            db.get_ex("key", &Duration::from_secs(120))
                .await
                .unwrap()
                .as_deref(),
            Some("value")
        );
        assert_eq!(used_bytes(&db), 8);
        assert!(db.expire("key", &Duration::from_secs(120)).await.unwrap());
        assert_eq!(used_bytes(&db), 8);

        // A zero TTL deletes the entry.
        assert_eq!(
            db.get_ex("key", &Duration::ZERO).await.unwrap().as_deref(),
            Some("value")
        );
        assert_eq!(used_bytes(&db), 0);

        db.set("key2", "value").await.unwrap(); // 9 bytes
        assert!(db.expire("key2", &Duration::ZERO).await.unwrap());
        assert_eq!(used_bytes(&db), 0);
    }

    #[tokio::test]
    async fn overwrite_is_not_limited_by_max_entries() {
        let db = DefaultBackend::new().max_entries(Some(10));
        for i in 0..10 {
            db.set(&format!("k{i}"), "value").await.unwrap();
        }

        // Overwrites do not change the entry count, so `max_entries` does not apply;
        // only the byte limit would.
        let db = db.max_entries(Some(2));
        assert!(db.set("k0", "new").await.is_ok());
        assert_eq!(db.keys("*").await.unwrap().len(), 10);
        assert_eq!(db.get("k0").await.unwrap().as_deref(), Some("new"));
    }

    #[tokio::test]
    async fn clone_shares_limits_and_storage() {
        let db = DefaultBackend::new()
            .max_entries(Some(1))
            .max_bytes(Some(3));
        let clone = db.clone();

        clone.set("a", "").await.unwrap(); // 1 byte
        assert_eq!(db.get("a").await.unwrap().as_deref(), Some(""));

        // The entry limit is shared and leaves no evictable entry.
        assert!(clone.set("b", "").await.is_err());
        assert!(db.del("a").await.unwrap());

        // The byte limit is shared too.
        db.set("c", "").await.unwrap(); // 1 byte
        db.set("c", "12").await.unwrap(); // 3 bytes
        assert_eq!(used_bytes(&db), 3);
        assert!(clone.set("c", "123").await.is_err()); // 4 bytes
        assert_eq!(used_bytes(&clone), 3);
    }
}
