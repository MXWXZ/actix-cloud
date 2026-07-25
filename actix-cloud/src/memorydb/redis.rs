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
    pub async fn new(dsn: &str) -> Result<Self> {
        let client = ConnectionManager::new(redis::Client::open(dsn)?).await?;
        Ok(Self { client })
    }
}

#[async_trait]
impl MemoryDB for RedisBackend {
    async fn set(&self, key: &str, value: &str) -> Result<()> {
        self.client
            .clone()
            .set(key, value)
            .await
            .map_err(Into::into)
    }

    async fn get(&self, key: &str) -> Result<Option<String>> {
        self.client.clone().get(key).await.map_err(Into::into)
    }

    async fn get_del(&self, key: &str) -> Result<Option<String>> {
        self.client.clone().get_del(key).await.map_err(Into::into)
    }

    async fn get_ex(&self, key: &str, ttl: &Duration) -> Result<Option<String>> {
        self.client
            .clone()
            .get_ex(key, Expiry::PX(ttl.as_millis().try_into()?))
            .await
            .map_err(Into::into)
    }

    async fn set_ex(&self, key: &str, value: &str, ttl: &Duration) -> Result<()> {
        self.client
            .clone()
            .pset_ex(key, value, ttl.as_millis().try_into()?)
            .await
            .map_err(Into::into)
    }

    async fn del(&self, key: &str) -> Result<bool> {
        self.client.clone().del(key).await.map_err(Into::into)
    }

    async fn expire(&self, key: &str, ttl: &Duration) -> Result<bool> {
        self.client
            .clone()
            .pexpire(key, ttl.as_millis().try_into()?)
            .await
            .map_err(Into::into)
    }

    async fn flush(&self) -> Result<()> {
        redis::cmd("FLUSHDB")
            .query_async(&mut self.client.clone())
            .await
            .map_err(Into::into)
    }

    async fn keys(&self, key: &str) -> Result<Vec<String>> {
        self.client.clone().keys(key).await.map_err(Into::into)
    }

    async fn dels(&self, keys: &[String]) -> Result<u64> {
        let mut p = redis::pipe();
        let mut p = p.atomic();
        for i in keys {
            p = p.del(i);
        }
        let res: Vec<u64> = p.query_async(&mut self.client.clone()).await?;
        Ok(res.into_iter().sum())
    }

    async fn ttl(&self, key: &str) -> Result<Option<Duration>> {
        let ret: i64 = self.client.clone().pttl(key).await?;
        if ret < 0 {
            Ok(None)
        } else {
            Ok(Some(Duration::from_millis(ret.try_into()?)))
        }
    }
}
