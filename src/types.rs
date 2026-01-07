//! Type checking and elaboration scaffolding.
//!
//! Policy (docs):
//! - Surface numeric types: Integer (arbitrary precision) and Float64.
//! - Backend/IR numeric types are LLVM-aligned (i32/i64/f32/f64...).
//! - Pure IR subtyping allows only integer widening (iN <: iM); float widening is NOT subtyping.
//! - Potentially lossy conversions happen only at boundaries as checked casts.

use crate::{ast, error::Error, Result};
use std::collections::{HashMap, HashSet};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedModule {
    pub module: ast::Module,
    pub inferred: HashMap<String, Scheme>,
}

// --- Milestone 2.2.1: Unification core (scaffolding) ---

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    Var(u32),
    Con(String),
    List(Box<Ty>),
    Tuple(Vec<Ty>),
    Record(Vec<(String, Ty)>),
    /// Open record (required fields only). Used for `{..., ...}` pattern matching.
    RecordOpen(Vec<(String, Ty)>),
    App { head: Box<Ty>, args: Vec<Ty> },
    Func(Box<Ty>, Box<Ty>),
}

fn fmt_ty_prec(
    f: &mut fmt::Formatter<'_>,
    ty: &Ty,
    parent_prec: u8,
    vars: &HashMap<u32, String>,
) -> fmt::Result {
    const PREC_FUNC: u8 = 0;
    const PREC_APP: u8 = 1;
    const PREC_ATOM: u8 = 2;

    match ty {
        Ty::Var(v) => match vars.get(v) {
            Some(name) => write!(f, "{name}"),
            None => write!(f, "t{v}"),
        },
        Ty::Con(name) => write!(f, "{name}"),
        Ty::List(t) => {
            write!(f, "[")?;
            fmt_ty_prec(f, t, PREC_FUNC, vars)?;
            write!(f, "]")
        }
        Ty::Tuple(ts) => {
            write!(f, "(")?;
            for (i, t) in ts.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                fmt_ty_prec(f, t, PREC_FUNC, vars)?;
            }
            write!(f, ")")
        }
        Ty::Record(fields) => {
            write!(f, "{{")?;
            for (i, (k, t)) in fields.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{k}: ")?;
                fmt_ty_prec(f, t, PREC_FUNC, vars)?;
            }
            write!(f, "}}")
        }
        Ty::RecordOpen(fields) => {
            write!(f, "{{")?;
            for (i, (k, t)) in fields.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{k}: ")?;
                fmt_ty_prec(f, t, PREC_FUNC, vars)?;
            }
            if !fields.is_empty() {
                write!(f, ", ")?;
            }
            write!(f, "...}}")
        }
        Ty::App { head, args } => {
            if parent_prec > PREC_APP {
                write!(f, "(")?;
            }
            fmt_ty_prec(f, head, PREC_ATOM, vars)?;
            for a in args {
                write!(f, " ")?;
                fmt_ty_prec(f, a, PREC_ATOM, vars)?;
            }
            if parent_prec > PREC_APP {
                write!(f, ")")?;
            }
            Ok(())
        }
        Ty::Func(a, b) => {
            if parent_prec > PREC_FUNC {
                write!(f, "(")?;
            }
            fmt_ty_prec(f, a, PREC_APP, vars)?;
            write!(f, " -> ")?;
            fmt_ty_prec(f, b, PREC_FUNC, vars)?;
            if parent_prec > PREC_FUNC {
                write!(f, ")")?;
            }
            Ok(())
        }
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_ty_prec(f, self, 0, &HashMap::new())
    }
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

            let a: HashMap<String, Ty> = a.into_iter().collect();
            let b: HashMap<String, Ty> = b.into_iter().collect();

            if a.len() != b.len() {
                return Err(Error::msg("record field mismatch"));
            }

            for (k, ta) in a {
                let tb = b
                    .get(&k)
                    .ok_or_else(|| Error::msg("record field mismatch"))?;
                unify_in(subst, ta, tb.clone())?;
            }

            Ok(())
        }
        (Ty::RecordOpen(req), Ty::Record(actual)) | (Ty::Record(actual), Ty::RecordOpen(req)) => {
            let actual: HashMap<String, Ty> = actual.into_iter().collect();
            for (k, t_req) in req {
                let t_act = actual
                    .get(&k)
                    .ok_or_else(|| Error::msg("record field mismatch"))?;
                unify_in(subst, t_req, t_act.clone())?;
            }
            Ok(())
        }
        (Ty::RecordOpen(a), Ty::RecordOpen(b)) => {
            let a: HashMap<String, Ty> = a.into_iter().collect();
            let b: HashMap<String, Ty> = b.into_iter().collect();
            for (k, ta) in a {
                if let Some(tb) = b.get(&k) {
                    unify_in(subst, ta, tb.clone())?;
                }
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
        Ty::Record(fields) | Ty::RecordOpen(fields) => {
            fields.iter().any(|(_, t)| occurs_in(subst, seen, v, t))
        }
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
        Ty::RecordOpen(fields) => Ty::RecordOpen(
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
pub enum Constraint {
    Show(Ty),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scheme {
    pub vars: Vec<u32>,
    pub constraints: Vec<Constraint>,
    pub ty: Ty,
}

impl Scheme {
    pub fn mono(ty: Ty) -> Self {
        Self {
            vars: vec![],
            constraints: vec![],
            ty,
        }
    }
}

impl fmt::Display for Scheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.vars.is_empty() {
            return fmt_ty_prec(f, &self.ty, 0, &HashMap::new());
        }

        let mut vs = self.vars.clone();
        vs.sort_unstable();

        let mut names = HashMap::new();
        for (i, v) in vs.iter().enumerate() {
            let name = if i < 26 {
                ((b'a' + i as u8) as char).to_string()
            } else {
                format!("a{i}")
            };
            names.insert(*v, name);
        }

        write!(f, "forall")?;
        for v in &vs {
            write!(f, " {}", names.get(v).expect("missing var name"))?;
        }
        write!(f, ". ")?;
        fmt_ty_prec(f, &self.ty, 0, &names)
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
        Ty::Record(fields) | Ty::RecordOpen(fields) => {
            fields.iter().flat_map(|(_, t)| ftv_ty(t)).collect()
        }
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

fn ftv_constraint(c: &Constraint) -> HashSet<u32> {
    match c {
        Constraint::Show(t) => ftv_ty(t),
    }
}

pub fn ftv_scheme(s: &Scheme) -> HashSet<u32> {
    let mut ftv = ftv_ty(&s.ty);
    for c in &s.constraints {
        ftv.extend(ftv_constraint(c));
    }
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
    Scheme {
        vars,
        constraints: vec![],
        ty,
    }
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
        Ty::RecordOpen(fields) => Ty::RecordOpen(
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

fn apply_constraint(subst: &Subst, c: &Constraint) -> Constraint {
    match c {
        Constraint::Show(t) => Constraint::Show(apply(subst, t.clone())),
    }
}

fn apply_scheme(subst: &Subst, s: &Scheme) -> Scheme {
    if s.vars.is_empty() {
        return Scheme {
            vars: vec![],
            constraints: s
                .constraints
                .iter()
                .map(|c| apply_constraint(subst, c))
                .collect(),
            ty: apply(subst, s.ty.clone()),
        };
    }

    let mut sub = subst.clone();
    for v in &s.vars {
        sub.remove(v);
    }

    Scheme {
        vars: s.vars.clone(),
        constraints: s
            .constraints
            .iter()
            .map(|c| apply_constraint(&sub, c))
            .collect(),
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
    env: &TypeEnv,
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
        Pattern::Hole(_) => Ok(cx.fresh()),
        Pattern::Literal(e) => Ok(match e {
            Expr::Unit => Ty::Con("Unit".to_string()),
            Expr::Integer(_) => Ty::Con("Integer".to_string()),
            Expr::Float64(_) => Ty::Con("Float64".to_string()),
            Expr::Bool(_) => Ty::Con("Bool".to_string()),
            Expr::String(_) => Ty::Con("String".to_string()),
            Expr::Char(_) => Ty::Con("Char".to_string()),
            _ => return Err(Error::msg("unsupported literal pattern")),
        }),
        Pattern::Tuple(ps) => Ok(Ty::Tuple(
            ps.iter()
                .map(|p| infer_pat_in(cx, subst, env, p, binds, seen))
                .collect::<Result<Vec<_>>>()?,
        )),
        Pattern::List(ps) => {
            if ps.is_empty() {
                return Ok(Ty::List(Box::new(cx.fresh())));
            }

            let first = infer_pat_in(cx, subst, env, &ps[0], binds, seen)?;
            for p in &ps[1..] {
                let t = infer_pat_in(cx, subst, env, p, binds, seen)?;
                let su = unify(apply(subst, first.clone()), apply(subst, t))?;
                *subst = compose(&su, subst);
            }
            Ok(Ty::List(Box::new(apply(subst, first))))
        }
        Pattern::Record(fields) => {
            let mut out = fields
                .iter()
                .map(|(n, p)| Ok((n.clone(), infer_pat_in(cx, subst, env, p, binds, seen)?)))
                .collect::<Result<Vec<_>>>()?;
            out.sort_by(|(a, _), (b, _)| a.cmp(b));
            Ok(Ty::Record(out))
        }
        Pattern::RecordLoose(fields) => {
            let mut out = fields
                .iter()
                .map(|(n, p)| Ok((n.clone(), infer_pat_in(cx, subst, env, p, binds, seen)?)))
                .collect::<Result<Vec<_>>>()?;
            out.sort_by(|(a, _), (b, _)| a.cmp(b));
            Ok(Ty::RecordOpen(out))
        }
        Pattern::Cons(hd, tl) => {
            let elem = cx.fresh();
            let t_hd = infer_pat_in(cx, subst, env, hd, binds, seen)?;
            let t_tl = infer_pat_in(cx, subst, env, tl, binds, seen)?;

            let su_hd = unify(apply(subst, t_hd), apply(subst, elem.clone()))?;
            *subst = compose(&su_hd, subst);

            let su_tl = unify(
                apply(subst, t_tl),
                apply(subst, Ty::List(Box::new(elem.clone()))),
            )?;
            *subst = compose(&su_tl, subst);

            Ok(apply(subst, Ty::List(Box::new(elem))))
        }
        Pattern::Or(a, b) => {
            let base_len = binds.len();
            let base_seen = seen.clone();
            let base_binds = binds.clone();

            let mut binds_a = base_binds.clone();
            let mut seen_a = base_seen.clone();
            let t_a = infer_pat_in(cx, subst, env, a, &mut binds_a, &mut seen_a)?;

            let mut binds_b = base_binds;
            let mut seen_b = base_seen;
            let t_b = infer_pat_in(cx, subst, env, b, &mut binds_b, &mut seen_b)?;

            let su_t = unify(apply(subst, t_a.clone()), apply(subst, t_b.clone()))?;
            *subst = compose(&su_t, subst);

            let map_a: HashMap<String, Ty> = binds_a[base_len..]
                .iter()
                .map(|(n, t)| (n.clone(), t.clone()))
                .collect();
            let map_b: HashMap<String, Ty> = binds_b[base_len..]
                .iter()
                .map(|(n, t)| (n.clone(), t.clone()))
                .collect();

            if map_a.len() != map_b.len() || map_a.keys().any(|k| !map_b.contains_key(k)) {
                return Err(Error::msg("or-pattern must bind the same variables"));
            }

            let mut names: Vec<_> = map_a.keys().cloned().collect();
            names.sort();
            for n in names {
                let ta = map_a.get(&n).unwrap().clone();
                let tb = map_b.get(&n).unwrap().clone();
                let su = unify(apply(subst, ta.clone()), apply(subst, tb))?;
                *subst = compose(&su, subst);
                let _ = seen.insert(n.clone());
                binds.push((n, apply(subst, ta)));
            }

            Ok(apply(subst, t_a))
        }
        Pattern::As(name, p) => {
            if !seen.insert(name.clone()) {
                return Err(Error::msg("duplicate pattern variable"));
            }
            let t = infer_pat_in(cx, subst, env, p, binds, seen)?;
            binds.push((name.clone(), apply(subst, t.clone())));
            Ok(t)
        }
        Pattern::View(p, e) => {
            let t_scrut = cx.fresh();
            let t_view = infer_pat_in(cx, subst, env, p, binds, seen)?;

            let env_in = apply_env(subst, env);
            let (s_e, t_e) = infer_expr_in(cx, &env_in, (**e).clone())?;
            *subst = compose(&s_e, subst);

            let su = unify(
                apply(subst, t_e),
                Ty::Func(
                    Box::new(apply(subst, t_scrut.clone())),
                    Box::new(apply(subst, t_view)),
                ),
            )?;
            *subst = compose(&su, subst);

            Ok(apply(subst, t_scrut))
        }
        Pattern::Constructor { name, args } => {
            let scheme = env
                .get(name)
                .ok_or_else(|| Error::msg("unknown constructor"))?;
            let mut ctor_ty = instantiate(cx, scheme);

            for p in args {
                let arg_pat_ty = infer_pat_in(cx, subst, env, p, binds, seen)?;
                let res = cx.fresh();

                let su = unify(
                    apply(subst, ctor_ty),
                    Ty::Func(Box::new(apply(subst, arg_pat_ty)), Box::new(res.clone())),
                )?;
                *subst = compose(&su, subst);
                ctor_ty = res;
            }

            Ok(apply(subst, ctor_ty))
        }
    }
}

pub fn infer_expr(expr: ast::Expr) -> Result<Ty> {
    let mut cx = InferCtx::default();
    let env = TypeEnv::new();
    let (s, t) = infer_expr_in(&mut cx, &env, expr)?;
    Ok(apply(&s, t))
}

pub fn infer_in_module(module: &ast::Module, expr: ast::Expr) -> Result<Ty> {
    let mut cx = InferCtx::default();
    let env = collect_ctor_env(&mut cx, module)?;
    let (s, t) = infer_expr_in(&mut cx, &env, expr)?;
    Ok(apply(&s, t))
}

pub fn infer_module(module: &ast::Module) -> Result<HashMap<String, Scheme>> {
    let mut cx = InferCtx::default();
    let mut env = collect_ctor_env(&mut cx, module)?;
    let mut subst = Subst::new();
    let mut out = HashMap::new();

    for it in &module.items {
        let ast::Item::Binding(b) = it else {
            continue;
        };

        let ctx_name = match &b.pat {
            ast::Pattern::Var(n) => n.as_str(),
            _ => "<pattern>",
        };

        let mut binds = Vec::new();
        let mut seen = HashSet::new();
        let pat_ty = infer_pat_in(&mut cx, &mut subst, &env, &b.pat, &mut binds, &mut seen)
            .map_err(|e| Error::msg(format!("in binding {ctx_name}: {e}")))?;

        let env_in = apply_env(&subst, &env);
        let (s_rhs, t_rhs) = infer_expr_in(&mut cx, &env_in, b.expr.clone())
            .map_err(|e| Error::msg(format!("in binding {ctx_name}: {e}")))?;
        subst = compose(&s_rhs, &subst);

        let s_pat = unify(apply(&subst, t_rhs), apply(&subst, pat_ty))
            .map_err(|e| Error::msg(format!("in binding {ctx_name}: {e}")))?;
        subst = compose(&s_pat, &subst);

        for (name, t) in binds {
            let env_gen = apply_env(&subst, &env);
            let scheme = generalize(&env_gen, apply(&subst, t));
            env.insert(name.clone(), scheme.clone());
            out.insert(name, scheme);
        }
    }

    Ok(out)
}

fn collect_ctor_env(cx: &mut InferCtx, module: &ast::Module) -> Result<TypeEnv> {
    let mut env = TypeEnv::new();

    // Minimal prelude:
    //   IO :: forall a. a -> IO a
    // This lets `do` blocks typecheck without requiring an explicit `data IO a = ...` in every module.
    let Ty::Var(a) = cx.fresh() else {
        unreachable!()
    };
    env.insert(
        "IO".to_string(),
        Scheme {
            vars: vec![a],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Var(a)),
                Box::new(Ty::App {
                    head: Box::new(Ty::Con("IO".to_string())),
                    args: vec![Ty::Var(a)],
                }),
            ),
        },
    );

    // concatMap :: forall a b. (a -> [b]) -> [a] -> [b]
    let Ty::Var(a) = cx.fresh() else {
        unreachable!()
    };
    let Ty::Var(b) = cx.fresh() else {
        unreachable!()
    };
    env.insert(
        "concatMap".to_string(),
        Scheme {
            vars: vec![a, b],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Func(
                    Box::new(Ty::Var(a)),
                    Box::new(Ty::List(Box::new(Ty::Var(b)))),
                )),
                Box::new(Ty::Func(
                    Box::new(Ty::List(Box::new(Ty::Var(a)))),
                    Box::new(Ty::List(Box::new(Ty::Var(b)))),
                )),
            ),
        },
    );

    // + :: Integer -> Integer -> Integer
    env.insert(
        "+".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Con("Integer".to_string())),
                Box::new(Ty::Func(
                    Box::new(Ty::Con("Integer".to_string())),
                    Box::new(Ty::Con("Integer".to_string())),
                )),
            ),
        },
    );

    // - :: Integer -> Integer -> Integer
    env.insert(
        "-".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Con("Integer".to_string())),
                Box::new(Ty::Func(
                    Box::new(Ty::Con("Integer".to_string())),
                    Box::new(Ty::Con("Integer".to_string())),
                )),
            ),
        },
    );

    // * :: Integer -> Integer -> Integer
    env.insert(
        "*".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Con("Integer".to_string())),
                Box::new(Ty::Func(
                    Box::new(Ty::Con("Integer".to_string())),
                    Box::new(Ty::Con("Integer".to_string())),
                )),
            ),
        },
    );

    // / :: Integer -> Integer -> Integer
    env.insert(
        "/".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Con("Integer".to_string())),
                Box::new(Ty::Func(
                    Box::new(Ty::Con("Integer".to_string())),
                    Box::new(Ty::Con("Integer".to_string())),
                )),
            ),
        },
    );

    // == :: Integer -> Integer -> Bool
    env.insert(
        "==".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Con("Integer".to_string())),
                Box::new(Ty::Func(
                    Box::new(Ty::Con("Integer".to_string())),
                    Box::new(Ty::Con("Bool".to_string())),
                )),
            ),
        },
    );

    // < :: Integer -> Integer -> Bool
    env.insert(
        "<".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Con("Integer".to_string())),
                Box::new(Ty::Func(
                    Box::new(Ty::Con("Integer".to_string())),
                    Box::new(Ty::Con("Bool".to_string())),
                )),
            ),
        },
    );

    // <= :: Integer -> Integer -> Bool
    env.insert(
        "<=".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Con("Integer".to_string())),
                Box::new(Ty::Func(
                    Box::new(Ty::Con("Integer".to_string())),
                    Box::new(Ty::Con("Bool".to_string())),
                )),
            ),
        },
    );

    // > :: Integer -> Integer -> Bool
    env.insert(
        ">".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Con("Integer".to_string())),
                Box::new(Ty::Func(
                    Box::new(Ty::Con("Integer".to_string())),
                    Box::new(Ty::Con("Bool".to_string())),
                )),
            ),
        },
    );

    // >= :: Integer -> Integer -> Bool
    env.insert(
        ">=".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Con("Integer".to_string())),
                Box::new(Ty::Func(
                    Box::new(Ty::Con("Integer".to_string())),
                    Box::new(Ty::Con("Bool".to_string())),
                )),
            ),
        },
    );

    // /= :: Integer -> Integer -> Bool
    env.insert(
        "/=".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Con("Integer".to_string())),
                Box::new(Ty::Func(
                    Box::new(Ty::Con("Integer".to_string())),
                    Box::new(Ty::Con("Bool".to_string())),
                )),
            ),
        },
    );

    // && :: Bool -> Bool -> Bool
    env.insert(
        "&&".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Con("Bool".to_string())),
                Box::new(Ty::Func(
                    Box::new(Ty::Con("Bool".to_string())),
                    Box::new(Ty::Con("Bool".to_string())),
                )),
            ),
        },
    );

    // || :: Bool -> Bool -> Bool
    env.insert(
        "||".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Con("Bool".to_string())),
                Box::new(Ty::Func(
                    Box::new(Ty::Con("Bool".to_string())),
                    Box::new(Ty::Con("Bool".to_string())),
                )),
            ),
        },
    );

    // not :: Bool -> Bool
    env.insert(
        "not".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Con("Bool".to_string())),
                Box::new(Ty::Con("Bool".to_string())),
            ),
        },
    );

    // intToString :: Integer -> String
    env.insert(
        "intToString".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Con("Integer".to_string())),
                Box::new(Ty::Con("String".to_string())),
            ),
        },
    );

    // boolToString :: Bool -> String
    env.insert(
        "boolToString".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Con("Bool".to_string())),
                Box::new(Ty::Con("String".to_string())),
            ),
        },
    );

    // show :: a -> String
    let Ty::Var(v) = cx.fresh() else { unreachable!() };
    env.insert(
        "show".to_string(),
        Scheme {
            vars: vec![v],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Var(v)),
                Box::new(Ty::Con("String".to_string())),
            ),
        },
    );

    // toString :: a -> String
    let Ty::Var(v) = cx.fresh() else { unreachable!() };
    env.insert(
        "toString".to_string(),
        Scheme {
            vars: vec![v],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Var(v)),
                Box::new(Ty::Con("String".to_string())),
            ),
        },
    );

    // stdoutWrite :: String -> IO Unit
    // Low-level IO primitive used as a building block for higher-level IO.
    env.insert(
        "stdoutWrite".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Con("String".to_string())),
                Box::new(Ty::App {
                    head: Box::new(Ty::Con("IO".to_string())),
                    args: vec![Ty::Con("Unit".to_string())],
                }),
            ),
        },
    );

    // stdinReadLine :: IO String
    // Low-level IO primitive used as a building block for higher-level IO.
    env.insert(
        "stdinReadLine".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::App {
                head: Box::new(Ty::Con("IO".to_string())),
                args: vec![Ty::Con("String".to_string())],
            },
        },
    );

    // readLine :: IO String
    // NOTE: currently a builtin for early ergonomics.
    // In the future, `readLine` should become a library function built on top of IO primitives
    // such as `stdinReadLine`.
    env.insert(
        "readLine".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::App {
                head: Box::new(Ty::Con("IO".to_string())),
                args: vec![Ty::Con("String".to_string())],
            },
        },
    );

    // print :: String -> IO Unit
    // NOTE: currently a builtin for observability.
    // In the future, `print` should become a library function built on top of IO primitives
    // such as `stdoutWrite`.
    env.insert(
        "print".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Con("String".to_string())),
                Box::new(Ty::App {
                    head: Box::new(Ty::Con("IO".to_string())),
                    args: vec![Ty::Con("Unit".to_string())],
                }),
            ),
        },
    );

    for it in &module.items {
        let ast::Item::DataDecl(d) = it else {
            continue;
        };

        let mut param_vars: HashMap<String, u32> = HashMap::new();
        for p in &d.params {
            let Ty::Var(v) = cx.fresh() else {
                unreachable!()
            };
            param_vars.insert(p.clone(), v);
        }

        let result_ty = if d.params.is_empty() {
            Ty::Con(d.name.clone())
        } else {
            Ty::App {
                head: Box::new(Ty::Con(d.name.clone())),
                args: d
                    .params
                    .iter()
                    .map(|p| Ty::Var(*param_vars.get(p).unwrap()))
                    .collect(),
            }
        };

        for ctor in &d.ctors {
            let mut holes = HashMap::new();
            let arg_tys: Vec<Ty> = ctor
                .args
                .iter()
                .map(|t| lower_surface_type_with_params(cx, t, &mut holes, &param_vars))
                .collect();

            let mut ty = result_ty.clone();
            for a in arg_tys.into_iter().rev() {
                ty = Ty::Func(Box::new(a), Box::new(ty));
            }

            let mut vars: Vec<u32> = ftv_ty(&ty).into_iter().collect();
            vars.sort_unstable();
            env.insert(ctor.name.clone(), Scheme { vars, constraints: vec![], ty });
        }
    }

    Ok(env)
}

fn lower_surface_type_with_params(
    cx: &mut InferCtx,
    ty: &ast::Type,
    holes: &mut HashMap<String, Ty>,
    params: &HashMap<String, u32>,
) -> Ty {
    use ast::Type;

    match ty {
        Type::Var(name) => params
            .get(name)
            .map(|v| Ty::Var(*v))
            .unwrap_or_else(|| Ty::Con(name.clone())),
        other => lower_surface_type(cx, other, holes),
    }
}

fn infer_expr_in(cx: &mut InferCtx, env: &TypeEnv, expr: ast::Expr) -> Result<(Subst, Ty)> {
    use ast::Expr;

    match expr {
        Expr::Unit => Ok((Subst::new(), Ty::Con("Unit".to_string()))),
        Expr::Integer(_) => Ok((Subst::new(), Ty::Con("Integer".to_string()))),
        Expr::Float64(_) => Ok((Subst::new(), Ty::Con("Float64".to_string()))),
        Expr::Bool(true) | Expr::Bool(false) => Ok((Subst::new(), Ty::Con("Bool".to_string()))),
        Expr::String(_) => Ok((Subst::new(), Ty::Con("String".to_string()))),
        Expr::Char(_) => Ok((Subst::new(), Ty::Con("Char".to_string()))),

        Expr::Var(name) => {
            let s = env
                .get(&name)
                .ok_or_else(|| Error::msg(format!("unbound variable: {name}")))?;
            Ok((Subst::new(), instantiate(cx, s)))
        }

        Expr::Ctor(name) => {
            let s = env
                .get(&name)
                .ok_or_else(|| Error::msg("unknown constructor"))?;
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
            let (s_cond, t_cond) = infer_expr_in(cx, env, *cond)
                .map_err(|e| Error::msg(format!("in if cond: {e}")))?;
            let s_bool = unify(apply(&s_cond, t_cond), Ty::Con("Bool".to_string()))
                .map_err(|e| Error::msg(format!("in if cond: {e}")))?;
            let mut s = compose(&s_bool, &s_cond);

            let env2 = apply_env(&s, env);
            let (s_then, t_then) = infer_expr_in(cx, &env2, *then_branch)
                .map_err(|e| Error::msg(format!("in if then: {e}")))?;
            s = compose(&s_then, &s);

            let env3 = apply_env(&s, env);
            let (s_else, t_else) = infer_expr_in(cx, &env3, *else_branch)
                .map_err(|e| Error::msg(format!("in if else: {e}")))?;
            s = compose(&s_else, &s);

            let s_res = unify(apply(&s, t_then.clone()), apply(&s, t_else))
                .map_err(|e| Error::msg(format!("in if branches: {e}")))?;
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

        Expr::Cons { head, tail } => {
            let (s_hd, t_hd) = infer_expr_in(cx, env, *head)?;
            let env2 = apply_env(&s_hd, env);
            let (s_tl, t_tl) = infer_expr_in(cx, &env2, *tail)?;
            let mut s = compose(&s_tl, &s_hd);

            let elem = cx.fresh();
            let su_tl = unify(apply(&s, t_tl), Ty::List(Box::new(elem.clone())))?;
            s = compose(&su_tl, &s);
            let su_hd = unify(apply(&s, t_hd), apply(&s, elem.clone()))?;
            s = compose(&su_hd, &s);

            Ok((s.clone(), Ty::List(Box::new(apply(&s, elem)))))
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
            out.sort_by(|(a, _), (b, _)| a.cmp(b));
            Ok((s, Ty::Record(out)))
        }

        Expr::Let { bindings, body } => {
            let mut s = Subst::new();
            let mut env2 = env.clone();

            for b in bindings {
                let ctx_name = match &b.pat {
                    ast::Pattern::Var(n) => n.as_str(),
                    _ => "<pattern>",
                };

                let mut binds = Vec::new();
                let mut seen = HashSet::new();
                let pat_ty = infer_pat_in(cx, &mut s, &env2, &b.pat, &mut binds, &mut seen)
                    .map_err(|e| Error::msg(format!("in let binding {ctx_name}: {e}")))?;

                let env_in = apply_env(&s, &env2);
                let (s_rhs, t_rhs) = infer_expr_in(cx, &env_in, b.expr)
                    .map_err(|e| Error::msg(format!("in let binding {ctx_name}: {e}")))?;
                s = compose(&s_rhs, &s);

                let s_pat = unify(apply(&s, t_rhs), apply(&s, pat_ty))
                    .map_err(|e| Error::msg(format!("in let binding {ctx_name}: {e}")))?;
                s = compose(&s_pat, &s);

                for (name, t) in binds {
                    let env_gen = apply_env(&s, &env2);
                    let scheme = generalize(&env_gen, apply(&s, t));
                    env2.insert(name, scheme);
                }
            }

            let env_body = apply_env(&s, &env2);
            let (s_body, t_body) = infer_expr_in(cx, &env_body, *body)
                .map_err(|e| Error::msg(format!("in let body: {e}")))?;
            let s = compose(&s_body, &s);
            Ok((s.clone(), apply(&s, t_body)))
        }

        Expr::Where { expr, bindings } => {
            let mut s = Subst::new();
            let mut env2 = env.clone();

            for b in bindings {
                let ctx_name = match &b.pat {
                    ast::Pattern::Var(n) => n.as_str(),
                    _ => "<pattern>",
                };

                let mut binds = Vec::new();
                let mut seen = HashSet::new();
                let pat_ty = infer_pat_in(cx, &mut s, &env2, &b.pat, &mut binds, &mut seen)
                    .map_err(|e| Error::msg(format!("in where binding {ctx_name}: {e}")))?;

                let env_in = apply_env(&s, &env2);
                let (s_rhs, t_rhs) = infer_expr_in(cx, &env_in, b.expr)
                    .map_err(|e| Error::msg(format!("in where binding {ctx_name}: {e}")))?;
                s = compose(&s_rhs, &s);

                let s_pat = unify(apply(&s, t_rhs), apply(&s, pat_ty))
                    .map_err(|e| Error::msg(format!("in where binding {ctx_name}: {e}")))?;
                s = compose(&s_pat, &s);

                for (name, t) in binds {
                    let env_gen = apply_env(&s, &env2);
                    let scheme = generalize(&env_gen, apply(&s, t));
                    env2.insert(name, scheme);
                }
            }

            let env_body = apply_env(&s, &env2);
            let (s_body, t_body) = infer_expr_in(cx, &env_body, *expr)
                .map_err(|e| Error::msg(format!("in where body: {e}")))?;
            let s = compose(&s_body, &s);
            Ok((s.clone(), apply(&s, t_body)))
        }

        Expr::Case { expr, arms } => {
            if arms.is_empty() {
                return Err(Error::msg("empty case"));
            }

            let (mut s, scrut_ty) = infer_expr_in(cx, env, *expr)
                .map_err(|e| Error::msg(format!("in case scrutinee: {e}")))?;
            let mut out_ty = cx.fresh();

            for (i, arm) in arms.into_iter().enumerate() {
                let arm_no = i + 1;
                let ast::CaseArm { pat, guard, body } = arm;

                let mut binds = Vec::new();
                let mut seen = HashSet::new();
                let pat_ty = infer_pat_in(cx, &mut s, env, &pat, &mut binds, &mut seen)
                    .map_err(|e| Error::msg(format!("in case arm {arm_no}: {e}")))?;

                let su_pat = unify(apply(&s, pat_ty), apply(&s, scrut_ty.clone()))
                    .map_err(|e| Error::msg(format!("in case arm {arm_no}: {e}")))?;
                s = compose(&su_pat, &s);

                let mut env_arm = apply_env(&s, env);
                for (name, t) in binds {
                    env_arm.insert(name, Scheme::mono(apply(&s, t)));
                }

                if let Some(g) = guard {
                    let (s_g, t_g) = infer_expr_in(cx, &env_arm, g)
                        .map_err(|e| Error::msg(format!("in case arm {arm_no} guard: {e}")))?;
                    s = compose(&s_g, &s);
                    let su_g = unify(apply(&s, t_g), Ty::Con("Bool".to_string()))
                        .map_err(|e| Error::msg(format!("in case arm {arm_no} guard: {e}")))?;
                    s = compose(&su_g, &s);
                    env_arm = apply_env(&s, &env_arm);
                }

                let (s_arm, arm_ty) = infer_expr_in(cx, &env_arm, body)
                    .map_err(|e| Error::msg(format!("in case arm {arm_no}: {e}")))?;
                s = compose(&s_arm, &s);

                let su_out = unify(apply(&s, out_ty.clone()), apply(&s, arm_ty))
                    .map_err(|e| Error::msg(format!("in case arm {arm_no}: {e}")))?;
                s = compose(&su_out, &s);
                out_ty = apply(&s, out_ty);
            }

            Ok((s.clone(), apply(&s, out_ty)))
        }

        Expr::Do(stmts) => {
            if stmts.is_empty() {
                return Err(Error::msg("empty do"));
            }

            let n = stmts.len();
            let mut s = Subst::new();
            let mut env2 = env.clone();

            let mut last_ty: Option<Ty> = None;

            for (i, stmt) in stmts.into_iter().enumerate() {
                let stmt_no = i + 1;
                let is_last = stmt_no == n;
                match stmt {
                    ast::DoStmt::Bind { pat, expr } => {
                        let env_in = apply_env(&s, &env2);
                        let (s_e, t_e) = infer_expr_in(cx, &env_in, expr)
                            .map_err(|e| Error::msg(format!("in do stmt {stmt_no} (<-): {e}")))?;
                        s = compose(&s_e, &s);

                        let a = cx.fresh();
                        let io_a = Ty::App {
                            head: Box::new(Ty::Con("IO".to_string())),
                            args: vec![a.clone()],
                        };
                        let su = unify(apply(&s, t_e), apply(&s, io_a))
                            .map_err(|e| Error::msg(format!("in do stmt {stmt_no} (<-): {e}")))?;
                        s = compose(&su, &s);

                        let mut binds = Vec::new();
                        let mut seen = HashSet::new();
                        let pat_ty = infer_pat_in(cx, &mut s, &env2, &pat, &mut binds, &mut seen)
                            .map_err(|e| Error::msg(format!("in do stmt {stmt_no} (<-): {e}")))?;
                        let su_pat = unify(apply(&s, pat_ty), apply(&s, a))
                            .map_err(|e| Error::msg(format!("in do stmt {stmt_no} (<-): {e}")))?;
                        s = compose(&su_pat, &s);

                        for (name, t) in binds {
                            env2.insert(name, Scheme::mono(apply(&s, t)));
                        }
                        last_ty = None;
                    }
                    ast::DoStmt::Expr(e) => {
                        let env_in = apply_env(&s, &env2);
                        let (s_e, t_e) = infer_expr_in(cx, &env_in, e)
                            .map_err(|e| Error::msg(format!("in do stmt {stmt_no}: {e}")))?;
                        s = compose(&s_e, &s);

                        let r = cx.fresh();
                        let io_r = Ty::App {
                            head: Box::new(Ty::Con("IO".to_string())),
                            args: vec![r.clone()],
                        };
                        let su = unify(apply(&s, t_e), apply(&s, io_r.clone()))
                            .map_err(|e| Error::msg(format!("in do stmt {stmt_no}: {e}")))?;
                        s = compose(&su, &s);

                        if is_last {
                            last_ty = Some(apply(&s, io_r));
                        } else {
                            last_ty = None;
                        }
                    }
                }
            }

            let last_ty = last_ty.ok_or_else(|| Error::msg("do must end with expression"))?;
            Ok((s.clone(), apply(&s, last_ty)))
        }
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
    if module
        .items
        .iter()
        .any(|it| matches!(it, ast::Item::Import(_)))
    {
        return Err(Error::msg("imports are not supported yet"));
    }

    let aliases = collect_type_aliases(&module);
    module.items = module
        .items
        .into_iter()
        .map(|it| expand_item(it, &aliases))
        .collect::<Result<Vec<_>>>()?;

    let inferred = infer_module(&module)?;

    if let Some(main) = inferred.get("main") {
        let expected = Ty::App {
            head: Box::new(Ty::Con("IO".to_string())),
            args: vec![Ty::Con("Unit".to_string())],
        };
        if !main.vars.is_empty() || main.ty != expected {
            return Err(Error::msg("main must have type IO Unit"));
        }
    }

    Ok(TypedModule { module, inferred })
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
            pat: expand_pat(b.pat, aliases)?,
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

fn expand_pat(pat: ast::Pattern, aliases: &HashMap<String, ast::TypeAlias>) -> Result<ast::Pattern> {
    use ast::Pattern;
    Ok(match pat {
        Pattern::Var(_) | Pattern::Wildcard | Pattern::Hole(_) | Pattern::Literal(_) => pat,
        Pattern::Tuple(ps) => Pattern::Tuple(
            ps.into_iter()
                .map(|p| expand_pat(p, aliases))
                .collect::<Result<Vec<_>>>()?,
        ),
        Pattern::List(ps) => Pattern::List(
            ps.into_iter()
                .map(|p| expand_pat(p, aliases))
                .collect::<Result<Vec<_>>>()?,
        ),
        Pattern::Record(fields) => Pattern::Record(
            fields
                .into_iter()
                .map(|(n, p)| Ok((n, expand_pat(p, aliases)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
        Pattern::RecordLoose(fields) => Pattern::RecordLoose(
            fields
                .into_iter()
                .map(|(n, p)| Ok((n, expand_pat(p, aliases)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
        Pattern::Cons(a, b) => Pattern::Cons(
            Box::new(expand_pat(*a, aliases)?),
            Box::new(expand_pat(*b, aliases)?),
        ),
        Pattern::Or(a, b) => Pattern::Or(
            Box::new(expand_pat(*a, aliases)?),
            Box::new(expand_pat(*b, aliases)?),
        ),
        Pattern::As(name, p) => Pattern::As(name, Box::new(expand_pat(*p, aliases)?)), 
        Pattern::View(p, e) => Pattern::View(
            Box::new(expand_pat(*p, aliases)?),
            Box::new(expand_expr(*e, aliases)?),
        ),
        Pattern::Constructor { name, args } => Pattern::Constructor {
            name,
            args: args
                .into_iter()
                .map(|p| expand_pat(p, aliases))
                .collect::<Result<Vec<_>>>()?,
        },
    })
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
                        pat: expand_pat(b.pat, aliases)?,
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
                        pat: expand_pat(b.pat, aliases)?,
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
                        ast::DoStmt::Bind { pat, expr } => ast::DoStmt::Bind {
                            pat: expand_pat(pat, aliases)?,
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
                .map(|a| {
                    Ok(ast::CaseArm {
                        pat: expand_pat(a.pat, aliases)?,
                        guard: a.guard.map(|g| expand_expr(g, aliases)).transpose()?,
                        body: expand_expr(a.body, aliases)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        },
        Expr::Cons { head, tail } => Expr::Cons {
            head: Box::new(expand_expr(*head, aliases)?),
            tail: Box::new(expand_expr(*tail, aliases)?),
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
    fn typecheck_rejects_imports() {
        let m = ast::Module {
            name: None,
            items: vec![ast::Item::Import(ast::ImportDecl {
                module: "Foo".to_string(),
                as_name: None,
            })],
        };
        assert!(typecheck(m).is_err());
    }

    #[test]
    fn scheme_display_renames_vars() {
        let s = Scheme {
            vars: vec![2],
            constraints: vec![],
            ty: Ty::List(Box::new(Ty::Var(2))),
        };
        assert_eq!(format!("{s}"), "forall a. [a]");
    }

    #[test]
    fn type_error_includes_binding_name() {
        let m = crate::parser::parse_module("x = y\n").unwrap();
        let e = typecheck(m).unwrap_err();
        assert!(format!("{e}").contains("in binding x"));
    }

    #[test]
    fn type_error_includes_let_binding_name() {
        let _ = infer_expr(ast::Expr::Let {
            bindings: vec![ast::Binding {
                pat: ast::Pattern::Var("x".to_string()),
                expr: ast::Expr::Var("y".to_string()),
            }],
            body: Box::new(ast::Expr::Var("x".to_string())),
        })
        .unwrap_err();

        let e = infer_expr(ast::Expr::Let {
            bindings: vec![ast::Binding {
                pat: ast::Pattern::Var("x".to_string()),
                expr: ast::Expr::Var("y".to_string()),
            }],
            body: Box::new(ast::Expr::Var("x".to_string())),
        })
        .unwrap_err();
        assert!(format!("{e}").contains("in let binding x"));
    }

    #[test]
    fn type_error_includes_where_binding_name() {
        let e = infer_expr(ast::Expr::Where {
            expr: Box::new(ast::Expr::Var("x".to_string())),
            bindings: vec![ast::Binding {
                pat: ast::Pattern::Var("x".to_string()),
                expr: ast::Expr::Var("y".to_string()),
            }],
        })
        .unwrap_err();
        assert!(format!("{e}").contains("in where binding x"));
    }

    #[test]
    fn type_error_includes_case_arm_number() {
        let e = infer_expr(ast::Expr::Case {
            expr: Box::new(ast::Expr::Integer("1".to_string())),
            arms: vec![
                ast::CaseArm {
                    pat: ast::Pattern::Wildcard,
                    guard: None,
                    body: ast::Expr::Var("y".to_string()),
                },
                ast::CaseArm {
                    pat: ast::Pattern::Wildcard,
                    guard: None,
                    body: ast::Expr::Integer("0".to_string()),
                },
            ],
        })
        .unwrap_err();
        assert!(format!("{e}").contains("in case arm 1"));
    }

    #[test]
    fn type_error_includes_do_stmt_number() {
        let e = infer_expr(ast::Expr::Do(vec![ast::DoStmt::Expr(ast::Expr::Var(
            "y".to_string(),
        ))]))
        .unwrap_err();
        assert!(format!("{e}").contains("in do stmt 1"));
    }

    #[test]
    fn type_error_includes_if_then_context() {
        let e = infer_expr(ast::Expr::If {
            cond: Box::new(ast::Expr::Bool(true)),
            then_branch: Box::new(ast::Expr::Var("y".to_string())),
            else_branch: Box::new(ast::Expr::Integer("0".to_string())),
        })
        .unwrap_err();
        assert!(format!("{e}").contains("in if then"));
    }

    #[test]
    fn type_error_includes_if_cond_context() {
        let e = infer_expr(ast::Expr::If {
            cond: Box::new(ast::Expr::Integer("1".to_string())),
            then_branch: Box::new(ast::Expr::Integer("0".to_string())),
            else_branch: Box::new(ast::Expr::Integer("0".to_string())),
        })
        .unwrap_err();
        assert!(format!("{e}").contains("in if cond"));
    }

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
                ("b".to_string(), ast::Pattern::Var("y".to_string())),
                ("a".to_string(), ast::Pattern::Var("x".to_string())),
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
    fn infer_record_field_mismatch_is_error() {
        let b = ast::Binding {
            pat: ast::Pattern::Record(vec![("a".to_string(), ast::Pattern::Wildcard)]),
            expr: ast::Expr::Record(vec![("b".to_string(), ast::Expr::Bool(true))]),
        };

        let _ = infer_expr(ast::Expr::Let {
            bindings: vec![b],
            body: Box::new(ast::Expr::Unit),
        })
        .unwrap_err();
    }

    #[test]
    fn infer_do_block() {
        let src = r#"data IO a = IO a

x = do
  y <- IO 1
  IO y
"#;
        let m = crate::parser::parse_module(src).unwrap();

        let crate::ast::Item::Binding(b) = &m.items[1] else {
            panic!("expected binding");
        };

        let ty = infer_in_module(&m, b.expr.clone()).unwrap();
        assert_eq!(
            ty,
            Ty::App {
                head: Box::new(Ty::Con("IO".to_string())),
                args: vec![Ty::Con("Integer".to_string())],
            }
        );
    }

    #[test]
    fn infer_do_bind_requires_io() {
        let src = r#"data IO a = IO a

x = do
  y <- 1
  IO y
"#;
        let m = crate::parser::parse_module(src).unwrap();

        let crate::ast::Item::Binding(b) = &m.items[1] else {
            panic!("expected binding");
        };

        let _ = infer_in_module(&m, b.expr.clone()).unwrap_err();
    }

    #[test]
    fn infer_do_uses_prelude_io() {
        let src = r#"x = do
  y <- IO 1
  IO y
"#;
        let m = crate::parser::parse_module(src).unwrap();

        let env = infer_module(&m).unwrap();
        assert_eq!(
            env.get("x").unwrap(),
            &Scheme::mono(Ty::App {
                head: Box::new(Ty::Con("IO".to_string())),
                args: vec![Ty::Con("Integer".to_string())],
            })
        );
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
                    ast::CaseArm {
                        pat: ast::Pattern::Literal(ast::Expr::Integer("0".to_string())),
                        guard: None,
                        body: ast::Expr::Bool(true),
                    },
                    ast::CaseArm {
                        pat: ast::Pattern::Wildcard,
                        guard: None,
                        body: ast::Expr::Bool(false),
                    },
                ],
            }),
        })
        .unwrap();

        assert_eq!(ty, Ty::Con("Bool".to_string()));
    }

    #[test]
    fn infer_case_adt_constructors() {
        let src = r#"data Maybe a = Nothing | Just a

x = case Just 1 of
  Just n -> n
  Nothing -> 0
"#;
        let m = crate::parser::parse_module(src).unwrap();

        let crate::ast::Item::Binding(b) = &m.items[1] else {
            panic!("expected binding");
        };

        let ty = infer_in_module(&m, b.expr.clone()).unwrap();
        assert_eq!(ty, Ty::Con("Integer".to_string()));

        let env = infer_module(&m).unwrap();
        assert_eq!(
            env.get("x").unwrap(),
            &Scheme::mono(Ty::Con("Integer".to_string()))
        );
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
                    ast::CaseArm {
                        pat: ast::Pattern::Literal(ast::Expr::Integer("0".to_string())),
                        guard: None,
                        body: ast::Expr::Bool(true),
                    },
                    ast::CaseArm {
                        pat: ast::Pattern::Wildcard,
                        guard: None,
                        body: ast::Expr::Integer("1".to_string()),
                    },
                ],
            }),
        })
        .unwrap_err();
    }
}
