//! Configuration options to tune the behaviour of [`SessionMiddleware`].

use std::sync::Arc;

use actix_web::cookie::{time::Duration, Key, SameSite};

use crate::memorydb::MemoryDB;

use super::{storage::SessionStore, SessionMiddleware};

/// A [session lifecycle](SessionLifecycle) strategy where the session cookie will be [persistent].
///
/// Persistent cookies have a pre-determined expiration, specified via the `Max-Age` or `Expires`
/// attribute. They do not disappear when the current browser session ends.
///
/// Due to its `Into<SessionLifecycle>` implementation, a `PersistentSession` can be passed directly
/// to [`SessionMiddlewareBuilder::session_lifecycle()`].
///
/// # Examples
/// ```
/// use actix_cloud::actix_web::cookie::time::Duration;
/// use actix_cloud::session::SessionMiddleware;
/// use actix_cloud::session::config::{PersistentSession, TtlExtensionPolicy};
///
/// const SECS_IN_WEEK: i64 = 60 * 60 * 24 * 7;
///
/// // a session lifecycle with a time-to-live (expiry) of 1 week and default extension policy
/// PersistentSession::default().session_ttl(Duration::seconds(SECS_IN_WEEK));
///
/// // a session lifecycle with the default time-to-live (expiry) and a custom extension policy
/// PersistentSession::default()
///     // this policy causes the session state's TTL to be refreshed on every request
///     .session_ttl_extension_policy(TtlExtensionPolicy::OnEveryRequest);
/// ```
///
/// [persistent]: https://www.whitehatsec.com/glossary/content/persistent-session-cookie
#[derive(Debug, Clone)]
pub struct PersistentSession {
    session_ttl: Duration,
    ttl_extension_policy: TtlExtensionPolicy,
}

impl PersistentSession {
    /// Specifies how long the session cookie should live.
    ///
    /// The session TTL is also used as the TTL for the session state in the storage backend.
    ///
    /// Defaults to 1 day.
    ///
    /// A persistent session can live more than the specified TTL if the TTL is extended.
    /// See [`session_ttl_extension_policy`](Self::session_ttl_extension_policy) for more details.
    #[doc(alias = "max_age", alias = "max age", alias = "expires")]
    pub fn session_ttl(mut self, session_ttl: Duration) -> Self {
        self.session_ttl = session_ttl;
        self
    }

    /// Determines under what circumstances the TTL of your session should be extended.
    /// See [`TtlExtensionPolicy`] for more details.
    ///
    /// Defaults to [`TtlExtensionPolicy::OnStateChanges`].
    pub fn session_ttl_extension_policy(
        mut self,
        ttl_extension_policy: TtlExtensionPolicy,
    ) -> Self {
        self.ttl_extension_policy = ttl_extension_policy;
        self
    }
}

impl Default for PersistentSession {
    fn default() -> Self {
        Self {
            session_ttl: default_ttl(),
            ttl_extension_policy: default_ttl_extension_policy(),
        }
    }
}

/// Configuration for which events should trigger an extension of the time-to-live for your session.
///
/// If you are using a [`BrowserSession`], `TtlExtensionPolicy` controls how often the TTL of the
/// session state should be refreshed. The browser is in control of the lifecycle of the session
/// cookie.
///
/// If you are using a [`PersistentSession`], `TtlExtensionPolicy` controls both the expiration of
/// the session cookie and the TTL of the session state on the storage backend.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum TtlExtensionPolicy {
    /// The TTL is refreshed every time the server receives a request associated with a session.
    ///
    /// # Performance impact
    /// Refreshing the TTL on every request is not free. It implies a refresh of the TTL on the
    /// session state. This translates into a request over the network if you are using a remote
    /// system as storage backend (e.g. Redis). This impacts both the total load on your storage
    /// backend (i.e. number of queries it has to handle) and the latency of the requests served by
    /// your server.
    OnEveryRequest,

    /// The TTL is refreshed every time the session state changes or the session key is renewed.
    OnStateChanges,
}

/// Determines how to secure the content of the session cookie.
///
/// Used by [`SessionMiddlewareBuilder::cookie_content_security`].
#[derive(Debug, Clone, Copy)]
pub enum CookieContentSecurity {
    /// The cookie content is encrypted when using `CookieContentSecurity::Private`.
    ///
    /// Encryption guarantees confidentiality and integrity: the client cannot tamper with the
    /// cookie content nor decode it, as long as the encryption key remains confidential.
    Private,

    /// The cookie content is signed when using `CookieContentSecurity::Signed`.
    ///
    /// Signing guarantees integrity, but it doesn't ensure confidentiality: the client cannot
    /// tamper with the cookie content, but they can read it.
    Signed,
}

pub(crate) const fn default_ttl() -> Duration {
    Duration::days(1)
}

pub(crate) const fn default_ttl_extension_policy() -> TtlExtensionPolicy {
    TtlExtensionPolicy::OnStateChanges
}

/// A fluent, customized [`SessionMiddleware`] builder.
#[must_use]
pub struct SessionMiddlewareBuilder {
    storage_backend: SessionStore,
    configuration: Configuration,
}

impl SessionMiddlewareBuilder {
    pub(crate) fn new(client: Arc<dyn MemoryDB>, configuration: Configuration) -> Self {
        Self {
            storage_backend: SessionStore::new(client),
            configuration,
        }
    }

    pub fn cache_keygen<F>(mut self, keygen: F) -> Self
    where
        F: Fn(&str) -> String + 'static + Send + Sync,
    {
        self.storage_backend.cache_keygen(keygen);
        self
    }

    /// Set the name of the cookie used to store the session ID.
    ///
    /// Defaults to `id`.
    pub fn cookie_name(mut self, name: String) -> Self {
        self.configuration.cookie.name = name;
        self
    }

    /// Set the `Secure` attribute for the cookie used to store the session ID.
    ///
    /// If the cookie is set as secure, it will only be transmitted when the connection is secure
    /// (using `https`).
    ///
    /// Default is `true`.
    pub fn cookie_secure(mut self, secure: bool) -> Self {
        self.configuration.cookie.secure = secure;
        self
    }

    /// Determines how session lifecycle should be managed.
    pub fn session_lifecycle(mut self, session_lifecycle: PersistentSession) -> Self {
        self.configuration.cookie.max_age = Some(session_lifecycle.session_ttl);
        self.configuration.session.state_ttl = session_lifecycle.session_ttl;
        self.configuration.ttl_extension_policy = session_lifecycle.ttl_extension_policy;

        self
    }

    /// Set the `SameSite` attribute for the cookie used to store the session ID.
    ///
    /// By default, the attribute is set to `Lax`.
    pub fn cookie_same_site(mut self, same_site: SameSite) -> Self {
        self.configuration.cookie.same_site = same_site;
        self
    }

    /// Set the `Path` attribute for the cookie used to store the session ID.
    ///
    /// By default, the attribute is set to `/`.
    pub fn cookie_path(mut self, path: String) -> Self {
        self.configuration.cookie.path = path;
        self
    }

    /// Set the `Domain` attribute for the cookie used to store the session ID.
    ///
    /// Use `None` to leave the attribute unspecified. If unspecified, the attribute defaults
    /// to the same host that set the cookie, excluding subdomains.
    ///
    /// By default, the attribute is left unspecified.
    pub fn cookie_domain(mut self, domain: Option<String>) -> Self {
        self.configuration.cookie.domain = domain;
        self
    }

    /// Choose how the session cookie content should be secured.
    ///
    /// - [`CookieContentSecurity::Private`] selects encrypted cookie content.
    /// - [`CookieContentSecurity::Signed`] selects signed cookie content.
    ///
    /// # Default
    /// By default, the cookie content is encrypted. Encrypted was chosen instead of signed as
    /// default because it reduces the chances of sensitive information being exposed in the session
    /// key by accident, regardless of SessionStore implementation you chose to use.
    ///
    /// For example, if you are using cookie-based storage, you definitely want the cookie content
    /// to be encrypted—the whole session state is embedded in the cookie! If you are using
    /// Redis-based storage, signed is more than enough - the cookie content is just a unique
    /// tamper-proof session key.
    pub fn cookie_content_security(mut self, content_security: CookieContentSecurity) -> Self {
        self.configuration.cookie.content_security = content_security;
        self
    }

    /// Set the `HttpOnly` attribute for the cookie used to store the session ID.
    ///
    /// If the cookie is set as `HttpOnly`, it will not be visible to any JavaScript snippets
    /// running in the browser.
    ///
    /// Default is `true`.
    pub fn cookie_http_only(mut self, http_only: bool) -> Self {
        self.configuration.cookie.http_only = http_only;
        self
    }

    /// Finalise the builder and return a [`SessionMiddleware`] instance.
    #[must_use]
    pub fn build(self) -> SessionMiddleware {
        SessionMiddleware::from_parts(self.storage_backend, self.configuration)
    }
}

#[derive(Clone)]
pub(crate) struct Configuration {
    pub(crate) cookie: CookieConfiguration,
    pub(crate) session: SessionConfiguration,
    pub(crate) ttl_extension_policy: TtlExtensionPolicy,
}

#[derive(Clone)]
pub(crate) struct SessionConfiguration {
    pub(crate) state_ttl: Duration,
}

#[derive(Clone)]
pub(crate) struct CookieConfiguration {
    pub(crate) secure: bool,
    pub(crate) http_only: bool,
    pub(crate) name: String,
    pub(crate) same_site: SameSite,
    pub(crate) path: String,
    pub(crate) domain: Option<String>,
    pub(crate) max_age: Option<Duration>,
    pub(crate) content_security: CookieContentSecurity,
    pub(crate) key: Key,
}

pub(crate) fn default_configuration(key: Key) -> Configuration {
    Configuration {
        cookie: CookieConfiguration {
            secure: true,
            http_only: true,
            name: "id".into(),
            same_site: SameSite::Lax,
            path: "/".into(),
            domain: None,
            max_age: None,
            content_security: CookieContentSecurity::Private,
            key,
        },
        session: SessionConfiguration {
            state_ttl: default_ttl(),
        },
        ttl_extension_policy: default_ttl_extension_policy(),
    }
}
