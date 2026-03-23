SHELL := /bin/bash

APP := code-obfuscator
TARGETS := x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-pc-windows-gnu x86_64-apple-darwin aarch64-apple-darwin

.PHONY: help build install dev fmt lint clippy test unit integration install-e2e e2e e2e-blackbox svt coverage release-cross release-artifacts ci mcp-docker-build mcp-docker-run clean

help: ## List available targets
	@awk 'BEGIN {FS = ":.*## "; printf "Available targets:\n"} /^[a-zA-Z0-9_.-]+:.*## / {printf "  %-18s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

build: ## Build the project
	cargo build

install: ## Build and install via the managed installer lifecycle
	./install --from-source

dev: ## Show CLI help for local development
	cargo run -- --help

fmt: ## Format all Rust code
	cargo fmt --all

lint: fmt clippy ## Run formatting and linting

clippy: ## Run clippy with warnings denied
	cargo clippy --all-targets --all-features -- -D warnings

test: unit integration install-e2e e2e ## Run the main automated test suites

unit: ## Run unit tests for binaries
	cargo test --bins

integration: ## Run MCP server integration tests
	cargo test --test mcp_server

install-e2e: ## Run installer lifecycle tests
	cargo test --test install_script

e2e: ## Run CLI end-to-end tests
	cargo test --test e2e

e2e-blackbox: ## Run the black-box shell scenario
	bash test/e2e_blackbox.sh

svt: ## Run ignored stress-validation tests
	cargo test --test svt -- --ignored

coverage: ## Generate coverage report
	cargo llvm-cov --workspace --all-features --fail-under-lines 70 --lcov --output-path coverage.lcov

release-cross: ## Build release binaries for supported targets
	@for t in $(TARGETS); do \
		cargo build --release --target $$t; \
	done

release-artifacts: release-cross ## Package release archives expected by the installer
	@mkdir -p dist
	@for t in $(TARGETS); do \
		if [ -f target/$$t/release/$(APP) ]; then ./scripts/package-release.sh $$t target/$$t/release/$(APP) dist; fi; \
		if [ -f target/$$t/release/$(APP).exe ]; then ./scripts/package-release.sh $$t target/$$t/release/$(APP).exe dist; fi; \
	done

ci: lint test coverage ## Run CI-equivalent local checks

mcp-docker-build: ## Build the local MCP Docker image
	docker build -f docker/mcp.Dockerfile -t code-obfuscator-mcp:local .

mcp-docker-run: mcp-docker-build ## Run the local MCP Docker image
	docker run --rm -i code-obfuscator-mcp:local

clean: ## Remove Cargo build artifacts
	cargo clean
