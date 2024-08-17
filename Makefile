SHELL = /bin/bash

.ONESHELL:
.PHONY: check help doc test

all: help

## check: Check code and style.
check:
	@cargo clippy --all-features -- -D clippy::all
	@cargo fmt --all -- --check

## doc: Test documents.
doc:
	@RUSTDOCFLAGS="--cfg docsrs" cargo +nightly doc --no-deps --all-features --open

## test: Run tests.
test:
	@cargo test --all-features

## help: Show this help.
help: Makefile
	@echo Usage: make [command]
	@sed -n 's/^##//p' $< | column -t -s ':' |  sed -e 's/^/ /'
