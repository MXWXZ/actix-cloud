# actix-cloud

Actix Cloud is an all-in-one web framework based on [Actix Web](https://crates.io/crates/actix-web).

## Features
Actix Cloud is highly configurable. You can only enable needed features, implement your own feature backend or even use other libraries.

- [logger](#logger) (Default: Enable)
- [i18n](#i18n) (Default: Disable)
- [security](#security) (Embedded)
- memorydb (Default: Disable)
  - [default](#memorydb-default) (Embedded)
  - [redis](#memorydb-redis) (Default: Disable)
- [auth](#auth) (Embedded)
- [session](#session) (Default: Disable)
- [config](#config) (Default: Disable)
  - config-json
  - config-yaml
  - config-toml
- [request](#request) (Embedded)
- [response](#response) (Default: Disable)
  - response-json
- [traceid](#traceid) (Default: Disable)
- [seaorm](#seaorm) (Default: Disable)
- [csrf](#csrf) (Default: Disable)

## Guide

### Quick Start
You can refer to [Hello world](examples/hello_world/) example for basic usage.

### Application
Since application configuration can be quite dynamic, you need to build on your own. Here are some useful middlewares:

```
App::new()
    .wrap(middleware::Compress::default()) // compress page
    .wrap(SecurityHeader::default().build()) // default security header
    .wrap(SessionMiddleware::builder(memorydb.clone(), Key::generate()).build()) // session
    ...
    .app_data(state_cloned.clone())
```

### logger
We use [tracing](https://crates.io/crates/tracing) as our logger library. It is thread safe. You can use it everywhere.

Start logger:
```
LoggerBuilder::new().level(Level::DEBUG).start()        // colorful output
LoggerBuilder::new().json().start() // json output
```
You can also customize the logger with `filter`, `transformer`, etc.

Reinit logger (e.g., in plugins), or manually send logs:
```
logger.init(LoggerBuilder::new());
logger.sender().send(...);
```

Reserved field:
- `_time`: timestamp in microseconds, override the log timestamp.

### i18n
We use `rust-i18n-support` from [rust-i18n](https://crates.io/crates/rust-i18n) as our i18n core. 

Load locale:
```
let locale = Locale::new(String::from("en-US")).add_locale(i18n!("locale"));
```

Translate:
```
t!(locale, "hello.world")
t!(locale, "hello.name", name = "MEME")
```

See [examples](examples/i18n) for more usage.

### security
Middleware to add security headers:
```
app.wrap(SecurityHeader::default().build())
```

Default header:
```
X-Content-Type-Options: nosniff
Referrer-Policy: strict-origin-when-cross-origin
X-Frame-Options: DENY
X-XSS-Protection: 1; mode=block
Cross-Origin-Opener-Policy: same-origin
Content-Security-Policy: default-src 'none'; script-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'
```

Enable HSTS when using HTTPS:
```
security_header.set_default_hsts();
```
```
Strict-Transport-Security: max-age=31536000; includeSubDomains; preload
```

### memorydb-default
Actix Cloud has a default memory database backend used for sessions. You can also use your own backend if you implement `actix_cloud::memorydb::MemoryDB`.

**Note: the default backend does not have memory limitation, DDoS is possible if gateway rate limiting is not implemented**

```
DefaultBackend::new()
```

### memorydb-redis
Redis can be used as another backend for memory database.

```
RedisBackend::new("redis://user:pass@127.0.0.1:6379/0").await.unwrap(),
```

### auth
Authentication is quite simple, you only need to implement a checker.

Checker is used to check the permission, the server will return 403 if the return value is false:
```
struct AuthChecker {
    need_admin: bool,
}

impl AuthChecker {
    fn new(need_admin: bool) -> Self {
        Self { need_admin }
    }
}

#[async_trait(?Send)]
impl Checker for AuthChecker {
    async fn check(&self, req: &mut ServiceRequest) -> Result<bool> {
        let qs = QString::from(req.query_string());
        let is_admin = if qs.get("admin").is_some_and(|x| x == "1") {
            true
        } else {
            false
        };
        if (is_admin && self.need_admin) || !self.need_admin {
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
```

Then build the `Router` and configure in the App using `build_router`:
```
app.service(scope("/api").configure(build_router(...)))
```

### session
Most features and usages are based on [actix-session](https://crates.io/crates/actix-session). Except for these:
- MemoryDB is the only supported storage.
- Error uses `actix-cloud::error::Error`.
- You can set `_ttl` in the session to override the TTL of the session.
- You can set `_id` in the session for reverse search.
  - Quote(") will be trimmed.
  - Another key will be set in memorydb: `{_id}_{session_key}`. You can use `keys` function to find all session key binding to a specific id.

```
app.wrap(SessionMiddleware::builder(memorydb.clone(), Key::generate()).build())
```

### config
[config-rs](https://crates.io/crates/config) is the underlying library.

Supported features:
- config-json: Support for JSON files.
- config-yaml: Support for YAML files.
- config-toml: Support for TOML files.

### request
Provide per-request extension.

Built-in middleware:
- Store in [extensions](https://docs.rs/actix-web/latest/actix_web/struct.HttpRequest.html#method.extensions_mut).
- If `i18n` feature is enabled, language is identified through the callback, or `locale.default` in `GlobalState`.

Enable built-in middleware:
```
app.wrap(request::Middleware::new())
```

Usage:
```
async fn handler(req: HttpRequest) -> impl Responder {
    let ext = req.extensions();
    let ext = ext.get::<Arc<actix_cloud::request::Extension>>().unwrap();
    ...
}

async fn handler(ext: ReqData<Arc<actix_cloud::request::Extension>>) -> impl Responder {
    ...
}
```

### response
Provide useful response type.

If `i18n` feature is enabled, response message will be translated automatically.

If `response-json` feature is enabled, response message will be converted to JSON automatically.

1. Create response yml files.
2. Use `build.rs` to generate source files.
3. Use `include!` to include generated files.

See [examples](examples/response) for detailed usage.

### traceid
Add trace ID for each request based on [tracing-actix-web](https://crates.io/crates/tracing-actix-web).

```
app.wrap(request::Middleware::new())
   .wrap(TracingLogger::default())      // This should be after request::Middleware
```

If you enable `request` feature, make sure it is before `TracingLogger` since the `trace_id` field is based on it.

### seaorm
Provide useful macros for [seaorm](https://crates.io/crates/sea-orm).

```
#[derive(...)]
#[sea_orm(...)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub created_at: i64,
    pub updated_at: i64,
}

#[entity_id(Uuid::new_v4())]    // generate new for `id` field.
#[entity_timestamp]             // automatically handle `created_at` and `updated_at` field.
impl ActiveModel {}

#[entity_behavior]              // enable `entity_id` and `entity_timestamp`.
impl ActiveModelBehavior for ActiveModel {}
```

### csrf
We use [double submit](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html#alternative-using-a-double-submit-cookie-pattern) to protect against CSRF attacks.

You can use `memorydb` to store and check CSRF tokens.

By default, CSRF checker is applied to:
- All [unsafe](https://developer.mozilla.org/en-US/docs/Glossary/Safe/HTTP) methods unless `CSRFType` is `Disabled`.
- All methods if `CSRFType` is `ForceHeader` or `ForceParam`.

Generally, `Param` and `ForceParam` type should only be used for websocket.

```
build_router(
    route,
    csrf::Middleware::new(
        String::from("CSRF_TOKEN"),     // csrf cookie
        String::from("X-CSRF-Token"),   // csrf header/param
        |req, token| Box::pin(async { Ok(true) })          // csrf checker
    ),
);
```

## License
This project is licensed under the [MIT license](LICENSE).
