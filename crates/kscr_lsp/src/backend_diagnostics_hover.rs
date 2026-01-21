use crate::backend_helpers::{create_diagnostic, qualified_ident_at_offset, span_to_range};
use crate::vfs::Document;
use kscr::{error::Error as KscrError, lexer, parser, types};
use std::time::{SystemTime, UNIX_EPOCH};
use tower_lsp::lsp_types::*;

pub(super) fn compute_diagnostics(doc: &Document) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    match lexer::lex(&doc.text) {
        Err(e) => {
            diagnostics.push(create_diagnostic(doc, &e, DiagnosticSeverity::ERROR));
            return diagnostics;
        }
        Ok(_tokens) => match parser::parse_module(&doc.text) {
            Err(e) => {
                diagnostics.push(create_diagnostic(doc, &e, DiagnosticSeverity::ERROR));
                return diagnostics;
            }
            Ok(_module) => {
                if let Err(e) = typecheck_document_text(&doc.uri, &doc.text) {
                    diagnostics.push(create_diagnostic(doc, &e, DiagnosticSeverity::ERROR));
                }
            }
        },
    }

    diagnostics
}

pub(super) fn typecheck_document_text(uri: &Url, text: &str) -> std::result::Result<(), KscrError> {
    typecheck_document_typed(uri, text).map(|_| ())
}

pub(super) fn typecheck_document_typed(
    uri: &Url,
    text: &str,
) -> std::result::Result<types::TypedModule, KscrError> {
    let path = uri
        .to_file_path()
        .map_err(|_| KscrError::msg("Cannot convert URI to file path"))?;

    if path.exists() {
        return types::typecheck_file(&path);
    }

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

pub(super) fn hover_in_doc(doc: &Document, pos: Position) -> Option<Hover> {
    let off = doc.position_to_offset(pos.line, pos.character)?;
    let (name, name_span) = qualified_ident_at_offset(&doc.text, off)?;

    let module = parser::parse_module(&doc.text).ok();
    let kind = module
        .as_ref()
        .and_then(|m| crate::backend_goto_completion::super_classify_toplevel_symbol(m, &name))
        .unwrap_or("identifier");

    let typed = typecheck_document_typed(&doc.uri, &doc.text).ok();
    let ty = typed
        .as_ref()
        .and_then(|tm| tm.inferred.get(&name).map(|s| s.to_string()));
    let doc_comment = typed.as_ref().and_then(|tm| tm.docs.get(&name).cloned());

    let range = span_to_range(doc, name_span);
    let mut value = match ty {
        Some(ty) => format!("```kscr\n{name} :: {ty}\n```"),
        None => format!("```kscr\n{kind} {name}\n```"),
    };
    if let Some(doc_comment) = doc_comment {
        value.push_str("\n---\n");
        value.push_str(&doc_comment);
    }

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range,
    })
}
