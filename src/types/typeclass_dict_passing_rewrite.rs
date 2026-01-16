use crate::ast;
use crate::types::*;
use crate::Result;

use super::typeclass_dict_passing_common as common;

use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy)]
struct RewriteCx<'a> {
    module_snapshot: &'a ast::Module,
    class_env: &'a ClassEnv,
    inferred: &'a HashMap<String, Scheme>,
    needs_dicts_global: &'a HashMap<String, Vec<String>>,
    needs_dicts_local: &'a HashMap<String, Vec<String>>,
    dicts_in_scope: &'a HashSet<String>,
    shadowed_in_scope: &'a HashSet<String>,
}

fn rewrite_expr_cx(cx: RewriteCx<'_>, expr: ast::Expr) -> Result<ast::Expr> {
    rewrite_expr_impl(cx, expr)
}

fn rewrite_var(cx: RewriteCx<'_>, span: ast::Span, name: String) -> Result<ast::Expr> {
    use ast::{Expr, ExprKind};

    let class_env = cx.class_env;
    let needs_dicts_global = cx.needs_dicts_global;
    let needs_dicts_local = cx.needs_dicts_local;
    let dicts_in_scope = cx.dicts_in_scope;
    let shadowed_in_scope = cx.shadowed_in_scope;

    let classes: Option<&Vec<String>> = if shadowed_in_scope.contains(&name) {
        needs_dicts_local.get(&name)
    } else {
        needs_dicts_local
            .get(&name)
            .or_else(|| needs_dicts_global.get(&name))
    };

    let Some(classes) = classes else {
        return Ok(Expr::new(span, ExprKind::Var(name)));
    };

    let mut dict_args: Vec<ast::Expr> = Vec::new();
    for class in classes {
        let param = common::dict_param_name(class);
        if dicts_in_scope.contains(&param) {
            dict_args.push(Expr::new(span, ExprKind::Var(param)));
            continue;
        }

        if let Some(d) = common::derive_dict_from_scope(span, class_env, dicts_in_scope, class) {
            dict_args.push(d);
            continue;
        }

        break;
    }

    if dict_args.is_empty() {
        Ok(Expr::new(span, ExprKind::Var(name)))
    } else {
        Ok(Expr::new(
            span,
            ExprKind::Apply {
                func: Box::new(Expr::new(span, ExprKind::Var(name))),
                args: dict_args,
            },
        ))
    }
}

fn rewrite_lambda(
    cx: RewriteCx<'_>,
    span: ast::Span,
    params: Vec<String>,
    body: ast::Expr,
) -> Result<ast::Expr> {
    use ast::{Expr, ExprKind};

    let dicts_in_scope = cx.dicts_in_scope;
    let shadowed_in_scope = cx.shadowed_in_scope;

    let mut scope = dicts_in_scope.clone();
    let mut shadowed = shadowed_in_scope.clone();
    for p in &params {
        shadowed.insert(p.clone());
        if p.starts_with("__dict_") {
            scope.insert(p.clone());
        }
    }
    let inner_cx = RewriteCx {
        dicts_in_scope: &scope,
        shadowed_in_scope: &shadowed,
        ..cx
    };

    Ok(Expr::new(
        span,
        ExprKind::Lambda {
            params,
            body: Box::new(rewrite_expr_cx(inner_cx, body)?),
        },
    ))
}

fn rewrite_apply_arg_vars(
    cx: RewriteCx<'_>,
    span: ast::Span,
    call_info: Option<&super::typeclass_dict_passing_common::CallInfo>,
    callsite_ground_tys: &[Ty],
    args: &mut [ast::Expr],
) -> Result<()> {
    use ast::{Expr, ExprKind};

    let class_env = cx.class_env;
    let needs_dicts_global = cx.needs_dicts_global;
    let needs_dicts_local = cx.needs_dicts_local;
    let dicts_in_scope = cx.dicts_in_scope;
    let shadowed_in_scope = cx.shadowed_in_scope;

    for (i, a) in args.iter_mut().enumerate() {
        let ast::ExprKind::Var(name) = &a.kind else {
            continue;
        };
        let classes: Option<&Vec<String>> = if shadowed_in_scope.contains(name) {
            needs_dicts_local.get(name)
        } else {
            needs_dicts_local
                .get(name)
                .or_else(|| needs_dicts_global.get(name))
        };
        let Some(classes) = classes else {
            continue;
        };

        let expected = call_info.and_then(|ci| ci.expected_arg_tys.get(i)).cloned();

        let mut dict_args: Vec<ast::Expr> = Vec::new();
        for class in classes {
            let mut picked: Option<ast::Expr> = None;

            let dict_var = common::dict_param_name(class);
            if dicts_in_scope.contains(&dict_var) {
                picked = Some(Expr::new(span, ExprKind::Var(dict_var)));
            }

            if picked.is_none() {
                if let Some(d) =
                    common::derive_dict_from_scope(span, class_env, dicts_in_scope, class)
                {
                    picked = Some(d);
                }
            }

            if picked.is_none() {
                if let Some(expected) = expected.as_ref() {
                    let target_ty = match expected {
                        Ty::Func(dom, _) => dom.as_ref().clone(),
                        other => other.clone(),
                    };
                    if let Some(d) = common::pick_instance_dict_expr_from_scope(
                        span,
                        class_env,
                        dicts_in_scope,
                        class,
                        &target_ty,
                    )? {
                        picked = Some(d);
                    }
                }
            }

            if picked.is_none() {
                for t in callsite_ground_tys {
                    if let Some(d) = common::pick_instance_dict_expr_from_scope(
                        span,
                        class_env,
                        dicts_in_scope,
                        class,
                        t,
                    )? {
                        picked = Some(d);
                        break;
                    }
                }
            }

            let Some(picked) = picked else {
                break;
            };
            dict_args.push(picked);
        }

        if dict_args.is_empty() {
            continue;
        }

        *a = Expr::new(
            span,
            ExprKind::Apply {
                func: Box::new(Expr::new(span, ExprKind::Var(name.clone()))),
                args: dict_args,
            },
        );
    }

    Ok(())
}

fn rewrite_apply_func_var(
    cx: RewriteCx<'_>,
    span: ast::Span,
    call_info: Option<&super::typeclass_dict_passing_common::CallInfo>,
    func_name: &str,
    args: &mut Vec<ast::Expr>,
) -> Result<()> {
    use ast::{Expr, ExprKind};

    let module_snapshot = cx.module_snapshot;
    let class_env = cx.class_env;
    let inferred = cx.inferred;
    let needs_dicts_global = cx.needs_dicts_global;
    let needs_dicts_local = cx.needs_dicts_local;
    let dicts_in_scope = cx.dicts_in_scope;
    let shadowed_in_scope = cx.shadowed_in_scope;

    let classes: Option<&Vec<String>> = if shadowed_in_scope.contains(func_name) {
        needs_dicts_local.get(func_name)
    } else {
        needs_dicts_local
            .get(func_name)
            .or_else(|| needs_dicts_global.get(func_name))
    };

    let Some(classes) = classes else {
        return Ok(());
    };

    let mut dict_args: Vec<ast::Expr> = Vec::new();
    for class in classes {
        let param = common::dict_param_name(class);
        if dicts_in_scope.contains(&param) {
            dict_args.push(Expr::new(span, ExprKind::Var(param)));
            continue;
        }

        if let Some(d) = common::derive_dict_from_scope(span, class_env, dicts_in_scope, class) {
            dict_args.push(d);
            continue;
        }

        if args.is_empty() {
            continue;
        }

        let mut picked: Option<ast::Expr> = None;

        if !shadowed_in_scope.contains(func_name) {
            if let Some(ci) = call_info {
                if let Some(target_ty) = ci.class_tys.get(class) {
                    picked = common::pick_instance_dict_expr_from_scope(
                        span,
                        class_env,
                        dicts_in_scope,
                        class,
                        target_ty,
                    )?;
                }
            }
        }

        if picked.is_none() {
            let mut first_non_ground: Option<Ty> = None;
            for a in args.iter() {
                let Ok(a_ty) =
                    infer_in_module_with_class_env(module_snapshot, class_env, inferred, a.clone())
                else {
                    continue;
                };

                if !ftv_ty(&a_ty).is_empty() {
                    if first_non_ground.is_none() {
                        first_non_ground = Some(a_ty);
                    }
                    continue;
                }

                if let Some(d) = common::pick_instance_dict_expr_from_scope(
                    span,
                    class_env,
                    dicts_in_scope,
                    class,
                    &a_ty,
                )? {
                    picked = Some(d);
                    break;
                }
            }

            if picked.is_none() {
                if let Some(target_ty) = first_non_ground {
                    return Err(crate::error::Error::msg(format!(
                        "cannot resolve dictionary for call to `{func_name}`: cannot infer instance head for {class} (type is not ground: {target_ty})"
                    )));
                }

                let hint = "<unknown>".to_string();
                return Err(crate::error::Error::msg(format!(
                    "cannot resolve dictionary for call to `{func_name}`: no ground argument type available for {class} (e.g. {hint})"
                )));
            }
        }

        dict_args.push(picked.expect("picked must be Some"));
    }

    if !dict_args.is_empty() {
        dict_args.append(args);
        *args = dict_args;
    }

    Ok(())
}

fn rewrite_apply(
    cx: RewriteCx<'_>,
    span: ast::Span,
    func: ast::Expr,
    args: Vec<ast::Expr>,
) -> Result<ast::Expr> {
    use ast::{Expr, ExprKind};

    let module_snapshot = cx.module_snapshot;
    let class_env = cx.class_env;
    let inferred = cx.inferred;
    let shadowed_in_scope = cx.shadowed_in_scope;

    use super::typeclass_dict_passing_common::CallInfo;

    let func = rewrite_expr_cx(cx, func)?;
    let mut args: Vec<_> = args
        .into_iter()
        .map(|a| rewrite_expr_cx(cx, a))
        .collect::<Result<Vec<_>>>()?;

    let call_info: Option<CallInfo> = if let ExprKind::Var(callee) = &func.kind {
        if shadowed_in_scope.contains(callee) {
            None
        } else {
            common::call_info_for_call(module_snapshot, class_env, inferred, callee, &args)
        }
    } else {
        None
    };

    let mut callsite_ground_tys: Vec<Ty> = Vec::new();
    for a in &args {
        if let Ok(t) =
            infer_in_module_with_class_env(module_snapshot, class_env, inferred, a.clone())
        {
            if ftv_ty(&t).is_empty() {
                callsite_ground_tys.push(t);
            }
        }
    }

    rewrite_apply_arg_vars(
        cx,
        span,
        call_info.as_ref(),
        &callsite_ground_tys,
        &mut args,
    )?;

    if let ExprKind::Var(name) = &func.kind {
        rewrite_apply_func_var(cx, span, call_info.as_ref(), name, &mut args)?;
    }

    Ok(Expr::new(
        span,
        ExprKind::Apply {
            func: Box::new(func),
            args,
        },
    ))
}

fn rewrite_if(
    cx: RewriteCx<'_>,
    span: ast::Span,
    cond: ast::Expr,
    then_branch: ast::Expr,
    else_branch: ast::Expr,
) -> Result<ast::Expr> {
    use ast::{Expr, ExprKind};

    Ok(Expr::new(
        span,
        ExprKind::If {
            cond: Box::new(rewrite_expr_cx(cx, cond)?),
            then_branch: Box::new(rewrite_expr_cx(cx, then_branch)?),
            else_branch: Box::new(rewrite_expr_cx(cx, else_branch)?),
        },
    ))
}

fn compute_local_needs(
    class_env: &ClassEnv,
    needs_dicts_global: &HashMap<String, Vec<String>>,
    needs_dicts_local: &HashMap<String, Vec<String>>,
    bindings: &[ast::Binding],
) -> HashMap<String, Vec<String>> {
    use ast::PatternKind;

    let mut local_needs: HashMap<String, Vec<String>> = HashMap::new();
    loop {
        let mut changed = false;

        let mut lookup: HashMap<String, Vec<String>> = HashMap::new();
        for (k, v) in needs_dicts_global {
            lookup.insert(k.clone(), v.clone());
        }
        for (k, v) in needs_dicts_local {
            lookup.insert(k.clone(), v.clone());
        }
        for (k, v) in &local_needs {
            lookup.insert(k.clone(), v.clone());
        }

        for b in bindings {
            let PatternKind::Var(name) = &b.pat.kind else {
                continue;
            };
            let mut req: HashSet<String> = HashSet::new();
            common::required_classes_in_expr(&b.expr, class_env, &lookup, &mut req);
            let mut classes: Vec<String> = req.into_iter().collect();
            classes.sort();
            if classes.is_empty() {
                continue;
            }
            match local_needs.get(name) {
                Some(existing) if *existing == classes => {}
                _ => {
                    local_needs.insert(name.clone(), classes);
                    changed = true;
                }
            }
        }

        if !changed {
            break;
        }
    }

    local_needs
}

fn rewrite_let(
    cx: RewriteCx<'_>,
    span: ast::Span,
    bindings: Vec<ast::Binding>,
    body: ast::Expr,
) -> Result<ast::Expr> {
    use ast::{Expr, ExprKind, PatternKind};

    let class_env = cx.class_env;
    let needs_dicts_global = cx.needs_dicts_global;
    let needs_dicts_local = cx.needs_dicts_local;
    let dicts_in_scope = cx.dicts_in_scope;
    let shadowed_in_scope = cx.shadowed_in_scope;

    let mut scope = dicts_in_scope.clone();
    let mut shadowed = shadowed_in_scope.clone();
    for b in &bindings {
        let mut names = HashSet::new();
        pat_defined_names(&b.pat, &mut names);
        for n in names {
            shadowed.insert(n.clone());
            if n.starts_with("__dict_") {
                scope.insert(n);
            }
        }
    }

    let local_needs =
        compute_local_needs(class_env, needs_dicts_global, needs_dicts_local, &bindings);

    let mut local2: HashMap<String, Vec<String>> = needs_dicts_local.clone();
    for (k, v) in &local_needs {
        local2.insert(k.clone(), v.clone());
    }

    let inner_cx = RewriteCx {
        needs_dicts_local: &local2,
        dicts_in_scope: &scope,
        shadowed_in_scope: &shadowed,
        ..cx
    };

    Ok(Expr::new(
        span,
        ExprKind::Let {
            bindings: bindings
                .into_iter()
                .map(|b| {
                    let mut expr = b.expr;
                    if let PatternKind::Var(name) = &b.pat.kind {
                        if let Some(classes) = local_needs.get(name) {
                            expr = common::add_dict_params_to_expr(expr.span, expr, classes);
                        }
                    }
                    Ok(ast::Binding {
                        pat: b.pat,
                        expr: rewrite_expr_cx(inner_cx, expr)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            body: Box::new(rewrite_expr_cx(inner_cx, body)?),
        },
    ))
}

fn rewrite_where(
    cx: RewriteCx<'_>,
    span: ast::Span,
    expr: ast::Expr,
    bindings: Vec<ast::Binding>,
) -> Result<ast::Expr> {
    use ast::{Expr, ExprKind, PatternKind};

    let class_env = cx.class_env;
    let needs_dicts_global = cx.needs_dicts_global;
    let needs_dicts_local = cx.needs_dicts_local;
    let dicts_in_scope = cx.dicts_in_scope;
    let shadowed_in_scope = cx.shadowed_in_scope;

    let mut scope = dicts_in_scope.clone();
    let mut shadowed = shadowed_in_scope.clone();
    for b in &bindings {
        let mut names = HashSet::new();
        pat_defined_names(&b.pat, &mut names);
        for n in names {
            shadowed.insert(n.clone());
            if n.starts_with("__dict_") {
                scope.insert(n);
            }
        }
    }

    let local_needs =
        compute_local_needs(class_env, needs_dicts_global, needs_dicts_local, &bindings);

    let mut local2: HashMap<String, Vec<String>> = needs_dicts_local.clone();
    for (k, v) in &local_needs {
        local2.insert(k.clone(), v.clone());
    }

    let inner_cx = RewriteCx {
        needs_dicts_local: &local2,
        dicts_in_scope: &scope,
        shadowed_in_scope: &shadowed,
        ..cx
    };

    Ok(Expr::new(
        span,
        ExprKind::Where {
            expr: Box::new(rewrite_expr_cx(inner_cx, expr)?),
            bindings: bindings
                .into_iter()
                .map(|b| {
                    let mut expr = b.expr;
                    if let PatternKind::Var(name) = &b.pat.kind {
                        if let Some(classes) = local_needs.get(name) {
                            expr = common::add_dict_params_to_expr(expr.span, expr, classes);
                        }
                    }
                    Ok(ast::Binding {
                        pat: b.pat,
                        expr: rewrite_expr_cx(inner_cx, expr)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        },
    ))
}

fn rewrite_do(cx: RewriteCx<'_>, span: ast::Span, stmts: Vec<ast::DoStmt>) -> Result<ast::Expr> {
    use ast::{Expr, ExprKind};

    let dicts_in_scope = cx.dicts_in_scope;
    let shadowed_in_scope = cx.shadowed_in_scope;

    let mut scope = dicts_in_scope.clone();
    let mut shadowed = shadowed_in_scope.clone();
    let mut out: Vec<ast::DoStmt> = Vec::with_capacity(stmts.len());

    for s in stmts {
        match s {
            ast::DoStmt::Bind { pat, expr } => {
                let inner_cx = RewriteCx {
                    dicts_in_scope: &scope,
                    shadowed_in_scope: &shadowed,
                    ..cx
                };
                let expr = rewrite_expr_cx(inner_cx, expr)?;

                let mut names = HashSet::new();
                pat_defined_names(&pat, &mut names);
                for n in names {
                    shadowed.insert(n.clone());
                    if n.starts_with("__dict_") {
                        scope.insert(n);
                    }
                }

                out.push(ast::DoStmt::Bind { pat, expr });
            }
            ast::DoStmt::Expr(e) => {
                let inner_cx = RewriteCx {
                    dicts_in_scope: &scope,
                    shadowed_in_scope: &shadowed,
                    ..cx
                };
                out.push(ast::DoStmt::Expr(rewrite_expr_cx(inner_cx, e)?));
            }
        }
    }

    Ok(Expr::new(span, ExprKind::Do(out)))
}

fn rewrite_case(
    cx: RewriteCx<'_>,
    span: ast::Span,
    expr: ast::Expr,
    arms: Vec<ast::CaseArm>,
) -> Result<ast::Expr> {
    use ast::{Expr, ExprKind};

    let dicts_in_scope = cx.dicts_in_scope;
    let shadowed_in_scope = cx.shadowed_in_scope;

    let expr = Box::new(rewrite_expr_cx(cx, expr)?);

    let arms = arms
        .into_iter()
        .map(|a| {
            let mut scope = dicts_in_scope.clone();
            let mut shadowed = shadowed_in_scope.clone();
            let mut names = HashSet::new();
            pat_defined_names(&a.pat, &mut names);
            for n in names {
                shadowed.insert(n.clone());
                if n.starts_with("__dict_") {
                    scope.insert(n);
                }
            }

            Ok(ast::CaseArm {
                pat: a.pat,
                guard: a
                    .guard
                    .map(|g| {
                        let inner_cx = RewriteCx {
                            dicts_in_scope: &scope,
                            shadowed_in_scope: &shadowed,
                            ..cx
                        };
                        rewrite_expr_cx(inner_cx, g)
                    })
                    .transpose()?,
                body: {
                    let inner_cx = RewriteCx {
                        dicts_in_scope: &scope,
                        shadowed_in_scope: &shadowed,
                        ..cx
                    };
                    rewrite_expr_cx(inner_cx, a.body)?
                },
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(Expr::new(span, ExprKind::Case { expr, arms }))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn rewrite_expr(
    module_snapshot: &ast::Module,
    class_env: &ClassEnv,
    inferred: &HashMap<String, Scheme>,
    needs_dicts_global: &HashMap<String, Vec<String>>,
    needs_dicts_local: &HashMap<String, Vec<String>>,
    dicts_in_scope: &HashSet<String>,
    shadowed_in_scope: &HashSet<String>,
    expr: ast::Expr,
) -> Result<ast::Expr> {
    let cx = RewriteCx {
        module_snapshot,
        class_env,
        inferred,
        needs_dicts_global,
        needs_dicts_local,
        dicts_in_scope,
        shadowed_in_scope,
    };

    rewrite_expr_impl(cx, expr)
}

fn rewrite_expr_impl(cx: RewriteCx<'_>, expr: ast::Expr) -> Result<ast::Expr> {
    use ast::{Expr, ExprKind};

    let _module_snapshot = cx.module_snapshot;
    let _class_env = cx.class_env;
    let _inferred = cx.inferred;
    let _needs_dicts_global = cx.needs_dicts_global;
    let _needs_dicts_local = cx.needs_dicts_local;
    let _dicts_in_scope = cx.dicts_in_scope;
    let _shadowed_in_scope = cx.shadowed_in_scope;

    let span = expr.span;

    Ok(match expr.kind {
        ExprKind::Var(name) => rewrite_var(cx, span, name)?,
        ExprKind::Lambda { params, body } => rewrite_lambda(cx, span, params, *body)?,
        ExprKind::Apply { func, args } => rewrite_apply(cx, span, *func, args)?,
        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => rewrite_if(cx, span, *cond, *then_branch, *else_branch)?,
        ExprKind::Let { bindings, body } => rewrite_let(cx, span, bindings, *body)?,
        ExprKind::Where { expr, bindings } => rewrite_where(cx, span, *expr, bindings)?,
        ExprKind::Annot { expr, ty } => Expr::new(
            span,
            ExprKind::Annot {
                expr: Box::new(rewrite_expr_cx(cx, *expr)?),
                ty,
            },
        ),
        ExprKind::Do(stmts) => rewrite_do(cx, span, stmts)?,
        ExprKind::Case { expr, arms } => rewrite_case(cx, span, *expr, arms)?,
        ExprKind::Cons { head, tail } => Expr::new(
            span,
            ExprKind::Cons {
                head: Box::new(rewrite_expr_cx(cx, *head)?),
                tail: Box::new(rewrite_expr_cx(cx, *tail)?),
            },
        ),
        ExprKind::List(es) => Expr::new(
            span,
            ExprKind::List(
                es.into_iter()
                    .map(|e| rewrite_expr_cx(cx, e))
                    .collect::<Result<Vec<_>>>()?,
            ),
        ),
        ExprKind::Tuple(es) => Expr::new(
            span,
            ExprKind::Tuple(
                es.into_iter()
                    .map(|e| rewrite_expr_cx(cx, e))
                    .collect::<Result<Vec<_>>>()?,
            ),
        ),
        ExprKind::Record(fields) => Expr::new(
            span,
            ExprKind::Record(
                fields
                    .into_iter()
                    .map(|(k, v)| Ok((k, rewrite_expr_cx(cx, v)?)))
                    .collect::<Result<Vec<_>>>()?,
            ),
        ),
        other => Expr::new(span, other),
    })
}

// moved to `typeclass_dict_passing_common.rs`
