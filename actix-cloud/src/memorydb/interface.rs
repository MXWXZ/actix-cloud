use std::time::Duration;

use async_trait::async_trait;

use crate::Result;

#[async_trait]
pub trait MemoryDB: Clone {
    async fn set<S>(&self, key: S, value: S) -> Result<()>
    where
        S: Into<String> + Send;

    async fn get<S>(&self, key: S) -> Result<Option<String>>
    where
        S: AsRef<str> + Send;

    async fn get_del<S>(&self, key: S) -> Result<Option<String>>
    where
        S: AsRef<str> + Send;

    async fn get_ex<S>(&self, key: S, ttl: &Duration) -> Result<Option<String>>
    where
        S: AsRef<str> + Send;

    async fn set_ex<S>(&self, key: S, value: S, ttl: &Duration) -> Result<()>
    where
        S: Into<String> + Send;

    async fn del<S>(&self, key: S) -> Result<bool>
    where
        S: AsRef<str> + Send;

    async fn expire<S>(&self, key: S, ttl: i64) -> Result<bool>
    where
        S: AsRef<str> + Send;

    async fn flush(&self) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::sleep;

    use crate::memorydb::default::DefaultBackend;
    #[cfg(feature = "redis")]
    use crate::memorydb::redis::RedisBackend;

    use super::*;

    #[cfg(feature = "redis")]
    async fn setup_redis() -> impl MemoryDB {
        RedisBackend::new("redis://127.0.0.1:6379/0").await.unwrap()
    }

    async fn setup_default() -> impl MemoryDB {
        DefaultBackend::new().await.unwrap()
    }

    #[tokio::test]
    async fn test_normal() {
        test_normal_fn("default", setup_default().await).await;
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

        r.set(key, value1).await.unwrap();
        assert_eq!(r.get(key).await.unwrap().unwrap(), value1);
        r.set(key, value2).await.unwrap();
        assert_eq!(r.get(key).await.unwrap().unwrap(), value2);

        assert_eq!(r.del(key).await.unwrap(), true);
        assert_eq!(r.del(key).await.unwrap(), false);
        assert_eq!(r.get(key).await.unwrap(), None);

        r.set("_actix_cloud_key1-1-1", value1).await.unwrap();
        r.set("_actix_cloud_key1-1-2", value1).await.unwrap();
        r.set("_actix_cloud_key1-2-1", value1).await.unwrap();
    }

    #[tokio::test]
    async fn test_ex() {
        test_ex_fn("default", setup_default().await).await;
        #[cfg(feature = "redis")]
        test_ex_fn("redis", setup_redis().await).await;
    }

    async fn test_ex_fn(name: &str, r: impl MemoryDB) {
        let key = "_actix_cloud_key2";
        let value = "value";

        println!("Backend: {}", name);

        let _ = r.del(key).await;

        r.set(key, value).await.unwrap();
        assert_eq!(r.get_del(key).await.unwrap().unwrap(), value);
        assert_eq!(r.get(key).await.unwrap(), None);

        r.set_ex(key, value, &Duration::from_secs(2)).await.unwrap();
        assert_eq!(r.get(key).await.unwrap().unwrap(), value);
        sleep(Duration::from_secs(1)).await;
        assert_eq!(
            r.get_ex(key, &Duration::from_secs(2))
                .await
                .unwrap()
                .unwrap(),
            value
        );
        sleep(Duration::from_secs(1)).await;
        assert_eq!(r.get(key).await.unwrap().unwrap(), value);
        sleep(Duration::from_secs(2)).await;
        assert_eq!(r.get(key).await.unwrap(), None);
    }

    #[tokio::test]
    async fn test_expire() {
        test_expire_fn("default", setup_default().await).await;
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
        assert_eq!(r.expire(key, 1).await.unwrap(), true);
        sleep(Duration::from_secs(2)).await;
        assert_eq!(r.get(key).await.unwrap(), None);
        assert_eq!(r.expire(key, 1).await.unwrap(), false);

        r.set_ex(key, value, &Duration::from_secs(1)).await.unwrap();
        assert_eq!(r.expire(key, 3).await.unwrap(), true);
        sleep(Duration::from_secs(2)).await;
        assert_eq!(r.get(key).await.unwrap().unwrap(), value);
        assert_eq!(r.expire(key, -1).await.unwrap(), true);
        assert_eq!(r.get(key).await.unwrap(), None);
        assert_eq!(r.expire(key, 0).await.unwrap(), false);
    }
}
