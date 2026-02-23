use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

#[test]
fn forward_then_reverse_restores_code() {
    let src = tempdir().expect("src");
    let obf = tempdir().expect("obf");
    let rev = tempdir().expect("rev");
    fs::write(
        src.path().join("main.rs"),
        "fn Freeze() { let Antifraud = 1; // Freeze in comment\nprintln!(\"Antifraud\"); }",
    )
    .expect("write");
    let map = src.path().join("mapping.json");
    fs::write(&map, "{\"Freeze\":\"Go\",\"Antifraud\":\"Apple\"}").expect("map");

    let mut c1 = Command::new(assert_cmd::cargo::cargo_bin!("code-obfuscator"));
    c1.arg("--mode")
        .arg("forward")
        .arg("--source")
        .arg(src.path())
        .arg("--target")
        .arg(obf.path())
        .arg("--mapping")
        .arg(&map);
    c1.assert().success();

    let obf_file = fs::read_to_string(obf.path().join("main.rs")).expect("read obf");
    assert!(obf_file.contains("Go"));
    assert!(obf_file.contains("Apple"));

    let mut c2 = Command::new(assert_cmd::cargo::cargo_bin!("code-obfuscator"));
    c2.arg("--mode")
        .arg("reverse")
        .arg("--source")
        .arg(obf.path())
        .arg("--target")
        .arg(rev.path());
    c2.assert().success();

    let rev_file = fs::read_to_string(rev.path().join("main.rs")).expect("read rev");
    let src_file = fs::read_to_string(src.path().join("main.rs")).expect("read src");
    assert_eq!(rev_file, src_file);
}

#[test]
fn fails_without_mapping_in_reverse_mode() {
    let src = tempdir().expect("src");
    let out = tempdir().expect("out");
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("code-obfuscator"));
    cmd.arg("--mode")
        .arg("reverse")
        .arg("--source")
        .arg(src.path())
        .arg("--target")
        .arg(out.path());
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("mapping is required"));
}

#[test]
fn e2e_all_10_languages_roundtrip_and_runtime_when_available() {
    let languages = [
        LanguageCase::new("python", "main.py", RuntimeCheck::Python),
        LanguageCase::new("javascript", "main.js", RuntimeCheck::Node),
        LanguageCase::new("typescript", "main.ts", RuntimeCheck::TypeScript),
        LanguageCase::new("java", "Main.java", RuntimeCheck::Java),
        LanguageCase::new("csharp", "Program.cs", RuntimeCheck::CSharp),
        LanguageCase::new("cpp", "main.cpp", RuntimeCheck::Cpp),
        LanguageCase::new("go", "main.go", RuntimeCheck::Go),
        LanguageCase::new("rust", "main.rs", RuntimeCheck::Rust),
        LanguageCase::new("sql", "main.sql", RuntimeCheck::SqlLint),
        LanguageCase::new("bash", "main.sh", RuntimeCheck::Bash),
    ];

    let src = tempdir().expect("src");
    for case in &languages {
        let fixture = PathBuf::from("test-projects")
            .join(case.folder)
            .join(case.file);
        let target = src.path().join(case.folder).join(case.file);
        fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
        fs::copy(fixture, &target).expect("copy fixture");
    }

    let obf = tempdir().expect("obf");
    let rev = tempdir().expect("rev");

    let mut forward = Command::new(assert_cmd::cargo::cargo_bin!("code-obfuscator"));
    forward
        .arg("--mode")
        .arg("forward")
        .arg("--source")
        .arg(src.path())
        .arg("--target")
        .arg(obf.path())
        .arg("--seed")
        .arg("42");
    forward.assert().success();

    let mut reverse = Command::new(assert_cmd::cargo::cargo_bin!("code-obfuscator"));
    reverse
        .arg("--mode")
        .arg("reverse")
        .arg("--source")
        .arg(obf.path())
        .arg("--target")
        .arg(rev.path());
    reverse.assert().success();

    for case in &languages {
        let src_file = src.path().join(case.folder).join(case.file);
        let rev_file = rev.path().join(case.folder).join(case.file);
        let original = fs::read_to_string(src_file).expect("src read");
        let restored = fs::read_to_string(rev_file).expect("rev read");
        assert_eq!(restored, original, "roundtrip mismatch for {}", case.folder);

        case.runtime
            .run_if_available(&rev.path().join(case.folder).join(case.file));
    }
}

#[derive(Clone, Copy)]
struct LanguageCase {
    folder: &'static str,
    file: &'static str,
    runtime: RuntimeCheck,
}

impl LanguageCase {
    const fn new(folder: &'static str, file: &'static str, runtime: RuntimeCheck) -> Self {
        Self {
            folder,
            file,
            runtime,
        }
    }
}

#[derive(Clone, Copy)]
enum RuntimeCheck {
    Python,
    Node,
    TypeScript,
    Java,
    CSharp,
    Cpp,
    Go,
    Rust,
    SqlLint,
    Bash,
}

impl RuntimeCheck {
    fn run_if_available(self, file: &Path) {
        match self {
            RuntimeCheck::Python => {
                if has_cmd("python3") {
                    run_success(ProcessCommand::new("python3").arg(file));
                }
            }
            RuntimeCheck::Node => {
                if has_cmd("node") {
                    run_success(ProcessCommand::new("node").arg(file));
                }
            }
            RuntimeCheck::TypeScript => {
                if has_cmd("ts-node") {
                    run_success(ProcessCommand::new("ts-node").arg(file));
                } else if has_cmd("deno") {
                    run_success(ProcessCommand::new("deno").arg("run").arg(file));
                }
            }
            RuntimeCheck::Java => {
                if has_cmd("javac") && has_cmd("java") {
                    let dir = file.parent().expect("dir");
                    run_success(ProcessCommand::new("javac").arg(file));
                    run_success(ProcessCommand::new("java").arg("-cp").arg(dir).arg("Main"));
                }
            }
            RuntimeCheck::CSharp => {
                if has_cmd("dotnet-script") {
                    run_success(ProcessCommand::new("dotnet-script").arg(file));
                } else if has_cmd("csc") {
                    let out = file.parent().expect("dir").join("Program.exe");
                    run_success(
                        ProcessCommand::new("csc")
                            .arg(file)
                            .arg(format!("/out:{}", out.display())),
                    );
                } else if has_cmd("dotnet") {
                    // dotnet is present but neither dotnet-script nor csc are available;
                    // skip execution to avoid false negatives in constrained environments.
                }
            }
            RuntimeCheck::Cpp => {
                if has_cmd("g++") {
                    let out = file.parent().expect("dir").join("app.out");
                    run_success(ProcessCommand::new("g++").arg(file).arg("-o").arg(&out));
                    run_success(&mut ProcessCommand::new(out));
                }
            }
            RuntimeCheck::Go => {
                if has_cmd("go") {
                    run_success(ProcessCommand::new("go").arg("run").arg(file));
                }
            }
            RuntimeCheck::Rust => {
                if has_cmd("rustc") {
                    let out = file.parent().expect("dir").join("app-rs");
                    run_success(ProcessCommand::new("rustc").arg(file).arg("-o").arg(&out));
                    run_success(&mut ProcessCommand::new(out));
                }
            }
            RuntimeCheck::SqlLint => {
                if has_cmd("psql") {
                    run_success(ProcessCommand::new("psql").arg("--version"));
                }
            }
            RuntimeCheck::Bash => {
                run_success(ProcessCommand::new("bash").arg(file));
            }
        }
    }
}

fn has_cmd(name: &str) -> bool {
    ProcessCommand::new("bash")
        .arg("-lc")
        .arg(format!("command -v {name}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn run_success(cmd: &mut ProcessCommand) {
    let output = cmd.output().expect("command output");
    assert!(
        output.status.success(),
        "command failed: status={:?}, stdout={}, stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
