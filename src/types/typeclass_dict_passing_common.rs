use crate::ast;
use crate::error::Error;
use crate::types::*;
use crate::Result;

use std::collections::{HashMap, HashSet};

fn poly_instance_origin(dict_name: &str) -> String {
    dict_name
        .rsplit_once('.')
        .map(|(module, _)| module.to_string())
        .unwrap_or_else(|| "<current module>".to_string())
}

fn poly_instance_overlap_details(candidates: &[&PolyInstance]) -> (String, String) {
    let mut candidate_notes: Vec<String> = candidates
        .iter()
        .map(|pi| format!("{} [head: {}]", pi.dict_name, pi.head_pat))
        .collect();
    candidate_notes.sort();
    candidate_notes.dedup();

    let mut origins: Vec<String> = candidates
        .iter()
        .map(|pi| poly_instance_origin(&pi.dict_name))
        .collect();
    origins.sort();
    origins.dedup();

    (candidate_notes.join(", "), origins.join(", "))
}

pub(super) fn rewrite_class_dict_passing_in_module(
    module: &mut ast::Module,
    class_env: &ClassEnv,
    inferred: &HashMap<String, Scheme>,
) -> Result<()> {
    use ast::PatternKind;

    fn is_injected_import_forwarder(b: &ast::Binding) -> bool {
        let injected =
            b.span.start == 0 && b.span.end == 0 && b.expr.span.start == 0 && b.expr.span.end == 0;
        let rhs_is_qual_var = matches!(&b.expr.kind, ast::ExprKind::Var(q) if q.contains('.'));
        injected && rhs_is_qual_var
    }

    // name -> classes (stable order) that require an explicit dictionary arg.
    let mut needs_dicts: HashMap<String, Vec<String>> = HashMap::new();
    for (name, scheme) in inferred {
        if name.starts_with("__dict_") || name.starts_with("__inst_") {
            continue;
        }

        let mut classes: Vec<String> = scheme
            .constraints
            .iter()
            .filter_map(|c| match c {
                Constraint::Class { class, .. } => Some(class.name.clone()),
                // Built-in constraints still require dictionary passing.
                Constraint::Show(_) => Some("Show".to_string()),
                Constraint::Eq(_) => Some("Eq".to_string()),
                Constraint::ShowRow(_) => Some("ShowRow".to_string()),
                Constraint::EqRow(_) => Some("EqRow".to_string()),
                _ => None,
            })
            .collect();
        classes.sort();
        classes.dedup();
        if !classes.is_empty() {
            needs_dicts.insert(name.clone(), classes);
        }
    }

    let snapshot = module.clone();

    // Build class method scheme index once per module rewrite.
    // This avoids constructing it repeatedly in call-site queries.
    let class_index: Option<super::ClassEnvIndex> = {
        let mut cx_for_index = InferCtx::default();
        super::build_class_method_scheme_index(&mut cx_for_index, class_env).ok()
    };

    // Build unqualified callee lookup once per module rewrite.
    // This avoids repeatedly scanning `inferred` for unique suffix matches.
    // Map: unqualified_name -> Some(qualified_name) if unique, None if ambiguous.
    let inferred_unqual_index: HashMap<String, Option<String>> = {
        let mut idx: HashMap<String, Option<String>> = HashMap::new();
        for k in inferred.keys() {
            if !k.contains('.') {
                continue;
            }
            let last = k.split('.').next_back().unwrap_or(k.as_str()).to_string();
            match idx.get_mut(&last) {
                None => {
                    idx.insert(last, Some(k.clone()));
                }
                Some(slot @ Some(_)) => {
                    *slot = None;
                }
                Some(None) => {}
            }
        }
        idx
    };

    // 0) Ensure ground instance dictionaries referenced by name actually exist as bindings.
    // Some constraints are discharged at typecheck time (e.g. `Monad IO`, `Ring Integer`) and
    // later rewrites may refer to their concrete dictionary names (e.g. `__dict_Monad_IO`).
    // When typechecking a flattened module (imports resolved), those dictionaries may live in
    // stdlib and not be present as values in the current module, so we inject forwarder
    // bindings that point to the imported dictionary variables.
    // NOTE: stdlib instance dictionaries (`__dict_*`) are now unqualified-forwarded in
    // `typecheck_with_stdlib_class_env` for the `typecheck_file()` path.

    // 1) Add dictionary params to constrained top-level bindings.
    module.items = module
        .items
        .drain(..)
        .map(|it| {
            let it = match it {
                ast::Item::Binding(b) => {
                    if let PatternKind::Var(name) = &b.pat.kind {
                        // Skip adding dict params to injected import-forwarders like `print = Prelude.print`.
                        // These forwarders are already aliases to dictionary-taking functions; adding dict params
                        // here can accidentally duplicate dictionary arguments (e.g. `Prelude.print d d`).
                        if is_injected_import_forwarder(&b) {
                            ast::Item::Binding(b)
                        } else if let Some(classes) = needs_dicts.get(name) {
                            ast::Item::Binding(ast::Binding {
                                doc: None,
                                pat: b.pat,
                                expr: add_dict_params_to_expr(b.expr.span, b.expr, classes),
                                span: b.span,
                            })
                        } else {
                            ast::Item::Binding(b)
                        }
                    } else {
                        ast::Item::Binding(b)
                    }
                }
                other => other,
            };
            Ok(it)
        })
        .collect::<Result<Vec<_>>>()?;

    // 2) Rewrite call sites to supply dictionaries.
    // Populate ground_dicts with all available instance dictionaries from ClassEnv.
    // This allows method references like `+` in nested contexts to resolve their dictionaries.
    // Strip module qualification to get unqualified dict names that match actual bindings in the module.
    let ground_dicts: HashSet<String> = class_env
        .instances
        .values()
        .filter_map(|dict_name| {
            // Extract unqualified name (last component after the last dot)
            dict_name.split('.').next_back().map(|s| s.to_string())
        })
        .collect();

    if std::env::var("KSCR_DEBUG_DICT").is_ok() {
        eprintln!(
            "[DICT] ground_dicts populated with {} entries:",
            ground_dicts.len()
        );
        for dict in &ground_dicts {
            eprintln!("[DICT]   - {}", dict);
        }
    }

    let empty_shadowed: HashSet<String> = HashSet::new();
    let empty_local: HashMap<String, Vec<String>> = HashMap::new();
    let empty_local_tys: HashMap<String, Ty> = HashMap::new();
    module.items = module
        .items
        .drain(..)
        .map(|it| {
            Ok(match it {
                ast::Item::Binding(mut b) => {
                    let expected_root_ty: Option<&Ty> = match &b.pat.kind {
                        ast::PatternKind::Var(name) => {
                            // Only seed expected types when we did not add dict params.
                            // Otherwise, scheme.ty would be misaligned with the rewritten lambda params.
                            if needs_dicts.contains_key(name) {
                                None
                            } else {
                                inferred.get(name).map(|s| &s.ty)
                            }
                        }
                        _ => None,
                    };

                    b.expr = super::typeclass_dict_passing_rewrite::rewrite_expr(
                        &snapshot,
                        class_env,
                        inferred,
                        &needs_dicts,
                        &empty_local,
                        &empty_local_tys,
                        class_index.as_ref(),
                        Some(&inferred_unqual_index),
                        expected_root_ty,
                        &ground_dicts,
                        &empty_shadowed,
                        b.expr,
                    )?;
                    ast::Item::Binding(b)
                }
                other => other,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(())
}

pub(super) fn dict_param_name(class: &str) -> String {
    // Use unqualified class name for dict parameters to avoid dots in parameter names.
    // For example, class "Prelude.Num.Num" becomes "__dict_Num" not "__dict_Prelude.Num.Num".
    let unqualified = class.split('.').next_back().unwrap_or(class);
    let result = format!("__dict_{unqualified}");
    if std::env::var("KSCR_DEBUG_DICT_PARAM").is_ok() {
        eprintln!("[DICT_PARAM] class='{}' -> param='{}'", class, result);
    }
    result
}

pub(super) fn super_field_name(class: &str) -> String {
    format!("__super_{}", mangle_ident(class))
}

pub(super) fn find_super_path(class_env: &ClassEnv, from: &str, to: &str) -> Option<Vec<String>> {
    use std::collections::{HashMap, VecDeque};

    fn find_unique_class_id_by_name(class_env: &ClassEnv, name: &str) -> Option<ast::ClassId> {
        // Prefer exact match; if `name` is unqualified, allow a unique suffix match.
        let mut found: Option<ast::ClassId> = None;

        // 1) Exact match (qualified or unqualified)
        for id in class_env.class_params.keys() {
            if id.name == name {
                if found.is_some() {
                    return None;
                }
                found = Some(id.clone());
            }
        }
        if found.is_some() {
            return found;
        }

        // 2) If unqualified, try unique suffix match by last segment.
        if !name.contains('.') {
            let mut suffix_found: Option<ast::ClassId> = None;
            for id in class_env.class_params.keys() {
                let last = id.name.split('.').next_back().unwrap_or(id.name.as_str());
                if last == name {
                    if suffix_found.is_some() {
                        return None;
                    }
                    suffix_found = Some(id.clone());
                }
            }
            return suffix_found;
        }

        None
    }

    if from == to {
        return None;
    }

    let from_id = find_unique_class_id_by_name(class_env, from)?;

    let mut q: VecDeque<ast::ClassId> = VecDeque::new();
    let mut prev: HashMap<ast::ClassId, ast::ClassId> = HashMap::new();
    q.push_back(from_id.clone());

    while let Some(c) = q.pop_front() {
        let Some(supers) = class_env.class_supers.get(&c) else {
            continue;
        };
        for p in supers {
            let ast::Predicate::Class { class: sup, .. } = p else {
                continue;
            };

            if prev.contains_key(sup) {
                continue;
            }
            prev.insert(sup.clone(), c.clone());
            if sup.name == to {
                // Reconstruct path: from -> ... -> to
                let mut path: Vec<String> = Vec::new();
                let mut cur = sup.clone();
                while cur != from_id {
                    path.push(cur.name.clone());
                    cur = prev.get(&cur)?.clone();
                }
                path.reverse();
                return Some(path);
            }
            q.push_back(sup.clone());
        }
    }

    None
}

pub(super) fn project_dict_along_path(
    span: ast::Span,
    mut base: ast::Expr,
    path: &[String],
) -> ast::Expr {
    for sup in path {
        let get = ast::Expr::new(span, ast::ExprKind::Var("__recordGet".to_string()));
        base = ast::Expr::new(
            span,
            ast::ExprKind::Apply {
                func: Box::new(get),
                args: vec![
                    base,
                    ast::Expr::new(span, ast::ExprKind::String(super_field_name(sup))),
                ],
            },
        );
    }
    base
}

pub(super) fn derive_dict_from_scope(
    span: ast::Span,
    class_env: &ClassEnv,
    dicts_in_scope: &HashSet<String>,
    wanted_class: &str,
) -> Option<ast::Expr> {
    let mut candidates: Vec<String> = dicts_in_scope.iter().cloned().collect();
    candidates.sort();

    for dict_var in candidates {
        let Some(sub) = dict_var.strip_prefix("__dict_") else {
            continue;
        };
        let Some(path) = find_super_path(class_env, sub, wanted_class) else {
            continue;
        };
        let base = ast::Expr::new(span, ast::ExprKind::Var(dict_var));
        return Some(project_dict_along_path(span, base, &path));
    }
    None
}

pub(super) fn is_syntactically_ground_value(e: &ast::Expr) -> bool {
    use ast::ExprKind;
    match &e.kind {
        ExprKind::Unit
        | ExprKind::Integer(_)
        | ExprKind::Float64(_)
        | ExprKind::Bool(_)
        | ExprKind::String(_)
        | ExprKind::Char(_) => true,
        ExprKind::List(es) => es.iter().all(is_syntactically_ground_value),
        ExprKind::Tuple(es) => es.iter().all(is_syntactically_ground_value),
        ExprKind::Record(fields) => fields.iter().all(|(_, v)| is_syntactically_ground_value(v)),
        _ => false,
    }
}

pub(super) fn required_classes_in_expr(
    expr: &ast::Expr,
    class_env: &ClassEnv,
    needs_dicts: &HashMap<String, Vec<String>>,
    out: &mut HashSet<String>,
) {
    use ast::ExprKind;
    match &expr.kind {
        ExprKind::Var(name) => required_classes_in_var(name, class_env, needs_dicts, out),
        ExprKind::Apply { func, args } => {
            required_classes_in_apply(func, args, class_env, needs_dicts, out);
            required_classes_in_expr(func, class_env, needs_dicts, out);
            for a in args {
                required_classes_in_expr(a, class_env, needs_dicts, out);
            }
        }
        ExprKind::Lambda { body, .. } => {
            required_classes_in_expr(body, class_env, needs_dicts, out);
        }
        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            required_classes_in_expr(cond, class_env, needs_dicts, out);
            required_classes_in_expr(then_branch, class_env, needs_dicts, out);
            required_classes_in_expr(else_branch, class_env, needs_dicts, out);
        }
        ExprKind::Let { bindings, body } => {
            for b in bindings {
                required_classes_in_expr(&b.expr, class_env, needs_dicts, out);
            }
            required_classes_in_expr(body, class_env, needs_dicts, out);
        }
        ExprKind::Where { expr, bindings } => {
            required_classes_in_expr(expr, class_env, needs_dicts, out);
            for b in bindings {
                required_classes_in_expr(&b.expr, class_env, needs_dicts, out);
            }
        }
        ExprKind::Annot { expr, .. } => {
            required_classes_in_expr(expr, class_env, needs_dicts, out);
        }
        ExprKind::Do(stmts) => {
            for s in stmts {
                match s {
                    ast::DoStmt::Bind { expr, .. } => {
                        required_classes_in_expr(expr, class_env, needs_dicts, out)
                    }
                    ast::DoStmt::Expr(e) => {
                        required_classes_in_expr(e, class_env, needs_dicts, out)
                    }
                }
            }
        }
        ExprKind::Case { expr, arms } => {
            required_classes_in_expr(expr, class_env, needs_dicts, out);
            for a in arms {
                if let Some(g) = &a.guard {
                    required_classes_in_expr(g, class_env, needs_dicts, out);
                }
                required_classes_in_expr(&a.body, class_env, needs_dicts, out);
            }
        }
        ExprKind::Cons { head, tail } => {
            required_classes_in_expr(head, class_env, needs_dicts, out);
            required_classes_in_expr(tail, class_env, needs_dicts, out);
        }
        ExprKind::List(es) | ExprKind::Tuple(es) => {
            for e in es {
                required_classes_in_expr(e, class_env, needs_dicts, out);
            }
        }
        ExprKind::Record(fields) => {
            for (_, v) in fields {
                required_classes_in_expr(v, class_env, needs_dicts, out);
            }
        }
        _ => {}
    }
}

fn insert_all(out: &mut HashSet<String>, classes: &[String]) {
    for c in classes {
        out.insert(c.clone());
    }
}

fn required_classes_in_var(
    name: &str,
    class_env: &ClassEnv,
    needs_dicts: &HashMap<String, Vec<String>>,
    out: &mut HashSet<String>,
) {
    if let Some(classes) = needs_dicts.get(name) {
        insert_all(out, classes);
        // If the name is a user-defined function/value with known requirements,
        // don't also treat it as a typeclass method.
        return;
    }
    if let Some(classes) = class_env.method_classes.get(name) {
        for c in classes {
            out.insert(c.name.clone());
        }
    }
}

fn required_classes_in_apply(
    func: &ast::Expr,
    args: &[ast::Expr],
    class_env: &ClassEnv,
    needs_dicts: &HashMap<String, Vec<String>>,
    out: &mut HashSet<String>,
) {
    use ast::ExprKind;

    let ExprKind::Var(name) = &func.kind else {
        return;
    };

    if let Some(classes) = class_env.method_classes.get(name) {
        if let Some(arg0) = args.first() {
            if !is_syntactically_ground_value(arg0) {
                if let Some(c) = classes.first() {
                    out.insert(c.name.clone());
                }
            }
        }
    }

    let Some(classes) = needs_dicts.get(name) else {
        return;
    };

    match args.first() {
        None => insert_all(out, classes),
        Some(arg0) => {
            if !is_syntactically_ground_value(arg0) {
                insert_all(out, classes);
            }
        }
    }
}

pub(super) fn add_dict_params_to_expr(
    span: ast::Span,
    expr: ast::Expr,
    classes: &[String],
) -> ast::Expr {
    use ast::{Expr, ExprKind};

    let mut dict_params: Vec<String> = classes.iter().map(|c| dict_param_name(c)).collect();
    if dict_params.is_empty() {
        return expr;
    }

    match expr.kind {
        ExprKind::Lambda { mut params, body } => {
            dict_params.append(&mut params);
            Expr::new(
                span,
                ExprKind::Lambda {
                    params: dict_params,
                    body,
                },
            )
        }
        ExprKind::Var(ref name) if name.contains('.') => {
            // Special case: if the expression is a qualified variable reference (e.g., an import
            // forwarder like `f = A.f`), wrap it in a lambda that passes the dict params through.
            // Otherwise, `f = \__dict -> A.f` would return A.f without applying the dict.
            // We only do this for qualified names to avoid breaking method references.
            let dict_args: Vec<Expr> = dict_params
                .iter()
                .map(|p| Expr::new(span, ExprKind::Var(p.clone())))
                .collect();
            Expr::new(
                span,
                ExprKind::Lambda {
                    params: dict_params.clone(),
                    body: Box::new(Expr::new(
                        span,
                        ExprKind::Apply {
                            func: Box::new(expr),
                            args: dict_args,
                        },
                    )),
                },
            )
        }
        other => Expr::new(
            span,
            ExprKind::Lambda {
                params: dict_params,
                body: Box::new(Expr::new(span, other)),
            },
        ),
    }
}

pub(super) fn pick_instance_dict_expr_from_scope(
    span: ast::Span,
    class_env: &ClassEnv,
    dicts_in_scope: &HashSet<String>,
    class: &str,
    ty: &Ty,
) -> Result<Option<ast::Expr>> {
    use ast::{Expr, ExprKind};

    if !ftv_ty(ty).is_empty() {
        // Not ground: we cannot soundly choose a global instance dictionary here.
        return Ok(None);
    }

    let ty_norm = super::normalize_ty_for_instance_key(ty);

    if let Ok(head) = instance_head_key_ty(&ty_norm) {
        // Stage 2: instances are keyed by ClassId, but dict passing currently selects by
        // unqualified class name (compatible with existing stdlib dict names).
        if let Some(class_id) = class_env
            .class_params
            .keys()
            .find(|id| {
                id.name.rsplit('.').next().unwrap_or(id.name.as_str())
                    == class.rsplit('.').next().unwrap_or(class)
            })
            .cloned()
        {
            let key = (class_id, head);
            if let Some(d) = class_env.instances.get(&key).cloned() {
                // For stdlib dicts (Prelude.*), use unqualified names at AST level.
                // For user-defined dicts, keep qualification to avoid conflicts.
                let dict_ref = if d.starts_with("Prelude.") {
                    d.split('.').next_back().unwrap_or(d.as_str()).to_string()
                } else {
                    d
                };
                return Ok(Some(Expr::new(span, ExprKind::Var(dict_ref))));
            }
        }
    }

    let mut candidates: Vec<&PolyInstance> = Vec::new();
    for pi in &class_env.poly_instances {
        if pi
            .class
            .name
            .rsplit('.')
            .next()
            .unwrap_or(pi.class.name.as_str())
            != class.rsplit('.').next().unwrap_or(class)
        {
            continue;
        }

        let mut map: HashMap<u32, u32> = HashMap::new();
        let mut next: u32 = 10_000;
        fn rename_vars(ty: &Ty, map: &mut HashMap<u32, u32>, next: &mut u32) -> Ty {
            match ty {
                Ty::Var(v) => {
                    let nv = *map.entry(*v).or_insert_with(|| {
                        let out = *next;
                        *next += 1;
                        out
                    });
                    Ty::Var(nv)
                }
                Ty::Con(c) => Ty::Con(c.clone()),
                Ty::App { head, args } => Ty::App {
                    head: Box::new(rename_vars(head, map, next)),
                    args: args.iter().map(|a| rename_vars(a, map, next)).collect(),
                },
                Ty::Func(a, b) => Ty::Func(
                    Box::new(rename_vars(a, map, next)),
                    Box::new(rename_vars(b, map, next)),
                ),
                Ty::Tuple(ts) => Ty::Tuple(ts.iter().map(|t| rename_vars(t, map, next)).collect()),
                Ty::List(t) => Ty::List(Box::new(rename_vars(t, map, next))),
                Ty::Record(fields) => Ty::Record(
                    fields
                        .iter()
                        .map(|(k, v)| (k.clone(), rename_vars(v, map, next)))
                        .collect(),
                ),
                Ty::RecordOpen(fields, rest) => Ty::RecordOpen(
                    fields
                        .iter()
                        .map(|(k, v)| (k.clone(), rename_vars(v, map, next)))
                        .collect(),
                    Box::new(rename_vars(rest, map, next)),
                ),
            }
        }

        let pat = rename_vars(&pi.head_pat, &mut map, &mut next);
        let ok = unify(pat.clone(), ty_norm.clone()).is_ok();
        if ok {
            candidates.push(pi);
        }
    }

    if candidates.is_empty() {
        return Ok(None);
    }
    if candidates.len() > 1 {
        let (candidate_notes, origin_notes) = poly_instance_overlap_details(&candidates);
        return Err(Error::msg(format!(
            "overlapping instances for `{}`: cannot choose for type {ty}; candidates: {}; import origins: {}",
            class, candidate_notes, origin_notes
        )));
    }

    let pi = candidates[0];
    // For stdlib dicts, use unqualified names (same as above)
    let dict_ref = if pi.dict_name.starts_with("Prelude.") {
        pi.dict_name
            .split('.')
            .next_back()
            .unwrap_or(pi.dict_name.as_str())
            .to_string()
    } else {
        pi.dict_name.clone()
    };
    let mut expr = Expr::new(span, ExprKind::Var(dict_ref));
    for i in 0..pi.ctx_len {
        let pname = format!("__ctx_dict_{i}");
        if !dicts_in_scope.contains(&pname) {
            return Err(Error::msg(format!(
                "missing instance context dictionary in scope: {pname}"
            )));
        }
        expr = Expr::new(
            span,
            ExprKind::Apply {
                func: Box::new(expr),
                args: vec![Expr::new(span, ExprKind::Var(pname))],
            },
        );
    }
    Ok(Some(expr))
}

pub(super) struct CallInfo {
    pub(super) expected_arg_tys: Vec<Ty>,
    pub(super) class_tys: HashMap<String, Ty>,
}

pub(super) fn call_info_for_call(
    module_snapshot: &ast::Module,
    class_env: &ClassEnv,
    inferred: &HashMap<String, Scheme>,
    callee: &str,
    args: &[ast::Expr],
    class_index: Option<&super::ClassEnvIndex>,
    inferred_unqual_index: Option<&HashMap<String, Option<String>>>,
) -> Option<CallInfo> {
    // Prefer exact match; if `callee` is unqualified, allow a unique suffix match
    // against inferred bindings (e.g. lookup "print" via "Prelude.print").
    // If not found, fall back to class-method schemes (methods are values).
    let scheme: Scheme = inferred
        .get(callee)
        .cloned()
        .or_else(|| {
            if callee.contains('.') {
                return None;
            }

            if let Some(idx) = inferred_unqual_index {
                if let Some(hit) = idx.get(callee) {
                    match hit {
                        Some(qualified) => return inferred.get(qualified).cloned(),
                        None => return None,
                    }
                }
            }

            let mut found: Option<Scheme> = None;
            for (k, v) in inferred.iter() {
                let last = k.split('.').next_back().unwrap_or(k.as_str());
                if last == callee {
                    if found.is_some() {
                        return None;
                    }
                    found = Some(v.clone());
                }
            }
            found
        })
        .or_else(|| {
            // e.g. `>>=` / `>>` inserted via class method index during inference,
            // but not necessarily present as a top-level binding in `inferred`.
            if !class_env.method_classes.contains_key(callee) {
                return None;
            }
            if let Some(idx) = class_index {
                return idx.methods_by_name.get(callee).cloned();
            }

            // Fallback: build on-demand if no module-level index was provided.
            let mut cx_for_index = InferCtx::default();
            let idx = super::build_class_method_scheme_index(&mut cx_for_index, class_env).ok()?;
            idx.methods_by_name.get(callee).cloned()
        })?;

    let mut cx = InferCtx::default();
    let (cs, mut callee_ty) = instantiate_qual(&mut cx, &scheme);
    let mut subst = Subst::new();
    let mut expected: Vec<Ty> = Vec::new();

    for arg in args {
        let Ty::Func(dom, cod) = callee_ty else {
            return None;
        };

        expected.push(apply(&subst, (*dom).clone()));

        if let Ok(arg_ty) =
            infer_in_module_with_class_env(module_snapshot, class_env, inferred, arg.clone())
        {
            if let Ok(s) = unify(apply(&subst, (*dom).clone()), apply(&subst, arg_ty)) {
                subst = compose(&s, &subst);
            }
        }

        callee_ty = apply(&subst, *cod);
    }

    let mut class_tys: HashMap<String, Ty> = HashMap::new();
    for c in cs {
        match c {
            Constraint::Class { class, ty } => {
                class_tys.insert(class.name, apply(&subst, ty));
            }
            Constraint::Show(ty) => {
                class_tys.insert("Show".to_string(), apply(&subst, ty));
            }
            Constraint::Eq(ty) => {
                class_tys.insert("Eq".to_string(), apply(&subst, ty));
            }
            Constraint::ShowRow(ty) => {
                class_tys.insert("ShowRow".to_string(), apply(&subst, ty));
            }
            Constraint::EqRow(ty) => {
                class_tys.insert("EqRow".to_string(), apply(&subst, ty));
            }
            _ => {}
        }
    }

    Some(CallInfo {
        expected_arg_tys: expected.into_iter().map(|t| apply(&subst, t)).collect(),
        class_tys,
    })
}
