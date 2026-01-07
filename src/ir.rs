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
    Cons {
        head: Box<IrExpr>,
        tail: Box<IrExpr>,
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
        Expr::Cons { head, tail } => IrExpr::Cons {
            head: Box::new(lower_expr(head, fresh)?),
            tail: Box::new(lower_expr(tail, fresh)?),
        },
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

#[derive(Debug, Clone)]
pub enum IoAction {
    Pure(Value),
    StdoutWrite(String),
    StdinReadLine,
    Bind {
        action: Box<IoAction>,
        param: String,
        body: Box<IrExpr>,
        env: std::collections::HashMap<String, Value>,
    },
    Then {
        first: Box<IoAction>,
        then_expr: Box<IrExpr>,
        env: std::collections::HashMap<String, Value>,
    },
}

#[derive(Debug, Clone)]
pub enum Value {
    Unit,
    Integer(String),
    Float64(String),
    Bool(bool),
    String(String),
    Char(char),
    Tuple(Vec<Value>),
    ListNil,
    ListCons(Box<Value>, Box<Value>),
    Record(Vec<(String, Value)>),
    Thunk(std::rc::Rc<std::cell::RefCell<ThunkState>>),
    IoAction(Box<IoAction>),
    IoCtor,
    BuiltinStdoutWrite,
    Closure {
        params: Vec<String>,
        body: Box<IrExpr>,
        env: std::collections::HashMap<String, Value>,
    },
}

#[derive(Debug)]
pub enum ThunkState {
    Unevaluated {
        expr: IrExpr,
        env: std::collections::HashMap<String, Value>,
    },
    Evaluating,
    Evaluated(Value),
}

#[derive(Clone)]
enum MemoValue {
    Unevaluated,
    Evaluating,
    Evaluated(Value),
}

struct Globals {
    defs: std::collections::HashMap<String, IrExpr>,
    memo: std::cell::RefCell<std::collections::HashMap<String, MemoValue>>,
}

pub fn run_main(module: &IrModule) -> Result<Value> {
    let g = Globals::from_module(module);
    let v = eval_var(&g, &std::collections::HashMap::new(), "main")?;
    let Value::IoAction(action) = v else {
        return Err(Error::msg("main did not evaluate to an IO action"));
    };
    run_io(&g, *action)
}

impl Globals {
    fn from_module(module: &IrModule) -> Self {
        let mut defs = std::collections::HashMap::new();
        let mut memo = std::collections::HashMap::new();
        for it in &module.items {
            let IrItem::Binding { name, expr } = it;
            defs.insert(name.clone(), expr.clone());
            memo.insert(name.clone(), MemoValue::Unevaluated);
        }
        Self {
            defs,
            memo: std::cell::RefCell::new(memo),
        }
    }
}

fn force_value(g: &Globals, v: Value) -> Result<Value> {
    match v {
        Value::Thunk(t) => force_thunk(g, &t),
        other => Ok(other),
    }
}

fn force_thunk(g: &Globals, t: &std::rc::Rc<std::cell::RefCell<ThunkState>>) -> Result<Value> {
    {
        let st = t.borrow();
        if let ThunkState::Evaluated(v) = &*st {
            return Ok(v.clone());
        }
        if matches!(&*st, ThunkState::Evaluating) {
            return Err(Error::msg("cyclic thunk"));
        }
    }

    let (expr, env) = {
        let mut st = t.borrow_mut();
        match std::mem::replace(&mut *st, ThunkState::Evaluating) {
            ThunkState::Unevaluated { expr, env } => (expr, env),
            ThunkState::Evaluated(v) => {
                *st = ThunkState::Evaluated(v.clone());
                return Ok(v);
            }
            ThunkState::Evaluating => {
                *st = ThunkState::Evaluating;
                return Err(Error::msg("cyclic thunk"));
            }
        }
    };

    let v = eval_expr(g, &env, &expr)?;
    *t.borrow_mut() = ThunkState::Evaluated(v.clone());
    Ok(v)
}

fn eval_var(g: &Globals, env: &std::collections::HashMap<String, Value>, name: &str) -> Result<Value> {
    if let Some(v) = env.get(name) {
        return force_value(g, v.clone());
    }

    if name == "IO" {
        // Built-in IO constructor used by the minimal typecheck prelude.
        return Ok(Value::IoCtor);
    }

    if name == "stdoutWrite" {
        return Ok(Value::BuiltinStdoutWrite);
    }

    if name == "stdinReadLine" {
        return Ok(Value::IoAction(Box::new(IoAction::StdinReadLine)));
    }

    if name == "readLine" {
        // NOTE: currently a builtin for early ergonomics.
        // In the future, `readLine` should become a library function built on top of IO primitives
        // such as `stdinReadLine`.
        return Ok(Value::IoAction(Box::new(IoAction::StdinReadLine)));
    }

    if name == "print" {
        // NOTE: temporary name for observability.
        // In the future, `print` should become a library function built on top of IO primitives
        // such as `stdoutWrite`.
        return Ok(Value::BuiltinStdoutWrite);
    }

    if !g.defs.contains_key(name) {
        return Err(Error::msg(format!("unbound variable: {name}")));
    }

    match g.memo.borrow().get(name).cloned() {
        Some(MemoValue::Evaluated(v)) => return Ok(v),
        Some(MemoValue::Evaluating) => {
            return Err(Error::msg(format!("cyclic definition: {name}")))
        }
        Some(MemoValue::Unevaluated) => {}
        None => {}
    }

    g.memo
        .borrow_mut()
        .insert(name.to_string(), MemoValue::Evaluating);

    let expr = g.defs.get(name).unwrap().clone();
    let v = eval_expr(g, &std::collections::HashMap::new(), &expr)?;
    g.memo
        .borrow_mut()
        .insert(name.to_string(), MemoValue::Evaluated(v.clone()));
    Ok(v)
}

fn eval_expr(
    g: &Globals,
    env: &std::collections::HashMap<String, Value>,
    expr: &IrExpr,
) -> Result<Value> {
    Ok(match expr {
        IrExpr::Unit => Value::Unit,
        IrExpr::Integer(s) => Value::Integer(s.clone()),
        IrExpr::Float64(s) => Value::Float64(s.clone()),
        IrExpr::Bool(b) => Value::Bool(*b),
        IrExpr::String(s) => Value::String(s.clone()),
        IrExpr::Char(c) => Value::Char(*c),
        IrExpr::Var(n) => eval_var(g, env, n)?,
        IrExpr::Lambda { params, body } => Value::Closure {
            params: params.clone(),
            body: body.clone(),
            env: env.clone(),
        },
        IrExpr::Apply { func, args } => {
            let mut f = eval_expr(g, env, func)?;
            for a in args {
                let t = std::rc::Rc::new(std::cell::RefCell::new(ThunkState::Unevaluated {
                    expr: a.clone(),
                    env: env.clone(),
                }));
                f = apply_one(g, f, Value::Thunk(t))?;
            }
            f
        }
        IrExpr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            let Value::Bool(b) = eval_expr(g, env, cond)? else {
                return Err(Error::msg("if condition did not evaluate to Bool"));
            };
            if b {
                eval_expr(g, env, then_branch)?
            } else {
                eval_expr(g, env, else_branch)?
            }
        }
        IrExpr::Let { bindings, body } => {
            let mut env2 = env.clone();
            for (name, e) in bindings {
                let t = std::rc::Rc::new(std::cell::RefCell::new(ThunkState::Unevaluated {
                    expr: e.clone(),
                    env: env2.clone(),
                }));
                env2.insert(name.clone(), Value::Thunk(t));
            }
            eval_expr(g, &env2, body)?
        }
        IrExpr::Cons { head, tail } => {
            let hd = Value::Thunk(std::rc::Rc::new(std::cell::RefCell::new(ThunkState::Unevaluated {
                expr: (**head).clone(),
                env: env.clone(),
            })));
            let tl = Value::Thunk(std::rc::Rc::new(std::cell::RefCell::new(ThunkState::Unevaluated {
                expr: (**tail).clone(),
                env: env.clone(),
            })));
            Value::ListCons(Box::new(hd), Box::new(tl))
        }
        IrExpr::List(es) => {
            let mut out = Value::ListNil;
            for e in es.iter().rev() {
                let hd = Value::Thunk(std::rc::Rc::new(std::cell::RefCell::new(ThunkState::Unevaluated {
                    expr: e.clone(),
                    env: env.clone(),
                })));
                out = Value::ListCons(Box::new(hd), Box::new(out));
            }
            out
        },
        IrExpr::Tuple(es) => Value::Tuple(
            es.iter()
                .map(|e| {
                    Value::Thunk(std::rc::Rc::new(std::cell::RefCell::new(
                        ThunkState::Unevaluated {
                            expr: e.clone(),
                            env: env.clone(),
                        },
                    )))
                })
                .collect(),
        ),
        IrExpr::Record(fields) => Value::Record(
            fields
                .iter()
                .map(|(n, e)| {
                    (
                        n.clone(),
                        Value::Thunk(std::rc::Rc::new(std::cell::RefCell::new(
                            ThunkState::Unevaluated {
                                expr: e.clone(),
                                env: env.clone(),
                            },
                        ))),
                    )
                })
                .collect(),
        ),
        IrExpr::Case { expr, arms } => {
            let scrut = eval_expr(g, env, expr)?;
            for arm in arms {
                if let Some(binds) = match_pat(g, env, &arm.pat, &scrut)? {
                    let mut env_arm = env.clone();
                    env_arm.extend(binds);
                    if let Some(guard) = &arm.guard {
                        let Value::Bool(b) = eval_expr(g, &env_arm, guard)? else {
                            return Err(Error::msg("case guard did not evaluate to Bool"));
                        };
                        if !b {
                            continue;
                        }
                    }
                    return eval_expr(g, &env_arm, &arm.body);
                }
            }
            return Err(Error::msg("non-exhaustive case"));
        }
        IrExpr::IoBind { action, param, body } => {
            let act = eval_expr(g, env, action)?;
            let Value::IoAction(act) = act else {
                return Err(Error::msg("IoBind action did not evaluate to an IO action"));
            };
            Value::IoAction(Box::new(IoAction::Bind {
                action: act,
                param: param.clone(),
                body: body.clone(),
                env: env.clone(),
            }))
        }
        IrExpr::IoThen { first, then_expr } => {
            let act = eval_expr(g, env, first)?;
            let Value::IoAction(act) = act else {
                return Err(Error::msg("IoThen first did not evaluate to an IO action"));
            };
            Value::IoAction(Box::new(IoAction::Then {
                first: act,
                then_expr: then_expr.clone(),
                env: env.clone(),
            }))
        }
    })
}

fn run_io(g: &Globals, action: IoAction) -> Result<Value> {
    match action {
        IoAction::Pure(v) => force_value(g, v),
        IoAction::StdoutWrite(s) => {
            use std::io::Write;
            print!("{s}");
            std::io::stdout().flush().ok();
            Ok(Value::Unit)
        }
        IoAction::StdinReadLine => {
            use std::io::BufRead;
            let mut s = String::new();
            std::io::stdin().lock().read_line(&mut s)?;
            while s.ends_with(['\n', '\r']) {
                s.pop();
            }
            Ok(Value::String(s))
        }
        IoAction::Bind {
            action,
            param,
            body,
            mut env,
        } => {
            let v = run_io(g, *action)?;
            env.insert(param, v);
            let act = eval_expr(g, &env, &body)?;
            let Value::IoAction(act) = act else {
                return Err(Error::msg("IoBind body did not evaluate to an IO action"));
            };
            run_io(g, *act)
        }
        IoAction::Then {
            first,
            then_expr,
            env,
        } => {
            let _ = run_io(g, *first)?;
            let act = eval_expr(g, &env, &then_expr)?;
            let Value::IoAction(act) = act else {
                return Err(Error::msg("IoThen body did not evaluate to an IO action"));
            };
            run_io(g, *act)
        }
    }
}

fn apply_one(g: &Globals, fun: Value, arg: Value) -> Result<Value> {
    match fun {
        Value::IoCtor => Ok(Value::IoAction(Box::new(IoAction::Pure(arg)))),
        Value::BuiltinStdoutWrite => {
            let arg = force_value(g, arg)?;
            let Value::String(s) = arg else {
                return Err(Error::msg("stdoutWrite expects String"));
            };
            Ok(Value::IoAction(Box::new(IoAction::StdoutWrite(s))))
        }
        Value::Closure {
            mut params,
            body,
            mut env,
        } => {
            let Some(p) = params.first().cloned() else {
                return Err(Error::msg("cannot apply function with no params"));
            };
            params.remove(0);
            env.insert(p, arg);
            if params.is_empty() {
                eval_expr(g, &env, &body)
            } else {
                Ok(Value::Closure { params, body, env })
            }
        }
        Value::Integer(_)
        | Value::Float64(_)
        | Value::Bool(_)
        | Value::String(_)
        | Value::Char(_)
        | Value::Unit
        | Value::Tuple(_)
        | Value::ListNil
        | Value::ListCons(_, _)
        | Value::Record(_)
        | Value::Thunk(_)
        | Value::IoAction(_) => Err(Error::msg("attempted to apply a non-function")),
    }
}

fn match_pat(
    g: &Globals,
    env: &std::collections::HashMap<String, Value>,
    pat: &IrPattern,
    val: &Value,
) -> Result<Option<std::collections::HashMap<String, Value>>> {
    use IrPattern as P;

    match pat {
        P::Wildcard => return Ok(Some(std::collections::HashMap::new())),
        P::Var(n) => {
            let mut m = std::collections::HashMap::new();
            m.insert(n.clone(), val.clone());
            return Ok(Some(m));
        }
        _ => {}
    }

    let val = force_value(g, val.clone())?;
    Ok(match (pat, &val) {
        (P::Literal(l), v) => {
            let ok = match (l, v) {
                (IrLiteral::Unit, Value::Unit) => true,
                (IrLiteral::Integer(a), Value::Integer(b)) => a == b,
                (IrLiteral::Float64(a), Value::Float64(b)) => a == b,
                (IrLiteral::Bool(a), Value::Bool(b)) => a == b,
                (IrLiteral::String(a), Value::String(b)) => a == b,
                (IrLiteral::Char(a), Value::Char(b)) => a == b,
                _ => false,
            };
            if ok { Some(std::collections::HashMap::new()) } else { None }
        }
        (P::Tuple(ps), Value::Tuple(vs)) if ps.len() == vs.len() => {
            let mut out = std::collections::HashMap::new();
            for (p, v) in ps.iter().zip(vs.iter()) {
                let Some(b) = match_pat(g, env, p, v)? else { return Ok(None) };
                out.extend(b);
            }
            Some(out)
        }
        (P::List(ps), v) => {
            let mut out = std::collections::HashMap::new();
            let mut cur = v.clone();
            for p in ps.iter() {
                let cur_forced = force_value(g, cur)?;
                let Value::ListCons(h, t) = cur_forced else { return Ok(None) };
                let Some(b) = match_pat(g, env, p, &h)? else { return Ok(None) };
                out.extend(b);
                cur = *t;
            }
            let cur = force_value(g, cur)?;
            if matches!(cur, Value::ListNil) { Some(out) } else { None }
        }
        (P::Cons(hd, tl), v) => {
            let v = v.clone();
            let v = force_value(g, v)?;
            let Value::ListCons(h, t) = v else { return Ok(None) };
            let mut out = std::collections::HashMap::new();
            let Some(b_hd) = match_pat(g, env, hd, &h)? else { return Ok(None) };
            out.extend(b_hd);
            let Some(b_tl) = match_pat(g, env, tl, &t)? else { return Ok(None) };
            out.extend(b_tl);
            Some(out)
        }
        (P::Record(fs), Value::Record(vs)) => {
            if fs.len() != vs.len() {
                return Ok(None);
            }
            let mut out = std::collections::HashMap::new();
            for (name, p) in fs {
                let Some((_, v)) = vs.iter().find(|(n, _)| n == name) else {
                    return Ok(None);
                };
                let Some(b) = match_pat(g, env, p, v)? else { return Ok(None) };
                out.extend(b);
            }
            Some(out)
        }
        (P::RecordLoose(fs), Value::Record(vs)) => {
            let mut out = std::collections::HashMap::new();
            for (name, p) in fs {
                let Some((_, v)) = vs.iter().find(|(n, _)| n == name) else {
                    return Ok(None);
                };
                let Some(b) = match_pat(g, env, p, v)? else { return Ok(None) };
                out.extend(b);
            }
            Some(out)
        }
        (P::As(n, p), v) => {
            let Some(mut b) = match_pat(g, env, p, v)? else { return Ok(None) };
            b.insert(n.clone(), v.clone());
            Some(b)
        }
        (P::Or(a, b), v) => {
            if let Some(binds) = match_pat(g, env, a, v)? {
                Some(binds)
            } else {
                match_pat(g, env, b, v)?
            }
        }
        (P::View(p, e), v) => {
            let fv = eval_expr(g, env, e)?;
            let v2 = apply_one(g, fv, v.clone())?;
            match_pat(g, env, p, &v2)?
        }
        _ => None,
    })
}
