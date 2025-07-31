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

struct Data(String, Option<i64>);

impl Data {
    fn now() -> i64 {
        Utc::now().timestamp()
    }

    fn parse_ttl(ttl: Option<i64>) -> Option<i64> {
        ttl.map(|x| Self::now().saturating_add(x))
    }

    fn new<S>(value: S, ttl: Option<i64>) -> Self
    where
        S: Into<String>,
    {
        Self(value.into(), Self::parse_ttl(ttl))
    }

    fn set_ttl(&mut self, ttl: Option<i64>) {
        self.1 = Self::parse_ttl(ttl);
    }

    fn get_ttl(&self) -> Option<i64> {
        self.1.map(|x| x.saturating_sub(Self::now()))
    }

    fn valid(&self) -> bool {
        if let Some(x) = self.1 {
            x > Self::now()
        } else {
            true
        }
    }
}

#[derive(Clone)]
pub struct DefaultBackend {
    data: Arc<RwLock<HashMap<String, Data>>>,
    capacity: Option<usize>,
}

impl DefaultBackend {
    pub fn new(capacity: Option<usize>) -> Self {
        Self {
            data: Default::default(),
            capacity,
        }
    }

    /// Evict `num` keys from memory. Return evicted number.
    ///
    /// - Evict any expired keys (`x`).
    /// - If `x < num`, evict at most `num-x` keys sorted by TTL.
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
        // full
        if let Some(x) = self.capacity {
            if x == wlock.len()
                && self.gc(&mut wlock, max(x / 10, 1)) == 0
                && wlock.get(key).is_none()
            {
                bail!("Capacity is full");
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
                v.set_ttl(Some(ttl.as_secs().try_into()?));
                Ok(Some(v.0.to_owned()))
            } else {
                wlock.remove(key);
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    async fn set_ex(&self, key: &str, value: &str, ttl: &Duration) -> Result<()> {
        let mut wlock = self.data.write();
        // full
        if let Some(x) = self.capacity {
            if x == wlock.len()
                && self.gc(&mut wlock, max(x / 10, 1)) == 0
                && wlock.get(key).is_none()
            {
                bail!("Capacity is full");
            }
        }
        wlock.insert(
            key.to_owned(),
            Data::new(value, Some(ttl.as_secs().try_into()?)),
        );
        Ok(())
    }

    async fn del(&self, key: &str) -> Result<bool> {
        Ok(self.data.write().remove(key).is_some())
    }

    async fn expire(&self, key: &str, ttl: i64) -> Result<bool> {
        if ttl <= 0 {
            self.del(key).await
        } else {
            let mut wlock = self.data.write();
            if let Some(v) = wlock.get_mut(key) {
                if v.valid() {
                    v.set_ttl(Some(ttl));
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
            if wlock.remove(i).is_some() {
                sum += 1;
            }
        }
        Ok(sum)
    }

    async fn ttl(&self, key: &str) -> Result<Option<i64>> {
        let rlock = self.data.read();
        if let Some(v) = rlock.get(key) {
            if v.valid() {
                Ok(v.get_ttl())
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
