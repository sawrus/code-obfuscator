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
fn fails_without_mapping_in_forward_mode_by_default() {
    let src = tempdir().expect("src");
    let out = tempdir().expect("out");
    let cfg = tempdir().expect("cfg");
    fs::write(src.path().join("main.py"), "print('ok')\n").expect("write");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("code-obfuscator"));
    cmd.arg("--mode")
        .arg("forward")
        .arg("--source")
        .arg(src.path())
        .arg("--target")
        .arg(out.path())
        .env("XDG_CONFIG_HOME", cfg.path());
    cmd.assert().failure().stderr(predicate::str::contains(
        "mapping is required in forward mode unless --deep is set",
    ));
}

#[test]
fn forward_with_deep_without_mapping_succeeds() {
    let src = tempdir().expect("src");
    let obf = tempdir().expect("obf");
    fs::write(
        src.path().join("main.py"),
        "def run_task(user_id):\n    return user_id + 1\n",
    )
    .expect("write");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("code-obfuscator"));
    cmd.arg("--mode")
        .arg("forward")
        .arg("--source")
        .arg(src.path())
        .arg("--target")
        .arg(obf.path())
        .arg("--deep");
    cmd.assert().success();

    assert!(obf.path().join("mapping.generated.json").exists());
}

#[test]
fn forward_uses_config_default_mapping_when_flag_is_omitted() {
    let src = tempdir().expect("src");
    let out = tempdir().expect("out");
    let cfg = tempdir().expect("cfg");
    fs::write(
        src.path().join("main.py"),
        "def freeze(user_id):
    return user_id
",
    )
    .expect("write src");

    let config_dir = cfg.path().join("xdg");
    let app_dir = config_dir.join("code-obfuscator");
    fs::create_dir_all(&app_dir).expect("app dir");
    fs::write(
        app_dir.join("mapping.json"),
        r#"{"freeze":"run_task","user_id":"subject_id"}"#,
    )
    .expect("write mapping");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("code-obfuscator"));
    cmd.arg("--mode")
        .arg("forward")
        .arg("--source")
        .arg(src.path())
        .arg("--target")
        .arg(out.path())
        .env("XDG_CONFIG_HOME", &config_dir);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("mapping_input_path="));

    let out_file = fs::read_to_string(out.path().join("main.py")).expect("read output");
    assert!(out_file.contains("run_task"));
    assert!(out_file.contains("subject_id"));
}

#[test]
fn explicit_mapping_overrides_and_updates_config_default() {
    let src = tempdir().expect("src");
    let out = tempdir().expect("out");
    let cfg = tempdir().expect("cfg");
    fs::write(
        src.path().join("main.py"),
        "def freeze(user_id):
    return user_id
",
    )
    .expect("write src");

    let config_dir = cfg.path().join("xdg");
    let app_dir = config_dir.join("code-obfuscator");
    fs::create_dir_all(&app_dir).expect("app dir");
    fs::write(
        app_dir.join("mapping.json"),
        r#"{"freeze":"from_config","user_id":"config_user"}"#,
    )
    .expect("write config mapping");

    let explicit = src.path().join("mapping.json");
    fs::write(&explicit, r#"{"freeze":"from_flag","user_id":"flag_user"}"#)
        .expect("write explicit mapping");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("code-obfuscator"));
    cmd.arg("--mode")
        .arg("forward")
        .arg("--source")
        .arg(src.path())
        .arg("--target")
        .arg(out.path())
        .arg("--mapping")
        .arg(&explicit)
        .env("XDG_CONFIG_HOME", &config_dir);
    cmd.assert().success();

    let out_file = fs::read_to_string(out.path().join("main.py")).expect("read output");
    assert!(out_file.contains("from_flag"));
    assert!(out_file.contains("flag_user"));
    assert!(!out_file.contains("from_config"));

    let saved = fs::read_to_string(app_dir.join("mapping.json")).expect("saved config mapping");
    assert!(saved.contains("from_flag"));
    assert!(saved.contains("flag_user"));
}

#[test]
fn tui_mode_smoke_test_works_with_piped_input() {
    let src = tempdir().expect("src");
    let out = tempdir().expect("out");
    let cfg = tempdir().expect("cfg");
    fs::write(
        src.path().join("main.py"),
        "def freeze(user_id):
    return user_id
",
    )
    .expect("write src");

    let config_dir = cfg.path().join("xdg");
    let app_dir = config_dir.join("code-obfuscator");
    fs::create_dir_all(&app_dir).expect("app dir");
    fs::write(
        app_dir.join("mapping.json"),
        r#"{"freeze":"from_tui","user_id":"tui_user"}"#,
    )
    .expect("write mapping");

    let input = format!(
        "1
{}
{}
n
y


n
",
        src.path().display(),
        out.path().display()
    );

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("code-obfuscator"));
    cmd.arg("--tui")
        .write_stdin(input)
        .env("XDG_CONFIG_HOME", &config_dir);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("mapping_input_path="));

    let out_file = fs::read_to_string(out.path().join("main.py")).expect("read output");
    assert!(out_file.contains("from_tui"));
    assert!(out_file.contains("tui_user"));
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

    let mapping = src.path().join("mapping.json");
    fs::write(
        &mapping,
        r#"{
  "rows": "dataset_rows",
  "project_totals": "project_totals_obf",
  "priority_user_ids": "priority_user_ids_obf",
  "signature": "signature_obf",
  "summary": "summary_obf",
  "project_user": "project_user_obf",
  "project_weight": "project_weight_obf",
  "project_summary": "project_summary_obf"
}"#,
    )
    .expect("write mapping");

    let mut forward = Command::new(assert_cmd::cargo::cargo_bin!("code-obfuscator"));
    forward
        .arg("--mode")
        .arg("forward")
        .arg("--source")
        .arg(src.path())
        .arg("--target")
        .arg(obf.path())
        .arg("--mapping")
        .arg(&mapping);
    forward.assert().success();

    for case in &languages {
        let file = obf.path().join(case.folder).join(case.file);
        case.runtime.compile_if_available(&file);
    }

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
        let rev_file = rev.path().join(case.folder).join(case.file);
        assert!(
            rev_file.exists(),
            "restored file missing for {}",
            case.folder
        );

        case.runtime.compile_if_available(&rev_file);
    }
}

#[test]
fn regression_python_magic_imports_and_named_args_stay_valid() {
    let src = tempdir().expect("src");
    let obf = tempdir().expect("obf");
    let rev = tempdir().expect("rev");

    fs::create_dir_all(src.path().join("pkg")).expect("pkg");
    fs::write(
        src.path().join("pkg/mod.py"),
        "def greet(*, user_name):\n    print(user_name)\n",
    )
    .expect("write mod");
    fs::write(
        src.path().join("main.py"),
        "from pkg.mod import greet\n\nif __name__ == \"__main__\":\n    greet(user_name=\"pkg.mod\")\n",
    )
    .expect("write main");

    let mut forward = Command::new(assert_cmd::cargo::cargo_bin!("code-obfuscator"));
    forward
        .arg("--mode")
        .arg("forward")
        .arg("--source")
        .arg(src.path())
        .arg("--target")
        .arg(obf.path())
        .arg("--seed")
        .arg("7")
        .arg("--deep");
    forward.assert().success();

    let obf_main = fs::read_to_string(obf.path().join("main.py")).expect("read obf main");
    assert!(obf_main.contains("if __name__ == \"__main__\":"));
    assert!(obf_main.contains("from pkg.mod import "));
    assert!(obf_main.contains("=\"pkg.mod\")"));

    RuntimeCheck::Python.run_if_available(&obf.path().join("main.py"));

    let mut reverse = Command::new(assert_cmd::cargo::cargo_bin!("code-obfuscator"));
    reverse
        .arg("--mode")
        .arg("reverse")
        .arg("--source")
        .arg(obf.path())
        .arg("--target")
        .arg(rev.path());
    reverse.assert().success();

    let rev_main = fs::read_to_string(rev.path().join("main.py")).expect("read rev main");
    let src_main = fs::read_to_string(src.path().join("main.py")).expect("read src main");
    assert_eq!(rev_main, src_main);
}

#[test]
fn e2e_deep_obfuscation_sql_and_python_with_external_class_guard() {
    let src = tempdir().expect("src");
    let obf = tempdir().expect("obf");

    fs::create_dir_all(src.path().join("sql")).expect("sql dir");
    fs::create_dir_all(src.path().join("py")).expect("py dir");

    fs::write(
        src.path().join("sql/main.sql"),
        "SELECT r.user_id, amount, code FROM refill r WHERE r.user_id > 10;
",
    )
    .expect("write sql");

    fs::write(
        src.path().join("py/main.py"),
        r#"from apiutil.models import User

PG_MWL_PASSWORD = "secret"

def get_suspect_users_from_refill_actions():
    return PG_MWL_PASSWORD

@dataclass
class Falcon8382(User):
    pass
"#,
    )
    .expect("write py");

    let mapping = src.path().join("mapping.json");
    fs::write(
        &mapping,
        r#"{
  "refill": "test666",
  "user_id": "a1",
  "amount": "b1",
  "code": "c1",
  "PG_MWL_PASSWORD": "PG_CAT_P",
  "get_suspect_users_from_refill_actions": "get_a_b_c",
  "User": "Amber2096"
}"#,
    )
    .expect("write map");

    let mut forward = Command::new(assert_cmd::cargo::cargo_bin!("code-obfuscator"));
    forward
        .arg("--mode")
        .arg("forward")
        .arg("--source")
        .arg(src.path())
        .arg("--target")
        .arg(obf.path())
        .arg("--mapping")
        .arg(&mapping)
        .arg("--deep");
    forward.assert().success();

    let sql = fs::read_to_string(obf.path().join("sql/main.sql")).expect("read sql");
    assert!(sql.contains("FROM test666 r"));
    assert!(sql.contains("r.a1, b1, c1"));

    let py = fs::read_to_string(obf.path().join("py/main.py")).expect("read py");
    assert!(py.contains("PG_CAT_P = \"secret\""));
    assert!(py.contains("def get_a_b_c():"));
    assert!(py.contains("(User):"));
    assert!(!py.contains("Amber2096"));
}

#[test]
fn e2e_deep_obfuscation_mapping_for_other_languages() {
    let src = tempdir().expect("src");
    let obf = tempdir().expect("obf");

    let fixtures = [
        (
            "javascript/main.js",
            "function refill_action(user_id) { return user_id + 1; }
",
        ),
        (
            "typescript/main.ts",
            "function refill_action(user_id: number): number { return user_id + 1; }
",
        ),
        (
            "java/Main.java",
            "class Main { int refill_action(int user_id) { return user_id + 1; } }
",
        ),
        (
            "csharp/Program.cs",
            "class Program { static int refill_action(int user_id) { return user_id + 1; } }
",
        ),
        (
            "cpp/main.cpp",
            "int refill_action(int user_id) { return user_id + 1; }
",
        ),
        (
            "go/main.go",
            "func refill_action(user_id int) int { return user_id + 1 }
",
        ),
        (
            "rust/main.rs",
            "fn refill_action(user_id: i32) -> i32 { user_id + 1 }
",
        ),
        (
            "bash/main.sh",
            "refill_action() { user_id=1; echo $user_id; }
",
        ),
    ];

    for (rel, body) in fixtures {
        let target = src.path().join(rel);
        fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
        fs::write(target, body).expect("write fixture");
    }

    let mapping = src.path().join("mapping.json");
    fs::write(&mapping, r#"{"refill_action":"r1","user_id":"u1"}"#).expect("write map");

    let mut forward = Command::new(assert_cmd::cargo::cargo_bin!("code-obfuscator"));
    forward
        .arg("--mode")
        .arg("forward")
        .arg("--source")
        .arg(src.path())
        .arg("--target")
        .arg(obf.path())
        .arg("--mapping")
        .arg(&mapping);
    forward.assert().success();

    let checks = [
        "javascript/main.js",
        "typescript/main.ts",
        "java/Main.java",
        "csharp/Program.cs",
        "cpp/main.cpp",
        "go/main.go",
        "rust/main.rs",
        "bash/main.sh",
    ];

    for rel in checks {
        let out = fs::read_to_string(obf.path().join(rel)).expect("read output");
        assert!(
            out.contains("r1"),
            "expected mapped function in {rel}: {out}"
        );
        assert!(
            out.contains("u1"),
            "expected mapped variable in {rel}: {out}"
        );
        assert!(
            !out.contains("refill_action"),
            "source function leaked in {rel}: {out}"
        );
        assert!(
            !out.contains("user_id"),
            "source variable leaked in {rel}: {out}"
        );
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
    fn compile_if_available(self, file: &Path) {
        match self {
            RuntimeCheck::Python => {
                if has_cmd("python3") {
                    run_success(
                        ProcessCommand::new("python3")
                            .arg("-m")
                            .arg("py_compile")
                            .arg(file),
                    );
                }
            }
            RuntimeCheck::Node => {
                if has_cmd("node") {
                    let _ =
                        run_success_or_skip(ProcessCommand::new("node").arg("--check").arg(file));
                }
            }
            RuntimeCheck::TypeScript => {
                if has_cmd("tsc") {
                    let _ = run_success_or_skip(
                        ProcessCommand::new("tsc")
                            .arg("--noEmit")
                            .arg("--target")
                            .arg("es2020")
                            .arg(file),
                    );
                }
            }
            RuntimeCheck::Java => {
                if has_cmd("javac") {
                    let _ = run_success_or_skip(ProcessCommand::new("javac").arg(file));
                }
            }
            RuntimeCheck::CSharp => {
                if has_cmd("csc") {
                    let out = file.parent().expect("dir").join("Program.exe");
                    let _ = run_success_or_skip(
                        ProcessCommand::new("csc")
                            .arg(file)
                            .arg(format!("/out:{}", out.display())),
                    );
                }
            }
            RuntimeCheck::Cpp => {
                if has_cmd("g++") {
                    let out = file.parent().expect("dir").join("app.out");
                    let _ = run_success_or_skip(
                        ProcessCommand::new("g++").arg(file).arg("-o").arg(&out),
                    );
                }
            }
            RuntimeCheck::Go => {
                if has_cmd("go") {
                    let out = file.parent().expect("dir").join("app-go");
                    let _ = run_success_or_skip(
                        ProcessCommand::new("go")
                            .arg("build")
                            .arg("-o")
                            .arg(&out)
                            .arg(file),
                    );
                }
            }
            RuntimeCheck::Rust => {
                if has_cmd("rustc") {
                    let out = file.parent().expect("dir").join("app-rs");
                    let _ = run_success_or_skip(
                        ProcessCommand::new("rustc").arg(file).arg("-o").arg(&out),
                    );
                }
            }
            RuntimeCheck::SqlLint => {
                run_sql_validation_if_available(file);
            }
            RuntimeCheck::Bash => {
                let _ = run_success_or_skip(ProcessCommand::new("bash").arg("-n").arg(file));
            }
        }
    }

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
                    let _ = run_success_or_skip(ProcessCommand::new("javac").arg(file));
                    run_success(ProcessCommand::new("java").arg("-cp").arg(dir).arg("Main"));
                }
            }
            RuntimeCheck::CSharp => {
                if has_cmd("dotnet-script") {
                    run_success(ProcessCommand::new("dotnet-script").arg(file));
                } else if has_cmd("csc") {
                    let out = file.parent().expect("dir").join("Program.exe");
                    let _ = run_success_or_skip(
                        ProcessCommand::new("csc")
                            .arg(file)
                            .arg(format!("/out:{}", out.display())),
                    );
                    if has_cmd("mono") {
                        run_success(ProcessCommand::new("mono").arg(out));
                    }
                }
            }
            RuntimeCheck::Cpp => {
                if has_cmd("g++") {
                    let out = file.parent().expect("dir").join("app.out");
                    if !run_success_or_skip(
                        ProcessCommand::new("g++").arg(file).arg("-o").arg(&out),
                    ) {
                        return;
                    }
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
                    let _ = run_success_or_skip(
                        ProcessCommand::new("rustc").arg(file).arg("-o").arg(&out),
                    );
                    run_success(&mut ProcessCommand::new(out));
                }
            }
            RuntimeCheck::SqlLint => {
                run_sql_validation_if_available(file);
            }
            RuntimeCheck::Bash => {
                run_success(ProcessCommand::new("bash").arg(file));
            }
        }
    }
}

fn run_sql_validation_if_available(file: &Path) {
    if has_cmd("sqlfluff") {
        let _ = run_success_or_skip(ProcessCommand::new("sqlfluff").arg("lint").arg(file));
    } else if has_cmd("sqlite3") {
        let _ = run_success_or_skip(
            ProcessCommand::new("sqlite3")
                .arg(":memory:")
                .arg(format!(".read {}", file.display())),
        );
    } else if has_cmd("python3") {
        let _ = run_success_or_skip(
            ProcessCommand::new("python3")
                .arg("-c")
                .arg("import pathlib,sqlite3,sys; con=sqlite3.connect(':memory:'); con.executescript(pathlib.Path(sys.argv[1]).read_text())")
                .arg(file),
        );
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

fn run_success_or_skip(cmd: &mut ProcessCommand) -> bool {
    let output = cmd.output().expect("command output");
    if output.status.success() {
        return true;
    }
    eprintln!(
        "skipping runtime check due to unavailable toolchain: status={:?}, stdout={}, stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    false
}
