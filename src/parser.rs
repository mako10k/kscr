use crate::{ast, error::Error, lexer, lexer::TokenKind, Result};

pub fn parse_module(src: &str) -> Result<ast::Module> {
    let tokens = lexer::lex(src);
    let mut ts = TokenStream::new(tokens);
    let mut items = Vec::new();

    while !ts.is_eof() {
        ts.skip_newlines();
        if ts.is_eof() {
            break;
        }

        let item = match ts.peek_kind() {
            Some(TokenKind::KwData) => parse_data_decl(&mut ts)?,
            Some(TokenKind::KwType) => parse_type_alias(&mut ts)?,
            Some(TokenKind::Ident(_)) => parse_binding(&mut ts)?,
            Some(_) => {
                return Err(Error::msg("unexpected token at top-level"));
            }
            None => break,
        };

        items.push(item);
        ts.consume_line_end();
    }

    Ok(ast::Module { items })
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
                | TokenKind::False,
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
    match ts.bump() {
        Some(TokenKind::True) => Ok(ast::Expr::Bool(true)),
        Some(TokenKind::False) => Ok(ast::Expr::Bool(false)),
        Some(TokenKind::Integer(s)) => Ok(ast::Expr::Integer(s)),
        Some(TokenKind::Float(s)) => Ok(ast::Expr::Float64(s)),
        Some(TokenKind::String(s)) => Ok(ast::Expr::String(s)),
        Some(TokenKind::Ident(s)) => Ok(ast::Expr::Var(s)),
        _ => Err(Error::msg("expected expression")),
    }
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
            Some(TokenKind::KwIf) => "if".to_string(),
            Some(TokenKind::KwThen) => "then".to_string(),
            Some(TokenKind::KwElse) => "else".to_string(),
            Some(TokenKind::KwType) => "type".to_string(),
            Some(TokenKind::KwData) => "data".to_string(),
            Some(TokenKind::Eq) => "=".to_string(),
            Some(TokenKind::Pipe) => "|".to_string(),
            Some(TokenKind::Backslash) => "\\".to_string(),
            Some(TokenKind::Arrow) => "->".to_string(),
            Some(TokenKind::LBracket) => "[".to_string(),
            Some(TokenKind::RBracket) => "]".to_string(),
            Some(TokenKind::Newline) | None => "".to_string(),
        }
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
            (Stop::LineEnd, _) => true,
            _ => true,
        }
    }
}
