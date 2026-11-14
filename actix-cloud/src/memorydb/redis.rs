//! Redis-backed [`MemoryDB`] backend.
//!
//! Values are stored as Redis strings and TTLs use Redis' millisecond expiration
//! commands, so the semantics match the other backends. The connection is managed by
//! a `redis::aio::ConnectionManager`, which reconnects automatically.
use std::time::Duration;

use async_trait::async_trait;
use redis::{aio::ConnectionManager, AsyncCommands, ErrorKind, Expiry, ServerErrorKind};

use super::interface::{pattern, ttl_millis, MemoryDB, MemoryDBResult};

/// Whether `err` is Redis' response to `RENAME` when the source key is missing.
///
/// Redis reports this as a generic `ERR` server error with the detail `no such key`,
/// so the error kind alone is not enough.
fn is_missing_key(err: &redis::RedisError) -> bool {
    matches!(
        err.kind(),
        ErrorKind::Server(ServerErrorKind::ResponseError)
    ) && err.detail() == Some("no such key")
}

/// Page size hint for [`SCAN`](https://redis.io/commands/scan): large enough to keep
/// round trips low without blocking the server like `KEYS` does.
const SCAN_COUNT: usize = 1000;

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
        let ms = ttl_millis(ttl)?;
        if ms == 0 {
            // GETEX rejects a zero expiration; deleting the key after reading it is
            // what a zero TTL means.
            return self.get_del(key).await;
        }
        // GETEX reads the value and replaces its expiration in one round trip.
        Ok(self
            .client
            .clone()
            .get_ex(key, Expiry::PX(ms as u64))
            .await?)
    }

    async fn set_ex(&self, key: &str, value: &str, ttl: &Duration) -> MemoryDBResult<()> {
        let ms = ttl_millis(ttl)?;
        if ms == 0 {
            // PSETEX rejects a zero expiration; a zero TTL deletes the key.
            self.del(key).await?;
            return Ok(());
        }
        // PSETEX sets the value with a millisecond expiration atomically.
        let _: () = self.client.clone().pset_ex(key, value, ms as u64).await?;
        Ok(())
    }

    async fn del(&self, key: &str) -> MemoryDBResult<bool> {
        Ok(self.client.clone().del(key).await?)
    }

    async fn rename(&self, from: &str, to: &str) -> MemoryDBResult<bool> {
        // `RENAME` is atomic and preserves the remaining TTL. It reports a missing
        // source as an error, which means nothing was moved.
        match self.client.clone().rename(from, to).await {
            Ok(()) => Ok(true),
            Err(err) if is_missing_key(&err) => Ok(false),
            Err(err) => Err(err.into()),
        }
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
        // Validate with the shared parser so invalid patterns fail like the default
        // backend instead of being interpreted leniently by Redis.
        pattern(key)?;
        // SCAN instead of KEYS: it does not block the server for the whole keyspace.
        let mut conn = self.client.clone();
        let mut iter: redis::AsyncIter<String> = conn
            .scan_options(
                redis::ScanOptions::default()
                    .with_pattern(key)
                    .with_count(SCAN_COUNT),
            )
            .await?;
        let mut keys = Vec::new();
        while let Some(item) = iter.next_item().await {
            keys.push(item?);
        }
        Ok(keys)
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
