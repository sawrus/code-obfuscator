use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::symlink;

use serde_json::{Value, json};
#[cfg(unix)]
use tempfile::tempdir;

fn send_request(stdin: &mut impl Write, req: &Value) {
    let body = serde_json::to_vec(req).expect("serialize req");
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    stdin.write_all(header.as_bytes()).expect("write header");
    stdin.write_all(&body).expect("write body");
    stdin.flush().expect("flush");
}

fn read_response(stdout: &mut impl Read) -> Value {
    let mut header_bytes = Vec::new();
    let mut buf = [0_u8; 1];
    loop {
        stdout.read_exact(&mut buf).expect("read header byte");
        header_bytes.push(buf[0]);
        if header_bytes.ends_with(b"\r\n\r\n") {
            break;
        }
    }

    let header = String::from_utf8(header_bytes).expect("header utf8");
    let len = header
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length:"))
        .expect("content length line")
        .trim()
        .parse::<usize>()
        .expect("length parse");

    let mut body = vec![0_u8; len];
    stdout.read_exact(&mut body).expect("read body");
    serde_json::from_slice(&body).expect("parse response")
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind free port");
    listener.local_addr().expect("local addr").port()
}

fn http_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().expect("lock")
}

fn wait_http_ready(addr: &str) {
    for _ in 0..40 {
        if TcpStream::connect(addr).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("http server did not become ready: {addr}");
}

fn kill_and_wait(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn hmac_signature_requires_secret_key_env_var() {
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_LOG_STDOUT", "false")
        .env_remove("MAPPING_SECRET_KEY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":1,
            "method":"tools/call",
            "params":{
                "name":"pull",
                "arguments":{
                    "root_dir":"/tmp",
                    "file_paths":[],
                    "options":{"request_id":"sec-sign-no-key","security":{"sign_mapping":true}}
                }
            }
        }),
    );

    let response = read_response(&mut stdout);
    let error = response["error"]["message"].as_str().unwrap_or_default();
    assert!(error.contains("MAPPING_SECRET_KEY"), "{response}");

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn encrypt_mapping_requires_encrypt_key_env_var() {
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_LOG_STDOUT", "false")
        .env_remove("MAPPING_ENCRYPT_KEY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":2,
            "method":"tools/call",
            "params":{
                "name":"pull",
                "arguments":{
                    "root_dir":"/tmp",
                    "file_paths":[],
                    "options":{"request_id":"sec-encrypt-no-key","security":{"encrypt_mapping":true}}
                }
            }
        }),
    );

    let response = read_response(&mut stdout);
    let error = response["error"]["message"].as_str().unwrap_or_default();
    assert!(error.contains("MAPPING_ENCRYPT_KEY"), "{response}");

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn http_rejects_request_body_larger_than_64mb_by_content_length() {
    let _guard = http_test_lock();
    let port = free_port();
    let addr = format!("127.0.0.1:{port}");

    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_HTTP_ADDR", &addr)
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    wait_http_ready(&addr);

    let mut stream = TcpStream::connect(&addr).expect("connect http");
    let huge = 64 * 1024 * 1024 + 1;
    let req = format!(
        "POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {huge}\r\n\r\n"
    );
    stream
        .write_all(req.as_bytes())
        .expect("write oversized req");

    let mut resp = String::new();
    stream.read_to_string(&mut resp).expect("read response");
    assert!(resp.starts_with("HTTP/1.1 400"), "{resp}");

    kill_and_wait(&mut child);
}

#[test]
fn pull_rejects_null_byte_in_path() {
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":5,
            "method":"tools/call",
            "params":{
                "name":"pull",
                "arguments":{
                    "root_dir":"/tmp",
                    "file_paths":["src/ma\u{0000}in.py"],
                    "options":{"request_id":"null-byte-1"}
                }
            }
        }),
    );

    let response = read_response(&mut stdout);
    let error = response["error"]["message"].as_str().unwrap_or_default();
    assert!(error.contains("null bytes"), "{response}");

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn pull_rejects_request_id_too_long() {
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");
    let long_id = "x".repeat(257);

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":6,
            "method":"tools/call",
            "params":{
                "name":"pull",
                "arguments":{
                    "root_dir":"/tmp",
                    "file_paths":[],
                    "options":{"request_id":long_id}
                }
            }
        }),
    );

    let response = read_response(&mut stdout);
    let error = response["error"]["message"].as_str().unwrap_or_default();
    assert!(error.contains("exceeds max length"), "{response}");

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn pull_rejects_parent_traversal_path() {
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":7,
            "method":"tools/call",
            "params":{
                "name":"pull",
                "arguments":{
                    "root_dir":"/tmp",
                    "file_paths":["../etc/passwd"],
                    "options":{"request_id":"traversal-1"}
                }
            }
        }),
    );

    let response = read_response(&mut stdout);
    let error = response["error"]["message"].as_str().unwrap_or_default();
    assert!(error.contains("parent traversal"), "{response}");

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn push_requires_existing_request_snapshot() {
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":8,
            "method":"tools/call",
            "params":{
                "name":"push",
                "arguments":{
                    "workspace_dir":"/tmp",
                    "options":{"request_id":"missing-session"}
                }
            }
        }),
    );

    let response = read_response(&mut stdout);
    let error = response["error"]["message"].as_str().unwrap_or_default();
    assert!(error.contains("unknown request_id"), "{response}");

    drop(stdin);
    let _ = child.wait();
}

#[cfg(unix)]
#[test]
fn push_rejects_writing_through_source_symlink() {
    let dir = tempdir().expect("tmp");
    let project = dir.path().join("project");
    let workspace = dir.path().join("workspace");
    fs::create_dir_all(project.join("app")).expect("mkdirs");
    fs::write(project.join("app/query.py"), "select * from bs.users\n").expect("write");
    fs::write(project.join("app/target.py"), "print('guard')\n").expect("write");

    let mapping_path = dir.path().join("mapping.default.json");
    fs::write(&mapping_path, r#"{"bs":"mmm"}"#).expect("write mapping");

    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_DEFAULT_MAPPING_PATH", &mapping_path)
        .env("MCP_LOG_STDOUT", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":9,
            "method":"tools/call",
            "params":{
                "name":"clone",
                "arguments":{
                    "root_dir":project.to_string_lossy().to_string(),
                    "workspace_dir":workspace.to_string_lossy().to_string(),
                    "options":{"request_id":"symlink-write-1"}
                }
            }
        }),
    );
    let clone_response = read_response(&mut stdout);
    assert!(clone_response.get("error").is_none(), "{clone_response}");

    let workspace_query = workspace.join("app/query.py");
    let content = fs::read_to_string(&workspace_query).expect("read workspace query");
    fs::write(&workspace_query, content.replace("select *", "select id"))
        .expect("rewrite workspace query");

    let source_query = project.join("app/query.py");
    fs::remove_file(&source_query).expect("remove source query");
    symlink("target.py", &source_query).expect("create source symlink");

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":10,
            "method":"tools/call",
            "params":{
                "name":"push",
                "arguments":{
                    "workspace_dir":workspace.to_string_lossy().to_string(),
                    "options":{"request_id":"symlink-write-1"}
                }
            }
        }),
    );
    let push_response = read_response(&mut stdout);
    let error = push_response["error"]["message"]
        .as_str()
        .unwrap_or_default();
    assert!(
        error.contains("refusing to write through symlink"),
        "{push_response}"
    );

    drop(stdin);
    let _ = child.wait();
}
