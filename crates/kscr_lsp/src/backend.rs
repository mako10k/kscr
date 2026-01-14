//! LSP backend implementation
//!
//! This module implements the Language Server Protocol for kscr.
//! It handles document synchronization, diagnostics, hover, and go-to-definition.

use crate::vfs::{Document, Vfs};
use kscr::{error::Error as KscrError, lexer, parser, types};
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

/// The LSP backend state
pub struct Backend {
    client: Client,
    vfs: Arc<RwLock<Vfs>>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            vfs: Arc::new(RwLock::new(Vfs::new())),
        }
    }

    /// Publish diagnostics for a document
    async fn publish_diagnostics(&self, uri: Url) {
        let vfs = self.vfs.read().await;
        let doc = match vfs.get(&uri) {
            Some(doc) => doc,
            None => return,
        };

        let diagnostics = self.compute_diagnostics(doc).await;
        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }

    /// Compute diagnostics for a document
    async fn compute_diagnostics(&self, doc: &Document) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        // First, try lexing
        match lexer::lex(&doc.text) {
            Err(e) => {
                // Lexer error
                diagnostics.push(create_diagnostic(
                    &doc.text,
                    &e.to_string(),
                    DiagnosticSeverity::ERROR,
                ));
                return diagnostics;
            }
            Ok(_tokens) => {
                // Lexing succeeded, try parsing
                match parser::parse_module(&doc.text) {
                    Err(e) => {
                        // Parse error
                        diagnostics.push(create_diagnostic(
                            &doc.text,
                            &e.to_string(),
                            DiagnosticSeverity::ERROR,
                        ));
                        return diagnostics;
                    }
                    Ok(_module) => {
                        // Parsing succeeded, try typechecking
                        // For typechecking, we need to write to a temp file
                        // because typecheck_file reads from disk
                        if let Err(e) = self.typecheck_document(doc).await {
                            diagnostics.push(create_diagnostic(
                                &doc.text,
                                &e.to_string(),
                                DiagnosticSeverity::ERROR,
                            ));
                        }
                    }
                }
            }
        }

        diagnostics
    }

    /// Typecheck a document
    async fn typecheck_document(&self, doc: &Document) -> std::result::Result<(), KscrError> {
        // For now, we use the file path if it exists
        // In the future, we should handle VFS-only documents
        let path = doc.uri.to_file_path().map_err(|_| {
            KscrError::msg("Cannot convert URI to file path")
        })?;

        if !path.exists() {
            // Document is not saved yet, skip typechecking for now
            // TODO: Write to temp file and typecheck
            return Ok(());
        }

        types::typecheck_file(&path)?;
        Ok(())
    }
}

/// Create a diagnostic from an error message
fn create_diagnostic(_source: &str, message: &str, severity: DiagnosticSeverity) -> Diagnostic {
    // Try to extract position information from error message
    // For now, we'll just report at the start of the file
    // TODO: Parse error messages to extract actual positions
    
    let range = Range {
        start: Position {
            line: 0,
            character: 0,
        },
        end: Position {
            line: 0,
            character: 0,
        },
    };

    Diagnostic {
        range,
        severity: Some(severity),
        code: None,
        code_description: None,
        source: Some("kscr".to_string()),
        message: message.to_string(),
        related_information: None,
        tags: None,
        data: None,
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _params: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "kscr-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::FULL),
                        will_save: None,
                        will_save_wait_until: None,
                        save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                            include_text: Some(false),
                        })),
                    },
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "kscr-lsp initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let text = params.text_document.text;
        let version = params.text_document.version;

        let mut vfs = self.vfs.write().await;
        vfs.insert(uri.clone(), text, version);
        drop(vfs);

        self.publish_diagnostics(uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let version = params.text_document.version;

        if let Some(change) = params.content_changes.into_iter().next() {
            let mut vfs = self.vfs.write().await;
            vfs.update(&uri, change.text, version);
            drop(vfs);

            self.publish_diagnostics(uri).await;
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        // Re-run diagnostics on save
        self.publish_diagnostics(params.text_document.uri).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let mut vfs = self.vfs.write().await;
        vfs.remove(&params.text_document.uri);
    }

    async fn hover(&self, _params: HoverParams) -> Result<Option<Hover>> {
        // TODO: Implement hover to show type information
        // This requires integrating with the type checker to get inferred types
        Ok(None)
    }

    async fn goto_definition(
        &self,
        _params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        // TODO: Implement go-to-definition
        // This requires building a symbol table and tracking definitions
        Ok(None)
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let vfs = self.vfs.read().await;
        let doc = match vfs.get(&params.text_document.uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        // Parse the module and extract symbols
        let module = match parser::parse_module(&doc.text) {
            Ok(m) => m,
            Err(_) => return Ok(None),
        };

        let mut symbols = Vec::new();

        // Extract top-level items as symbols
        for item in &module.items {
            if let Some(symbol) = item_to_symbol(item, doc) {
                symbols.push(symbol);
            }
        }

        if symbols.is_empty() {
            Ok(None)
        } else {
            Ok(Some(DocumentSymbolResponse::Nested(symbols)))
        }
    }
}

/// Convert an AST item to a document symbol
fn item_to_symbol(item: &kscr::ast::Item, _doc: &Document) -> Option<DocumentSymbol> {
    use kscr::ast::Item;

    match item {
        Item::Binding(binding) => {
            // Extract name from pattern
            let name = match &binding.pat.kind {
                kscr::ast::PatternKind::Var(name) => name.clone(),
                _ => return None, // Complex patterns not supported yet
            };
            let range = Range::default(); // TODO: Extract actual range from span
            #[allow(deprecated)]
            Some(DocumentSymbol {
                name,
                detail: None,
                kind: SymbolKind::FUNCTION,
                tags: None,
                deprecated: None,
                range,
                selection_range: range,
                children: None,
            })
        }
        Item::DataDecl(data) => {
            let range = Range::default();
            #[allow(deprecated)]
            Some(DocumentSymbol {
                name: data.name.clone(),
                detail: None,
                kind: SymbolKind::STRUCT,
                tags: None,
                deprecated: None,
                range,
                selection_range: range,
                children: None,
            })
        }
        Item::TypeAlias(alias) => {
            let range = Range::default();
            #[allow(deprecated)]
            Some(DocumentSymbol {
                name: alias.name.clone(),
                detail: None,
                kind: SymbolKind::CLASS,
                tags: None,
                deprecated: None,
                range,
                selection_range: range,
                children: None,
            })
        }
        Item::ClassDecl(class) => {
            let range = Range::default();
            #[allow(deprecated)]
            Some(DocumentSymbol {
                name: class.name.clone(),
                detail: None,
                kind: SymbolKind::INTERFACE,
                tags: None,
                deprecated: None,
                range,
                selection_range: range,
                children: None,
            })
        }
        Item::InstanceDecl(_) => {
            // Instances don't have a simple name, skip for now
            None
        }
        Item::Fixity(_) | Item::Import(_) | Item::Export(_) => {
            // These are not symbols we want to show
            None
        }
    }
}
