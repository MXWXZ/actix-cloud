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