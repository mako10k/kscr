use crate::{ast, error::Error, Result};

pub fn parse_module(src: &str) -> Result<ast::Module> {
    // TODO: implement according to docs/LanguageBNF.md
    if src.trim().is_empty() {
        return Ok(ast::Module { items: vec![] });
    }

    // Temporary: accept a single binding like: name = 123
    let (lhs, rhs) = src
        .split_once('=')
        .ok_or_else(|| Error::msg("expected '=' (scaffold parser only supports: name = literal)"))?;

    let name = lhs.trim().to_string();
    if name.is_empty() {
        return Err(Error::msg("empty binding name"));
    }

    let lit = rhs.trim();
    let expr = if lit == "True" {
        ast::Expr::Bool(true)
    } else if lit == "False" {
        ast::Expr::Bool(false)
    } else if lit.starts_with('"') && lit.ends_with('"') && lit.len() >= 2 {
        ast::Expr::String(lit[1..lit.len() - 1].to_string())
    } else if lit.contains('.') {
        ast::Expr::Float64(lit.to_string())
    } else {
        ast::Expr::Integer(lit.to_string())
    };

    Ok(ast::Module {
        items: vec![ast::Item::Binding(ast::Binding { name, expr })],
    })
}
