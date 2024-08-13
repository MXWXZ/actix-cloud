use std::time::Duration;

use async_trait::async_trait;
use redis::{aio::ConnectionManager, AsyncCommands, Expiry};

use super::interface::MemoryDB;
use crate::Result;

#[derive(Clone)]
pub struct RedisBackend {
    client: ConnectionManager,
}

impl RedisBackend {
    pub async fn new(dsn: &str) -> Result<impl MemoryDB> {
        let client = ConnectionManager::new(redis::Client::open(dsn)?).await?;
        Ok(Self { client })
    }
}

#[async_trait]
impl MemoryDB for RedisBackend {
    async fn set<S>(&self, key: S, value: S) -> Result<()>
    where
        S: Into<String> + Send,
    {
        self.client
            .clone()
            .set(key.into(), value.into())
            .await
            .map_err(Into::into)
    }

    async fn get<S>(&self, key: S) -> Result<Option<String>>
    where
        S: AsRef<str> + Send,
    {
        self.client
            .clone()
            .get(key.as_ref())
            .await
            .map_err(Into::into)
    }

    async fn get_del<S>(&self, key: S) -> Result<Option<String>>
    where
        S: AsRef<str> + Send,
    {
        self.client
            .clone()
            .get_del(key.as_ref())
            .await
            .map_err(Into::into)
    }

    async fn get_ex<S>(&self, key: S, ttl: &Duration) -> Result<Option<String>>
    where
        S: AsRef<str> + Send,
    {
        self.client
            .clone()
            .get_ex(key.as_ref(), Expiry::EX(ttl.as_secs()))
            .await
            .map_err(Into::into)
    }

    async fn set_ex<S>(&self, key: S, value: S, ttl: &Duration) -> Result<()>
    where
        S: Into<String> + Send,
    {
        self.client
            .clone()
            .set_ex(key.into(), value.into(), ttl.as_secs())
            .await
            .map_err(Into::into)
    }

    async fn del<S>(&self, key: S) -> Result<bool>
    where
        S: AsRef<str> + Send,
    {
        self.client
            .clone()
            .del(key.as_ref())
            .await
            .map_err(Into::into)
    }

    async fn expire<S>(&self, key: S, ttl: i64) -> Result<bool>
    where
        S: AsRef<str> + Send,
    {
        self.client
            .clone()
            .expire(key.as_ref(), ttl)
            .await
            .map_err(Into::into)
    }

    async fn flush(&self) -> Result<()> {
        redis::cmd("FLUSHDB")
            .query_async(&mut self.client.clone())
            .await
            .map_err(Into::into)
    }
}
