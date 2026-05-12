//! IR scaffolding.

use crate::{ast, error::Error, Result};
pub use kscr_ir::ir::*;
pub use kscr_ir::optimize::{
    run_passes, CaseSimplification, ConstantFolding, DeadCodeElimination, OptimizationPass,
};

#[cfg(feature = "unsafe_bigint")]
type Integer = kscr_unsafe_bigint::Integer;
#[cfg(not(feature = "unsafe_bigint"))]
type Integer = crate::safe_bigint::Integer;

// NOTE: IR data types are defined in `crates/kscr_ir` and re-exported here.

/// Apply default optimization passes to an IR module.
///
/// This applies a standard set of safe optimizations:
/// 1. Constant folding
/// 2. Case simplification
/// 3. Dead code elimination
pub fn optimize_ir(module: &IrModule) -> IrModule {
    let passes: Vec<Box<dyn OptimizationPass>> = vec![
        Box::new(ConstantFolding),
        Box::new(CaseSimplification),
        Box::new(DeadCodeElimination),
    ];
    run_passes(module, &passes)
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
            ast::ExprKind::Var(v) if v.contains('.') => {
                out.insert(name.clone(), v.clone());
            }
            ast::ExprKind::Ctor(v) => {
                let v = v.qualified_text();
                if v.contains('.') {
                    out.insert(name.clone(), v.clone());
                }
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
            // Use the local (unqualified) constructor name for runtime pattern matching.
            // Qualification is only needed for compile-time resolution, not runtime matching.
            let name = name.local_name().to_string();
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
    use ast::ExprKind;
    Ok(match &expr.kind {
        // literals
        ExprKind::Unit => IrExpr::Unit,
        ExprKind::Integer(s) => IrExpr::Integer(s.clone()),
        ExprKind::Float64(s) => IrExpr::Float64(s.clone()),
        ExprKind::Bool(b) => IrExpr::Bool(*b),
        ExprKind::String(s) => IrExpr::String(s.clone()),
        ExprKind::Char(c) => IrExpr::Char(*c),
        ExprKind::Var(v) => IrExpr::Var(v.clone()),
        ExprKind::Ctor(v) => {
            let name = v.qualified_text();
            if name.contains('.') {
                IrExpr::Var(name)
            } else {
                IrExpr::Var(ctor_aliases.get(v.local_name()).cloned().unwrap_or(name))
            }
        }
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
        ExprKind::Let { bindings, body } => IrExpr::Let {
            bindings: lower_let_like_bindings(bindings, fresh, ctor_aliases, "_ir_let")?,
            body: Box::new(lower_expr(body, fresh, ctor_aliases)?),
        },
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
        ExprKind::Where { expr, bindings } => IrExpr::Let {
            bindings: lower_let_like_bindings(bindings, fresh, ctor_aliases, "_ir_where")?,
            body: Box::new(lower_expr(expr, fresh, ctor_aliases)?),
        },
    })
}

fn lower_let_like_bindings(
    bindings: &[ast::Binding],
    fresh: &mut usize,
    ctor_aliases: &std::collections::HashMap<String, String>,
    tmp_prefix: &str,
) -> Result<Vec<(String, IrExpr)>> {
    use ast::PatternKind;

    // Order-independent recursive let-bindings.
    // - Variable binders become direct bindings.
    // - Pattern binders are lowered as:
    //     tmp = rhs
    //     v = case tmp of pat -> v
    //   for each bound variable v.
    let mut out: Vec<(String, IrExpr)> = Vec::new();

    for b in bindings {
        match &b.pat.kind {
            PatternKind::Var(name) => {
                out.push((name.clone(), lower_expr(&b.expr, fresh, ctor_aliases)?));
            }
            pat => {
                let mut vars = std::collections::BTreeSet::new();
                collect_pat_vars(&b.pat, &mut vars);
                if vars.is_empty() {
                    continue;
                }

                let tmp = format!("{tmp_prefix}{fresh}");
                *fresh += 1;
                out.push((tmp.clone(), lower_expr(&b.expr, fresh, ctor_aliases)?));

                let ir_pat = lower_pat(
                    &ast::Pattern {
                        span: b.pat.span,
                        kind: pat.clone(),
                    },
                    fresh,
                    ctor_aliases,
                )?;

                for v in vars {
                    out.push((
                        v.clone(),
                        IrExpr::Case {
                            expr: Box::new(IrExpr::Var(tmp.clone())),
                            arms: vec![IrCaseArm {
                                pat: ir_pat.clone(),
                                guard: None,
                                body: IrExpr::Var(v),
                            }],
                        },
                    ));
                }
            }
        }
    }

    Ok(out)
}

#[derive(Debug, Clone)]
pub enum IoAction {
    Pure(Value),
    StdoutWrite(String),
    StdinReadLine,
    GetArgs,
    ReadFile(String),
    WriteFile {
        path: String,
        content: String,
    },
    ExitWith(i64),

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

    // IO sequencing primitives expressed as values (used by the Prelude Monad IO instance).
    BindValue {
        action: Box<IoAction>,
        func: Value,
    },
    ThenValue {
        first: Box<IoAction>,
        then_action: Box<IoAction>,
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

type SharedEnv = std::rc::Rc<std::collections::HashMap<String, Value>>;

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
    BuiltinPutStrLn,
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
    BuiltinQuotInt,
    BuiltinQuotInt1(Box<Value>),
    BuiltinRemInt,
    BuiltinRemInt1(Box<Value>),
    BuiltinDivInt,
    BuiltinDivInt1(Box<Value>),
    BuiltinModInt,
    BuiltinModInt1(Box<Value>),
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
    BuiltinListAppend,
    BuiltinListAppend1(Box<Value>),
    BuiltinShow,
    BuiltinShowDictApply,
    BuiltinShowDictApply1(Box<Value>),
    BuiltinShowDictApply2(Box<Value>, Box<Value>),
    BuiltinEqDictApply,
    BuiltinEqDictApply1(Box<Value>),
    BuiltinEqDictApply2(Box<Value>, Box<Value>),
    BuiltinEqDictApply3(Box<Value>, Box<Value>, Box<Value>),
    BuiltinRecordGet,
    BuiltinRecordGet1(Box<Value>),
    BuiltinError,
    BuiltinThrow,
    BuiltinCatch,
    BuiltinCatch1(Box<Value>),
    BuiltinTry,
    BuiltinIoBind,
    BuiltinIoBind1(Box<Value>),
    BuiltinIoThen,
    BuiltinIoThen1(Box<Value>),
    BuiltinFfiAddI32,
    BuiltinFfiAddI32_1(Box<Value>),
    BuiltinFfiAddF32,
    BuiltinFfiAddF32_1(Box<Value>),
    #[cfg(feature = "unsafe_ffi")]
    BuiltinFfiPuts,
    BuiltinReadFile,
    BuiltinWriteFile,
    BuiltinWriteFile1(Box<Value>),
    BuiltinExitWith,
    Closure {
        params: Vec<String>,
        body: Box<IrExpr>,
        env: SharedEnv,
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
    if !g.defs.contains_key("main") {
        return Err(Error::msg("main does not exist"));
    }
    let v = eval_var(&g, &std::collections::HashMap::new(), "main")?;
    let v = force_value(&g, v)?;
    let v = auto_apply_io_dict(&g, v)?;

    let Value::IoAction(action) = v else {
        return Err(Error::msg(format!(
            "main did not evaluate to an IO action (got {})",
            value_type_name(&v)
        )));
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

fn value_type_name(v: &Value) -> &'static str {
    match v {
        Value::Unit => "Unit",
        Value::Integer(_) => "Integer",
        Value::Float64(_) => "Float64",
        Value::Bool(_) => "Bool",
        Value::String(_) => "String",
        Value::Char(_) => "Char",
        Value::Tuple(_) => "Tuple",
        Value::ListNil => "List",
        Value::ListCons(_, _) => "List",
        Value::Record(_) => "Record",
        Value::Thunk(_) => "Thunk",
        Value::IoAction(_) => "IoAction",
        Value::IoCtor => "IoCtor",
        Value::Closure { .. } => "Closure",
        _ => "Builtin",
    }
}

fn is_probably_typeclass_dict_record(v: &Value) -> bool {
    let Value::Record(fields) = v else {
        return false;
    };

    // Avoid treating constructor-encoded records as dictionaries.
    if fields.iter().any(|(k, _)| k == "__ctor" || k == "__args") {
        return false;
    }

    fields.iter().any(|(k, _)| {
        k == "=="
            || k == "/="
            || k == "show"
            || k == "+"
            || k == "-"
            || k == "*"
            || k == "/"
            || k.starts_with("__super_")
    })
}

/// Auto-apply a dictionary when a Closure expects one in an IO context.
/// This handles do-notation that desugars to lambdas expecting dictionaries.
///
/// The heuristic checks if the Closure has exactly one parameter starting with
/// "__dict_" and attempts to find a matching IO instance dictionary by appending
/// "_IO" to the parameter name.
///
/// While this relies on naming conventions, it matches the desugaring behavior
/// of the typechecker/compiler, which generates these parameter names.
///
/// Additionally, for certain classes (Num, Eq, Show), if no concrete dict is
/// available in globals, we inject a default dictionary at runtime.
fn auto_apply_io_dict(g: &Globals, v: Value) -> Result<Value> {
    if let Value::Closure {
        params,
        body: _,
        env: _,
    } = &v
    {
        if params.len() == 1 && params[0].starts_with("__dict_") {
            // Most dictionary-passing rewrites use the concrete dictionary name directly
            // (e.g. `__dict_Monad_IO`).
            let candidates = [params[0].clone(), format!("{}_IO", params[0])];

            // Try to find dict in globals first
            for name in &candidates {
                if let Ok(dict) = eval_var(g, &std::collections::HashMap::new(), name) {
                    let v = apply_one(g, v, dict)?;
                    return force_value(g, v);
                }
            }

            // If no dict found in globals, try to inject a default dict for specific classes
            // Extract class name from param (e.g., "__dict_Prelude.Show.Show" -> "Prelude.Show.Show")
            let class_name = params[0].strip_prefix("__dict_").unwrap_or("");
            if let Ok(default_dict) = try_get_default_dict(class_name) {
                let v = apply_one(g, v, default_dict)?;
                return force_value(g, v);
            }
        }
    }
    Ok(v)
}

/// Try to get a default dictionary for a class.
/// Only supports Num, Eq, and Show - other classes return None.
fn try_get_default_dict(class_name: &str) -> Result<Value> {
    let unqualified = class_name.rsplit('.').next().unwrap_or(class_name);

    match unqualified {
        "Show" => {
            // Default Show instance uses show_value_str
            Ok(Value::Record(vec![(
                "show".to_string(),
                Value::Closure {
                    params: vec!["_dict".to_string(), "x".to_string()],
                    body: Box::new(IrExpr::Apply {
                        func: Box::new(IrExpr::Var("show".to_string())),
                        args: vec![IrExpr::Var("x".to_string())],
                    }),
                    env: std::collections::HashMap::new().into(),
                },
            )]))
        }
        "Num" => {
            // Default Num instance uses Integer operations
            Ok(Value::Record(vec![
                (
                    "+".to_string(),
                    Value::Closure {
                        params: vec!["_dict".to_string(), "a".to_string(), "b".to_string()],
                        body: Box::new(IrExpr::Apply {
                            func: Box::new(IrExpr::Var("+".to_string())),
                            args: vec![IrExpr::Var("a".to_string()), IrExpr::Var("b".to_string())],
                        }),
                        env: std::collections::HashMap::new().into(),
                    },
                ),
                (
                    "-".to_string(),
                    Value::Closure {
                        params: vec!["_dict".to_string(), "a".to_string(), "b".to_string()],
                        body: Box::new(IrExpr::Apply {
                            func: Box::new(IrExpr::Var("-".to_string())),
                            args: vec![IrExpr::Var("a".to_string()), IrExpr::Var("b".to_string())],
                        }),
                        env: std::collections::HashMap::new().into(),
                    },
                ),
                (
                    "*".to_string(),
                    Value::Closure {
                        params: vec!["_dict".to_string(), "a".to_string(), "b".to_string()],
                        body: Box::new(IrExpr::Apply {
                            func: Box::new(IrExpr::Var("*".to_string())),
                            args: vec![IrExpr::Var("a".to_string()), IrExpr::Var("b".to_string())],
                        }),
                        env: std::collections::HashMap::new().into(),
                    },
                ),
            ]))
        }
        "Eq" => {
            // Default Eq instance uses structural equality
            Ok(Value::Record(vec![
                (
                    "==".to_string(),
                    Value::Closure {
                        params: vec!["_dict".to_string(), "a".to_string(), "b".to_string()],
                        body: Box::new(IrExpr::Apply {
                            func: Box::new(IrExpr::Var("==".to_string())),
                            args: vec![IrExpr::Var("a".to_string()), IrExpr::Var("b".to_string())],
                        }),
                        env: std::collections::HashMap::new().into(),
                    },
                ),
                (
                    "/=".to_string(),
                    Value::Closure {
                        params: vec!["_dict".to_string(), "a".to_string(), "b".to_string()],
                        body: Box::new(IrExpr::If {
                            cond: Box::new(IrExpr::Apply {
                                func: Box::new(IrExpr::Var("==".to_string())),
                                args: vec![
                                    IrExpr::Var("a".to_string()),
                                    IrExpr::Var("b".to_string()),
                                ],
                            }),
                            then_branch: Box::new(IrExpr::Bool(false)),
                            else_branch: Box::new(IrExpr::Bool(true)),
                        }),
                        env: std::collections::HashMap::new().into(),
                    },
                ),
            ]))
        }
        _ => {
            // For classes without default dicts, return error
            Err(Error::msg(format!(
                "no default dictionary available for class: {}",
                class_name
            )))
        }
    }
}

/// Auto-apply a dictionary when a Closure expects one.
/// This handles dict-lambdas that escape into contexts where concrete values are expected,
/// such as arithmetic operations (e.g., `b + 1` where `b` is a dict-lambda).
///
/// The function checks if the value is a Closure with exactly one parameter starting with
/// "__dict_" and attempts to find a matching dictionary from globals, or uses a default
/// dictionary for classes like Num, Eq, Show.
fn auto_apply_dict(g: &Globals, v: Value) -> Result<Value> {
    if let Value::Closure {
        params,
        body: _,
        env: _,
    } = &v
    {
        if params.len() == 1 && params[0].starts_with("__dict_") {
            // Try to find dict in globals first
            if let Ok(dict) = eval_var(g, &std::collections::HashMap::new(), &params[0]) {
                // Apply the dict but don't recursively force to avoid infinite loops
                return apply_one(g, v, dict);
            }

            // If no dict found in globals, try to inject a default dict for specific classes
            // Extract class name from param (e.g., "__dict_Num_Integer" -> "Num")
            let class_name = params[0].strip_prefix("__dict_").unwrap_or("");
            // Handle both short names (e.g., "Num_Integer") and qualified names
            let class_base = class_name.split('_').next().unwrap_or(class_name);
            let class_unqualified = class_base.rsplit('.').next().unwrap_or(class_base);

            if let Ok(default_dict) = try_get_default_dict(class_unqualified) {
                // Apply the dict but don't recursively force to avoid infinite loops
                return apply_one(g, v, default_dict);
            }
        }
    }
    Ok(v)
}

/// Force a value and auto-apply dict-lambdas for builtin operations.
/// This is used in contexts where we expect concrete values (e.g., arithmetic).
fn force_and_auto_apply(g: &Globals, v: Value) -> Result<Value> {
    let mut v = force_value(g, v)?;
    // Dict-lambdas can escape into runtime contexts that expect concrete values.
    // Apply a matching dictionary if possible, then force again so callers don't
    // observe thunks/closures at value boundaries.
    for _ in 0..4 {
        let next = auto_apply_dict(g, v)?;
        let next = force_value(g, next)?;
        match &next {
            Value::Closure { params, .. }
                if params.len() == 1 && params[0].starts_with("__dict_") =>
            {
                v = next;
                continue;
            }
            _ => return Ok(next),
        }
    }
    Ok(v)
}

fn force_value(g: &Globals, mut v: Value) -> Result<Value> {
    loop {
        match v {
            Value::Thunk(t) => {
                let forced = force_thunk(g, &t)?;
                // Handle indirection thunks (e.g. `[] ++ xs` returns `xs` without forcing).
                if let Value::Thunk(t2) = &forced {
                    if std::rc::Rc::ptr_eq(&t, t2) {
                        return Ok(forced);
                    }
                }
                v = forced;
            }
            other => return Ok(other),
        }
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

fn eval_qualified_dict_var_fallback(
    g: &Globals,
    env: &std::collections::HashMap<String, Value>,
    name: &str,
) -> Option<Value> {
    // AST-only typecheck paths can reference qualified dict vars like
    // `Prelude.__dict_Monad_IO` even when the runtime globals only contain
    // the unqualified name (e.g. `__dict_Monad_IO`).
    if !name.contains('.') {
        return None;
    }

    let last = name.rsplit('.').next()?;
    if !(last.starts_with("__dict_") || last.starts_with("__inst_")) {
        return None;
    }

    eval_var(g, env, last).ok()
}

fn eval_var(
    g: &Globals,
    env: &std::collections::HashMap<String, Value>,
    name: &str,
) -> Result<Value> {
    if let Some(v) = env.get(name) {
        return force_value(g, v.clone());
    }

    if let Some(v) = eval_builtin_var(g, name) {
        return Ok(v);
    }

    if !g.defs.contains_key(name) {
        if let Some(v) = eval_qualified_dict_var_fallback(g, env, name) {
            return Ok(v);
        }
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

fn eval_builtin_var(g: &Globals, name: &str) -> Option<Value> {
    // Some builtins are only used as a fallback when there isn't a user definition.
    fn only_if_undefined(g: &Globals, name: &str, v: Value) -> Option<Value> {
        if g.defs.contains_key(name) {
            None
        } else {
            Some(v)
        }
    }

    // `.ksif`-only / AST-only compilation paths can produce qualified dict/inst references
    // like `Prelude.__dict_Monad_IO` or `Prelude.Num.__dict_Num_Integer`.
    // When the qualified name is not defined in globals, fall back to the unqualified
    // `__dict_...` / `__inst_...` name so existing builtin minimal dictionaries and
    // default dict injection can apply.
    if !g.defs.contains_key(name) {
        if let Some(pos) = name.rfind(".__dict_") {
            let unqualified = &name[pos + 1..];
            if let Some(v) = eval_builtin_var(g, unqualified) {
                return Some(v);
            }
        }
        if let Some(pos) = name.rfind(".__inst_") {
            let unqualified = &name[pos + 1..];
            if let Some(v) = eval_builtin_var(g, unqualified) {
                return Some(v);
            }
        }
    }

    let v = match name {
        // Built-in IO constructor used by the minimal typecheck prelude.
        "IO" => Value::IoCtor,

        "stdoutWrite" => Value::BuiltinStdoutWrite,
        // Prelude defines `putStrLn` as `stdoutWrite (s ++ "\n")`. Provide it as a builtin
        // so `.ksif`-only compilation can still run programs that use Prelude I/O.
        "putStrLn" => Value::BuiltinPutStrLn,
        "concatMap" => Value::BuiltinConcatMap,

        "+" => Value::BuiltinAdd,
        "-" => Value::BuiltinSub,
        "*" => Value::BuiltinMul,
        "/" => Value::BuiltinDiv,

        "__quotInt" => Value::BuiltinQuotInt,
        "__remInt" => Value::BuiltinRemInt,
        "__divInt" => Value::BuiltinDivInt,
        "__modInt" => Value::BuiltinModInt,

        // Integer arithmetic builtins (used by Num Integer instance).
        "__builtin_Integer_add" => Value::BuiltinAdd,
        "__builtin_Integer_mul" => Value::BuiltinMul,

        "==" => return only_if_undefined(g, name, Value::BuiltinEq),
        "<" => Value::BuiltinLtInt,
        "<=" => Value::BuiltinLeInt,
        ">" => Value::BuiltinGtInt,
        ">=" => Value::BuiltinGeInt,
        "/=" => return only_if_undefined(g, name, Value::BuiltinNe),

        "&&" => Value::BuiltinAnd,
        "||" => Value::BuiltinOr,
        "not" => Value::BuiltinNot,

        "intToString" => Value::BuiltinIntToString,
        "boolToString" => Value::BuiltinBoolToString,
        "++" => Value::BuiltinListAppend,

        // Stable primitives used by derived instances (avoid conflict with overloaded names).
        "__primShow" => Value::BuiltinShow,
        "__primEq" => Value::BuiltinEq,

        "show" | "toString" => return only_if_undefined(g, name, Value::BuiltinShow),
        "__show" | "__toString" => Value::BuiltinShowDictApply,

        "__builtinShowDict" => Value::Record(vec![("show".to_string(), Value::BuiltinShow)]),

        "__eq" => Value::BuiltinEqDictApply,
        "__builtinEqDict" => Value::Record(vec![
            ("==".to_string(), Value::BuiltinEq),
            (
                "/=".to_string(),
                Value::Closure {
                    params: vec!["_dict".to_string(), "a".to_string(), "b".to_string()],
                    body: Box::new(IrExpr::If {
                        cond: Box::new(IrExpr::Apply {
                            func: Box::new(IrExpr::Var("==".to_string())),
                            args: vec![IrExpr::Var("a".to_string()), IrExpr::Var("b".to_string())],
                        }),
                        then_branch: Box::new(IrExpr::Bool(false)),
                        else_branch: Box::new(IrExpr::Bool(true)),
                    }),
                    env: std::collections::HashMap::new().into(),
                },
            ),
        ]),

        // Minimal dictionaries for `.ksif`-only execution.
        // These are methods that accept the dictionary as their first argument; we ignore it.
        "__dict_Prelude.Monad.Monad" | "__dict_Prelude.Monad.Monad_IO" => {
            return only_if_undefined(
                g,
                name,
                Value::Record(vec![
                    (
                        ">>".to_string(),
                        Value::Closure {
                            params: vec![
                                "_dict".to_string(),
                                "first".to_string(),
                                "second".to_string(),
                            ],
                            body: Box::new(IrExpr::Apply {
                                func: Box::new(IrExpr::Var("__ioThen".to_string())),
                                args: vec![
                                    IrExpr::Var("first".to_string()),
                                    IrExpr::Var("second".to_string()),
                                ],
                            }),
                            env: std::collections::HashMap::new().into(),
                        },
                    ),
                    (
                        ">>=".to_string(),
                        Value::Closure {
                            params: vec!["_dict".to_string(), "act".to_string(), "f".to_string()],
                            body: Box::new(IrExpr::Apply {
                                func: Box::new(IrExpr::Var("__ioBind".to_string())),
                                args: vec![
                                    IrExpr::Var("act".to_string()),
                                    IrExpr::Var("f".to_string()),
                                ],
                            }),
                            env: std::collections::HashMap::new().into(),
                        },
                    ),
                    (
                        "return".to_string(),
                        Value::Closure {
                            params: vec!["_dict".to_string(), "x".to_string()],
                            body: Box::new(IrExpr::Apply {
                                func: Box::new(IrExpr::Var("IO".to_string())),
                                args: vec![IrExpr::Var("x".to_string())],
                            }),
                            env: std::collections::HashMap::new().into(),
                        },
                    ),
                ]),
            );
        }

        // Backwards-compat / alternate naming.
        "__dict_Monad_IO" => {
            return only_if_undefined(
                g,
                name,
                Value::Record(vec![
                    (
                        ">>".to_string(),
                        Value::Closure {
                            params: vec![
                                "_dict".to_string(),
                                "first".to_string(),
                                "second".to_string(),
                            ],
                            body: Box::new(IrExpr::Apply {
                                func: Box::new(IrExpr::Var("__ioThen".to_string())),
                                args: vec![
                                    IrExpr::Var("first".to_string()),
                                    IrExpr::Var("second".to_string()),
                                ],
                            }),
                            env: std::collections::HashMap::new().into(),
                        },
                    ),
                    (
                        ">>=".to_string(),
                        Value::Closure {
                            params: vec!["_dict".to_string(), "act".to_string(), "f".to_string()],
                            body: Box::new(IrExpr::Apply {
                                func: Box::new(IrExpr::Var("__ioBind".to_string())),
                                args: vec![
                                    IrExpr::Var("act".to_string()),
                                    IrExpr::Var("f".to_string()),
                                ],
                            }),
                            env: std::collections::HashMap::new().into(),
                        },
                    ),
                    (
                        "return".to_string(),
                        Value::Closure {
                            params: vec!["_dict".to_string(), "x".to_string()],
                            body: Box::new(IrExpr::Apply {
                                func: Box::new(IrExpr::Var("IO".to_string())),
                                args: vec![IrExpr::Var("x".to_string())],
                            }),
                            env: std::collections::HashMap::new().into(),
                        },
                    ),
                ]),
            );
        }

        "__recordGet" => Value::BuiltinRecordGet,
        "error" => Value::BuiltinError,
        "throw" => Value::BuiltinThrow,
        "catch" => Value::BuiltinCatch,
        "try" => Value::BuiltinTry,

        "__ioBind" => Value::BuiltinIoBind,
        "__ioThen" => Value::BuiltinIoThen,

        "ffiAddI32" => Value::BuiltinFfiAddI32,
        "ffiAddF32" => Value::BuiltinFfiAddF32,

        #[cfg(feature = "unsafe_ffi")]
        "ffiPuts" => Value::BuiltinFfiPuts,

        "getArgs" => Value::IoAction(Box::new(IoAction::GetArgs)),
        "readFile" => Value::BuiltinReadFile,
        "writeFile" => Value::BuiltinWriteFile,
        "exitWith" => Value::BuiltinExitWith,

        "stdinReadLine" => Value::IoAction(Box::new(IoAction::StdinReadLine)),
        "readLine" => {
            return only_if_undefined(g, name, Value::IoAction(Box::new(IoAction::StdinReadLine)));
        }
        "print" => {
            return only_if_undefined(g, name, Value::BuiltinStdoutWrite);
        }

        n if n.starts_with("__dict_") => {
            // If stdlib instance dictionaries are missing (e.g. AST-only typecheck prelude),
            // fall back to a minimal default dictionary for supported classes.
            if g.defs.contains_key(n) {
                return None;
            }

            let class_name = n.strip_prefix("__dict_").unwrap_or("");
            let class_base = class_name.split('_').next().unwrap_or(class_name);
            let class_unqualified = class_base.rsplit('.').next().unwrap_or(class_base);

            if let Ok(default_dict) = try_get_default_dict(class_unqualified) {
                return Some(default_dict);
            }

            return None;
        }

        _ => return None,
    };
    Some(v)
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
            env: std::rc::Rc::new(env.clone()),
        },
        IrExpr::Apply { func, args } => eval_apply(g, env, func, args)?,
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
        IrExpr::Let { bindings, body } => eval_let(g, env, bindings, body)?,
        IrExpr::Cons { head, tail } => eval_cons(env, head, tail),
        IrExpr::List(es) => eval_list(env, es),
        IrExpr::Tuple(es) => eval_tuple(env, es),
        IrExpr::Record(fields) => eval_record(env, fields),
        IrExpr::CheckedCast { expr, target } => {
            let v = eval_expr(g, env, expr)?;
            checked_cast(g, v, *target)?
        }
        IrExpr::Case { expr, arms } => eval_case(g, env, expr, arms)?,
        IrExpr::IoBind {
            action,
            param,
            body,
        } => eval_ir_io_bind(g, env, action, param, body)?,
        IrExpr::IoThen { first, then_expr } => eval_ir_io_then(g, env, first, then_expr)?,
    })
}

fn mk_thunk(env: &std::collections::HashMap<String, Value>, expr: IrExpr) -> Value {
    Value::Thunk(std::rc::Rc::new(std::cell::RefCell::new(
        ThunkState::Unevaluated {
            expr,
            env: env.clone(),
        },
    )))
}

fn eval_apply(
    g: &Globals,
    env: &std::collections::HashMap<String, Value>,
    func: &IrExpr,
    args: &[IrExpr],
) -> Result<Value> {
    let mut f = eval_expr(g, env, func)?;
    for a in args {
        f = apply_one(g, f, mk_thunk(env, a.clone()))?;
    }
    Ok(f)
}

fn eval_let(
    g: &Globals,
    env: &std::collections::HashMap<String, Value>,
    bindings: &[(String, IrExpr)],
    body: &IrExpr,
) -> Result<Value> {
    // Recursive, order-independent let-bindings.
    // Allocate thunks first so each RHS sees the full environment.
    let mut env2 = env.clone();
    let mut thunks: Vec<(std::rc::Rc<std::cell::RefCell<ThunkState>>, IrExpr)> = Vec::new();

    for (name, e) in bindings {
        let t = std::rc::Rc::new(std::cell::RefCell::new(ThunkState::Unevaluated {
            expr: IrExpr::Unit,
            env: std::collections::HashMap::new(),
        }));
        env2.insert(name.clone(), Value::Thunk(t.clone()));
        thunks.push((t, e.clone()));
    }

    for (t, e) in thunks {
        *t.borrow_mut() = ThunkState::Unevaluated {
            expr: e,
            env: env2.clone(),
        };
    }

    let result = eval_expr(g, &env2, body)?;
    // Auto-apply dict lambdas when returning from let expressions
    auto_apply_io_dict(g, result)
}

fn eval_cons(
    env: &std::collections::HashMap<String, Value>,
    head: &IrExpr,
    tail: &IrExpr,
) -> Value {
    let hd = mk_thunk(env, head.clone());
    let tl = mk_thunk(env, tail.clone());
    Value::ListCons(Box::new(hd), Box::new(tl))
}

fn eval_list(env: &std::collections::HashMap<String, Value>, es: &[IrExpr]) -> Value {
    let mut out = Value::ListNil;
    for e in es.iter().rev() {
        let hd = mk_thunk(env, e.clone());
        out = Value::ListCons(Box::new(hd), Box::new(out));
    }
    out
}

fn eval_tuple(env: &std::collections::HashMap<String, Value>, es: &[IrExpr]) -> Value {
    Value::Tuple(es.iter().map(|e| mk_thunk(env, e.clone())).collect())
}

fn eval_record(
    env: &std::collections::HashMap<String, Value>,
    fields: &[(String, IrExpr)],
) -> Value {
    Value::Record(
        fields
            .iter()
            .map(|(n, e)| (n.clone(), mk_thunk(env, e.clone())))
            .collect(),
    )
}

fn eval_case(
    g: &Globals,
    env: &std::collections::HashMap<String, Value>,
    expr: &IrExpr,
    arms: &[IrCaseArm],
) -> Result<Value> {
    let scrut = eval_expr(g, env, expr)?;
    // Auto-apply dict-lambdas on the scrutinee before pattern matching
    let scrut = auto_apply_dict(g, scrut)?;
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
            let result = eval_expr(g, &env_arm, &arm.body)?;
            // Auto-apply dict lambdas when returning from case branches
            return auto_apply_io_dict(g, result);
        }
    }
    Err(Error::msg("non-exhaustive case"))
}

fn eval_ir_io_bind(
    g: &Globals,
    env: &std::collections::HashMap<String, Value>,
    action: &IrExpr,
    param: &str,
    body: &IrExpr,
) -> Result<Value> {
    let act = eval_expr(g, env, action)?;
    let Value::IoAction(act) = act else {
        return Err(Error::msg("IoBind action did not evaluate to an IO action"));
    };
    Ok(Value::IoAction(Box::new(IoAction::Bind {
        action: act,
        param: param.to_string(),
        body: Box::new(body.clone()),
        env: env.clone(),
    })))
}

fn eval_ir_io_then(
    g: &Globals,
    env: &std::collections::HashMap<String, Value>,
    first: &IrExpr,
    then_expr: &IrExpr,
) -> Result<Value> {
    let act = eval_expr(g, env, first)?;
    let Value::IoAction(act) = act else {
        return Err(Error::msg("IoThen first did not evaluate to an IO action"));
    };
    Ok(Value::IoAction(Box::new(IoAction::Then {
        first: act,
        then_expr: Box::new(then_expr.clone()),
        env: env.clone(),
    })))
}

fn run_io(g: &Globals, action: IoAction) -> Result<IoOutcome> {
    match action {
        IoAction::Pure(v) => Ok(IoOutcome::Value(force_value(g, v)?)),
        IoAction::StdoutWrite(s) => run_io_stdout_write(s),
        IoAction::StdinReadLine => run_io_stdin_readline(),
        IoAction::GetArgs => run_io_get_args(),
        IoAction::ReadFile(path) => run_io_read_file(path),
        IoAction::WriteFile { path, content } => run_io_write_file(path, content),
        IoAction::ExitWith(code) => run_io_exit_with(code),

        #[cfg(feature = "unsafe_ffi")]
        IoAction::FfiPuts(s) => run_io_ffi_puts(&s),

        IoAction::Throw(e) => Ok(IoOutcome::Thrown(e)),
        IoAction::Catch { action, handler } => run_io_catch(g, *action, handler),
        IoAction::Try { action } => run_io_try(g, *action),

        IoAction::BindValue { action, func } => run_io_bind_value(g, *action, func),
        IoAction::ThenValue { first, then_action } => run_io_then_value(g, *first, *then_action),

        IoAction::Bind {
            action,
            param,
            body,
            env,
        } => run_io_bind(g, *action, param, *body, env),
        IoAction::Then {
            first,
            then_expr,
            env,
        } => run_io_then(g, *first, *then_expr, env),
    }
}

fn run_io_stdout_write(s: String) -> Result<IoOutcome> {
    use std::io::Write;
    print!("{s}");
    std::io::stdout().flush().ok();
    Ok(IoOutcome::Value(Value::Unit))
}

fn run_io_stdin_readline() -> Result<IoOutcome> {
    use std::io::BufRead;
    let mut s = String::new();
    std::io::stdin().lock().read_line(&mut s)?;
    while s.ends_with(['\n', '\r']) {
        s.pop();
    }
    Ok(IoOutcome::Value(string_to_char_list(&s)))
}

fn run_io_get_args() -> Result<IoOutcome> {
    let args: Vec<Value> = std::env::args()
        .map(|arg| string_to_char_list(&arg))
        .collect();
    let mut list = Value::ListNil;
    for arg in args.into_iter().rev() {
        list = Value::ListCons(Box::new(arg), Box::new(list));
    }
    Ok(IoOutcome::Value(list))
}

fn run_io_read_file(path: String) -> Result<IoOutcome> {
    let content = std::fs::read_to_string(&path)
        .map_err(|e| Error::msg(format!("readFile: failed to read '{}': {}", path, e)))?;
    Ok(IoOutcome::Value(Value::String(content)))
}

fn run_io_write_file(path: String, content: String) -> Result<IoOutcome> {
    std::fs::write(&path, &content)
        .map_err(|e| Error::msg(format!("writeFile: failed to write '{}': {}", path, e)))?;
    Ok(IoOutcome::Value(Value::Unit))
}

fn run_io_exit_with(code: i64) -> Result<IoOutcome> {
    std::process::exit(code as i32);
    // Note: std::process::exit never returns, but Rust doesn't have a ! return type for functions
    // that call exit, so we need this unreachable code to satisfy the type checker
    #[allow(unreachable_code)]
    {
        Ok(IoOutcome::Value(Value::Unit))
    }
}

#[cfg(feature = "unsafe_ffi")]
fn run_io_ffi_puts(s: &str) -> Result<IoOutcome> {
    crate::debug::unsafe_used("ffiPuts");
    let rc =
        kscr_unsafe_ffi::puts_checked(s).map_err(|_| Error::msg("ffiPuts: string contains NUL"))?;
    Ok(IoOutcome::Value(Value::Integer(int_from_i64(rc as i64))))
}

fn run_io_catch(g: &Globals, action: IoAction, handler: Value) -> Result<IoOutcome> {
    match run_io(g, action)? {
        IoOutcome::Value(v) => Ok(IoOutcome::Value(v)),
        IoOutcome::Thrown(e) => {
            let h = force_value(g, handler)?;
            let act = apply_one(g, h, string_to_char_list(&e))?;
            let Value::IoAction(act) = act else {
                return Err(Error::msg("catch handler did not evaluate to an IO action"));
            };
            run_io(g, *act)
        }
    }
}

fn run_io_try(g: &Globals, action: IoAction) -> Result<IoOutcome> {
    match run_io(g, action)? {
        IoOutcome::Value(v) => {
            let ctor = eval_var(g, &std::collections::HashMap::new(), "Right")?;
            Ok(IoOutcome::Value(apply_one(g, ctor, v)?))
        }
        IoOutcome::Thrown(e) => {
            let ctor = eval_var(g, &std::collections::HashMap::new(), "Left")?;
            Ok(IoOutcome::Value(apply_one(
                g,
                ctor,
                string_to_char_list(&e),
            )?))
        }
    }
}

fn run_io_bind_value(g: &Globals, action: IoAction, func: Value) -> Result<IoOutcome> {
    let v = match run_io(g, action)? {
        IoOutcome::Value(v) => v,
        IoOutcome::Thrown(e) => return Ok(IoOutcome::Thrown(e)),
    };
    let func = force_value(g, func)?;
    let act = apply_one(g, func, v)?;
    let act = force_value(g, act)?;
    let act = auto_apply_io_dict(g, act)?;
    let Value::IoAction(act) = act else {
        return Err(Error::msg(format!(
            "__ioBind: body did not evaluate to an IO action (got {})",
            value_type_name(&act)
        )));
    };
    run_io(g, *act)
}

fn run_io_then_value(g: &Globals, first: IoAction, then_action: IoAction) -> Result<IoOutcome> {
    match run_io(g, first)? {
        IoOutcome::Value(_) => {}
        IoOutcome::Thrown(e) => return Ok(IoOutcome::Thrown(e)),
    }
    run_io(g, then_action)
}

fn run_io_bind(
    g: &Globals,
    action: IoAction,
    param: String,
    body: IrExpr,
    mut env: std::collections::HashMap<String, Value>,
) -> Result<IoOutcome> {
    let v = match run_io(g, action)? {
        IoOutcome::Value(v) => v,
        IoOutcome::Thrown(e) => return Ok(IoOutcome::Thrown(e)),
    };
    env.insert(param, v);
    let act = eval_expr(g, &env, &body)?;
    let act = force_value(g, act)?;
    let act = auto_apply_io_dict(g, act)?;
    let Value::IoAction(act) = act else {
        return Err(Error::msg(format!(
            "IoBind body did not evaluate to an IO action (got {})",
            value_type_name(&act)
        )));
    };
    run_io(g, *act)
}

fn run_io_then(
    g: &Globals,
    first: IoAction,
    then_expr: IrExpr,
    env: std::collections::HashMap<String, Value>,
) -> Result<IoOutcome> {
    match run_io(g, first)? {
        IoOutcome::Value(_) => {}
        IoOutcome::Thrown(e) => return Ok(IoOutcome::Thrown(e)),
    }
    let act = eval_expr(g, &env, &then_expr)?;
    let act = force_value(g, act)?;
    let act = auto_apply_io_dict(g, act)?;
    let Value::IoAction(act) = act else {
        return Err(Error::msg(format!(
            "IoThen body did not evaluate to an IO action (got {})",
            value_type_name(&act)
        )));
    };
    run_io(g, *act)
}

fn apply_one(g: &Globals, fun: Value, arg: Value) -> Result<Value> {
    match fun {
        Value::IoCtor => Ok(Value::IoAction(Box::new(IoAction::Pure(arg)))),
        Value::BuiltinStdoutWrite => {
            let arg = force_and_auto_apply(g, arg)?;
            // Dict-passing can supply a leading `Show` dictionary to `print`.
            // When `print` falls back to this builtin, ignore that dictionary.
            if is_probably_typeclass_dict_record(&arg) {
                Ok(Value::BuiltinStdoutWrite)
            } else {
                apply_builtin_stdout_write(g, arg)
            }
        }
        Value::BuiltinPutStrLn => apply_builtin_put_str_ln(g, arg),
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

        Value::BuiltinQuotInt => Ok(Value::BuiltinQuotInt1(Box::new(arg))),
        Value::BuiltinQuotInt1(a) => quot_int(g, *a, arg),
        Value::BuiltinRemInt => Ok(Value::BuiltinRemInt1(Box::new(arg))),
        Value::BuiltinRemInt1(a) => rem_int(g, *a, arg),
        Value::BuiltinDivInt => Ok(Value::BuiltinDivInt1(Box::new(arg))),
        Value::BuiltinDivInt1(a) => div_floor_int(g, *a, arg),
        Value::BuiltinModInt => Ok(Value::BuiltinModInt1(Box::new(arg))),
        Value::BuiltinModInt1(a) => mod_floor_int(g, *a, arg),

        Value::BuiltinEq => {
            let arg = force_and_auto_apply(g, arg)?;
            // Typeclass method calls pass the instance dictionary as the first arg.
            // When `__primEq` is used as an instance method body (e.g. `== = __primEq`),
            // ignore that leading dictionary.
            if is_probably_typeclass_dict_record(&arg) {
                Ok(Value::BuiltinEq)
            } else {
                Ok(Value::BuiltinEq1(Box::new(arg)))
            }
        }
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
        Value::BuiltinNe => {
            let arg = force_and_auto_apply(g, arg)?;
            if is_probably_typeclass_dict_record(&arg) {
                Ok(Value::BuiltinNe)
            } else {
                Ok(Value::BuiltinNe1(Box::new(arg)))
            }
        }
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
        Value::BuiltinListAppend => Ok(Value::BuiltinListAppend1(Box::new(arg))),
        Value::BuiltinListAppend1(a) => list_append(g, *a, arg),
        Value::BuiltinShowDictApply => Ok(Value::BuiltinShowDictApply1(Box::new(arg))),
        Value::BuiltinShowDictApply1(builtin_dict) => {
            Ok(Value::BuiltinShowDictApply2(builtin_dict, Box::new(arg)))
        }
        Value::BuiltinShowDictApply2(builtin_dict, inst_dict) => {
            show_with_dict(g, *builtin_dict, *inst_dict, arg)
        }
        Value::BuiltinEqDictApply => Ok(Value::BuiltinEqDictApply1(Box::new(arg))),
        Value::BuiltinEqDictApply1(builtin_dict) => {
            Ok(Value::BuiltinEqDictApply2(builtin_dict, Box::new(arg)))
        }
        Value::BuiltinEqDictApply2(builtin_dict, inst_dict) => Ok(Value::BuiltinEqDictApply3(
            builtin_dict,
            inst_dict,
            Box::new(arg),
        )),
        Value::BuiltinEqDictApply3(builtin_dict, inst_dict, a) => {
            eq_with_dict(g, *builtin_dict, *inst_dict, *a, arg)
        }
        Value::BuiltinRecordGet => Ok(Value::BuiltinRecordGet1(Box::new(arg))),
        Value::BuiltinRecordGet1(d) => record_get(g, *d, arg),
        Value::BuiltinShow => {
            let arg = force_and_auto_apply(g, arg)?;
            // See BuiltinEq note above.
            if is_probably_typeclass_dict_record(&arg) {
                Ok(Value::BuiltinShow)
            } else {
                show_to_string(g, arg)
            }
        }

        Value::BuiltinError => builtin_error(g, arg),
        Value::BuiltinThrow => builtin_throw(g, arg),
        Value::BuiltinCatch => Ok(Value::BuiltinCatch1(Box::new(arg))),
        Value::BuiltinCatch1(act) => builtin_catch1(g, *act, arg),
        Value::BuiltinTry => builtin_try(g, arg),

        Value::BuiltinIoBind => Ok(Value::BuiltinIoBind1(Box::new(arg))),
        Value::BuiltinIoBind1(act) => builtin_io_bind1(g, *act, arg),

        Value::BuiltinIoThen => Ok(Value::BuiltinIoThen1(Box::new(arg))),
        Value::BuiltinIoThen1(first) => builtin_io_then1(g, *first, arg),

        Value::BuiltinFfiAddI32 => Ok(Value::BuiltinFfiAddI32_1(Box::new(arg))),
        Value::BuiltinFfiAddI32_1(a) => ffi_add_i32(g, *a, arg),
        Value::BuiltinFfiAddF32 => Ok(Value::BuiltinFfiAddF32_1(Box::new(arg))),
        Value::BuiltinFfiAddF32_1(a) => ffi_add_f32(g, *a, arg),

        #[cfg(feature = "unsafe_ffi")]
        Value::BuiltinFfiPuts => builtin_ffi_puts(g, arg),

        Value::BuiltinReadFile => builtin_read_file(g, arg),
        Value::BuiltinWriteFile => Ok(Value::BuiltinWriteFile1(Box::new(arg))),
        Value::BuiltinWriteFile1(path) => builtin_write_file(g, *path, arg),
        Value::BuiltinExitWith => builtin_exit_with(g, arg),

        Value::Closure { params, body, env } => apply_closure(g, params, body, env, arg),

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

fn apply_builtin_stdout_write(g: &Globals, arg: Value) -> Result<Value> {
    let arg = force_value(g, arg)?;
    let s = value_to_string(g, arg)?;
    Ok(Value::IoAction(Box::new(IoAction::StdoutWrite(s))))
}

fn apply_builtin_put_str_ln(g: &Globals, arg: Value) -> Result<Value> {
    let arg = force_value(g, arg)?;
    let s = value_to_string(g, arg)?;
    Ok(Value::IoAction(Box::new(IoAction::StdoutWrite(format!(
        "{s}\n"
    )))))
}

fn builtin_read_file(g: &Globals, path: Value) -> Result<Value> {
    let path = force_value(g, path)?;
    let path_str = value_to_string(g, path)?;
    Ok(Value::IoAction(Box::new(IoAction::ReadFile(path_str))))
}

fn builtin_write_file(g: &Globals, path: Value, content: Value) -> Result<Value> {
    let path = force_value(g, path)?;
    let path_str = value_to_string(g, path)?;
    let content = force_value(g, content)?;
    let content_str = value_to_string(g, content)?;
    Ok(Value::IoAction(Box::new(IoAction::WriteFile {
        path: path_str,
        content: content_str,
    })))
}

fn builtin_exit_with(g: &Globals, code: Value) -> Result<Value> {
    let code = force_value(g, code)?;
    let Value::Integer(i) = code else {
        return Err(Error::msg("exitWith: expected Integer"));
    };
    let code_i64 = int_to_i64(&i)?;
    Ok(Value::IoAction(Box::new(IoAction::ExitWith(code_i64))))
}

fn builtin_error(g: &Globals, arg: Value) -> Result<Value> {
    let arg = force_value(g, arg)?;
    let s = value_to_string(g, arg)?;
    Err(Error::msg(format!("error: {s}")))
}

fn builtin_throw(g: &Globals, arg: Value) -> Result<Value> {
    let arg = force_value(g, arg)?;
    let s = value_to_string(g, arg)?;
    Ok(Value::IoAction(Box::new(IoAction::Throw(s))))
}

fn builtin_catch1(g: &Globals, act: Value, handler: Value) -> Result<Value> {
    let act = force_value(g, act)?;
    let Value::IoAction(act) = act else {
        return Err(Error::msg("catch expects IO action"));
    };
    let handler = force_value(g, handler)?;
    Ok(Value::IoAction(Box::new(IoAction::Catch {
        action: act,
        handler,
    })))
}

fn builtin_try(g: &Globals, arg: Value) -> Result<Value> {
    let act = force_value(g, arg)?;
    let Value::IoAction(act) = act else {
        return Err(Error::msg("try expects IO action"));
    };
    Ok(Value::IoAction(Box::new(IoAction::Try { action: act })))
}

fn builtin_io_bind1(g: &Globals, act: Value, func: Value) -> Result<Value> {
    let act = force_value(g, act)?;
    let act = auto_apply_io_dict(g, act)?;
    let Value::IoAction(act) = act else {
        return Err(Error::msg(format!(
            "__ioBind expects IO action (got {})",
            value_type_name(&act)
        )));
    };
    Ok(Value::IoAction(Box::new(IoAction::BindValue {
        action: act,
        func,
    })))
}

fn builtin_io_then1(g: &Globals, first: Value, then_action: Value) -> Result<Value> {
    let first = force_value(g, first)?;
    let first = auto_apply_io_dict(g, first)?;
    let Value::IoAction(first) = first else {
        return Err(Error::msg(format!(
            "__ioThen expects IO action (got {} for first argument)",
            value_type_name(&first)
        )));
    };
    let then_action = force_value(g, then_action)?;
    let then_action = auto_apply_io_dict(g, then_action)?;
    let Value::IoAction(then_action) = then_action else {
        return Err(Error::msg(format!(
            "__ioThen expects IO action (got {} for second argument)",
            value_type_name(&then_action)
        )));
    };
    Ok(Value::IoAction(Box::new(IoAction::ThenValue {
        first,
        then_action,
    })))
}

#[cfg(feature = "unsafe_ffi")]
fn builtin_ffi_puts(g: &Globals, arg: Value) -> Result<Value> {
    let arg = force_value(g, arg)?;
    let s = value_to_string(g, arg)?;
    Ok(Value::IoAction(Box::new(IoAction::FfiPuts(s))))
}

fn apply_closure(
    g: &Globals,
    params: Vec<String>,
    body: Box<IrExpr>,
    env: SharedEnv,
    arg: Value,
) -> Result<Value> {
    if params.is_empty() {
        return Err(Error::msg("cannot apply function with no params"));
    }

    let mut params = params;
    let mut env = (*env).clone();
    let p = params.remove(0);
    env.insert(p, arg);
    if params.is_empty() {
        eval_expr(g, &env, &body)
    } else {
        Ok(Value::Closure {
            params,
            body,
            env: std::rc::Rc::new(env),
        })
    }
}

fn quot_rem_trunc(a: Integer, b: Integer) -> (Integer, Integer) {
    // Truncating division (toward zero), compatible with current Integer `/`.
    // Remainder computed as: r = a - (a / b) * b.
    let q = a.clone() / b.clone();
    let r = a - (q.clone() * b);
    (q, r)
}

#[allow(clippy::assign_op_pattern)]
fn div_mod_floor(a: Integer, b: Integer) -> (Integer, Integer) {
    // Floor division / modulus (Haskell-like `div`/`mod`).
    // If signs differ and remainder is non-zero, adjust:
    //   q' = q - 1
    //   r' = r + b
    let (mut q, mut r) = quot_rem_trunc(a.clone(), b.clone());
    if !int_is_zero(&r) {
        let zero = int_from_i64(0);
        let signs_differ = (a < zero) != (b < int_from_i64(0));
        if signs_differ {
            q = q - int_from_i64(1);
            r = r + b;
        }
    }
    (q, r)
}

fn quot_int(g: &Globals, a: Value, b: Value) -> Result<Value> {
    let a = force_and_auto_apply(g, a)?;
    let b = force_and_auto_apply(g, b)?;
    let Value::Integer(a) = a else {
        return Err(Error::msg("__quotInt expects Integer"));
    };
    let Value::Integer(b) = b else {
        return Err(Error::msg("__quotInt expects Integer"));
    };
    if int_is_zero(&b) {
        return Err(Error::msg("division by zero"));
    }
    let (q, _) = quot_rem_trunc(a, b);
    Ok(Value::Integer(q))
}

fn rem_int(g: &Globals, a: Value, b: Value) -> Result<Value> {
    let a = force_and_auto_apply(g, a)?;
    let b = force_and_auto_apply(g, b)?;
    let Value::Integer(a) = a else {
        return Err(Error::msg("__remInt expects Integer"));
    };
    let Value::Integer(b) = b else {
        return Err(Error::msg("__remInt expects Integer"));
    };
    if int_is_zero(&b) {
        return Err(Error::msg("division by zero"));
    }
    let (_, r) = quot_rem_trunc(a, b);
    Ok(Value::Integer(r))
}

fn div_floor_int(g: &Globals, a: Value, b: Value) -> Result<Value> {
    let a = force_and_auto_apply(g, a)?;
    let b = force_and_auto_apply(g, b)?;
    let Value::Integer(a) = a else {
        return Err(Error::msg("__divInt expects Integer"));
    };
    let Value::Integer(b) = b else {
        return Err(Error::msg("__divInt expects Integer"));
    };
    if int_is_zero(&b) {
        return Err(Error::msg("division by zero"));
    }
    let (q, _) = div_mod_floor(a, b);
    Ok(Value::Integer(q))
}

fn mod_floor_int(g: &Globals, a: Value, b: Value) -> Result<Value> {
    let a = force_and_auto_apply(g, a)?;
    let b = force_and_auto_apply(g, b)?;
    let Value::Integer(a) = a else {
        return Err(Error::msg("__modInt expects Integer"));
    };
    let Value::Integer(b) = b else {
        return Err(Error::msg("__modInt expects Integer"));
    };
    if int_is_zero(&b) {
        return Err(Error::msg("division by zero"));
    }
    let (_, r) = div_mod_floor(a, b);
    Ok(Value::Integer(r))
}

fn record_get(g: &Globals, dict: Value, label: Value) -> Result<Value> {
    let dict = force_value(g, dict)?;
    let Value::Record(fields) = dict else {
        if std::env::var("KSCR_DEBUG_RECORDGET").ok().as_deref() == Some("1") {
            eprintln!("[KSCR_DEBUG_RECORDGET] __recordGet got non-record: {dict:?}");
        }
        return Err(Error::msg("__recordGet expects a record"));
    };

    let label = force_value(g, label)?;
    let label = match label {
        Value::String(s) => s,
        other => value_to_string(g, other)
            .map_err(|_| Error::msg("__recordGet expects String/[Char] label"))?,
    };

    let Some((_, v)) = fields.into_iter().find(|(k, _)| k == &label) else {
        return Err(Error::msg(format!("record missing field: {label}")));
    };
    force_value(g, v)
}

fn vec_chars_to_list(chars: Vec<char>) -> Value {
    let mut out = Value::ListNil;
    for ch in chars.into_iter().rev() {
        out = Value::ListCons(Box::new(Value::Char(ch)), Box::new(out));
    }
    out
}

fn string_to_char_list(s: &str) -> Value {
    vec_chars_to_list(s.chars().collect())
}

fn value_to_string(g: &Globals, v: Value) -> Result<String> {
    let v = force_and_auto_apply(g, v)?;
    match v {
        Value::String(s) => Ok(s),
        Value::ListNil | Value::ListCons(_, _) => {
            let elems = list_to_vec(g, v)?;
            let mut out = String::new();
            for e in elems {
                let e = force_and_auto_apply(g, e)?;
                let Value::Char(ch) = e else {
                    return Err(Error::msg("expected [Char]"));
                };
                out.push(ch);
            }
            Ok(out)
        }
        other => {
            if std::env::var("KSCR_DEBUG_VALUE_TO_STRING").ok().as_deref() == Some("1") {
                eprintln!("[KSCR_DEBUG_VALUE_TO_STRING] got non-string value: {other:?}");
            }
            Err(Error::msg("expected String/[Char]"))
        }
    }
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
            Value::String(s) => {
                out.extend(s.chars().map(Value::Char));
                return Ok(out);
            }
            other => return Err(Error::msg(format!("expected List, got {other:?}"))),
        }
    }
}

fn string_uncons(s: String) -> Option<(Value, Value)> {
    let mut chars = s.chars();
    let head = chars.next()?;
    let tail = chars.as_str();
    let tail_value = if tail.is_empty() {
        Value::ListNil
    } else {
        Value::String(tail.to_string())
    };
    Some((Value::Char(head), tail_value))
}

fn list_uncons(g: &Globals, v: Value) -> Result<Option<(Value, Value)>> {
    let v = force_value(g, v)?;
    match v {
        Value::ListNil => Ok(None),
        Value::ListCons(h, t) => Ok(Some((*h, *t))),
        Value::String(s) => Ok(string_uncons(s)),
        _ => Ok(None),
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

fn int_to_i64(n: &Integer) -> Result<i64> {
    #[cfg(feature = "unsafe_bigint")]
    {
        if !kscr_unsafe_bigint::in_i64_range(n) {
            return Err(Error::msg("integer out of range for i64"));
        }
        Ok(kscr_unsafe_bigint::to_i64_range_checked(n.clone()))
    }

    #[cfg(not(feature = "unsafe_bigint"))]
    {
        if !crate::safe_bigint::in_i64_range(n) {
            return Err(Error::msg("integer out of range for i64"));
        }
        Ok(crate::safe_bigint::to_i64_range_checked(n.clone()))
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
    let a = force_and_auto_apply(g, a)?;
    let b = force_and_auto_apply(g, b)?;
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
    let a = force_and_auto_apply(g, a)?;
    let b = force_and_auto_apply(g, b)?;
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
    let a = force_and_auto_apply(g, a)?;
    let b = force_and_auto_apply(g, b)?;
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
    let a = force_and_auto_apply(g, a)?;
    let b = force_and_auto_apply(g, b)?;
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
    let a = force_and_auto_apply(g, a)?;
    let b = force_and_auto_apply(g, b)?;
    let Value::Integer(a) = a else {
        return Err(Error::msg("== expects Integer"));
    };
    let Value::Integer(b) = b else {
        return Err(Error::msg("== expects Integer"));
    };
    Ok(Value::Bool(a == b))
}

fn lt_int(g: &Globals, a: Value, b: Value) -> Result<Value> {
    let a = force_and_auto_apply(g, a)?;
    let b = force_and_auto_apply(g, b)?;
    let Value::Integer(a) = a else {
        return Err(Error::msg("< expects Integer"));
    };
    let Value::Integer(b) = b else {
        return Err(Error::msg("< expects Integer"));
    };
    Ok(Value::Bool(a < b))
}

fn le_int(g: &Globals, a: Value, b: Value) -> Result<Value> {
    let a = force_and_auto_apply(g, a)?;
    let b = force_and_auto_apply(g, b)?;
    let Value::Integer(a) = a else {
        return Err(Error::msg("<= expects Integer"));
    };
    let Value::Integer(b) = b else {
        return Err(Error::msg("<= expects Integer"));
    };
    Ok(Value::Bool(a <= b))
}

fn gt_int(g: &Globals, a: Value, b: Value) -> Result<Value> {
    let a = force_and_auto_apply(g, a)?;
    let b = force_and_auto_apply(g, b)?;
    let Value::Integer(a) = a else {
        return Err(Error::msg("> expects Integer"));
    };
    let Value::Integer(b) = b else {
        return Err(Error::msg("> expects Integer"));
    };
    Ok(Value::Bool(a > b))
}

fn ge_int(g: &Globals, a: Value, b: Value) -> Result<Value> {
    let a = force_and_auto_apply(g, a)?;
    let b = force_and_auto_apply(g, b)?;
    let Value::Integer(a) = a else {
        return Err(Error::msg(">= expects Integer"));
    };
    let Value::Integer(b) = b else {
        return Err(Error::msg(">= expects Integer"));
    };
    Ok(Value::Bool(a >= b))
}

fn ne_int(g: &Globals, a: Value, b: Value) -> Result<Value> {
    let a = force_and_auto_apply(g, a)?;
    let b = force_and_auto_apply(g, b)?;
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
    Ok(string_to_char_list(&a.to_string()))
}

fn bool_to_string(g: &Globals, a: Value) -> Result<Value> {
    let a = force_value(g, a)?;
    let Value::Bool(a) = a else {
        return Err(Error::msg("boolToString expects Bool"));
    };
    Ok(string_to_char_list(if a { "True" } else { "False" }))
}

fn list_append(g: &Globals, a: Value, b: Value) -> Result<Value> {
    let mut a = force_value(g, a)?;

    // Force again if still a thunk (can happen with nested lazy evaluation)
    if matches!(a, Value::Thunk(_)) {
        a = force_value(g, a)?;
    }

    match a {
        Value::ListNil => Ok(b),
        Value::ListCons(h, t) => {
            // NOTE: this is eager in the left spine (MVP), but does not force elements.
            let rest = list_append(g, *t, b)?;
            Ok(Value::ListCons(h, Box::new(rest)))
        }
        Value::String(ref s) => {
            // Convert string literal to [Char] and append with b.
            // This supports mixing String literals with [Char] since String = [Char] in Prelude.
            let char_list = string_to_char_list(s);
            list_append(g, char_list, b)
        }
        _ => Err(Error::msg("++ expects List")),
    }
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
    let v = force_and_auto_apply(g, v)?;
    Ok(match v {
        Value::Integer(n) => n.to_string(),
        Value::Float64(x) => x.to_string(),
        Value::Bool(b) => if b { "True" } else { "False" }.to_string(),
        Value::String(s) => quote_string(&s),
        Value::Char(c) => quote_char(c),
        Value::Unit => "()".to_string(),
        Value::Tuple(vs) => {
            let mut parts: Vec<String> = Vec::new();
            for v in vs {
                let v_forced = force_value(g, v)?;
                parts.push(show_value_str(g, v_forced)?);
            }
            format!("({})", parts.join(", "))
        }
        Value::ListNil | Value::ListCons(_, _) => {
            // Haskell-like Show instance special-case: [Char] prints as a quoted string.
            let elems = list_to_vec(g, v)?;
            let mut chars = Vec::with_capacity(elems.len());
            let mut all_char = true;
            for e in &elems {
                let e = force_and_auto_apply(g, e.clone())?;
                if let Value::Char(ch) = e {
                    chars.push(ch);
                } else {
                    all_char = false;
                    break;
                }
            }
            if all_char {
                let s: String = chars.into_iter().collect();
                return Ok(quote_string(&s));
            }

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
                let ctor = force_and_auto_apply(g, ctor)?;
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
        other => {
            if std::env::var("KSCR_DEBUG_SHOW_VALUE_STR").ok().as_deref() == Some("1") {
                eprintln!("[KSCR_DEBUG_SHOW_VALUE_STR] got non-printable value: {other:?}");
            }
            return Err(Error::msg("show/toString expects a printable value"));
        }
    })
}

fn show_to_string(g: &Globals, a: Value) -> Result<Value> {
    Ok(string_to_char_list(&show_value_str(g, a)?))
}

fn show_with_dict(g: &Globals, builtin_dict: Value, dict: Value, a: Value) -> Result<Value> {
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
    let f = apply_one(g, show_fn, builtin_dict)?;
    apply_one(g, f, a)
}

fn eq_list_like_values(g: &Globals, a: Value, b: Value) -> Result<bool> {
    let a_elems = list_to_vec(g, a)?;
    let b_elems = list_to_vec(g, b)?;
    if a_elems.len() != b_elems.len() {
        return Ok(false);
    }
    for (x, y) in a_elems.into_iter().zip(b_elems) {
        if !eq_values(g, x, y)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn eq_values(g: &Globals, a: Value, b: Value) -> Result<bool> {
    let a = force_and_auto_apply(g, a)?;
    let b = force_and_auto_apply(g, b)?;

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
            for (x, y) in as_.into_iter().zip(bs) {
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
        (a @ Value::String(_), b @ Value::ListNil)
        | (a @ Value::String(_), b @ Value::ListCons(_, _))
        | (a @ Value::ListNil, b @ Value::String(_))
        | (a @ Value::ListCons(_, _), b @ Value::String(_)) => eq_list_like_values(g, a, b)?,

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
                for (x, y) in a_elems.into_iter().zip(b_elems) {
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
            for ((ak, av), (bk, bv)) in a_fields.into_iter().zip(b_fields) {
                if ak != bk {
                    return Ok(false);
                }
                if !eq_values(g, av, bv)? {
                    return Ok(false);
                }
            }
            true
        }

        (a, b) => {
            if std::env::var("KSCR_DEBUG_EQ_VALUES").ok().as_deref() == Some("1") {
                eprintln!("[KSCR_DEBUG_EQ_VALUES] got non-equatable values: a={a:?} b={b:?}");
            }
            return Err(Error::msg("== expects equatable values"));
        }
    })
}

fn eq_value(g: &Globals, a: Value, b: Value) -> Result<Value> {
    Ok(Value::Bool(eq_values(g, a, b)?))
}

fn ne_value(g: &Globals, a: Value, b: Value) -> Result<Value> {
    Ok(Value::Bool(!eq_values(g, a, b)?))
}

fn eq_with_dict(
    g: &Globals,
    builtin_dict: Value,
    dict: Value,
    a: Value,
    b: Value,
) -> Result<Value> {
    let dict = force_value(g, dict)?;
    let Value::Record(fields) = dict else {
        return Err(Error::msg("__eq expects an Eq dictionary record"));
    };

    let Some((_, eq_fn)) = fields.into_iter().find(|(k, _)| k == "==") else {
        return Err(Error::msg("Eq dictionary missing field: =="));
    };
    let eq_fn = force_value(g, eq_fn)?;
    let f = apply_one(g, eq_fn, builtin_dict)?;
    let f = apply_one(g, f, a)?;
    apply_one(g, f, b)
}

fn match_pat_trivial(
    pat: &IrPattern,
    val: &Value,
) -> Option<std::collections::HashMap<String, Value>> {
    use IrPattern as P;
    match pat {
        P::Wildcard => Some(std::collections::HashMap::new()),
        P::Var(n) => {
            let mut m = std::collections::HashMap::new();
            m.insert(n.clone(), val.clone());
            Some(m)
        }
        _ => None,
    }
}

fn match_pat_literal(l: &IrLiteral, v: &Value) -> Result<bool> {
    Ok(match (l, v) {
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
    })
}

fn match_pat_tuple(
    g: &Globals,
    env: &std::collections::HashMap<String, Value>,
    ps: &[IrPattern],
    vs: &[Value],
) -> Result<Option<std::collections::HashMap<String, Value>>> {
    if ps.len() != vs.len() {
        return Ok(None);
    }
    let mut out = std::collections::HashMap::new();
    for (p, v) in ps.iter().zip(vs.iter()) {
        let Some(b) = match_pat(g, env, p, v)? else {
            return Ok(None);
        };
        out.extend(b);
    }
    Ok(Some(out))
}

fn match_pat_list(
    g: &Globals,
    env: &std::collections::HashMap<String, Value>,
    ps: &[IrPattern],
    v: &Value,
) -> Result<Option<std::collections::HashMap<String, Value>>> {
    let mut out = std::collections::HashMap::new();
    let mut cur = v.clone();
    for p in ps.iter() {
        let Some((h, t)) = list_uncons(g, cur)? else {
            return Ok(None);
        };
        let Some(b) = match_pat(g, env, p, &h)? else {
            return Ok(None);
        };
        out.extend(b);
        cur = t;
    }
    let cur = force_value(g, cur)?;
    if matches!(cur, Value::ListNil) || matches!(&cur, Value::String(s) if s.is_empty()) {
        Ok(Some(out))
    } else {
        Ok(None)
    }
}

fn match_pat_cons(
    g: &Globals,
    env: &std::collections::HashMap<String, Value>,
    hd: &IrPattern,
    tl: &IrPattern,
    v: &Value,
) -> Result<Option<std::collections::HashMap<String, Value>>> {
    let Some((h, t)) = list_uncons(g, force_and_auto_apply(g, v.clone())?)? else {
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
    Ok(Some(out))
}

fn match_pat_record(
    g: &Globals,
    env: &std::collections::HashMap<String, Value>,
    fs: &[(String, IrPattern)],
    vs: &[(String, Value)],
) -> Result<Option<std::collections::HashMap<String, Value>>> {
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
    Ok(Some(out))
}

fn match_pat_record_loose(
    g: &Globals,
    env: &std::collections::HashMap<String, Value>,
    fs: &[(String, IrPattern)],
    rest: &Option<String>,
    vs: &[(String, Value)],
) -> Result<Option<std::collections::HashMap<String, Value>>> {
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

    Ok(Some(out))
}

fn match_pat_constructor(
    g: &Globals,
    env: &std::collections::HashMap<String, Value>,
    name: &str,
    args: &[IrPattern],
    vs: &[(String, Value)],
) -> Result<Option<std::collections::HashMap<String, Value>>> {
    let Some((_, ctor_v)) = vs.iter().find(|(n, _)| n == "__ctor") else {
        return Ok(None);
    };
    let ctor_v = force_value(g, ctor_v.clone())?;
    let Value::String(ctor_name) = ctor_v else {
        return Ok(None);
    };
    if ctor_name != name {
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
    Ok(Some(out))
}

fn match_pat_view(
    g: &Globals,
    env: &std::collections::HashMap<String, Value>,
    p: &IrPattern,
    e: &IrExpr,
    v: &Value,
) -> Result<Option<std::collections::HashMap<String, Value>>> {
    let fv = eval_expr(g, env, e)?;
    let v2 = apply_one(g, fv, v.clone())?;
    match_pat(g, env, p, &v2)
}

fn match_pat(
    g: &Globals,
    env: &std::collections::HashMap<String, Value>,
    pat: &IrPattern,
    val: &Value,
) -> Result<Option<std::collections::HashMap<String, Value>>> {
    use IrPattern as P;

    if let Some(binds) = match_pat_trivial(pat, val) {
        return Ok(Some(binds));
    }

    let val = force_and_auto_apply(g, val.clone())?;
    match (pat, &val) {
        (P::Literal(l), v) => {
            if match_pat_literal(l, v)? {
                Ok(Some(std::collections::HashMap::new()))
            } else {
                Ok(None)
            }
        }
        (P::Tuple(ps), Value::Tuple(vs)) => match_pat_tuple(g, env, ps, vs),
        (P::List(ps), v) => match_pat_list(g, env, ps, v),
        (P::Cons(hd, tl), v) => match_pat_cons(g, env, hd, tl, v),
        (P::Record(fs), Value::Record(vs)) => match_pat_record(g, env, fs, vs),
        (P::RecordLoose(fs, rest), Value::Record(vs)) => {
            match_pat_record_loose(g, env, fs, rest, vs)
        }
        (P::As(n, p), v) => {
            let Some(mut b) = match_pat(g, env, p, v)? else {
                return Ok(None);
            };
            b.insert(n.clone(), v.clone());
            Ok(Some(b))
        }
        (P::Or(a, b), v) => {
            if let Some(binds) = match_pat(g, env, a, v)? {
                Ok(Some(binds))
            } else {
                match_pat(g, env, b, v)
            }
        }
        (P::Constructor { name, args }, Value::Record(vs)) => {
            match_pat_constructor(g, env, name, args, vs)
        }
        (P::View(p, e), v) => match_pat_view(g, env, p, e, v),
        _ => Ok(None),
    }
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

    fn chars(s: &str) -> Value {
        string_to_char_list(s)
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

    #[test]
    fn eq_values_accepts_string_charlist_interop() {
        let g0 = Globals::from_module(&IrModule { items: vec![] });

        assert!(eq_values(&g0, Value::String("ab".to_string()), chars("ab")).unwrap());
        assert!(eq_values(&g0, chars("ab"), Value::String("ab".to_string())).unwrap());
        assert!(eq_values(&g0, Value::String(String::new()), Value::ListNil).unwrap());
    }
}
