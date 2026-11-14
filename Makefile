SHELL = /bin/bash
# .ONESHELL runs the whole recipe in one shell; without -e make only sees the
# last command's status and earlier failures are silently ignored.
.SHELLFLAGS := -eu -o pipefail -c

.ONESHELL:
.PHONY: check help doc test test_dev

all: help

## check: Check code and style.
check:
	@cargo clippy --all-features --all-targets -- -D warnings
	@cargo fmt --all -- --check

## doc: Test documents.
doc:
	@RUSTDOCFLAGS="--cfg docsrs" cargo +nightly doc --no-deps --all-features --open

## test_dev: Run tests for development.
test_dev:
	@cargo test --all-features

## test: Run tests.
test:
	@cargo test --all-features -- --skip _redis

## help: Show this help.
help: Makefile
	@echo Usage: make [command]
	@sed -n 's/^##//p' $< | column -t -s ':' |  sed -e 's/^/ /'
