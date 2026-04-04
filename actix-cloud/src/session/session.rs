use std::{
    cell::{Ref, RefCell},
    mem,
    rc::Rc,
};

use actix_utils::future::{ready, Ready};
use actix_web::{
    dev::{Extensions, Payload, ServiceRequest, ServiceResponse},
    error::Error,
    FromRequest, HttpMessage, HttpRequest,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{Map, Value};

use crate::Result;

/// The primary interface to access and modify session state.
///
/// [`Session`] is an [extractor](#impl-FromRequest)—you can specify it as an input type for your
/// request handlers and it will be automatically extracted from the incoming request.
///
/// You can also retrieve a [`Session`] object from an `HttpRequest` or a `ServiceRequest` using
/// [`SessionExt`].
///
/// [`SessionExt`]: super::SessionExt
#[derive(Clone)]
pub struct Session(Rc<RefCell<SessionInner>>);

/// Status of a [`Session`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SessionStatus {
    /// Session state has been updated - the changes will have to be persisted to the backend.
    Changed,

    /// The session has been flagged for deletion - the session cookie will be removed from
    /// the client and the session state will be deleted from the session store.
    ///
    /// Most operations on the session after it has been marked for deletion will have no effect.
    Purged,

    /// The session has been flagged for renewal.
    ///
    /// The session key will be regenerated and the time-to-live of the session state will be
    /// extended.
    Renewed,

    /// The session state has not been modified since its creation/retrieval.
    #[default]
    Unchanged,
}

#[derive(Default)]
struct SessionInner {
    state: Map<String, Value>,
    status: SessionStatus,
}

impl Session {
    /// Get a `value` from the session.
    ///
    /// It returns an error if it fails to deserialize as `T` the JSON value associated with `key`.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        if let Some(value) = self.0.borrow().state.get(key) {
            Ok(Some(serde_json::from_value::<T>(value.clone())?))
        } else {
            Ok(None)
        }
    }

    /// Returns `true` if the session contains a value for the specified `key`.
    pub fn contains_key(&self, key: &str) -> bool {
        self.0.borrow().state.contains_key(key)
    }

    /// Get all raw key-value data from the session.
    ///
    /// Note that values are JSON values.
    pub fn entries(&self) -> Ref<'_, Map<String, Value>> {
        Ref::map(self.0.borrow(), |inner| &inner.state)
    }

    /// Returns session status.
    pub fn status(&self) -> SessionStatus {
        Ref::map(self.0.borrow(), |inner| &inner.status).clone()
    }

    /// Inserts a key-value pair into the session.
    ///
    /// Any serializable value can be used and will be encoded as JSON in session data, hence why
    /// only a reference to the value is taken.
    ///
    /// # Errors
    ///
    /// Returns an error if JSON serialization of `value` fails.
    pub fn insert<T: Serialize>(&self, key: impl Into<String>, value: T) -> Result<()> {
        let mut inner = self.0.borrow_mut();

        if inner.status != SessionStatus::Purged {
            if inner.status != SessionStatus::Renewed {
                inner.status = SessionStatus::Changed;
            }

            let key = key.into();
            let val = serde_json::to_value(&value)?;

            inner.state.insert(key, val);
        }

        Ok(())
    }

    /// Updates a key-value pair into the session.
    ///
    /// If the key exists then update it to the new value and place it back in. If the key does not
    /// exist it will not be updated.
    ///
    /// Any serializable value can be used and will be encoded as JSON in the session data, hence
    /// why only a reference to the value is taken.
    ///
    /// # Errors
    ///
    /// Returns an error if JSON serialization of the value fails.
    pub fn update<T: Serialize + DeserializeOwned, F>(
        &self,
        key: impl Into<String>,
        updater: F,
    ) -> Result<()>
    where
        F: FnOnce(T) -> T,
    {
        let mut inner = self.0.borrow_mut();
        let key_str = key.into();

        if let Some(val) = inner.state.get(&key_str) {
            let value = serde_json::from_value(val.clone())?;

            let val = serde_json::to_value(updater(value))?;

            inner.state.insert(key_str, val);
        }

        Ok(())
    }

    /// Updates a key-value pair into the session, or inserts a default value.
    ///
    /// If the key exists then update it to the new value and place it back in. If the key does not
    /// exist the default value will be inserted instead.
    ///
    /// Any serializable value can be used and will be encoded as JSON in session data, hence why
    /// only a reference to the value is taken.
    ///
    /// # Errors
    ///
    /// Returns error if JSON serialization of a value fails.
    pub fn update_or<T: Serialize + DeserializeOwned, F>(
        &self,
        key: &str,
        default_value: T,
        updater: F,
    ) -> Result<()>
    where
        F: FnOnce(T) -> T,
    {
        if self.contains_key(key) {
            self.update(key, updater)
        } else {
            self.insert(key, default_value)
        }
    }

    /// Remove value from the session.
    ///
    /// If present, the JSON value is returned.
    pub fn remove(&self, key: &str) -> Option<Value> {
        let mut inner = self.0.borrow_mut();

        if inner.status != SessionStatus::Purged {
            if inner.status != SessionStatus::Renewed {
                inner.status = SessionStatus::Changed;
            }
            return inner.state.remove(key);
        }

        None
    }

    /// Remove value from the session and deserialize.
    ///
    /// Returns `None` if key was not present in session. Returns `T` if deserialization succeeds,
    /// otherwise returns the raw JSON value.
    pub fn remove_as<T: DeserializeOwned>(&self, key: &str) -> Option<Result<T, Value>> {
        self.remove(key)
            .map(|value| match serde_json::from_value::<T>(value.clone()) {
                Ok(val) => Ok(val),
                Err(_err) => Err(value),
            })
    }

    /// Clear the session.
    pub fn clear(&self) {
        let mut inner = self.0.borrow_mut();

        if inner.status != SessionStatus::Purged {
            if inner.status != SessionStatus::Renewed {
                inner.status = SessionStatus::Changed;
            }
            inner.state.clear()
        }
    }

    /// Removes session both client and server side.
    pub fn purge(&self) {
        let mut inner = self.0.borrow_mut();
        inner.status = SessionStatus::Purged;
        inner.state.clear();
    }

    /// Renews the session key, assigning existing session state to new key.
    pub fn renew(&self) {
        let mut inner = self.0.borrow_mut();

        if inner.status != SessionStatus::Purged {
            inner.status = SessionStatus::Renewed;
        }
    }

    /// Adds the given key-value pairs to the session on the request.
    ///
    /// Values that match keys already existing on the session will be overwritten. Values should
    /// already be JSON values.
    #[allow(clippy::needless_pass_by_ref_mut)]
    pub(crate) fn set_session(
        req: &mut ServiceRequest,
        data: impl IntoIterator<Item = (String, Value)>,
    ) {
        let session = Session::get_session(&mut req.extensions_mut());
        let mut inner = session.0.borrow_mut();
        inner.state.extend(data);
    }

    /// Returns session status and iterator of key-value pairs of changes.
    ///
    /// This is a destructive operation - the session state is removed from the request extensions
    /// typemap, leaving behind a new empty map. It should only be used when the session is being
    /// finalised (i.e. in `SessionMiddleware`).
    #[allow(clippy::needless_pass_by_ref_mut)]
    pub(crate) fn get_changes<B>(
        res: &mut ServiceResponse<B>,
    ) -> (SessionStatus, Map<String, Value>) {
        if let Some(s_impl) = res
            .request()
            .extensions()
            .get::<Rc<RefCell<SessionInner>>>()
        {
            let state = mem::take(&mut s_impl.borrow_mut().state);
            (s_impl.borrow().status.clone(), state)
        } else {
            (SessionStatus::Unchanged, Map::new())
        }
    }

    pub(crate) fn get_session(extensions: &mut Extensions) -> Session {
        if let Some(s_impl) = extensions.get::<Rc<RefCell<SessionInner>>>() {
            return Session(Rc::clone(s_impl));
        }

        let inner = Rc::new(RefCell::new(SessionInner::default()));
        extensions.insert(inner.clone());

        Session(inner)
    }
}

/// Extractor implementation for [`Session`]s.
impl FromRequest for Session {
    type Error = Error;
    type Future = Ready<Result<Session, Error>>;

    #[inline]
    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        ready(Ok(Session::get_session(&mut req.extensions_mut())))
    }
}
