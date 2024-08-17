use std::{net::SocketAddr, rc::Rc};

use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    HttpMessage as _,
};
use chrono::{DateTime, Utc};
use futures::future::{ready, LocalBoxFuture, Ready};

#[derive(Debug)]
pub struct Extension {
    /// Request start time.
    pub start_time: DateTime<Utc>,

    #[cfg(feature = "i18n")]
    /// Request language.
    ///
    /// Identified through the `lang` query parameter.
    pub lang: String,

    #[cfg(feature = "traceid")]
    pub trace_id: String,

    pub real_ip: SocketAddr,
}

pub struct Middleware {
    real_ip: Rc<dyn Fn(&ServiceRequest) -> SocketAddr>,
}

impl Middleware {
    fn default_real_ip(req: &ServiceRequest) -> SocketAddr {
        req.peer_addr().unwrap()
    }

    pub fn new() -> Self {
        Self {
            real_ip: Rc::new(Self::default_real_ip),
        }
    }

    pub fn real_ip<F>(mut self, f: F) -> Self
    where
        F: Fn(&ServiceRequest) -> SocketAddr + 'static,
    {
        self.real_ip = Rc::new(f);
        self
    }
}

impl<S, B> Transform<S, ServiceRequest> for Middleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = actix_web::Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = actix_web::Error;
    type InitError = ();
    type Transform = MiddlewareService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        let real_ip = self.real_ip.clone();
        ready(Ok(MiddlewareService { service, real_ip }))
    }
}

pub struct MiddlewareService<S> {
    service: S,
    real_ip: Rc<dyn Fn(&ServiceRequest) -> SocketAddr>,
}

impl<S, B> Service<ServiceRequest> for MiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = actix_web::Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = actix_web::Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        #[cfg(feature = "i18n")]
        let state = req
            .app_data::<actix_web::web::Data<crate::state::GlobalState>>()
            .unwrap();
        let ext = Extension {
            start_time: Utc::now(),
            #[cfg(feature = "i18n")]
            lang: qstring::QString::from(req.query_string())
                .get("lang")
                .unwrap_or_else(|| &state.locale.default)
                .to_owned(),
            #[cfg(feature = "traceid")]
            trace_id: req
                .extensions()
                .get::<tracing_actix_web::RequestId>()
                .map_or_else(String::new, ToString::to_string),
            real_ip: (self.real_ip)(&req),
        };
        req.extensions_mut().insert(ext);
        Box::pin(self.service.call(req))
    }
}
