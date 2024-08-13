use std::{collections::HashMap, sync::Arc, time::Duration, u64};

use async_trait::async_trait;
use chrono::Utc;
use parking_lot::RwLock;

use super::interface::MemoryDB;
use crate::{error::Error, Result};

struct Data(String, Option<u64>);

impl Data {
    fn now() -> Result<u64> {
        Utc::now()
            .timestamp()
            .try_into()
            .map_err(|_| Error::Timestamp("non-positive timestamp"))
    }

    fn parse_ttl(ttl: Option<u64>) -> Result<Option<u64>> {
        if let Some(x) = ttl {
            Ok(Some(
                Self::now()?
                    .checked_add(x)
                    .ok_or(Error::Timestamp("timestamp overflow"))?,
            ))
        } else {
            Ok(None)
        }
    }

    fn new<S>(value: S, ttl: Option<u64>) -> Result<Self>
    where
        S: Into<String> + Send,
    {
        Ok(Self(value.into(), Self::parse_ttl(ttl)?))
    }

    fn set_ttl(&mut self, ttl: Option<u64>) -> Result<()> {
        self.1 = Self::parse_ttl(ttl)?;
        Ok(())
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
    pub async fn new() -> Result<impl MemoryDB> {
        Ok(Self {
            data: Arc::new(RwLock::new(HashMap::new())),
        })
    }
}

#[async_trait]
impl MemoryDB for DefaultBackend {
    async fn set<S>(&self, key: S, value: S) -> Result<()>
    where
        S: Into<String> + Send,
    {
        self.data
            .write()
            .insert(key.into(), Data::new(value, None)?);
        Ok(())
    }

    async fn get<S>(&self, key: S) -> Result<Option<String>>
    where
        S: AsRef<str> + Send,
    {
        let rlock = self.data.read();
        if let Some(v) = rlock.get(key.as_ref()) {
            if v.valid()? {
                Ok(Some(v.0.to_owned()))
            } else {
                drop(rlock);
                self.data.write().remove(key.as_ref());
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    async fn get_del<S>(&self, key: S) -> Result<Option<String>>
    where
        S: AsRef<str> + Send,
    {
        let v = self.data.write().remove(key.as_ref());
        if let Some(v) = v {
            if v.valid()? {
                return Ok(Some(v.0));
            }
        }
        Ok(None)
    }

    async fn get_ex<S>(&self, key: S, ttl: &Duration) -> Result<Option<String>>
    where
        S: AsRef<str> + Send,
    {
        let mut wlock = self.data.write();
        if let Some(v) = wlock.get_mut(key.as_ref()) {
            if v.valid()? {
                v.set_ttl(Some(ttl.as_secs()))?;
                Ok(Some(v.0.to_owned()))
            } else {
                wlock.remove(key.as_ref());
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    async fn set_ex<S>(&self, key: S, value: S, ttl: &Duration) -> Result<()>
    where
        S: Into<String> + Send,
    {
        self.data
            .write()
            .insert(key.into(), Data::new(value, Some(ttl.as_secs()))?);
        Ok(())
    }

    async fn del<S>(&self, key: S) -> Result<bool>
    where
        S: AsRef<str> + Send,
    {
        Ok(self.data.write().remove(key.as_ref()).is_some())
    }

    async fn expire<S>(&self, key: S, ttl: i64) -> Result<bool>
    where
        S: AsRef<str> + Send,
    {
        if ttl <= 0 {
            self.del(key).await
        } else {
            let mut wlock = self.data.write();
            if let Some(v) = wlock.get_mut(key.as_ref()) {
                if v.valid()? {
                    v.set_ttl(Some(ttl as u64))?;
                    Ok(true)
                } else {
                    wlock.remove(key.as_ref());
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
}
