SHELL := /bin/bash

APP := code-obfuscator
TARGETS := x86_64-unknown-linux-gnu x86_64-pc-windows-gnu x86_64-apple-darwin

.PHONY: build fmt clippy test unit e2e svt coverage release-cross clean

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

clean:
	cargo clean
