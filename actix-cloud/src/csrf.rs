use std::{future::Future, rc::Rc};

use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    HttpMessage, HttpRequest,
};
use futures::future::{ready, LocalBoxFuture, Ready};
use qstring::QString;

use crate::router::CSRFType;

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
    fn get_safe_header(req: &ServiceRequest, name: &str) -> Option<String> {
        let mut ret: Vec<&str> = req
            .headers()
            .get_all(name)
            .map(|x| x.to_str().unwrap())
            .collect();
        if ret.len() != 1 {
            return None;
        }
        ret.pop().map(ToOwned::to_owned)
    }

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
            let csrf = req.extensions().get::<CSRFType>().unwrap().to_owned();
            if csrf.is_force_header() || csrf.is_force_param() || !req.method().is_safe() {
                let ret = match csrf {
                    CSRFType::Header => {
                        Self::check_csrf(&req, &cookie, &header, checker, false).await
                    }
                    CSRFType::Param => {
                        Self::check_csrf(&req, &cookie, &header, checker, true).await
                    }
                    CSRFType::ForceHeader => {
                        Self::check_csrf(&req, &cookie, &header, checker, false).await
                    }
                    CSRFType::ForceParam => {
                        Self::check_csrf(&req, &cookie, &header, checker, true).await
                    }
                    CSRFType::Disabled => Ok(true),
                }?;
                if !ret {
                    return Err(actix_web::error::ErrorBadRequest("CSRF check failed"));
                }
            }
            srv.call(req).await
        })
    }
}
