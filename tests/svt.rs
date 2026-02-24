use std::fs;
use std::time::Instant;

use assert_cmd::Command;
use tempfile::tempdir;

#[test]
#[ignore = "svt"]
fn svt_large_tree_finishes_and_reports_stats() {
    let src = tempdir().expect("src");
    let out = tempdir().expect("out");
    let exts = [
        "rs", "py", "js", "ts", "java", "cs", "cpp", "go", "sql", "sh",
    ];
    for i in 0..500 {
        let ext = exts[i % exts.len()];
        let file = src.path().join(format!("m{i}.{ext}"));
        fs::write(
            file,
            format!(
                "fn refill_action{i}() {{ let user_id{i} = {i}; }} # refill_action{i} user_id{i}"
            ),
        )
        .expect("write");
    }

    let mapping = src.path().join("mapping.json");
    fs::write(
        &mapping,
        r#"{"refill_action1":"r1","user_id1":"u1","refill_action11":"r11","user_id11":"u11"}"#,
    )
    .expect("map");

    let start = Instant::now();
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("code-obfuscator"));
    cmd.arg("--mode")
        .arg("forward")
        .arg("--source")
        .arg(src.path())
        .arg("--target")
        .arg(out.path())
        .arg("--mapping")
        .arg(&mapping);
    let assert = cmd.assert().success();
    let elapsed = start.elapsed();

    assert!(elapsed.as_secs() < 30, "svt took too long: {elapsed:?}");
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout");
    assert!(stdout.contains("processed_files=500"));

    let py1_out = fs::read_to_string(out.path().join("m1.py")).expect("read py1");
    assert!(py1_out.contains("r1"));
    assert!(py1_out.contains("u1"));
    let py11_out = fs::read_to_string(out.path().join("m11.py")).expect("read py11");
    assert!(py11_out.contains("r11"));
    assert!(py11_out.contains("u11"));
}
