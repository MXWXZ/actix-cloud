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
- [request] (#request) (Embedded)
- [response](#response) (Default: Disable)
  - response-json
- [traceid](#traceid) (Default: Disable)

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
Authentication is quite simple, you only need to implement an extractor and a checker.

Extractor is used to extract your own authentication type from request. For example, assume we use 0 for guest and 1 for admin. Our authentication type is just `Vec<u32>`:
```
fn perm_extractor(req: &mut ServiceRequest) -> Vec<u32> {
    let mut ret = Vec::new();
    ret.push(0); // guest permission is assigned by default.

    // test if query string has `admin=1`.
    let qs = QString::from(req.query_string());
    if qs.get("admin").is_some_and(|x| x == "1") {
        ret.push(1);
    }
    ret
}
```

Checker is used to check the permission, the server will return 403 if the return value is false:
```
fn is_guest(p: Vec<u32>) -> bool {
    p.into_iter().find(|x| *x == 0).is_some()
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
- If `i18n` feature is enabled, language is identified through the `lang` query parameter, or `locale.default` in `GlobalState`.

Enable built-in middleware:
```
app.wrap(request::Middleware::new())
```

Usage:
```
async fn handler(req: HttpRequest) -> impl Responder {
    let ext = req.extensions();
    let ext = ext.get::<actix_cloud::request::Extension>().unwrap();
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

## License
This project is licensed under the [MIT license](LICENSE).
