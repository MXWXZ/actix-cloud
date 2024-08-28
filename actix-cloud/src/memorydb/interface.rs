use std::time::Duration;

use async_trait::async_trait;

use crate::Result;

#[async_trait]
pub trait MemoryDB: Send + Sync {
    async fn set(&self, key: &str, value: &str) -> Result<()>;
    async fn get(&self, key: &str) -> Result<Option<String>>;
    async fn get_del(&self, key: &str) -> Result<Option<String>>;
    async fn get_ex(&self, key: &str, ttl: &Duration) -> Result<Option<String>>;
    async fn set_ex(&self, key: &str, value: &str, ttl: &Duration) -> Result<()>;
    async fn del(&self, key: &str) -> Result<bool>;
    async fn expire(&self, key: &str, ttl: i64) -> Result<bool>;
    async fn flush(&self) -> Result<()>;
    async fn keys(&self, key: &str) -> Result<Vec<String>>;
    async fn dels(&self, keys: &[String]) -> Result<u64>;
}

#[cfg(test)]
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
        DefaultBackend::new()
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

        r.set(key, value1).await.unwrap();
        assert_eq!(r.get(key).await.unwrap().unwrap(), value1);
        r.set(key, value2).await.unwrap();
        assert_eq!(r.get(key).await.unwrap().unwrap(), value2);

        assert_eq!(r.del(key).await.unwrap(), true);
        assert_eq!(r.del(key).await.unwrap(), false);
        assert_eq!(r.get(key).await.unwrap(), None);
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
        let ret = r.keys("_actix_cl?ud_bkey*").await.unwrap();
        assert_eq!(ret.len(), 2);
        assert_eq!(
            (ret[0] == key1 && ret[1] == key2) || (ret[1] == key1 && ret[0] == key2),
            true
        );
        assert_eq!(
            r.dels(&vec![key1.to_owned(), key2.to_owned()])
                .await
                .unwrap(),
            2
        );
        assert_eq!(r.keys("_actix_cl?ud_bkey*").await.unwrap().len(), 0);
    }
}
