//! IR scaffolding.

use crate::{ast, error::Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrModule {
    pub items: Vec<IrItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrItem {
    Binding { name: String, expr: IrExpr },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrExpr {
    Unit,
    Integer(String),
    Float64(String),
    Bool(bool),
    String(String),
    Char(char),
    Var(String),
    Lambda { params: Vec<String>, body: Box<IrExpr> },
    Apply { func: Box<IrExpr>, args: Vec<IrExpr> },
    If {
        cond: Box<IrExpr>,
        then_branch: Box<IrExpr>,
        else_branch: Box<IrExpr>,
    },
    Let {
        bindings: Vec<(String, IrExpr)>,
        body: Box<IrExpr>,
    },
    List(Vec<IrExpr>),
    Tuple(Vec<IrExpr>),
    Record(Vec<(String, IrExpr)>),
}

pub fn lower_to_ir(module: &ast::Module) -> Result<IrModule> {
    let mut items = Vec::new();
    for it in &module.items {
        let ast::Item::Binding(b) = it else {
            continue;
        };
        let ast::Pattern::Var(name) = &b.pat else {
            return Err(Error::msg("IR lowering supports only variable bindings"));
        };
        let expr = lower_expr(&b.expr)?;
        items.push(IrItem::Binding {
            name: name.clone(),
            expr,
        });
    }
    Ok(IrModule { items })
}

fn lower_expr(expr: &ast::Expr) -> Result<IrExpr> {
    use ast::Expr;
    Ok(match expr {
        Expr::Unit => IrExpr::Unit,
        Expr::Integer(s) => IrExpr::Integer(s.clone()),
        Expr::Float64(s) => IrExpr::Float64(s.clone()),
        Expr::Bool(b) => IrExpr::Bool(*b),
        Expr::String(s) => IrExpr::String(s.clone()),
        Expr::Char(c) => IrExpr::Char(*c),
        Expr::Var(v) => IrExpr::Var(v.clone()),
        Expr::Ctor(v) => IrExpr::Var(v.clone()),
        Expr::Lambda { params, body } => IrExpr::Lambda {
            params: params.clone(),
            body: Box::new(lower_expr(body)?),
        },
        Expr::Apply { func, args } => IrExpr::Apply {
            func: Box::new(lower_expr(func)?),
            args: args.iter().map(lower_expr).collect::<Result<Vec<_>>>()?,
        },
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => IrExpr::If {
            cond: Box::new(lower_expr(cond)?),
            then_branch: Box::new(lower_expr(then_branch)?),
            else_branch: Box::new(lower_expr(else_branch)?),
        },
        Expr::Let { bindings, body } => {
            let mut bs = Vec::new();
            for b in bindings {
                let ast::Pattern::Var(name) = &b.pat else {
                    return Err(Error::msg("IR lowering supports only variable let-bindings"));
                };
                bs.push((name.clone(), lower_expr(&b.expr)?));
            }
            IrExpr::Let {
                bindings: bs,
                body: Box::new(lower_expr(body)?),
            }
        }
        Expr::List(es) => IrExpr::List(es.iter().map(lower_expr).collect::<Result<Vec<_>>>()?),
        Expr::Tuple(es) => IrExpr::Tuple(es.iter().map(lower_expr).collect::<Result<Vec<_>>>()?),
        Expr::Record(fields) => IrExpr::Record(
            fields
                .iter()
                .map(|(n, e)| Ok((n.clone(), lower_expr(e)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
        _ => return Err(Error::msg("expression is not supported in IR lowering yet")),
    })
}
