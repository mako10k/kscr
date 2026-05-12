use crate::backend_helpers::{find_decl_name_span, find_decl_name_span_any};
use crate::vfs::Document;
use kscr::ast::{Binding, ExprKind, Item, PatternKind};
use kscr::lexer;
use kscr::parser;
use tower_lsp::lsp_types::{
    Position, Range, SemanticToken, SemanticTokenType, SemanticTokens, SemanticTokensDelta,
    SemanticTokensEdit, SemanticTokensFullDeltaResult, SemanticTokensLegend,
};

const TOKEN_TYPE_FUNCTION: u32 = 0;
const TOKEN_TYPE_TYPE: u32 = 1;
const TOKEN_TYPE_CLASS: u32 = 2;
const TOKEN_TYPE_METHOD: u32 = 3;
const TOKEN_TYPE_ENUM_MEMBER: u32 = 4;
const TOKEN_TYPE_VARIABLE: u32 = 5;
const TOKEN_TYPE_PARAMETER: u32 = 6;

pub(crate) fn semantic_tokens_legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: vec![
            SemanticTokenType::FUNCTION,
            SemanticTokenType::TYPE,
            SemanticTokenType::CLASS,
            SemanticTokenType::METHOD,
            SemanticTokenType::ENUM_MEMBER,
            SemanticTokenType::VARIABLE,
            SemanticTokenType::PARAMETER,
        ],
        token_modifiers: Vec::new(),
    }
}

pub(crate) fn semantic_tokens_in_doc(doc: &Document) -> Option<SemanticTokens> {
    let mut raw = collect_raw_tokens(doc)?;
    raw.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    raw.dedup();
    Some(encode_tokens(doc, raw))
}

pub(crate) fn semantic_tokens_in_range(doc: &Document, range: Range) -> Option<SemanticTokens> {
    let mut raw = collect_raw_tokens(doc)?;
    raw.retain(|(line, start, _length, _ty)| token_in_range(*line, *start, &range));
    raw.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    raw.dedup();
    Some(encode_tokens(doc, raw))
}

pub(crate) fn semantic_tokens_full_delta_from_previous(
    previous: &SemanticTokens,
    current: SemanticTokens,
) -> SemanticTokensFullDeltaResult {
    let old_flat = flatten_tokens(&previous.data);
    let new_flat = flatten_tokens(&current.data);

    let mut prefix = 0usize;
    while prefix < old_flat.len() && prefix < new_flat.len() && old_flat[prefix] == new_flat[prefix]
    {
        prefix += 1;
    }

    let mut suffix = 0usize;
    while suffix < old_flat.len().saturating_sub(prefix)
        && suffix < new_flat.len().saturating_sub(prefix)
        && old_flat[old_flat.len() - 1 - suffix] == new_flat[new_flat.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let old_mid_end = old_flat.len().saturating_sub(suffix);
    let new_mid_end = new_flat.len().saturating_sub(suffix);

    let old_mid = &old_flat[prefix..old_mid_end];
    let new_mid = &new_flat[prefix..new_mid_end];

    let edit_data = if new_mid.is_empty() {
        None
    } else {
        Some(unflatten_tokens(new_mid))
    };

    let edits = if old_mid.is_empty() && new_mid.is_empty() {
        Vec::new()
    } else {
        vec![SemanticTokensEdit {
            start: prefix as u32,
            delete_count: old_mid.len() as u32,
            data: edit_data,
        }]
    };

    SemanticTokensFullDeltaResult::TokensDelta(SemanticTokensDelta {
        result_id: current.result_id,
        edits,
    })
}

fn flatten_tokens(tokens: &[SemanticToken]) -> Vec<u32> {
    let mut out = Vec::with_capacity(tokens.len() * 5);
    for token in tokens {
        out.push(token.delta_line);
        out.push(token.delta_start);
        out.push(token.length);
        out.push(token.token_type);
        out.push(token.token_modifiers_bitset);
    }
    out
}

fn unflatten_tokens(flat: &[u32]) -> Vec<SemanticToken> {
    flat.chunks_exact(5)
        .map(|c| SemanticToken {
            delta_line: c[0],
            delta_start: c[1],
            length: c[2],
            token_type: c[3],
            token_modifiers_bitset: c[4],
        })
        .collect()
}

fn collect_raw_tokens(doc: &Document) -> Option<Vec<(u32, u32, u32, u32)>> {
    let module = parser::parse_module(&doc.text).ok()?;
    let lexed = lexer::lex(&doc.text).ok()?;
    let mut raw: Vec<(u32, u32, u32, u32)> = Vec::new();

    for item in &module.items {
        match item {
            Item::Binding(binding) => {
                if let PatternKind::Var(_) = &binding.pat.kind {
                    let lhs_params = binding_lhs_parameter_spans(doc, &lexed, binding);
                    let token_type = if !lhs_params.is_empty()
                        || matches!(binding.expr.kind, ExprKind::Lambda { .. })
                    {
                        TOKEN_TYPE_FUNCTION
                    } else {
                        TOKEN_TYPE_VARIABLE
                    };
                    push_span_token(doc, binding.pat.span, token_type, &mut raw);
                    for span in lhs_params {
                        push_span_token(doc, span, TOKEN_TYPE_PARAMETER, &mut raw);
                    }
                }
            }
            Item::TypeAlias(alias) => {
                if let Some(span) =
                    find_decl_name_span(doc, lexer::TokenKind::KwType, alias.name.as_str())
                {
                    push_span_token(doc, span, TOKEN_TYPE_TYPE, &mut raw);
                }
            }
            Item::DataDecl(data) => {
                if let Some(span) = find_decl_name_span_any(
                    doc,
                    &[lexer::TokenKind::KwData, lexer::TokenKind::KwNewtype],
                    data.name.as_str(),
                ) {
                    push_span_token(doc, span, TOKEN_TYPE_TYPE, &mut raw);
                }
                for ctor in &data.ctors {
                    push_span_token(doc, ctor.span, TOKEN_TYPE_ENUM_MEMBER, &mut raw);
                }
            }
            Item::ClassDecl(class) => {
                if let Some(span) =
                    find_decl_name_span(doc, lexer::TokenKind::KwClass, class.name.as_str())
                {
                    push_span_token(doc, span, TOKEN_TYPE_CLASS, &mut raw);
                }
            }
            Item::InstanceDecl(inst) => {
                for method in &inst.methods {
                    if let PatternKind::Var(_) = &method.pat.kind {
                        push_span_token(doc, method.pat.span, TOKEN_TYPE_METHOD, &mut raw);
                        for span in binding_lhs_parameter_spans(doc, &lexed, method) {
                            push_span_token(doc, span, TOKEN_TYPE_PARAMETER, &mut raw);
                        }
                    }
                }
            }
            Item::Import(_) | Item::Export(_) | Item::Fixity(_) => {}
        }
    }

    for span in lambda_parameter_spans(&lexed) {
        push_span_token(doc, span, TOKEN_TYPE_PARAMETER, &mut raw);
    }

    Some(raw)
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
            lexer::TokenKind::Ident(_) if !seen_name => {
                seen_name = true;
            }
            lexer::TokenKind::Ident(_) if seen_name => out.push(token.span),
            lexer::TokenKind::Eq if seen_name => break,
            lexer::TokenKind::ColonColon if seen_name => return Vec::new(),
            _ => {}
        }
    }

    out
}

fn lambda_parameter_spans(tokens: &[lexer::Token]) -> Vec<kscr::lexer::Span> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < tokens.len() {
        if tokens[i].kind != lexer::TokenKind::Backslash {
            i += 1;
            continue;
        }

        i += 1;
        while i < tokens.len() {
            match &tokens[i].kind {
                lexer::TokenKind::Ident(_) => {
                    out.push(tokens[i].span);
                    i += 1;
                }
                lexer::TokenKind::Arrow => break,
                lexer::TokenKind::Newline | lexer::TokenKind::Indent | lexer::TokenKind::Dedent => {
                    i += 1;
                }
                _ => break,
            }
        }
    }
    out
}

fn encode_tokens(doc: &Document, raw: Vec<(u32, u32, u32, u32)>) -> SemanticTokens {
    let mut data = Vec::with_capacity(raw.len());
    let mut prev_line = 0;
    let mut prev_start = 0;

    for (line, start, length, token_type) in raw {
        if length == 0 {
            continue;
        }

        let delta_line = line.saturating_sub(prev_line);
        let delta_start = if delta_line == 0 {
            start.saturating_sub(prev_start)
        } else {
            start
        };

        data.push(SemanticToken {
            delta_line,
            delta_start,
            length,
            token_type,
            token_modifiers_bitset: 0,
        });

        prev_line = line;
        prev_start = start;
    }

    SemanticTokens {
        result_id: Some(doc.version.to_string()),
        data,
    }
}

fn token_in_range(line: u32, start_col: u32, range: &Range) -> bool {
    let pos = Position {
        line,
        character: start_col,
    };

    if pos.line < range.start.line || pos.line > range.end.line {
        return false;
    }
    if pos.line == range.start.line && pos.character < range.start.character {
        return false;
    }
    if pos.line == range.end.line && pos.character >= range.end.character {
        return false;
    }
    true
}

fn push_span_token(
    doc: &Document,
    span: kscr::lexer::Span,
    token_type: u32,
    out: &mut Vec<(u32, u32, u32, u32)>,
) {
    let start = span.start.min(doc.text.len());
    let mut end = span.end.min(doc.text.len());
    if end < start {
        end = start;
    }
    if end == start {
        return;
    }

    let Some((line, start_col)) = doc.offset_to_position(start) else {
        return;
    };
    let Some((end_line, end_col)) = doc.offset_to_position(end) else {
        return;
    };
    if end_line != line {
        return;
    }
    if end_col <= start_col {
        return;
    }

    out.push((line, start_col, end_col - start_col, token_type));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_tokens_non_empty_for_basic_module() {
        let uri = tower_lsp::lsp_types::Url::parse("file:///semantic_tokens_test.ks").unwrap();
        let src = r#"module Main where
  data Opt a = Some a | None
  class ShowLike a where
    showLike :: a -> String
  answer = Some 42
"#;

        let doc = Document::new(uri, src.to_string(), 1);
        let tokens = semantic_tokens_in_doc(&doc).unwrap();
        assert!(!tokens.data.is_empty());
    }

    #[test]
    fn semantic_token_legend_has_expected_order() {
        let legend = semantic_tokens_legend();
        assert_eq!(legend.token_types.len(), 7);
        assert_eq!(legend.token_types[0], SemanticTokenType::FUNCTION);
        assert_eq!(legend.token_types[1], SemanticTokenType::TYPE);
        assert_eq!(legend.token_types[2], SemanticTokenType::CLASS);
        assert_eq!(legend.token_types[3], SemanticTokenType::METHOD);
        assert_eq!(legend.token_types[4], SemanticTokenType::ENUM_MEMBER);
        assert_eq!(legend.token_types[5], SemanticTokenType::VARIABLE);
        assert_eq!(legend.token_types[6], SemanticTokenType::PARAMETER);
    }

    #[test]
    fn semantic_tokens_include_variable_and_parameter_kinds() {
        let uri = tower_lsp::lsp_types::Url::parse("file:///semantic_tokens_params.ks").unwrap();
        let src = r#"module Main where
  applyTwice f x = f (f x)
  answer = 42
  idFn = \value -> value
"#;

        let doc = Document::new(uri, src.to_string(), 1);
        let tokens = semantic_tokens_in_doc(&doc).unwrap();
        let seen: std::collections::HashSet<u32> =
            tokens.data.iter().map(|token| token.token_type).collect();

        assert!(seen.contains(&TOKEN_TYPE_FUNCTION));
        assert!(seen.contains(&TOKEN_TYPE_VARIABLE));
        assert!(seen.contains(&TOKEN_TYPE_PARAMETER));
    }

    #[test]
    fn semantic_tokens_in_range_filters_outside_lines() {
        let uri = tower_lsp::lsp_types::Url::parse("file:///semantic_tokens_range.ks").unwrap();
        let src = r#"module Main where
  data Opt a = Some a | None
  answer = Some 42
"#;
        let doc = Document::new(uri, src.to_string(), 3);

        let range = Range {
            start: Position {
                line: 2,
                character: 0,
            },
            end: Position {
                line: 3,
                character: 0,
            },
        };
        let tokens = semantic_tokens_in_range(&doc, range).unwrap();
        assert!(!tokens.data.is_empty());
        assert!(tokens
            .data
            .iter()
            .all(|t| t.delta_line == 0 || t.delta_line == 2));
    }

    #[test]
    fn semantic_tokens_full_delta_returns_tokens_variant() {
        let uri = tower_lsp::lsp_types::Url::parse("file:///semantic_tokens_delta.ks").unwrap();
        let src = "module Main where\n  x = 1\n";
        let doc = Document::new(uri, src.to_string(), 7);

        let tokens = semantic_tokens_in_doc(&doc).unwrap();
        match SemanticTokensFullDeltaResult::Tokens(tokens) {
            SemanticTokensFullDeltaResult::Tokens(tokens) => {
                assert_eq!(tokens.result_id.as_deref(), Some("7"));
                assert!(!tokens.data.is_empty());
            }
            SemanticTokensFullDeltaResult::TokensDelta(_)
            | SemanticTokensFullDeltaResult::PartialTokensDelta { .. } => {
                panic!("expected full tokens fallback")
            }
        }
    }

    #[test]
    fn semantic_tokens_full_delta_returns_delta_when_previous_exists() {
        let uri =
            tower_lsp::lsp_types::Url::parse("file:///semantic_tokens_delta_prev.ks").unwrap();
        let old_src = "module Main where\n  x = 1\n";
        let new_src = "module Main where\n  xyz = 1\n";

        let old_doc = Document::new(uri.clone(), old_src.to_string(), 1);
        let new_doc = Document::new(uri, new_src.to_string(), 2);

        let previous = semantic_tokens_in_doc(&old_doc).unwrap();
        let current = semantic_tokens_in_doc(&new_doc).unwrap();
        let delta = semantic_tokens_full_delta_from_previous(&previous, current);
        match delta {
            SemanticTokensFullDeltaResult::TokensDelta(d) => {
                assert_eq!(d.result_id.as_deref(), Some("2"));
                assert!(!d.edits.is_empty());
            }
            _ => panic!("expected TokensDelta"),
        }
    }
}
