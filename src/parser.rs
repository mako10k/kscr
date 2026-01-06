use crate::{ast, error::Error, Result};

pub fn parse_module(src: &str) -> Result<ast::Module> {
    // 複数行の name = value バインディングをすべて解析
    let mut items = Vec::new();
    for line in src.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        let (lhs, rhs) = match line.split_once('=') {
            Some(pair) => pair,
            None => return Err(Error::msg("expected '=' in binding line")),
        };
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
        items.push(ast::Item::Binding(ast::Binding { name, expr }));
    }
    Ok(ast::Module { items })
}
