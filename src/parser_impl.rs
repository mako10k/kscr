use crate::{ast, error::Error, lexer, lexer::TokenKind, Result};
use std::collections::HashMap;

use crate::parser::token_stream::{self, compute_line_starts, Assoc, Fixity, TokenStream};

#[path = "parser_impl/type_expr.rs"]
mod type_expr;

#[path = "parser_impl/pattern.rs"]
mod pattern;

pub fn parse_module(src: &str) -> Result<ast::Module> {
    let tokens = lexer::lex(src)?;
    let fixities = node_collect_fixities(&tokens);
    let line_starts = compute_line_starts(src);
    let mut ts = TokenStream::new(tokens, fixities, line_starts);
    ts.skip_newlines();

    if matches!(ts.peek_kind(), Some(TokenKind::KwModule)) {
        node_parse_module_decl(&mut ts)
    } else {
        let items = node_parse_items_until(&mut ts, StopAt::Eof)?;
        Ok(ast::Module { name: None, items })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StopAt {
    Dedent,
    Eof,
}

fn is_stop_at_bound(ts: &TokenStream, stop_at: StopAt) -> bool {
    // When parsing a top-level item list, a trailing layout `Dedent` can appear
    // right at EOF. Treat it as EOF here too.
    if matches!(stop_at, StopAt::Eof) && matches!(ts.peek_kind(), Some(TokenKind::Dedent)) {
        return true;
    }
    if ts.is_eof() {
        return true;
    }
    matches!(stop_at, StopAt::Dedent) && matches!(ts.peek_kind(), Some(TokenKind::Dedent))
}

fn should_skip_newlines(ts: &TokenStream, stop_at: StopAt) -> bool {
    // When ending a layout block at EOF, the lexer can emit a bare `Dedent`.
    // Don't skip over it here; allow the caller to see it.
    !(matches!(stop_at, StopAt::Dedent) && matches!(ts.peek_kind(), Some(TokenKind::Dedent)))
}

fn err_if_sig_would_cross_decl(
    ts: &TokenStream,
    signature_buf: &HashMap<String, ast::QualType>,
) -> Result<()> {
    if signature_buf.is_empty() {
        return Ok(());
    }
    match ts.peek_kind() {
        Some(TokenKind::KwImport)
        | Some(TokenKind::KwExport)
        | Some(TokenKind::KwInfix)
        | Some(TokenKind::KwInfixl)
        | Some(TokenKind::KwInfixr)
        | Some(TokenKind::KwData)
        | Some(TokenKind::KwType)
        | Some(TokenKind::KwClass)
        | Some(TokenKind::KwInstance) => {
            Err(ts.err_here("type signature must be followed by a binding"))
        }
        _ => Ok(()),
    }
}

fn try_consume_sig_line(
    ts: &mut TokenStream,
    signature_buf: &mut HashMap<String, ast::QualType>,
) -> Result<bool> {
    let save = (ts.i, ts.last_span_end);
    if let Some((name, ty)) = try_parse_toplevel_sig_line(ts)? {
        signature_buf.insert(name, ty);
        return Ok(true);
    }
    (ts.i, ts.last_span_end) = save;
    Ok(false)
}

fn collect_fixities(tokens: &[lexer::Token]) -> HashMap<String, Fixity> {
    // Default fixities.
    let mut m: HashMap<String, Fixity> = HashMap::new();
    m.insert(
        "*".to_string(),
        Fixity {
            prec: 70,
            assoc: Assoc::Left,
        },
    );
    m.insert(
        "/".to_string(),
        Fixity {
            prec: 70,
            assoc: Assoc::Left,
        },
    );
    m.insert(
        "+".to_string(),
        Fixity {
            prec: 60,
            assoc: Assoc::Left,
        },
    );
    m.insert(
        "-".to_string(),
        Fixity {
            prec: 60,
            assoc: Assoc::Left,
        },
    );
    m.insert(
        "++".to_string(),
        Fixity {
            prec: 60,
            assoc: Assoc::Left,
        },
    );
    m.insert(
        "==".to_string(),
        Fixity {
            prec: 50,
            assoc: Assoc::Left,
        },
    );
    m.insert(
        "!=".to_string(),
        Fixity {
            prec: 50,
            assoc: Assoc::Left,
        },
    );
    m.insert(
        "<".to_string(),
        Fixity {
            prec: 50,
            assoc: Assoc::Left,
        },
    );
    m.insert(
        "<=".to_string(),
        Fixity {
            prec: 50,
            assoc: Assoc::Left,
        },
    );
    m.insert(
        ">".to_string(),
        Fixity {
            prec: 50,
            assoc: Assoc::Left,
        },
    );
    m.insert(
        ">=".to_string(),
        Fixity {
            prec: 50,
            assoc: Assoc::Left,
        },
    );
    m.insert(
        "&&".to_string(),
        Fixity {
            prec: 40,
            assoc: Assoc::Left,
        },
    );
    m.insert(
        "||".to_string(),
        Fixity {
            prec: 30,
            assoc: Assoc::Left,
        },
    );

    // NOTE: Full fixity decl parsing happens during module parsing.
    // Here we just provide defaults so the token stream can parse expressions.
    // This is consistent with older behavior.
    let _ = tokens;
    m
}

fn node_collect_fixities(tokens: &[lexer::Token]) -> HashMap<String, Fixity> {
    collect_fixities(tokens)
}

fn node_parse_module_decl(ts: &mut TokenStream) -> Result<ast::Module> {
    parse_module_decl(ts)
}

fn node_parse_items_until(ts: &mut TokenStream, stop: StopAt) -> Result<Vec<ast::Item>> {
    parse_items_until(ts, stop)
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

fn parse_items_until(ts: &mut TokenStream, stop_at: StopAt) -> Result<Vec<ast::Item>> {
    let mut items = Vec::new();
    let mut pending: Option<PendingFun> = None;
    let mut signature_buf: HashMap<String, ast::QualType> = HashMap::new();

    let flush_pending_fun = |ts: &mut TokenStream,
                             items: &mut Vec<ast::Item>,
                             pending: &mut Option<PendingFun>,
                             signature_buf: &mut HashMap<String, ast::QualType>|
     -> Result<()> {
        if let Some(p) = pending.take() {
            let mut b = desugar_fun(ts, p.name, p.arity, p.clauses);
            if let ast::PatternKind::Var(def_name) = &b.pat.kind {
                if let Some(sig_ty) = signature_buf.remove(def_name) {
                    let span = b.expr.span;
                    b.expr = ast::Expr::new(
                        span,
                        ast::ExprKind::Annot {
                            expr: Box::new(b.expr),
                            ty: sig_ty,
                        },
                    );
                }
            }
            items.push(ast::Item::Binding(b));
        }
        Ok(())
    };

    loop {
        if is_stop_at_bound(ts, stop_at) {
            break;
        }
        if should_skip_newlines(ts, stop_at) {
            ts.skip_newlines();
        }
        if is_stop_at_bound(ts, stop_at) {
            break;
        }
        err_if_sig_would_cross_decl(ts, &signature_buf)?;

        let tok = ts.peek_kind().cloned();
        match tok {
            Some(TokenKind::KwImport) => {
                flush_pending_fun(ts, &mut items, &mut pending, &mut signature_buf)?;
                items.push(parse_import_decl(ts)?);
            }
            Some(TokenKind::KwExport) => {
                flush_pending_fun(ts, &mut items, &mut pending, &mut signature_buf)?;
                items.push(parse_export_decl(ts)?);
            }
            Some(TokenKind::KwInfix) | Some(TokenKind::KwInfixl) | Some(TokenKind::KwInfixr) => {
                flush_pending_fun(ts, &mut items, &mut pending, &mut signature_buf)?;
                items.push(parse_fixity_decl(ts)?);
            }
            Some(TokenKind::KwData) => {
                flush_pending_fun(ts, &mut items, &mut pending, &mut signature_buf)?;
                items.push(parse_data_decl(ts)?);
            }
            Some(TokenKind::KwType) => {
                flush_pending_fun(ts, &mut items, &mut pending, &mut signature_buf)?;
                items.push(parse_type_alias(ts)?);
            }
            Some(TokenKind::KwClass) => {
                flush_pending_fun(ts, &mut items, &mut pending, &mut signature_buf)?;
                items.push(parse_class_decl(ts)?);
            }
            Some(TokenKind::KwInstance) => {
                flush_pending_fun(ts, &mut items, &mut pending, &mut signature_buf)?;
                items.push(parse_instance_decl(ts)?);
            }
            Some(TokenKind::Ident(_))
            | Some(TokenKind::LParen)
            | Some(TokenKind::Question)
            | Some(TokenKind::LBrace) => {
                // Either a signature line `x :: ...` or a binding/fun-clause.
                if try_consume_sig_line(ts, &mut signature_buf)? {
                    continue;
                }

                match parse_binding_or_fun_clause(ts, Stop::LineEnd)? {
                    ParsedBind::Binding(b) => {
                        flush_pending_fun(ts, &mut items, &mut pending, &mut signature_buf)?;
                        items.push(ast::Item::Binding(b));
                    }
                    ParsedBind::FunClause(c) => {
                        push_fun_clause_item(ts, &mut items, &mut pending, c)?;
                    }
                }
            }
            Some(TokenKind::Newline) => {
                ts.bump();
                continue;
            }
            Some(TokenKind::Dedent) if matches!(stop_at, StopAt::Dedent) => {
                break;
            }
            Some(_) => {
                return Err(ts.err_here("unexpected token at top level"));
            }
            None => break,
        }

        ts.consume_line_end();
    }

    flush_pending_fun(ts, &mut items, &mut pending, &mut signature_buf)?;
    Ok(items)
}

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

fn try_parse_toplevel_sig_line(ts: &mut TokenStream) -> Result<Option<(String, ast::QualType)>> {
    let save = (ts.i, ts.last_span_end);
    let Ok(name) = ts.expect_ident() else {
        return Ok(None);
    };
    if !matches!(ts.peek_kind(), Some(TokenKind::ColonColon)) {
        (ts.i, ts.last_span_end) = save;
        return Ok(None);
    }
    ts.expect(TokenKind::ColonColon)?;
    let ty = parse_qual_type(ts, Stop::LineEnd)?;
    Ok(Some((name, ty)))
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
        // If that fails, accept infix ctor: `a :*: b`.
        let save = (ts.i, ts.last_span_end);
        let parsed = if let Ok(ctor_name) = parse_ctor_name(ts) {
            let mut args = Vec::new();
            while matches!(ts.peek_kind(), Some(TokenKind::Ident(s)) if s != "deriving")
                || matches!(
                    ts.peek_kind(),
                    Some(TokenKind::LParen) | Some(TokenKind::LBracket) | Some(TokenKind::LBrace)
                )
            {
                args.push(parse_type_atom(ts, Stop::LineEnd, is_type_alias_end)?);
            }
            Some(ast::DataCtor {
                name: ctor_name,
                args,
            })
        } else {
            (ts.i, ts.last_span_end) = save;
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

fn parse_class_supers(ts: &mut TokenStream) -> Result<Vec<ast::Predicate>> {
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
        return Ok(supers);
    }

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
    Ok(supers)
}

fn parse_class_method_sig_line(ts: &mut TokenStream) -> Result<Option<ast::ClassMethodSig>> {
    // Try parsing a signature line first:
    //   f :: ...
    //   (++) :: ...
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
            ts.consume_line_end();
            return Ok(Some(ast::ClassMethodSig { name: mname, ty }));
        }
    }

    (ts.i, ts.last_span_end) = save;
    Ok(None)
}

fn class_body_allows_default_item(ts: &TokenStream) -> bool {
    matches!(
        ts.peek_kind(),
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
        )
    )
}

fn parse_class_body(ts: &mut TokenStream) -> Result<(Vec<ast::ClassMethodSig>, Vec<ast::Binding>)> {
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

        if let Some(sig) = parse_class_method_sig_line(ts)? {
            methods.push(sig);
            continue;
        }

        if class_body_allows_default_item(ts) {
            match parse_binding_or_fun_clause(ts, Stop::LineEnd)? {
                ParsedBind::Binding(b) => {
                    flush_pending_fun_item(ts, &mut default_items, pending.take())?;
                    default_items.push(ast::Item::Binding(b));
                }
                ParsedBind::FunClause(c) => {
                    push_fun_clause_item(ts, &mut default_items, &mut pending, c)?;
                }
            }
            ts.consume_line_end();
            continue;
        }

        match ts.peek_kind() {
            Some(_) => return Err(ts.err_here("unexpected token in class")),
            None => break,
        }
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

    Ok((methods, default_methods))
}

fn parse_class_decl(ts: &mut TokenStream) -> Result<ast::Item> {
    ts.expect(TokenKind::KwClass)?;

    let supers = parse_class_supers(ts)?;

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

    let (methods, default_methods) = parse_class_body(ts)?;

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

    // Optional class context (single or parenthesized list) before `=>`.
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
    let pat = pattern::parse_pattern(ts)?;

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
                ast::ExprKind::Ctor(ast::ResolvedName::Unresolved(op))
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
    // NOTE: signature lines (`name :: T`) are handled in `parse_items_until` so we don't
    // accidentally consume tokens that should belong to the following binding/clause.
    // `fname pat1 pat2 = body` / guarded: `fname pat1 | guard = body`
    // Operator forms supported:
    // - `(++) a b = ...`
    // - `a ++ b = ...`
    // Disambiguation: reject pattern-bind continuations like `x:xs = ...`.

    if let Some(parsed) = try_parse_funclause_paren_op(ts, stop)? {
        return Ok(parsed);
    }
    if let Some(parsed) = try_parse_funclause_infix_op(ts, stop)? {
        return Ok(parsed);
    }
    if let Some(parsed) = try_parse_funclause_prefix_ident(ts, stop)? {
        return Ok(parsed);
    }
    Ok(ParsedBind::Binding(parse_binding_simple(ts, stop)?))
}

fn try_parse_funclause_paren_op(ts: &mut TokenStream, stop: Stop) -> Result<Option<ParsedBind>> {
    if !matches!(ts.peek_kind(), Some(TokenKind::LParen)) {
        return Ok(None);
    }

    let save = (ts.i, ts.last_span_end);
    let name = match parse_paren_operator_name(ts) {
        Ok(name) => name,
        Err(_) => {
            (ts.i, ts.last_span_end) = save;
            return Ok(None);
        }
    };
    if is_ctor_symbol(&name) {
        return Err(ts.err_here("operators starting with ':' are constructors"));
    }

    let mut args = Vec::new();
    while can_start_pattern_atom(ts.peek_kind()) {
        args.push(pattern::parse_pattern(ts)?);
    }
    if args.is_empty() {
        (ts.i, ts.last_span_end) = save;
        return Ok(None);
    }

    let (guard, body) = parse_guard_and_body(ts, stop)?;
    Ok(Some(ParsedBind::FunClause(FunClause {
        name,
        args,
        guard,
        body,
    })))
}

fn try_parse_funclause_infix_op(ts: &mut TokenStream, stop: Stop) -> Result<Option<ParsedBind>> {
    let save = (ts.i, ts.last_span_end);
    let lhs_pat = match pattern::parse_pattern(ts) {
        Ok(p) => p,
        Err(_) => {
            (ts.i, ts.last_span_end) = save;
            return Ok(None);
        }
    };
    if !(is_sym_op_token(ts.peek_kind()) || matches!(ts.peek_kind(), Some(TokenKind::Backtick))) {
        (ts.i, ts.last_span_end) = save;
        return Ok(None);
    }

    let op = parse_operator_name(ts)?;
    if is_ctor_symbol(&op) {
        // `x:xs = ...` is a pattern binding, not an infix fun clause.
        (ts.i, ts.last_span_end) = save;
        return Ok(None);
    }
    if is_upper_by_last_segment(&op) {
        (ts.i, ts.last_span_end) = save;
        return Ok(None);
    }

    let rhs_pat = pattern::parse_pattern(ts)?;
    let (guard, body) = parse_guard_and_body(ts, stop)?;
    Ok(Some(ParsedBind::FunClause(FunClause {
        name: op,
        args: vec![lhs_pat, rhs_pat],
        guard,
        body,
    })))
}

fn try_parse_funclause_prefix_ident(
    ts: &mut TokenStream,
    stop: Stop,
) -> Result<Option<ParsedBind>> {
    let save = (ts.i, ts.last_span_end);
    let name = match ts.expect_ident() {
        Ok(n) => n,
        Err(_) => {
            (ts.i, ts.last_span_end) = save;
            return Ok(None);
        }
    };
    if is_ctor_symbol(&name) {
        return Err(ts.err_here("operators starting with ':' are constructors"));
    }

    // If a colon follows immediately, this is a pattern binding (`x:xs = ...`).
    if matches!(ts.peek_kind(), Some(TokenKind::Colon)) {
        (ts.i, ts.last_span_end) = save;
        return Ok(None);
    }

    let mut args = Vec::new();
    while can_start_pattern_atom(ts.peek_kind()) {
        args.push(pattern::parse_pattern(ts)?);
    }
    if args.is_empty() {
        (ts.i, ts.last_span_end) = save;
        return Ok(None);
    }

    let (guard, body) = parse_guard_and_body(ts, stop)?;
    Ok(Some(ParsedBind::FunClause(FunClause {
        name,
        args,
        guard,
        body,
    })))
}

fn parse_guard_and_body(
    ts: &mut TokenStream,
    stop: Stop,
) -> Result<(Option<ast::Expr>, ast::Expr)> {
    if matches!(ts.peek_kind(), Some(TokenKind::Pipe)) {
        ts.bump();
        let guard = parse_expr(ts, Stop::LineEnd)?;
        ts.expect(TokenKind::Eq)?;
        let body = parse_expr(ts, stop)?;
        return Ok((Some(guard), body));
    }
    ts.expect(TokenKind::Eq)?;
    let body = parse_expr(ts, stop)?;
    Ok((None, body))
}

fn can_start_pattern_atom(kind: Option<&TokenKind>) -> bool {
    matches!(
        kind,
        Some(TokenKind::Ident(_))
            | Some(TokenKind::LParen)
            | Some(TokenKind::LBracket)
            | Some(TokenKind::LBrace)
            | Some(TokenKind::Integer(_))
            | Some(TokenKind::Float(_))
            | Some(TokenKind::String(_))
            | Some(TokenKind::Char(_))
            | Some(TokenKind::True)
            | Some(TokenKind::False)
            | Some(TokenKind::Question)
            | Some(TokenKind::Colon)
    )
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
    Arrow,
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
            Stop::Arrow => token_stream::Stop::Arrow,
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
            if let Ok(pat) = pattern::parse_pattern(ts) {
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
        if let Ok(pat) = pattern::parse_pattern(ts) {
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

            let pat = pattern::parse_pattern(ts)?;

            // Disambiguation:
            // - `pat1 | pat2 ->` is an or-pattern arm
            // - `pat | guard_expr ->` is a guard
            // Prefer treating `|` as the start of a guard unless we can prove it forms
            // an or-pattern *followed by* `->`.
            let guard = if matches!(ts.peek_kind(), Some(TokenKind::Pipe)) {
                // Prefer parsing `| <expr> ->` as a guard.
                let save = (ts.i, ts.last_span_end);
                ts.bump();
                if let Ok(g) = parse_expr(ts, Stop::Arrow) {
                    if matches!(ts.peek_kind(), Some(TokenKind::Arrow)) {
                        Some(g)
                    } else {
                        (ts.i, ts.last_span_end) = save;
                        None
                    }
                } else {
                    (ts.i, ts.last_span_end) = save;
                    None
                }
            } else {
                None
            };

            // Or-pattern arms are parsed by `pattern::parse_pattern` itself.

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

        let pat = pattern::parse_pattern(ts)?;

        // Same disambiguation as indent-block case arms.
        let guard = if matches!(ts.peek_kind(), Some(TokenKind::Pipe)) {
            let save = (ts.i, ts.last_span_end);
            ts.bump();
            if let Ok(g) = parse_expr(ts, Stop::Arrow) {
                if matches!(ts.peek_kind(), Some(TokenKind::Arrow)) {
                    Some(g)
                } else {
                    (ts.i, ts.last_span_end) = save;
                    None
                }
            } else {
                (ts.i, ts.last_span_end) = save;
                None
            }
        } else {
            None
        };

        // Or-pattern arms are parsed by `pattern::parse_pattern` itself.

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
    type_expr::parse_annot(ts, expr, stop)
}

fn parse_predicate(ts: &mut TokenStream, stop: Stop) -> Result<ast::Predicate> {
    type_expr::parse_predicate_in_root(ts, stop)
}

fn is_class_super_parens(ts: &TokenStream) -> bool {
    type_expr::is_class_super_parens_in_root(ts)
}

fn parse_qual_type(ts: &mut TokenStream, stop: Stop) -> Result<ast::QualType> {
    type_expr::parse_qual_type(ts, stop)
}

fn is_type_alias_end(kind: Option<&TokenKind>, _stop: Stop) -> bool {
    type_expr::is_type_alias_end_public(kind, Stop::LineEnd)
}

fn is_type_end(kind: Option<&TokenKind>, stop: Stop) -> bool {
    type_expr::is_type_end_in_root(kind, stop)
}

fn parse_type_expr(
    ts: &mut TokenStream,
    stop: Stop,
    end: fn(Option<&TokenKind>, Stop) -> bool,
) -> Result<ast::Type> {
    type_expr::parse_type_expr_in_root(ts, stop, end)
}

fn parse_type_atom(
    ts: &mut TokenStream,
    stop: Stop,
    end: fn(Option<&TokenKind>, Stop) -> bool,
) -> Result<ast::Type> {
    type_expr::parse_type_atom_in_root(ts, stop, end)
}

fn parse_where(ts: &mut TokenStream, expr: ast::Expr) -> Result<ast::Expr> {
    let start = expr.span.start;
    ts.expect(TokenKind::KwWhere)?;

    let bindings = if matches!(ts.peek_kind(), Some(TokenKind::LBrace)) {
        ts.bump();
        let mut bindings = Vec::new();
        let mut pending: Option<PendingFun> = None;

        loop {
            ts.skip_newlines();
            if matches!(ts.peek_kind(), Some(TokenKind::RBrace)) {
                break;
            }
            if ts.is_eof() {
                return Err(ts.err_here("unexpected EOF in where"));
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

            if matches!(ts.peek_kind(), Some(TokenKind::Semicolon)) {
                ts.bump();
            } else {
                break;
            }
        }
        flush_pending_fun_binding(ts, &mut bindings, pending.take());
        ts.expect(TokenKind::RBrace)?;
        bindings
    } else {
        // Allow `where x = 1; y = 2` inline bindings.
        // Also allow `where` followed by a layout-based block.
        let mut is_layout = false;
        if matches!(ts.peek_kind(), Some(TokenKind::Newline)) {
            is_layout = true;
            ts.consume_line_end();
            ts.skip_newlines();
            ts.expect(TokenKind::Indent)?;
        }

        let mut bindings = Vec::new();
        let mut pending: Option<PendingFun> = None;

        loop {
            if is_layout {
                // In a layout-based `where` block, newlines are significant:
                // do not skip them, otherwise trailing bindings can leak out.
                if matches!(ts.peek_kind(), Some(TokenKind::Newline)) {
                    ts.consume_line_end();
                    continue;
                }
            } else {
                // In an inline `where`, tolerate extra blank lines.
                ts.skip_newlines();
            }
            if matches!(ts.peek_kind(), Some(TokenKind::Dedent)) {
                break;
            }
            // Inline `where` often ends at EOF (e.g. module ends).
            if ts.is_eof() {
                break;
            }

            match parse_binding_or_fun_clause(ts, Stop::LineEnd)? {
                ParsedBind::Binding(b) => {
                    flush_pending_fun_binding(ts, &mut bindings, pending.take());
                    bindings.push(b);
                }
                ParsedBind::FunClause(c) => {
                    push_fun_clause_binding(ts, &mut bindings, &mut pending, c)
                }
            }

            if matches!(ts.peek_kind(), Some(TokenKind::Semicolon)) {
                ts.bump();
                continue;
            }

            // Layout-based where continues until Dedent.
            if matches!(ts.peek_kind(), Some(TokenKind::Dedent)) {
                break;
            }

            // Inline where ends at newline.
            if !is_layout && matches!(ts.peek_kind(), Some(TokenKind::Newline)) {
                ts.consume_line_end();
                break;
            }

            // Otherwise, keep parsing (for layout-based blocks where newlines may be skipped).
        }
        flush_pending_fun_binding(ts, &mut bindings, pending.take());

        if matches!(ts.peek_kind(), Some(TokenKind::Dedent)) {
            ts.expect(TokenKind::Dedent)?;
            ts.consume_line_end();
        }
        bindings
    };

    Ok(expr_from(
        ts,
        start,
        ast::ExprKind::Where {
            expr: Box::new(expr),
            bindings,
        },
    ))
}

fn parse_infix_application(ts: &mut TokenStream, stop: Stop) -> Result<ast::Expr> {
    parse_binops(ts, stop, 0)
}

fn parse_binops(ts: &mut TokenStream, stop: Stop, min_prec: u8) -> Result<ast::Expr> {
    let mut lhs = parse_application(ts, stop)?;

    while ts.can_continue_expr(stop.to_token_stream()) {
        let save = (ts.i, ts.last_span_end);
        let Some(opinfo) = try_parse_binop(ts)? else {
            break;
        };

        if opinfo.fixity.prec < min_prec {
            (ts.i, ts.last_span_end) = save;
            break;
        }

        let rhs_min_prec = rhs_min_prec(opinfo.is_cons, opinfo.fixity);
        let rhs = parse_binops(ts, stop, rhs_min_prec)?;
        lhs = build_binop_expr(ts, lhs, rhs, opinfo);
    }

    Ok(lhs)
}

#[derive(Clone)]
struct BinOpInfo {
    op: String,
    fixity: Fixity,
    is_cons: bool,
}

fn try_parse_binop(ts: &mut TokenStream) -> Result<Option<BinOpInfo>> {
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
        Some(TokenKind::Star) => bump_fixed_op(ts, "*"),
        Some(TokenKind::Slash) => bump_fixed_op(ts, "/"),
        Some(TokenKind::Plus) => bump_fixed_op(ts, "+"),
        Some(TokenKind::Minus) => bump_fixed_op(ts, "-"),
        Some(TokenKind::PlusPlus) => bump_fixed_op(ts, "++"),
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
        Some(TokenKind::EqEq) => bump_fixed_op(ts, "=="),
        Some(TokenKind::SlashEq) => bump_fixed_op(ts, "/="),
        Some(TokenKind::Lt) => bump_fixed_op(ts, "<"),
        Some(TokenKind::Le) => bump_fixed_op(ts, "<="),
        Some(TokenKind::Gt) => bump_fixed_op(ts, ">"),
        Some(TokenKind::Ge) => bump_fixed_op(ts, ">="),
        Some(TokenKind::GtGt) => bump_fixed_op(ts, ">>"),
        Some(TokenKind::GtGtEq) => bump_fixed_op(ts, ">>="),
        Some(TokenKind::AndAnd) => bump_fixed_op(ts, "&&"),
        Some(TokenKind::OrOr) => bump_fixed_op(ts, "||"),
        _ => return Ok(None),
    };

    Ok(Some(BinOpInfo {
        op,
        fixity,
        is_cons,
    }))
}

fn bump_fixed_op(ts: &mut TokenStream, op: &str) -> (String, Fixity) {
    ts.bump();
    (op.to_string(), ts.fixity(op))
}

fn rhs_min_prec(is_cons: bool, fixity: Fixity) -> u8 {
    if is_cons {
        return fixity.prec;
    }
    match fixity.assoc {
        Assoc::Right => fixity.prec,
        _ => fixity.prec + 1,
    }
}

fn build_binop_expr(
    ts: &TokenStream,
    lhs: ast::Expr,
    rhs: ast::Expr,
    opinfo: BinOpInfo,
) -> ast::Expr {
    let start = lhs.span.start;
    if opinfo.is_cons {
        return expr_from(
            ts,
            start,
            ast::ExprKind::Cons {
                head: Box::new(lhs),
                tail: Box::new(rhs),
            },
        );
    }

    let func_kind = op_expr_kind(opinfo.op);
    expr_from(
        ts,
        start,
        ast::ExprKind::Apply {
            func: Box::new(ast::Expr::dummy(func_kind)),
            args: vec![lhs, rhs],
        },
    )
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
                Ok(expr_from(
                    ts,
                    start,
                    ast::ExprKind::Ctor(ast::ResolvedName::Unresolved(s)),
                ))
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

    if let Some(expr) = try_parse_paren_section_prefix(ts, start)? {
        return Ok(expr);
    }
    if let Some(expr) = try_parse_paren_section_suffix(ts, start)? {
        return Ok(expr);
    }

    parse_paren_tuple_or_group(ts, start)
}

fn parse_backtick_or_sym_op(ts: &mut TokenStream) -> Result<String> {
    if matches!(ts.peek_kind(), Some(TokenKind::Backtick)) {
        ts.expect(TokenKind::Backtick)?;
        let op = ts.expect_ident()?;
        ts.expect(TokenKind::Backtick)?;
        return Ok(op);
    }
    parse_fixity_op(ts)
}

fn try_parse_paren_section_prefix(ts: &mut TokenStream, start: usize) -> Result<Option<ast::Expr>> {
    let save = (ts.i, ts.last_span_end);
    if !(matches!(ts.peek_kind(), Some(TokenKind::Backtick)) || is_sym_op_token(ts.peek_kind())) {
        return Ok(None);
    }

    let op = match parse_backtick_or_sym_op(ts) {
        Ok(op) => op,
        Err(_) => {
            (ts.i, ts.last_span_end) = save;
            return Ok(None);
        }
    };
    if matches!(ts.peek_kind(), Some(TokenKind::RParen)) {
        ts.bump();
        if op == ":" {
            return Ok(Some(make_cons_lambda_2(ts, start)));
        }
        return Ok(Some(expr_from(ts, start, op_expr_kind(op))));
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
    Ok(Some(expr_from(
        ts,
        start,
        ast::ExprKind::Lambda {
            params: vec![param],
            body: Box::new(body),
        },
    )))
}

fn try_parse_paren_section_suffix(ts: &mut TokenStream, start: usize) -> Result<Option<ast::Expr>> {
    let save = (ts.i, ts.last_span_end);
    let lhs = match parse_application(ts, Stop::LineEnd) {
        Ok(lhs) => lhs,
        Err(_) => {
            (ts.i, ts.last_span_end) = save;
            return Ok(None);
        }
    };

    if !(matches!(ts.peek_kind(), Some(TokenKind::Backtick)) || is_sym_op_token(ts.peek_kind())) {
        (ts.i, ts.last_span_end) = save;
        return Ok(None);
    }
    let op = parse_backtick_or_sym_op(ts)?;
    if !matches!(ts.peek_kind(), Some(TokenKind::RParen)) {
        (ts.i, ts.last_span_end) = save;
        return Ok(None);
    }

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
    Ok(Some(expr_from(
        ts,
        start,
        ast::ExprKind::Lambda {
            params: vec![param],
            body: Box::new(body),
        },
    )))
}

fn parse_paren_tuple_or_group(ts: &mut TokenStream, start: usize) -> Result<ast::Expr> {
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

fn make_cons_lambda_2(ts: &mut TokenStream, start: usize) -> ast::Expr {
    let a = ts.fresh_name("__cons_a");
    let b = ts.fresh_name("__cons_b");
    let body = ast::Expr::dummy(ast::ExprKind::Cons {
        head: Box::new(ast::Expr::dummy(ast::ExprKind::Var(a.clone()))),
        tail: Box::new(ast::Expr::dummy(ast::ExprKind::Var(b.clone()))),
    });
    expr_from(
        ts,
        start,
        ast::ExprKind::Lambda {
            params: vec![a, b],
            body: Box::new(body),
        },
    )
}

fn parse_list_expr(ts: &mut TokenStream) -> Result<ast::Expr> {
    let start = ts.peek_span().map(|s| s.start).unwrap_or(0);
    ts.expect(TokenKind::LBracket)?;

    if matches!(ts.peek_kind(), Some(TokenKind::RBracket)) {
        ts.bump();
        return Ok(expr_from(ts, start, ast::ExprKind::List(Vec::new())));
    }

    let first = parse_expr(ts, Stop::Pattern)?;

    if let Some(expr) = try_parse_list_range(ts, start, first.clone())? {
        return Ok(expr);
    }
    if let Some(expr) = try_parse_list_step_range(ts, start, first.clone())? {
        return Ok(expr);
    }
    if let Some(expr) = try_parse_list_comprehension(ts, start, first.clone())? {
        return Ok(expr);
    }

    parse_list_literal_tail(ts, start, first)
}

fn apply_var(name: &str, args: Vec<ast::Expr>) -> ast::ExprKind {
    ast::ExprKind::Apply {
        func: Box::new(ast::Expr::dummy(ast::ExprKind::Var(name.to_string()))),
        args,
    }
}

enum ListCompGen {
    Bind(ast::Pattern, ast::Expr),
    Guard(ast::Expr),
}

fn try_parse_list_range(
    ts: &mut TokenStream,
    start: usize,
    first: ast::Expr,
) -> Result<Option<ast::Expr>> {
    // [a..b] => enumFromTo a b
    // [a..]  => enumFrom a
    if !matches!(ts.peek_kind(), Some(TokenKind::Operator(op)) if op == "..") {
        return Ok(None);
    }
    ts.bump();

    if matches!(ts.peek_kind(), Some(TokenKind::RBracket)) {
        ts.bump();
        return Ok(Some(expr_from(
            ts,
            start,
            apply_var("enumFrom", vec![first]),
        )));
    }

    let end = parse_expr(ts, Stop::Pattern)?;
    ts.expect(TokenKind::RBracket)?;
    Ok(Some(expr_from(
        ts,
        start,
        apply_var("enumFromTo", vec![first, end]),
    )))
}

fn try_parse_list_step_range(
    ts: &mut TokenStream,
    start: usize,
    first: ast::Expr,
) -> Result<Option<ast::Expr>> {
    // [a,b..c] => enumFromThenTo a b c
    // [a,b..]  => enumFromThen a b
    if !matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
        return Ok(None);
    }

    let save = (ts.i, ts.last_span_end);
    ts.bump();
    let second = match parse_expr(ts, Stop::Pattern) {
        Ok(e) => e,
        Err(e) => {
            (ts.i, ts.last_span_end) = save;
            return Err(e);
        }
    };
    if !matches!(ts.peek_kind(), Some(TokenKind::Operator(op)) if op == "..") {
        (ts.i, ts.last_span_end) = save;
        return Ok(None);
    }
    ts.bump();

    if matches!(ts.peek_kind(), Some(TokenKind::RBracket)) {
        ts.bump();
        return Ok(Some(expr_from(
            ts,
            start,
            apply_var("enumFromThen", vec![first, second]),
        )));
    }

    let end = parse_expr(ts, Stop::Pattern)?;
    ts.expect(TokenKind::RBracket)?;
    Ok(Some(expr_from(
        ts,
        start,
        apply_var("enumFromThenTo", vec![first, second, end]),
    )))
}

fn try_parse_list_comprehension(
    ts: &mut TokenStream,
    start: usize,
    first: ast::Expr,
) -> Result<Option<ast::Expr>> {
    // [expr | generator_list]
    if !matches!(ts.peek_kind(), Some(TokenKind::Pipe)) {
        return Ok(None);
    }
    ts.bump();

    let gens = parse_list_generators(ts)?;
    ts.expect(TokenKind::RBracket)?;

    let mut out = ast::Expr::dummy(ast::ExprKind::List(vec![first]));
    for g in gens.into_iter().rev() {
        out = ast::Expr::dummy(match g {
            ListCompGen::Guard(cond) => ast::ExprKind::If {
                cond: Box::new(cond),
                then_branch: Box::new(out),
                else_branch: Box::new(ast::Expr::dummy(ast::ExprKind::List(Vec::new()))),
            },
            ListCompGen::Bind(pat, xs) => build_list_comp_bind(ts, pat, xs, out),
        });
    }

    let ast::Expr { kind, .. } = out;
    Ok(Some(expr_from(ts, start, kind)))
}

fn parse_list_generators(ts: &mut TokenStream) -> Result<Vec<ListCompGen>> {
    let mut gens = Vec::new();
    loop {
        let save = (ts.i, ts.last_span_end);
        if let Ok(pat) = pattern::parse_pattern(ts) {
            if matches!(ts.peek_kind(), Some(TokenKind::LeftArrow)) {
                ts.bump();
                let rhs = parse_expr(ts, Stop::Pattern)?;
                gens.push(ListCompGen::Bind(pat, rhs));
            } else {
                (ts.i, ts.last_span_end) = save;
                let e = parse_expr(ts, Stop::Pattern)?;
                gens.push(ListCompGen::Guard(e));
            }
        } else {
            (ts.i, ts.last_span_end) = save;
            let e = parse_expr(ts, Stop::Pattern)?;
            gens.push(ListCompGen::Guard(e));
        }

        if matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
            ts.bump();
            continue;
        }
        break;
    }
    Ok(gens)
}

fn build_list_comp_bind(
    ts: &mut TokenStream,
    pat: ast::Pattern,
    xs: ast::Expr,
    out: ast::Expr,
) -> ast::ExprKind {
    match pat.kind {
        ast::PatternKind::Var(name) => apply_var(
            "concatMap",
            vec![
                ast::Expr::dummy(ast::ExprKind::Lambda {
                    params: vec![name],
                    body: Box::new(out),
                }),
                xs,
            ],
        ),
        ast::PatternKind::Wildcard => apply_var(
            "concatMap",
            vec![
                ast::Expr::dummy(ast::ExprKind::Lambda {
                    params: vec!["_".to_string()],
                    body: Box::new(out),
                }),
                xs,
            ],
        ),
        other_kind => {
            let other_pat = ast::Pattern {
                kind: other_kind,
                span: pat.span,
            };
            let tmp = ts.fresh_name("_lc");
            apply_var(
                "concatMap",
                vec![
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
                                    pat: ast::Pattern::dummy(ast::PatternKind::Wildcard),
                                    guard: None,
                                    body: ast::Expr::dummy(ast::ExprKind::List(Vec::new())),
                                },
                            ],
                        })),
                    }),
                    xs,
                ],
            )
        }
    }
}

fn parse_list_literal_tail(
    ts: &mut TokenStream,
    start: usize,
    first: ast::Expr,
) -> Result<ast::Expr> {
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
