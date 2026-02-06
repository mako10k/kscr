use crate::ast;
use crate::types::*;
use crate::Result;

use super::typeclass_dict_passing_common as common;

use std::collections::{HashMap, HashSet};

fn lookup_class_ty<'a>(class_tys: &'a HashMap<String, Ty>, class: &str) -> Option<&'a Ty> {
    if let Some(t) = class_tys.get(class) {
        return Some(t);
    }
    let unqualified = class.split('.').next_back().unwrap_or(class);
    if let Some(t) = class_tys.get(unqualified) {
        return Some(t);
    }

    // Unique suffix match (by last segment)
    let mut found: Option<&Ty> = None;
    for (k, v) in class_tys.iter() {
        let last = k.split('.').next_back().unwrap_or(k.as_str());
        if last == unqualified {
            if found.is_some() {
                return None;
            }
            found = Some(v);
        }
    }
    found
}

fn placeholder_local_ty(name: &str) -> Ty {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut h = DefaultHasher::new();
    name.hash(&mut h);
    let v = (h.finish() as u32) | 0x8000_0000;
    Ty::Var(v)
}

#[derive(Clone, Copy)]
struct RewriteCx<'a> {
    module_snapshot: &'a ast::Module,
    class_env: &'a ClassEnv,
    inferred: &'a HashMap<String, Scheme>,
    needs_dicts_global: &'a HashMap<String, Vec<String>>,
    needs_dicts_local: &'a HashMap<String, Vec<String>>,
    local_tys: &'a HashMap<String, Ty>,
    class_index: Option<&'a super::ClassEnvIndex>,
    inferred_unqual_index: Option<&'a HashMap<String, Option<String>>>,
    dicts_in_scope: &'a HashSet<String>,
    shadowed_in_scope: &'a HashSet<String>,
}

fn rewrite_expr_cx(cx: RewriteCx<'_>, expr: ast::Expr) -> Result<ast::Expr> {
    rewrite_expr_impl(cx, expr)
}

fn resolve_dict_arg_from_scope(
    span: ast::Span,
    class_env: &ClassEnv,
    dicts_in_scope: &HashSet<String>,
    class: &str,
) -> Option<ast::Expr> {
    use ast::{Expr, ExprKind};

    let param = common::dict_param_name(class);

    if std::env::var("KSCR_DEBUG_DICT").is_ok() {
        eprintln!(
            "[DICT] resolve_dict_arg_from_scope: looking for class '{}' (param: '{}')",
            class, param
        );
        eprintln!(
            "[DICT]   dicts_in_scope has {} entries",
            dicts_in_scope.len()
        );
        if dicts_in_scope.len() < 20 {
            for dict in dicts_in_scope {
                eprintln!("[DICT]     - {}", dict);
            }
        }
    }

    if dicts_in_scope.contains(&param) {
        if std::env::var("KSCR_DEBUG_DICT").is_ok() {
            eprintln!("[DICT]   -> Found direct param: {}", param);
        }
        return Some(Expr::new(span, ExprKind::Var(param)));
    }

    let result = common::derive_dict_from_scope(span, class_env, dicts_in_scope, class);
    if std::env::var("KSCR_DEBUG_DICT").is_ok() {
        if result.is_some() {
            eprintln!("[DICT]   -> Derived from scope");
        } else {
            eprintln!("[DICT]   -> NOT FOUND");
        }
    }
    result
}

fn infer_in_module_with_class_env_and_ground_locals(
    module: &ast::Module,
    class_env: &ClassEnv,
    inferred: &HashMap<String, Scheme>,
    local_tys: &HashMap<String, Ty>,
    expr: ast::Expr,
) -> Result<Ty> {
    let mut cx = InferCtx::default();
    let data_env = collect_data_env(module);
    let mut env = collect_ctor_env_with_class_env(&mut cx, module, class_env, None)?;

    // Rewrite-time inference often sees unqualified names (e.g. `try`, `throw`, `print`)
    // even when the inferred environment stores them qualified (e.g. `Prelude.try`).
    // Add a best-effort unqualified alias when the suffix is unique.
    let mut suffix_counts: HashMap<String, usize> = HashMap::new();
    for name in inferred.keys() {
        if name.contains('.') {
            let suffix = name.rsplit('.').next().unwrap_or(name.as_str());
            *suffix_counts.entry(suffix.to_string()).or_insert(0) += 1;
        }
    }

    for (name, scheme) in inferred {
        if !env.contains_key(name) {
            env.insert(
                name.clone(),
                EnvEntry {
                    scheme: scheme.clone(),
                    def_site: None,
                },
            );
        }

        if name.contains('.') {
            let suffix = name.rsplit('.').next().unwrap_or(name.as_str()).to_string();
            if suffix_counts.get(&suffix).copied().unwrap_or(0) == 1 && !env.contains_key(&suffix)
            {
                env.insert(
                    suffix,
                    EnvEntry {
                        scheme: scheme.clone(),
                        def_site: None,
                    },
                );
            }
        }
    }

    fn refresh_tyvars(cx: &mut InferCtx, ty: &Ty, m: &mut HashMap<u32, Ty>) -> Ty {
        match ty {
            Ty::Var(v) => m.entry(*v).or_insert_with(|| cx.fresh()).clone(),
            Ty::Con(c) => Ty::Con(c.clone()),
            Ty::List(t) => Ty::List(Box::new(refresh_tyvars(cx, t, m))),
            Ty::Tuple(ts) => Ty::Tuple(ts.iter().map(|t| refresh_tyvars(cx, t, m)).collect()),
            Ty::Record(fields) => Ty::Record(
                fields
                    .iter()
                    .map(|(k, t)| (k.clone(), refresh_tyvars(cx, t, m)))
                    .collect(),
            ),
            Ty::RecordOpen(fields, rest) => Ty::RecordOpen(
                fields
                    .iter()
                    .map(|(k, t)| (k.clone(), refresh_tyvars(cx, t, m)))
                    .collect(),
                Box::new(refresh_tyvars(cx, rest, m)),
            ),
            Ty::App { head, args } => Ty::App {
                head: Box::new(refresh_tyvars(cx, head, m)),
                args: args.iter().map(|t| refresh_tyvars(cx, t, m)).collect(),
            },
            Ty::Func(a, b) => Ty::Func(
                Box::new(refresh_tyvars(cx, a, m)),
                Box::new(refresh_tyvars(cx, b, m)),
            ),
        }
    }

    // Inject local types (including non-ground), refreshing tyvar ids to avoid collisions
    // across InferCtx instances created during rewrite.
    let mut refreshed: HashMap<u32, Ty> = HashMap::new();
    for (name, ty) in local_tys {
        if !env.contains_key(name) {
            let ty2 = refresh_tyvars(&mut cx, ty, &mut refreshed);
            env.insert(
                name.clone(),
                EnvEntry {
                    scheme: Scheme::mono(ty2),
                    def_site: None,
                },
            );
        }
    }


    let (s, cs, t) = infer_expr_in(&mut cx, &data_env, &Subst::new(), &env, expr)?;
    let _ = simplify_constraints(&data_env, class_env, apply_constraints(&s, cs))?;
    Ok(apply(&s, t))
}

fn infer_in_rewrite(cx: RewriteCx<'_>, expr: ast::Expr) -> Result<Ty> {
    if let ast::ExprKind::Var(name) = &expr.kind {
        if let Some(t) = cx.local_tys.get(name) {
            return Ok(t.clone());
        }
    }

    infer_in_module_with_class_env_and_ground_locals(
        cx.module_snapshot,
        cx.class_env,
        cx.inferred,
        cx.local_tys,
        expr,
    )
}

fn rewrite_var(cx: RewriteCx<'_>, span: ast::Span, name: String) -> Result<ast::Expr> {
    use ast::{Expr, ExprKind};

    let _module_snapshot = cx.module_snapshot;
    let class_env = cx.class_env;
    let _inferred = cx.inferred;
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

    let mut all_resolved = true;

    for class in classes {
        let mut picked: Option<ast::Expr> = None;

        // First try to resolve from scope (existing logic)
        if let Some(d) = resolve_dict_arg_from_scope(span, class_env, dicts_in_scope, class) {
            picked = Some(d);
        }

        // If not in scope, try to pick a concrete instance based on the variable's type
        if picked.is_none() {
            let var_expr = Expr::new(span, ExprKind::Var(name.clone()));
            if let Ok(var_ty) =
                infer_in_rewrite(cx, var_expr)
            {
                // Try to pick instance based on the inferred type
                if let Some(d) = common::pick_instance_dict_expr_from_scope(
                    span,
                    class_env,
                    dicts_in_scope,
                    class,
                    &var_ty,
                )? {
                    picked = Some(d);
                }
            }
        }

        if picked.is_none() {
            all_resolved = false;
            break;
        }
        dict_args.push(picked.unwrap());
    }

    // Only apply dicts if ALL of them were resolved
    // If some couldn't be resolved, leave the variable bare so Apply rewriting can handle it
    if all_resolved && !dict_args.is_empty() {
        Ok(Expr::new(
            span,
            ExprKind::Apply {
                func: Box::new(Expr::new(span, ExprKind::Var(name))),
                args: dict_args,
            },
        ))
    } else {
        Ok(Expr::new(span, ExprKind::Var(name)))
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
    let mut local_tys = cx.local_tys.clone();

    for p in &params {
        shadowed.insert(p.clone());

        // Shadow outer names.
        local_tys.remove(p);

        if p.starts_with("__dict_") {
            scope.insert(p.clone());
        } else {
            // Best-effort placeholder so infer_in_rewrite can see local binders.
            local_tys.insert(p.clone(), placeholder_local_ty(p));
        }
    }

    let inner_cx = RewriteCx {
        local_tys: &local_tys,
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

fn rewrite_lambda_with_expected(
    cx: RewriteCx<'_>,
    span: ast::Span,
    params: Vec<String>,
    body: ast::Expr,
    expected_ty: Option<&Ty>,
) -> Result<ast::Expr> {
    use ast::{Expr, ExprKind};

    let dicts_in_scope = cx.dicts_in_scope;
    let shadowed_in_scope = cx.shadowed_in_scope;

    // Extract expected parameter types from the call-site expected type.
    // This is crucial for cases like: `catch action (\e -> print e)` where
    // the lambda alone only yields `Show a => a -> IO Unit`, but the call-site
    // tells us `e : String`.
    let mut expected_param_tys: Vec<Ty> = Vec::new();
    let mut t = expected_ty.cloned();
    for _ in 0..params.len() {
        let Some(tt) = t else {
            break;
        };
        match tt {
            Ty::Func(dom, cod) => {
                expected_param_tys.push(*dom);
                t = Some(*cod);
            }
            _ => break,
        }
    }

    let mut scope = dicts_in_scope.clone();
    let mut shadowed = shadowed_in_scope.clone();
    let mut local_tys = cx.local_tys.clone();

    for (i, p) in params.iter().enumerate() {
        shadowed.insert(p.clone());

        // Always shadow outer names; then re-insert if we learned a ground param type.
        local_tys.remove(p);
        if let Some(t) = expected_param_tys.get(i) {
            if !p.starts_with("__dict_") {
                // Keep even non-ground expected types; they are often informative enough
                // to recover ground constructor argument types in nested pattern matches.
                local_tys.insert(p.clone(), t.clone());
            }
        } else if !p.starts_with("__dict_") {
            local_tys.insert(p.clone(), placeholder_local_ty(p));
        }

        if p.starts_with("__dict_") {
            scope.insert(p.clone());
        }
    }

    let inner_cx = RewriteCx {
        local_tys: &local_tys,
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
    args_snapshot_for_infer: &[ast::Expr],
    args: &mut Vec<ast::Expr>,
) -> Result<()> {
    let _module_snapshot = cx.module_snapshot;
    let class_env = cx.class_env;
    let _inferred = cx.inferred;
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
        if let Some(d) = resolve_dict_arg_from_scope(span, class_env, dicts_in_scope, class) {
            dict_args.push(d);
            continue;
        }

        if args_snapshot_for_infer.is_empty() {
            continue;
        }

        let mut picked: Option<ast::Expr> = None;

        if !shadowed_in_scope.contains(func_name) {
            if let Some(ci) = call_info {
                if let Some(target_ty) = lookup_class_ty(&ci.class_tys, class) {
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
            for a in args_snapshot_for_infer.iter() {
                let a_ty = match infer_in_rewrite(cx, a.clone()) {
                    Ok(t) => t,
                    Err(e) => {
                        if std::env::var("KSCR_DEBUG_DICT").is_ok() {
                            eprintln!("[DICT]   arg type inference failed: {e}");
                            eprintln!("[DICT]     arg: {a:?}");
                            if cx.local_tys.len() <= 20 {
                                for (k, v) in cx.local_tys {
                                    eprintln!("[DICT]     local_tys: {k}: {v}");
                                }
                            }
                        }
                        continue;
                    }
                };

                if std::env::var("KSCR_DEBUG_DICT").is_ok() {
                    eprintln!("[DICT]   arg: {a:?}");
                    eprintln!("[DICT]   arg inferred type: {a_ty}");
                }

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

            // Fallback: if rewrite-time inference cannot make any argument ground,
            // use the expected argument types computed during typecheck (CallInfo).
            // This is important for cases like `case ... of Left e -> print e`,
            // where `e` can stay a tyvar in rewrite-time inference even though the
            // original typecheck knows it is, say, `String`.
            if picked.is_none() && !shadowed_in_scope.contains(func_name) {
                if let Some(ci) = call_info {
                    for t in ci.expected_arg_tys.iter() {
                        if !ftv_ty(t).is_empty() {
                            continue;
                        }
                        if let Some(d) = common::pick_instance_dict_expr_from_scope(
                            span,
                            class_env,
                            dicts_in_scope,
                            class,
                            t,
                        )? {
                            if std::env::var("KSCR_DEBUG_DICT").is_ok() {
                                eprintln!("[DICT]   picked from expected_arg_tys: {t}");
                            }
                            picked = Some(d);
                            break;
                        }
                    }
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

    // IMPORTANT: compute call-site type info from the *pre-rewrite* AST.
    // Rewriting can inject dict vars that are not present in `inferred`, which would
    // break `infer_in_module_with_class_env` and prevent instance selection.
    let args_snapshot = args.clone();
    let call_info_pre: Option<CallInfo> = if let ExprKind::Var(callee) = &func.kind {
        if shadowed_in_scope.contains(callee) {
            None
        } else {
            common::call_info_for_call(
                module_snapshot,
                class_env,
                inferred,
                callee,
                &args_snapshot,
                cx.class_index,
                cx.inferred_unqual_index,
            )
        }
    } else {
        None
    };

    let mut callsite_ground_tys: Vec<Ty> = Vec::new();
    for a in &args_snapshot {
        if let Ok(t) =
            infer_in_rewrite(cx, a.clone())
        {
            if ftv_ty(&t).is_empty() {
                callsite_ground_tys.push(t);
            }
        }
    }

    let func = rewrite_expr_cx(cx, func)?;

    let mut rewritten_args: Vec<ast::Expr> = Vec::with_capacity(args.len());
    for (i, a) in args.into_iter().enumerate() {
        let expected_arg_ty = call_info_pre
            .as_ref()
            .and_then(|ci| ci.expected_arg_tys.get(i));

        let rewritten = match a.kind {
            ExprKind::Lambda { params, body } => {
                if std::env::var("KSCR_DEBUG_DICT").is_ok() {
                    if let Some(t) = expected_arg_ty {
                        eprintln!("[DICT] apply: lambda arg #{i} expected type: {t}");
                    } else {
                        eprintln!("[DICT] apply: lambda arg #{i} expected type: <none>");
                    }
                    eprintln!("[DICT] apply: callee pre-rewrite: {func:?}");
                }
                rewrite_lambda_with_expected(cx, a.span, params, *body, expected_arg_ty)?
            }
            _ => rewrite_expr_cx(cx, a)?,
        };
        rewritten_args.push(rewritten);
    }

    let mut args = rewritten_args;

    rewrite_apply_arg_vars(
        cx,
        span,
        call_info_pre.as_ref(),
        &callsite_ground_tys,
        &mut args,
    )?;

    if let ExprKind::Var(name) = &func.kind {
        rewrite_apply_func_var(
            cx,
            span,
            call_info_pre.as_ref(),
            name,
            &args_snapshot,
            &mut args,
        )?;
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



fn bind_pat_ground_tys(
    module: &ast::Module,
    class_env: &ClassEnv,
    pat: &ast::Pattern,
    ty: &Ty,
    out: &mut HashMap<String, Ty>,
) {
    use ast::PatternKind;

    match (&pat.kind, ty) {
        (PatternKind::Var(name), _) => {
            if ftv_ty(ty).is_empty() {
                out.insert(name.clone(), ty.clone());
            }
        }
        (PatternKind::As(name, p), _) => {
            if ftv_ty(ty).is_empty() {
                out.insert(name.clone(), ty.clone());
            }
            bind_pat_ground_tys(module, class_env, p, ty, out);
        }
        (PatternKind::Tuple(ps), Ty::Tuple(ts)) if ps.len() == ts.len() => {
            for (p, t) in ps.iter().zip(ts.iter()) {
                bind_pat_ground_tys(module, class_env, p, t, out);
            }
        }
        (PatternKind::List(ps), Ty::List(elem)) => {
            for p in ps {
                bind_pat_ground_tys(module, class_env, p, elem, out);
            }
        }
        (PatternKind::Cons(hd, tl), Ty::List(elem)) => {
            bind_pat_ground_tys(module, class_env, hd, elem, out);
            bind_pat_ground_tys(module, class_env, tl, ty, out);
        }
        (PatternKind::Record(fields), Ty::Record(ts)) => {
            let map = ts.iter().cloned().collect::<std::collections::HashMap<_, _>>();
            for (label, p) in fields {
                if let Some(t) = map.get(label) {
                    bind_pat_ground_tys(module, class_env, p, t, out);
                }
            }
        }
        (PatternKind::Record(fields), Ty::RecordOpen(ts, _rest)) => {
            let map = ts.iter().cloned().collect::<std::collections::HashMap<_, _>>();
            for (label, p) in fields {
                if let Some(t) = map.get(label) {
                    bind_pat_ground_tys(module, class_env, p, t, out);
                }
            }
        }
        (PatternKind::RecordLoose(fields, _rest_name), Ty::Record(ts)) => {
            let map = ts.iter().cloned().collect::<std::collections::HashMap<_, _>>();
            for (label, p) in fields {
                if let Some(t) = map.get(label) {
                    bind_pat_ground_tys(module, class_env, p, t, out);
                }
            }
        }
        (PatternKind::RecordLoose(fields, _rest_name), Ty::RecordOpen(ts, _rest)) => {
            let map = ts.iter().cloned().collect::<std::collections::HashMap<_, _>>();
            for (label, p) in fields {
                if let Some(t) = map.get(label) {
                    bind_pat_ground_tys(module, class_env, p, t, out);
                }
            }
        }
        (PatternKind::Constructor { name, args }, _) => {
            let mut infer_cx = InferCtx::default();
            let Ok(env) = collect_ctor_env_with_class_env(&mut infer_cx, module, class_env, None) else {
                return;
            };

            let qualified = name.qualified_text();
            let unqualified = name.local_name();
            let entry = env.get(&qualified).or_else(|| env.get(unqualified));
            let Some(entry) = entry else {
                return;
            };

            let ctor_ty = instantiate(&mut infer_cx, &entry.scheme);
            let mut arg_tys: Vec<Ty> = Vec::new();
            let mut result_ty = ctor_ty;
            while let Ty::Func(a, b) = result_ty {
                arg_tys.push(*a);
                result_ty = *b;
            }

            let Ok(subst) = unify(result_ty, ty.clone()) else {
                return;
            };

            for (p, t) in args.iter().zip(arg_tys.into_iter()) {
                let t = apply(&subst, t);
                bind_pat_ground_tys(module, class_env, p, &t, out);
            }
        }
        _ => {}
    }
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

fn with_extended_binding_scope<R>(
    cx: RewriteCx<'_>,
    bindings: Vec<ast::Binding>,
    f: impl FnOnce(RewriteCx<'_>, &HashMap<String, Vec<String>>, Vec<ast::Binding>) -> Result<R>,
) -> Result<R> {
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

    f(inner_cx, &local_needs, bindings)
}

fn rewrite_let(
    cx: RewriteCx<'_>,
    span: ast::Span,
    bindings: Vec<ast::Binding>,
    body: ast::Expr,
) -> Result<ast::Expr> {
    use ast::{Expr, ExprKind};
    use ast::PatternKind;

    with_extended_binding_scope(cx, bindings, |inner_cx, local_needs, bindings| {
        // Shadow all binders from the outer local type env first (let binds shadow outer names).
        let mut local_tys2 = inner_cx.local_tys.clone();
        for b in &bindings {
            let mut names = HashSet::new();
            pat_defined_names(&b.pat, &mut names);
            for n in names {
                local_tys2.remove(&n);
            }
        }

        let mut out_bindings: Vec<ast::Binding> = Vec::with_capacity(bindings.len());

        // Sequential: earlier binders are in scope for later binders (for our ground-type tracking).
        for b in bindings {
            let ast::Binding { doc: _, pat, expr, span: _ } = b;
            let span = expr.span;
            let expr_for_infer = expr.clone();

            let (rewritten_expr, inferred_t) = {
                let cx_bind = RewriteCx {
                    local_tys: &local_tys2,
                    ..inner_cx
                };

                let mut expr2 = expr;
                if let PatternKind::Var(name) = &pat.kind {
                    if let Some(classes) = local_needs.get(name) {
                        expr2 = common::add_dict_params_to_expr(expr2.span, expr2, classes);
                    }
                }

                let rewritten_expr = rewrite_expr_cx(cx_bind, expr2)?;
                let inferred_t = infer_in_rewrite(cx_bind, expr_for_infer);
                (rewritten_expr, inferred_t)
            };

            if let Ok(t) = inferred_t {
                bind_pat_ground_tys(
                    inner_cx.module_snapshot,
                    inner_cx.class_env,
                    &pat,
                    &t,
                    &mut local_tys2,
                );
            }

            out_bindings.push(ast::Binding {
                doc: None,
                pat,
                expr: rewritten_expr,
                span,
            });
        }

        let cx_body = RewriteCx {
            local_tys: &local_tys2,
            ..inner_cx
        };

        Ok(Expr::new(
            span,
            ExprKind::Let {
                bindings: out_bindings,
                body: Box::new(rewrite_expr_cx(cx_body, body)?),
            },
        ))
    })
}

fn rewrite_where(
    cx: RewriteCx<'_>,
    span: ast::Span,
    expr: ast::Expr,
    bindings: Vec<ast::Binding>,
) -> Result<ast::Expr> {
    use ast::{Expr, ExprKind};
    use ast::PatternKind;

    with_extended_binding_scope(cx, bindings, |inner_cx, local_needs, bindings| {
        // Shadow all binders from the outer local type env first (where binds shadow outer names).
        let mut local_tys2 = inner_cx.local_tys.clone();
        for b in &bindings {
            let mut names = HashSet::new();
            pat_defined_names(&b.pat, &mut names);
            for n in names {
                local_tys2.remove(&n);
            }
        }

        let mut out_bindings: Vec<ast::Binding> = Vec::with_capacity(bindings.len());

        // Sequential for our ground-type tracking: earlier binders may help later binder inference.
        for b in bindings {
            let ast::Binding { doc: _, pat, expr, span: _ } = b;
            let span = expr.span;
            let expr_for_infer = expr.clone();

            let (rewritten_expr, inferred_t) = {
                let cx_bind = RewriteCx {
                    local_tys: &local_tys2,
                    ..inner_cx
                };

                let mut expr2 = expr;
                if let PatternKind::Var(name) = &pat.kind {
                    if let Some(classes) = local_needs.get(name) {
                        expr2 = common::add_dict_params_to_expr(expr2.span, expr2, classes);
                    }
                }

                let rewritten_expr = rewrite_expr_cx(cx_bind, expr2)?;
                let inferred_t = infer_in_rewrite(cx_bind, expr_for_infer);
                (rewritten_expr, inferred_t)
            };

            if let Ok(t) = inferred_t {
                bind_pat_ground_tys(
                    inner_cx.module_snapshot,
                    inner_cx.class_env,
                    &pat,
                    &t,
                    &mut local_tys2,
                );
            }

            out_bindings.push(ast::Binding {
                doc: None,
                pat,
                expr: rewritten_expr,
                span,
            });
        }

        let cx_expr = RewriteCx {
            local_tys: &local_tys2,
            ..inner_cx
        };

        Ok(Expr::new(
            span,
            ExprKind::Where {
                expr: Box::new(rewrite_expr_cx(cx_expr, expr)?),
                bindings: out_bindings,
            },
        ))
    })
}

fn rewrite_do(cx: RewriteCx<'_>, span: ast::Span, stmts: Vec<ast::DoStmt>) -> Result<ast::Expr> {
    use ast::{Expr, ExprKind};

    let dicts_in_scope = cx.dicts_in_scope;
    let shadowed_in_scope = cx.shadowed_in_scope;

    let mut scope = dicts_in_scope.clone();
    let mut shadowed = shadowed_in_scope.clone();
    let mut local_tys2 = cx.local_tys.clone();

    let mut out: Vec<ast::DoStmt> = Vec::with_capacity(stmts.len());

    for s in stmts {
        match s {
            ast::DoStmt::Bind { pat, expr } => {
                let expr_for_infer = expr.clone();

                let (expr, inferred_inner) = {
                    let inner_cx = RewriteCx {
                        local_tys: &local_tys2,
                        dicts_in_scope: &scope,
                        shadowed_in_scope: &shadowed,
                        ..cx
                    };

                    let inferred_t = infer_in_rewrite(inner_cx, expr_for_infer);
                    if std::env::var("KSCR_DEBUG_DICT").is_ok() {
                        match &inferred_t {
                            Ok(t) => eprintln!("[DICT] do-bind inferred expr type: {t}"),
                            Err(e) => eprintln!("[DICT] do-bind inference failed: {e}"),
                        }
                    }

                    let inferred_inner: Option<Ty> = match &inferred_t {
                        Ok(Ty::App { args, .. }) => args.last().cloned(),
                        _ => None,
                    };

                    let expr = rewrite_expr_cx(inner_cx, expr)?;
                    (expr, inferred_inner)
                };

                // Shadow + (best-effort) record the bound value type (the last arg of `m a`).
                let mut names = HashSet::new();
                pat_defined_names(&pat, &mut names);
                for n in &names {
                    local_tys2.remove(n);
                }

                // Seed placeholders for binders so rewrite-time inference can make progress
                // even when the binder type isn't ground yet.
                for n in &names {
                    if !n.starts_with("__dict_") {
                        local_tys2.insert(n.clone(), placeholder_local_ty(n));
                    }
                }

                if let Some(inner) = &inferred_inner {
                    // For do-bind variables, it's useful to keep even non-ground types
                    // so later pattern matches can recover ground constructor arg types.
                    match &pat.kind {
                        ast::PatternKind::Var(name) => {
                            local_tys2.insert(name.clone(), inner.clone());
                        }
                        ast::PatternKind::As(name, _) => {
                            local_tys2.insert(name.clone(), inner.clone());
                        }
                        _ => {}
                    }

                    bind_pat_ground_tys(cx.module_snapshot, cx.class_env, &pat, inner, &mut local_tys2);
                }

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
                    local_tys: &local_tys2,
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

    // Best-effort ground scrutinee type, used to seed local var types for simple patterns.
    let scrut_ty = infer_in_rewrite(cx, expr.clone()).ok();
    if std::env::var("KSCR_DEBUG_DICT").is_ok() {
        match &scrut_ty {
            Some(t) => eprintln!("[DICT] case scrutinee inferred type: {t}"),
            None => eprintln!("[DICT] case scrutinee inference failed"),
        }
        eprintln!("[DICT] case scrutinee expr: {expr:?}");
    }

    let expr = Box::new(rewrite_expr_cx(cx, expr)?);

    let arms = arms
        .into_iter()
        .map(|a| {
            let mut scope = dicts_in_scope.clone();
            let mut shadowed = shadowed_in_scope.clone();
            let mut local_tys2 = cx.local_tys.clone();

            let mut names = HashSet::new();
            pat_defined_names(&a.pat, &mut names);
            for n in &names {
                shadowed.insert(n.clone());
                local_tys2.remove(n);
                if n.starts_with("__dict_") {
                    scope.insert(n.clone());
                } else {
                    // Seed placeholders so rewrite-time inference can see pattern binders.
                    local_tys2.insert(n.clone(), placeholder_local_ty(n));
                }
            }

            if let Some(t) = &scrut_ty {
                bind_pat_ground_tys(cx.module_snapshot, cx.class_env, &a.pat, t, &mut local_tys2);
            }

            if std::env::var("KSCR_DEBUG_DICT").is_ok() {
                let mut names_dbg = HashSet::new();
                pat_defined_names(&a.pat, &mut names_dbg);
                for n in names_dbg {
                    if let Some(t) = local_tys2.get(&n) {
                        eprintln!("[DICT] case binder: {n} : {t}");
                    } else {
                        eprintln!("[DICT] case binder: {n} : <none>");
                    }
                }
            }

            Ok(ast::CaseArm {
                pat: a.pat,
                guard: a
                    .guard
                    .map(|g| {
                        let inner_cx = RewriteCx {
                            local_tys: &local_tys2,
                            dicts_in_scope: &scope,
                            shadowed_in_scope: &shadowed,
                            ..cx
                        };
                        rewrite_expr_cx(inner_cx, g)
                    })
                    .transpose()?,
                body: {
                    let inner_cx = RewriteCx {
                        local_tys: &local_tys2,
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
    local_tys: &HashMap<String, Ty>,
    class_index: Option<&super::ClassEnvIndex>,
    inferred_unqual_index: Option<&HashMap<String, Option<String>>>,
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
        local_tys,
        class_index,
        inferred_unqual_index,
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
