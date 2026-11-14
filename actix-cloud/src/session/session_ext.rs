use actix_web::{
    dev::{ServiceRequest, ServiceResponse},
    guard::GuardContext,
    HttpMessage, HttpRequest,
};

use super::Session;

/// Extract a [`Session`] object from various `actix-web` types (e.g. `HttpRequest`,
/// `ServiceRequest`, `ServiceResponse`).
pub trait SessionExt {
    /// Extract a [`Session`] object.
    fn get_session(&self) -> Session;
}

impl SessionExt for HttpRequest {
    fn get_session(&self) -> Session {
        Session::get_session(&mut self.extensions_mut())
    }
}

impl SessionExt for ServiceRequest {
    fn get_session(&self) -> Session {
        Session::get_session(&mut self.extensions_mut())
    }
}

impl SessionExt for ServiceResponse {
    fn get_session(&self) -> Session {
        self.request().get_session()
    }
}

impl SessionExt for GuardContext<'_> {
    fn get_session(&self) -> Session {
        Session::get_session(&mut self.req_data_mut())
    }
}

#[cfg(test)]
mod tests {
    use actix_web::{test as web_test, HttpResponse};

    use super::*;

    #[actix_web::test]
    async fn request_and_service_request_share_one_session() {
        let req = web_test::TestRequest::default().to_http_request();
        req.get_session().insert("a", 1).unwrap();
        assert_eq!(req.get_session().get::<i32>("a").unwrap(), Some(1));

        let srv = web_test::TestRequest::default().to_srv_request();
        srv.get_session().insert("b", 2).unwrap();
        assert_eq!(srv.get_session().get::<i32>("b").unwrap(), Some(2));
    }

    #[actix_web::test]
    async fn service_response_uses_the_request_session() {
        let req = web_test::TestRequest::default().to_srv_request();
        req.get_session().insert("a", 1).unwrap();

        let rsp = req.into_response(HttpResponse::Ok().finish());
        assert_eq!(rsp.get_session().get::<i32>("a").unwrap(), Some(1));
    }
}
