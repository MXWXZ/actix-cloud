use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::anyhow;
use async_trait::async_trait;
use chrono::Utc;
use glob::Pattern;
use parking_lot::RwLock;

use super::interface::MemoryDB;
use crate::Result;

struct Data(String, Option<u64>);

impl Data {
    fn now() -> Result<u64> {
        Utc::now().timestamp().try_into().map_err(Into::into)
    }

    fn parse_ttl(ttl: Option<u64>) -> Result<Option<u64>> {
        if let Some(x) = ttl {
            Ok(Some(
                Self::now()?
                    .checked_add(x)
                    .ok_or(anyhow!("timestamp overflow"))?,
            ))
        } else {
            Ok(None)
        }
    }

    fn new<S>(value: S, ttl: Option<u64>) -> Result<Self>
    where
        S: Into<String>,
    {
        Ok(Self(value.into(), Self::parse_ttl(ttl)?))
    }

    fn set_ttl(&mut self, ttl: Option<u64>) -> Result<()> {
        self.1 = Self::parse_ttl(ttl)?;
        Ok(())
    }

    fn get_ttl(&self) -> Result<Option<u64>> {
        if let Some(x) = self.1 {
            Ok(Some(
                x.checked_sub(Self::now()?)
                    .ok_or(anyhow!("timestamp overflow"))?,
            ))
        } else {
            Ok(None)
        }
    }

    fn valid(&self) -> Result<bool> {
        if let Some(x) = self.1 {
            if x > Self::now()? {
                Ok(true)
            } else {
                Ok(false)
            }
        } else {
            Ok(true)
        }
    }
}

#[derive(Clone)]
pub struct DefaultBackend {
    data: Arc<RwLock<HashMap<String, Data>>>,
}

impl DefaultBackend {
    pub fn new() -> Self {
        Self {
            data: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for DefaultBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MemoryDB for DefaultBackend {
    async fn set(&self, key: &str, value: &str) -> Result<()> {
        self.data
            .write()
            .insert(key.to_owned(), Data::new(value, None)?);
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<String>> {
        let rlock = self.data.read();
        if let Some(v) = rlock.get(key) {
            if v.valid()? {
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
            if v.valid()? {
                return Ok(Some(v.0));
            }
        }
        Ok(None)
    }

    async fn get_ex(&self, key: &str, ttl: &Duration) -> Result<Option<String>> {
        let mut wlock = self.data.write();
        if let Some(v) = wlock.get_mut(key) {
            if v.valid()? {
                v.set_ttl(Some(ttl.as_secs()))?;
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
        self.data
            .write()
            .insert(key.to_owned(), Data::new(value, Some(ttl.as_secs()))?);
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
                if v.valid()? {
                    v.set_ttl(Some(ttl as u64))?;
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
            if v.valid()? && p.matches(k) {
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

    async fn ttl(&self, key: &str) -> Result<Option<u64>> {
        let rlock = self.data.read();
        if let Some(v) = rlock.get(key) {
            if v.valid()? {
                Ok(v.get_ttl()?)
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
