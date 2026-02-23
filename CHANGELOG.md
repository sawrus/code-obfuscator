# Changelog

## [0.2.0] - 2026-02-23

### Added
- Language-aware detection of obfuscation terms for 10 languages: Python, JavaScript, TypeScript, Java, C#, C/C++, Go, Rust, SQL, Bash.
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
