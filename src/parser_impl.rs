use crate::{ast, error::Error, lexer, lexer::TokenKind, Result};
use std::collections::HashMap;

use crate::parser::token_stream::{self, compute_line_starts, Assoc, Fixity, TokenStream};

fn expr_from(ts: &TokenStream, start: usize, kind: ast::ExprKind) -> ast::Expr {
    ast::Expr::new(ts.span_from(start), kind)
}

fn pat_from(ts: &TokenStream, start: usize, kind: ast::PatternKind) -> ast::Pattern {
    ast::Pattern::new(ts.span_from(start), kind)
}

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
    let fixities = collect_fixities(&tokens);
    let line_starts = compute_line_starts(src);
    let mut ts = TokenStream::new(tokens, fixities, line_starts);

    ts.skip_newlines();

    if matches!(ts.peek_kind(), Some(TokenKind::KwModule)) {
        parse_module_decl(&mut ts)
    } else {
        let items = parse_items_until(&mut ts, StopAt::Eof)?;
        Ok(ast::Module { name: None, items })
    }
}

fn token_op_name(kind: &TokenKind) -> Option<String> {
    Some(match kind {
        TokenKind::Ident(s) => s.clone(),
        TokenKind::Operator(s) => s.clone(),
        TokenKind::Plus => "+".to_string(),
        TokenKind::Minus => "-".to_string(),
        TokenKind::Star => "*".to_string(),
        TokenKind::Slash => "/".to_string(),
        TokenKind::PlusPlus => "++".to_string(),
        TokenKind::Colon => ":".to_string(),
        TokenKind::EqEq => "==".to_string(),
        TokenKind::SlashEq => "/=".to_string(),
        TokenKind::Lt => "<".to_string(),
        TokenKind::Le => "<=".to_string(),
        TokenKind::Gt => ">".to_string(),
        TokenKind::Ge => ">=".to_string(),
        TokenKind::GtGt => ">>".to_string(),
        TokenKind::GtGtEq => ">>=".to_string(),
        TokenKind::AndAnd => "&&".to_string(),
        TokenKind::OrOr => "||".to_string(),
        _ => return None,
    })
}

fn collect_fixities(tokens: &[lexer::Token]) -> HashMap<String, Fixity> {
    let mut out = HashMap::new();
    let mut i = 0usize;
    while i < tokens.len() {
        let assoc = match &tokens[i].kind {
            TokenKind::KwInfix => Some(Assoc::Non),
            TokenKind::KwInfixl => Some(Assoc::Left),
            TokenKind::KwInfixr => Some(Assoc::Right),
            _ => None,
        };
        let Some(assoc) = assoc else {
            i += 1;
            continue;
        };
        i += 1;

        let Some(lexer::Token {
            kind: TokenKind::Integer(p),
            ..
        }) = tokens.get(i)
        else {
            continue;
        };
        let Ok(prec) = p.parse::<u8>() else {
            continue;
        };
        i += 1;

        while i < tokens.len() {
            match &tokens[i].kind {
                TokenKind::Newline | TokenKind::Dedent => break,
                TokenKind::Comma => {
                    i += 1;
                    continue;
                }
                k => {
                    if let Some(op) = token_op_name(k) {
                        out.insert(op, Fixity { prec, assoc });
                    }
                    i += 1;
                }
            }
        }
    }
    out
}

fn parse_module_decl(ts: &mut TokenStream) -> Result<ast::Module> {
    ts.expect(TokenKind::KwModule)?;
    let name = parse_maybe_qualified_ident(ts)?;
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
    let mut pending: Option<PendingFun> = None;

    loop {
        ts.skip_newlines();
        if ts.is_eof() {
            break;
        }
        if matches!(stop_at, StopAt::Dedent) && matches!(ts.peek_kind(), Some(TokenKind::Dedent)) {
            break;
        }

        let tok = ts.peek_kind().cloned();
        match tok {
            Some(TokenKind::KwImport) => {
                flush_pending_fun_item(ts, &mut items, pending.take())?;
                items.push(parse_import_decl(ts)?);
            }
            Some(TokenKind::KwExport) => {
                flush_pending_fun_item(ts, &mut items, pending.take())?;
                items.push(parse_export_decl(ts)?);
            }
            Some(TokenKind::KwInfix | TokenKind::KwInfixl | TokenKind::KwInfixr) => {
                flush_pending_fun_item(ts, &mut items, pending.take())?;
                items.push(parse_fixity_decl(ts)?);
            }
            Some(TokenKind::KwData) => {
                flush_pending_fun_item(ts, &mut items, pending.take())?;
                items.push(parse_data_decl(ts)?);
            }
            Some(TokenKind::KwType) => {
                flush_pending_fun_item(ts, &mut items, pending.take())?;
                items.push(parse_type_alias(ts)?);
            }
            Some(TokenKind::KwClass) => {
                flush_pending_fun_item(ts, &mut items, pending.take())?;
                items.push(parse_class_decl(ts)?);
            }
            Some(TokenKind::KwInstance) => {
                flush_pending_fun_item(ts, &mut items, pending.take())?;
                items.push(parse_instance_decl(ts)?);
            }
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
            ) => match parse_binding_or_fun_clause(ts, Stop::LineEnd)? {
                ParsedBind::Binding(b) => {
                    flush_pending_fun_item(ts, &mut items, pending.take())?;
                    items.push(ast::Item::Binding(b));
                }
                ParsedBind::FunClause(c) => {
                    push_fun_clause_item(ts, &mut items, &mut pending, c)?;
                }
            },
            Some(_) => return Err(ts.err_here("unexpected token at top-level")),
            None => break,
        }

        ts.consume_line_end();

        if matches!(stop_at, StopAt::Eof) && ts.is_eof() {
            break;
        }
    }

    flush_pending_fun_item(ts, &mut items, pending.take())?;
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
        // Try prefix ctor first: `Ctor a b` / `(:*:) a b`.
        // If that fails, accept Haskell-style infix ctor: `a :*: b`.
        let save = (ts.i, ts.last_span_end);
        let parsed = if let Ok(ctor_name) = parse_ctor_name(ts) {
            let mut args = Vec::new();
            while matches!(
                ts.peek_kind(),
                Some(TokenKind::Ident(s)) if s != "deriving"
            ) || matches!(
                ts.peek_kind(),
                Some(TokenKind::LParen) | Some(TokenKind::LBracket) | Some(TokenKind::LBrace)
            ) {
                // Atom-level type parsing for ctor args.
                args.push(parse_type_atom(ts, Stop::LineEnd, is_type_alias_end)?);
            }
            Some(ast::DataCtor {
                name: ctor_name,
                args,
            })
        } else {
            (ts.i, ts.last_span_end) = save;
            // Infix ctor form: `<ty> :<op>: <ty>`.
            let lhs = parse_type_atom(ts, Stop::LineEnd, is_type_alias_end)?;
            let Some(TokenKind::Operator(op)) = ts.peek_kind() else {
                return Err(ts.err_here("expected constructor name"));
            };
            if !is_ctor_symbol(op.as_str()) {
                return Err(ts.err_here("expected ':'-prefixed constructor operator"));
            }
            let op = op.clone();
            ts.bump();
            let rhs = parse_type_atom(ts, Stop::LineEnd, is_type_alias_end)?;
            Some(ast::DataCtor {
                name: op,
                args: vec![lhs, rhs],
            })
        };

        if let Some(ctor) = parsed {
            ctors.push(ctor);
        }

        match ts.peek_kind() {
            Some(TokenKind::Pipe) => {
                ts.bump();
            }
            Some(TokenKind::Newline) | None => break,
            _ => break,
        }
    }

    let mut deriving = Vec::new();
    if matches!(ts.peek_kind(), Some(TokenKind::Ident(s)) if s == "deriving") {
        ts.bump();
        if matches!(ts.peek_kind(), Some(TokenKind::LParen)) {
            ts.bump();
            if matches!(ts.peek_kind(), Some(TokenKind::Ident(_))) {
                deriving.push(ts.expect_ident()?);
                while matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
                    ts.bump();
                    deriving.push(ts.expect_ident()?);
                }
            }
            ts.expect(TokenKind::RParen)?;
        } else {
            deriving.push(ts.expect_ident()?);
        }
    }

    Ok(ast::Item::DataDecl(ast::DataDecl {
        name,
        params,
        ctors,
        deriving,
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

fn parse_class_decl(ts: &mut TokenStream) -> Result<ast::Item> {
    ts.expect(TokenKind::KwClass)?;

    // Optional superclass context (Haskell-style):
    //   class (p1, p2) => C a where
    //   class p => C a where
    let mut supers: Vec<ast::Predicate> = Vec::new();
    if is_class_super_parens(ts) {
        ts.bump();
        supers.push(parse_predicate(ts, Stop::LineEnd)?);
        while matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
            ts.bump();
            supers.push(parse_predicate(ts, Stop::LineEnd)?);
        }
        ts.expect(TokenKind::RParen)?;
        ts.expect(TokenKind::FatArrow)?;
    } else {
        // Single predicate form, but only if followed by `=>`.
        let save = ts.i;
        if let Ok(pred) = parse_predicate(ts, Stop::LineEnd) {
            if matches!(ts.peek_kind(), Some(TokenKind::FatArrow)) {
                ts.bump();
                supers.push(pred);
            } else {
                ts.i = save;
            }
        } else {
            ts.i = save;
        }
    }

    let name = ts.expect_ident()?;
    let param = ts.expect_ident()?;
    ts.expect(TokenKind::KwWhere)?;
    ts.consume_line_end();
    ts.skip_newlines();
    if !matches!(ts.peek_kind(), Some(TokenKind::Indent)) {
        // Allow empty class bodies.
        return Ok(ast::Item::ClassDecl(ast::ClassDecl {
            name,
            param,
            supers,
            methods: Vec::new(),
            default_methods: Vec::new(),
        }));
    }

    ts.expect(TokenKind::Indent)?;

    let mut methods: Vec<ast::ClassMethodSig> = Vec::new();

    let mut default_items: Vec<ast::Item> = Vec::new();
    let mut pending: Option<PendingFun> = None;

    loop {
        ts.skip_newlines();
        if matches!(ts.peek_kind(), Some(TokenKind::Dedent)) {
            break;
        }
        if ts.is_eof() {
            return Err(ts.err_here("unexpected EOF in class"));
        }

        // Try parsing a signature line first:
        //   f :: ...
        //   (++) :: ...
        {
            let save = (ts.i, ts.last_span_end);

            // Parse a method name token that can be either an identifier or a parenthesized operator.
            let maybe_name = match ts.peek_kind() {
                Some(TokenKind::Ident(_)) => ts.expect_ident().ok(),
                Some(TokenKind::LParen) => {
                    // Only treat it as an operator-name if it looks like one.
                    let save2 = (ts.i, ts.last_span_end);
                    ts.bump();
                    let op_ok = matches!(ts.peek_kind(), Some(TokenKind::Backtick))
                        || is_sym_op_token(ts.peek_kind());
                    (ts.i, ts.last_span_end) = save2;
                    if op_ok {
                        parse_paren_operator_name(ts).ok()
                    } else {
                        None
                    }
                }
                _ => None,
            };

            if let Some(mname) = maybe_name {
                if matches!(ts.peek_kind(), Some(TokenKind::ColonColon)) {
                    ts.expect(TokenKind::ColonColon)?;
                    let ty = parse_qual_type(ts, Stop::LineEnd)?;
                    methods.push(ast::ClassMethodSig { name: mname, ty });
                    ts.consume_line_end();
                    continue;
                }
            }

            (ts.i, ts.last_span_end) = save;
        }

        // Otherwise parse a default method binding / clause.
        match ts.peek_kind() {
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
            ) => match parse_binding_or_fun_clause(ts, Stop::LineEnd)? {
                ParsedBind::Binding(b) => {
                    flush_pending_fun_item(ts, &mut default_items, pending.take())?;
                    default_items.push(ast::Item::Binding(b));
                }
                ParsedBind::FunClause(c) => {
                    push_fun_clause_item(ts, &mut default_items, &mut pending, c)?;
                }
            },
            Some(_) => return Err(ts.err_here("unexpected token in class")),
            None => break,
        }

        ts.consume_line_end();
    }

    flush_pending_fun_item(ts, &mut default_items, pending.take())?;

    ts.expect(TokenKind::Dedent)?;
    ts.consume_line_end();

    let mut default_methods: Vec<ast::Binding> = Vec::new();
    for it in default_items {
        let ast::Item::Binding(b) = it else {
            return Err(ts.err_here("unexpected item in class"));
        };
        default_methods.push(b);
    }

    Ok(ast::Item::ClassDecl(ast::ClassDecl {
        name,
        param,
        supers,
        methods,
        default_methods,
    }))
}

fn parse_instance_decl(ts: &mut TokenStream) -> Result<ast::Item> {
    ts.expect(TokenKind::KwInstance)?;

    // Optional instance context (Haskell-style):
    //   instance (p1, p2) => C t where
    //   instance p => C t where
    let mut preds: Vec<ast::Predicate> = Vec::new();
    if is_class_super_parens(ts) {
        ts.bump();
        preds.push(parse_predicate(ts, Stop::LineEnd)?);
        while matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
            ts.bump();
            preds.push(parse_predicate(ts, Stop::LineEnd)?);
        }
        ts.expect(TokenKind::RParen)?;
        ts.expect(TokenKind::FatArrow)?;
    } else {
        // Single predicate form, but only if followed by `=>`.
        let save = ts.i;
        if let Ok(pred) = parse_predicate(ts, Stop::LineEnd) {
            if matches!(ts.peek_kind(), Some(TokenKind::FatArrow)) {
                ts.bump();
                preds.push(pred);
            } else {
                ts.i = save;
            }
        } else {
            ts.i = save;
        }
    }

    let class = ts.expect_ident()?;
    let ty = parse_type_expr(ts, Stop::LineEnd, is_type_end)?;
    ts.expect(TokenKind::KwWhere)?;
    ts.consume_line_end();
    ts.skip_newlines();
    if !matches!(ts.peek_kind(), Some(TokenKind::Indent)) {
        // Allow empty instance bodies (useful when all methods have class defaults).
        return Ok(ast::Item::InstanceDecl(ast::InstanceDecl {
            preds,
            class,
            ty,
            methods: Vec::new(),
        }));
    }

    ts.expect(TokenKind::Indent)?;

    let mut method_items: Vec<ast::Item> = Vec::new();
    let mut pending: Option<PendingFun> = None;
    loop {
        ts.skip_newlines();
        if matches!(ts.peek_kind(), Some(TokenKind::Dedent)) {
            break;
        }
        if ts.is_eof() {
            return Err(ts.err_here("unexpected EOF in instance"));
        }

        match ts.peek_kind() {
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
            ) => match parse_binding_or_fun_clause(ts, Stop::LineEnd)? {
                ParsedBind::Binding(b) => {
                    flush_pending_fun_item(ts, &mut method_items, pending.take())?;
                    method_items.push(ast::Item::Binding(b));
                }
                ParsedBind::FunClause(c) => {
                    push_fun_clause_item(ts, &mut method_items, &mut pending, c)?;
                }
            },
            Some(_) => return Err(ts.err_here("unexpected token in instance")),
            None => break,
        }

        ts.consume_line_end();
    }

    flush_pending_fun_item(ts, &mut method_items, pending.take())?;
    ts.expect(TokenKind::Dedent)?;
    ts.consume_line_end();

    let mut methods: Vec<ast::Binding> = Vec::new();
    for it in method_items {
        let ast::Item::Binding(b) = it else {
            return Err(ts.err_here("unexpected item in instance"));
        };
        methods.push(b);
    }

    Ok(ast::Item::InstanceDecl(ast::InstanceDecl {
        preds,
        class,
        ty,
        methods,
    }))
}

fn parse_import_decl(ts: &mut TokenStream) -> Result<ast::Item> {
    ts.expect(TokenKind::KwImport)?;

    let qualified = matches!(ts.peek_kind(), Some(TokenKind::Ident(s)) if s == "qualified");
    if qualified {
        ts.bump();
    }

    let module = parse_maybe_qualified_ident(ts)?;

    let as_name = match ts.peek_kind() {
        Some(TokenKind::Ident(s)) if s == "as" => {
            ts.bump();
            Some(ts.expect_ident()?)
        }
        _ => None,
    };

    Ok(ast::Item::Import(ast::ImportDecl {
        module,
        qualified,
        as_name,
    }))
}

fn parse_export_decl(ts: &mut TokenStream) -> Result<ast::Item> {
    ts.expect(TokenKind::KwExport)?;

    fn parse_export_spec(ts: &mut TokenStream) -> Result<ast::ExportSpec> {
        let name = match ts.peek_kind() {
            Some(TokenKind::Ident(_)) => ts.expect_ident()?,
            Some(TokenKind::LParen) => parse_paren_operator_name(ts)?,
            _ => return Err(ts.err_here("expected export name")),
        };

        if !matches!(ts.peek_kind(), Some(TokenKind::LParen)) {
            return Ok(ast::ExportSpec::Name(name));
        }

        ts.bump();

        let spec = if matches!(ts.peek_kind(), Some(TokenKind::Dot)) {
            ts.expect(TokenKind::Dot)?;
            ts.expect(TokenKind::Dot)?;
            ts.expect(TokenKind::RParen)?;
            ast::ExportSpec::Type {
                name,
                ctors: ast::ExportCtors::All,
            }
        } else if matches!(ts.peek_kind(), Some(TokenKind::Operator(op)) if op == "..") {
            ts.bump();
            ts.expect(TokenKind::RParen)?;
            ast::ExportSpec::Type {
                name,
                ctors: ast::ExportCtors::All,
            }
        } else {
            let mut ctors = Vec::new();
            ctors.push(parse_ctor_name(ts)?);
            while matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
                ts.bump();
                ctors.push(parse_ctor_name(ts)?);
            }
            ts.expect(TokenKind::RParen)?;
            ast::ExportSpec::Type {
                name,
                ctors: ast::ExportCtors::Some(ctors),
            }
        };

        Ok(spec)
    }

    let mut specs = Vec::new();
    specs.push(parse_export_spec(ts)?);
    while matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
        ts.bump();
        specs.push(parse_export_spec(ts)?);
    }

    Ok(ast::Item::Export(ast::ExportDecl { specs }))
}

fn parse_fixity_op(ts: &mut TokenStream) -> Result<String> {
    match ts.peek_kind() {
        Some(TokenKind::Ident(_)) => Ok(ts.expect_ident()?),
        Some(TokenKind::Operator(_)) => match ts.bump() {
            Some(TokenKind::Operator(s)) => Ok(s),
            _ => unreachable!(),
        },
        Some(TokenKind::Plus) => {
            ts.bump();
            Ok("+".to_string())
        }
        Some(TokenKind::Minus) => {
            ts.bump();
            Ok("-".to_string())
        }
        Some(TokenKind::Star) => {
            ts.bump();
            Ok("*".to_string())
        }
        Some(TokenKind::Slash) => {
            ts.bump();
            Ok("/".to_string())
        }
        Some(TokenKind::PlusPlus) => {
            ts.bump();
            Ok("++".to_string())
        }
        Some(TokenKind::Colon) => {
            ts.bump();
            Ok(":".to_string())
        }
        Some(TokenKind::EqEq) => {
            ts.bump();
            Ok("==".to_string())
        }
        Some(TokenKind::SlashEq) => {
            ts.bump();
            Ok("/=".to_string())
        }
        Some(TokenKind::Lt) => {
            ts.bump();
            Ok("<".to_string())
        }
        Some(TokenKind::Le) => {
            ts.bump();
            Ok("<=".to_string())
        }
        Some(TokenKind::Gt) => {
            ts.bump();
            Ok(">".to_string())
        }
        Some(TokenKind::Ge) => {
            ts.bump();
            Ok(">=".to_string())
        }
        Some(TokenKind::GtGt) => {
            ts.bump();
            Ok(">>".to_string())
        }
        Some(TokenKind::GtGtEq) => {
            ts.bump();
            Ok(">>=".to_string())
        }
        Some(TokenKind::AndAnd) => {
            ts.bump();
            Ok("&&".to_string())
        }
        Some(TokenKind::OrOr) => {
            ts.bump();
            Ok("||".to_string())
        }
        _ => Err(ts.err_here("expected operator name")),
    }
}

fn parse_fixity_decl(ts: &mut TokenStream) -> Result<ast::Item> {
    let assoc = match ts.peek_kind() {
        Some(TokenKind::KwInfix) => {
            ts.bump();
            ast::FixityAssoc::Infix
        }
        Some(TokenKind::KwInfixl) => {
            ts.bump();
            ast::FixityAssoc::Infixl
        }
        Some(TokenKind::KwInfixr) => {
            ts.bump();
            ast::FixityAssoc::Infixr
        }
        _ => return Err(ts.err_here("expected fixity keyword")),
    };

    let prec_tok_span = ts.peek_span().unwrap_or(lexer::Span {
        start: ts.last_span_end,
        end: ts.last_span_end,
    });
    let prec_pos = ts.pos_str_here();
    let prec = match ts.bump() {
        Some(TokenKind::Integer(s)) => s.parse::<u8>().map_err(|_| {
            Error::msg_with_span(
                format!("invalid fixity precedence at {prec_pos}"),
                prec_tok_span,
            )
        })?,
        _ => {
            return Err(Error::msg_with_span(
                format!("expected fixity precedence at {prec_pos}"),
                prec_tok_span,
            ))
        }
    };

    let mut ops = Vec::new();
    ops.push(parse_fixity_op(ts)?);
    while matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
        ts.bump();
        ops.push(parse_fixity_op(ts)?);
    }

    Ok(ast::Item::Fixity(ast::FixityDecl { assoc, prec, ops }))
}

fn parse_ctor_name(ts: &mut TokenStream) -> Result<String> {
    match ts.peek_kind() {
        Some(TokenKind::Ident(_)) => {
            let name_span = ts.peek_span().unwrap_or(lexer::Span {
                start: ts.last_span_end,
                end: ts.last_span_end,
            });
            let name = ts.expect_ident()?;
            if name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                Ok(name)
            } else {
                Err(Error::msg_with_span(
                    format!(
                        "expected constructor name at {}",
                        ts.pos_str_at(name_span.start)
                    ),
                    name_span,
                ))
            }
        }
        Some(TokenKind::Operator(op)) => {
            let op_span = ts.peek_span().unwrap_or(lexer::Span {
                start: ts.last_span_end,
                end: ts.last_span_end,
            });
            let op = op.clone();
            ts.bump();
            if is_ctor_symbol(&op) {
                Ok(op)
            } else {
                Err(Error::msg_with_span(
                    format!(
                        "expected constructor name at {}",
                        ts.pos_str_at(op_span.start)
                    ),
                    op_span,
                ))
            }
        }
        Some(TokenKind::LParen) => {
            let lparen_span = ts.peek_span().unwrap_or(lexer::Span {
                start: ts.last_span_end,
                end: ts.last_span_end,
            });
            // Allow parenthesized operator constructors: (:*:) / (:)
            let save = (ts.i, ts.last_span_end);
            ts.bump();
            let op_ok = matches!(ts.peek_kind(), Some(TokenKind::Backtick))
                || is_sym_op_token(ts.peek_kind());
            if !op_ok {
                (ts.i, ts.last_span_end) = save;
                return Err(Error::msg_with_span(
                    format!(
                        "expected constructor name at {}",
                        ts.pos_str_at(lparen_span.start)
                    ),
                    lparen_span,
                ));
            }
            let op = parse_operator_name(ts)?;
            ts.expect(TokenKind::RParen)?;
            if is_ctor_symbol(&op) {
                Ok(op)
            } else {
                Err(Error::msg_with_span(
                    format!(
                        "expected ':'-prefixed constructor operator at {}",
                        ts.pos_str_at(lparen_span.start)
                    ),
                    lparen_span,
                ))
            }
        }
        _ => Err(ts.err_here("expected constructor name")),
    }
}

#[derive(Clone)]
struct FunClause {
    name: String,
    args: Vec<ast::Pattern>,
    guard: Option<ast::Expr>,
    body: ast::Expr,
}

enum ParsedBind {
    Binding(ast::Binding),
    FunClause(FunClause),
}

struct PendingFun {
    name: String,
    arity: usize,
    clauses: Vec<(Vec<ast::Pattern>, Option<ast::Expr>, ast::Expr)>,
}

fn flush_pending_fun_item(
    ts: &mut TokenStream,
    out: &mut Vec<ast::Item>,
    pending: Option<PendingFun>,
) -> Result<()> {
    let Some(p) = pending else {
        return Ok(());
    };
    out.push(ast::Item::Binding(desugar_fun(
        ts, p.name, p.arity, p.clauses,
    )));
    Ok(())
}

fn flush_pending_fun_binding(
    ts: &mut TokenStream,
    out: &mut Vec<ast::Binding>,
    pending: Option<PendingFun>,
) {
    let Some(p) = pending else {
        return;
    };
    out.push(desugar_fun(ts, p.name, p.arity, p.clauses));
}

fn push_fun_clause_binding(
    ts: &mut TokenStream,
    out: &mut Vec<ast::Binding>,
    pending: &mut Option<PendingFun>,
    c: FunClause,
) {
    let arity = c.args.len();
    let clause = (c.args, c.guard, c.body);

    match pending {
        Some(p) if p.name == c.name && p.arity == arity => {
            p.clauses.push(clause);
        }
        Some(_) => {
            flush_pending_fun_binding(ts, out, pending.take());
            *pending = Some(PendingFun {
                name: c.name,
                arity,
                clauses: vec![clause],
            });
        }
        None => {
            *pending = Some(PendingFun {
                name: c.name,
                arity,
                clauses: vec![clause],
            });
        }
    }
}

fn push_fun_clause_item(
    ts: &mut TokenStream,
    out: &mut Vec<ast::Item>,
    pending: &mut Option<PendingFun>,
    c: FunClause,
) -> Result<()> {
    let arity = c.args.len();
    let clause = (c.args, c.guard, c.body);

    match pending {
        Some(p) if p.name == c.name && p.arity == arity => {
            p.clauses.push(clause);
        }
        Some(_) => {
            flush_pending_fun_item(ts, out, pending.take())?;
            *pending = Some(PendingFun {
                name: c.name,
                arity,
                clauses: vec![clause],
            });
        }
        None => {
            *pending = Some(PendingFun {
                name: c.name,
                arity,
                clauses: vec![clause],
            });
        }
    }

    Ok(())
}

fn desugar_fun(
    ts: &mut TokenStream,
    name: String,
    arity: usize,
    clauses: Vec<(Vec<ast::Pattern>, Option<ast::Expr>, ast::Expr)>,
) -> ast::Binding {
    let params: Vec<String> = (0..arity).map(|_| ts.fresh_name("_arg")).collect();
    let scrut = if arity == 1 {
        ast::Expr::dummy(ast::ExprKind::Var(params[0].clone()))
    } else {
        ast::Expr::dummy(ast::ExprKind::Tuple(
            params
                .iter()
                .map(|p| ast::Expr::dummy(ast::ExprKind::Var(p.clone())))
                .collect(),
        ))
    };

    let arms = clauses
        .into_iter()
        .map(|(pats, guard, body)| ast::CaseArm {
            pat: if arity == 1 {
                pats.into_iter().next().expect("arity=1 clause")
            } else {
                ast::Pattern::dummy(ast::PatternKind::Tuple(pats))
            },
            guard,
            body,
        })
        .collect();

    let body = ast::Expr::dummy(ast::ExprKind::Case {
        expr: Box::new(scrut),
        arms,
    });

    ast::Binding {
        pat: ast::Pattern::dummy(ast::PatternKind::Var(name)),
        expr: ast::Expr::dummy(ast::ExprKind::Lambda {
            params,
            body: Box::new(body),
        }),
    }
}

fn parse_expr_after_newline(ts: &mut TokenStream, stop: Stop) -> Result<ast::Expr> {
    if !matches!(ts.peek_kind(), Some(TokenKind::Newline)) {
        return parse_expr(ts, stop);
    }

    ts.consume_line_end();
    ts.skip_newlines();

    // Allow an optional indentation wrapper (common after then/else).
    if matches!(ts.peek_kind(), Some(TokenKind::Indent)) {
        ts.expect(TokenKind::Indent)?;
        let expr = parse_expr(ts, stop)?;
        ts.consume_line_end();
        ts.expect(TokenKind::Dedent)?;
        return Ok(expr);
    }

    parse_expr(ts, stop)
}

fn parse_eq_rhs(ts: &mut TokenStream, stop: Stop) -> Result<ast::Expr> {
    // Support Haskell-like layout:
    //   x =
    //     expr
    if !matches!(ts.peek_kind(), Some(TokenKind::Newline)) {
        return parse_expr(ts, stop);
    }

    ts.consume_line_end();
    ts.skip_newlines();
    ts.expect(TokenKind::Indent)?;

    let expr = parse_expr(ts, stop)?;

    ts.consume_line_end();
    ts.expect(TokenKind::Dedent)?;

    Ok(expr)
}

fn parse_binding_simple(ts: &mut TokenStream, stop: Stop) -> Result<ast::Binding> {
    let pat = parse_pattern(ts)?;
    ts.expect(TokenKind::Eq)?;
    let expr = parse_eq_rhs(ts, stop)?;
    Ok(ast::Binding { pat, expr })
}

fn is_sym_op_token(kind: Option<&TokenKind>) -> bool {
    matches!(
        kind,
        Some(
            TokenKind::Operator(_)
                | TokenKind::Colon
                | TokenKind::Plus
                | TokenKind::Minus
                | TokenKind::Star
                | TokenKind::Slash
                | TokenKind::PlusPlus
                | TokenKind::EqEq
                | TokenKind::SlashEq
                | TokenKind::Lt
                | TokenKind::Le
                | TokenKind::Gt
                | TokenKind::Ge
                | TokenKind::GtGt
                | TokenKind::GtGtEq
                | TokenKind::AndAnd
                | TokenKind::OrOr
        )
    )
}

fn is_ctor_symbol(op: &str) -> bool {
    op.starts_with(':')
}

fn op_expr_kind(op: String) -> ast::ExprKind {
    if is_ctor_symbol(&op) || is_upper_by_last_segment(&op) {
        ast::ExprKind::Ctor(op)
    } else {
        ast::ExprKind::Var(op)
    }
}

fn parse_operator_name(ts: &mut TokenStream) -> Result<String> {
    if matches!(ts.peek_kind(), Some(TokenKind::Backtick)) {
        ts.expect(TokenKind::Backtick)?;
        let op = ts.expect_ident()?;
        ts.expect(TokenKind::Backtick)?;
        return Ok(op);
    }
    parse_fixity_op(ts)
}

fn parse_paren_operator_name(ts: &mut TokenStream) -> Result<String> {
    ts.expect(TokenKind::LParen)?;
    let op = parse_operator_name(ts)?;
    ts.expect(TokenKind::RParen)?;
    Ok(op)
}

fn parse_binding_or_fun_clause(ts: &mut TokenStream, stop: Stop) -> Result<ParsedBind> {
    // `fname pat1 pat2 = body` / guarded: `fname pat1 | guard = body`
    // Operator forms supported:
    // - `(++) a b = ...`
    // - `a ++ b = ...`
    // Disambiguation: reject pattern-bind continuations like `x:xs = ...` and `xs@_ = ...`.

    // Infix operator clause: `a ++ b = body`.
    {
        let save = (ts.i, ts.last_span_end);
        if let Ok(lhs) = parse_cons_pattern(ts) {
            if matches!(ts.peek_kind(), Some(TokenKind::Backtick))
                || (is_sym_op_token(ts.peek_kind())
                    && !matches!(ts.peek_kind(), Some(TokenKind::Colon)))
            {
                let op = parse_operator_name(ts)?;
                if is_ctor_symbol(&op) {
                    return Err(ts.err_here("operators starting with ':' are reserved"));
                }
                let rhs = parse_cons_pattern(ts)?;

                if matches!(ts.peek_kind(), Some(TokenKind::Eq) | Some(TokenKind::Pipe)) {
                    let guard = if matches!(ts.peek_kind(), Some(TokenKind::Pipe)) {
                        ts.bump();
                        Some(parse_expr(ts, Stop::Pattern)?)
                    } else {
                        None
                    };
                    ts.expect(TokenKind::Eq)?;
                    let body = parse_eq_rhs(ts, stop)?;
                    return Ok(ParsedBind::FunClause(FunClause {
                        name: op,
                        args: vec![lhs, rhs],
                        guard,
                        body,
                    }));
                }
            }
        }
        (ts.i, ts.last_span_end) = save;
    }

    // Parenthesized operator name clause: `(++) a b = body`.
    if matches!(ts.peek_kind(), Some(TokenKind::LParen)) {
        let save = (ts.i, ts.last_span_end);
        ts.bump();
        let op_ok =
            matches!(ts.peek_kind(), Some(TokenKind::Backtick)) || is_sym_op_token(ts.peek_kind());
        (ts.i, ts.last_span_end) = save;

        if op_ok {
            let save = (ts.i, ts.last_span_end);
            if let Ok(name) = parse_paren_operator_name(ts) {
                if is_ctor_symbol(&name) {
                    return Err(ts.err_here("operators starting with ':' are reserved"));
                }
                if !matches!(ts.peek_kind(), Some(TokenKind::Eq)) {
                    let mut args = Vec::new();
                    while !matches!(ts.peek_kind(), Some(TokenKind::Eq) | Some(TokenKind::Pipe)) {
                        // Do not treat `|` as an or-pattern here; `|` starts a guard.
                        // Or-patterns in fun args must be parenthesized.
                        args.push(parse_cons_pattern(ts)?);
                    }
                    if !args.is_empty() {
                        let guard = if matches!(ts.peek_kind(), Some(TokenKind::Pipe)) {
                            ts.bump();
                            Some(parse_expr(ts, Stop::Pattern)?)
                        } else {
                            None
                        };
                        ts.expect(TokenKind::Eq)?;
                        let body = parse_eq_rhs(ts, stop)?;
                        return Ok(ParsedBind::FunClause(FunClause {
                            name,
                            args,
                            guard,
                            body,
                        }));
                    }
                }
            }
            (ts.i, ts.last_span_end) = save;
        }
    }

    // Regular identifier clause: `f x y = body`.
    if matches!(ts.peek_kind(), Some(TokenKind::Ident(_))) {
        let save = (ts.i, ts.last_span_end);
        let name = ts.expect_ident()?;

        if !matches!(ts.peek_kind(), Some(TokenKind::Eq))
            && !matches!(ts.peek_kind(), Some(TokenKind::Colon) | Some(TokenKind::At))
        {
            let mut args = Vec::new();
            while !matches!(ts.peek_kind(), Some(TokenKind::Eq) | Some(TokenKind::Pipe)) {
                // Do not treat `|` as an or-pattern here; `|` starts a guard.
                // Or-patterns in fun args must be parenthesized.
                args.push(parse_cons_pattern(ts)?);
            }
            if !args.is_empty() {
                let guard = if matches!(ts.peek_kind(), Some(TokenKind::Pipe)) {
                    ts.bump();
                    Some(parse_expr(ts, Stop::Pattern)?)
                } else {
                    None
                };
                ts.expect(TokenKind::Eq)?;
                let body = parse_eq_rhs(ts, stop)?;
                return Ok(ParsedBind::FunClause(FunClause {
                    name,
                    args,
                    guard,
                    body,
                }));
            }
        }

        (ts.i, ts.last_span_end) = save;
    }

    Ok(ParsedBind::Binding(parse_binding_simple(ts, stop)?))
}

#[derive(Clone, Copy)]
enum Stop {
    LineEnd,
    Then,
    Else,
    Of,
    Pattern,
    LetBind,
    SemiOrRBrace,
}

impl Stop {
    fn to_token_stream(self) -> token_stream::Stop {
        match self {
            Stop::Then => token_stream::Stop::Then,
            Stop::Else => token_stream::Stop::Else,
            Stop::Of => token_stream::Stop::Of,
            Stop::LetBind => token_stream::Stop::LetBind,
            Stop::SemiOrRBrace => token_stream::Stop::SemiOrRBrace,
            Stop::Pattern => token_stream::Stop::Pattern,
            Stop::LineEnd => token_stream::Stop::LineEnd,
        }
    }
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
    let start = ts.peek_span().map(|s| s.start).unwrap_or(0);
    ts.expect(TokenKind::Backslash)?;
    let mut params = Vec::new();
    while matches!(ts.peek_kind(), Some(TokenKind::Ident(_))) {
        params.push(ts.expect_ident()?);
    }
    if params.is_empty() {
        return Err(ts.err_here("expected lambda parameter"));
    }
    ts.expect(TokenKind::Arrow)?;
    let body = Box::new(parse_expr(ts, stop)?);
    Ok(expr_from(ts, start, ast::ExprKind::Lambda { params, body }))
}

fn parse_if(ts: &mut TokenStream, stop: Stop) -> Result<ast::Expr> {
    let start = ts.peek_span().map(|s| s.start).unwrap_or(0);
    ts.expect(TokenKind::KwIf)?;
    let cond = Box::new(parse_expr(ts, Stop::Then)?);
    ts.expect(TokenKind::KwThen)?;
    let then_branch = Box::new(parse_expr_after_newline(ts, Stop::Else)?);
    ts.expect(TokenKind::KwElse)?;
    let else_branch = Box::new(parse_expr_after_newline(ts, stop)?);
    Ok(expr_from(
        ts,
        start,
        ast::ExprKind::If {
            cond,
            then_branch,
            else_branch,
        },
    ))
}

fn parse_let(ts: &mut TokenStream, stop: Stop) -> Result<ast::Expr> {
    let start = ts.peek_span().map(|s| s.start).unwrap_or(0);
    ts.expect(TokenKind::KwLet)?;

    let bindings = if matches!(ts.peek_kind(), Some(TokenKind::Newline)) {
        ts.consume_line_end();
        ts.skip_newlines();
        ts.expect(TokenKind::Indent)?;

        let mut bs = Vec::new();
        let mut pending: Option<PendingFun> = None;
        loop {
            ts.skip_newlines();
            if matches!(ts.peek_kind(), Some(TokenKind::Dedent)) {
                break;
            }
            if ts.is_eof() {
                return Err(ts.err_here("unexpected EOF in let"));
            }
            match parse_binding_or_fun_clause(ts, Stop::LineEnd)? {
                ParsedBind::Binding(b) => {
                    flush_pending_fun_binding(ts, &mut bs, pending.take());
                    bs.push(b);
                }
                ParsedBind::FunClause(c) => push_fun_clause_binding(ts, &mut bs, &mut pending, c),
            }
            ts.consume_line_end();
        }
        flush_pending_fun_binding(ts, &mut bs, pending.take());

        ts.expect(TokenKind::Dedent)?;
        ts.consume_line_end();
        bs
    } else {
        let mut bs = Vec::new();
        let mut pending: Option<PendingFun> = None;

        match parse_binding_or_fun_clause(ts, Stop::LetBind)? {
            ParsedBind::Binding(b) => bs.push(b),
            ParsedBind::FunClause(c) => push_fun_clause_binding(ts, &mut bs, &mut pending, c),
        }
        while matches!(ts.peek_kind(), Some(TokenKind::Semicolon)) {
            ts.bump();
            match parse_binding_or_fun_clause(ts, Stop::LetBind)? {
                ParsedBind::Binding(b) => {
                    flush_pending_fun_binding(ts, &mut bs, pending.take());
                    bs.push(b);
                }
                ParsedBind::FunClause(c) => push_fun_clause_binding(ts, &mut bs, &mut pending, c),
            }
        }
        flush_pending_fun_binding(ts, &mut bs, pending.take());
        bs
    };

    ts.expect(TokenKind::KwIn)?;
    let body = Box::new(parse_expr_after_newline(ts, stop)?);
    Ok(expr_from(ts, start, ast::ExprKind::Let { bindings, body }))
}

fn parse_do(ts: &mut TokenStream, _stop: Stop) -> Result<ast::Expr> {
    let start = ts.peek_span().map(|s| s.start).unwrap_or(0);
    ts.expect(TokenKind::KwDo)?;

    // do { stmt; stmt; ... }
    if matches!(ts.peek_kind(), Some(TokenKind::LBrace)) {
        ts.bump();
        let mut stmts = Vec::new();
        loop {
            if matches!(ts.peek_kind(), Some(TokenKind::RBrace)) {
                break;
            }
            if ts.is_eof() {
                return Err(ts.err_here("unexpected EOF in do"));
            }

            // Bind statement is `pat <- expr`.
            let save = (ts.i, ts.last_span_end);
            if let Ok(pat) = parse_pattern(ts) {
                if matches!(ts.peek_kind(), Some(TokenKind::LeftArrow)) {
                    ts.bump();
                    let expr = parse_expr(ts, Stop::SemiOrRBrace)?;
                    stmts.push(ast::DoStmt::Bind { pat, expr });
                } else {
                    (ts.i, ts.last_span_end) = save;
                    let expr = parse_expr(ts, Stop::SemiOrRBrace)?;
                    stmts.push(ast::DoStmt::Expr(expr));
                }
            } else {
                (ts.i, ts.last_span_end) = save;
                let expr = parse_expr(ts, Stop::SemiOrRBrace)?;
                stmts.push(ast::DoStmt::Expr(expr));
            }

            if matches!(ts.peek_kind(), Some(TokenKind::Semicolon)) {
                ts.bump();
            } else {
                break;
            }
        }
        ts.expect(TokenKind::RBrace)?;
        return Ok(expr_from(ts, start, ast::ExprKind::Do(stmts)));
    }

    // do\n  ... (indent block)
    if !matches!(ts.peek_kind(), Some(TokenKind::Newline)) {
        return Err(ts.err_here("expected newline after 'do'"));
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
            return Err(ts.err_here("unexpected EOF in do"));
        }

        // Bind statement is `pat <- expr`.
        let save = (ts.i, ts.last_span_end);
        if let Ok(pat) = parse_pattern(ts) {
            if matches!(ts.peek_kind(), Some(TokenKind::LeftArrow)) {
                ts.bump();
                let expr = parse_expr(ts, Stop::LineEnd)?;
                stmts.push(ast::DoStmt::Bind { pat, expr });
                ts.consume_line_end();
                continue;
            }
        }
        (ts.i, ts.last_span_end) = save;

        let expr = parse_expr(ts, Stop::LineEnd)?;
        stmts.push(ast::DoStmt::Expr(expr));
        ts.consume_line_end();
    }

    ts.expect(TokenKind::Dedent)?;
    ts.consume_line_end();

    Ok(expr_from(ts, start, ast::ExprKind::Do(stmts)))
}

fn parse_case(ts: &mut TokenStream, _stop: Stop) -> Result<ast::Expr> {
    let start = ts.peek_span().map(|s| s.start).unwrap_or(0);
    ts.expect(TokenKind::KwCase)?;
    let expr = Box::new(parse_expr(ts, Stop::Of)?);
    ts.expect(TokenKind::KwOf)?;

    // Support both:
    //   case e of\n  ... (indent block)
    // and inline:
    //   case e of pat -> expr; pat2 -> expr2
    if matches!(ts.peek_kind(), Some(TokenKind::Newline)) {
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
                return Err(ts.err_here("unexpected EOF in case"));
            }

            let mut pat = parse_cons_pattern(ts)?;

            // Disambiguation: prefer `or-pattern` when `| <pattern> ->` is possible;
            // otherwise treat it as a case guard `| <expr> ->`.
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

        return Ok(expr_from(ts, start, ast::ExprKind::Case { expr, arms }));
    }

    let mut arms = Vec::new();
    loop {
        if ts.is_eof() {
            return Err(ts.err_here("unexpected EOF in case"));
        }

        let mut pat = parse_cons_pattern(ts)?;

        // Disambiguation: prefer `or-pattern` when `| <pattern> ->` is possible;
        // otherwise treat it as a case guard `| <expr> ->`.
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

        let guard = if matches!(ts.peek_kind(), Some(TokenKind::Pipe)) {
            ts.bump();
            Some(parse_expr(ts, Stop::Pattern)?)
        } else {
            None
        };

        ts.expect(TokenKind::Arrow)?;
        let body = parse_expr(ts, Stop::SemiOrRBrace)?;
        arms.push(ast::CaseArm { pat, guard, body });

        if matches!(ts.peek_kind(), Some(TokenKind::Semicolon)) {
            ts.bump();
            continue;
        }
        break;
    }

    Ok(expr_from(ts, start, ast::ExprKind::Case { expr, arms }))
}

fn parse_annot(ts: &mut TokenStream, expr: ast::Expr, stop: Stop) -> Result<ast::Expr> {
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
                        ok = matches!(
                            ts.tokens.get(j + 1).map(|t| &t.kind),
                            Some(TokenKind::FatArrow)
                        );
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
    let start = expr.span.start;
    ts.expect(TokenKind::KwWhere)?;

    if matches!(ts.peek_kind(), Some(TokenKind::LBrace)) {
        ts.bump();
        let mut bindings = Vec::new();
        let mut pending: Option<PendingFun> = None;

        if !matches!(ts.peek_kind(), Some(TokenKind::RBrace)) {
            match parse_binding_or_fun_clause(ts, Stop::SemiOrRBrace)? {
                ParsedBind::Binding(b) => bindings.push(b),
                ParsedBind::FunClause(c) => {
                    push_fun_clause_binding(ts, &mut bindings, &mut pending, c)
                }
            }
            while matches!(ts.peek_kind(), Some(TokenKind::Semicolon)) {
                ts.bump();
                if matches!(ts.peek_kind(), Some(TokenKind::RBrace)) {
                    break;
                }
                match parse_binding_or_fun_clause(ts, Stop::SemiOrRBrace)? {
                    ParsedBind::Binding(b) => {
                        flush_pending_fun_binding(ts, &mut bindings, pending.take());
                        bindings.push(b);
                    }
                    ParsedBind::FunClause(c) => {
                        push_fun_clause_binding(ts, &mut bindings, &mut pending, c)
                    }
                }
            }
        }
        flush_pending_fun_binding(ts, &mut bindings, pending.take());

        ts.expect(TokenKind::RBrace)?;
        return Ok(expr_from(
            ts,
            start,
            ast::ExprKind::Where {
                expr: Box::new(expr),
                bindings,
            },
        ));
    }

    // Support both:
    //   expr where\n  ... (indent block)
    // and inline:
    //   expr where x = 1; y = 2
    if !matches!(ts.peek_kind(), Some(TokenKind::Newline)) {
        let mut bindings = Vec::new();
        let mut pending: Option<PendingFun> = None;

        match parse_binding_or_fun_clause(ts, Stop::LetBind)? {
            ParsedBind::Binding(b) => bindings.push(b),
            ParsedBind::FunClause(c) => push_fun_clause_binding(ts, &mut bindings, &mut pending, c),
        }
        while matches!(ts.peek_kind(), Some(TokenKind::Semicolon)) {
            ts.bump();
            match parse_binding_or_fun_clause(ts, Stop::LetBind)? {
                ParsedBind::Binding(b) => {
                    flush_pending_fun_binding(ts, &mut bindings, pending.take());
                    bindings.push(b);
                }
                ParsedBind::FunClause(c) => {
                    push_fun_clause_binding(ts, &mut bindings, &mut pending, c)
                }
            }
        }
        flush_pending_fun_binding(ts, &mut bindings, pending.take());

        return Ok(expr_from(
            ts,
            start,
            ast::ExprKind::Where {
                expr: Box::new(expr),
                bindings,
            },
        ));
    }

    ts.consume_line_end();
    ts.skip_newlines();
    ts.expect(TokenKind::Indent)?;

    let mut bindings = Vec::new();
    let mut pending: Option<PendingFun> = None;
    loop {
        ts.skip_newlines();
        if matches!(ts.peek_kind(), Some(TokenKind::Dedent)) {
            break;
        }
        if ts.is_eof() {
            return Err(ts.err_here("unexpected EOF in where"));
        }
        match parse_binding_or_fun_clause(ts, Stop::LineEnd)? {
            ParsedBind::Binding(b) => {
                flush_pending_fun_binding(ts, &mut bindings, pending.take());
                bindings.push(b);
            }
            ParsedBind::FunClause(c) => push_fun_clause_binding(ts, &mut bindings, &mut pending, c),
        }
        ts.consume_line_end();
    }
    flush_pending_fun_binding(ts, &mut bindings, pending.take());

    ts.expect(TokenKind::Dedent)?;
    ts.consume_line_end();

    Ok(expr_from(
        ts,
        start,
        ast::ExprKind::Where {
            expr: Box::new(expr),
            bindings,
        },
    ))
}

fn parse_pattern(ts: &mut TokenStream) -> Result<ast::Pattern> {
    parse_or_pattern(ts)
}

fn parse_or_pattern(ts: &mut TokenStream) -> Result<ast::Pattern> {
    let mut pat = parse_cons_pattern(ts)?;
    while matches!(ts.peek_kind(), Some(TokenKind::Pipe)) {
        ts.bump();
        let rhs = parse_cons_pattern(ts)?;
        let start = pat.span.start;
        pat = pat_from(
            ts,
            start,
            ast::PatternKind::Or(Box::new(pat), Box::new(rhs)),
        );
    }
    Ok(pat)
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

fn parse_pattern_atom(ts: &mut TokenStream) -> Result<ast::Pattern> {
    let start = ts.peek_span().map(|s| s.start).unwrap_or(0);
    match ts.peek_kind() {
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
            let lit = expr_from(ts, start, ast::ExprKind::Bool(true));
            Ok(pat_from(ts, start, ast::PatternKind::Literal(lit)))
        }
        Some(TokenKind::False) => {
            ts.bump();
            let lit = expr_from(ts, start, ast::ExprKind::Bool(false));
            Ok(pat_from(ts, start, ast::PatternKind::Literal(lit)))
        }
        Some(TokenKind::Integer(_)) => match ts.bump() {
            Some(TokenKind::Integer(s)) => {
                let lit = expr_from(ts, start, ast::ExprKind::Integer(s));
                Ok(pat_from(ts, start, ast::PatternKind::Literal(lit)))
            }
            _ => unreachable!(),
        },
        Some(TokenKind::Float(_)) => match ts.bump() {
            Some(TokenKind::Float(s)) => {
                let lit = expr_from(ts, start, ast::ExprKind::Float64(s));
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
                let lit = expr_from(ts, start, ast::ExprKind::Char(ch));
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
        let unit = expr_from(ts, start, ast::ExprKind::Unit);
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
        return Ok(pat_from(ts, start, ast::PatternKind::List(Vec::new())));
    }

    let mut elems = Vec::new();
    elems.push(parse_pattern(ts)?);
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
        return Ok(pat_from(ts, start, ast::PatternKind::Record(Vec::new())));
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
                    return Err(ts.err_here("...rest must be a variable"));
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
        pat_from(ts, start, ast::PatternKind::RecordLoose(fields, rest))
    } else {
        pat_from(ts, start, ast::PatternKind::Record(fields))
    })
}

fn parse_infix_application(ts: &mut TokenStream, stop: Stop) -> Result<ast::Expr> {
    parse_binops(ts, stop, 0)
}

fn parse_binops(ts: &mut TokenStream, stop: Stop, min_prec: u8) -> Result<ast::Expr> {
    let mut lhs = parse_application(ts, stop)?;

    while ts.can_continue_expr(stop.to_token_stream()) {
        let save = (ts.i, ts.last_span_end);
        let is_cons = matches!(ts.peek_kind(), Some(TokenKind::Colon));

        let (op, fixity) = match ts.peek_kind() {
            Some(TokenKind::Backtick) => {
                ts.expect(TokenKind::Backtick)?;
                let op = ts.expect_ident()?;
                ts.expect(TokenKind::Backtick)?;
                (op.clone(), ts.fixity(&op))
            }
            Some(TokenKind::Operator(op)) => {
                let op = op.clone();
                ts.bump();
                (op.clone(), ts.fixity(&op))
            }
            Some(TokenKind::Star) => {
                ts.bump();
                ("*".to_string(), ts.fixity("*"))
            }
            Some(TokenKind::Slash) => {
                ts.bump();
                ("/".to_string(), ts.fixity("/"))
            }
            Some(TokenKind::Plus) => {
                ts.bump();
                ("+".to_string(), ts.fixity("+"))
            }
            Some(TokenKind::Minus) => {
                ts.bump();
                ("-".to_string(), ts.fixity("-"))
            }
            Some(TokenKind::PlusPlus) => {
                ts.bump();
                ("++".to_string(), ts.fixity("++"))
            }
            Some(TokenKind::Colon) => {
                ts.bump();
                (
                    ":".to_string(),
                    Fixity {
                        prec: 55,
                        assoc: Assoc::Right,
                    },
                )
            }
            Some(TokenKind::EqEq) => {
                ts.bump();
                ("==".to_string(), ts.fixity("=="))
            }
            Some(TokenKind::SlashEq) => {
                ts.bump();
                ("/=".to_string(), ts.fixity("/="))
            }
            Some(TokenKind::Lt) => {
                ts.bump();
                ("<".to_string(), ts.fixity("<"))
            }
            Some(TokenKind::Le) => {
                ts.bump();
                ("<=".to_string(), ts.fixity("<="))
            }
            Some(TokenKind::Gt) => {
                ts.bump();
                (">".to_string(), ts.fixity(">"))
            }
            Some(TokenKind::Ge) => {
                ts.bump();
                (">=".to_string(), ts.fixity(">="))
            }
            Some(TokenKind::GtGt) => {
                ts.bump();
                (">>".to_string(), ts.fixity(">>"))
            }
            Some(TokenKind::GtGtEq) => {
                ts.bump();
                (">>=".to_string(), ts.fixity(">>="))
            }
            Some(TokenKind::AndAnd) => {
                ts.bump();
                ("&&".to_string(), ts.fixity("&&"))
            }
            Some(TokenKind::OrOr) => {
                ts.bump();
                ("||".to_string(), ts.fixity("||"))
            }
            _ => break,
        };

        if fixity.prec < min_prec {
            (ts.i, ts.last_span_end) = save;
            break;
        }

        let rhs_min_prec = if is_cons {
            fixity.prec
        } else {
            match fixity.assoc {
                Assoc::Right => fixity.prec,
                _ => fixity.prec + 1,
            }
        };

        let rhs = parse_binops(ts, stop, rhs_min_prec)?;
        let start = lhs.span.start;
        lhs = if is_cons {
            expr_from(
                ts,
                start,
                ast::ExprKind::Cons {
                    head: Box::new(lhs),
                    tail: Box::new(rhs),
                },
            )
        } else {
            let func_kind = op_expr_kind(op);
            expr_from(
                ts,
                start,
                ast::ExprKind::Apply {
                    func: Box::new(ast::Expr::dummy(func_kind)),
                    args: vec![lhs, rhs],
                },
            )
        };
    }

    Ok(lhs)
}

fn parse_application(ts: &mut TokenStream, stop: Stop) -> Result<ast::Expr> {
    let mut exprs = Vec::new();
    exprs.push(parse_atom(ts)?);

    while ts.can_continue_expr(stop.to_token_stream()) {
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
        let start = func.span.start;
        Ok(expr_from(
            ts,
            start,
            ast::ExprKind::Apply { func, args: exprs },
        ))
    }
}

fn parse_atom(ts: &mut TokenStream) -> Result<ast::Expr> {
    let start = ts.peek_span().map(|s| s.start).unwrap_or(0);
    match ts.peek_kind() {
        Some(TokenKind::True) => {
            ts.bump();
            Ok(expr_from(ts, start, ast::ExprKind::Bool(true)))
        }
        Some(TokenKind::False) => {
            ts.bump();
            Ok(expr_from(ts, start, ast::ExprKind::Bool(false)))
        }
        Some(TokenKind::Integer(_)) => match ts.bump() {
            Some(TokenKind::Integer(s)) => Ok(expr_from(ts, start, ast::ExprKind::Integer(s))),
            _ => unreachable!(),
        },
        Some(TokenKind::Float(_)) => match ts.bump() {
            Some(TokenKind::Float(s)) => Ok(expr_from(ts, start, ast::ExprKind::Float64(s))),
            _ => unreachable!(),
        },
        Some(TokenKind::String(_)) => match ts.bump() {
            Some(TokenKind::String(s)) => {
                // Desugar string literal expressions into list-of-char expressions.
                // This aligns with Haskell surface semantics where String ~ [Char].
                let es = s
                    .chars()
                    .map(|ch| ast::Expr::dummy(ast::ExprKind::Char(ch)))
                    .collect::<Vec<_>>();
                Ok(expr_from(ts, start, ast::ExprKind::List(es)))
            }
            _ => unreachable!(),
        },
        Some(TokenKind::Char(_)) => match ts.bump() {
            Some(TokenKind::Char(ch)) => Ok(expr_from(ts, start, ast::ExprKind::Char(ch))),
            _ => unreachable!(),
        },
        Some(TokenKind::Ident(_)) => {
            let s = parse_maybe_qualified_ident(ts)?;
            if is_upper_by_last_segment(&s) {
                Ok(expr_from(ts, start, ast::ExprKind::Ctor(s)))
            } else {
                Ok(expr_from(ts, start, ast::ExprKind::Var(s)))
            }
        }
        Some(TokenKind::LBracket) => parse_list_expr(ts),
        Some(TokenKind::LParen) => parse_paren_or_tuple_expr(ts),
        Some(TokenKind::LBrace) => parse_record_expr(ts),
        _ => Err(ts.err_here("expected expression")),
    }
}

fn parse_paren_or_tuple_expr(ts: &mut TokenStream) -> Result<ast::Expr> {
    let start = ts.peek_span().map(|s| s.start).unwrap_or(0);
    ts.expect(TokenKind::LParen)?;

    if matches!(ts.peek_kind(), Some(TokenKind::RParen)) {
        ts.bump();
        return Ok(expr_from(ts, start, ast::ExprKind::Unit));
    }

    // Sections + operator prefixification:
    //   (op)      => op
    //   (op x)    => \a -> a `op` x
    //   (x op)    => \a -> x `op` a
    {
        let save = (ts.i, ts.last_span_end);
        if matches!(ts.peek_kind(), Some(TokenKind::Backtick)) || is_sym_op_token(ts.peek_kind()) {
            let op = if matches!(ts.peek_kind(), Some(TokenKind::Backtick)) {
                ts.expect(TokenKind::Backtick)?;
                let op = ts.expect_ident()?;
                ts.expect(TokenKind::Backtick)?;
                op
            } else {
                parse_fixity_op(ts)?
            };

            if matches!(ts.peek_kind(), Some(TokenKind::RParen)) {
                ts.bump();
                if op == ":" {
                    let a = ts.fresh_name("__cons_a");
                    let b = ts.fresh_name("__cons_b");
                    let body = ast::Expr::dummy(ast::ExprKind::Cons {
                        head: Box::new(ast::Expr::dummy(ast::ExprKind::Var(a.clone()))),
                        tail: Box::new(ast::Expr::dummy(ast::ExprKind::Var(b.clone()))),
                    });
                    return Ok(expr_from(
                        ts,
                        start,
                        ast::ExprKind::Lambda {
                            params: vec![a, b],
                            body: Box::new(body),
                        },
                    ));
                }
                return Ok(expr_from(ts, start, op_expr_kind(op)));
            }

            let rhs = parse_expr(ts, Stop::LineEnd)?;
            ts.expect(TokenKind::RParen)?;
            let param = ts.fresh_name("__section");
            let body = if op == ":" {
                ast::Expr::dummy(ast::ExprKind::Cons {
                    head: Box::new(ast::Expr::dummy(ast::ExprKind::Var(param.clone()))),
                    tail: Box::new(rhs),
                })
            } else {
                ast::Expr::dummy(ast::ExprKind::Apply {
                    func: Box::new(ast::Expr::dummy(op_expr_kind(op))),
                    args: vec![ast::Expr::dummy(ast::ExprKind::Var(param.clone())), rhs],
                })
            };
            return Ok(expr_from(
                ts,
                start,
                ast::ExprKind::Lambda {
                    params: vec![param],
                    body: Box::new(body),
                },
            ));
        }
        (ts.i, ts.last_span_end) = save;
    }

    {
        let save = (ts.i, ts.last_span_end);
        if let Ok(lhs) = parse_application(ts, Stop::LineEnd) {
            if matches!(ts.peek_kind(), Some(TokenKind::Backtick))
                || is_sym_op_token(ts.peek_kind())
            {
                let op = if matches!(ts.peek_kind(), Some(TokenKind::Backtick)) {
                    ts.expect(TokenKind::Backtick)?;
                    let op = ts.expect_ident()?;
                    ts.expect(TokenKind::Backtick)?;
                    op
                } else {
                    parse_fixity_op(ts)?
                };

                if matches!(ts.peek_kind(), Some(TokenKind::RParen)) {
                    ts.bump();
                    let param = ts.fresh_name("__section");
                    let body = if op == ":" {
                        ast::Expr::dummy(ast::ExprKind::Cons {
                            head: Box::new(lhs),
                            tail: Box::new(ast::Expr::dummy(ast::ExprKind::Var(param.clone()))),
                        })
                    } else {
                        ast::Expr::dummy(ast::ExprKind::Apply {
                            func: Box::new(ast::Expr::dummy(op_expr_kind(op))),
                            args: vec![lhs, ast::Expr::dummy(ast::ExprKind::Var(param.clone()))],
                        })
                    };
                    return Ok(expr_from(
                        ts,
                        start,
                        ast::ExprKind::Lambda {
                            params: vec![param],
                            body: Box::new(body),
                        },
                    ));
                }
            }
        }
        (ts.i, ts.last_span_end) = save;
    }

    let first = parse_expr(ts, Stop::LineEnd)?;
    if matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
        let mut elems = vec![first];
        while matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
            ts.bump();
            elems.push(parse_expr(ts, Stop::LineEnd)?);
        }
        ts.expect(TokenKind::RParen)?;
        Ok(expr_from(ts, start, ast::ExprKind::Tuple(elems)))
    } else {
        ts.expect(TokenKind::RParen)?;
        let ast::Expr { kind, .. } = first;
        Ok(expr_from(ts, start, kind))
    }
}

fn parse_list_expr(ts: &mut TokenStream) -> Result<ast::Expr> {
    let start = ts.peek_span().map(|s| s.start).unwrap_or(0);
    ts.expect(TokenKind::LBracket)?;

    if matches!(ts.peek_kind(), Some(TokenKind::RBracket)) {
        ts.bump();
        return Ok(expr_from(ts, start, ast::ExprKind::List(Vec::new())));
    }

    let first = parse_expr(ts, Stop::Pattern)?;

    // List ranges (Enum-based desugar)
    // - [a..b]      => enumFromTo a b
    // - [a..]       => enumFrom a
    // - [a,b..c]    => enumFromThenTo a b c
    // - [a,b..]     => enumFromThen a b
    if matches!(ts.peek_kind(), Some(TokenKind::Operator(op)) if op == "..") {
        ts.bump();

        if matches!(ts.peek_kind(), Some(TokenKind::RBracket)) {
            ts.bump();
            let kind = ast::ExprKind::Apply {
                func: Box::new(ast::Expr::dummy(ast::ExprKind::Var("enumFrom".to_string()))),
                args: vec![first],
            };
            return Ok(expr_from(ts, start, kind));
        }

        let end = parse_expr(ts, Stop::Pattern)?;
        ts.expect(TokenKind::RBracket)?;

        let kind = ast::ExprKind::Apply {
            func: Box::new(ast::Expr::dummy(ast::ExprKind::Var(
                "enumFromTo".to_string(),
            ))),
            args: vec![first, end],
        };
        return Ok(expr_from(ts, start, kind));
    }

    // Step ranges start with a comma.
    if matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
        let save = (ts.i, ts.last_span_end);
        ts.bump();
        let second = parse_expr(ts, Stop::Pattern)?;
        if matches!(ts.peek_kind(), Some(TokenKind::Operator(op)) if op == "..") {
            ts.bump();

            if matches!(ts.peek_kind(), Some(TokenKind::RBracket)) {
                ts.bump();
                let kind = ast::ExprKind::Apply {
                    func: Box::new(ast::Expr::dummy(ast::ExprKind::Var(
                        "enumFromThen".to_string(),
                    ))),
                    args: vec![first, second],
                };
                return Ok(expr_from(ts, start, kind));
            }

            let end = parse_expr(ts, Stop::Pattern)?;
            ts.expect(TokenKind::RBracket)?;
            let kind = ast::ExprKind::Apply {
                func: Box::new(ast::Expr::dummy(ast::ExprKind::Var(
                    "enumFromThenTo".to_string(),
                ))),
                args: vec![first, second, end],
            };
            return Ok(expr_from(ts, start, kind));
        }

        // Not a step range; rewind and parse as a normal list literal.
        (ts.i, ts.last_span_end) = save;
    }

    // List comprehension: [ expr | generator_list ]
    if matches!(ts.peek_kind(), Some(TokenKind::Pipe)) {
        ts.bump();

        enum Gen {
            Bind(ast::Pattern, ast::Expr),
            Guard(ast::Expr),
        }

        let mut gens = Vec::new();
        loop {
            let save = (ts.i, ts.last_span_end);
            if let Ok(pat) = parse_pattern(ts) {
                if matches!(ts.peek_kind(), Some(TokenKind::LeftArrow)) {
                    ts.bump();
                    let rhs = parse_expr(ts, Stop::Pattern)?;
                    gens.push(Gen::Bind(pat, rhs));
                } else {
                    (ts.i, ts.last_span_end) = save;
                    let e = parse_expr(ts, Stop::Pattern)?;
                    gens.push(Gen::Guard(e));
                }
            } else {
                (ts.i, ts.last_span_end) = save;
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

        let mut out = ast::Expr::dummy(ast::ExprKind::List(vec![first]));
        for g in gens.into_iter().rev() {
            out = ast::Expr::dummy(match g {
                Gen::Guard(cond) => ast::ExprKind::If {
                    cond: Box::new(cond),
                    then_branch: Box::new(out),
                    else_branch: Box::new(ast::Expr::dummy(ast::ExprKind::List(Vec::new()))),
                },
                Gen::Bind(pat, xs) => match pat.kind {
                    ast::PatternKind::Var(name) => ast::ExprKind::Apply {
                        func: Box::new(ast::Expr::dummy(ast::ExprKind::Var(
                            "concatMap".to_string(),
                        ))),
                        args: vec![
                            ast::Expr::dummy(ast::ExprKind::Lambda {
                                params: vec![name],
                                body: Box::new(out),
                            }),
                            xs,
                        ],
                    },
                    ast::PatternKind::Wildcard => ast::ExprKind::Apply {
                        func: Box::new(ast::Expr::dummy(ast::ExprKind::Var(
                            "concatMap".to_string(),
                        ))),
                        args: vec![
                            ast::Expr::dummy(ast::ExprKind::Lambda {
                                params: vec!["_".to_string()],
                                body: Box::new(out),
                            }),
                            xs,
                        ],
                    },
                    other_kind => {
                        let other_pat = ast::Pattern {
                            kind: other_kind,
                            span: pat.span,
                        };
                        let tmp = ts.fresh_name("_lc");
                        ast::ExprKind::Apply {
                            func: Box::new(ast::Expr::dummy(ast::ExprKind::Var(
                                "concatMap".to_string(),
                            ))),
                            args: vec![
                                ast::Expr::dummy(ast::ExprKind::Lambda {
                                    params: vec![tmp.clone()],
                                    body: Box::new(ast::Expr::dummy(ast::ExprKind::Case {
                                        expr: Box::new(ast::Expr::dummy(ast::ExprKind::Var(tmp))),
                                        arms: vec![
                                            ast::CaseArm {
                                                pat: other_pat,
                                                guard: None,
                                                body: out,
                                            },
                                            ast::CaseArm {
                                                pat: ast::Pattern::dummy(
                                                    ast::PatternKind::Wildcard,
                                                ),
                                                guard: None,
                                                body: ast::Expr::dummy(ast::ExprKind::List(
                                                    Vec::new(),
                                                )),
                                            },
                                        ],
                                    })),
                                }),
                                xs,
                            ],
                        }
                    }
                },
            });
        }

        let ast::Expr { kind, .. } = out;
        return Ok(expr_from(ts, start, kind));
    }

    // List literal: [e1, e2, ...]
    let mut elems = vec![first];
    while matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
        ts.bump();
        elems.push(parse_expr(ts, Stop::Pattern)?);
    }

    ts.expect(TokenKind::RBracket)?;
    Ok(expr_from(ts, start, ast::ExprKind::List(elems)))
}

fn parse_record_expr(ts: &mut TokenStream) -> Result<ast::Expr> {
    let start = ts.peek_span().map(|s| s.start).unwrap_or(0);
    ts.expect(TokenKind::LBrace)?;

    if matches!(ts.peek_kind(), Some(TokenKind::RBrace)) {
        ts.bump();
        return Ok(expr_from(ts, start, ast::ExprKind::Record(Vec::new())));
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
    Ok(expr_from(ts, start, ast::ExprKind::Record(fields)))
}
