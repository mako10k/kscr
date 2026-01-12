//! IR scaffolding.

use crate::{ast, error::Error, Result};

#[cfg(feature = "unsafe_bigint")]
type Integer = kscr_unsafe_bigint::Integer;
#[cfg(not(feature = "unsafe_bigint"))]
type Integer = crate::safe_bigint::Integer;

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
    RecordLoose(Vec<(String, IrPattern)>, Option<String>),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastTarget {
    I32,
    I64,
    F32,
    F64,
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
    Lambda {
        params: Vec<String>,
        body: Box<IrExpr>,
    },
    Apply {
        func: Box<IrExpr>,
        args: Vec<IrExpr>,
    },
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
    CheckedCast {
        expr: Box<IrExpr>,
        target: CastTarget,
    },
}

fn last_ty_seg(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

fn cast_target_from_type(ty: &ast::Type) -> Option<CastTarget> {
    match ty {
        ast::Type::Var(name) => match last_ty_seg(name) {
            "i32" => Some(CastTarget::I32),
            "i64" => Some(CastTarget::I64),
            "f32" => Some(CastTarget::F32),
            "f64" => Some(CastTarget::F64),
            _ => None,
        },
        _ => None,
    }
}

fn collect_ctor_aliases(module: &ast::Module) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for it in &module.items {
        let ast::Item::Binding(b) = it else {
            continue;
        };
        let ast::PatternKind::Var(name) = &b.pat.kind else {
            continue;
        };
        match &b.expr.kind {
            ast::ExprKind::Var(v) | ast::ExprKind::Ctor(v) if v.contains('.') => {
                out.insert(name.clone(), v.clone());
            }
            _ => {}
        }
    }
    out
}

pub fn lower_to_ir(module: &ast::Module) -> Result<IrModule> {
    let mut items = Vec::new();
    let mut fresh = 0usize;
    let ctor_aliases = collect_ctor_aliases(module);

    // Lower `data` declarations into constructor bindings.
    // For now, constructors are encoded as records: { __ctor: "CtorName", __args: [...] }.
    for it in &module.items {
        let ast::Item::DataDecl(d) = it else {
            continue;
        };

        for ctor in &d.ctors {
            let ctor_name = ctor.name.clone();
            let arity = ctor.args.len();

            let args_expr = IrExpr::List(
                (0..arity)
                    .map(|i| IrExpr::Var(format!("_arg{i}")))
                    .collect(),
            );
            let body = IrExpr::Record(vec![
                ("__ctor".to_string(), IrExpr::String(ctor_name.clone())),
                ("__args".to_string(), args_expr),
            ]);

            let expr = if arity == 0 {
                body
            } else {
                IrExpr::Lambda {
                    params: (0..arity).map(|i| format!("_arg{i}")).collect(),
                    body: Box::new(body),
                }
            };

            items.push(IrItem::Binding {
                name: ctor_name,
                expr,
            });
        }
    }

    for it in &module.items {
        let ast::Item::Binding(b) = it else {
            continue;
        };

        match &b.pat {
            ast::Pattern {
                kind: ast::PatternKind::Var(name),
                ..
            } => {
                let expr = lower_expr(&b.expr, &mut fresh, &ctor_aliases)?;
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
                    expr: lower_expr(&b.expr, &mut fresh, &ctor_aliases)?,
                });

                let ir_pat = lower_pat(pat, &mut fresh, &ctor_aliases)?;
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

fn lower_do(
    stmts: &[ast::DoStmt],
    fresh: &mut usize,
    ctor_aliases: &std::collections::HashMap<String, String>,
) -> Result<IrExpr> {
    if stmts.is_empty() {
        return Err(Error::msg("empty do"));
    }

    let mut it = stmts.iter().rev();
    let Some(ast::DoStmt::Expr(last)) = it.next() else {
        return Err(Error::msg("do must end with expression"));
    };
    let mut acc = lower_expr(last, fresh, ctor_aliases)?;

    for stmt in it {
        match stmt {
            ast::DoStmt::Expr(e) => {
                acc = IrExpr::IoThen {
                    first: Box::new(lower_expr(e, fresh, ctor_aliases)?),
                    then_expr: Box::new(acc),
                };
            }
            ast::DoStmt::Bind { pat, expr } => {
                let tmp = format!("_do{fresh}");
                *fresh += 1;

                let body = match pat {
                    ast::Pattern {
                        kind: ast::PatternKind::Var(name),
                        ..
                    } => IrExpr::IoBind {
                        action: Box::new(lower_expr(expr, fresh, ctor_aliases)?),
                        param: name.clone(),
                        body: Box::new(acc),
                    },
                    other => {
                        let ir_pat = lower_pat(other, fresh, ctor_aliases)?;
                        IrExpr::IoBind {
                            action: Box::new(lower_expr(expr, fresh, ctor_aliases)?),
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
    use ast::PatternKind;
    match &pat.kind {
        PatternKind::Var(n) => {
            out.insert(n.clone());
        }
        PatternKind::As(n, p) => {
            out.insert(n.clone());
            collect_pat_vars(p, out);
        }
        PatternKind::Tuple(ps) | PatternKind::List(ps) => {
            for p in ps {
                collect_pat_vars(p, out);
            }
        }
        PatternKind::Record(fs) => {
            for (_, p) in fs {
                collect_pat_vars(p, out);
            }
        }
        PatternKind::RecordLoose(fs, rest) => {
            for (_, p) in fs {
                collect_pat_vars(p, out);
            }
            if let Some(n) = rest {
                out.insert(n.clone());
            }
        }
        PatternKind::Cons(a, b) | PatternKind::Or(a, b) => {
            collect_pat_vars(a, out);
            collect_pat_vars(b, out);
        }
        PatternKind::Constructor { args, .. } => {
            for p in args {
                collect_pat_vars(p, out);
            }
        }
        PatternKind::View(p, _) => collect_pat_vars(p, out),
        PatternKind::Wildcard | PatternKind::Hole(_) | PatternKind::Literal(_) => {}
    }
}

fn lower_lit_expr(expr: &ast::Expr) -> Result<IrLiteral> {
    use ast::ExprKind;
    Ok(match &expr.kind {
        ExprKind::Unit => IrLiteral::Unit,
        ExprKind::Integer(s) => IrLiteral::Integer(s.clone()),
        ExprKind::Float64(s) => IrLiteral::Float64(s.clone()),
        ExprKind::Bool(b) => IrLiteral::Bool(*b),
        ExprKind::String(s) => IrLiteral::String(s.clone()),
        ExprKind::Char(c) => IrLiteral::Char(*c),
        _ => return Err(Error::msg("unsupported literal")),
    })
}

fn lower_pat(
    pat: &ast::Pattern,
    fresh: &mut usize,
    ctor_aliases: &std::collections::HashMap<String, String>,
) -> Result<IrPattern> {
    use ast::PatternKind;
    Ok(match &pat.kind {
        PatternKind::Var(n) => IrPattern::Var(n.clone()),
        PatternKind::Wildcard => IrPattern::Wildcard,
        PatternKind::Hole(_) => IrPattern::Wildcard,
        PatternKind::Literal(e) => IrPattern::Literal(lower_lit_expr(e)?),
        PatternKind::Tuple(ps) => IrPattern::Tuple(
            ps.iter()
                .map(|p| lower_pat(p, fresh, ctor_aliases))
                .collect::<Result<Vec<_>>>()?,
        ),
        PatternKind::List(ps) => IrPattern::List(
            ps.iter()
                .map(|p| lower_pat(p, fresh, ctor_aliases))
                .collect::<Result<Vec<_>>>()?,
        ),
        PatternKind::Record(fields) => IrPattern::Record(
            fields
                .iter()
                .map(|(n, p)| Ok((n.clone(), lower_pat(p, fresh, ctor_aliases)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
        PatternKind::RecordLoose(fields, rest) => IrPattern::RecordLoose(
            fields
                .iter()
                .map(|(n, p)| Ok((n.clone(), lower_pat(p, fresh, ctor_aliases)?)))
                .collect::<Result<Vec<_>>>()?,
            rest.clone(),
        ),
        PatternKind::Cons(a, b) => IrPattern::Cons(
            Box::new(lower_pat(a, fresh, ctor_aliases)?),
            Box::new(lower_pat(b, fresh, ctor_aliases)?),
        ),
        PatternKind::Or(a, b) => IrPattern::Or(
            Box::new(lower_pat(a, fresh, ctor_aliases)?),
            Box::new(lower_pat(b, fresh, ctor_aliases)?),
        ),
        PatternKind::As(n, p) => {
            IrPattern::As(n.clone(), Box::new(lower_pat(p, fresh, ctor_aliases)?))
        }
        PatternKind::View(p, e) => IrPattern::View(
            Box::new(lower_pat(p, fresh, ctor_aliases)?),
            Box::new(lower_expr(e, fresh, ctor_aliases)?),
        ),
        PatternKind::Constructor { name, args } => {
            let name = if name.contains('.') {
                name.clone()
            } else {
                ctor_aliases
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| name.clone())
            };
            IrPattern::Constructor {
                name,
                args: args
                    .iter()
                    .map(|p| lower_pat(p, fresh, ctor_aliases))
                    .collect::<Result<Vec<_>>>()?,
            }
        }
    })
}

fn lower_expr(
    expr: &ast::Expr,
    fresh: &mut usize,
    ctor_aliases: &std::collections::HashMap<String, String>,
) -> Result<IrExpr> {
    use ast::{ExprKind, PatternKind};
    Ok(match &expr.kind {
        // literals
        ExprKind::Unit => IrExpr::Unit,
        ExprKind::Integer(s) => IrExpr::Integer(s.clone()),
        ExprKind::Float64(s) => IrExpr::Float64(s.clone()),
        ExprKind::Bool(b) => IrExpr::Bool(*b),
        ExprKind::String(s) => IrExpr::String(s.clone()),
        ExprKind::Char(c) => IrExpr::Char(*c),
        ExprKind::Var(v) => IrExpr::Var(v.clone()),
        ExprKind::Ctor(v) => IrExpr::Var(v.clone()),
        ExprKind::Lambda { params, body } => IrExpr::Lambda {
            params: params.clone(),
            body: Box::new(lower_expr(body, fresh, ctor_aliases)?),
        },
        ExprKind::Apply { func, args } => IrExpr::Apply {
            func: Box::new(lower_expr(func, fresh, ctor_aliases)?),
            args: args
                .iter()
                .map(|e| lower_expr(e, fresh, ctor_aliases))
                .collect::<Result<Vec<_>>>()?,
        },
        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => IrExpr::If {
            cond: Box::new(lower_expr(cond, fresh, ctor_aliases)?),
            then_branch: Box::new(lower_expr(then_branch, fresh, ctor_aliases)?),
            else_branch: Box::new(lower_expr(else_branch, fresh, ctor_aliases)?),
        },
        ExprKind::Let { bindings, body } => {
            // Lower sequential let-bindings.
            let mut acc = lower_expr(body, fresh, ctor_aliases)?;
            for b in bindings.iter().rev() {
                match &b.pat.kind {
                    PatternKind::Var(name) => {
                        acc = IrExpr::Let {
                            bindings: vec![(
                                name.clone(),
                                lower_expr(&b.expr, fresh, ctor_aliases)?,
                            )],
                            body: Box::new(acc),
                        };
                    }
                    pat => {
                        acc = IrExpr::Case {
                            expr: Box::new(lower_expr(&b.expr, fresh, ctor_aliases)?),
                            arms: vec![IrCaseArm {
                                pat: lower_pat(
                                    &ast::Pattern {
                                        span: b.pat.span,
                                        kind: pat.clone(),
                                    },
                                    fresh,
                                    ctor_aliases,
                                )?,
                                guard: None,
                                body: acc,
                            }],
                        };
                    }
                }
            }
            acc
        }
        ExprKind::Cons { head, tail } => IrExpr::Cons {
            head: Box::new(lower_expr(head, fresh, ctor_aliases)?),
            tail: Box::new(lower_expr(tail, fresh, ctor_aliases)?),
        },
        ExprKind::List(es) => IrExpr::List(
            es.iter()
                .map(|e| lower_expr(e, fresh, ctor_aliases))
                .collect::<Result<Vec<_>>>()?,
        ),
        ExprKind::Tuple(es) => IrExpr::Tuple(
            es.iter()
                .map(|e| lower_expr(e, fresh, ctor_aliases))
                .collect::<Result<Vec<_>>>()?,
        ),
        ExprKind::Record(fields) => IrExpr::Record(
            fields
                .iter()
                .map(|(n, e): &(String, ast::Expr)| {
                    Ok((n.clone(), lower_expr(e, fresh, ctor_aliases)?))
                })
                .collect::<Result<Vec<_>>>()?,
        ),
        ExprKind::Case { expr, arms } => IrExpr::Case {
            expr: Box::new(lower_expr(expr, fresh, ctor_aliases)?),
            arms: arms
                .iter()
                .map(|a| {
                    Ok(IrCaseArm {
                        pat: lower_pat(&a.pat, fresh, ctor_aliases)?,
                        guard: a
                            .guard
                            .as_ref()
                            .map(|e| lower_expr(e, fresh, ctor_aliases))
                            .transpose()?,
                        body: lower_expr(&a.body, fresh, ctor_aliases)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        },
        ExprKind::Do(stmts) => lower_do(stmts, fresh, ctor_aliases)?,
        ExprKind::Annot { expr, ty } => {
            let inner = lower_expr(expr, fresh, ctor_aliases)?;
            if let Some(target) = cast_target_from_type(&ty.ty) {
                IrExpr::CheckedCast {
                    expr: Box::new(inner),
                    target,
                }
            } else {
                inner
            }
        }
        ExprKind::Where { expr, bindings } => {
            // Lower sequential where-bindings.
            let mut acc = lower_expr(expr, fresh, ctor_aliases)?;
            for b in bindings.iter().rev() {
                match &b.pat.kind {
                    PatternKind::Var(name) => {
                        acc = IrExpr::Let {
                            bindings: vec![(
                                name.clone(),
                                lower_expr(&b.expr, fresh, ctor_aliases)?,
                            )],
                            body: Box::new(acc),
                        };
                    }
                    pat => {
                        acc = IrExpr::Case {
                            expr: Box::new(lower_expr(&b.expr, fresh, ctor_aliases)?),
                            arms: vec![IrCaseArm {
                                pat: lower_pat(
                                    &ast::Pattern {
                                        span: b.pat.span,
                                        kind: pat.clone(),
                                    },
                                    fresh,
                                    ctor_aliases,
                                )?,
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

    #[cfg(feature = "unsafe_ffi")]
    FfiPuts(String),

    // Exceptions via IO (MVP: String exceptions)
    Throw(String),
    Catch {
        action: Box<IoAction>,
        handler: Value,
    },
    Try {
        action: Box<IoAction>,
    },

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
    Integer(Integer),
    Float64(f64),
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
    BuiltinConcatMap,
    BuiltinConcatMap1(Box<Value>),
    BuiltinAdd,
    BuiltinAdd1(Box<Value>),
    BuiltinSub,
    BuiltinSub1(Box<Value>),
    BuiltinMul,
    BuiltinMul1(Box<Value>),
    BuiltinDiv,
    BuiltinDiv1(Box<Value>),
    BuiltinEq,
    BuiltinEq1(Box<Value>),
    BuiltinEqInt,
    BuiltinEqInt1(Box<Value>),
    BuiltinLtInt,
    BuiltinLtInt1(Box<Value>),
    BuiltinLeInt,
    BuiltinLeInt1(Box<Value>),
    BuiltinGtInt,
    BuiltinGtInt1(Box<Value>),
    BuiltinGeInt,
    BuiltinGeInt1(Box<Value>),
    BuiltinNe,
    BuiltinNe1(Box<Value>),
    BuiltinNeInt,
    BuiltinNeInt1(Box<Value>),
    BuiltinAnd,
    BuiltinAnd1(Box<Value>),
    BuiltinOr,
    BuiltinOr1(Box<Value>),
    BuiltinNot,
    BuiltinIntToString,
    BuiltinBoolToString,
    BuiltinStrAppend,
    BuiltinStrAppend1(Box<Value>),
    BuiltinShow,
    BuiltinShowDictApply,
    BuiltinShowDictApply1(Box<Value>),
    BuiltinEqDictApply,
    BuiltinEqDictApply1(Box<Value>),
    BuiltinEqDictApply2(Box<Value>, Box<Value>),
    BuiltinRecordGet,
    BuiltinRecordGet1(Box<Value>),
    BuiltinThrow,
    BuiltinCatch,
    BuiltinCatch1(Box<Value>),
    BuiltinTry,
    BuiltinFfiAddI32,
    BuiltinFfiAddI32_1(Box<Value>),
    BuiltinFfiAddF32,
    BuiltinFfiAddF32_1(Box<Value>),
    #[cfg(feature = "unsafe_ffi")]
    BuiltinFfiPuts,
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

#[derive(Debug)]
enum IoOutcome {
    Value(Value),
    Thrown(String),
}

pub fn run_main(module: &IrModule) -> Result<Value> {
    let g = Globals::from_module(module);
    let v = eval_var(&g, &std::collections::HashMap::new(), "main")?;
    let Value::IoAction(action) = v else {
        return Err(Error::msg("main did not evaluate to an IO action"));
    };
    match run_io(&g, *action)? {
        IoOutcome::Value(v) => Ok(v),
        IoOutcome::Thrown(e) => Err(Error::msg(format!("uncaught exception: {e}"))),
    }
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

fn eval_var(
    g: &Globals,
    env: &std::collections::HashMap<String, Value>,
    name: &str,
) -> Result<Value> {
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

    if name == "concatMap" {
        return Ok(Value::BuiltinConcatMap);
    }

    if name == "+" {
        return Ok(Value::BuiltinAdd);
    }

    if name == "-" {
        return Ok(Value::BuiltinSub);
    }

    if name == "*" {
        return Ok(Value::BuiltinMul);
    }

    if name == "/" {
        return Ok(Value::BuiltinDiv);
    }

    if name == "==" {
        return Ok(Value::BuiltinEq);
    }

    if name == "<" {
        return Ok(Value::BuiltinLtInt);
    }

    if name == "<=" {
        return Ok(Value::BuiltinLeInt);
    }

    if name == ">" {
        return Ok(Value::BuiltinGtInt);
    }

    if name == ">=" {
        return Ok(Value::BuiltinGeInt);
    }

    if name == "/=" {
        return Ok(Value::BuiltinNe);
    }

    if name == "&&" {
        return Ok(Value::BuiltinAnd);
    }

    if name == "||" {
        return Ok(Value::BuiltinOr);
    }

    if name == "not" {
        return Ok(Value::BuiltinNot);
    }

    if name == "intToString" {
        return Ok(Value::BuiltinIntToString);
    }

    if name == "boolToString" {
        return Ok(Value::BuiltinBoolToString);
    }

    if name == "++" {
        return Ok(Value::BuiltinStrAppend);
    }

    if name == "show" || name == "toString" {
        return Ok(Value::BuiltinShow);
    }

    if name == "__show" || name == "__toString" {
        return Ok(Value::BuiltinShowDictApply);
    }

    if name == "__builtinShowDict" {
        return Ok(Value::Record(vec![(
            "show".to_string(),
            Value::BuiltinShow,
        )]));
    }

    if name == "__eq" {
        return Ok(Value::BuiltinEqDictApply);
    }

    if name == "__builtinEqDict" {
        return Ok(Value::Record(vec![("eq".to_string(), Value::BuiltinEq)]));
    }

    if name == "__recordGet" {
        return Ok(Value::BuiltinRecordGet);
    }

    if name == "throw" {
        return Ok(Value::BuiltinThrow);
    }

    if name == "catch" {
        return Ok(Value::BuiltinCatch);
    }

    if name == "try" {
        return Ok(Value::BuiltinTry);
    }

    if name == "ffiAddI32" {
        return Ok(Value::BuiltinFfiAddI32);
    }

    if name == "ffiAddF32" {
        return Ok(Value::BuiltinFfiAddF32);
    }

    if name == "ffiPuts" {
        #[cfg(feature = "unsafe_ffi")]
        return Ok(Value::BuiltinFfiPuts);
    }

    if name == "stdinReadLine" {
        return Ok(Value::IoAction(Box::new(IoAction::StdinReadLine)));
    }

    if name == "readLine" && !g.defs.contains_key(name) {
        // NOTE: currently a builtin for early ergonomics.
        // In the future, `readLine` should become a library function built on top of IO primitives
        // such as `stdinReadLine`.
        return Ok(Value::IoAction(Box::new(IoAction::StdinReadLine)));
    }

    if name == "print" && !g.defs.contains_key(name) {
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
        IrExpr::Integer(s) => Value::Integer(parse_integer(s)?),
        IrExpr::Float64(s) => Value::Float64(parse_f64(s)?),
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
            let hd = Value::Thunk(std::rc::Rc::new(std::cell::RefCell::new(
                ThunkState::Unevaluated {
                    expr: (**head).clone(),
                    env: env.clone(),
                },
            )));
            let tl = Value::Thunk(std::rc::Rc::new(std::cell::RefCell::new(
                ThunkState::Unevaluated {
                    expr: (**tail).clone(),
                    env: env.clone(),
                },
            )));
            Value::ListCons(Box::new(hd), Box::new(tl))
        }
        IrExpr::List(es) => {
            let mut out = Value::ListNil;
            for e in es.iter().rev() {
                let hd = Value::Thunk(std::rc::Rc::new(std::cell::RefCell::new(
                    ThunkState::Unevaluated {
                        expr: e.clone(),
                        env: env.clone(),
                    },
                )));
                out = Value::ListCons(Box::new(hd), Box::new(out));
            }
            out
        }
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
        IrExpr::CheckedCast { expr, target } => {
            let v = eval_expr(g, env, expr)?;
            checked_cast(g, v, *target)?
        }
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
        IrExpr::IoBind {
            action,
            param,
            body,
        } => {
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

fn run_io(g: &Globals, action: IoAction) -> Result<IoOutcome> {
    match action {
        IoAction::Pure(v) => Ok(IoOutcome::Value(force_value(g, v)?)),
        IoAction::StdoutWrite(s) => {
            use std::io::Write;
            print!("{s}");
            std::io::stdout().flush().ok();
            Ok(IoOutcome::Value(Value::Unit))
        }
        IoAction::StdinReadLine => {
            use std::io::BufRead;
            let mut s = String::new();
            std::io::stdin().lock().read_line(&mut s)?;
            while s.ends_with(['\n', '\r']) {
                s.pop();
            }
            Ok(IoOutcome::Value(Value::String(s)))
        }

        #[cfg(feature = "unsafe_ffi")]
        IoAction::FfiPuts(s) => {
            crate::debug::unsafe_used("ffiPuts");
            let rc = kscr_unsafe_ffi::puts_checked(&s)
                .map_err(|_| Error::msg("ffiPuts: string contains NUL"))?;
            Ok(IoOutcome::Value(Value::Integer(int_from_i64(rc as i64))))
        }

        IoAction::Throw(e) => Ok(IoOutcome::Thrown(e)),
        IoAction::Catch { action, handler } => match run_io(g, *action)? {
            IoOutcome::Value(v) => Ok(IoOutcome::Value(v)),
            IoOutcome::Thrown(e) => {
                let h = force_value(g, handler)?;
                let act = apply_one(g, h, Value::String(e))?;
                let Value::IoAction(act) = act else {
                    return Err(Error::msg("catch handler did not evaluate to an IO action"));
                };
                run_io(g, *act)
            }
        },
        IoAction::Try { action } => match run_io(g, *action)? {
            IoOutcome::Value(v) => {
                let ctor = eval_var(g, &std::collections::HashMap::new(), "Right")?;
                Ok(IoOutcome::Value(apply_one(g, ctor, v)?))
            }
            IoOutcome::Thrown(e) => {
                let ctor = eval_var(g, &std::collections::HashMap::new(), "Left")?;
                Ok(IoOutcome::Value(apply_one(g, ctor, Value::String(e))?))
            }
        },

        IoAction::Bind {
            action,
            param,
            body,
            mut env,
        } => {
            let v = match run_io(g, *action)? {
                IoOutcome::Value(v) => v,
                IoOutcome::Thrown(e) => return Ok(IoOutcome::Thrown(e)),
            };
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
            match run_io(g, *first)? {
                IoOutcome::Value(_) => {}
                IoOutcome::Thrown(e) => return Ok(IoOutcome::Thrown(e)),
            }
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
        Value::BuiltinConcatMap => Ok(Value::BuiltinConcatMap1(Box::new(arg))),
        Value::BuiltinConcatMap1(f) => concat_map(g, *f, arg),
        Value::BuiltinAdd => Ok(Value::BuiltinAdd1(Box::new(arg))),
        Value::BuiltinAdd1(a) => add_int(g, *a, arg),
        Value::BuiltinSub => Ok(Value::BuiltinSub1(Box::new(arg))),
        Value::BuiltinSub1(a) => sub_int(g, *a, arg),
        Value::BuiltinMul => Ok(Value::BuiltinMul1(Box::new(arg))),
        Value::BuiltinMul1(a) => mul_int(g, *a, arg),
        Value::BuiltinDiv => Ok(Value::BuiltinDiv1(Box::new(arg))),
        Value::BuiltinDiv1(a) => div_int(g, *a, arg),
        Value::BuiltinEq => Ok(Value::BuiltinEq1(Box::new(arg))),
        Value::BuiltinEq1(a) => eq_value(g, *a, arg),
        Value::BuiltinEqInt => Ok(Value::BuiltinEqInt1(Box::new(arg))),
        Value::BuiltinEqInt1(a) => eq_int(g, *a, arg),
        Value::BuiltinLtInt => Ok(Value::BuiltinLtInt1(Box::new(arg))),
        Value::BuiltinLtInt1(a) => lt_int(g, *a, arg),
        Value::BuiltinLeInt => Ok(Value::BuiltinLeInt1(Box::new(arg))),
        Value::BuiltinLeInt1(a) => le_int(g, *a, arg),
        Value::BuiltinGtInt => Ok(Value::BuiltinGtInt1(Box::new(arg))),
        Value::BuiltinGtInt1(a) => gt_int(g, *a, arg),
        Value::BuiltinGeInt => Ok(Value::BuiltinGeInt1(Box::new(arg))),
        Value::BuiltinGeInt1(a) => ge_int(g, *a, arg),
        Value::BuiltinNe => Ok(Value::BuiltinNe1(Box::new(arg))),
        Value::BuiltinNe1(a) => ne_value(g, *a, arg),
        Value::BuiltinNeInt => Ok(Value::BuiltinNeInt1(Box::new(arg))),
        Value::BuiltinNeInt1(a) => ne_int(g, *a, arg),
        Value::BuiltinAnd => Ok(Value::BuiltinAnd1(Box::new(arg))),
        Value::BuiltinAnd1(a) => and_bool(g, *a, arg),
        Value::BuiltinOr => Ok(Value::BuiltinOr1(Box::new(arg))),
        Value::BuiltinOr1(a) => or_bool(g, *a, arg),
        Value::BuiltinNot => not_bool(g, arg),
        Value::BuiltinIntToString => int_to_string(g, arg),
        Value::BuiltinBoolToString => bool_to_string(g, arg),
        Value::BuiltinStrAppend => Ok(Value::BuiltinStrAppend1(Box::new(arg))),
        Value::BuiltinStrAppend1(a) => str_append(g, *a, arg),
        Value::BuiltinShowDictApply => Ok(Value::BuiltinShowDictApply1(Box::new(arg))),
        Value::BuiltinShowDictApply1(d) => show_with_dict(g, *d, arg),
        Value::BuiltinEqDictApply => Ok(Value::BuiltinEqDictApply1(Box::new(arg))),
        Value::BuiltinEqDictApply1(d) => Ok(Value::BuiltinEqDictApply2(d, Box::new(arg))),
        Value::BuiltinEqDictApply2(d, a) => eq_with_dict(g, *d, *a, arg),
        Value::BuiltinRecordGet => Ok(Value::BuiltinRecordGet1(Box::new(arg))),
        Value::BuiltinRecordGet1(d) => record_get(g, *d, arg),
        Value::BuiltinShow => show_to_string(g, arg),
        Value::BuiltinThrow => {
            let arg = force_value(g, arg)?;
            let Value::String(s) = arg else {
                return Err(Error::msg("throw expects String"));
            };
            Ok(Value::IoAction(Box::new(IoAction::Throw(s))))
        }
        Value::BuiltinCatch => Ok(Value::BuiltinCatch1(Box::new(arg))),
        Value::BuiltinCatch1(act) => {
            let act = force_value(g, *act)?;
            let Value::IoAction(act) = act else {
                return Err(Error::msg("catch expects IO action"));
            };
            let handler = force_value(g, arg)?;
            Ok(Value::IoAction(Box::new(IoAction::Catch {
                action: act,
                handler,
            })))
        }
        Value::BuiltinTry => {
            let act = force_value(g, arg)?;
            let Value::IoAction(act) = act else {
                return Err(Error::msg("try expects IO action"));
            };
            Ok(Value::IoAction(Box::new(IoAction::Try { action: act })))
        }
        Value::BuiltinFfiAddI32 => Ok(Value::BuiltinFfiAddI32_1(Box::new(arg))),
        Value::BuiltinFfiAddI32_1(a) => ffi_add_i32(g, *a, arg),
        Value::BuiltinFfiAddF32 => Ok(Value::BuiltinFfiAddF32_1(Box::new(arg))),
        Value::BuiltinFfiAddF32_1(a) => ffi_add_f32(g, *a, arg),
        #[cfg(feature = "unsafe_ffi")]
        Value::BuiltinFfiPuts => {
            let arg = force_value(g, arg)?;
            let Value::String(s) = arg else {
                return Err(Error::msg("ffiPuts expects String"));
            };
            Ok(Value::IoAction(Box::new(IoAction::FfiPuts(s))))
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

fn record_get(g: &Globals, dict: Value, label: Value) -> Result<Value> {
    let dict = force_value(g, dict)?;
    let Value::Record(fields) = dict else {
        return Err(Error::msg("__recordGet expects a record"));
    };

    let label = force_value(g, label)?;
    let Value::String(label) = label else {
        return Err(Error::msg("__recordGet expects String label"));
    };

    let Some((_, v)) = fields.into_iter().find(|(k, _)| k == &label) else {
        return Err(Error::msg(format!("record missing field: {label}")));
    };
    force_value(g, v)
}

fn list_to_vec(g: &Globals, mut v: Value) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    loop {
        v = force_value(g, v)?;
        match v {
            Value::ListNil => return Ok(out),
            Value::ListCons(h, t) => {
                out.push(*h);
                v = *t;
            }
            other => return Err(Error::msg(format!("expected List, got {other:?}"))),
        }
    }
}

fn vec_to_list(mut elems: Vec<Value>) -> Value {
    let mut out = Value::ListNil;
    while let Some(v) = elems.pop() {
        out = Value::ListCons(Box::new(v), Box::new(out));
    }
    out
}

fn concat_map(g: &Globals, f: Value, xs: Value) -> Result<Value> {
    let f = force_value(g, f)?;
    let xs = list_to_vec(g, xs)?;
    let mut out = Vec::new();

    for x in xs {
        let ys = apply_one(g, f.clone(), x)?;
        out.extend(list_to_vec(g, ys)?);
    }

    Ok(vec_to_list(out))
}

fn int_from_i64(n: i64) -> Integer {
    #[cfg(feature = "unsafe_bigint")]
    {
        kscr_unsafe_bigint::int_from_i64(n)
    }

    #[cfg(not(feature = "unsafe_bigint"))]
    {
        crate::safe_bigint::int_from_i64(n)
    }
}

fn parse_integer(s: &str) -> Result<Integer> {
    #[cfg(feature = "unsafe_bigint")]
    {
        crate::debug::unsafe_used("bigint");
        kscr_unsafe_bigint::parse_integer(s)
            .map_err(|_| Error::msg(format!("invalid integer: {s}")))
    }

    #[cfg(not(feature = "unsafe_bigint"))]
    {
        crate::safe_bigint::parse_integer(s)
            .map_err(|_| Error::msg(format!("invalid integer: {s}")))
    }
}

fn int_is_zero(n: &Integer) -> bool {
    #[cfg(feature = "unsafe_bigint")]
    {
        kscr_unsafe_bigint::is_zero(n)
    }

    #[cfg(not(feature = "unsafe_bigint"))]
    {
        crate::safe_bigint::is_zero(n)
    }
}

fn parse_f64(s: &str) -> Result<f64> {
    s.parse::<f64>()
        .map_err(|_| Error::msg(format!("invalid float64: {s}")))
}

fn checked_cast(g: &Globals, v: Value, target: CastTarget) -> Result<Value> {
    let v = force_value(g, v)?;
    match target {
        CastTarget::I32 => {
            let Value::Integer(n) = v else {
                return Err(Error::msg("checked cast to i32 expects Integer"));
            };

            #[cfg(feature = "unsafe_bigint")]
            {
                if !kscr_unsafe_bigint::in_i32_range(&n) {
                    return Err(Error::msg("integer out of range for i32"));
                }
            }

            #[cfg(not(feature = "unsafe_bigint"))]
            {
                if !crate::safe_bigint::in_i32_range(&n) {
                    return Err(Error::msg("integer out of range for i32"));
                }
            }

            Ok(Value::Integer(n))
        }
        CastTarget::I64 => {
            let Value::Integer(n) = v else {
                return Err(Error::msg("checked cast to i64 expects Integer"));
            };

            #[cfg(feature = "unsafe_bigint")]
            {
                if !kscr_unsafe_bigint::in_i64_range(&n) {
                    return Err(Error::msg("integer out of range for i64"));
                }
            }

            #[cfg(not(feature = "unsafe_bigint"))]
            {
                if !crate::safe_bigint::in_i64_range(&n) {
                    return Err(Error::msg("integer out of range for i64"));
                }
            }

            Ok(Value::Integer(n))
        }
        CastTarget::F32 => {
            let Value::Float64(x) = v else {
                return Err(Error::msg("checked cast to f32 expects Float64"));
            };
            let y = x as f32;
            if y.is_infinite() && x.is_finite() {
                return Err(Error::msg("float overflow for f32"));
            }
            Ok(Value::Float64(y as f64))
        }
        CastTarget::F64 => {
            let Value::Float64(x) = v else {
                return Err(Error::msg("checked cast to f64 expects Float64"));
            };
            Ok(Value::Float64(x))
        }
    }
}

fn to_i32_checked(n: Integer, ctx: &str) -> Result<i32> {
    #[cfg(feature = "unsafe_bigint")]
    {
        if !kscr_unsafe_bigint::in_i32_range(&n) {
            return Err(Error::msg(format!("{ctx}: integer out of range for i32")));
        }
        Ok(kscr_unsafe_bigint::to_i32_range_checked(n))
    }

    #[cfg(not(feature = "unsafe_bigint"))]
    {
        if !crate::safe_bigint::in_i32_range(&n) {
            return Err(Error::msg(format!("{ctx}: integer out of range for i32")));
        }
        Ok(crate::safe_bigint::to_i32_range_checked(n))
    }
}

fn to_f32_checked(x: f64, ctx: &str) -> Result<f32> {
    let y = x as f32;
    if y.is_infinite() && x.is_finite() {
        return Err(Error::msg(format!("{ctx}: float overflow for f32")));
    }
    Ok(y)
}

fn ffi_add_i32(g: &Globals, a: Value, b: Value) -> Result<Value> {
    #[cfg(feature = "unsafe_ffi")]
    crate::debug::unsafe_used("ffiAddI32");

    let a = force_value(g, a)?;
    let b = force_value(g, b)?;
    let Value::Integer(a) = a else {
        return Err(Error::msg("ffiAddI32 expects Integer"));
    };
    let Value::Integer(b) = b else {
        return Err(Error::msg("ffiAddI32 expects Integer"));
    };
    let a = to_i32_checked(a, "ffiAddI32")?;
    let b = to_i32_checked(b, "ffiAddI32")?;
    let out = a
        .checked_add(b)
        .ok_or_else(|| Error::msg("ffiAddI32: i32 overflow"))?;
    Ok(Value::Integer(int_from_i64(out as i64)))
}

fn ffi_add_f32(g: &Globals, a: Value, b: Value) -> Result<Value> {
    #[cfg(feature = "unsafe_ffi")]
    crate::debug::unsafe_used("ffiAddF32");

    let a = force_value(g, a)?;
    let b = force_value(g, b)?;
    let Value::Float64(a) = a else {
        return Err(Error::msg("ffiAddF32 expects Float64"));
    };
    let Value::Float64(b) = b else {
        return Err(Error::msg("ffiAddF32 expects Float64"));
    };
    let a = to_f32_checked(a, "ffiAddF32")?;
    let b = to_f32_checked(b, "ffiAddF32")?;
    let out = a + b;
    if out.is_infinite() {
        return Err(Error::msg("ffiAddF32: float overflow for f32"));
    }
    Ok(Value::Float64(out as f64))
}

fn add_int(g: &Globals, a: Value, b: Value) -> Result<Value> {
    let a = force_value(g, a)?;
    let b = force_value(g, b)?;
    let Value::Integer(a) = a else {
        return Err(Error::msg("+ expects Integer"));
    };
    let Value::Integer(b) = b else {
        return Err(Error::msg("+ expects Integer"));
    };

    let out = a + b;

    Ok(Value::Integer(out))
}

fn sub_int(g: &Globals, a: Value, b: Value) -> Result<Value> {
    let a = force_value(g, a)?;
    let b = force_value(g, b)?;
    let Value::Integer(a) = a else {
        return Err(Error::msg("- expects Integer"));
    };
    let Value::Integer(b) = b else {
        return Err(Error::msg("- expects Integer"));
    };

    let out = a - b;

    Ok(Value::Integer(out))
}

fn mul_int(g: &Globals, a: Value, b: Value) -> Result<Value> {
    let a = force_value(g, a)?;
    let b = force_value(g, b)?;
    let Value::Integer(a) = a else {
        return Err(Error::msg("* expects Integer"));
    };
    let Value::Integer(b) = b else {
        return Err(Error::msg("* expects Integer"));
    };

    let out = a * b;

    Ok(Value::Integer(out))
}

fn div_int(g: &Globals, a: Value, b: Value) -> Result<Value> {
    let a = force_value(g, a)?;
    let b = force_value(g, b)?;
    let Value::Integer(a) = a else {
        return Err(Error::msg("/ expects Integer"));
    };
    let Value::Integer(b) = b else {
        return Err(Error::msg("/ expects Integer"));
    };

    if int_is_zero(&b) {
        return Err(Error::msg("division by zero"));
    }

    let out = a / b;

    Ok(Value::Integer(out))
}

fn eq_int(g: &Globals, a: Value, b: Value) -> Result<Value> {
    let a = force_value(g, a)?;
    let b = force_value(g, b)?;
    let Value::Integer(a) = a else {
        return Err(Error::msg("== expects Integer"));
    };
    let Value::Integer(b) = b else {
        return Err(Error::msg("== expects Integer"));
    };
    Ok(Value::Bool(a == b))
}

fn lt_int(g: &Globals, a: Value, b: Value) -> Result<Value> {
    let a = force_value(g, a)?;
    let b = force_value(g, b)?;
    let Value::Integer(a) = a else {
        return Err(Error::msg("< expects Integer"));
    };
    let Value::Integer(b) = b else {
        return Err(Error::msg("< expects Integer"));
    };
    Ok(Value::Bool(a < b))
}

fn le_int(g: &Globals, a: Value, b: Value) -> Result<Value> {
    let a = force_value(g, a)?;
    let b = force_value(g, b)?;
    let Value::Integer(a) = a else {
        return Err(Error::msg("<= expects Integer"));
    };
    let Value::Integer(b) = b else {
        return Err(Error::msg("<= expects Integer"));
    };
    Ok(Value::Bool(a <= b))
}

fn gt_int(g: &Globals, a: Value, b: Value) -> Result<Value> {
    let a = force_value(g, a)?;
    let b = force_value(g, b)?;
    let Value::Integer(a) = a else {
        return Err(Error::msg("> expects Integer"));
    };
    let Value::Integer(b) = b else {
        return Err(Error::msg("> expects Integer"));
    };
    Ok(Value::Bool(a > b))
}

fn ge_int(g: &Globals, a: Value, b: Value) -> Result<Value> {
    let a = force_value(g, a)?;
    let b = force_value(g, b)?;
    let Value::Integer(a) = a else {
        return Err(Error::msg(">= expects Integer"));
    };
    let Value::Integer(b) = b else {
        return Err(Error::msg(">= expects Integer"));
    };
    Ok(Value::Bool(a >= b))
}

fn ne_int(g: &Globals, a: Value, b: Value) -> Result<Value> {
    let a = force_value(g, a)?;
    let b = force_value(g, b)?;
    let Value::Integer(a) = a else {
        return Err(Error::msg("/= expects Integer"));
    };
    let Value::Integer(b) = b else {
        return Err(Error::msg("/= expects Integer"));
    };
    Ok(Value::Bool(a != b))
}

fn and_bool(g: &Globals, a: Value, b: Value) -> Result<Value> {
    let a = force_value(g, a)?;
    let Value::Bool(a) = a else {
        return Err(Error::msg("&& expects Bool"));
    };
    if !a {
        return Ok(Value::Bool(false));
    }
    let b = force_value(g, b)?;
    let Value::Bool(b) = b else {
        return Err(Error::msg("&& expects Bool"));
    };
    Ok(Value::Bool(b))
}

fn or_bool(g: &Globals, a: Value, b: Value) -> Result<Value> {
    let a = force_value(g, a)?;
    let Value::Bool(a) = a else {
        return Err(Error::msg("|| expects Bool"));
    };
    if a {
        return Ok(Value::Bool(true));
    }
    let b = force_value(g, b)?;
    let Value::Bool(b) = b else {
        return Err(Error::msg("|| expects Bool"));
    };
    Ok(Value::Bool(b))
}

fn not_bool(g: &Globals, a: Value) -> Result<Value> {
    let a = force_value(g, a)?;
    let Value::Bool(a) = a else {
        return Err(Error::msg("not expects Bool"));
    };
    Ok(Value::Bool(!a))
}

fn int_to_string(g: &Globals, a: Value) -> Result<Value> {
    let a = force_value(g, a)?;
    let Value::Integer(a) = a else {
        return Err(Error::msg("intToString expects Integer"));
    };
    Ok(Value::String(a.to_string()))
}

fn bool_to_string(g: &Globals, a: Value) -> Result<Value> {
    let a = force_value(g, a)?;
    let Value::Bool(a) = a else {
        return Err(Error::msg("boolToString expects Bool"));
    };
    Ok(Value::String(if a { "True" } else { "False" }.to_string()))
}

fn str_append(g: &Globals, a: Value, b: Value) -> Result<Value> {
    let a = force_value(g, a)?;
    let b = force_value(g, b)?;
    let Value::String(a) = a else {
        return Err(Error::msg("++ expects String"));
    };
    let Value::String(b) = b else {
        return Err(Error::msg("++ expects String"));
    };
    Ok(Value::String(format!("{a}{b}")))
}

fn quote_string(s: &str) -> String {
    let mut out = String::new();
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn quote_char(c: char) -> String {
    let mut out = String::new();
    out.push('\'');
    match c {
        '\\' => out.push_str("\\\\"),
        '\'' => out.push_str("\\\'"),
        '\n' => out.push_str("\\n"),
        '\t' => out.push_str("\\t"),
        '\r' => out.push_str("\\r"),
        other => out.push(other),
    }
    out.push('\'');
    out
}

fn show_value_str(g: &Globals, v: Value) -> Result<String> {
    let v = force_value(g, v)?;
    Ok(match v {
        Value::Integer(n) => n.to_string(),
        Value::Float64(x) => x.to_string(),
        Value::Bool(b) => if b { "True" } else { "False" }.to_string(),
        Value::String(s) => quote_string(&s),
        Value::Char(c) => quote_char(c),
        Value::Unit => "()".to_string(),
        Value::Tuple(vs) => {
            let mut parts = Vec::new();
            for v in vs {
                parts.push(show_value_str(g, v)?);
            }
            format!("({})", parts.join(", "))
        }
        Value::ListNil | Value::ListCons(_, _) => {
            let elems = list_to_vec(g, v)?;
            let mut parts = Vec::new();
            for e in elems {
                parts.push(show_value_str(g, e)?);
            }
            format!("[{}]", parts.join(", "))
        }
        Value::Record(mut fields) => {
            // Pretty-print constructor encoding: { __ctor: "C", __args: [...] }
            let ctor = fields
                .iter()
                .find_map(|(k, v)| if k == "__ctor" { Some(v.clone()) } else { None });
            let args = fields
                .iter()
                .find_map(|(k, v)| if k == "__args" { Some(v.clone()) } else { None });

            if let (Some(ctor), Some(args)) = (ctor, args) {
                let ctor = force_value(g, ctor)?;
                if let Value::String(ctor) = ctor {
                    let elems = list_to_vec(g, args)?;
                    if elems.is_empty() {
                        return Ok(ctor);
                    }
                    let mut parts = Vec::new();
                    for e in elems {
                        parts.push(show_value_str(g, e)?);
                    }
                    return Ok(format!("{ctor} {}", parts.join(" ")));
                }
            }

            fields.sort_by(|(a, _), (b, _)| a.cmp(b));
            let mut parts = Vec::new();
            for (k, v) in fields {
                parts.push(format!("{k}: {}", show_value_str(g, v)?));
            }
            format!("{{{}}}", parts.join(", "))
        }
        _ => return Err(Error::msg("show/toString expects a printable value")),
    })
}

fn show_to_string(g: &Globals, a: Value) -> Result<Value> {
    Ok(Value::String(show_value_str(g, a)?))
}

fn show_with_dict(g: &Globals, dict: Value, a: Value) -> Result<Value> {
    let dict = force_value(g, dict)?;
    let Value::Record(fields) = dict else {
        return Err(Error::msg(
            "__show/__toString expects a Show dictionary record",
        ));
    };

    let Some((_, show_fn)) = fields.into_iter().find(|(k, _)| k == "show") else {
        return Err(Error::msg("Show dictionary missing field: show"));
    };
    let show_fn = force_value(g, show_fn)?;
    apply_one(g, show_fn, a)
}

fn eq_values(g: &Globals, a: Value, b: Value) -> Result<bool> {
    let a = force_value(g, a)?;
    let b = force_value(g, b)?;

    Ok(match (a, b) {
        (Value::Unit, Value::Unit) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Char(a), Value::Char(b)) => a == b,
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Float64(a), Value::Float64(b)) => a == b,
        (Value::Integer(a), Value::Integer(b)) => a == b,

        (Value::Tuple(as_), Value::Tuple(bs)) => {
            if as_.len() != bs.len() {
                return Ok(false);
            }
            for (x, y) in as_.into_iter().zip(bs.into_iter()) {
                if !eq_values(g, x, y)? {
                    return Ok(false);
                }
            }
            true
        }

        (Value::ListNil, Value::ListNil) => true,
        (Value::ListNil, Value::ListCons(_, _)) | (Value::ListCons(_, _), Value::ListNil) => false,
        (Value::ListCons(a_hd, a_tl), Value::ListCons(b_hd, b_tl)) => {
            eq_values(g, *a_hd, *b_hd)? && eq_values(g, *a_tl, *b_tl)?
        }

        (Value::Record(mut a_fields), Value::Record(mut b_fields)) => {
            let a_ctor =
                a_fields
                    .iter()
                    .find_map(|(k, v)| if k == "__ctor" { Some(v.clone()) } else { None });
            let a_args =
                a_fields
                    .iter()
                    .find_map(|(k, v)| if k == "__args" { Some(v.clone()) } else { None });
            let b_ctor =
                b_fields
                    .iter()
                    .find_map(|(k, v)| if k == "__ctor" { Some(v.clone()) } else { None });
            let b_args =
                b_fields
                    .iter()
                    .find_map(|(k, v)| if k == "__args" { Some(v.clone()) } else { None });

            if let (Some(a_ctor), Some(a_args), Some(b_ctor), Some(b_args)) =
                (a_ctor, a_args, b_ctor, b_args)
            {
                let a_ctor = force_value(g, a_ctor)?;
                let b_ctor = force_value(g, b_ctor)?;
                let (Value::String(a_ctor), Value::String(b_ctor)) = (a_ctor, b_ctor) else {
                    return Ok(false);
                };
                if a_ctor != b_ctor {
                    return Ok(false);
                }
                let a_elems = list_to_vec(g, a_args)?;
                let b_elems = list_to_vec(g, b_args)?;
                if a_elems.len() != b_elems.len() {
                    return Ok(false);
                }
                for (x, y) in a_elems.into_iter().zip(b_elems.into_iter()) {
                    if !eq_values(g, x, y)? {
                        return Ok(false);
                    }
                }
                return Ok(true);
            }

            a_fields.sort_by(|(a, _), (b, _)| a.cmp(b));
            b_fields.sort_by(|(a, _), (b, _)| a.cmp(b));
            if a_fields.len() != b_fields.len() {
                return Ok(false);
            }
            for ((ak, av), (bk, bv)) in a_fields.into_iter().zip(b_fields.into_iter()) {
                if ak != bk {
                    return Ok(false);
                }
                if !eq_values(g, av, bv)? {
                    return Ok(false);
                }
            }
            true
        }

        _ => return Err(Error::msg("== expects equatable values")),
    })
}

fn eq_value(g: &Globals, a: Value, b: Value) -> Result<Value> {
    Ok(Value::Bool(eq_values(g, a, b)?))
}

fn ne_value(g: &Globals, a: Value, b: Value) -> Result<Value> {
    Ok(Value::Bool(!eq_values(g, a, b)?))
}

fn eq_with_dict(g: &Globals, dict: Value, a: Value, b: Value) -> Result<Value> {
    let dict = force_value(g, dict)?;
    let Value::Record(fields) = dict else {
        return Err(Error::msg("__eq expects an Eq dictionary record"));
    };

    let Some((_, eq_fn)) = fields.into_iter().find(|(k, _)| k == "eq") else {
        return Err(Error::msg("Eq dictionary missing field: eq"));
    };
    let eq_fn = force_value(g, eq_fn)?;
    let f = apply_one(g, eq_fn, a)?;
    apply_one(g, f, b)
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
                (IrLiteral::Integer(a), Value::Integer(b)) => {
                    let aa = parse_integer(a)?;
                    #[cfg(feature = "unsafe_bigint")]
                    {
                        aa == b.clone()
                    }
                    #[cfg(not(feature = "unsafe_bigint"))]
                    {
                        aa == *b
                    }
                }
                (IrLiteral::Float64(a), Value::Float64(b)) => parse_f64(a)? == *b,
                (IrLiteral::Bool(a), Value::Bool(b)) => a == b,
                (IrLiteral::String(a), Value::String(b)) => a == b,
                (IrLiteral::Char(a), Value::Char(b)) => a == b,
                _ => false,
            };
            if ok {
                Some(std::collections::HashMap::new())
            } else {
                None
            }
        }
        (P::Tuple(ps), Value::Tuple(vs)) if ps.len() == vs.len() => {
            let mut out = std::collections::HashMap::new();
            for (p, v) in ps.iter().zip(vs.iter()) {
                let Some(b) = match_pat(g, env, p, v)? else {
                    return Ok(None);
                };
                out.extend(b);
            }
            Some(out)
        }
        (P::List(ps), v) => {
            let mut out = std::collections::HashMap::new();
            let mut cur = v.clone();
            for p in ps.iter() {
                let cur_forced = force_value(g, cur)?;
                let Value::ListCons(h, t) = cur_forced else {
                    return Ok(None);
                };
                let Some(b) = match_pat(g, env, p, &h)? else {
                    return Ok(None);
                };
                out.extend(b);
                cur = *t;
            }
            let cur = force_value(g, cur)?;
            if matches!(cur, Value::ListNil) {
                Some(out)
            } else {
                None
            }
        }
        (P::Cons(hd, tl), v) => {
            let v = v.clone();
            let v = force_value(g, v)?;
            let Value::ListCons(h, t) = v else {
                return Ok(None);
            };
            let mut out = std::collections::HashMap::new();
            let Some(b_hd) = match_pat(g, env, hd, &h)? else {
                return Ok(None);
            };
            out.extend(b_hd);
            let Some(b_tl) = match_pat(g, env, tl, &t)? else {
                return Ok(None);
            };
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
                let Some(b) = match_pat(g, env, p, v)? else {
                    return Ok(None);
                };
                out.extend(b);
            }
            Some(out)
        }
        (P::RecordLoose(fs, rest), Value::Record(vs)) => {
            let mut out = std::collections::HashMap::new();

            let mut required = std::collections::HashSet::new();
            for (name, p) in fs {
                required.insert(name.clone());
                let Some((_, v)) = vs.iter().find(|(n, _)| n == name) else {
                    return Ok(None);
                };
                let Some(b) = match_pat(g, env, p, v)? else {
                    return Ok(None);
                };
                out.extend(b);
            }

            if let Some(rest) = rest {
                let rest_fields: Vec<(String, Value)> = vs
                    .iter()
                    .filter(|(k, _)| !required.contains(k))
                    .cloned()
                    .collect();
                out.insert(rest.clone(), Value::Record(rest_fields));
            }

            Some(out)
        }
        (P::As(n, p), v) => {
            let Some(mut b) = match_pat(g, env, p, v)? else {
                return Ok(None);
            };
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
        (P::Constructor { name, args }, Value::Record(vs)) => {
            let Some((_, ctor_v)) = vs.iter().find(|(n, _)| n == "__ctor") else {
                return Ok(None);
            };
            let ctor_v = force_value(g, ctor_v.clone())?;
            let Value::String(ctor_name) = ctor_v else {
                return Ok(None);
            };
            if &ctor_name != name {
                return Ok(None);
            }

            let Some((_, args_v)) = vs.iter().find(|(n, _)| n == "__args") else {
                return Ok(None);
            };
            let args_v = force_value(g, args_v.clone())?;
            let vs = list_to_vec(g, args_v)?;
            if args.len() != vs.len() {
                return Ok(None);
            }

            let mut out = std::collections::HashMap::new();
            for (p, v) in args.iter().zip(vs.iter()) {
                let Some(b) = match_pat(g, env, p, v)? else {
                    return Ok(None);
                };
                out.extend(b);
            }
            Some(out)
        }
        (P::View(p, e), v) => {
            let fv = eval_expr(g, env, e)?;
            let v2 = apply_one(g, fv, v.clone())?;
            match_pat(g, env, p, &v2)?
        }
        _ => None,
    })
}

#[cfg(test)]
mod show_roundtrip_tests {
    use super::*;

    fn eval_show_str(s: &str) -> Result<String> {
        let src = format!("x = {s}\nmain = IO ()\n");
        let m = crate::parser::parse_module(&src)?;
        let tm = crate::types::typecheck(m)?;
        let ir = crate::ir::lower_to_ir(&tm.module)?;
        let g = Globals::from_module(&ir);
        let v = eval_var(&g, &std::collections::HashMap::new(), "x")?;
        show_value_str(&g, v)
    }

    fn list_of(elems: Vec<Value>) -> Value {
        let mut out = Value::ListNil;
        for e in elems.into_iter().rev() {
            out = Value::ListCons(Box::new(e), Box::new(out));
        }
        out
    }

    fn int(n: i64) -> Value {
        Value::Integer(int_from_i64(n))
    }

    #[test]
    fn show_value_str_roundtrips_through_parser_for_literals() {
        let g0 = Globals::from_module(&IrModule { items: vec![] });

        let cases = vec![
            int(123),
            Value::Bool(true),
            Value::Unit,
            Value::Char('\n'),
            Value::Char('\\'),
            Value::String("hello".to_string()),
            Value::String("a\n\"b\\c".to_string()),
            Value::Tuple(vec![int(1), Value::String("x".to_string())]),
            list_of(vec![int(1), int(2)]),
            Value::Record(vec![
                ("a".to_string(), int(1)),
                ("b".to_string(), Value::String("x".to_string())),
            ]),
        ];

        for v in cases {
            let s1 = show_value_str(&g0, v).unwrap();
            let s2 =
                eval_show_str(&s1).unwrap_or_else(|e| panic!("failed to roundtrip: {s1}: {e}"));
            assert_eq!(s1, s2);
        }
    }
}
