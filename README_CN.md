# actix-cloud

[![crates.io](https://img.shields.io/crates/v/actix-cloud?label=crates.io&style=flat-square)](https://crates.io/crates/actix-cloud)
[![docs.rs](https://img.shields.io/docsrs/actix-cloud?style=flat-square)](https://docs.rs/actix-cloud/latest)
![license](https://img.shields.io/github/license/mxwxz/actix-cloud?style=flat-square)

[English](README.md) | **简体中文**

Actix Cloud 是一个基于 [Actix Web](https://crates.io/crates/actix-web) 的全功能 Web 框架。

## 安装

```toml
[dependencies]
actix-cloud = "0.8"
```

需要 Rust 1.89 或更高版本。每个模块都由 cargo feature 控制，可以只启用需要的部分、替换自定义后端，或与其他库组合使用。标记为 *(默认)* 的 feature 默认启用，*(可选)* 的需要显式开启；`all` 会启用全部 feature。

```toml
actix-cloud = { version = "0.8", default-features = false, features = [
    "logger", "state", "response-json",
] }
```

## 功能

| Feature | 默认 | 说明 |
| --- | --- | --- |
| [logger](#logger) | 是 | 基于 tracing 的日志，支持彩色和 JSON 输出 |
| [i18n](#i18n) | 是 | 编译期加载语言文件、`t!`、响应翻译（自动启用 `state`、`request`） |
| [security](#security) | 是 | 安全响应头中间件 |
| [state](#state) | 是 | `GlobalState` 和 `ServerHandle` |
| [request](#request) | 是 | 请求级 `Extension`（语言、真实 IP、trace id） |
| [traceid](#traceid) | 是 | 基于 `tracing-actix-web` 的 trace id |
| [auth](#auth) | 是 | `Checker` 鉴权与 `build_router`（feature `router`） |
| [session](#session) | 是 | 基于 `memorydb` 的会话中间件 |
| [csrf](#csrf) | 是 | 双提交 CSRF 校验（自动启用 `router`） |
| [memorydb](#memorydb-default) | 随 `session` | 键值抽象与进程内后端 |
| [redis](#memorydb-redis) | 否 | `memorydb` 的 Redis 后端 |
| [response](#response) | 随 `response-json` | 统一 `Response` 类型 |
| `response-json` | 是 | JSON 响应 `JsonResponse`（自动启用 `response`） |
| `response-build` | 否 | 在 `build.rs` 中把 YAML 生成为响应枚举 |
| [config](#config) | 随 `config-yaml` | 重新导出的 `config` crate |
| `config-yaml` / `config-json` / `config-toml` | 仅 YAML | 配置文件格式 |
| [utils](#utils) | 是 | 随机字符串、重启、rustls 配置 |
| `macros` | 是 | `#[actix_cloud::main]` 与编译期 `i18n!` |
| `rustls` | 是 | rustls 辅助函数与重新导出 |
| `all` | 否 | 启用全部 feature |

## 快速开始

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

完整的路由、鉴权、安全头和中间件顺序示例见 [hello_world](examples/hello_world/)。

## 指南

### state
`GlobalState` 是共享的应用状态，以 actix-web `Data<GlobalState>` 的形式通过 `.app_data(...)` 挂载：

```rust
GlobalState {
    memorydb: Arc<dyn MemoryDB>,                // feature `memorydb`
    config: config::Config,                     // feature `config`
    logger: Option<Logger>,                     // feature `logger`
    locale: Locale,                             // feature `i18n`
    server: ServerHandle,                       // 始终存在
}
.build()    // -> Data<GlobalState>
```

完整构造示例：

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

`ServerHandle` 管理服务器生命周期：
- `start(server)` 保存句柄并阻塞到服务器退出。
- `stop(graceful)` 发送停止信号。
- `running`、`start_time`、`stop_time` 暴露服务器状态。

### logger
日志底层使用 [tracing](https://crates.io/crates/tracing)，线程安全，可以在任何地方使用。

启动日志：

```rust
let (logger, guard) = LoggerBuilder::new()
    .level(Level::DEBUG)
    .filename()
    .line_number()
    .start();
```

- 默认彩色输出，`INFO` 级别；`.json()` 切换为 JSON lines。
- `WARN`/`ERROR` 写入 stderr，其他级别写入 stdout。
- 消息文本和字段/span 值在进入彩色 writer 前会做转义，不可信内容无法伪造日志行或注入终端控制序列；自己的终端输出可以使用 `logger::escape_control`。
- 必须让 `LoggerGuard` 存活：它被 drop 时会刷新剩余日志并停止日志线程。

Builder 选项：

| 方法 | 说明 |
| --- | --- |
| `level(Level)` | 最大日志级别，默认 `INFO`。 |
| `json()` | 使用 JSON writer 代替彩色 writer。 |
| `filename()` / `line_number()` | 在每条日志中记录文件/行号。 |
| `filter(Fn(&LogItem) -> bool)` | 返回 `false` 丢弃该条日志。 |
| `transformer(Fn(LogItem) -> LogItem)` | 在写入前改写日志。 |
| `handler(Fn(&Map) -> Future<bool>)` | 最先执行的原始 JSON handler，返回 `false` 跳过后续处理。 |
| `json_writer` / `color_writer` | 自定义 writer（`Fn(LogItem, Box<dyn Write>) -> io::Result<()>`）。 |

等待所有已排队的日志写完：

```rust
actix_cloud::logger::flush();
```

Fatal 宏——记录错误并以退出码 1 结束进程（会先刷新日志）：

```rust
fatal!("config file not found: {}", path);
let home = unwrap_fatal!(std::env::var("HOME"));
let home = expect_fatal!(std::env::var("HOME"), "HOME is not set");
```

可覆盖的保留字段：

| 字段 | 说明 |
| --- | --- |
| `_time` | 微秒时间戳，覆盖日志时间。 |
| `_filename` / `_line_number` | 覆盖记录的文件/行号（仅在启用 `filename()` / `line_number()` 时生效）。 |

### i18n
i18n 核心使用 [rust-i18n](https://crates.io/crates/rust-i18n) 的 `rust-i18n-support`。

语言文件是以 locale 命名的 YAML（例如 `locale/en-US.yml`），支持 `%{name}` 占位符：

```yaml
hello.world: Hello world.
hello.name: Hello %{name}.
```

加载语言文件。`i18n!` 会在编译期按给定路径 glob `**/*.yml`：

```rust
let locale = Locale::new("en-US").add_locale(i18n!("locale"));
```

使用 `t!` 翻译，回退顺序为：默认语言 → key 本身：

```rust
t!(locale, "hello.world")                          // 默认语言
t!(locale, "hello.world", "zh-CN")                 // 指定语言
t!(locale, "hello.name", name = "MEME")            // 变量
t!(locale, "hello.name", "zh-CN", name = "MEME")   // 两者同时
```

`map!{"key" => "value"}` 是创建 `BTreeMap` 的辅助宏，适合构造响应数据。

Web 请求中，把 locale 放进 `GlobalState`，并通过 `request::Middleware` 的回调识别请求语言。识别结果存入 `Extension.lang` 用于翻译 `Response` 消息；回调返回 `None` 时使用 `locale.default`：

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

更多用法见 [i18n](examples/i18n) 示例。

### security
添加安全响应头的中间件：

```rust
app.wrap(SecurityHeader::default().build())
```

默认响应头：

```text
X-Content-Type-Options: nosniff
Referrer-Policy: strict-origin-when-cross-origin
X-Frame-Options: DENY
Cross-Origin-Opener-Policy: same-origin
Content-Security-Policy: default-src 'none'; script-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'
```

handler 可以在响应上覆盖单独的响应头。

使用 HTTPS 时启用 HSTS：

```rust
let mut header = SecurityHeader::default();
header.set_default_hsts();  // max-age=31536000; includeSubDomains; preload
app.wrap(header.build())
```

HSTS 默认关闭：在纯 HTTP 上发送会让客户端无法再访问站点。只在 HTTPS 部署中启用；`preload` 要等确认域名需要加入浏览器预加载列表后再使用。

### memorydb-default
Actix Cloud 自带一个进程内存储后端，默认用于会话。也可以实现对象安全的 `MemoryDB` trait 使用自己的后端，框架内统一以 `Arc<dyn MemoryDB>` 共享。

**默认 memorydb 仅用于演示，不保证性能。**

`DefaultBackend::new()` 是**无上限**的：条目会一直留在内存中，直到过期或被覆盖。会话中间件会把每个非空会话写入该后端，因此任何面向不可信客户端的部署都必须配置 `max_entries` 和/或 `max_bytes`，否则匿名请求可以不断创建会话直到内存耗尽。`max_bytes` 只统计逻辑负载（`key.len() + value.len()`），不包含字符串和哈希表的分配开销，所以大量小条目实际占用会高于计数；建议同时设置 `max_entries`。

```rust
DefaultBackend::new()                              // 无任何限制，可能被 DDoS！
DefaultBackend::new().max_entries(Some(100000000)) // 最多 100000000 个条目。
DefaultBackend::new().max_bytes(Some(100000000))   // key+value 负载最多 100000000 字节。
DefaultBackend::new().max_bytes(Some(100000000)).bytes_evict_ratio(0.2) // 每次 GC 回收字节上限的 20%。
```

两个上限都是可选的，可以同时使用。`max_entries` 只在写入新 key 时检查；覆盖已有 key 不会改变条目数，因此只有 `max_bytes` 对覆盖写生效。`max_bytes` 统计 `key.len() + value.len()` 的 UTF-8 字节数，不包含内部结构和 TTL 元数据。

写入超过某个上限时的淘汰策略：
- 每次按超出上限的 10% 批量淘汰（至少 1 个条目或 1 字节）。比例可通过 `entry_evict_ratio` / `bytes_evict_ratio` 分别配置（默认 `0.1`，限制在 `0.0..=1.0`）。
- 先淘汰已过期条目，再按 TTL 从短到长淘汰带 TTL 的条目。
- 没有 TTL 的条目永不淘汰。
- 单条 key+value 超过 `max_bytes` 时直接拒绝，不做任何淘汰。
- 仍然无法容纳写入时返回 `MemoryDBError`。

内部使用惰性删除以提升性能，已经过期但尚未回收的条目仍然计入上限。

### memorydb-redis
Redis 可以作为另一个 memorydb 后端，连接会自动重连。

```rust
RedisBackend::new("redis://user:pass@127.0.0.1:6379/0").await.unwrap(),
```

### auth
鉴权很简单：实现一个 checker 即可。

返回 `Ok(())` 放行，返回 `Err` 拒绝。错误会原样返回给客户端，因此可以用 `ErrorUnauthorized`、`ErrorForbidden` 等自行决定状态码：

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

每个路由是一个 `Router`，包含路径、actix-web 路由、可选的 checker，以及（feature `csrf`）CSRF 校验级别：

```rust
Router {
    path: String::from("/admin"),
    route: get().to(admin_page),
    checker: Some(Rc::new(AuthChecker::new(true))),
    csrf: CSRFType::Header,     // feature `csrf`
}
```

用 `build_router` 把路由配置到 scope 上：

```rust
app.service(scope("/api").configure(build_router(vec![...])))

// 启用 `csrf` feature 时，同时传入 CSRF 中间件：
app.service(scope("/api").configure(build_router(vec![...], csrf::Middleware::new(
    String::from("CSRF_TOKEN"),     // csrf cookie
    String::from("X-CSRF-Token"),   // csrf header/param
    |req, token| Box::pin(async { Ok(true) })  // 额外的 csrf checker
))))
```

`path` 为空的路由会被跳过。鉴权 checker 总是先于 CSRF 执行。

### session
大部分功能和用法基于 [actix-session](https://crates.io/crates/actix-session)，区别如下：
- 只支持 `MemoryDB` 存储。
- 可以在会话中设置 `_ttl`（毫秒）覆盖会话 TTL（以及 cookie 的 `Max-Age`）。
- 可以在会话中设置 `_id` 用于反向查找。
  - 引号（`"`）会被去掉。
  - memorydb 中会额外写入一个 key：`{_id}_{session_key}`，可以用 `keys` 找到某个 id 绑定的所有会话 key（例如 `keys("{id}_*")`）。
- 重复的会话 cookie 会被拒绝，而不是静默使用第一个。

创建中间件。使用固定 key 可以在重启后保持会话；`Key::generate()` 每次重启都会生成新 key：

```rust
app.wrap(SessionMiddleware::new(memorydb.clone(), Key::generate()))

app.wrap(
    SessionMiddleware::builder(memorydb.clone(), key)
        .cookie_name(String::from("session"))
        .cookie_secure(false)                       // 本地 HTTP 测试用
        .session_lifecycle(
            PersistentSession::default()            // cookie 和状态 TTL
                .session_ttl(Duration::days(7))
                .session_ttl_extension_policy(TtlExtensionPolicy::OnEveryRequest),
        )
        .build(),
)
```

`Session` 是一个 extractor，也可以通过 `SessionExt::get_session()` 从 `HttpRequest` / `ServiceRequest` / `ServiceResponse` / `GuardContext` 获取。在请求之外可以使用 `Session::new()` 创建独立会话：

```rust
async fn handler(session: Session) -> RspResult<impl Responder> {
    session.insert("visited", true)?;
    if let Some(visited) = session.get::<bool>("visited")? { ... }
    Ok(Response::new(GeneralResponse::Ok))
}
```

### config
底层使用 [config-rs](https://crates.io/crates/config)，以 `actix_cloud::config` 重新导出。加载配置后放入 `GlobalState.config`。

支持的格式：
- `config-yaml`（默认）：YAML。
- `config-json`：JSON。
- `config-toml`：TOML。

结合 serde 和 `validator` 的用法见 [config](examples/config) 示例。

### request
提供请求级扩展数据。

内置中间件：
- 把 `Extension` 存入 [extensions](https://docs.rs/actix-web/latest/actix_web/struct.HttpRequest.html#method.extensions_mut)。
- 通过回调解析客户端真实 IP，默认使用连接的对端地址。
- 启用 `i18n` feature 时，通过回调识别语言，失败时使用 `GlobalState` 中的 `locale.default`。
- 启用 `traceid` feature 且设置了 `trace_header` 时，把 trace id 写入响应头。

启用内置中间件：

```rust
let fallback = SocketAddr::new(std::net::Ipv4Addr::UNSPECIFIED.into(), 0);
app.wrap(
    request::Middleware::new()
        .real_ip(move |req| req.peer_addr().unwrap_or(fallback))
        .lang(|req| Some(String::from("en-US")))
        .trace_header("X-Trace-Id")
)
```

`real_ip` 默认使用对端地址。反向代理之后，所有请求的对端都是代理，应用通常会改为读取 `X-Real-IP` / `X-Forwarded-For`。只有当对端地址属于受信代理、且代理会覆盖客户端自带的值时才信任这些头；否则客户端可以伪造来源 IP（例如绕过基于 IP 的限流）。

中间件会插入 `Extension`，handler 中可以直接读取：

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

`Extension` 字段：`start_time`（请求开始时间）、`real_ip`（客户端 IP）、`lang`（feature `i18n`）、`trace_id`（feature `traceid`，由 `TracingLogger` 生成）。

### response
提供统一的响应类型。

启用 `i18n` feature 时，响应消息会自动翻译；启用 `response-json` feature 时，`JsonResponse` 会把 body 序列化为 JSON。

可以用响应码（见下文）或原始 HTTP 状态码构造响应：

```rust
Response::new(GeneralResponse::Ok)                      // 200 + 业务码/消息
Response::new(GeneralResponse::Ok).data(...)            // 带数据
Response::new(GeneralResponse::Ok).builder(|rsp| { ... })   // 自定义 HttpResponseBuilder
Response::created(GeneralResponse::Ok, "/item/1")       // 201 + Location
Response::new_code(StatusCode::OK).message("hello")     // 原始状态码
Response::redirect(StatusCode::FOUND, "/login")         // 3xx + Location
```

快捷方法：`Response::no_content()`（204）、`Response::bad_request(msg)`、`Response::forbidden()`、`Response::not_found()`，以及用于文件下载的 `Response::file(name, bytes)`。启用 `i18n` 时，`.translate()` 可以让 `.message(...)` 设置的消息参与翻译。

启用 `response-json` feature 时：

```rust
let data = map! {
    "key1" => "value1",
    "key2" => "value2"
};
JsonResponse::new(GeneralResponse::Ok).json(data)
```

成功响应（2xx）的 body 为 `{"code": ..., "message": ..., "data": ...}`（`204` 无 body、无 JSON 头）。失败响应的 body 是纯文本消息。

handler 可以返回 `RspResult<T>`；`ResponseError` 可以包装任意标准错误，并且始终返回**不透明**的 500（不会向客户端泄漏错误详情）：

```rust
async fn page() -> RspResult<impl Responder> {
    let data = get_data()?;
    Ok(JsonResponse::new(GeneralResponse::Ok).json(data))
}
```

从 YAML 生成响应码：

1. 创建响应 YAML 文件，一个文件对应一个枚举（例如 `response/general.yml` → `GeneralResponse`），每项为 `name: {code, message}`，其中 `message` 是 i18n key：

```yaml
Ok:
  code: 0
  message: "response.ok"
```

2. 在 `build.rs` 中生成源码（把 `response-build` 作为 build dependency）：

```rust
use actix_cloud::response_build::generate_response;

fn main() {
    generate_response("", "response", "response.rs").unwrap();
}
```

3. 用 `include!` 引入生成的文件（枚举自动实现 `ResponseCodeTrait`）：

```rust
include!(concat!(env!("OUT_DIR"), "/response.rs"));
```

详细用法见 [response](examples/response) 示例。

### traceid
基于 [tracing-actix-web](https://crates.io/crates/tracing-actix-web) 为每个请求生成 trace ID。

```rust
app.wrap(request::Middleware::new())
   .wrap(TracingLogger::default())      // 应位于 request::Middleware 之后
```

如果启用了 `request` feature，请确保 `request::Middleware` 在 `TracingLogger` 之前包装，因为 `trace_id` 字段来自后者。在 request 中间件上设置 `trace_header` 可以把 trace id 同时写入响应头。

### csrf
使用[双提交 cookie](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html#alternative-using-a-double-submit-cookie-pattern)模式防御 CSRF。

token 存在 cookie 中，并必须在 header（或查询参数，一般仅用于 WebSocket）中回显。框架只负责传输层的双提交校验：token 对用户是否真正有效（例如是否绑定会话）属于应用逻辑，应在 `checker` 回调中完成。你可以在其中用 `memorydb` 存储和校验 CSRF token。CSRF 在鉴权之后检查。

重复或为空的 CSRF cookie、header、查询参数都会被拒绝。`Param` 模式的 token 会出现在 URL 中，可能通过 `Referer`、日志和浏览器历史泄漏，优先使用 `Header`；建议使用 `__Host-` cookie 前缀，并在认证后轮换 token。

通过 `Router.csrf` 为每个路由配置校验级别：
- `CSRFType::Header` / `CSRFType::Param`：检查所有[不安全](https://developer.mozilla.org/en-US/docs/Glossary/Safe/HTTP)方法；`Param` 额外接受查询参数中的 token。
- `CSRFType::ForceHeader` / `CSRFType::ForceParam`：所有方法都检查，包括安全方法。
- `CSRFType::Disabled`：该路由永不检查。

通常 `Param` 和 `ForceParam` 只应用于 WebSocket。

```rust
build_router(
    route,
    csrf::Middleware::new(
        String::from("CSRF_TOKEN"),     // csrf cookie
        String::from("X-CSRF-Token"),   // csrf header/param
        // checker 是应用校验 token 的地方（例如绑定到会话）；见模块文档。
        |req, token| Box::pin(async { Ok(true) })
    ),
);
```

### utils
- `is_unique(iter)` / `is_default(&t)`。
- `rand_string(n)`（`[a-zA-Z0-9]`）、`rand_string_hex(n)`（`[a-f0-9]`）、`rand_string_all(n)`（可打印 ASCII）。
- `restart()` — 使用相同参数和环境启动新进程。
- `load_rustls_config(cert, key)`（feature `rustls`）— 加载 rustls `ServerConfig`，仅支持 PKCS 8 私钥。

## 开发

```bash
make check     # clippy + rustfmt
make test      # 不需要 Redis 的测试
make test_dev  # 全部测试，需要 127.0.0.1:6379 上的 Redis
make doc       # docs.rs 风格的文档
```

## 许可证
本项目使用 [MIT 许可证](LICENSE)。
