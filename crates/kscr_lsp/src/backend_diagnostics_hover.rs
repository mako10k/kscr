use crate::backend_helpers::{
    contextual_ident_kind_at_offset, create_diagnostic, qualified_ident_at_offset,
    qualified_ident_parts_at_offset, span_to_range,
};
use crate::vfs::Document;
use kscr::{ast, error::Error as KscrError, lexer, parser, types};
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
    if let Some(module_hover) = hover_module_qualifier_in_doc(doc, off) {
        return Some(module_hover);
    }

    let (name, name_span) = qualified_ident_at_offset(&doc.text, off)?;
    let typed = typecheck_document_typed(&doc.uri, &doc.text).ok();

    if let Some(param_hover) = hover_parameter_in_doc(doc, off, &name, typed.as_ref()) {
        return Some(param_hover);
    }
    if let Some(param_hover) = hover_parameter_use_in_doc(doc, off, typed.as_ref()) {
        return Some(param_hover);
    }

    let module = parser::parse_module(&doc.text).ok();
    let method_name = name.rsplit('.').next().unwrap_or(&name);
    let method_ty = typed
        .as_ref()
        .and_then(|tm| tm.class_methods.get(method_name).cloned());

    let kind = contextual_ident_kind_at_offset(&doc.text, off)
        .or_else(|| {
            module
                .as_ref()
                .and_then(|m| crate::backend_goto_completion::super_classify_toplevel_symbol(m, &name))
        })
        .or_else(|| method_ty.as_ref().map(|_| "class method"))
        .unwrap_or("identifier");

    let ty = typed
        .as_ref()
        .and_then(|tm| tm.inferred.get(&name).cloned())
        .or(method_ty)
        .or_else(|| types::builtin_hover_scheme(&name))
        .map(|s| s.to_string());
    let doc_comment = typed.as_ref().and_then(|tm| tm.docs.get(&name).cloned());
    let builtin_kind = types::builtin_hover_kind(&name);

    let range = span_to_range(doc, name_span);
    let hover_kind = builtin_kind.map(|k| k.hover_label()).unwrap_or(kind);
    let mut value = match ty {
        Some(ty) if hover_kind != "identifier" => {
            format!("```kscr\n{hover_kind} {name} :: {ty}\n```")
        }
        Some(ty) => format!("```kscr\n{name} :: {ty}\n```"),
        None => format!(
            "```kscr\n{} {}\n```",
            hover_kind,
            name
        ),
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

fn hover_module_qualifier_in_doc(doc: &Document, offset: usize) -> Option<Hover> {
    let ident = qualified_ident_parts_at_offset(&doc.text, offset)?;
    let module = parser::parse_module(&doc.text).ok()?;

    if let Some(module_name) = module.name.as_ref() {
        let module_segments: Vec<_> = module_name.split('.').collect();
        if ident.full_name == *module_name && ident.segment_index < module_segments.len() {
            return Some(simple_hover(
                doc,
                ident.current_span,
                &format!("module {module_name}"),
            ));
        }
    }

    for item in &module.items {
        let ast::Item::Import(import) = item else {
            continue;
        };

        let local = import.as_name.as_deref().unwrap_or(&import.module);
        let local_segments: Vec<_> = local.split('.').collect();
        let matches_exact = ident.full_name == local && ident.segment_index < local_segments.len();
        let matches_qualifier = ident.segments.len() > local_segments.len()
            && ident.segment_index < local_segments.len()
            && ident.segments[..local_segments.len()].join(".") == local;

        if matches_exact || matches_qualifier {
            let head = if import.as_name.is_some() {
                format!("module alias {local} = {}", import.module)
            } else {
                format!("module {}", import.module)
            };
            return Some(simple_hover(doc, ident.current_span, &head));
        }
    }

    None
}

fn hover_parameter_in_doc(
    doc: &Document,
    offset: usize,
    name: &str,
    typed: Option<&types::TypedModule>,
) -> Option<Hover> {
    let typed = typed?;
    let module = parser::parse_module(&doc.text).ok()?;
    let tokens = lexer::lex(&doc.text).ok()?;

    for item in &module.items {
        let ast::Item::Binding(binding) = item else {
            continue;
        };
        let ast::PatternKind::Var(binding_name) = &binding.pat.kind else {
            continue;
        };
        let Some(scheme) = typed.inferred.get(binding_name) else {
            continue;
        };

        let mut spans = binding_lhs_parameter_spans(doc, &tokens, binding);
        spans.extend(leading_lambda_parameter_spans(&tokens, binding));
        let arg_types = function_argument_types(&scheme.ty, spans.len());

        for (span, ty) in spans.into_iter().zip(arg_types.into_iter()) {
            if span.start <= offset && offset < span.end {
                let range = span_to_range(doc, span);
                return Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: format!("```kscr\nparameter {name} :: {ty}\n```"),
                    }),
                    range,
                });
            }
        }
    }

    None
}

fn hover_parameter_use_in_doc(
    doc: &Document,
    offset: usize,
    typed: Option<&types::TypedModule>,
) -> Option<Hover> {
    let typed = typed?;
    let module = parser::parse_module(&doc.text).ok()?;
    let tokens = lexer::lex(&doc.text).ok()?;
    let ident = qualified_ident_parts_at_offset(&doc.text, offset)?;
    if ident.segments.len() != 1 {
        return None;
    }

    for item in &module.items {
        let ast::Item::Binding(binding) = item else {
            continue;
        };
        let ast::PatternKind::Var(binding_name) = &binding.pat.kind else {
            continue;
        };
        let Some(scheme) = typed.inferred.get(binding_name) else {
            continue;
        };
        if offset < binding.expr.span.start || binding.expr.span.end <= offset {
            continue;
        }

        let mut spans = binding_lhs_parameter_spans(doc, &tokens, binding);
        spans.extend(leading_lambda_parameter_spans(&tokens, binding));
        let arg_types = function_argument_types(&scheme.ty, spans.len());

        for (span, ty) in spans.into_iter().zip(arg_types.into_iter()) {
            if span.start <= offset && offset < span.end {
                return None;
            }

            let Some(name) = doc.text.get(span.start..span.end) else {
                continue;
            };
            if name == ident.current_name {
                return Some(simple_hover(
                    doc,
                    ident.current_span,
                    &format!("parameter {name} :: {ty}"),
                ));
            }
        }
    }

    None
}

fn simple_hover(doc: &Document, span: lexer::Span, head: &str) -> Hover {
    Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("```kscr\n{head}\n```"),
        }),
        range: span_to_range(doc, span),
    }
}

pub(super) fn function_argument_types(ty: &types::Ty, count: usize) -> Vec<types::Ty> {
    let mut out = Vec::new();
    let mut current = ty;
    while out.len() < count {
        let types::Ty::Func(arg, rest) = current else {
            break;
        };
        out.push((**arg).clone());
        current = rest;
    }
    out
}

pub(super) fn binding_lhs_parameter_spans(
    doc: &Document,
    tokens: &[lexer::Token],
    binding: &kscr::ast::Binding,
) -> Vec<kscr::lexer::Span> {
    let Some((binding_line, _)) = doc.offset_to_position(binding.pat.span.start) else {
        return Vec::new();
    };

    let mut seen_name = false;
    let mut out = Vec::new();
    for token in tokens {
        if token.span.start < binding.pat.span.start {
            continue;
        }
        if token.span.start >= binding.span.end {
            break;
        }

        let Some((line, _)) = doc.offset_to_position(token.span.start) else {
            continue;
        };
        if line != binding_line {
            if seen_name {
                break;
            }
            continue;
        }

        match &token.kind {
            lexer::TokenKind::Ident(_) if !seen_name => seen_name = true,
            lexer::TokenKind::Ident(_) if seen_name => out.push(token.span),
            lexer::TokenKind::Eq if seen_name => break,
            lexer::TokenKind::ColonColon if seen_name => return Vec::new(),
            _ => {}
        }
    }
    out
}

pub(super) fn leading_lambda_parameter_spans(
    tokens: &[lexer::Token],
    binding: &kscr::ast::Binding,
) -> Vec<kscr::lexer::Span> {
    if !matches!(binding.expr.kind, kscr::ast::ExprKind::Lambda { .. }) {
        return Vec::new();
    }

    let mut in_expr = false;
    let mut out = Vec::new();
    for token in tokens {
        if token.span.start < binding.expr.span.start {
            continue;
        }
        if token.span.start >= binding.expr.span.end {
            break;
        }

        if !in_expr {
            if token.kind == lexer::TokenKind::Backslash {
                in_expr = true;
            }
            continue;
        }

        match &token.kind {
            lexer::TokenKind::Ident(_) => out.push(token.span),
            lexer::TokenKind::Arrow => break,
            lexer::TokenKind::Newline | lexer::TokenKind::Indent | lexer::TokenKind::Dedent => {}
            _ => break,
        }
    }
    out
}
