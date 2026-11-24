# 0.8.1
## New
1. `MemoryDB::incr`/`incr_by` and `decr`/`decr_by` atomically update `i64` counters. A missing or expired key starts at `0`, the key's TTL is preserved, and a non-integer value or an overflow returns the new `NotAnInteger`/`IntegerOverflow` errors without modifying the key.
2. `DefaultBackend::evict_expired_only` makes GC reap expired entries only; entries whose TTL has not expired are then never evicted (default `false`).

# 0.8.0
## Breaking changes
1. `DefaultBackend::new()` no longer takes a capacity argument; configure limits with `.max_entries(...)` and `.max_bytes(...)` instead. Both are unbounded by default.
2. `anyhow` is gone: `actix_cloud::{Error, Result, bail}` and the `anyhow` feature are removed. Errors now use typed enums. `ResponseError` wraps `Box<dyn Error + Send + Sync>`, and `Session`'s `FromRequest::Error` is now `Infallible`.
3. Fallible APIs return standard error types: `ServerHandle::start` and `utils::restart` return `io::Result`, logger writer callbacks return `io::Result<()>`, and `MemoryDB` methods return `MemoryDBResult`.
4. `MemoryDB` has a new required `rename` method; custom backends must implement it.

## New
1. `DefaultBackend` supports `max_entries`/`max_bytes` limits with a configurable eviction batch (`entry_evict_ratio`/`bytes_evict_ratio`, default `0.1`); entries larger than `max_bytes` are rejected.
2. `MemoryDB::rename` atomically moves a value and its remaining TTL to another key, replacing the destination.
3. `logger::escape_control` escapes control characters (C0, DEL and C1) in untrusted text for terminal output while keeping printable Unicode.
4. `Session::new()` creates a standalone empty session, useful in tests.

## Fix
1. `RedisBackend` now matches the default backend: zero-TTL `set_ex`/`get_ex` delete the key, invalid `keys` patterns return `InvalidPattern`, TTL overflow returns `TtlOutOfRange`, and `keys` no longer blocks the server on large databases.
2. Sessions: `update`/`update_or` persist changes to loaded sessions, purging removes the reverse-lookup entry, renewal rotates the key atomically, and duplicated session cookies are rejected.
3. CSRF rejects duplicated or empty cookies, headers and query parameters, compares tokens in constant time, and returns 500 instead of panicking when mounted outside `build_router`.
4. `JsonResponse` no longer panics on invalid builder headers and sends error messages as `text/plain; charset=utf-8`; `response_build` reports empty YAML files as `BuildError::Format`.
5. The color log writer escapes control characters in messages and field/span values, and `request::Middleware` skips invalid trace headers instead of panicking.
6. The `i18n` feature now enables `state` and `request` automatically.

# 0.7.3
## Fix
1. `build_router` now registers all routes for the same path on a single resource, so a request with an unregistered method returns 405 Method Not Allowed (with an `Allow` header) instead of 404 Not Found.
2. `DefaultBackend::del`/`dels` now treat expired keys as non-existent — `del` returns `false` and `dels` does not count them — matching Redis semantics.

## Changes
1. Documentation improvements and unit tests across the crate.

# 0.7.2
## Fix
1. Fix 0.7.1 bug.

# 0.7.1
## Changes
1. The default color logger now shows `http.method` and `http.target` span fields; other `http.*` span fields are still filtered out.

# 0.7.0
## Breaking changes
1. `router::Checker::check` now returns `Result<(), actix_web::Error>` instead of `Result<bool>`. Return `Ok(())` to allow the request, or an error (e.g. `ErrorUnauthorized`, `ErrorForbidden`) to reject it with that error's status code and message. The error is returned to the client as-is — the framework no longer generates 403/500 responses for failed checks.

## Changes
1. Error messages start with a lowercase letter.

# 0.6.7
## Fix
1. `DefaultBackend` no longer rejects writes when it is below the configured capacity.

## Changes
1. Overwriting an existing key no longer triggers eviction of other entries when the database is at capacity.

# 0.6.6
## Breaking changes
1. `logger::FatalExt` is removed — `unwrap_fatal()` / `expect_fatal()` are replaced by the `unwrap_fatal!` and `expect_fatal!` macros. They expand at the call site, so the log records the caller's module, file and line instead of the logger's own location.

## Changes
1. The reserved `_filename` and `_line_number` fields are now only applied when `LoggerBuilder::filename()` / `line_number()` are enabled.

# 0.6.5
## New
1. Log entries support the reserved `_filename` and `_line_number` fields to override the recorded file and line, like the existing `_time` field.

## Changes
1. `logger::FatalExt` — `unwrap_fatal()` and `expect_fatal()` now record the caller's file and line in the log entry instead of the line inside `logger.rs`.

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