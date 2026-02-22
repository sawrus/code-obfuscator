use std::fs;

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
        "fn Freeze() { let Antifraud = 1; }",
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
    assert_eq!(rev_file, "fn Freeze() { let Antifraud = 1; }");
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
