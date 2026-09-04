//! the `rut-lsp` binary — the language server over stdio (RFC 0041 §2).

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) =
        tower_lsp_server::LspService::new(|client| rut_lsp::server::Backend::new(client));
    tower_lsp_server::Server::new(stdin, stdout, socket).serve(service).await;
}
