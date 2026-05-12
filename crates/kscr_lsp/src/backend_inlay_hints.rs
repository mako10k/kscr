use crate::backend_diagnostics_hover;
use crate::vfs::Document;
use kscr::ast::{Binding, ExprKind, Item, PatternKind};
use kscr::lexer;
use kscr::parser;
use kscr::types::{Scheme, Ty};
use tower_lsp::lsp_types::{InlayHint, InlayHintKind, InlayHintLabel, Position, Range};

pub(crate) fn inlay_hints_in_doc(doc: &Document, range: Range) -> Option<Vec<InlayHint>> {
    let module = parser::parse_module(&doc.text).ok()?;
    let typed = backend_diagnostics_hover::typecheck_document_typed(&doc.uri, &doc.text).ok()?;
    let tokens = lexer::lex(&doc.text).ok()?;

    let mut hints = Vec::new();
    for item in &module.items {
        let Item::Binding(binding) = item else {
            continue;
        };
        let PatternKind::Var(name) = &binding.pat.kind else {
            continue;
        };
        let Some(scheme) = typed.inferred.get(name) else {
            continue;
        };

        push_binding_type_hint(doc, binding, scheme, &range, &mut hints);

        let mut parameter_spans = binding_lhs_parameter_spans(doc, &tokens, binding);
        parameter_spans.extend(leading_lambda_parameter_spans(&tokens, binding));
        if parameter_spans.is_empty() {
            continue;
        }

        let parameter_count = parameter_spans.len();
        for (span, ty) in parameter_spans
            .into_iter()
            .zip(function_argument_types(&scheme.ty, parameter_count))
        {
            push_type_hint(doc, span.end, ty.to_string(), &range, &mut hints);
        }
    }

    Some(hints)
}

fn push_binding_type_hint(
    doc: &Document,
    binding: &Binding,
    scheme: &Scheme,
    range: &Range,
    out: &mut Vec<InlayHint>,
) {
    let label = if scheme.constraints.is_empty() {
        format!(":: {}", scheme.ty)
    } else {
        format!(":: {scheme}")
    };
    push_type_hint(doc, binding.pat.span.end, label, range, out);
}

fn push_type_hint(
    doc: &Document,
    offset: usize,
    label: String,
    range: &Range,
    out: &mut Vec<InlayHint>,
) {
    let Some((line, character)) = doc.offset_to_position(offset) else {
        return;
    };
    let pos = Position { line, character };
    if !position_in_range(pos, range) {
        return;
    }

    out.push(InlayHint {
        position: pos,
        label: InlayHintLabel::String(label),
        kind: Some(InlayHintKind::TYPE),
        text_edits: None,
        tooltip: None,
        padding_left: Some(true),
        padding_right: None,
        data: None,
    });
}

fn function_argument_types(ty: &Ty, count: usize) -> Vec<Ty> {
    let mut out = Vec::new();
    let mut current = ty;
    while out.len() < count {
        let Ty::Func(arg, rest) = current else {
            break;
        };
        out.push((**arg).clone());
        current = rest;
    }
    out
}

fn binding_lhs_parameter_spans(
    doc: &Document,
    tokens: &[lexer::Token],
    binding: &Binding,
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

fn leading_lambda_parameter_spans(
    tokens: &[lexer::Token],
    binding: &Binding,
) -> Vec<kscr::lexer::Span> {
    if !matches!(binding.expr.kind, ExprKind::Lambda { .. }) {
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

fn position_in_range(pos: Position, range: &Range) -> bool {
    if pos.line < range.start.line || pos.line > range.end.line {
        return false;
    }
    if pos.line == range.start.line && pos.character < range.start.character {
        return false;
    }
    if pos.line == range.end.line && pos.character > range.end.character {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tower_lsp::lsp_types::Url;

    #[test]
    fn inlay_hints_include_binding_and_parameter_types() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("kscr-lsp-inlay-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();

        let path = dir.join("Main.ks");
        let src = "module Main where\n  inc x = x + 1\n  answer = 42\n";
        std::fs::write(&path, src).unwrap();

        let uri = Url::from_file_path(&path).unwrap();
        let doc = Document::new(uri, src.to_string(), 1);
        let hints = inlay_hints_in_doc(
            &doc,
            Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 10,
                    character: 0,
                },
            },
        )
        .unwrap();

        let labels: Vec<String> = hints
            .iter()
            .map(|hint| match &hint.label {
                InlayHintLabel::String(s) => s.clone(),
                InlayHintLabel::LabelParts(parts) => parts.iter().map(|part| part.value.clone()).collect(),
            })
            .collect();

        assert!(labels.iter().any(|label| label.contains(":: Integer -> Integer")));
        assert!(labels.iter().any(|label| label.contains(":: Integer")));

        let _ = std::fs::remove_dir_all(&dir);
    }
}