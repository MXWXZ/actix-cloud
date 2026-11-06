//! Declarative router with per-route auth and CSRF configuration.
//!
//! Build a `Vec<Router>` and pass it to [`build_router`]; the returned closure is
//! configured on a scope (`scope("/api").configure(build_router(...))`). Each route
//! runs the [`Checker`] auth guard first, then CSRF (feature `csrf`).
use std::{
    fmt::Debug,
    future::{ready, Ready},
    rc::Rc,
};

#[cfg(feature = "csrf")]
use actix_web::HttpMessage;
use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    web,
    web::ServiceConfig,
    Route,
};
use anyhow::Result;
use async_trait::async_trait;
use futures::future::LocalBoxFuture;

#[cfg(feature = "csrf")]
/// Configure routes on a scope, wrapping each route with the CSRF middleware
/// (innermost) and the auth guard.
pub fn build_router<F, Fut>(
    router: Vec<Router>,
    csrf: crate::csrf::Middleware<F>,
) -> impl FnOnce(&mut ServiceConfig)
where
    F: Fn(actix_web::HttpRequest, String) -> Fut + 'static,
    Fut: futures::Future<Output = Result<bool, actix_web::Error>>,
{
    move |cfg| {
        configure_routes(
            cfg,
            router
                .into_iter()
                .filter(|i| !i.path.is_empty())
                .map(|i| {
                    let route = i.route.wrap(csrf.clone()).wrap(RouterGuard {
                        checker: i.checker,
                        csrf: i.csrf,
                    });
                    (i.path, route)
                })
                .collect(),
        );
    }
}

#[cfg(not(feature = "csrf"))]
/// Configure routes on a scope, wrapping each route with the auth guard.
pub fn build_router(router: Vec<Router>) -> impl FnOnce(&mut ServiceConfig) {
    |cfg| {
        configure_routes(
            cfg,
            router
                .into_iter()
                .filter(|i| !i.path.is_empty())
                .map(|i| {
                    let route = i.route.wrap(RouterGuard { checker: i.checker });
                    (i.path, route)
                })
                .collect(),
        );
    }
}

/// Register all routes for the same path on one [`actix_web::Resource`].
///
/// [`ServiceConfig::route`] moves a route's guards (including its method guard) up to
/// the resource, so registering each route separately makes a wrong-method request
/// miss the resource entirely and return 404. Keeping the routes on a single resource
/// lets it match on path only and fall back to 405 when no route's method matches.
fn configure_routes(cfg: &mut ServiceConfig, routes: Vec<(String, Route)>) {
    let mut resources: Vec<(String, Vec<Route>)> = Vec::new();
    for (path, route) in routes {
        if let Some((_, routes)) = resources.iter_mut().find(|(p, _)| *p == path) {
            routes.push(route);
        } else {
            resources.push((path, vec![route]));
        }
    }
    for (path, routes) in resources {
        cfg.service(
            routes
                .into_iter()
                .fold(web::resource(path), |resource, route| resource.route(route)),
        );
    }
}

/// Per-route permission checker.
///
/// Return `Ok(())` to allow the request, or an `Err` (e.g. `ErrorUnauthorized` /
/// `ErrorForbidden`) to reject it with the error's status code. `?Send` is required
/// because actix runs each worker single-threaded.
#[async_trait(?Send)]
pub trait Checker {
    async fn check(&self, req: &mut ServiceRequest) -> Result<(), actix_web::Error>;
}

#[cfg(feature = "csrf")]
/// Per-route CSRF enforcement level (see [`crate::csrf`]).
///
/// By default checks apply to all unsafe methods; `ForceHeader`/`ForceParam` check on
/// every method, and `Disabled` never checks.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, enum_as_inner::EnumAsInner)]
pub enum CSRFType {
    Header,
    Param,
    ForceHeader,
    ForceParam,
    Disabled,
}

/// A route entry for [`build_router`].
pub struct Router {
    /// Route path registered on the scope.
    pub path: String,
    /// Actix-web route (method + handler).
    pub route: Route,
    /// Auth checker; `None` skips the auth check for this route.
    pub checker: Option<Rc<dyn Checker>>,
    #[cfg(feature = "csrf")]
    /// CSRF enforcement level for this route.
    pub csrf: CSRFType,
}

pub(crate) struct RouterGuard {
    checker: Option<Rc<dyn Checker>>,
    #[cfg(feature = "csrf")]
    csrf: CSRFType,
}

impl<S, B> Transform<S, ServiceRequest> for RouterGuard
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = actix_web::Error> + 'static,
    S::Future: 'static,
    B: 'static + Debug,
{
    type Response = ServiceResponse<B>;
    type Error = actix_web::Error;
    type InitError = ();
    type Transform = RouterGuardMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(RouterGuardMiddleware {
            service: Rc::new(service),
            checker: self.checker.clone(),
            #[cfg(feature = "csrf")]
            csrf: self.csrf,
        }))
    }
}

pub(crate) struct RouterGuardMiddleware<S> {
    service: Rc<S>,
    checker: Option<Rc<dyn Checker>>,
    #[cfg(feature = "csrf")]
    csrf: CSRFType,
}

impl<S, B> Service<ServiceRequest> for RouterGuardMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = actix_web::Error> + 'static,
    S::Future: 'static,
    B: 'static + Debug,
{
    type Response = ServiceResponse<B>;
    type Error = actix_web::Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, mut req: ServiceRequest) -> Self::Future {
        let srv = self.service.clone();
        let checker = self.checker.clone();
        #[cfg(feature = "csrf")]
        req.extensions_mut().insert(self.csrf);
        Box::pin(async move {
            if let Some(checker) = checker {
                checker.check(&mut req).await?;
            }
            srv.call(req).await
        })
    }
}

#[cfg(test)]
mod tests {
    use actix_web::{
        http::{header, StatusCode},
        test, web, App, HttpResponse,
    };

    use super::*;

    fn router(path: &str, route: Route) -> Router {
        Router {
            path: path.to_owned(),
            route,
            checker: None,
            #[cfg(feature = "csrf")]
            csrf: CSRFType::Disabled,
        }
    }

    async fn call(routers: Vec<Router>, req: test::TestRequest) -> ServiceResponse {
        #[cfg(not(feature = "csrf"))]
        let app = App::new().service(web::scope("/api").configure(build_router(routers)));
        #[cfg(feature = "csrf")]
        let app = App::new().service(web::scope("/api").configure(build_router(
            routers,
            crate::csrf::Middleware::new("_csrf".to_owned(), "x-csrf".to_owned(), |_, _| async {
                Ok::<_, actix_web::Error>(true)
            }),
        )));
        let app = test::init_service(app).await;
        test::call_service(&app, req.to_request()).await
    }

    fn get_and_post() -> Vec<Router> {
        vec![
            router("/hello", web::get().to(|| async { HttpResponse::Ok() })),
            router(
                "/hello",
                web::post().to(|| async { HttpResponse::Created() }),
            ),
        ]
    }

    #[actix_web::test]
    async fn wrong_method_returns_405() {
        let routers = vec![router(
            "/hello",
            web::post().to(|| async { HttpResponse::Ok() }),
        )];
        let resp = call(routers, test::TestRequest::get().uri("/api/hello")).await;
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(
            resp.headers()
                .get(header::ALLOW)
                .and_then(|v| v.to_str().ok()),
            Some("POST")
        );
    }

    #[actix_web::test]
    async fn all_methods_on_same_path_are_registered() {
        let resp = call(get_and_post(), test::TestRequest::get().uri("/api/hello")).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = call(get_and_post(), test::TestRequest::post().uri("/api/hello")).await;
        assert_eq!(resp.status(), StatusCode::CREATED);

        let resp = call(get_and_post(), test::TestRequest::put().uri("/api/hello")).await;
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(
            resp.headers()
                .get(header::ALLOW)
                .and_then(|v| v.to_str().ok()),
            Some("GET, POST")
        );
    }
}
