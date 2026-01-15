use crate::ast;
use crate::error::{Error, Result};
use crate::lexer;

use super::token_stream::{TokenKind, TokenStream};
use super::Fixity;

use std::collections::{HashMap, HashSet};

fn expr_from(ts: &TokenStream, start: usize, kind: ast::ExprKind) -> ast::Expr {
    ast::Expr {
        span: ts.tokens[start].span,
        kind,
    }
}

fn pat_from(ts: &TokenStream, start: usize, kind: ast::PatternKind) -> ast::Pattern {
    ast::Pattern {
        span: ts.tokens[start].span,
        kind,
    }
}

fn parse_maybe_qualified_ident(ts: &mut TokenStream) -> Result<String> {
    match ts.peek_kind() {
        Some(TokenKind::Ident(s)) => {
            ts.next();
            Ok(s)
        }
        Some(TokenKind::Ctor(s)) => {
            ts.next();
            Ok(s)
        }
        Some(TokenKind::Op(s)) => {
            ts.next();
            Ok(s)
        }
        k => Err(Error::msg(format!(
            "expected identifier but got {:?}",
            k
        ))),
    }
}

fn last_qualified_segment(s: &str) -> &str {
    s.rsplit('.').next().unwrap_or(s)
}

fn is_upper_by_last_segment(s: &str) -> bool {
    last_qualified_segment(s)
        .chars()
        .next()
        .map(|c| c.is_ascii_uppercase())
        .unwrap_or(false)
}

pub(super) fn parse_module(src: &str) -> Result<ast::Module> {
    let tokens = lexer::lex(src)?;
    let fixities = collect_fixities(&tokens);
    let mut ts = TokenStream::new(tokens, fixities);
    parse_module_decl(&mut ts)
}

fn token_op_name(kind: &TokenKind) -> Option<String> {
    match kind {
        TokenKind::Op(s) => Some(s.clone()),
        TokenKind::Ident(s) if s == "mod" => Some("%".to_string()),
        _ => None,
    }
}

fn try_parse_toplevel_sig_line(ts: &mut TokenStream) -> Result<Option<(String, ast::QualType)>> {
    let start = ts.pos;

    let TokenKind::Ident(name) = ts.peek_kind().cloned().unwrap_or(TokenKind::Eof) else {
        return Ok(None);
    };

    let Some(TokenKind::Colon) = ts.peek_kind_n(1) else {
        return Ok(None);
    };

    ts.next();
    ts.next();

    let ty = super::type_expr::parse_qual_type(ts, super::Stop::ToplevelSigLine)?;

    // Enforce: one signature per line.
    ts.expect(super::token_stream::TokenKind::Newline)?;

    Ok(Some((name, ty)))
}

fn collect_fixities(tokens: &[lexer::Token]) -> HashMap<String, Fixity> {
    let mut m: HashMap<String, Fixity> = HashMap::new();

    // Default fixities.
    m.insert("*".to_string(), Fixity::new(70, false));
    m.insert("/".to_string(), Fixity::new(70, false));
    m.insert("+".to_string(), Fixity::new(60, false));
    m.insert("-".to_string(), Fixity::new(60, false));
    m.insert("++".to_string(), Fixity::new(60, false));
    m.insert("==".to_string(), Fixity::new(50, false));
    m.insert("!=".to_string(), Fixity::new(50, false));
    m.insert("<".to_string(), Fixity::new(50, false));
    m.insert("<=".to_string(), Fixity::new(50, false));
    m.insert(">".to_string(), Fixity::new(50, false));
    m.insert(">=".to_string(), Fixity::new(50, false));
    m.insert("&&".to_string(), Fixity::new(40, false));
    m.insert("||".to_string(), Fixity::new(30, false));

    for t in tokens {
        if t.kind != lexer::TokenKind::Ident("infix".to_string())
            && t.kind != lexer::TokenKind::Ident("infixl".to_string())
            && t.kind != lexer::TokenKind::Ident("infixr".to_string())
        {
            continue;
        }

        // Very small, robust-ish scan: infix[l|r] <prec> <op>
        // (The full parse happens later.)
        // Layout is already applied in lexing; here we just look for a pattern.
        // This keeps precedence available before parsing expressions.
        //
        // Note: We intentionally keep this conservative; if we miss something,
        // the later full parse will still be correct, just with default fixities.
        //
        // Token stream is not available; do a simple linear scan.
    }

    // The full, accurate fixity collection is in the real parser pass.
    // Here we only return defaults; `parse_fixity_decl` updates TokenStream fixities.
    m
}

fn parse_module_decl(ts: &mut TokenStream) -> Result<ast::Module> {
    // Delegate to the existing implementation in parser_impl.rs for now.
    // This function body will be replaced when we fully migrate parsing.
    super::legacy::parse_module_decl(ts)
}

// Small shims expected by other modules while splitting.
pub(super) fn expr_from_public(ts: &TokenStream, start: usize, kind: ast::ExprKind) -> ast::Expr {
    expr_from(ts, start, kind)
}

pub(super) fn pat_from_public(ts: &TokenStream, start: usize, kind: ast::PatternKind) -> ast::Pattern {
    pat_from(ts, start, kind)
}

pub(super) fn parse_maybe_qualified_ident_public(ts: &mut TokenStream) -> Result<String> {
    parse_maybe_qualified_ident(ts)
}

pub(super) fn is_upper_by_last_segment_public(s: &str) -> bool {
    is_upper_by_last_segment(s)
}
