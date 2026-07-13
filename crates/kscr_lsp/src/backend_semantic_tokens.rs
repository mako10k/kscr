use crate::backend_helpers::{find_ctor_name_span, find_decl_name_span, find_decl_name_span_any};
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
const TOKEN_TYPE_NAMESPACE: u32 = 7;

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
            SemanticTokenType::NAMESPACE,
        ],
        token_modifiers: Vec::new(),
    }
}

pub(crate) fn semantic_tokens_in_doc(doc: &Document) -> Option<SemanticTokens> {
    let mut raw = collect_raw_tokens(doc)?;
    raw.sort_by_key(|a| (a.0, a.1));
    raw.dedup();
    Some(encode_tokens(doc, raw))
}

pub(crate) fn semantic_tokens_in_range(doc: &Document, range: Range) -> Option<SemanticTokens> {
    let mut raw = collect_raw_tokens(doc)?;
    raw.retain(|(line, start, _length, _ty)| token_in_range(*line, *start, &range));
    raw.sort_by_key(|a| (a.0, a.1));
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

    for span in module_decl_name_spans(&lexed) {
        push_span_token(doc, span, TOKEN_TYPE_NAMESPACE, &mut raw);
    }
    for span in import_module_name_spans(&lexed) {
        push_span_token(doc, span, TOKEN_TYPE_NAMESPACE, &mut raw);
    }

    let mut search_from = 0usize;
    for item in &module.items {
        match item {
            Item::Binding(binding) => {
                if let PatternKind::Var(_) = &binding.pat.kind {
                    if let Some(name_span) = binding_name_span(doc, &lexed, binding) {
                        let lhs_params =
                            binding_lhs_parameter_spans(doc, &lexed, binding, name_span);
                        let token_type = if !lhs_params.is_empty()
                            || matches!(binding.expr.kind, ExprKind::Lambda { .. })
                        {
                            TOKEN_TYPE_FUNCTION
                        } else {
                            TOKEN_TYPE_VARIABLE
                        };
                        push_span_token(doc, name_span, token_type, &mut raw);
                        for span in lhs_params {
                            push_span_token(doc, span, TOKEN_TYPE_PARAMETER, &mut raw);
                        }
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
                    if let Some(span) = find_ctor_name_span(doc, ctor.span, &ctor.name) {
                        push_span_token(doc, span, TOKEN_TYPE_ENUM_MEMBER, &mut raw);
                    }
                }
            }
            Item::ClassDecl(class) => {
                if let Some(span) =
                    find_decl_name_span(doc, lexer::TokenKind::KwClass, class.name.as_str())
                {
                    push_span_token(doc, span, TOKEN_TYPE_CLASS, &mut raw);
                }
                for binding in &class.default_methods {
                    if let PatternKind::Var(_) = &binding.pat.kind {
                        if let Some(name_span) = binding_name_span(doc, &lexed, binding) {
                            push_span_token(doc, name_span, TOKEN_TYPE_METHOD, &mut raw);
                            for span in binding_lhs_parameter_spans(doc, &lexed, binding, name_span)
                            {
                                push_span_token(doc, span, TOKEN_TYPE_PARAMETER, &mut raw);
                            }
                        }
                    }
                }
            }
            Item::InstanceDecl(inst) => {
                if let Some((span, next_from)) =
                    instance_class_name_span(&lexed, search_from, &inst.class.name)
                {
                    push_span_token(doc, span, TOKEN_TYPE_CLASS, &mut raw);
                    search_from = next_from;
                }
                for method in &inst.methods {
                    if let PatternKind::Var(_) = &method.pat.kind {
                        if let Some(name_span) = binding_name_span(doc, &lexed, method) {
                            push_span_token(doc, name_span, TOKEN_TYPE_METHOD, &mut raw);
                            for span in binding_lhs_parameter_spans(doc, &lexed, method, name_span)
                            {
                                push_span_token(doc, span, TOKEN_TYPE_PARAMETER, &mut raw);
                            }
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

fn module_decl_name_spans(tokens: &[lexer::Token]) -> Vec<kscr::lexer::Span> {
    qualified_name_spans_after_keyword(tokens, lexer::TokenKind::KwModule)
        .into_iter()
        .next()
        .unwrap_or_default()
}

fn import_module_name_spans(tokens: &[lexer::Token]) -> Vec<kscr::lexer::Span> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < tokens.len() {
        if tokens[i].kind != lexer::TokenKind::KwImport {
            i += 1;
            continue;
        }
        i += 1;
        if matches!(&tokens.get(i).map(|t| &t.kind), Some(lexer::TokenKind::Ident(s)) if s == "qualified")
        {
            i += 1;
        }

        while i < tokens.len() {
            match &tokens[i].kind {
                lexer::TokenKind::Ident(_) => {
                    out.push(tokens[i].span);
                    i += 1;
                    if matches!(tokens.get(i).map(|t| &t.kind), Some(lexer::TokenKind::Dot)) {
                        i += 1;
                        continue;
                    }
                    break;
                }
                _ => break,
            }
        }
    }
    out
}

fn qualified_name_spans_after_keyword(
    tokens: &[lexer::Token],
    keyword: lexer::TokenKind,
) -> Vec<Vec<kscr::lexer::Span>> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < tokens.len() {
        if tokens[i].kind != keyword {
            i += 1;
            continue;
        }
        i += 1;
        let mut spans = Vec::new();
        while i < tokens.len() {
            match &tokens[i].kind {
                lexer::TokenKind::Ident(_) => {
                    spans.push(tokens[i].span);
                    i += 1;
                    if matches!(tokens.get(i).map(|t| &t.kind), Some(lexer::TokenKind::Dot)) {
                        i += 1;
                        continue;
                    }
                    break;
                }
                _ => break,
            }
        }
        if !spans.is_empty() {
            out.push(spans);
        }
    }
    out
}

fn instance_class_name_span(
    tokens: &[lexer::Token],
    search_from: usize,
    class_name: &str,
) -> Option<(kscr::lexer::Span, usize)> {
    let mut i = search_from;
    while i < tokens.len() {
        if tokens[i].kind != lexer::TokenKind::KwInstance {
            i += 1;
            continue;
        }

        let mut last_fat_arrow = None;
        let mut where_pos = None;
        let start = i;
        i += 1;
        while i < tokens.len() {
            match tokens[i].kind {
                lexer::TokenKind::FatArrow => last_fat_arrow = Some(i),
                lexer::TokenKind::KwWhere => {
                    where_pos = Some(i);
                    break;
                }
                _ => {}
            }
            i += 1;
        }

        let end = where_pos?;
        let scan_start = last_fat_arrow.map(|idx| idx + 1).unwrap_or(start + 1);
        for token in tokens.iter().take(end).skip(scan_start) {
            if let lexer::TokenKind::Ident(name) = &token.kind {
                if name == class_name {
                    return Some((token.span, end + 1));
                }
            }
        }
    }
    None
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

    fn absolute_tokens(tokens: &SemanticTokens) -> Vec<(u32, u32, u32, u32)> {
        let mut out = Vec::new();
        let mut line = 0;
        let mut start = 0;
        for token in &tokens.data {
            line += token.delta_line;
            if token.delta_line == 0 {
                start += token.delta_start;
            } else {
                start = token.delta_start;
            }
            out.push((line, start, token.length, token.token_type));
        }
        out
    }

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
        assert_eq!(legend.token_types.len(), 8);
        assert_eq!(legend.token_types[0], SemanticTokenType::FUNCTION);
        assert_eq!(legend.token_types[1], SemanticTokenType::TYPE);
        assert_eq!(legend.token_types[2], SemanticTokenType::CLASS);
        assert_eq!(legend.token_types[3], SemanticTokenType::METHOD);
        assert_eq!(legend.token_types[4], SemanticTokenType::ENUM_MEMBER);
        assert_eq!(legend.token_types[5], SemanticTokenType::VARIABLE);
        assert_eq!(legend.token_types[6], SemanticTokenType::PARAMETER);
        assert_eq!(legend.token_types[7], SemanticTokenType::NAMESPACE);
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
    fn semantic_tokens_mark_symbolic_binding_and_all_lhs_params() {
        let uri = tower_lsp::lsp_types::Url::parse("file:///semantic_tokens_symbolic.ks").unwrap();
        let src = "module Main where\n  (/^) x y = x\n";

        let doc = Document::new(uri, src.to_string(), 1);
        let tokens = semantic_tokens_in_doc(&doc).unwrap();
        let tokens = absolute_tokens(&tokens);

        assert!(
            tokens
                .iter()
                .any(|(line, start, length, token_type)| *line == 1
                    && *start == 3
                    && *length == 2
                    && *token_type == TOKEN_TYPE_FUNCTION),
            "tokens: {tokens:?}"
        );
        assert!(
            tokens
                .iter()
                .any(|(line, start, length, token_type)| *line == 1
                    && *start == 7
                    && *length == 1
                    && *token_type == TOKEN_TYPE_PARAMETER),
            "tokens: {tokens:?}"
        );
        assert!(
            tokens
                .iter()
                .any(|(line, start, length, token_type)| *line == 1
                    && *start == 9
                    && *length == 1
                    && *token_type == TOKEN_TYPE_PARAMETER),
            "tokens: {tokens:?}"
        );
    }

    #[test]
    fn semantic_tokens_mark_class_default_method_parameters() {
        let uri =
            tower_lsp::lsp_types::Url::parse("file:///semantic_tokens_class_default.ks").unwrap();
        let src = "module Main where\n  class Field a where\n    divide :: a -> a -> a\n    divide x y = x\n";

        let doc = Document::new(uri, src.to_string(), 1);
        let tokens = absolute_tokens(&semantic_tokens_in_doc(&doc).unwrap());

        assert!(
            tokens
                .iter()
                .any(|(line, start, length, token_type)| *line == 3
                    && *start == 4
                    && *length == 6
                    && *token_type == TOKEN_TYPE_METHOD),
            "tokens: {tokens:?}"
        );
        assert!(
            tokens
                .iter()
                .any(|(line, start, length, token_type)| *line == 3
                    && *start == 11
                    && *length == 1
                    && *token_type == TOKEN_TYPE_PARAMETER),
            "tokens: {tokens:?}"
        );
        assert!(
            tokens
                .iter()
                .any(|(line, start, length, token_type)| *line == 3
                    && *start == 13
                    && *length == 1
                    && *token_type == TOKEN_TYPE_PARAMETER),
            "tokens: {tokens:?}"
        );
    }

    #[test]
    fn semantic_tokens_cover_real_prelude_field_bindings() {
        let path = crate::backend_helpers::repo_source_path("stdlib/Prelude/Field.ks");
        let src = std::fs::read_to_string(&path).unwrap();
        let uri = tower_lsp::lsp_types::Url::from_file_path(std::fs::canonicalize(&path).unwrap())
            .unwrap();
        let doc = Document::new(uri, src, 1);
        let tokens = absolute_tokens(
            &semantic_tokens_in_range(
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
            .unwrap(),
        );

        assert!(
            tokens
                .iter()
                .any(|(line, start, length, token_type)| *line == 12
                    && *start == 4
                    && *length == 6
                    && *token_type == TOKEN_TYPE_METHOD),
            "tokens: {tokens:?}"
        );
        assert!(
            tokens
                .iter()
                .any(|(line, start, length, token_type)| *line == 12
                    && *start == 11
                    && *length == 1
                    && *token_type == TOKEN_TYPE_PARAMETER),
            "tokens: {tokens:?}"
        );
        assert!(
            tokens
                .iter()
                .any(|(line, start, length, token_type)| *line == 12
                    && *start == 13
                    && *length == 1
                    && *token_type == TOKEN_TYPE_PARAMETER),
            "tokens: {tokens:?}"
        );
        assert!(
            tokens
                .iter()
                .any(|(line, start, length, token_type)| *line == 14
                    && *start == 3
                    && *length == 2
                    && *token_type == TOKEN_TYPE_FUNCTION),
            "tokens: {tokens:?}"
        );
        assert!(
            tokens
                .iter()
                .any(|(line, start, length, token_type)| *line == 14
                    && *start == 7
                    && *length == 1
                    && *token_type == TOKEN_TYPE_PARAMETER),
            "tokens: {tokens:?}"
        );
        assert!(
            tokens
                .iter()
                .any(|(line, start, length, token_type)| *line == 14
                    && *start == 9
                    && *length == 1
                    && *token_type == TOKEN_TYPE_PARAMETER),
            "tokens: {tokens:?}"
        );
    }

    #[test]
    fn semantic_tokens_mark_only_ctor_name_in_data_decl() {
        let uri = tower_lsp::lsp_types::Url::parse("file:///semantic_tokens_ctor_decl.ks").unwrap();
        let src = r#"module Main where
  data Pair = Pair Integer Integer
"#;

        let doc = Document::new(uri, src.to_string(), 1);
        let tokens = semantic_tokens_in_doc(&doc).unwrap();

        let mut enum_member_lengths = Vec::new();
        for token in &tokens.data {
            if token.token_type == TOKEN_TYPE_ENUM_MEMBER {
                enum_member_lengths.push(token.length);
            }
        }

        assert_eq!(enum_member_lengths, vec![4]);
    }

    #[test]
    fn semantic_tokens_mark_symbolic_ctor_names_in_data_decls() {
        for (name, decl) in [
            ("prefix", "data Pair a b = (:*:) a b"),
            ("infix", "data Pair a b = a :*: b"),
            ("infix_uppercase", "data Pair b = A :*: b"),
        ] {
            let uri = tower_lsp::lsp_types::Url::parse(&format!(
                "file:///semantic_tokens_{name}_ctor_decl.ks"
            ))
            .unwrap();
            let src = format!("module Main where\n  {decl}\n");
            let doc = Document::new(uri, src, 1);
            let tokens = semantic_tokens_in_doc(&doc).unwrap();

            let enum_member_lengths: Vec<u32> = tokens
                .data
                .iter()
                .filter(|token| token.token_type == TOKEN_TYPE_ENUM_MEMBER)
                .map(|token| token.length)
                .collect();

            assert_eq!(enum_member_lengths, vec![3], "declaration: `{decl}`");
        }
    }

    #[test]
    fn semantic_tokens_classify_module_import_and_instance_class_names() {
        let uri =
            tower_lsp::lsp_types::Url::parse("file:///semantic_tokens_namespaces.ks").unwrap();
        let src = r#"module ManualSemigroup where
  import Prelude

  data Pair = Pair Integer Integer

  instance Semigroup Pair where
    (<>) = \x y -> x
"#;

        let doc = Document::new(uri, src.to_string(), 1);
        let tokens = semantic_tokens_in_doc(&doc).unwrap();

        let mut namespace_lengths = Vec::new();
        let mut class_lengths = Vec::new();
        for token in &tokens.data {
            if token.token_type == TOKEN_TYPE_NAMESPACE {
                namespace_lengths.push(token.length);
            }
            if token.token_type == TOKEN_TYPE_CLASS {
                class_lengths.push(token.length);
            }
        }

        assert!(namespace_lengths.contains(&15));
        assert!(namespace_lengths.contains(&7));
        assert!(class_lengths.contains(&9));
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
