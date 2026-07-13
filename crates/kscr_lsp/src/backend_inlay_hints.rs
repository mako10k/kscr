use crate::backend_diagnostics_hover;
use crate::vfs::Document;
use kscr::ast::{Binding, ExprKind, Item, PatternKind};
use kscr::lexer;
use kscr::parser;
use kscr::types::{self, Scheme, Ty};
use tower_lsp::lsp_types::{InlayHint, InlayHintKind, InlayHintLabel, Position, Range};

pub(crate) fn inlay_hints_in_doc(doc: &Document, range: Range) -> Option<Vec<InlayHint>> {
    let module = parser::parse_module(&doc.text).ok()?;
    let typed = backend_diagnostics_hover::typecheck_document_typed(&doc.uri, &doc.text).ok();
    let tokens = lexer::lex(&doc.text).ok()?;

    let mut hints = Vec::new();
    for binding in bindings_in_module(&module) {
        let PatternKind::Var(name) = &binding.pat.kind else {
            continue;
        };
        let scheme = typed.as_ref().and_then(|typed| {
            typed
                .inferred
                .get(name)
                .or_else(|| typed.class_methods.get(name))
        });
        let class_sig = class_method_signature_text(&module, name);
        if scheme.is_none() && class_sig.is_none() {
            continue;
        }

        let Some(name_span) = binding_name_span(doc, &tokens, binding) else {
            continue;
        };

        push_binding_type_hint(
            doc,
            binding_type_hint_offset(doc, &tokens, binding, name_span),
            scheme,
            class_sig.as_ref().map(|(label, _)| label.as_str()),
            &range,
            &mut hints,
        );

        let mut parameter_spans = binding_lhs_parameter_spans(doc, &tokens, binding, name_span);
        parameter_spans.extend(leading_lambda_parameter_spans(&tokens, binding));
        if parameter_spans.is_empty() {
            continue;
        }

        let parameter_labels = if let Some(scheme) = scheme {
            let parameter_count = parameter_spans.len();
            let parameter_types = function_argument_types(&scheme.ty, parameter_count);
            types::format_pretty_tys_with_scheme(scheme, &parameter_types)
        } else {
            class_sig
                .as_ref()
                .map(|(_, params)| params.clone())
                .unwrap_or_default()
        };
        for (span, label) in parameter_spans.into_iter().zip(parameter_labels) {
            push_type_hint(doc, span.end, format!(":: {label}"), &range, &mut hints);
        }
    }

    Some(hints)
}

fn bindings_in_module(module: &kscr::ast::Module) -> Vec<&Binding> {
    let mut out = Vec::new();
    for item in &module.items {
        match item {
            Item::Binding(binding) => out.push(binding),
            Item::ClassDecl(class) => out.extend(class.default_methods.iter()),
            Item::InstanceDecl(inst) => out.extend(inst.methods.iter()),
            Item::Import(_)
            | Item::Export(_)
            | Item::Fixity(_)
            | Item::TypeAlias(_)
            | Item::DataDecl(_) => {}
        }
    }
    out
}

fn push_binding_type_hint(
    doc: &Document,
    offset: usize,
    scheme: Option<&Scheme>,
    fallback_label: Option<&str>,
    range: &Range,
    out: &mut Vec<InlayHint>,
) {
    let label = if let Some(scheme) = scheme {
        if scheme.constraints.is_empty() {
            format!(":: {}", types::format_pretty_ty(&scheme.ty))
        } else {
            format!(":: {}", types::format_pretty_scheme(scheme))
        }
    } else {
        format!(":: {}", fallback_label.unwrap_or("_"))
    };
    push_type_hint(doc, offset, label, range, out);
}

fn binding_type_hint_offset(
    doc: &Document,
    tokens: &[lexer::Token],
    binding: &Binding,
    name_span: kscr::lexer::Span,
) -> usize {
    let Some((binding_line, _)) = doc.offset_to_position(binding.pat.span.start) else {
        return name_span.end;
    };

    let mut saw_name = false;
    for token in tokens {
        if token.span.start < binding.pat.span.start {
            continue;
        }

        let Some((line, _)) = doc.offset_to_position(token.span.start) else {
            continue;
        };
        if line != binding_line {
            break;
        }

        if token.span == name_span {
            saw_name = true;
            continue;
        }

        if saw_name && token.kind == lexer::TokenKind::RParen {
            return token.span.end;
        }

        if token.kind == lexer::TokenKind::Eq || token.kind == lexer::TokenKind::ColonColon {
            break;
        }
    }

    name_span.end
}

fn class_method_signature_text(
    module: &kscr::ast::Module,
    binding_name: &str,
) -> Option<(String, Vec<String>)> {
    module.items.iter().find_map(|item| {
        let Item::ClassDecl(class) = item else {
            return None;
        };
        class
            .methods
            .iter()
            .find(|method| method.name == binding_name)
            .map(|method| {
                let params = ast_function_argument_types(&method.ty.ty)
                    .into_iter()
                    .map(format_ast_type)
                    .collect::<Vec<_>>();
                (
                    format!(
                        "{} {} => {}",
                        class.name,
                        class.param,
                        format_ast_type(&method.ty.ty)
                    ),
                    params,
                )
            })
    })
}

fn ast_function_argument_types<'a>(ty: &'a kscr::ast::Type) -> Vec<&'a kscr::ast::Type> {
    let mut out = Vec::new();
    let mut current = ty;
    while let kscr::ast::Type::Func(arg, rest) = current {
        out.push(arg.as_ref());
        current = rest;
    }
    out
}

fn format_ast_type(ty: &kscr::ast::Type) -> String {
    match ty {
        kscr::ast::Type::Unit => "()".to_string(),
        kscr::ast::Type::Integer => "Integer".to_string(),
        kscr::ast::Type::Bool => "Bool".to_string(),
        kscr::ast::Type::Float64 => "Float64".to_string(),
        kscr::ast::Type::Char => "Char".to_string(),
        kscr::ast::Type::String => "String".to_string(),
        kscr::ast::Type::List(item) => format!("[{}]", format_ast_type(item)),
        kscr::ast::Type::Tuple(items) => {
            format!(
                "({})",
                items
                    .iter()
                    .map(format_ast_type)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        kscr::ast::Type::Record(fields) => format!(
            "{{{}}}",
            fields
                .iter()
                .map(|(name, ty)| format!("{name}: {}", format_ast_type(ty)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        kscr::ast::Type::RecordOpen(fields, rest) => format!(
            "{{{}, ..{}}}",
            fields
                .iter()
                .map(|(name, ty)| format!("{name}: {}", format_ast_type(ty)))
                .collect::<Vec<_>>()
                .join(", "),
            format_ast_type(rest)
        ),
        kscr::ast::Type::Hole(name) => name.clone().unwrap_or_else(|| "_".to_string()),
        kscr::ast::Type::Var(name) => name.clone(),
        kscr::ast::Type::App { head, args } => {
            let mut parts = vec![format_ast_type(head)];
            parts.extend(args.iter().map(format_ast_type));
            parts.join(" ")
        }
        kscr::ast::Type::Func(arg, rest) => {
            format!("{} -> {}", format_ast_type(arg), format_ast_type(rest))
        }
    }
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

fn is_operator_name_token(kind: &lexer::TokenKind) -> bool {
    matches!(
        kind,
        lexer::TokenKind::Operator(_)
            | lexer::TokenKind::Colon
            | lexer::TokenKind::Plus
            | lexer::TokenKind::Minus
            | lexer::TokenKind::Star
            | lexer::TokenKind::Slash
            | lexer::TokenKind::PlusPlus
            | lexer::TokenKind::EqEq
            | lexer::TokenKind::SlashEq
            | lexer::TokenKind::Lt
            | lexer::TokenKind::Le
            | lexer::TokenKind::Gt
            | lexer::TokenKind::Ge
            | lexer::TokenKind::GtGt
            | lexer::TokenKind::GtGtEq
            | lexer::TokenKind::AndAnd
            | lexer::TokenKind::OrOr
    )
}

fn binding_lhs_parameter_spans(
    doc: &Document,
    tokens: &[lexer::Token],
    binding: &Binding,
    name_span: kscr::lexer::Span,
) -> Vec<kscr::lexer::Span> {
    let Some((binding_line, _)) = doc.offset_to_position(binding.pat.span.start) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for token in tokens {
        if token.span.start <= name_span.start {
            continue;
        }

        let Some((line, _)) = doc.offset_to_position(token.span.start) else {
            continue;
        };
        if line != binding_line {
            break;
        }

        match &token.kind {
            lexer::TokenKind::Ident(_) => out.push(token.span),
            lexer::TokenKind::Eq => break,
            lexer::TokenKind::ColonColon => return Vec::new(),
            _ => {}
        }
    }
    out
}

fn binding_name_span(
    doc: &Document,
    tokens: &[lexer::Token],
    binding: &Binding,
) -> Option<kscr::lexer::Span> {
    let (binding_line, _) = doc.offset_to_position(binding.pat.span.start)?;

    let mut saw_lparen = false;
    for token in tokens {
        if token.span.start < binding.pat.span.start {
            continue;
        }

        let (line, _) = doc.offset_to_position(token.span.start)?;
        if line != binding_line {
            break;
        }

        match &token.kind {
            lexer::TokenKind::Ident(_) => return Some(token.span),
            kind if saw_lparen && is_operator_name_token(kind) => return Some(token.span),
            lexer::TokenKind::LParen => saw_lparen = true,
            lexer::TokenKind::Eq | lexer::TokenKind::ColonColon => break,
            lexer::TokenKind::Newline | lexer::TokenKind::Indent | lexer::TokenKind::Dedent => {}
            _ => break,
        }
    }

    None
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
    use kscr::parser;
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
                InlayHintLabel::LabelParts(parts) => {
                    parts.iter().map(|part| part.value.clone()).collect()
                }
            })
            .collect();

        assert!(labels
            .iter()
            .any(|label| label.contains(":: Integer -> Integer")));
        assert!(labels.iter().any(|label| label.contains(":: Integer")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inlay_hints_anchor_on_function_name_and_lhs_param() {
        let src = "module Main where\n  funcB x = x * 2\n";
        let module = parser::parse_module(src).unwrap();
        let binding = match &module.items[0] {
            Item::Binding(binding) => binding,
            other => panic!("expected binding, got {other:?}"),
        };

        assert_eq!(binding.pat.span.start, src.find("funcB").unwrap());
        assert_eq!(
            binding.pat.span.end,
            src.find("funcB").unwrap() + "funcB".len()
        );

        let uri = Url::parse("file:///test.ks").unwrap();
        let doc = Document::new(uri, src.to_string(), 1);
        let tokens = lexer::lex(src).unwrap();
        let name_span = binding_name_span(&doc, &tokens, binding).unwrap();
        assert_eq!(name_span.start, src.find("funcB").unwrap());
        assert_eq!(name_span.end, src.find("funcB").unwrap() + "funcB".len());

        let param_spans = binding_lhs_parameter_spans(&doc, &tokens, binding, name_span);
        assert_eq!(param_spans.len(), 1);
        assert_eq!(param_spans[0].start, src.find("x =").unwrap());
        assert_eq!(param_spans[0].end, src.find("x =").unwrap() + 1);
    }

    #[test]
    fn inlay_hints_attach_to_symbolic_binding_and_all_lhs_params() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("kscr-lsp-inlay-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();

        let path = dir.join("Main.ks");
        let src = "module Main where\n  (/^) x y = x\n";
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

        let labels_and_columns: Vec<(String, u32)> = hints
            .iter()
            .map(|hint| {
                let label = match &hint.label {
                    InlayHintLabel::String(s) => s.clone(),
                    InlayHintLabel::LabelParts(parts) => {
                        parts.iter().map(|part| part.value.clone()).collect()
                    }
                };
                (label, hint.position.character)
            })
            .collect();

        let slash_column = (src.lines().nth(1).unwrap().find(')').unwrap() + 1) as u32;
        let x_column = src.lines().nth(1).unwrap().find("x y").unwrap() as u32 + 1;
        let y_column = src.lines().nth(1).unwrap().rfind('y').unwrap() as u32 + 1;

        assert!(
            labels_and_columns
                .iter()
                .any(|(label, column)| label.contains(":: a -> b -> a") && *column == slash_column),
            "labels_and_columns: {labels_and_columns:?}"
        );
        assert!(
            labels_and_columns
                .iter()
                .any(|(label, column)| label == ":: a" && *column == x_column),
            "labels_and_columns: {labels_and_columns:?}"
        );
        assert!(
            labels_and_columns
                .iter()
                .any(|(label, column)| label == ":: b" && *column == y_column),
            "labels_and_columns: {labels_and_columns:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inlay_hints_pretty_print_type_variables() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("kscr-lsp-inlay-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();

        let path = dir.join("Main.ks");
        let src = "module Main where\n  on f g x y = f (g x) (g y)\n";
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
                InlayHintLabel::LabelParts(parts) => {
                    parts.iter().map(|part| part.value.clone()).collect()
                }
            })
            .collect();

        assert!(
            labels
                .iter()
                .any(|label| label.contains(":: (a -> a -> b) -> (c -> a) -> c -> c -> b")),
            "labels: {labels:?}"
        );
        assert!(
            !labels
                .iter()
                .any(|label| label.contains("t0") || label.contains("t1") || label.contains("t2")),
            "labels: {labels:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inlay_hints_include_class_default_method_parameters() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("kscr-lsp-inlay-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();

        let path = dir.join("Main.ks");
        let src = "module Main where\n  class Field a where\n    divide :: a -> a -> a\n    divide x y = x\n";
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
                InlayHintLabel::LabelParts(parts) => {
                    parts.iter().map(|part| part.value.clone()).collect()
                }
            })
            .collect();

        assert!(
            labels
                .iter()
                .any(|label| label.contains("Field a => a -> a -> a")),
            "labels: {labels:?}"
        );
        assert!(
            labels.iter().any(|label| label == ":: a"),
            "labels: {labels:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inlay_hints_cover_real_prelude_field_bindings() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("stdlib/Prelude/Field.ks");
        let src = std::fs::read_to_string(&path).unwrap();
        let uri = Url::from_file_path(std::fs::canonicalize(&path).unwrap()).unwrap();
        let doc = Document::new(uri, src, 1);
        let hints = inlay_hints_in_doc(
            &doc,
            Range {
                start: Position {
                    line: 12,
                    character: 0,
                },
                end: Position {
                    line: 15,
                    character: 40,
                },
            },
        )
        .unwrap();

        let labels_and_positions: Vec<(String, u32, u32)> = hints
            .iter()
            .map(|hint| {
                let label = match &hint.label {
                    InlayHintLabel::String(s) => s.clone(),
                    InlayHintLabel::LabelParts(parts) => {
                        parts.iter().map(|part| part.value.clone()).collect()
                    }
                };
                (label, hint.position.line, hint.position.character)
            })
            .collect();

        assert!(
            labels_and_positions
                .iter()
                .any(|(label, line, _)| *line == 12 && label.contains("Field a => a -> a -> a")),
            "labels_and_positions: {labels_and_positions:?}"
        );
        assert!(
            labels_and_positions
                .iter()
                .any(|(label, line, col)| *line == 12 && *col == 12 && label == ":: a"),
            "labels_and_positions: {labels_and_positions:?}"
        );
        assert!(
            labels_and_positions
                .iter()
                .any(|(label, line, col)| *line == 12 && *col == 14 && label == ":: a"),
            "labels_and_positions: {labels_and_positions:?}"
        );
        assert!(
            labels_and_positions
                .iter()
                .any(|(label, line, col)| *line == 14
                    && *col == 6
                    && label.contains("Field a => a -> a -> a")),
            "labels_and_positions: {labels_and_positions:?}"
        );
        assert!(
            labels_and_positions
                .iter()
                .any(|(label, line, col)| *line == 14 && *col == 8 && label == ":: a"),
            "labels_and_positions: {labels_and_positions:?}"
        );
        assert!(
            labels_and_positions
                .iter()
                .any(|(label, line, col)| *line == 14 && *col == 10 && label == ":: a"),
            "labels_and_positions: {labels_and_positions:?}"
        );
    }
}
