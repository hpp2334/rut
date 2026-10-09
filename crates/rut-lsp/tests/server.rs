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
    spawn_at("").await
}

/// `root`: a file:// uri scanned for rut files at initialize (empty = none)
async fn spawn_at(root: &str) -> Editor {
    let (client_tx, server_rx) = tokio::io::duplex(4096);
    let (server_tx, client_rx) = tokio::io::duplex(4096);
    let (service, socket) = LspService::new(|client| Backend::new(client));
    tokio::spawn(async move {
        Server::new(server_rx, server_tx, socket).serve(service).await;
    });
    let mut editor = Editor { tx: client_tx, rx: client_rx, next_id: 0 };
    let mut drain = Vec::new();
    let mut init_params = json!({"capabilities": {}});
    if !root.is_empty() {
        init_params["rootUri"] = json!(root);
    }
    let init = editor
        .request(
            "initialize",
            init_params,
            &mut drain,
        )
        .await;
    assert!(init["result"]["capabilities"]["semanticTokensProvider"].is_object());
    assert!(init["result"]["capabilities"]["documentSymbolProvider"].is_boolean());
    assert!(init["result"]["capabilities"]["textDocumentSync"].is_object());
    assert!(init["result"]["capabilities"]["referencesProvider"].is_boolean());
    assert!(init["result"]["capabilities"]["signatureHelpProvider"].is_object());
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
    let src = "enum Color { Red, Green }\nfn area(r: f64) -> f64 { return r * 3.14; }\n";
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
async fn broken_file_publishes_error_diags() {    let mut editor = spawn().await;
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
    // `host fn` is only legal in .d.rut — the URI suffix
    // routes it to Mode::Decl
    let uri = "file:///w/plugin.d.rut";
    editor
        .notify(
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": uri, "languageId": "rut", "version": 1,
                "text": "host fn now_ms() -> u64;\n"
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


/// (line, character) of the `n`-th occurrence of `needle` in `src` —
/// chars within the line, like LSP positions
fn pos_of(src: &str, needle: &str, n: usize) -> (u32, u32) {
    let mut seen = 0;
    for (li, line) in src.lines().enumerate() {
        let mut from = 0;
        while let Some(at) = line[from..].find(needle) {
            if seen == n {
                return (li as u32, (from + at) as u32);
            }
            seen += 1;
            from += at + needle.len();
        }
    }
    panic!("{needle} occurrence {n} not found");
}

#[tokio::test]
async fn hover_on_the_wire() {
    let mut editor = spawn().await;
    let src = "\
// a circle
class Circle {
    pub r: f64;
    fn area(self) -> f64 { return 3.14; }
}
fn go(c: Circle) -> f64 { return c.area(); }
";
    let uri = "file:///w/hover.rut";
    editor
        .notify(
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": uri, "languageId": "rut", "version": 1, "text": src
            }}),
        )
        .await;

    let mut notes = Vec::new();
    // method hover at the use site `c.area()`
    let (l, c) = pos_of(src, "area", 1);
    let h = editor
        .request(
            "textDocument/hover",
            json!({"textDocument": {"uri": uri}, "position": {"line": l, "character": c}}),
            &mut notes,
        )
        .await;
    let md = h["result"]["contents"]["value"].as_str().expect("hover markdown");
    assert!(md.contains("fn area(self) -> f64"), "signature: {md}");
    assert!(md.contains("in `Circle`"), "owner: {md}");

    // class hover shows the struct definition + doc
    let (l, c) = pos_of(src, "Circle", 1);
    let h = editor
        .request(
            "textDocument/hover",
            json!({"textDocument": {"uri": uri}, "position": {"line": l, "character": c}}),
            &mut notes,
        )
        .await;
    let md = h["result"]["contents"]["value"].as_str().expect("hover markdown");
    assert!(md.contains("class Circle {"), "{md}");
    assert!(md.contains("pub r: f64"), "{md}");
    assert!(md.contains("a circle"), "{md}");

    // miss is null, never wrong text — inside the header comment
    let h = editor
        .request(
            "textDocument/hover",
            json!({"textDocument": {"uri": uri}, "position": {"line": 0, "character": 3}}),
            &mut notes,
        )
        .await;
    assert!(h["result"].is_null(), "no definition there: {h}");
}

#[tokio::test]
async fn hover_resolves_std_surface() {
    let mut editor = spawn().await;
    // v1.1: the str/bytes members are declared per type in the prelude
    let src = "fn n() -> i32 { return \"abc\".len(); }\n";
    let uri = "file:///w/str.rut";
    editor
        .notify(
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": uri, "languageId": "rut", "version": 1, "text": src
            }}),
        )
        .await;
    let mut notes = Vec::new();
    let (l, c) = pos_of(src, "len", 0);
    let h = editor
        .request(
            "textDocument/hover",
            json!({"textDocument": {"uri": uri}, "position": {"line": l, "character": c}}),
            &mut notes,
        )
        .await;
    let md = h["result"]["contents"]["value"].as_str().expect("hover markdown");
    assert!(md.contains("len"), "string member: {md}");
    assert!(md.contains("core"), "provenance: {md}");
}

#[tokio::test]
async fn inlay_hints_on_the_wire() {
    let mut editor = spawn().await;
    let src = "\
class Circle {
    r: f64;
}
impl Circle {
    fn grown(self, k: f64) -> Circle { return self; }
}
fn go() -> f64 {
    let c = Circle.new(1.0);
    let big = c.grown(2.0);
    return big.r;
}
";
    let uri = "file:///w/inlay.rut";
    editor
        .notify(
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": uri, "languageId": "rut", "version": 1, "text": src
            }}),
        )
        .await;

    let mut notes = Vec::new();
    // whole document
    let resp = editor
        .request(
            "textDocument/inlayHint",
            json!({
                "textDocument": {"uri": uri},
                "range": {"start": {"line": 0, "character": 0}, "end": {"line": 20, "character": 0}}
            }),
            &mut notes,
        )
        .await;
    let hints = resp["result"].as_array().expect("hint array");
    let labels: Vec<&str> = hints
        .iter()
        .map(|h| h["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, vec![": Circle", ": Circle", "k:"], "unannotated lets then the call arg");
    // kinds share the LSP numbering (1 Type, 2 Parameter)
    assert_eq!(hints[0]["kind"], 1);
    assert_eq!(hints[2]["kind"], 2);
    // the type hint hangs right after the declaring ident (`let c⌊:⌋`)
    assert_eq!(hints[0]["position"]["line"], 7);
    assert_eq!(hints[0]["position"]["character"], 9);
    // the tooltip is the hover markdown (the consistency law)
    let tip = hints[0]["tooltip"]["value"].as_str().expect("tooltip");
    assert!(tip.contains("let c: Circle"), "{tip}");
    assert!(tip.contains("type inferred"), "{tip}");
    // the param hint sits at the arg's first byte (`grown(⌊k:⌋2.0)`)
    assert_eq!(hints[2]["position"]["line"], 8);
    assert_eq!(hints[2]["position"]["character"], 22);

    // a range elsewhere is empty — the client asks per visible region
    let resp = editor
        .request(
            "textDocument/inlayHint",
            json!({
                "textDocument": {"uri": uri},
                "range": {"start": {"line": 0, "character": 0}, "end": {"line": 4, "character": 0}}
            }),
            &mut notes,
        )
        .await;
    assert!(resp["result"].as_array().expect("hint array").is_empty());

    // an unknown document is null, never a guess
    let resp = editor
        .request(
            "textDocument/inlayHint",
            json!({
                "textDocument": {"uri": "file:///w/never-opened.rut"},
                "range": {"start": {"line": 0, "character": 0}, "end": {"line": 1, "character": 0}}
            }),
            &mut notes,
        )
        .await;
    assert!(resp["result"].is_null());
}

#[tokio::test]
async fn references_and_signature_help_on_the_wire() {
    let mut editor = spawn().await;
    // capabilities: both features advertised
    // (checked on the initialize result inside spawn() for the older
    // ones; here the two new ones)
    // — spawn() asserts the pre-phase capabilities; the new ones:
    let src = "\
class Circle {
    r: f64;
}
impl Circle {
    fn grown(self, k: f64) -> Circle { return self; }
}
fn hex_val(c: str, k: i32) -> i32 {
    return k;
}
fn go() -> i32 {
    let v = 1;
    if (v > 0) {
        let v = 2;
        return hex_val(\"a\", v);
    }
    return v + hex_val(\"b\", 0);
}
";
    let uri = "file:///w/refs.rut";
    editor
        .notify(
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": uri, "languageId": "rut", "version": 1, "text": src
            }}),
        )
        .await;
    let mut notes = Vec::new();

    // ---- references: the OUTER v — its if-condition use + final
    // return; the inner shadow's decl/return do NOT leak in ----
    let (l, c) = pos_of(src, "let v = 1;", 0);
    let resp = editor
        .request(
            "textDocument/references",
            json!({
                "textDocument": {"uri": uri},
                "position": {"line": l, "character": c + 4},
                "context": {"includeDeclaration": false}
            }),
            &mut notes,
        )
        .await;
    let locs = resp["result"].as_array().expect("reference locations");
    let mut spots: Vec<(u32, u32)> = locs
        .iter()
        .map(|l| {
            (
                l["range"]["start"]["line"].as_u64().unwrap() as u32,
                l["range"]["start"]["character"].as_u64().unwrap() as u32,
            )
        })
        .collect();
    spots.sort();
    let (cl, cc) = pos_of(src, "if (v > 0)", 0);
    let (rl, rc) = pos_of(src, "return v +", 0);
    assert_eq!(
        spots,
        vec![(cl, cc + 4), (rl, rc + 7)],
        "shadow-aware use set: {locs:?}"
    );

    // the INNER v: exactly its own hex_val argument
    let (l2, c2) = pos_of(src, "let v = 2;", 0);
    let resp = editor
        .request(
            "textDocument/references",
            json!({
                "textDocument": {"uri": uri},
                "position": {"line": l2, "character": c2 + 4},
                "context": {"includeDeclaration": false}
            }),
            &mut notes,
        )
        .await;
    let locs = resp["result"].as_array().expect("inner refs");
    assert_eq!(locs.len(), 1, "the shadow keeps its own uses: {locs:?}");
    assert_eq!(locs[0]["range"]["start"]["line"], 13);

    // includeDeclaration appends the declaring ident
    let resp = editor
        .request(
            "textDocument/references",
            json!({
                "textDocument": {"uri": uri},
                "position": {"line": l2, "character": c2 + 4},
                "context": {"includeDeclaration": true}
            }),
            &mut notes,
        )
        .await;
    let locs = resp["result"].as_array().expect("decl-inclusive refs");
    assert_eq!(locs.len(), 2, "use + decl: {locs:?}");

    // ---- signature help: active parameter from the comma depth ----
    // needle: h0 e1 x2 _3 v4 a5 l6 (7 "8 a9 "10 ,11 ' '12 v13
    let (l, c) = pos_of(src, "hex_val(\"a\", v)", 0);
    let resp = editor
        .request(
            "textDocument/signatureHelp",
            json!({
                "textDocument": {"uri": uri},
                "position": {"line": l, "character": c + 8}
            }),
            &mut notes,
        )
        .await;
    let help = resp["result"].as_object().expect("signature help");
    let sig = &help["signatures"][0];
    assert_eq!(
        sig["label"].as_str().unwrap(),
        "fn hex_val(c: str, k: i32) -> i32",
        "the signature renders verbatim"
    );
    let params = sig["parameters"].as_array().unwrap();
    let labels: Vec<&str> = params
        .iter()
        .map(|p| p["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, vec!["c: str", "k: i32"], "params as written");
    assert_eq!(help["activeSignature"], 0);
    assert_eq!(help["activeParameter"], 0, "the cursor sits on the first arg");
    // the second arg highlights slot 1
    let resp = editor
        .request(
            "textDocument/signatureHelp",
            json!({
                "textDocument": {"uri": uri},
                "position": {"line": l, "character": c + 13}
            }),
            &mut notes,
        )
        .await;
    assert_eq!(resp["result"]["activeParameter"], 1);

    // ---- a mismatch shows NO help (never wrong help) ----
    let bad = "\
fn f(a: i32, b: i32) -> i32 {
    return a;
}
entry fn main() -> i32 {
    return f(1, 2, 3);
}
";
    let bad_uri = "file:///w/arity.rut";
    editor
        .notify(
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": bad_uri, "languageId": "rut", "version": 1, "text": bad
            }}),
        )
        .await;
    let (bl, bc) = pos_of(bad, "3);", 0);
    let resp = editor
        .request(
            "textDocument/signatureHelp",
            json!({
                "textDocument": {"uri": bad_uri},
                "position": {"line": bl, "character": bc}
            }),
            &mut notes,
        )
        .await;
    assert!(resp["result"].is_null(), "three args for two params: {resp}");

    // ---- cross-file references: the reverse use-graph edge. The
    // server face indexes the workspace at initialize, so the importing
    // doc and the exporting lib both live on a scratch disk workspace ----
    let ws = std::env::temp_dir().join(format!("rut-lsp-refs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(ws.join("gadgets")).unwrap();
    let lib_src = "class Widget {\n    id: i32;\n}\n";
    let main_src = "use gadgets::{ Widget };\nfn main() -> nil {\n    let w = Widget.new();\n}\n";
    let lib_uri = format!("file://{}/gadgets/lib.rut", ws.to_str().unwrap());
    let main_uri = format!("file://{}/main.rut", ws.to_str().unwrap());
    std::fs::write(ws.join("gadgets/lib.rut"), lib_src).unwrap();
    std::fs::write(ws.join("main.rut"), main_src).unwrap();
    let ws_uri = format!("file://{}", ws.to_str().unwrap());
    let mut root_editor = spawn_at(&ws_uri).await;
    // the workspace scan runs off the hot initialize path — give it a
    // beat before the first cross-file query
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let mut notes = Vec::new();
    root_editor
        .notify(
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": lib_uri, "languageId": "rut", "version": 1, "text": lib_src
            }}),
        )
        .await;
    root_editor
        .notify(
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": main_uri, "languageId": "rut", "version": 1, "text": main_src
            }}),
        )
        .await;
    // references FROM THE LIB'S DECL: the importing doc's use name +
    // usage come back through the reverse edge
    let (wl, wc) = pos_of(lib_src, "class Widget", 0);
    let resp = root_editor
        .request(
            "textDocument/references",
            json!({
                "textDocument": {"uri": lib_uri},
                "position": {"line": wl, "character": wc + 6},
                "context": {"includeDeclaration": false}
            }),
            &mut notes,
        )
        .await;
    let locs = resp["result"].as_array().expect("cross-file refs");
    assert_eq!(locs.len(), 2, "use name + usage: {locs:?}");
    assert!(locs.iter().all(|l| l["uri"] == main_uri), "{locs:?}");
    let _ = remove_ws(ws);
}

fn remove_ws(ws: std::path::PathBuf) -> std::io::Result<()> {
    std::fs::remove_dir_all(ws)
}

// ---- the dep walk over the wire (server-side manifest deps, cache-only) ----
//
// Mirrors the wasm e2e's fixture workspace: a path dep OUTSIDE the
// workspace root (the scan never reaches it — only the manifest walk
// can), a url dep primed into `.rut/cache/` from the committed dist/std
// bundle (the offline pattern), a url dep whose cache is a MISS, and an
// unpinned url row — the two showMessage lanes.

const POUCH_BUNDLE: &[u8] = include_bytes!("../../../dist/std/pouch.rutbundle");

/// the same digest the server's mount door checks (the pin is law)
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

struct DepFixture {
    base: std::path::PathBuf,
    /// the workspace root — initialize's rootUri
    ws: std::path::PathBuf,
    main_src: String,
    main_uri: String,
}

fn write_dep_fixture(name: &str) -> DepFixture {
    let base = std::env::temp_dir().join(format!("rut-lsp-deps-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let ws = base.join("ws");
    let dep = base.join("outside"); // a SIBLING of ws — outside the scan root
    std::fs::create_dir_all(ws.join(".rut/cache")).unwrap();
    std::fs::create_dir_all(&dep).unwrap();

    // the path dep — a module directory with its own manifest. The
    // lib spells a `pub mod layout;` edge and the child dir carries
    // its `mod.rut` — the editor walk mounts the tree like the loader
    std::fs::write(
        dep.join("rut.jsonc"),
        r#"{ "name": "gadgets", "entry": { "lib": "./lib.rut" } }"#,
    )
    .unwrap();
    let lib_src = "pub class Widget {\n    id: i32;\n}\npub fn tag() -> i32 {\n    return 1;\n}\npub mod layout;\n";
    std::fs::write(dep.join("lib.rut"), lib_src).unwrap();
    std::fs::create_dir_all(dep.join("layout")).unwrap();
    std::fs::write(
        dep.join("layout/mod.rut"),
        "pub struct Column {\n    w: i32;\n}\npub fn mk() -> Column { return Column { w: 1 }; }\n",
    )
    .unwrap();

    // the url rows: pouch pinned + PRIMED from the committed bundle,
    // ghost pinned but primed nowhere (the miss), bare unpinned
    const POUCH_URL: &str = "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v8/dist/std/pouch.rutbundle";
    const GHOST_URL: &str = "https://example.invalid/ghost.rutbundle";
    const BARE_URL: &str = "https://example.invalid/bare.rutbundle";
    let pouch_cache = rut_lsp::deps::cache_path(POUCH_URL); // .rut/cache/<sha256(url)>.rutbundle
    std::fs::write(ws.join(&pouch_cache), POUCH_BUNDLE).unwrap();
    let manifest = format!(
        r#"{{
  // the editor rides the same grammar the CLI parses
  "name": "wsapp",
  "entry": {{ "lib": "./main.rut" }},
  "deps": {{
    "gadgets": {{ "path": "../outside" }},
    "pouch": {{ "url": "{POUCH_URL}", "sha256": "{}" }},
    "ghost": {{ "url": "{GHOST_URL}", "sha256": "{}" }},
    "bare": {{ "url": "{BARE_URL}" }}
  }}
}}"#,
        sha256_hex(POUCH_BUNDLE),
        "0".repeat(64),
    );
    std::fs::write(ws.join("rut.jsonc"), manifest).unwrap();

    let main_src = "\
use gadgets::{ Widget };
use gadgets::layout::{ Column };
use pouch::{ Vec };
mod helpers;
fn main() -> nil {
    let w = Widget.new();
    let v: Vec<i32> = Vec.new();
    let c: Column = Column { w: 1 };
    let a: helpers.Slot = helpers.mk(1);
    v.push(1);
}
";
    std::fs::write(ws.join("main.rut"), main_src).unwrap();
    std::fs::create_dir_all(ws.join("helpers")).unwrap();
    std::fs::write(
        ws.join("helpers/mod.rut"),
        "pub struct Slot {\n    n: i32;\n}\npub fn mk(n: i32) -> Slot { return Slot { n: n }; }\n",
    )
    .unwrap();
    let main_uri = format!("file://{}/main.rut", ws.display());
    DepFixture { base, ws, main_src: main_src.to_string(), main_uri }
}

/// the pouch bundle's own source, only reachable through the CACHE mount
/// — the std surface's pouch index carries the relative `rut/pouch.rut`
/// origin instead, so this uri on the wire PROVES the cached bytes
/// mounted
const BUNDLE_POUCH_ORIGIN: &str = "bundle:pouch/pouch.rut";

#[tokio::test]
async fn dep_walk_resolves_path_and_cached_url_deps() {
    let fx = write_dep_fixture("resolve");
    let ws_uri = format!("file://{}", fx.ws.display());
    let mut editor = spawn_at(&ws_uri).await;
    // the scan + dep walk run off the hot initialize path — give them a
    // beat before the first cross-dep query
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let mut notes = Vec::new();
    editor
        .notify(
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": fx.main_uri, "languageId": "rut", "version": 1, "text": fx.main_src
            }}),
        )
        .await;

    // the path dep OUTSIDE the scan root: go-to-definition from the
    // `Widget.new()` use site lands in the dep's own file
    let (l, c) = pos_of(&fx.main_src, "Widget", 1);
    let resp = editor
        .request(
            "textDocument/definition",
            json!({"textDocument": {"uri": fx.main_uri}, "position": {"line": l, "character": c}}),
            &mut notes,
        )
        .await;
    let locs = resp["result"].as_array().expect("definition locations");
    assert!(
        locs.iter().any(|l| l["uri"].as_str().unwrap().ends_with("/outside/lib.rut")),
        "the outside path dep answers F12: {resp}"
    );

    // the CACHED url dep: the use-line `Vec` jumps to the bundle's own
    // origin — minted only by mounting the pinned cache bytes
    let (l, c) = pos_of(&fx.main_src, "Vec", 0);
    let resp = editor
        .request(
            "textDocument/definition",
            json!({"textDocument": {"uri": fx.main_uri}, "position": {"line": l, "character": c}}),
            &mut notes,
        )
        .await;
    let locs = resp["result"].as_array().expect("use-path definition");
    let uris: Vec<&str> = locs.iter().map(|l| l["uri"].as_str().unwrap()).collect();
    assert!(
        uris.contains(&BUNDLE_POUCH_ORIGIN),
        "the cached bundle's surface answers the use graph: {uris:?}"
    );

    // and the bundle's member surface hovers (v.push)
    let (l, c) = pos_of(&fx.main_src, "push", 0);
    let h = editor
        .request(
            "textDocument/hover",
            json!({"textDocument": {"uri": fx.main_uri}, "position": {"line": l, "character": c}}),
            &mut notes,
        )
        .await;
    let md = h["result"]["contents"]["value"].as_str().expect("hover markdown");
    assert!(md.contains("push"), "the bundle member surface: {md}");

    // the path dep completes: `use gadgets::⏐` offers its public names
    let scratch = "use gadgets::\nfn main() -> nil {\n}\n";
    let scratch_uri = format!("file://{}/scratch.rut", fx.ws.display());
    editor
        .notify(
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": scratch_uri, "languageId": "rut", "version": 1, "text": scratch
            }}),
        )
        .await;
    let comp = editor
        .request(
            "textDocument/completion",
            json!({"textDocument": {"uri": scratch_uri}, "position": {"line": 0, "character": 13}}),
            &mut notes,
        )
        .await;
    let items = comp["result"].as_array().expect("completion items");
    let labels: Vec<&str> = items.iter().map(|i| i["label"].as_str().unwrap()).collect();
    assert!(labels.contains(&"Widget"), "the dep type completes: {labels:?}");
    assert!(labels.contains(&"tag"), "the dep fn completes: {labels:?}");
    let _ = remove_ws(fx.base);
}

/// the dep walk's mod dimension: the path dep's `pub mod layout;`
/// mounts `layout/mod.rut`, the root package's own `mod helpers;`
/// mounts `helpers/mod.rut` — use-through-mods completion, the
/// segment hover, the `mod` decl hover, and the position-path tier
/// all answer on the wire
#[tokio::test]
async fn mod_walk_completion_and_hover_on_the_wire() {
    let fx = write_dep_fixture("modwalk");
    let ws_uri = format!("file://{}", fx.ws.display());
    let mut editor = spawn_at(&ws_uri).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let mut notes = Vec::new();
    editor
        .notify(
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": fx.main_uri, "languageId": "rut", "version": 1, "text": fx.main_src
            }}),
        )
        .await;

    // use-through-mods completion: `use gadgets::layout::⏐` (a fresh
    // doc, the typed prefix empty) offers the mounted child's exports
    let scratch = "use gadgets::layout::\nfn main() -> nil {\n}\n";
    let scratch_uri = format!("file://{}/modscratch.rut", fx.ws.display());
    editor
        .notify(
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": scratch_uri, "languageId": "rut", "version": 1, "text": scratch
            }}),
        )
        .await;
    let comp = editor
        .request(
            "textDocument/completion",
            json!({"textDocument": {"uri": scratch_uri}, "position": {"line": 0, "character": 20}}),
            &mut notes,
        )
        .await;
    let items = comp["result"].as_array().expect("completion items");
    let labels: Vec<&str> = items.iter().map(|i| i["label"].as_str().unwrap()).collect();
    assert!(labels.contains(&"Column"), "the walked module's type completes: {labels:?}");
    assert!(labels.contains(&"mk"), "the walked module's fn completes: {labels:?}");

    // the segment hover: `layout` inside `use gadgets::layout::..`
    // renders the resolved module (mounted file + child count)
    let (l, c) = pos_of(&fx.main_src, "layout", 0);
    let h = editor
        .request(
            "textDocument/hover",
            json!({"textDocument": {"uri": fx.main_uri}, "position": {"line": l, "character": c}}),
            &mut notes,
        )
        .await;
    let md = h["result"]["contents"]["value"].as_str().expect("segment hover markdown");
    assert!(md.contains("file module"), "{md}");
    assert!(md.contains("layout/mod.rut"), "{md}");

    // the `mod helpers;` decl hover: kind + mounted file (no children)
    let (l, c) = pos_of(&fx.main_src, "helpers", 0);
    let h = editor
        .request(
            "textDocument/hover",
            json!({"textDocument": {"uri": fx.main_uri}, "position": {"line": l, "character": c}}),
            &mut notes,
        )
        .await;
    let md = h["result"]["contents"]["value"].as_str().expect("mod decl hover markdown");
    assert!(md.contains("\nmod helpers;"), "{md}");
    assert!(md.contains("file module — `helpers/mod.rut`"), "{md}");
    assert!(md.contains("no child modules"), "{md}");

    // position-path completion: after `helpers::` in the package's own
    // file, the module's members visible to the root
    let (l, c) = pos_of(&fx.main_src, "Slot", 0);
    let comp = editor
        .request(
            "textDocument/completion",
            json!({"textDocument": {"uri": fx.main_uri}, "position": {"line": l, "character": c}}),
            &mut notes,
        )
        .await;
    let items = comp["result"].as_array().expect("position completion items");
    let labels: Vec<&str> = items.iter().map(|i| i["label"].as_str().unwrap()).collect();
    assert!(labels.contains(&"Slot"), "the module's type completes: {labels:?}");
    assert!(labels.contains(&"mk"), "the module's fn completes: {labels:?}");
    let _ = remove_ws(fx.base);
}

#[tokio::test]
async fn dep_walk_hint_once_for_cache_miss_and_unpinned() {
    let fx = write_dep_fixture("hints");
    let ws_uri = format!("file://{}", fx.ws.display());
    let mut editor = spawn_at(&ws_uri).await;
    editor.notify("initialized", json!({})).await;

    // the walk's two url-dep hints trail the initialize response: the
    // MISS (ghost) and the UNPINNED row (bare) — each ONE info message
    // naming the dep and the `rut fetch` alternative, then the wire goes
    // quiet (no spam, no third hint)
    let mut hints = Vec::new();
    for _ in 0..50 {
        match tokio::time::timeout(std::time::Duration::from_millis(100), editor.recv()).await {
            Ok(msg) if msg["method"] == "window/showMessage" => hints.push(msg),
            Ok(_) => continue,
            Err(_) => break, // the walk finished — quiet wire
        }
    }
    assert_eq!(hints.len(), 2, "miss + unpinned, once each: {hints:?}");
    assert!(hints.iter().all(|h| h["params"]["type"] == 3), "info severity: {hints:?}");
    let ghost = hints
        .iter()
        .find(|h| h["params"]["message"].as_str().unwrap().contains("ghost"))
        .expect("the cache-miss hint names the dep");
    assert!(ghost["params"]["message"].as_str().unwrap().contains("rut fetch"), "{ghost}");
    let bare = hints
        .iter()
        .find(|h| h["params"]["message"].as_str().unwrap().contains("bare"))
        .expect("the unpinned hint names the dep");
    assert!(bare["params"]["message"].as_str().unwrap().contains("sha256"), "{bare}");
    let _ = remove_ws(fx.base);
}

#[tokio::test]
async fn completion_parity_sort_text_and_additional_edits_on_the_wire() {
    let fx = write_dep_fixture("parity");
    let ws_uri = format!("file://{}", fx.ws.display());
    let mut editor = spawn_at(&ws_uri).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    // `gadgets` is only visible through the dep walk (it lives OUTSIDE
    // the scan root) — the auto-import tier must offer Widget with the
    // use-insert riding additionalTextEdits, exactly the wasm face's wire
    let src = "fn main() -> nil {\n    let w = Wid;\n}\n";
    let uri = format!("file://{}/parity.rut", fx.ws.display());
    editor
        .notify(
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": uri, "languageId": "rut", "version": 1, "text": src
            }}),
        )
        .await;
    let mut notes = Vec::new();
    let (l, c) = pos_of(src, "Wid", 0);
    let comp = editor
        .request(
            "textDocument/completion",
            json!({"textDocument": {"uri": uri}, "position": {"line": l, "character": c + 3}}),
            &mut notes,
        )
        .await;
    let items = comp["result"].as_array().expect("completion items");
    let imp = items
        .iter()
        .find(|i| i["label"] == "Widget" && i["additionalTextEdits"].is_array())
        .expect("the auto-import offer for the dep's type");
    assert_eq!(imp["detail"], "gadgets::Widget — import");
    assert_eq!(imp["sortText"], "~Widget", "the tier sorts after the locals");
    let edits = imp["additionalTextEdits"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0]["newText"], "use gadgets::Widget;\n");
    // no leading use in this doc — the insert lands at the body top
    assert_eq!(edits[0]["range"]["start"]["line"], 0);
    assert_eq!(edits[0]["range"]["start"]["character"], 0);
    let _ = remove_ws(fx.base);
}
