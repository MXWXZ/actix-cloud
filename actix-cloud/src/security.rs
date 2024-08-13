use std::fmt::Display;

use actix_web::middleware;

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
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

#[derive(Clone, Debug)]
pub enum XFrameOptions {
    Deny,
    SameOrigin,
}

impl Display for XFrameOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            XFrameOptions::Deny => "DENY",
            XFrameOptions::SameOrigin => "SAMEORIGIN",
        })
    }
}

#[derive(Clone, Debug)]
pub enum XXSSProtection {
    Disable,
    Enable,
    EnableBlock,
    EnableReport(String),
}

impl Display for XXSSProtection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            XXSSProtection::Disable => f.write_str("0"),
            XXSSProtection::Enable => f.write_str("1"),
            XXSSProtection::EnableBlock => f.write_str("1; mode=block"),
            XXSSProtection::EnableReport(x) => f.write_str(&format!("1; report={}", x)),
        }
    }
}

#[derive(Clone, Debug)]
pub enum CrossOriginOpenerPolicy {
    UnsafeNone,
    SameOriginAllowPopups,
    SameOrigin,
}

impl Display for CrossOriginOpenerPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            CrossOriginOpenerPolicy::UnsafeNone => "unsafe-none",
            CrossOriginOpenerPolicy::SameOriginAllowPopups => "same-origin-allow-popups",
            CrossOriginOpenerPolicy::SameOrigin => "same-origin",
        })
    }
}

#[derive(Clone, Debug)]
pub enum StrictTransportSecurity {
    MaxAge(u32),
    IncludeSubDomains(u32),
    Preload(u32),
}

impl Display for StrictTransportSecurity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

#[derive(Clone, Debug)]
pub struct SecurityHeader {
    pub referer_policy: RefererPolicy,
    pub x_frame_options: XFrameOptions,
    pub x_xss_protection: XXSSProtection,
    pub cross_origin_opener_policy: CrossOriginOpenerPolicy,
    pub content_security_policy: String,
    pub strict_transport_security: Option<StrictTransportSecurity>,
}

impl Default for SecurityHeader {
    fn default() -> Self {
        Self {
            referer_policy: RefererPolicy::StrictOriginWhenCrossOrigin,
            x_frame_options: XFrameOptions::Deny,
            x_xss_protection: XXSSProtection::EnableBlock,
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

    pub fn build(self) -> middleware::DefaultHeaders {
        let mut ret = middleware::DefaultHeaders::new()
            .add(("X-Content-Type-Options", "nosniff"))
            .add(("Referrer-Policy", self.referer_policy.to_string()))
            .add(("X-Frame-Options", self.x_frame_options.to_string()))
            .add(("X-XSS-Protection", self.x_xss_protection.to_string()))
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
