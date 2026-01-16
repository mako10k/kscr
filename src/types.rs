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
mod stdlib_cache;
mod toposort;
mod typeclass_dict_passing_common;
mod typeclass_dict_passing_rewrite;
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
    App {
        head: Box<Ty>,
        args: Vec<Ty>,
    },
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

fn last_ty_seg(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

fn backend_int_bits(name: &str) -> Option<u16> {
    let name = last_ty_seg(name);
    let rest = name.strip_prefix('i')?;
    rest.parse::<u16>().ok()
}

fn backend_float_bits(name: &str) -> Option<u16> {
    let name = last_ty_seg(name);
    let rest = name.strip_prefix('f')?;
    rest.parse::<u16>().ok()
}

fn numeric_compatible(a: &str, b: &str) -> bool {
    let a0 = last_ty_seg(a);
    let b0 = last_ty_seg(b);

    // Surface `Integer` is compatible with backend `iN`.
    if (a0 == "Integer" && backend_int_bits(b0).is_some())
        || (b0 == "Integer" && backend_int_bits(a0).is_some())
        || (backend_int_bits(a0).is_some() && backend_int_bits(b0).is_some())
    {
        return true;
    }

    // Surface `Float64` is compatible with backend `fN`.
    if (a0 == "Float64" && backend_float_bits(b0).is_some())
        || (b0 == "Float64" && backend_float_bits(a0).is_some())
        || (backend_float_bits(a0).is_some() && backend_float_bits(b0).is_some())
    {
        return true;
    }

    false
}

fn unify_in(subst: &mut Subst, a: Ty, b: Ty) -> Result<()> {
    let a = apply(subst, a);
    let b = apply(subst, b);

    match (a, b) {
        (Ty::Var(v), t) | (t, Ty::Var(v)) => bind_var(subst, v, t),
        (Ty::Con(a), Ty::Con(b)) if a == b => Ok(()),
        (Ty::Con(a), Ty::Con(b)) if numeric_compatible(&a, &b) => Ok(()),
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
            fields.iter().any(|(_, t)| occurs_in(subst, seen, v, t))
                || occurs_in(subst, seen, v, rest)
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Constraint {
    Show(Ty),
    ShowRow(Ty),
    Eq(Ty),
    EqRow(Ty),
    /// User-defined typeclass constraint: `C t`.
    Class {
        class: String,
        ty: Ty,
    },
    /// Field absence constraint for row types (records/open records/row variables).
    Lacks {
        label: String,
        row: Ty,
    },
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
        Constraint::Eq(t) => {
            write!(f, "Eq ")?;
            fmt_ty_prec(f, t, 0, vars)
        }
        Constraint::EqRow(t) => {
            write!(f, "EqRow ")?;
            fmt_ty_prec(f, t, 0, vars)
        }
        Constraint::Class { class, ty } => {
            write!(f, "{class} ")?;
            fmt_ty_prec(f, ty, 0, vars)
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
        Constraint::Show(t)
        | Constraint::ShowRow(t)
        | Constraint::Eq(t)
        | Constraint::EqRow(t)
        | Constraint::Class { ty: t, .. } => ftv_ty(t),
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
        Constraint::Eq(t) => Constraint::Eq(replace_vars(t, m)),
        Constraint::EqRow(t) => Constraint::EqRow(replace_vars(t, m)),
        Constraint::Class { class, ty } => Constraint::Class {
            class: class.clone(),
            ty: replace_vars(ty, m),
        },
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
        Constraint::Eq(t) => Constraint::Eq(apply(subst, t.clone())),
        Constraint::EqRow(t) => Constraint::EqRow(apply(subst, t.clone())),
        Constraint::Class { class, ty } => Constraint::Class {
            class: class.clone(),
            ty: apply(subst, ty.clone()),
        },
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

fn infer_pat_var(
    cx: &mut InferCtx,
    pat: &ast::Pattern,
    name: &str,
    binds: &mut Vec<(String, Ty)>,
    seen: &mut HashSet<String>,
) -> Result<Ty> {
    if !seen.insert(name.to_string()) {
        return Err(Error::msg_with_span("duplicate pattern variable", pat.span));
    }
    let t = cx.fresh();
    binds.push((name.to_string(), t.clone()));
    Ok(t)
}

fn infer_pat_literal(e: &ast::Expr) -> Result<Ty> {
    use ast::ExprKind;

    Ok(match &e.kind {
        ExprKind::Unit => Ty::Con("Unit".to_string()),
        ExprKind::Integer(_) => Ty::Con("Integer".to_string()),
        ExprKind::Float64(_) => Ty::Con("Float64".to_string()),
        ExprKind::Bool(_) => Ty::Con("Bool".to_string()),
        ExprKind::String(_) => Ty::List(Box::new(Ty::Con("Char".to_string()))),
        ExprKind::Char(_) => Ty::Con("Char".to_string()),
        _ => return Err(Error::msg("unsupported literal pattern")),
    })
}

#[allow(clippy::too_many_arguments)]
fn infer_pat_tuple(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    subst: &mut Subst,
    env: &TypeEnv,
    ps: &[ast::Pattern],
    binds: &mut Vec<(String, Ty)>,
    seen: &mut HashSet<String>,
    cs_out: &mut Vec<Constraint>,
) -> Result<Ty> {
    Ok(Ty::Tuple(
        ps.iter()
            .map(|p| infer_pat_in(cx, data_env, subst, env, p, binds, seen, cs_out))
            .collect::<Result<Vec<_>>>()?,
    ))
}

#[allow(clippy::too_many_arguments)]
fn infer_pat_list(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    subst: &mut Subst,
    env: &TypeEnv,
    ps: &[ast::Pattern],
    binds: &mut Vec<(String, Ty)>,
    seen: &mut HashSet<String>,
    cs_out: &mut Vec<Constraint>,
) -> Result<Ty> {
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

#[allow(clippy::too_many_arguments)]
fn infer_pat_record(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    subst: &mut Subst,
    env: &TypeEnv,
    fields: &[(String, ast::Pattern)],
    binds: &mut Vec<(String, Ty)>,
    seen: &mut HashSet<String>,
    cs_out: &mut Vec<Constraint>,
) -> Result<Ty> {
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

#[allow(clippy::too_many_arguments)]
fn infer_pat_record_loose(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    subst: &mut Subst,
    env: &TypeEnv,
    fields: &[(String, ast::Pattern)],
    rest_name: &Option<String>,
    pat: &ast::Pattern,
    binds: &mut Vec<(String, Ty)>,
    seen: &mut HashSet<String>,
    cs_out: &mut Vec<Constraint>,
) -> Result<Ty> {
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
            return Err(Error::msg_with_span("duplicate pattern variable", pat.span));
        }
        binds.push((name.clone(), rest_ty.clone()));
    }

    Ok(Ty::RecordOpen(out, Box::new(rest_ty)))
}

#[allow(clippy::too_many_arguments)]
fn infer_pat_cons(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    subst: &mut Subst,
    env: &TypeEnv,
    hd: &ast::Pattern,
    tl: &ast::Pattern,
    binds: &mut Vec<(String, Ty)>,
    seen: &mut HashSet<String>,
    cs_out: &mut Vec<Constraint>,
) -> Result<Ty> {
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

#[allow(clippy::too_many_arguments)]
fn infer_pat_or(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    subst: &mut Subst,
    env: &TypeEnv,
    a: &ast::Pattern,
    b: &ast::Pattern,
    binds: &mut Vec<(String, Ty)>,
    seen: &mut HashSet<String>,
    cs_out: &mut Vec<Constraint>,
) -> Result<Ty> {
    let base_len = binds.len();
    let base_seen = seen.clone();
    let base_binds = binds.clone();

    let mut binds_a = base_binds.clone();
    let mut seen_a = base_seen.clone();
    let mut cs_a = Vec::new();
    let t_a = infer_pat_in(
        cx,
        data_env,
        subst,
        env,
        a,
        &mut binds_a,
        &mut seen_a,
        &mut cs_a,
    )?;

    let mut binds_b = base_binds;
    let mut seen_b = base_seen;
    let mut cs_b = Vec::new();
    let t_b = infer_pat_in(
        cx,
        data_env,
        subst,
        env,
        b,
        &mut binds_b,
        &mut seen_b,
        &mut cs_b,
    )?;

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

#[allow(clippy::too_many_arguments)]
fn infer_pat_as(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    subst: &mut Subst,
    env: &TypeEnv,
    name: &str,
    p: &ast::Pattern,
    pat: &ast::Pattern,
    binds: &mut Vec<(String, Ty)>,
    seen: &mut HashSet<String>,
    cs_out: &mut Vec<Constraint>,
) -> Result<Ty> {
    if !seen.insert(name.to_string()) {
        return Err(Error::msg_with_span("duplicate pattern variable", pat.span));
    }
    let t = infer_pat_in(cx, data_env, subst, env, p, binds, seen, cs_out)?;
    binds.push((name.to_string(), apply(subst, t.clone())));
    Ok(t)
}

#[allow(clippy::too_many_arguments)]
fn infer_pat_view(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    subst: &mut Subst,
    env: &TypeEnv,
    p: &ast::Pattern,
    e: &ast::Expr,
    binds: &mut Vec<(String, Ty)>,
    seen: &mut HashSet<String>,
    cs_out: &mut Vec<Constraint>,
) -> Result<Ty> {
    let t_scrut = cx.fresh();
    let t_view = infer_pat_in(cx, data_env, subst, env, p, binds, seen, cs_out)?;

    let env_in = apply_env(subst, env);
    let (s_e, _cs_e, t_e) = infer_expr_in(cx, data_env, &env_in, e.clone())?;
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

#[allow(clippy::too_many_arguments)]
fn infer_pat_constructor(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    subst: &mut Subst,
    env: &TypeEnv,
    name: &str,
    args: &[ast::Pattern],
    binds: &mut Vec<(String, Ty)>,
    seen: &mut HashSet<String>,
    cs_out: &mut Vec<Constraint>,
) -> Result<Ty> {
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
    use ast::PatternKind;

    match &pat.kind {
        PatternKind::Var(name) => infer_pat_var(cx, pat, name, binds, seen),
        PatternKind::Wildcard | PatternKind::Hole(_) => Ok(cx.fresh()),
        PatternKind::Literal(e) => infer_pat_literal(e),
        PatternKind::Tuple(ps) => infer_pat_tuple(cx, data_env, subst, env, ps, binds, seen, cs_out),
        PatternKind::List(ps) => infer_pat_list(cx, data_env, subst, env, ps, binds, seen, cs_out),
        PatternKind::Record(fields) => {
            infer_pat_record(cx, data_env, subst, env, fields, binds, seen, cs_out)
        }
        PatternKind::RecordLoose(fields, rest_name) => infer_pat_record_loose(
            cx, data_env, subst, env, fields, rest_name, pat, binds, seen, cs_out,
        ),
        PatternKind::Cons(hd, tl) => infer_pat_cons(cx, data_env, subst, env, hd, tl, binds, seen, cs_out),
        PatternKind::Or(a, b) => infer_pat_or(cx, data_env, subst, env, a, b, binds, seen, cs_out),
        PatternKind::As(name, p) => infer_pat_as(cx, data_env, subst, env, name, p, pat, binds, seen, cs_out),
        PatternKind::View(p, e) => infer_pat_view(cx, data_env, subst, env, p, e, binds, seen, cs_out),
        PatternKind::Constructor { name, args } => {
            infer_pat_constructor(cx, data_env, subst, env, name, args, binds, seen, cs_out)
        }
    }
}

pub fn infer_expr(expr: ast::Expr) -> Result<Ty> {
    let mut cx = InferCtx::default();
    let data_env = DataEnv::new();
    let class_env = ClassEnv::default();
    let env = TypeEnv::new();
    let (s, cs, t) = infer_expr_in(&mut cx, &data_env, &env, expr)?;
    let _ = simplify_constraints(&data_env, &class_env, apply_constraints(&s, cs))?;
    Ok(apply(&s, t))
}

pub fn infer_in_module(module: &ast::Module, expr: ast::Expr) -> Result<Ty> {
    let mut cx = InferCtx::default();
    let data_env = collect_data_env(module);
    let class_env = ClassEnv::default();
    let env = collect_ctor_env(&mut cx, module)?;
    let (s, cs, t) = infer_expr_in(&mut cx, &data_env, &env, expr)?;
    let _ = simplify_constraints(&data_env, &class_env, apply_constraints(&s, cs))?;
    Ok(apply(&s, t))
}

pub fn infer_module(module: &ast::Module) -> Result<HashMap<String, Scheme>> {
    infer_module_with_class_env(module, &ClassEnv::default())
}

fn collect_deps_in_pattern(
    pat: &ast::Pattern,
    name_to_binding: &HashMap<String, usize>,
    bound: &HashSet<String>,
    out: &mut HashSet<usize>,
) {
    use ast::PatternKind;
    match &pat.kind {
        PatternKind::View(p, e) => {
            collect_deps_in_expr(e, name_to_binding, bound, out);
            collect_deps_in_pattern(p, name_to_binding, bound, out);
        }
        PatternKind::Tuple(ps) | PatternKind::List(ps) => {
            for p in ps {
                collect_deps_in_pattern(p, name_to_binding, bound, out);
            }
        }
        PatternKind::Record(fs) | PatternKind::RecordLoose(fs, _) => {
            for (_, p) in fs {
                collect_deps_in_pattern(p, name_to_binding, bound, out);
            }
        }
        PatternKind::Cons(a, b) | PatternKind::Or(a, b) => {
            collect_deps_in_pattern(a, name_to_binding, bound, out);
            collect_deps_in_pattern(b, name_to_binding, bound, out);
        }
        PatternKind::As(_, p) => collect_deps_in_pattern(p, name_to_binding, bound, out),
        PatternKind::Literal(e) => collect_deps_in_expr(e, name_to_binding, bound, out),
        PatternKind::Constructor { args, .. } => {
            for p in args {
                collect_deps_in_pattern(p, name_to_binding, bound, out);
            }
        }
        PatternKind::Var(_) | PatternKind::Wildcard | PatternKind::Hole(_) => {}
    }
}

fn collect_deps_in_binding_seq(
    bindings: &[ast::Binding],
    body: Option<&ast::Expr>,
    name_to_binding: &HashMap<String, usize>,
    bound: &HashSet<String>,
    out: &mut HashSet<usize>,
) {
    let mut bound_seq = bound.clone();
    for b in bindings {
        collect_deps_in_expr(&b.expr, name_to_binding, &bound_seq, out);
        let mut names = HashSet::new();
        pat_defined_names(&b.pat, &mut names);
        for n in names {
            bound_seq.insert(n);
        }
    }
    if let Some(body) = body {
        collect_deps_in_expr(body, name_to_binding, &bound_seq, out);
    }
}

fn collect_deps_in_lambda(
    params: &[String],
    body: &ast::Expr,
    name_to_binding: &HashMap<String, usize>,
    bound: &HashSet<String>,
    out: &mut HashSet<usize>,
) {
    let mut bound2 = bound.clone();
    for p in params {
        bound2.insert(p.clone());
    }
    collect_deps_in_expr(body, name_to_binding, &bound2, out);
}

fn collect_deps_in_do(
    stmts: &[ast::DoStmt],
    name_to_binding: &HashMap<String, usize>,
    bound: &HashSet<String>,
    out: &mut HashSet<usize>,
) {
    let mut bound_do = bound.clone();
    for s in stmts {
        match s {
            ast::DoStmt::Bind { pat, expr } => {
                collect_deps_in_expr(expr, name_to_binding, &bound_do, out);
                let mut names = HashSet::new();
                pat_defined_names(pat, &mut names);
                for n in names {
                    bound_do.insert(n);
                }
            }
            ast::DoStmt::Expr(e) => {
                collect_deps_in_expr(e, name_to_binding, &bound_do, out);
            }
        }
    }
}

fn collect_deps_in_case(
    expr: &ast::Expr,
    arms: &[ast::CaseArm],
    name_to_binding: &HashMap<String, usize>,
    bound: &HashSet<String>,
    out: &mut HashSet<usize>,
) {
    collect_deps_in_expr(expr, name_to_binding, bound, out);
    for a in arms {
        collect_deps_in_pattern(&a.pat, name_to_binding, bound, out);

        let mut bound_arm = bound.clone();
        let mut names = HashSet::new();
        pat_defined_names(&a.pat, &mut names);
        for n in names {
            bound_arm.insert(n);
        }
        if let Some(g) = &a.guard {
            collect_deps_in_expr(g, name_to_binding, &bound_arm, out);
        }
        collect_deps_in_expr(&a.body, name_to_binding, &bound_arm, out);
    }
}

fn collect_deps_in_expr(
    expr: &ast::Expr,
    name_to_binding: &HashMap<String, usize>,
    bound: &HashSet<String>,
    out: &mut HashSet<usize>,
) {
    use ast::ExprKind;
    match &expr.kind {
        ExprKind::Var(n) => {
            if !bound.contains(n) {
                if let Some(i) = name_to_binding.get(n) {
                    out.insert(*i);
                }
            }
        }
        ExprKind::Ctor(_)
        | ExprKind::Unit
        | ExprKind::Integer(_)
        | ExprKind::Float64(_)
        | ExprKind::Bool(_)
        | ExprKind::String(_)
        | ExprKind::Char(_) => {}
        ExprKind::Lambda { params, body } => {
            collect_deps_in_lambda(params, body, name_to_binding, bound, out);
        }
        ExprKind::Apply { func, args } => {
            collect_deps_in_expr(func, name_to_binding, bound, out);
            for a in args {
                collect_deps_in_expr(a, name_to_binding, bound, out);
            }
        }
        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            collect_deps_in_expr(cond, name_to_binding, bound, out);
            collect_deps_in_expr(then_branch, name_to_binding, bound, out);
            collect_deps_in_expr(else_branch, name_to_binding, bound, out);
        }
        ExprKind::Let { bindings, body } => {
            collect_deps_in_binding_seq(bindings, Some(body), name_to_binding, bound, out);
        }
        ExprKind::Where { expr, bindings } => {
            // The `where` bindings are in scope in `expr`.
            let mut all_names = HashSet::new();
            for b in bindings {
                pat_defined_names(&b.pat, &mut all_names);
            }
            let mut bound_expr = bound.clone();
            for n in all_names {
                bound_expr.insert(n);
            }
            collect_deps_in_expr(expr, name_to_binding, &bound_expr, out);

            // Sequential bindings: earlier where-binders are in scope for later ones.
            collect_deps_in_binding_seq(bindings, None, name_to_binding, bound, out);
        }
        ExprKind::Annot { expr, .. } => collect_deps_in_expr(expr, name_to_binding, bound, out),
        ExprKind::Do(stmts) => {
            collect_deps_in_do(stmts, name_to_binding, bound, out);
        }
        ExprKind::Case { expr, arms } => {
            collect_deps_in_case(expr, arms, name_to_binding, bound, out);
        }
        ExprKind::Cons { head, tail } => {
            collect_deps_in_expr(head, name_to_binding, bound, out);
            collect_deps_in_expr(tail, name_to_binding, bound, out);
        }
        ExprKind::List(es) | ExprKind::Tuple(es) => {
            for e in es {
                collect_deps_in_expr(e, name_to_binding, bound, out);
            }
        }
        ExprKind::Record(fs) => {
            for (_, e) in fs {
                collect_deps_in_expr(e, name_to_binding, bound, out);
            }
        }
    }
}

fn tarjan_scc(graph: &[Vec<usize>]) -> Vec<Vec<usize>> {
    struct TarjanState {
        index: usize,
        indices: Vec<Option<usize>>,
        lowlink: Vec<usize>,
        stack: Vec<usize>,
        on_stack: Vec<bool>,
        out: Vec<Vec<usize>>,
    }

    fn strongconnect(v: usize, graph: &[Vec<usize>], st: &mut TarjanState) {
        st.indices[v] = Some(st.index);
        st.lowlink[v] = st.index;
        st.index += 1;
        st.stack.push(v);
        st.on_stack[v] = true;

        for &w in &graph[v] {
            if st.indices[w].is_none() {
                strongconnect(w, graph, st);
                st.lowlink[v] = st.lowlink[v].min(st.lowlink[w]);
            } else if st.on_stack[w] {
                st.lowlink[v] = st.lowlink[v].min(st.indices[w].unwrap());
            }
        }

        if st.lowlink[v] == st.indices[v].unwrap() {
            let mut comp = Vec::new();
            loop {
                let w = st.stack.pop().unwrap();
                st.on_stack[w] = false;
                comp.push(w);
                if w == v {
                    break;
                }
            }
            st.out.push(comp);
        }
    }

    let n = graph.len();
    let mut st = TarjanState {
        index: 0,
        indices: vec![None; n],
        lowlink: vec![0usize; n],
        stack: Vec::new(),
        on_stack: vec![false; n],
        out: Vec::new(),
    };

    for v in 0..n {
        if st.indices[v].is_none() {
            strongconnect(v, graph, &mut st);
        }
    }

    st.out
}

fn collect_ctor_env(cx: &mut InferCtx, module: &ast::Module) -> Result<TypeEnv> {
    collect_ctor_env_with_class_env(cx, module, &ClassEnv::default())
}

fn collect_ctor_env_with_class_env(
    cx: &mut InferCtx,
    module: &ast::Module,
    class_env: &ClassEnv,
) -> Result<TypeEnv> {
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

    // __ioBind :: forall a b. IO a -> (a -> IO b) -> IO b
    let Ty::Var(a) = cx.fresh() else {
        unreachable!()
    };
    let Ty::Var(b) = cx.fresh() else {
        unreachable!()
    };
    let io_a = Ty::App {
        head: Box::new(Ty::Con("IO".to_string())),
        args: vec![Ty::Var(a)],
    };
    let io_b = Ty::App {
        head: Box::new(Ty::Con("IO".to_string())),
        args: vec![Ty::Var(b)],
    };
    env.insert(
        "__ioBind".to_string(),
        Scheme {
            vars: vec![a, b],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(io_a.clone()),
                Box::new(Ty::Func(
                    Box::new(Ty::Func(Box::new(Ty::Var(a)), Box::new(io_b.clone()))),
                    Box::new(io_b),
                )),
            ),
        },
    );

    // __ioThen :: forall a b. IO a -> IO b -> IO b
    let Ty::Var(a) = cx.fresh() else {
        unreachable!()
    };
    let Ty::Var(b) = cx.fresh() else {
        unreachable!()
    };
    let io_a = Ty::App {
        head: Box::new(Ty::Con("IO".to_string())),
        args: vec![Ty::Var(a)],
    };
    let io_b = Ty::App {
        head: Box::new(Ty::Con("IO".to_string())),
        args: vec![Ty::Var(b)],
    };
    env.insert(
        "__ioThen".to_string(),
        Scheme {
            vars: vec![a, b],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(io_a),
                Box::new(Ty::Func(Box::new(io_b.clone()), Box::new(io_b))),
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

    // Integer division primitives (used by stdlib Integral Integer instance).
    // __quotInt :: Integer -> Integer -> Integer   (truncate toward 0)
    // __remInt  :: Integer -> Integer -> Integer   (remainder, sign follows dividend)
    // __divInt  :: Integer -> Integer -> Integer   (floor division)
    // __modInt  :: Integer -> Integer -> Integer   (modulus, sign follows divisor)
    for name in ["__quotInt", "__remInt", "__divInt", "__modInt"] {
        env.insert(
            name.to_string(),
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
    }

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

    // == :: Eq a => a -> a -> Bool
    let Ty::Var(v) = cx.fresh() else {
        unreachable!()
    };
    env.insert(
        "==".to_string(),
        Scheme {
            vars: vec![v],
            constraints: vec![Constraint::Eq(Ty::Var(v))],
            ty: Ty::Func(
                Box::new(Ty::Var(v)),
                Box::new(Ty::Func(
                    Box::new(Ty::Var(v)),
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

    // /= :: Eq a => a -> a -> Bool
    let Ty::Var(v) = cx.fresh() else {
        unreachable!()
    };
    env.insert(
        "/=".to_string(),
        Scheme {
            vars: vec![v],
            constraints: vec![Constraint::Eq(Ty::Var(v))],
            ty: Ty::Func(
                Box::new(Ty::Var(v)),
                Box::new(Ty::Func(
                    Box::new(Ty::Var(v)),
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

    let char_list = Ty::List(Box::new(Ty::Con("Char".to_string())));

    // intToString :: Integer -> [Char]
    env.insert(
        "intToString".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Con("Integer".to_string())),
                Box::new(char_list.clone()),
            ),
        },
    );

    // boolToString :: Bool -> [Char]
    env.insert(
        "boolToString".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Con("Bool".to_string())),
                Box::new(char_list.clone()),
            ),
        },
    );

    // ++ :: forall a. [a] -> [a] -> [a]
    let Ty::Var(v) = cx.fresh() else {
        unreachable!()
    };
    let list_a = Ty::List(Box::new(Ty::Var(v)));
    env.insert(
        "++".to_string(),
        Scheme {
            vars: vec![v],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(list_a.clone()),
                Box::new(Ty::Func(Box::new(list_a.clone()), Box::new(list_a))),
            ),
        },
    );

    // show :: Show a => a -> [Char]
    let Ty::Var(v) = cx.fresh() else {
        unreachable!()
    };
    env.insert(
        "show".to_string(),
        Scheme {
            vars: vec![v],
            constraints: vec![Constraint::Show(Ty::Var(v))],
            ty: Ty::Func(Box::new(Ty::Var(v)), Box::new(char_list.clone())),
        },
    );

    // toString :: Show a => a -> [Char]
    let Ty::Var(v) = cx.fresh() else {
        unreachable!()
    };
    env.insert(
        "toString".to_string(),
        Scheme {
            vars: vec![v],
            constraints: vec![Constraint::Show(Ty::Var(v))],
            ty: Ty::Func(Box::new(Ty::Var(v)), Box::new(char_list.clone())),
        },
    );

    // stdoutWrite :: [Char] -> IO Unit
    // Low-level IO primitive used as a building block for higher-level IO.
    env.insert(
        "stdoutWrite".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(char_list.clone()),
                Box::new(Ty::App {
                    head: Box::new(Ty::Con("IO".to_string())),
                    args: vec![Ty::Con("Unit".to_string())],
                }),
            ),
        },
    );

    // stdinReadLine :: IO [Char]
    // Low-level IO primitive used as a building block for higher-level IO.
    env.insert(
        "stdinReadLine".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::App {
                head: Box::new(Ty::Con("IO".to_string())),
                args: vec![char_list.clone()],
            },
        },
    );

    // readLine :: IO [Char]
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
                args: vec![char_list.clone()],
            },
        },
    );

    // print :: [Char] -> IO Unit
    // NOTE: currently a builtin for observability.
    // In the future, `print` should become a library function built on top of IO primitives
    // such as `stdoutWrite`.
    env.insert(
        "print".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(char_list.clone()),
                Box::new(Ty::App {
                    head: Box::new(Ty::Con("IO".to_string())),
                    args: vec![Ty::Con("Unit".to_string())],
                }),
            ),
        },
    );

    // error :: forall a. [Char] -> a
    // Pure bottom value used for explicit partiality / testing laziness.
    let Ty::Var(a) = cx.fresh() else {
        unreachable!()
    };
    env.insert(
        "error".to_string(),
        Scheme {
            vars: vec![a],
            constraints: vec![],
            ty: Ty::Func(Box::new(char_list.clone()), Box::new(Ty::Var(a))),
        },
    );

    // throw :: forall a. [Char] -> IO a
    let Ty::Var(a) = cx.fresh() else {
        unreachable!()
    };
    env.insert(
        "throw".to_string(),
        Scheme {
            vars: vec![a],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(char_list.clone()),
                Box::new(Ty::App {
                    head: Box::new(Ty::Con("IO".to_string())),
                    args: vec![Ty::Var(a)],
                }),
            ),
        },
    );

    // catch :: forall a. IO a -> ([Char] -> IO a) -> IO a
    let Ty::Var(a) = cx.fresh() else {
        unreachable!()
    };
    let io_a = Ty::App {
        head: Box::new(Ty::Con("IO".to_string())),
        args: vec![Ty::Var(a)],
    };
    let handler = Ty::Func(Box::new(char_list.clone()), Box::new(io_a.clone()));
    env.insert(
        "catch".to_string(),
        Scheme {
            vars: vec![a],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(io_a.clone()),
                Box::new(Ty::Func(Box::new(handler), Box::new(io_a))),
            ),
        },
    );

    // try :: forall a. IO a -> IO (Prelude.Either [Char] a)
    let Ty::Var(a) = cx.fresh() else {
        unreachable!()
    };
    let io_a = Ty::App {
        head: Box::new(Ty::Con("IO".to_string())),
        args: vec![Ty::Var(a)],
    };
    let either = Ty::App {
        head: Box::new(Ty::Con("Prelude.Either".to_string())),
        args: vec![char_list.clone(), Ty::Var(a)],
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

    // P6: unsafe-free "FFI" boundary scaffolding.
    // ffiAddI32 :: i32 -> i32 -> i32
    env.insert(
        "ffiAddI32".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Con("i32".to_string())),
                Box::new(Ty::Func(
                    Box::new(Ty::Con("i32".to_string())),
                    Box::new(Ty::Con("i32".to_string())),
                )),
            ),
        },
    );

    // ffiAddF32 :: f32 -> f32 -> f32
    env.insert(
        "ffiAddF32".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Con("f32".to_string())),
                Box::new(Ty::Func(
                    Box::new(Ty::Con("f32".to_string())),
                    Box::new(Ty::Con("f32".to_string())),
                )),
            ),
        },
    );

    // P9: real C ABI FFI (unsafe isolated; feature-gated).
    // ffiPuts :: String -> IO i32
    #[cfg(feature = "unsafe_ffi")]
    env.insert(
        "ffiPuts".to_string(),
        Scheme {
            vars: vec![],
            constraints: vec![],
            ty: Ty::Func(
                Box::new(Ty::Con("String".to_string())),
                Box::new(Ty::App {
                    head: Box::new(Ty::Con("IO".to_string())),
                    args: vec![Ty::Con("i32".to_string())],
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
            env.insert(
                ctor.name.clone(),
                Scheme {
                    vars,
                    constraints: vec![],
                    ty,
                },
            );
        }
    }

    // Add class methods as overloaded functions.
    for ((class, method), qt) in &class_env.methods {
        // If the module defines a value with the same name, let it win.
        if env.contains_key(method) {
            continue;
        }

        let param = class_env
            .class_params
            .get(class)
            .ok_or_else(|| Error::msg("internal: missing class param"))?
            .clone();

        let mut holes: HashMap<String, Ty> = HashMap::new();
        let class_param_ty = holes.entry(param).or_insert_with(|| cx.fresh()).clone();

        let mut cs: Vec<Constraint> = Vec::new();
        cs.push(Constraint::Class {
            class: class.clone(),
            ty: class_param_ty,
        });

        for p in &qt.preds {
            match p {
                ast::Predicate::Show(t) => {
                    let t = lower_surface_type(cx, t, &mut holes);
                    cs.push(Constraint::Show(t));
                }
                ast::Predicate::ShowRow(t) => {
                    let t = lower_surface_type(cx, t, &mut holes);
                    cs.push(Constraint::ShowRow(t));
                }
                ast::Predicate::Eq(t) => {
                    let t = lower_surface_type(cx, t, &mut holes);
                    cs.push(Constraint::Eq(t));
                }
                ast::Predicate::EqRow(t) => {
                    let t = lower_surface_type(cx, t, &mut holes);
                    cs.push(Constraint::EqRow(t));
                }
                ast::Predicate::Class { class, ty } => {
                    let t = lower_surface_type(cx, ty, &mut holes);
                    cs.push(Constraint::Class {
                        class: class.clone(),
                        ty: t,
                    });
                }
                ast::Predicate::Lacks { label, row } => {
                    let row = lower_surface_type(cx, row, &mut holes);
                    cs.push(Constraint::Lacks {
                        label: label.clone(),
                        row,
                    });
                }
            }
        }

        let ty = lower_surface_type(cx, &qt.ty, &mut holes);
        let mut vars: Vec<u32> = ftv_ty(&ty).into_iter().collect();
        for c in &cs {
            vars.extend(ftv_constraint(c));
        }
        vars.sort_unstable();
        vars.dedup();

        env.insert(
            method.clone(),
            Scheme {
                vars,
                constraints: cs,
                ty,
            },
        );
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
    cs.into_iter()
        .map(|c| apply_constraint(subst, &c))
        .collect()
}

type DataEnv = HashMap<String, ast::DataDecl>;

#[derive(Debug, Default, Clone)]
struct ClassEnv {
    /// class name -> parameter name (e.g. `class C a where` => (C, a))
    class_params: HashMap<String, String>,
    /// class name -> superclass predicates (Haskell-style)
    class_supers: HashMap<String, Vec<ast::Predicate>>,
    /// method name -> list of classes that define it
    method_classes: HashMap<String, Vec<String>>,
    /// (class, method) -> declared method type
    methods: HashMap<(String, String), ast::QualType>,
    /// (class, instance-head-type-key) -> dictionary binding name
    instances: HashMap<(String, String), String>,
    /// Non-ground instances (dictionary passing). These are selected by unification on the
    /// instance head pattern, and require dictionaries for their context predicates.
    poly_instances: Vec<PolyInstance>,
}

#[derive(Debug, Clone)]
struct PolyInstance {
    class: String,
    /// Instance head pattern as an internal type (may contain Ty::Var).
    head_pat: Ty,
    /// How many context dictionary arguments the instance dictionary expects.
    ctx_len: usize,
    /// Dictionary binding name (a value or a function if ctx_len > 0).
    dict_name: String,
}

fn instance_head_key_ast(ty: &ast::Type) -> Result<String> {
    use ast::Type;

    fn is_lowercase_ident(s: &str) -> bool {
        s.chars().next().is_some_and(|c| c.is_ascii_lowercase())
    }

    Ok(match ty {
        Type::Unit => "Unit".to_string(),
        Type::Integer => "Integer".to_string(),
        Type::Bool => "Bool".to_string(),
        Type::Float64 => "Float64".to_string(),
        Type::Char => "Char".to_string(),
        Type::String => "String".to_string(),
        Type::Var(name) => {
            if is_lowercase_ident(name) {
                return Err(Error::msg("poly instance head"));
            }
            name.clone()
        }
        Type::App { head, args } => {
            let mut out = instance_head_key_ast(head)?;
            for a in args {
                out.push('_');
                out.push_str(&instance_head_key_ast(a)?);
            }
            out
        }
        _ => {
            return Err(Error::msg(
                "MVP: instance head supports only constructor/app types",
            ))
        }
    })
}

fn instance_head_key_ty(ty: &Ty) -> Result<String> {
    Ok(match ty {
        Ty::Con(name) => name.clone(),
        Ty::App { head, args } => {
            let mut out = instance_head_key_ty(head)?;
            for a in args {
                out.push('_');
                out.push_str(&instance_head_key_ty(a)?);
            }
            out
        }
        _ => {
            return Err(Error::msg(
                "MVP: class constraints support only constructor/app instance heads",
            ))
        }
    })
}

fn mangle_ident(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    out
}

fn desugar_typeclasses(module: &mut ast::Module) -> Result<ClassEnv> {
    let mut env = ClassEnv::default();

    // class name -> method names (declaration order)
    let mut class_method_names: HashMap<String, Vec<String>> = HashMap::new();
    // (class, method) -> default implementation expression
    let mut class_default_methods: HashMap<(String, String), ast::Expr> = HashMap::new();

    // Collect class method signatures + defaults.
    for it in &module.items {
        let ast::Item::ClassDecl(c) = it else {
            continue;
        };

        if env.class_params.contains_key(&c.name) {
            return Err(Error::msg("duplicate class"));
        }
        env.class_params.insert(c.name.clone(), c.param.clone());
        env.class_supers.insert(c.name.clone(), c.supers.clone());

        for m in &c.methods {
            class_method_names
                .entry(c.name.clone())
                .or_default()
                .push(m.name.clone());
            env.method_classes
                .entry(m.name.clone())
                .or_default()
                .push(c.name.clone());
            env.methods
                .insert((c.name.clone(), m.name.clone()), m.ty.clone());
        }

        for b in &c.default_methods {
            let ast::PatternKind::Var(mname) = &b.pat.kind else {
                return Err(Error::msg(
                    "MVP: class default methods must be simple variable bindings",
                ));
            };
            let key = (c.name.clone(), mname.clone());
            if class_default_methods.contains_key(&key) {
                return Err(Error::msg("duplicate class default method"));
            }
            class_default_methods.insert(key, b.expr.clone());
        }
    }
    for classes in env.method_classes.values_mut() {
        classes.sort();
        classes.dedup();
    }

    // MVP: avoid ambiguous unqualified method names.
    for (m, classes) in &env.method_classes {
        if classes.len() > 1 {
            return Err(Error::msg(format!(
                "ambiguous method name: {m} (defined in classes: {})",
                classes.join(", ")
            )));
        }
    }

    // Validate superclass predicates (minimal, Haskell-aligned):
    // - super predicates must be of the form `P a` where `a` is the class parameter
    // - user-defined superclasses must refer to known classes
    for (class, supers) in &env.class_supers {
        let Some(param) = env.class_params.get(class) else {
            return Err(Error::msg("internal: missing class param"));
        };

        for p in supers {
            match p {
                ast::Predicate::Show(ast::Type::Var(v))
                | ast::Predicate::ShowRow(ast::Type::Var(v))
                | ast::Predicate::Eq(ast::Type::Var(v))
                | ast::Predicate::EqRow(ast::Type::Var(v))
                    if v == param => {}

                ast::Predicate::Class {
                    class: sup,
                    ty: ast::Type::Var(v),
                } if v == param => {
                    if !env.class_params.contains_key(sup) {
                        return Err(Error::msg(format!(
                            "unknown superclass `{sup}` in class `{class}`"
                        )));
                    }
                }

                _ => {
                    return Err(Error::msg(format!(
                        "MVP: superclass constraints must be of the form `C {param}`"
                    )))
                }
            }
        }
    }

    // Detect cycles in the user-defined superclass graph.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mark {
        Temp,
        Perm,
    }

    fn dfs_cycle(
        env: &ClassEnv,
        node: &str,
        marks: &mut HashMap<String, Mark>,
        stack: &mut Vec<String>,
    ) -> Result<()> {
        if matches!(marks.get(node), Some(Mark::Perm)) {
            return Ok(());
        }
        if matches!(marks.get(node), Some(Mark::Temp)) {
            // Found a cycle; report a readable path.
            stack.push(node.to_string());
            return Err(Error::msg(format!(
                "cyclic superclass constraints: {}",
                stack.join(" => ")
            )));
        }

        marks.insert(node.to_string(), Mark::Temp);
        stack.push(node.to_string());

        if let Some(supers) = env.class_supers.get(node) {
            for p in supers {
                if let ast::Predicate::Class { class: sup, .. } = p {
                    dfs_cycle(env, sup, marks, stack)?;
                }
            }
        }

        stack.pop();
        marks.insert(node.to_string(), Mark::Perm);
        Ok(())
    }

    let mut marks: HashMap<String, Mark> = HashMap::new();
    for c in env.class_params.keys() {
        let mut stack: Vec<String> = Vec::new();
        dfs_cycle(&env, c, &mut marks, &mut stack)?;
    }

    // Desugar instances into bindings for dictionaries + method implementations.
    // We do this in two phases so superclass dict references are independent of instance order.
    let instance_decls: Vec<ast::InstanceDecl> = module
        .items
        .iter()
        .filter_map(|it| match it {
            ast::Item::InstanceDecl(inst) => Some(inst.clone()),
            _ => None,
        })
        .collect();

    // Phase 1: pre-register all instance dictionary names.
    // We also collect polymorphic instance metadata for later selection.
    let mut poly_to_register: Vec<PolyInstance> = Vec::new();
    for inst in &instance_decls {
        match instance_head_key_ast(&inst.ty) {
            Ok(ty_key) => {
                let ty_mangled = mangle_ident(&ty_key);
                let dict_name = format!("__dict_{}_{}", inst.class, ty_mangled);

                let key = (inst.class.clone(), ty_key);
                if env.instances.contains_key(&key) {
                    return Err(Error::msg("duplicate instance"));
                }
                env.instances.insert(key, dict_name);
            }
            Err(_) => {
                // Polymorphic (non-ground) instance: pre-register a stable dictionary name.
                let dict_name = format!("__dict_{}_poly{}", inst.class, poly_to_register.len());

                // Lower the instance head type into an internal type pattern.
                let mut cx = InferCtx::default();
                let head_pat = lower_surface_type(&mut cx, &inst.ty, &mut HashMap::new());

                poly_to_register.push(PolyInstance {
                    class: inst.class.clone(),
                    head_pat,
                    ctx_len: inst.preds.len(),
                    dict_name,
                });
            }
        }
    }
    env.poly_instances.extend(poly_to_register);

    fn super_field_name(class: &str) -> String {
        format!("__super_{}", mangle_ident(class))
    }

    fn dict_param_name(class: &str) -> String {
        format!("__dict_{class}")
    }

    fn ctx_param_name(i: usize) -> String {
        format!("__ctx_dict_{i}")
    }

    fn class_name_of_pred(p: &ast::Predicate) -> Option<&str> {
        match p {
            ast::Predicate::Class { class, .. } => Some(class.as_str()),
            _ => None,
        }
    }

    fn add_params_to_expr(span: ast::Span, expr: ast::Expr, params: &[String]) -> ast::Expr {
        use ast::ExprKind;

        if params.is_empty() {
            return expr;
        }

        match expr.kind {
            ExprKind::Lambda {
                params: mut ps,
                body,
            } => {
                let mut all: Vec<String> = params.to_vec();
                all.append(&mut ps);
                ast::Expr::new(span, ExprKind::Lambda { params: all, body })
            }
            other => ast::Expr::new(
                span,
                ExprKind::Lambda {
                    params: params.to_vec(),
                    body: Box::new(ast::Expr::new(span, other)),
                },
            ),
        }
    }

    // Phase 2: generate impl bindings + dictionary records.
    let mut extra_items: Vec<ast::Item> = Vec::new();
    for inst in &instance_decls {
        let (ty_key_opt, ty_mangled, dict_name) = match instance_head_key_ast(&inst.ty) {
            Ok(ty_key) => {
                let ty_mangled = mangle_ident(&ty_key);
                let dict_key = (inst.class.clone(), ty_key.clone());
                let dict_name = env
                    .instances
                    .get(&dict_key)
                    .cloned()
                    .ok_or_else(|| Error::msg("internal: missing instance dict name"))?;
                (Some(ty_key), ty_mangled, dict_name)
            }
            Err(_) => {
                let mut cx = InferCtx::default();
                let head_pat = lower_surface_type(&mut cx, &inst.ty, &mut HashMap::new());
                let Some(pi) = env.poly_instances.iter().find(|pi| {
                    pi.class == inst.class
                        && pi.head_pat == head_pat
                        && pi.ctx_len == inst.preds.len()
                }) else {
                    return Err(Error::msg("internal: missing poly instance"));
                };
                (None, "poly".to_string(), pi.dict_name.clone())
            }
        };

        let Some(method_names) = class_method_names.get(&inst.class) else {
            return Err(Error::msg("unknown class in instance"));
        };

        let mut inst_methods: HashMap<String, ast::Expr> = HashMap::new();
        for b in &inst.methods {
            let ast::PatternKind::Var(mname) = &b.pat.kind else {
                return Err(Error::msg(
                    "MVP: instance methods must be simple variable bindings",
                ));
            };
            if inst_methods.contains_key(mname) {
                return Err(Error::msg("duplicate instance method"));
            }
            inst_methods.insert(mname.clone(), b.expr.clone());
        }

        // Direct user-defined superclasses (for dict fields + params).
        let mut direct_supers: Vec<String> = Vec::new();
        if let Some(supers) = env.class_supers.get(&inst.class) {
            for p in supers {
                if let ast::Predicate::Class { class: sup, .. } = p {
                    direct_supers.push(sup.clone());
                }
            }
        }
        direct_supers.sort();
        direct_supers.dedup();

        // Self dictionary param is always available inside method bodies.
        // This is important for class default methods that call other methods (e.g. Monad.(>>)
        // calling (>>=)).
        //
        // NOTE: we intentionally do NOT reference the dictionary binding name (e.g. `__dict_C_T`)
        // from within its own record literal, to avoid introducing accidental recursion.
        let self_param_name: String = dict_param_name(&inst.class);

        // Extra params available inside instance method bodies:
        // - the instance's own dictionary (self)
        // - dictionaries for instance context predicates (`instance P => ...`)
        let mut extra_param_names: Vec<String> = vec![self_param_name.clone()];

        // Collect context dictionaries and map class name -> param var for easy lookup.
        let mut ctx_dict_by_class: HashMap<String, String> = HashMap::new();
        for (i, p) in inst.preds.iter().enumerate() {
            let Some(cls) = class_name_of_pred(p) else {
                return Err(Error::msg(
                    "MVP: instance context supports only class predicates (C t)",
                ));
            };
            // Reject duplicates to avoid ambiguity.
            if ctx_dict_by_class.contains_key(cls) {
                return Err(Error::msg(
                    "MVP: duplicate class in instance context is not supported",
                ));
            }
            let pname = ctx_param_name(i);
            extra_param_names.push(pname.clone());
            ctx_dict_by_class.insert(cls.to_string(), pname);
        }

        let mut super_dict_names: Vec<String> = Vec::new();
        for sup in &direct_supers {
            if let Some(pname) = ctx_dict_by_class.get(sup) {
                super_dict_names.push(pname.clone());
                continue;
            }

            let Some(ty_key) = ty_key_opt.clone() else {
                return Err(Error::msg(
                    "MVP: superclass resolution for non-ground instance heads is not supported yet",
                ));
            };
            let sup_key = (sup.clone(), ty_key.clone());
            let Some(sup_dict_name) = env.instances.get(&sup_key) else {
                return Err(Error::msg(format!(
                    "missing superclass instance required by `{}`: {} {}",
                    inst.class, sup, ty_key
                )));
            };
            super_dict_names.push(sup_dict_name.clone());
        }

        // Method impl bindings (instance overrides or class defaults).
        let mut dict_fields: Vec<(String, ast::Expr)> = Vec::new();
        for mname in method_names {
            let expr = if let Some(e) = inst_methods.get(mname) {
                e.clone()
            } else if let Some(e) = class_default_methods.get(&(inst.class.clone(), mname.clone()))
            {
                e.clone()
            } else {
                return Err(Error::msg(format!(
                    "missing method implementation for `{}` in instance {} {}",
                    mname,
                    inst.class,
                    ty_key_opt.clone().unwrap_or_else(|| "<poly>".to_string())
                )));
            };

            let impl_name = format!(
                "__inst_{}_{}_{}",
                inst.class,
                ty_mangled,
                mangle_ident(mname)
            );

            let expr = add_params_to_expr(ast::dummy_span(), expr, &extra_param_names);
            extra_items.push(ast::Item::Binding(ast::Binding {
                pat: ast::Pattern::new(ast::dummy_span(), ast::PatternKind::Var(impl_name.clone())),
                expr,
            }));

            // Store the method implementation itself.
            // Call sites will pass the selected dictionary as the first argument.
            dict_fields.push((
                mname.clone(),
                ast::Expr::new(ast::dummy_span(), ast::ExprKind::Var(impl_name)),
            ));
        }

        // Superclass dictionary fields (Haskell-style dictionary embedding).
        for (sup, sup_dict_name) in direct_supers.into_iter().zip(super_dict_names.into_iter()) {
            dict_fields.push((
                super_field_name(&sup),
                ast::Expr::new(ast::dummy_span(), ast::ExprKind::Var(sup_dict_name)),
            ));
        }

        // Dictionary binding.
        // If the instance has a context (`instance P => ...`), we represent the dictionary as a
        // function that takes those context dictionaries as parameters.
        let ctx_params: Vec<String> = (0..inst.preds.len()).map(ctx_param_name).collect();
        let dict_expr = ast::Expr::new(ast::dummy_span(), ast::ExprKind::Record(dict_fields));
        let dict_expr = add_params_to_expr(ast::dummy_span(), dict_expr, &ctx_params);
        extra_items.push(ast::Item::Binding(ast::Binding {
            pat: ast::Pattern::new(ast::dummy_span(), ast::PatternKind::Var(dict_name)),
            expr: dict_expr,
        }));
    }

    module.items = module
        .items
        .drain(..)
        .filter(|it| !matches!(it, ast::Item::ClassDecl(_) | ast::Item::InstanceDecl(_)))
        .chain(extra_items)
        .collect();

    Ok(env)
}

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
        Type::Var(name) => params
            .get(name)
            .cloned()
            .unwrap_or_else(|| Ty::Con(name.clone())),
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
        Type::Hole(None) => {
            return Err(Error::msg(
                "type holes in data declarations are not supported",
            ))
        }
    })
}

fn show_primitives(name: &str) -> bool {
    matches!(name, "Integer" | "Bool" | "Char" | "Unit")
}

fn eq_primitives(name: &str) -> bool {
    matches!(name, "Integer" | "Bool" | "Char" | "Unit" | "Float64")
}

fn unqual_name_last_segment(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

fn case_pat_is_catch_all(p: &ast::Pattern) -> bool {
    use ast::PatternKind;
    match &p.kind {
        PatternKind::Var(_) | PatternKind::Wildcard | PatternKind::Hole(_) => true,
        PatternKind::As(_, inner) => case_pat_is_catch_all(inner),
        PatternKind::Or(a, b) => case_pat_is_catch_all(a) || case_pat_is_catch_all(b),
        _ => false,
    }
}

fn case_pat_is_list_cons_all(p: &ast::Pattern) -> bool {
    use ast::PatternKind;
    match &p.kind {
        PatternKind::As(_, inner) => case_pat_is_list_cons_all(inner),
        PatternKind::Or(a, b) => case_pat_is_list_cons_all(a) || case_pat_is_list_cons_all(b),
        PatternKind::Cons(_, tail) => case_pat_is_catch_all(tail),
        _ => false,
    }
}

fn case_collect_top_alts(p: &ast::Pattern, out: &mut Vec<String>) {
    use ast::{ExprKind, PatternKind};
    match &p.kind {
        PatternKind::As(_, inner) => case_collect_top_alts(inner, out),
        PatternKind::Or(a, b) => {
            case_collect_top_alts(a, out);
            case_collect_top_alts(b, out);
        }
        PatternKind::Constructor { name, .. } => {
            out.push(format!("ctor:{}", unqual_name_last_segment(name)))
        }
        PatternKind::Cons(_, _) if case_pat_is_list_cons_all(p) => {
            out.push("list:cons_all".to_string())
        }
        PatternKind::List(ps) if ps.is_empty() => out.push("list:nil".to_string()),
        PatternKind::Literal(e) => match &e.kind {
            ExprKind::Bool(b) => out.push(format!("bool:{b}")),
            ExprKind::Unit => out.push("unit".to_string()),
            _ => {}
        },
        _ => {}
    }
}

fn case_has_unguarded_catch_all(arms: &[(ast::Pattern, bool)]) -> bool {
    arms.iter()
        .any(|(pat, has_guard)| !*has_guard && case_pat_is_catch_all(pat))
}

fn case_collect_unguarded_top_alts(arms: &[(ast::Pattern, bool)], out: &mut Vec<String>) {
    for (pat, has_guard) in arms {
        if *has_guard {
            continue;
        }
        case_collect_top_alts(pat, out);
    }
}

fn normalize_string_alts(alts: &mut Vec<String>) {
    alts.sort();
    alts.dedup();
}

fn check_case_primitive_exhaustive(scrut_ty: &Ty, alts: &[String]) -> Result<Option<Result<()>>> {
    match scrut_ty {
        Ty::Con(name) if name == "Bool" => {
            let has_true = alts.iter().any(|a| a == "bool:true");
            let has_false = alts.iter().any(|a| a == "bool:false");
            return Ok(Some(if has_true && has_false {
                Ok(())
            } else {
                Err(Error::msg(
                    "non-exhaustive case: missing Bool branch (add `_ -> ...`)",
                ))
            }));
        }
        Ty::Con(name) if name == "Unit" => {
            return Ok(Some(if alts.iter().any(|a| a == "unit") {
                Ok(())
            } else {
                Err(Error::msg("non-exhaustive case on Unit (add `_ -> ...`)"))
            }));
        }
        Ty::List(_) => {
            let has_nil = alts.iter().any(|a| a == "list:nil");
            let has_cons = alts.iter().any(|a| a == "list:cons_all");
            return Ok(Some(if has_nil && has_cons {
                Ok(())
            } else {
                Err(Error::msg(
                    "non-exhaustive case on List: missing `[]` or `(_:_)` (add `_ -> ...`)",
                ))
            }));
        }
        Ty::Con(name) if matches!(name.as_str(), "Integer" | "Float64" | "Char") => {
            return Ok(Some(Err(Error::msg(format!(
                "non-exhaustive case on {name} (add `_ -> ...`)"
            )))));
        }
        Ty::Var(_) => return Ok(Some(Ok(()))),
        _ => {}
    }
    Ok(None)
}

fn adt_head_type_name(scrut_ty: &Ty) -> Option<String> {
    match scrut_ty {
        Ty::Con(n) => Some(n.clone()),
        Ty::App { head, .. } => match head.as_ref() {
            Ty::Con(n) => Some(n.clone()),
            _ => None,
        },
        _ => None,
    }
}

fn check_case_adt_exhaustive(
    data_env: &DataEnv,
    scrut_ty: &Ty,
    alts: &[String],
) -> Result<Option<Result<()>>> {
    let (Ty::App { .. } | Ty::Con(_)) = scrut_ty else {
        return Ok(None);
    };

    let Some(ty_name) = adt_head_type_name(scrut_ty) else {
        return Ok(Some(Ok(())));
    };

    let Some(d) = data_env.get(&ty_name) else {
        return Ok(Some(Ok(())));
    };

    let mut missing: Vec<String> = Vec::new();
    for c in &d.ctors {
        let key = format!("ctor:{}", unqual_name_last_segment(&c.name));
        if !alts.iter().any(|a| a == &key) {
            missing.push(c.name.clone());
        }
    }

    Ok(Some(if missing.is_empty() {
        Ok(())
    } else {
        Err(Error::msg(format!(
            "non-exhaustive case on {ty_name}: missing constructors: {}",
            missing.join(", ")
        )))
    }))
}

fn lower_super_predicate_for_constraints(p: &ast::Predicate, ty: &Ty) -> Constraint {
    match p {
        ast::Predicate::Show(_) => Constraint::Show(ty.clone()),
        ast::Predicate::ShowRow(_) => Constraint::ShowRow(ty.clone()),
        ast::Predicate::Eq(_) => Constraint::Eq(ty.clone()),
        ast::Predicate::EqRow(_) => Constraint::EqRow(ty.clone()),
        ast::Predicate::Class { class, .. } => Constraint::Class {
            class: class.clone(),
            ty: ty.clone(),
        },
        ast::Predicate::Lacks { .. } => unreachable!(
            "internal error: Lacks predicate is not allowed in superclass constraints"
        ),
    }
}

fn simplify_process_constraint(
    data_env: &DataEnv,
    class_env: &ClassEnv,
    expanded: &mut HashMap<String, ()>,
    in_progress: &mut Vec<Ty>,
    work: &mut std::collections::VecDeque<Constraint>,
    out: &mut Vec<Constraint>,
    c: Constraint,
) -> Result<()> {
    match c {
        Constraint::Show(t) => out.extend(entails_show(data_env, &t, in_progress)?),
        Constraint::ShowRow(t) => out.extend(entails_show_row(data_env, &t, in_progress)?),
        Constraint::Eq(t) => out.extend(entails_eq(data_env, &t, in_progress)?),
        Constraint::EqRow(t) => out.extend(entails_eq_row(data_env, &t, in_progress)?),
        Constraint::Lacks { label, row } => out.extend(entails_lacks(&label, &row)?),
        Constraint::Class { class, ty } => {
            let expand_key = format!("{class}:{ty:?}");
            if expanded.insert(expand_key, ()).is_none() {
                if let Some(supers) = class_env.class_supers.get(&class) {
                    for p in supers {
                        work.push_back(lower_super_predicate_for_constraints(p, &ty));
                    }
                }
            }

            if !ftv_ty(&ty).is_empty() {
                out.push(Constraint::Class { class, ty });
            } else {
                let key = (class.clone(), instance_head_key_ty(&ty)?);
                if !class_env.instances.contains_key(&key) {
                    return Err(Error::msg(format!("cannot satisfy constraint: {class} {ty}")));
                }
            }
        }
    }
    Ok(())
}

fn is_superclass_of_class_env(class_env: &ClassEnv, sub: &str, sup: &str) -> bool {
    use std::collections::{HashSet, VecDeque};

    if sub == sup {
        return false;
    }

    let mut seen: HashSet<String> = HashSet::new();
    let mut q: VecDeque<String> = VecDeque::new();
    q.push_back(sub.to_string());

    while let Some(c) = q.pop_front() {
        if !seen.insert(c.clone()) {
            continue;
        }
        let Some(supers) = class_env.class_supers.get(&c) else {
            continue;
        };
        for p in supers {
            let ast::Predicate::Class { class: next, .. } = p else {
                continue;
            };
            if next == sup {
                return true;
            }
            q.push_back(next.clone());
        }
    }

    false
}

fn simplify_context_reduce_user_classes(class_env: &ClassEnv, cs: Vec<Constraint>) -> Vec<Constraint> {
    let mut keep: Vec<bool> = vec![true; cs.len()];
    for (i, ci_constraint) in cs.iter().enumerate() {
        let Constraint::Class { class: ci, ty: ti } = ci_constraint else {
            continue;
        };

        for (j, cj_constraint) in cs.iter().enumerate() {
            if i == j {
                continue;
            }
            let Constraint::Class { class: cj, ty: tj } = cj_constraint else {
                continue;
            };

            if ti == tj && is_superclass_of_class_env(class_env, cj, ci) {
                keep[i] = false;
                break;
            }
        }
    }
    cs.into_iter()
        .enumerate()
        .filter_map(|(i, c)| if keep[i] { Some(c) } else { None })
        .collect()
}

fn sort_dedup_constraints_stable(mut cs: Vec<Constraint>) -> Vec<Constraint> {
    cs.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    cs.dedup();
    cs
}

fn check_case_exhaustive(
    data_env: &DataEnv,
    scrut_ty: &Ty,
    arms: &[(ast::Pattern, bool)],
) -> Result<()> {
    // Guarded arms are conservatively treated as non-covering.
    if case_has_unguarded_catch_all(arms) {
        return Ok(());
    }

    let mut alts: Vec<String> = Vec::new();
    case_collect_unguarded_top_alts(arms, &mut alts);

    normalize_string_alts(&mut alts);

    if let Some(res) = check_case_primitive_exhaustive(scrut_ty, &alts)? {
        return res;
    }

    if let Some(res) = check_case_adt_exhaustive(data_env, scrut_ty, &alts)? {
        return res;
    }

    Ok(())
}

fn data_derives_show(d: &ast::DataDecl) -> bool {
    d.deriving.iter().any(|c| c == "Show")
}

fn data_derives_eq(d: &ast::DataDecl) -> bool {
    d.deriving.iter().any(|c| c == "Eq")
}

fn entails_show(data_env: &DataEnv, ty: &Ty, in_progress: &mut Vec<Ty>) -> Result<Vec<Constraint>> {
    Ok(match ty {
        Ty::Var(_) => vec![Constraint::Show(ty.clone())],
        Ty::Con(name) => {
            if show_primitives(name) {
                vec![]
            } else if let Some(d) = data_env.get(name) {
                if !data_derives_show(d) {
                    return Err(Error::msg(format!("cannot satisfy constraint: Show {ty}")));
                }
                if !d.params.is_empty() {
                    return Err(Error::msg(format!("cannot satisfy constraint: Show {ty}")));
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
            if !data_derives_show(d) {
                return Err(Error::msg(format!("cannot satisfy constraint: Show {ty}")));
            }
            if d.params.len() != args.len() {
                return Err(Error::msg(format!("cannot satisfy constraint: Show {ty}")));
            }
            entails_show_data_decl(data_env, d, args, in_progress)?
        }
        Ty::Func(_, _) => return Err(Error::msg(format!("cannot satisfy constraint: Show {ty}"))),
    })
}

fn entails_show_row(
    data_env: &DataEnv,
    ty: &Ty,
    in_progress: &mut Vec<Ty>,
) -> Result<Vec<Constraint>> {
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
        _ => {
            return Err(Error::msg(format!(
                "cannot satisfy constraint: ShowRow {ty}"
            )))
        }
    })
}

fn entails_eq(data_env: &DataEnv, ty: &Ty, in_progress: &mut Vec<Ty>) -> Result<Vec<Constraint>> {
    Ok(match ty {
        Ty::Var(_) => vec![Constraint::Eq(ty.clone())],
        Ty::Con(name) => {
            if eq_primitives(name) {
                vec![]
            } else if let Some(d) = data_env.get(name) {
                if !data_derives_eq(d) {
                    return Err(Error::msg(format!("cannot satisfy constraint: Eq {ty}")));
                }
                if !d.params.is_empty() {
                    return Err(Error::msg(format!("cannot satisfy constraint: Eq {ty}")));
                }
                entails_eq_data_decl(data_env, d, &[], in_progress)?
            } else {
                return Err(Error::msg(format!("cannot satisfy constraint: Eq {ty}")));
            }
        }
        Ty::List(t) => entails_eq(data_env, t, in_progress)?,
        Ty::Tuple(ts) => {
            let mut out = Vec::new();
            for t in ts {
                out.extend(entails_eq(data_env, t, in_progress)?);
            }
            out
        }
        Ty::Record(fields) => {
            let mut out = Vec::new();
            for (_, t) in fields {
                out.extend(entails_eq(data_env, t, in_progress)?);
            }
            out
        }
        Ty::RecordOpen(fields, rest) => {
            let mut out = Vec::new();
            for (_, t) in fields {
                out.extend(entails_eq(data_env, t, in_progress)?);
            }
            out.push(Constraint::EqRow((**rest).clone()));
            out
        }
        Ty::App { head, args } => {
            let Ty::Con(name) = &**head else {
                return Err(Error::msg(format!("cannot satisfy constraint: Eq {ty}")));
            };
            let Some(d) = data_env.get(name) else {
                return Err(Error::msg(format!("cannot satisfy constraint: Eq {ty}")));
            };
            if !data_derives_eq(d) {
                return Err(Error::msg(format!("cannot satisfy constraint: Eq {ty}")));
            }
            if d.params.len() != args.len() {
                return Err(Error::msg(format!("cannot satisfy constraint: Eq {ty}")));
            }
            entails_eq_data_decl(data_env, d, args, in_progress)?
        }
        Ty::Func(_, _) => return Err(Error::msg(format!("cannot satisfy constraint: Eq {ty}"))),
    })
}

fn entails_eq_row(
    data_env: &DataEnv,
    ty: &Ty,
    in_progress: &mut Vec<Ty>,
) -> Result<Vec<Constraint>> {
    Ok(match ty {
        Ty::Var(_) => vec![Constraint::EqRow(ty.clone())],
        Ty::Record(fields) => {
            let mut out = Vec::new();
            for (_, t) in fields {
                out.extend(entails_eq(data_env, t, in_progress)?);
            }
            out
        }
        Ty::RecordOpen(fields, rest) => {
            let mut out = Vec::new();
            for (_, t) in fields {
                out.extend(entails_eq(data_env, t, in_progress)?);
            }
            out.extend(entails_eq_row(data_env, rest, in_progress)?);
            out
        }
        _ => return Err(Error::msg(format!("cannot satisfy constraint: EqRow {ty}"))),
    })
}

fn entails_eq_data_decl(
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
            out.extend(entails_eq(data_env, &t, in_progress)?);
        }
    }

    in_progress.pop();
    Ok(out)
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

fn simplify_constraints(
    data_env: &DataEnv,
    class_env: &ClassEnv,
    cs: Vec<Constraint>,
) -> Result<Vec<Constraint>> {
    use std::collections::VecDeque;

    let mut out = Vec::new();
    let mut in_progress = Vec::new();
    let mut work: VecDeque<Constraint> = cs.into_iter().collect();
    let mut expanded: HashMap<String, ()> = HashMap::new();

    while let Some(c) = work.pop_front() {
        simplify_process_constraint(
            data_env,
            class_env,
            &mut expanded,
            &mut in_progress,
            &mut work,
            &mut out,
            c,
        )?;
    }

    out = simplify_context_reduce_user_classes(class_env, out);
    Ok(sort_dedup_constraints_stable(out))
}

fn build_letrec_binding_metadata(
    bindings: &[ast::Binding],
) -> (
    Vec<String>,
    Vec<HashSet<String>>,
    HashMap<String, usize>,
) {
    let n = bindings.len();
    let mut ctx_names: Vec<String> = Vec::with_capacity(n);
    let mut defined_names: Vec<HashSet<String>> = Vec::with_capacity(n);
    for b in bindings {
        let ctx_name = match &b.pat.kind {
            ast::PatternKind::Var(name) => name.clone(),
            _ => "<pattern>".to_string(),
        };
        ctx_names.push(ctx_name);
        let mut names = HashSet::new();
        pat_defined_names(&b.pat, &mut names);
        defined_names.push(names);
    }

    let mut name_to_binding: HashMap<String, usize> = HashMap::new();
    for (i, names) in defined_names.iter().enumerate() {
        for name in names {
            name_to_binding.insert(name.clone(), i);
        }
    }

    (ctx_names, defined_names, name_to_binding)
}

fn build_letrec_dep_graph(
    bindings: &[ast::Binding],
    name_to_binding: &HashMap<String, usize>,
) -> Vec<Vec<usize>> {
    let n = bindings.len();
    let mut graph: Vec<Vec<usize>> = vec![Vec::new(); n];
    for i in 0..n {
        let mut deps = HashSet::new();
        let empty: HashSet<String> = HashSet::new();
        collect_deps_in_expr(&bindings[i].expr, name_to_binding, &empty, &mut deps);
        graph[i] = deps.into_iter().collect();
    }
    graph
}

fn topo_order_sccs(graph: &[Vec<usize>], comps: &[Vec<usize>]) -> Result<Vec<usize>> {
    let n = graph.len();
    let mut node_to_comp = vec![0usize; n];
    for (ci, comp) in comps.iter().enumerate() {
        for &v in comp {
            node_to_comp[v] = ci;
        }
    }

    // Component graph: dependency -> dependent.
    let comp_n = comps.len();
    let mut comp_edges: Vec<HashSet<usize>> = vec![HashSet::new(); comp_n];
    let mut indeg = vec![0usize; comp_n];
    for u in 0..n {
        let cu = node_to_comp[u];
        for &v in &graph[u] {
            let cv = node_to_comp[v];
            if cu == cv {
                continue;
            }
            if comp_edges[cv].insert(cu) {
                indeg[cu] += 1;
            }
        }
    }

    let mut queue: std::collections::VecDeque<usize> = indeg
        .iter()
        .enumerate()
        .filter_map(|(i, &d)| if d == 0 { Some(i) } else { None })
        .collect();
    let mut comp_order = Vec::with_capacity(comp_n);
    while let Some(c) = queue.pop_front() {
        comp_order.push(c);
        for &to in comp_edges[c].iter() {
            indeg[to] -= 1;
            if indeg[to] == 0 {
                queue.push_back(to);
            }
        }
    }
    if comp_order.len() != comp_n {
        return Err(Error::msg("internal error: cyclic component graph"));
    }
    Ok(comp_order)
}

type LetrecBindingInfer = (Vec<(String, Ty)>, Vec<Constraint>);

struct LetrecInferCx<'a> {
    cx: &'a mut InferCtx,
    data_env: &'a DataEnv,
    subst: &'a mut Subst,
    env_scc: &'a TypeEnv,
    placeholders: &'a HashMap<String, Ty>,
    ctx_prefix: &'a str,
    ctx_name: &'a str,
}

fn infer_one_letrec_binding(cxi: &mut LetrecInferCx<'_>, b: ast::Binding) -> Result<LetrecBindingInfer> {
    let mut binds = Vec::new();
    let mut seen = HashSet::new();
    let mut cs_pat = Vec::new();
    let pat_ty = infer_pat_in(
        cxi.cx,
        cxi.data_env,
        cxi.subst,
        cxi.env_scc,
        &b.pat,
        &mut binds,
        &mut seen,
        &mut cs_pat,
    )
    .map_err(|e| e.with_context(format!("in {} binding {}", cxi.ctx_prefix, cxi.ctx_name)))?;

    let env_in = apply_env(cxi.subst, cxi.env_scc);
    let (s_rhs, cs_rhs, t_rhs) = infer_expr_in(cxi.cx, cxi.data_env, &env_in, b.expr)
        .map_err(|e| e.with_context(format!("in {} binding {}", cxi.ctx_prefix, cxi.ctx_name)))?;
    *cxi.subst = compose(&s_rhs, cxi.subst);

    let s_pat = unify(apply(cxi.subst, t_rhs), apply(cxi.subst, pat_ty))
        .map_err(|e| e.with_context(format!("in {} binding {}", cxi.ctx_prefix, cxi.ctx_name)))?;
    *cxi.subst = compose(&s_pat, cxi.subst);

    // Connect binder types to their placeholders so recursive references unify.
    for (name, t) in &binds {
        if let Some(ph) = cxi.placeholders.get(name).cloned() {
            let su = unify(apply(cxi.subst, t.clone()), apply(cxi.subst, ph))
                .map_err(|e| e.with_context(format!("in {} binding {}", cxi.ctx_prefix, cxi.ctx_name)))?;
            *cxi.subst = compose(&su, cxi.subst);
        }
    }

    let mut cs = cs_rhs;
    cs.extend(cs_pat);
    Ok((binds, cs))
}

fn infer_local_letrec_bindings(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    base_env: &TypeEnv,
    bindings: Vec<ast::Binding>,
    ctx_prefix: &str,
) -> Result<(Subst, TypeEnv)> {
    // Constraints produced while typechecking local bindings are captured in their
    // generalized schemes. They should not leak to the surrounding expression.
    let mut s = Subst::new();
    let mut env_global = base_env.clone();

    let n = bindings.len();
    if n == 0 {
        return Ok((s, env_global));
    }

    let (ctx_names, defined_names, name_to_binding) =
        build_letrec_binding_metadata(&bindings);
    let graph = build_letrec_dep_graph(&bindings, &name_to_binding);

    let comps = tarjan_scc(&graph);
    let comp_order = topo_order_sccs(&graph, &comps)?;

    for ci in comp_order {
        let comp = &comps[ci];

        // Placeholders for all names in this SCC (monomorphic during inference).
        let mut placeholders: HashMap<String, Ty> = HashMap::new();
        let mut env_scc = env_global.clone();
        for &bi in comp {
            for name in &defined_names[bi] {
                let tv = cx.fresh();
                placeholders.insert(name.clone(), tv.clone());
                env_scc.insert(name.clone(), Scheme::mono(tv));
            }
        }

        let mut per_bind: Vec<LetrecBindingInfer> = Vec::new();
        for &bi in comp {
            let b = bindings[bi].clone();
            let ctx_name = &ctx_names[bi];

            let mut cxi = LetrecInferCx {
                cx,
                data_env,
                subst: &mut s,
                env_scc: &env_scc,
                placeholders: &placeholders,
                ctx_prefix,
                ctx_name,
            };
            per_bind.push(infer_one_letrec_binding(&mut cxi, b)?);
        }

        let env_gen_base = apply_env(&s, &env_global);
        let mut new_schemes: Vec<(String, Scheme)> = Vec::new();
        for (binds, cs) in per_bind {
            for (name, t) in binds {
                let cs = simplify_constraints(
                    data_env,
                    &ClassEnv::default(),
                    apply_constraints(&s, cs.clone()),
                )?;
                let scheme = generalize_qual(&env_gen_base, cs, apply(&s, t));
                new_schemes.push((name, scheme));
            }
        }

        for (name, scheme) in new_schemes {
            env_global.insert(name, scheme);
        }
    }

    Ok((s, env_global))
}

fn infer_expr_in(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    env: &TypeEnv,
    expr: ast::Expr,
) -> Result<(Subst, Vec<Constraint>, Ty)> {
    use ast::ExprKind;

    let span = expr.span;

    match expr.kind {
        ExprKind::Unit => Ok((Subst::new(), vec![], Ty::Con("Unit".to_string()))),
        ExprKind::Integer(_) => Ok((Subst::new(), vec![], Ty::Con("Integer".to_string()))),
        ExprKind::Float64(_) => Ok((Subst::new(), vec![], Ty::Con("Float64".to_string()))),
        ExprKind::Bool(true) | ExprKind::Bool(false) => {
            Ok((Subst::new(), vec![], Ty::Con("Bool".to_string())))
        }
        ExprKind::String(_) => Ok((
            Subst::new(),
            vec![],
            Ty::List(Box::new(Ty::Con("Char".to_string()))),
        )),
        ExprKind::Char(_) => Ok((Subst::new(), vec![], Ty::Con("Char".to_string()))),

        ExprKind::Var(name) => {
            let s = env
                .get(&name)
                .ok_or_else(|| Error::msg_with_span(format!("unbound variable: {name}"), span))?;
            let (cs, ty) = instantiate_qual(cx, s);
            Ok((Subst::new(), cs, ty))
        }

        ExprKind::Ctor(name) => {
            let s = env
                .get(&name)
                .ok_or_else(|| Error::msg_with_span("unknown constructor", span))?;
            let (cs, ty) = instantiate_qual(cx, s);
            Ok((Subst::new(), cs, ty))
        }

        ExprKind::Lambda { params, body } => {
            infer_expr_lambda(cx, data_env, env, span, params, *body)
        }

        ExprKind::Apply { func, args } => {
            infer_expr_apply(cx, data_env, env, *func, args)
        }

        ExprKind::Annot { expr, ty } => {
            infer_expr_annot(cx, data_env, env, *expr, ty)
        }

        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            infer_expr_if(cx, data_env, env, *cond, *then_branch, *else_branch)
        }

        ExprKind::Tuple(elems) => {
            infer_expr_tuple(cx, data_env, env, elems)
        }

        ExprKind::Cons { head, tail } => {
            infer_expr_cons(cx, data_env, env, *head, *tail)
        }

        ExprKind::List(elems) => {
            infer_expr_list(cx, data_env, env, elems)
        }

        ExprKind::Record(fields) => {
            infer_expr_record(cx, data_env, env, fields)
        }

        ExprKind::Let { bindings, body } => {
            let (s_bind, env2) = infer_local_letrec_bindings(cx, data_env, env, bindings, "let")?;
            let env_body = apply_env(&s_bind, &env2);
            let (s_body, cs_body, t_body) = infer_expr_in(cx, data_env, &env_body, *body)
                .map_err(|e| e.with_context("in let body"))?;
            let s = compose(&s_body, &s_bind);
            Ok((s.clone(), apply_constraints(&s, cs_body), apply(&s, t_body)))
        }

        ExprKind::Where { expr, bindings } => {
            let (s_bind, env2) = infer_local_letrec_bindings(cx, data_env, env, bindings, "where")?;
            let env_body = apply_env(&s_bind, &env2);
            let (s_body, cs_body, t_body) = infer_expr_in(cx, data_env, &env_body, *expr)
                .map_err(|e| e.with_context("in where body"))?;
            let s = compose(&s_body, &s_bind);
            Ok((s.clone(), apply_constraints(&s, cs_body), apply(&s, t_body)))
        }

        ExprKind::Case { expr, arms } => {
            infer_expr_case(cx, data_env, env, span, *expr, arms)
        }

        ExprKind::Do(stmts) => {
            infer_expr_do(cx, data_env, env, span, stmts)
        }
    }
}

fn infer_expr_lambda(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    env: &TypeEnv,
    span: ast::Span,
    params: Vec<String>,
    body: ast::Expr,
) -> Result<(Subst, Vec<Constraint>, Ty)> {
    if params.is_empty() {
        return Err(Error::msg_with_span("expected lambda parameter", span));
    }

    let mut env2 = env.clone();
    let mut param_tys = Vec::new();
    for p in &params {
        let tv = cx.fresh();
        env2.insert(p.clone(), Scheme::mono(tv.clone()));
        param_tys.push(tv);
    }

    let (s_body, cs_body, body_ty) = infer_expr_in(cx, data_env, &env2, body)?;
    let mut out = apply(&s_body, body_ty);
    for pty in param_tys.into_iter().rev() {
        out = Ty::Func(Box::new(apply(&s_body, pty)), Box::new(out));
    }

    Ok((s_body, cs_body, out))
}

fn infer_expr_apply(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    env: &TypeEnv,
    func: ast::Expr,
    args: Vec<ast::Expr>,
) -> Result<(Subst, Vec<Constraint>, Ty)> {
    let (mut s, mut cs, mut fun_ty) = infer_expr_in(cx, data_env, env, func)?;

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

fn infer_expr_annot(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    env: &TypeEnv,
    expr: ast::Expr,
    ty: ast::QualType,
) -> Result<(Subst, Vec<Constraint>, Ty)> {
    let (s1, mut cs1, t1) = infer_expr_in(cx, data_env, env, expr)?;
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
            ast::Predicate::Eq(t) => {
                let t = lower_surface_type(cx, t, &mut holes);
                cs1.push(Constraint::Eq(t));
            }
            ast::Predicate::EqRow(t) => {
                let t = lower_surface_type(cx, t, &mut holes);
                cs1.push(Constraint::EqRow(t));
            }
            ast::Predicate::Lacks { label, row } => {
                let row = lower_surface_type(cx, row, &mut holes);
                cs1.push(Constraint::Lacks {
                    label: label.clone(),
                    row,
                });
            }
            ast::Predicate::Class { class, ty } => {
                let ty = lower_surface_type(cx, ty, &mut holes);
                cs1.push(Constraint::Class {
                    class: class.clone(),
                    ty,
                });
            }
        }
    }

    let t_ann = lower_surface_type(cx, &ty.ty, &mut holes);
    let s2 = unify(apply(&s1, t1), apply(&s1, t_ann.clone()))?;
    let s = compose(&s2, &s1);
    Ok((s.clone(), apply_constraints(&s, cs1), apply(&s, t_ann)))
}

fn infer_expr_if(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    env: &TypeEnv,
    cond: ast::Expr,
    then_branch: ast::Expr,
    else_branch: ast::Expr,
) -> Result<(Subst, Vec<Constraint>, Ty)> {
    let (s_cond, cs_cond, t_cond) = infer_expr_in(cx, data_env, env, cond)
        .map_err(|e| e.with_context("in if cond"))?;
    let s_bool = unify(apply(&s_cond, t_cond), Ty::Con("Bool".to_string()))
        .map_err(|e| e.with_context("in if cond"))?;
    let mut s = compose(&s_bool, &s_cond);
    let mut cs = apply_constraints(&s, cs_cond);

    let env2 = apply_env(&s, env);
    let (s_then, cs_then, t_then) = infer_expr_in(cx, data_env, &env2, then_branch)
        .map_err(|e| e.with_context("in if then"))?;
    s = compose(&s_then, &s);
    cs = apply_constraints(&s, cs);
    cs.extend(apply_constraints(&s, cs_then));

    let env3 = apply_env(&s, env);
    let (s_else, cs_else, t_else) = infer_expr_in(cx, data_env, &env3, else_branch)
        .map_err(|e| e.with_context("in if else"))?;
    s = compose(&s_else, &s);
    cs = apply_constraints(&s, cs);
    cs.extend(apply_constraints(&s, cs_else));

    let s_res = unify(apply(&s, t_then.clone()), apply(&s, t_else))
        .map_err(|e| e.with_context("in if branches"))?;
    s = compose(&s_res, &s);
    cs = apply_constraints(&s, cs);
    Ok((s.clone(), cs, apply(&s, apply(&s, t_then))))
}

fn infer_expr_tuple(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    env: &TypeEnv,
    elems: Vec<ast::Expr>,
) -> Result<(Subst, Vec<Constraint>, Ty)> {
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

fn infer_expr_cons(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    env: &TypeEnv,
    head: ast::Expr,
    tail: ast::Expr,
) -> Result<(Subst, Vec<Constraint>, Ty)> {
    let (s_hd, cs_hd, t_hd) = infer_expr_in(cx, data_env, env, head)?;
    let env2 = apply_env(&s_hd, env);
    let (s_tl, cs_tl, t_tl) = infer_expr_in(cx, data_env, &env2, tail)?;
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

fn infer_expr_list(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    env: &TypeEnv,
    elems: Vec<ast::Expr>,
) -> Result<(Subst, Vec<Constraint>, Ty)> {
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

fn infer_expr_record(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    env: &TypeEnv,
    fields: Vec<(String, ast::Expr)>,
) -> Result<(Subst, Vec<Constraint>, Ty)> {
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

fn infer_expr_case(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    env: &TypeEnv,
    span: ast::Span,
    expr: ast::Expr,
    arms: Vec<ast::CaseArm>,
) -> Result<(Subst, Vec<Constraint>, Ty)> {
    if arms.is_empty() {
        return Err(Error::msg_with_span("empty case", span));
    }

    let (mut s, mut cs, scrut_ty) = infer_expr_in(cx, data_env, env, expr)
        .map_err(|e| e.with_context("in case scrutinee"))?;
    let mut out_ty = cx.fresh();

    let mut pats_for_exhaustive_check: Vec<(ast::Pattern, bool)> = Vec::new();

    for (i, arm) in arms.into_iter().enumerate() {
        let arm_no = i + 1;
        let ast::CaseArm { pat, guard, body } = arm;

        pats_for_exhaustive_check.push((pat.clone(), guard.is_some()));

        let mut binds = Vec::new();
        let mut seen = HashSet::new();
        let mut cs_pat = Vec::new();
        let pat_ty = infer_pat_in(cx, data_env, &mut s, env, &pat, &mut binds, &mut seen, &mut cs_pat)
            .map_err(|e| e.with_context(format!("in case arm {arm_no}")))?;

        let su_pat = unify(apply(&s, pat_ty), apply(&s, scrut_ty.clone()))
            .map_err(|e| e.with_context(format!("in case arm {arm_no}")))?;
        s = compose(&su_pat, &s);
        cs = apply_constraints(&s, cs);
        cs.extend(apply_constraints(&s, cs_pat));

        let mut env_arm = apply_env(&s, env);
        for (name, t) in binds {
            env_arm.insert(name, Scheme::mono(apply(&s, t)));
        }

        if let Some(g) = guard {
            let (s_g, cs_g, t_g) = infer_expr_in(cx, data_env, &env_arm, g)
                .map_err(|e| e.with_context(format!("in case arm {arm_no} guard")))?;
            s = compose(&s_g, &s);
            cs = apply_constraints(&s, cs);
            cs.extend(apply_constraints(&s, cs_g));

            let su_g = unify(apply(&s, t_g), Ty::Con("Bool".to_string()))
                .map_err(|e| e.with_context(format!("in case arm {arm_no} guard")))?;
            s = compose(&su_g, &s);
            cs = apply_constraints(&s, cs);
            env_arm = apply_env(&s, &env_arm);
        }

        let (s_arm, cs_arm, arm_ty) = infer_expr_in(cx, data_env, &env_arm, body)
            .map_err(|e| e.with_context(format!("in case arm {arm_no}")))?;
        s = compose(&s_arm, &s);
        cs = apply_constraints(&s, cs);
        cs.extend(apply_constraints(&s, cs_arm));

        let su_out = unify(apply(&s, out_ty.clone()), apply(&s, arm_ty))
            .map_err(|e| e.with_context(format!("in case arm {arm_no}")))?;
        s = compose(&su_out, &s);
        cs = apply_constraints(&s, cs);
        out_ty = apply(&s, out_ty);
    }

    let scrut_ty = apply(&s, scrut_ty);
    check_case_exhaustive(data_env, &scrut_ty, &pats_for_exhaustive_check)
        .map_err(|e| e.with_context("in case"))?;

    Ok((s.clone(), cs, apply(&s, out_ty)))
}

fn infer_expr_do(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    env: &TypeEnv,
    span: ast::Span,
    stmts: Vec<ast::DoStmt>,
) -> Result<(Subst, Vec<Constraint>, Ty)> {
    if stmts.is_empty() {
        return Err(Error::msg_with_span("empty do", span));
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
                    .map_err(|e| e.with_context(format!("in do stmt {stmt_no} (<-)")))?;

                let env_in = apply_env(&s, &env2);
                let (s_e, cs_e, t_e) = infer_expr_in(cx, data_env, &env_in, expr)
                    .map_err(|e| e.with_context(format!("in do stmt {stmt_no} (<-)")))?;
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
                .map_err(|e| e.with_context(format!("in do stmt {stmt_no} (<-)")))?;
                s = compose(&su, &s);
                cs = apply_constraints(&s, cs);

                let su_pat = unify(apply(&s, pat_ty), apply(&s, io_r.clone()))
                    .map_err(|e| e.with_context(format!("in do stmt {stmt_no} (<-)")))?;
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
                    .map_err(|e| e.with_context(format!("in do stmt {stmt_no}")))?;
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
                .map_err(|e| e.with_context(format!("in do stmt {stmt_no}")))?;
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
    let last_ty = last_ty.ok_or_else(|| Error::msg_with_span("do must end with expression", span))?;
    Ok((s.clone(), cs, apply(&s, last_ty)))
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

pub fn stdlib_root() -> PathBuf {
    stdlib_cache::stdlib_root()
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
        emitted_qualified: HashSet::new(),
        emitted_unqualified: HashSet::new(),
    };

    let entry_mod = loader.load_ast(&entry)?;

    let mut items = Vec::new();
    let mut defined: HashMap<String, String> = HashMap::new();

    let mut deps = Vec::new();
    loader.collect_imports(&entry_mod, entry_dir, &mut deps)?;

    // Temporary diagnostic to pinpoint duplicate qualified forwarders in stdlib smoke tests.
    // (Kept behind env var to avoid affecting normal runs.)
    let debug_imports = std::env::var("KSCR_DEBUG_IMPORTS").ok().is_some();
    if debug_imports {
        eprintln!("[KSCR_DEBUG_IMPORTS] entry: {}", entry.display());
        let mut counts: HashMap<String, usize> = HashMap::new();
        for it in &deps {
            let mut names = HashSet::new();
            item_defined_names(it, &mut names);
            for n in names {
                *counts.entry(n).or_insert(0) += 1;
            }
        }
        let mut dupes: Vec<(String, usize)> = counts.into_iter().filter(|(_, k)| *k > 1).collect();
        dupes.sort_by(|a, b| a.0.cmp(&b.0));
        for (n, k) in dupes {
            if n.starts_with("L.") {
                eprintln!("[KSCR_DEBUG_IMPORTS] deps defines {n} x{k}");
                // Print rough origins for each duplicate.
                for it in &deps {
                    let mut names = HashSet::new();
                    item_defined_names(it, &mut names);
                    if names.contains(&n) {
                        eprintln!("[KSCR_DEBUG_IMPORTS]   origin: {}", name_origin_hint(it, &n));
                    }
                }
            }
        }
    }

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
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ImportQualKey {
    path: PathBuf,
    qual: String,
}

struct ModuleLoader {
    cache: HashMap<PathBuf, ast::Module>,
    stack: Vec<PathBuf>,
    emitted_qualified: HashSet<ImportQualKey>,
    emitted_unqualified: HashSet<PathBuf>,
}

#[derive(Clone, Debug, Default)]
struct QualEnv {
    allowed: HashSet<String>,
    /// local qualifier -> canonical module name
    local_to_module: HashMap<String, String>,
    /// canonical module names that have multiple local qualifiers
    ambiguous_modules: HashSet<String>,
}

fn module_qual_env(module: &ast::Module) -> QualEnv {
    let mut env = QualEnv::default();
    let mut module_counts: HashMap<String, usize> = HashMap::new();

    for it in &module.items {
        let ast::Item::Import(id) = it else {
            continue;
        };
        let local = id.as_name.clone().unwrap_or_else(|| id.module.clone());
        env.allowed.insert(local.clone());
        env.local_to_module.insert(local, id.module.clone());
    }

    // `import qualified M as Q` introduces both `Q.x` and (optionally) the canonical prefix
    // `M.x` for convenience. This matches existing tests that use `import qualified Data.List as L`
    // but still refer to members as `Data.List.map` in desugared stdlib imports.
    for it in &module.items {
        let ast::Item::Import(id) = it else {
            continue;
        };
        if !id.qualified {
            continue;
        }
        let canon = id.module.clone();
        env.allowed.insert(canon.clone());
        env.local_to_module.insert(canon, id.module.clone());
    }

    for module_name in env.local_to_module.values() {
        *module_counts.entry(module_name.clone()).or_insert(0) += 1;
    }
    env.ambiguous_modules = module_counts
        .into_iter()
        .filter_map(|(m, n)| if n > 1 { Some(m) } else { None })
        .collect();

    env
}

fn desugar_qualified_ref(name: &str, env: &QualEnv) -> Result<String> {
    let Some((qual, _member)) = name.rsplit_once('.') else {
        return Ok(name.to_string());
    };

    if !env.allowed.contains(qual) {
        let mut allowed: Vec<_> = env.allowed.iter().cloned().collect();
        allowed.sort();
        return Err(Error::msg(format!(
            "unknown qualifier {qual} in {name} (allowed: {})",
            allowed.join(", ")
        )));
    };

    Ok(name.to_string())
}

fn desugar_qualified_expr(expr: ast::Expr, env: &QualEnv) -> Result<ast::Expr> {
    use ast::ExprKind;
    let span = expr.span;
    let kind = match expr.kind {
        ExprKind::Var(n) => ExprKind::Var(desugar_qualified_ref(&n, env)?),
        ExprKind::Ctor(n) => ExprKind::Ctor(desugar_qualified_ref(&n, env)?),
        ExprKind::Lambda { params, body } => ExprKind::Lambda {
            params,
            body: Box::new(desugar_qualified_expr(*body, env)?),
        },
        ExprKind::Apply { func, args } => ExprKind::Apply {
            func: Box::new(desugar_qualified_expr(*func, env)?),
            args: args
                .into_iter()
                .map(|e| desugar_qualified_expr(e, env))
                .collect::<Result<Vec<_>>>()?,
        },
        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => ExprKind::If {
            cond: Box::new(desugar_qualified_expr(*cond, env)?),
            then_branch: Box::new(desugar_qualified_expr(*then_branch, env)?),
            else_branch: Box::new(desugar_qualified_expr(*else_branch, env)?),
        },
        ExprKind::Let { bindings, body } => ExprKind::Let {
            bindings: bindings
                .into_iter()
                .map(|b| desugar_qualified_binding(b, env))
                .collect::<Result<Vec<_>>>()?,
            body: Box::new(desugar_qualified_expr(*body, env)?),
        },
        ExprKind::Where { expr, bindings } => ExprKind::Where {
            expr: Box::new(desugar_qualified_expr(*expr, env)?),
            bindings: bindings
                .into_iter()
                .map(|b| desugar_qualified_binding(b, env))
                .collect::<Result<Vec<_>>>()?,
        },
        ExprKind::Annot { expr, ty } => ExprKind::Annot {
            expr: Box::new(desugar_qualified_expr(*expr, env)?),
            ty: desugar_qualified_qual_type(ty, env)?,
        },
        ExprKind::Do(stmts) => ExprKind::Do(
            stmts
                .into_iter()
                .map(|s| desugar_qualified_do_stmt(s, env))
                .collect::<Result<Vec<_>>>()?,
        ),
        ExprKind::Case { expr, arms } => ExprKind::Case {
            expr: Box::new(desugar_qualified_expr(*expr, env)?),
            arms: arms
                .into_iter()
                .map(|a| desugar_qualified_case_arm(a, env))
                .collect::<Result<Vec<_>>>()?,
        },
        ExprKind::Cons { head, tail } => ExprKind::Cons {
            head: Box::new(desugar_qualified_expr(*head, env)?),
            tail: Box::new(desugar_qualified_expr(*tail, env)?),
        },
        ExprKind::List(es) => ExprKind::List(
            es.into_iter()
                .map(|e| desugar_qualified_expr(e, env))
                .collect::<Result<Vec<_>>>()?,
        ),
        ExprKind::Tuple(es) => ExprKind::Tuple(
            es.into_iter()
                .map(|e| desugar_qualified_expr(e, env))
                .collect::<Result<Vec<_>>>()?,
        ),
        ExprKind::Record(fs) => ExprKind::Record(
            fs.into_iter()
                .map(|(l, e)| Ok((l, desugar_qualified_expr(e, env)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
        x => x,
    };
    Ok(ast::Expr::new(span, kind))
}

fn desugar_qualified_case_arm(arm: ast::CaseArm, env: &QualEnv) -> Result<ast::CaseArm> {
    Ok(ast::CaseArm {
        pat: desugar_qualified_pattern(arm.pat, env)?,
        guard: arm
            .guard
            .map(|e| desugar_qualified_expr(e, env))
            .transpose()?,
        body: desugar_qualified_expr(arm.body, env)?,
    })
}

fn desugar_qualified_do_stmt(stmt: ast::DoStmt, env: &QualEnv) -> Result<ast::DoStmt> {
    Ok(match stmt {
        ast::DoStmt::Bind { pat, expr } => ast::DoStmt::Bind {
            pat: desugar_qualified_pattern(pat, env)?,
            expr: desugar_qualified_expr(expr, env)?,
        },
        ast::DoStmt::Expr(e) => ast::DoStmt::Expr(desugar_qualified_expr(e, env)?),
    })
}

fn desugar_qualified_binding(b: ast::Binding, env: &QualEnv) -> Result<ast::Binding> {
    Ok(ast::Binding {
        pat: desugar_qualified_pattern(b.pat, env)?,
        expr: desugar_qualified_expr(b.expr, env)?,
    })
}

fn desugar_qualified_pattern(p: ast::Pattern, env: &QualEnv) -> Result<ast::Pattern> {
    use ast::PatternKind;
    let span = p.span;
    let kind = match p.kind {
        PatternKind::Var(n) => {
            if n.contains('.') {
                return Err(Error::msg(format!(
                    "qualified name is not allowed in binder: {n}"
                )));
            }
            PatternKind::Var(n)
        }
        PatternKind::As(n, p) => {
            if n.contains('.') {
                return Err(Error::msg(format!(
                    "qualified name is not allowed in binder: {n}"
                )));
            }
            PatternKind::As(n, Box::new(desugar_qualified_pattern(*p, env)?))
        }
        PatternKind::Tuple(ps) => PatternKind::Tuple(
            ps.into_iter()
                .map(|p| desugar_qualified_pattern(p, env))
                .collect::<Result<Vec<_>>>()?,
        ),
        PatternKind::List(ps) => PatternKind::List(
            ps.into_iter()
                .map(|p| desugar_qualified_pattern(p, env))
                .collect::<Result<Vec<_>>>()?,
        ),
        PatternKind::Record(fs) => PatternKind::Record(
            fs.into_iter()
                .map(|(l, p)| Ok((l, desugar_qualified_pattern(p, env)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
        PatternKind::RecordLoose(fs, rest) => {
            if let Some(rest_name) = rest.as_ref() {
                if rest_name.contains('.') {
                    return Err(Error::msg(format!(
                        "qualified name is not allowed in binder: {rest_name}"
                    )));
                }
            }
            PatternKind::RecordLoose(
                fs.into_iter()
                    .map(|(l, p)| Ok((l, desugar_qualified_pattern(p, env)?)))
                    .collect::<Result<Vec<_>>>()?,
                rest,
            )
        }
        PatternKind::Cons(a, b) => PatternKind::Cons(
            Box::new(desugar_qualified_pattern(*a, env)?),
            Box::new(desugar_qualified_pattern(*b, env)?),
        ),
        PatternKind::Or(a, b) => PatternKind::Or(
            Box::new(desugar_qualified_pattern(*a, env)?),
            Box::new(desugar_qualified_pattern(*b, env)?),
        ),
        PatternKind::View(p, e) => PatternKind::View(
            Box::new(desugar_qualified_pattern(*p, env)?),
            Box::new(desugar_qualified_expr(*e, env)?),
        ),
        PatternKind::Constructor { name, args } => PatternKind::Constructor {
            name: desugar_qualified_ref(&name, env)?,
            args: args
                .into_iter()
                .map(|p| desugar_qualified_pattern(p, env))
                .collect::<Result<Vec<_>>>()?,
        },
        PatternKind::Literal(e) => PatternKind::Literal(desugar_qualified_expr(e, env)?),
        x => x,
    };
    Ok(ast::Pattern::new(span, kind))
}

fn desugar_qualified_type(ty: ast::Type, env: &QualEnv) -> Result<ast::Type> {
    use ast::Type;
    Ok(match ty {
        Type::List(t) => Type::List(Box::new(desugar_qualified_type(*t, env)?)),
        Type::Tuple(ts) => Type::Tuple(
            ts.into_iter()
                .map(|t| desugar_qualified_type(t, env))
                .collect::<Result<Vec<_>>>()?,
        ),
        Type::Record(fs) => Type::Record(
            fs.into_iter()
                .map(|(l, t)| Ok((l, desugar_qualified_type(t, env)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
        Type::RecordOpen(fs, r) => Type::RecordOpen(
            fs.into_iter()
                .map(|(l, t)| Ok((l, desugar_qualified_type(t, env)?)))
                .collect::<Result<Vec<_>>>()?,
            Box::new(desugar_qualified_type(*r, env)?),
        ),
        Type::Var(n) => Type::Var(desugar_qualified_ref(&n, env)?),
        Type::App { head, args } => Type::App {
            head: Box::new(desugar_qualified_type(*head, env)?),
            args: args
                .into_iter()
                .map(|t| desugar_qualified_type(t, env))
                .collect::<Result<Vec<_>>>()?,
        },
        x => x,
    })
}

fn desugar_qualified_predicate(p: ast::Predicate, env: &QualEnv) -> Result<ast::Predicate> {
    Ok(match p {
        ast::Predicate::Show(t) => ast::Predicate::Show(desugar_qualified_type(t, env)?),
        ast::Predicate::ShowRow(t) => ast::Predicate::ShowRow(desugar_qualified_type(t, env)?),
        ast::Predicate::Eq(t) => ast::Predicate::Eq(desugar_qualified_type(t, env)?),
        ast::Predicate::EqRow(t) => ast::Predicate::EqRow(desugar_qualified_type(t, env)?),
        ast::Predicate::Class { class, ty } => ast::Predicate::Class {
            class,
            ty: desugar_qualified_type(ty, env)?,
        },
        ast::Predicate::Lacks { label, row } => ast::Predicate::Lacks {
            label,
            row: desugar_qualified_type(row, env)?,
        },
    })
}

fn desugar_qualified_qual_type(qt: ast::QualType, env: &QualEnv) -> Result<ast::QualType> {
    Ok(ast::QualType {
        preds: qt
            .preds
            .into_iter()
            .map(|p| desugar_qualified_predicate(p, env))
            .collect::<Result<Vec<_>>>()?,
        ty: desugar_qualified_type(qt.ty, env)?,
    })
}

fn desugar_module_qualified_names(module: &mut ast::Module) -> Result<()> {
    let env = module_qual_env(module);

    module.items = module
        .items
        .clone()
        .into_iter()
        .map(|it| {
            Ok(match it {
                ast::Item::Binding(b) => ast::Item::Binding(desugar_qualified_binding(b, &env)?),
                ast::Item::TypeAlias(mut ta) => {
                    ta.ty = desugar_qualified_type(ta.ty, &env)?;
                    ast::Item::TypeAlias(ta)
                }
                ast::Item::DataDecl(mut dd) => {
                    for ctor in &mut dd.ctors {
                        ctor.args = ctor
                            .args
                            .clone()
                            .into_iter()
                            .map(|t| desugar_qualified_type(t, &env))
                            .collect::<Result<Vec<_>>>()?;
                    }
                    ast::Item::DataDecl(dd)
                }
                ast::Item::ClassDecl(mut c) => {
                    for m in &mut c.methods {
                        m.ty = desugar_qualified_qual_type(m.ty.clone(), &env)?;
                    }
                    ast::Item::ClassDecl(c)
                }
                ast::Item::InstanceDecl(mut inst) => {
                    inst.preds = inst
                        .preds
                        .into_iter()
                        .map(|p| desugar_qualified_predicate(p, &env))
                        .collect::<Result<Vec<_>>>()?;
                    inst.ty = desugar_qualified_type(inst.ty, &env)?;
                    inst.methods = inst
                        .methods
                        .into_iter()
                        .map(|b| desugar_qualified_binding(b, &env))
                        .collect::<Result<Vec<_>>>()?;
                    ast::Item::InstanceDecl(inst)
                }
                x @ (ast::Item::Import(_) | ast::Item::Export(_) | ast::Item::Fixity(_)) => x,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(())
}

impl ModuleLoader {
    fn debug_print_import(&self, enabled: bool, id: &ast::ImportDecl) {
        if !enabled {
            return;
        }
        eprintln!(
            "[KSCR_DEBUG_IMPORTS] saw import: module={} qualified={} as={:?}",
            id.module, id.qualified, id.as_name
        );
    }

    fn resolve_import_path(&self, dir: &Path, module: &str) -> Result<std::path::PathBuf> {
        let rel = module.replace('.', "/");
        let local = dir.join(format!("{}.ks", rel));
        let stdlib = stdlib_cache::stdlib_root().join(format!("{}.ks", rel));

        std::fs::canonicalize(&local)
            .or_else(|_| std::fs::canonicalize(&stdlib))
            .map_err(|_| {
                Error::msg(format!(
                    "cannot find module file for import {} (tried: {}, {})",
                    module,
                    local.display(),
                    stdlib.display()
                ))
            })
    }

    fn load_ast(&mut self, path: &Path) -> Result<ast::Module> {
        if let Some(m) = self.cache.get(path) {
            return Ok(m.clone());
        }
        if let Some(m) = stdlib_cache::load_ast_stdlib_cached(path)? {
            self.cache.insert(path.to_path_buf(), m.clone());
            return Ok(m);
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
        let debug_imports = std::env::var("KSCR_DEBUG_IMPORTS").ok().is_some();

        // Track `(imported file, qualifier)` pairs emitted during this single call.
        // This avoids duplicates when the same import is syntactically present multiple times
        // in one module (directly or via desugaring).
        let mut local_emitted_qualified: HashSet<ImportQualKey> = HashSet::new();

        for it in &module.items {
            let ast::Item::Import(id) = it else {
                continue;
            };

            self.debug_print_import(debug_imports, id);

            let primary_qual = id.as_name.as_deref().unwrap_or(&id.module);
            let canonical_qual = id.module.as_str();

            // NOTE: For `import qualified M as Q`, we only need to emit a single qualified
            // forwarder set. Emitting more than one can create duplicate names like `Q.x`.

            let p = self.resolve_import_path(dir, &id.module)?;

            if let Some(pos) = self.stack.iter().position(|x| x == &p) {
                let mut chain: Vec<String> = self.stack[pos..]
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect();
                chain.push(p.display().to_string());
                return Err(Error::msg(format!(
                    "cyclic imports: {}",
                    chain.join(" -> ")
                )));
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

            let exports = module_exported_names(&imported)?;

            // Emit qualified bindings for each (module file, qualifier) pair.
            // IMPORTANT:
            // - `import qualified M as Q` creates bindings `Q.x`.
            // - `import M as Q` is unqualified, but must still allow `Q.x` dotted refs.
            //   For that, we also emit qualified bindings under the alias.
            // - Plain `import M` uses the canonical qualifier only.
            let quals: Vec<&str> = if id.qualified || id.as_name.is_some() {
                vec![primary_qual]
            } else {
                vec![canonical_qual]
            };

            for qual in quals {
                let key = ImportQualKey {
                    path: p.clone(),
                    qual: qual.to_string(),
                };
                if local_emitted_qualified.insert(key.clone()) && self.emitted_qualified.insert(key) {
                    let items = import_qualified_items_for_decl(&imported, qual, &exports)?;
                    if debug_imports {
                        let mut counts: HashMap<String, usize> = HashMap::new();
                        for it in &items {
                            let mut names = HashSet::new();
                            item_defined_names(it, &mut names);
                            for n in names {
                                *counts.entry(n).or_insert(0) += 1;
                            }
                        }
                        if let Some(k) = counts.get("L.null") {
                            eprintln!(
                                "[KSCR_DEBUG_IMPORTS] import_qualified_items_for_decl produced L.null x{k} for module {} as {qual}",
                                id.module
                            );
                            for it in &items {
                                let mut names = HashSet::new();
                                item_defined_names(it, &mut names);
                                if names.contains("L.null") {
                                    eprintln!("[KSCR_DEBUG_IMPORTS]   L.null item: {it:?}");
                                }
                            }
                        }
                    }
                    out.extend(items);
                }
            }

            // Emit unqualified forwarders at most once per module file.
            // This keeps the flattened module namespace reasonably conflict-free.
            // NOTE: even for `import qualified ... as ...`, we still want the canonical module
            // name to be usable as a qualifier inside the importing module. Do this by emitting
            // `M.x = Q.x` forwarders (qualified-to-qualified), not by importing unqualified.
            if !id.qualified && self.emitted_unqualified.insert(p) {
                out.extend(import_unqualified_forwarders(
                    &imported,
                    primary_qual,
                    &exports,
                )?);
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
            ast::Item::Import(_)
            | ast::Item::Export(_)
            | ast::Item::Fixity(_)
            | ast::Item::ClassDecl(_)
            | ast::Item::InstanceDecl(_) => None,
            it => Some(it.clone()),
        })
        .collect()
}

fn module_exported_names(module: &ast::Module) -> Result<HashSet<String>> {
    let mut exports = HashSet::new();
    let mut has_export_decl = false;

    for it in &module.items {
        let ast::Item::Export(ed) = it else {
            continue;
        };
        has_export_decl = true;
        for spec in &ed.specs {
            match spec {
                ast::ExportSpec::Name(n) => {
                    exports.insert(n.clone());
                }
                ast::ExportSpec::Type { name, ctors } => {
                    exports.insert(name.clone());

                    let dd = module.items.iter().find_map(|it| match it {
                        ast::Item::DataDecl(d) if d.name == *name => Some(d),
                        _ => None,
                    });
                    let Some(dd) = dd else {
                        // Type exports can refer to classes too (e.g. `Monad(..)`), which are not
                        // `DataDecl`s. For MVP export checking, allow these through.
                        exports.insert(name.clone());
                        continue;
                    };

                    match ctors {
                        ast::ExportCtors::All => {
                            exports.extend(dd.ctors.iter().map(|c| c.name.clone()));
                        }
                        ast::ExportCtors::Some(cs) => {
                            let known: HashSet<&str> =
                                dd.ctors.iter().map(|c| c.name.as_str()).collect();
                            for c in cs {
                                if !known.contains(c.as_str()) {
                                    return Err(Error::msg(format!(
                                        "export list references unknown constructor: {name}({c})"
                                    )));
                                }
                                exports.insert(c.clone());
                            }
                        }
                    }
                }
            }
        }
    }

    if !has_export_decl {
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
                ast::Item::Import(_)
                | ast::Item::Export(_)
                | ast::Item::Fixity(_)
                | ast::Item::ClassDecl(_)
                | ast::Item::InstanceDecl(_) => {}
            }
        }
        return Ok(all);
    }

    Ok(exports)
}

fn import_qualified_items_for_decl(
    module: &ast::Module,
    qual: &str,
    exports: &HashSet<String>,
) -> Result<Vec<ast::Item>> {
    // Always provide qualifier names (but only for exported items).
    let mut out = qualify_items(module, qual, exports)?;

    // Some imported modules (notably stdlib) may still carry function clauses as multiple
    // top-level bindings with the same name. After qualification this becomes a conflict like
    // `L.null`, but simply dropping one clause changes semantics.
    //
    // For now, apply this merge only to stdlib-style List predicates (e.g. `null`) to avoid
    // altering general import/qualification semantics.
    out = merge_duplicate_bindings_for_names(out, &["null"])?;

    // MVP: always import class/instance declarations.
    // Rationale: instances are required for constraint solving, and class declarations carry
    // method types used to recognize method calls.
    //
    // NOTE: we must also qualify *types inside* these declarations (instance heads, method
    // signatures, superclass predicates) so that instance resolution matches the qualified type
    // constructors introduced by imports (e.g. `Prelude.Maybe`).
    out.extend(qualify_class_instance_decls(module, qual, exports)?);
    Ok(out)
}

fn merge_build_rename_map(from: &[String], to: &[String]) -> HashMap<String, String> {
    let mut map: HashMap<String, String> = HashMap::new();
    for (f, t) in from.iter().zip(to.iter()) {
        if f != t {
            map.insert(f.clone(), t.clone());
        }
    }
    map
}

fn merge_subst_pat(p: &ast::Pattern, map: &HashMap<String, String>) -> ast::Pattern {
    use ast::PatternKind;
    let mut out = p.clone();
    out.kind = match &p.kind {
        PatternKind::Var(n) => {
            if let Some(n2) = map.get(n) {
                PatternKind::Var(n2.clone())
            } else {
                PatternKind::Var(n.clone())
            }
        }
        PatternKind::Tuple(ps) => PatternKind::Tuple(ps.iter().map(|p| merge_subst_pat(p, map)).collect()),
        PatternKind::List(ps) => PatternKind::List(ps.iter().map(|p| merge_subst_pat(p, map)).collect()),
        PatternKind::Record(fields) => PatternKind::Record(
            fields
                .iter()
                .map(|(k, p)| (k.clone(), merge_subst_pat(p, map)))
                .collect(),
        ),
        PatternKind::Constructor { name, args } => PatternKind::Constructor {
            name: name.clone(),
            args: args.iter().map(|p| merge_subst_pat(p, map)).collect(),
        },
        PatternKind::As(n, p) => {
            let n2 = map.get(n).cloned().unwrap_or_else(|| n.clone());
            PatternKind::As(n2, Box::new(merge_subst_pat(p, map)))
        }
        // Literals / wildcards
        other => other.clone(),
    };
    out
}

fn merge_subst_expr(e: &ast::Expr, map: &HashMap<String, String>) -> ast::Expr {
    use ast::ExprKind;
    let mut out = e.clone();
    out.kind = match &e.kind {
        ExprKind::Var(n) => {
            if let Some(n2) = map.get(n) {
                ExprKind::Var(n2.clone())
            } else {
                ExprKind::Var(n.clone())
            }
        }
        ExprKind::Apply { func, args } => ExprKind::Apply {
            func: Box::new(merge_subst_expr(func, map)),
            args: args.iter().map(|e| merge_subst_expr(e, map)).collect(),
        },
        ExprKind::Lambda { params, body } => ExprKind::Lambda {
            // NOTE: We assume no shadowing here because we're only rewriting the
            // specific case-scrutinee tuple expression produced for top-level clauses.
            params: params
                .iter()
                .map(|p| map.get(p).cloned().unwrap_or_else(|| p.clone()))
                .collect(),
            body: Box::new(merge_subst_expr(body, map)),
        },
        ExprKind::Let { bindings, body } => ExprKind::Let {
            bindings: bindings
                .iter()
                .map(|b| {
                    let mut b2 = b.clone();
                    b2.pat = merge_subst_pat(&b.pat, map);
                    b2.expr = merge_subst_expr(&b.expr, map);
                    b2
                })
                .collect(),
            body: Box::new(merge_subst_expr(body, map)),
        },
        ExprKind::Case { expr, arms } => ExprKind::Case {
            expr: Box::new(merge_subst_expr(expr, map)),
            arms: arms
                .iter()
                .map(|a| {
                    let mut a2 = a.clone();
                    a2.pat = merge_subst_pat(&a.pat, map);
                    a2.guard = a.guard.as_ref().map(|g| merge_subst_expr(g, map));
                    a2.body = merge_subst_expr(&a.body, map);
                    a2
                })
                .collect(),
        },
        ExprKind::Tuple(es) => ExprKind::Tuple(es.iter().map(|e| merge_subst_expr(e, map)).collect()),
        ExprKind::List(es) => ExprKind::List(es.iter().map(|e| merge_subst_expr(e, map)).collect()),
        ExprKind::Record(fields) => ExprKind::Record(
            fields
                .iter()
                .map(|(k, e)| (k.clone(), merge_subst_expr(e, map)))
                .collect(),
        ),
        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => ExprKind::If {
            cond: Box::new(merge_subst_expr(cond, map)),
            then_branch: Box::new(merge_subst_expr(then_branch, map)),
            else_branch: Box::new(merge_subst_expr(else_branch, map)),
        },
        other => other.clone(),
    };
    out
}

fn merge_substitute_vars_in_expr(expr: &ast::Expr, from: &[String], to: &[String]) -> ast::Expr {
    let map = merge_build_rename_map(from, to);
    if map.is_empty() {
        return expr.clone();
    }
    merge_subst_expr(expr, &map)
}

fn merge_eta_collapse_to_unary(expr: ast::Expr) -> Result<ast::Expr> {
    use ast::{Expr, ExprKind};
    let ExprKind::Lambda { params, body } = expr.kind else {
        return Ok(expr);
    };
    if params.len() <= 1 {
        return Ok(Expr::new(expr.span, ExprKind::Lambda { params, body }));
    }
    let first = params[0].clone();
    let rest: Vec<String> = params[1..].to_vec();
    let applied = Expr::new(
        expr.span,
        ExprKind::Apply {
            func: Box::new(Expr::new(expr.span, ExprKind::Lambda { params: rest, body })),
            args: vec![Expr::new(expr.span, ExprKind::Var(first.clone()))],
        },
    );
    Ok(Expr::new(
        expr.span,
        ExprKind::Lambda {
            params: vec![first],
            body: Box::new(applied),
        },
    ))
}

fn merge_unwrap_case_body(mut e: ast::Expr) -> Option<(ast::Expr, Vec<ast::CaseArm>)> {
    // In some transformations (e.g. eta-expansion), the body may become an application.
    // We only care about recovering the original `case` arms.
    loop {
        match &e.kind {
            ast::ExprKind::Case { expr, arms } => return Some(((**expr).clone(), arms.clone())),
            ast::ExprKind::Apply { func, args: _ } => {
                // Peel left-nested applications; `case` may live inside `func`.
                e = (**func).clone();
            }
            ast::ExprKind::Lambda { params: _, body } => {
                // Eta-collapsing can leave us with an inner lambda; keep unwrapping.
                e = (**body).clone();
            }
            _ => return None,
        }
    }
}

fn merge_binding_name(b: &ast::Binding) -> Option<&str> {
    match &b.pat.kind {
        ast::PatternKind::Var(n) => Some(n.as_str()),
        _ => None,
    }
}

fn merge_rewrite_tuple_scrutinee_arms_to_cons(b_scrut: &ast::Expr, b_arms: &mut [ast::CaseArm]) {
    // If RHS was a tuple-scrutinee clause, collapsing to unary means we must
    // also rewrite its patterns to match on the single arg directly.
    if let ast::ExprKind::Tuple(_) = &b_scrut.kind {
        for arm in b_arms {
            if let ast::PatternKind::Tuple(ps) = &arm.pat.kind {
                if ps.len() == 3 {
                    // Expect (_, Constructor(:), _) shape.
                    if let ast::PatternKind::Constructor { name: ctor, args } = &ps[1].kind {
                        if ctor == ":" && args.is_empty() {
                            arm.pat = ast::Pattern::new(
                                arm.pat.span,
                                ast::PatternKind::Cons(
                                    Box::new(ast::Pattern::new(ps[0].span, ps[0].kind.clone())),
                                    Box::new(ast::Pattern::new(ps[2].span, ps[2].kind.clone())),
                                ),
                            );
                        }
                    }
                }
            }
        }
    }
}

fn merge_is_collapse_needed(expr: &ast::Expr) -> bool {
    match &expr.kind {
        ast::ExprKind::Lambda { params, .. } => params.len() > 1,
        _ => false,
    }
}

fn merge_rebuild_lambda_case(span: ast::Span, params: Vec<String>, scrut: ast::Expr, arms: Vec<ast::CaseArm>) -> ast::Expr {
    ast::Expr::new(
        span,
        ast::ExprKind::Lambda {
            params,
            body: Box::new(ast::Expr::new(
                span,
                ast::ExprKind::Case {
                    expr: Box::new(scrut),
                    arms,
                },
            )),
        },
    )
}

fn merge_try_merge_lambda_clauses(prev: &mut ast::Binding, b: &ast::Binding, name: &str) -> Result<bool> {
    let (ast::ExprKind::Lambda { params: p_params, body: p_body }, ast::ExprKind::Lambda { params: b_params, body: b_body }) =
        (&prev.expr.kind, &b.expr.kind)
    else {
        return Err(Error::msg(format!(
            "duplicate binding for `{name}` cannot be merged (unexpected shape)"
        )));
    };

    // If arity differs, collapse both sides to unary so we can merge on one scrutinee.
    if p_params.len() != b_params.len() {
        while merge_is_collapse_needed(&prev.expr) {
            prev.expr = merge_eta_collapse_to_unary(prev.expr.clone())?;
        }

        let mut b_expr = b.expr.clone();
        while merge_is_collapse_needed(&b_expr) {
            b_expr = merge_eta_collapse_to_unary(b_expr)?;
        }

        let (ast::ExprKind::Lambda { params: p_params, body: p_body }, ast::ExprKind::Lambda { params: b_params, body: b_body }) =
            (&prev.expr.kind, &b_expr.kind)
        else {
            return Err(Error::msg(format!(
                "duplicate binding for `{name}` cannot be merged (unexpected shape after collapse)"
            )));
        };
        if p_params.len() != b_params.len() {
            return Err(Error::msg(format!(
                "duplicate binding for `{name}` cannot be merged (arity mismatch)"
            )));
        }

        let Some((p_scrut, mut p_arms)) = merge_unwrap_case_body((**p_body).clone()) else {
            return Err(Error::msg(format!(
                "duplicate binding for `{name}` cannot be merged (expected case body)"
            )));
        };
        let Some((b_scrut, mut b_arms)) = merge_unwrap_case_body((**b_body).clone()) else {
            return Err(Error::msg(format!(
                "duplicate binding for `{name}` cannot be merged (expected case body)"
            )));
        };

        merge_rewrite_tuple_scrutinee_arms_to_cons(&b_scrut, &mut b_arms);

        let b_scrut = merge_substitute_vars_in_expr(&b_scrut, b_params, p_params);
        if p_scrut.kind != b_scrut.kind {
            // Special-case: we sometimes collapse a multi-arg tuple-scrutinee clause
            // into unary form; treat its scrutinee as the single param.
            if let ast::ExprKind::Var(v) = &p_scrut.kind {
                if v != &p_params[0] {
                    return Err(Error::msg(format!(
                        "duplicate binding for `{name}` cannot be merged (scrutinee mismatch)"
                    )));
                }
            } else {
                return Err(Error::msg(format!(
                    "duplicate binding for `{name}` cannot be merged (scrutinee mismatch)"
                )));
            }
        }

        p_arms.extend(b_arms);
        prev.expr = merge_rebuild_lambda_case(prev.expr.span, p_params.clone(), p_scrut, p_arms);
        return Ok(true);
    }

    let Some((p_scrut, mut p_arms)) = merge_unwrap_case_body((**p_body).clone()) else {
        return Err(Error::msg(format!(
            "duplicate binding for `{name}` cannot be merged (expected case body)"
        )));
    };
    let Some((b_scrut, b_arms)) = merge_unwrap_case_body((**b_body).clone()) else {
        return Err(Error::msg(format!(
            "duplicate binding for `{name}` cannot be merged (expected case body)"
        )));
    };

    // Scrutinees should be equivalent up to alpha-renaming of the params.
    let b_scrut = merge_substitute_vars_in_expr(&b_scrut, b_params, p_params);
    if p_scrut.kind != b_scrut.kind {
        return Err(Error::msg(format!(
            "duplicate binding for `{name}` cannot be merged (scrutinee mismatch)"
        )));
    }

    p_arms.extend(b_arms);
    prev.expr = merge_rebuild_lambda_case(prev.expr.span, p_params.clone(), p_scrut, p_arms);
    Ok(true)
}

fn merge_duplicate_bindings_for_names(items: Vec<ast::Item>, names: &[&str]) -> Result<Vec<ast::Item>> {
    use ast::Item;

    let target: HashSet<String> = names.iter().map(|n| n.to_string()).collect();

    // Preserve overall item order while merging only duplicate name bindings.
    let mut out: Vec<Item> = Vec::with_capacity(items.len());

    // name -> index in `out` of the binding we are accumulating into.
    let mut seen: HashMap<String, usize> = HashMap::new();

    for it in items {
        let Item::Binding(b) = it else {
            out.push(it);
            continue;
        };

        let Some(name) = merge_binding_name(&b) else {
            out.push(Item::Binding(b));
            continue;
        };

        // Only merge the configured targets (helps avoid regressions).
        // Here `name` is already qualified like `L.null`.
        let suffix = name.rsplit('.').next().unwrap_or(name);
        if !target.contains(suffix) {
            out.push(Item::Binding(b));
            continue;
        }

        let name = name.to_string();
        if let Some(&idx) = seen.get(&name) {
            let Item::Binding(prev) = &mut out[idx] else {
                // Should be impossible.
                out.push(Item::Binding(b));
                continue;
            };

            #[cfg(test)]
            {
                if std::env::var("KSCR_DEBUG_IMPORTS").ok().as_deref() == Some("1")
                    && name == "L.null"
                {
                    eprintln!("[KSCR_DEBUG_IMPORTS] merge candidate {name}:");
                    eprintln!("  prev.pat = {:?}", prev.pat.kind);
                    eprintln!("  prev.expr = {:?}", prev.expr.kind);
                    eprintln!("  new .pat = {:?}", b.pat.kind);
                    eprintln!("  new .expr = {:?}", b.expr.kind);
                }
            }

            let _merged = merge_try_merge_lambda_clauses(prev, &b, &name)?;
        } else {
            let idx = out.len();
            out.push(Item::Binding(b));
            seen.insert(name, idx);
        }
    }

    Ok(out)
}

fn import_unqualified_forwarders(
    module: &ast::Module,
    qual: &str,
    exports: &HashSet<String>,
) -> Result<Vec<ast::Item>> {
    // Bring unqualified exports as simple forwarders: `x = QUAL.x`.
    let mut out = Vec::new();

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
            ast::Item::Import(_)
            | ast::Item::Export(_)
            | ast::Item::Fixity(_)
            | ast::Item::ClassDecl(_)
            | ast::Item::InstanceDecl(_) => {}
        }
    }

    for n in exports.iter() {
        if values.contains(n) {
            out.push(ast::Item::Binding(ast::Binding {
                pat: ast::Pattern::dummy(ast::PatternKind::Var(n.clone())),
                expr: ast::Expr::dummy(ast::ExprKind::Var(format!("{qual}.{n}"))),
            }));
        }

        if let Some(ta) = type_aliases.get(n) {
            let head = ast::Type::Var(format!("{qual}.{}", ta.name));
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

fn qualify_class_instance_decls(
    module: &ast::Module,
    qual: &str,
    exports: &HashSet<String>,
) -> Result<Vec<ast::Item>> {
    let mut types: HashSet<String> = HashSet::new();
    for it in import_items(module) {
        match it {
            ast::Item::TypeAlias(ta) => {
                types.insert(ta.name);
            }
            ast::Item::DataDecl(d) => {
                types.insert(d.name);
            }
            _ => {}
        }
    }

    let priv_qual = format!("{qual}$p");
    let type_map: HashMap<String, String> = types
        .into_iter()
        .map(|n| {
            let q = if exports.contains(&n) {
                qual
            } else {
                &priv_qual
            };
            (n.clone(), format!("{q}.{n}"))
        })
        .collect();

    module
        .items
        .iter()
        .filter(|it| matches!(it, ast::Item::ClassDecl(_) | ast::Item::InstanceDecl(_)))
        .cloned()
        .map(|it| {
            Ok(match it {
                ast::Item::ClassDecl(mut c) => {
                    c.supers = c
                        .supers
                        .into_iter()
                        .map(|p| qualify_predicate(p, &type_map))
                        .collect::<Result<Vec<_>>>()?;
                    for m in &mut c.methods {
                        m.ty = qualify_qual_type(m.ty.clone(), &type_map)?;
                    }
                    ast::Item::ClassDecl(c)
                }
                ast::Item::InstanceDecl(mut inst) => {
                    inst.ty = qualify_type(inst.ty, &type_map)?;
                    ast::Item::InstanceDecl(inst)
                }
                other => other,
            })
        })
        .collect::<Result<Vec<_>>>()
}

fn qualify_items(
    module: &ast::Module,
    qual: &str,
    exports: &HashSet<String>,
) -> Result<Vec<ast::Item>> {
    // Collect definers once to avoid accidentally duplicating a name when the same
    // underlying item appears twice (e.g., through prior transformations).
    let mut values = HashSet::new();
    let mut types = HashSet::new();
    let mut ctors = HashSet::new();

    let mut defined_names: HashSet<String> = HashSet::new();
    for it in import_items(module) {
        item_defined_names(&it, &mut defined_names);
        match &it {
            ast::Item::Binding(b) => pat_defined_names(&b.pat, &mut values),
            ast::Item::TypeAlias(ta) => {
                types.insert(ta.name.clone());
            }
            ast::Item::DataDecl(d) => {
                types.insert(d.name.clone());
                ctors.extend(d.ctors.iter().map(|c| c.name.clone()));
            }
            ast::Item::Import(_)
            | ast::Item::Export(_)
            | ast::Item::Fixity(_)
            | ast::Item::ClassDecl(_)
            | ast::Item::InstanceDecl(_) => {}
        }
    }

    let priv_qual = format!("{qual}$p");

    // Build maps only for names that are actually exported OR needed as private qualified
    // helpers, but never duplicate keys.
    let val_map: HashMap<String, String> = values
        .into_iter()
        .filter(|n| defined_names.contains(n))
        .map(|n| {
            let q = if exports.contains(&n) {
                qual
            } else {
                &priv_qual
            };
            (n.clone(), format!("{q}.{n}"))
        })
        .collect();
    let type_map: HashMap<String, String> = types
        .into_iter()
        .filter(|n| defined_names.contains(n))
        .map(|n| {
            let q = if exports.contains(&n) {
                qual
            } else {
                &priv_qual
            };
            (n.clone(), format!("{q}.{n}"))
        })
        .collect();
    let ctor_map: HashMap<String, String> = ctors
        .into_iter()
        .filter(|n| defined_names.contains(n))
        .map(|n| {
            let q = if exports.contains(&n) {
                qual
            } else {
                &priv_qual
            };
            (n.clone(), format!("{q}.{n}"))
        })
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
                ctor.name = ctor_map
                    .get(&ctor.name)
                    .cloned()
                    .unwrap_or(ctor.name.clone());
                ctor.args = ctor
                    .args
                    .clone()
                    .into_iter()
                    .map(|t| qualify_type(t, type_map))
                    .collect::<Result<Vec<_>>>()?;
            }
            ast::Item::DataDecl(d)
        }
        x @ (ast::Item::Import(_)
        | ast::Item::Export(_)
        | ast::Item::Fixity(_)
        | ast::Item::ClassDecl(_)
        | ast::Item::InstanceDecl(_)) => x,
    })
}

fn qualify_expr_boxed(
    expr: &ast::Expr,
    val_map: &HashMap<String, String>,
    type_map: &HashMap<String, String>,
    ctor_map: &HashMap<String, String>,
) -> Result<Box<ast::Expr>> {
    Ok(Box::new(qualify_expr(expr.clone(), val_map, type_map, ctor_map)?))
}

fn qualify_expr_vec(
    exprs: Vec<ast::Expr>,
    val_map: &HashMap<String, String>,
    type_map: &HashMap<String, String>,
    ctor_map: &HashMap<String, String>,
) -> Result<Vec<ast::Expr>> {
    exprs
        .into_iter()
        .map(|e| qualify_expr(e, val_map, type_map, ctor_map))
        .collect()
}

fn qualify_local_bindings(
    bindings: Vec<ast::Binding>,
    val_map: &HashMap<String, String>,
    type_map: &HashMap<String, String>,
    ctor_map: &HashMap<String, String>,
) -> Result<Vec<ast::Binding>> {
    bindings
        .into_iter()
        .map(|b| qualify_local_binding(b, val_map, type_map, ctor_map))
        .collect()
}

fn qualify_record_fields(
    fs: Vec<(String, ast::Expr)>,
    val_map: &HashMap<String, String>,
    type_map: &HashMap<String, String>,
    ctor_map: &HashMap<String, String>,
) -> Result<Vec<(String, ast::Expr)>> {
    fs.into_iter()
        .map(|(l, e)| Ok((l, qualify_expr(e, val_map, type_map, ctor_map)?)))
        .collect()
}

fn qualify_expr(
    expr: ast::Expr,
    val_map: &HashMap<String, String>,
    type_map: &HashMap<String, String>,
    ctor_map: &HashMap<String, String>,
) -> Result<ast::Expr> {
    use ast::{Expr, ExprKind};
    let span = expr.span;
    Ok(match expr.kind {
        ExprKind::Var(n) => Expr::new(span, ExprKind::Var(val_map.get(&n).cloned().unwrap_or(n))),
        ExprKind::Ctor(n) => {
            Expr::new(span, ExprKind::Ctor(ctor_map.get(&n).cloned().unwrap_or(n)))
        }
        ExprKind::Lambda { params, body } => Expr::new(
            span,
            ExprKind::Lambda {
                params,
                body: qualify_expr_boxed(&body, val_map, type_map, ctor_map)?,
            },
        ),
        ExprKind::Apply { func, args } => Expr::new(
            span,
            ExprKind::Apply {
                func: qualify_expr_boxed(&func, val_map, type_map, ctor_map)?,
                args: qualify_expr_vec(args, val_map, type_map, ctor_map)?,
            },
        ),
        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => Expr::new(
            span,
            ExprKind::If {
                cond: qualify_expr_boxed(&cond, val_map, type_map, ctor_map)?,
                then_branch: qualify_expr_boxed(&then_branch, val_map, type_map, ctor_map)?,
                else_branch: qualify_expr_boxed(&else_branch, val_map, type_map, ctor_map)?,
            },
        ),
        ExprKind::Let { bindings, body } => Expr::new(
            span,
            ExprKind::Let {
                bindings: qualify_local_bindings(bindings, val_map, type_map, ctor_map)?,
                body: qualify_expr_boxed(&body, val_map, type_map, ctor_map)?,
            },
        ),
        ExprKind::Where { expr, bindings } => Expr::new(
            span,
            ExprKind::Where {
                expr: qualify_expr_boxed(&expr, val_map, type_map, ctor_map)?,
                bindings: qualify_local_bindings(bindings, val_map, type_map, ctor_map)?,
            },
        ),
        ExprKind::Annot { expr, ty } => Expr::new(
            span,
            ExprKind::Annot {
                expr: qualify_expr_boxed(&expr, val_map, type_map, ctor_map)?,
                ty: qualify_qual_type(ty, type_map)?,
            },
        ),
        ExprKind::Do(stmts) => Expr::new(
            span,
            ExprKind::Do(
                stmts
                    .into_iter()
                    .map(|s| qualify_do_stmt(s, val_map, type_map, ctor_map))
                    .collect::<Result<Vec<_>>>()?,
            ),
        ),
        ExprKind::Case { expr, arms } => Expr::new(
            span,
            ExprKind::Case {
                expr: qualify_expr_boxed(&expr, val_map, type_map, ctor_map)?,
                arms: arms
                    .into_iter()
                    .map(|a| qualify_case_arm(a, val_map, type_map, ctor_map))
                    .collect::<Result<Vec<_>>>()?,
            },
        ),
        ExprKind::Cons { head, tail } => Expr::new(
            span,
            ExprKind::Cons {
                head: qualify_expr_boxed(&head, val_map, type_map, ctor_map)?,
                tail: qualify_expr_boxed(&tail, val_map, type_map, ctor_map)?,
            },
        ),
        ExprKind::List(es) => Expr::new(span, ExprKind::List(qualify_expr_vec(es, val_map, type_map, ctor_map)?)),
        ExprKind::Tuple(es) => Expr::new(span, ExprKind::Tuple(qualify_expr_vec(es, val_map, type_map, ctor_map)?)),
        ExprKind::Record(fs) => Expr::new(
            span,
            ExprKind::Record(qualify_record_fields(fs, val_map, type_map, ctor_map)?),
        ),
        other => Expr::new(span, other),
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
    use ast::{Pattern, PatternKind};
    let span = p.span;
    Ok(match p.kind {
        PatternKind::Var(n) => Pattern::new(
            span,
            PatternKind::Var(val_map.get(&n).cloned().unwrap_or(n)),
        ),
        PatternKind::As(n, p) => Pattern::new(
            span,
            PatternKind::As(
                val_map.get(&n).cloned().unwrap_or(n),
                Box::new(qualify_pat_binders(*p, val_map)?),
            ),
        ),
        PatternKind::Tuple(ps) => Pattern::new(
            span,
            PatternKind::Tuple(
                ps.into_iter()
                    .map(|p| qualify_pat_binders(p, val_map))
                    .collect::<Result<Vec<_>>>()?,
            ),
        ),
        PatternKind::List(ps) => Pattern::new(
            span,
            PatternKind::List(
                ps.into_iter()
                    .map(|p| qualify_pat_binders(p, val_map))
                    .collect::<Result<Vec<_>>>()?,
            ),
        ),
        PatternKind::Record(fs) => Pattern::new(
            span,
            PatternKind::Record(
                fs.into_iter()
                    .map(|(l, p)| Ok((l, qualify_pat_binders(p, val_map)?)))
                    .collect::<Result<Vec<_>>>()?,
            ),
        ),
        PatternKind::RecordLoose(fs, rest) => Pattern::new(
            span,
            PatternKind::RecordLoose(
                fs.into_iter()
                    .map(|(l, p)| Ok((l, qualify_pat_binders(p, val_map)?)))
                    .collect::<Result<Vec<_>>>()?,
                rest.map(|n| val_map.get(&n).cloned().unwrap_or(n)),
            ),
        ),
        PatternKind::Cons(a, b) => Pattern::new(
            span,
            PatternKind::Cons(
                Box::new(qualify_pat_binders(*a, val_map)?),
                Box::new(qualify_pat_binders(*b, val_map)?),
            ),
        ),
        PatternKind::Or(a, b) => Pattern::new(
            span,
            PatternKind::Or(
                Box::new(qualify_pat_binders(*a, val_map)?),
                Box::new(qualify_pat_binders(*b, val_map)?),
            ),
        ),
        PatternKind::View(p, e) => Pattern::new(
            span,
            PatternKind::View(Box::new(qualify_pat_binders(*p, val_map)?), e),
        ),
        PatternKind::Constructor { name, args } => {
            Pattern::new(span, PatternKind::Constructor { name, args })
        }
        other => Pattern::new(span, other),
    })
}

fn qualify_pat_nonbinders(
    p: ast::Pattern,
    ctor_map: &HashMap<String, String>,
    val_map: &HashMap<String, String>,
    type_map: &HashMap<String, String>,
) -> Result<ast::Pattern> {
    let _ = type_map;
    use ast::{Pattern, PatternKind};
    let span = p.span;
    Ok(match p.kind {
        PatternKind::Tuple(ps) => Pattern::new(
            span,
            PatternKind::Tuple(
                ps.into_iter()
                    .map(|p| qualify_pat_nonbinders(p, ctor_map, val_map, type_map))
                    .collect::<Result<Vec<_>>>()?,
            ),
        ),
        PatternKind::List(ps) => Pattern::new(
            span,
            PatternKind::List(
                ps.into_iter()
                    .map(|p| qualify_pat_nonbinders(p, ctor_map, val_map, type_map))
                    .collect::<Result<Vec<_>>>()?,
            ),
        ),
        PatternKind::Record(fs) => Pattern::new(
            span,
            PatternKind::Record(
                fs.into_iter()
                    .map(|(l, p)| Ok((l, qualify_pat_nonbinders(p, ctor_map, val_map, type_map)?)))
                    .collect::<Result<Vec<_>>>()?,
            ),
        ),
        PatternKind::RecordLoose(fs, rest) => Pattern::new(
            span,
            PatternKind::RecordLoose(
                fs.into_iter()
                    .map(|(l, p)| Ok((l, qualify_pat_nonbinders(p, ctor_map, val_map, type_map)?)))
                    .collect::<Result<Vec<_>>>()?,
                rest,
            ),
        ),
        PatternKind::Cons(a, b) => Pattern::new(
            span,
            PatternKind::Cons(
                Box::new(qualify_pat_nonbinders(*a, ctor_map, val_map, type_map)?),
                Box::new(qualify_pat_nonbinders(*b, ctor_map, val_map, type_map)?),
            ),
        ),
        PatternKind::Or(a, b) => Pattern::new(
            span,
            PatternKind::Or(
                Box::new(qualify_pat_nonbinders(*a, ctor_map, val_map, type_map)?),
                Box::new(qualify_pat_nonbinders(*b, ctor_map, val_map, type_map)?),
            ),
        ),
        PatternKind::As(n, p) => Pattern::new(
            span,
            PatternKind::As(
                n,
                Box::new(qualify_pat_nonbinders(*p, ctor_map, val_map, type_map)?),
            ),
        ),
        PatternKind::View(p, e) => Pattern::new(
            span,
            PatternKind::View(
                Box::new(qualify_pat_nonbinders(*p, ctor_map, val_map, type_map)?),
                Box::new(qualify_expr(*e, val_map, type_map, ctor_map)?),
            ),
        ),
        PatternKind::Constructor { name, args } => Pattern::new(
            span,
            PatternKind::Constructor {
                name: ctor_map.get(&name).cloned().unwrap_or(name),
                args: args
                    .into_iter()
                    .map(|p| qualify_pat_nonbinders(p, ctor_map, val_map, type_map))
                    .collect::<Result<Vec<_>>>()?,
            },
        ),
        PatternKind::Literal(e) => Pattern::new(
            span,
            PatternKind::Literal(qualify_expr(e, val_map, type_map, ctor_map)?),
        ),
        other => Pattern::new(span, other),
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

fn qualify_predicate(
    p: ast::Predicate,
    type_map: &HashMap<String, String>,
) -> Result<ast::Predicate> {
    Ok(match p {
        ast::Predicate::Show(t) => ast::Predicate::Show(qualify_type(t, type_map)?),
        ast::Predicate::ShowRow(t) => ast::Predicate::ShowRow(qualify_type(t, type_map)?),
        ast::Predicate::Eq(t) => ast::Predicate::Eq(qualify_type(t, type_map)?),
        ast::Predicate::EqRow(t) => ast::Predicate::EqRow(qualify_type(t, type_map)?),
        ast::Predicate::Class { class, ty } => ast::Predicate::Class {
            class,
            ty: qualify_type(ty, type_map)?,
        },
        ast::Predicate::Lacks { label, row } => ast::Predicate::Lacks {
            label,
            row: qualify_type(row, type_map)?,
        },
    })
}

fn qualify_qual_type(
    qt: ast::QualType,
    type_map: &HashMap<String, String>,
) -> Result<ast::QualType> {
    Ok(ast::QualType {
        preds: qt
            .preds
            .into_iter()
            .map(|p| qualify_predicate(p, type_map))
            .collect::<Result<Vec<_>>>()?,
        ty: qualify_type(qt.ty, type_map)?,
    })
}

fn name_origin_hint(it: &ast::Item, name: &str) -> String {
    fn qual_of(s: &str) -> Option<String> {
        s.split_once('.').map(|(q, _)| q.to_string())
    }
    fn qual_of_type(ty: &ast::Type) -> Option<String> {
        match ty {
            ast::Type::Var(n) => qual_of(n),
            ast::Type::App { head, .. } => qual_of_type(head),
            _ => None,
        }
    }

    match it {
        ast::Item::Binding(b) => {
            if let ast::PatternKind::Var(n) = &b.pat.kind {
                if n == name {
                    if let ast::ExprKind::Var(v) = &b.expr.kind {
                        if v.ends_with(&format!(".{name}")) {
                            if let Some(q) = qual_of(v) {
                                return format!("import {q}");
                            }
                        }
                    }
                }
            }
            "local".to_string()
        }
        ast::Item::TypeAlias(ta) => {
            if let Some(q) = qual_of(&ta.name) {
                return format!("import {q}");
            }
            if let Some(q) = qual_of_type(&ta.ty) {
                return format!("import {q}");
            }
            "local".to_string()
        }
        ast::Item::DataDecl(d) => {
            if let Some(q) = qual_of(&d.name) {
                return format!("import {q}");
            }
            for c in &d.ctors {
                if let Some(q) = qual_of(&c.name) {
                    return format!("import {q}");
                }
            }
            "local".to_string()
        }
        ast::Item::Import(_)
        | ast::Item::Export(_)
        | ast::Item::Fixity(_)
        | ast::Item::ClassDecl(_)
        | ast::Item::InstanceDecl(_) => "<meta>".to_string(),
    }
}

fn push_item_checked(
    items: &mut Vec<ast::Item>,
    defined: &mut HashMap<String, String>,
    it: ast::Item,
) -> Result<()> {
    let mut names = HashSet::new();
    item_defined_names(&it, &mut names);
    for n in names {
        let origin = name_origin_hint(&it, &n);
        if let Some(prev) = defined.get(&n) {
            return Err(Error::msg(format!(
                "name conflict: {n} (previously from {prev}, now from {origin}); try `import ... as ...` or qualify",
            )));
        }
        defined.insert(n, origin);
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
        ast::Item::Import(_)
        | ast::Item::Export(_)
        | ast::Item::Fixity(_)
        | ast::Item::ClassDecl(_)
        | ast::Item::InstanceDecl(_) => {}
    }
}

fn pat_defined_names(p: &ast::Pattern, out: &mut HashSet<String>) {
    use ast::PatternKind;
    match &p.kind {
        PatternKind::Var(n) => {
            out.insert(n.clone());
        }
        PatternKind::As(n, p) => {
            out.insert(n.clone());
            pat_defined_names(p, out);
        }
        PatternKind::Tuple(ps) | PatternKind::List(ps) => {
            for p in ps {
                pat_defined_names(p, out);
            }
        }
        PatternKind::Record(fs) | PatternKind::RecordLoose(fs, _) => {
            for (_, p) in fs {
                pat_defined_names(p, out);
            }
            if let PatternKind::RecordLoose(_, Some(rest)) = &p.kind {
                out.insert(rest.clone());
            }
        }
        PatternKind::Cons(a, b) | PatternKind::Or(a, b) => {
            pat_defined_names(a, out);
            pat_defined_names(b, out);
        }
        PatternKind::View(p, _) => pat_defined_names(p, out),
        PatternKind::Constructor { args, .. } => {
            for p in args {
                pat_defined_names(p, out);
            }
        }
        PatternKind::Wildcard | PatternKind::Hole(_) | PatternKind::Literal(_) => {}
    }
}

fn rewrite_show_calls_in_binding(b: ast::Binding) -> ast::Binding {
    ast::Binding {
        pat: b.pat,
        expr: rewrite_show_calls_in_expr(b.expr),
    }
}

fn rewrite_show_apply_builtin_show(span: ast::Span, args: &mut Vec<ast::Expr>) -> Option<ast::Expr> {
    use ast::ExprKind;
    if args.len() != 1 {
        return None;
    }

    let a0 = args.remove(0);
    Some(ast::Expr::new(
        span,
        ExprKind::Apply {
            func: Box::new(ast::Expr::new(span, ExprKind::Var("__show".to_string()))),
            args: vec![
                ast::Expr::new(span, ExprKind::Var("__builtinShowDict".to_string())),
                a0,
            ],
        },
    ))
}

fn rewrite_show_apply_builtin_to_string(span: ast::Span, args: &mut Vec<ast::Expr>) -> Option<ast::Expr> {
    use ast::ExprKind;
    if args.len() != 1 {
        return None;
    }

    let a0 = args.remove(0);
    Some(ast::Expr::new(
        span,
        ExprKind::Apply {
            func: Box::new(ast::Expr::new(span, ExprKind::Var("__toString".to_string()))),
            args: vec![
                ast::Expr::new(span, ExprKind::Var("__builtinShowDict".to_string())),
                a0,
            ],
        },
    ))
}

fn rewrite_show_apply_builtin_eq(span: ast::Span, args: &mut Vec<ast::Expr>) -> Option<ast::Expr> {
    use ast::ExprKind;
    match args.len() {
        1 => {
            let a0 = args.remove(0);
            Some(ast::Expr::new(
                span,
                ExprKind::Apply {
                    func: Box::new(ast::Expr::new(span, ExprKind::Var("__eq".to_string()))),
                    args: vec![
                        ast::Expr::new(span, ExprKind::Var("__builtinEqDict".to_string())),
                        a0,
                    ],
                },
            ))
        }
        2 => {
            let a = args.remove(0);
            let b = args.remove(0);
            Some(ast::Expr::new(
                span,
                ExprKind::Apply {
                    func: Box::new(ast::Expr::new(span, ExprKind::Var("__eq".to_string()))),
                    args: vec![
                        ast::Expr::new(span, ExprKind::Var("__builtinEqDict".to_string())),
                        a,
                        b,
                    ],
                },
            ))
        }
        _ => None,
    }
}

fn rewrite_show_apply_builtin_neq(span: ast::Span, args: &mut Vec<ast::Expr>) -> Option<ast::Expr> {
    use ast::ExprKind;
    match args.len() {
        1 => {
            // Section: `(/= a)` becomes `\b -> not (__eq __builtinEqDict a b)`.
            let a = args.remove(0);
            let bname = "__kscr_neq_rhs".to_string();
            Some(ast::Expr::new(
                span,
                ExprKind::Lambda {
                    params: vec![bname.clone()],
                    body: Box::new(ast::Expr::new(
                        span,
                        ExprKind::Apply {
                            func: Box::new(ast::Expr::new(span, ExprKind::Var("not".to_string()))),
                            args: vec![ast::Expr::new(
                                span,
                                ExprKind::Apply {
                                    func: Box::new(ast::Expr::new(span, ExprKind::Var("__eq".to_string()))),
                                    args: vec![
                                        ast::Expr::new(span, ExprKind::Var("__builtinEqDict".to_string())),
                                        a,
                                        ast::Expr::new(span, ExprKind::Var(bname)),
                                    ],
                                },
                            )],
                        },
                    )),
                },
            ))
        }
        2 => {
            let a = args.remove(0);
            let b = args.remove(0);
            Some(ast::Expr::new(
                span,
                ExprKind::Apply {
                    func: Box::new(ast::Expr::new(span, ExprKind::Var("not".to_string()))),
                    args: vec![ast::Expr::new(
                        span,
                        ExprKind::Apply {
                            func: Box::new(ast::Expr::new(span, ExprKind::Var("__eq".to_string()))),
                            args: vec![
                                ast::Expr::new(span, ExprKind::Var("__builtinEqDict".to_string())),
                                a,
                                b,
                            ],
                        },
                    )],
                },
            ))
        }
        _ => None,
    }
}

fn rewrite_show_apply_special(span: ast::Span, func: &ast::Expr, mut args: Vec<ast::Expr>) -> Option<ast::Expr> {
    use ast::ExprKind;
    let ExprKind::Var(n) = &func.kind else {
        return None;
    };

    match n.as_str() {
        "show" => rewrite_show_apply_builtin_show(span, &mut args),
        "toString" => rewrite_show_apply_builtin_to_string(span, &mut args),
        "==" => rewrite_show_apply_builtin_eq(span, &mut args),
        "/=" => rewrite_show_apply_builtin_neq(span, &mut args),
        _ => None,
    }
}

fn rewrite_show_calls_in_apply(span: ast::Span, func: ast::Expr, args: Vec<ast::Expr>) -> ast::Expr {
    use ast::{Expr, ExprKind};

    let func = rewrite_show_calls_in_expr(func);
    let args: Vec<_> = args.into_iter().map(rewrite_show_calls_in_expr).collect();

    if let Some(rewritten) = rewrite_show_apply_special(span, &func, args.clone()) {
        return rewritten;
    }

    Expr::new(
        span,
        ExprKind::Apply {
            func: Box::new(func),
            args,
        },
    )
}

fn rewrite_show_calls_in_lambda(span: ast::Span, params: Vec<String>, body: ast::Expr) -> ast::Expr {
    use ast::{Expr, ExprKind};
    Expr::new(
        span,
        ExprKind::Lambda {
            params,
            body: Box::new(rewrite_show_calls_in_expr(body)),
        },
    )
}

fn rewrite_show_calls_in_if(
    span: ast::Span,
    cond: ast::Expr,
    then_branch: ast::Expr,
    else_branch: ast::Expr,
) -> ast::Expr {
    use ast::{Expr, ExprKind};
    Expr::new(
        span,
        ExprKind::If {
            cond: Box::new(rewrite_show_calls_in_expr(cond)),
            then_branch: Box::new(rewrite_show_calls_in_expr(then_branch)),
            else_branch: Box::new(rewrite_show_calls_in_expr(else_branch)),
        },
    )
}

fn rewrite_show_calls_in_let(span: ast::Span, bindings: Vec<ast::Binding>, body: ast::Expr) -> ast::Expr {
    use ast::{Expr, ExprKind};
    Expr::new(
        span,
        ExprKind::Let {
            bindings: bindings
                .into_iter()
                .map(rewrite_show_calls_in_binding)
                .collect(),
            body: Box::new(rewrite_show_calls_in_expr(body)),
        },
    )
}

fn rewrite_show_calls_in_where(
    span: ast::Span,
    expr: ast::Expr,
    bindings: Vec<ast::Binding>,
) -> ast::Expr {
    use ast::{Expr, ExprKind};
    Expr::new(
        span,
        ExprKind::Where {
            expr: Box::new(rewrite_show_calls_in_expr(expr)),
            bindings: bindings
                .into_iter()
                .map(rewrite_show_calls_in_binding)
                .collect(),
        },
    )
}

fn rewrite_show_calls_in_annot(span: ast::Span, expr: ast::Expr, ty: ast::QualType) -> ast::Expr {
    use ast::{Expr, ExprKind};
    Expr::new(
        span,
        ExprKind::Annot {
            expr: Box::new(rewrite_show_calls_in_expr(expr)),
            ty,
        },
    )
}

fn rewrite_show_calls_in_do(span: ast::Span, stmts: Vec<ast::DoStmt>) -> ast::Expr {
    use ast::{Expr, ExprKind};
    Expr::new(
        span,
        ExprKind::Do(
            stmts
                .into_iter()
                .map(|s| match s {
                    ast::DoStmt::Bind { pat, expr } => ast::DoStmt::Bind {
                        pat,
                        expr: rewrite_show_calls_in_expr(expr),
                    },
                    ast::DoStmt::Expr(e) => ast::DoStmt::Expr(rewrite_show_calls_in_expr(e)),
                })
                .collect(),
        ),
    )
}

fn rewrite_show_calls_in_case(span: ast::Span, expr: ast::Expr, arms: Vec<ast::CaseArm>) -> ast::Expr {
    use ast::{Expr, ExprKind};
    Expr::new(
        span,
        ExprKind::Case {
            expr: Box::new(rewrite_show_calls_in_expr(expr)),
            arms: arms
                .into_iter()
                .map(|a| ast::CaseArm {
                    pat: a.pat,
                    guard: a.guard.map(rewrite_show_calls_in_expr),
                    body: rewrite_show_calls_in_expr(a.body),
                })
                .collect(),
        },
    )
}

fn rewrite_show_calls_in_cons(span: ast::Span, head: ast::Expr, tail: ast::Expr) -> ast::Expr {
    use ast::{Expr, ExprKind};
    Expr::new(
        span,
        ExprKind::Cons {
            head: Box::new(rewrite_show_calls_in_expr(head)),
            tail: Box::new(rewrite_show_calls_in_expr(tail)),
        },
    )
}

fn rewrite_show_calls_in_list(span: ast::Span, es: Vec<ast::Expr>) -> ast::Expr {
    use ast::{Expr, ExprKind};
    Expr::new(
        span,
        ExprKind::List(es.into_iter().map(rewrite_show_calls_in_expr).collect()),
    )
}

fn rewrite_show_calls_in_tuple(span: ast::Span, es: Vec<ast::Expr>) -> ast::Expr {
    use ast::{Expr, ExprKind};
    Expr::new(
        span,
        ExprKind::Tuple(es.into_iter().map(rewrite_show_calls_in_expr).collect()),
    )
}

fn rewrite_show_calls_in_record(span: ast::Span, fs: Vec<(String, ast::Expr)>) -> ast::Expr {
    use ast::{Expr, ExprKind};
    Expr::new(
        span,
        ExprKind::Record(
            fs.into_iter()
                .map(|(l, e)| (l, rewrite_show_calls_in_expr(e)))
                .collect(),
        ),
    )
}

fn rewrite_show_calls_in_expr(expr: ast::Expr) -> ast::Expr {
    use ast::{Expr, ExprKind};
    let span = expr.span;
    match expr.kind {
        ExprKind::Lambda { params, body } => rewrite_show_calls_in_lambda(span, params, *body),
        ExprKind::Apply { func, args } => rewrite_show_calls_in_apply(span, *func, args),
        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => rewrite_show_calls_in_if(span, *cond, *then_branch, *else_branch),
        ExprKind::Let { bindings, body } => rewrite_show_calls_in_let(span, bindings, *body),
        ExprKind::Where { expr, bindings } => rewrite_show_calls_in_where(span, *expr, bindings),
        ExprKind::Annot { expr, ty } => rewrite_show_calls_in_annot(span, *expr, ty),
        ExprKind::Do(stmts) => rewrite_show_calls_in_do(span, stmts),
        ExprKind::Case { expr, arms } => rewrite_show_calls_in_case(span, *expr, arms),
        ExprKind::Cons { head, tail } => rewrite_show_calls_in_cons(span, *head, *tail),
        ExprKind::List(es) => rewrite_show_calls_in_list(span, es),
        ExprKind::Tuple(es) => rewrite_show_calls_in_tuple(span, es),
        ExprKind::Record(fs) => rewrite_show_calls_in_record(span, fs),
        other => Expr::new(span, other),
    }
}

fn rewrite_show_calls_in_module(module: &mut ast::Module) {
    module.items = module
        .items
        .drain(..)
        .map(|it| match it {
            ast::Item::Binding(b) => ast::Item::Binding(rewrite_show_calls_in_binding(b)),
            other => other,
        })
        .collect();
}

fn infer_in_module_with_class_env(
    module: &ast::Module,
    class_env: &ClassEnv,
    inferred: &HashMap<String, Scheme>,
    expr: ast::Expr,
) -> Result<Ty> {
    let mut cx = InferCtx::default();
    let data_env = collect_data_env(module);
    let mut env = collect_ctor_env_with_class_env(&mut cx, module, class_env)?;
    // Add inferred binding types (module + imported forwarders). This is important for
    // inferring argument types during later desugaring passes.
    for (name, scheme) in inferred {
        if !env.contains_key(name) {
            env.insert(name.clone(), scheme.clone());
        }
    }
    let (s, cs, t) = infer_expr_in(&mut cx, &data_env, &env, expr)?;
    let _ = simplify_constraints(&data_env, class_env, apply_constraints(&s, cs))?;
    Ok(apply(&s, t))
}

// moved to `src/types/typeclass_dict_passing.rs`

fn rewrite_class_method_calls_in_module(
    module: &mut ast::Module,
    class_env: &ClassEnv,
    inferred: &HashMap<String, Scheme>,
) -> Result<()> {
    fn instance_head_key_ty_for_class(class: &str, ty: &Ty) -> Result<String> {
        // MVP: higher-kinded classes select instances by the type constructor head.
        // e.g. `Functor` instance is declared for `Maybe`, but call sites see `Maybe a`.
        if class == "Monad" || class == "Applicative" || class == "Functor" {
            return Ok(match ty {
                Ty::Con(name) => name.clone(),
                Ty::App { head, .. } => {
                    match head.as_ref() {
                        Ty::Con(name) => name.clone(),
                        _ => return Err(Error::msg(
                            "MVP: class constraints support only constructor/app instance heads",
                        )),
                    }
                }
                _ => {
                    return Err(Error::msg(
                        "MVP: class constraints support only constructor/app instance heads",
                    ))
                }
            });
        }
        instance_head_key_ty(ty)
    }

    fn rewrite_expr(
        module_snapshot: &ast::Module,
        class_env: &ClassEnv,
        inferred: &HashMap<String, Scheme>,
        dicts_in_scope: &HashSet<String>,
        known_dicts_in_scope: &HashMap<String, String>,
        expr: ast::Expr,
    ) -> Result<ast::Expr> {
        use ast::{Expr, ExprKind};

        fn super_field_name(class: &str) -> String {
            format!("__super_{}", mangle_ident(class))
        }

        fn find_super_path(class_env: &ClassEnv, from: &str, to: &str) -> Option<Vec<String>> {
            use std::collections::{HashMap, VecDeque};

            if from == to {
                return None;
            }

            let mut q: VecDeque<String> = VecDeque::new();
            let mut prev: HashMap<String, String> = HashMap::new();
            q.push_back(from.to_string());
            prev.insert(from.to_string(), "".to_string());

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
                    if sup == to {
                        let mut path: Vec<String> = Vec::new();
                        let mut cur = to.to_string();
                        while cur != from {
                            path.push(cur.clone());
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

        fn project_dict_along_path(
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

        fn derive_dict_expr_from_candidates(
            span: ast::Span,
            class_env: &ClassEnv,
            wanted_class: &str,
            dicts_in_scope: &HashSet<String>,
            known_dicts_in_scope: &HashMap<String, String>,
        ) -> Option<ast::Expr> {
            // Candidates from in-scope dictionary parameters.
            let mut param_candidates: Vec<(String, ast::Expr)> = dicts_in_scope
                .iter()
                .filter_map(|name| {
                    let c = name.strip_prefix("__dict_")?.to_string();
                    Some((c, ast::Expr::new(span, ast::ExprKind::Var(name.clone()))))
                })
                .collect();
            param_candidates.sort_by(|(a, _), (b, _)| a.cmp(b));

            // Candidates from previously chosen concrete dictionaries.
            let mut known_candidates: Vec<(String, ast::Expr)> = known_dicts_in_scope
                .iter()
                .map(|(c, n)| {
                    (
                        c.clone(),
                        ast::Expr::new(span, ast::ExprKind::Var(n.clone())),
                    )
                })
                .collect();
            known_candidates.sort_by(|(a, _), (b, _)| a.cmp(b));

            for (base_class, base_expr) in param_candidates.into_iter().chain(known_candidates) {
                let Some(path) = find_super_path(class_env, &base_class, wanted_class) else {
                    continue;
                };
                return Some(project_dict_along_path(span, base_expr, &path));
            }

            None
        }

        let span = expr.span;
        Ok(match expr.kind {
            ExprKind::Var(mname) => {
                // Class method used as a value.
                // Rewrite `m` into a dictionary-projected function value.
                if let Some(classes) = class_env.method_classes.get(&mname) {
                    let Some(class) = classes.first() else {
                        return Err(Error::msg("internal: empty method class list"));
                    };

                    let dict_var = format!("__dict_{class}");

                    // Prefer in-scope dictionary params; otherwise reuse/derive from known dicts.
                    // If none are available, fall back to a lambda that takes the dictionary.
                    let dict_expr: Option<ast::Expr> = if dicts_in_scope.contains(&dict_var) {
                        Some(Expr::new(span, ExprKind::Var(dict_var.clone())))
                    } else if let Some(d) = known_dicts_in_scope.get(class) {
                        Some(Expr::new(span, ExprKind::Var(d.clone())))
                    } else {
                        derive_dict_expr_from_candidates(
                            span,
                            class_env,
                            class,
                            dicts_in_scope,
                            known_dicts_in_scope,
                        )
                    };

                    let make_method_value = |dict_expr: ast::Expr| {
                        let get = Expr::new(span, ExprKind::Var("__recordGet".to_string()));
                        let method_fn = Expr::new(
                            span,
                            ExprKind::Apply {
                                func: Box::new(get),
                                args: vec![
                                    dict_expr.clone(),
                                    Expr::new(span, ExprKind::String(mname.clone())),
                                ],
                            },
                        );
                        // Stored methods expect the dictionary as an explicit first argument.
                        Expr::new(
                            span,
                            ExprKind::Apply {
                                func: Box::new(method_fn),
                                args: vec![dict_expr],
                            },
                        )
                    };

                    if let Some(dict_expr) = dict_expr {
                        make_method_value(dict_expr)
                    } else {
                        let param = dict_var;
                        let dict_expr = Expr::new(span, ExprKind::Var(param.clone()));
                        Expr::new(
                            span,
                            ExprKind::Lambda {
                                params: vec![param],
                                body: Box::new(make_method_value(dict_expr)),
                            },
                        )
                    }
                } else {
                    Expr::new(span, ExprKind::Var(mname))
                }
            }
            ExprKind::Lambda { params, body } => {
                let mut scope = dicts_in_scope.clone();
                for p in &params {
                    if p.starts_with("__dict_") {
                        scope.insert(p.clone());
                    }
                }
                Expr::new(
                    span,
                    ExprKind::Lambda {
                        params,
                        body: Box::new(rewrite_expr(
                            module_snapshot,
                            class_env,
                            inferred,
                            &scope,
                            known_dicts_in_scope,
                            *body,
                        )?),
                    },
                )
            }
            ExprKind::Apply { func, args } => {
                // Fast path for class method calls so we can propagate chosen dictionaries into
                // nested expressions (notably useful for `Monad.return` inside `do`).
                if let ExprKind::Var(mname) = &func.kind {
                    if let Some(classes) = class_env.method_classes.get(mname) {
                        let Some(class) = classes.first() else {
                            return Err(Error::msg("internal: empty method class list"));
                        };

                        let dict_var = format!("__dict_{class}");
                        let mut chosen_name_for_known: Option<String> = None;

                        // Dictionary resolution order (important):
                        // 1) Explicit in-scope dictionary parameters.
                        // 2) Resolve by argument types (when possible).
                        // 3) Reuse/derive from previously chosen dictionaries (helps `return` in `do`).
                        //
                        // Without (2) before (3), a surrounding `Monad IO` call can incorrectly
                        // force unrelated nested calls like `fmap ... (Just 1)` to use `Functor IO`.
                        let dict_expr: ast::Expr = if dicts_in_scope.contains(&dict_var) {
                            chosen_name_for_known = Some(dict_var.clone());
                            Expr::new(span, ExprKind::Var(dict_var))
                        } else {
                            // Resolve dictionary by any usable argument type.
                            // This helps cases like `eq x 1` where `x` isn't ground but `1` is.
                            let mut first_non_ground: Option<Ty> = None;
                            let mut first_missing_instance: Option<Ty> = None;
                            let mut dict_name: Option<String> = None;

                            // Syntactic fallback (important for `do` desugaring): for `Monad`,
                            // we can often pick the instance by the type constructor head even
                            // if type inference for the argument fails.
                            if class == "Monad" {
                                fn monad_syntactic_head(e: &ast::Expr) -> Option<String> {
                                    match &e.kind {
                                        ast::ExprKind::Ctor(n) => Some(n.clone()),
                                        ast::ExprKind::Apply { func, .. } => match &func.kind {
                                            ast::ExprKind::Ctor(n) => Some(n.clone()),
                                            _ => None,
                                        },
                                        _ => None,
                                    }
                                }

                                for a in &args {
                                    if let Some(head) = monad_syntactic_head(a) {
                                        let key = (class.clone(), head);
                                        if let Some(d) = class_env.instances.get(&key) {
                                            dict_name = Some(d.clone());
                                            break;
                                        }
                                    }
                                }
                            }

                            if dict_name.is_none() {
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
                                            first_non_ground = Some(a_ty.clone());
                                        }
                                        // For most classes we require a ground type to pick an
                                        // instance. `Monad` is special: we can pick the instance by
                                        // the type constructor head (e.g. `IO` from `IO a`) even when
                                        // `a` is unknown.
                                        if class != "Monad" {
                                            continue;
                                        }
                                    }

                                    let Ok(head) = instance_head_key_ty_for_class(class, &a_ty)
                                    else {
                                        // This argument is ground but isn't a supported instance head.
                                        // Keep searching other arguments that might yield an instance.
                                        continue;
                                    };

                                    let key = (class.clone(), head);
                                    if let Some(d) = class_env.instances.get(&key) {
                                        dict_name = Some(d.clone());
                                        break;
                                    }

                                    if first_missing_instance.is_none() {
                                        first_missing_instance = Some(a_ty);
                                    }
                                }
                            }

                            if let Some(dict_name) = dict_name {
                                chosen_name_for_known = Some(dict_name.clone());
                                Expr::new(span, ExprKind::Var(dict_name))
                            } else if let Some(d) = known_dicts_in_scope.get(class) {
                                chosen_name_for_known = Some(d.clone());
                                Expr::new(span, ExprKind::Var(d.clone()))
                            } else if let Some(d) = derive_dict_expr_from_candidates(
                                span,
                                class_env,
                                class,
                                dicts_in_scope,
                                known_dicts_in_scope,
                            ) {
                                d
                            } else {
                                if let Some(ty) = first_missing_instance {
                                    return Err(Error::msg(format!(
                                        "no instance found for method call `{mname}`: {class} {ty}"
                                    )));
                                }

                                let ty_hint = first_non_ground.unwrap_or_else(|| {
                                    args.first()
                                        .and_then(|a0| {
                                            infer_in_module_with_class_env(
                                                module_snapshot,
                                                class_env,
                                                inferred,
                                                a0.clone(),
                                            )
                                            .ok()
                                        })
                                        .unwrap_or(Ty::Con("<unknown>".to_string()))
                                });

                                return Err(Error::msg(format!(
                                    "cannot resolve method call `{mname}`: no ground argument type available (e.g. {ty_hint})"
                                )));
                            }
                        };

                        // Propagate the chosen dictionary name to nested expressions, so that
                        // subsequent method calls (e.g. `return`) can reuse it.
                        let mut known = known_dicts_in_scope.clone();
                        if let Some(chosen) = chosen_name_for_known.clone() {
                            known.insert(class.clone(), chosen);
                        }

                        let new_args: Vec<_> = args
                            .into_iter()
                            .map(|a| {
                                rewrite_expr(
                                    module_snapshot,
                                    class_env,
                                    inferred,
                                    dicts_in_scope,
                                    &known,
                                    a,
                                )
                            })
                            .collect::<Result<Vec<_>>>()?;

                        let get = Expr::new(span, ExprKind::Var("__recordGet".to_string()));
                        let method_fn = Expr::new(
                            span,
                            ExprKind::Apply {
                                func: Box::new(get),
                                args: vec![
                                    dict_expr.clone(),
                                    Expr::new(span, ExprKind::String(mname.clone())),
                                ],
                            },
                        );

                        // Stored methods expect the dictionary as an explicit first argument.
                        let mut call_args = Vec::with_capacity(1 + new_args.len());
                        call_args.push(dict_expr);
                        call_args.extend(new_args);

                        return Ok(Expr::new(
                            span,
                            ExprKind::Apply {
                                func: Box::new(method_fn),
                                args: call_args,
                            },
                        ));
                    }
                }

                let func = rewrite_expr(
                    module_snapshot,
                    class_env,
                    inferred,
                    dicts_in_scope,
                    known_dicts_in_scope,
                    *func,
                )?;
                let args: Vec<_> = args
                    .into_iter()
                    .map(|a| {
                        rewrite_expr(
                            module_snapshot,
                            class_env,
                            inferred,
                            dicts_in_scope,
                            known_dicts_in_scope,
                            a,
                        )
                    })
                    .collect::<Result<Vec<_>>>()?;
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
                    cond: Box::new(rewrite_expr(
                        module_snapshot,
                        class_env,
                        inferred,
                        dicts_in_scope,
                        known_dicts_in_scope,
                        *cond,
                    )?),
                    then_branch: Box::new(rewrite_expr(
                        module_snapshot,
                        class_env,
                        inferred,
                        dicts_in_scope,
                        known_dicts_in_scope,
                        *then_branch,
                    )?),
                    else_branch: Box::new(rewrite_expr(
                        module_snapshot,
                        class_env,
                        inferred,
                        dicts_in_scope,
                        known_dicts_in_scope,
                        *else_branch,
                    )?),
                },
            ),
            ExprKind::Let { bindings, body } => Expr::new(
                span,
                ExprKind::Let {
                    bindings: bindings
                        .into_iter()
                        .map(|b| {
                            Ok(ast::Binding {
                                pat: b.pat,
                                expr: rewrite_expr(
                                    module_snapshot,
                                    class_env,
                                    inferred,
                                    dicts_in_scope,
                                    known_dicts_in_scope,
                                    b.expr,
                                )?,
                            })
                        })
                        .collect::<Result<Vec<_>>>()?,
                    body: Box::new(rewrite_expr(
                        module_snapshot,
                        class_env,
                        inferred,
                        dicts_in_scope,
                        known_dicts_in_scope,
                        *body,
                    )?),
                },
            ),
            ExprKind::Where { expr, bindings } => Expr::new(
                span,
                ExprKind::Where {
                    expr: Box::new(rewrite_expr(
                        module_snapshot,
                        class_env,
                        inferred,
                        dicts_in_scope,
                        known_dicts_in_scope,
                        *expr,
                    )?),
                    bindings: bindings
                        .into_iter()
                        .map(|b| {
                            Ok(ast::Binding {
                                pat: b.pat,
                                expr: rewrite_expr(
                                    module_snapshot,
                                    class_env,
                                    inferred,
                                    dicts_in_scope,
                                    known_dicts_in_scope,
                                    b.expr,
                                )?,
                            })
                        })
                        .collect::<Result<Vec<_>>>()?,
                },
            ),
            ExprKind::Annot { expr, ty } => Expr::new(
                span,
                ExprKind::Annot {
                    expr: Box::new(rewrite_expr(
                        module_snapshot,
                        class_env,
                        inferred,
                        dicts_in_scope,
                        known_dicts_in_scope,
                        *expr,
                    )?),
                    ty,
                },
            ),
            ExprKind::Do(stmts) => Expr::new(
                span,
                ExprKind::Do(
                    stmts
                        .into_iter()
                        .map(|s| {
                            Ok(match s {
                                ast::DoStmt::Bind { pat, expr } => ast::DoStmt::Bind {
                                    pat,
                                    expr: rewrite_expr(
                                        module_snapshot,
                                        class_env,
                                        inferred,
                                        dicts_in_scope,
                                        known_dicts_in_scope,
                                        expr,
                                    )?,
                                },
                                ast::DoStmt::Expr(e) => ast::DoStmt::Expr(rewrite_expr(
                                    module_snapshot,
                                    class_env,
                                    inferred,
                                    dicts_in_scope,
                                    known_dicts_in_scope,
                                    e,
                                )?),
                            })
                        })
                        .collect::<Result<Vec<_>>>()?,
                ),
            ),
            ExprKind::Case { expr, arms } => Expr::new(
                span,
                ExprKind::Case {
                    expr: Box::new(rewrite_expr(
                        module_snapshot,
                        class_env,
                        inferred,
                        dicts_in_scope,
                        known_dicts_in_scope,
                        *expr,
                    )?),
                    arms: arms
                        .into_iter()
                        .map(|a| {
                            Ok(ast::CaseArm {
                                pat: a.pat,
                                guard: a
                                    .guard
                                    .map(|g| {
                                        rewrite_expr(
                                            module_snapshot,
                                            class_env,
                                            inferred,
                                            dicts_in_scope,
                                            known_dicts_in_scope,
                                            g,
                                        )
                                    })
                                    .transpose()?,
                                body: rewrite_expr(
                                    module_snapshot,
                                    class_env,
                                    inferred,
                                    dicts_in_scope,
                                    known_dicts_in_scope,
                                    a.body,
                                )?,
                            })
                        })
                        .collect::<Result<Vec<_>>>()?,
                },
            ),
            ExprKind::Cons { head, tail } => Expr::new(
                span,
                ExprKind::Cons {
                    head: Box::new(rewrite_expr(
                        module_snapshot,
                        class_env,
                        inferred,
                        dicts_in_scope,
                        known_dicts_in_scope,
                        *head,
                    )?),
                    tail: Box::new(rewrite_expr(
                        module_snapshot,
                        class_env,
                        inferred,
                        dicts_in_scope,
                        known_dicts_in_scope,
                        *tail,
                    )?),
                },
            ),
            ExprKind::List(es) => Expr::new(
                span,
                ExprKind::List(
                    es.into_iter()
                        .map(|e| {
                            rewrite_expr(
                                module_snapshot,
                                class_env,
                                inferred,
                                dicts_in_scope,
                                known_dicts_in_scope,
                                e,
                            )
                        })
                        .collect::<Result<Vec<_>>>()?,
                ),
            ),
            ExprKind::Tuple(es) => Expr::new(
                span,
                ExprKind::Tuple(
                    es.into_iter()
                        .map(|e| {
                            rewrite_expr(
                                module_snapshot,
                                class_env,
                                inferred,
                                dicts_in_scope,
                                known_dicts_in_scope,
                                e,
                            )
                        })
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
                                rewrite_expr(
                                    module_snapshot,
                                    class_env,
                                    inferred,
                                    dicts_in_scope,
                                    known_dicts_in_scope,
                                    v,
                                )?,
                            ))
                        })
                        .collect::<Result<Vec<_>>>()?,
                ),
            ),
            other => Expr::new(span, other),
        })
    }

    let snapshot = module.clone();
    let empty_scope: HashSet<String> = HashSet::new();
    let empty_known: HashMap<String, String> = HashMap::new();
    module.items = module
        .items
        .drain(..)
        .map(|it| {
            Ok(match it {
                ast::Item::Binding(b) => ast::Item::Binding(ast::Binding {
                    pat: b.pat,
                    expr: rewrite_expr(
                        &snapshot,
                        class_env,
                        inferred,
                        &empty_scope,
                        &empty_known,
                        b.expr,
                    )?,
                }),
                other => other,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(())
}

fn infer_module_with_class_env(
    module: &ast::Module,
    class_env: &ClassEnv,
) -> Result<HashMap<String, Scheme>> {
    // Order-independent top-level inference (Haskell-like): compute SCCs of top-level bindings,
    // then typecheck SCCs in dependency order, generalizing non-recursive groups.
    let mut cx = InferCtx::default();
    let data_env = collect_data_env(module);
    let mut env_global = collect_ctor_env_with_class_env(&mut cx, module, class_env)?;

    let (bindings, ctx_names, defined_names, comps, comp_order) =
        infer_module_binding_scc_order(module)?;

    let mut subst = Subst::new();
    let mut out = HashMap::new();

    type BindingInfer = (Vec<(String, Ty)>, Vec<Constraint>);

    for ci in comp_order {
        let comp = &comps[ci];

        // Pre-bind all names in this SCC as monomorphic placeholders.
        let mut env_scc = env_global.clone();
        let mut scc_names: HashSet<String> = HashSet::new();
        for &bi in comp {
            for n in &defined_names[bi] {
                scc_names.insert(n.clone());
            }
        }

        let mut scc_names: Vec<String> = scc_names.into_iter().collect();
        scc_names.sort();

        for name in scc_names.iter() {
            let Ty::Var(v) = cx.fresh() else {
                unreachable!()
            };
            env_scc.insert(
                name.clone(),
                Scheme {
                    vars: vec![],
                    constraints: vec![],
                    ty: Ty::Var(v),
                },
            );
        }

        // Infer each binding in the SCC under the placeholder environment.
        let mut per_bind: Vec<BindingInfer> = Vec::new();
        for &bi in comp {
            let b = &bindings[bi];
            let ctx_name = &ctx_names[bi];

            let mut binds = Vec::new();
            let mut seen = HashSet::new();
            let mut cs_pat = Vec::new();
            let pat_ty = infer_pat_in(
                &mut cx,
                &data_env,
                &mut subst,
                &env_scc,
                &b.pat,
                &mut binds,
                &mut seen,
                &mut cs_pat,
            )
            .map_err(|e| e.with_context(format!("in binding {ctx_name}")))?;

            let env_in = apply_env(&subst, &env_scc);
            let (s_rhs, cs_rhs, t_rhs) = infer_expr_in(&mut cx, &data_env, &env_in, b.expr.clone())
                .map_err(|e| e.with_context(format!("in binding {ctx_name}")))?;
            subst = compose(&s_rhs, &subst);

            let s_pat = unify(apply(&subst, t_rhs), apply(&subst, pat_ty))
                .map_err(|e| e.with_context(format!("in binding {ctx_name}")))?;
            subst = compose(&s_pat, &subst);

            let mut cs = cs_rhs;
            cs.extend(cs_pat);
            per_bind.push((binds, cs));
        }

        // Generalize all names in the SCC against the environment *outside* the SCC.
        let env_gen_base = apply_env(&subst, &env_global);
        let mut new_schemes: Vec<(String, Scheme)> = Vec::new();
        for (binds, cs) in per_bind {
            for (name, t) in binds {
                let cs = simplify_constraints(
                    &data_env,
                    class_env,
                    apply_constraints(&subst, cs.clone()),
                )?;
                let scheme = generalize_qual(&env_gen_base, cs, apply(&subst, t));
                new_schemes.push((name, scheme));
            }
        }

        for (name, scheme) in new_schemes {
            env_global.insert(name.clone(), scheme.clone());
            out.insert(name, scheme);
        }
    }

    Ok(out)
}

type InferModuleBindingSccOrder = (
    Vec<ast::Binding>,
    Vec<String>,
    Vec<HashSet<String>>,
    Vec<Vec<usize>>,
    Vec<usize>,
);

fn infer_module_binding_scc_order(
    module: &ast::Module,
) -> Result<InferModuleBindingSccOrder> {
    // Collect top-level bindings as nodes.
    let mut bindings: Vec<ast::Binding> = Vec::new();
    let mut ctx_names: Vec<String> = Vec::new();
    let mut defined_names: Vec<HashSet<String>> = Vec::new();

    for it in &module.items {
        let ast::Item::Binding(b) = it else {
            continue;
        };

        let ctx = match &b.pat.kind {
            ast::PatternKind::Var(n) => n.clone(),
            _ => "<pattern>".to_string(),
        };

        let mut names = HashSet::new();
        pat_defined_names(&b.pat, &mut names);

        bindings.push(b.clone());
        ctx_names.push(ctx);
        defined_names.push(names);
    }

    let n = bindings.len();
    let mut name_to_binding: HashMap<String, usize> = HashMap::new();
    for (i, names) in defined_names.iter().enumerate() {
        let mut ns: Vec<&String> = names.iter().collect();
        ns.sort();
        for name in ns {
            // If there are duplicates, let the later phase produce a readable error.
            name_to_binding.insert(name.clone(), i);
        }
    }

    // Build dependency graph between binding-nodes.
    let mut graph: Vec<Vec<usize>> = vec![Vec::new(); n];
    for i in 0..n {
        let mut deps = HashSet::new();
        let empty: HashSet<String> = HashSet::new();
        collect_deps_in_expr(&bindings[i].expr, &name_to_binding, &empty, &mut deps);
        let mut dv: Vec<usize> = deps.into_iter().collect();
        dv.sort_unstable();
        graph[i] = dv;
    }

    let comps = tarjan_scc(&graph);
    let mut node_to_comp = vec![0usize; n];
    for (ci, comp) in comps.iter().enumerate() {
        for &v in comp {
            node_to_comp[v] = ci;
        }
    }

    // Build component graph with edges dependency -> dependent.
    let comp_n = comps.len();
    let mut comp_edges: Vec<HashSet<usize>> = vec![HashSet::new(); comp_n];
    let mut indeg = vec![0usize; comp_n];
    for u in 0..n {
        let cu = node_to_comp[u];
        for &v in &graph[u] {
            let cv = node_to_comp[v];
            if cu == cv {
                continue;
            }
            // u depends on v, so cv -> cu
            if comp_edges[cv].insert(cu) {
                indeg[cu] += 1;
            }
        }
    }

    let comp_order = toposort::kahn_deterministic(&comp_edges, indeg)?;
    Ok((bindings, ctx_names, defined_names, comps, comp_order))
}

pub fn typecheck(mut module: ast::Module) -> Result<TypedModule> {
    if module
        .items
        .iter()
        .any(|it| matches!(it, ast::Item::Import(_)))
    {
        return Err(Error::msg("imports are not supported yet"));
    }

    // Try to hit the module-level typecheck cache.
    if let Some(inferred) = stdlib_cache::check_module_typecheck_cache(&module) {
        return Ok(TypedModule { module, inferred });
    }

    let aliases = collect_type_aliases(&module);
    module.items = module
        .items
        .into_iter()
        .map(|it| expand_item(it, &aliases))
        .collect::<Result<Vec<_>>>()?;

    let class_env = desugar_typeclasses(&mut module)?;

    // If `Monad` is available, desugar `do`-notation into `(>>=)` / `(>>)`. This allows `do` to
    // work for non-IO monads (via type classes).
    if class_env.class_params.contains_key("Monad") {
        desugar_do_to_monad_ops_in_module(&mut module)?;
    }

    let inferred = infer_module_with_class_env(&module, &class_env)?;

    if let Some(main) = inferred.get("main") {
        let expected = Ty::App {
            head: Box::new(Ty::Con("IO".to_string())),
            args: vec![Ty::Con("Unit".to_string())],
        };
        if !main.vars.is_empty() || !main.constraints.is_empty() || main.ty != expected {
            return Err(Error::msg("main must have type IO Unit"));
        }
    }

    typeclass_dict_passing_common::rewrite_class_dict_passing_in_module(&mut module, &class_env, &inferred)?;

    rewrite_class_method_calls_in_module(&mut module, &class_env, &inferred)?;

    // MVP: start routing `show`/`toString` calls through an explicit Show dictionary.
    rewrite_show_calls_in_module(&mut module);

    // Store in cache for future lookups.
    stdlib_cache::store_module_typecheck_cache(&module, &inferred);

    Ok(TypedModule { module, inferred })
}

fn desugar_do_to_monad_ops_in_module(module: &mut ast::Module) -> Result<()> {
    fn desugar_binding(binding: &mut ast::Binding, fresh: &mut usize) -> Result<()> {
        let expr = std::mem::replace(&mut binding.expr, ast::Expr::dummy(ast::ExprKind::Unit));
        binding.expr = desugar_monad_do_expr(expr, fresh)?;
        Ok(())
    }

    let mut fresh = 0usize;
    for item in &mut module.items {
        match item {
            ast::Item::Binding(b) => desugar_binding(b, &mut fresh)?,
            ast::Item::ClassDecl(c) => {
                for b in &mut c.default_methods {
                    desugar_binding(b, &mut fresh)?;
                }
            }
            ast::Item::InstanceDecl(i) => {
                for b in &mut i.methods {
                    desugar_binding(b, &mut fresh)?;
                }
            }
            _ => {}
        }
    }

    Ok(())
}

fn desugar_apply2(span: ast::Span, op: &str, a: ast::Expr, b: ast::Expr) -> ast::Expr {
    ast::Expr::new(
        span,
        ast::ExprKind::Apply {
            func: Box::new(ast::Expr::new(span, ast::ExprKind::Var(op.to_string()))),
            args: vec![a, b],
        },
    )
}

fn desugar_lambda1(span: ast::Span, param: String, body: ast::Expr) -> ast::Expr {
    ast::Expr::new(
        span,
        ast::ExprKind::Lambda {
            params: vec![param],
            body: Box::new(body),
        },
    )
}

fn desugar_monad_do_block(stmts: Vec<ast::DoStmt>, fresh: &mut usize, span: ast::Span) -> Result<ast::Expr> {
    use ast::{DoStmt, Expr, ExprKind, PatternKind};

    if stmts.is_empty() {
        return Err(Error::msg("empty do-block"));
    }

    let mut it = stmts.into_iter();
    let last = it.next_back().unwrap();

    let mut acc = match last {
        DoStmt::Expr(e) => desugar_monad_do_expr(e, fresh)?,
        DoStmt::Bind { .. } => {
            return Err(Error::msg("do-block must end with an expression statement"))
        }
    };

    while let Some(stmt) = it.next_back() {
        match stmt {
            DoStmt::Expr(e) => {
                let e = desugar_monad_do_expr(e, fresh)?;
                acc = desugar_apply2(span, ">>", e, acc);
            }
            DoStmt::Bind { pat, expr } => {
                let rhs = desugar_monad_do_expr(expr, fresh)?;
                match pat.kind {
                    PatternKind::Var(name) => {
                        let k = desugar_lambda1(span, name, acc);
                        acc = desugar_apply2(span, ">>=", rhs, k);
                    }
                    PatternKind::Wildcard => {
                        let name = format!("__do_ignored{}", *fresh);
                        *fresh += 1;
                        let k = desugar_lambda1(span, name, acc);
                        acc = desugar_apply2(span, ">>=", rhs, k);
                    }
                    _ => {
                        let tmp = format!("__do_tmp{}", *fresh);
                        *fresh += 1;
                        let case_expr = Expr::new(
                            span,
                            ExprKind::Case {
                                expr: Box::new(Expr::new(span, ExprKind::Var(tmp.clone()))),
                                arms: vec![ast::CaseArm {
                                    pat,
                                    guard: None,
                                    body: acc,
                                }],
                            },
                        );
                        let k = desugar_lambda1(span, tmp, case_expr);
                        acc = desugar_apply2(span, ">>=", rhs, k);
                    }
                }
            }
        }
    }
    Ok(acc)
}

fn desugar_monad_do_expr(expr: ast::Expr, fresh: &mut usize) -> Result<ast::Expr> {
    use ast::{Expr, ExprKind};

    let span = expr.span;
    Ok(match expr.kind {
        ExprKind::Do(stmts) => desugar_monad_do_block(stmts, fresh, span)?,
        ExprKind::Lambda { params, body } => {
            desugar_monad_do_lambda(span, params, *body, fresh)?
        }
        ExprKind::Apply { func, args } => desugar_monad_do_apply(span, *func, args, fresh)?,
        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => desugar_monad_do_if(span, *cond, *then_branch, *else_branch, fresh)?,
        ExprKind::Let { bindings, body } => desugar_monad_do_let(span, bindings, *body, fresh)?,
        ExprKind::Where { expr, bindings } => {
            desugar_monad_do_where(span, *expr, bindings, fresh)?
        }
        ExprKind::Annot { expr, ty } => desugar_monad_do_annot(span, *expr, ty, fresh)?,
        ExprKind::Case { expr, arms } => desugar_monad_do_case(span, *expr, arms, fresh)?,
        ExprKind::Cons { head, tail } => desugar_monad_do_cons(span, *head, *tail, fresh)?,
        ExprKind::List(es) => desugar_monad_do_list(span, es, fresh)?,
        ExprKind::Tuple(es) => desugar_monad_do_tuple(span, es, fresh)?,
        ExprKind::Record(fields) => desugar_monad_do_record(span, fields, fresh)?,
        other => Expr::new(span, other),
    })
}

fn desugar_monad_do_lambda(
    span: ast::Span,
    params: Vec<String>,
    body: ast::Expr,
    fresh: &mut usize,
) -> Result<ast::Expr> {
    Ok(ast::Expr::new(
        span,
        ast::ExprKind::Lambda {
            params,
            body: Box::new(desugar_monad_do_expr(body, fresh)?),
        },
    ))
}

fn desugar_monad_do_apply(
    span: ast::Span,
    func: ast::Expr,
    args: Vec<ast::Expr>,
    fresh: &mut usize,
) -> Result<ast::Expr> {
    Ok(ast::Expr::new(
        span,
        ast::ExprKind::Apply {
            func: Box::new(desugar_monad_do_expr(func, fresh)?),
            args: args
                .into_iter()
                .map(|a| desugar_monad_do_expr(a, fresh))
                .collect::<Result<Vec<_>>>()?,
        },
    ))
}

fn desugar_monad_do_if(
    span: ast::Span,
    cond: ast::Expr,
    then_branch: ast::Expr,
    else_branch: ast::Expr,
    fresh: &mut usize,
) -> Result<ast::Expr> {
    Ok(ast::Expr::new(
        span,
        ast::ExprKind::If {
            cond: Box::new(desugar_monad_do_expr(cond, fresh)?),
            then_branch: Box::new(desugar_monad_do_expr(then_branch, fresh)?),
            else_branch: Box::new(desugar_monad_do_expr(else_branch, fresh)?),
        },
    ))
}

fn desugar_monad_do_bindings(
    bindings: Vec<ast::Binding>,
    fresh: &mut usize,
) -> Result<Vec<ast::Binding>> {
    bindings
        .into_iter()
        .map(|b| {
            Ok(ast::Binding {
                pat: b.pat,
                expr: desugar_monad_do_expr(b.expr, fresh)?,
            })
        })
        .collect::<Result<Vec<_>>>()
}

fn desugar_monad_do_let(
    span: ast::Span,
    bindings: Vec<ast::Binding>,
    body: ast::Expr,
    fresh: &mut usize,
) -> Result<ast::Expr> {
    Ok(ast::Expr::new(
        span,
        ast::ExprKind::Let {
            bindings: desugar_monad_do_bindings(bindings, fresh)?,
            body: Box::new(desugar_monad_do_expr(body, fresh)?),
        },
    ))
}

fn desugar_monad_do_where(
    span: ast::Span,
    expr: ast::Expr,
    bindings: Vec<ast::Binding>,
    fresh: &mut usize,
) -> Result<ast::Expr> {
    Ok(ast::Expr::new(
        span,
        ast::ExprKind::Where {
            expr: Box::new(desugar_monad_do_expr(expr, fresh)?),
            bindings: desugar_monad_do_bindings(bindings, fresh)?,
        },
    ))
}

fn desugar_monad_do_annot(
    span: ast::Span,
    expr: ast::Expr,
    ty: ast::QualType,
    fresh: &mut usize,
) -> Result<ast::Expr> {
    Ok(ast::Expr::new(
        span,
        ast::ExprKind::Annot {
            expr: Box::new(desugar_monad_do_expr(expr, fresh)?),
            ty,
        },
    ))
}

fn desugar_monad_do_case(
    span: ast::Span,
    expr: ast::Expr,
    arms: Vec<ast::CaseArm>,
    fresh: &mut usize,
) -> Result<ast::Expr> {
    Ok(ast::Expr::new(
        span,
        ast::ExprKind::Case {
            expr: Box::new(desugar_monad_do_expr(expr, fresh)?),
            arms: arms
                .into_iter()
                .map(|a| {
                    Ok(ast::CaseArm {
                        pat: a.pat,
                        guard: a
                            .guard
                            .map(|g| desugar_monad_do_expr(g, fresh))
                            .transpose()?,
                        body: desugar_monad_do_expr(a.body, fresh)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        },
    ))
}

fn desugar_monad_do_cons(
    span: ast::Span,
    head: ast::Expr,
    tail: ast::Expr,
    fresh: &mut usize,
) -> Result<ast::Expr> {
    Ok(ast::Expr::new(
        span,
        ast::ExprKind::Cons {
            head: Box::new(desugar_monad_do_expr(head, fresh)?),
            tail: Box::new(desugar_monad_do_expr(tail, fresh)?),
        },
    ))
}

fn desugar_monad_do_list(
    span: ast::Span,
    es: Vec<ast::Expr>,
    fresh: &mut usize,
) -> Result<ast::Expr> {
    Ok(ast::Expr::new(
        span,
        ast::ExprKind::List(
            es.into_iter()
                .map(|e| desugar_monad_do_expr(e, fresh))
                .collect::<Result<Vec<_>>>()?,
        ),
    ))
}

fn desugar_monad_do_tuple(
    span: ast::Span,
    es: Vec<ast::Expr>,
    fresh: &mut usize,
) -> Result<ast::Expr> {
    Ok(ast::Expr::new(
        span,
        ast::ExprKind::Tuple(
            es.into_iter()
                .map(|e| desugar_monad_do_expr(e, fresh))
                .collect::<Result<Vec<_>>>()?,
        ),
    ))
}

fn desugar_monad_do_record(
    span: ast::Span,
    fields: Vec<(String, ast::Expr)>,
    fresh: &mut usize,
) -> Result<ast::Expr> {
    Ok(ast::Expr::new(
        span,
        ast::ExprKind::Record(
            fields
                .into_iter()
                .map(|(k, v)| Ok((k, desugar_monad_do_expr(v, fresh)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
    ))
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
            deriving: d.deriving,
        })),
        ast::Item::ClassDecl(c) => Ok(ast::Item::ClassDecl(ast::ClassDecl {
            name: c.name,
            param: c.param,
            supers: c
                .supers
                .into_iter()
                .map(|p| {
                    Ok(match p {
                        ast::Predicate::Show(t) => {
                            ast::Predicate::Show(expand_type(t, aliases, &mut Vec::new())?)
                        }
                        ast::Predicate::ShowRow(t) => {
                            ast::Predicate::ShowRow(expand_type(t, aliases, &mut Vec::new())?)
                        }
                        ast::Predicate::Eq(t) => {
                            ast::Predicate::Eq(expand_type(t, aliases, &mut Vec::new())?)
                        }
                        ast::Predicate::EqRow(t) => {
                            ast::Predicate::EqRow(expand_type(t, aliases, &mut Vec::new())?)
                        }
                        ast::Predicate::Class { class, ty } => ast::Predicate::Class {
                            class,
                            ty: expand_type(ty, aliases, &mut Vec::new())?,
                        },
                        ast::Predicate::Lacks { label, row } => ast::Predicate::Lacks {
                            label,
                            row: expand_type(row, aliases, &mut Vec::new())?,
                        },
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            methods: c
                .methods
                .into_iter()
                .map(|m| {
                    Ok(ast::ClassMethodSig {
                        name: m.name,
                        ty: ast::QualType {
                            preds: m
                                .ty
                                .preds
                                .into_iter()
                                .map(|p| {
                                    Ok(match p {
                                        ast::Predicate::Show(t) => ast::Predicate::Show(
                                            expand_type(t, aliases, &mut Vec::new())?,
                                        ),
                                        ast::Predicate::ShowRow(t) => ast::Predicate::ShowRow(
                                            expand_type(t, aliases, &mut Vec::new())?,
                                        ),
                                        ast::Predicate::Eq(t) => ast::Predicate::Eq(expand_type(
                                            t,
                                            aliases,
                                            &mut Vec::new(),
                                        )?),
                                        ast::Predicate::EqRow(t) => ast::Predicate::EqRow(
                                            expand_type(t, aliases, &mut Vec::new())?,
                                        ),
                                        ast::Predicate::Class { class, ty } => {
                                            ast::Predicate::Class {
                                                class,
                                                ty: expand_type(ty, aliases, &mut Vec::new())?,
                                            }
                                        }
                                        ast::Predicate::Lacks { label, row } => {
                                            ast::Predicate::Lacks {
                                                label,
                                                row: expand_type(row, aliases, &mut Vec::new())?,
                                            }
                                        }
                                    })
                                })
                                .collect::<Result<Vec<_>>>()?,
                            ty: expand_type(m.ty.ty, aliases, &mut Vec::new())?,
                        },
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            default_methods: c
                .default_methods
                .into_iter()
                .map(|b| {
                    Ok(ast::Binding {
                        pat: expand_pat(b.pat, aliases)?,
                        expr: expand_expr(b.expr, aliases)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        })),
        ast::Item::InstanceDecl(inst) => Ok(ast::Item::InstanceDecl(ast::InstanceDecl {
            preds: inst
                .preds
                .into_iter()
                .map(|p| {
                    Ok(match p {
                        ast::Predicate::Show(t) => {
                            ast::Predicate::Show(expand_type(t, aliases, &mut Vec::new())?)
                        }
                        ast::Predicate::ShowRow(t) => {
                            ast::Predicate::ShowRow(expand_type(t, aliases, &mut Vec::new())?)
                        }
                        ast::Predicate::Eq(t) => {
                            ast::Predicate::Eq(expand_type(t, aliases, &mut Vec::new())?)
                        }
                        ast::Predicate::EqRow(t) => {
                            ast::Predicate::EqRow(expand_type(t, aliases, &mut Vec::new())?)
                        }
                        ast::Predicate::Class { class, ty } => ast::Predicate::Class {
                            class,
                            ty: expand_type(ty, aliases, &mut Vec::new())?,
                        },
                        ast::Predicate::Lacks { label, row } => ast::Predicate::Lacks {
                            label,
                            row: expand_type(row, aliases, &mut Vec::new())?,
                        },
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            class: inst.class,
            ty: expand_type(inst.ty, aliases, &mut Vec::new())?,
            methods: inst
                .methods
                .into_iter()
                .map(|b| {
                    Ok(ast::Binding {
                        pat: expand_pat(b.pat, aliases)?,
                        expr: expand_expr(b.expr, aliases)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        })),
        it @ (ast::Item::Import(_) | ast::Item::Export(_) | ast::Item::Fixity(_)) => Ok(it),
    }
}

fn expand_pat(
    pat: ast::Pattern,
    aliases: &HashMap<String, ast::TypeAlias>,
) -> Result<ast::Pattern> {
    use ast::{Pattern, PatternKind};
    let span = pat.span;
    Ok(match pat.kind {
        kind @ (PatternKind::Var(_)
        | PatternKind::Wildcard
        | PatternKind::Hole(_)
        | PatternKind::Literal(_)) => Pattern::new(span, kind),
        PatternKind::Tuple(ps) => Pattern::new(
            span,
            PatternKind::Tuple(
                ps.into_iter()
                    .map(|p| expand_pat(p, aliases))
                    .collect::<Result<Vec<_>>>()?,
            ),
        ),
        PatternKind::List(ps) => Pattern::new(
            span,
            PatternKind::List(
                ps.into_iter()
                    .map(|p| expand_pat(p, aliases))
                    .collect::<Result<Vec<_>>>()?,
            ),
        ),
        PatternKind::Record(fields) => Pattern::new(
            span,
            PatternKind::Record(
                fields
                    .into_iter()
                    .map(|(n, p)| Ok((n, expand_pat(p, aliases)?)))
                    .collect::<Result<Vec<_>>>()?,
            ),
        ),
        PatternKind::RecordLoose(fields, rest) => Pattern::new(
            span,
            PatternKind::RecordLoose(
                fields
                    .into_iter()
                    .map(|(n, p)| Ok((n, expand_pat(p, aliases)?)))
                    .collect::<Result<Vec<_>>>()?,
                rest,
            ),
        ),
        PatternKind::Cons(a, b) => Pattern::new(
            span,
            PatternKind::Cons(
                Box::new(expand_pat(*a, aliases)?),
                Box::new(expand_pat(*b, aliases)?),
            ),
        ),
        PatternKind::Or(a, b) => Pattern::new(
            span,
            PatternKind::Or(
                Box::new(expand_pat(*a, aliases)?),
                Box::new(expand_pat(*b, aliases)?),
            ),
        ),
        PatternKind::As(name, p) => Pattern::new(
            span,
            PatternKind::As(name, Box::new(expand_pat(*p, aliases)?)),
        ),
        PatternKind::View(p, e) => Pattern::new(
            span,
            PatternKind::View(
                Box::new(expand_pat(*p, aliases)?),
                Box::new(expand_expr(*e, aliases)?),
            ),
        ),
        PatternKind::Constructor { name, args } => Pattern::new(
            span,
            PatternKind::Constructor {
                name,
                args: args
                    .into_iter()
                    .map(|p| expand_pat(p, aliases))
                    .collect::<Result<Vec<_>>>()?,
            },
        ),
    })
}

fn expand_expr(expr: ast::Expr, aliases: &HashMap<String, ast::TypeAlias>) -> Result<ast::Expr> {
    use ast::{Expr, ExprKind};
    let span = expr.span;
    Ok(match expr.kind {
        ExprKind::Lambda { params, body } => Expr::new(
            span,
            ExprKind::Lambda {
                params,
                body: Box::new(expand_expr(*body, aliases)?),
            },
        ),
        ExprKind::Apply { func, args } => Expr::new(
            span,
            ExprKind::Apply {
                func: Box::new(expand_expr(*func, aliases)?),
                args: args
                    .into_iter()
                    .map(|e| expand_expr(e, aliases))
                    .collect::<Result<Vec<_>>>()?,
            },
        ),
        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => Expr::new(
            span,
            ExprKind::If {
                cond: Box::new(expand_expr(*cond, aliases)?),
                then_branch: Box::new(expand_expr(*then_branch, aliases)?),
                else_branch: Box::new(expand_expr(*else_branch, aliases)?),
            },
        ),
        ExprKind::Let { bindings, body } => Expr::new(
            span,
            ExprKind::Let {
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
        ),
        ExprKind::Where { expr, bindings } => Expr::new(
            span,
            ExprKind::Where {
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
        ),
        ExprKind::Annot { expr, ty } => Expr::new(
            span,
            ExprKind::Annot {
                expr: Box::new(expand_expr(*expr, aliases)?),
                ty: expand_qual_type(ty, aliases)?,
            },
        ),
        ExprKind::Do(stmts) => Expr::new(
            span,
            ExprKind::Do(
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
        ),
        ExprKind::Case { expr, arms } => Expr::new(
            span,
            ExprKind::Case {
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
        ),
        ExprKind::Cons { head, tail } => Expr::new(
            span,
            ExprKind::Cons {
                head: Box::new(expand_expr(*head, aliases)?),
                tail: Box::new(expand_expr(*tail, aliases)?),
            },
        ),
        ExprKind::List(v) => Expr::new(
            span,
            ExprKind::List(
                v.into_iter()
                    .map(|e| expand_expr(e, aliases))
                    .collect::<Result<Vec<_>>>()?,
            ),
        ),
        ExprKind::Tuple(v) => Expr::new(
            span,
            ExprKind::Tuple(
                v.into_iter()
                    .map(|e| expand_expr(e, aliases))
                    .collect::<Result<Vec<_>>>()?,
            ),
        ),
        ExprKind::Record(fields) => Expr::new(
            span,
            ExprKind::Record(
                fields
                    .into_iter()
                    .map(|(n, e)| Ok((n, expand_expr(e, aliases)?)))
                    .collect::<Result<Vec<_>>>()?,
            ),
        ),
        other => Expr::new(span, other),
    })
}

fn expand_qual_type(
    ty: ast::QualType,
    aliases: &HashMap<String, ast::TypeAlias>,
) -> Result<ast::QualType> {
    let mut stack = Vec::new();
    let preds = ty
        .preds
        .into_iter()
        .map(|p| {
            Ok(match p {
                ast::Predicate::Show(t) => {
                    ast::Predicate::Show(expand_type(t, aliases, &mut stack)?)
                }
                ast::Predicate::ShowRow(t) => {
                    ast::Predicate::ShowRow(expand_type(t, aliases, &mut stack)?)
                }
                ast::Predicate::Eq(t) => ast::Predicate::Eq(expand_type(t, aliases, &mut stack)?),
                ast::Predicate::EqRow(t) => {
                    ast::Predicate::EqRow(expand_type(t, aliases, &mut stack)?)
                }
                ast::Predicate::Class { class, ty } => ast::Predicate::Class {
                    class,
                    ty: expand_type(ty, aliases, &mut stack)?,
                },
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
                qualified: false,
                as_name: None,
            })],
        };
        assert!(typecheck(m).is_err());
    }

    #[test]
    fn typecheck_file_imports_data_type_exported_by_type_name() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_typecheck_file_imports_ok_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("A.ks");
        std::fs::write(
            &a,
            "module A where\n  export Maybe(..)\n  data Maybe a = Nothing | Just a\n",
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
    fn typecheck_file_imports_ctor_subset() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_typecheck_file_imports_ctor_subset_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("A.ks");
        std::fs::write(
            &a,
            "module A where\n  export Maybe(Just)\n  data Maybe a = Nothing | Just a\n",
        )
        .unwrap();

        let ok = dir.join("Ok.ks");
        std::fs::write(
            &ok,
            "module Ok where\n  import A\n  x = Just 1\n  main = IO ()\n",
        )
        .unwrap();
        let _tm = typecheck_file(&ok).unwrap();

        let bad = dir.join("Bad.ks");
        std::fs::write(
            &bad,
            "module Bad where\n  import A\n  x = case Just 1 of\n    Nothing -> 0\n    Just n -> n\n  main = IO ()\n",
        )
        .unwrap();
        let e = typecheck_file(&bad).unwrap_err();
        assert!(format!("{e}").contains("unknown constructor"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn typecheck_file_imports_ctor_subset_qualified_cannot_bypass() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_typecheck_file_imports_ctor_subset_qual_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("A.ks");
        std::fs::write(
            &a,
            "module A where\n  export Maybe(Just)\n  data Maybe a = Nothing | Just a\n",
        )
        .unwrap();

        let main = dir.join("Main.ks");
        std::fs::write(
            &main,
            "module Main where\n  import qualified A\n  x = A.Nothing\n  main = IO ()\n",
        )
        .unwrap();

        let e = typecheck_file(&main).unwrap_err();
        assert!(format!("{e}").contains("unknown constructor"));

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
            "module Main where\n  import qualified A as A1\n  import qualified B as B1\n  y = A1.x + B1.x\n  main = IO ()\n",
        )
        .unwrap();

        let _tm = typecheck_file(&main).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn typecheck_file_reports_name_conflict_with_sources() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_typecheck_file_name_conflict_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("A.ks");
        std::fs::write(&a, "module A where\n  export x\n  x = 1\n").unwrap();

        let b = dir.join("B.ks");
        std::fs::write(&b, "module B where\n  export x\n  x = 2\n").unwrap();

        let main = dir.join("Main.ks");
        std::fs::write(
            &main,
            "module Main where\n  import A\n  import B\n  y = x\n  main = IO ()\n",
        )
        .unwrap();

        let e = typecheck_file(&main).unwrap_err();
        let msg = format!("{e}");
        assert!(msg.contains("name conflict: x"));
        assert!(msg.contains("import A"));
        assert!(msg.contains("import B"));

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
    fn typecheck_file_import_search_prefers_local_over_stdlib() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_typecheck_file_import_search_local_over_stdlib_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Shadow stdlib Prelude with a local Prelude.
        let prelude = dir.join("Prelude.ks");
        std::fs::write(
            &prelude,
            "module Prelude where\n  export localOnly\n  localOnly = 1\n",
        )
        .unwrap();

        let main = dir.join("Main.ks");
        std::fs::write(
            &main,
            "module Main where\n  import Prelude\n  y = localOnly\n  main = IO ()\n",
        )
        .unwrap();

        let _tm = typecheck_file(&main).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn typecheck_file_import_errors_show_tried_paths() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_typecheck_file_import_tried_paths_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let main = dir.join("Main.ks");
        std::fs::write(
            &main,
            "module Main where\n  import Missing\n  main = IO ()\n",
        )
        .unwrap();

        let e = typecheck_file(&main).unwrap_err();
        let msg = format!("{e}");
        assert!(msg.contains("cannot find module file for import Missing"));
        assert!(msg.contains("tried:"));
        assert!(msg.contains("stdlib"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn typecheck_file_rejects_module_name_mismatch() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_typecheck_file_module_name_mismatch_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("A.ks");
        std::fs::write(&a, "module Wrong where\n  x = 1\n").unwrap();

        let main = dir.join("Main.ks");
        std::fs::write(
            &main,
            "module Main where\n  import A\n  y = A.x\n  main = IO ()\n",
        )
        .unwrap();

        let e = typecheck_file(&main).unwrap_err();
        assert!(format!("{e}").contains("module name mismatch: import A"));

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
            ty: Ty::Func(
                Box::new(Ty::Var(2)),
                Box::new(Ty::Con("String".to_string())),
            ),
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
        assert_eq!(
            format!("{s}"),
            "forall a b. (Show a, Lacks \"x\" b) => b -> b"
        );
    }

    #[test]
    fn infer_annotated_show_constraint_roundtrips_via_display() {
        let m = crate::parser::parse_module("x = (\\y -> y) :: Show a => a -> a\n").unwrap();
        let env = infer_module(&m).unwrap();
        assert_eq!(
            format!("{}", env.get("x").unwrap()),
            "forall a. Show a => a -> a"
        );
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
        let ast::Expr {
            kind: ast::ExprKind::Annot { ty, .. },
            ..
        } = &b.expr
        else {
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
                ast::Predicate::Show(t) => {
                    cs.push(Constraint::Show(lower_surface_type(&mut cx, t, &mut holes)))
                }
                ast::Predicate::ShowRow(t) => cs.push(Constraint::ShowRow(lower_surface_type(
                    &mut cx, t, &mut holes,
                ))),
                ast::Predicate::Eq(t) => {
                    cs.push(Constraint::Eq(lower_surface_type(&mut cx, t, &mut holes)))
                }
                ast::Predicate::EqRow(t) => cs.push(Constraint::EqRow(lower_surface_type(
                    &mut cx, t, &mut holes,
                ))),
                ast::Predicate::Lacks { label, row } => cs.push(Constraint::Lacks {
                    label: label.clone(),
                    row: lower_surface_type(&mut cx, row, &mut holes),
                }),
                ast::Predicate::Class { class, ty } => cs.push(Constraint::Class {
                    class: class.clone(),
                    ty: lower_surface_type(&mut cx, ty, &mut holes),
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

    fn canon_constraint_in(
        c: &Constraint,
        m: &mut HashMap<u32, u32>,
        next: &mut u32,
    ) -> Constraint {
        match c {
            Constraint::Show(t) => Constraint::Show(canon_ty_in(t, m, next)),
            Constraint::ShowRow(t) => Constraint::ShowRow(canon_ty_in(t, m, next)),
            Constraint::Eq(t) => Constraint::Eq(canon_ty_in(t, m, next)),
            Constraint::EqRow(t) => Constraint::EqRow(canon_ty_in(t, m, next)),
            Constraint::Class { class, ty } => Constraint::Class {
                class: class.clone(),
                ty: canon_ty_in(ty, m, next),
            },
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
        let _ = infer_expr(ast::Expr::dummy(ast::ExprKind::Let {
            bindings: vec![ast::Binding {
                pat: ast::Pattern::dummy(ast::PatternKind::Var("x".to_string())),
                expr: ast::Expr::dummy(ast::ExprKind::Var("y".to_string())),
            }],
            body: Box::new(ast::Expr::dummy(ast::ExprKind::Var("x".to_string()))),
        }))
        .unwrap_err();

        let e = infer_expr(ast::Expr::dummy(ast::ExprKind::Let {
            bindings: vec![ast::Binding {
                pat: ast::Pattern::dummy(ast::PatternKind::Var("x".to_string())),
                expr: ast::Expr::dummy(ast::ExprKind::Var("y".to_string())),
            }],
            body: Box::new(ast::Expr::dummy(ast::ExprKind::Var("x".to_string()))),
        }))
        .unwrap_err();
        assert!(format!("{e}").contains("in let binding x"));
    }

    #[test]
    fn type_error_includes_where_binding_name() {
        let e = infer_expr(ast::Expr::dummy(ast::ExprKind::Where {
            expr: Box::new(ast::Expr::dummy(ast::ExprKind::Var("x".to_string()))),
            bindings: vec![ast::Binding {
                pat: ast::Pattern::dummy(ast::PatternKind::Var("x".to_string())),
                expr: ast::Expr::dummy(ast::ExprKind::Var("y".to_string())),
            }],
        }))
        .unwrap_err();
        assert!(format!("{e}").contains("in where binding x"));
    }

    #[test]
    fn type_error_includes_case_arm_number() {
        let e = infer_expr(ast::Expr::dummy(ast::ExprKind::Case {
            expr: Box::new(ast::Expr::dummy(ast::ExprKind::Integer("1".to_string()))),
            arms: vec![
                ast::CaseArm {
                    pat: ast::Pattern::dummy(ast::PatternKind::Wildcard),
                    guard: None,
                    body: ast::Expr::dummy(ast::ExprKind::Var("y".to_string())),
                },
                ast::CaseArm {
                    pat: ast::Pattern::dummy(ast::PatternKind::Wildcard),
                    guard: None,
                    body: ast::Expr::dummy(ast::ExprKind::Integer("0".to_string())),
                },
            ],
        }))
        .unwrap_err();
        assert!(format!("{e}").contains("in case arm 1"));
    }

    #[test]
    fn type_error_includes_do_stmt_number() {
        let e = infer_expr(ast::Expr::dummy(ast::ExprKind::Do(vec![
            ast::DoStmt::Expr(ast::Expr::dummy(ast::ExprKind::Var("y".to_string()))),
        ])))
        .unwrap_err();
        assert!(format!("{e}").contains("in do stmt 1"));
    }

    #[test]
    fn type_error_includes_if_then_context() {
        let e = infer_expr(ast::Expr::dummy(ast::ExprKind::If {
            cond: Box::new(ast::Expr::dummy(ast::ExprKind::Bool(true))),
            then_branch: Box::new(ast::Expr::dummy(ast::ExprKind::Var("y".to_string()))),
            else_branch: Box::new(ast::Expr::dummy(ast::ExprKind::Integer("0".to_string()))),
        }))
        .unwrap_err();
        assert!(format!("{e}").contains("in if then"));
    }

    #[test]
    fn type_error_includes_if_cond_context() {
        let e = infer_expr(ast::Expr::dummy(ast::ExprKind::If {
            cond: Box::new(ast::Expr::dummy(ast::ExprKind::Integer("1".to_string()))),
            then_branch: Box::new(ast::Expr::dummy(ast::ExprKind::Integer("0".to_string()))),
            else_branch: Box::new(ast::Expr::dummy(ast::ExprKind::Integer("0".to_string()))),
        }))
        .unwrap_err();
        assert!(format!("{e}").contains("in if cond"));
    }

    #[test]
    fn infer_identity_lambda() {
        let ty = infer_expr(ast::Expr::dummy(ast::ExprKind::Lambda {
            params: vec!["x".to_string()],
            body: Box::new(ast::Expr::dummy(ast::ExprKind::Var("x".to_string()))),
        }))
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
        let id = ast::Expr::dummy(ast::ExprKind::Lambda {
            params: vec!["x".to_string()],
            body: Box::new(ast::Expr::dummy(ast::ExprKind::Var("x".to_string()))),
        });

        let ty = infer_expr(ast::Expr::dummy(ast::ExprKind::Apply {
            func: Box::new(id),
            args: vec![ast::Expr::dummy(ast::ExprKind::Integer("1".to_string()))],
        }))
        .unwrap();

        assert_eq!(ty, Ty::Con("Integer".to_string()));
    }

    #[test]
    fn infer_let_generalizes() {
        let id_binding = ast::Binding {
            pat: ast::Pattern::dummy(ast::PatternKind::Var("id".to_string())),
            expr: ast::Expr::dummy(ast::ExprKind::Lambda {
                params: vec!["x".to_string()],
                body: Box::new(ast::Expr::dummy(ast::ExprKind::Var("x".to_string()))),
            }),
        };

        let body = ast::Expr::dummy(ast::ExprKind::Tuple(vec![
            ast::Expr::dummy(ast::ExprKind::Apply {
                func: Box::new(ast::Expr::dummy(ast::ExprKind::Var("id".to_string()))),
                args: vec![ast::Expr::dummy(ast::ExprKind::Integer("1".to_string()))],
            }),
            ast::Expr::dummy(ast::ExprKind::Apply {
                func: Box::new(ast::Expr::dummy(ast::ExprKind::Var("id".to_string()))),
                args: vec![ast::Expr::dummy(ast::ExprKind::Bool(true))],
            }),
        ]));

        let ty = infer_expr(ast::Expr::dummy(ast::ExprKind::Let {
            bindings: vec![id_binding],
            body: Box::new(body),
        }))
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
            pat: ast::Pattern::dummy(ast::PatternKind::Tuple(vec![
                ast::Pattern::dummy(ast::PatternKind::Var("a".to_string())),
                ast::Pattern::dummy(ast::PatternKind::Var("b".to_string())),
            ])),
            expr: ast::Expr::dummy(ast::ExprKind::Tuple(vec![
                ast::Expr::dummy(ast::ExprKind::Integer("1".to_string())),
                ast::Expr::dummy(ast::ExprKind::Bool(true)),
            ])),
        };

        let ty = infer_expr(ast::Expr::dummy(ast::ExprKind::Let {
            bindings: vec![b],
            body: Box::new(ast::Expr::dummy(ast::ExprKind::Var("b".to_string()))),
        }))
        .unwrap();

        assert_eq!(ty, Ty::Con("Bool".to_string()));
    }

    #[test]
    fn infer_duplicate_pattern_vars_is_error() {
        let b = ast::Binding {
            pat: ast::Pattern::dummy(ast::PatternKind::Tuple(vec![
                ast::Pattern::dummy(ast::PatternKind::Var("x".to_string())),
                ast::Pattern::dummy(ast::PatternKind::Var("x".to_string())),
            ])),
            expr: ast::Expr::dummy(ast::ExprKind::Tuple(vec![
                ast::Expr::dummy(ast::ExprKind::Integer("1".to_string())),
                ast::Expr::dummy(ast::ExprKind::Integer("2".to_string())),
            ])),
        };

        let _ = infer_expr(ast::Expr::dummy(ast::ExprKind::Let {
            bindings: vec![b],
            body: Box::new(ast::Expr::dummy(ast::ExprKind::Var("x".to_string()))),
        }))
        .unwrap_err();
    }

    #[test]
    fn infer_let_list_pattern() {
        let b = ast::Binding {
            pat: ast::Pattern::dummy(ast::PatternKind::List(vec![
                ast::Pattern::dummy(ast::PatternKind::Var("x".to_string())),
                ast::Pattern::dummy(ast::PatternKind::Var("y".to_string())),
            ])),
            expr: ast::Expr::dummy(ast::ExprKind::List(vec![
                ast::Expr::dummy(ast::ExprKind::Integer("1".to_string())),
                ast::Expr::dummy(ast::ExprKind::Integer("2".to_string())),
            ])),
        };

        let ty = infer_expr(ast::Expr::dummy(ast::ExprKind::Let {
            bindings: vec![b],
            body: Box::new(ast::Expr::dummy(ast::ExprKind::Var("y".to_string()))),
        }))
        .unwrap();

        assert_eq!(ty, Ty::Con("Integer".to_string()));
    }

    #[test]
    fn infer_let_record_pattern() {
        let b = ast::Binding {
            pat: ast::Pattern::dummy(ast::PatternKind::Record(vec![
                (
                    "b".to_string(),
                    ast::Pattern::dummy(ast::PatternKind::Var("y".to_string())),
                ),
                (
                    "a".to_string(),
                    ast::Pattern::dummy(ast::PatternKind::Var("x".to_string())),
                ),
            ])),
            expr: ast::Expr::dummy(ast::ExprKind::Record(vec![
                (
                    "a".to_string(),
                    ast::Expr::dummy(ast::ExprKind::Integer("1".to_string())),
                ),
                ("b".to_string(), ast::Expr::dummy(ast::ExprKind::Bool(true))),
            ])),
        };

        let ty = infer_expr(ast::Expr::dummy(ast::ExprKind::Let {
            bindings: vec![b],
            body: Box::new(ast::Expr::dummy(ast::ExprKind::Var("y".to_string()))),
        }))
        .unwrap();

        assert_eq!(ty, Ty::Con("Bool".to_string()));
    }

    #[test]
    fn infer_record_field_mismatch_is_error() {
        let b = ast::Binding {
            pat: ast::Pattern::dummy(ast::PatternKind::Record(vec![(
                "a".to_string(),
                ast::Pattern::dummy(ast::PatternKind::Wildcard),
            )])),
            expr: ast::Expr::dummy(ast::ExprKind::Record(vec![(
                "b".to_string(),
                ast::Expr::dummy(ast::ExprKind::Bool(true)),
            )])),
        };

        let _ = infer_expr(ast::Expr::dummy(ast::ExprKind::Let {
            bindings: vec![b],
            body: Box::new(ast::Expr::dummy(ast::ExprKind::Unit)),
        }))
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
        assert_eq!(**b, Ty::List(Box::new(Ty::Con("Char".to_string()))));
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

        assert_eq!(
            env.get("a").unwrap(),
            &Scheme::mono(Ty::List(Box::new(Ty::Con("Char".to_string()))))
        );
        assert_eq!(
            env.get("b").unwrap(),
            &Scheme::mono(Ty::List(Box::new(Ty::Con("Char".to_string()))))
        );
        assert_eq!(
            env.get("c").unwrap(),
            &Scheme::mono(Ty::List(Box::new(Ty::Con("Char".to_string()))))
        );
    }

    #[test]
    fn infer_show_data_is_ok() {
        let src = r#"data Maybe a = Nothing | Just a deriving Show
x = show (Just 1)
y = show Nothing
"#;
        let m = crate::parser::parse_module(src).unwrap();
        let env = infer_module(&m).unwrap();

        assert_eq!(
            env.get("x").unwrap(),
            &Scheme::mono(Ty::List(Box::new(Ty::Con("Char".to_string()))))
        );

        let y = env.get("y").unwrap();
        assert_eq!(y.ty, Ty::List(Box::new(Ty::Con("Char".to_string()))));
        assert!(y
            .constraints
            .iter()
            .any(|c| matches!(c, Constraint::Show(_))));
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

        assert!(s
            .constraints
            .iter()
            .any(|c| matches!(c, Constraint::Show(_))));
        assert!(s
            .constraints
            .iter()
            .any(|c| matches!(c, Constraint::ShowRow(_))));
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
        assert!(simplify_constraints(&data_env, &ClassEnv::default(), cs).is_err());
    }

    #[test]
    fn infer_annotation_mismatch_is_error() {
        let _ = infer_expr(ast::Expr::dummy(ast::ExprKind::Annot {
            expr: Box::new(ast::Expr::dummy(ast::ExprKind::Integer("1".to_string()))),
            ty: ast::QualType {
                preds: vec![],
                ty: ast::Type::Bool,
            },
        }))
        .unwrap_err();
    }

    #[test]
    fn infer_annotation_hole_resolves() {
        let ty = infer_expr(ast::Expr::dummy(ast::ExprKind::Annot {
            expr: Box::new(ast::Expr::dummy(ast::ExprKind::Integer("1".to_string()))),
            ty: ast::QualType {
                preds: vec![],
                ty: ast::Type::Hole(None),
            },
        }))
        .unwrap();
        assert_eq!(ty, Ty::Con("Integer".to_string()));
    }

    #[test]
    fn infer_if_expr() {
        let ty = infer_expr(ast::Expr::dummy(ast::ExprKind::If {
            cond: Box::new(ast::Expr::dummy(ast::ExprKind::Bool(true))),
            then_branch: Box::new(ast::Expr::dummy(ast::ExprKind::Integer("1".to_string()))),
            else_branch: Box::new(ast::Expr::dummy(ast::ExprKind::Integer("2".to_string()))),
        }))
        .unwrap();
        assert_eq!(ty, Ty::Con("Integer".to_string()));
    }

    #[test]
    fn infer_if_mismatch_is_error() {
        let _ = infer_expr(ast::Expr::dummy(ast::ExprKind::If {
            cond: Box::new(ast::Expr::dummy(ast::ExprKind::Bool(true))),
            then_branch: Box::new(ast::Expr::dummy(ast::ExprKind::Integer("1".to_string()))),
            else_branch: Box::new(ast::Expr::dummy(ast::ExprKind::Bool(false))),
        }))
        .unwrap_err();
    }

    #[test]
    fn infer_case_expr() {
        let x_bind = ast::Binding {
            pat: ast::Pattern::dummy(ast::PatternKind::Var("x".to_string())),
            expr: ast::Expr::dummy(ast::ExprKind::Integer("1".to_string())),
        };

        let ty = infer_expr(ast::Expr::dummy(ast::ExprKind::Let {
            bindings: vec![x_bind],
            body: Box::new(ast::Expr::dummy(ast::ExprKind::Case {
                expr: Box::new(ast::Expr::dummy(ast::ExprKind::Var("x".to_string()))),
                arms: vec![
                    ast::CaseArm {
                        pat: ast::Pattern::dummy(ast::PatternKind::Literal(ast::Expr::dummy(
                            ast::ExprKind::Integer("0".to_string()),
                        ))),
                        guard: None,
                        body: ast::Expr::dummy(ast::ExprKind::Bool(true)),
                    },
                    ast::CaseArm {
                        pat: ast::Pattern::dummy(ast::PatternKind::Wildcard),
                        guard: None,
                        body: ast::Expr::dummy(ast::ExprKind::Bool(false)),
                    },
                ],
            })),
        }))
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
            pat: ast::Pattern::dummy(ast::PatternKind::Var("x".to_string())),
            expr: ast::Expr::dummy(ast::ExprKind::Integer("1".to_string())),
        };

        let _ = infer_expr(ast::Expr::dummy(ast::ExprKind::Let {
            bindings: vec![x_bind],
            body: Box::new(ast::Expr::dummy(ast::ExprKind::Case {
                expr: Box::new(ast::Expr::dummy(ast::ExprKind::Var("x".to_string()))),
                arms: vec![
                    ast::CaseArm {
                        pat: ast::Pattern::dummy(ast::PatternKind::Literal(ast::Expr::dummy(
                            ast::ExprKind::Integer("0".to_string()),
                        ))),
                        guard: None,
                        body: ast::Expr::dummy(ast::ExprKind::Bool(true)),
                    },
                    ast::CaseArm {
                        pat: ast::Pattern::dummy(ast::PatternKind::Wildcard),
                        guard: None,
                        body: ast::Expr::dummy(ast::ExprKind::Integer("1".to_string())),
                    },
                ],
            })),
        }))
        .unwrap_err();
    }
}
