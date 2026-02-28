use crate::backend_helpers::find_decl_name_span;
use crate::vfs::Document;
use kscr::ast::{Item, PatternKind};
use kscr::lexer;
use kscr::parser;
use tower_lsp::lsp_types::{
    Position, Range, SemanticToken, SemanticTokenType, SemanticTokens,
    SemanticTokensFullDeltaResult, SemanticTokensLegend,
};

const TOKEN_TYPE_FUNCTION: u32 = 0;
const TOKEN_TYPE_TYPE: u32 = 1;
const TOKEN_TYPE_CLASS: u32 = 2;
const TOKEN_TYPE_METHOD: u32 = 3;
const TOKEN_TYPE_ENUM_MEMBER: u32 = 4;

pub(crate) fn semantic_tokens_legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: vec![
            SemanticTokenType::FUNCTION,
            SemanticTokenType::TYPE,
            SemanticTokenType::CLASS,
            SemanticTokenType::METHOD,
            SemanticTokenType::ENUM_MEMBER,
        ],
        token_modifiers: Vec::new(),
    }
}

pub(crate) fn semantic_tokens_in_doc(doc: &Document) -> Option<SemanticTokens> {
    let mut raw = collect_raw_tokens(doc)?;
    raw.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    Some(encode_tokens(doc, raw))
}

pub(crate) fn semantic_tokens_in_range(doc: &Document, range: Range) -> Option<SemanticTokens> {
    let mut raw = collect_raw_tokens(doc)?;
    raw.retain(|(line, start, _length, _ty)| token_in_range(*line, *start, &range));
    raw.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    Some(encode_tokens(doc, raw))
}

pub(crate) fn semantic_tokens_full_delta_in_doc(
    doc: &Document,
    _previous_result_id: Option<&str>,
) -> Option<SemanticTokensFullDeltaResult> {
    semantic_tokens_in_doc(doc).map(SemanticTokensFullDeltaResult::Tokens)
}

fn collect_raw_tokens(doc: &Document) -> Option<Vec<(u32, u32, u32, u32)>> {
    let module = parser::parse_module(&doc.text).ok()?;
    let mut raw: Vec<(u32, u32, u32, u32)> = Vec::new();

    for item in &module.items {
        match item {
            Item::Binding(binding) => {
                if let PatternKind::Var(_) = &binding.pat.kind {
                    push_span_token(doc, binding.pat.span, TOKEN_TYPE_FUNCTION, &mut raw);
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
                if let Some(span) =
                    find_decl_name_span(doc, lexer::TokenKind::KwData, data.name.as_str())
                {
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
                    }
                }
            }
            Item::Import(_) | Item::Export(_) | Item::Fixity(_) => {}
        }
    }

    Some(raw)
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
        assert_eq!(legend.token_types.len(), 5);
        assert_eq!(legend.token_types[0], SemanticTokenType::FUNCTION);
        assert_eq!(legend.token_types[1], SemanticTokenType::TYPE);
        assert_eq!(legend.token_types[2], SemanticTokenType::CLASS);
        assert_eq!(legend.token_types[3], SemanticTokenType::METHOD);
        assert_eq!(legend.token_types[4], SemanticTokenType::ENUM_MEMBER);
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

        let delta = semantic_tokens_full_delta_in_doc(&doc, Some("6")).unwrap();
        match delta {
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
}
