//! CSRF protection based on the [double submit cookie] pattern.
//!
//! The token is stored in a cookie and must be echoed in a header (or a query
//! parameter, generally only for websocket). Checks are applied per route through
//! [`crate::build_router`], and run **after** the auth checker.
//!
//! This middleware performs the transport-level double-submit check only. Whether a
//! matching token is actually valid for the user (for example, bound to the session)
//! is application-specific: implement that in the `checker` callback. Duplicated or
//! empty cookies, headers and query parameters are rejected, and tokens are compared
//! in constant time.
//!
//! `Param`/`ForceParam` put the token in the URL, where it can leak through
//! `Referer`, logs and browser history; prefer `Header` except for WebSocket
//! handshakes. A `__Host-` cookie prefix is recommended, and tokens should be rotated
//! after authentication or privilege changes.
//!
//! [double submit cookie]: https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html#alternative-using-a-double-submit-cookie-pattern
use std::{future::Future, rc::Rc};

use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    HttpMessage, HttpRequest,
};
use futures::future::{ready, LocalBoxFuture, Ready};
use qstring::QString;
use subtle::ConstantTimeEq as _;

use crate::router::CSRFType;

/// Outcome of reading the CSRF header.
enum HeaderToken {
    /// The header is absent.
    Missing,
    /// The header is present exactly once with an empty value; a query parameter
    /// may still be used in `Param`/`ForceParam` mode.
    Empty,
    /// The header is present more than once or is not valid UTF-8.
    Invalid,
    /// The header is present exactly once with a non-empty value.
    Value(String),
}

/// CSRF middleware, configured through [`crate::build_router`].
///
/// `cookie` is the cookie holding the token, `header` is the header (or query
/// parameter) expected to carry the same token, and `checker` is an extra validation
/// callback invoked with the request and the token after both values match.
pub struct Middleware<F> {
    cookie: Rc<String>,
    header: Rc<String>,
    checker: Rc<F>,
}

impl<F> Clone for Middleware<F> {
    fn clone(&self) -> Self {
        Self {
            cookie: self.cookie.clone(),
            header: self.header.clone(),
            checker: self.checker.clone(),
        }
    }
}

impl<F, Fut> Middleware<F>
where
    F: Fn(HttpRequest, String) -> Fut,
    Fut: Future<Output = Result<bool, actix_web::Error>>,
{
    pub fn new(cookie: String, header: String, checker: F) -> Self {
        Self {
            cookie: Rc::new(cookie),
            header: Rc::new(header),
            checker: Rc::new(checker),
        }
    }
}

impl<S, B, F, Fut> Transform<S, ServiceRequest> for Middleware<F>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = actix_web::Error> + 'static,
    S::Future: 'static,
    B: 'static,
    F: Fn(HttpRequest, String) -> Fut + 'static,
    Fut: Future<Output = Result<bool, actix_web::Error>>,
{
    type Response = ServiceResponse<B>;
    type Error = actix_web::Error;
    type InitError = ();
    type Transform = MiddlewareService<S, F>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(MiddlewareService {
            service: Rc::new(service),
            cookie: self.cookie.clone(),
            header: self.header.clone(),
            checker: self.checker.clone(),
        }))
    }
}

pub struct MiddlewareService<S, F> {
    service: Rc<S>,
    cookie: Rc<String>,
    header: Rc<String>,
    checker: Rc<F>,
}

impl<S, B, F, Fut> MiddlewareService<S, F>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = actix_web::Error> + 'static,
    S::Future: 'static,
    B: 'static,
    F: Fn(HttpRequest, String) -> Fut + 'static,
    Fut: Future<Output = Result<bool, actix_web::Error>>,
{
    /// Read the CSRF header, rejecting duplicated or malformed values.
    fn get_safe_header(req: &ServiceRequest, name: &str) -> HeaderToken {
        let mut values = req.headers().get_all(name);
        let Some(value) = values.next() else {
            return HeaderToken::Missing;
        };
        if values.next().is_some() {
            return HeaderToken::Invalid;
        }
        let Ok(value) = value.to_str() else {
            return HeaderToken::Invalid;
        };
        if value.is_empty() {
            HeaderToken::Empty
        } else {
            HeaderToken::Value(value.to_owned())
        }
    }

    /// Read a single, non-empty cookie value; `None` when the cookie is missing,
    /// empty, duplicated, or the cookie headers cannot be parsed.
    fn get_safe_cookie(req: &ServiceRequest, name: &str) -> Option<String> {
        let cookie = crate::cookie::unique_cookie(req, name)?;
        if cookie.value().is_empty() {
            return None;
        }
        Some(cookie.value().to_owned())
    }

    /// Read a single, non-empty query parameter. Duplicates are rejected instead of
    /// silently picking the first one.
    fn get_safe_param(req: &ServiceRequest, name: &str) -> Option<String> {
        let qs = QString::from(req.query_string());
        let mut values = qs
            .to_pairs()
            .into_iter()
            .filter(|(key, _)| *key == name)
            .map(|(_, value)| value);
        let value = values.next()?;
        if values.next().is_some() || value.is_empty() {
            return None;
        }
        Some(value.to_owned())
    }

    /// Double-submit check: cookie and header (or query parameter) must both be
    /// present, equal, and accepted by the extra `checker` callback.
    ///
    /// The checker is the application's hook for deciding whether a matching token is
    /// actually valid (e.g. bound to the session); this middleware only enforces the
    /// transport-level double-submit rule.
    async fn check_csrf(
        req: &ServiceRequest,
        cookie: &str,
        header: &str,
        checker: Rc<F>,
        allow_param: bool,
    ) -> Result<bool, actix_web::Error> {
        let Some(cookie) = Self::get_safe_cookie(req, cookie) else {
            return Ok(false);
        };
        let csrf = match Self::get_safe_header(req, header) {
            HeaderToken::Value(value) => Some(value),
            // A duplicated or malformed header must not silently fall back to the
            // query parameter.
            HeaderToken::Invalid => return Ok(false),
            HeaderToken::Missing | HeaderToken::Empty if allow_param => {
                Self::get_safe_param(req, header)
            }
            HeaderToken::Missing | HeaderToken::Empty => None,
        };
        let Some(csrf) = csrf else {
            return Ok(false);
        };
        if !bool::from(csrf.as_bytes().ct_eq(cookie.as_bytes())) {
            return Ok(false);
        }
        checker(req.request().clone(), csrf).await
    }
}

impl<S, B, F, Fut> Service<ServiceRequest> for MiddlewareService<S, F>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = actix_web::Error> + 'static,
    S::Future: 'static,
    B: 'static,
    F: Fn(HttpRequest, String) -> Fut + 'static,
    Fut: Future<Output = Result<bool, actix_web::Error>>,
{
    type Response = ServiceResponse<B>;
    type Error = actix_web::Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let srv = self.service.clone();
        let header = self.header.clone();
        let cookie = self.cookie.clone();
        let checker = self.checker.clone();
        Box::pin(async move {
            // `CSRFType` is inserted per-route by the router guard; the middleware must
            // be used through `build_router`. Refuse the request instead of panicking
            // when it is mounted directly.
            let Some(csrf) = req.extensions().get::<CSRFType>().copied() else {
                return Err(actix_web::error::ErrorInternalServerError(
                    "CSRF middleware must be used through build_router",
                ));
            };
            let enforce = !csrf.is_disabled()
                && (csrf.is_force_header() || csrf.is_force_param() || !req.method().is_safe());
            if enforce {
                // `Param`/`ForceParam` also accept the token as a query parameter,
                // generally only used for websocket.
                let allow_param = csrf.is_param() || csrf.is_force_param();
                if !Self::check_csrf(&req, &cookie, &header, checker, allow_param).await? {
                    return Err(actix_web::error::ErrorBadRequest("CSRF check failed"));
                }
            }
            srv.call(req).await
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Mutex,
        },
    };

    use actix_web::{
        cookie::Cookie,
        http::{Method, StatusCode},
        web, App, HttpResponse,
    };

    use super::*;
    use crate::router::{build_router, Router};

    fn router(method: Method, csrf: CSRFType) -> Router {
        Router {
            path: "/test".to_owned(),
            route: web::route()
                .method(method)
                .to(|| async { HttpResponse::Ok().finish() }),
            checker: None,
            csrf,
        }
    }

    fn get_and_post(csrf: CSRFType) -> Vec<Router> {
        vec![router(Method::GET, csrf), router(Method::POST, csrf)]
    }

    fn checked() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    fn cookie() -> Cookie<'static> {
        Cookie::new("_csrf", "token")
    }

    /// Run `req` through a scope configured with `build_router` and a CSRF middleware
    /// whose extra checker records the call into `checked` and accepts the `token`
    /// value only when `checker_ok` is set. Returns the response status code; errors
    /// raised by the middleware are rendered into their HTTP status.
    async fn call(
        routers: Vec<Router>,
        checker_ok: bool,
        checked: Arc<AtomicBool>,
        req: actix_web::test::TestRequest,
    ) -> StatusCode {
        let mw = Middleware::new("_csrf".to_owned(), "x-csrf".to_owned(), move |_, token| {
            checked.store(true, Ordering::SeqCst);
            future::ready(Ok::<_, actix_web::Error>(checker_ok && token == "token"))
        });
        let app = actix_web::test::init_service(
            App::new().service(web::scope("/api").configure(build_router(routers, mw))),
        )
        .await;
        match actix_web::test::try_call_service(&app, req.to_request()).await {
            Ok(rsp) => rsp.status(),
            Err(err) => err.as_response_error().error_response().status(),
        }
    }

    #[actix_web::test]
    async fn unsafe_method_without_token_is_rejected() {
        let flag = checked();
        let rsp = call(
            get_and_post(CSRFType::Header),
            true,
            flag.clone(),
            actix_web::test::TestRequest::post().uri("/api/test"),
        )
        .await;
        assert_eq!(rsp, StatusCode::BAD_REQUEST);
        // The checker is not even invoked when cookie/header are missing.
        assert!(!flag.load(Ordering::SeqCst));
    }

    #[actix_web::test]
    async fn matching_token_passes() {
        let flag = checked();
        let rsp = call(
            get_and_post(CSRFType::Header),
            true,
            flag.clone(),
            actix_web::test::TestRequest::post()
                .uri("/api/test")
                .cookie(cookie())
                .insert_header(("x-csrf", "token")),
        )
        .await;
        assert_eq!(rsp, StatusCode::OK);
        assert!(flag.load(Ordering::SeqCst));
    }

    #[actix_web::test]
    async fn mismatched_token_is_rejected() {
        let flag = checked();
        let rsp = call(
            get_and_post(CSRFType::Header),
            true,
            flag.clone(),
            actix_web::test::TestRequest::post()
                .uri("/api/test")
                .cookie(cookie())
                .insert_header(("x-csrf", "other")),
        )
        .await;
        assert_eq!(rsp, StatusCode::BAD_REQUEST);
        assert!(!flag.load(Ordering::SeqCst));
    }

    #[actix_web::test]
    async fn duplicated_header_is_rejected() {
        let flag = checked();
        let rsp = call(
            get_and_post(CSRFType::Header),
            true,
            flag.clone(),
            actix_web::test::TestRequest::post()
                .uri("/api/test")
                .cookie(cookie())
                .append_header(("x-csrf", "token"))
                .append_header(("x-csrf", "token")),
        )
        .await;
        assert_eq!(rsp, StatusCode::BAD_REQUEST);
        assert!(!flag.load(Ordering::SeqCst));
    }

    #[actix_web::test]
    async fn checker_can_reject_valid_token() {
        let flag = checked();
        let rsp = call(
            get_and_post(CSRFType::Header),
            false,
            flag.clone(),
            actix_web::test::TestRequest::post()
                .uri("/api/test")
                .cookie(cookie())
                .insert_header(("x-csrf", "token")),
        )
        .await;
        assert_eq!(rsp, StatusCode::BAD_REQUEST);
        assert!(flag.load(Ordering::SeqCst));
    }

    #[actix_web::test]
    async fn safe_method_is_not_checked() {
        let flag = checked();
        let rsp = call(
            get_and_post(CSRFType::Header),
            true,
            flag.clone(),
            actix_web::test::TestRequest::get().uri("/api/test"),
        )
        .await;
        assert_eq!(rsp, StatusCode::OK);
        assert!(!flag.load(Ordering::SeqCst));
    }

    #[actix_web::test]
    async fn force_header_checks_safe_methods() {
        let flag = checked();
        let rsp = call(
            get_and_post(CSRFType::ForceHeader),
            true,
            flag.clone(),
            actix_web::test::TestRequest::get().uri("/api/test"),
        )
        .await;
        assert_eq!(rsp, StatusCode::BAD_REQUEST);
    }

    #[actix_web::test]
    async fn param_mode_accepts_query_token() {
        let flag = checked();
        let rsp = call(
            get_and_post(CSRFType::Param),
            true,
            flag.clone(),
            actix_web::test::TestRequest::post()
                .uri("/api/test?x-csrf=token")
                .cookie(cookie()),
        )
        .await;
        assert_eq!(rsp, StatusCode::OK);
        assert!(flag.load(Ordering::SeqCst));
    }

    #[actix_web::test]
    async fn disabled_never_checks() {
        let flag = checked();
        let rsp = call(
            get_and_post(CSRFType::Disabled),
            true,
            flag.clone(),
            actix_web::test::TestRequest::post().uri("/api/test"),
        )
        .await;
        assert_eq!(rsp, StatusCode::OK);
        assert!(!flag.load(Ordering::SeqCst));
    }

    #[actix_web::test]
    async fn force_param_checks_safe_methods() {
        let flag = checked();
        let rsp = call(
            get_and_post(CSRFType::ForceParam),
            true,
            flag.clone(),
            actix_web::test::TestRequest::get()
                .uri("/api/test?x-csrf=token")
                .cookie(cookie()),
        )
        .await;
        assert_eq!(rsp, StatusCode::OK);
        assert!(flag.load(Ordering::SeqCst));
    }

    #[actix_web::test]
    async fn param_mode_still_requires_the_cookie() {
        let flag = checked();
        let rsp = call(
            get_and_post(CSRFType::Param),
            true,
            flag.clone(),
            actix_web::test::TestRequest::post().uri("/api/test?x-csrf=token"),
        )
        .await;
        assert_eq!(rsp, StatusCode::BAD_REQUEST);
        assert!(!flag.load(Ordering::SeqCst));
    }

    #[actix_web::test]
    async fn param_mode_prefers_a_present_header_over_the_query() {
        let flag = checked();
        let rsp = call(
            get_and_post(CSRFType::Param),
            true,
            flag.clone(),
            actix_web::test::TestRequest::post()
                .uri("/api/test?x-csrf=token")
                .cookie(cookie())
                .insert_header(("x-csrf", "other")),
        )
        .await;
        // The mismatching header wins; the query parameter is not a fallback.
        assert_eq!(rsp, StatusCode::BAD_REQUEST);
        assert!(!flag.load(Ordering::SeqCst));
    }

    #[actix_web::test]
    async fn empty_header_falls_back_to_the_query() {
        let flag = checked();
        let rsp = call(
            get_and_post(CSRFType::Param),
            true,
            flag.clone(),
            actix_web::test::TestRequest::post()
                .uri("/api/test?x-csrf=token")
                .cookie(cookie())
                .insert_header(("x-csrf", "")),
        )
        .await;
        assert_eq!(rsp, StatusCode::OK);
        assert!(flag.load(Ordering::SeqCst));
    }

    #[actix_web::test]
    async fn empty_header_is_rejected_in_header_mode() {
        let flag = checked();
        let rsp = call(
            get_and_post(CSRFType::Header),
            true,
            flag.clone(),
            actix_web::test::TestRequest::post()
                .uri("/api/test")
                .cookie(cookie())
                .insert_header(("x-csrf", "")),
        )
        .await;
        assert_eq!(rsp, StatusCode::BAD_REQUEST);
        assert!(!flag.load(Ordering::SeqCst));
    }

    #[actix_web::test]
    async fn middleware_without_router_returns_500() {
        // Mounted directly (not through `build_router`), there is no `CSRFType`
        // extension; the request must fail with 500 instead of panicking.
        let mw = Middleware::new("_csrf".to_owned(), "x-csrf".to_owned(), |_, _| {
            future::ready(Ok::<_, actix_web::Error>(true))
        });
        let app = actix_web::test::init_service(App::new().wrap(mw).route(
            "/test",
            web::get().to(|| async { HttpResponse::Ok().finish() }),
        ))
        .await;

        let result = actix_web::test::try_call_service(
            &app,
            actix_web::test::TestRequest::get()
                .uri("/test")
                .to_request(),
        )
        .await;
        let status = match result {
            Ok(rsp) => rsp.status(),
            Err(err) => err.as_response_error().error_response().status(),
        };

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[actix_web::test]
    async fn checker_errors_are_propagated() {
        let mw = Middleware::new("_csrf".to_owned(), "x-csrf".to_owned(), |_, _| async {
            Err::<bool, _>(actix_web::error::ErrorInternalServerError("checker failed"))
        });
        let app = actix_web::test::init_service(App::new().service(
            web::scope("/api").configure(build_router(get_and_post(CSRFType::Header), mw)),
        ))
        .await;

        let result = actix_web::test::try_call_service(
            &app,
            actix_web::test::TestRequest::post()
                .uri("/api/test")
                .cookie(cookie())
                .insert_header(("x-csrf", "token"))
                .to_request(),
        )
        .await;
        let status = match result {
            Ok(rsp) => rsp.status(),
            Err(err) => err.as_response_error().error_response().status(),
        };

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    // Security edge cases.

    #[actix_web::test]
    async fn header_without_cookie_is_rejected() {
        let flag = checked();
        let rsp = call(
            get_and_post(CSRFType::Header),
            true,
            flag.clone(),
            actix_web::test::TestRequest::post()
                .uri("/api/test")
                .insert_header(("x-csrf", "token")),
        )
        .await;
        assert_eq!(rsp, StatusCode::BAD_REQUEST);
        assert!(!flag.load(Ordering::SeqCst));
    }

    #[actix_web::test]
    async fn cookie_without_header_is_rejected() {
        let flag = checked();
        let rsp = call(
            get_and_post(CSRFType::Header),
            true,
            flag.clone(),
            actix_web::test::TestRequest::post()
                .uri("/api/test")
                .cookie(cookie()),
        )
        .await;
        assert_eq!(rsp, StatusCode::BAD_REQUEST);
        assert!(!flag.load(Ordering::SeqCst));
    }

    #[actix_web::test]
    async fn token_comparison_is_case_sensitive_and_exact() {
        for token in ["Token", "TOKEN", "tokenx", "xtoken"] {
            let flag = checked();
            let rsp = call(
                get_and_post(CSRFType::Header),
                true,
                flag.clone(),
                actix_web::test::TestRequest::post()
                    .uri("/api/test")
                    .cookie(cookie())
                    .insert_header(("x-csrf", token)),
            )
            .await;
            assert_eq!(rsp, StatusCode::BAD_REQUEST, "token {token:?}");
            assert!(!flag.load(Ordering::SeqCst), "token {token:?}");
        }
    }

    #[actix_web::test]
    async fn header_name_matching_is_case_insensitive() {
        let flag = checked();
        let rsp = call(
            get_and_post(CSRFType::Header),
            true,
            flag.clone(),
            actix_web::test::TestRequest::post()
                .uri("/api/test")
                .cookie(cookie())
                .insert_header(("X-CSRF", "token")),
        )
        .await;
        assert_eq!(rsp, StatusCode::OK);
        assert!(flag.load(Ordering::SeqCst));
    }

    #[actix_web::test]
    async fn force_header_accepts_a_valid_token_on_safe_methods() {
        let flag = checked();
        let rsp = call(
            get_and_post(CSRFType::ForceHeader),
            true,
            flag.clone(),
            actix_web::test::TestRequest::get()
                .uri("/api/test")
                .cookie(cookie())
                .insert_header(("x-csrf", "token")),
        )
        .await;
        assert_eq!(rsp, StatusCode::OK);
        assert!(flag.load(Ordering::SeqCst));
    }

    #[actix_web::test]
    async fn param_mode_does_not_check_safe_methods() {
        let flag = checked();
        let rsp = call(
            get_and_post(CSRFType::Param),
            true,
            flag.clone(),
            actix_web::test::TestRequest::get().uri("/api/test"),
        )
        .await;
        assert_eq!(rsp, StatusCode::OK);
        assert!(!flag.load(Ordering::SeqCst));
    }

    #[actix_web::test]
    async fn custom_cookie_and_header_names_are_used() {
        let mw = Middleware::new(
            "csrf_token".to_owned(),
            "x-xsrf-token".to_owned(),
            |_, _| future::ready(Ok::<_, actix_web::Error>(true)),
        );
        let app = actix_web::test::init_service(App::new().service(
            web::scope("/api").configure(build_router(get_and_post(CSRFType::Header), mw)),
        ))
        .await;

        let custom = actix_web::test::TestRequest::post()
            .uri("/api/test")
            .cookie(Cookie::new("csrf_token", "token"))
            .insert_header(("x-xsrf-token", "token"))
            .to_request();
        // Render middleware rejections as their HTTP status instead of panicking.
        let status = match actix_web::test::try_call_service(&app, custom).await {
            Ok(rsp) => rsp.status(),
            Err(err) => err.as_response_error().error_response().status(),
        };
        assert_eq!(status, StatusCode::OK);

        // The default names only work when the middleware is configured with them.
        let default = actix_web::test::TestRequest::post()
            .uri("/api/test")
            .cookie(cookie())
            .insert_header(("x-csrf", "token"))
            .to_request();
        let status = match actix_web::test::try_call_service(&app, default).await {
            Ok(rsp) => rsp.status(),
            Err(err) => err.as_response_error().error_response().status(),
        };
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[actix_web::test]
    async fn checker_receives_the_verified_token() {
        let seen = Arc::new(Mutex::new(None));
        let seen_clone = seen.clone();
        let mw = Middleware::new("_csrf".to_owned(), "x-csrf".to_owned(), move |_, token| {
            *seen_clone.lock().unwrap() = Some(token);
            future::ready(Ok::<_, actix_web::Error>(true))
        });
        let app = actix_web::test::init_service(App::new().service(
            web::scope("/api").configure(build_router(get_and_post(CSRFType::Header), mw)),
        ))
        .await;

        let rsp = actix_web::test::call_service(
            &app,
            actix_web::test::TestRequest::post()
                .uri("/api/test")
                .cookie(cookie())
                .insert_header(("x-csrf", "token"))
                .to_request(),
        )
        .await;

        assert_eq!(rsp.status(), StatusCode::OK);
        assert_eq!(seen.lock().unwrap().as_deref(), Some("token"));
    }

    // Input hardening.

    #[actix_web::test]
    async fn duplicate_cookies_are_rejected() {
        let flag = checked();
        // `TestRequest::cookie` deduplicates through a cookie jar, so send the raw
        // header to exercise cookie shadowing.
        let rsp = call(
            get_and_post(CSRFType::Header),
            true,
            flag.clone(),
            actix_web::test::TestRequest::post()
                .uri("/api/test")
                .insert_header(("Cookie", "_csrf=token; _csrf=token"))
                .insert_header(("x-csrf", "token")),
        )
        .await;
        assert_eq!(rsp, StatusCode::BAD_REQUEST);
        assert!(!flag.load(Ordering::SeqCst));
    }

    #[actix_web::test]
    async fn duplicate_query_parameters_are_rejected() {
        let flag = checked();
        let rsp = call(
            get_and_post(CSRFType::Param),
            true,
            flag.clone(),
            actix_web::test::TestRequest::post()
                .uri("/api/test?x-csrf=token&x-csrf=token")
                .cookie(cookie()),
        )
        .await;
        assert_eq!(rsp, StatusCode::BAD_REQUEST);
        assert!(!flag.load(Ordering::SeqCst));
    }

    #[actix_web::test]
    async fn empty_cookie_is_rejected() {
        // An empty token is not a token, even when both sides match.
        for csrf in [CSRFType::Header, CSRFType::Param] {
            let flag = checked();
            let rsp = call(
                get_and_post(csrf),
                true,
                flag.clone(),
                actix_web::test::TestRequest::post()
                    .uri("/api/test")
                    .cookie(Cookie::new("_csrf", ""))
                    .insert_header(("x-csrf", "")),
            )
            .await;
            assert_eq!(rsp, StatusCode::BAD_REQUEST);
            assert!(!flag.load(Ordering::SeqCst));
        }
    }

    #[actix_web::test]
    async fn empty_query_token_is_rejected() {
        let flag = checked();
        let rsp = call(
            get_and_post(CSRFType::Param),
            true,
            flag.clone(),
            actix_web::test::TestRequest::post()
                .uri("/api/test?x-csrf=")
                .cookie(cookie()),
        )
        .await;
        assert_eq!(rsp, StatusCode::BAD_REQUEST);
        assert!(!flag.load(Ordering::SeqCst));
    }

    #[actix_web::test]
    async fn mixed_empty_and_valid_duplicate_header_is_rejected() {
        // Duplicated headers are rejected in every mode; the query must not be used
        // as a fallback.
        let flag = checked();
        let rsp = call(
            get_and_post(CSRFType::Param),
            true,
            flag.clone(),
            actix_web::test::TestRequest::post()
                .uri("/api/test?x-csrf=token")
                .cookie(cookie())
                .append_header(("x-csrf", ""))
                .append_header(("x-csrf", "token")),
        )
        .await;
        assert_eq!(rsp, StatusCode::BAD_REQUEST);
        assert!(!flag.load(Ordering::SeqCst));
    }
}
