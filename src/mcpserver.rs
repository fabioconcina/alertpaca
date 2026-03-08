use std::io::{self, BufRead, Write};

use serde_json::{json, Value};

use crate::checks;
use crate::config::Config;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Run the MCP server on stdio, blocking until stdin closes.
pub fn run(config: &Config) -> io::Result<()> {
    let stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();

    for line in stdin.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                write_response(&mut stdout, json_rpc_error(Value::Null, -32700, "Parse error"))?;
                continue;
            }
        };

        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");

        let response = match method {
            "initialize" => handle_initialize(&id),
            "notifications/initialized" => continue, // notification, no response
            "tools/list" => handle_tools_list(&id),
            "tools/call" => handle_tools_call(&id, &req, config),
            "ping" => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
            _ => json_rpc_error(id, -32601, &format!("Method not found: {method}")),
        };

        write_response(&mut stdout, response)?;
    }

    Ok(())
}

fn write_response(out: &mut impl Write, response: Value) -> io::Result<()> {
    let s = serde_json::to_string(&response)
        .map_err(io::Error::other)?;
    writeln!(out, "{s}")?;
    out.flush()
}

fn json_rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

fn handle_initialize(id: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "alertpaca",
                "version": VERSION
            }
        }
    })
}

fn handle_tools_list(id: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "tools": [
                {
                    "name": "check_health",
                    "description": "Run all server health checks and return results with status (Ok, Warning, Critical, Skipped) for system resources, services, backups, certificates, and open ports.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {},
                        "additionalProperties": false
                    }
                }
            ]
        }
    })
}

fn handle_tools_call(id: &Value, req: &Value, config: &Config) -> Value {
    let tool_name = req
        .pointer("/params/name")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if tool_name != "check_health" {
        return json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{ "type": "text", "text": format!("Unknown tool: {tool_name}") }],
                "isError": true
            }
        });
    }

    let results = checks::run_all_checks(config);
    let data = serde_json::to_string_pretty(&results).unwrap_or_else(|e| format!("Error: {e}"));

    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{ "type": "text", "text": data }],
            "isError": false
        }
    })
}
