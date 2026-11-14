use std::{
    cell::{Ref, RefCell},
    convert::Infallible,
    future::{ready, Ready},
    mem,
    rc::Rc,
};

use actix_web::{
    dev::{Extensions, Payload, ServiceRequest, ServiceResponse},
    FromRequest, HttpMessage, HttpRequest,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{Map, Value};

use crate::session::error::SessionError;

/// The primary interface to access and modify session state.
///
/// [`Session`] is an [extractor](#impl-FromRequest)—you can specify it as an input type for your
/// request handlers and it will be automatically extracted from the incoming request.
///
/// You can also retrieve a [`Session`] object from an `HttpRequest` or a `ServiceRequest` using
/// [`SessionExt`].
///
/// For tests outside of request handling, use [`Session::new`] to create a standalone empty
/// session.
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
    /// Creates a standalone empty [`Session`].
    ///
    /// This is useful for testing code that accepts a [`Session`] directly. Sessions created using
    /// this constructor are not attached to an Actix Web request, so their state will not be
    /// persisted by [`SessionMiddleware`].
    ///
    /// [`SessionMiddleware`]: super::SessionMiddleware
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a `value` from the session.
    ///
    /// It returns an error if it fails to deserialize as `T` the JSON value associated with `key`.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, SessionError> {
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
    pub fn insert<T: Serialize>(
        &self,
        key: impl Into<String>,
        value: T,
    ) -> Result<(), SessionError> {
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
    /// exist it will not be updated. A successful update marks the session as
    /// [`SessionStatus::Changed`], so the middleware persists it.
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
    ) -> Result<(), SessionError>
    where
        F: FnOnce(T) -> T,
    {
        let mut inner = self.0.borrow_mut();
        if inner.status == SessionStatus::Purged {
            return Ok(());
        }
        let key_str = key.into();

        if let Some(val) = inner.state.get(&key_str) {
            let value = serde_json::from_value(val.clone())?;

            let val = serde_json::to_value(updater(value))?;

            inner.state.insert(key_str, val);
            // Only mark the session after the value was successfully replaced, so a
            // failed (de)serialization does not trigger a needless write.
            if inner.status != SessionStatus::Renewed {
                inner.status = SessionStatus::Changed;
            }
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
    ) -> Result<(), SessionError>
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

        let session = Session::default();
        extensions.insert(Rc::clone(&session.0));

        session
    }
}

impl Default for Session {
    fn default() -> Self {
        Self(Rc::new(RefCell::new(SessionInner::default())))
    }
}

/// Extractor implementation for [`Session`]s.
impl FromRequest for Session {
    type Error = Infallible;
    type Future = Ready<Result<Session, Self::Error>>;

    #[inline]
    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        ready(Ok(Session::get_session(&mut req.extensions_mut())))
    }
}

#[cfg(test)]
mod tests {
    use actix_web::{test as web_test, HttpResponse};
    use serde::{ser::Error as _, Serializer};

    use super::*;

    /// A value whose serialization always fails.
    struct FailingSerialize;

    impl Serialize for FailingSerialize {
        fn serialize<S>(&self, _: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            Err(S::Error::custom("cannot serialize"))
        }
    }

    // Basic state management.

    #[test]
    fn new_session_is_empty_and_unchanged() {
        let s = Session::new();

        assert_eq!(s.status(), SessionStatus::Unchanged);
        assert!(s.entries().is_empty());
        assert!(!s.contains_key("missing"));
        assert_eq!(s.get::<i32>("missing").unwrap(), None);
        assert!(s.remove("missing").is_none());
        assert!(s.remove_as::<i32>("missing").is_none());
    }

    #[test]
    fn insert_get_and_overwrite() {
        let s = Session::new();

        s.insert("key", "value").unwrap();
        assert!(s.contains_key("key"));
        assert_eq!(s.get::<String>("key").unwrap().as_deref(), Some("value"));
        assert_eq!(s.status(), SessionStatus::Changed);
        assert_eq!(s.entries().get("key"), Some(&serde_json::json!("value")));

        // Overwriting replaces the value.
        s.insert("key", 42).unwrap();
        assert_eq!(s.get::<i32>("key").unwrap(), Some(42));

        // `impl Into<String>` accepts owned keys as well.
        let owned = String::from("owned");
        s.insert(owned.clone(), true).unwrap();
        assert_eq!(s.get::<bool>(&owned).unwrap(), Some(true));
    }

    #[test]
    fn get_reports_type_mismatch() {
        let s = Session::new();
        s.insert("key", "text").unwrap();

        assert!(matches!(s.get::<i32>("key"), Err(SessionError::Serde(_))));
    }

    #[test]
    fn insert_reports_serialization_errors() {
        let s = Session::new();

        let err = s.insert("key", FailingSerialize).unwrap_err();
        assert!(matches!(err, SessionError::Serde(_)));
        assert!(!s.contains_key("key"));
    }

    #[test]
    fn update_changes_existing_keys_only() {
        let s = Session::new();
        s.insert("count", 1).unwrap();

        s.update::<i32, _>("count", |v| v + 1).unwrap();
        assert_eq!(s.get::<i32>("count").unwrap(), Some(2));

        // Updating a missing key does nothing.
        s.update::<i32, _>("missing", |v| v + 1).unwrap();
        assert!(!s.contains_key("missing"));

        // The existing value must deserialize into the requested type.
        assert!(matches!(
            s.update::<String, _>("count", |v| v),
            Err(SessionError::Serde(_))
        ));
    }

    #[test]
    fn update_or_inserts_the_default() {
        let s = Session::new();

        // Missing: the default is inserted without running the updater.
        s.update_or("count", 5, |v| v + 1).unwrap();
        assert_eq!(s.get::<i32>("count").unwrap(), Some(5));

        // Present: the updater is applied.
        s.update_or("count", 5, |v| v + 1).unwrap();
        assert_eq!(s.get::<i32>("count").unwrap(), Some(6));
    }

    // Update persistence.

    /// Build a session in the same state as one loaded by the middleware: state is
    /// present but the status is `Unchanged`.
    fn loaded_session() -> Session {
        let mut req = web_test::TestRequest::default().to_srv_request();
        Session::set_session(&mut req, [("count".to_owned(), serde_json::json!(1))]);
        let session = Session::get_session(&mut req.extensions_mut());
        session
    }

    #[test]
    fn update_marks_a_loaded_session_as_changed() {
        let s = loaded_session();
        assert_eq!(s.status(), SessionStatus::Unchanged);

        s.update::<i32, _>("count", |v| v + 1).unwrap();

        assert_eq!(s.get::<i32>("count").unwrap(), Some(2));
        assert_eq!(s.status(), SessionStatus::Changed);
    }

    #[test]
    fn update_without_an_existing_key_keeps_the_status() {
        let s = loaded_session();

        s.update::<i32, _>("missing", |v| v + 1).unwrap();

        assert!(!s.contains_key("missing"));
        assert_eq!(s.status(), SessionStatus::Unchanged);
    }

    #[test]
    fn failed_update_keeps_the_status() {
        let s = loaded_session();

        assert!(matches!(
            s.update::<String, _>("count", |v| v),
            Err(SessionError::Serde(_))
        ));

        // The stored value is untouched and no write is triggered.
        assert_eq!(s.get::<i32>("count").unwrap(), Some(1));
        assert_eq!(s.status(), SessionStatus::Unchanged);
    }

    #[test]
    fn update_keeps_the_renewed_status() {
        let s = loaded_session();
        s.renew();

        s.update::<i32, _>("count", |v| v + 1).unwrap();

        assert_eq!(s.get::<i32>("count").unwrap(), Some(2));
        assert_eq!(s.status(), SessionStatus::Renewed);
    }

    #[test]
    fn update_on_a_purged_session_is_a_no_op() {
        let s = loaded_session();
        s.purge();

        s.update::<i32, _>("count", |_| panic!("updater must not run"))
            .unwrap();

        assert!(s.entries().is_empty());
        assert_eq!(s.status(), SessionStatus::Purged);
    }

    #[test]
    fn update_or_marks_a_loaded_session_as_changed() {
        let s = loaded_session();
        s.update_or("count", 0, |v| v + 1).unwrap();
        assert_eq!(s.get::<i32>("count").unwrap(), Some(2));
        assert_eq!(s.status(), SessionStatus::Changed);

        // The insert branch also marks the session as changed.
        let s = loaded_session();
        s.update_or("other", 7, |v| v + 1).unwrap();
        assert_eq!(s.get::<i32>("other").unwrap(), Some(7));
        assert_eq!(s.status(), SessionStatus::Changed);
    }

    #[test]
    fn remove_and_remove_as() {
        let s = Session::new();

        s.insert("text", "value").unwrap();
        assert_eq!(s.remove("text"), Some(serde_json::json!("value")));
        assert!(!s.contains_key("text"));
        assert!(s.remove("text").is_none());

        s.insert("text", "value").unwrap();
        assert_eq!(s.remove_as::<String>("text"), Some(Ok("value".to_owned())));

        // Failed deserialization returns the raw JSON value.
        s.insert("text", "value").unwrap();
        assert_eq!(
            s.remove_as::<i32>("text"),
            Some(Err(serde_json::json!("value")))
        );
        assert!(!s.contains_key("text"));
    }

    // Lifecycle status.

    #[test]
    fn clear_sets_changed_and_purge_blocks_mutations() {
        let s = Session::new();
        s.insert("a", 1).unwrap();

        s.clear();
        assert!(s.entries().is_empty());
        assert_eq!(s.status(), SessionStatus::Changed);

        s.insert("b", 2).unwrap();
        s.purge();
        assert_eq!(s.status(), SessionStatus::Purged);
        assert!(!s.contains_key("b"));

        // Mutations after purge are no-ops.
        s.insert("c", 3).unwrap();
        assert!(!s.contains_key("c"));
        s.clear();
        s.renew();
        assert_eq!(s.status(), SessionStatus::Purged);
        assert!(s.remove("b").is_none());
    }

    #[test]
    fn renew_keeps_state_and_status() {
        let s = Session::new();
        s.insert("a", 1).unwrap();

        s.renew();
        assert_eq!(s.status(), SessionStatus::Renewed);
        assert_eq!(s.get::<i32>("a").unwrap(), Some(1));

        // Later mutations keep the Renewed status until the middleware finalizes it.
        s.insert("b", 2).unwrap();
        s.clear();
        assert_eq!(s.status(), SessionStatus::Renewed);

        // Purge wins over renew.
        s.purge();
        assert_eq!(s.status(), SessionStatus::Purged);
    }

    // Request integration.

    #[actix_web::test]
    async fn from_request_reuses_the_session_in_extensions() {
        let req = web_test::TestRequest::default().to_http_request();

        let session = Session::from_request(&req, &mut actix_web::dev::Payload::None)
            .await
            .unwrap();
        session.insert("a", 1).unwrap();

        // Extracting again yields the same underlying state.
        let again = Session::get_session(&mut req.extensions_mut());
        assert_eq!(again.get::<i32>("a").unwrap(), Some(1));
    }

    #[actix_web::test]
    async fn set_session_and_get_changes() {
        let mut req = web_test::TestRequest::default().to_srv_request();
        Session::set_session(&mut req, [("a".to_owned(), serde_json::json!(1))]);

        let mut rsp = req.into_response(HttpResponse::Ok().finish());
        let (status, state) = Session::get_changes(&mut rsp);
        assert_eq!(status, SessionStatus::Unchanged);
        assert_eq!(state.get("a"), Some(&serde_json::json!(1)));

        // The state is taken out of the request extensions.
        let (status, state) = Session::get_changes(&mut rsp);
        assert_eq!(status, SessionStatus::Unchanged);
        assert!(state.is_empty());

        // A response without a session reports no changes.
        let mut rsp = web_test::TestRequest::default()
            .to_srv_request()
            .into_response(HttpResponse::Ok().finish());
        assert_eq!(
            Session::get_changes(&mut rsp),
            (SessionStatus::Unchanged, Map::new())
        );
    }
}
