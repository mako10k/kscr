use crate::{ast, lexer::TokenKind, Result};

use crate::parser::token_stream::TokenStream;

use super::{expr_from, last_qualified_segment, parse_maybe_qualified_ident, Stop};

fn is_pred_end(kind: Option<&TokenKind>, _stop: Stop) -> bool {
    matches!(
        kind,
        None | Some(TokenKind::Newline)
            | Some(TokenKind::Comma)
            | Some(TokenKind::RParen)
            | Some(TokenKind::FatArrow)
            | Some(TokenKind::Dedent)
    )
}

pub(crate) fn parse_predicate_in_root(ts: &mut TokenStream, stop: Stop) -> Result<ast::Predicate> {
    parse_predicate(ts, stop)
}

fn parse_predicate(ts: &mut TokenStream, stop: Stop) -> Result<ast::Predicate> {
    let name = ts.expect_ident()?;
    match name.as_str() {
        "Show" => Ok(ast::Predicate::Show(parse_type_expr(
            ts,
            stop,
            is_pred_end,
        )?)),
        "ShowRow" => Ok(ast::Predicate::ShowRow(parse_type_expr(
            ts,
            stop,
            is_pred_end,
        )?)),
        "Eq" => Ok(ast::Predicate::Eq(parse_type_expr(ts, stop, is_pred_end)?)),
        "EqRow" => Ok(ast::Predicate::EqRow(parse_type_expr(
            ts,
            stop,
            is_pred_end,
        )?)),
        "Lacks" => {
            let label = match ts.bump() {
                Some(TokenKind::String(s)) => s,
                _ => return Err(ts.err_here("expected string literal after Lacks")),
            };
            let row = parse_type_expr(ts, stop, is_pred_end)?;
            Ok(ast::Predicate::Lacks { label, row })
        }
        other => Ok(ast::Predicate::Class {
            class: other.to_string(),
            ty: parse_type_expr(ts, stop, is_pred_end)?,
        }),
    }
}

fn is_class_super_parens(ts: &TokenStream) -> bool {
    if !matches!(ts.peek_kind(), Some(TokenKind::LParen)) {
        return false;
    }

    let mut depth: i32 = 0;
    let mut j = ts.i;
    while let Some(tok) = ts.tokens.get(j) {
        match &tok.kind {
            TokenKind::LParen => depth += 1,
            TokenKind::RParen => {
                depth -= 1;
                if depth == 0 {
                    return matches!(
                        ts.tokens.get(j + 1).map(|t| &t.kind),
                        Some(TokenKind::FatArrow)
                    );
                }
            }
            _ => {}
        }
        j += 1;
    }
    false
}

pub(crate) fn is_class_super_parens_in_root(ts: &TokenStream) -> bool {
    is_class_super_parens(ts)
}

pub(crate) fn parse_qual_type(ts: &mut TokenStream, stop: Stop) -> Result<ast::QualType> {
    // (p1, p2, ...) => T
    // We only treat parentheses as predicate groups when they are followed by `=>`.
    let is_qual_parens = is_class_super_parens(ts);

    if is_qual_parens {
        ts.bump();
        let mut preds = Vec::new();
        preds.push(parse_predicate(ts, stop)?);
        while matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
            ts.bump();
            preds.push(parse_predicate(ts, stop)?);
        }
        ts.expect(TokenKind::RParen)?;
        ts.expect(TokenKind::FatArrow)?;
        let ty = parse_type_expr(ts, stop, is_type_end)?;
        return Ok(ast::QualType { preds, ty });
    }

    // p => T (single predicate)
    {
        let save2 = ts.i;
        if let Ok(pred) = parse_predicate(ts, stop) {
            if matches!(ts.peek_kind(), Some(TokenKind::FatArrow)) {
                ts.bump();
                let ty = parse_type_expr(ts, stop, is_type_end)?;
                return Ok(ast::QualType {
                    preds: vec![pred],
                    ty,
                });
            }
        }
        ts.i = save2;
    }

    // Just a type.
    let ty = parse_type_expr(ts, stop, is_type_end)?;
    Ok(ast::QualType { preds: vec![], ty })
}

fn is_type_end(kind: Option<&TokenKind>, stop: Stop) -> bool {
    match kind {
        None
        | Some(TokenKind::Newline)
        | Some(TokenKind::Comma)
        | Some(TokenKind::RParen)
        | Some(TokenKind::FatArrow)
        | Some(TokenKind::RBracket)
        | Some(TokenKind::RBrace)
        | Some(TokenKind::Dedent)
        | Some(TokenKind::ColonColon)
        | Some(TokenKind::KwWhere) => true,
        Some(TokenKind::KwThen) if matches!(stop, Stop::Then) => true,
        Some(TokenKind::KwElse) if matches!(stop, Stop::Else) => true,
        Some(TokenKind::KwOf) if matches!(stop, Stop::Of) => true,
        _ => false,
    }
}

fn is_type_alias_end(kind: Option<&TokenKind>, _stop: Stop) -> bool {
    matches!(kind, None | Some(TokenKind::Newline))
}

fn parse_type_expr(
    ts: &mut TokenStream,
    stop: Stop,
    end: fn(Option<&TokenKind>, Stop) -> bool,
) -> Result<ast::Type> {
    parse_type_func(ts, stop, end)
}

pub(crate) fn parse_type_expr_in_root(
    ts: &mut TokenStream,
    stop: Stop,
    end: fn(Option<&TokenKind>, Stop) -> bool,
) -> Result<ast::Type> {
    parse_type_expr(ts, stop, end)
}

fn parse_type_func(
    ts: &mut TokenStream,
    stop: Stop,
    end: fn(Option<&TokenKind>, Stop) -> bool,
) -> Result<ast::Type> {
    let lhs = parse_type_app(ts, stop, end)?;
    if matches!(ts.peek_kind(), Some(TokenKind::Arrow)) && !end(ts.peek_kind(), stop) {
        ts.bump();
        let rhs = parse_type_func(ts, stop, end)?;
        Ok(ast::Type::Func(Box::new(lhs), Box::new(rhs)))
    } else {
        Ok(lhs)
    }
}

fn parse_type_app(
    ts: &mut TokenStream,
    stop: Stop,
    end: fn(Option<&TokenKind>, Stop) -> bool,
) -> Result<ast::Type> {
    let head = parse_type_atom(ts, stop, end)?;

    let mut args = Vec::new();
    while !end(ts.peek_kind(), stop) && is_type_atom_start(ts.peek_kind()) {
        args.push(parse_type_atom(ts, stop, end)?);
    }

    if args.is_empty() {
        Ok(head)
    } else {
        Ok(ast::Type::App {
            head: Box::new(head),
            args,
        })
    }
}

fn is_type_atom_start(kind: Option<&TokenKind>) -> bool {
    matches!(
        kind,
        Some(TokenKind::Ident(_))
            | Some(TokenKind::LParen)
            | Some(TokenKind::LBracket)
            | Some(TokenKind::LBrace)
            | Some(TokenKind::Question)
    )
}

fn parse_type_atom(
    ts: &mut TokenStream,
    stop: Stop,
    _end: fn(Option<&TokenKind>, Stop) -> bool,
) -> Result<ast::Type> {
    match ts.peek_kind() {
        Some(TokenKind::Question) => {
            ts.bump();
            let name = if matches!(ts.peek_kind(), Some(TokenKind::Ident(_))) {
                Some(ts.expect_ident()?)
            } else {
                None
            };
            Ok(ast::Type::Hole(name))
        }
        Some(TokenKind::Ident(_)) => {
            let s = parse_maybe_qualified_ident(ts)?;
            let last = last_qualified_segment(&s);
            Ok(match last {
                "Integer" => ast::Type::Integer,
                "Bool" => ast::Type::Bool,
                "Float64" => ast::Type::Float64,
                "Char" => ast::Type::Char,
                _ => ast::Type::Var(s),
            })
        }
        Some(TokenKind::LParen) => {
            ts.expect(TokenKind::LParen)?;
            if matches!(ts.peek_kind(), Some(TokenKind::RParen)) {
                ts.bump();
                return Ok(ast::Type::Unit);
            }

            let first = parse_type_expr(ts, stop, is_paren_type_end)?;
            if matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
                let mut elems = vec![first];
                while matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
                    ts.bump();
                    elems.push(parse_type_expr(ts, stop, is_paren_type_end)?);
                }
                ts.expect(TokenKind::RParen)?;
                Ok(ast::Type::Tuple(elems))
            } else {
                ts.expect(TokenKind::RParen)?;
                Ok(first)
            }
        }
        Some(TokenKind::LBracket) => {
            ts.expect(TokenKind::LBracket)?;
            let elem = parse_type_expr(ts, stop, is_bracket_type_end)?;
            ts.expect(TokenKind::RBracket)?;
            Ok(ast::Type::List(Box::new(elem)))
        }
        Some(TokenKind::LBrace) => {
            ts.expect(TokenKind::LBrace)?;
            if matches!(ts.peek_kind(), Some(TokenKind::RBrace)) {
                ts.bump();
                return Ok(ast::Type::Record(Vec::new()));
            }

            let mut fields = Vec::new();
            let name = ts.expect_ident()?;
            ts.expect(TokenKind::Colon)?;
            let ty = parse_type_expr(ts, stop, is_record_field_type_end)?;
            fields.push((name, ty));

            let mut rest: Option<ast::Type> = None;
            while matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
                ts.bump();

                if matches!(ts.peek_kind(), Some(TokenKind::Ellipsis)) {
                    ts.bump();
                    rest = Some(if matches!(ts.peek_kind(), Some(TokenKind::Ident(_))) {
                        ast::Type::Var(ts.expect_ident()?)
                    } else {
                        ast::Type::Var(ts.fresh_name("r"))
                    });
                    break;
                }

                let name = ts.expect_ident()?;
                ts.expect(TokenKind::Colon)?;
                let ty = parse_type_expr(ts, stop, is_record_field_type_end)?;
                fields.push((name, ty));
            }

            ts.expect(TokenKind::RBrace)?;
            Ok(match rest {
                Some(r) => ast::Type::RecordOpen(fields, Box::new(r)),
                None => ast::Type::Record(fields),
            })
        }
        _ => Err(ts.err_here("expected type")),
    }
}

pub(crate) fn parse_type_atom_in_root(
    ts: &mut TokenStream,
    stop: Stop,
    end: fn(Option<&TokenKind>, Stop) -> bool,
) -> Result<ast::Type> {
    parse_type_atom(ts, stop, end)
}

pub(crate) fn is_type_end_in_root(kind: Option<&TokenKind>, stop: Stop) -> bool {
    is_type_end(kind, stop)
}

fn is_paren_type_end(kind: Option<&TokenKind>, _stop: Stop) -> bool {
    matches!(kind, Some(TokenKind::Comma) | Some(TokenKind::RParen))
}

fn is_bracket_type_end(kind: Option<&TokenKind>, _stop: Stop) -> bool {
    matches!(kind, Some(TokenKind::RBracket))
}

fn is_record_field_type_end(kind: Option<&TokenKind>, _stop: Stop) -> bool {
    matches!(kind, Some(TokenKind::Comma) | Some(TokenKind::RBrace))
}

pub(crate) fn parse_annot(ts: &mut TokenStream, expr: ast::Expr, stop: Stop) -> Result<ast::Expr> {
    let start = expr.span.start;
    ts.expect(TokenKind::ColonColon)?;

    let ty = parse_qual_type(ts, stop)?;

    Ok(expr_from(
        ts,
        start,
        ast::ExprKind::Annot {
            expr: Box::new(expr),
            ty,
        },
    ))
}

pub(crate) fn is_type_alias_end_public(kind: Option<&TokenKind>, stop: Stop) -> bool {
    is_type_alias_end(kind, stop)
}
