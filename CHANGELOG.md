# 0.4.14
## New
1. Default memory database now support basic capacity and key eviction.

# 0.4.13
## Changes
1. Default feature change.
2. Dependency update.

# 0.4.12
## Changes
1. Memory database support `ttl` function.

# 0.4.11
## Changes
1. Make clippy happy.
2. Bump `thiserror` to 2.0.

# 0.4.10
## Changes
1. `Locale::new` support `Into<String>` instead of `String`.

# 0.4.9
## New
1. New feature `serde` to add `Serialize`, `Deserialize` for supported structs.

# 0.4.8
## Changes
1. `handler` function can be async.

# 0.4.7
## Changes
1. Logger can configure `handler` through the function.

# 0.4.6
## Changes
1. Logger can use `handler` to configure custom handlers.

# 0.4.5
## Changes
1. Make features to be more configurable to reduce compilation size.

# 0.4.4
## Changes
1. Dependencies do not rely on exact version.

# 0.4.3
## Changes
1. Update dependencies.

# 0.4.2
## Changes
1. entity_timestamp macro need to import `chrono` manually.

# 0.4.1
## Fix
1. rustls provider will be set by default.

# 0.4.0
## New
1. `generate_response` now support custom import prefix.
2. Feature: `all`.

## Fix
1. `actix_cloud::main` can be used as re-export.
2. `chrono` is re-exported.
3. Middleware error can be handled correctly.

# 0.3.0
## Breaking changes
1. `Logger::init` no longer consumes builder.
2. `GlobalState::logger` is now optional.
3. `LoggerBuilder::start` will return a guard to consume all logs when dropped.
4. `Router` now uses `Checker` trait to configure the checker.
5. `MemoryDB` is now wrapped with `Arc`.
6. `SessionStore` is now object safe.
7. `Extension` is now wrapped with `Arc`.

## New
1. utils: `load_rustls_config`.
2. Feature: `seaorm`, `csrf`.
3. `MemoryDB` now supports `keys` and `dels`.
4. `request::Middleware` now supports `trace_header` for trace id.
5. `Extension::lang` can be identified through the custom callback.
6. `Session` now support override `session_key`.

## Fix
1. Logger can consume all incoming logs when exited.

# 0.2.0
## Breaking changes
1. `Error` is using `anyhow::Error` as the backend.
2. `StopHandle` is renamed to `ServerHandle`.
3. `GlobalState` is now object safe.
4. `memorydb` is now optional.

## New
1. utils: `restart`.
2. Start/stop timestamp and server status are stored in `GlobalState::ServerHandle`.
3. Feature: `config`, `request`, `response`, `traceid`.

# 0.1.0
First release.