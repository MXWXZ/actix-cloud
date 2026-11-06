//! CSRF protection based on the [double submit cookie] pattern.
//!
//! The token is stored in a cookie and must be echoed in a header (or a query
//! parameter, generally only for websocket). Checks are applied per route through
//! [`crate::build_router`], and run **after** the auth checker.
//!
//! [double submit cookie]: https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html#alternative-using-a-double-submit-cookie-pattern
use std::{future::Future, rc::Rc};

use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    HttpMessage, HttpRequest,
};
use futures::future::{ready, LocalBoxFuture, Ready};
use qstring::QString;

use crate::router::CSRFType;

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
    /// Read a single, non-empty header value; `None` when the header is missing,
    /// empty, duplicated (to avoid smuggling), or not valid unicode.
    fn get_safe_header(req: &ServiceRequest, name: &str) -> Option<String> {
        let mut values = req
            .headers()
            .get_all(name)
            .map(|x| x.to_str().unwrap_or_default())
            .filter(|x| !x.is_empty());
        let ret = values.next()?;
        // Reject duplicated header values to avoid smuggling.
        if values.next().is_some() {
            return None;
        }
        Some(ret.to_owned())
    }

    /// Double-submit check: cookie and header (or query parameter) must both be
    /// present, equal, and accepted by the extra `checker` callback.
    async fn check_csrf(
        req: &ServiceRequest,
        cookie: &str,
        header: &str,
        checker: Rc<F>,
        allow_param: bool,
    ) -> Result<bool, actix_web::Error> {
        let Some(cookie) = req.cookie(cookie) else {
            return Ok(false);
        };
        let mut csrf = Self::get_safe_header(req, header);
        if csrf.is_none() && allow_param {
            let qs = QString::from(req.query_string());
            csrf = qs.get(header).map(ToOwned::to_owned);
        }
        let Some(csrf) = csrf else {
            return Ok(false);
        };
        if csrf != cookie.value() {
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
            // be used through `build_router`.
            let csrf = req.extensions().get::<CSRFType>().unwrap().to_owned();
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
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
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
            std::future::ready(Ok::<_, actix_web::Error>(checker_ok && token == "token"))
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
}
