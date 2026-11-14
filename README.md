# actix-cloud

[![crates.io](https://img.shields.io/crates/v/actix-cloud?label=crates.io&style=flat-square)](https://crates.io/crates/actix-cloud)
[![docs.rs](https://img.shields.io/docsrs/actix-cloud?style=flat-square)](https://docs.rs/actix-cloud/latest)
![license](https://img.shields.io/github/license/mxwxz/actix-cloud?style=flat-square)

**English** | [简体中文](README_CN.md)

Actix Cloud is an all-in-one web framework based on [Actix Web](https://crates.io/crates/actix-web).

## Installation

```toml
[dependencies]
actix-cloud = "0.8"
```

Requires Rust 1.89 or newer. Every module is behind a cargo feature, so you can enable only what you use, swap in your own backends, or combine it with other libraries. Features marked *(default)* are enabled by default; *(optional)* ones must be enabled explicitly. The `all` feature enables everything.

```toml
actix-cloud = { version = "0.8", default-features = false, features = [
    "logger", "state", "response-json",
] }
```

## Features

| Feature                                       | Default             | Description                                                                   |
| --------------------------------------------- | ------------------- | ----------------------------------------------------------------------------- |
| [logger](#logger)                             | yes                 | Tracing-based logger with color and JSON writers                              |
| [i18n](#i18n)                                 | yes                 | Compile-time locales, `t!`, translated responses (enables `state`, `request`) |
| [security](#security)                         | yes                 | Security headers middleware                                                   |
| [state](#state)                               | yes                 | `GlobalState` and `ServerHandle`                                              |
| [request](#request)                           | yes                 | Per-request `Extension` (language, real IP, trace id)                         |
| [traceid](#traceid)                           | yes                 | Trace ids via `tracing-actix-web`                                             |
| [auth](#auth)                                 | yes                 | `Checker` auth and `build_router` (feature `router`)                          |
| [session](#session)                           | yes                 | Session middleware backed by `memorydb`                                       |
| [csrf](#csrf)                                 | yes                 | Double-submit CSRF checks (enables `router`)                                  |
| [memorydb](#memorydb-default)                 | via `session`       | Key-value abstraction with an in-process backend                              |
| [redis](#memorydb-redis)                      | no                  | Redis backend for `memorydb`                                                  |
| [response](#response)                         | via `response-json` | Uniform `Response` type                                                       |
| `response-json`                               | yes                 | JSON `JsonResponse` (enables `response`)                                      |
| `response-build`                              | no                  | YAML → response enum codegen for `build.rs`                                   |
| [config](#config)                             | via `config-yaml`   | Re-exported `config` crate                                                    |
| `config-yaml` / `config-json` / `config-toml` | YAML only           | Configuration file formats                                                    |
| [utils](#utils)                               | yes                 | Random strings, restart, rustls config                                        |
| `macros`                                      | yes                 | `#[actix_cloud::main]` and compile-time `i18n!`                               |
| `rustls`                                      | yes                 | rustls helpers and re-export                                                  |
| `all`                                         | no                  | Enable every feature                                                          |

## Quick start

```rust
use std::{io, sync::Arc};

use actix_cloud::{
    actix_web::{web, App, HttpServer, Responder},
    config::Config,
    i18n::Locale,
    logger::LoggerBuilder,
    memorydb::default::DefaultBackend,
    state::{GlobalState, ServerHandle},
};

async fn hello() -> impl Responder {
    "Hello world!"
}

#[actix_cloud::main]
async fn main() -> io::Result<()> {
    let (logger, _guard) = LoggerBuilder::new().start();

    let state = GlobalState {
        memorydb: Arc::new(DefaultBackend::new()),
        config: Config::default(),
        logger: Some(logger),
        locale: Locale::new("en-US"),
        server: ServerHandle::default(),
    }
    .build();

    let state_cloned = state.clone();
    let server = HttpServer::new(move || {
        App::new()
            .app_data(state_cloned.clone())
            .route("/", web::get().to(hello))
    })
    .bind(("127.0.0.1", 8080))?
    .run();

    state.server.start(server).await.unwrap();
    Ok(())
}
```

See the [hello_world](examples/hello_world/) example for routing, auth, security headers and middleware ordering.

## Guide

### state
`GlobalState` is the shared application state, stored as actix-web `Data<GlobalState>` and attached with `.app_data(...)`:

```rust
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

```rust
let state = GlobalState {
    memorydb: Arc::new(DefaultBackend::new()),
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
- `running`, `start_time` and `stop_time` expose the server status.

### logger
We use [tracing](https://crates.io/crates/tracing) as our logger library. It is thread safe, so you can use it anywhere.

Start the logger:

```rust
let (logger, guard) = LoggerBuilder::new()
    .level(Level::DEBUG)
    .filename()
    .line_number()
    .start();
```

- Default output is colorful at `INFO` level. Use `.json()` for JSON-lines output.
- `WARN`/`ERROR` are written to stderr, other levels to stdout.
- Message text and field/span values are escaped before they reach the color writer, so untrusted content cannot forge log lines or inject terminal escape sequences. Use `logger::escape_control` for your own terminal output.
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
| `json_writer` / `color_writer`        | Custom writers (`Fn(LogItem, Box<dyn Write>) -> io::Result<()>`).         |

Wait until every queued log entry has been written out:

```rust
actix_cloud::logger::flush();
```

Fatal macros — log an error and exit the process with code 1 (queued logs are flushed first):

```rust
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

```yaml
hello.world: Hello world.
hello.name: Hello %{name}.
```

Load the locale. `i18n!` globs `**/*.yml` under the given path at compile time:

```rust
let locale = Locale::new("en-US").add_locale(i18n!("locale"));
```

Translate with `t!`, falling back to the default locale, then to the key itself:

```rust
t!(locale, "hello.world")                          // default locale
t!(locale, "hello.world", "zh-CN")                 // specific locale
t!(locale, "hello.name", name = "MEME")            // variables
t!(locale, "hello.name", "zh-CN", name = "MEME")   // both
```

`map!{"key" => "value"}` is a helper macro to create a `BTreeMap`, useful when building response data.

For web requests, register the locale in `GlobalState` and identify the request language with the `request::Middleware` callback. The identified language is stored in `Extension.lang` and used to translate `Response` messages; returning `None` falls back to `locale.default`:

```rust
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

```rust
app.wrap(SecurityHeader::default().build())
```

Default headers:

```text
X-Content-Type-Options: nosniff
Referrer-Policy: strict-origin-when-cross-origin
X-Frame-Options: DENY
Cross-Origin-Opener-Policy: same-origin
Content-Security-Policy: default-src 'none'; script-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'
```

Handlers can override individual headers by setting them on the response.

Enable HSTS when using HTTPS:

```rust
let mut header = SecurityHeader::default();
header.set_default_hsts();  // max-age=31536000; includeSubDomains; preload
app.wrap(header.build())
```

HSTS is opt-in because sending it over plain HTTP would lock clients out of the site. Only enable it on HTTPS deployments, and only use `preload` once you are ready for the domain to be added to browser preload lists.

### memorydb-default
Actix Cloud has a default in-process memory database backend used for sessions. You can also use your own backend by implementing the object-safe `MemoryDB` trait. Backends are shared as `Arc<dyn MemoryDB>`.

**The default memorydb is only for demo purpose. The performance is not guaranteed.**

`DefaultBackend::new()` is **unbounded**: entries stay in memory until they expire or are overwritten. Session middleware writes every non-empty session into this backend, so a deployment reachable by untrusted clients must configure `max_entries` and/or `max_bytes` — otherwise anonymous requests can keep creating sessions until memory is exhausted. `max_bytes` counts logical payload (`key.len() + value.len()`), not the allocator overhead of the stored strings and hash table, so many small entries use more memory than the counter reports; setting `max_entries` as well is recommended.

```rust
DefaultBackend::new()                              // No resource limitation, DDoS is possible!
DefaultBackend::new().max_entries(Some(100000000)) // Maximum 100000000 entries.
DefaultBackend::new().max_bytes(Some(100000000))   // Maximum 100000000 bytes of key+value payload.
DefaultBackend::new().max_bytes(Some(100000000)).bytes_evict_ratio(0.2) // Reclaim 20% of the byte limit per GC.
```

Both limits are optional and can be combined. `max_entries` is enforced when a write adds a new key; overwriting an existing key does not change the entry count, so only `max_bytes` applies to overwrites. `max_bytes` counts `key.len() + value.len()` UTF-8 bytes; internal allocations and TTL metadata are not counted.

When a write would exceed a configured limit, key eviction policy is:
- Evict a batch of 10% of the exceeded limit at a time (at least 1 entry or 1 byte). The ratio is configurable per limit with `entry_evict_ratio` / `bytes_evict_ratio` (default `0.1`, clamped to `0.0..=1.0`).
- Expired keys go first, then keys having a TTL attribute, sorted by TTL from shortest to longest.
- Keys without a TTL are never evicted.
- A single key+value larger than `max_bytes` is rejected without evicting anything.
- If the write cannot be accommodated, a `MemoryDBError` is returned.

The internal implementation uses lazy deletion for performance, so expired entries that have not been reaped yet still count toward the limits.

### memorydb-redis
Redis can be used as another backend for the memory database. The connection reconnects automatically.

```rust
RedisBackend::new("redis://user:pass@127.0.0.1:6379/0").await.unwrap(),
```

### auth
Authentication is quite simple: implement a checker.

Return `Ok(())` to allow the request, or an `Err` to reject it. The error is returned to the client as-is, so you decide the status code with `ErrorUnauthorized`, `ErrorForbidden`, etc.:

```rust
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

```rust
Router {
    path: String::from("/admin"),
    route: get().to(admin_page),
    checker: Some(Rc::new(AuthChecker::new(true))),
    csrf: CSRFType::Header,     // feature `csrf`
}
```

Configure the routers with `build_router` on a scope:

```rust
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
Most features and usages are based on [actix-session](https://crates.io/crates/actix-session), except for these differences:
- `MemoryDB` is the only supported storage.
- You can set `_ttl` (milliseconds) in the session to override the TTL of the session (and the cookie's `Max-Age`).
- You can set `_id` in the session for reverse search.
  - Quotes (`"`) are trimmed.
  - Another key is set in memorydb: `{_id}_{session_key}`. You can use `keys` to find all session keys bound to a specific id (e.g. `keys("{id}_*")`).
- Duplicated session cookies are rejected rather than silently using the first one.

Create the middleware. Use a fixed key to keep sessions across restarts; `Key::generate()` creates a new one for every restart:

```rust
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

`Session` is an extractor and is available from `HttpRequest` / `ServiceRequest` / `ServiceResponse` / `GuardContext` through `SessionExt::get_session()`. Use `Session::new()` for a standalone session outside a request:

```rust
async fn handler(session: Session) -> RspResult<impl Responder> {
    session.insert("visited", true)?;
    if let Some(visited) = session.get::<bool>("visited")? { ... }
    Ok(Response::new(GeneralResponse::Ok))
}
```

### config
[config-rs](https://crates.io/crates/config) is the underlying library, re-exported as `actix_cloud::config`. Load your configuration and store it in `GlobalState.config`.

Supported formats:
- `config-yaml` (default): YAML files.
- `config-json`: JSON files.
- `config-toml`: TOML files.

See the [config](examples/config) example for usage with serde and `validator`.

### request
Provide per-request extension data.

Built-in middleware:
- Stores the `Extension` in [extensions](https://docs.rs/actix-web/latest/actix_web/struct.HttpRequest.html#method.extensions_mut).
- Resolves the client real IP through a callback, defaulting to the peer address.
- If the `i18n` feature is enabled, identifies the language through the callback, or `locale.default` in `GlobalState`.
- If the `traceid` feature is enabled, attaches the trace id as a response header when `trace_header` is set.

Enable the built-in middleware:

```rust
let fallback = SocketAddr::new(std::net::Ipv4Addr::UNSPECIFIED.into(), 0);
app.wrap(
    request::Middleware::new()
        .real_ip(move |req| req.peer_addr().unwrap_or(fallback))
        .lang(|req| Some(String::from("en-US")))
        .trace_header("X-Trace-Id")
)
```

`real_ip` defaults to the peer address. Behind a reverse proxy every request appears to come from the proxy, so apps often read `X-Real-IP` / `X-Forwarded-For` instead. Only trust those headers when the peer address belongs to a trusted proxy and the proxy overwrites client-supplied values; otherwise clients can forge their source IP (e.g. to bypass IP-based rate limits).

The middleware inserts an `Extension`, available in handlers:

```rust
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

With the `i18n` feature, response messages are translated automatically. With the `response-json` feature, `JsonResponse` serializes the body as JSON.

Build a response from a response code (see below) or a raw HTTP status code:

```rust
Response::new(GeneralResponse::Ok)                      // 200 + business code/message
Response::new(GeneralResponse::Ok).data(...)            // with payload
Response::new(GeneralResponse::Ok).builder(|rsp| { ... })   // customize the HttpResponseBuilder
Response::created(GeneralResponse::Ok, "/item/1")       // 201 + Location
Response::new_code(StatusCode::OK).message("hello")     // raw status code
Response::redirect(StatusCode::FOUND, "/login")         // 3xx + Location
```

Shortcuts: `Response::no_content()` (204), `Response::bad_request(msg)`, `Response::forbidden()`, `Response::not_found()`, and `Response::file(name, bytes)` for file downloads. Under the `i18n` feature, `.translate()` enables translation for messages set with `.message(...)`.

With the `response-json` feature:

```rust
let data = map! {
    "key1" => "value1",
    "key2" => "value2"
};
JsonResponse::new(GeneralResponse::Ok).json(data)
```

On success (2xx) the body is `{"code": ..., "message": ..., "data": ...}` (`204` has no body and no JSON header). On failure the body is the plain-text message.

Handlers can return `RspResult<T>`; `ResponseError` wraps any standard error and always responds with an **opaque** 500 (no error details leak to the client):

```rust
async fn page() -> RspResult<impl Responder> {
    let data = get_data()?;
    Ok(JsonResponse::new(GeneralResponse::Ok).json(data))
}
```

To generate response codes from YAML files:

1. Create response YAML files, one enum per file (e.g. `response/general.yml` → `GeneralResponse`), each entry `name: {code, message}` — `message` is an i18n key:

```yaml
Ok:
  code: 0
  message: "response.ok"
```

2. Use `build.rs` to generate the source file (`response-build` as a build dependency):

```rust
use actix_cloud::response_build::generate_response;

fn main() {
    generate_response("", "response", "response.rs").unwrap();
}
```

3. Use `include!` to include the generated file (the enums implement `ResponseCodeTrait` automatically):

```rust
include!(concat!(env!("OUT_DIR"), "/response.rs"));
```

See the [response](examples/response) example for detailed usage.

### traceid
Add a trace ID to each request based on [tracing-actix-web](https://crates.io/crates/tracing-actix-web).

```rust
app.wrap(request::Middleware::new())
   .wrap(TracingLogger::default())      // This should be after request::Middleware
```

If you enable the `request` feature, make sure `request::Middleware` is wrapped before `TracingLogger`, since the `trace_id` field is based on it. Set `trace_header` on the request middleware to also write the trace id into a response header.

### csrf
We use the [double submit cookie](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html#alternative-using-a-double-submit-cookie-pattern) pattern to protect against CSRF attacks.

The token is stored in a cookie and must be echoed in a header (or a query parameter, generally only for websocket). The framework performs the transport-level double-submit check only: whether a matching token is actually valid for the user (e.g. bound to the session) is application-specific and belongs in the `checker` callback. You can use `memorydb` to store and check CSRF tokens there. CSRF is checked after auth.

Duplicated or empty CSRF cookies, headers and query parameters are rejected. `Param` tokens are carried in the URL and can leak through `Referer`, logs and browser history, so prefer `Header`; a `__Host-` cookie prefix and token rotation after authentication are recommended.

Enforcement is configured per route through `Router.csrf`:
- `CSRFType::Header` / `CSRFType::Param`: check all [unsafe](https://developer.mozilla.org/en-US/docs/Glossary/Safe/HTTP) methods; `Param` also accepts the token as a query parameter.
- `CSRFType::ForceHeader` / `CSRFType::ForceParam`: check every method, including safe ones.
- `CSRFType::Disabled`: never check this route.

Generally, `Param` and `ForceParam` should only be used for websocket.

```rust
build_router(
    route,
    csrf::Middleware::new(
        String::from("CSRF_TOKEN"),     // csrf cookie
        String::from("X-CSRF-Token"),   // csrf header/param
        // The checker is where the application validates the token
        // (e.g. binds it to the session); see module docs.
        |req, token| Box::pin(async { Ok(true) })
    ),
);
```

### utils
- `is_unique(iter)` / `is_default(&t)`.
- `rand_string(n)` (`[a-zA-Z0-9]`), `rand_string_hex(n)` (`[a-f0-9]`), `rand_string_all(n)` (printable ASCII).
- `restart()` — spawn a new process with the same arguments/environment.
- `load_rustls_config(cert, key)` (feature `rustls`) — load a rustls `ServerConfig`; only PKCS 8 private keys are supported.

## Development

```bash
make check     # clippy + rustfmt
make test      # tests that do not need Redis
make test_dev  # all tests, needs Redis on 127.0.0.1:6379
make doc       # docs.rs-style documentation
```

## License
This project is licensed under the [MIT license](LICENSE).
