//! Middleware to add security headers to every response.
//!
//! Wrap the app with [`SecurityHeader::default().build()`](SecurityHeader::build) and
//! customize fields or enable HSTS (see [`SecurityHeader::set_default_hsts`]) as needed.
use std::fmt::{self, Display};

use actix_web::middleware;

/// Value of the `Referrer-Policy` header.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug)]
pub enum RefererPolicy {
    NoReferrer,
    NoReferrerWhenDowngrade,
    Origin,
    OriginWhenCrossOrigin,
    SameOrigin,
    StrictOrigin,
    StrictOriginWhenCrossOrigin,
    UnsafeUrl,
}

impl Display for RefererPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            RefererPolicy::NoReferrer => "no-referrer",
            RefererPolicy::NoReferrerWhenDowngrade => "no-referrer-when-downgrade",
            RefererPolicy::Origin => "origin",
            RefererPolicy::OriginWhenCrossOrigin => "origin-when-cross-origin",
            RefererPolicy::SameOrigin => "same-origin",
            RefererPolicy::StrictOrigin => "strict-origin",
            RefererPolicy::StrictOriginWhenCrossOrigin => "strict-origin-when-cross-origin",
            RefererPolicy::UnsafeUrl => "unsafe-url",
        })
    }
}

/// Value of the `X-Frame-Options` header.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug)]
pub enum XFrameOptions {
    Deny,
    SameOrigin,
}

impl Display for XFrameOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            XFrameOptions::Deny => "DENY",
            XFrameOptions::SameOrigin => "SAMEORIGIN",
        })
    }
}

/// Value of the `Cross-Origin-Opener-Policy` header.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug)]
pub enum CrossOriginOpenerPolicy {
    UnsafeNone,
    SameOriginAllowPopups,
    SameOrigin,
}

impl Display for CrossOriginOpenerPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            CrossOriginOpenerPolicy::UnsafeNone => "unsafe-none",
            CrossOriginOpenerPolicy::SameOriginAllowPopups => "same-origin-allow-popups",
            CrossOriginOpenerPolicy::SameOrigin => "same-origin",
        })
    }
}

/// Value of the `Strict-Transport-Security` (HSTS) header. The payload is `max-age` in
/// seconds; variants add the `includeSubDomains` / `preload` directives.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug)]
pub enum StrictTransportSecurity {
    MaxAge(u32),
    IncludeSubDomains(u32),
    Preload(u32),
}

impl Display for StrictTransportSecurity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StrictTransportSecurity::MaxAge(x) => f.write_str(&format!("max-age={}", x)),
            StrictTransportSecurity::IncludeSubDomains(x) => {
                f.write_str(&format!("max-age={}; includeSubDomains", x))
            }
            StrictTransportSecurity::Preload(x) => {
                f.write_str(&format!("max-age={}; includeSubDomains; preload", x))
            }
        }
    }
}

/// Security headers applied to every response by [`SecurityHeader::build`].
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug)]
pub struct SecurityHeader {
    /// Value of the `Referrer-Policy` header.
    pub referer_policy: RefererPolicy,
    /// Value of the `X-Frame-Options` header.
    pub x_frame_options: XFrameOptions,
    /// Value of the `Cross-Origin-Opener-Policy` header.
    pub cross_origin_opener_policy: CrossOriginOpenerPolicy,
    /// Value of the `Content-Security-Policy` header.
    pub content_security_policy: String,
    /// Value of the `Strict-Transport-Security` header; `None` omits it.
    ///
    /// See [`Self::set_default_hsts`] for the recommended one-year policy.
    pub strict_transport_security: Option<StrictTransportSecurity>,
}

impl Default for SecurityHeader {
    fn default() -> Self {
        Self {
            referer_policy: RefererPolicy::StrictOriginWhenCrossOrigin,
            x_frame_options: XFrameOptions::Deny,
            cross_origin_opener_policy: CrossOriginOpenerPolicy::SameOrigin,
            content_security_policy: String::from("default-src 'none'; script-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"),
            strict_transport_security: None,
        }
    }
}

impl SecurityHeader {
    /// Set default HSTS to 1 year, includeSubDomains and preload.
    ///
    /// `max-age=31536000; includeSubDomains; preload`
    pub fn set_default_hsts(&mut self) {
        self.strict_transport_security = Some(StrictTransportSecurity::Preload(31536000));
    }

    /// Build the actix-web [`DefaultHeaders`](middleware::DefaultHeaders) middleware
    /// carrying all configured security headers (plus HSTS when set).
    pub fn build(self) -> middleware::DefaultHeaders {
        let mut ret = middleware::DefaultHeaders::new()
            .add(("X-Content-Type-Options", "nosniff"))
            .add(("Referrer-Policy", self.referer_policy.to_string()))
            .add(("X-Frame-Options", self.x_frame_options.to_string()))
            .add((
                "Cross-Origin-Opener-Policy",
                self.cross_origin_opener_policy.to_string(),
            ))
            .add(("Content-Security-Policy", self.content_security_policy));
        if let Some(hsts) = self.strict_transport_security {
            ret = ret.add(("Strict-Transport-Security", hsts.to_string()));
        }
        ret
    }
}

#[cfg(test)]
mod tests {
    use actix_web::{test as web_test, web, App, HttpResponse};

    use super::*;

    #[test]
    fn header_values_are_formatted() {
        assert_eq!(RefererPolicy::NoReferrer.to_string(), "no-referrer");
        assert_eq!(
            RefererPolicy::StrictOriginWhenCrossOrigin.to_string(),
            "strict-origin-when-cross-origin"
        );
        assert_eq!(XFrameOptions::Deny.to_string(), "DENY");
        assert_eq!(XFrameOptions::SameOrigin.to_string(), "SAMEORIGIN");
        assert_eq!(
            CrossOriginOpenerPolicy::UnsafeNone.to_string(),
            "unsafe-none"
        );
        assert_eq!(
            CrossOriginOpenerPolicy::SameOriginAllowPopups.to_string(),
            "same-origin-allow-popups"
        );
        assert_eq!(
            StrictTransportSecurity::MaxAge(60).to_string(),
            "max-age=60"
        );
        assert_eq!(
            StrictTransportSecurity::IncludeSubDomains(60).to_string(),
            "max-age=60; includeSubDomains"
        );
        assert_eq!(
            StrictTransportSecurity::Preload(60).to_string(),
            "max-age=60; includeSubDomains; preload"
        );
    }

    async fn headers_of(security: SecurityHeader) -> actix_web::dev::ServiceResponse {
        let app = web_test::init_service(
            App::new()
                .wrap(security.build())
                .route("/", web::get().to(|| async { HttpResponse::Ok().finish() })),
        )
        .await;
        web_test::call_service(&app, web_test::TestRequest::get().uri("/").to_request()).await
    }

    #[actix_web::test]
    async fn default_headers_are_applied() {
        let rsp = headers_of(SecurityHeader::default()).await;

        assert_eq!(
            rsp.headers()
                .get("X-Content-Type-Options")
                .unwrap()
                .to_str()
                .unwrap(),
            "nosniff"
        );
        assert_eq!(
            rsp.headers()
                .get("Referrer-Policy")
                .unwrap()
                .to_str()
                .unwrap(),
            "strict-origin-when-cross-origin"
        );
        assert_eq!(
            rsp.headers()
                .get("X-Frame-Options")
                .unwrap()
                .to_str()
                .unwrap(),
            "DENY"
        );
        assert_eq!(
            rsp.headers()
                .get("Cross-Origin-Opener-Policy")
                .unwrap()
                .to_str()
                .unwrap(),
            "same-origin"
        );
        // HSTS is opt-in.
        assert!(rsp.headers().get("Strict-Transport-Security").is_none());
    }

    #[actix_web::test]
    async fn default_hsts_is_opt_in() {
        let mut security = SecurityHeader::default();
        security.set_default_hsts();
        let rsp = headers_of(security).await;

        assert_eq!(
            rsp.headers()
                .get("Strict-Transport-Security")
                .unwrap()
                .to_str()
                .unwrap(),
            "max-age=31536000; includeSubDomains; preload"
        );
    }
}
