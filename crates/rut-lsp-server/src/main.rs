//! the `rut-lsp` binary — the language server over stdio.
//!
//! The transport IS this crate (`rut-lsp-server`); the queries are
//! `rut-lsp`'s. Editors spawn `rut-lsp`; the VS Code extension and the
//! playground instead bundle `rut-lsp-wasm`, the in-process twin over the
//! same core.

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = tower_lsp_server::LspService::new(rut_lsp_server::Backend::new);
    tower_lsp_server::Server::new(stdin, stdout, socket)
        .serve(service)
        .await;
}
