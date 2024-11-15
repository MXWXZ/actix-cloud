use std::{net::SocketAddr, rc::Rc, sync::Arc};

use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    HttpMessage as _,
};
use chrono::{DateTime, Utc};
use futures::future::{ready, LocalBoxFuture, Ready};

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone)]
pub struct Extension {
    /// Request start time.
    pub start_time: DateTime<Utc>,

    #[cfg(feature = "i18n")]
    /// Request language.
    pub lang: String,

    #[cfg(feature = "traceid")]
    pub trace_id: String,

    pub real_ip: SocketAddr,
}

pub type RealIPFunc = Rc<dyn Fn(&ServiceRequest) -> SocketAddr>;
pub type LangFunc = Rc<dyn Fn(&ServiceRequest) -> Option<String>>;

pub struct Middleware {
    real_ip: RealIPFunc,
    #[cfg(feature = "traceid")]
    trace_header: Rc<Option<String>>,
    #[cfg(feature = "i18n")]
    lang: LangFunc,
}

impl Middleware {
    fn default_real_ip(req: &ServiceRequest) -> SocketAddr {
        req.peer_addr().unwrap()
    }

    #[cfg(feature = "i18n")]
    fn default_lang(_: &ServiceRequest) -> Option<String> {
        None
    }

    pub fn new() -> Self {
        Self {
            real_ip: Rc::new(Self::default_real_ip),
            #[cfg(feature = "traceid")]
            trace_header: Rc::new(None),
            #[cfg(feature = "i18n")]
            lang: Rc::new(Self::default_lang),
        }
    }

    #[cfg(feature = "traceid")]
    pub fn trace_header<S>(mut self, s: S) -> Self
    where
        S: Into<String>,
    {
        self.trace_header = Rc::new(Some(s.into()));
        self
    }

    pub fn real_ip<F>(mut self, f: F) -> Self
    where
        F: Fn(&ServiceRequest) -> SocketAddr + 'static,
    {
        self.real_ip = Rc::new(f);
        self
    }

    #[cfg(feature = "i18n")]
    pub fn lang<F>(mut self, f: F) -> Self
    where
        F: Fn(&ServiceRequest) -> Option<String> + 'static,
    {
        self.lang = Rc::new(f);
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
        ready(Ok(MiddlewareService {
            service: Rc::new(service),
            real_ip: self.real_ip.clone(),
            #[cfg(feature = "traceid")]
            trace_header: self.trace_header.clone(),
            #[cfg(feature = "i18n")]
            lang: self.lang.clone(),
        }))
    }
}

pub struct MiddlewareService<S> {
    service: Rc<S>,
    real_ip: RealIPFunc,
    #[cfg(feature = "traceid")]
    trace_header: Rc<Option<String>>,
    #[cfg(feature = "i18n")]
    lang: LangFunc,
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
        #[cfg(feature = "traceid")]
        let trace_id = req
            .extensions()
            .get::<tracing_actix_web::RequestId>()
            .unwrap()
            .to_string();
        let ext = Extension {
            start_time: Utc::now(),
            #[cfg(feature = "i18n")]
            lang: (self.lang)(&req).unwrap_or_else(|| state.locale.default.clone()),
            #[cfg(feature = "traceid")]
            trace_id: trace_id.clone(),
            real_ip: (self.real_ip)(&req),
        };
        #[cfg(feature = "traceid")]
        let header = self.trace_header.clone();
        req.extensions_mut().insert(Arc::new(ext));

        #[cfg(not(feature = "traceid"))]
        return Box::pin(self.service.call(req));
        #[cfg(feature = "traceid")]
        {
            use futures::FutureExt;
            use std::str::FromStr;
            return Box::pin(self.service.call(req).map(move |x| {
                if let Some(header) = header.as_ref() {
                    x.map(|mut x| {
                        x.headers_mut().insert(
                            actix_web::http::header::HeaderName::from_str(header).unwrap(),
                            actix_web::http::header::HeaderValue::from_str(&trace_id).unwrap(),
                        );
                        x
                    })
                } else {
                    x
                }
            }));
        }
    }
}
