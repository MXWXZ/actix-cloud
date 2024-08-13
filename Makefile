SHELL = /bin/bash

.ONESHELL:
.PHONY: check help

all: help

## check: Check code and style.
check:
	@cargo clippy -- -D clippy::all
	@cargo fmt --all -- --check

## help: Show this help.
help: Makefile
	@echo Usage: make [command]
	@sed -n 's/^##//p' $< | column -t -s ':' |  sed -e 's/^/ /'
