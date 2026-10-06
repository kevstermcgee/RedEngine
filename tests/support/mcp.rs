//! A minimal MCP client for tests: newline-delimited JSON-RPC 2.0 over the stdio of a `red_engine2 mcp` child. It counts what an agent would pay (bytes it sends,
//! text it reads, the size of `tools/list`) and fails loudly if the server's stdout carries anything that is not a protocol message.
#![allow(dead_code)]

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// One tool result as an agent sees it.
pub struct Reply {
    /// The text content, concatenated.
    pub text: String,
    /// Base64 payloads of the images in the result.
    pub images: Vec<String>,
    /// `isError` was set.
    pub is_error: bool,
}

/// A running server and what talking to it has cost so far.
pub struct McpClient {
    child: Child,
    stdin: ChildStdin,
    out: BufReader<ChildStdout>,
    id: u64,
    /// Bytes of tool arguments sent.
    pub sent_bytes: usize,
    /// Bytes of result text read.
    pub text_bytes: usize,
    /// Bytes of the `tools/list` result (what every session loads into the agent's context).
    pub tools_list_bytes: usize,
}

impl McpClient {
    /// Starts `bin mcp` in `cwd` and completes the handshake.
    pub fn start(bin: &str, cwd: &Path) -> McpClient {
        let mut child = Command::new(bin)
            .arg("mcp")
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start red_engine2 mcp");
        let stdin = child.stdin.take().unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        let mut c = McpClient { child, stdin, out, id: 0, sent_bytes: 0, text_bytes: 0, tools_list_bytes: 0 };
        let init = c.rpc("initialize", json!({"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "tests", "version": "1"}}), true);
        assert!(init["result"]["capabilities"]["tools"].is_object(), "the server must advertise tools: {init}");
        c.rpc("notifications/initialized", json!({}), false);
        c
    }

    fn rpc(&mut self, method: &str, params: Value, reply: bool) -> Value {
        let mut msg = json!({"jsonrpc": "2.0", "method": method, "params": params});
        if reply {
            self.id += 1;
            msg["id"] = json!(self.id);
        }
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();
        if !reply {
            return Value::Null;
        }
        loop {
            let mut line = String::new();
            assert!(self.out.read_line(&mut line).unwrap() > 0, "the server closed its stdout");
            let v: Value = serde_json::from_str(line.trim()).unwrap_or_else(|e| panic!("stdout carried a line that is not a protocol message ({e}): {line:?}"));
            if v["id"] == json!(self.id) {
                return v;
            }
        }
    }

    /// `tools/list`.
    pub fn list(&mut self) -> Vec<Value> {
        let r = self.rpc("tools/list", json!({}), true);
        self.tools_list_bytes = serde_json::to_string(&r["result"]).unwrap().len();
        r["result"]["tools"].as_array().cloned().unwrap_or_default()
    }

    /// `tools/call`.
    pub fn call(&mut self, name: &str, args: Value) -> Reply {
        self.sent_bytes += args.to_string().len();
        let r = self.rpc("tools/call", json!({"name": name, "arguments": args}), true);
        assert!(r.get("error").is_none(), "a protocol error for {name}: {r}");
        let mut reply = Reply { text: String::new(), images: Vec::new(), is_error: r["result"]["isError"] == json!(true) };
        for c in r["result"]["content"].as_array().into_iter().flatten() {
            match c["type"].as_str() {
                Some("text") => reply.text.push_str(c["text"].as_str().unwrap_or("")),
                Some("image") => reply.images.push(c["data"].as_str().unwrap_or("").to_string()),
                _ => {}
            }
        }
        self.text_bytes += reply.text.len();
        reply
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
