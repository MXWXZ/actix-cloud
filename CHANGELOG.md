# 0.6.4
## New
1. `fatal!` — logs a message at `ERROR` level, flushes all queued logs, then exits the process with code 1.
2. `logger::flush()` — waits until every log entry queued before the call has been written out.
3. `logger::FatalExt` — `unwrap_fatal()` and `expect_fatal()` for `Result<T, E>` (`E: Display`), which log the error and exit with code 1 on `Err`.

# 0.6.3
## Fix
1. `JsonResponse` no longer sets JSON content-type on 204 No Content responses.
2. `ServerHandle::stop` no longer panics when the server has never been started.
3. `request::Middleware` no longer panics when `GlobalState` is not registered, `TracingLogger` is not wrapped before it, or the peer address is absent; it falls back to default values instead.
4. `response_build::generate_response` now returns `BuildError::File` instead of panicking on invalid response file names.

## Changes
1. Documentation improvements across the crate.

# 0.6.2
## Fix
1. `JsonResponse` now does not add JSON headers when `data` is `None`.

# 0.6.1
## Breaking changes
1. `Response` methods now takes `StatusCode` instead of `u16`.

## New
1. `Response::created` — creates a 201 Created response with a Location header.
2. `Response::no_content` — creates a 204 No Content response.

## Changes
1. `JsonResponse::respond_to` now uses `is_success()` instead of `== 200`, so all 2xx codes get proper JSON body responses.
2. `JsonResponse::respond_to` now uses `finish()` (empty body) when the message is empty, instead of wrapping an empty string in a body.

# 0.6.0
## Breaking changes
1. `seaorm` feature has been moved to actix-cloud-extra.
2. Memorydb `expire` and `ttl` use `Duration` instead of seconds.
3. `X-XSS-Protection` is now dropped.
4. Memorydb now support millisecond TTL. `_ttl` in session is now milliseconds instead of seconds.

## Fix
1. `rand_string_hex` returns correct length for odd `n`.
2. `rand_string_all` can now produce `~`.
3. `Response` no longer panics on invalid HTTP code, returns 500 instead.

# 0.5.1
## Changes
1. Revert 0.5.0 `set_default_hsts` changes.

# 0.5.0
## Breaking changes
1. `set_default_hsts` is removed, now is default.

## New
1. Add `default` for `LoggerBuilder` and `Middleware`.

## Changes
1. Minor session change according to `actix-session`.
2. Fix several unexpected unwrap.

# 0.4.15
## New
1. `Response` allows for multiple `builder`.
2. `Response` add several convenient methods.

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