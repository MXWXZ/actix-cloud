//! Request-cookie helpers shared by the session and CSRF middlewares.
use actix_web::{cookie::Cookie, dev::ServiceRequest};

/// Return the single cookie named `name`.
///
/// Returns `None` when the cookie is missing, duplicated, or the `Cookie` headers
/// cannot be parsed. Rejecting duplicates prevents cookie shadowing, where a second
/// cookie with the same name would otherwise silently win or lose depending on the
/// parser. The value is returned as-is; each caller applies its own policy.
pub(crate) fn unique_cookie(req: &ServiceRequest, name: &str) -> Option<Cookie<'static>> {
    let cookies = req.cookies().ok()?;
    let mut matches = cookies.iter().filter(|c| c.name() == name);
    let cookie = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(cookie.clone())
}
