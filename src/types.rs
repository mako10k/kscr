//! Type checking and elaboration scaffolding.
//!
//! Policy (docs):
//! - Surface numeric types: Integer (arbitrary precision) and Float64.
//! - Backend/IR numeric types are LLVM-aligned (i32/i64/f32/f64...).
//! - Pure IR subtyping allows only integer widening (iN <: iM); float widening is NOT subtyping.
//! - Potentially lossy conversions happen only at boundaries as checked casts.

use crate::{ast, error::Error, Result};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedModule {
    pub module: ast::Module,
}

// --- Milestone 2.2.1: Unification core (scaffolding) ---

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    Var(u32),
    Con(String),
    List(Box<Ty>),
    Tuple(Vec<Ty>),
    Record(Vec<(String, Ty)>),
    App { head: Box<Ty>, args: Vec<Ty> },
    Func(Box<Ty>, Box<Ty>),
}

#[derive(Debug, Default)]
pub struct InferCtx {
    next_var: u32,
}

impl InferCtx {
    pub fn fresh(&mut self) -> Ty {
        let v = self.next_var;
        self.next_var += 1;
        Ty::Var(v)
    }
}

pub type Subst = HashMap<u32, Ty>;

pub fn unify(a: Ty, b: Ty) -> Result<Subst> {
    let mut subst = Subst::new();
    unify_in(&mut subst, a, b)?;
    Ok(subst)
}

fn unify_in(subst: &mut Subst, a: Ty, b: Ty) -> Result<()> {
    let a = apply(subst, a);
    let b = apply(subst, b);

    match (a, b) {
        (Ty::Var(v), t) | (t, Ty::Var(v)) => bind_var(subst, v, t),
        (Ty::Con(a), Ty::Con(b)) if a == b => Ok(()),
        (Ty::List(a), Ty::List(b)) => unify_in(subst, *a, *b),
        (Ty::Tuple(a), Ty::Tuple(b)) => {
            if a.len() != b.len() {
                return Err(Error::msg("tuple arity mismatch"));
            }
            for (x, y) in a.into_iter().zip(b) {
                unify_in(subst, x, y)?;
            }
            Ok(())
        }
        (Ty::Record(a), Ty::Record(b)) => {
            if a.len() != b.len() {
                return Err(Error::msg("record arity mismatch"));
            }
            for ((na, ta), (nb, tb)) in a.into_iter().zip(b) {
                if na != nb {
                    return Err(Error::msg("record field mismatch"));
                }
                unify_in(subst, ta, tb)?;
            }
            Ok(())
        }
        (Ty::App { head: ha, args: aa }, Ty::App { head: hb, args: ab }) => {
            if aa.len() != ab.len() {
                return Err(Error::msg("type application arity mismatch"));
            }
            unify_in(subst, *ha, *hb)?;
            for (x, y) in aa.into_iter().zip(ab) {
                unify_in(subst, x, y)?;
            }
            Ok(())
        }
        (Ty::Func(a1, a2), Ty::Func(b1, b2)) => {
            unify_in(subst, *a1, *b1)?;
            unify_in(subst, *a2, *b2)
        }
        _ => Err(Error::msg("cannot unify")),
    }
}

fn bind_var(subst: &mut Subst, v: u32, t: Ty) -> Result<()> {
    match t {
        Ty::Var(v2) if v == v2 => return Ok(()),
        _ => {}
    }

    if occurs(subst, v, &t) {
        return Err(Error::msg("occurs check"));
    }

    subst.insert(v, t);
    Ok(())
}

fn occurs(subst: &Subst, v: u32, t: &Ty) -> bool {
    let mut seen = HashSet::new();
    occurs_in(subst, &mut seen, v, t)
}

fn occurs_in(subst: &Subst, seen: &mut HashSet<u32>, v: u32, t: &Ty) -> bool {
    match t {
        Ty::Var(x) => {
            if *x == v {
                return true;
            }
            if !seen.insert(*x) {
                return false;
            }
            match subst.get(x) {
                Some(t) => occurs_in(subst, seen, v, t),
                None => false,
            }
        }
        Ty::List(t) => occurs_in(subst, seen, v, t),
        Ty::Tuple(ts) => ts.iter().any(|t| occurs_in(subst, seen, v, t)),
        Ty::Record(fields) => fields.iter().any(|(_, t)| occurs_in(subst, seen, v, t)),
        Ty::App { head, args } => {
            occurs_in(subst, seen, v, head) || args.iter().any(|t| occurs_in(subst, seen, v, t))
        }
        Ty::Func(a, b) => occurs_in(subst, seen, v, a) || occurs_in(subst, seen, v, b),
        Ty::Con(_) => false,
    }
}

pub fn apply(subst: &Subst, t: Ty) -> Ty {
    match t {
        Ty::Var(v) => match subst.get(&v) {
            Some(t) => apply(subst, t.clone()),
            None => Ty::Var(v),
        },
        Ty::List(t) => Ty::List(Box::new(apply(subst, *t))),
        Ty::Tuple(ts) => Ty::Tuple(ts.into_iter().map(|t| apply(subst, t)).collect()),
        Ty::Record(fields) => Ty::Record(
            fields
                .into_iter()
                .map(|(n, t)| (n, apply(subst, t)))
                .collect(),
        ),
        Ty::App { head, args } => Ty::App {
            head: Box::new(apply(subst, *head)),
            args: args.into_iter().map(|t| apply(subst, t)).collect(),
        },
        Ty::Func(a, b) => Ty::Func(Box::new(apply(subst, *a)), Box::new(apply(subst, *b))),
        c @ Ty::Con(_) => c,
    }
}

// --- Milestone 2.2.1/2.2.2: Schemes + minimal expression inference ---

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scheme {
    pub vars: Vec<u32>,
    pub ty: Ty,
}

impl Scheme {
    pub fn mono(ty: Ty) -> Self {
        Self { vars: vec![], ty }
    }
}

type TypeEnv = HashMap<String, Scheme>;

pub fn compose(s1: &Subst, s2: &Subst) -> Subst {
    let mut out: Subst = s2.iter().map(|(v, t)| (*v, apply(s1, t.clone()))).collect();
    for (v, t) in s1 {
        out.insert(*v, t.clone());
    }
    out
}

pub fn ftv_ty(ty: &Ty) -> HashSet<u32> {
    match ty {
        Ty::Var(v) => [*v].into_iter().collect(),
        Ty::Con(_) => HashSet::new(),
        Ty::List(t) => ftv_ty(t),
        Ty::Tuple(ts) => ts.iter().flat_map(ftv_ty).collect(),
        Ty::Record(fields) => fields.iter().flat_map(|(_, t)| ftv_ty(t)).collect(),
        Ty::App { head, args } => {
            let mut s = ftv_ty(head);
            for t in args {
                s.extend(ftv_ty(t));
            }
            s
        }
        Ty::Func(a, b) => {
            let mut s = ftv_ty(a);
            s.extend(ftv_ty(b));
            s
        }
    }
}

pub fn ftv_scheme(s: &Scheme) -> HashSet<u32> {
    let mut ftv = ftv_ty(&s.ty);
    for v in &s.vars {
        ftv.remove(v);
    }
    ftv
}

pub fn ftv_env(env: &TypeEnv) -> HashSet<u32> {
    env.values().flat_map(ftv_scheme).collect()
}

pub fn generalize(env: &TypeEnv, ty: Ty) -> Scheme {
    let env_ftv = ftv_env(env);
    let mut vars: Vec<u32> = ftv_ty(&ty).difference(&env_ftv).copied().collect();
    vars.sort_unstable();
    Scheme { vars, ty }
}

pub fn instantiate(cx: &mut InferCtx, s: &Scheme) -> Ty {
    let mut m: HashMap<u32, Ty> = HashMap::new();
    for v in &s.vars {
        m.insert(*v, cx.fresh());
    }
    replace_vars(&s.ty, &m)
}

fn replace_vars(ty: &Ty, m: &HashMap<u32, Ty>) -> Ty {
    match ty {
        Ty::Var(v) => m.get(v).cloned().unwrap_or(Ty::Var(*v)),
        Ty::Con(c) => Ty::Con(c.clone()),
        Ty::List(t) => Ty::List(Box::new(replace_vars(t, m))),
        Ty::Tuple(ts) => Ty::Tuple(ts.iter().map(|t| replace_vars(t, m)).collect()),
        Ty::Record(fields) => Ty::Record(
            fields
                .iter()
                .map(|(n, t)| (n.clone(), replace_vars(t, m)))
                .collect(),
        ),
        Ty::App { head, args } => Ty::App {
            head: Box::new(replace_vars(head, m)),
            args: args.iter().map(|t| replace_vars(t, m)).collect(),
        },
        Ty::Func(a, b) => Ty::Func(Box::new(replace_vars(a, m)), Box::new(replace_vars(b, m))),
    }
}

fn apply_scheme(subst: &Subst, s: &Scheme) -> Scheme {
    if s.vars.is_empty() {
        return Scheme {
            vars: vec![],
            ty: apply(subst, s.ty.clone()),
        };
    }

    let mut sub = subst.clone();
    for v in &s.vars {
        sub.remove(v);
    }

    Scheme {
        vars: s.vars.clone(),
        ty: apply(&sub, s.ty.clone()),
    }
}

fn apply_env(subst: &Subst, env: &TypeEnv) -> TypeEnv {
    env.iter()
        .map(|(k, v)| (k.clone(), apply_scheme(subst, v)))
        .collect()
}

fn infer_pat_in(
    cx: &mut InferCtx,
    subst: &mut Subst,
    pat: &ast::Pattern,
    binds: &mut Vec<(String, Ty)>,
    seen: &mut HashSet<String>,
) -> Result<Ty> {
    use ast::{Expr, Pattern};

    match pat {
        Pattern::Var(name) => {
            if !seen.insert(name.clone()) {
                return Err(Error::msg("duplicate pattern variable"));
            }
            let t = cx.fresh();
            binds.push((name.clone(), t.clone()));
            Ok(t)
        }
        Pattern::Wildcard => Ok(cx.fresh()),
        Pattern::Literal(e) => Ok(match e {
            Expr::Unit => Ty::Con("Unit".to_string()),
            Expr::Integer(_) => Ty::Con("Integer".to_string()),
            Expr::Float64(_) => Ty::Con("Float64".to_string()),
            Expr::Bool(_) => Ty::Con("Bool".to_string()),
            Expr::String(_) => Ty::Con("String".to_string()),
            _ => return Err(Error::msg("unsupported literal pattern")),
        }),
        Pattern::Tuple(ps) => Ok(Ty::Tuple(
            ps.iter()
                .map(|p| infer_pat_in(cx, subst, p, binds, seen))
                .collect::<Result<Vec<_>>>()?,
        )),
        Pattern::List(ps) => {
            if ps.is_empty() {
                return Ok(Ty::List(Box::new(cx.fresh())));
            }

            let first = infer_pat_in(cx, subst, &ps[0], binds, seen)?;
            for p in &ps[1..] {
                let t = infer_pat_in(cx, subst, p, binds, seen)?;
                let su = unify(apply(subst, first.clone()), apply(subst, t))?;
                *subst = compose(&su, subst);
            }
            Ok(Ty::List(Box::new(apply(subst, first))))
        }
        Pattern::Record(fields) => Ok(Ty::Record(
            fields
                .iter()
                .map(|(n, p)| Ok((n.clone(), infer_pat_in(cx, subst, p, binds, seen)?)))
                .collect::<Result<Vec<_>>>()?,
        )),
        _ => Err(Error::msg("pattern inference not implemented")),
    }
}

pub fn infer_expr(expr: ast::Expr) -> Result<Ty> {
    let mut cx = InferCtx::default();
    let env = TypeEnv::new();
    let (s, t) = infer_expr_in(&mut cx, &env, expr)?;
    Ok(apply(&s, t))
}

fn infer_expr_in(cx: &mut InferCtx, env: &TypeEnv, expr: ast::Expr) -> Result<(Subst, Ty)> {
    use ast::Expr;

    match expr {
        Expr::Unit => Ok((Subst::new(), Ty::Con("Unit".to_string()))),
        Expr::Integer(_) => Ok((Subst::new(), Ty::Con("Integer".to_string()))),
        Expr::Float64(_) => Ok((Subst::new(), Ty::Con("Float64".to_string()))),
        Expr::Bool(true) | Expr::Bool(false) => Ok((Subst::new(), Ty::Con("Bool".to_string()))),
        Expr::String(_) => Ok((Subst::new(), Ty::Con("String".to_string()))),

        Expr::Var(name) => {
            let s = env
                .get(&name)
                .ok_or_else(|| Error::msg("unbound variable"))?;
            Ok((Subst::new(), instantiate(cx, s)))
        }

        Expr::Lambda { params, body } => {
            if params.is_empty() {
                return Err(Error::msg("expected lambda parameter"));
            }

            let mut env2 = env.clone();
            let mut param_tys = Vec::new();
            for p in &params {
                let tv = cx.fresh();
                env2.insert(p.clone(), Scheme::mono(tv.clone()));
                param_tys.push(tv);
            }

            let (s_body, body_ty) = infer_expr_in(cx, &env2, *body)?;
            let mut out = apply(&s_body, body_ty);
            for pty in param_tys.into_iter().rev() {
                out = Ty::Func(Box::new(apply(&s_body, pty)), Box::new(out));
            }

            Ok((s_body, out))
        }

        Expr::Apply { func, args } => {
            let (mut s, mut fun_ty) = infer_expr_in(cx, env, *func)?;

            for arg in args {
                let env2 = apply_env(&s, env);
                let (s_arg, arg_ty) = infer_expr_in(cx, &env2, arg)?;
                s = compose(&s_arg, &s);

                fun_ty = apply(&s, fun_ty);
                let res = cx.fresh();

                let s_unify = unify(
                    fun_ty,
                    Ty::Func(Box::new(apply(&s, arg_ty)), Box::new(res.clone())),
                )?;
                s = compose(&s_unify, &s);
                fun_ty = apply(&s, res);
            }

            Ok((s, fun_ty))
        }

        Expr::Annot { expr, ty } => {
            let (s1, t1) = infer_expr_in(cx, env, *expr)?;
            let mut holes = HashMap::new();
            let t_ann = lower_surface_type(cx, &ty, &mut holes);
            let s2 = unify(apply(&s1, t1), apply(&s1, t_ann.clone()))?;
            let s = compose(&s2, &s1);
            Ok((s.clone(), apply(&s, t_ann)))
        }

        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            let (s_cond, t_cond) = infer_expr_in(cx, env, *cond)?;
            let s_bool = unify(apply(&s_cond, t_cond), Ty::Con("Bool".to_string()))?;
            let mut s = compose(&s_bool, &s_cond);

            let env2 = apply_env(&s, env);
            let (s_then, t_then) = infer_expr_in(cx, &env2, *then_branch)?;
            s = compose(&s_then, &s);

            let env3 = apply_env(&s, env);
            let (s_else, t_else) = infer_expr_in(cx, &env3, *else_branch)?;
            s = compose(&s_else, &s);

            let s_res = unify(apply(&s, t_then.clone()), apply(&s, t_else))?;
            s = compose(&s_res, &s);
            Ok((s.clone(), apply(&s, apply(&s, t_then))))
        }

        Expr::Tuple(elems) => {
            let mut s = Subst::new();
            let mut ts = Vec::new();
            for e in elems {
                let env2 = apply_env(&s, env);
                let (s_e, t_e) = infer_expr_in(cx, &env2, e)?;
                s = compose(&s_e, &s);
                ts.push(apply(&s, t_e));
            }
            Ok((s, Ty::Tuple(ts)))
        }

        Expr::List(elems) => {
            if elems.is_empty() {
                return Ok((Subst::new(), Ty::List(Box::new(cx.fresh()))));
            }

            let (mut s, first_ty) = infer_expr_in(cx, env, elems[0].clone())?;
            let mut elem_ty = apply(&s, first_ty);

            for e in elems.into_iter().skip(1) {
                let env2 = apply_env(&s, env);
                let (s_e, t_e) = infer_expr_in(cx, &env2, e)?;
                s = compose(&s_e, &s);

                let su = unify(apply(&s, elem_ty.clone()), apply(&s, t_e))?;
                s = compose(&su, &s);
                elem_ty = apply(&s, elem_ty);
            }

            Ok((s.clone(), Ty::List(Box::new(apply(&s, elem_ty)))))
        }

        Expr::Record(fields) => {
            let mut s = Subst::new();
            let mut out = Vec::new();
            for (name, e) in fields {
                let env2 = apply_env(&s, env);
                let (s_e, t_e) = infer_expr_in(cx, &env2, e)?;
                s = compose(&s_e, &s);
                out.push((name, apply(&s, t_e)));
            }
            Ok((s, Ty::Record(out)))
        }

        Expr::Let { bindings, body } => {
            let mut s = Subst::new();
            let mut env2 = env.clone();

            for b in bindings {
                let mut binds = Vec::new();
                let mut seen = HashSet::new();
                let pat_ty = infer_pat_in(cx, &mut s, &b.pat, &mut binds, &mut seen)?;

                let env_in = apply_env(&s, &env2);
                let (s_rhs, t_rhs) = infer_expr_in(cx, &env_in, b.expr)?;
                s = compose(&s_rhs, &s);

                let s_pat = unify(apply(&s, t_rhs), apply(&s, pat_ty))?;
                s = compose(&s_pat, &s);

                for (name, t) in binds {
                    let env_gen = apply_env(&s, &env2);
                    let scheme = generalize(&env_gen, apply(&s, t));
                    env2.insert(name, scheme);
                }
            }

            let env_body = apply_env(&s, &env2);
            let (s_body, t_body) = infer_expr_in(cx, &env_body, *body)?;
            let s = compose(&s_body, &s);
            Ok((s.clone(), apply(&s, t_body)))
        }

        Expr::Where { expr, bindings } => infer_expr_in(
            cx,
            env,
            Expr::Let {
                bindings,
                body: expr,
            },
        ),

        Expr::Case { expr, arms } => {
            if arms.is_empty() {
                return Err(Error::msg("empty case"));
            }

            let (mut s, scrut_ty) = infer_expr_in(cx, env, *expr)?;
            let mut out_ty = cx.fresh();

            for (pat, arm_expr) in arms {
                let mut binds = Vec::new();
                let mut seen = HashSet::new();
                let pat_ty = infer_pat_in(cx, &mut s, &pat, &mut binds, &mut seen)?;

                let su_pat = unify(apply(&s, pat_ty), apply(&s, scrut_ty.clone()))?;
                s = compose(&su_pat, &s);

                let mut env_arm = apply_env(&s, env);
                for (name, t) in binds {
                    env_arm.insert(name, Scheme::mono(apply(&s, t)));
                }

                let (s_arm, arm_ty) = infer_expr_in(cx, &env_arm, arm_expr)?;
                s = compose(&s_arm, &s);

                let su_out = unify(apply(&s, out_ty.clone()), apply(&s, arm_ty))?;
                s = compose(&su_out, &s);
                out_ty = apply(&s, out_ty);
            }

            Ok((s.clone(), apply(&s, out_ty)))
        }

        _ => Err(Error::msg("inference not implemented for this expression")),
    }
}

fn lower_surface_type(cx: &mut InferCtx, ty: &ast::Type, holes: &mut HashMap<String, Ty>) -> Ty {
    use ast::Type;

    match ty {
        Type::Unit => Ty::Con("Unit".to_string()),
        Type::Integer => Ty::Con("Integer".to_string()),
        Type::Bool => Ty::Con("Bool".to_string()),
        Type::Float64 => Ty::Con("Float64".to_string()),
        Type::Char => Ty::Con("Char".to_string()),
        Type::String => Ty::Con("String".to_string()),

        Type::List(t) => Ty::List(Box::new(lower_surface_type(cx, t, holes))),
        Type::Tuple(ts) => Ty::Tuple(
            ts.iter()
                .map(|t| lower_surface_type(cx, t, holes))
                .collect(),
        ),
        Type::Record(fields) => Ty::Record(
            fields
                .iter()
                .map(|(n, t)| (n.clone(), lower_surface_type(cx, t, holes)))
                .collect(),
        ),
        Type::Func(a, b) => Ty::Func(
            Box::new(lower_surface_type(cx, a, holes)),
            Box::new(lower_surface_type(cx, b, holes)),
        ),
        Type::App { head, args } => Ty::App {
            head: Box::new(lower_surface_type(cx, head, holes)),
            args: args
                .iter()
                .map(|t| lower_surface_type(cx, t, holes))
                .collect(),
        },

        Type::Hole(Some(name)) => holes
            .entry(name.clone())
            .or_insert_with(|| cx.fresh())
            .clone(),
        Type::Hole(None) => cx.fresh(),

        Type::Var(name) => Ty::Con(name.clone()),
    }
}

pub fn typecheck(mut module: ast::Module) -> Result<TypedModule> {
    let aliases = collect_type_aliases(&module);
    module.items = module
        .items
        .into_iter()
        .map(|it| expand_item(it, &aliases))
        .collect::<Result<Vec<_>>>()?;
    Ok(TypedModule { module })
}

fn collect_type_aliases(module: &ast::Module) -> HashMap<String, ast::TypeAlias> {
    module
        .items
        .iter()
        .filter_map(|it| match it {
            ast::Item::TypeAlias(ta) => Some((ta.name.clone(), ta.clone())),
            _ => None,
        })
        .collect()
}

fn expand_item(item: ast::Item, aliases: &HashMap<String, ast::TypeAlias>) -> Result<ast::Item> {
    match item {
        ast::Item::Binding(b) => Ok(ast::Item::Binding(ast::Binding {
            pat: b.pat,
            expr: expand_expr(b.expr, aliases)?,
        })),
        ast::Item::TypeAlias(ta) => Ok(ast::Item::TypeAlias(ast::TypeAlias {
            name: ta.name,
            params: ta.params,
            ty: expand_type(ta.ty, aliases, &mut Vec::new())?,
        })),
        ast::Item::DataDecl(d) => Ok(ast::Item::DataDecl(ast::DataDecl {
            name: d.name,
            params: d.params,
            ctors: d
                .ctors
                .into_iter()
                .map(|c| {
                    Ok(ast::DataCtor {
                        name: c.name,
                        args: c
                            .args
                            .into_iter()
                            .map(|t| expand_type(t, aliases, &mut Vec::new()))
                            .collect::<Result<Vec<_>>>()?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        })),
        it @ (ast::Item::Import(_) | ast::Item::Export(_)) => Ok(it),
    }
}

fn expand_expr(expr: ast::Expr, aliases: &HashMap<String, ast::TypeAlias>) -> Result<ast::Expr> {
    use ast::Expr;
    Ok(match expr {
        Expr::Lambda { params, body } => Expr::Lambda {
            params,
            body: Box::new(expand_expr(*body, aliases)?),
        },
        Expr::Apply { func, args } => Expr::Apply {
            func: Box::new(expand_expr(*func, aliases)?),
            args: args
                .into_iter()
                .map(|e| expand_expr(e, aliases))
                .collect::<Result<Vec<_>>>()?,
        },
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => Expr::If {
            cond: Box::new(expand_expr(*cond, aliases)?),
            then_branch: Box::new(expand_expr(*then_branch, aliases)?),
            else_branch: Box::new(expand_expr(*else_branch, aliases)?),
        },
        Expr::Let { bindings, body } => Expr::Let {
            bindings: bindings
                .into_iter()
                .map(|b| {
                    Ok(ast::Binding {
                        pat: b.pat,
                        expr: expand_expr(b.expr, aliases)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            body: Box::new(expand_expr(*body, aliases)?),
        },
        Expr::Where { expr, bindings } => Expr::Where {
            expr: Box::new(expand_expr(*expr, aliases)?),
            bindings: bindings
                .into_iter()
                .map(|b| {
                    Ok(ast::Binding {
                        pat: b.pat,
                        expr: expand_expr(b.expr, aliases)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        },
        Expr::Annot { expr, ty } => Expr::Annot {
            expr: Box::new(expand_expr(*expr, aliases)?),
            ty: expand_type(ty, aliases, &mut Vec::new())?,
        },
        Expr::Do(stmts) => Expr::Do(
            stmts
                .into_iter()
                .map(|s| {
                    Ok(match s {
                        ast::DoStmt::Bind { name, expr } => ast::DoStmt::Bind {
                            name,
                            expr: expand_expr(expr, aliases)?,
                        },
                        ast::DoStmt::Expr(e) => ast::DoStmt::Expr(expand_expr(e, aliases)?),
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        ),
        Expr::Case { expr, arms } => Expr::Case {
            expr: Box::new(expand_expr(*expr, aliases)?),
            arms: arms
                .into_iter()
                .map(|(p, e)| Ok((p, expand_expr(e, aliases)?)))
                .collect::<Result<Vec<_>>>()?,
        },
        Expr::List(v) => Expr::List(
            v.into_iter()
                .map(|e| expand_expr(e, aliases))
                .collect::<Result<Vec<_>>>()?,
        ),
        Expr::Tuple(v) => Expr::Tuple(
            v.into_iter()
                .map(|e| expand_expr(e, aliases))
                .collect::<Result<Vec<_>>>()?,
        ),
        Expr::Record(fields) => Expr::Record(
            fields
                .into_iter()
                .map(|(n, e)| Ok((n, expand_expr(e, aliases)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
        other => other,
    })
}

fn expand_type(
    ty: ast::Type,
    aliases: &HashMap<String, ast::TypeAlias>,
    stack: &mut Vec<String>,
) -> Result<ast::Type> {
    use ast::Type;

    Ok(match ty {
        Type::Hole(name) => Type::Hole(name),
        Type::List(t) => Type::List(Box::new(expand_type(*t, aliases, stack)?)),
        Type::Tuple(ts) => Type::Tuple(
            ts.into_iter()
                .map(|t| expand_type(t, aliases, stack))
                .collect::<Result<Vec<_>>>()?,
        ),
        Type::Record(fields) => Type::Record(
            fields
                .into_iter()
                .map(|(n, t)| Ok((n, expand_type(t, aliases, stack)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
        Type::Func(a, b) => Type::Func(
            Box::new(expand_type(*a, aliases, stack)?),
            Box::new(expand_type(*b, aliases, stack)?),
        ),
        Type::App { head, args } => {
            let args = args
                .into_iter()
                .map(|t| expand_type(t, aliases, stack))
                .collect::<Result<Vec<_>>>()?;

            match *head {
                Type::Var(name) => {
                    if let Some(alias) = aliases.get(&name) {
                        if alias.params.len() != args.len() {
                            return Err(Error::msg("type alias arity mismatch"));
                        }
                        expand_alias(alias, &args, aliases, stack)?
                    } else {
                        Type::App {
                            head: Box::new(Type::Var(name)),
                            args,
                        }
                    }
                }
                head => Type::App {
                    head: Box::new(expand_type(head, aliases, stack)?),
                    args,
                },
            }
        }
        Type::Var(name) => {
            if let Some(alias) = aliases.get(&name) {
                if alias.params.is_empty() {
                    expand_alias(alias, &[], aliases, stack)?
                } else {
                    Type::Var(name)
                }
            } else {
                Type::Var(name)
            }
        }
        other => other,
    })
}

fn expand_alias(
    alias: &ast::TypeAlias,
    args: &[ast::Type],
    aliases: &HashMap<String, ast::TypeAlias>,
    stack: &mut Vec<String>,
) -> Result<ast::Type> {
    if stack.contains(&alias.name) {
        return Err(Error::msg("recursive type alias"));
    }
    stack.push(alias.name.clone());

    let env: HashMap<String, ast::Type> = alias
        .params
        .iter()
        .cloned()
        .zip(args.iter().cloned())
        .collect();

    let ty = subst_type(alias.ty.clone(), &env);
    let ty = expand_type(ty, aliases, stack)?;

    stack.pop();
    Ok(ty)
}

fn subst_type(ty: ast::Type, env: &HashMap<String, ast::Type>) -> ast::Type {
    use ast::Type;

    match ty {
        Type::Hole(name) => Type::Hole(name),
        Type::Var(v) => env.get(&v).cloned().unwrap_or(Type::Var(v)),
        Type::List(t) => Type::List(Box::new(subst_type(*t, env))),
        Type::Tuple(ts) => Type::Tuple(ts.into_iter().map(|t| subst_type(t, env)).collect()),
        Type::Record(fields) => Type::Record(
            fields
                .into_iter()
                .map(|(n, t)| (n, subst_type(t, env)))
                .collect(),
        ),
        Type::App { head, args } => Type::App {
            head: Box::new(subst_type(*head, env)),
            args: args.into_iter().map(|t| subst_type(t, env)).collect(),
        },
        Type::Func(a, b) => {
            Type::Func(Box::new(subst_type(*a, env)), Box::new(subst_type(*b, env)))
        }
        other => other,
    }
}

#[cfg(test)]
mod unification_tests {
    use super::*;

    #[test]
    fn unify_var_with_con() {
        let mut cx = InferCtx::default();
        let a = cx.fresh();
        let subst = unify(a.clone(), Ty::Con("Integer".to_string())).unwrap();
        assert_eq!(apply(&subst, a), Ty::Con("Integer".to_string()));
    }

    #[test]
    fn unify_func() {
        let mut cx = InferCtx::default();
        let a = cx.fresh();
        let b = cx.fresh();

        let t1 = Ty::Func(Box::new(a.clone()), Box::new(Ty::Con("Bool".to_string())));
        let t2 = Ty::Func(
            Box::new(Ty::Con("Integer".to_string())),
            Box::new(b.clone()),
        );

        let subst = unify(t1, t2).unwrap();
        assert_eq!(apply(&subst, a), Ty::Con("Integer".to_string()));
        assert_eq!(apply(&subst, b), Ty::Con("Bool".to_string()));
    }

    #[test]
    fn occurs_check_rejects_recursive() {
        let mut cx = InferCtx::default();
        let a = cx.fresh();
        let t = Ty::List(Box::new(a.clone()));
        assert!(unify(a, t).is_err());
    }
}

#[cfg(test)]
mod inference_tests {
    use super::*;

    #[test]
    fn infer_identity_lambda() {
        let ty = infer_expr(ast::Expr::Lambda {
            params: vec!["x".to_string()],
            body: Box::new(ast::Expr::Var("x".to_string())),
        })
        .unwrap();

        let Ty::Func(a, b) = ty else {
            panic!("expected function");
        };
        let Ty::Var(va) = *a else {
            panic!("expected var");
        };
        let Ty::Var(vb) = *b else {
            panic!("expected var");
        };
        assert_eq!(va, vb);
    }

    #[test]
    fn infer_apply_identity() {
        let id = ast::Expr::Lambda {
            params: vec!["x".to_string()],
            body: Box::new(ast::Expr::Var("x".to_string())),
        };

        let ty = infer_expr(ast::Expr::Apply {
            func: Box::new(id),
            args: vec![ast::Expr::Integer("1".to_string())],
        })
        .unwrap();

        assert_eq!(ty, Ty::Con("Integer".to_string()));
    }

    #[test]
    fn infer_let_generalizes() {
        let id_binding = ast::Binding {
            pat: ast::Pattern::Var("id".to_string()),
            expr: ast::Expr::Lambda {
                params: vec!["x".to_string()],
                body: Box::new(ast::Expr::Var("x".to_string())),
            },
        };

        let body = ast::Expr::Tuple(vec![
            ast::Expr::Apply {
                func: Box::new(ast::Expr::Var("id".to_string())),
                args: vec![ast::Expr::Integer("1".to_string())],
            },
            ast::Expr::Apply {
                func: Box::new(ast::Expr::Var("id".to_string())),
                args: vec![ast::Expr::Bool(true)],
            },
        ]);

        let ty = infer_expr(ast::Expr::Let {
            bindings: vec![id_binding],
            body: Box::new(body),
        })
        .unwrap();

        assert_eq!(
            ty,
            Ty::Tuple(vec![
                Ty::Con("Integer".to_string()),
                Ty::Con("Bool".to_string())
            ])
        );
    }

    #[test]
    fn infer_let_tuple_pattern() {
        let b = ast::Binding {
            pat: ast::Pattern::Tuple(vec![
                ast::Pattern::Var("a".to_string()),
                ast::Pattern::Var("b".to_string()),
            ]),
            expr: ast::Expr::Tuple(vec![
                ast::Expr::Integer("1".to_string()),
                ast::Expr::Bool(true),
            ]),
        };

        let ty = infer_expr(ast::Expr::Let {
            bindings: vec![b],
            body: Box::new(ast::Expr::Var("b".to_string())),
        })
        .unwrap();

        assert_eq!(ty, Ty::Con("Bool".to_string()));
    }

    #[test]
    fn infer_duplicate_pattern_vars_is_error() {
        let b = ast::Binding {
            pat: ast::Pattern::Tuple(vec![
                ast::Pattern::Var("x".to_string()),
                ast::Pattern::Var("x".to_string()),
            ]),
            expr: ast::Expr::Tuple(vec![
                ast::Expr::Integer("1".to_string()),
                ast::Expr::Integer("2".to_string()),
            ]),
        };

        let _ = infer_expr(ast::Expr::Let {
            bindings: vec![b],
            body: Box::new(ast::Expr::Var("x".to_string())),
        })
        .unwrap_err();
    }

    #[test]
    fn infer_let_list_pattern() {
        let b = ast::Binding {
            pat: ast::Pattern::List(vec![
                ast::Pattern::Var("x".to_string()),
                ast::Pattern::Var("y".to_string()),
            ]),
            expr: ast::Expr::List(vec![
                ast::Expr::Integer("1".to_string()),
                ast::Expr::Integer("2".to_string()),
            ]),
        };

        let ty = infer_expr(ast::Expr::Let {
            bindings: vec![b],
            body: Box::new(ast::Expr::Var("y".to_string())),
        })
        .unwrap();

        assert_eq!(ty, Ty::Con("Integer".to_string()));
    }

    #[test]
    fn infer_let_record_pattern() {
        let b = ast::Binding {
            pat: ast::Pattern::Record(vec![
                ("a".to_string(), ast::Pattern::Var("x".to_string())),
                ("b".to_string(), ast::Pattern::Var("y".to_string())),
            ]),
            expr: ast::Expr::Record(vec![
                ("a".to_string(), ast::Expr::Integer("1".to_string())),
                ("b".to_string(), ast::Expr::Bool(true)),
            ]),
        };

        let ty = infer_expr(ast::Expr::Let {
            bindings: vec![b],
            body: Box::new(ast::Expr::Var("y".to_string())),
        })
        .unwrap();

        assert_eq!(ty, Ty::Con("Bool".to_string()));
    }

    #[test]
    fn infer_annotation_mismatch_is_error() {
        let _ = infer_expr(ast::Expr::Annot {
            expr: Box::new(ast::Expr::Integer("1".to_string())),
            ty: ast::Type::Bool,
        })
        .unwrap_err();
    }

    #[test]
    fn infer_annotation_hole_resolves() {
        let ty = infer_expr(ast::Expr::Annot {
            expr: Box::new(ast::Expr::Integer("1".to_string())),
            ty: ast::Type::Hole(None),
        })
        .unwrap();
        assert_eq!(ty, Ty::Con("Integer".to_string()));
    }

    #[test]
    fn infer_if_expr() {
        let ty = infer_expr(ast::Expr::If {
            cond: Box::new(ast::Expr::Bool(true)),
            then_branch: Box::new(ast::Expr::Integer("1".to_string())),
            else_branch: Box::new(ast::Expr::Integer("2".to_string())),
        })
        .unwrap();
        assert_eq!(ty, Ty::Con("Integer".to_string()));
    }

    #[test]
    fn infer_if_mismatch_is_error() {
        let _ = infer_expr(ast::Expr::If {
            cond: Box::new(ast::Expr::Bool(true)),
            then_branch: Box::new(ast::Expr::Integer("1".to_string())),
            else_branch: Box::new(ast::Expr::Bool(false)),
        })
        .unwrap_err();
    }

    #[test]
    fn infer_case_expr() {
        let x_bind = ast::Binding {
            pat: ast::Pattern::Var("x".to_string()),
            expr: ast::Expr::Integer("1".to_string()),
        };

        let ty = infer_expr(ast::Expr::Let {
            bindings: vec![x_bind],
            body: Box::new(ast::Expr::Case {
                expr: Box::new(ast::Expr::Var("x".to_string())),
                arms: vec![
                    (
                        ast::Pattern::Literal(ast::Expr::Integer("0".to_string())),
                        ast::Expr::Bool(true),
                    ),
                    (ast::Pattern::Wildcard, ast::Expr::Bool(false)),
                ],
            }),
        })
        .unwrap();

        assert_eq!(ty, Ty::Con("Bool".to_string()));
    }

    #[test]
    fn infer_case_arm_mismatch_is_error() {
        let x_bind = ast::Binding {
            pat: ast::Pattern::Var("x".to_string()),
            expr: ast::Expr::Integer("1".to_string()),
        };

        let _ = infer_expr(ast::Expr::Let {
            bindings: vec![x_bind],
            body: Box::new(ast::Expr::Case {
                expr: Box::new(ast::Expr::Var("x".to_string())),
                arms: vec![
                    (
                        ast::Pattern::Literal(ast::Expr::Integer("0".to_string())),
                        ast::Expr::Bool(true),
                    ),
                    (ast::Pattern::Wildcard, ast::Expr::Integer("1".to_string())),
                ],
            }),
        })
        .unwrap_err();
    }
}
