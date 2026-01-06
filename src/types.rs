//! Type checking and elaboration scaffolding.
//!
//! Policy (docs):
//! - Surface numeric types: Integer (arbitrary precision) and Float64.
//! - Backend/IR numeric types are LLVM-aligned (i32/i64/f32/f64...).
//! - Pure IR subtyping allows only integer widening (iN <: iM); float widening is NOT subtyping.
//! - Potentially lossy conversions happen only at boundaries as checked casts.

use crate::{ast, error::Error, Result};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedModule {
    pub module: ast::Module,
}

pub fn typecheck(mut module: ast::Module) -> Result<TypedModule> {
    let aliases = collect_type_aliases(&module);
    module.items = module
        .items
        .into_iter()
        .map(|it| expand_item(it, &aliases))
        .collect::<Result<Vec<_>>>()?;
    Ok(TypedModule { module })
}

fn collect_type_aliases(module: &ast::Module) -> HashMap<String, ast::TypeAlias> {
    module
        .items
        .iter()
        .filter_map(|it| match it {
            ast::Item::TypeAlias(ta) => Some((ta.name.clone(), ta.clone())),
            _ => None,
        })
        .collect()
}

fn expand_item(item: ast::Item, aliases: &HashMap<String, ast::TypeAlias>) -> Result<ast::Item> {
    match item {
        ast::Item::Binding(b) => Ok(ast::Item::Binding(ast::Binding {
            pat: b.pat,
            expr: expand_expr(b.expr, aliases)?,
        })),
        ast::Item::TypeAlias(ta) => Ok(ast::Item::TypeAlias(ast::TypeAlias {
            name: ta.name,
            params: ta.params,
            ty: expand_type(ta.ty, aliases, &mut Vec::new())?,
        })),
        ast::Item::DataDecl(d) => Ok(ast::Item::DataDecl(ast::DataDecl {
            name: d.name,
            params: d.params,
            ctors: d
                .ctors
                .into_iter()
                .map(|c| {
                    Ok(ast::DataCtor {
                        name: c.name,
                        args: c
                            .args
                            .into_iter()
                            .map(|t| expand_type(t, aliases, &mut Vec::new()))
                            .collect::<Result<Vec<_>>>()?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        })),
        it @ (ast::Item::Import(_) | ast::Item::Export(_)) => Ok(it),
    }
}

fn expand_expr(expr: ast::Expr, aliases: &HashMap<String, ast::TypeAlias>) -> Result<ast::Expr> {
    use ast::Expr;
    Ok(match expr {
        Expr::Lambda { params, body } => Expr::Lambda {
            params,
            body: Box::new(expand_expr(*body, aliases)?),
        },
        Expr::Apply { func, args } => Expr::Apply {
            func: Box::new(expand_expr(*func, aliases)?),
            args: args
                .into_iter()
                .map(|e| expand_expr(e, aliases))
                .collect::<Result<Vec<_>>>()?,
        },
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => Expr::If {
            cond: Box::new(expand_expr(*cond, aliases)?),
            then_branch: Box::new(expand_expr(*then_branch, aliases)?),
            else_branch: Box::new(expand_expr(*else_branch, aliases)?),
        },
        Expr::Let { bindings, body } => Expr::Let {
            bindings: bindings
                .into_iter()
                .map(|b| {
                    Ok(ast::Binding {
                        pat: b.pat,
                        expr: expand_expr(b.expr, aliases)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            body: Box::new(expand_expr(*body, aliases)?),
        },
        Expr::Where { expr, bindings } => Expr::Where {
            expr: Box::new(expand_expr(*expr, aliases)?),
            bindings: bindings
                .into_iter()
                .map(|b| {
                    Ok(ast::Binding {
                        pat: b.pat,
                        expr: expand_expr(b.expr, aliases)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        },
        Expr::Annot { expr, ty } => Expr::Annot {
            expr: Box::new(expand_expr(*expr, aliases)?),
            ty: expand_type(ty, aliases, &mut Vec::new())?,
        },
        Expr::Do(stmts) => Expr::Do(
            stmts
                .into_iter()
                .map(|s| {
                    Ok(match s {
                        ast::DoStmt::Bind { name, expr } => ast::DoStmt::Bind {
                            name,
                            expr: expand_expr(expr, aliases)?,
                        },
                        ast::DoStmt::Expr(e) => ast::DoStmt::Expr(expand_expr(e, aliases)?),
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        ),
        Expr::Case { expr, arms } => Expr::Case {
            expr: Box::new(expand_expr(*expr, aliases)?),
            arms: arms
                .into_iter()
                .map(|(p, e)| Ok((p, expand_expr(e, aliases)?)))
                .collect::<Result<Vec<_>>>()?,
        },
        Expr::List(v) => Expr::List(
            v.into_iter()
                .map(|e| expand_expr(e, aliases))
                .collect::<Result<Vec<_>>>()?,
        ),
        Expr::Tuple(v) => Expr::Tuple(
            v.into_iter()
                .map(|e| expand_expr(e, aliases))
                .collect::<Result<Vec<_>>>()?,
        ),
        Expr::Record(fields) => Expr::Record(
            fields
                .into_iter()
                .map(|(n, e)| Ok((n, expand_expr(e, aliases)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
        other => other,
    })
}

fn expand_type(
    ty: ast::Type,
    aliases: &HashMap<String, ast::TypeAlias>,
    stack: &mut Vec<String>,
) -> Result<ast::Type> {
    use ast::Type;

    Ok(match ty {
        Type::Hole(name) => Type::Hole(name),
        Type::List(t) => Type::List(Box::new(expand_type(*t, aliases, stack)?)),
        Type::Tuple(ts) => Type::Tuple(
            ts.into_iter()
                .map(|t| expand_type(t, aliases, stack))
                .collect::<Result<Vec<_>>>()?,
        ),
        Type::Record(fields) => Type::Record(
            fields
                .into_iter()
                .map(|(n, t)| Ok((n, expand_type(t, aliases, stack)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
        Type::Func(a, b) => Type::Func(
            Box::new(expand_type(*a, aliases, stack)?),
            Box::new(expand_type(*b, aliases, stack)?),
        ),
        Type::App { head, args } => {
            let args = args
                .into_iter()
                .map(|t| expand_type(t, aliases, stack))
                .collect::<Result<Vec<_>>>()?;

            match *head {
                Type::Var(name) => {
                    if let Some(alias) = aliases.get(&name) {
                        if alias.params.len() != args.len() {
                            return Err(Error::msg("type alias arity mismatch"));
                        }
                        expand_alias(alias, &args, aliases, stack)?
                    } else {
                        Type::App {
                            head: Box::new(Type::Var(name)),
                            args,
                        }
                    }
                }
                head => Type::App {
                    head: Box::new(expand_type(head, aliases, stack)?),
                    args,
                },
            }
        }
        Type::Var(name) => {
            if let Some(alias) = aliases.get(&name) {
                if alias.params.is_empty() {
                    expand_alias(alias, &[], aliases, stack)?
                } else {
                    Type::Var(name)
                }
            } else {
                Type::Var(name)
            }
        }
        other => other,
    })
}

fn expand_alias(
    alias: &ast::TypeAlias,
    args: &[ast::Type],
    aliases: &HashMap<String, ast::TypeAlias>,
    stack: &mut Vec<String>,
) -> Result<ast::Type> {
    if stack.contains(&alias.name) {
        return Err(Error::msg("recursive type alias"));
    }
    stack.push(alias.name.clone());

    let env: HashMap<String, ast::Type> = alias
        .params
        .iter()
        .cloned()
        .zip(args.iter().cloned())
        .collect();

    let ty = subst_type(alias.ty.clone(), &env);
    let ty = expand_type(ty, aliases, stack)?;

    stack.pop();
    Ok(ty)
}

fn subst_type(ty: ast::Type, env: &HashMap<String, ast::Type>) -> ast::Type {
    use ast::Type;

    match ty {
        Type::Hole(name) => Type::Hole(name),
        Type::Var(v) => env.get(&v).cloned().unwrap_or(Type::Var(v)),
        Type::List(t) => Type::List(Box::new(subst_type(*t, env))),
        Type::Tuple(ts) => Type::Tuple(ts.into_iter().map(|t| subst_type(t, env)).collect()),
        Type::Record(fields) => Type::Record(
            fields
                .into_iter()
                .map(|(n, t)| (n, subst_type(t, env)))
                .collect(),
        ),
        Type::App { head, args } => Type::App {
            head: Box::new(subst_type(*head, env)),
            args: args.into_iter().map(|t| subst_type(t, env)).collect(),
        },
        Type::Func(a, b) => {
            Type::Func(Box::new(subst_type(*a, env)), Box::new(subst_type(*b, env)))
        }
        other => other,
    }
}
