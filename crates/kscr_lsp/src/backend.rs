//! LSP backend implementation.

use crate::backend_diagnostics_hover as diag_hover;
use crate::backend_goto_completion;
use crate::backend_inlay_hints;
use crate::backend_references_rename;
use crate::backend_semantic_tokens;
use crate::backend_symbols;
use crate::vfs::{Document, Vfs};
use kscr::parser;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

/// The LSP backend state
pub struct Backend {
    client: Client,
    vfs: Arc<RwLock<Vfs>>,
    semantic_tokens_cache: Arc<RwLock<HashMap<Url, SemanticTokens>>>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            vfs: Arc::new(RwLock::new(Vfs::new())),
            semantic_tokens_cache: Arc::new(RwLock::new(HashMap::new())),
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
                inlay_hint_provider: Some(OneOf::Left(true)),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            work_done_progress_options: WorkDoneProgressOptions {
                                work_done_progress: Some(false),
                            },
                            legend: backend_semantic_tokens::semantic_tokens_legend(),
                            range: Some(true),
                            full: Some(SemanticTokensFullOptions::Delta { delta: Some(true) }),
                        },
                    ),
                ),
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

            let mut cache = self.semantic_tokens_cache.write().await;
            cache.remove(&uri);
            drop(cache);

            self.publish_diagnostics(uri).await;
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        // Re-run diagnostics on save
        self.publish_diagnostics(params.text_document.uri).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        let mut vfs = self.vfs.write().await;
        vfs.remove(&uri);
        drop(vfs);

        let mut cache = self.semantic_tokens_cache.write().await;
        cache.remove(&uri);
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

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let vfs = self.vfs.read().await;
        let doc = match vfs.get(&params.text_document.uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let Some(tokens) = backend_semantic_tokens::semantic_tokens_in_doc(doc) else {
            return Ok(None);
        };

        let mut cache = self.semantic_tokens_cache.write().await;
        cache.insert(doc.uri.clone(), tokens.clone());

        let tokens = Some(SemanticTokensResult::Tokens(tokens));
        Ok(tokens)
    }

    async fn semantic_tokens_range(
        &self,
        params: SemanticTokensRangeParams,
    ) -> Result<Option<SemanticTokensRangeResult>> {
        let vfs = self.vfs.read().await;
        let doc = match vfs.get(&params.text_document.uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let tokens = backend_semantic_tokens::semantic_tokens_in_range(doc, params.range)
            .map(SemanticTokensRangeResult::Tokens);
        Ok(tokens)
    }

    async fn semantic_tokens_full_delta(
        &self,
        params: SemanticTokensDeltaParams,
    ) -> Result<Option<SemanticTokensFullDeltaResult>> {
        let vfs = self.vfs.read().await;
        let doc = match vfs.get(&params.text_document.uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let previous_result_id = params.previous_result_id;

        let Some(current_tokens) = backend_semantic_tokens::semantic_tokens_in_doc(doc) else {
            return Ok(None);
        };

        let previous = {
            let cache = self.semantic_tokens_cache.read().await;
            cache
                .get(&doc.uri)
                .filter(|t| t.result_id.as_deref() == Some(previous_result_id.as_str()))
                .cloned()
        };

        let result = if let Some(previous_tokens) = previous.as_ref() {
            backend_semantic_tokens::semantic_tokens_full_delta_from_previous(
                previous_tokens,
                current_tokens.clone(),
            )
        } else {
            SemanticTokensFullDeltaResult::Tokens(current_tokens.clone())
        };

        let mut cache = self.semantic_tokens_cache.write().await;
        cache.insert(doc.uri.clone(), current_tokens);

        Ok(Some(result))
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let vfs = self.vfs.read().await;
        let doc = match vfs.get(&params.text_document.uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        Ok(backend_inlay_hints::inlay_hints_in_doc(doc, params.range))
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
    fn hover_shows_ctor_doc_comment() {
        let src_typed = r#"module Main where
    data Opt a = {-| some ctor doc -} Some a | None

    x = Some 1
"#
        .to_string();

        let tmp_dir = std::env::temp_dir().join("kscr_tests");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let path = tmp_dir.join("hover_ctor_doc.smoke.ks");
        std::fs::write(&path, &src_typed).unwrap();

        let uri = Url::from_file_path(&path).unwrap();
        let doc = Document::new(uri, src_typed, 1);

        // Position on "Some" in "x = Some 1".
        let pos = Position {
            line: 3,
            character: 8,
        };
        let h = hover_in_doc(&doc, pos).unwrap();
        let s = match h.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("unexpected hover contents"),
        };
        assert!(s.contains("Some"));
        assert!(s.contains("some ctor doc"));
    }

    #[test]
    fn hover_classifies_module_import_and_instance_class_names() {
        let uri = Url::parse("file:///test.ks").unwrap();
        let src = r#"module ManualSemigroup where
  import Prelude
  data Pair = Pair Integer Integer
  instance Semigroup Pair where
    (<>) = \x y -> x
"#
        .to_string();
        let doc = Document::new(uri, src, 1);

        let module_hover = hover_in_doc(
            &doc,
            Position {
                line: 0,
                character: 9,
            },
        )
        .unwrap();
        let module_text = match module_hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("unexpected hover contents"),
        };
        assert!(module_text.contains("module ManualSemigroup"));

        let import_hover = hover_in_doc(
            &doc,
            Position {
                line: 1,
                character: 10,
            },
        )
        .unwrap();
        let import_text = match import_hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("unexpected hover contents"),
        };
        assert!(import_text.contains("module Prelude"));

        let class_hover = hover_in_doc(
            &doc,
            Position {
                line: 3,
                character: 12,
            },
        )
        .unwrap();
        let class_text = match class_hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("unexpected hover contents"),
        };
        assert!(class_text.contains("class Semigroup"));
    }

    #[test]
    fn hover_classifies_builtin_type_and_value() {
        let uri = Url::parse("file:///test.ks").unwrap();
        let src = r#"module Main where
  import Prelude
  data Pair = Pair Integer Integer
  main = stdoutWrite "x"
"#
        .to_string();
        let doc = Document::new(uri, src, 1);

        let integer_hover = hover_in_doc(
            &doc,
            Position {
                line: 2,
                character: 20,
            },
        )
        .unwrap();
        let integer_text = match integer_hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("unexpected hover contents"),
        };
        assert!(integer_text.contains("built-in type Integer") || integer_text.contains("Integer"));

        let stdout_hover = hover_in_doc(
            &doc,
            Position {
                line: 3,
                character: 10,
            },
        )
        .unwrap();
        let stdout_text = match stdout_hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("unexpected hover contents"),
        };
        assert!(stdout_text.contains("stdoutWrite :: [Char] -> IO Unit"));
    }

    #[test]
    fn hover_shows_formal_parameter_type() {
        let src = r#"module Main where
  funcB :: Integer -> Integer
  funcB x = x * 2
"#
        .to_string();
        let tmp_dir = std::env::temp_dir().join("kscr_tests");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let path = tmp_dir.join("hover_param_type.smoke.ks");
        std::fs::write(&path, &src).unwrap();
        let uri = Url::from_file_path(&path).unwrap();
        let doc = Document::new(uri, src, 1);

        let hover = hover_in_doc(
            &doc,
            Position {
                line: 2,
                character: 8,
            },
        )
        .unwrap();
        let text = match hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("unexpected hover contents"),
        };
        assert!(text.contains("parameter x :: Integer"), "actual hover: {text}");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn hover_shows_parameter_value_use_type() {
        let src = r#"module Main where
  funcB :: Integer -> Integer
  funcB x = x * 2
"#
        .to_string();
        let tmp_dir = std::env::temp_dir().join("kscr_tests");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let path = tmp_dir.join("hover_param_use.smoke.ks");
        std::fs::write(&path, &src).unwrap();
        let uri = Url::from_file_path(&path).unwrap();
        let doc = Document::new(uri, src, 1);

        let hover = hover_in_doc(
            &doc,
            Position {
                line: 2,
                character: 12,
            },
        )
        .unwrap();
        let text = match hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("unexpected hover contents"),
        };
        assert!(text.contains("parameter x :: Integer"), "actual hover: {text}");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn hover_classifies_module_alias_qualifier() {
        let src = r#"module Main where
  import qualified Prelude as P
  main = P.show 1
"#
        .to_string();
        let tmp_dir = std::env::temp_dir().join("kscr_tests");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let path = tmp_dir.join("hover_method_alias.smoke.ks");
        std::fs::write(&path, &src).unwrap();
        let uri = Url::from_file_path(&path).unwrap();
        let doc = Document::new(uri, src, 1);

        let alias_hover = hover_in_doc(
            &doc,
            Position {
                line: 2,
                character: 9,
            },
        )
        .unwrap();
        let alias_text = match alias_hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("unexpected hover contents"),
        };
        assert!(alias_text.contains("module alias P = Prelude"), "actual hover: {alias_text}");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn hover_classifies_class_method_use() {
        let src = r#"module Main where
  import Prelude
  showInt = show 1
"#
        .to_string();
        let tmp_dir = std::env::temp_dir().join("kscr_tests");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let path = tmp_dir.join("hover_method.smoke.ks");
        std::fs::write(&path, &src).unwrap();
        let uri = Url::from_file_path(&path).unwrap();
        let doc = Document::new(uri, src, 1);

        let method_hover = hover_in_doc(
            &doc,
            Position {
                line: 2,
                character: 12,
            },
        )
        .unwrap();
        let method_text = match method_hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("unexpected hover contents"),
        };
        assert!(method_text.contains("class method show ::"), "actual hover: {method_text}");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn hover_shows_symbolic_operator_use_type() {
        let src = r#"module Main where
  import Prelude
  value = 1 + 2
"#
        .to_string();
        let tmp_dir = std::env::temp_dir().join("kscr_tests");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let path = tmp_dir.join("hover_symbolic_operator.smoke.ks");
        std::fs::write(&path, &src).unwrap();
        let uri = Url::from_file_path(&path).unwrap();
        let doc = Document::new(uri, src, 1);

        let hover = hover_in_doc(
            &doc,
            Position {
                line: 2,
                character: 12,
            },
        )
        .unwrap();
        let text = match hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("unexpected hover contents"),
        };
        assert!(text.contains("class method + ::"), "actual hover: {text}");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn hover_keeps_backtick_infix_identifier_working() {
        let src = r#"module Main where
  add a b = a + b
  value = 1 `add` 2
"#
        .to_string();
        let tmp_dir = std::env::temp_dir().join("kscr_tests");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let path = tmp_dir.join("hover_backtick_infix.smoke.ks");
        std::fs::write(&path, &src).unwrap();
        let uri = Url::from_file_path(&path).unwrap();
        let doc = Document::new(uri, src, 1);

        let hover = hover_in_doc(
            &doc,
            Position {
                line: 2,
                character: 13,
            },
        )
        .unwrap();
        let text = match hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("unexpected hover contents"),
        };
        assert!(text.contains("add :: Integer -> Integer -> Integer"), "actual hover: {text}");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn hover_shows_where_local_parameter() {
        let src = r#"module Main where
  value = foo 1 where
    foo y = y + 1
"#
        .to_string();
        let tmp_dir = std::env::temp_dir().join("kscr_tests");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let path = tmp_dir.join("hover_where_param.smoke.ks");
        std::fs::write(&path, &src).unwrap();
        let uri = Url::from_file_path(&path).unwrap();
        let doc = Document::new(uri, src, 1);

        let binder_hover = hover_in_doc(
            &doc,
            Position {
                line: 2,
                character: 8,
            },
        )
        .unwrap();
        let binder_text = match binder_hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("unexpected hover contents"),
        };
        assert!(binder_text.contains("parameter y"), "actual hover: {binder_text}");

        let use_hover = hover_in_doc(
            &doc,
            Position {
                line: 2,
                character: 12,
            },
        )
        .unwrap();
        let use_text = match use_hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("unexpected hover contents"),
        };
        assert!(use_text.contains("parameter y"), "actual hover: {use_text}");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn hover_shows_where_local_binding_type() {
        let src = r#"module Main where
  value = foo 1 where
    foo y = y + 1
"#
        .to_string();
        let tmp_dir = std::env::temp_dir().join("kscr_tests");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let path = tmp_dir.join("hover_where_binding.smoke.ks");
        std::fs::write(&path, &src).unwrap();
        let uri = Url::from_file_path(&path).unwrap();
        let doc = Document::new(uri, src, 1);

        let binder_hover = hover_in_doc(
            &doc,
            Position {
                line: 2,
                character: 4,
            },
        )
        .unwrap();
        let binder_text = match binder_hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("unexpected hover contents"),
        };
        assert!(binder_text.contains("binding foo :: Integer -> Integer"), "actual hover: {binder_text}");

        let use_hover = hover_in_doc(
            &doc,
            Position {
                line: 1,
                character: 10,
            },
        )
        .unwrap();
        let use_text = match use_hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("unexpected hover contents"),
        };
        assert!(use_text.contains("binding foo :: Integer -> Integer"), "actual hover: {use_text}");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn hover_pretty_prints_type_variables() {
        let src = r#"module Main where
  on f g x y = f (g x) (g y)
"#
        .to_string();
        let tmp_dir = std::env::temp_dir().join("kscr_tests");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let path = tmp_dir.join("hover_pretty_vars.smoke.ks");
        std::fs::write(&path, &src).unwrap();
        let uri = Url::from_file_path(&path).unwrap();
        let doc = Document::new(uri, src, 1);

        let hover = hover_in_doc(
            &doc,
            Position {
                line: 1,
                character: 3,
            },
        )
        .unwrap();
        let text = match hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("unexpected hover contents"),
        };
        assert!(
            !text.as_bytes().windows(2).any(|window| window[0] == b't' && window[1].is_ascii_digit()),
            "actual hover: {text}"
        );
        assert!(text.contains("a") && text.contains("b"), "actual hover: {text}");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn hover_shows_prelude_symbolic_method_declaration() {
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = repo_root.join("stdlib/Prelude/Applicative.ks");
        let src = std::fs::read_to_string(&path).unwrap();
        let uri = Url::from_file_path(&path).unwrap();
        let doc = Document::new(uri, src, 1);

        let hover = hover_in_doc(
            &doc,
            Position {
                line: 9,
                character: 6,
            },
        )
        .unwrap();
        let text = match hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("unexpected hover contents"),
        };
        assert!(text.contains("class method <*> ::"), "actual hover: {text}");
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
