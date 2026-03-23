#![allow(dead_code)]

#[path = "../src/error.rs"]
mod error;
#[path = "../src/fs_ops.rs"]
mod fs_ops;
#[path = "../src/language.rs"]
mod language;
#[path = "../src/obfuscator.rs"]
mod obfuscator;

use std::collections::BTreeMap;
use std::fs;
use std::time::Instant;

use tempfile::TempDir;

#[test]
#[ignore]
fn bench_deep_and_global_mode_10k_files() {
    let src = TempDir::new().expect("tmp src");

    let n = 10_000;
    for i in 0..n {
        let content = format!(
            "class CustomerOrder{i}:\n    def process_payment_{i}(self, amount_{i}):\n        return amount_{i} * 2\n\norder_{i} = CustomerOrder{i}()\n"
        );
        fs::write(src.path().join(format!("file_{i}.py")), content).expect("write synthetic file");
    }

    let mut mapping: BTreeMap<String, String> = BTreeMap::new();
    for i in 0..1000 {
        mapping.insert(format!("CustomerOrder{i}"), format!("PyClassA{i}"));
        mapping.insert(format!("process_payment_{i}"), format!("py_method_a_{i}"));
        mapping.insert(format!("amount_{i}"), format!("py_var_a_{i}"));
        mapping.insert(format!("order_{i}"), format!("py_inst_a_{i}"));
    }

    let files = fs_ops::read_text_tree(src.path()).expect("read synthetic tree");
    assert_eq!(files.len(), n, "unexpected synthetic file count");

    let deep_start = Instant::now();
    let deep_out = obfuscator::transform_files(&files, &mapping).expect("deep transform");
    let deep_elapsed = deep_start.elapsed();

    let global_start = Instant::now();
    let global_out =
        obfuscator::transform_files_global(&files, &mapping).expect("global transform");
    let global_elapsed = global_start.elapsed();

    println!("Files: {n}");
    println!("Deep elapsed: {:.2}s", deep_elapsed.as_secs_f64());
    println!(
        "Deep throughput: {:.0} files/sec",
        n as f64 / deep_elapsed.as_secs_f64()
    );
    println!("Global elapsed: {:.2}s", global_elapsed.as_secs_f64());
    println!(
        "Global throughput: {:.0} files/sec",
        n as f64 / global_elapsed.as_secs_f64()
    );

    assert_eq!(deep_out.len(), n, "deep output file count mismatch");
    assert_eq!(global_out.len(), n, "global output file count mismatch");
    assert!(
        deep_elapsed.as_secs_f64() < 10.0,
        "Too slow: {:.2}s for {n} files (need < 10s)",
        deep_elapsed.as_secs_f64()
    );
}
