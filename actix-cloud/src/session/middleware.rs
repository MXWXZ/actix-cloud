use std::{
    borrow::Cow,
    fmt,
    future::{ready, Future, Ready},
    pin::Pin,
    rc::Rc,
    sync::Arc,
};

use actix_web::{
    body::MessageBody,
    cookie::{time::Duration, Cookie, CookieJar, Key},
    dev::{forward_ready, ResponseHead, Service, ServiceRequest, ServiceResponse, Transform},
    http::header::{HeaderValue, InvalidHeaderValue, SET_COOKIE},
    HttpResponse,
};
use serde_json::{Map, Value};

use super::{
    config::{
        self, Configuration, CookieConfiguration, CookieContentSecurity, SessionMiddlewareBuilder,
        TtlExtensionPolicy,
    },
    storage::{SessionKey, SessionStore},
    Session, SessionStatus,
};
use crate::memorydb::MemoryDB;

/// A middleware for session management in Actix Web applications.
///
/// [`SessionMiddleware`] takes care of a few jobs:
///
/// - Instructs the session storage backend to create/update/delete/retrieve the state attached to
///   a session according to its status and the operations that have been performed against it;
/// - Set/remove a cookie, on the client side, to enable a user to be consistently associated with
///   the same session across multiple HTTP requests.
///
/// Use [`SessionMiddleware::new`] to initialize the session framework using the default parameters.
/// To create a new instance of [`SessionMiddleware`] you need to provide:
///
/// - a [`MemoryDB`] instance used as the session storage backend;
/// - a secret key, to sign or encrypt the content of client-side session cookie.
///
/// # Storage capacity
///
/// Every non-empty session is written to the provided [`MemoryDB`]. The in-process
/// [`DefaultBackend`](crate::memorydb::default::DefaultBackend) is unbounded unless
/// configured, so a deployment reachable by untrusted clients should set
/// [`max_entries`](crate::memorydb::default::DefaultBackend::max_entries) and
/// [`max_bytes`](crate::memorydb::default::DefaultBackend::max_bytes); otherwise
/// anonymous sessions can grow without limit.
///
/// # How did we choose defaults?
/// You should not regret adding `actix-session` to your dependencies and going to production using
/// the default configuration. That is why, when in doubt, we opt to use the most secure option for
/// each configuration parameter.
///
/// We expose knobs to change the default to suit your needs—i.e., if you know what you are doing,
/// we will not stop you. But being a subject-matter expert should not be a requirement to deploy
/// reasonably secure implementation of sessions.
#[derive(Clone)]
pub struct SessionMiddleware {
    storage_backend: Rc<SessionStore>,
    configuration: Rc<Configuration>,
}

impl SessionMiddleware {
    /// Use [`SessionMiddleware::new`] to initialize the session framework using the default
    /// parameters.
    ///
    /// To create a new instance of [`SessionMiddleware`] you need to provide:
    /// - a [`MemoryDB`] instance used as the session storage backend;
    /// - a secret key, to sign or encrypt the content of client-side session cookie.
    pub fn new(client: Arc<dyn MemoryDB>, key: Key) -> Self {
        Self::builder(client, key).build()
    }

    /// A fluent API to configure [`SessionMiddleware`].
    ///
    /// It takes as input the two required inputs to create a new instance of [`SessionMiddleware`]:
    /// - a [`MemoryDB`] instance used as the session storage backend;
    /// - a secret key, to sign or encrypt the content of client-side session cookie.
    pub fn builder(client: Arc<dyn MemoryDB>, key: Key) -> SessionMiddlewareBuilder {
        SessionMiddlewareBuilder::new(client, config::default_configuration(key))
    }

    pub(crate) fn from_parts(store: SessionStore, configuration: Configuration) -> Self {
        Self {
            storage_backend: Rc::new(store),
            configuration: Rc::new(configuration),
        }
    }
}

impl<S, B> Transform<S, ServiceRequest> for SessionMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = actix_web::Error> + 'static,
    S::Future: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<B>;
    type Error = actix_web::Error;
    type Transform = InnerSessionMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(InnerSessionMiddleware {
            service: Rc::new(service),
            configuration: Rc::clone(&self.configuration),
            storage_backend: Rc::clone(&self.storage_backend),
        }))
    }
}

/// Short-hand to create an `actix_web::Error` instance that will result in an `Internal Server
/// Error` response while preserving the error root cause (e.g. in logs).
fn e500<E: fmt::Debug + fmt::Display + 'static>(err: E) -> actix_web::Error {
    // We do not use `actix_web::error::ErrorInternalServerError` because we do not want to
    // leak internal implementation details to the caller.
    //
    // `actix_web::error::ErrorInternalServerError` includes the error Display representation
    // as body of the error responses, leading to messages like "There was an issue persisting
    // the session state" reaching API clients. We don't want that, we want opaque 500s.
    actix_web::error::InternalError::from_response(
        err,
        HttpResponse::InternalServerError().finish(),
    )
    .into()
}

#[doc(hidden)]
#[non_exhaustive]
pub struct InnerSessionMiddleware<S> {
    service: Rc<S>,
    configuration: Rc<Configuration>,
    storage_backend: Rc<SessionStore>,
}

impl<S, B> Service<ServiceRequest> for InnerSessionMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = actix_web::Error> + 'static,
    S::Future: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = actix_web::Error;
    #[allow(clippy::type_complexity)]
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    forward_ready!(service);

    fn call(&self, mut req: ServiceRequest) -> Self::Future {
        let service = Rc::clone(&self.service);
        let storage_backend = Rc::clone(&self.storage_backend);
        let configuration = Rc::clone(&self.configuration);

        Box::pin(async move {
            let session_key = extract_session_key(&req, &configuration.cookie);
            let (session_key, session_state) =
                load_session_state(session_key, storage_backend.as_ref()).await?;

            // `purge` clears the state, so keep the id recorded when the session was
            // loaded: its reverse lookup entry still has to be deleted.
            let previous_id = session_id(&session_state);
            Session::set_session(&mut req, session_state);

            let mut res = service.call(req).await?;
            let (status, session_state) = Session::get_changes(&mut res);

            let mut ttl = configuration.session.state_ttl;
            let mut cookie = Cow::Borrowed(&configuration.cookie);
            if let Some(x) = session_state.get("_ttl") {
                if let Some(x) = x.as_i64() {
                    ttl = Duration::milliseconds(x);
                    let mut tmp = cookie.into_owned();
                    tmp.max_age = Some(ttl);
                    cookie = Cow::Owned(tmp);
                }
            }
            let id = if status == SessionStatus::Purged {
                previous_id
            } else {
                session_id(&session_state)
            };

            match session_key {
                None => {
                    // we do not create an entry in the session store if there is no state attached
                    // to a fresh session
                    if !session_state.is_empty() {
                        let session_key = storage_backend
                            .save(session_state, &id, &ttl)
                            .await
                            .map_err(e500)?;

                        set_session_cookie(res.response_mut().head_mut(), session_key, &cookie)
                            .map_err(e500)?;
                    }
                }

                Some(session_key) => {
                    match status {
                        SessionStatus::Changed => {
                            let session_key = storage_backend
                                .update(session_key, session_state, &id, &ttl)
                                .await
                                .map_err(e500)?;

                            set_session_cookie(res.response_mut().head_mut(), session_key, &cookie)
                                .map_err(e500)?;
                        }

                        SessionStatus::Purged => {
                            storage_backend
                                .delete(&session_key, &id)
                                .await
                                .map_err(e500)?;

                            delete_session_cookie(res.response_mut().head_mut(), &cookie)
                                .map_err(e500)?;
                        }

                        SessionStatus::Renewed => {
                            let session_key = storage_backend
                                .renew(session_key, session_state, &id, &ttl)
                                .await
                                .map_err(e500)?;

                            set_session_cookie(res.response_mut().head_mut(), session_key, &cookie)
                                .map_err(e500)?;
                        }

                        SessionStatus::Unchanged => {
                            if matches!(
                                configuration.ttl_extension_policy,
                                TtlExtensionPolicy::OnEveryRequest
                            ) {
                                storage_backend
                                    .update_ttl(&session_key, &id, &ttl)
                                    .await
                                    .map_err(e500)?;

                                if configuration.cookie.max_age.is_some() {
                                    set_session_cookie(
                                        res.response_mut().head_mut(),
                                        session_key,
                                        &cookie,
                                    )
                                    .map_err(e500)?;
                                }
                            }
                        }
                    };
                }
            }

            Ok(res)
        })
    }
}

/// Examines the session cookie attached to the incoming request, if there is one, and tries
/// to extract the session key.
///
/// It returns `None` if there is no session cookie or if the session cookie is considered invalid
/// (e.g., when failing a signature check).
fn extract_session_key(req: &ServiceRequest, config: &CookieConfiguration) -> Option<SessionKey> {
    // Duplicated cookies are rejected instead of silently using the first one.
    let session_cookie = crate::cookie::unique_cookie(req, &config.name)?;

    let mut jar = CookieJar::new();
    jar.add_original(session_cookie);

    let verification_result = match config.content_security {
        CookieContentSecurity::Signed => jar.signed(&config.key).get(&config.name),
        CookieContentSecurity::Private => jar.private(&config.key).get(&config.name),
    };

    verification_result?.value().to_owned().try_into().ok()
}

/// Extract the reverse-lookup id (`_id`) from session state.
fn session_id(state: &Map<String, Value>) -> Option<String> {
    state
        .get("_id")
        .map(|x| x.to_string().trim_matches('"').to_owned())
}

async fn load_session_state(
    session_key: Option<SessionKey>,
    storage_backend: &SessionStore,
) -> Result<(Option<SessionKey>, Map<String, Value>), actix_web::Error> {
    if let Some(session_key) = session_key {
        match storage_backend.load(&session_key).await {
            Ok(state) => {
                if let Some(state) = state {
                    Ok((Some(session_key), state))
                } else {
                    // We discard the existing session key given that the state attached to it can
                    // no longer be found (e.g. it expired or we suffered some data loss in the
                    // storage). Regenerating the session key will trigger the `save` workflow
                    // instead of the `update` workflow if the session state is modified during the
                    // lifecycle of the current request.

                    Ok((None, Map::new()))
                }
            }

            Err(err) => Err(e500(err)),
        }
    } else {
        Ok((None, Map::new()))
    }
}

fn set_session_cookie(
    response: &mut ResponseHead,
    session_key: SessionKey,
    config: &CookieConfiguration,
) -> Result<(), InvalidHeaderValue> {
    let value: String = session_key.into();
    let mut cookie = Cookie::new(config.name.clone(), value);

    cookie.set_secure(config.secure);
    cookie.set_http_only(config.http_only);
    cookie.set_same_site(config.same_site);
    cookie.set_path(config.path.clone());

    if let Some(max_age) = config.max_age {
        cookie.set_max_age(max_age);
    }

    if let Some(ref domain) = config.domain {
        cookie.set_domain(domain.clone());
    }

    let mut jar = CookieJar::new();
    match config.content_security {
        CookieContentSecurity::Signed => jar.signed_mut(&config.key).add(cookie),
        CookieContentSecurity::Private => jar.private_mut(&config.key).add(cookie),
    }

    // set cookie
    let cookie = jar.delta().next().unwrap();
    let val = HeaderValue::from_str(&cookie.encoded().to_string())?;

    response.headers_mut().append(SET_COOKIE, val);

    Ok(())
}

fn delete_session_cookie(
    response: &mut ResponseHead,
    config: &CookieConfiguration,
) -> Result<(), InvalidHeaderValue> {
    let removal_cookie = Cookie::build(config.name.clone(), "")
        .path(config.path.clone())
        .secure(config.secure)
        .http_only(config.http_only)
        .same_site(config.same_site);

    let mut removal_cookie = if let Some(ref domain) = config.domain {
        removal_cookie.domain(domain)
    } else {
        removal_cookie
    }
    .finish();

    removal_cookie.make_removal();

    let val = HeaderValue::from_str(&removal_cookie.to_string())?;
    response.headers_mut().append(SET_COOKIE, val);

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration as StdDuration};

    use actix_web::{
        cookie::{time::Duration as CookieDuration, Cookie, Key, SameSite},
        http::{header, StatusCode},
        test, web, App, HttpResponse,
    };

    use super::*;
    use crate::{
        memorydb::{default::DefaultBackend, MemoryDB},
        session::{
            config::{CookieContentSecurity, PersistentSession, TtlExtensionPolicy},
            Session,
        },
    };

    fn backend() -> Arc<dyn MemoryDB> {
        Arc::new(DefaultBackend::new())
    }

    fn find_cookie<'a, B>(rsp: &'a ServiceResponse<B>, name: &str) -> Option<Cookie<'a>> {
        rsp.response().cookies().find(|c| c.name() == name)
    }

    async fn insert_user(session: Session) -> HttpResponse {
        session.insert("user", 1).unwrap();
        HttpResponse::Ok().finish()
    }

    async fn read_user(session: Session) -> HttpResponse {
        let value = session.get::<i32>("user").unwrap();
        HttpResponse::Ok().body(value.map_or_else(|| "none".to_owned(), |v| v.to_string()))
    }

    async fn purge_session(session: Session) -> HttpResponse {
        session.purge();
        HttpResponse::Ok().finish()
    }

    async fn renew_session(session: Session) -> HttpResponse {
        session.renew();
        HttpResponse::Ok().finish()
    }

    async fn touch_session(_: Session) -> HttpResponse {
        HttpResponse::Ok().finish()
    }

    async fn insert_ttl(session: Session) -> HttpResponse {
        session.insert("_ttl", 5000).unwrap();
        session.insert("user", 1).unwrap();
        HttpResponse::Ok().finish()
    }

    async fn insert_id(session: Session) -> HttpResponse {
        session.insert("_id", "user1").unwrap();
        session.insert("user", 1).unwrap();
        HttpResponse::Ok().finish()
    }

    async fn insert_secret(session: Session) -> HttpResponse {
        session.insert("secret", "SESSION_SECRET_MARKER").unwrap();
        HttpResponse::Ok().finish()
    }

    async fn insert_count(session: Session) -> HttpResponse {
        session.insert("count", 1).unwrap();
        HttpResponse::Ok().finish()
    }

    async fn bump_count(session: Session) -> HttpResponse {
        session.update::<i32, _>("count", |v| v + 1).unwrap();
        HttpResponse::Ok().finish()
    }

    async fn read_count(session: Session) -> HttpResponse {
        let value = session.get::<i32>("count").unwrap();
        HttpResponse::Ok().body(value.map_or_else(|| "none".to_owned(), |v| v.to_string()))
    }

    /// Flip the first character of an encoded cookie value so the signature or the
    /// ciphertext no longer verifies.
    fn tamper(cookie: &Cookie<'_>) -> Cookie<'static> {
        let mut value = cookie.value().to_owned();
        let first = value.chars().next().expect("non-empty cookie value");
        let replacement = if first == 'A' { 'B' } else { 'A' };
        value.replace_range(0..first.len_utf8(), &replacement.to_string());
        Cookie::new(cookie.name().to_owned(), value)
    }

    // Persistence.

    #[actix_web::test]
    async fn new_session_is_saved_with_a_secure_cookie() {
        let backend = backend();
        let app = test::init_service(
            App::new()
                .wrap(SessionMiddleware::new(backend.clone(), Key::generate()))
                .route("/", web::get().to(insert_user)),
        )
        .await;

        let rsp = test::call_service(&app, test::TestRequest::get().uri("/").to_request()).await;
        assert_eq!(rsp.status(), StatusCode::OK);

        let cookie = find_cookie(&rsp, "id").expect("session cookie");
        assert!(!cookie.value().is_empty());
        assert_eq!(cookie.http_only(), Some(true));
        assert_eq!(cookie.secure(), Some(true));
        assert_eq!(cookie.same_site(), Some(SameSite::Lax));
        assert_eq!(cookie.path(), Some("/"));

        let keys = backend.keys("*").await.unwrap();
        assert_eq!(keys.len(), 1);
        let ttl = backend.ttl(&keys[0]).await.unwrap().unwrap();
        assert!(ttl > StdDuration::from_secs(23 * 60 * 60));
        assert!(ttl <= StdDuration::from_secs(24 * 60 * 60));
    }

    #[actix_web::test]
    async fn subsequent_requests_load_the_session() {
        let backend = backend();
        let app = test::init_service(
            App::new()
                .wrap(SessionMiddleware::new(backend.clone(), Key::generate()))
                .route("/insert", web::get().to(insert_user))
                .route("/read", web::get().to(read_user)),
        )
        .await;

        let rsp =
            test::call_service(&app, test::TestRequest::get().uri("/insert").to_request()).await;
        let cookie = find_cookie(&rsp, "id").unwrap().into_owned();

        let rsp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/read")
                .cookie(cookie)
                .to_request(),
        )
        .await;
        // An unchanged session is not written back and gets no new cookie.
        assert!(find_cookie(&rsp, "id").is_none());
        assert_eq!(test::read_body(rsp).await.as_ref(), b"1");
        assert_eq!(backend.keys("*").await.unwrap().len(), 1);
    }

    // Lifecycle.

    #[actix_web::test]
    async fn purge_deletes_state_and_expires_the_cookie() {
        let backend = backend();
        let app = test::init_service(
            App::new()
                .wrap(SessionMiddleware::new(backend.clone(), Key::generate()))
                .route("/insert", web::get().to(insert_user))
                .route("/purge", web::get().to(purge_session)),
        )
        .await;

        let rsp =
            test::call_service(&app, test::TestRequest::get().uri("/insert").to_request()).await;
        let cookie = find_cookie(&rsp, "id").unwrap().into_owned();

        let rsp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/purge")
                .cookie(cookie)
                .to_request(),
        )
        .await;
        assert_eq!(rsp.status(), StatusCode::OK);

        let removal = find_cookie(&rsp, "id").expect("removal cookie");
        assert_eq!(removal.value(), "");
        assert_eq!(removal.max_age(), Some(CookieDuration::ZERO));
        assert!(backend.keys("*").await.unwrap().is_empty());
    }

    #[actix_web::test]
    async fn renew_rotates_the_session_key() {
        let backend = backend();
        let app = test::init_service(
            App::new()
                .wrap(SessionMiddleware::new(backend.clone(), Key::generate()))
                .route("/insert", web::get().to(insert_user))
                .route("/renew", web::get().to(renew_session))
                .route("/read", web::get().to(read_user)),
        )
        .await;

        let rsp =
            test::call_service(&app, test::TestRequest::get().uri("/insert").to_request()).await;
        let old = find_cookie(&rsp, "id").unwrap().into_owned();
        let old_value = old.value().to_owned();

        let rsp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/renew")
                .cookie(old.clone())
                .to_request(),
        )
        .await;
        let new = find_cookie(&rsp, "id").unwrap().into_owned();
        assert_ne!(new.value(), old_value);
        // Only the new key holds the session state.
        assert_eq!(backend.keys("*").await.unwrap().len(), 1);

        let rsp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/read")
                .cookie(old)
                .to_request(),
        )
        .await;
        assert_eq!(test::read_body(rsp).await.as_ref(), b"none");

        let rsp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/read")
                .cookie(new)
                .to_request(),
        )
        .await;
        assert_eq!(test::read_body(rsp).await.as_ref(), b"1");
    }

    #[actix_web::test]
    async fn ttl_override_updates_cookie_and_state_ttl() {
        let backend = backend();
        let app = test::init_service(
            App::new()
                .wrap(SessionMiddleware::new(backend.clone(), Key::generate()))
                .route("/", web::get().to(insert_ttl)),
        )
        .await;

        let rsp = test::call_service(&app, test::TestRequest::get().uri("/").to_request()).await;
        assert_eq!(
            find_cookie(&rsp, "id").unwrap().max_age(),
            Some(CookieDuration::seconds(5))
        );

        let keys = backend.keys("*").await.unwrap();
        let ttl = backend.ttl(&keys[0]).await.unwrap().unwrap();
        assert!(ttl > StdDuration::from_secs(4));
        assert!(ttl <= StdDuration::from_secs(5));
    }

    #[actix_web::test]
    async fn id_records_and_removes_the_reverse_lookup() {
        let backend = backend();
        let app = test::init_service(
            App::new()
                .wrap(SessionMiddleware::new(backend.clone(), Key::generate()))
                .route("/insert", web::get().to(insert_id))
                .route("/purge", web::get().to(purge_session)),
        )
        .await;

        let rsp =
            test::call_service(&app, test::TestRequest::get().uri("/insert").to_request()).await;
        let cookie = find_cookie(&rsp, "id").unwrap().into_owned();

        let mut keys = backend.keys("*").await.unwrap();
        keys.sort();
        assert_eq!(keys.len(), 2);
        let id_key = keys
            .iter()
            .find(|k| k.starts_with("user1_"))
            .expect("reverse lookup key");
        assert_eq!(backend.get(id_key).await.unwrap().as_deref(), Some("1"));

        test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/purge")
                .cookie(cookie)
                .to_request(),
        )
        .await;
        assert!(backend.keys("*").await.unwrap().is_empty());
    }

    // Edge cases and configuration.

    #[actix_web::test]
    async fn invalid_cookie_starts_a_fresh_session() {
        let backend = backend();
        let app = test::init_service(
            App::new()
                .wrap(SessionMiddleware::new(backend.clone(), Key::generate()))
                .route("/", web::get().to(insert_user)),
        )
        .await;

        let rsp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/")
                .cookie(Cookie::new("id", "bogus"))
                .to_request(),
        )
        .await;

        assert_eq!(rsp.status(), StatusCode::OK);
        assert!(find_cookie(&rsp, "id").is_some());
        assert_eq!(backend.keys("*").await.unwrap().len(), 1);
    }

    #[actix_web::test]
    async fn duplicate_session_cookies_are_rejected() {
        let backend = backend();
        let app = test::init_service(
            App::new()
                .wrap(SessionMiddleware::new(backend.clone(), Key::generate()))
                .route("/insert", web::get().to(insert_user))
                .route("/read", web::get().to(read_user)),
        )
        .await;

        let rsp =
            test::call_service(&app, test::TestRequest::get().uri("/insert").to_request()).await;
        let cookie = find_cookie(&rsp, "id").unwrap().into_owned();
        // `TestRequest::cookie` deduplicates through a cookie jar, so send the raw
        // header to exercise cookie shadowing.
        let duplicated = format!("id={}; id={}", cookie.value(), cookie.value());

        let rsp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/read")
                .insert_header((header::COOKIE, duplicated))
                .to_request(),
        )
        .await;

        // The duplicated cookie is ignored, so a fresh empty session is used.
        assert_eq!(test::read_body(rsp).await.as_ref(), b"none");
        // The stored session was not touched.
        assert_eq!(backend.keys("*").await.unwrap().len(), 1);
    }

    #[actix_web::test]
    async fn lost_backend_state_starts_a_fresh_session() {
        let backend = backend();
        let app = test::init_service(
            App::new()
                .wrap(SessionMiddleware::new(backend.clone(), Key::generate()))
                .route("/insert", web::get().to(insert_user))
                .route("/read", web::get().to(read_user)),
        )
        .await;

        let rsp =
            test::call_service(&app, test::TestRequest::get().uri("/insert").to_request()).await;
        let cookie = find_cookie(&rsp, "id").unwrap().into_owned();
        backend.flush().await.unwrap();

        // The signed cookie is still valid but its state is gone.
        let rsp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/read")
                .cookie(cookie.clone())
                .to_request(),
        )
        .await;
        assert!(find_cookie(&rsp, "id").is_none());
        assert_eq!(test::read_body(rsp).await.as_ref(), b"none");

        // A changed session is saved under a new key.
        let rsp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/insert")
                .cookie(cookie)
                .to_request(),
        )
        .await;
        assert!(find_cookie(&rsp, "id").is_some());
        assert_eq!(backend.keys("*").await.unwrap().len(), 1);
    }

    #[actix_web::test]
    async fn ttl_extension_policy_refreshes_unchanged_sessions() {
        let backend = backend();
        let mw = SessionMiddleware::builder(backend.clone(), Key::generate())
            .session_lifecycle(
                PersistentSession::default()
                    .session_ttl_extension_policy(TtlExtensionPolicy::OnEveryRequest),
            )
            .build();
        let app = test::init_service(
            App::new()
                .wrap(mw)
                .route("/insert", web::get().to(insert_user))
                .route("/touch", web::get().to(touch_session)),
        )
        .await;

        let rsp =
            test::call_service(&app, test::TestRequest::get().uri("/insert").to_request()).await;
        let cookie = find_cookie(&rsp, "id").unwrap().into_owned();
        // A persistent session always sets Max-Age.
        assert_eq!(cookie.max_age(), Some(CookieDuration::days(1)));

        let rsp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/touch")
                .cookie(cookie)
                .to_request(),
        )
        .await;
        assert_eq!(rsp.status(), StatusCode::OK);
        // The TTL is refreshed and the cookie is re-sent even without state changes.
        assert!(find_cookie(&rsp, "id").is_some());
        assert_eq!(backend.keys("*").await.unwrap().len(), 1);
    }

    #[actix_web::test]
    async fn cookie_configuration_is_applied_and_signed_cookies_load() {
        let backend = backend();
        let mw = SessionMiddleware::builder(backend.clone(), Key::generate())
            .cookie_name("sid".into())
            .cookie_secure(false)
            .cookie_http_only(false)
            .cookie_same_site(SameSite::Strict)
            .cookie_path("/app".into())
            .cookie_domain(Some("example.com".into()))
            .cookie_content_security(CookieContentSecurity::Signed)
            .build();
        let app = test::init_service(
            App::new()
                .wrap(mw)
                .route("/insert", web::get().to(insert_user))
                .route("/read", web::get().to(read_user)),
        )
        .await;

        let rsp =
            test::call_service(&app, test::TestRequest::get().uri("/insert").to_request()).await;
        let cookie = find_cookie(&rsp, "sid").expect("session cookie");
        assert_eq!(cookie.secure(), None);
        assert_eq!(cookie.http_only(), None);
        assert_eq!(cookie.same_site(), Some(SameSite::Strict));
        assert_eq!(cookie.path(), Some("/app"));
        assert_eq!(cookie.domain(), Some("example.com"));
        let cookie = cookie.into_owned();

        let rsp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/read")
                .cookie(cookie)
                .to_request(),
        )
        .await;
        assert_eq!(test::read_body(rsp).await.as_ref(), b"1");
    }

    #[actix_web::test]
    async fn handler_error_responses_still_finalize_the_session() {
        async fn failing(session: Session) -> actix_web::Result<HttpResponse> {
            session.insert("user", 1).unwrap();
            Err(actix_web::error::ErrorInternalServerError("boom"))
        }

        let backend = backend();
        let app = test::init_service(
            App::new()
                .wrap(SessionMiddleware::new(backend.clone(), Key::generate()))
                .route("/", web::get().to(failing)),
        )
        .await;

        let rsp = test::call_service(&app, test::TestRequest::get().uri("/").to_request()).await;

        // `Result` handlers turn errors into responses inside the route, so the
        // middleware still observes the changed session and saves it.
        assert_eq!(rsp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(find_cookie(&rsp, "id").is_some());
        assert_eq!(backend.keys("*").await.unwrap().len(), 1);
    }

    // Security properties.

    #[actix_web::test]
    async fn tampered_session_cookies_are_rejected() {
        for content_security in [
            CookieContentSecurity::Private,
            CookieContentSecurity::Signed,
        ] {
            let backend = backend();
            let mw = SessionMiddleware::builder(backend.clone(), Key::generate())
                .cookie_content_security(content_security)
                .build();
            let app = test::init_service(
                App::new()
                    .wrap(mw)
                    .route("/insert", web::get().to(insert_user))
                    .route("/read", web::get().to(read_user)),
            )
            .await;

            let rsp =
                test::call_service(&app, test::TestRequest::get().uri("/insert").to_request())
                    .await;
            let cookie = find_cookie(&rsp, "id").unwrap().into_owned();

            // A modified cookie must not load the original state.
            let rsp = test::call_service(
                &app,
                test::TestRequest::get()
                    .uri("/read")
                    .cookie(tamper(&cookie))
                    .to_request(),
            )
            .await;
            assert_eq!(
                test::read_body(rsp).await.as_ref(),
                b"none",
                "{content_security:?}"
            );

            // The untampered cookie still works.
            let rsp = test::call_service(
                &app,
                test::TestRequest::get()
                    .uri("/read")
                    .cookie(cookie)
                    .to_request(),
            )
            .await;
            assert_eq!(
                test::read_body(rsp).await.as_ref(),
                b"1",
                "{content_security:?}"
            );
        }
    }

    #[actix_web::test]
    async fn a_cookie_from_another_key_is_rejected() {
        let backend = backend();
        let app_a = test::init_service(
            App::new()
                .wrap(SessionMiddleware::new(backend.clone(), Key::generate()))
                .route("/insert", web::get().to(insert_user)),
        )
        .await;
        let app_b = test::init_service(
            App::new()
                .wrap(SessionMiddleware::new(backend.clone(), Key::generate()))
                .route("/read", web::get().to(read_user)),
        )
        .await;

        let rsp =
            test::call_service(&app_a, test::TestRequest::get().uri("/insert").to_request()).await;
        let cookie = find_cookie(&rsp, "id").unwrap().into_owned();

        // The session key is signed/encrypted with a different key, so it cannot be
        // used to load another application's sessions.
        let rsp = test::call_service(
            &app_b,
            test::TestRequest::get()
                .uri("/read")
                .cookie(cookie)
                .to_request(),
        )
        .await;
        assert_eq!(test::read_body(rsp).await.as_ref(), b"none");
    }

    #[actix_web::test]
    async fn session_state_is_not_stored_in_the_cookie() {
        for content_security in [
            CookieContentSecurity::Private,
            CookieContentSecurity::Signed,
        ] {
            let backend = backend();
            let mw = SessionMiddleware::builder(backend, Key::generate())
                .cookie_content_security(content_security)
                .build();
            let app =
                test::init_service(App::new().wrap(mw).route("/", web::get().to(insert_secret)))
                    .await;

            let rsp =
                test::call_service(&app, test::TestRequest::get().uri("/").to_request()).await;
            let cookie = find_cookie(&rsp, "id").expect("session cookie");

            // Session state lives in the backend; the cookie only carries the key.
            assert!(!cookie.value().contains("SESSION_SECRET_MARKER"));
        }
    }

    #[actix_web::test]
    async fn sessions_without_state_get_no_cookie() {
        let backend = backend();
        let app = test::init_service(
            App::new()
                .wrap(SessionMiddleware::new(backend.clone(), Key::generate()))
                .route("/", web::get().to(touch_session)),
        )
        .await;

        let rsp = test::call_service(&app, test::TestRequest::get().uri("/").to_request()).await;

        // No tracking cookie or backend entry is created for an empty session.
        assert!(find_cookie(&rsp, "id").is_none());
        assert!(backend.keys("*").await.unwrap().is_empty());
    }

    #[actix_web::test]
    async fn renew_moves_the_reverse_lookup_key() {
        let backend = backend();
        let app = test::init_service(
            App::new()
                .wrap(SessionMiddleware::new(backend.clone(), Key::generate()))
                .route("/insert", web::get().to(insert_id))
                .route("/renew", web::get().to(renew_session))
                .route("/read", web::get().to(read_user)),
        )
        .await;

        let rsp =
            test::call_service(&app, test::TestRequest::get().uri("/insert").to_request()).await;
        let old_cookie = find_cookie(&rsp, "id").unwrap().into_owned();

        let before = backend.keys("*").await.unwrap();
        assert_eq!(before.len(), 2);
        let old_key = before
            .iter()
            .find(|k| !k.starts_with("user1_"))
            .expect("session key")
            .clone();
        let old_reverse = format!("user1_{old_key}");
        assert!(before.contains(&old_reverse));

        let rsp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/renew")
                .cookie(old_cookie.clone())
                .to_request(),
        )
        .await;
        let new_cookie = find_cookie(&rsp, "id").unwrap().into_owned();

        let after = backend.keys("*").await.unwrap();
        assert_eq!(after.len(), 2);
        assert!(!after.contains(&old_key));
        assert!(!after.contains(&old_reverse));
        let new_key = after
            .iter()
            .find(|k| !k.starts_with("user1_"))
            .expect("session key")
            .clone();
        assert!(after.contains(&format!("user1_{new_key}")));

        // The old cookie is dead, the new one carries the state.
        let rsp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/read")
                .cookie(old_cookie)
                .to_request(),
        )
        .await;
        assert_eq!(test::read_body(rsp).await.as_ref(), b"none");

        let rsp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/read")
                .cookie(new_cookie)
                .to_request(),
        )
        .await;
        assert_eq!(test::read_body(rsp).await.as_ref(), b"1");
    }

    #[actix_web::test]
    async fn update_on_a_loaded_session_is_persisted() {
        let backend = backend();
        let app = test::init_service(
            App::new()
                .wrap(SessionMiddleware::new(backend.clone(), Key::generate()))
                .route("/insert", web::get().to(insert_count))
                .route("/bump", web::get().to(bump_count))
                .route("/read", web::get().to(read_count)),
        )
        .await;

        let rsp =
            test::call_service(&app, test::TestRequest::get().uri("/insert").to_request()).await;
        let cookie = find_cookie(&rsp, "id").unwrap().into_owned();

        let rsp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/bump")
                .cookie(cookie.clone())
                .to_request(),
        )
        .await;
        assert_eq!(rsp.status(), StatusCode::OK);

        let rsp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/read")
                .cookie(cookie)
                .to_request(),
        )
        .await;
        assert_eq!(test::read_body(rsp).await.as_ref(), b"2");
    }
}
