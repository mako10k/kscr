//! LSP backend implementation
//!
//! This module implements the Language Server Protocol for kscr.
//! It handles document synchronization, diagnostics, hover, and go-to-definition.

use crate::vfs::{Document, Vfs};
use kscr::{error::Error as KscrError, lexer, parser, types};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
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
                    doc,
                    &e,
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
                            doc,
                            &e,
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
                                doc,
                                &e,
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
        typecheck_document_text(&doc.uri, &doc.text)
    }
}

fn typecheck_document_text(uri: &Url, text: &str) -> std::result::Result<(), KscrError> {
    let path = uri
        .to_file_path()
        .map_err(|_| KscrError::msg("Cannot convert URI to file path"))?;

    if path.exists() {
        types::typecheck_file(&path)?;
        return Ok(());
    }

    // Unsaved file: write to a temp file in the same directory so that
    // import resolution (relative to importer dir) behaves as expected.
    let parent = path
        .parent()
        .filter(|p| p.exists())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(std::env::temp_dir);

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unsaved");
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let tmp_path = parent.join(format!(".kscr-lsp-{stem}-{pid}-{nanos}.ks"));
    std::fs::write(&tmp_path, text)?;

    let res = types::typecheck_file(&tmp_path).map(|_| ());
    let _ = std::fs::remove_file(&tmp_path);
    res
}

fn span_to_range(doc: &Document, span: kscr::lexer::Span) -> Option<Range> {
    let len = doc.text.len();
    let start_off = span.start.min(len);
    let mut end_off = span.end.min(len);

    if end_off < start_off {
        end_off = start_off;
    }
    if end_off == start_off && end_off < len {
        end_off += 1;
    }

    let (sl, sc) = doc.offset_to_position(start_off)?;
    let (el, ec) = doc.offset_to_position(end_off)?;

    Some(Range {
        start: Position {
            line: sl,
            character: sc,
        },
        end: Position {
            line: el,
            character: ec,
        },
    })
}

/// Create a diagnostic from an error
fn create_diagnostic(doc: &Document, err: &KscrError, severity: DiagnosticSeverity) -> Diagnostic {
    let range = err.span().and_then(|s| span_to_range(doc, s)).unwrap_or(Range {
        start: Position {
            line: 0,
            character: 0,
        },
        end: Position {
            line: 0,
            character: 0,
        },
    });

    Diagnostic {
        range,
        severity: Some(severity),
        code: None,
        code_description: None,
        source: Some("kscr".to_string()),
        message: err.to_string(),
        related_information: None,
        tags: None,
        data: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_to_range_basic() {
        let uri = Url::parse("file:///test.ks").unwrap();
        let doc = Document::new(uri, "abc\ndef".to_string(), 1);
        let r = span_to_range(&doc, kscr::lexer::Span { start: 4, end: 6 }).unwrap();
        assert_eq!(r.start.line, 1);
        assert_eq!(r.start.character, 0);
        assert_eq!(r.end.line, 1);
        assert_eq!(r.end.character, 2);
    }

    #[test]
    fn typecheck_unsaved_document_uses_tempfile() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("kscr-lsp-unsaved-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();

        let path = dir.join("Main.ks");
        assert!(!path.exists());

        let uri = Url::from_file_path(&path).unwrap();
        let src = "module Main where\n  main = IO ()\n";
        typecheck_document_text(&uri, src).unwrap();

        let _ = std::fs::remove_dir_all(&dir);
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
