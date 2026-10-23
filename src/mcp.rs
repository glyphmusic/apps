//! MCP (Model Context Protocol) server of the app: AI assistants (Claude,
//! and any other MCP client) control Glyph through the tools in
//! `glyph_protocol::mcp`.
//!
//! Local mode: a tiny HTTP server on `127.0.0.1:<port>/mcp` (Streamable HTTP,
//! plain JSON answers, no event stream), guarded by a bearer token and
//! refusing browser origins other than localhost. Every tool call is handed
//! to the app's UI thread through a channel ([`McpRequest`]) and answered from
//! there (see `crate::mcp_tools`). Server mode reuses the same channel: the
//! relay loop (in `mcp_tools`) feeds calls fetched from the Glyph server.

use glyph_protocol::mcp;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// One tool call for the app; the answer (an MCP `CallToolResult`) goes back
/// through `reply`.
pub struct McpRequest {
    pub name: String,
    pub args: serde_json::Value,
    pub reply: flume::Sender<serde_json::Value>,
}

/// The running local server; dropping it stops it.
pub struct LocalServer {
    pub port: u16,
    stop: Arc<AtomicBool>,
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// A fresh random token (256 bits, hex).
pub fn new_token() -> String {
    glyph_core::random_token()
}

/// Starts the local server. Calls arrive on `calls`.
pub fn start_local(port: u16, token: String, calls: flume::Sender<McpRequest>) -> std::io::Result<LocalServer> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let token = Arc::new(token);
    std::thread::Builder::new().name("glyph-mcp".into()).spawn(move || {
        tracing::info!(port, "MCP server listening on 127.0.0.1");
        while !stop2.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let (token, calls) = (token.clone(), calls.clone());
                    let _ = std::thread::Builder::new().name("glyph-mcp-conn".into()).spawn(move || {
                        if let Err(e) = handle(stream, &token, &calls) {
                            tracing::debug!("MCP connection: {e}");
                        }
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(60)),
                Err(e) => {
                    tracing::warn!("MCP accept failed: {e}");
                    std::thread::sleep(Duration::from_millis(250));
                }
            }
        }
        tracing::info!("MCP server stopped");
    })?;
    Ok(LocalServer { port, stop })
}

const MAX_BODY: usize = 1024 * 1024;

fn respond(mut s: &TcpStream, status: &str, body: &[u8], ctype: &str) -> std::io::Result<()> {
    write!(
        s,
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    s.write_all(body)?;
    s.flush()
}

/// Constant-time comparison of the bearer token.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn handle(stream: TcpStream, token: &str, calls: &flume::Sender<McpRequest>) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;
    // Bound the entire request head, including lines without a newline.
    let mut reader = BufReader::new(&stream).take(16 * 1024);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
    let mut headers: Vec<(String, String)> = Vec::new();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
        if headers.len() > 100 {
            return respond(&stream, "431 Request Header Fields Too Large", b"", "text/plain");
        }
    }
    if reader.limit() == 0 {
        return respond(&stream, "431 Request Header Fields Too Large", b"", "text/plain");
    }
    let mut reader = reader.into_inner();
    let header = |k: &str| headers.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    // Browsers may only come from this computer (DNS rebinding).
    if let Some(origin) = header("origin") {
        let host = origin.split("://").nth(1).unwrap_or("").split([':', '/']).next().unwrap_or("");
        if !matches!(host, "localhost" | "127.0.0.1" | "[::1]") {
            return respond(&stream, "403 Forbidden", b"{\"error\":\"origin not allowed\"}", "application/json");
        }
    }
    if path.split('?').next() != Some("/mcp") {
        return respond(&stream, "404 Not Found", b"", "text/plain");
    }
    let auth = header("authorization").and_then(|a| a.strip_prefix("Bearer ").or_else(|| a.strip_prefix("bearer "))).unwrap_or("");
    if !same(auth.trim(), token) {
        return respond(&stream, "401 Unauthorized", b"{\"error\":\"missing or wrong bearer token\"}", "application/json");
    }
    if method != "POST" {
        // No server-initiated event stream: answers come with each POST.
        return respond(&stream, "405 Method Not Allowed", b"", "text/plain");
    }
    if header("transfer-encoding").is_some() || headers.iter().filter(|(k, _)| k == "content-length").count() > 1 {
        return respond(&stream, "400 Bad Request", b"unsupported request framing", "text/plain");
    }
    let len: usize = match header("content-length").and_then(|v| v.parse().ok()) {
        Some(len) => len,
        None => return respond(&stream, "400 Bad Request", b"valid content-length required", "text/plain"),
    };
    if len > MAX_BODY {
        return respond(&stream, "413 Payload Too Large", b"", "text/plain");
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;
    let Ok(msg) = serde_json::from_slice::<serde_json::Value>(&body) else {
        let err = serde_json::json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"parse error"}});
        return respond(&stream, "400 Bad Request", err.to_string().as_bytes(), "application/json");
    };
    let answers: Vec<serde_json::Value> = match &msg {
        serde_json::Value::Array(batch) if batch.is_empty() => vec![invalid_request()],
        serde_json::Value::Array(batch) => batch.iter().filter_map(|m| answer(m, calls)).collect(),
        m => answer(m, calls).into_iter().collect(),
    };
    if answers.is_empty() {
        // Only notifications / responses.
        return respond(&stream, "202 Accepted", b"", "text/plain");
    }
    let out = if msg.as_array().is_some_and(|b| !b.is_empty()) { serde_json::Value::Array(answers) } else { answers.into_iter().next().unwrap_or_default() };
    respond(&stream, "200 OK", out.to_string().as_bytes(), "application/json")
}

/// The JSON-RPC answer to one message (`None` for notifications).
fn invalid_request() -> serde_json::Value {
    serde_json::json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"invalid request"}})
}

pub fn answer(m: &serde_json::Value, calls: &flume::Sender<McpRequest>) -> Option<serde_json::Value> {
    if !m.is_object() || m["jsonrpc"] != "2.0" || !m["method"].is_string()
        || m.get("id").is_some_and(|id| !(id.is_string() || id.is_number() || id.is_null()))
    {
        return Some(invalid_request());
    }
    let id = m.get("id").cloned()?;
    let method = m["method"].as_str().unwrap_or("");
    let ok = |result: serde_json::Value| serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result });
    let err = |code: i64, message: String| serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } });
    if m.get("params").is_some_and(|p| !p.is_object()) {
        return Some(err(-32602, "params must be an object".into()));
    }
    // The app thread answers with the tools its user allows.
    if method == "tools/list" {
        let listed = call(mcp::LIST_TOOLS.into(), serde_json::json!({}), calls);
        return Some(if listed["tools"].is_array() { ok(listed) } else { err(-32603, "Glyph did not answer".into()) });
    }
    if let Some(r) = mcp::handle_meta(method, "glyph", env!("CARGO_PKG_VERSION")) {
        return Some(match r {
            Ok(v) => ok(v),
            Err((c, msg)) => err(c, msg),
        });
    }
    if method != "tools/call" {
        return None;
    }
    let name = m["params"]["name"].as_str().unwrap_or("").to_string();
    if !mcp::tools().iter().any(|t| t["name"] == name.as_str()) {
        return Some(err(-32602, format!("unknown tool: {name}")));
    }
    let args = m["params"].get("arguments").cloned().unwrap_or(serde_json::json!({}));
    if !args.is_object() {
        return Some(err(-32602, "tool arguments must be an object".into()));
    }
    Some(ok(call(name, args, calls)))
}

/// Runs one tool in the app and waits for its result.
pub fn call(name: String, args: serde_json::Value, calls: &flume::Sender<McpRequest>) -> serde_json::Value {
    let (tx, rx) = flume::bounded(1);
    if calls.send(McpRequest { name, args, reply: tx }).is_err() {
        return mcp::text_result(&"Glyph is closing.".into(), true);
    }
    match rx.recv_timeout(Duration::from_millis(mcp::CALL_TIMEOUT_MS)) {
        Ok(r) => r,
        Err(flume::RecvTimeoutError::Timeout) => mcp::text_result(&"Glyph did not answer in time.".into(), true),
        Err(flume::RecvTimeoutError::Disconnected) => mcp::text_result(&"Glyph closed before answering.".into(), true),
    }
}

/// Client configuration for the user to paste (Claude Desktop / Code and
/// other MCP clients that speak Streamable HTTP).
pub fn client_config(url: &str, token: &str) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "mcpServers": { "glyph": { "type": "http", "url": url, "headers": { "Authorization": format!("Bearer {token}") } } }
    }))
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_malformed_requests_without_dispatching() {
        let (tx, rx) = flume::unbounded();
        for msg in [serde_json::json!(null), serde_json::json!(7), serde_json::json!({}),
            serde_json::json!({"jsonrpc":"1.0","id":1,"method":"ping"}),
            serde_json::json!({"jsonrpc":"2.0","id":[],"method":"ping"})] {
            assert_eq!(answer(&msg, &tx).unwrap()["error"]["code"], -32600);
        }
        for params in [serde_json::json!([]), serde_json::json!({"name":"now_playing","arguments":null})] {
            let msg = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":params});
            assert_eq!(answer(&msg, &tx).unwrap()["error"]["code"], -32602);
        }
        assert!(rx.is_empty());
    }

    #[test]
    fn validates_http_framing_and_batches() {
        let (tx, _rx) = flume::unbounded();
        let srv = start_local(0, "token".into(), tx).unwrap();
        let send = |extra: &str, body: &str| {
            let mut s = TcpStream::connect(("127.0.0.1", srv.port)).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            write!(s, "POST /mcp HTTP/1.1\r\nAuthorization: Bearer token\r\n{extra}\r\n{body}").unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).unwrap();
            out
        };
        assert!(send("Content-Length: nope\r\n", "").starts_with("HTTP/1.1 400"));
        assert!(send("Content-Length: 0\r\nContent-Length: 0\r\n", "").starts_with("HTTP/1.1 400"));
        assert!(send("Transfer-Encoding: chunked\r\n", "").starts_with("HTTP/1.1 400"));
        let empty = send("Content-Length: 2\r\n", "[]");
        assert!(empty.contains("\"code\":-32600"), "{empty}");
        let notification = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        assert!(send(&format!("Content-Length: {}\r\n", notification.len()), notification).starts_with("HTTP/1.1 202"));
    }

    #[test]
    fn answers_meta_and_routes_tool_calls() {
        let (tx, rx) = flume::unbounded::<McpRequest>();
        std::thread::spawn(move || {
            while let Ok(r) = rx.recv() {
                let answer = if r.name == mcp::LIST_TOOLS {
                    serde_json::json!({ "tools": mcp::Permissions::default().tools() })
                } else {
                    glyph_protocol::mcp::text_result(&serde_json::json!({ "tool": r.name }), false)
                };
                let _ = r.reply.send(answer);
            }
        });
        let list = answer(&serde_json::json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}), &tx).unwrap();
        let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(names.contains(&"play") && !names.contains(&"delete_playlist"), "{names:?}");
        let init = answer(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}), &tx).unwrap();
        assert_eq!(init["result"]["protocolVersion"], mcp::PROTOCOL_VERSION);
        assert!(answer(&serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"}), &tx).is_none());
        let call = answer(&serde_json::json!({"jsonrpc":"2.0","id":"a","method":"tools/call","params":{"name":"now_playing","arguments":{}}}), &tx).unwrap();
        assert_eq!(call["result"]["structuredContent"]["tool"], "now_playing");
        let bad = answer(&serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"rm_rf"}}), &tx).unwrap();
        assert_eq!(bad["error"]["code"], -32602);
    }

    #[test]
    fn local_server_needs_the_token() {
        let (tx, _rx) = flume::unbounded::<McpRequest>();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let _srv = start_local(port, "secret-token".into(), tx).unwrap();
        let send = |auth: &str| {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            let body = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
            write!(s, "POST /mcp HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {auth}\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).unwrap();
            out
        };
        assert!(send("wrong").starts_with("HTTP/1.1 401"));
        let ok = send("secret-token");
        assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
        assert!(ok.contains("\"result\":{}"));
    }
}
