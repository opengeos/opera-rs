use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn opera_bin() -> &'static str {
    env!("CARGO_BIN_EXE_opera")
}

#[test]
fn mcp_server_roundtrips_initialize_tools_prompts_and_tool_calls() {
    let mut child = Command::new(opera_bin())
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn opera mcp");

    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let mut reader = BufReader::new(stdout);

    let mut send = |req: Value| -> Value {
        writeln!(stdin, "{req}").expect("write request");
        stdin.flush().expect("flush request");
        let mut line = String::new();
        reader.read_line(&mut line).expect("read reply");
        serde_json::from_str(&line).expect("parse reply JSON")
    };

    let init = send(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {"protocolVersion": "2025-06-18"}
    }));
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["name"], "opera-rs");

    let tools = send(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    }));
    let list = tools["result"]["tools"].as_array().unwrap();
    assert!(list.len() >= 13);

    let products_call = send(json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "products",
            "arguments": {"args": ["--json"]}
        }
    }));
    assert_eq!(products_call["result"]["isError"], false);
    let text = products_call["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    let parsed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["products"].as_array().unwrap().len(), 9);

    let tile_call = send(json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "tilejson",
            "arguments": {
                "args": [
                    "--concept-id", "C2617126679-POCLOUD",
                    "--product", "dswx-s1",
                    "--band", "B01_WTR",
                    "--water-only",
                    "--url-only",
                    "--json"
                ]
            }
        }
    }));
    assert_eq!(tile_call["result"]["isError"], false);
    let tile_text = tile_call["result"]["content"][0]["text"].as_str().unwrap();
    assert!(tile_text.contains("WebMercatorQuad/tilejson.json"));

    let prompts = send(json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "prompts/list",
        "params": {}
    }));
    assert_eq!(prompts["result"]["prompts"].as_array().unwrap().len(), 4);

    let batch = send(json!([
        {"jsonrpc": "2.0", "id": 6, "method": "ping", "params": {}},
        {"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": "products", "arguments": {"args": ["rtc-s1", "--json"]}}}
    ]));
    assert_eq!(batch.as_array().unwrap().len(), 2);

    drop(stdin);
    let status = child.wait().expect("wait opera mcp");
    assert!(status.success());
}

#[test]
fn cli_json_usage_returns_valid_schema() {
    let out = Command::new(opera_bin())
        .arg("--json-usage")
        .output()
        .expect("run opera --json-usage");
    assert!(out.status.success());
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["program"], "opera");
    assert!(
        doc["command"]["subcommands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == "flood-extent")
    );
}
