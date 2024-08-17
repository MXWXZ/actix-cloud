SHELL = /bin/bash

.ONESHELL:
.PHONY: check help doc test test_dev

all: help

## check: Check code and style.
check:
	@cargo clippy --all-features -- -D clippy::all
	@cargo fmt --all -- --check

## doc: Test documents.
doc:
	@RUSTDOCFLAGS="--cfg docsrs" cargo +nightly doc --no-deps --all-features --open

## test_dev: Run tests for development.
test_dev:
	@cargo test --all-features

## test: Run tests.
test:
	@cargo test --all-features -- --skip memorydb::interface::tests

## help: Show this help.
help: Makefile
	@echo Usage: make [command]
	@sed -n 's/^##//p' $< | column -t -s ':' |  sed -e 's/^/ /'
