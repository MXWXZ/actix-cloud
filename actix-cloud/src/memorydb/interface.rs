//! The [`MemoryDB`] trait and the conformance tests shared by all backends.
//!
//! Backends store string values under string keys with millisecond-precision TTLs.
//! New backends should be plugged into the generic tests at the bottom of this module.
use std::time::Duration;

use async_trait::async_trait;

use crate::Result;

/// Object-safe async key-value database trait.
///
/// All values are strings and all TTLs are millisecond-precision [`Duration`]s.
/// This is the abstraction behind session storage and CSRF token storage; you can
/// implement it to plug in your own backend (see [`DefaultBackend`](super::default::DefaultBackend)
/// and [`RedisBackend`](super::redis::RedisBackend) for reference implementations).
///
/// The trait is implemented for `Arc<dyn MemoryDB>`, so backends are shared as
/// `Arc<dyn MemoryDB>` throughout the framework.
#[async_trait]
pub trait MemoryDB: Send + Sync {
    /// Set `key` to `value` without expiration.
    async fn set(&self, key: &str, value: &str) -> Result<()>;

    /// Get the value of `key`, or `None` if it does not exist or has expired.
    async fn get(&self, key: &str) -> Result<Option<String>>;

    /// Get the value of `key` and delete it atomically.
    async fn get_del(&self, key: &str) -> Result<Option<String>>;

    /// Get the value of `key` and reset its TTL to `ttl`.
    async fn get_ex(&self, key: &str, ttl: &Duration) -> Result<Option<String>>;

    /// Set `key` to `value` with an expiration of `ttl`.
    async fn set_ex(&self, key: &str, value: &str, ttl: &Duration) -> Result<()>;

    /// Delete `key`. Returns whether the key existed.
    ///
    /// An expired key counts as non-existent.
    async fn del(&self, key: &str) -> Result<bool>;

    /// Set the TTL of `key` to `ttl`. Returns whether the key exists.
    /// A zero `ttl` deletes the key.
    async fn expire(&self, key: &str, ttl: &Duration) -> Result<bool>;

    /// Remove all keys from the database.
    ///
    /// # Warning
    /// On shared backends such as Redis this flushes the whole database (`FLUSHDB`).
    async fn flush(&self) -> Result<()>;

    /// Get all keys matching the glob `key` pattern (e.g. `prefix*`).
    async fn keys(&self, key: &str) -> Result<Vec<String>>;

    /// Delete multiple keys. Returns the number of keys deleted.
    ///
    /// Expired keys count as non-existent and are not included in the count.
    async fn dels(&self, keys: &[String]) -> Result<u64>;

    /// Get the remaining TTL of `key`, or `None` if the key does not exist or has no TTL.
    async fn ttl(&self, key: &str) -> Result<Option<Duration>>;
}

#[cfg(test)]
/// Conformance tests shared by every backend.
///
/// Each `test_*` entry point runs the same assertions against the default backend and,
/// with the `redis` feature enabled, against Redis at `redis://127.0.0.1:6379/0`.
/// Because of that dependency these tests are excluded by `make test`; use
/// `make test_dev` when a local Redis is available.
mod tests {
    use std::time::Duration;
    use tokio::time::sleep;

    use super::*;
    use crate::memorydb::default::DefaultBackend;

    #[cfg(feature = "redis")]
    async fn setup_redis() -> impl MemoryDB {
        crate::memorydb::redis::RedisBackend::new("redis://127.0.0.1:6379/0")
            .await
            .unwrap()
    }

    fn setup_default() -> impl MemoryDB {
        DefaultBackend::new(None)
    }

    #[tokio::test]
    async fn test_normal() {
        test_normal_fn("default", setup_default()).await;
        #[cfg(feature = "redis")]
        test_normal_fn("redis", setup_redis().await).await;
    }

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

    #[tokio::test]
    async fn test_ex() {
        test_ex_fn("default", setup_default()).await;
        #[cfg(feature = "redis")]
        test_ex_fn("redis", setup_redis().await).await;
    }

    async fn test_ex_fn(name: &str, r: impl MemoryDB) {
        let key = "_actix_cloud_key2";
        let value = "value";

        println!("Backend: {}", name);

        let _ = r.del(key).await;

        r.set(key, value).await.unwrap();
        assert_eq!(r.ttl(key).await.unwrap(), None);
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
    }

    #[tokio::test]
    async fn test_expire() {
        test_expire_fn("default", setup_default()).await;
        #[cfg(feature = "redis")]
        test_expire_fn("redis", setup_redis().await).await;
    }

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
        // A zero TTL deletes the key.
        assert!(r.expire(key, &Duration::default()).await.unwrap());
        assert_eq!(r.get(key).await.unwrap(), None);
        assert!(!r.expire(key, &Duration::default()).await.unwrap());
    }

    #[tokio::test]
    async fn test_del_expired() {
        test_del_expired_fn("default", setup_default()).await;
        #[cfg(feature = "redis")]
        test_del_expired_fn("redis", setup_redis().await).await;
    }

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
    }

    #[tokio::test]
    async fn test_batch() {
        test_batch_fn("default", setup_default()).await;
        #[cfg(feature = "redis")]
        test_batch_fn("redis", setup_redis().await).await;
    }

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
        assert_eq!(
            r.dels(&[key1.to_owned(), key2.to_owned()]).await.unwrap(),
            2
        );
        assert!(r.keys("_actix_cl?ud_bkey*").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_default_capacity() {
        let key1 = "_actix_cloud_ckey1";
        let key2 = "_actix_cloud_ckey2";
        let key3 = "_actix_cloud_ckey3";
        let value = "value";

        let r = DefaultBackend::new(Some(2));
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
