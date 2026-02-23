SHELL := /bin/bash

APP := code-obfuscator
TARGETS := x86_64-unknown-linux-gnu x86_64-pc-windows-gnu x86_64-apple-darwin

.PHONY: build fmt clippy test unit e2e svt coverage release-cross release-artifacts ci clean

build:
	cargo build

fmt:
	cargo fmt --all

clippy:
	cargo clippy --all-targets --all-features -- -D warnings

test: unit e2e

unit:
	cargo test --bins

e2e:
	cargo test --test e2e

svt:
	cargo test --test svt -- --ignored

coverage:
	cargo llvm-cov --workspace --all-features --lcov --output-path coverage.lcov

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
