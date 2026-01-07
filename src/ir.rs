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
pub enum IrLiteral {
    Unit,
    Integer(String),
    Float64(String),
    Bool(bool),
    String(String),
    Char(char),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrPattern {
    Var(String),
    Wildcard,
    Literal(IrLiteral),
    Tuple(Vec<IrPattern>),
    List(Vec<IrPattern>),
    Record(Vec<(String, IrPattern)>),
    RecordLoose(Vec<(String, IrPattern)>),
    Cons(Box<IrPattern>, Box<IrPattern>),
    Constructor { name: String, args: Vec<IrPattern> },
    Or(Box<IrPattern>, Box<IrPattern>),
    As(String, Box<IrPattern>),
    View(Box<IrPattern>, Box<IrExpr>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrCaseArm {
    pub pat: IrPattern,
    pub guard: Option<IrExpr>,
    pub body: IrExpr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrDoStmt {
    Bind { pat: IrPattern, expr: IrExpr },
    Expr(IrExpr),
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
    Case {
        expr: Box<IrExpr>,
        arms: Vec<IrCaseArm>,
    },
    Do(Vec<IrDoStmt>),
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

fn lower_lit_expr(expr: &ast::Expr) -> Result<IrLiteral> {
    use ast::Expr;
    Ok(match expr {
        Expr::Unit => IrLiteral::Unit,
        Expr::Integer(s) => IrLiteral::Integer(s.clone()),
        Expr::Float64(s) => IrLiteral::Float64(s.clone()),
        Expr::Bool(b) => IrLiteral::Bool(*b),
        Expr::String(s) => IrLiteral::String(s.clone()),
        Expr::Char(c) => IrLiteral::Char(*c),
        _ => return Err(Error::msg("unsupported literal")),
    })
}

fn lower_pat(pat: &ast::Pattern) -> Result<IrPattern> {
    use ast::Pattern;
    Ok(match pat {
        Pattern::Var(n) => IrPattern::Var(n.clone()),
        Pattern::Wildcard => IrPattern::Wildcard,
        Pattern::Hole(_) => IrPattern::Wildcard,
        Pattern::Literal(e) => IrPattern::Literal(lower_lit_expr(e)?),
        Pattern::Tuple(ps) => IrPattern::Tuple(ps.iter().map(lower_pat).collect::<Result<Vec<_>>>()?),
        Pattern::List(ps) => IrPattern::List(ps.iter().map(lower_pat).collect::<Result<Vec<_>>>()?),
        Pattern::Record(fields) => IrPattern::Record(
            fields
                .iter()
                .map(|(n, p)| Ok((n.clone(), lower_pat(p)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
        Pattern::RecordLoose(fields) => IrPattern::RecordLoose(
            fields
                .iter()
                .map(|(n, p)| Ok((n.clone(), lower_pat(p)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
        Pattern::Cons(a, b) => IrPattern::Cons(Box::new(lower_pat(a)?), Box::new(lower_pat(b)?)),
        Pattern::Or(a, b) => IrPattern::Or(Box::new(lower_pat(a)?), Box::new(lower_pat(b)?)),
        Pattern::As(n, p) => IrPattern::As(n.clone(), Box::new(lower_pat(p)?)),
        Pattern::View(p, e) => IrPattern::View(Box::new(lower_pat(p)?), Box::new(lower_expr(e)?)),
        Pattern::Constructor { name, args } => IrPattern::Constructor {
            name: name.clone(),
            args: args.iter().map(lower_pat).collect::<Result<Vec<_>>>()?,
        },
    })
}

fn lower_expr(expr: &ast::Expr) -> Result<IrExpr> {
    use ast::Expr;
    Ok(match expr { 
        // literals

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
        Expr::Case { expr, arms } => IrExpr::Case {
            expr: Box::new(lower_expr(expr)?),
            arms: arms
                .iter()
                .map(|a| {
                    Ok(IrCaseArm {
                        pat: lower_pat(&a.pat)?,
                        guard: a.guard.as_ref().map(lower_expr).transpose()?,
                        body: lower_expr(&a.body)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        },
        Expr::Do(stmts) => IrExpr::Do(
            stmts
                .iter()
                .map(|s| {
                    Ok(match s {
                        ast::DoStmt::Bind { pat, expr } => IrDoStmt::Bind {
                            pat: lower_pat(pat)?,
                            expr: lower_expr(expr)?,
                        },
                        ast::DoStmt::Expr(e) => IrDoStmt::Expr(lower_expr(e)?),
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        ),
        Expr::Annot { expr, .. } => lower_expr(expr)?,
        Expr::Where { expr, bindings } => {
            let mut bs = Vec::new();
            for b in bindings {
                let ast::Pattern::Var(name) = &b.pat else {
                    return Err(Error::msg("IR lowering supports only variable where-bindings"));
                };
                bs.push((name.clone(), lower_expr(&b.expr)?));
            }
            IrExpr::Let {
                bindings: bs,
                body: Box::new(lower_expr(expr)?),
            }
        }
        _ => return Err(Error::msg("expression is not supported in IR lowering yet")),
    })
}
