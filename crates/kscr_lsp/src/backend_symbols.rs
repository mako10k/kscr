use crate::backend_helpers::span_to_range;
use crate::vfs::Document;
use kscr::lexer;
use tower_lsp::lsp_types::*;

fn find_decl_name_span(
    doc: &Document,
    kw: lexer::TokenKind,
    name: &str,
) -> Option<kscr::lexer::Span> {
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

pub(super) fn item_to_symbol(
    item: &kscr::ast::Item,
    doc: &Document,
) -> Option<DocumentSymbol> {
    use kscr::ast::Item;

    match item {
        Item::Binding(binding) => {
            let name = match &binding.pat.kind {
                kscr::ast::PatternKind::Var(name) => name.clone(),
                _ => return None,
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
        Item::InstanceDecl(_) => None,
        Item::Fixity(_) | Item::Import(_) | Item::Export(_) => None,
    }
}
