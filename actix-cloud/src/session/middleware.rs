use std::{borrow::Cow, collections::HashMap, fmt, future::Future, pin::Pin, rc::Rc};

use actix_utils::future::{ready, Ready};
use actix_web::{
    body::MessageBody,
    cookie::{time::Duration, Cookie, CookieJar, Key},
    dev::{forward_ready, ResponseHead, Service, ServiceRequest, ServiceResponse, Transform},
    http::header::{HeaderValue, SET_COOKIE},
    HttpResponse,
};

use super::{
    config::{
        self, Configuration, CookieConfiguration, CookieContentSecurity, SessionMiddlewareBuilder,
        TtlExtensionPolicy,
    },
    storage::{SessionKey, SessionStore},
    Session, SessionStatus,
};
use crate::{error, memorydb::MemoryDB, Result};

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
/// - an instance of the session storage backend you wish to use (i.e. an implementation of
///   [`SessionStore`]);
/// - a secret key, to sign or encrypt the content of client-side session cookie.
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
pub struct SessionMiddleware<M: MemoryDB> {
    storage_backend: Rc<SessionStore<M>>,
    configuration: Rc<Configuration>,
}

impl<M: MemoryDB> SessionMiddleware<M> {
    /// Use [`SessionMiddleware::new`] to initialize the session framework using the default
    /// parameters.
    ///
    /// To create a new instance of [`SessionMiddleware`] you need to provide:
    /// - an instance of the session storage backend you wish to use (i.e. an implementation of
    ///   [`SessionStore`]);
    /// - a secret key, to sign or encrypt the content of client-side session cookie.
    pub fn new(client: M, key: Key) -> Self {
        Self::builder(client, key).build()
    }

    /// A fluent API to configure [`SessionMiddleware`].
    ///
    /// It takes as input the two required inputs to create a new instance of [`SessionMiddleware`]:
    /// - an instance of the session storage backend you wish to use (i.e. an implementation of
    ///   [`SessionStore`]);
    /// - a secret key, to sign or encrypt the content of client-side session cookie.
    pub fn builder(client: M, key: Key) -> SessionMiddlewareBuilder<M> {
        SessionMiddlewareBuilder::new(client, config::default_configuration(key))
    }

    pub(crate) fn from_parts(store: SessionStore<M>, configuration: Configuration) -> Self {
        Self {
            storage_backend: Rc::new(store),
            configuration: Rc::new(configuration),
        }
    }
}

impl<S, B, M> Transform<S, ServiceRequest> for SessionMiddleware<M>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = actix_web::Error> + 'static,
    S::Future: 'static,
    B: MessageBody + 'static,
    M: MemoryDB + 'static,
{
    type Response = ServiceResponse<B>;
    type Error = actix_web::Error;
    type Transform = InnerSessionMiddleware<S, M>;
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
pub struct InnerSessionMiddleware<S, M: MemoryDB> {
    service: Rc<S>,
    configuration: Rc<Configuration>,
    storage_backend: Rc<SessionStore<M>>,
}

impl<S, B, M> Service<ServiceRequest> for InnerSessionMiddleware<S, M>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = actix_web::Error> + 'static,
    S::Future: 'static,
    M: MemoryDB + 'static,
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

            Session::set_session(&mut req, session_state);

            let mut res = service.call(req).await?;
            let (status, session_state) = Session::get_changes(&mut res);

            let mut ttl = configuration.session.state_ttl;
            let mut cookie = Cow::Borrowed(&configuration.cookie);
            if let Some(x) = session_state.get("_ttl") {
                if let Ok(x) = x.parse() {
                    ttl = Duration::seconds(x);
                    let mut tmp = cookie.into_owned();
                    tmp.max_age = Some(ttl);
                    cookie = Cow::Owned(tmp);
                }
            }

            match session_key {
                None => {
                    // we do not create an entry in the session store if there is no state attached
                    // to a fresh session
                    if !session_state.is_empty() {
                        let session_key = storage_backend
                            .save(session_state, &ttl)
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
                                .update(session_key, session_state, &ttl)
                                .await
                                .map_err(e500)?;

                            set_session_cookie(res.response_mut().head_mut(), session_key, &cookie)
                                .map_err(e500)?;
                        }

                        SessionStatus::Purged => {
                            storage_backend.delete(&session_key).await.map_err(e500)?;

                            delete_session_cookie(res.response_mut().head_mut(), &cookie)
                                .map_err(e500)?;
                        }

                        SessionStatus::Renewed => {
                            storage_backend.delete(&session_key).await.map_err(e500)?;

                            let session_key = storage_backend
                                .save(session_state, &ttl)
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
                                    .update_ttl(&session_key, &ttl)
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
    let cookies = req.cookies().ok()?;
    let session_cookie = cookies
        .iter()
        .find(|&cookie| cookie.name() == config.name)?;

    let mut jar = CookieJar::new();
    jar.add_original(session_cookie.clone());

    let verification_result = match config.content_security {
        CookieContentSecurity::Signed => jar.signed(&config.key).get(&config.name),
        CookieContentSecurity::Private => jar.private(&config.key).get(&config.name),
    };

    verification_result?.value().to_owned().try_into().ok()
}

async fn load_session_state<M: MemoryDB>(
    session_key: Option<SessionKey>,
    storage_backend: &SessionStore<M>,
) -> Result<(Option<SessionKey>, HashMap<String, String>), actix_web::Error> {
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

                    Ok((None, HashMap::new()))
                }
            }

            Err(err) => Err(e500(err)),
        }
    } else {
        Ok((None, HashMap::new()))
    }
}

fn set_session_cookie(
    response: &mut ResponseHead,
    session_key: SessionKey,
    config: &CookieConfiguration,
) -> Result<()> {
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
    let val = HeaderValue::from_str(&cookie.encoded().to_string())
        .map_err(|e| error::Error::Session(e.to_string()))?;

    response.headers_mut().append(SET_COOKIE, val);

    Ok(())
}

fn delete_session_cookie(response: &mut ResponseHead, config: &CookieConfiguration) -> Result<()> {
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

    let val = HeaderValue::from_str(&removal_cookie.to_string())
        .map_err(|e| error::Error::Session(e.to_string()))?;
    response.headers_mut().append(SET_COOKIE, val);

    Ok(())
}
