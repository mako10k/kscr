use crate::{ast, error::Error, Result};

pub fn parse_module(src: &str) -> Result<ast::Module> {
    let mut items = Vec::new();
    for line in src.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("data ") {
            // data宣言: data Name a = Ctor1 | Ctor2 a
            let (name_and_params, ctors_str) = match rest.split_once('=') {
                Some(pair) => pair,
                None => return Err(Error::msg("expected '=' in data declaration")),
            };
            let mut name_params = name_and_params.split_whitespace();
            let name = name_params.next().unwrap_or("").to_string();
            let params: Vec<String> = name_params.map(|s| s.to_string()).collect();
            let ctors: Vec<ast::DataCtor> = ctors_str
                .split('|')
                .map(|ctor| {
                    let ctor = ctor.trim();
                    let mut parts = ctor.split_whitespace();
                    let ctor_name = parts.next().unwrap_or("").to_string();
                    let args: Vec<ast::Type> =
                        parts.map(|_| ast::Type::Var("_".to_string())).collect(); // 型詳細は省略
                    ast::DataCtor {
                        name: ctor_name,
                        args,
                    }
                })
                .collect();
            items.push(ast::Item::DataDecl(ast::DataDecl {
                name,
                params,
                ctors,
            }));
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
            let expr = parse_expr(rhs);
            items.push(ast::Item::Binding(ast::Binding { name, expr }));
        }
    }
    Ok(ast::Module { items })
}

fn parse_expr(src: &str) -> ast::Expr {
    let src = src.trim();
    // lambda式: \x y -> expr
    if let Some(rest) = src.strip_prefix("\\") {
        if let Some((params_str, body_str)) = rest.split_once("->") {
            let params: Vec<String> = params_str
                .split_whitespace()
                .map(|s| s.to_string())
                .collect();
            let body = Box::new(parse_expr(body_str));
            return ast::Expr::Lambda { params, body };
        }
    }
    // if式: if cond then expr else expr
    if let Some(src) = src.strip_prefix("if ") {
        if let Some((cond_then, else_branch)) = src.split_once("else") {
            if let Some((cond, then_branch)) = cond_then.split_once("then") {
                let cond = Box::new(parse_expr(cond.trim()));
                let then_branch = Box::new(parse_expr(then_branch.trim()));
                let else_branch = Box::new(parse_expr(else_branch.trim()));
                return ast::Expr::If {
                    cond,
                    then_branch,
                    else_branch,
                };
            }
        }
    }
    // 関数適用: f x y → f(x, y)（空白区切りで分割）
    let parts: Vec<&str> = src.split_whitespace().collect();
    if parts.len() > 1 {
        let func = Box::new(parse_expr(parts[0]));
        let args = parts[1..].iter().map(|s| parse_expr(s)).collect();
        return ast::Expr::Apply { func, args };
    }
    // リテラル・変数
    if src == "True" {
        return ast::Expr::Bool(true);
    }
    if src == "False" {
        return ast::Expr::Bool(false);
    }
    if src.starts_with('"') && src.ends_with('"') && src.len() >= 2 {
        return ast::Expr::String(src[1..src.len() - 1].to_string());
    }
    if src.contains('.') {
        return ast::Expr::Float64(src.to_string());
    }
    if src.chars().all(|c| c.is_ascii_digit()) {
        return ast::Expr::Integer(src.to_string());
    }
    ast::Expr::Var(src.to_string())
}
