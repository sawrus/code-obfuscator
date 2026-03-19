SHELL := /bin/bash

APP := code-obfuscator
TARGETS := x86_64-unknown-linux-gnu x86_64-pc-windows-gnu x86_64-apple-darwin

.PHONY: build fmt clippy test unit integration e2e e2e-blackbox svt coverage release-cross release-artifacts ci mcp-docker-build mcp-docker-run clean

build:
	cargo build

fmt:
	cargo fmt --all

clippy:
	cargo clippy --all-targets --all-features -- -D warnings

test: unit integration e2e

unit:
	cargo test --bins

integration:
	cargo test --test mcp_server

e2e:
	cargo test --test e2e

e2e-blackbox:
	bash test/e2e_blackbox.sh

svt:
	cargo test --test svt -- --ignored

coverage:
	cargo llvm-cov --workspace --all-features --fail-under-lines 70 --lcov --output-path coverage.lcov

release-cross:
	@for t in $(TARGETS); do \
		cargo build --release --target $$t; \
	done

release-artifacts: release-cross
	@mkdir -p dist
	@for t in $(TARGETS); do \
		if [ -f target/$$t/release/$(APP) ]; then cp target/$$t/release/$(APP) dist/$(APP)-$$t; fi; \
		if [ -f target/$$t/release/$(APP).exe ]; then cp target/$$t/release/$(APP).exe dist/$(APP)-$$t.exe; fi; \
	done

ci: fmt clippy test coverage

clean:
	cargo clean


mcp-docker-build:
	docker build -f docker/mcp.Dockerfile -t code-obfuscator-mcp:local .

mcp-docker-run: mcp-docker-build
	docker run --rm -i code-obfuscator-mcp:local
