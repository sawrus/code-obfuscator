use std::io::{Read, Write};
use std::process::{Command, Stdio};

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

#[test]
fn mcp_roundtrip_obfuscate_then_deobfuscate() {
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
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
                    "manual_mapping":{"run_order":"x_run"}
                }
            }
        }),
    );
    let obf_response = read_response(&mut stdout);
    assert!(obf_response.get("error").is_none(), "{obf_response}");

    let text = obf_response["result"]["content"][0]["text"]
        .as_str()
        .expect("text payload");
    let payload: Value = serde_json::from_str(text).expect("parse tool result");

    let obf_files = payload["obfuscated_files"].clone();
    let mapping_payload = payload["mapping_payload"].clone();

    send_request(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":2,
            "method":"tools/call",
            "params":{
                "name":"deobfuscate_project",
                "arguments":{
                    "llm_output_files": obf_files,
                    "mapping_payload": mapping_payload
                }
            }
        }),
    );
    let rev_response = read_response(&mut stdout);
    assert!(rev_response.get("error").is_none(), "{rev_response}");

    let rev_text = rev_response["result"]["content"][0]["text"]
        .as_str()
        .expect("rev text payload");
    let restored: Value = serde_json::from_str(rev_text).expect("restored payload");
    let first_content = restored["restored_files"][0]["content"]
        .as_str()
        .expect("content");

    assert!(first_content.contains("run_order"));
    assert!(first_content.contains("order_id"));

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_deobfuscate_fails_fast_on_missing_tokens() {
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("mcp-server"))
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
            "id":10,
            "method":"tools/call",
            "params":{
                "name":"deobfuscate_project",
                "arguments":{
                    "llm_output_files":[{"path":"a.py","content":"print('nothing')"}],
                    "mapping_payload":{
                        "mapping":{
                            "forward":{"run_order":"x_run"},
                            "reverse":{"x_run":"run_order"}
                        },
                        "created_at_epoch_s": 1
                    }
                }
            }
        }),
    );

    let response = read_response(&mut stdout);
    let error_text = response["error"]["message"].as_str().unwrap_or_default();
    assert!(error_text.contains("fail-fast"), "{response}");

    drop(stdin);
    let _ = child.wait();
}
