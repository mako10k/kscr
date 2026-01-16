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
    rewrite_expr(
        cx.module_snapshot,
        cx.class_env,
        cx.inferred,
        cx.needs_dicts_global,
        cx.needs_dicts_local,
        cx.dicts_in_scope,
        cx.shadowed_in_scope,
        expr,
    )
}

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
    use ast::{Expr, ExprKind, PatternKind};

    let cx = RewriteCx {
        module_snapshot,
        class_env,
        inferred,
        needs_dicts_global,
        needs_dicts_local,
        dicts_in_scope,
        shadowed_in_scope,
    };

    let span = expr.span;

    use super::typeclass_dict_passing_common::CallInfo;

    Ok(match expr.kind {
        ExprKind::Var(name) => {
            let classes: Option<&Vec<String>> = if shadowed_in_scope.contains(&name) {
                needs_dicts_local.get(&name)
            } else {
                needs_dicts_local.get(&name).or_else(|| needs_dicts_global.get(&name))
            };

            if let Some(classes) = classes {
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
                    Expr::new(span, ExprKind::Var(name))
                } else {
                    Expr::new(
                        span,
                        ExprKind::Apply {
                            func: Box::new(Expr::new(span, ExprKind::Var(name))),
                            args: dict_args,
                        },
                    )
                }
            } else {
                Expr::new(span, ExprKind::Var(name))
            }
        }
        ExprKind::Lambda { params, body } => {
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
            Expr::new(
                span,
                ExprKind::Lambda {
                    params,
                    body: Box::new(rewrite_expr_cx(inner_cx, *body)?),
                },
            )
        }
        ExprKind::Apply { func, args } => {
            let func = rewrite_expr_cx(cx, *func)?;
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
                if let Ok(t) = infer_in_module_with_class_env(module_snapshot, class_env, inferred, a.clone()) {
                    if ftv_ty(&t).is_empty() {
                        callsite_ground_tys.push(t);
                    }
                }
            }

            for (i, a) in args.iter_mut().enumerate() {
                let ast::ExprKind::Var(name) = &a.kind else {
                    continue;
                };
                let classes: Option<&Vec<String>> = if shadowed_in_scope.contains(name) {
                    needs_dicts_local.get(name)
                } else {
                    needs_dicts_local.get(name).or_else(|| needs_dicts_global.get(name))
                };
                let Some(classes) = classes else {
                    continue;
                };

                let expected = call_info
                    .as_ref()
                    .and_then(|ci| ci.expected_arg_tys.get(i))
                    .cloned();

                let mut dict_args: Vec<ast::Expr> = Vec::new();
                for class in classes {
                    let mut picked: Option<ast::Expr> = None;

                    let dict_var = common::dict_param_name(class);
                    if dicts_in_scope.contains(&dict_var) {
                        picked = Some(Expr::new(span, ExprKind::Var(dict_var)));
                    }

                    if picked.is_none() {
                        if let Some(d) = common::derive_dict_from_scope(span, class_env, dicts_in_scope, class) {
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
                        for t in &callsite_ground_tys {
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

            if let ExprKind::Var(name) = &func.kind {
                let classes: Option<&Vec<String>> = if shadowed_in_scope.contains(name) {
                    needs_dicts_local.get(name)
                } else {
                    needs_dicts_local.get(name).or_else(|| needs_dicts_global.get(name))
                };

                if let Some(classes) = classes {
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

                        if !shadowed_in_scope.contains(name) {
                            if let Some(ci) = call_info.as_ref() {
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
                            for a in &args {
                                let Ok(a_ty) = infer_in_module_with_class_env(
                                    module_snapshot,
                                    class_env,
                                    inferred,
                                    a.clone(),
                                ) else {
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
                                        "cannot resolve dictionary for call to `{name}`: cannot infer instance head for {class} (type is not ground: {target_ty})"
                                    )));
                                }

                                let hint = "<unknown>".to_string();
                                return Err(crate::error::Error::msg(format!(
                                    "cannot resolve dictionary for call to `{name}`: no ground argument type available for {class} (e.g. {hint})"
                                )));
                            }
                        }

                        dict_args.push(picked.expect("picked must be Some"));
                    }

                    if !dict_args.is_empty() {
                        dict_args.extend(args);
                        args = dict_args;
                    }
                }
            }

            Expr::new(
                span,
                ExprKind::Apply {
                    func: Box::new(func),
                    args,
                },
            )
        }
        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => Expr::new(
            span,
            ExprKind::If {
                cond: Box::new(rewrite_expr_cx(cx, *cond)?),
                then_branch: Box::new(rewrite_expr_cx(cx, *then_branch)?),
                else_branch: Box::new(rewrite_expr_cx(cx, *else_branch)?),
            },
        ),
        ExprKind::Let { bindings, body } => {
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

                for b in &bindings {
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

            Expr::new(
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
                    body: Box::new(rewrite_expr_cx(inner_cx, *body)?),
                },
            )
        }
        ExprKind::Where { expr, bindings } => {
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

                for b in &bindings {
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

            Expr::new(
                span,
                ExprKind::Where {
                    expr: Box::new(rewrite_expr_cx(inner_cx, *expr)?),
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
            )
        }
        ExprKind::Annot { expr, ty } => Expr::new(
            span,
            ExprKind::Annot {
                expr: Box::new(rewrite_expr_cx(cx, *expr)?),
                ty,
            },
        ),
        ExprKind::Do(stmts) => {
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
            Expr::new(span, ExprKind::Do(out))
        }
        ExprKind::Case { expr, arms } => {
            let expr = Box::new(rewrite_expr_cx(cx, *expr)?);

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

            Expr::new(span, ExprKind::Case { expr, arms })
        }
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
                    .map(|(k, v)| {
                        Ok((
                            k,
                            rewrite_expr_cx(cx, v)?,
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?,
            ),
        ),
        other => Expr::new(span, other),
    })
}

// moved to `typeclass_dict_passing_common.rs`
