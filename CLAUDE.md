# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

`actix-cloud` is an all-in-one web framework built on Actix Web, published to crates.io. Cargo workspace with two members:

- `actix-cloud/` — the framework itself
- `actix-cloud-codegen/` — proc macros (`i18n!` compile-time locale loading; `main` attribute that just emits `#[actix_web::rt::main(system = ...)]`)

`examples/` is **excluded from the workspace**; each example is a standalone crate referencing `actix-cloud` by path with explicit features. Run them with `cargo run` from inside the example directory.

## Commands

```bash
make check    # cargo clippy --all-features -- -D clippy::all && cargo fmt --all -- --check
make test     # cargo test --all-features -- --skip memorydb::interface::tests
make test_dev # cargo test --all-features (includes tests needing Redis on 127.0.0.1:6379)
make doc      # nightly, docsrs-style docs build
```

- The `memorydb::interface::tests` skip exists because those tests exercise all backends including Redis — they need a live Redis server. Use `make test` unless Redis is running.
- Single test: `cargo test --all-features <name>`; always pass a feature set (`--all-features` or `--no-default-features --features "..."`) since default features are a strict subset.
- CI runs on pushing a `v*` tag: `make check` + `make test`, then publishes `actix-cloud-codegen` first, then `actix-cloud`. Commits are simply the version number; every release updates `CHANGELOG.md` (breaking changes are called out under `## Breaking changes`).

## Architecture

### Feature gating is the defining constraint

Every module is behind a cargo feature, and features pull their own optional dependencies with the `dep:` syntax (e.g. `csrf = ["dep:qstring", "router", ...]`). Consequences:

- Dependencies are re-exported only under their feature: `#[cfg(feature = "actix-web")] pub use actix_web;` (see `src/lib.rs`). Do the same for any new dependency so downstream users don't get it transitively.
- Modules reference each other via cfg: e.g. `GlobalState` fields for memorydb/config/logger/locale each exist only when that feature is on. New cross-module code must be cfg-gated the same way.
- Code must compile under any valid feature combination; CI only checks `--all-features`, so mind the graph when touching feature definitions.

### Core abstractions and how they connect

- **`MemoryDB` trait** (`src/memorydb/interface.rs`) — object-safe async KV store (get/set, TTL via `Duration`, `keys`/`dels`). Backends: `default::DefaultBackend` (in-process, TTL-ordered eviction, lazy deletion — explicitly demo-grade) and `redis::RedisBackend` (feature `redis`). It's used as `Arc<dyn MemoryDB>` everywhere. `interface.rs` tests all backends through shared generic test fns — a new backend just gets plugged into those tests.
- **`GlobalState`** (`src/state.rs`) — the app-state struct users populate and attach via `.build() -> Data<GlobalState>`. Contains optional logger, config, locale, the shared memorydb, and `ServerHandle` (holds the actix server handle, start/stop timestamps, graceful stop).
- **Router/auth pipeline** (`src/router.rs` + `src/csrf.rs`) — users declare `Vec<Router>` (path + actix `Route` + optional `Rc<dyn Checker>` + `CSRFType`); `build_router(routers, csrf_middleware)` returns the closure for `scope().configure(...)`. Per route, middleware wraps as: **auth (`RouterGuard`) first, then CSRF**. `Checker` is `#[async_trait(?Send)]` because actix runs per-worker single-threaded; the same applies to `Router::checker` being `Rc`.
- **Session** (`src/session/`) — a fork of `actix-session` adapted to store in `MemoryDB` instead. Non-standard conventions: `_ttl` in the session overrides TTL (milliseconds), `_id` enables reverse lookup via an extra `{_id}_{session_key}` key in memorydb.
- **Request extension** (`src/request.rs`) — middleware inserting `Arc<Extension>` (language, real IP, trace_id) into request extensions. **Ordering: `request::Middleware` must be wrapped before `TracingLogger`** because it reads the trace id.
- **Response/i18n coupling** — `Response`/`JsonResponse` (`src/response.rs`) translate their message via the request's locale (feature `i18n`) and serialize to JSON (feature `response-json`). `response_build::generate_response` (feature `response-build`, a normal function for `build.rs`, not a proc macro) walks a directory of YAML files and generates a Rust enum of response codes — see `examples/response/build.rs` for the canonical usage.
- **Logger** (`src/logger.rs`) — tracing-based; `LoggerBuilder::start()` returns `(Logger, LoggerGuard)`. `Logger` hands out a channel `sender()` so plugins can feed log maps in (reserved `_time` field, µs); `LoggerGuard` flushes remaining logs on drop.

### Macros split

Runtime-facing macros `t!` and `map!` are `macro_rules!` in `src/i18n.rs`. Anything needing compilation-time file access lives in `actix-cloud-codegen` (`i18n!` globs `**/*.yml` locale files at compile time via `rust-i18n-support`). The two crates are versioned and published together: `actix-cloud` depends on codegen with both `path` and `version`, so local builds use the path but a published `actix-cloud` resolves the requirement from crates.io — hence codegen must be published first, with a version satisfying the requirement in `actix-cloud/Cargo.toml`.
