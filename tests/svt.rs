use std::fs;
use std::time::Instant;

use assert_cmd::Command;
use tempfile::tempdir;

#[test]
#[ignore = "svt"]
fn svt_large_tree_finishes_and_reports_stats() {
    let src = tempdir().expect("src");
    let out = tempdir().expect("out");
    for i in 0..300 {
        let file = src.path().join(format!("m{i}.rs"));
        fs::write(
            file,
            format!("fn Freeze{i}() {{ let Antifraud{i} = {i}; }}"),
        )
        .expect("write");
    }

    let start = Instant::now();
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("code-obfuscator"));
    cmd.arg("--mode")
        .arg("forward")
        .arg("--source")
        .arg(src.path())
        .arg("--target")
        .arg(out.path())
        .arg("--seed")
        .arg("7");
    let assert = cmd.assert().success();
    let elapsed = start.elapsed();

    assert!(elapsed.as_secs() < 30, "svt took too long: {elapsed:?}");
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout");
    assert!(stdout.contains("processed_files=300"));
}
