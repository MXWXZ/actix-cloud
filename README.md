# actix-cloud

[![crates.io](https://img.shields.io/crates/v/actix-cloud?label=crates.io&style=flat-square)](https://crates.io/crates/actix-cloud)
[![docs.rs](https://img.shields.io/docsrs/actix-cloud?style=flat-square)](https://docs.rs/actix-cloud/latest)
![license](https://img.shields.io/github/license/mxwxz/actix-cloud?style=flat-square)

Actix Cloud is an all-in-one web framework based on [Actix Web](https://crates.io/crates/actix-web).

## Features
Actix Cloud is highly configurable. You can only enable needed features, implement your own feature backend or even use other libraries.

Items marked *(Default)* are enabled by default; *(Optional)* features must be enabled explicitly.

- [logger](#logger) (Default)
- [i18n](#i18n) (Default)
- [security](#security) (Default)
- [state](#state) (Default)
- [request](#request) (Default)
- [traceid](#traceid) (Default)
- [auth](#auth) (Default, feature `router`)
- [session](#session) (Default)
- [csrf](#csrf) (Default)
- memorydb
  - [default backend](#memorydb-default) (Default)
  - [redis](#memorydb-redis) (Optional, feature `redis`)
- [response](#response)
  - response-json (Default)
  - response-build (Optional, only for `build.rs`)
- [config](#config)
  - config-yaml (Default)
  - config-json / config-toml (Optional)
- [utils](#utils) (Default) — random strings, restart, rustls config
- `all`: enable everything.

## Guide

### Quick Start
You can refer to the [hello_world](examples/hello_world/) example for the full code.

### state
`GlobalState` is the shared application state, stored as actix-web `Data<GlobalState>` and attached with `.app_data(...)`:

```
GlobalState {
    memorydb: Arc<dyn MemoryDB>,                // feature `memorydb`
    config: config::Config,                     // feature `config`
    logger: Option<Logger>,                     // feature `logger`
    locale: Locale,                             // feature `i18n`
    server: ServerHandle,                       // always present
}
.build()    // -> Data<GlobalState>
```

Full-featured construction:
```
let state = GlobalState {
    memorydb: Arc::new(DefaultBackend::new(None)),
    config: Config::default(),
    logger: Some(logger),
    locale: Locale::new("en-US").add_locale(i18n!("locale")),
    server: ServerHandle::default(),
}
.build();
```

`ServerHandle` owns the server lifecycle:
- `start(server)` sets the handle and blocks until the server exits.
- `stop(graceful)` sends the stop signal.
- `running`, `start_time`, `stop_time` expose the server status.

### logger
We use [tracing](https://crates.io/crates/tracing) as our logger library. It is thread safe. You can use it everywhere.

Start the logger:
```
let (logger, guard) = LoggerBuilder::new()
    .level(Level::DEBUG)
    .filename()
    .line_number()
    .start();
```

- Default output is colorful, `INFO` level. Use `.json()` for JSON-lines output.
- `WARN`/`ERROR` are written to stderr, other levels to stdout.
- Keep the `LoggerGuard` alive: dropping it flushes queued logs and stops the logger thread.

Builder options:

| Method                                | Description                                                               |
| ------------------------------------- | ------------------------------------------------------------------------- |
| `level(Level)`                        | Max log level, default `INFO`.                                            |
| `json()`                              | Use the JSON writer instead of the colorful one.                          |
| `filename()` / `line_number()`        | Include the event's file / line in each entry.                            |
| `filter(Fn(&LogItem) -> bool)`        | Return `false` to drop a log entry.                                       |
| `transformer(Fn(LogItem) -> LogItem)` | Rewrite entries on the fly.                                               |
| `handler(Fn(&Map) -> Future<bool>)`   | Raw JSON handler invoked first; return `false` to skip normal processing. |
| `json_writer` / `color_writer`        | Custom writers (`Fn(LogItem, Box<dyn Write>) -> Result<()>`).             |

Wait until every queued log entry has been written out:
```
actix_cloud::logger::flush();
```

Fatal macros — log an error and exit the process with code 1 (queued logs are flushed first):
```
fatal!("config file not found: {}", path);
let home = unwrap_fatal!(std::env::var("HOME"));
let home = expect_fatal!(std::env::var("HOME"), "HOME is not set");
```

Reserved fields, overridable in log records:

| Field                        | Description                                                                                      |
| ---------------------------- | ------------------------------------------------------------------------------------------------ |
| `_time`                      | Timestamp in microseconds, overrides the log timestamp.                                          |
| `_filename` / `_line_number` | Override the recorded file / line (only applied when `filename()` / `line_number()` is enabled). |

### i18n
We use `rust-i18n-support` from [rust-i18n](https://crates.io/crates/rust-i18n) as our i18n core.

Locale files are YAML files named after the locale (e.g. `locale/en-US.yml`), with `%{name}` placeholders:
```
hello.world: Hello world.
hello.name: Hello %{name}.
```

Load the locale. `i18n!` globs `**/*.yml` under the given path at compile time:
```
let locale = Locale::new("en-US").add_locale(i18n!("locale"));
```

Translate with `t!`, falling back to the default locale, then to the key itself:
```
t!(locale, "hello.world")                          // default locale
t!(locale, "hello.world", "zh-CN")                 // specific locale
t!(locale, "hello.name", name = "MEME")            // variables
t!(locale, "hello.name", "zh-CN", name = "MEME")   // both
```

`map!{"key" => "value"}` is a helper macro to create a `BTreeMap`, useful when building response data.

For web requests, register the locale in `GlobalState` and identify the request language with the `request::Middleware` callback; the identified language is stored in `Extension.lang` and used to translate `Response` messages, or falls back to `locale.default` when returning `None`:

```
let state = GlobalState {
    locale: Locale::new("en-US").add_locale(i18n!("locale")),
    ...
}
.build();

app.wrap(
    request::Middleware::new().lang(|req| {
        req.headers()
            .get("Accept-Language")
            .and_then(|x| x.to_str().ok())
            .and_then(|x| x.split(',').next())
            .map(String::from)
    })
)
```

See the [i18n](examples/i18n) example for more usage.

### security
Middleware to add security headers:
```
app.wrap(SecurityHeader::default().build())
```

Default headers:
```
X-Content-Type-Options: nosniff
Referrer-Policy: strict-origin-when-cross-origin
X-Frame-Options: DENY
Cross-Origin-Opener-Policy: same-origin
Content-Security-Policy: default-src 'none'; script-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'
```

Enable HSTS when using HTTPS:
```
let mut header = SecurityHeader::default();
header.set_default_hsts();  // max-age=31536000; includeSubDomains; preload
app.wrap(header.build())
```

### memorydb-default
Actix Cloud has a default memory database backend used for sessions. You can also use your own backend by implementing the object-safe `MemoryDB` trait. Backends are shared as `Arc<dyn MemoryDB>`.

**The default memorydb is only for demo purpose. The performance is not guaranteed.**

```
DefaultBackend::new(None)   // No resource limitation, DDoS is possible!
DefaultBackend::new(Some(100000000))  // Maximum 100000000 entries.
```

When the database is full, key eviction policy is:
- Evict 10% keys at a time.
- Expired keys go first, then keys having TTL attribute, sorted by TTL from shortest to longest.
- If no keys are evicted, an error will be returned.

Note that the internal implementation uses lazy deletion for performance.

### memorydb-redis
Redis can be used as another backend for memory database. The connection reconnects automatically.

```
RedisBackend::new("redis://user:pass@127.0.0.1:6379/0").await.unwrap(),
```

### auth
Authentication is quite simple, you only need to implement a checker.

Return `Ok(())` to allow the request, or an `Err` to reject it. The error is returned to the client as-is, so you decide the status code with `ErrorUnauthorized`, `ErrorForbidden`, etc.:
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
    async fn check(&self, req: &mut ServiceRequest) -> Result<(), Error> {
        let qs = QString::from(req.query_string());
        let is_admin = if qs.get("admin").is_some_and(|x| x == "1") {
            true
        } else {
            false
        };
        if (is_admin && self.need_admin) || !self.need_admin {
            Ok(())
        } else {
            Err(ErrorForbidden("permission denied"))
        }
    }
}
```

Each route is a `Router` with the path, the actix-web route, an optional checker and (feature `csrf`) the CSRF enforcement level:

```
Router {
    path: String::from("/admin"),
    route: get().to(admin_page),
    checker: Some(Rc::new(AuthChecker::new(true))),
    csrf: CSRFType::Header,     // feature `csrf`
}
```

Configure the routers with `build_router` on a scope:
```
app.service(scope("/api").configure(build_router(vec![...])))

// With the `csrf` feature, pass the CSRF middleware as well:
app.service(scope("/api").configure(build_router(vec![...], csrf::Middleware::new(
    String::from("CSRF_TOKEN"),     // csrf cookie
    String::from("X-CSRF-Token"),   // csrf header/param
    |req, token| Box::pin(async { Ok(true) })  // extra csrf checker
))))
```

Routes with an empty `path` are skipped. The auth checker always runs before CSRF.

### session
Most features and usages are based on [actix-session](https://crates.io/crates/actix-session). Except for these:
- MemoryDB is the only supported storage.
- You can set `_ttl` (milliseconds) in the session to override the TTL of the session (and the cookie's `Max-Age`).
- You can set `_id` in the session for reverse search.
  - Quote(") will be trimmed.
  - Another key will be set in memorydb: `{_id}_{session_key}`. You can use the `keys` function to find all session keys binding to a specific id (e.g. `keys("{id}_*")`).

Create the middleware. Use a fixed key to keep sessions across restarts, `Key::generate()` creates a new one for every restart:
```
app.wrap(SessionMiddleware::new(memorydb.clone(), Key::generate()))

app.wrap(
    SessionMiddleware::builder(memorydb.clone(), key)
        .cookie_name(String::from("session"))
        .cookie_secure(false)                       // for local http testing
        .session_lifecycle(
            PersistentSession::default()            // cookie + state TTL
                .session_ttl(Duration::days(7))
                .session_ttl_extension_policy(TtlExtensionPolicy::OnEveryRequest),
        )
        .build(),
)
```

`Session` is an extractor and is available from `HttpRequest` / `ServiceRequest` / `ServiceResponse` / `GuardContext` through `SessionExt::get_session()`:
```
async fn handler(session: Session) -> RspResult<impl Responder> {
    session.insert("visited", true)?;
    if let Some(visited) = session.get::<bool>("visited")? { ... }
    Ok(Response::new(GeneralResponse::Ok))
}
```

### config
[config-rs](https://crates.io/crates/config) is the underlying library, re-exported as `actix_cloud::config`. Load your configuration and store it in `GlobalState.config`.

Supported features:
- config-yaml (Default): Support for YAML files.
- config-json: Support for JSON files.
- config-toml: Support for TOML files.

See the [config](examples/config) example for usage with serde and `validator`.

### request
Provide per-request extension.

Built-in middleware:
- Store in [extensions](https://docs.rs/actix-web/latest/actix_web/struct.HttpRequest.html#method.extensions_mut).
- Resolve the client real IP through a callback, defaulting to the peer address.
- If `i18n` feature is enabled, identify the language through the callback, or `locale.default` in `GlobalState`.
- If `traceid` feature is enabled, attach the trace id as a response header when `trace_header` is set.

Enable the built-in middleware:
```
let fallback = SocketAddr::new(std::net::Ipv4Addr::UNSPECIFIED.into(), 0);
app.wrap(
    request::Middleware::new()
        .real_ip(move |req| req.peer_addr().unwrap_or(fallback))
        .lang(|req| Some(String::from("en-US")))
        .trace_header("X-Trace-Id")
)
```

The middleware inserts an `Extension`, available in handlers:
```
async fn handler(req: HttpRequest) -> impl Responder {
    let ext = req.extensions();
    let ext = ext.get::<Arc<actix_cloud::request::Extension>>().unwrap();
    let (lang, ip, trace_id) = (&ext.lang, ext.real_ip, &ext.trace_id);
    ...
}

async fn handler(ext: ReqData<Arc<actix_cloud::request::Extension>>) -> impl Responder {
    ...
}
```

`Extension` fields: `start_time` (request start time), `real_ip` (client IP), `lang` (feature `i18n`), `trace_id` (feature `traceid`, generated by `TracingLogger`).

### response
Provide useful response types.

If `i18n` feature is enabled, response message will be translated automatically.

If `response-json` feature is enabled, `JsonResponse` serializes the body as JSON.

Build a response from a response code (see below) or a raw HTTP status code:
```
Response::new(GeneralResponse::Ok)                      // 200 + business code/message
Response::new(GeneralResponse::Ok).data(...)            // with payload
Response::new(GeneralResponse::Ok).builder(|rsp| { ... })   // customize the HttpResponseBuilder
Response::created(GeneralResponse::Ok, "/item/1")       // 201 + Location
Response::new_code(StatusCode::OK).message("hello")     // raw status code
Response::redirect(StatusCode::FOUND, "/login")         // 3xx + Location
```

Shortcuts: `Response::no_content()` (204), `Response::bad_request(msg)`, `Response::forbidden()`, `Response::not_found()`, and `Response::file(name, bytes)` for file downloads. Under the `i18n` feature, `.translate()` enables translation for messages set with `.message(...)`.

With feature `response-json`:
```
let data = map! {
    "key1" => "value1",
    "key2" => "value2"
};
JsonResponse::new(GeneralResponse::Ok).json(data)
```

On success (2xx) the body is `{"code": ..., "message": ..., "data": ...}` (`204` has no body and no JSON header). On failure the body is the message as-is.

Handlers can return `RspResult<T>`; `ResponseError` wraps an `anyhow::Error` and always responds with an **opaque** 500 (no error details leak to the client):
```
async fn page() -> RspResult<impl Responder> {
    let data = get_data()?;
    Ok(JsonResponse::new(GeneralResponse::Ok).json(data))
}
```

To generate response codes from YAML files:

1. Create response yml files, one enum per file (e.g. `response/general.yml` → `GeneralResponse`), each entry `name: {code, message}` — `message` is an i18n key:
```
Ok:
  code: 0
  message: "response.ok"
```
2. Use `build.rs` to generate source files (feature `response-build` as build dependency):
```
use actix_cloud::response_build::generate_response;

fn main() {
    generate_response("", "response", "response.rs").unwrap();
}
```
3. Use `include!` to include the generated files (the enums implement `ResponseCodeTrait` automatically):
```
include!(concat!(env!("OUT_DIR"), "/response.rs"));
```

See the [response](examples/response) example for detailed usage.

### traceid
Add trace ID for each request based on [tracing-actix-web](https://crates.io/crates/tracing-actix-web).

```
app.wrap(request::Middleware::new())
   .wrap(TracingLogger::default())      // This should be after request::Middleware
```

If you enable `request` feature, make sure `request::Middleware` is wrapped before `TracingLogger` since the `trace_id` field is based on it. Set `trace_header` on the request middleware to also write the trace id into a response header.

### csrf
We use [double submit](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html#alternative-using-a-double-submit-cookie-pattern) to protect against CSRF attacks.

The token is stored in a cookie and must be echoed in a header (or a query parameter, generally only for websocket). You can use `memorydb` to store and check CSRF tokens. CSRF will be checked AFTER the auth.

Enforcement is configured per route through `Router.csrf`:
- `CSRFType::Header` / `CSRFType::Param`: check all [unsafe](https://developer.mozilla.org/en-US/docs/Glossary/Safe/HTTP) methods; `Param` also accepts the token as a query parameter.
- `CSRFType::ForceHeader` / `CSRFType::ForceParam`: check every method, including safe ones.
- `CSRFType::Disabled`: never check this route.

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

### utils
- `is_unique(iter)` / `is_default(&t)`.
- `rand_string(n)` (`[a-zA-Z0-9]`), `rand_string_hex(n)` (`[a-f0-9]`), `rand_string_all(n)` (printable ascii).
- `restart()` — spawn a new process with the same arguments/environment.
- `load_rustls_config(cert, key)` (feature `rustls`) — load a rustls `ServerConfig`; only PKCS 8 private keys are supported.

## License
This project is licensed under the [MIT license](LICENSE).
