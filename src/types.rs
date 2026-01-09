//! Type checking and elaboration scaffolding.
//!
//! Policy (docs):
//! - Surface numeric types: Integer (arbitrary precision) and Float64.
//! - Backend/IR numeric types are LLVM-aligned (i32/i64/f32/f64...).
//! - Pure IR subtyping allows only integer widening (iN <: iM); float widening is NOT subtyping.
//! - Potentially lossy conversions happen only at boundaries as checked casts.

use crate::{ast, error::Error, parser, Result};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

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
    /// Open record (required fields + residual row).
    ///
    /// Produced by `{x, ...}` pattern matching; the `rest` captures the remaining fields.
    RecordOpen(Vec<(String, Ty)>, Box<Ty>),
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
        Ty::RecordOpen(fields, rest) => {
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
            write!(f, "...")?;
            fmt_ty_prec(f, rest, PREC_ATOM, vars)?;
            write!(f, "}}")
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
        (Ty::RecordOpen(req, req_rest), Ty::Record(actual))
        | (Ty::Record(actual), Ty::RecordOpen(req, req_rest)) => {
            let actual: HashMap<String, Ty> = actual.into_iter().collect();

            let mut required = HashSet::new();
            for (k, t_req) in req {
                required.insert(k.clone());
                let t_act = actual
                    .get(&k)
                    .ok_or_else(|| Error::msg("record field mismatch"))?;
                unify_in(subst, t_req, t_act.clone())?;
            }

            let mut rest_fields: Vec<(String, Ty)> = actual
                .into_iter()
                .filter(|(k, _)| !required.contains(k))
                .collect();
            rest_fields.sort_by(|(a, _), (b, _)| a.cmp(b));
            unify_in(subst, *req_rest, Ty::Record(rest_fields))?;

            Ok(())
        }
        (Ty::RecordOpen(a, ra), Ty::RecordOpen(b, rb)) => {
            let a: HashMap<String, Ty> = a.into_iter().collect();
            let b: HashMap<String, Ty> = b.into_iter().collect();
            for (k, ta) in a {
                if let Some(tb) = b.get(&k) {
                    unify_in(subst, ta, tb.clone())?;
                }
            }
            unify_in(subst, *ra, *rb)?;
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
        Ty::RecordOpen(fields, rest) => {
            fields.iter().any(|(_, t)| occurs_in(subst, seen, v, t)) || occurs_in(subst, seen, v, rest)
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
        Ty::RecordOpen(fields, rest) => Ty::RecordOpen(
            fields
                .into_iter()
                .map(|(n, t)| (n, apply(subst, t)))
                .collect(),
            Box::new(apply(subst, *rest)),
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
    ShowRow(Ty),
    /// Field absence constraint for row types (records/open records/row variables).
    Lacks { label: String, row: Ty },
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

fn fmt_constraint(
    f: &mut fmt::Formatter<'_>,
    c: &Constraint,
    vars: &HashMap<u32, String>,
) -> fmt::Result {
    match c {
        Constraint::Show(t) => {
            write!(f, "Show ")?;
            fmt_ty_prec(f, t, 0, vars)
        }
        Constraint::ShowRow(t) => {
            write!(f, "ShowRow ")?;
            fmt_ty_prec(f, t, 0, vars)
        }
        Constraint::Lacks { label, row } => {
            write!(f, "Lacks \"{label}\" ")?;
            fmt_ty_prec(f, row, 0, vars)
        }
    }
}

fn fmt_constraints(
    f: &mut fmt::Formatter<'_>,
    cs: &[Constraint],
    vars: &HashMap<u32, String>,
) -> fmt::Result {
    if cs.len() == 1 {
        return fmt_constraint(f, &cs[0], vars);
    }

    write!(f, "(")?;
    for (i, c) in cs.iter().enumerate() {
        if i > 0 {
            write!(f, ", ")?;
        }
        fmt_constraint(f, c, vars)?;
    }
    write!(f, ")")
}

impl fmt::Display for Scheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.vars.is_empty() {
            let names = HashMap::new();
            if self.constraints.is_empty() {
                return fmt_ty_prec(f, &self.ty, 0, &names);
            }
            fmt_constraints(f, &self.constraints, &names)?;
            write!(f, " => ")?;
            return fmt_ty_prec(f, &self.ty, 0, &names);
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
        if !self.constraints.is_empty() {
            fmt_constraints(f, &self.constraints, &names)?;
            write!(f, " => ")?;
        }
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
        Ty::Record(fields) => fields.iter().flat_map(|(_, t)| ftv_ty(t)).collect(),
        Ty::RecordOpen(fields, rest) => {
            let mut s: HashSet<u32> = fields.iter().flat_map(|(_, t)| ftv_ty(t)).collect();
            s.extend(ftv_ty(rest));
            s
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
        Constraint::Show(t) | Constraint::ShowRow(t) => ftv_ty(t),
        Constraint::Lacks { row, .. } => ftv_ty(row),
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
    generalize_qual(env, vec![], ty)
}

pub fn generalize_qual(env: &TypeEnv, constraints: Vec<Constraint>, ty: Ty) -> Scheme {
    let env_ftv = ftv_env(env);
    let mut ftv = ftv_ty(&ty);
    for c in &constraints {
        ftv.extend(ftv_constraint(c));
    }
    let mut vars: Vec<u32> = ftv.difference(&env_ftv).copied().collect();
    vars.sort_unstable();
    Scheme {
        vars,
        constraints,
        ty,
    }
}

pub fn instantiate(cx: &mut InferCtx, s: &Scheme) -> Ty {
    let (_, ty) = instantiate_qual(cx, s);
    ty
}

fn replace_vars_constraint(c: &Constraint, m: &HashMap<u32, Ty>) -> Constraint {
    match c {
        Constraint::Show(t) => Constraint::Show(replace_vars(t, m)),
        Constraint::ShowRow(t) => Constraint::ShowRow(replace_vars(t, m)),
        Constraint::Lacks { label, row } => Constraint::Lacks {
            label: label.clone(),
            row: replace_vars(row, m),
        },
    }
}

pub fn instantiate_qual(cx: &mut InferCtx, s: &Scheme) -> (Vec<Constraint>, Ty) {
    let mut m: HashMap<u32, Ty> = HashMap::new();
    for v in &s.vars {
        m.insert(*v, cx.fresh());
    }
    (
        s.constraints
            .iter()
            .map(|c| replace_vars_constraint(c, &m))
            .collect(),
        replace_vars(&s.ty, &m),
    )
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
        Ty::RecordOpen(fields, rest) => Ty::RecordOpen(
            fields
                .iter()
                .map(|(n, t)| (n.clone(), replace_vars(t, m)))
                .collect(),
            Box::new(replace_vars(rest, m)),
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
        Constraint::ShowRow(t) => Constraint::ShowRow(apply(subst, t.clone())),
        Constraint::Lacks { label, row } => Constraint::Lacks {
            label: label.clone(),
            row: apply(subst, row.clone()),
        },
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

#[allow(clippy::too_many_arguments)]
fn infer_pat_in(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    subst: &mut Subst,
    env: &TypeEnv,
    pat: &ast::Pattern,
    binds: &mut Vec<(String, Ty)>,
    seen: &mut HashSet<String>,
    cs_out: &mut Vec<Constraint>,
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
                .map(|p| infer_pat_in(cx, data_env, subst, env, p, binds, seen, cs_out))
                .collect::<Result<Vec<_>>>()?,
        )),

        Pattern::List(ps) => {
            if ps.is_empty() {
                return Ok(Ty::List(Box::new(cx.fresh())));
            }

            let first = infer_pat_in(cx, data_env, subst, env, &ps[0], binds, seen, cs_out)?;
            for p in &ps[1..] {
                let t = infer_pat_in(cx, data_env, subst, env, p, binds, seen, cs_out)?;
                let su = unify(apply(subst, first.clone()), apply(subst, t))?;
                *subst = compose(&su, subst);
            }
            Ok(Ty::List(Box::new(apply(subst, first))))
        }
        Pattern::Record(fields) => {
            let mut out = fields
                .iter()
                .map(|(n, p)| {
                    Ok((
                        n.clone(),
                        infer_pat_in(cx, data_env, subst, env, p, binds, seen, cs_out)?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            out.sort_by(|(a, _), (b, _)| a.cmp(b));
            Ok(Ty::Record(out))
        }
        Pattern::RecordLoose(fields, rest_name) => {
            let mut out = fields
                .iter()
                .map(|(n, p)| {
                    Ok((
                        n.clone(),
                        infer_pat_in(cx, data_env, subst, env, p, binds, seen, cs_out)?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            out.sort_by(|(a, _), (b, _)| a.cmp(b));

            let rest_ty = cx.fresh();
            if rest_name.is_some() {
                for (n, _) in &out {
                    cs_out.push(Constraint::Lacks {
                        label: n.clone(),
                        row: rest_ty.clone(),
                    });
                }
            }

            if let Some(name) = rest_name {
                if !seen.insert(name.clone()) {
                    return Err(Error::msg("duplicate pattern variable"));
                }
                binds.push((name.clone(), rest_ty.clone()));
            }

            Ok(Ty::RecordOpen(out, Box::new(rest_ty)))
        }
        Pattern::Cons(hd, tl) => {
            let elem = cx.fresh();
            let t_hd = infer_pat_in(cx, data_env, subst, env, hd, binds, seen, cs_out)?;
            let t_tl = infer_pat_in(cx, data_env, subst, env, tl, binds, seen, cs_out)?;

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
            let mut cs_a = Vec::new();
            let t_a = infer_pat_in(cx, data_env, subst, env, a, &mut binds_a, &mut seen_a, &mut cs_a)?;

            let mut binds_b = base_binds;
            let mut seen_b = base_seen;
            let mut cs_b = Vec::new();
            let t_b = infer_pat_in(cx, data_env, subst, env, b, &mut binds_b, &mut seen_b, &mut cs_b)?;

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

            let mut ca = apply_constraints(subst, cs_a);
            let mut cb = apply_constraints(subst, cs_b);
            ca.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
            cb.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
            ca.dedup();
            cb.dedup();
            if ca != cb {
                return Err(Error::msg("or-pattern must yield the same constraints"));
            }
            cs_out.extend(ca);

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
            let t = infer_pat_in(cx, data_env, subst, env, p, binds, seen, cs_out)?;
            binds.push((name.clone(), apply(subst, t.clone())));
            Ok(t)
        }
        Pattern::View(p, e) => {
            let t_scrut = cx.fresh();
            let t_view = infer_pat_in(cx, data_env, subst, env, p, binds, seen, cs_out)?;

            let env_in = apply_env(subst, env);
            let (s_e, _cs_e, t_e) = infer_expr_in(cx, data_env, &env_in, (**e).clone())?;
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
                let arg_pat_ty = infer_pat_in(cx, data_env, subst, env, p, binds, seen, cs_out)?;
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
    let data_env = DataEnv::new();
    let env = TypeEnv::new();
    let (s, cs, t) = infer_expr_in(&mut cx, &data_env, &env, expr)?;
    let _ = simplify_constraints(&data_env, apply_constraints(&s, cs))?;
    Ok(apply(&s, t))
}

pub fn infer_in_module(module: &ast::Module, expr: ast::Expr) -> Result<Ty> {
    let mut cx = InferCtx::default();
    let data_env = collect_data_env(module);
    let env = collect_ctor_env(&mut cx, module)?;
    let (s, cs, t) = infer_expr_in(&mut cx, &data_env, &env, expr)?;
    let _ = simplify_constraints(&data_env, apply_constraints(&s, cs))?;
    Ok(apply(&s, t))
}

pub fn infer_module(module: &ast::Module) -> Result<HashMap<String, Scheme>> {
    let mut cx = InferCtx::default();
    let data_env = collect_data_env(module);
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
        let mut cs_pat = Vec::new();
        let pat_ty = infer_pat_in(
            &mut cx,
            &data_env,
            &mut subst,
            &env,
            &b.pat,
            &mut binds,
            &mut seen,
            &mut cs_pat,
        )
        .map_err(|e| Error::msg(format!("in binding {ctx_name}: {e}")))?;

        let env_in = apply_env(&subst, &env);
        let (s_rhs, cs_rhs, t_rhs) = infer_expr_in(&mut cx, &data_env, &env_in, b.expr.clone())
            .map_err(|e| Error::msg(format!("in binding {ctx_name}: {e}")))?;
        subst = compose(&s_rhs, &subst);

        let s_pat = unify(apply(&subst, t_rhs), apply(&subst, pat_ty))
            .map_err(|e| Error::msg(format!("in binding {ctx_name}: {e}")))?;
        subst = compose(&s_pat, &subst);

        for (name, t) in binds {
            let env_gen = apply_env(&subst, &env);
            let mut cs = cs_rhs.clone();
            cs.extend(cs_pat.clone());
            let cs = simplify_constraints(&data_env, apply_constraints(&subst, cs))?;
            let scheme = generalize_qual(&env_gen, cs, apply(&subst, t));
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

    // ++ :: String -> String -> String
    env.insert(
        "++".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Con("String".to_string())),
                Box::new(Ty::Func(
                    Box::new(Ty::Con("String".to_string())),
                    Box::new(Ty::Con("String".to_string())),
                )),
            ),
        },
    );

    // show :: Show a => a -> String
    let Ty::Var(v) = cx.fresh() else { unreachable!() };
    env.insert(
        "show".to_string(),
        Scheme {
            vars: vec![v],
            constraints: vec![Constraint::Show(Ty::Var(v))],
            ty: Ty::Func(
                Box::new(Ty::Var(v)),
                Box::new(Ty::Con("String".to_string())),
            ),
        },
    );

    // toString :: Show a => a -> String
    let Ty::Var(v) = cx.fresh() else { unreachable!() };
    env.insert(
        "toString".to_string(),
        Scheme {
            vars: vec![v],
            constraints: vec![Constraint::Show(Ty::Var(v))],
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

    // throw :: forall a. String -> IO a
    let Ty::Var(a) = cx.fresh() else { unreachable!() };
    env.insert(
        "throw".to_string(),
        Scheme {
            vars: vec![a],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Con("String".to_string())),
                Box::new(Ty::App {
                    head: Box::new(Ty::Con("IO".to_string())),
                    args: vec![Ty::Var(a)],
                }),
            ),
        },
    );

    // catch :: forall a. IO a -> (String -> IO a) -> IO a
    let Ty::Var(a) = cx.fresh() else { unreachable!() };
    let io_a = Ty::App {
        head: Box::new(Ty::Con("IO".to_string())),
        args: vec![Ty::Var(a)],
    };
    let handler = Ty::Func(
        Box::new(Ty::Con("String".to_string())),
        Box::new(io_a.clone()),
    );
    env.insert(
        "catch".to_string(),
        Scheme {
            vars: vec![a],
            constraints: vec![],
            ty: Ty::Func(Box::new(io_a.clone()), Box::new(Ty::Func(Box::new(handler), Box::new(io_a)))),
        },
    );

    // try :: forall a. IO a -> IO (Prelude.Either String a)
    let Ty::Var(a) = cx.fresh() else { unreachable!() };
    let io_a = Ty::App {
        head: Box::new(Ty::Con("IO".to_string())),
        args: vec![Ty::Var(a)],
    };
    let either = Ty::App {
        head: Box::new(Ty::Con("Prelude.Either".to_string())),
        args: vec![Ty::Con("String".to_string()), Ty::Var(a)],
    };
    env.insert(
        "try".to_string(),
        Scheme {
            vars: vec![a],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(io_a),
                Box::new(Ty::App {
                    head: Box::new(Ty::Con("IO".to_string())),
                    args: vec![either],
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

fn apply_constraints(subst: &Subst, cs: Vec<Constraint>) -> Vec<Constraint> {
    cs.into_iter().map(|c| apply_constraint(subst, &c)).collect()
}

type DataEnv = HashMap<String, ast::DataDecl>;

fn collect_data_env(module: &ast::Module) -> DataEnv {
    module
        .items
        .iter()
        .filter_map(|it| match it {
            ast::Item::DataDecl(d) => Some((d.name.clone(), d.clone())),
            _ => None,
        })
        .collect()
}

fn lower_surface_type_with_tys(
    ty: &ast::Type,
    holes: &mut HashMap<String, Ty>,
    params: &HashMap<String, Ty>,
) -> Result<Ty> {
    use ast::Type;
    Ok(match ty {
        Type::Var(name) => params.get(name).cloned().unwrap_or_else(|| Ty::Con(name.clone())),
        Type::Unit => Ty::Con("Unit".to_string()),
        Type::Integer => Ty::Con("Integer".to_string()),
        Type::Bool => Ty::Con("Bool".to_string()),
        Type::Float64 => Ty::Con("Float64".to_string()),
        Type::Char => Ty::Con("Char".to_string()),
        Type::String => Ty::Con("String".to_string()),
        Type::List(t) => Ty::List(Box::new(lower_surface_type_with_tys(t, holes, params)?)),
        Type::Tuple(ts) => Ty::Tuple(
            ts.iter()
                .map(|t| lower_surface_type_with_tys(t, holes, params))
                .collect::<Result<Vec<_>>>()?,
        ),
        Type::Record(fields) => Ty::Record(
            fields
                .iter()
                .map(|(n, t)| Ok((n.clone(), lower_surface_type_with_tys(t, holes, params)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
        Type::RecordOpen(fields, rest) => Ty::RecordOpen(
            fields
                .iter()
                .map(|(n, t)| Ok((n.clone(), lower_surface_type_with_tys(t, holes, params)?)))
                .collect::<Result<Vec<_>>>()?,
            Box::new(lower_surface_type_with_tys(rest, holes, params)?),
        ),
        Type::Func(a, b) => Ty::Func(
            Box::new(lower_surface_type_with_tys(a, holes, params)?),
            Box::new(lower_surface_type_with_tys(b, holes, params)?),
        ),
        Type::App { head, args } => Ty::App {
            head: Box::new(lower_surface_type_with_tys(head, holes, params)?),
            args: args
                .iter()
                .map(|t| lower_surface_type_with_tys(t, holes, params))
                .collect::<Result<Vec<_>>>()?,
        },
        Type::Hole(Some(name)) => holes
            .get(name)
            .cloned()
            .ok_or_else(|| Error::msg("type holes in data declarations are not supported"))?,
        Type::Hole(None) => return Err(Error::msg("type holes in data declarations are not supported")),
    })
}

fn show_primitives(name: &str) -> bool {
    matches!(name, "Integer" | "Bool" | "String" | "Char" | "Unit")
}

fn entails_show(data_env: &DataEnv, ty: &Ty, in_progress: &mut Vec<Ty>) -> Result<Vec<Constraint>> {
    Ok(match ty {
        Ty::Var(_) => vec![Constraint::Show(ty.clone())],
        Ty::Con(name) => {
            if show_primitives(name) {
                vec![]
            } else if let Some(d) = data_env.get(name) {
                if !d.params.is_empty() {
                    return Err(Error::msg(format!(
                        "cannot satisfy constraint: Show {ty}"
                    )));
                }
                entails_show_data_decl(data_env, d, &[], in_progress)?
            } else {
                return Err(Error::msg(format!("cannot satisfy constraint: Show {ty}")));
            }
        }
        Ty::List(t) => entails_show(data_env, t, in_progress)?,
        Ty::Tuple(ts) => {
            let mut out = Vec::new();
            for t in ts {
                out.extend(entails_show(data_env, t, in_progress)?);
            }
            out
        }
        Ty::Record(fields) => {
            let mut out = Vec::new();
            for (_, t) in fields {
                out.extend(entails_show(data_env, t, in_progress)?);
            }
            out
        }
        Ty::RecordOpen(fields, rest) => {
            let mut out = Vec::new();
            for (_, t) in fields {
                out.extend(entails_show(data_env, t, in_progress)?);
            }
            out.push(Constraint::ShowRow((**rest).clone()));
            out
        }
        Ty::App { head, args } => {
            let Ty::Con(name) = &**head else {
                return Err(Error::msg(format!("cannot satisfy constraint: Show {ty}")));
            };
            let Some(d) = data_env.get(name) else {
                return Err(Error::msg(format!("cannot satisfy constraint: Show {ty}")));
            };
            if d.params.len() != args.len() {
                return Err(Error::msg(format!("cannot satisfy constraint: Show {ty}")));
            }
            entails_show_data_decl(data_env, d, args, in_progress)?
        }
        Ty::Func(_, _) => return Err(Error::msg(format!("cannot satisfy constraint: Show {ty}"))),
    })
}

fn entails_show_row(data_env: &DataEnv, ty: &Ty, in_progress: &mut Vec<Ty>) -> Result<Vec<Constraint>> {
    Ok(match ty {
        Ty::Var(_) => vec![Constraint::ShowRow(ty.clone())],
        Ty::Record(fields) => {
            let mut out = Vec::new();
            for (_, t) in fields {
                out.extend(entails_show(data_env, t, in_progress)?);
            }
            out
        }
        Ty::RecordOpen(fields, rest) => {
            let mut out = Vec::new();
            for (_, t) in fields {
                out.extend(entails_show(data_env, t, in_progress)?);
            }
            out.extend(entails_show_row(data_env, rest, in_progress)?);
            out
        }
        _ => return Err(Error::msg(format!("cannot satisfy constraint: ShowRow {ty}"))),
    })
}

fn entails_show_data_decl(
    data_env: &DataEnv,
    d: &ast::DataDecl,
    args: &[Ty],
    in_progress: &mut Vec<Ty>,
) -> Result<Vec<Constraint>> {
    let self_ty = if args.is_empty() {
        Ty::Con(d.name.clone())
    } else {
        Ty::App {
            head: Box::new(Ty::Con(d.name.clone())),
            args: args.to_vec(),
        }
    };

    if in_progress.contains(&self_ty) {
        return Ok(vec![]);
    }

    in_progress.push(self_ty);
    let mut out = Vec::new();

    let mut param_map: HashMap<String, Ty> = HashMap::new();
    for (p, a) in d.params.iter().zip(args.iter()) {
        param_map.insert(p.clone(), a.clone());
    }

    for ctor in &d.ctors {
        let mut holes = HashMap::new();
        for t_ast in &ctor.args {
            let t = lower_surface_type_with_tys(t_ast, &mut holes, &param_map)?;
            out.extend(entails_show(data_env, &t, in_progress)?);
        }
    }

    in_progress.pop();
    Ok(out)
}

fn entails_lacks(label: &str, row: &Ty) -> Result<Vec<Constraint>> {
    Ok(match row {
        Ty::Var(_) => vec![Constraint::Lacks {
            label: label.to_string(),
            row: row.clone(),
        }],
        Ty::Record(fields) => {
            if fields.iter().any(|(k, _)| k == label) {
                return Err(Error::msg(format!(
                    "cannot satisfy constraint: Lacks {label} {row}"
                )));
            }
            vec![]
        }
        Ty::RecordOpen(fields, rest) => {
            if fields.iter().any(|(k, _)| k == label) {
                return Err(Error::msg(format!(
                    "cannot satisfy constraint: Lacks {label} {row}"
                )));
            }
            entails_lacks(label, rest)?
        }
        _ => {
            return Err(Error::msg(format!(
                "cannot satisfy constraint: Lacks {label} {row}"
            )))
        }
    })
}

fn simplify_constraints(data_env: &DataEnv, cs: Vec<Constraint>) -> Result<Vec<Constraint>> {
    let mut out = Vec::new();
    let mut in_progress = Vec::new();

    for c in cs {
        match c {
            Constraint::Show(t) => out.extend(entails_show(data_env, &t, &mut in_progress)?),
            Constraint::ShowRow(t) => out.extend(entails_show_row(data_env, &t, &mut in_progress)?),
            Constraint::Lacks { label, row } => out.extend(entails_lacks(&label, &row)?),
        }
    }

    // Dedup for stability.
    out.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    out.dedup();
    Ok(out)
}

fn infer_expr_in(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    env: &TypeEnv,
    expr: ast::Expr,
) -> Result<(Subst, Vec<Constraint>, Ty)> {
    use ast::Expr;

    match expr {
        Expr::Unit => Ok((Subst::new(), vec![], Ty::Con("Unit".to_string()))),
        Expr::Integer(_) => Ok((Subst::new(), vec![], Ty::Con("Integer".to_string()))),
        Expr::Float64(_) => Ok((Subst::new(), vec![], Ty::Con("Float64".to_string()))),
        Expr::Bool(true) | Expr::Bool(false) => Ok((Subst::new(), vec![], Ty::Con("Bool".to_string()))),
        Expr::String(_) => Ok((Subst::new(), vec![], Ty::Con("String".to_string()))),
        Expr::Char(_) => Ok((Subst::new(), vec![], Ty::Con("Char".to_string()))),

        Expr::Var(name) => {
            let s = env
                .get(&name)
                .ok_or_else(|| Error::msg(format!("unbound variable: {name}")))?;
            let (cs, ty) = instantiate_qual(cx, s);
            Ok((Subst::new(), cs, ty))
        }

        Expr::Ctor(name) => {
            let s = env
                .get(&name)
                .ok_or_else(|| Error::msg("unknown constructor"))?;
            let (cs, ty) = instantiate_qual(cx, s);
            Ok((Subst::new(), cs, ty))
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

            let (s_body, cs_body, body_ty) = infer_expr_in(cx, data_env, &env2, *body)?;
            let mut out = apply(&s_body, body_ty);
            for pty in param_tys.into_iter().rev() {
                out = Ty::Func(Box::new(apply(&s_body, pty)), Box::new(out));
            }

            Ok((s_body, cs_body, out))
        }

        Expr::Apply { func, args } => {
            let (mut s, mut cs, mut fun_ty) = infer_expr_in(cx, data_env, env, *func)?;

            for arg in args {
                let env2 = apply_env(&s, env);
                let (s_arg, cs_arg, arg_ty) = infer_expr_in(cx, data_env, &env2, arg)?;
                s = compose(&s_arg, &s);

                cs = apply_constraints(&s, cs);
                cs.extend(apply_constraints(&s, cs_arg));

                fun_ty = apply(&s, fun_ty);
                let res = cx.fresh();

                let s_unify = unify(
                    fun_ty,
                    Ty::Func(Box::new(apply(&s, arg_ty)), Box::new(res.clone())),
                )?;
                s = compose(&s_unify, &s);
                cs = apply_constraints(&s, cs);
                fun_ty = apply(&s, res);
            }

            Ok((s, cs, fun_ty))
        }

        Expr::Annot { expr, ty } => {
            let (s1, mut cs1, t1) = infer_expr_in(cx, data_env, env, *expr)?;
            let mut holes = HashMap::new();

            // Lower predicates + annotated type using a shared var map.
            for p in &ty.preds {
                match p {
                    ast::Predicate::Show(t) => {
                        let t = lower_surface_type(cx, t, &mut holes);
                        cs1.push(Constraint::Show(t));
                    }
                    ast::Predicate::ShowRow(t) => {
                        let t = lower_surface_type(cx, t, &mut holes);
                        cs1.push(Constraint::ShowRow(t));
                    }
                    ast::Predicate::Lacks { label, row } => {
                        let row = lower_surface_type(cx, row, &mut holes);
                        cs1.push(Constraint::Lacks {
                            label: label.clone(),
                            row,
                        });
                    }
                }
            }

            let t_ann = lower_surface_type(cx, &ty.ty, &mut holes);
            let s2 = unify(apply(&s1, t1), apply(&s1, t_ann.clone()))?;
            let s = compose(&s2, &s1);
            Ok((s.clone(), apply_constraints(&s, cs1), apply(&s, t_ann)))
        }

        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            let (s_cond, cs_cond, t_cond) = infer_expr_in(cx, data_env, env, *cond)
                .map_err(|e| Error::msg(format!("in if cond: {e}")))?;
            let s_bool = unify(apply(&s_cond, t_cond), Ty::Con("Bool".to_string()))
                .map_err(|e| Error::msg(format!("in if cond: {e}")))?;
            let mut s = compose(&s_bool, &s_cond);
            let mut cs = apply_constraints(&s, cs_cond);

            let env2 = apply_env(&s, env);
            let (s_then, cs_then, t_then) = infer_expr_in(cx, data_env, &env2, *then_branch)
                .map_err(|e| Error::msg(format!("in if then: {e}")))?;
            s = compose(&s_then, &s);
            cs = apply_constraints(&s, cs);
            cs.extend(apply_constraints(&s, cs_then));

            let env3 = apply_env(&s, env);
            let (s_else, cs_else, t_else) = infer_expr_in(cx, data_env, &env3, *else_branch)
                .map_err(|e| Error::msg(format!("in if else: {e}")))?;
            s = compose(&s_else, &s);
            cs = apply_constraints(&s, cs);
            cs.extend(apply_constraints(&s, cs_else));

            let s_res = unify(apply(&s, t_then.clone()), apply(&s, t_else))
                .map_err(|e| Error::msg(format!("in if branches: {e}")))?;
            s = compose(&s_res, &s);
            cs = apply_constraints(&s, cs);
            Ok((s.clone(), cs, apply(&s, apply(&s, t_then))))
        }

        Expr::Tuple(elems) => {
            let mut s = Subst::new();
            let mut cs: Vec<Constraint> = vec![];
            let mut ts = Vec::new();
            for e in elems {
                let env2 = apply_env(&s, env);
                let (s_e, cs_e, t_e) = infer_expr_in(cx, data_env, &env2, e)?;
                s = compose(&s_e, &s);
                cs = apply_constraints(&s, cs);
                cs.extend(apply_constraints(&s, cs_e));
                ts.push(apply(&s, t_e));
            }
            Ok((s, cs, Ty::Tuple(ts)))
        }

        Expr::Cons { head, tail } => {
            let (s_hd, cs_hd, t_hd) = infer_expr_in(cx, data_env, env, *head)?;
            let env2 = apply_env(&s_hd, env);
            let (s_tl, cs_tl, t_tl) = infer_expr_in(cx, data_env, &env2, *tail)?;
            let mut s = compose(&s_tl, &s_hd);
            let mut cs = apply_constraints(&s, cs_hd);
            cs.extend(apply_constraints(&s, cs_tl));

            let elem = cx.fresh();
            let su_tl = unify(apply(&s, t_tl), Ty::List(Box::new(elem.clone())))?;
            s = compose(&su_tl, &s);
            cs = apply_constraints(&s, cs);

            let su_hd = unify(apply(&s, t_hd), apply(&s, elem.clone()))?;
            s = compose(&su_hd, &s);
            cs = apply_constraints(&s, cs);

            Ok((s.clone(), cs, Ty::List(Box::new(apply(&s, elem)))))
        }

        Expr::List(elems) => {
            if elems.is_empty() {
                return Ok((Subst::new(), vec![], Ty::List(Box::new(cx.fresh()))));
            }

            let (mut s, mut cs, first_ty) = infer_expr_in(cx, data_env, env, elems[0].clone())?;
            let mut elem_ty = apply(&s, first_ty);

            for e in elems.into_iter().skip(1) {
                let env2 = apply_env(&s, env);
                let (s_e, cs_e, t_e) = infer_expr_in(cx, data_env, &env2, e)?;
                s = compose(&s_e, &s);
                cs = apply_constraints(&s, cs);
                cs.extend(apply_constraints(&s, cs_e));

                let su = unify(apply(&s, elem_ty.clone()), apply(&s, t_e))?;
                s = compose(&su, &s);
                cs = apply_constraints(&s, cs);
                elem_ty = apply(&s, elem_ty);
            }

            Ok((s.clone(), cs, Ty::List(Box::new(apply(&s, elem_ty)))))
        }

        Expr::Record(fields) => {
            let mut s = Subst::new();
            let mut cs: Vec<Constraint> = vec![];
            let mut out = Vec::new();
            for (name, e) in fields {
                let env2 = apply_env(&s, env);
                let (s_e, cs_e, t_e) = infer_expr_in(cx, data_env, &env2, e)?;
                s = compose(&s_e, &s);
                cs = apply_constraints(&s, cs);
                cs.extend(apply_constraints(&s, cs_e));
                out.push((name, apply(&s, t_e)));
            }
            out.sort_by(|(a, _), (b, _)| a.cmp(b));
            Ok((s, cs, Ty::Record(out)))
        }

        Expr::Let { bindings, body } => {
            let mut s = Subst::new();
            let mut cs: Vec<Constraint> = vec![];
            let mut env2 = env.clone();

            for b in bindings {
                let ctx_name = match &b.pat {
                    ast::Pattern::Var(n) => n.as_str(),
                    _ => "<pattern>",
                };

                let mut binds = Vec::new();
                let mut seen = HashSet::new();
                let mut cs_pat = Vec::new();
                let pat_ty = infer_pat_in(
                    cx,
                    data_env,
                    &mut s,
                    &env2,
                    &b.pat,
                    &mut binds,
                    &mut seen,
                    &mut cs_pat,
                )
                .map_err(|e| Error::msg(format!("in let binding {ctx_name}: {e}")))?;

                let env_in = apply_env(&s, &env2);
                let (s_rhs, cs_rhs, t_rhs) = infer_expr_in(cx, data_env, &env_in, b.expr)
                    .map_err(|e| Error::msg(format!("in let binding {ctx_name}: {e}")))?;
                s = compose(&s_rhs, &s);
                cs = apply_constraints(&s, cs);
                cs.extend(apply_constraints(&s, cs_rhs.clone()));

                let s_pat = unify(apply(&s, t_rhs), apply(&s, pat_ty))
                    .map_err(|e| Error::msg(format!("in let binding {ctx_name}: {e}")))?;
                s = compose(&s_pat, &s);
                cs = apply_constraints(&s, cs);

                for (name, t) in binds {
                    let env_gen = apply_env(&s, &env2);
                    let mut cs = cs_rhs.clone();
                    cs.extend(cs_pat.clone());
                    let cs = simplify_constraints(data_env, apply_constraints(&s, cs))?;
                    let scheme = generalize_qual(&env_gen, cs, apply(&s, t));
                    env2.insert(name, scheme);
                }
            }

            let env_body = apply_env(&s, &env2);
            let (s_body, cs_body, t_body) = infer_expr_in(cx, data_env, &env_body, *body)
                .map_err(|e| Error::msg(format!("in let body: {e}")))?;
            let s = compose(&s_body, &s);
            let mut cs = apply_constraints(&s, cs);
            cs.extend(apply_constraints(&s, cs_body));
            Ok((s.clone(), cs, apply(&s, t_body)))
        }

        Expr::Where { expr, bindings } => {
            let mut s = Subst::new();
            let mut cs: Vec<Constraint> = vec![];
            let mut env2 = env.clone();

            for b in bindings {
                let ctx_name = match &b.pat {
                    ast::Pattern::Var(n) => n.as_str(),
                    _ => "<pattern>",
                };

                let mut binds = Vec::new();
                let mut seen = HashSet::new();
                let mut cs_pat = Vec::new();
                let pat_ty = infer_pat_in(
                    cx,
                    data_env,
                    &mut s,
                    &env2,
                    &b.pat,
                    &mut binds,
                    &mut seen,
                    &mut cs_pat,
                )
                .map_err(|e| Error::msg(format!("in where binding {ctx_name}: {e}")))?;

                let env_in = apply_env(&s, &env2);
                let (s_rhs, cs_rhs, t_rhs) = infer_expr_in(cx, data_env, &env_in, b.expr)
                    .map_err(|e| Error::msg(format!("in where binding {ctx_name}: {e}")))?;
                s = compose(&s_rhs, &s);
                cs = apply_constraints(&s, cs);
                cs.extend(apply_constraints(&s, cs_rhs.clone()));

                let s_pat = unify(apply(&s, t_rhs), apply(&s, pat_ty))
                    .map_err(|e| Error::msg(format!("in where binding {ctx_name}: {e}")))?;
                s = compose(&s_pat, &s);
                cs = apply_constraints(&s, cs);

                for (name, t) in binds {
                    let env_gen = apply_env(&s, &env2);
                    let mut cs = cs_rhs.clone();
                    cs.extend(cs_pat.clone());
                    let cs = simplify_constraints(data_env, apply_constraints(&s, cs))?;
                    let scheme = generalize_qual(&env_gen, cs, apply(&s, t));
                    env2.insert(name, scheme);
                }
            }

            let env_body = apply_env(&s, &env2);
            let (s_body, cs_body, t_body) = infer_expr_in(cx, data_env, &env_body, *expr)
                .map_err(|e| Error::msg(format!("in where body: {e}")))?;
            let s = compose(&s_body, &s);
            let mut cs = apply_constraints(&s, cs);
            cs.extend(apply_constraints(&s, cs_body));
            Ok((s.clone(), cs, apply(&s, t_body)))
        }

        Expr::Case { expr, arms } => {
            if arms.is_empty() {
                return Err(Error::msg("empty case"));
            }

            let (mut s, mut cs, scrut_ty) = infer_expr_in(cx, data_env, env, *expr)
                .map_err(|e| Error::msg(format!("in case scrutinee: {e}")))?;
            let mut out_ty = cx.fresh();

            for (i, arm) in arms.into_iter().enumerate() {
                let arm_no = i + 1;
                let ast::CaseArm { pat, guard, body } = arm;

                let mut binds = Vec::new();
                let mut seen = HashSet::new();
                let mut cs_pat = Vec::new();
                let pat_ty = infer_pat_in(cx, data_env, &mut s, env, &pat, &mut binds, &mut seen, &mut cs_pat)
                    .map_err(|e| Error::msg(format!("in case arm {arm_no}: {e}")))?;

                let su_pat = unify(apply(&s, pat_ty), apply(&s, scrut_ty.clone()))
                    .map_err(|e| Error::msg(format!("in case arm {arm_no}: {e}")))?;
                s = compose(&su_pat, &s);
                cs = apply_constraints(&s, cs);
                cs.extend(apply_constraints(&s, cs_pat));

                let mut env_arm = apply_env(&s, env);
                for (name, t) in binds {
                    env_arm.insert(name, Scheme::mono(apply(&s, t)));
                }

                if let Some(g) = guard {
                    let (s_g, cs_g, t_g) = infer_expr_in(cx, data_env, &env_arm, g)
                        .map_err(|e| Error::msg(format!("in case arm {arm_no} guard: {e}")))?;
                    s = compose(&s_g, &s);
                    cs = apply_constraints(&s, cs);
                    cs.extend(apply_constraints(&s, cs_g));

                    let su_g = unify(apply(&s, t_g), Ty::Con("Bool".to_string()))
                        .map_err(|e| Error::msg(format!("in case arm {arm_no} guard: {e}")))?;
                    s = compose(&su_g, &s);
                    cs = apply_constraints(&s, cs);
                    env_arm = apply_env(&s, &env_arm);
                }

                let (s_arm, cs_arm, arm_ty) = infer_expr_in(cx, data_env, &env_arm, body)
                    .map_err(|e| Error::msg(format!("in case arm {arm_no}: {e}")))?;
                s = compose(&s_arm, &s);
                cs = apply_constraints(&s, cs);
                cs.extend(apply_constraints(&s, cs_arm));

                let su_out = unify(apply(&s, out_ty.clone()), apply(&s, arm_ty))
                    .map_err(|e| Error::msg(format!("in case arm {arm_no}: {e}")))?;
                s = compose(&su_out, &s);
                cs = apply_constraints(&s, cs);
                out_ty = apply(&s, out_ty);
            }

            Ok((s.clone(), cs, apply(&s, out_ty)))
        }

        Expr::Do(stmts) => {
            if stmts.is_empty() {
                return Err(Error::msg("empty do"));
            }

            let n = stmts.len();
            let mut s = Subst::new();
            let mut cs: Vec<Constraint> = vec![];
            let mut env2 = env.clone();

            let mut last_ty: Option<Ty> = None;

            for (i, stmt) in stmts.into_iter().enumerate() {
                let stmt_no = i + 1;
                let is_last = i + 1 == n;

                match stmt {
                    ast::DoStmt::Bind { pat, expr } => {
                        let mut binds = Vec::new();
                        let mut seen = HashSet::new();
                        let mut cs_pat = Vec::new();
                        let pat_ty = infer_pat_in(cx, data_env, &mut s, &env2, &pat, &mut binds, &mut seen, &mut cs_pat)
                            .map_err(|e| Error::msg(format!("in do stmt {stmt_no} (<-): {e}")))?;

                        let env_in = apply_env(&s, &env2);
                        let (s_e, cs_e, t_e) = infer_expr_in(cx, data_env, &env_in, expr)
                            .map_err(|e| Error::msg(format!("in do stmt {stmt_no} (<-): {e}")))?;
                        s = compose(&s_e, &s);
                        cs = apply_constraints(&s, cs);
                        cs.extend(apply_constraints(&s, cs_e));

                        let io_r = cx.fresh();
                        let su = unify(
                            apply(&s, t_e),
                            Ty::App {
                                head: Box::new(Ty::Con("IO".to_string())),
                                args: vec![io_r.clone()],
                            },
                        )
                        .map_err(|e| Error::msg(format!("in do stmt {stmt_no} (<-): {e}")))?;
                        s = compose(&su, &s);
                        cs = apply_constraints(&s, cs);

                        let su_pat = unify(apply(&s, pat_ty), apply(&s, io_r.clone()))
                            .map_err(|e| Error::msg(format!("in do stmt {stmt_no} (<-): {e}")))?;
                        s = compose(&su_pat, &s);
                        cs = apply_constraints(&s, cs);
                        cs.extend(apply_constraints(&s, cs_pat));

                        env2 = apply_env(&s, &env2);
                        for (name, t) in binds {
                            env2.insert(name, Scheme::mono(apply(&s, t)));
                        }

                        if is_last {
                            last_ty = Some(Ty::App {
                                head: Box::new(Ty::Con("IO".to_string())),
                                args: vec![apply(&s, Ty::Con("Unit".to_string()))],
                            });
                        } else {
                            last_ty = None;
                        }
                    }
                    ast::DoStmt::Expr(e) => {
                        let env_in = apply_env(&s, &env2);
                        let (s_e, cs_e, t_e) = infer_expr_in(cx, data_env, &env_in, e)
                            .map_err(|e| Error::msg(format!("in do stmt {stmt_no}: {e}")))?;
                        s = compose(&s_e, &s);
                        cs = apply_constraints(&s, cs);
                        cs.extend(apply_constraints(&s, cs_e));

                        let io_r = cx.fresh();
                        let su = unify(
                            apply(&s, t_e),
                            Ty::App {
                                head: Box::new(Ty::Con("IO".to_string())),
                                args: vec![io_r.clone()],
                            },
                        )
                        .map_err(|e| Error::msg(format!("in do stmt {stmt_no}: {e}")))?;
                        s = compose(&su, &s);
                        cs = apply_constraints(&s, cs);

                        if is_last {
                            last_ty = Some(Ty::App {
                                head: Box::new(Ty::Con("IO".to_string())),
                                args: vec![apply(&s, io_r)],
                            });
                        } else {
                            last_ty = None;
                        }
                    }
                }
            }

            let last_ty = last_ty.ok_or_else(|| Error::msg("do must end with expression"))?;
            Ok((s.clone(), cs, apply(&s, last_ty)))
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
        Type::RecordOpen(fields, rest) => Ty::RecordOpen(
            fields
                .iter()
                .map(|(n, t)| (n.clone(), lower_surface_type(cx, t, holes)))
                .collect(),
            Box::new(lower_surface_type(cx, rest, holes)),
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

        Type::Var(name) => {
            if name.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
                holes
                    .entry(name.clone())
                    .or_insert_with(|| cx.fresh())
                    .clone()
            } else {
                Ty::Con(name.clone())
            }
        }
    }
}

pub fn typecheck_file(entry: &Path) -> Result<TypedModule> {
    let module = load_module_with_imports(entry)?;
    typecheck(module)
}

fn load_module_with_imports(entry: &Path) -> Result<ast::Module> {
    let entry = std::fs::canonicalize(entry)?;
    let entry_dir = entry.parent().unwrap_or_else(|| Path::new("."));

    let mut loader = ModuleLoader {
        cache: HashMap::new(),
        stack: vec![entry.clone()],
        emitted: HashSet::new(),
    };

    let entry_mod = loader.load_ast(&entry)?;

    let mut items = Vec::new();
    let mut defined = HashSet::new();

    let mut deps = Vec::new();
    loader.collect_imports(&entry_mod, entry_dir, &mut deps)?;

    for it in deps {
        push_item_checked(&mut items, &mut defined, it)?;
    }

    for it in entry_mod.items {
        if matches!(it, ast::Item::Import(_)) {
            continue;
        }
        push_item_checked(&mut items, &mut defined, it)?;
    }

    Ok(ast::Module {
        name: entry_mod.name,
        items,
    })
}

struct ModuleLoader {
    cache: HashMap<PathBuf, ast::Module>,
    stack: Vec<PathBuf>,
    emitted: HashSet<PathBuf>,
}

fn module_allowed_qualifiers(module: &ast::Module) -> HashSet<String> {
    module
        .items
        .iter()
        .filter_map(|it| match it {
            ast::Item::Import(id) => Some(id),
            _ => None,
        })
        .map(|id| id.as_name.clone().unwrap_or_else(|| id.module.clone()))
        .collect()
}

fn desugar_qualified_ref(name: &str, allowed: &HashSet<String>) -> Result<String> {
    let Some((qual, _)) = name.rsplit_once('.') else {
        return Ok(name.to_string());
    };
    if !allowed.contains(qual) {
        return Err(Error::msg(format!("unknown qualifier {qual} in {name}")));
    }
    Ok(name.to_string())
}

fn desugar_qualified_expr(expr: ast::Expr, allowed: &HashSet<String>) -> Result<ast::Expr> {
    use ast::Expr;
    Ok(match expr {
        Expr::Var(n) => Expr::Var(desugar_qualified_ref(&n, allowed)?),
        Expr::Ctor(n) => Expr::Ctor(desugar_qualified_ref(&n, allowed)?),
        Expr::Lambda { params, body } => Expr::Lambda {
            params,
            body: Box::new(desugar_qualified_expr(*body, allowed)?),
        },
        Expr::Apply { func, args } => Expr::Apply {
            func: Box::new(desugar_qualified_expr(*func, allowed)?),
            args: args
                .into_iter()
                .map(|e| desugar_qualified_expr(e, allowed))
                .collect::<Result<Vec<_>>>()?,
        },
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => Expr::If {
            cond: Box::new(desugar_qualified_expr(*cond, allowed)?),
            then_branch: Box::new(desugar_qualified_expr(*then_branch, allowed)?),
            else_branch: Box::new(desugar_qualified_expr(*else_branch, allowed)?),
        },
        Expr::Let { bindings, body } => Expr::Let {
            bindings: bindings
                .into_iter()
                .map(|b| desugar_qualified_binding(b, allowed))
                .collect::<Result<Vec<_>>>()?,
            body: Box::new(desugar_qualified_expr(*body, allowed)?),
        },
        Expr::Where { expr, bindings } => Expr::Where {
            expr: Box::new(desugar_qualified_expr(*expr, allowed)?),
            bindings: bindings
                .into_iter()
                .map(|b| desugar_qualified_binding(b, allowed))
                .collect::<Result<Vec<_>>>()?,
        },
        Expr::Annot { expr, ty } => Expr::Annot {
            expr: Box::new(desugar_qualified_expr(*expr, allowed)?),
            ty: desugar_qualified_qual_type(ty, allowed)?,
        },
        Expr::Do(stmts) => Expr::Do(
            stmts
                .into_iter()
                .map(|s| desugar_qualified_do_stmt(s, allowed))
                .collect::<Result<Vec<_>>>()?,
        ),
        Expr::Case { expr, arms } => Expr::Case {
            expr: Box::new(desugar_qualified_expr(*expr, allowed)?),
            arms: arms
                .into_iter()
                .map(|a| desugar_qualified_case_arm(a, allowed))
                .collect::<Result<Vec<_>>>()?,
        },
        Expr::Cons { head, tail } => Expr::Cons {
            head: Box::new(desugar_qualified_expr(*head, allowed)?),
            tail: Box::new(desugar_qualified_expr(*tail, allowed)?),
        },
        Expr::List(es) => Expr::List(
            es.into_iter()
                .map(|e| desugar_qualified_expr(e, allowed))
                .collect::<Result<Vec<_>>>()?,
        ),
        Expr::Tuple(es) => Expr::Tuple(
            es.into_iter()
                .map(|e| desugar_qualified_expr(e, allowed))
                .collect::<Result<Vec<_>>>()?,
        ),
        Expr::Record(fs) => Expr::Record(
            fs.into_iter()
                .map(|(l, e)| Ok((l, desugar_qualified_expr(e, allowed)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
        x => x,
    })
}

fn desugar_qualified_case_arm(arm: ast::CaseArm, allowed: &HashSet<String>) -> Result<ast::CaseArm> {
    Ok(ast::CaseArm {
        pat: desugar_qualified_pattern(arm.pat, allowed)?,
        guard: arm
            .guard
            .map(|e| desugar_qualified_expr(e, allowed))
            .transpose()?,
        body: desugar_qualified_expr(arm.body, allowed)?,
    })
}

fn desugar_qualified_do_stmt(stmt: ast::DoStmt, allowed: &HashSet<String>) -> Result<ast::DoStmt> {
    Ok(match stmt {
        ast::DoStmt::Bind { pat, expr } => ast::DoStmt::Bind {
            pat: desugar_qualified_pattern(pat, allowed)?,
            expr: desugar_qualified_expr(expr, allowed)?,
        },
        ast::DoStmt::Expr(e) => ast::DoStmt::Expr(desugar_qualified_expr(e, allowed)?),
    })
}

fn desugar_qualified_binding(b: ast::Binding, allowed: &HashSet<String>) -> Result<ast::Binding> {
    Ok(ast::Binding {
        pat: desugar_qualified_pattern(b.pat, allowed)?,
        expr: desugar_qualified_expr(b.expr, allowed)?,
    })
}

fn desugar_qualified_pattern(p: ast::Pattern, allowed: &HashSet<String>) -> Result<ast::Pattern> {
    use ast::Pattern;
    Ok(match p {
        Pattern::Var(n) => {
            if n.contains('.') {
                return Err(Error::msg(format!(
                    "qualified name is not allowed in binder: {n}"
                )));
            }
            Pattern::Var(n)
        }
        Pattern::As(n, p) => {
            if n.contains('.') {
                return Err(Error::msg(format!(
                    "qualified name is not allowed in binder: {n}"
                )));
            }
            Pattern::As(n, Box::new(desugar_qualified_pattern(*p, allowed)?))
        }
        Pattern::Tuple(ps) => Pattern::Tuple(
            ps.into_iter()
                .map(|p| desugar_qualified_pattern(p, allowed))
                .collect::<Result<Vec<_>>>()?,
        ),
        Pattern::List(ps) => Pattern::List(
            ps.into_iter()
                .map(|p| desugar_qualified_pattern(p, allowed))
                .collect::<Result<Vec<_>>>()?,
        ),
        Pattern::Record(fs) => Pattern::Record(
            fs.into_iter()
                .map(|(l, p)| Ok((l, desugar_qualified_pattern(p, allowed)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
        Pattern::RecordLoose(fs, rest) => {
            if rest.as_ref().is_some_and(|n| n.contains('.')) {
                return Err(Error::msg(format!(
                    "qualified name is not allowed in binder: {}",
                    rest.unwrap()
                )));
            }
            Pattern::RecordLoose(
                fs.into_iter()
                    .map(|(l, p)| Ok((l, desugar_qualified_pattern(p, allowed)?)))
                    .collect::<Result<Vec<_>>>()?,
                rest,
            )
        }
        Pattern::Cons(a, b) => Pattern::Cons(
            Box::new(desugar_qualified_pattern(*a, allowed)?),
            Box::new(desugar_qualified_pattern(*b, allowed)?),
        ),
        Pattern::Or(a, b) => Pattern::Or(
            Box::new(desugar_qualified_pattern(*a, allowed)?),
            Box::new(desugar_qualified_pattern(*b, allowed)?),
        ),
        Pattern::View(p, e) => Pattern::View(
            Box::new(desugar_qualified_pattern(*p, allowed)?),
            Box::new(desugar_qualified_expr(*e, allowed)?),
        ),
        Pattern::Constructor { name, args } => Pattern::Constructor {
            name: desugar_qualified_ref(&name, allowed)?,
            args: args
                .into_iter()
                .map(|p| desugar_qualified_pattern(p, allowed))
                .collect::<Result<Vec<_>>>()?,
        },
        Pattern::Literal(e) => Pattern::Literal(desugar_qualified_expr(e, allowed)?),
        x => x,
    })
}

fn desugar_qualified_type(ty: ast::Type, allowed: &HashSet<String>) -> Result<ast::Type> {
    use ast::Type;
    Ok(match ty {
        Type::List(t) => Type::List(Box::new(desugar_qualified_type(*t, allowed)?)),
        Type::Tuple(ts) => Type::Tuple(
            ts.into_iter()
                .map(|t| desugar_qualified_type(t, allowed))
                .collect::<Result<Vec<_>>>()?,
        ),
        Type::Record(fs) => Type::Record(
            fs.into_iter()
                .map(|(l, t)| Ok((l, desugar_qualified_type(t, allowed)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
        Type::RecordOpen(fs, r) => Type::RecordOpen(
            fs.into_iter()
                .map(|(l, t)| Ok((l, desugar_qualified_type(t, allowed)?)))
                .collect::<Result<Vec<_>>>()?,
            Box::new(desugar_qualified_type(*r, allowed)?),
        ),
        Type::Var(n) => Type::Var(desugar_qualified_ref(&n, allowed)?),
        Type::App { head, args } => Type::App {
            head: Box::new(desugar_qualified_type(*head, allowed)?),
            args: args
                .into_iter()
                .map(|t| desugar_qualified_type(t, allowed))
                .collect::<Result<Vec<_>>>()?,
        },
        Type::Func(a, b) => Type::Func(
            Box::new(desugar_qualified_type(*a, allowed)?),
            Box::new(desugar_qualified_type(*b, allowed)?),
        ),
        x => x,
    })
}

fn desugar_qualified_predicate(p: ast::Predicate, allowed: &HashSet<String>) -> Result<ast::Predicate> {
    Ok(match p {
        ast::Predicate::Show(t) => ast::Predicate::Show(desugar_qualified_type(t, allowed)?),
        ast::Predicate::ShowRow(t) => ast::Predicate::ShowRow(desugar_qualified_type(t, allowed)?),
        ast::Predicate::Lacks { label, row } => ast::Predicate::Lacks {
            label,
            row: desugar_qualified_type(row, allowed)?,
        },
    })
}

fn desugar_qualified_qual_type(qt: ast::QualType, allowed: &HashSet<String>) -> Result<ast::QualType> {
    Ok(ast::QualType {
        preds: qt
            .preds
            .into_iter()
            .map(|p| desugar_qualified_predicate(p, allowed))
            .collect::<Result<Vec<_>>>()?,
        ty: desugar_qualified_type(qt.ty, allowed)?,
    })
}

fn desugar_module_qualified_names(module: &mut ast::Module) -> Result<()> {
    let allowed = module_allowed_qualifiers(module);

    module.items = module
        .items
        .clone()
        .into_iter()
        .map(|it| {
            Ok(match it {
                ast::Item::Binding(b) => ast::Item::Binding(desugar_qualified_binding(b, &allowed)?),
                ast::Item::TypeAlias(mut ta) => {
                    ta.ty = desugar_qualified_type(ta.ty, &allowed)?;
                    ast::Item::TypeAlias(ta)
                }
                ast::Item::DataDecl(mut dd) => {
                    for ctor in &mut dd.ctors {
                        ctor.args = ctor
                            .args
                            .clone()
                            .into_iter()
                            .map(|t| desugar_qualified_type(t, &allowed))
                            .collect::<Result<Vec<_>>>()?;
                    }
                    ast::Item::DataDecl(dd)
                }
                x @ (ast::Item::Import(_) | ast::Item::Export(_) | ast::Item::Fixity(_)) => x,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(())
}

impl ModuleLoader {
    fn load_ast(&mut self, path: &Path) -> Result<ast::Module> {
        if let Some(m) = self.cache.get(path) {
            return Ok(m.clone());
        }

        let src = std::fs::read_to_string(path)?;
        let mut m = parser::parse_module(&src)?;
        desugar_module_qualified_names(&mut m)?;

        self.cache.insert(path.to_path_buf(), m.clone());
        Ok(m)
    }

    fn collect_imports(
        &mut self,
        module: &ast::Module,
        dir: &Path,
        out: &mut Vec<ast::Item>,
    ) -> Result<()> {
        for it in &module.items {
            let ast::Item::Import(id) = it else {
                continue;
            };

            let local = dir.join(format!("{}.ks", id.module));
            let p = std::fs::canonicalize(&local)
                .or_else(|_| {
                    let stdlib = Path::new(env!("CARGO_MANIFEST_DIR")).join("stdlib");
                    std::fs::canonicalize(stdlib.join(format!("{}.ks", id.module)))
                })
                .map_err(|_| Error::msg(format!("cannot find module file for import {}", id.module)))?;

            if let Some(pos) = self.stack.iter().position(|x| x == &p) {
                let mut chain: Vec<String> = self.stack[pos..]
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect();
                chain.push(p.display().to_string());
                return Err(Error::msg(format!("cyclic imports: {}", chain.join(" -> "))));
            }

            let imported = self.load_ast(&p)?;
            let Some(name) = &imported.name else {
                return Err(Error::msg(format!(
                    "imported module {} must have a module header",
                    id.module
                )));
            };
            if name != &id.module {
                return Err(Error::msg(format!(
                    "module name mismatch: import {} but file declares module {}",
                    id.module, name
                )));
            }

            self.stack.push(p.clone());
            let imported_dir = p.parent().unwrap_or(dir);
            self.collect_imports(&imported, imported_dir, out)?;
            self.stack.pop();

            if self.emitted.insert(p) {
                out.extend(import_items_for_decl(&imported, id)?);
            }
        }
        Ok(())
    }
}

fn import_items(module: &ast::Module) -> Vec<ast::Item> {
    module
        .items
        .iter()
        .filter_map(|it| match it {
            ast::Item::Import(_) | ast::Item::Export(_) | ast::Item::Fixity(_) => None,
            it => Some(it.clone()),
        })
        .collect()
}

fn module_exported_names(module: &ast::Module) -> HashSet<String> {
    let mut exports = HashSet::new();
    for it in &module.items {
        if let ast::Item::Export(ed) = it {
            exports.extend(ed.names.iter().cloned());
        }
    }

    if exports.is_empty() {
        let mut all = HashSet::new();
        for it in &module.items {
            match it {
                ast::Item::Binding(b) => pat_defined_names(&b.pat, &mut all),
                ast::Item::TypeAlias(ta) => {
                    all.insert(ta.name.clone());
                }
                ast::Item::DataDecl(d) => {
                    all.insert(d.name.clone());
                    all.extend(d.ctors.iter().map(|c| c.name.clone()));
                }
                ast::Item::Import(_) | ast::Item::Export(_) | ast::Item::Fixity(_) => {}
            }
        }
        return all;
    }

    // If a data type name is exported, also export its constructors.
    for it in &module.items {
        let ast::Item::DataDecl(d) = it else {
            continue;
        };
        if exports.contains(&d.name) {
            exports.extend(d.ctors.iter().map(|c| c.name.clone()));
        }
    }

    exports
}

fn import_items_for_decl(module: &ast::Module, decl: &ast::ImportDecl) -> Result<Vec<ast::Item>> {
    // Haskell-leaning behavior:
    // - `import A` brings unqualified exports + qualifier `A.`
    // - `import A as OM` acts like `import qualified A as OM` (qualified-only)
    if let Some(as_name) = &decl.as_name {
        return qualify_items(module, as_name);
    }

    let mut out = Vec::new();

    // Always provide module-qualified names (A.x).
    out.extend(qualify_items(module, &decl.module)?);

    // Bring unqualified exports as simple forwarders: `x = A.x`.
    let exports = module_exported_names(module);

    let mut values = HashSet::new();
    let mut type_aliases = HashMap::new();
    for it in import_items(module) {
        match it {
            ast::Item::Binding(b) => pat_defined_names(&b.pat, &mut values),
            ast::Item::TypeAlias(ta) => {
                type_aliases.insert(ta.name.clone(), ta);
            }
            ast::Item::DataDecl(d) => {
                values.extend(d.ctors.iter().map(|c| c.name.clone()));
            }
            ast::Item::Import(_) | ast::Item::Export(_) | ast::Item::Fixity(_) => {}
        }
    }

    for n in exports.iter() {
        if values.contains(n) {
            out.push(ast::Item::Binding(ast::Binding {
                pat: ast::Pattern::Var(n.clone()),
                expr: ast::Expr::Var(format!("{}.{}", decl.module, n)),
            }));
        }

        if let Some(ta) = type_aliases.get(n) {
            let head = ast::Type::Var(format!("{}.{}", decl.module, ta.name));
            let ty = if ta.params.is_empty() {
                head
            } else {
                ast::Type::App {
                    head: Box::new(head),
                    args: ta.params.iter().cloned().map(ast::Type::Var).collect(),
                }
            };
            out.push(ast::Item::TypeAlias(ast::TypeAlias {
                name: ta.name.clone(),
                params: ta.params.clone(),
                ty,
            }));
        }
    }

    Ok(out)
}

fn qualify_items(module: &ast::Module, qual: &str) -> Result<Vec<ast::Item>> {
    let mut values = HashSet::new();
    let mut types = HashSet::new();
    let mut ctors = HashSet::new();

    for it in import_items(module) {
        match &it {
            ast::Item::Binding(b) => pat_defined_names(&b.pat, &mut values),
            ast::Item::TypeAlias(ta) => {
                types.insert(ta.name.clone());
            }
            ast::Item::DataDecl(d) => {
                types.insert(d.name.clone());
                ctors.extend(d.ctors.iter().map(|c| c.name.clone()));
            }
            ast::Item::Import(_) | ast::Item::Export(_) | ast::Item::Fixity(_) => {}
        }
    }

    let val_map: HashMap<String, String> = values
        .iter()
        .map(|n| (n.clone(), format!("{qual}.{n}")))
        .collect();
    let type_map: HashMap<String, String> = types
        .iter()
        .map(|n| (n.clone(), format!("{qual}.{n}")))
        .collect();
    let ctor_map: HashMap<String, String> = ctors
        .iter()
        .map(|n| (n.clone(), format!("{qual}.{n}")))
        .collect();

    import_items(module)
        .into_iter()
        .map(|it| qualify_item(it, &val_map, &type_map, &ctor_map))
        .collect::<Result<Vec<_>>>()
}

fn qualify_item(
    it: ast::Item,
    val_map: &HashMap<String, String>,
    type_map: &HashMap<String, String>,
    ctor_map: &HashMap<String, String>,
) -> Result<ast::Item> {
    Ok(match it {
        ast::Item::Binding(b) => ast::Item::Binding(ast::Binding {
            pat: qualify_pat_binders(b.pat, val_map)?,
            expr: qualify_expr(b.expr, val_map, type_map, ctor_map)?,
        }),
        ast::Item::TypeAlias(mut ta) => {
            ta.name = type_map.get(&ta.name).cloned().unwrap_or(ta.name);
            ta.ty = qualify_type(ta.ty, type_map)?;
            ast::Item::TypeAlias(ta)
        }
        ast::Item::DataDecl(mut d) => {
            d.name = type_map.get(&d.name).cloned().unwrap_or(d.name);
            for ctor in &mut d.ctors {
                ctor.name = ctor_map.get(&ctor.name).cloned().unwrap_or(ctor.name.clone());
                ctor.args = ctor
                    .args
                    .clone()
                    .into_iter()
                    .map(|t| qualify_type(t, type_map))
                    .collect::<Result<Vec<_>>>()?;
            }
            ast::Item::DataDecl(d)
        }
        x @ (ast::Item::Import(_) | ast::Item::Export(_) | ast::Item::Fixity(_)) => x,
    })
}

fn qualify_expr(
    expr: ast::Expr,
    val_map: &HashMap<String, String>,
    type_map: &HashMap<String, String>,
    ctor_map: &HashMap<String, String>,
) -> Result<ast::Expr> {
    use ast::Expr;
    Ok(match expr {
        Expr::Var(n) => Expr::Var(val_map.get(&n).cloned().unwrap_or(n)),
        Expr::Ctor(n) => Expr::Ctor(ctor_map.get(&n).cloned().unwrap_or(n)),
        Expr::Lambda { params, body } => Expr::Lambda {
            params,
            body: Box::new(qualify_expr(*body, val_map, type_map, ctor_map)?),
        },
        Expr::Apply { func, args } => Expr::Apply {
            func: Box::new(qualify_expr(*func, val_map, type_map, ctor_map)?),
            args: args
                .into_iter()
                .map(|e| qualify_expr(e, val_map, type_map, ctor_map))
                .collect::<Result<Vec<_>>>()?,
        },
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => Expr::If {
            cond: Box::new(qualify_expr(*cond, val_map, type_map, ctor_map)?),
            then_branch: Box::new(qualify_expr(*then_branch, val_map, type_map, ctor_map)?),
            else_branch: Box::new(qualify_expr(*else_branch, val_map, type_map, ctor_map)?),
        },
        Expr::Let { bindings, body } => Expr::Let {
            bindings: bindings
                .into_iter()
                .map(|b| qualify_local_binding(b, val_map, type_map, ctor_map))
                .collect::<Result<Vec<_>>>()?,
            body: Box::new(qualify_expr(*body, val_map, type_map, ctor_map)?),
        },
        Expr::Where { expr, bindings } => Expr::Where {
            expr: Box::new(qualify_expr(*expr, val_map, type_map, ctor_map)?),
            bindings: bindings
                .into_iter()
                .map(|b| qualify_local_binding(b, val_map, type_map, ctor_map))
                .collect::<Result<Vec<_>>>()?,
        },
        Expr::Annot { expr, ty } => Expr::Annot {
            expr: Box::new(qualify_expr(*expr, val_map, type_map, ctor_map)?),
            ty: qualify_qual_type(ty, type_map)?,
        },
        Expr::Do(stmts) => Expr::Do(
            stmts
                .into_iter()
                .map(|s| qualify_do_stmt(s, val_map, type_map, ctor_map))
                .collect::<Result<Vec<_>>>()?,
        ),
        Expr::Case { expr, arms } => Expr::Case {
            expr: Box::new(qualify_expr(*expr, val_map, type_map, ctor_map)?),
            arms: arms
                .into_iter()
                .map(|a| qualify_case_arm(a, val_map, type_map, ctor_map))
                .collect::<Result<Vec<_>>>()?,
        },
        Expr::Cons { head, tail } => Expr::Cons {
            head: Box::new(qualify_expr(*head, val_map, type_map, ctor_map)?),
            tail: Box::new(qualify_expr(*tail, val_map, type_map, ctor_map)?),
        },
        Expr::List(es) => Expr::List(
            es.into_iter()
                .map(|e| qualify_expr(e, val_map, type_map, ctor_map))
                .collect::<Result<Vec<_>>>()?,
        ),
        Expr::Tuple(es) => Expr::Tuple(
            es.into_iter()
                .map(|e| qualify_expr(e, val_map, type_map, ctor_map))
                .collect::<Result<Vec<_>>>()?,
        ),
        Expr::Record(fs) => Expr::Record(
            fs.into_iter()
                .map(|(l, e)| Ok((l, qualify_expr(e, val_map, type_map, ctor_map)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
        x => x,
    })
}

fn qualify_case_arm(
    arm: ast::CaseArm,
    val_map: &HashMap<String, String>,
    type_map: &HashMap<String, String>,
    ctor_map: &HashMap<String, String>,
) -> Result<ast::CaseArm> {
    Ok(ast::CaseArm {
        pat: qualify_pat_nonbinders(arm.pat, ctor_map, val_map, type_map)?,
        guard: arm
            .guard
            .map(|e| qualify_expr(e, val_map, type_map, ctor_map))
            .transpose()?,
        body: qualify_expr(arm.body, val_map, type_map, ctor_map)?,
    })
}

fn qualify_do_stmt(
    stmt: ast::DoStmt,
    val_map: &HashMap<String, String>,
    type_map: &HashMap<String, String>,
    ctor_map: &HashMap<String, String>,
) -> Result<ast::DoStmt> {
    Ok(match stmt {
        ast::DoStmt::Bind { pat, expr } => ast::DoStmt::Bind {
            pat: qualify_pat_nonbinders(pat, ctor_map, val_map, type_map)?,
            expr: qualify_expr(expr, val_map, type_map, ctor_map)?,
        },
        ast::DoStmt::Expr(e) => ast::DoStmt::Expr(qualify_expr(e, val_map, type_map, ctor_map)?),
    })
}

fn qualify_local_binding(
    b: ast::Binding,
    val_map: &HashMap<String, String>,
    type_map: &HashMap<String, String>,
    ctor_map: &HashMap<String, String>,
) -> Result<ast::Binding> {
    Ok(ast::Binding {
        pat: qualify_pat_nonbinders(b.pat, ctor_map, val_map, type_map)?,
        expr: qualify_expr(b.expr, val_map, type_map, ctor_map)?,
    })
}

fn qualify_pat_binders(p: ast::Pattern, val_map: &HashMap<String, String>) -> Result<ast::Pattern> {
    use ast::Pattern;
    Ok(match p {
        Pattern::Var(n) => Pattern::Var(val_map.get(&n).cloned().unwrap_or(n)),
        Pattern::As(n, p) => Pattern::As(
            val_map.get(&n).cloned().unwrap_or(n),
            Box::new(qualify_pat_binders(*p, val_map)?),
        ),
        Pattern::Tuple(ps) => Pattern::Tuple(
            ps.into_iter()
                .map(|p| qualify_pat_binders(p, val_map))
                .collect::<Result<Vec<_>>>()?,
        ),
        Pattern::List(ps) => Pattern::List(
            ps.into_iter()
                .map(|p| qualify_pat_binders(p, val_map))
                .collect::<Result<Vec<_>>>()?,
        ),
        Pattern::Record(fs) => Pattern::Record(
            fs.into_iter()
                .map(|(l, p)| Ok((l, qualify_pat_binders(p, val_map)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
        Pattern::RecordLoose(fs, rest) => Pattern::RecordLoose(
            fs.into_iter()
                .map(|(l, p)| Ok((l, qualify_pat_binders(p, val_map)?)))
                .collect::<Result<Vec<_>>>()?,
            rest.map(|n| val_map.get(&n).cloned().unwrap_or(n)),
        ),
        Pattern::Cons(a, b) => Pattern::Cons(
            Box::new(qualify_pat_binders(*a, val_map)?),
            Box::new(qualify_pat_binders(*b, val_map)?),
        ),
        Pattern::Or(a, b) => Pattern::Or(
            Box::new(qualify_pat_binders(*a, val_map)?),
            Box::new(qualify_pat_binders(*b, val_map)?),
        ),
        Pattern::View(p, e) => Pattern::View(
            Box::new(qualify_pat_binders(*p, val_map)?),
            e,
        ),
        Pattern::Constructor { name, args } => Pattern::Constructor { name, args },
        x => x,
    })
}

fn qualify_pat_nonbinders(
    p: ast::Pattern,
    ctor_map: &HashMap<String, String>,
    val_map: &HashMap<String, String>,
    type_map: &HashMap<String, String>,
) -> Result<ast::Pattern> {
    let _ = type_map;
    use ast::Pattern;
    Ok(match p {
        Pattern::Tuple(ps) => Pattern::Tuple(
            ps.into_iter()
                .map(|p| qualify_pat_nonbinders(p, ctor_map, val_map, type_map))
                .collect::<Result<Vec<_>>>()?,
        ),
        Pattern::List(ps) => Pattern::List(
            ps.into_iter()
                .map(|p| qualify_pat_nonbinders(p, ctor_map, val_map, type_map))
                .collect::<Result<Vec<_>>>()?,
        ),
        Pattern::Record(fs) => Pattern::Record(
            fs.into_iter()
                .map(|(l, p)| Ok((l, qualify_pat_nonbinders(p, ctor_map, val_map, type_map)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
        Pattern::RecordLoose(fs, rest) => Pattern::RecordLoose(
            fs.into_iter()
                .map(|(l, p)| Ok((l, qualify_pat_nonbinders(p, ctor_map, val_map, type_map)?)))
                .collect::<Result<Vec<_>>>()?,
            rest,
        ),
        Pattern::Cons(a, b) => Pattern::Cons(
            Box::new(qualify_pat_nonbinders(*a, ctor_map, val_map, type_map)?),
            Box::new(qualify_pat_nonbinders(*b, ctor_map, val_map, type_map)?),
        ),
        Pattern::Or(a, b) => Pattern::Or(
            Box::new(qualify_pat_nonbinders(*a, ctor_map, val_map, type_map)?),
            Box::new(qualify_pat_nonbinders(*b, ctor_map, val_map, type_map)?),
        ),
        Pattern::As(n, p) => Pattern::As(
            n,
            Box::new(qualify_pat_nonbinders(*p, ctor_map, val_map, type_map)?),
        ),
        Pattern::View(p, e) => Pattern::View(
            Box::new(qualify_pat_nonbinders(*p, ctor_map, val_map, type_map)?),
            Box::new(qualify_expr(*e, val_map, type_map, ctor_map)?),
        ),
        Pattern::Constructor { name, args } => Pattern::Constructor {
            name: ctor_map.get(&name).cloned().unwrap_or(name),
            args: args
                .into_iter()
                .map(|p| qualify_pat_nonbinders(p, ctor_map, val_map, type_map))
                .collect::<Result<Vec<_>>>()?,
        },
        Pattern::Literal(e) => Pattern::Literal(qualify_expr(e, val_map, type_map, ctor_map)?),
        x => x,
    })
}

fn qualify_type(ty: ast::Type, type_map: &HashMap<String, String>) -> Result<ast::Type> {
    use ast::Type;
    Ok(match ty {
        Type::List(t) => Type::List(Box::new(qualify_type(*t, type_map)?)),
        Type::Tuple(ts) => Type::Tuple(
            ts.into_iter()
                .map(|t| qualify_type(t, type_map))
                .collect::<Result<Vec<_>>>()?,
        ),
        Type::Record(fs) => Type::Record(
            fs.into_iter()
                .map(|(l, t)| Ok((l, qualify_type(t, type_map)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
        Type::RecordOpen(fs, r) => Type::RecordOpen(
            fs.into_iter()
                .map(|(l, t)| Ok((l, qualify_type(t, type_map)?)))
                .collect::<Result<Vec<_>>>()?,
            Box::new(qualify_type(*r, type_map)?),
        ),
        Type::Var(n) => Type::Var(type_map.get(&n).cloned().unwrap_or(n)),
        Type::App { head, args } => Type::App {
            head: Box::new(qualify_type(*head, type_map)?),
            args: args
                .into_iter()
                .map(|t| qualify_type(t, type_map))
                .collect::<Result<Vec<_>>>()?,
        },
        Type::Func(a, b) => Type::Func(
            Box::new(qualify_type(*a, type_map)?),
            Box::new(qualify_type(*b, type_map)?),
        ),
        x => x,
    })
}

fn qualify_predicate(p: ast::Predicate, type_map: &HashMap<String, String>) -> Result<ast::Predicate> {
    Ok(match p {
        ast::Predicate::Show(t) => ast::Predicate::Show(qualify_type(t, type_map)?),
        ast::Predicate::ShowRow(t) => ast::Predicate::ShowRow(qualify_type(t, type_map)?),
        ast::Predicate::Lacks { label, row } => ast::Predicate::Lacks {
            label,
            row: qualify_type(row, type_map)?,
        },
    })
}

fn qualify_qual_type(qt: ast::QualType, type_map: &HashMap<String, String>) -> Result<ast::QualType> {
    Ok(ast::QualType {
        preds: qt
            .preds
            .into_iter()
            .map(|p| qualify_predicate(p, type_map))
            .collect::<Result<Vec<_>>>()?,
        ty: qualify_type(qt.ty, type_map)?,
    })
}

fn push_item_checked(items: &mut Vec<ast::Item>, defined: &mut HashSet<String>, it: ast::Item) -> Result<()> {
    let mut names = HashSet::new();
    item_defined_names(&it, &mut names);
    for n in names {
        if !defined.insert(n.clone()) {
            return Err(Error::msg(format!("name conflict: {n}")));
        }
    }
    items.push(it);
    Ok(())
}

fn item_defined_names(it: &ast::Item, out: &mut HashSet<String>) {
    match it {
        ast::Item::Binding(b) => pat_defined_names(&b.pat, out),
        ast::Item::TypeAlias(ta) => {
            out.insert(ta.name.clone());
        }
        ast::Item::DataDecl(d) => {
            out.insert(d.name.clone());
            out.extend(d.ctors.iter().map(|c| c.name.clone()));
        }
        ast::Item::Import(_) | ast::Item::Export(_) | ast::Item::Fixity(_) => {}
    }
}

fn pat_defined_names(p: &ast::Pattern, out: &mut HashSet<String>) {
    use ast::Pattern;
    match p {
        Pattern::Var(n) => {
            out.insert(n.clone());
        }
        Pattern::As(n, p) => {
            out.insert(n.clone());
            pat_defined_names(p, out);
        }
        Pattern::Tuple(ps) | Pattern::List(ps) => {
            for p in ps {
                pat_defined_names(p, out);
            }
        }
        Pattern::Record(fs) | Pattern::RecordLoose(fs, _) => {
            for (_, p) in fs {
                pat_defined_names(p, out);
            }
            if let Pattern::RecordLoose(_, Some(rest)) = p {
                out.insert(rest.clone());
            }
        }
        Pattern::Cons(a, b) | Pattern::Or(a, b) => {
            pat_defined_names(a, out);
            pat_defined_names(b, out);
        }
        Pattern::View(p, _) => pat_defined_names(p, out),
        Pattern::Constructor { args, .. } => {
            for p in args {
                pat_defined_names(p, out);
            }
        }
        Pattern::Wildcard | Pattern::Hole(_) | Pattern::Literal(_) => {}
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
        if !main.vars.is_empty() || !main.constraints.is_empty() || main.ty != expected {
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
        it @ (ast::Item::Import(_) | ast::Item::Export(_) | ast::Item::Fixity(_)) => Ok(it),
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
        Pattern::RecordLoose(fields, rest) => Pattern::RecordLoose(
            fields
                .into_iter()
                .map(|(n, p)| Ok((n, expand_pat(p, aliases)?)))
                .collect::<Result<Vec<_>>>()?,
            rest,
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
            ty: expand_qual_type(ty, aliases)?,
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

fn expand_qual_type(ty: ast::QualType, aliases: &HashMap<String, ast::TypeAlias>) -> Result<ast::QualType> {
    let mut stack = Vec::new();
    let preds = ty
        .preds
        .into_iter()
        .map(|p| {
            Ok(match p {
                ast::Predicate::Show(t) => ast::Predicate::Show(expand_type(t, aliases, &mut stack)?),
                ast::Predicate::ShowRow(t) => {
                    ast::Predicate::ShowRow(expand_type(t, aliases, &mut stack)?)
                }
                ast::Predicate::Lacks { label, row } => ast::Predicate::Lacks {
                    label,
                    row: expand_type(row, aliases, &mut stack)?,
                },
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let ty2 = expand_type(ty.ty, aliases, &mut stack)?;
    Ok(ast::QualType { preds, ty: ty2 })
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
        Type::RecordOpen(fields, rest) => Type::RecordOpen(
            fields
                .into_iter()
                .map(|(n, t)| Ok((n, expand_type(t, aliases, stack)?)))
                .collect::<Result<Vec<_>>>()?,
            Box::new(expand_type(*rest, aliases, stack)?),
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
        Type::RecordOpen(fields, rest) => Type::RecordOpen(
            fields
                .into_iter()
                .map(|(n, t)| (n, subst_type(t, env)))
                .collect(),
            Box::new(subst_type(*rest, env)),
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
    fn typecheck_file_imports_data_type_exported_by_type_name() {
        let dir = std::env::temp_dir().join(format!("kscr_typecheck_file_imports_ok_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("A.ks");
        std::fs::write(
            &a,
            "module A where\n  export Maybe\n  data Maybe a = Nothing | Just a\n",
        )
        .unwrap();

        let main = dir.join("Main.ks");
        std::fs::write(
            &main,
            "module Main where\n  import A\n  x = Just 1\n  main = IO ()\n",
        )
        .unwrap();

        let _tm = typecheck_file(&main).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn typecheck_file_imports_module_with_private_helpers() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_typecheck_file_imports_helpers_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("A.ks");
        std::fs::write(
            &a,
            "module A where\n  export f\n  g = \\x -> x\n  f = \\y -> g y\n",
        )
        .unwrap();

        let main = dir.join("Main.ks");
        std::fs::write(
            &main,
            "module Main where\n  import A\n  x = f 1\n  main = IO ()\n",
        )
        .unwrap();

        let _tm = typecheck_file(&main).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn typecheck_file_imports_respect_exports_unqualified() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_typecheck_file_imports_exports_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("A.ks");
        std::fs::write(
            &a,
            "module A where\n  export f\n  g = \\x -> x\n  f = \\y -> g y\n",
        )
        .unwrap();

        let main = dir.join("Main.ks");
        std::fs::write(
            &main,
            "module Main where\n  import A\n  x = g 1\n  main = IO ()\n",
        )
        .unwrap();

        let e = typecheck_file(&main).unwrap_err();
        assert!(format!("{e}").contains("unbound variable: g"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn typecheck_file_import_as_allows_dotted_refs() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_typecheck_file_import_as_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("A.ks");
        std::fs::write(&a, "module A where\n  x = 1\n").unwrap();

        let main = dir.join("Main.ks");
        std::fs::write(
            &main,
            "module Main where\n  import A as OM\n  y = OM.x + 1\n  main = IO ()\n",
        )
        .unwrap();

        let _tm = typecheck_file(&main).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn typecheck_file_import_as_disambiguates_same_name() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_typecheck_file_import_as_disambiguates_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("A.ks");
        std::fs::write(&a, "module A where\n  x = 1\n").unwrap();

        let b = dir.join("B.ks");
        std::fs::write(&b, "module B where\n  x = 2\n").unwrap();

        let main = dir.join("Main.ks");
        std::fs::write(
            &main,
            "module Main where\n  import A as A1\n  import B as B1\n  y = A1.x + B1.x\n  main = IO ()\n",
        )
        .unwrap();

        let _tm = typecheck_file(&main).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn typecheck_file_allows_module_qualifier_without_as() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_typecheck_file_module_qualifier_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("A.ks");
        std::fs::write(&a, "module A where\n  x = 1\n").unwrap();

        let main = dir.join("Main.ks");
        std::fs::write(
            &main,
            "module Main where\n  import A\n  y = A.x + 1\n  main = IO ()\n",
        )
        .unwrap();

        let _tm = typecheck_file(&main).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn typecheck_file_rejects_unknown_qualifier() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_typecheck_file_unknown_qualifier_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("A.ks");
        std::fs::write(&a, "module A where\n  x = 1\n").unwrap();

        let main = dir.join("Main.ks");
        std::fs::write(
            &main,
            "module Main where\n  import A\n  y = Q.x + 1\n  main = IO ()\n",
        )
        .unwrap();

        let e = typecheck_file(&main).unwrap_err();
        assert!(format!("{e}").contains("unknown qualifier Q"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn typecheck_file_reports_cyclic_imports() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_typecheck_file_cyclic_imports_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("A.ks");
        std::fs::write(&a, "module A where\n  import B\n  x = 1\n").unwrap();

        let b = dir.join("B.ks");
        std::fs::write(&b, "module B where\n  import A\n  y = 2\n").unwrap();

        let e = typecheck_file(&a).unwrap_err();
        assert!(format!("{e}").contains("cyclic imports"));

        let _ = std::fs::remove_dir_all(dir);
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
    fn scheme_display_includes_single_constraint() {
        let s = Scheme {
            vars: vec![2],
            constraints: vec![Constraint::Show(Ty::Var(2))],
            ty: Ty::Func(Box::new(Ty::Var(2)), Box::new(Ty::Con("String".to_string()))),
        };
        assert_eq!(format!("{s}"), "forall a. Show a => a -> String");
    }

    #[test]
    fn scheme_display_includes_multiple_constraints() {
        let s = Scheme {
            vars: vec![2, 3],
            constraints: vec![
                Constraint::Show(Ty::Var(2)),
                Constraint::Lacks {
                    label: "x".to_string(),
                    row: Ty::Var(3),
                },
            ],
            ty: Ty::Func(Box::new(Ty::Var(3)), Box::new(Ty::Var(3))),
        };
        assert_eq!(format!("{s}"), "forall a b. (Show a, Lacks \"x\" b) => b -> b");
    }

    #[test]
    fn infer_annotated_show_constraint_roundtrips_via_display() {
        let m = crate::parser::parse_module("x = (\\y -> y) :: Show a => a -> a\n").unwrap();
        let env = infer_module(&m).unwrap();
        assert_eq!(format!("{}", env.get("x").unwrap()), "forall a. Show a => a -> a");
    }

    #[test]
    fn infer_annotated_lacks_constraint_roundtrips_via_display() {
        let m = crate::parser::parse_module("f = (\\r -> r) :: Lacks \"a\" r => r -> r\n").unwrap();
        let env = infer_module(&m).unwrap();
        assert_eq!(
            format!("{}", env.get("f").unwrap()),
            "forall a. Lacks \"a\" a => a -> a"
        );
    }

    fn strip_forall(s: &str) -> &str {
        match s.split_once(". ") {
            Some((_, rest)) => rest,
            None => s,
        }
    }

    fn parse_qual_type_from_str(s: &str) -> ast::QualType {
        let src = format!("x = 1 :: {s}\n");
        let m = crate::parser::parse_module(&src).unwrap();
        let ast::Item::Binding(b) = &m.items[0] else {
            panic!("expected binding");
        };
        let ast::Expr::Annot { ty, .. } = &b.expr else {
            panic!("expected annotation");
        };
        ty.clone()
    }

    fn lower_qual_type_for_test(qt: &ast::QualType) -> (Vec<Constraint>, Ty) {
        let mut cx = InferCtx::default();
        let mut holes = HashMap::new();

        let mut cs = Vec::new();
        for p in &qt.preds {
            match p {
                ast::Predicate::Show(t) => cs.push(Constraint::Show(lower_surface_type(&mut cx, t, &mut holes))),
                ast::Predicate::ShowRow(t) => {
                    cs.push(Constraint::ShowRow(lower_surface_type(&mut cx, t, &mut holes)))
                }
                ast::Predicate::Lacks { label, row } => cs.push(Constraint::Lacks {
                    label: label.clone(),
                    row: lower_surface_type(&mut cx, row, &mut holes),
                }),
            }
        }

        let t = lower_surface_type(&mut cx, &qt.ty, &mut holes);
        (cs, t)
    }

    fn canon_ty_in(ty: &Ty, m: &mut HashMap<u32, u32>, next: &mut u32) -> Ty {
        match ty {
            Ty::Var(v) => {
                let v2 = *m.entry(*v).or_insert_with(|| {
                    let n = *next;
                    *next += 1;
                    n
                });
                Ty::Var(v2)
            }
            Ty::Con(c) => Ty::Con(c.clone()),
            Ty::List(t) => Ty::List(Box::new(canon_ty_in(t, m, next))),
            Ty::Tuple(ts) => Ty::Tuple(ts.iter().map(|t| canon_ty_in(t, m, next)).collect()),
            Ty::Record(fields) => Ty::Record(
                fields
                    .iter()
                    .map(|(k, t)| (k.clone(), canon_ty_in(t, m, next)))
                    .collect(),
            ),
            Ty::RecordOpen(fields, rest) => Ty::RecordOpen(
                fields
                    .iter()
                    .map(|(k, t)| (k.clone(), canon_ty_in(t, m, next)))
                    .collect(),
                Box::new(canon_ty_in(rest, m, next)),
            ),
            Ty::App { head, args } => Ty::App {
                head: Box::new(canon_ty_in(head, m, next)),
                args: args.iter().map(|t| canon_ty_in(t, m, next)).collect(),
            },
            Ty::Func(a, b) => Ty::Func(
                Box::new(canon_ty_in(a, m, next)),
                Box::new(canon_ty_in(b, m, next)),
            ),
        }
    }

    fn canon_constraint_in(c: &Constraint, m: &mut HashMap<u32, u32>, next: &mut u32) -> Constraint {
        match c {
            Constraint::Show(t) => Constraint::Show(canon_ty_in(t, m, next)),
            Constraint::ShowRow(t) => Constraint::ShowRow(canon_ty_in(t, m, next)),
            Constraint::Lacks { label, row } => Constraint::Lacks {
                label: label.clone(),
                row: canon_ty_in(row, m, next),
            },
        }
    }

    fn canon(cs: &[Constraint], ty: &Ty) -> (Vec<Constraint>, Ty) {
        let mut m = HashMap::new();
        let mut next = 0;
        let cs2 = cs
            .iter()
            .map(|c| canon_constraint_in(c, &mut m, &mut next))
            .collect();
        let ty2 = canon_ty_in(ty, &mut m, &mut next);
        (cs2, ty2)
    }

    #[test]
    fn roundtrip_scheme_display_parse_open_record_type() {
        let s = Scheme {
            vars: vec![0, 1],
            constraints: vec![Constraint::Lacks {
                label: "x".to_string(),
                row: Ty::Var(1),
            }],
            ty: Ty::Func(
                Box::new(Ty::RecordOpen(
                    vec![("a".to_string(), Ty::Var(0))],
                    Box::new(Ty::Var(1)),
                )),
                Box::new(Ty::Var(0)),
            ),
        };

        let printed = format!("{s}");
        let qt = parse_qual_type_from_str(strip_forall(&printed));
        let (cs_p, ty_p) = lower_qual_type_for_test(&qt);

        assert_eq!(canon(&s.constraints, &s.ty), canon(&cs_p, &ty_p));
    }

    #[test]
    fn roundtrip_scheme_display_parse_showrow_open_record() {
        let s = Scheme {
            vars: vec![0],
            constraints: vec![Constraint::ShowRow(Ty::RecordOpen(
                vec![("a".to_string(), Ty::Con("Integer".to_string()))],
                Box::new(Ty::Var(0)),
            ))],
            ty: Ty::Con("Unit".to_string()),
        };

        let printed = format!("{s}");
        let qt = parse_qual_type_from_str(strip_forall(&printed));
        let (cs_p, ty_p) = lower_qual_type_for_test(&qt);

        assert_eq!(canon(&s.constraints, &s.ty), canon(&cs_p, &ty_p));
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
    fn infer_show_is_qualified() {
        let src = "f = show\n";
        let m = crate::parser::parse_module(src).unwrap();
        let env = infer_module(&m).unwrap();
        let s = env.get("f").unwrap();

        assert_eq!(s.constraints.len(), 1);
        let t = match &s.constraints[0] {
            Constraint::Show(t) => t,
            other => panic!("expected Show constraint, got {other:?}"),
        };

        let Ty::Func(a, b) = &s.ty else {
            panic!("expected function type");
        };
        assert_eq!(**b, Ty::Con("String".to_string()));
        assert_eq!(&**a, t);
    }

    #[test]
    fn infer_show_function_is_error() {
        let src = "x = show (\\y -> y)\n";
        let m = crate::parser::parse_module(src).unwrap();
        let _ = infer_module(&m).unwrap_err();
    }

    #[test]
    fn infer_show_list_tuple_record() {
        let src = "a = show [1, 2]\n\
 b = show (1, True)\n\
 c = show {a: 1, b: True}\n";
        let m = crate::parser::parse_module(src).unwrap();
        let env = infer_module(&m).unwrap();

        assert_eq!(env.get("a").unwrap(), &Scheme::mono(Ty::Con("String".to_string())));
        assert_eq!(env.get("b").unwrap(), &Scheme::mono(Ty::Con("String".to_string())));
        assert_eq!(env.get("c").unwrap(), &Scheme::mono(Ty::Con("String".to_string())));
    }

    #[test]
    fn infer_show_data_is_ok() {
        let src = r#"data Maybe a = Nothing | Just a
x = show (Just 1)
y = show Nothing
"#;
        let m = crate::parser::parse_module(src).unwrap();
        let env = infer_module(&m).unwrap();

        assert_eq!(env.get("x").unwrap(), &Scheme::mono(Ty::Con("String".to_string())));

        let y = env.get("y").unwrap();
        assert_eq!(y.ty, Ty::Con("String".to_string()));
        assert!(y.constraints.iter().any(|c| matches!(c, Constraint::Show(_))));
    }

    #[test]
    fn infer_show_data_with_function_field_is_error() {
        let src = r#"data Bad = Bad (Integer -> Integer)
x = show (Bad (\y -> y))
"#;
        let m = crate::parser::parse_module(src).unwrap();
        let _ = infer_module(&m).unwrap_err();
    }

    #[test]
    fn infer_show_open_record_requires_showrow() {
        let src = "f = \\r -> case r of\n  {a: a, ...} -> show r\n";
        let m = crate::parser::parse_module(src).unwrap();
        let env = infer_module(&m).unwrap();
        let s = env.get("f").unwrap();

        assert!(s.constraints.iter().any(|c| matches!(c, Constraint::Show(_))));
        assert!(s.constraints.iter().any(|c| matches!(c, Constraint::ShowRow(_))));
    }

    #[test]
    fn infer_record_loose_with_rest_adds_lacks() {
        let src = "f = \\r -> case r of\n  {a: a, ...rest} -> rest\n";
        let m = crate::parser::parse_module(src).unwrap();
        let env = infer_module(&m).unwrap();
        let s = env.get("f").unwrap();

        assert!(s
            .constraints
            .iter()
            .any(|c| matches!(c, Constraint::Lacks { label, .. } if label == "a")));
    }

    #[test]
    fn simplify_lacks_rejects_present_label() {
        let data_env = DataEnv::new();
        let cs = vec![Constraint::Lacks {
            label: "a".to_string(),
            row: Ty::Record(vec![("a".to_string(), Ty::Con("Integer".to_string()))]),
        }];
        assert!(simplify_constraints(&data_env, cs).is_err());
    }

    #[test]
    fn infer_annotation_mismatch_is_error() {
        let _ = infer_expr(ast::Expr::Annot {
            expr: Box::new(ast::Expr::Integer("1".to_string())),
            ty: ast::QualType {
                preds: vec![],
                ty: ast::Type::Bool,
            },
        })
        .unwrap_err();
    }

    #[test]
    fn infer_annotation_hole_resolves() {
        let ty = infer_expr(ast::Expr::Annot {
            expr: Box::new(ast::Expr::Integer("1".to_string())),
            ty: ast::QualType {
                preds: vec![],
                ty: ast::Type::Hole(None),
            },
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
