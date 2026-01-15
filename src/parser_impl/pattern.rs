use crate::{ast, lexer::TokenKind, Result};

use crate::parser::token_stream::TokenStream;

use super::{is_ctor_symbol, is_sym_op_token, is_upper_by_last_segment, parse_expr, parse_maybe_qualified_ident, parse_operator_name, pat_from, Stop};

pub(super) fn parse_pattern(ts: &mut TokenStream) -> Result<ast::Pattern> {
    // Patterns support `x:xs` cons syntax; parse it at the top-level.
    parse_or_pattern(ts)
}

fn parse_or_pattern(ts: &mut TokenStream) -> Result<ast::Pattern> {
    let mut pat = parse_cons_pattern(ts)?;

    // IMPORTANT: `|` is used both for or-patterns (in case arms) and for guards (`pat | expr ->`).
    // Only treat it as an or-pattern when we can see it forms `pat1 | pat2 ->`.
    while matches!(ts.peek_kind(), Some(TokenKind::Pipe)) {
        let save = (ts.i, ts.last_span_end);
        ts.bump();

        if let Ok(rhs) = parse_cons_pattern(ts) {
            if matches!(ts.peek_kind(), Some(TokenKind::Arrow)) {
                let start = pat.span.start;
                pat = pat_from(
                    ts,
                    start,
                    ast::PatternKind::Or(Box::new(pat), Box::new(rhs)),
                );
                continue;
            }
        }

        (ts.i, ts.last_span_end) = save;
        break;
    }

    Ok(normalize_cons_constructor_in_pattern(pat))
}

fn parse_cons_pattern(ts: &mut TokenStream) -> Result<ast::Pattern> {
    let pat = parse_app_pattern(ts)?;

    // Cons pattern: x : xs (right-associative)
    if matches!(ts.peek_kind(), Some(TokenKind::Colon)) {
        ts.bump();
        let tail = parse_cons_pattern(ts)?;
        let start = pat.span.start;
        return Ok(pat_from(
            ts,
            start,
            ast::PatternKind::Cons(Box::new(pat), Box::new(tail)),
        ));
    }

    // Infix constructor operator pattern: a :*: b (right-associative)
    if let Some(TokenKind::Operator(op)) = ts.peek_kind() {
        if is_ctor_symbol(op.as_str()) {
            let op = op.clone();
            ts.bump();
            let rhs = parse_cons_pattern(ts)?;
            let start = pat.span.start;
            return Ok(pat_from(
                ts,
                start,
                ast::PatternKind::Constructor {
                    name: op,
                    args: vec![pat, rhs],
                },
            ));
        }
    }

    Ok(pat)
}

fn normalize_cons_constructor_in_pattern(mut pat: ast::Pattern) -> ast::Pattern {
    // Convert `(:) a b` patterns desugared as `Constructor { name: ":", args: [a,b] }`
    // into `Cons(a,b)` so downstream typechecking (and tests) see `Cons`.
    if let ast::PatternKind::Constructor { name, args } = &pat.kind {
        if name == ":" && args.len() == 2 {
            let lhs = args[0].clone();
            let rhs = args[1].clone();
            pat.kind = ast::PatternKind::Cons(Box::new(lhs), Box::new(rhs));
        }
    }
    pat
}

fn parse_app_pattern(ts: &mut TokenStream) -> Result<ast::Pattern> {
    let mut pat = parse_pattern_atom(ts)?;

    // As-pattern: x @ pat
    if let ast::PatternKind::Var(name) = &pat.kind {
        if matches!(ts.peek_kind(), Some(TokenKind::At)) {
            let name = name.clone();
            ts.bump();
            let inner = parse_pattern(ts)?;
            let start = pat.span.start;
            pat = pat_from(ts, start, ast::PatternKind::As(name, Box::new(inner)));
        }
    }

    // Constructor application: Just x y
    {
        let start = pat.span.start;
        let kind = std::mem::replace(&mut pat.kind, ast::PatternKind::Wildcard);
        if let ast::PatternKind::Constructor { name, mut args } = kind {
            while ts.can_continue_pattern() {
                args.push(parse_pattern_atom(ts)?);
            }
            if name == ":" {
                if args.len() == 2 {
                    let lhs = args.remove(0);
                    let rhs = args.remove(0);
                    pat = pat_from(
                        ts,
                        start,
                        ast::PatternKind::Cons(Box::new(lhs), Box::new(rhs)),
                    );
                } else {
                    return Err(ts.err_here("(:) pattern expects exactly 2 arguments"));
                }
            } else {
                pat = pat_from(ts, start, ast::PatternKind::Constructor { name, args });
            }
        } else {
            pat.kind = kind;
        }
    }

    Ok(pat)
}

pub(super) fn parse_pattern_atom(ts: &mut TokenStream) -> Result<ast::Pattern> {
    let start = ts.peek_span().map(|s| s.start).unwrap_or(0);
    match ts.peek_kind() {
        Some(TokenKind::Colon) => {
            // `:` as a standalone pattern atom represents the list constructor.
            // `parse_cons_pattern` uses the `TokenKind::Colon` branch before reaching here,
            // so this mainly supports `(:)` and other contexts.
            ts.bump();
            Ok(pat_from(
                ts,
                start,
                ast::PatternKind::Constructor {
                    name: ":".to_string(),
                    args: vec![],
                },
            ))
        }
        Some(TokenKind::LParen) => parse_paren_or_tuple_pattern(ts),
        Some(TokenKind::LBracket) => parse_list_pattern(ts),
        Some(TokenKind::LBrace) => parse_record_pattern(ts),

        Some(TokenKind::Ident(s)) if s == "_" => {
            ts.bump();
            Ok(pat_from(ts, start, ast::PatternKind::Wildcard))
        }
        Some(TokenKind::Question) => {
            ts.bump();
            let name = match ts.peek_kind() {
                Some(TokenKind::Ident(_)) => Some(ts.expect_ident()?),
                _ => None,
            };
            Ok(pat_from(ts, start, ast::PatternKind::Hole(name)))
        }
        Some(TokenKind::Ident(_)) => {
            let s = parse_maybe_qualified_ident(ts)?;
            if is_upper_by_last_segment(&s) {
                Ok(pat_from(
                    ts,
                    start,
                    ast::PatternKind::Constructor {
                        name: s,
                        args: vec![],
                    },
                ))
            } else {
                Ok(pat_from(ts, start, ast::PatternKind::Var(s)))
            }
        }

        Some(TokenKind::True) => {
            ts.bump();
            let lit = super::expr_from(ts, start, ast::ExprKind::Bool(true));
            Ok(pat_from(ts, start, ast::PatternKind::Literal(lit)))
        }
        Some(TokenKind::False) => {
            ts.bump();
            let lit = super::expr_from(ts, start, ast::ExprKind::Bool(false));
            Ok(pat_from(ts, start, ast::PatternKind::Literal(lit)))
        }
        Some(TokenKind::Integer(_)) => match ts.bump() {
            Some(TokenKind::Integer(s)) => {
                let lit = super::expr_from(ts, start, ast::ExprKind::Integer(s));
                Ok(pat_from(ts, start, ast::PatternKind::Literal(lit)))
            }
            _ => unreachable!(),
        },
        Some(TokenKind::Float(_)) => match ts.bump() {
            Some(TokenKind::Float(s)) => {
                let lit = super::expr_from(ts, start, ast::ExprKind::Float64(s));
                Ok(pat_from(ts, start, ast::PatternKind::Literal(lit)))
            }
            _ => unreachable!(),
        },
        Some(TokenKind::String(_)) => match ts.bump() {
            Some(TokenKind::String(s)) => {
                // Desugar string literal patterns into list-of-char patterns.
                // This aligns with Haskell surface semantics where String ~ [Char].
                let ps = s
                    .chars()
                    .map(|ch| {
                        let lit = ast::Expr::dummy(ast::ExprKind::Char(ch));
                        ast::Pattern::dummy(ast::PatternKind::Literal(lit))
                    })
                    .collect::<Vec<_>>();
                Ok(pat_from(ts, start, ast::PatternKind::List(ps)))
            }
            _ => unreachable!(),
        },
        Some(TokenKind::Char(_)) => match ts.bump() {
            Some(TokenKind::Char(ch)) => {
                let lit = super::expr_from(ts, start, ast::ExprKind::Char(ch));
                Ok(pat_from(ts, start, ast::PatternKind::Literal(lit)))
            }
            _ => unreachable!(),
        },

        _ => Err(ts.err_here("expected pattern")),
    }
}

fn parse_paren_or_tuple_pattern(ts: &mut TokenStream) -> Result<ast::Pattern> {
    let start = ts.peek_span().map(|s| s.start).unwrap_or(0);
    ts.expect(TokenKind::LParen)?;

    if matches!(ts.peek_kind(), Some(TokenKind::RParen)) {
        ts.bump();
        let unit = super::expr_from(ts, start, ast::ExprKind::Unit);
        return Ok(pat_from(ts, start, ast::PatternKind::Literal(unit)));
    }

    // Operator binder pattern: `(++)`.
    if matches!(ts.peek_kind(), Some(TokenKind::Backtick)) || is_sym_op_token(ts.peek_kind()) {
        let op = parse_operator_name(ts)?;
        ts.expect(TokenKind::RParen)?;
        if op == ":" || is_ctor_symbol(&op) {
            return Ok(pat_from(
                ts,
                start,
                ast::PatternKind::Constructor {
                    name: op,
                    args: vec![],
                },
            ));
        }
        return Ok(pat_from(ts, start, ast::PatternKind::Var(op)));
    }

    let first = parse_pattern(ts)?;

    // View pattern must be parenthesized: (pat <- expr)
    if matches!(ts.peek_kind(), Some(TokenKind::LeftArrow)) {
        ts.bump();
        let expr = parse_expr(ts, Stop::Pattern)?;
        ts.expect(TokenKind::RParen)?;
        return Ok(pat_from(
            ts,
            start,
            ast::PatternKind::View(Box::new(first), Box::new(expr)),
        ));
    }

    if matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
        let mut elems = vec![first];
        while matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
            ts.bump();
            elems.push(parse_pattern(ts)?);
        }
        ts.expect(TokenKind::RParen)?;
        Ok(pat_from(ts, start, ast::PatternKind::Tuple(elems)))
    } else {
        ts.expect(TokenKind::RParen)?;
        let ast::Pattern { kind, .. } = first;
        Ok(pat_from(ts, start, kind))
    }
}

fn parse_list_pattern(ts: &mut TokenStream) -> Result<ast::Pattern> {
    let start = ts.peek_span().map(|s| s.start).unwrap_or(0);
    ts.expect(TokenKind::LBracket)?;

    if matches!(ts.peek_kind(), Some(TokenKind::RBracket)) {
        ts.bump();
        return Ok(pat_from(ts, start, ast::PatternKind::List(vec![])));
    }

    let mut elems = vec![parse_pattern(ts)?];
    while matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
        ts.bump();
        elems.push(parse_pattern(ts)?);
    }

    ts.expect(TokenKind::RBracket)?;
    Ok(pat_from(ts, start, ast::PatternKind::List(elems)))
}

fn parse_record_pattern(ts: &mut TokenStream) -> Result<ast::Pattern> {
    let start = ts.peek_span().map(|s| s.start).unwrap_or(0);
    ts.expect(TokenKind::LBrace)?;

    if matches!(ts.peek_kind(), Some(TokenKind::RBrace)) {
        ts.bump();
        return Ok(pat_from(ts, start, ast::PatternKind::Record(vec![])));
    }

    let mut strict_fields: Vec<(String, ast::Pattern)> = Vec::new();
    let mut loose_fields: Vec<(String, ast::Pattern)> = Vec::new();
    let mut is_loose = false;
    let mut rest: Option<String> = None;

    loop {
        // Rest marker: `{...}` or `{..., ...rest}`.
        if matches!(ts.peek_kind(), Some(TokenKind::Ellipsis)) {
            is_loose = true;
            ts.bump();
            if let Some(TokenKind::Ident(name)) = ts.peek_kind() {
                let name = name.clone();
                ts.bump();
                rest = Some(name);
            }
            ts.expect(TokenKind::RBrace)?;
            break;
        }

        let name = parse_maybe_qualified_ident(ts)?;
        match ts.peek_kind() {
            Some(TokenKind::Eq) => {
                ts.bump();
                let pat = parse_pattern(ts)?;
                strict_fields.push((name, pat));
            }
            Some(TokenKind::Colon) => {
                ts.bump();
                let pat = parse_pattern(ts)?;
                // `:` is the "pattern binding" form used by loose record patterns.
                // We store it separately and decide later whether this becomes
                // `Record` or `RecordLoose`.
                loose_fields.push((name, pat));
            }
            _ => return Err(ts.err_here("expected '=' or ':' in record pattern")),
        }

        match ts.peek_kind() {
            Some(TokenKind::Comma) => {
                ts.bump();
                continue;
            }
            Some(TokenKind::RBrace) => {
                ts.bump();
                break;
            }
            _ => return Err(ts.err_here("expected ',' or '}' in record pattern")),
        }
    }

    if is_loose {
        // `{x: a, ...}` and `{x: a, ...r}` are loose record patterns.
        Ok(pat_from(ts, start, ast::PatternKind::RecordLoose(loose_fields, rest)))
    } else {
        // In strict record patterns, tests allow `:` interchangeably with `=`.
        if !loose_fields.is_empty() {
            strict_fields.extend(loose_fields);
        }
        Ok(pat_from(ts, start, ast::PatternKind::Record(strict_fields)))
    }
}
