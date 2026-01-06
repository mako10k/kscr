use crate::{ast, error::Error, Result};

pub fn parse_module(src: &str) -> Result<ast::Module> {
    let mut items = Vec::new();
    for line in src.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        if let Some(rest) = line.strip_prefix("data ") {
            // data宣言: data Name a = Ctor1 | Ctor2 a
            let (name_and_params, ctors_str) = match rest.split_once('=') {
                Some(pair) => pair,
                None => return Err(Error::msg("expected '=' in data declaration")),
            };
            let mut name_params = name_and_params.split_whitespace();
            let name = name_params.next().unwrap_or("").to_string();
            let params: Vec<String> = name_params.map(|s| s.to_string()).collect();
            let ctors: Vec<ast::DataCtor> = ctors_str.split('|').map(|ctor| {
                let ctor = ctor.trim();
                let mut parts = ctor.split_whitespace();
                let ctor_name = parts.next().unwrap_or("").to_string();
                let args: Vec<ast::Type> = parts.map(|_| ast::Type::Var("_".to_string())).collect(); // 型詳細は省略
                ast::DataCtor { name: ctor_name, args }
            }).collect();
            items.push(ast::Item::DataDecl(ast::DataDecl { name, params, ctors }));
        } else if let Some(rest) = line.strip_prefix("type ") {
            // typeエイリアス: type Name a = TypeExpr
            let (name_and_params, ty_str) = match rest.split_once('=') {
                Some(pair) => pair,
                None => return Err(Error::msg("expected '=' in type alias declaration")),
            };
            let mut name_params = name_and_params.split_whitespace();
            let name = name_params.next().unwrap_or("").to_string();
            let params: Vec<String> = name_params.map(|s| s.to_string()).collect();
            // 型詳細は省略（Varで仮実装）
            let ty = ast::Type::Var(ty_str.trim().to_string());
            items.push(ast::Item::TypeAlias(ast::TypeAlias { name, params, ty }));
        } else if let Some((lhs, rhs)) = line.split_once('=') {
            // binding
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
    }
    Ok(ast::Module { items })
}
