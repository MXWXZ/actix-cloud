//! Redis-backed [`MemoryDB`] backend.
//!
//! Values are stored as Redis strings and TTLs use Redis' millisecond expiration
//! commands, so the semantics match the other backends. The connection is managed by
//! a `redis::aio::ConnectionManager`, which reconnects automatically.
use std::time::Duration;

use async_trait::async_trait;
use redis::{aio::ConnectionManager, AsyncCommands, Expiry};

use super::interface::{MemoryDB, MemoryDBError, MemoryDBResult};

/// Redis memory database backend.
///
/// Uses a `redis::aio::ConnectionManager`, which reconnects to Redis automatically,
/// so the backend can be cloned and shared freely.
#[derive(Clone)]
pub struct RedisBackend {
    client: ConnectionManager,
}

impl RedisBackend {
    /// Connect to Redis with a DSN like `redis://user:pass@127.0.0.1:6379/0`.
    pub async fn new(dsn: &str) -> MemoryDBResult<Self> {
        let client = ConnectionManager::new(redis::Client::open(dsn)?).await?;
        Ok(Self { client })
    }
}

/// Convert a TTL to Redis milliseconds.
///
/// The result is non-negative, so casting it to `u64` for commands that take an
/// unsigned duration is lossless.
fn ttl_millis(ttl: &Duration) -> MemoryDBResult<i64> {
    i64::try_from(ttl.as_millis()).map_err(|_| MemoryDBError::TtlOutOfRange)
}

#[async_trait]
impl MemoryDB for RedisBackend {
    async fn set(&self, key: &str, value: &str) -> MemoryDBResult<()> {
        // SET also clears any TTL previously attached to the key.
        let _: () = self.client.clone().set(key, value).await?;
        Ok(())
    }

    async fn get(&self, key: &str) -> MemoryDBResult<Option<String>> {
        Ok(self.client.clone().get(key).await?)
    }

    async fn get_del(&self, key: &str) -> MemoryDBResult<Option<String>> {
        // GETDEL removes the key atomically on the server.
        Ok(self.client.clone().get_del(key).await?)
    }

    async fn get_ex(&self, key: &str, ttl: &Duration) -> MemoryDBResult<Option<String>> {
        // GETEX reads the value and replaces its expiration in one round trip.
        Ok(self
            .client
            .clone()
            .get_ex(key, Expiry::PX(ttl_millis(ttl)? as u64))
            .await?)
    }

    async fn set_ex(&self, key: &str, value: &str, ttl: &Duration) -> MemoryDBResult<()> {
        // PSETEX sets the value with a millisecond expiration atomically.
        let _: () = self
            .client
            .clone()
            .pset_ex(key, value, ttl_millis(ttl)? as u64)
            .await?;
        Ok(())
    }

    async fn del(&self, key: &str) -> MemoryDBResult<bool> {
        Ok(self.client.clone().del(key).await?)
    }

    async fn expire(&self, key: &str, ttl: &Duration) -> MemoryDBResult<bool> {
        // PEXPIRE reports false when the key does not exist; a zero TTL deletes it.
        Ok(self.client.clone().pexpire(key, ttl_millis(ttl)?).await?)
    }

    async fn flush(&self) -> MemoryDBResult<()> {
        // FLUSHDB clears only the selected database (see `MemoryDB::flush`).
        redis::cmd("FLUSHDB")
            .query_async::<()>(&mut self.client.clone())
            .await?;
        Ok(())
    }

    async fn keys(&self, key: &str) -> MemoryDBResult<Vec<String>> {
        Ok(self.client.clone().keys(key).await?)
    }

    async fn dels(&self, keys: &[String]) -> MemoryDBResult<u64> {
        // Run the DELs in a MULTI/EXEC transaction and sum their results.
        let mut p = redis::pipe();
        let mut p = p.atomic();
        for i in keys {
            p = p.del(i);
        }
        let res: Vec<u64> = p.query_async(&mut self.client.clone()).await?;
        Ok(res.into_iter().sum())
    }

    async fn ttl(&self, key: &str) -> MemoryDBResult<Option<Duration>> {
        // PTTL returns -2 for a missing key and -1 for a key without TTL; the trait
        // exposes both cases as `None`.
        let ret: i64 = self.client.clone().pttl(key).await?;
        if ret < 0 {
            Ok(None)
        } else {
            Ok(Some(Duration::from_millis(ret.unsigned_abs())))
        }
    }
}
