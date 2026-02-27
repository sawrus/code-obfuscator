# Changelog

## [0.4.0] - 2026-02-27

### Added
- English changelog entries for recent releases.

### Changed
- Package version bumped to `0.4.0`.

## [0.3.0] - 2026-02-24

### Added
- Additional unit tests validating deep identifier obfuscation in JavaScript, TypeScript, Java, and Go.
- A new e2e scenario with an explicit `mapping.json` that verifies identifier replacement in JavaScript, TypeScript, Java, C#, C++, Go, Rust, and Bash.
- Extended SVT coverage: the mixed-language stress test now validates actual mapping-driven replacements in output files.

### Changed
- Package version bumped to `0.3.0`.
- README cleaned up to remove user-specific private examples and keep only general capability descriptions.

## [0.2.0] - 2026-02-23

### Added
- Language-aware detection of obfuscation terms for 10 languages: Python, JavaScript, TypeScript, Java, C#, C/C++, Go, Rust, SQL, Bash.
- Deep SQL obfuscation coverage for table and column identifiers including qualified references (`r.user_id`).
- Deep Python obfuscation for snake_case/method/constant identifiers via mapping, including env-style names.
- Safety guard: imported Python symbols and builtins are no longer obfuscated (e.g. external base classes stay intact).
- New unit and e2e regression tests for SQL/Python deep obfuscation scenarios.
- Extended test fixtures in `test-projects/` for all supported languages.
- E2E multi-language roundtrip test with runtime validation via available compilers/interpreters.
- CI workflow for build/test/coverage via Makefile (`make ci`).
- Release workflow triggered by semver-style tags (`v*`) with multi-OS binaries.
- New `SAMPLES.md` with practical business examples.

### Changed
- Forward obfuscation term detection now considers file language and keyword filtering.
- Makefile enriched with `ci` and `release-artifacts` targets.
- SVT updated to stress mixed-language large trees.

### Business Impact
- Enables safe code sharing with cloud/local LLMs (including Ollama) in polyglot teams.
- Reduces accidental leakage of product/domain identifiers across code, comments, and string literals.
- Preserves reversible mapping to integrate AI-generated changes back into production code.
