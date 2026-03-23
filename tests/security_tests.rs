use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

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
                "name":"obfuscate_project",
                "arguments":{
                    "project_files":[{"path":"src/main.py","content":"def run_order(order_id):\n    return order_id\n"}],
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
fn deobfuscate_rejects_client_supplied_mapping_payload_tampering_attempt() {
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
        .env("MCP_ALLOW_DIRECT_DEOBFUSCATION", "true")
        .env("MAPPING_SECRET_KEY", "test-signing-key")
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
            "id":2,
            "method":"tools/call",
            "params":{
                "name":"obfuscate_project",
                "arguments":{
                    "project_files":[{"path":"a.py","content":"def run_order(order_id):\n    return order_id\n"}],
                    "options":{"request_id":"sec-sign-tamper","security":{"sign_mapping":true}}
                }
            }
        }),
    );
    let obf_response = read_response(&mut stdout);
    assert!(obf_response.get("error").is_none(), "{obf_response}");

    let obf_text = obf_response["result"]["content"][0]["text"]
        .as_str()
        .expect("obfuscate payload text");
    let obf_payload: Value = serde_json::from_str(obf_text).expect("parse obfuscate payload");
    let obfuscated_content = obf_payload["obfuscated_files"][0]["content"]
        .as_str()
        .expect("obfuscated content")
        .to_string();

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":3,
            "method":"tools/call",
            "params":{
                "name":"deobfuscate_project",
                "arguments":{
                    "llm_output_files":[{"path":"a.py","content":obfuscated_content}],
                    "mapping_payload":{"signature":"bad-signature"},
                    "options":{"request_id":"sec-sign-tamper"}
                }
            }
        }),
    );

    let response = read_response(&mut stdout);
    let error = response["error"]["message"].as_str().unwrap_or_default();
    assert!(error.contains("unknown field"), "{response}");
    assert!(error.contains("mapping_payload"), "{response}");

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
            "id":4,
            "method":"tools/call",
            "params":{
                "name":"obfuscate_project",
                "arguments":{
                    "project_files":[{"path":"src/main.py","content":"print('x')\n"}],
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
fn obfuscate_rejects_null_byte_in_path() {
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
                "name":"obfuscate_project",
                "arguments":{
                    "project_files":[{"path":"src/fo\u{0000}o.py","content":"print('x')\n"}],
                    "options":{"request_id":"sec-null-byte"}
                }
            }
        }),
    );

    let response = read_response(&mut stdout);
    let error = response["error"]["message"].as_str().unwrap_or_default();
    assert!(
        error.contains("null bytes") || error.contains("invalid path"),
        "{response}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn obfuscate_rejects_request_id_too_long() {
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
            "id":6,
            "method":"tools/call",
            "params":{
                "name":"obfuscate_project",
                "arguments":{
                    "project_files":[{"path":"src/main.py","content":"print('x')\n"}],
                    "options":{"request_id":"x".repeat(300)}
                }
            }
        }),
    );

    let response = read_response(&mut stdout);
    let error = response["error"]["message"].as_str().unwrap_or_default();
    assert!(
        error.contains("exceeds max length") || error.contains("request_id"),
        "{response}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn obfuscate_rejects_parent_traversal_path() {
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
                "name":"obfuscate_project",
                "arguments":{
                    "project_files":[{"path":"../secret.txt","content":"hidden\n"}],
                    "options":{"request_id":"sec-parent-traversal"}
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
