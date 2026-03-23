use std::fs;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;

use tempfile::tempdir;

fn cargo_binary() -> PathBuf {
    PathBuf::from(assert_cmd::cargo::cargo_bin!("code-obfuscator"))
}

#[test]
fn install_script_installs_binary_and_updates_shell_rc() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let home = tempdir().expect("home");
    let install_home = home.path().join(".code-obfuscator-test");
    let install_dir = home.path().join(".local/bin");
    let bashrc = home.path().join(".bashrc");
    fs::write(&bashrc, "# test rc\n").expect("write bashrc");

    let output = ProcessCommand::new("bash")
        .arg("./install")
        .arg("--binary")
        .arg(cargo_binary())
        .current_dir(&repo)
        .env("HOME", home.path())
        .env("SHELL", "/bin/bash")
        .env("CODE_OBFUSCATOR_HOME", &install_home)
        .env("CODE_OBFUSCATOR_INSTALL_DIR", &install_dir)
        .output()
        .expect("run install");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let installed = install_dir.join("code-obfuscator");
    assert!(installed.exists(), "installed binary missing");

    let version_output = ProcessCommand::new(&installed)
        .arg("--version")
        .output()
        .expect("run installed binary");
    assert!(version_output.status.success());
    assert!(String::from_utf8_lossy(&version_output.stdout).contains("0.5.0"));

    let bashrc_text = fs::read_to_string(&bashrc).expect("read bashrc");
    assert!(bashrc_text.contains("export PATH="));
    assert!(bashrc_text.contains(&install_dir.display().to_string()));
}

#[test]
fn install_script_installs_from_archive_without_modifying_path() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let home = tempdir().expect("home");
    let install_home = home.path().join(".code-obfuscator-test");
    let install_dir = home.path().join(".local/bin");
    let bashrc = home.path().join(".bashrc");
    fs::write(&bashrc, "# untouched\n").expect("write bashrc");

    let package_dir = tempdir().expect("package dir");
    let payload_dir = package_dir.path().join("payload");
    fs::create_dir_all(&payload_dir).expect("payload dir");
    fs::copy(cargo_binary(), payload_dir.join("code-obfuscator")).expect("copy binary");

    let archive = package_dir.path().join("code-obfuscator-linux-x64.tar.gz");
    let status = ProcessCommand::new("tar")
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(&payload_dir)
        .arg("code-obfuscator")
        .status()
        .expect("create archive");
    assert!(status.success());

    let output = ProcessCommand::new("bash")
        .arg("./install")
        .arg("--archive")
        .arg(&archive)
        .arg("--version")
        .arg("9.9.9")
        .arg("--no-modify-path")
        .current_dir(&repo)
        .env("HOME", home.path())
        .env("SHELL", "/bin/bash")
        .env("CODE_OBFUSCATOR_HOME", &install_home)
        .env("CODE_OBFUSCATOR_INSTALL_DIR", &install_dir)
        .output()
        .expect("run install from archive");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let version_file = install_home.join("current-version");
    let version_text = fs::read_to_string(version_file).expect("read version file");
    assert_eq!(version_text.trim(), "9.9.9");

    let bashrc_text = fs::read_to_string(&bashrc).expect("read bashrc");
    assert_eq!(bashrc_text, "# untouched\n");
}
