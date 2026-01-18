//! LSP backend implementation.

use crate::backend_diagnostics_hover as diag_hover;
use crate::backend_goto_completion;
use crate::backend_references_rename;
use crate::backend_symbols;
use crate::vfs::{Document, Vfs};
use kscr::parser;
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
        diag_hover::compute_diagnostics(doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_diagnostics_hover::{hover_in_doc, typecheck_document_text};
    use crate::backend_goto_completion::goto_definition_in_doc;
    use crate::backend_helpers::span_to_range;
    use crate::backend_symbols::item_to_symbol;
    use std::time::{SystemTime, UNIX_EPOCH};

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

    #[test]
    fn goto_definition_toplevel_binding() {
        let uri = Url::parse("file:///test.ks").unwrap();
        let src = "module Main where\n  foo = 1\n  bar = foo\n".to_string();
        let doc = Document::new(uri.clone(), src, 1);

        // position on the reference "foo" in "bar = foo"
        let pos = Position {
            line: 2,
            character: 8,
        };
        let loc = goto_definition_in_doc(&doc, pos).unwrap();
        assert_eq!(loc.uri, uri);
        assert_eq!(loc.range.start.line, 1);
        assert_eq!(loc.range.start.character, 2);
        assert_eq!(loc.range.end.line, 1);
        assert_eq!(loc.range.end.character, 5);
    }

    #[test]
    fn goto_definition_cross_file_qualified_import() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("kscr-lsp-goto-cross-{nanos}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("A.ks");
        std::fs::write(&a, "module A where\n  x = 1\n").unwrap();

        let main = dir.join("Main.ks");
        let main_src = "module Main where\n  import A\n  y = A.x\n";
        std::fs::write(&main, main_src).unwrap();

        let uri = Url::from_file_path(&main).unwrap();
        let doc = Document::new(uri, main_src.to_string(), 1);

        // position on the reference "x" in "A.x"
        let pos = Position {
            line: 2,
            character: 8,
        };
        let loc = goto_definition_in_doc(&doc, pos).unwrap();

        let a_uri = Url::from_file_path(&a).unwrap();
        assert_eq!(loc.uri, a_uri);
        assert_eq!(loc.range.start.line, 1);
        assert_eq!(loc.range.start.character, 2);
        assert_eq!(loc.range.end.line, 1);
        assert_eq!(loc.range.end.character, 3);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hover_on_identifier() {
        let uri = Url::parse("file:///test.ks").unwrap();
        let src = "module Main where\n  foo = 1\n".to_string();
        let doc = Document::new(uri, src, 1);

        let pos = Position {
            line: 1,
            character: 3,
        };
        let h = hover_in_doc(&doc, pos).unwrap();
        let s = match h.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("unexpected hover contents"),
        };
        assert!(s.contains("foo"));
    }

    #[test]
    fn document_symbols_have_reasonable_ranges() {
        let uri = Url::parse("file:///test.ks").unwrap();
        let src = "module M where\n  x = 1\n  data Foo = Bar\n".to_string();
        let doc = Document::new(uri, src, 1);

        let module = kscr::parser::parse_module(&doc.text).unwrap();
        let mut symbols = Vec::new();
        for item in &module.items {
            if let Some(s) = item_to_symbol(item, &doc) {
                symbols.push(s);
            }
        }

        let x = symbols.iter().find(|s| s.name == "x").unwrap();
        assert_eq!(x.range.start.line, 1);
        assert_eq!(x.range.start.character, 2);
        assert_eq!(x.range.end.line, 1);
        assert_eq!(x.range.end.character, 3);

        let foo = symbols.iter().find(|s| s.name == "Foo").unwrap();
        assert_eq!(foo.range.start.line, 2);
        assert_eq!(foo.range.start.character, 7);
        assert_eq!(foo.range.end.line, 2);
        assert_eq!(foo.range.end.character, 10);
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
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(false),
                    trigger_characters: Some(vec![".".to_string()]),
                    ..Default::default()
                }),
                references_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Left(true)),
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

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let vfs = self.vfs.read().await;
        let uri = params.text_document_position_params.text_document.uri;
        let doc = match vfs.get(&uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        Ok(diag_hover::hover_in_doc(
            doc,
            params.text_document_position_params.position,
        ))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let vfs = self.vfs.read().await;
        let uri = params.text_document_position_params.text_document.uri;
        let doc = match vfs.get(&uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let loc = backend_goto_completion::goto_definition_in_doc(
            doc,
            params.text_document_position_params.position,
        );
        Ok(loc.map(GotoDefinitionResponse::Scalar))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let vfs = self.vfs.read().await;
        let uri = params.text_document_position.text_document.uri;
        let doc = match vfs.get(&uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let tm = match diag_hover::typecheck_document_typed(&doc.uri, &doc.text) {
            Ok(tm) => tm,
            Err(_) => return Ok(Some(CompletionResponse::Array(Vec::new()))),
        };

        let items = backend_goto_completion::completion_items_in_doc(
            doc,
            params.text_document_position.position,
            &tm,
        )
        .unwrap_or_default();
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let vfs = self.vfs.read().await;
        let uri = params.text_document_position.text_document.uri;
        let doc = match vfs.get(&uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let locs = backend_references_rename::references_in_vfs(
            &vfs,
            doc,
            params.text_document_position.position,
        );
        Ok(Some(locs))
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let vfs = self.vfs.read().await;
        let uri = params.text_document_position.text_document.uri;
        let doc = match vfs.get(&uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        Ok(backend_references_rename::rename_in_vfs(
            &vfs,
            doc,
            params.text_document_position.position,
            &params.new_name,
        ))
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
            if let Some(symbol) = backend_symbols::item_to_symbol(item, doc) {
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
