use crate::{ast, error::Error, lexer, lexer::TokenKind, Result};

fn parse_maybe_qualified_ident(ts: &mut TokenStream) -> Result<String> {
    let mut s = ts.expect_ident()?;
    while matches!(ts.peek_kind(), Some(TokenKind::Dot)) {
        ts.bump();
        s.push('.');
        s.push_str(&ts.expect_ident()?);
    }
    Ok(s)
}

fn last_qualified_segment(s: &str) -> &str {
    s.rsplit('.').next().unwrap_or(s)
}

fn is_upper_by_last_segment(s: &str) -> bool {
    last_qualified_segment(s)
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_uppercase())
}

pub fn parse_module(src: &str) -> Result<ast::Module> {
    let tokens = lexer::lex(src)?;
    let mut ts = TokenStream::new(tokens);

    ts.skip_newlines();

    if matches!(ts.peek_kind(), Some(TokenKind::KwModule)) {
        parse_module_decl(&mut ts)
    } else {
        let items = parse_items_until(&mut ts, StopAt::Eof)?;
        Ok(ast::Module { name: None, items })
    }
}

fn parse_module_decl(ts: &mut TokenStream) -> Result<ast::Module> {
    ts.expect(TokenKind::KwModule)?;
    let name = ts.expect_ident()?;
    ts.expect(TokenKind::KwWhere)?;
    ts.consume_line_end();
    ts.skip_newlines();
    ts.expect(TokenKind::Indent)?;

    let items = parse_items_until(ts, StopAt::Dedent)?;

    ts.expect(TokenKind::Dedent)?;
    ts.consume_line_end();

    Ok(ast::Module {
        name: Some(name),
        items,
    })
}

#[derive(Clone, Copy)]
enum StopAt {
    Dedent,
    Eof,
}

fn parse_items_until(ts: &mut TokenStream, stop_at: StopAt) -> Result<Vec<ast::Item>> {
    let mut items = Vec::new();
    loop {
        ts.skip_newlines();
        if ts.is_eof() {
            break;
        }
        if matches!(stop_at, StopAt::Dedent) && matches!(ts.peek_kind(), Some(TokenKind::Dedent)) {
            break;
        }

        let item = match ts.peek_kind() {
            Some(TokenKind::KwImport) => parse_import_decl(ts)?,
            Some(TokenKind::KwExport) => parse_export_decl(ts)?,
            Some(TokenKind::KwData) => parse_data_decl(ts)?,
            Some(TokenKind::KwType) => parse_type_alias(ts)?,
            Some(
                TokenKind::Ident(_)
                | TokenKind::LParen
                | TokenKind::LBracket
                | TokenKind::LBrace
                | TokenKind::Integer(_)
                | TokenKind::Float(_)
                | TokenKind::String(_)
                | TokenKind::Char(_)
                | TokenKind::True
                | TokenKind::False
                | TokenKind::Question,
            ) => parse_binding(ts)?,
            Some(_) => return Err(Error::msg("unexpected token at top-level")),
            None => break,
        };
        items.push(item);
        ts.consume_line_end();

        if matches!(stop_at, StopAt::Eof) && ts.is_eof() {
            break;
        }
    }
    Ok(items)
}

fn parse_data_decl(ts: &mut TokenStream) -> Result<ast::Item> {
    ts.expect(TokenKind::KwData)?;
    let name = ts.expect_ident()?;

    let mut params = Vec::new();
    while matches!(ts.peek_kind(), Some(TokenKind::Ident(_))) {
        params.push(ts.expect_ident()?);
    }

    ts.expect(TokenKind::Eq)?;

    let mut ctors = Vec::new();
    loop {
        let ctor_name = ts.expect_ident()?;
        let mut args = Vec::new();
        while matches!(
            ts.peek_kind(),
            Some(TokenKind::Ident(_))
                | Some(TokenKind::LParen)
                | Some(TokenKind::LBracket)
                | Some(TokenKind::LBrace)
        ) {
            // Atom-level type parsing for ctor args.
            args.push(parse_type_atom(ts, Stop::LineEnd, is_type_alias_end)?);
        }
        ctors.push(ast::DataCtor {
            name: ctor_name,
            args,
        });

        match ts.peek_kind() {
            Some(TokenKind::Pipe) => {
                ts.bump();
            }
            Some(TokenKind::Newline) | None => break,
            _ => break,
        }
    }

    Ok(ast::Item::DataDecl(ast::DataDecl {
        name,
        params,
        ctors,
    }))
}

fn parse_type_alias(ts: &mut TokenStream) -> Result<ast::Item> {
    ts.expect(TokenKind::KwType)?;
    let name = ts.expect_ident()?;

    let mut params = Vec::new();
    while matches!(ts.peek_kind(), Some(TokenKind::Ident(_))) {
        params.push(ts.expect_ident()?);
    }

    ts.expect(TokenKind::Eq)?;

    let ty = parse_type_expr(ts, Stop::LineEnd, is_type_alias_end)?;

    Ok(ast::Item::TypeAlias(ast::TypeAlias { name, params, ty }))
}

fn parse_import_decl(ts: &mut TokenStream) -> Result<ast::Item> {
    ts.expect(TokenKind::KwImport)?;
    let module = ts.expect_ident()?;

    let as_name = match ts.peek_kind() {
        Some(TokenKind::Ident(s)) if s == "as" => {
            ts.bump();
            Some(ts.expect_ident()?)
        }
        _ => None,
    };

    Ok(ast::Item::Import(ast::ImportDecl { module, as_name }))
}

fn parse_export_decl(ts: &mut TokenStream) -> Result<ast::Item> {
    ts.expect(TokenKind::KwExport)?;

    let mut names = Vec::new();
    names.push(ts.expect_ident()?);
    while matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
        ts.bump();
        names.push(ts.expect_ident()?);
    }

    Ok(ast::Item::Export(ast::ExportDecl { names }))
}

fn parse_binding(ts: &mut TokenStream) -> Result<ast::Item> {
    let pat = parse_pattern(ts)?;
    ts.expect(TokenKind::Eq)?;
    let expr = parse_expr(ts, Stop::LineEnd)?;
    Ok(ast::Item::Binding(ast::Binding { pat, expr }))
}

#[derive(Clone, Copy)]
enum Stop {
    LineEnd,
    Then,
    Else,
    In,
    Of,
    Pattern,
}

fn parse_expr(ts: &mut TokenStream, stop: Stop) -> Result<ast::Expr> {
    let mut expr = match ts.peek_kind() {
        Some(TokenKind::Backslash) => parse_lambda(ts, stop)?,
        Some(TokenKind::KwIf) => parse_if(ts, stop)?,
        Some(TokenKind::KwLet) => parse_let(ts, stop)?,
        Some(TokenKind::KwCase) => parse_case(ts, stop)?,
        Some(TokenKind::KwDo) => parse_do(ts, stop)?,
        _ => parse_infix_application(ts, stop)?,
    };

    while let Some(TokenKind::ColonColon) = ts.peek_kind() {
        expr = parse_annot(ts, expr, stop)?;
    }

    while let Some(TokenKind::KwWhere) = ts.peek_kind() {
        expr = parse_where(ts, expr)?;
    }

    Ok(expr)
}

fn parse_lambda(ts: &mut TokenStream, stop: Stop) -> Result<ast::Expr> {
    ts.expect(TokenKind::Backslash)?;
    let mut params = Vec::new();
    while matches!(ts.peek_kind(), Some(TokenKind::Ident(_))) {
        params.push(ts.expect_ident()?);
    }
    if params.is_empty() {
        return Err(Error::msg("expected lambda parameter"));
    }
    ts.expect(TokenKind::Arrow)?;
    let body = Box::new(parse_expr(ts, stop)?);
    Ok(ast::Expr::Lambda { params, body })
}

fn parse_if(ts: &mut TokenStream, stop: Stop) -> Result<ast::Expr> {
    ts.expect(TokenKind::KwIf)?;
    let cond = Box::new(parse_expr(ts, Stop::Then)?);
    ts.expect(TokenKind::KwThen)?;
    let then_branch = Box::new(parse_expr(ts, Stop::Else)?);
    ts.expect(TokenKind::KwElse)?;
    let else_branch = Box::new(parse_expr(ts, stop)?);
    Ok(ast::Expr::If {
        cond,
        then_branch,
        else_branch,
    })
}

fn parse_let(ts: &mut TokenStream, stop: Stop) -> Result<ast::Expr> {
    ts.expect(TokenKind::KwLet)?;

    let bindings = if matches!(ts.peek_kind(), Some(TokenKind::Newline)) {
        ts.consume_line_end();
        ts.skip_newlines();
        ts.expect(TokenKind::Indent)?;

        let mut bs = Vec::new();
        loop {
            ts.skip_newlines();
            if matches!(ts.peek_kind(), Some(TokenKind::Dedent)) {
                break;
            }
            if ts.is_eof() {
                return Err(Error::msg("unexpected EOF in let"));
            }
            bs.push(parse_let_binding_line(ts)?);
            ts.consume_line_end();
        }

        ts.expect(TokenKind::Dedent)?;
        ts.consume_line_end();
        bs
    } else {
        vec![parse_let_binding_inline(ts)?]
    };

    ts.expect(TokenKind::KwIn)?;
    let body = Box::new(parse_expr(ts, stop)?);
    Ok(ast::Expr::Let { bindings, body })
}

fn parse_let_binding_line(ts: &mut TokenStream) -> Result<ast::Binding> {
    let pat = parse_pattern(ts)?;
    ts.expect(TokenKind::Eq)?;
    let expr = parse_expr(ts, Stop::LineEnd)?;
    Ok(ast::Binding { pat, expr })
}

fn parse_let_binding_inline(ts: &mut TokenStream) -> Result<ast::Binding> {
    let pat = parse_pattern(ts)?;
    ts.expect(TokenKind::Eq)?;
    let expr = parse_expr(ts, Stop::In)?;
    Ok(ast::Binding { pat, expr })
}

fn parse_do(ts: &mut TokenStream, _stop: Stop) -> Result<ast::Expr> {
    ts.expect(TokenKind::KwDo)?;

    if !matches!(ts.peek_kind(), Some(TokenKind::Newline)) {
        return Err(Error::msg("expected newline after 'do'"));
    }

    ts.consume_line_end();
    ts.skip_newlines();
    ts.expect(TokenKind::Indent)?;

    let mut stmts = Vec::new();
    loop {
        ts.skip_newlines();
        if matches!(ts.peek_kind(), Some(TokenKind::Dedent)) {
            break;
        }
        if ts.is_eof() {
            return Err(Error::msg("unexpected EOF in do"));
        }

        // Bind statement is `pat <- expr`.
        let save = ts.i;
        if let Ok(pat) = parse_pattern(ts) {
            if matches!(ts.peek_kind(), Some(TokenKind::LeftArrow)) {
                ts.bump();
                let expr = parse_expr(ts, Stop::LineEnd)?;
                stmts.push(ast::DoStmt::Bind { pat, expr });
                ts.consume_line_end();
                continue;
            }
        }
        ts.i = save;

        let expr = parse_expr(ts, Stop::LineEnd)?;
        stmts.push(ast::DoStmt::Expr(expr));
        ts.consume_line_end();
    }

    ts.expect(TokenKind::Dedent)?;
    ts.consume_line_end();

    Ok(ast::Expr::Do(stmts))
}

fn parse_case(ts: &mut TokenStream, _stop: Stop) -> Result<ast::Expr> {
    ts.expect(TokenKind::KwCase)?;
    let expr = Box::new(parse_expr(ts, Stop::Of)?);
    ts.expect(TokenKind::KwOf)?;

    if !matches!(ts.peek_kind(), Some(TokenKind::Newline)) {
        return Err(Error::msg("expected newline after 'of'"));
    }

    ts.consume_line_end();
    ts.skip_newlines();
    ts.expect(TokenKind::Indent)?;

    let mut arms = Vec::new();
    loop {
        ts.skip_newlines();
        if matches!(ts.peek_kind(), Some(TokenKind::Dedent)) {
            break;
        }
        if ts.is_eof() {
            return Err(Error::msg("unexpected EOF in case"));
        }

        let mut pat = parse_cons_pattern(ts)?;

        // Disambiguation: prefer `or-pattern` when `| <pattern> ->` is possible;
        // otherwise treat it as a case guard `| <expr> ->`.
        while matches!(ts.peek_kind(), Some(TokenKind::Pipe)) {
            let save = ts.i;
            ts.bump();
            if let Ok(rhs) = parse_cons_pattern(ts) {
                if matches!(ts.peek_kind(), Some(TokenKind::Arrow)) {
                    pat = ast::Pattern::Or(Box::new(pat), Box::new(rhs));
                    continue;
                }
            }
            ts.i = save;
            break;
        }

        let guard = if matches!(ts.peek_kind(), Some(TokenKind::Pipe)) {
            ts.bump();
            Some(parse_expr(ts, Stop::Pattern)?)
        } else {
            None
        };

        ts.expect(TokenKind::Arrow)?;
        let body = parse_expr(ts, Stop::LineEnd)?;
        arms.push(ast::CaseArm { pat, guard, body });
        ts.consume_line_end();
    }

    ts.expect(TokenKind::Dedent)?;
    ts.consume_line_end();

    Ok(ast::Expr::Case { expr, arms })
}

fn parse_annot(ts: &mut TokenStream, expr: ast::Expr, stop: Stop) -> Result<ast::Expr> {
    ts.expect(TokenKind::ColonColon)?;

    let ty = parse_qual_type(ts, stop)?;

    Ok(ast::Expr::Annot {
        expr: Box::new(expr),
        ty,
    })
}

fn is_pred_end(kind: Option<&TokenKind>, _stop: Stop) -> bool {
    matches!(
        kind,
        None
            | Some(TokenKind::Newline)
            | Some(TokenKind::Comma)
            | Some(TokenKind::RParen)
            | Some(TokenKind::FatArrow)
            | Some(TokenKind::Dedent)
    )
}

fn parse_predicate(ts: &mut TokenStream, stop: Stop) -> Result<ast::Predicate> {
    let name = ts.expect_ident()?;
    match name.as_str() {
        "Show" => Ok(ast::Predicate::Show(parse_type_expr(ts, stop, is_pred_end)?)),
        "ShowRow" => Ok(ast::Predicate::ShowRow(parse_type_expr(ts, stop, is_pred_end)?)),
        "Lacks" => {
            let label = match ts.bump() {
                Some(TokenKind::String(s)) => s,
                _ => return Err(Error::msg("expected string literal after Lacks")),
            };
            let row = parse_type_expr(ts, stop, is_pred_end)?;
            Ok(ast::Predicate::Lacks { label, row })
        }
        _ => Err(Error::msg("unknown constraint predicate")),
    }
}

fn parse_qual_type(ts: &mut TokenStream, stop: Stop) -> Result<ast::QualType> {
    // (p1, p2, ...) => T
    // We only treat parentheses as predicate groups when they are followed by `=>`.
    let is_qual_parens = if matches!(ts.peek_kind(), Some(TokenKind::LParen)) {
        let mut depth: i32 = 0;
        let mut j = ts.i;
        let mut ok = false;
        while let Some(tok) = ts.tokens.get(j) {
            match &tok.kind {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        ok = matches!(ts.tokens.get(j + 1).map(|t| &t.kind), Some(TokenKind::FatArrow));
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        ok
    } else {
        false
    };

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
        Some(TokenKind::KwIn) if matches!(stop, Stop::In) => true,
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
                "String" => ast::Type::String,
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
        _ => Err(Error::msg("expected type")),
    }
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

fn parse_where(ts: &mut TokenStream, expr: ast::Expr) -> Result<ast::Expr> {
    ts.expect(TokenKind::KwWhere)?;

    if !matches!(ts.peek_kind(), Some(TokenKind::Newline)) {
        return Err(Error::msg("expected newline after 'where'"));
    }

    ts.consume_line_end();
    ts.skip_newlines();
    ts.expect(TokenKind::Indent)?;

    let mut bindings = Vec::new();
    loop {
        ts.skip_newlines();
        if matches!(ts.peek_kind(), Some(TokenKind::Dedent)) {
            break;
        }
        if ts.is_eof() {
            return Err(Error::msg("unexpected EOF in where"));
        }
        bindings.push(parse_let_binding_line(ts)?);
        ts.consume_line_end();
    }

    ts.expect(TokenKind::Dedent)?;
    ts.consume_line_end();

    Ok(ast::Expr::Where {
        expr: Box::new(expr),
        bindings,
    })
}

fn parse_pattern(ts: &mut TokenStream) -> Result<ast::Pattern> {
    parse_or_pattern(ts)
}

fn parse_or_pattern(ts: &mut TokenStream) -> Result<ast::Pattern> {
    let mut pat = parse_cons_pattern(ts)?;
    while matches!(ts.peek_kind(), Some(TokenKind::Pipe)) {
        ts.bump();
        let rhs = parse_cons_pattern(ts)?;
        pat = ast::Pattern::Or(Box::new(pat), Box::new(rhs));
    }
    Ok(pat)
}

fn parse_cons_pattern(ts: &mut TokenStream) -> Result<ast::Pattern> {
    let pat = parse_app_pattern(ts)?;

    // Cons pattern: x : xs (right-associative)
    if matches!(ts.peek_kind(), Some(TokenKind::Colon)) {
        ts.bump();
        let tail = parse_cons_pattern(ts)?;
        return Ok(ast::Pattern::Cons(Box::new(pat), Box::new(tail)));
    }

    Ok(pat)
}

fn parse_app_pattern(ts: &mut TokenStream) -> Result<ast::Pattern> {
    let mut pat = parse_pattern_atom(ts)?;

    // As-pattern: x @ pat
    if let ast::Pattern::Var(name) = &pat {
        if matches!(ts.peek_kind(), Some(TokenKind::At)) {
            let name = name.clone();
            ts.bump();
            let inner = parse_pattern(ts)?;
            pat = ast::Pattern::As(name, Box::new(inner));
        }
    }

    // Constructor application: Just x y
    if let ast::Pattern::Constructor { name, mut args } = pat {
        while ts.can_continue_pattern() {
            args.push(parse_pattern_atom(ts)?);
        }
        pat = ast::Pattern::Constructor { name, args };
    }

    Ok(pat)
}

fn parse_pattern_atom(ts: &mut TokenStream) -> Result<ast::Pattern> {
    match ts.peek_kind() {
        Some(TokenKind::LParen) => parse_paren_or_tuple_pattern(ts),
        Some(TokenKind::LBracket) => parse_list_pattern(ts),
        Some(TokenKind::LBrace) => parse_record_pattern(ts),

        Some(TokenKind::Ident(s)) if s == "_" => {
            ts.bump();
            Ok(ast::Pattern::Wildcard)
        }
        Some(TokenKind::Question) => {
            ts.bump();
            let name = match ts.peek_kind() {
                Some(TokenKind::Ident(_)) => Some(ts.expect_ident()?),
                _ => None,
            };
            Ok(ast::Pattern::Hole(name))
        }
        Some(TokenKind::Ident(_)) => {
            let s = parse_maybe_qualified_ident(ts)?;
            if is_upper_by_last_segment(&s) {
                Ok(ast::Pattern::Constructor {
                    name: s,
                    args: vec![],
                })
            } else {
                Ok(ast::Pattern::Var(s))
            }
        }

        Some(TokenKind::True) => {
            ts.bump();
            Ok(ast::Pattern::Literal(ast::Expr::Bool(true)))
        }
        Some(TokenKind::False) => {
            ts.bump();
            Ok(ast::Pattern::Literal(ast::Expr::Bool(false)))
        }
        Some(TokenKind::Integer(_)) => match ts.bump() {
            Some(TokenKind::Integer(s)) => Ok(ast::Pattern::Literal(ast::Expr::Integer(s))),
            _ => unreachable!(),
        },
        Some(TokenKind::Float(_)) => match ts.bump() {
            Some(TokenKind::Float(s)) => Ok(ast::Pattern::Literal(ast::Expr::Float64(s))),
            _ => unreachable!(),
        },
        Some(TokenKind::String(_)) => match ts.bump() {
            Some(TokenKind::String(s)) => Ok(ast::Pattern::Literal(ast::Expr::String(s))),
            _ => unreachable!(),
        },
        Some(TokenKind::Char(_)) => match ts.bump() {
            Some(TokenKind::Char(ch)) => Ok(ast::Pattern::Literal(ast::Expr::Char(ch))),
            _ => unreachable!(),
        },

        _ => Err(Error::msg("expected pattern")),
    }
}

fn parse_paren_or_tuple_pattern(ts: &mut TokenStream) -> Result<ast::Pattern> {
    ts.expect(TokenKind::LParen)?;

    if matches!(ts.peek_kind(), Some(TokenKind::RParen)) {
        ts.bump();
        return Ok(ast::Pattern::Literal(ast::Expr::Unit));
    }

    let first = parse_pattern(ts)?;

    // View pattern must be parenthesized: (pat <- expr)
    if matches!(ts.peek_kind(), Some(TokenKind::LeftArrow)) {
        ts.bump();
        let expr = parse_expr(ts, Stop::Pattern)?;
        ts.expect(TokenKind::RParen)?;
        return Ok(ast::Pattern::View(Box::new(first), Box::new(expr)));
    }

    if matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
        let mut elems = vec![first];
        while matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
            ts.bump();
            elems.push(parse_pattern(ts)?);
        }
        ts.expect(TokenKind::RParen)?;
        Ok(ast::Pattern::Tuple(elems))
    } else {
        ts.expect(TokenKind::RParen)?;
        Ok(first)
    }
}

fn parse_list_pattern(ts: &mut TokenStream) -> Result<ast::Pattern> {
    ts.expect(TokenKind::LBracket)?;

    if matches!(ts.peek_kind(), Some(TokenKind::RBracket)) {
        ts.bump();
        return Ok(ast::Pattern::List(Vec::new()));
    }

    let mut elems = Vec::new();
    elems.push(parse_pattern(ts)?);
    while matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
        ts.bump();
        elems.push(parse_pattern(ts)?);
    }

    ts.expect(TokenKind::RBracket)?;
    Ok(ast::Pattern::List(elems))
}

fn parse_record_pattern(ts: &mut TokenStream) -> Result<ast::Pattern> {
    ts.expect(TokenKind::LBrace)?;

    if matches!(ts.peek_kind(), Some(TokenKind::RBrace)) {
        ts.bump();
        return Ok(ast::Pattern::Record(Vec::new()));
    }

    let mut fields = Vec::new();
    let name = ts.expect_ident()?;
    ts.expect(TokenKind::Colon)?;
    let pat = parse_pattern(ts)?;
    fields.push((name, pat));

    let mut loose = false;
    let mut rest: Option<String> = None;
    while matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
        ts.bump();
        if matches!(ts.peek_kind(), Some(TokenKind::Ellipsis)) {
            ts.bump();
            loose = true;
            if matches!(ts.peek_kind(), Some(TokenKind::Ident(_))) {
                let n = ts.expect_ident()?;
                if n.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                    return Err(Error::msg("...rest must be a variable"));
                }
                rest = Some(n);
            }
            break;
        }

        let name = ts.expect_ident()?;
        ts.expect(TokenKind::Colon)?;
        let pat = parse_pattern(ts)?;
        fields.push((name, pat));
    }

    ts.expect(TokenKind::RBrace)?;
    Ok(if loose {
        ast::Pattern::RecordLoose(fields, rest)
    } else {
        ast::Pattern::Record(fields)
    })
}

fn parse_infix_application(ts: &mut TokenStream, stop: Stop) -> Result<ast::Expr> {
    parse_binops(ts, stop, 0)
}

fn parse_binops(ts: &mut TokenStream, stop: Stop, min_prec: u8) -> Result<ast::Expr> {
    let mut lhs = parse_application(ts, stop)?;

    while ts.can_continue_expr(stop) {
        let is_cons = matches!(ts.peek_kind(), Some(TokenKind::Colon));
        let (prec, is_backtick) = match ts.peek_kind() {
            Some(TokenKind::Backtick) => (60u8, true),
            Some(TokenKind::Star) | Some(TokenKind::Slash) => (70u8, false),
            Some(TokenKind::Plus) | Some(TokenKind::Minus) => (60u8, false),
            Some(TokenKind::Colon) => (55u8, false),
            Some(TokenKind::EqEq)
            | Some(TokenKind::SlashEq)
            | Some(TokenKind::Lt)
            | Some(TokenKind::Le)
            | Some(TokenKind::Gt)
            | Some(TokenKind::Ge) => (50u8, false),
            Some(TokenKind::AndAnd) => (40u8, false),
            Some(TokenKind::OrOr) => (30u8, false),
            _ => break,
        };

        if prec < min_prec {
            break;
        }

        let op = if is_backtick {
            ts.expect(TokenKind::Backtick)?;
            let op = ts.expect_ident()?;
            ts.expect(TokenKind::Backtick)?;
            op
        } else {
            match ts.bump() {
                Some(TokenKind::Plus) => "+".to_string(),
                Some(TokenKind::Minus) => "-".to_string(),
                Some(TokenKind::Star) => "*".to_string(),
                Some(TokenKind::Slash) => "/".to_string(),
                Some(TokenKind::Colon) => ":".to_string(),
                Some(TokenKind::EqEq) => "==".to_string(),
                Some(TokenKind::SlashEq) => "/=".to_string(),
                Some(TokenKind::Lt) => "<".to_string(),
                Some(TokenKind::Le) => "<=".to_string(),
                Some(TokenKind::Gt) => ">".to_string(),
                Some(TokenKind::Ge) => ">=".to_string(),
                Some(TokenKind::AndAnd) => "&&".to_string(),
                Some(TokenKind::OrOr) => "||".to_string(),
                _ => unreachable!(),
            }
        };

        let rhs = parse_binops(ts, stop, if is_cons { prec } else { prec + 1 })?;
        lhs = if is_cons {
            ast::Expr::Cons {
                head: Box::new(lhs),
                tail: Box::new(rhs),
            }
        } else {
            ast::Expr::Apply {
                func: Box::new(ast::Expr::Var(op)),
                args: vec![lhs, rhs],
            }
        };
    }

    Ok(lhs)
}

fn parse_application(ts: &mut TokenStream, stop: Stop) -> Result<ast::Expr> {
    let mut exprs = Vec::new();
    exprs.push(parse_atom(ts)?);

    while ts.can_continue_expr(stop) {
        match ts.peek_kind() {
            Some(TokenKind::Backslash)
            | Some(TokenKind::KwIf)
            | Some(TokenKind::KwLet)
            | Some(TokenKind::KwCase)
            | Some(TokenKind::KwDo) => {
                exprs.push(parse_expr(ts, stop)?);
            }
            Some(
                TokenKind::Ident(_)
                | TokenKind::Integer(_)
                | TokenKind::Float(_)
                | TokenKind::String(_)
                | TokenKind::Char(_)
                | TokenKind::True
                | TokenKind::False
                | TokenKind::LBracket
                | TokenKind::LParen
                | TokenKind::LBrace,
            ) => {
                exprs.push(parse_atom(ts)?);
            }
            _ => break,
        }
    }

    if exprs.len() == 1 {
        Ok(exprs.remove(0))
    } else {
        let func = Box::new(exprs.remove(0));
        Ok(ast::Expr::Apply { func, args: exprs })
    }
}

fn parse_atom(ts: &mut TokenStream) -> Result<ast::Expr> {
    match ts.peek_kind() {
        Some(TokenKind::True) => {
            ts.bump();
            Ok(ast::Expr::Bool(true))
        }
        Some(TokenKind::False) => {
            ts.bump();
            Ok(ast::Expr::Bool(false))
        }
        Some(TokenKind::Integer(_)) => match ts.bump() {
            Some(TokenKind::Integer(s)) => Ok(ast::Expr::Integer(s)),
            _ => unreachable!(),
        },
        Some(TokenKind::Float(_)) => match ts.bump() {
            Some(TokenKind::Float(s)) => Ok(ast::Expr::Float64(s)),
            _ => unreachable!(),
        },
        Some(TokenKind::String(_)) => match ts.bump() {
            Some(TokenKind::String(s)) => Ok(ast::Expr::String(s)),
            _ => unreachable!(),
        },
        Some(TokenKind::Char(_)) => match ts.bump() {
            Some(TokenKind::Char(ch)) => Ok(ast::Expr::Char(ch)),
            _ => unreachable!(),
        },
        Some(TokenKind::Ident(_)) => {
            let s = parse_maybe_qualified_ident(ts)?;
            if is_upper_by_last_segment(&s) {
                Ok(ast::Expr::Ctor(s))
            } else {
                Ok(ast::Expr::Var(s))
            }
        }
        Some(TokenKind::LBracket) => parse_list_expr(ts),
        Some(TokenKind::LParen) => parse_paren_or_tuple_expr(ts),
        Some(TokenKind::LBrace) => parse_record_expr(ts),
        _ => Err(Error::msg("expected expression")),
    }
}

fn parse_paren_or_tuple_expr(ts: &mut TokenStream) -> Result<ast::Expr> {
    ts.expect(TokenKind::LParen)?;

    if matches!(ts.peek_kind(), Some(TokenKind::RParen)) {
        ts.bump();
        return Ok(ast::Expr::Unit);
    }

    let first = parse_expr(ts, Stop::LineEnd)?;
    if matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
        let mut elems = vec![first];
        while matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
            ts.bump();
            elems.push(parse_expr(ts, Stop::LineEnd)?);
        }
        ts.expect(TokenKind::RParen)?;
        Ok(ast::Expr::Tuple(elems))
    } else {
        ts.expect(TokenKind::RParen)?;
        Ok(first)
    }
}

fn parse_list_expr(ts: &mut TokenStream) -> Result<ast::Expr> {
    ts.expect(TokenKind::LBracket)?;

    if matches!(ts.peek_kind(), Some(TokenKind::RBracket)) {
        ts.bump();
        return Ok(ast::Expr::List(Vec::new()));
    }

    let first = parse_expr(ts, Stop::Pattern)?;

    // List comprehension: [ expr | generator_list ]
    if matches!(ts.peek_kind(), Some(TokenKind::Pipe)) {
        ts.bump();

        enum Gen {
            Bind(ast::Pattern, ast::Expr),
            Guard(ast::Expr),
        }

        let mut gens = Vec::new();
        loop {
            let save = ts.i;
            if let Ok(pat) = parse_pattern(ts) {
                if matches!(ts.peek_kind(), Some(TokenKind::LeftArrow)) {
                    ts.bump();
                    let rhs = parse_expr(ts, Stop::Pattern)?;
                    gens.push(Gen::Bind(pat, rhs));
                } else {
                    ts.i = save;
                    let e = parse_expr(ts, Stop::Pattern)?;
                    gens.push(Gen::Guard(e));
                }
            } else {
                ts.i = save;
                let e = parse_expr(ts, Stop::Pattern)?;
                gens.push(Gen::Guard(e));
            }

            if matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
                ts.bump();
                continue;
            }
            break;
        }

        ts.expect(TokenKind::RBracket)?;

        let mut out = ast::Expr::List(vec![first]);
        for g in gens.into_iter().rev() {
            out = match g {
                Gen::Guard(cond) => ast::Expr::If {
                    cond: Box::new(cond),
                    then_branch: Box::new(out),
                    else_branch: Box::new(ast::Expr::List(Vec::new())),
                },
                Gen::Bind(pat, xs) => match pat {
                    ast::Pattern::Var(name) => ast::Expr::Apply {
                        func: Box::new(ast::Expr::Var("concatMap".to_string())),
                        args: vec![
                            ast::Expr::Lambda {
                                params: vec![name],
                                body: Box::new(out),
                            },
                            xs,
                        ],
                    },
                    ast::Pattern::Wildcard => ast::Expr::Apply {
                        func: Box::new(ast::Expr::Var("concatMap".to_string())),
                        args: vec![
                            ast::Expr::Lambda {
                                params: vec!["_".to_string()],
                                body: Box::new(out),
                            },
                            xs,
                        ],
                    },
                    other_pat => {
                        let tmp = ts.fresh_name("_lc");
                        ast::Expr::Apply {
                            func: Box::new(ast::Expr::Var("concatMap".to_string())),
                            args: vec![
                                ast::Expr::Lambda {
                                    params: vec![tmp.clone()],
                                    body: Box::new(ast::Expr::Case {
                                        expr: Box::new(ast::Expr::Var(tmp)),
                                        arms: vec![
                                            ast::CaseArm {
                                                pat: other_pat,
                                                guard: None,
                                                body: out,
                                            },
                                            ast::CaseArm {
                                                pat: ast::Pattern::Wildcard,
                                                guard: None,
                                                body: ast::Expr::List(Vec::new()),
                                            },
                                        ],
                                    }),
                                },
                                xs,
                            ],
                        }
                    }
                },
            };
        }

        return Ok(out);
    }

    // List literal: [e1, e2, ...]
    let mut elems = vec![first];
    while matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
        ts.bump();
        elems.push(parse_expr(ts, Stop::Pattern)?);
    }

    ts.expect(TokenKind::RBracket)?;
    Ok(ast::Expr::List(elems))
}

fn parse_record_expr(ts: &mut TokenStream) -> Result<ast::Expr> {
    ts.expect(TokenKind::LBrace)?;

    if matches!(ts.peek_kind(), Some(TokenKind::RBrace)) {
        ts.bump();
        return Ok(ast::Expr::Record(Vec::new()));
    }

    let mut fields = Vec::new();
    let name = ts.expect_ident()?;
    ts.expect(TokenKind::Colon)?;
    let expr = parse_expr(ts, Stop::LineEnd)?;
    fields.push((name, expr));

    while matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
        ts.bump();
        let name = ts.expect_ident()?;
        ts.expect(TokenKind::Colon)?;
        let expr = parse_expr(ts, Stop::LineEnd)?;
        fields.push((name, expr));
    }

    ts.expect(TokenKind::RBrace)?;
    Ok(ast::Expr::Record(fields))
}

struct TokenStream {
    tokens: Vec<lexer::Token>,
    i: usize,
    gensym: u32,
}

impl TokenStream {
    fn new(tokens: Vec<lexer::Token>) -> Self {
        Self {
            tokens,
            i: 0,
            gensym: 0,
        }
    }

    fn fresh_name(&mut self, prefix: &str) -> String {
        let n = self.gensym;
        self.gensym += 1;
        format!("{prefix}{n}")
    }

    fn is_eof(&self) -> bool {
        self.i >= self.tokens.len()
    }

    fn peek_kind(&self) -> Option<&TokenKind> {
        self.tokens.get(self.i).map(|t| &t.kind)
    }

    fn bump(&mut self) -> Option<TokenKind> {
        let t = self.tokens.get(self.i)?.kind.clone();
        self.i += 1;
        Some(t)
    }

    fn expect(&mut self, kind: TokenKind) -> Result<()> {
        let got = self.bump().ok_or_else(|| Error::msg("unexpected EOF"))?;
        if got == kind {
            Ok(())
        } else {
            Err(Error::msg("unexpected token"))
        }
    }

    fn expect_ident(&mut self) -> Result<String> {
        match self.bump() {
            Some(TokenKind::Ident(s)) => Ok(s),
            _ => Err(Error::msg("expected identifier")),
        }
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek_kind(), Some(TokenKind::Newline)) {
            self.i += 1;
        }
    }

    fn consume_line_end(&mut self) {
        while matches!(self.peek_kind(), Some(TokenKind::Newline)) {
            self.i += 1;
        }
    }

    fn can_continue_expr(&self, stop: Stop) -> bool {
        match (stop, self.peek_kind()) {
            (_, None) => false,
            (_, Some(TokenKind::Newline)) => false,
            (Stop::Then, Some(TokenKind::KwThen)) => false,
            (Stop::Else, Some(TokenKind::KwElse)) => false,
            (Stop::In, Some(TokenKind::KwIn)) => false,
            (Stop::Of, Some(TokenKind::KwOf)) => false,
            (Stop::Pattern, Some(TokenKind::Arrow | TokenKind::Eq | TokenKind::Comma)) => false,
            (Stop::Pattern, Some(TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace)) => false,
            (Stop::Pattern, Some(TokenKind::Dedent)) => false,
            (Stop::LineEnd, _) => true,
            _ => true,
        }
    }

    fn can_continue_pattern(&self) -> bool {
        !matches!(
            self.peek_kind(),
            None | Some(TokenKind::Newline)
                | Some(TokenKind::Dedent)
                | Some(TokenKind::Arrow)
                | Some(TokenKind::LeftArrow)
                | Some(TokenKind::Eq)
                | Some(TokenKind::Comma)
                | Some(TokenKind::Colon)
                | Some(TokenKind::Pipe)
                | Some(TokenKind::At)
                | Some(TokenKind::Ellipsis)
                | Some(TokenKind::RParen)
                | Some(TokenKind::RBracket)
                | Some(TokenKind::RBrace)
        )
    }
}
