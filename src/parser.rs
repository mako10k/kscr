use crate::{ast, error::Error, lexer, lexer::TokenKind, Result};

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
            Some(TokenKind::Ident(_)) => parse_binding(ts)?,
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
            Some(TokenKind::Ident(_)) | Some(TokenKind::LBracket)
        ) {
            // Placeholder type parsing: keep a minimal representation.
            args.push(parse_type_placeholder(ts)?);
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

    let mut ty_src = String::new();
    while !matches!(ts.peek_kind(), Some(TokenKind::Newline) | None) {
        if !ty_src.is_empty() {
            ty_src.push(' ');
        }
        ty_src.push_str(&ts.bump_text());
    }

    Ok(ast::Item::TypeAlias(ast::TypeAlias {
        name,
        params,
        ty: ast::Type::Var(ty_src),
    }))
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
    let name = ts.expect_ident()?;
    ts.expect(TokenKind::Eq)?;
    let expr = parse_expr(ts, Stop::LineEnd)?;
    Ok(ast::Item::Binding(ast::Binding { name, expr }))
}

#[derive(Clone, Copy)]
enum Stop {
    LineEnd,
    Then,
    Else,
}

fn parse_expr(ts: &mut TokenStream, stop: Stop) -> Result<ast::Expr> {
    match ts.peek_kind() {
        Some(TokenKind::Backslash) => parse_lambda(ts, stop),
        Some(TokenKind::KwIf) => parse_if(ts, stop),
        _ => parse_application(ts, stop),
    }
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

fn parse_application(ts: &mut TokenStream, stop: Stop) -> Result<ast::Expr> {
    let mut exprs = Vec::new();
    exprs.push(parse_atom(ts)?);

    while ts.can_continue_expr(stop) {
        match ts.peek_kind() {
            Some(TokenKind::Backslash) | Some(TokenKind::KwIf) => {
                exprs.push(parse_expr(ts, stop)?);
            }
            Some(
                TokenKind::Ident(_)
                | TokenKind::Integer(_)
                | TokenKind::Float(_)
                | TokenKind::String(_)
                | TokenKind::True
                | TokenKind::False
                | TokenKind::LBracket
                | TokenKind::LParen,
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
        Some(TokenKind::Ident(_)) => match ts.bump() {
            Some(TokenKind::Ident(s)) => Ok(ast::Expr::Var(s)),
            _ => unreachable!(),
        },
        Some(TokenKind::LBracket) => parse_list_expr(ts),
        Some(TokenKind::LParen) => parse_paren_or_tuple_expr(ts),
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

    let mut elems = Vec::new();
    elems.push(parse_expr(ts, Stop::LineEnd)?);
    while matches!(ts.peek_kind(), Some(TokenKind::Comma)) {
        ts.bump();
        elems.push(parse_expr(ts, Stop::LineEnd)?);
    }

    ts.expect(TokenKind::RBracket)?;
    Ok(ast::Expr::List(elems))
}

fn parse_type_placeholder(ts: &mut TokenStream) -> Result<ast::Type> {
    // Minimal placeholder for now.
    let mut s = String::new();

    if !matches!(
        ts.peek_kind(),
        Some(TokenKind::Ident(_)) | Some(TokenKind::LBracket)
    ) {
        return Err(Error::msg("expected type"));
    }

    while matches!(
        ts.peek_kind(),
        Some(TokenKind::Ident(_)) | Some(TokenKind::LBracket) | Some(TokenKind::RBracket)
    ) {
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(&ts.bump_text());

        // Stop after simple bracketed type like [Char] or a single identifier.
        if !matches!(
            ts.peek_kind(),
            Some(TokenKind::Ident(_)) | Some(TokenKind::LBracket)
        ) {
            break;
        }
    }

    Ok(ast::Type::Var(s))
}

struct TokenStream {
    tokens: Vec<lexer::Token>,
    i: usize,
}

impl TokenStream {
    fn new(tokens: Vec<lexer::Token>) -> Self {
        Self { tokens, i: 0 }
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

    fn bump_text(&mut self) -> String {
        match self.bump() {
            Some(TokenKind::Ident(s)) => s,
            Some(TokenKind::Integer(s)) => s,
            Some(TokenKind::Float(s)) => s,
            Some(TokenKind::String(s)) => format!("\"{}\"", s),
            Some(TokenKind::True) => "True".to_string(),
            Some(TokenKind::False) => "False".to_string(),
            Some(TokenKind::KwModule) => "module".to_string(),
            Some(TokenKind::KwWhere) => "where".to_string(),
            Some(TokenKind::KwImport) => "import".to_string(),
            Some(TokenKind::KwExport) => "export".to_string(),
            Some(TokenKind::KwIf) => "if".to_string(),
            Some(TokenKind::KwThen) => "then".to_string(),
            Some(TokenKind::KwElse) => "else".to_string(),
            Some(TokenKind::KwType) => "type".to_string(),
            Some(TokenKind::KwData) => "data".to_string(),
            Some(TokenKind::Eq) => "=".to_string(),
            Some(TokenKind::Pipe) => "|".to_string(),
            Some(TokenKind::Comma) => ",".to_string(),
            Some(TokenKind::Backslash) => "\\".to_string(),
            Some(TokenKind::Arrow) => "->".to_string(),
            Some(TokenKind::LParen) => "(".to_string(),
            Some(TokenKind::RParen) => ")".to_string(),
            Some(TokenKind::LBracket) => "[".to_string(),
            Some(TokenKind::RBracket) => "]".to_string(),
            Some(TokenKind::Newline) => "".to_string(),
            Some(TokenKind::Indent) => "INDENT".to_string(),
            Some(TokenKind::Dedent) => "DEDENT".to_string(),
            None => "".to_string(),
        }
    }

    // bump_text is defined above.

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
            (Stop::LineEnd, _) => true,
            _ => true,
        }
    }
}
