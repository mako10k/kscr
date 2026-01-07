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
    IoBind {
        action: Box<IrExpr>,
        param: String,
        body: Box<IrExpr>,
    },
    IoThen {
        first: Box<IrExpr>,
        then_expr: Box<IrExpr>,
    },
    List(Vec<IrExpr>),
    Tuple(Vec<IrExpr>),
    Record(Vec<(String, IrExpr)>),
}

pub fn lower_to_ir(module: &ast::Module) -> Result<IrModule> {
    let mut items = Vec::new();
    let mut fresh = 0usize;

    for it in &module.items {
        let ast::Item::Binding(b) = it else {
            continue;
        };

        match &b.pat {
            ast::Pattern::Var(name) => {
                let expr = lower_expr(&b.expr, &mut fresh)?;
                items.push(IrItem::Binding {
                    name: name.clone(),
                    expr,
                });
            }
            pat => {
                let mut vars = std::collections::BTreeSet::new();
                collect_pat_vars(pat, &mut vars);
                if vars.is_empty() {
                    return Err(Error::msg(
                        "IR lowering supports only bindings that introduce variables",
                    ));
                }

                let tmp = format!("_ir_top{fresh}");
                fresh += 1;
                items.push(IrItem::Binding {
                    name: tmp.clone(),
                    expr: lower_expr(&b.expr, &mut fresh)?,
                });

                let ir_pat = lower_pat(pat, &mut fresh)?;
                for v in vars {
                    items.push(IrItem::Binding {
                        name: v.clone(),
                        expr: IrExpr::Case {
                            expr: Box::new(IrExpr::Var(tmp.clone())),
                            arms: vec![IrCaseArm {
                                pat: ir_pat.clone(),
                                guard: None,
                                body: IrExpr::Var(v),
                            }],
                        },
                    });
                }
            }
        }
    }

    Ok(IrModule { items })
}

fn lower_do(stmts: &[ast::DoStmt], fresh: &mut usize) -> Result<IrExpr> {
    if stmts.is_empty() {
        return Err(Error::msg("empty do"));
    }

    let mut it = stmts.iter().rev();
    let Some(ast::DoStmt::Expr(last)) = it.next() else {
        return Err(Error::msg("do must end with expression"));
    };
    let mut acc = lower_expr(last, fresh)?;

    for stmt in it {
        match stmt {
            ast::DoStmt::Expr(e) => {
                acc = IrExpr::IoThen {
                    first: Box::new(lower_expr(e, fresh)?),
                    then_expr: Box::new(acc),
                };
            }
            ast::DoStmt::Bind { pat, expr } => {
                let tmp = format!("_do{fresh}");
                *fresh += 1;

                let body = match pat {
                    ast::Pattern::Var(name) => IrExpr::IoBind {
                        action: Box::new(lower_expr(expr, fresh)?),
                        param: name.clone(),
                        body: Box::new(acc),
                    },
                    other => {
                        let ir_pat = lower_pat(other, fresh)?;
                        IrExpr::IoBind {
                            action: Box::new(lower_expr(expr, fresh)?),
                            param: tmp.clone(),
                            body: Box::new(IrExpr::Case {
                                expr: Box::new(IrExpr::Var(tmp)),
                                arms: vec![IrCaseArm {
                                    pat: ir_pat,
                                    guard: None,
                                    body: acc,
                                }],
                            }),
                        }
                    }
                };

                acc = body;
            }
        }
    }

    Ok(acc)
}

fn collect_pat_vars(pat: &ast::Pattern, out: &mut std::collections::BTreeSet<String>) {
    use ast::Pattern;
    match pat {
        Pattern::Var(n) => {
            out.insert(n.clone());
        }
        Pattern::As(n, p) => {
            out.insert(n.clone());
            collect_pat_vars(p, out);
        }
        Pattern::Tuple(ps) | Pattern::List(ps) => {
            for p in ps {
                collect_pat_vars(p, out);
            }
        }
        Pattern::Record(fs) | Pattern::RecordLoose(fs) => {
            for (_, p) in fs {
                collect_pat_vars(p, out);
            }
        }
        Pattern::Cons(a, b) | Pattern::Or(a, b) => {
            collect_pat_vars(a, out);
            collect_pat_vars(b, out);
        }
        Pattern::Constructor { args, .. } => {
            for p in args {
                collect_pat_vars(p, out);
            }
        }
        Pattern::View(p, _) => collect_pat_vars(p, out),
        Pattern::Wildcard | Pattern::Hole(_) | Pattern::Literal(_) => {}
    }
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

fn lower_pat(pat: &ast::Pattern, fresh: &mut usize) -> Result<IrPattern> {
    use ast::Pattern;
    Ok(match pat {
        Pattern::Var(n) => IrPattern::Var(n.clone()),
        Pattern::Wildcard => IrPattern::Wildcard,
        Pattern::Hole(_) => IrPattern::Wildcard,
        Pattern::Literal(e) => IrPattern::Literal(lower_lit_expr(e)?),
        Pattern::Tuple(ps) => IrPattern::Tuple(
            ps.iter()
                .map(|p| lower_pat(p, fresh))
                .collect::<Result<Vec<_>>>()?,
        ),
        Pattern::List(ps) => IrPattern::List(
            ps.iter()
                .map(|p| lower_pat(p, fresh))
                .collect::<Result<Vec<_>>>()?,
        ),
        Pattern::Record(fields) => IrPattern::Record(
            fields
                .iter()
                .map(|(n, p)| Ok((n.clone(), lower_pat(p, fresh)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
        Pattern::RecordLoose(fields) => IrPattern::RecordLoose(
            fields
                .iter()
                .map(|(n, p)| Ok((n.clone(), lower_pat(p, fresh)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
        Pattern::Cons(a, b) => IrPattern::Cons(
            Box::new(lower_pat(a, fresh)?),
            Box::new(lower_pat(b, fresh)?),
        ),
        Pattern::Or(a, b) => IrPattern::Or(Box::new(lower_pat(a, fresh)?), Box::new(lower_pat(b, fresh)?)),
        Pattern::As(n, p) => IrPattern::As(n.clone(), Box::new(lower_pat(p, fresh)?)),
        Pattern::View(p, e) => IrPattern::View(
            Box::new(lower_pat(p, fresh)?),
            Box::new(lower_expr(e, fresh)?),
        ),
        Pattern::Constructor { name, args } => IrPattern::Constructor {
            name: name.clone(),
            args: args
                .iter()
                .map(|p| lower_pat(p, fresh))
                .collect::<Result<Vec<_>>>()?,
        },
    })
}

fn lower_expr(expr: &ast::Expr, fresh: &mut usize) -> Result<IrExpr> {
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
            body: Box::new(lower_expr(body, fresh)?),
        },
        Expr::Apply { func, args } => IrExpr::Apply {
            func: Box::new(lower_expr(func, fresh)?),
            args: args
                .iter()
                .map(|e| lower_expr(e, fresh))
                .collect::<Result<Vec<_>>>()?,
        },
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => IrExpr::If {
            cond: Box::new(lower_expr(cond, fresh)?),
            then_branch: Box::new(lower_expr(then_branch, fresh)?),
            else_branch: Box::new(lower_expr(else_branch, fresh)?),
        },
        Expr::Let { bindings, body } => {
            // Lower sequential let-bindings.
            let mut acc = lower_expr(body, fresh)?;
            for b in bindings.iter().rev() {
                match &b.pat {
                    ast::Pattern::Var(name) => {
                        acc = IrExpr::Let {
                            bindings: vec![(name.clone(), lower_expr(&b.expr, fresh)?)],
                            body: Box::new(acc),
                        };
                    }
                    pat => {
                        acc = IrExpr::Case {
                            expr: Box::new(lower_expr(&b.expr, fresh)?),
                            arms: vec![IrCaseArm {
                                pat: lower_pat(pat, fresh)?,
                                guard: None,
                                body: acc,
                            }],
                        };
                    }
                }
            }
            acc
        }
        Expr::List(es) => IrExpr::List(
            es.iter()
                .map(|e| lower_expr(e, fresh))
                .collect::<Result<Vec<_>>>()?,
        ),
        Expr::Tuple(es) => IrExpr::Tuple(
            es.iter()
                .map(|e| lower_expr(e, fresh))
                .collect::<Result<Vec<_>>>()?,
        ),
        Expr::Record(fields) => IrExpr::Record(
            fields
                .iter()
                .map(|(n, e)| Ok((n.clone(), lower_expr(e, fresh)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
        Expr::Case { expr, arms } => IrExpr::Case {
            expr: Box::new(lower_expr(expr, fresh)?),
            arms: arms
                .iter()
                .map(|a| {
                    Ok(IrCaseArm {
                        pat: lower_pat(&a.pat, fresh)?,
                        guard: a.guard.as_ref().map(|e| lower_expr(e, fresh)).transpose()?,
                        body: lower_expr(&a.body, fresh)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        },
        Expr::Do(stmts) => lower_do(stmts, fresh)?,
        Expr::Annot { expr, .. } => lower_expr(expr, fresh)?,
        Expr::Where { expr, bindings } => {
            // Lower sequential where-bindings.
            let mut acc = lower_expr(expr, fresh)?;
            for b in bindings.iter().rev() {
                match &b.pat {
                    ast::Pattern::Var(name) => {
                        acc = IrExpr::Let {
                            bindings: vec![(name.clone(), lower_expr(&b.expr, fresh)?)],
                            body: Box::new(acc),
                        };
                    }
                    pat => {
                        acc = IrExpr::Case {
                            expr: Box::new(lower_expr(&b.expr, fresh)?),
                            arms: vec![IrCaseArm {
                                pat: lower_pat(pat, fresh)?,
                                guard: None,
                                body: acc,
                            }],
                        };
                    }
                }
            }
            acc
        }
    })
}
