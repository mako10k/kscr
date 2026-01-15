//! LSP backend implementation
//!
//! This module implements the Language Server Protocol for kscr.
//! It handles document synchronization, diagnostics, hover, and go-to-definition.

use crate::vfs::{Document, Vfs};
use kscr::{error::Error as KscrError, lexer, parser, types};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
    typecheck_document_typed(uri, text).map(|_| ())
}

fn typecheck_document_typed(
    uri: &Url,
    text: &str,
) -> std::result::Result<types::TypedModule, KscrError> {
    let path = uri
        .to_file_path()
        .map_err(|_| KscrError::msg("Cannot convert URI to file path"))?;

    if path.exists() {
        return types::typecheck_file(&path);
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

    let res = types::typecheck_file(&tmp_path);
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

fn qualified_ident_at_offset(src: &str, offset: usize) -> Option<(String, kscr::lexer::Span)> {
    use lexer::TokenKind;

    let toks = lexer::lex(src).ok()?;
    let i = toks
        .iter()
        .position(|t| t.span.start <= offset && offset < t.span.end && t.span.end > t.span.start)
        .or_else(|| {
            toks.iter().position(|t| t.span.start < offset && offset <= t.span.end && t.span.end > t.span.start)
        })?;

    let lexer::Token {
        kind: TokenKind::Ident(_),
        ..
    } = &toks[i]
    else {
        return None;
    };

    let mut start = i;
    while start >= 2 {
        if toks[start - 1].kind == TokenKind::Dot {
            if matches!(toks[start - 2].kind, TokenKind::Ident(_)) {
                start -= 2;
                continue;
            }
        }
        break;
    }

    let mut end = i;
    while end + 2 < toks.len() {
        if toks[end + 1].kind == TokenKind::Dot {
            if matches!(toks[end + 2].kind, TokenKind::Ident(_)) {
                end += 2;
                continue;
            }
        }
        break;
    }

    let mut parts = Vec::new();
    let mut span = toks[start].span;
    let mut j = start;
    while j <= end {
        if let TokenKind::Ident(s) = &toks[j].kind {
            parts.push(s.clone());
            span.end = toks[j].span.end;
        }
        j += 2;
    }

    Some((parts.join("."), span))
}

fn resolve_import_path(module: &str, base_dir: &Path) -> Option<PathBuf> {
    let rel = module.replace('.', "/");
    let local = base_dir.join(format!("{rel}.ks"));
    let stdlib = types::stdlib_root().join(format!("{rel}.ks"));

    std::fs::canonicalize(&local)
        .or_else(|_| std::fs::canonicalize(&stdlib))
        .ok()
}

fn find_toplevel_span_in_doc(doc: &Document, module: &kscr::ast::Module, name: &str) -> Option<kscr::lexer::Span> {
    let defs = toplevel_binding_spans(module);
    if let Some(s) = defs.get(name).copied() {
        return Some(s);
    }

    match classify_toplevel_symbol(module, name)? {
        "type" => find_decl_name_span(doc, lexer::TokenKind::KwType, name),
        "data" => find_decl_name_span(doc, lexer::TokenKind::KwData, name),
        "class" => find_decl_name_span(doc, lexer::TokenKind::KwClass, name),
        "ctor" => {
            // Best-effort: search within the matching data decl region for ctor name.
            // This is a lexer-based span finder because ctor spans are not stored in the AST yet.
            let toks = lexer::lex(&doc.text).ok()?;
            for it in &module.items {
                let kscr::ast::Item::DataDecl(dd) = it else { continue };
                if !dd.ctors.iter().any(|c| c.name == name) {
                    continue;
                }

                // Locate `data <dd.name>` and then scan forward until the next top-level decl.
                let mut idx = 0usize;
                while idx + 1 < toks.len() {
                    if toks[idx].kind == lexer::TokenKind::KwData {
                        if let lexer::TokenKind::Ident(n) = &toks[idx + 1].kind {
                            if n == &dd.name {
                                break;
                            }
                        }
                    }
                    idx += 1;
                }
                if idx + 1 >= toks.len() {
                    continue;
                }

                let mut depth = 0usize;
                let mut j = idx + 2;
                while j < toks.len() {
                    match toks[j].kind {
                        lexer::TokenKind::Indent => depth += 1,
                        lexer::TokenKind::Dedent => depth = depth.saturating_sub(1),
                        lexer::TokenKind::KwData
                        | lexer::TokenKind::KwType
                        | lexer::TokenKind::KwClass
                        | lexer::TokenKind::KwInstance
                        | lexer::TokenKind::KwLet
                        | lexer::TokenKind::KwModule
                            if depth == 0 => {
                                break;
                            }
                        _ => {}
                    }

                    if let lexer::TokenKind::Ident(n) = &toks[j].kind {
                        if n == name {
                            return Some(toks[j].span);
                        }
                    }
                    j += 1;
                }
            }
            None
        }
        _ => None,
    }
}

fn goto_definition_cross_file(doc: &Document, name: &str) -> Option<Location> {
    let (qual, member) = name.rsplit_once('.')?;

    let this_path = doc.uri.to_file_path().ok()?;
    let base_dir = this_path
        .parent()
        .filter(|p| p.exists())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(std::env::temp_dir);

    let this_module = parser::parse_module(&doc.text).ok()?;

    // Map local qualifier -> canonical module name (same rule as typechecker).
    let mut qual_to_module: HashMap<String, String> = HashMap::new();
    for it in &this_module.items {
        let kscr::ast::Item::Import(id) = it else {
            continue;
        };
        let local = id.as_name.clone().unwrap_or_else(|| id.module.clone());
        qual_to_module.insert(local, id.module.clone());
    }

    let target_module = qual_to_module.get(qual)?.clone();
    let target_path = resolve_import_path(&target_module, &base_dir)?;

    let text = std::fs::read_to_string(&target_path).ok()?;
    let uri = Url::from_file_path(&target_path).ok()?;
    let target_doc = Document::new(uri.clone(), text, 0);
    let target_module_ast = parser::parse_module(&target_doc.text).ok()?;

    let span = find_toplevel_span_in_doc(&target_doc, &target_module_ast, member)?;
    let range = span_to_range(&target_doc, span)?;

    Some(Location { uri, range })
}

fn goto_definition_unqualified_import(doc: &Document, name: &str) -> Option<Location> {
    let this_path = doc.uri.to_file_path().ok()?;
    let base_dir = this_path
        .parent()
        .filter(|p| p.exists())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(std::env::temp_dir);

    let this_module = parser::parse_module(&doc.text).ok()?;

    for it in &this_module.items {
        let kscr::ast::Item::Import(id) = it else {
            continue;
        };
        if id.qualified {
            continue;
        }
        let target_path = resolve_import_path(&id.module, &base_dir)?;
        let text = std::fs::read_to_string(&target_path).ok()?;
        let uri = Url::from_file_path(&target_path).ok()?;
        let target_doc = Document::new(uri.clone(), text, 0);
        let target_module_ast = parser::parse_module(&target_doc.text).ok()?;

        if let Some(span) = find_toplevel_span_in_doc(&target_doc, &target_module_ast, name) {
            let range = span_to_range(&target_doc, span)?;
            return Some(Location { uri, range });
        }
    }

    None
}

fn toplevel_binding_spans(module: &kscr::ast::Module) -> HashMap<String, kscr::lexer::Span> {
    use kscr::ast::{Item, PatternKind};

    let mut m = HashMap::new();
    for item in &module.items {
        if let Item::Binding(b) = item {
            if let PatternKind::Var(name) = &b.pat.kind {
                m.insert(name.clone(), b.pat.span);
            }
        }
    }
    m
}

fn classify_toplevel_symbol(module: &kscr::ast::Module, name: &str) -> Option<&'static str> {
    use kscr::ast::Item;

    for item in &module.items {
        match item {
            Item::Binding(b) => {
                if matches!(&b.pat.kind, kscr::ast::PatternKind::Var(n) if n == name) {
                    return Some("binding");
                }
            }
            Item::TypeAlias(a) if a.name == name => {
                return Some("type");
            }
            Item::DataDecl(d) => {
                if d.name == name {
                    return Some("data");
                }
                if d.ctors.iter().any(|c| c.name == name) {
                    return Some("ctor");
                }
            }
            Item::ClassDecl(c) if c.name == name => {
                return Some("class");
            }
            _ => {}
        }
    }

    None
}

fn goto_definition_in_doc(doc: &Document, pos: Position) -> Option<Location> {
    let off = doc.position_to_offset(pos.line, pos.character)?;

    let (name, _name_span) = qualified_ident_at_offset(&doc.text, off)?;

    if name.contains('.') {
        return goto_definition_cross_file(doc, &name);
    }

    let module = parser::parse_module(&doc.text).ok()?;
    if let Some(span) = toplevel_binding_spans(&module).get(&name).copied() {
        let range = span_to_range(doc, span)?;
        return Some(Location {
            uri: doc.uri.clone(),
            range,
        });
    }

    goto_definition_unqualified_import(doc, &name)
}

fn completion_items_in_doc(doc: &Document, pos: Position) -> Option<Vec<CompletionItem>> {
    let off = doc.position_to_offset(pos.line, pos.character)?;

    let mut start_off = off;
    while start_off > 0 {
        let b = doc.text.as_bytes()[start_off - 1];
        let ok = b.is_ascii_alphanumeric() || b == b'_' || b == b'.';
        if !ok {
            break;
        }
        start_off -= 1;
    }

    let prefix = doc.text.get(start_off..off).unwrap_or("");

    let (sl, sc) = doc.offset_to_position(start_off)?;
    let (el, ec) = doc.offset_to_position(off)?;
    let range = Range {
        start: Position {
            line: sl,
            character: sc,
        },
        end: Position {
            line: el,
            character: ec,
        },
    };

    let tm = typecheck_document_typed(&doc.uri, &doc.text).ok()?;

    let mut names: Vec<String> = tm
        .inferred
        .keys()
        .filter(|n| n.starts_with(prefix))
        .take(200)
        .cloned()
        .collect();
    names.sort();

    Some(
        names
            .into_iter()
            .map(|name| CompletionItem {
                label: name.clone(),
                kind: Some(if name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                    CompletionItemKind::CLASS
                } else {
                    CompletionItemKind::VARIABLE
                }),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range,
                    new_text: name,
                })),
                ..Default::default()
            })
            .collect(),
    )
}

fn hover_in_doc(doc: &Document, pos: Position) -> Option<Hover> {
    let off = doc.position_to_offset(pos.line, pos.character)?;
    let (name, name_span) = qualified_ident_at_offset(&doc.text, off)?;

    let module = parser::parse_module(&doc.text).ok();
    let kind = module
        .as_ref()
        .and_then(|m| classify_toplevel_symbol(m, &name))
        .unwrap_or("identifier");

    let ty = typecheck_document_typed(&doc.uri, &doc.text)
        .ok()
        .and_then(|tm| tm.inferred.get(&name).map(|s| s.to_string()));

    let range = span_to_range(doc, name_span);
    let value = match ty {
        Some(ty) => format!("```kscr\n{name} :: {ty}\n```"),
        None => format!("```kscr\n{kind} {name}\n```"),
    };

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range,
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

        Ok(hover_in_doc(doc, params.text_document_position_params.position))
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

        let loc = goto_definition_in_doc(doc, params.text_document_position_params.position);
        Ok(loc.map(GotoDefinitionResponse::Scalar))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let vfs = self.vfs.read().await;
        let uri = params.text_document_position.text_document.uri;
        let doc = match vfs.get(&uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let items = completion_items_in_doc(doc, params.text_document_position.position)
            .unwrap_or_default();
        Ok(Some(CompletionResponse::Array(items)))
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

fn find_decl_name_span(doc: &Document, kw: lexer::TokenKind, name: &str) -> Option<kscr::lexer::Span> {
    let toks = lexer::lex(&doc.text).ok()?;
    for w in toks.windows(2) {
        if w[0].kind == kw {
            if let lexer::TokenKind::Ident(n) = &w[1].kind {
                if n == name {
                    return Some(w[1].span);
                }
            }
        }
    }
    None
}

/// Convert an AST item to a document symbol
fn item_to_symbol(item: &kscr::ast::Item, doc: &Document) -> Option<DocumentSymbol> {
    use kscr::ast::Item;

    match item {
        Item::Binding(binding) => {
            // Extract name from pattern
            let name = match &binding.pat.kind {
                kscr::ast::PatternKind::Var(name) => name.clone(),
                _ => return None, // Complex patterns not supported yet
            };
            let range = span_to_range(doc, binding.pat.span).unwrap_or_default();
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
            let range = find_decl_name_span(doc, lexer::TokenKind::KwData, &data.name)
                .and_then(|s| span_to_range(doc, s))
                .unwrap_or_default();
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
            let range = find_decl_name_span(doc, lexer::TokenKind::KwType, &alias.name)
                .and_then(|s| span_to_range(doc, s))
                .unwrap_or_default();
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
            let range = find_decl_name_span(doc, lexer::TokenKind::KwClass, &class.name)
                .and_then(|s| span_to_range(doc, s))
                .unwrap_or_default();
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
