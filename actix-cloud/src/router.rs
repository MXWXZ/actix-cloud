use std::{
    fmt::Debug,
    future::{ready, Ready},
    rc::Rc,
};

use actix_web::{
    body::EitherBody,
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    web::ServiceConfig,
    HttpResponse, Route,
};
use futures::future::LocalBoxFuture;

pub fn build_router<T: 'static>(router: Vec<Router<T>>) -> impl FnOnce(&mut ServiceConfig) {
    |cfg| {
        for i in router {
            if !i.path.is_empty() {
                cfg.route(
                    &i.path,
                    i.route.wrap(RouterGuard {
                        extractor: Rc::new(i.extractor),
                        checker: Rc::new(i.checker),
                    }),
                );
            }
        }
    }
}

pub struct Router<T> {
    pub path: String,
    pub route: Route,
    pub extractor: Box<dyn Fn(&mut ServiceRequest) -> T>,
    pub checker: Box<dyn Fn(T) -> bool>,
}

pub(crate) struct RouterGuard<E, C, T>
where
    E: Fn(&mut ServiceRequest) -> T + 'static,
    C: Fn(T) -> bool + 'static,
{
    extractor: Rc<E>,
    checker: Rc<C>,
}

impl<S: 'static, B, E, C, T> Transform<S, ServiceRequest> for RouterGuard<E, C, T>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = actix_web::Error>,
    S::Future: 'static,
    B: 'static + Debug,
    E: Fn(&mut ServiceRequest) -> T + 'static,
    C: Fn(T) -> bool + 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = actix_web::Error;
    type InitError = ();
    type Transform = RouterGuardMiddleware<S, E, C, T>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        let extractor = self.extractor.clone();
        let checker = self.checker.clone();
        ready(Ok(RouterGuardMiddleware {
            service: Rc::new(service),
            extractor,
            checker,
        }))
    }
}

pub(crate) struct RouterGuardMiddleware<S, E, C, T>
where
    E: Fn(&mut ServiceRequest) -> T + 'static,
    C: Fn(T) -> bool + 'static,
{
    service: Rc<S>,
    extractor: Rc<E>,
    checker: Rc<C>,
}

impl<S, B, E, C, T> Service<ServiceRequest> for RouterGuardMiddleware<S, E, C, T>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = actix_web::Error> + 'static,
    S::Future: 'static,
    B: 'static + Debug,
    E: Fn(&mut ServiceRequest) -> T + 'static,
    C: Fn(T) -> bool + 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = actix_web::Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, mut req: ServiceRequest) -> Self::Future {
        let srv = self.service.clone();
        let extractor = self.extractor.clone();
        let checker = self.checker.clone();
        Box::pin(async move {
            let perm = extractor(&mut req);
            if checker(perm) {
                Ok(srv.call(req).await?.map_into_left_body())
            } else {
                Ok(req.into_response(HttpResponse::Forbidden().finish().map_into_right_body()))
            }
        })
    }
}
