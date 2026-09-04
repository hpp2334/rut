//! End-to-end over the wire — the real `Server` on in-memory duplex
//! streams, framed JSON-RPC exactly like an editor speaks it: initialize →
//! didOpen (diagnostics on the wire) → semanticTokens/full → documentSymbol.

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tower_lsp_server::{LspService, Server};

use rut_lsp::server::Backend;

struct Editor {
    tx: DuplexStream, // -> server stdin
    rx: DuplexStream, // <- server stdout
    next_id: i64,
}

impl Editor {
    async fn send(&mut self, method: &str, params: Value, id: Option<i64>) {
        let mut v = json!({"jsonrpc": "2.0", "method": method});
        if !params.is_null() {
            v["params"] = params;
        }
        if let Some(id) = id {
            v["id"] = json!(id);
        }
        let body = v.to_string();
        let frame = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        self.tx.write_all(frame.as_bytes()).await.unwrap();
    }

    /// Read one framed message (headers + body).
    async fn recv(&mut self) -> Value {
        let mut header = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            self.rx.read_exact(&mut byte).await.unwrap();
            header.push(byte[0]);
            if header.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let header = String::from_utf8_lossy(&header);
        let len: usize = header
            .lines()
            .find_map(|l| l.strip_prefix("Content-Length: "))
            .and_then(|v| v.trim().parse().ok())
            .expect("content-length header");
        let mut body = vec![0u8; len];
        self.rx.read_exact(&mut body).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    /// Send a request; return its response (server notifications such as
    /// publishDiagnostics are drained and returned too, via `notifications`).
    async fn request(&mut self, method: &str, params: Value, notifications: &mut Vec<Value>) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(method, params, Some(id)).await;
        loop {
            let msg = self.recv().await;
            if msg.get("id").and_then(Value::as_i64) == Some(id) {
                return msg;
            }
            notifications.push(msg);
        }
    }

    async fn notify(&mut self, method: &str, params: Value) {
        self.send(method, params, None).await;
    }

    /// Notifications (publishDiagnostics) can trail the next response —
    /// the server dispatches concurrently — so drain with a timeout until
    /// something matches.
    async fn drain_until(&mut self, mut pred: impl FnMut(&Value) -> bool) -> Option<Value> {
        for _ in 0..50 {
            match tokio::time::timeout(std::time::Duration::from_millis(100), self.recv()).await {
                Ok(msg) if pred(&msg) => return Some(msg),
                Ok(_) => continue,
                Err(_) => return None, // wire went quiet
            }
        }
        None
    }
}

async fn spawn() -> Editor {
    let (client_tx, server_rx) = tokio::io::duplex(4096);
    let (server_tx, client_rx) = tokio::io::duplex(4096);
    let (service, socket) = LspService::new(|client| Backend::new(client));
    tokio::spawn(async move {
        Server::new(server_rx, server_tx, socket).serve(service).await;
    });
    let mut editor = Editor { tx: client_tx, rx: client_rx, next_id: 0 };
    let mut drain = Vec::new();
    let init = editor
        .request(
            "initialize",
            json!({"capabilities": {}}),
            &mut drain,
        )
        .await;
    assert!(init["result"]["capabilities"]["semanticTokensProvider"].is_object());
    assert!(init["result"]["capabilities"]["documentSymbolProvider"].is_boolean());
    assert!(init["result"]["capabilities"]["textDocumentSync"].is_object());
    editor.notify("initialized", json!({})).await;
    editor
}

#[tokio::test]
async fn initialize_shutdown_round_trip() {
    let mut editor = spawn().await;
    let mut drain = Vec::new();
    let resp = editor.request("shutdown", Value::Null, &mut drain).await;
    assert!(resp.get("error").is_none(), "shutdown must succeed: {resp}");
    editor.notify("exit", json!({})).await;
}

#[tokio::test]
async fn semantic_tokens_symbols_and_diags_on_the_wire() {
    let mut editor = spawn().await;
    let src = "enum Color { Red, Green }\nfn area(r: f64): f64 { return r * 3.14; }\n";
    let uri = "file:///w/t.rut";
    editor
        .notify(
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": uri, "languageId": "rut", "version": 1, "text": src
            }}),
        )
        .await;

    let mut notes = Vec::new();
    let tokens = editor
        .request(
            "textDocument/semanticTokens/full",
            json!({"textDocument": {"uri": uri}}),
            &mut notes,
        )
        .await;
    let data = tokens["result"]["data"].as_array().expect("token data");
    assert_eq!(data.len() % 5, 0, "quintuple stream");
    assert!(!data.is_empty(), "clean file still yields tokens");

    // the empty-diagnostics publish either trailed into `notes` during the
    // request drain, or is still on the wire (concurrent dispatch)
    let publish = match notes
        .iter()
        .rev()
        .find(|m| m["method"] == "textDocument/publishDiagnostics")
    {
        Some(p) => p.clone(),
        None => editor
            .drain_until(|m| m["method"] == "textDocument/publishDiagnostics")
            .await
            .expect("publishDiagnostics arrives, even empty"),
    };
    assert_eq!(publish["params"]["uri"], uri);

    let mut none = Vec::new();
    let syms = editor
        .request(
            "textDocument/documentSymbol",
            json!({"textDocument": {"uri": uri}}),
            &mut none,
        )
        .await;
    let arr = syms["result"].as_array().expect("symbol array");
    let names: Vec<&str> = arr.iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["Color", "area"]);
}

#[tokio::test]
async fn broken_file_publishes_error_diags() {
    let mut editor = spawn().await;
    let uri = "file:///w/broken.rut";
    editor
        .notify(
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": uri, "languageId": "rut", "version": 1,
                "text": "fn broken(: unit {}\n"
            }}),
        )
        .await;
    let mut notes = Vec::new();
    editor.request("textDocument/documentSymbol", json!({"textDocument": {"uri": uri}}), &mut notes).await;
    let publish = match notes
        .iter()
        .rev()
        .find(|m| m["method"] == "textDocument/publishDiagnostics")
    {
        Some(p) => p.clone(),
        None => editor
            .drain_until(|m| m["method"] == "textDocument/publishDiagnostics")
            .await
            .expect("diagnostics published"),
    };
    let diags = publish["params"]["diagnostics"].as_array().unwrap();
    assert!(!diags.is_empty(), "parse error must surface");
    assert_eq!(diags[0]["source"], "rut");
    assert_eq!(diags[0]["severity"], 1); // ERROR
}

#[tokio::test]
async fn decl_files_parse_in_decl_mode() {
    let mut editor = spawn().await;
    // `host fn` is only legal in .d.rut (RFC 0030 §3) — the URI suffix
    // routes it to Mode::Decl
    let uri = "file:///w/plugin.d.rut";
    editor
        .notify(
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": uri, "languageId": "rut", "version": 1,
                "text": "host fn now_ms(): u64;\n"
            }}),
        )
        .await;
    let mut notes = Vec::new();
    let syms = editor
        .request("textDocument/documentSymbol", json!({"textDocument": {"uri": uri}}), &mut notes)
        .await;
    let arr = syms["result"].as_array().expect("symbol array");
    assert_eq!(arr[0]["name"], "now_ms");
    // and no diagnostics: Decl mode accepts the surface decl
    let has_publish = notes.iter().any(|n| {
        n["method"] == "textDocument/publishDiagnostics"
            && !n["params"]["diagnostics"].as_array().unwrap().is_empty()
    });
    assert!(!has_publish, "surface decls must not diagnose: {notes:?}");
}

