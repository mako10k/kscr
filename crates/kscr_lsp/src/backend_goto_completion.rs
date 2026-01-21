use crate::backend_helpers::{find_decl_name_span, qualified_ident_at_offset, span_to_range};
use crate::vfs::Document;
use kscr::{lexer, parser, types};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tower_lsp::lsp_types::*;

fn resolve_import_path(module: &str, base_dir: &Path) -> Option<PathBuf> {
    let rel = module.replace('.', "/");
    let local = base_dir.join(format!("{rel}.ks"));
    let stdlib = types::stdlib_root().join(format!("{rel}.ks"));

    std::fs::canonicalize(&local)
        .or_else(|_| std::fs::canonicalize(&stdlib))
        .ok()
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

pub(super) fn super_classify_toplevel_symbol(
    module: &kscr::ast::Module,
    name: &str,
) -> Option<&'static str> {
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

fn find_toplevel_span_in_doc(
    doc: &Document,
    module: &kscr::ast::Module,
    name: &str,
) -> Option<kscr::lexer::Span> {
    let defs = toplevel_binding_spans(module);
    if let Some(s) = defs.get(name).copied() {
        return Some(s);
    }

    match super_classify_toplevel_symbol(module, name)? {
        "type" => find_decl_name_span(doc, lexer::TokenKind::KwType, name),
        "data" => find_decl_name_span(doc, lexer::TokenKind::KwData, name),
        "class" => find_decl_name_span(doc, lexer::TokenKind::KwClass, name),
        "ctor" => {
            let toks = lexer::lex(&doc.text).ok()?;
            for it in &module.items {
                let kscr::ast::Item::DataDecl(dd) = it else {
                    continue;
                };
                if !dd.ctors.iter().any(|c| c.name == name) {
                    continue;
                }

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
                            if depth == 0 =>
                        {
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

pub(super) fn goto_definition_in_doc(doc: &Document, pos: Position) -> Option<Location> {
    let off = doc.position_to_offset(pos.line, pos.character)?;

    // First: if the cursor is on an `import <Module>` module name, jump to that module.
    {
        let toks = lexer::lex(&doc.text).ok()?;
        let i = toks
            .iter()
            .position(|t| t.span.start <= off && off < t.span.end && t.span.end > t.span.start)
            .or_else(|| {
                toks.iter().position(|t| {
                    t.span.start < off && off <= t.span.end && t.span.end > t.span.start
                })
            })?;

        if toks[i].kind == lexer::TokenKind::KwImport {
            // Allow clicking on the `import` keyword itself.
            if let Some(p) = toks.get(i + 1) {
                if let lexer::TokenKind::Ident(module) = &p.kind {
                    let this_path = doc.uri.to_file_path().ok()?;
                    let base_dir = this_path
                        .parent()
                        .filter(|p| p.exists())
                        .map(|p| p.to_path_buf())
                        .unwrap_or_else(std::env::temp_dir);
                    let target_path = resolve_import_path(module, &base_dir)?;
                    let uri = Url::from_file_path(&target_path).ok()?;
                    return Some(Location {
                        uri,
                        range: Range {
                            start: Position {
                                line: 0,
                                character: 0,
                            },
                            end: Position {
                                line: 0,
                                character: 0,
                            },
                        },
                    });
                }
            }
        }

        // Allow clicking on the module identifier.
        if matches!(toks[i].kind, lexer::TokenKind::Ident(_))
            && i >= 2
            && toks[i - 1].kind == lexer::TokenKind::KwImport
        {
            if let lexer::TokenKind::Ident(module) = &toks[i].kind {
                let this_path = doc.uri.to_file_path().ok()?;
                let base_dir = this_path
                    .parent()
                    .filter(|p| p.exists())
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(std::env::temp_dir);
                let target_path = resolve_import_path(module, &base_dir)?;
                let uri = Url::from_file_path(&target_path).ok()?;
                return Some(Location {
                    uri,
                    range: Range {
                        start: Position {
                            line: 0,
                            character: 0,
                        },
                        end: Position {
                            line: 0,
                            character: 0,
                        },
                    },
                });
            }
        }
    }

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

pub(super) fn completion_items_in_doc(
    doc: &Document,
    pos: Position,
    tm: &types::TypedModule,
) -> Option<Vec<CompletionItem>> {
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

    let mut names: Vec<String> = tm
        .inferred
        .keys()
        .cloned()
        .chain(tm.docs.keys().cloned())
        .filter(|n| n.starts_with(prefix))
        .take(200)
        .collect();
    names.sort();
    names.dedup();

    // Best-effort: classify completion kind from the typechecked module.
    // This keeps working even when the document text is incomplete.
    let local_kind = {
        let m = &tm.module;
        Some(
            names
                .iter()
                .filter_map(|n| {
                    super_classify_toplevel_symbol(m, n).map(|k| {
                        (
                            n.clone(),
                            match k {
                                "binding" => CompletionItemKind::VARIABLE,
                                "type" => CompletionItemKind::TYPE_PARAMETER,
                                "data" => CompletionItemKind::STRUCT,
                                "ctor" => CompletionItemKind::CONSTRUCTOR,
                                "class" => CompletionItemKind::CLASS,
                                _ => CompletionItemKind::TEXT,
                            },
                        )
                    })
                })
                .collect::<HashMap<String, CompletionItemKind>>(),
        )
    };

    Some(
        names
            .into_iter()
            .map(|name| CompletionItem {
                label: name.clone(),
                kind: Some(
                    local_kind
                        .as_ref()
                        .and_then(|m| m.get(&name).copied())
                        .unwrap_or_else(|| {
                            if name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                                CompletionItemKind::CLASS
                            } else {
                                CompletionItemKind::VARIABLE
                            }
                        }),
                ),
                documentation: tm
                    .docs
                    .get(&name)
                    .map(|doc| Documentation::MarkupContent(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: doc.clone(),
                    })),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range,
                    new_text: name,
                })),
                ..Default::default()
            })
            .collect(),
    )
}
