//! the `rut-lsp` binary — the language server over stdio (RFC 0041 §2).
//! Native only: stdio IS the transport, and tokio rejects `io-std` on
//! wasm32 — there the language core ships as the `rut-lsp-wasm` module
//! instead (the VS Code extension's in-process face).

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) =
        tower_lsp_server::LspService::new(|client| rut_lsp::server::Backend::new(client));
    tower_lsp_server::Server::new(stdin, stdout, socket).serve(service).await;
}

// wasm32 stub — the target-gated deps (and the `server` module) resolve
// out per-target, so the workspace check has a compilable `main` here
#[cfg(target_arch = "wasm32")]
fn main() {}
