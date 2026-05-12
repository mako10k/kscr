mod backend;
mod backend_diagnostics_hover;
pub(crate) mod backend_goto_completion;
mod backend_helpers;
mod backend_inlay_hints;
mod backend_references_rename;
mod backend_semantic_tokens;
mod backend_symbols;
pub(crate) mod vfs;

use tower_lsp::{LspService, Server};

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(backend::Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
