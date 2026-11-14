//! The [`MemoryDB`] trait and the conformance tests shared by all backends.
//!
//! Backends store string values under string keys with millisecond-precision TTLs.
//! New backends should be plugged into the generic tests at the bottom of this module.
use std::{
    error::Error,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;

/// Errors returned by [`MemoryDB`] operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MemoryDBError {
    /// A write would exceed the backend's entry limit.
    #[error("max entries exceeded")]
    MaxEntriesExceeded,
    /// A write would exceed the backend's byte limit.
    #[error("max bytes exceeded")]
    MaxBytesExceeded,
    /// A single entry is larger than the backend's byte limit.
    #[error("entry is larger than max bytes")]
    EntryTooLarge,
    /// A TTL cannot be represented in milliseconds by the backend, or its absolute
    /// expiration timestamp would overflow.
    #[error("ttl is out of range")]
    TtlOutOfRange,
    /// The glob pattern passed to [`MemoryDB::keys`] is invalid.
    #[error("invalid key pattern: {0}")]
    InvalidPattern(String),
    /// The Redis backend returned an error.
    #[cfg(feature = "redis")]
    #[error(transparent)]
    Redis(#[from] redis::RedisError),
    /// An error from a custom backend.
    #[error("{0}")]
    Other(#[source] Box<dyn Error + Send + Sync>),
}

impl MemoryDBError {
    /// Wrap an arbitrary backend error.
    pub fn other<E>(err: E) -> Self
    where
        E: Into<Box<dyn Error + Send + Sync>>,
    {
        Self::Other(err.into())
    }
}

/// [`Result`] alias with [`MemoryDBError`] as the error type.
pub type MemoryDBResult<T> = Result<T, MemoryDBError>;

/// Convert a TTL to milliseconds, rejecting values the backend cannot represent.
///
/// The TTL must fit in `i64` milliseconds and the resulting absolute expiration must
/// not overflow; otherwise [`MemoryDBError::TtlOutOfRange`] is returned. Shared by the
/// built-in backends so that they accept and reject the same TTLs.
pub(crate) fn ttl_millis(ttl: &Duration) -> MemoryDBResult<i64> {
    let ms = i64::try_from(ttl.as_millis()).map_err(|_| MemoryDBError::TtlOutOfRange)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    now.checked_add(ms).ok_or(MemoryDBError::TtlOutOfRange)?;
    Ok(ms)
}

/// Compile a key glob, mapping parse errors to [`MemoryDBError::InvalidPattern`].
///
/// Both built-in backends validate patterns with the same parser so that invalid
/// patterns fail consistently.
pub(crate) fn pattern(key: &str) -> MemoryDBResult<glob::Pattern> {
    glob::Pattern::new(key).map_err(|e| MemoryDBError::InvalidPattern(e.to_string()))
}

/// Object-safe async key-value database trait.
///
/// All values are strings and all TTLs are millisecond-precision [`Duration`]s.
/// This is the abstraction behind session storage and CSRF token storage; you can
/// implement it to plug in your own backend (see [`DefaultBackend`](super::default::DefaultBackend)
/// and [`RedisBackend`](super::redis::RedisBackend) for reference implementations).
///
/// Backends are shared as `Arc<dyn MemoryDB>` throughout the framework.
#[async_trait]
pub trait MemoryDB: Send + Sync {
    /// Set `key` to `value` without expiration.
    async fn set(&self, key: &str, value: &str) -> MemoryDBResult<()>;

    /// Get the value of `key`, or `None` if it does not exist or has expired.
    async fn get(&self, key: &str) -> MemoryDBResult<Option<String>>;

    /// Get the value of `key` and delete it atomically.
    async fn get_del(&self, key: &str) -> MemoryDBResult<Option<String>>;

    /// Get the value of `key` and reset its TTL to `ttl`.
    ///
    /// A zero `ttl` deletes the key after reading it.
    async fn get_ex(&self, key: &str, ttl: &Duration) -> MemoryDBResult<Option<String>>;

    /// Set `key` to `value` with an expiration of `ttl`.
    ///
    /// A zero `ttl` deletes the key.
    async fn set_ex(&self, key: &str, value: &str, ttl: &Duration) -> MemoryDBResult<()>;

    /// Delete `key`. Returns whether the key existed.
    ///
    /// An expired key counts as non-existent.
    async fn del(&self, key: &str) -> MemoryDBResult<bool>;

    /// Atomically move the value of `from` to `to`, replacing `to` if it exists.
    ///
    /// The remaining TTL of `from` moves with the value. Returns `false` when `from`
    /// does not exist or has expired; `to` is then left untouched. Renaming a key to
    /// itself keeps it and returns `true`.
    async fn rename(&self, from: &str, to: &str) -> MemoryDBResult<bool>;

    /// Set the TTL of `key` to `ttl`. Returns whether the key exists.
    /// A zero `ttl` deletes the key.
    async fn expire(&self, key: &str, ttl: &Duration) -> MemoryDBResult<bool>;

    /// Remove all keys from the database.
    ///
    /// # Warning
    /// On shared backends such as Redis this flushes the whole database (`FLUSHDB`).
    async fn flush(&self) -> MemoryDBResult<()>;

    /// Get all keys matching the glob `key` pattern (e.g. `prefix*`).
    ///
    /// Invalid patterns return [`MemoryDBError::InvalidPattern`]. Pattern behavior may
    /// differ between backends, so prefer simple syntax when the backend can vary:
    /// - `*`, `?`, `[abc]` and `[a-z]` match identically for ASCII keys.
    /// - Not portable: negation (`[!abc]` in the default backend, `[^abc]` in Redis;
    ///   each backend treats the other spelling literally), backslash escapes, `**`,
    ///   reversed ranges such as `[z-a]`, unusual class forms such as `[a-]` or `[]]`,
    ///   and non-ASCII characters inside classes.
    /// - Redis matches `?` and `[...]` byte-wise while the default backend matches them
    ///   per character, so use `*` to match non-ASCII keys.
    async fn keys(&self, key: &str) -> MemoryDBResult<Vec<String>>;

    /// Delete multiple keys. Returns the number of keys deleted.
    ///
    /// Expired keys count as non-existent and are not included in the count.
    async fn dels(&self, keys: &[String]) -> MemoryDBResult<u64>;

    /// Get the remaining TTL of `key`, or `None` if the key does not exist or has no TTL.
    async fn ttl(&self, key: &str) -> MemoryDBResult<Option<Duration>>;
}

#[cfg(test)]
/// Conformance tests shared by every backend.
///
/// Each `test_*` entry point runs the same assertions against the default backend and,
/// with the `redis` feature enabled, a matching `test_*_redis` entry point runs them
/// against Redis at `redis://127.0.0.1:6379/0`. `make test` skips the `_redis` cases;
/// use `make test_dev` when a local Redis is available.
mod tests {
    use std::time::Duration;
    use tokio::time::sleep;

    use super::*;
    use crate::memorydb::default::DefaultBackend;

    fn setup_default() -> impl MemoryDB {
        DefaultBackend::new()
    }

    #[cfg(feature = "redis")]
    async fn setup_redis() -> impl MemoryDB {
        crate::memorydb::redis::RedisBackend::new("redis://127.0.0.1:6379/0")
            .await
            .unwrap()
    }

    /// Redis database reserved for destructive tests such as `flush`.
    #[cfg(feature = "redis")]
    async fn setup_redis_isolated() -> impl MemoryDB {
        crate::memorydb::redis::RedisBackend::new("redis://127.0.0.1:6379/15")
            .await
            .unwrap()
    }

    // Run the same conformance body against every available backend.
    macro_rules! backend_test {
        ($test:ident, $test_redis:ident, $body:ident) => {
            #[tokio::test]
            async fn $test() {
                $body("default", setup_default()).await;
            }

            #[cfg(feature = "redis")]
            #[tokio::test]
            async fn $test_redis() {
                $body("redis", setup_redis().await).await;
            }
        };
    }

    // --- Basic operations ---

    backend_test!(test_normal, test_normal_redis, test_normal_fn);

    async fn test_normal_fn(name: &str, r: impl MemoryDB) {
        let key = "_actix_cloud_key1";
        let value1 = "value1";
        let value2 = "value2";

        println!("Backend: {}", name);

        let _ = r.del(key).await;

        assert_eq!(r.get(key).await.unwrap(), None);
        assert_eq!(r.get_del(key).await.unwrap(), None);
        assert_eq!(r.ttl(key).await.unwrap(), None);

        r.set(key, value1).await.unwrap();
        assert_eq!(r.get(key).await.unwrap().unwrap(), value1);
        // A plain set leaves the key without an expiration.
        assert_eq!(r.ttl(key).await.unwrap(), None);
        r.set(key, value2).await.unwrap();
        assert_eq!(r.get(key).await.unwrap().unwrap(), value2);

        assert!(r.del(key).await.unwrap());
        assert!(!r.del(key).await.unwrap());
        assert_eq!(r.get(key).await.unwrap(), None);
        assert_eq!(r.ttl(key).await.unwrap(), None);
    }

    backend_test!(
        test_set_clears_ttl,
        test_set_clears_ttl_redis,
        test_set_clears_ttl_fn
    );

    async fn test_set_clears_ttl_fn(name: &str, r: impl MemoryDB) {
        let key = "_actix_cloud_okey";

        println!("Backend: {}", name);

        let _ = r.del(key).await;

        r.set_ex(key, "v1", &Duration::from_secs(60)).await.unwrap();
        assert!(r.ttl(key).await.unwrap().is_some());

        // A plain set replaces the entry and clears its TTL.
        r.set(key, "v2").await.unwrap();
        assert_eq!(r.get(key).await.unwrap().as_deref(), Some("v2"));
        assert_eq!(r.ttl(key).await.unwrap(), None);

        assert!(r.del(key).await.unwrap());
    }

    backend_test!(
        test_missing_keys,
        test_missing_keys_redis,
        test_missing_keys_fn
    );

    async fn test_missing_keys_fn(name: &str, r: impl MemoryDB) {
        let key = "_actix_cloud_mkey";

        println!("Backend: {}", name);

        let _ = r.del(key).await;

        assert_eq!(r.get(key).await.unwrap(), None);
        assert_eq!(r.get_del(key).await.unwrap(), None);
        assert_eq!(r.get_ex(key, &Duration::from_secs(1)).await.unwrap(), None);
        assert_eq!(r.ttl(key).await.unwrap(), None);
        assert!(!r.del(key).await.unwrap());
        assert!(!r.expire(key, &Duration::from_secs(1)).await.unwrap());
        assert!(!r.expire(key, &Duration::ZERO).await.unwrap());
        assert_eq!(r.dels(&[key.to_owned()]).await.unwrap(), 0);
        assert!(r.keys(key).await.unwrap().is_empty());
    }

    backend_test!(
        test_empty_key_and_value,
        test_empty_key_and_value_redis,
        test_empty_key_and_value_fn
    );

    async fn test_empty_key_and_value_fn(name: &str, r: impl MemoryDB) {
        println!("Backend: {}", name);

        // The empty key is a normal key.
        assert_eq!(r.get_del("").await.unwrap(), None);
        r.set("", "").await.unwrap();
        assert_eq!(r.get("").await.unwrap().as_deref(), Some(""));
        assert_eq!(r.ttl("").await.unwrap(), None);
        // The empty pattern matches only the empty key.
        assert_eq!(r.keys("").await.unwrap(), [String::new()]);
        assert_eq!(r.get_del("").await.unwrap().as_deref(), Some(""));

        let key = "_actix_cloud_ekey";
        let _ = r.del(key).await;
        r.set_ex(key, "", &Duration::from_secs(60)).await.unwrap();
        assert_eq!(r.get(key).await.unwrap().as_deref(), Some(""));
        assert!(r.del(key).await.unwrap());
    }

    backend_test!(
        test_unicode_payload,
        test_unicode_payload_redis,
        test_unicode_payload_fn
    );

    async fn test_unicode_payload_fn(name: &str, r: impl MemoryDB) {
        let key = "_actix_cloud_ükey";
        // Multi-byte characters, a newline and a NUL byte must round-trip exactly.
        let value = "值\n\0é";

        println!("Backend: {}", name);

        let _ = r.del(key).await;

        r.set(key, value).await.unwrap();
        assert_eq!(r.get(key).await.unwrap().as_deref(), Some(value));
        // Match the non-ASCII key by a literal prefix plus `*`: `?` and `[...]` are
        // byte-wise in Redis but character-wise in the default backend.
        assert_eq!(r.keys("_actix_cloud_ü*").await.unwrap(), [key.to_owned()]);
        assert!(r.del(key).await.unwrap());
    }

    backend_test!(test_dels_batch, test_dels_batch_redis, test_dels_batch_fn);

    async fn test_dels_batch_fn(name: &str, r: impl MemoryDB) {
        let key1 = "_actix_cloud_dkey1";
        let key2 = "_actix_cloud_dkey2";
        let missing = "_actix_cloud_dkey3";

        println!("Backend: {}", name);

        let _ = r
            .dels(&[key1.to_owned(), key2.to_owned(), missing.to_owned()])
            .await;

        assert_eq!(r.dels(&[]).await.unwrap(), 0);

        r.set(key1, "1").await.unwrap();
        r.set(key2, "2").await.unwrap();
        // Duplicates count once; missing keys do not count.
        assert_eq!(
            r.dels(&[key1.to_owned(), key1.to_owned(), missing.to_owned()])
                .await
                .unwrap(),
            1
        );
        assert_eq!(r.get(key1).await.unwrap(), None);
        assert_eq!(r.get(key2).await.unwrap().as_deref(), Some("2"));
        assert!(r.del(key2).await.unwrap());
    }

    backend_test!(test_rename, test_rename_redis, test_rename_fn);

    async fn test_rename_fn(name: &str, r: impl MemoryDB) {
        let src = "_actix_cloud_rename_src";
        let dst = "_actix_cloud_rename_dst";

        println!("Backend: {}", name);

        let _ = r.del(src).await;
        let _ = r.del(dst).await;

        // A missing source changes nothing.
        r.set(dst, "keep").await.unwrap();
        assert!(!r.rename(src, dst).await.unwrap());
        assert_eq!(r.get(dst).await.unwrap().as_deref(), Some("keep"));
        assert!(r.del(dst).await.unwrap());

        // The value and its remaining TTL move to the destination.
        r.set_ex(src, "value", &Duration::from_secs(60))
            .await
            .unwrap();
        assert!(r.rename(src, dst).await.unwrap());
        assert_eq!(r.get(src).await.unwrap(), None);
        assert_eq!(r.get(dst).await.unwrap().as_deref(), Some("value"));
        let ttl = r.ttl(dst).await.unwrap().unwrap();
        assert!(ttl > Duration::from_secs(59));
        assert!(ttl <= Duration::from_secs(60));

        // The destination is replaced, including its TTL.
        r.set(src, "new").await.unwrap();
        assert!(r.rename(src, dst).await.unwrap());
        assert_eq!(r.get(src).await.unwrap(), None);
        assert_eq!(r.get(dst).await.unwrap().as_deref(), Some("new"));
        assert_eq!(r.ttl(dst).await.unwrap(), None);

        // Renaming to itself keeps the key.
        assert!(r.rename(dst, dst).await.unwrap());
        assert_eq!(r.get(dst).await.unwrap().as_deref(), Some("new"));

        // An expired source counts as missing and leaves the destination alone.
        r.set_ex(src, "expired", &Duration::from_millis(10))
            .await
            .unwrap();
        sleep(Duration::from_millis(20)).await;
        assert!(!r.rename(src, dst).await.unwrap());
        assert_eq!(r.get(dst).await.unwrap().as_deref(), Some("new"));

        assert!(r.del(dst).await.unwrap());
    }

    // --- Expiration ---

    backend_test!(test_ex, test_ex_redis, test_ex_fn);

    async fn test_ex_fn(name: &str, r: impl MemoryDB) {
        let key = "_actix_cloud_key2";
        let value = "value";

        println!("Backend: {}", name);

        let _ = r.del(key).await;

        r.set(key, value).await.unwrap();
        assert_eq!(r.ttl(key).await.unwrap(), None);
        // `get_ex` adds a TTL to a key that did not have one.
        assert_eq!(
            r.get_ex(key, &Duration::from_secs(2)).await.unwrap(),
            Some(value.to_owned())
        );
        assert!(r.ttl(key).await.unwrap().unwrap().as_millis() > 1500);
        assert_eq!(r.get_del(key).await.unwrap().unwrap(), value);
        assert_eq!(r.get(key).await.unwrap(), None);

        r.set_ex(key, value, &Duration::from_secs(2)).await.unwrap();
        assert_eq!(r.get(key).await.unwrap().unwrap(), value);
        assert!(r.ttl(key).await.unwrap().unwrap().as_millis() > 1500);
        sleep(Duration::from_secs(1)).await;
        assert!(r.ttl(key).await.unwrap().unwrap().as_millis() > 500);
        assert_eq!(
            r.get_ex(key, &Duration::from_secs(2))
                .await
                .unwrap()
                .unwrap(),
            value
        );
        assert!(r.ttl(key).await.unwrap().unwrap().as_millis() > 1500);
        sleep(Duration::from_secs(1)).await;
        assert!(r.ttl(key).await.unwrap().unwrap().as_millis() > 500);
        assert_eq!(r.get(key).await.unwrap().unwrap(), value);
        sleep(Duration::from_secs(2)).await;
        assert_eq!(r.ttl(key).await.unwrap(), None);
        assert_eq!(r.get(key).await.unwrap(), None);

        // `set_ex` overwrites both the value and the TTL.
        r.set_ex(key, "older", &Duration::from_secs(60))
            .await
            .unwrap();
        r.set_ex(key, "newer", &Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(r.get(key).await.unwrap().as_deref(), Some("newer"));
        assert!(r.ttl(key).await.unwrap().unwrap().as_millis() <= 1000);
        assert!(r.del(key).await.unwrap());
    }

    backend_test!(test_expire, test_expire_redis, test_expire_fn);

    async fn test_expire_fn(name: &str, r: impl MemoryDB) {
        let key = "_actix_cloud_key3";
        let value = "value";

        println!("Backend: {}", name);

        let _ = r.del(key).await;

        r.set(key, value).await.unwrap();
        assert_eq!(r.get(key).await.unwrap().unwrap(), value);
        assert!(r.expire(key, &Duration::from_secs(1)).await.unwrap());
        sleep(Duration::from_secs(2)).await;
        assert_eq!(r.get(key).await.unwrap(), None);
        assert!(!r.expire(key, &Duration::from_secs(1)).await.unwrap());

        r.set_ex(key, value, &Duration::from_secs(1)).await.unwrap();
        // Extending the TTL keeps the key alive past its original expiration.
        assert!(r.expire(key, &Duration::from_secs(3)).await.unwrap());
        sleep(Duration::from_secs(2)).await;
        assert_eq!(r.get(key).await.unwrap().unwrap(), value);

        // Shortening the TTL expires the key sooner.
        assert!(r.expire(key, &Duration::from_millis(300)).await.unwrap());
        assert!(r.ttl(key).await.unwrap().unwrap().as_millis() <= 300);
        sleep(Duration::from_millis(600)).await;
        assert_eq!(r.get(key).await.unwrap(), None);

        // A zero TTL deletes the key.
        r.set(key, value).await.unwrap();
        assert!(r.expire(key, &Duration::ZERO).await.unwrap());
        assert_eq!(r.get(key).await.unwrap(), None);
        assert!(!r.expire(key, &Duration::ZERO).await.unwrap());
    }

    backend_test!(test_zero_ttl, test_zero_ttl_redis, test_zero_ttl_fn);

    async fn test_zero_ttl_fn(name: &str, r: impl MemoryDB) {
        let key = "_actix_cloud_zkey";

        println!("Backend: {}", name);

        let _ = r.del(key).await;

        // `set_ex` with a zero TTL deletes the key.
        r.set(key, "old").await.unwrap();
        r.set_ex(key, "new", &Duration::ZERO).await.unwrap();
        assert_eq!(r.get(key).await.unwrap(), None);

        // `get_ex` with a zero TTL returns the value and deletes the key.
        r.set(key, "value").await.unwrap();
        assert_eq!(
            r.get_ex(key, &Duration::ZERO).await.unwrap().as_deref(),
            Some("value")
        );
        assert_eq!(r.get(key).await.unwrap(), None);

        // Zero TTL operations on a missing key do not create it.
        let missing = "_actix_cloud_zkey2";
        let _ = r.del(missing).await;
        r.set_ex(missing, "value", &Duration::ZERO).await.unwrap();
        assert_eq!(r.get(missing).await.unwrap(), None);
        assert_eq!(r.get_ex(missing, &Duration::ZERO).await.unwrap(), None);
        assert!(!r.expire(missing, &Duration::ZERO).await.unwrap());

        // Sub-millisecond TTLs are truncated to zero milliseconds by every
        // TTL-taking operation.
        let sub_ms = Duration::from_nanos(500);
        r.set(key, "value").await.unwrap();
        r.set_ex(key, "value", &sub_ms).await.unwrap();
        assert_eq!(r.get(key).await.unwrap(), None);

        r.set(key, "value").await.unwrap();
        assert_eq!(
            r.get_ex(key, &sub_ms).await.unwrap().as_deref(),
            Some("value")
        );
        assert_eq!(r.get(key).await.unwrap(), None);

        r.set(key, "value").await.unwrap();
        assert!(r.expire(key, &sub_ms).await.unwrap());
        assert_eq!(r.get(key).await.unwrap(), None);
    }

    backend_test!(
        test_expired_access,
        test_expired_access_redis,
        test_expired_access_fn
    );

    async fn test_expired_access_fn(name: &str, r: impl MemoryDB) {
        let key = "_actix_cloud_xkey";
        let ttl = Duration::from_millis(100);
        let pause = Duration::from_millis(300);

        println!("Backend: {}", name);

        let _ = r.del(key).await;

        // Each operation is tested on a freshly expired key, because reading an
        // expired key may reap it in the default backend.
        r.set_ex(key, "value", &ttl).await.unwrap();
        sleep(pause).await;
        assert_eq!(r.get(key).await.unwrap(), None);
        assert_eq!(r.ttl(key).await.unwrap(), None);

        r.set_ex(key, "value", &ttl).await.unwrap();
        sleep(pause).await;
        assert_eq!(r.get_del(key).await.unwrap(), None);

        r.set_ex(key, "value", &ttl).await.unwrap();
        sleep(pause).await;
        assert!(!r.expire(key, &Duration::from_secs(60)).await.unwrap());

        r.set_ex(key, "value", &ttl).await.unwrap();
        sleep(pause).await;
        assert_eq!(r.get_ex(key, &Duration::from_secs(60)).await.unwrap(), None);

        // Writing over an expired key succeeds and clears the TTL.
        r.set_ex(key, "old", &ttl).await.unwrap();
        sleep(pause).await;
        r.set(key, "new").await.unwrap();
        assert_eq!(r.get(key).await.unwrap().as_deref(), Some("new"));
        assert_eq!(r.ttl(key).await.unwrap(), None);
        assert!(r.del(key).await.unwrap());
    }

    backend_test!(
        test_ttl_out_of_range,
        test_ttl_out_of_range_redis,
        test_ttl_out_of_range_fn
    );

    async fn test_ttl_out_of_range_fn(name: &str, r: impl MemoryDB) {
        let key = "_actix_cloud_tkey";
        let other = "_actix_cloud_tkey2";

        println!("Backend: {}", name);

        let _ = r.del(key).await;
        let _ = r.del(other).await;
        r.set(key, "value").await.unwrap();

        let too_large = Duration::from_millis(u64::MAX);
        let overflow = Duration::from_millis(i64::MAX as u64);
        for ttl in [&too_large, &overflow] {
            assert!(matches!(
                r.set_ex(other, "value", ttl).await,
                Err(MemoryDBError::TtlOutOfRange)
            ));
            assert!(matches!(
                r.get_ex(key, ttl).await,
                Err(MemoryDBError::TtlOutOfRange)
            ));
            assert!(matches!(
                r.expire(key, ttl).await,
                Err(MemoryDBError::TtlOutOfRange)
            ));
        }

        // Failed TTL operations leave the database untouched.
        assert_eq!(r.get(key).await.unwrap().as_deref(), Some("value"));
        assert_eq!(r.ttl(key).await.unwrap(), None);
        assert_eq!(r.get(other).await.unwrap(), None);

        // Large TTLs that do not overflow are accepted.
        let hundred_years = Duration::from_secs(100 * 365 * 24 * 60 * 60);
        r.set_ex(key, "value", &hundred_years).await.unwrap();
        assert!(r.ttl(key).await.unwrap().unwrap() > Duration::from_secs(99 * 365 * 24 * 60 * 60));
        assert!(r.del(key).await.unwrap());
    }

    backend_test!(
        test_del_expired,
        test_del_expired_redis,
        test_del_expired_fn
    );

    async fn test_del_expired_fn(name: &str, r: impl MemoryDB) {
        let key = "_actix_cloud_key4";
        let value = "value";

        println!("Backend: {}", name);

        let _ = r.del(key).await;

        // `del` must treat an expired key as non-existent. The key is not read
        // beforehand so the default backend cannot reap it lazily.
        r.set_ex(key, value, &Duration::from_millis(100))
            .await
            .unwrap();
        sleep(Duration::from_millis(300)).await;
        assert!(!r.del(key).await.unwrap());
        assert_eq!(r.get(key).await.unwrap(), None);

        // `dels` removes expired keys but does not count them.
        r.set_ex(key, value, &Duration::from_millis(100))
            .await
            .unwrap();
        sleep(Duration::from_millis(300)).await;
        assert_eq!(r.dels(&[key.to_owned()]).await.unwrap(), 0);

        // A live key is still counted.
        r.set(key, value).await.unwrap();
        assert_eq!(r.dels(&[key.to_owned()]).await.unwrap(), 1);
        assert_eq!(r.get(key).await.unwrap(), None);

        // A mixed batch counts live keys only.
        let live = "_actix_cloud_key4_live";
        let expired = "_actix_cloud_key4_expired";
        let _ = r.dels(&[live.to_owned(), expired.to_owned()]).await;
        r.set(live, value).await.unwrap();
        r.set_ex(expired, value, &Duration::from_millis(100))
            .await
            .unwrap();
        sleep(Duration::from_millis(300)).await;
        assert_eq!(
            r.dels(&[live.to_owned(), expired.to_owned()])
                .await
                .unwrap(),
            1
        );
        assert_eq!(r.get(live).await.unwrap(), None);
    }

    // --- Keys ---

    backend_test!(test_batch, test_batch_redis, test_batch_fn);

    async fn test_batch_fn(name: &str, r: impl MemoryDB) {
        let key1 = "_actix_cloud_bkey1";
        let key2 = "_actix_cloud_bkey2";
        let value = "value";

        println!("Backend: {}", name);

        let _ = r.del(key1).await;
        let _ = r.del(key2).await;

        r.set(key1, value).await.unwrap();
        r.set(key2, value).await.unwrap();
        let mut ret = r.keys("_actix_cl?ud_bkey*").await.unwrap();
        ret.sort();
        assert_eq!(ret, [key1, key2]);

        // Character classes work in both implementations.
        ret = r.keys("_actix_cloud_bkey[12]").await.unwrap();
        ret.sort();
        assert_eq!(ret, [key1, key2]);

        assert_eq!(
            r.dels(&[key1.to_owned(), key2.to_owned()]).await.unwrap(),
            2
        );
        assert!(r.keys("_actix_cl?ud_bkey*").await.unwrap().is_empty());
    }

    backend_test!(
        test_keys_portable_subset,
        test_keys_portable_subset_redis,
        test_keys_portable_subset_fn
    );

    async fn test_keys_portable_subset_fn(name: &str, r: impl MemoryDB) {
        let keys = [
            "_actix_cloud_glob_a1",
            "_actix_cloud_glob_a2",
            "_actix_cloud_glob_b1",
        ];

        println!("Backend: {}", name);

        for key in keys {
            let _ = r.del(key).await;
            r.set(key, "value").await.unwrap();
        }

        // The documented portable subset: `*`, `?`, `[abc]` and `[a-z]` on ASCII
        // keys must behave identically in every backend.
        let mut ret = r.keys("_actix_cloud_glob_*").await.unwrap();
        ret.sort();
        assert_eq!(ret, keys);

        ret = r.keys("_actix_cloud_glob_a?").await.unwrap();
        ret.sort();
        assert_eq!(ret, [keys[0], keys[1]]);

        ret = r.keys("_actix_cloud_glob_[ab]1").await.unwrap();
        ret.sort();
        assert_eq!(ret, [keys[0], keys[2]]);

        ret = r.keys("_actix_cloud_glob_[ab][12]").await.unwrap();
        ret.sort();
        assert_eq!(ret, keys);

        ret = r.keys("_actix_cloud_glob_a[0-9]").await.unwrap();
        ret.sort();
        assert_eq!(ret, [keys[0], keys[1]]);

        for key in keys {
            assert!(r.del(key).await.unwrap());
        }
    }

    backend_test!(
        test_keys_hides_expired,
        test_keys_hides_expired_redis,
        test_keys_hides_expired_fn
    );

    async fn test_keys_hides_expired_fn(name: &str, r: impl MemoryDB) {
        let live = "_actix_cloud_gkey_live";
        let expired = "_actix_cloud_gkey_expired";

        println!("Backend: {}", name);

        let _ = r.dels(&[live.to_owned(), expired.to_owned()]).await;

        r.set(live, "1").await.unwrap();
        r.set_ex(expired, "2", &Duration::from_millis(100))
            .await
            .unwrap();
        sleep(Duration::from_millis(300)).await;

        assert_eq!(
            r.keys("_actix_cloud_gkey_*").await.unwrap(),
            [live.to_owned()]
        );
        assert_eq!(r.get(expired).await.unwrap(), None);
        assert!(r.del(live).await.unwrap());
    }

    backend_test!(
        test_keys_invalid_pattern,
        test_keys_invalid_pattern_redis,
        test_keys_invalid_pattern_fn
    );

    async fn test_keys_invalid_pattern_fn(name: &str, r: impl MemoryDB) {
        println!("Backend: {}", name);

        assert!(matches!(
            r.keys("[").await,
            Err(MemoryDBError::InvalidPattern(_))
        ));
    }

    backend_test!(
        test_keys_pagination,
        test_keys_pagination_redis,
        test_keys_pagination_fn
    );

    async fn test_keys_pagination_fn(name: &str, r: impl MemoryDB) {
        // More keys than the Redis SCAN page size, so the cursor has to be followed
        // across several round trips.
        const COUNT: usize = 2500;
        let prefix = "_actix_cloud_scan_";

        println!("Backend: {}", name);

        let stale = r.keys(&format!("{prefix}*")).await.unwrap();
        if !stale.is_empty() {
            r.dels(&stale).await.unwrap();
        }

        let keys: Vec<String> = (0..COUNT).map(|i| format!("{prefix}{i:04}")).collect();
        for key in &keys {
            r.set(key, "v").await.unwrap();
        }

        let mut found = r.keys(&format!("{prefix}*")).await.unwrap();
        found.sort_unstable();
        let mut expected = keys.clone();
        expected.sort_unstable();
        assert_eq!(found, expected);

        assert_eq!(r.dels(&keys).await.unwrap(), COUNT as u64);
    }

    // --- Flush ---

    #[tokio::test]
    async fn test_flush() {
        test_flush_fn("default", setup_default()).await;
    }

    #[cfg(feature = "redis")]
    #[tokio::test]
    async fn test_flush_redis() {
        test_flush_fn("redis", setup_redis_isolated().await).await;
    }

    async fn test_flush_fn(name: &str, r: impl MemoryDB) {
        let key1 = "_actix_cloud_fkey1";
        let key2 = "_actix_cloud_fkey2";

        println!("Backend: {}", name);

        r.set(key1, "1").await.unwrap();
        r.set(key2, "2").await.unwrap();
        r.flush().await.unwrap();

        assert_eq!(r.get(key1).await.unwrap(), None);
        assert_eq!(r.get(key2).await.unwrap(), None);
        assert!(r.keys("_actix_cloud_fkey*").await.unwrap().is_empty());
    }

    // --- Default backend only ---

    #[tokio::test]
    async fn test_default_capacity() {
        let key1 = "_actix_cloud_ckey1";
        let key2 = "_actix_cloud_ckey2";
        let key3 = "_actix_cloud_ckey3";
        let value = "value";

        let r = DefaultBackend::new().max_entries(Some(2));
        let _ = r.del(key1).await;
        let _ = r.del(key2).await;
        let _ = r.del(key3).await;

        // Fill the capacity with persistent keys: nothing can be evicted.
        assert!(r.set(key1, value).await.is_ok());
        assert!(r.set(key2, value).await.is_ok());
        assert!(r.set(key3, value).await.is_err());
        // Overwriting existing keys does not need free space.
        assert!(r.set(key1, value).await.is_ok());
        assert!(r.set(key2, value).await.is_ok());

        // key2 expires, so inserting key3 can reap it.
        assert!(r.set_ex(key2, value, &Duration::from_secs(1)).await.is_ok());
        sleep(Duration::from_secs(2)).await;
        assert!(r.set(key3, value).await.is_ok());
        assert_eq!(r.get(key1).await.unwrap().unwrap(), value);
        assert_eq!(r.get(key2).await.unwrap(), None);

        // key3 has a TTL, so it becomes the eviction candidate for key2.
        assert!(r.set_ex(key3, value, &Duration::from_secs(3)).await.is_ok());
        sleep(Duration::from_secs(1)).await;
        assert!(r.set(key2, value).await.is_ok());
        assert_eq!(r.get(key1).await.unwrap().unwrap(), value);
        assert_eq!(r.get(key3).await.unwrap(), None);

        // Of the two TTL entries, key2 (shorter TTL) is evicted before key1.
        assert!(r.del(key1).await.unwrap());
        assert!(r.del(key2).await.unwrap());
        assert!(r.set_ex(key1, value, &Duration::from_secs(3)).await.is_ok());
        assert!(r.set_ex(key2, value, &Duration::from_secs(2)).await.is_ok());
        assert!(r.set_ex(key3, value, &Duration::from_secs(2)).await.is_ok());
        assert_eq!(r.get(key1).await.unwrap().unwrap(), value);
        assert_eq!(r.get(key2).await.unwrap(), None);
        assert_eq!(r.get(key3).await.unwrap().unwrap(), value);

        assert!(r.del(key1).await.unwrap());
        assert!(r.del(key3).await.unwrap());
    }
}
