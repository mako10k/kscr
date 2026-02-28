//! Type checking and elaboration scaffolding.
//!
//! Policy (docs):
//! - Surface numeric types: Integer (arbitrary precision) and Float64.
//! - Backend/IR numeric types are LLVM-aligned (i32/i64/f32/f64...).
//! - Pure IR subtyping allows only integer widening (iN <: iM); float widening is NOT subtyping.
//! - Potentially lossy conversions happen only at boundaries as checked casts.

use crate::{ast, error::Error, parser, Result};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
mod stdlib_cache;
mod toposort;
mod typeclass_dict_passing_common;
mod typeclass_dict_passing_rewrite;

/// Global configuration for KSIF rebuild policy.
#[derive(Debug, Clone, Copy, Default)]
pub struct KsifRebuildPolicy {
    /// Force rebuild all .ksif files regardless of hash validity.
    pub force_rebuild: bool,
    /// If true, when forcing rebuild, only rebuild the target module, not its dependencies.
    pub suppress_recursive_rebuild: bool,
}

static KSIF_REBUILD_POLICY: OnceLock<std::sync::Mutex<KsifRebuildPolicy>> = OnceLock::new();

/// Set the global KSIF rebuild policy.
pub fn set_ksif_rebuild_policy(policy: KsifRebuildPolicy) {
    KSIF_REBUILD_POLICY
        .get_or_init(|| std::sync::Mutex::new(KsifRebuildPolicy::default()))
        .lock()
        .unwrap()
        .clone_from(&policy);
}

/// Get the current KSIF rebuild policy.
pub(crate) fn get_ksif_rebuild_policy() -> KsifRebuildPolicy {
    *KSIF_REBUILD_POLICY
        .get_or_init(|| std::sync::Mutex::new(KsifRebuildPolicy::default()))
        .lock()
        .unwrap()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedModule {
    pub module: ast::Module,
    pub inferred: HashMap<String, Scheme>,
    pub docs: HashMap<String, String>,
}

fn collect_toplevel_docs(module: &ast::Module) -> HashMap<String, String> {
    use ast::{Item, PatternKind};

    let mut out = HashMap::new();
    for it in &module.items {
        match it {
            Item::Binding(b) => {
                let Some(doc) = &b.doc else { continue };
                let PatternKind::Var(name) = &b.pat.kind else {
                    continue;
                };
                out.insert(name.clone(), doc.clone());
            }
            Item::TypeAlias(ta) => {
                let Some(doc) = &ta.doc else { continue };
                out.insert(ta.name.clone(), doc.clone());
            }
            Item::DataDecl(d) => {
                if let Some(doc) = &d.doc {
                    out.insert(d.name.clone(), doc.clone());
                }

                // Constructor docs: prefer ctor-level docs when present;
                // otherwise fall back to the parent data decl docs (if any).
                for ctor in &d.ctors {
                    if let Some(ctor_doc) = ctor.doc.as_ref().or(d.doc.as_ref()) {
                        out.insert(ctor.name.clone(), ctor_doc.clone());
                    }
                }
            }
            Item::ClassDecl(c) => {
                let Some(doc) = &c.doc else { continue };
                out.insert(c.name.clone(), doc.clone());
            }
            Item::Import(_) | Item::Export(_) | Item::Fixity(_) | Item::InstanceDecl(_) => {}
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DefSite {
    path: PathBuf,
    span: ast::Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DefLoc {
    path: PathBuf,
    line: usize,
    col: usize,
}

#[derive(Debug, Default, Clone)]
struct DefSiteIndex {
    /// Qualified type constructor name -> definition site.
    /// Example: `Prelude.Maybe` -> (stdlib/Prelude.ks, span-of-data-decl)
    type_ctor: HashMap<String, DefSite>,

    /// Qualified value constructor name -> definition site.
    /// Example: `Prelude.Just` -> (stdlib/Prelude.ks, span-of-ctor)
    value_ctor: HashMap<String, DefSite>,

    /// Qualified type alias name -> definition site.
    /// Example: `Prelude.String` -> (stdlib/Prelude.ks, span-of-type-alias)
    type_alias: HashMap<String, DefSite>,
}

// (reserved) definition-location helpers will be added once def-site evidence is wired
// into unify/unknown-name diagnostics.

/// Symbol kind for module exports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    /// Value-level binding (function, constant, pattern-bound name).
    Value,
    /// Type constructor (from data declaration).
    Type,
    /// Type alias.
    TypeAlias,
    /// Data constructor.
    Ctor,
    /// Type class.
    Class,
    /// Instance declaration (not typically exported by name, but tracked for completeness).
    Instance,
}

/// Per-module export table mapping exported names to their symbol kinds.
///
/// This structure is keyed by symbol name and tracks what kind of symbol each export represents.
/// In Stage 1, we use this internally but continue to provide the old HashSet<String> interface
/// to existing callers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportTable {
    /// Map from exported symbol name to its kind.
    entries: HashMap<String, SymbolKind>,
}

impl ExportTable {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    fn insert(&mut self, name: String, kind: SymbolKind) {
        self.entries.insert(name, kind);
    }

    fn extend(&mut self, names: impl IntoIterator<Item = (String, SymbolKind)>) {
        self.entries.extend(names);
    }

    /// Derive the old HashSet<String> view for backward compatibility.
    pub fn as_name_set(&self) -> HashSet<String> {
        self.entries.keys().cloned().collect()
    }
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

fn pretty_ty_pair(a: &Ty, b: &Ty) -> (String, String) {
    #[derive(Clone)]
    struct PrettyTy<'a> {
        ty: &'a Ty,
        vars: &'a HashMap<u32, String>,
    }

    impl fmt::Display for PrettyTy<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            fmt_ty_prec(f, self.ty, 0, self.vars)
        }
    }

    let mut vars: HashMap<u32, String> = HashMap::new();
    assign_ty_var_names(a, &mut vars);
    assign_ty_var_names(b, &mut vars);
    (
        format!("{}", PrettyTy { ty: a, vars: &vars }),
        format!("{}", PrettyTy { ty: b, vars: &vars }),
    )
}

fn assign_ty_var_names(ty: &Ty, vars: &mut HashMap<u32, String>) {
    fn next_name(i: usize) -> String {
        // a..z, a1..z1, a2..z2, ...
        let ch = (b'a' + (i % 26) as u8) as char;
        let suffix = i / 26;
        if suffix == 0 {
            ch.to_string()
        } else {
            format!("{ch}{suffix}")
        }
    }

    fn go(t: &Ty, vars: &mut HashMap<u32, String>, next: &mut usize) {
        match t {
            Ty::Var(v) => {
                if !vars.contains_key(v) {
                    let name = next_name(*next);
                    *next += 1;
                    vars.insert(*v, name);
                }
            }
            Ty::Con(_) => {}
            Ty::List(t) => go(t, vars, next),
            Ty::Tuple(ts) => ts.iter().for_each(|t| go(t, vars, next)),
            Ty::Record(fs) => fs.iter().for_each(|(_, t)| go(t, vars, next)),
            Ty::RecordOpen(fs, rest) => {
                fs.iter().for_each(|(_, t)| go(t, vars, next));
                go(rest, vars, next);
            }
            Ty::App { head, args } => {
                go(head, vars, next);
                args.iter().for_each(|t| go(t, vars, next));
            }
            Ty::Func(a, b) => {
                go(a, vars, next);
                go(b, vars, next);
            }
        }
    }

    let mut next = vars.len();
    go(ty, vars, &mut next);
}

#[derive(Debug)]
pub struct InferCtx {
    next_var: u32,
    class_env: ClassEnvIndex,
    /// Full class env for solving class constraints inside local `let`/`where`.
    ///
    /// This is cloned into an `Arc` at module inference entrypoints.
    full_class_env: std::sync::Arc<ClassEnv>,
}

impl Default for InferCtx {
    fn default() -> Self {
        Self {
            next_var: 0,
            class_env: ClassEnvIndex::default(),
            full_class_env: std::sync::Arc::new(ClassEnv::default()),
        }
    }
}

#[derive(Debug, Default, Clone)]
struct NameHints {
    /// Unqualified type constructor name -> resolved/qualified name.
    /// Example: `Maybe` -> `Prelude.Maybe`.
    type_ctor: HashMap<UnqualName, QualName>,

    /// Unqualified value constructor name -> resolved/qualified name.
    /// Example: `Just` -> `Prelude.Just`.
    value_ctor: HashMap<UnqualName, QualName>,

    /// Unqualified type alias name -> resolved/qualified name.
    /// Example: `String` -> `Prelude.String`.
    type_alias: HashMap<UnqualName, QualName>,

    /// Best-effort alias chain within the current module: `type Text = String` yields
    /// `Text -> String`.
    ///
    /// This intentionally only tracks the simplest shape (RHS is a type var/name), because
    /// it's just used to enrich unify diagnostics.
    type_alias_rhs_unqual: HashMap<UnqualName, UnqualName>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct ModuleName(String);

impl ModuleName {}

impl fmt::Display for ModuleName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct UnqualName(String);

impl UnqualName {
    fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UnqualName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct QualName {
    module: ModuleName,
    name: UnqualName,
}

impl QualName {
    fn parse(s: &str) -> Option<Self> {
        let (m, n) = s.rsplit_once('.')?;
        Some(Self {
            module: ModuleName(m.to_string()),
            name: UnqualName(n.to_string()),
        })
    }

    fn as_key(&self) -> String {
        format!("{}.{}", self.module.0, self.name.0)
    }
}

impl fmt::Display for QualName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_key())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum DefHintKind {
    TypeCtor,
    TypeAlias,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DefHint {
    kind: DefHintKind,
    /// Unqualified name that appeared in the type/error context.
    unqualified: UnqualName,
    /// Resolved qualified name (e.g. `Prelude.Maybe`).
    qualified: QualName,
}

thread_local! {
    #[allow(clippy::missing_const_for_thread_local)]
    static TL_NAME_HINTS: RefCell<NameHints> = RefCell::new(NameHints::default());
}

thread_local! {
    static TL_DEF_EVIDENCE: RefCell<Option<DefEvidenceCtx>> = const { RefCell::new(None) };
}

thread_local! {
    // Type alias usages encountered during AST lowering/expansion.
    // Key: unqualified alias name (e.g. `String`). Value: resolved qualified name (e.g. `Prelude.String`).
    static TL_ALIAS_EVIDENCE: RefCell<Vec<(UnqualName, QualName)>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone)]
struct DefEvidenceCtx {
    def_sites: DefSiteIndex,
    sources: HashMap<PathBuf, String>,
}

impl DefEvidenceCtx {
    fn from_loader(loader: &ModuleLoader) -> Self {
        Self {
            def_sites: loader.def_sites.clone(),
            sources: loader.sources.clone(),
        }
    }

    fn def_loc(&self, site: &DefSite) -> Option<DefLoc> {
        let src = self.sources.get(&site.path)?;
        let start_off = site.span.start.min(src.len());

        let mut line: usize = 1;
        let mut last_nl: usize = 0;
        for (i, ch) in src.char_indices() {
            if i >= start_off {
                break;
            }
            if ch == '\n' {
                line += 1;
                last_nl = i + 1;
            }
        }
        let col = src[last_nl..start_off].chars().count() + 1;
        Some(DefLoc {
            path: site.path.clone(),
            line,
            col,
        })
    }

    fn site_for_hint(&self, hint: &DefHint) -> Option<DefSite> {
        let key = hint.qualified.as_key();
        match hint.kind {
            DefHintKind::TypeCtor => self.def_sites.type_ctor.get(&key).cloned(),
            DefHintKind::TypeAlias => self.def_sites.type_alias.get(&key).cloned(),
        }
    }
}

struct WithDefEvidence;

impl WithDefEvidence {
    fn run<T>(ctx: DefEvidenceCtx, f: impl FnOnce() -> T) -> T {
        TL_DEF_EVIDENCE.with(|slot| {
            let prev = slot.replace(Some(ctx));
            let out = f();
            slot.replace(prev);
            out
        })
    }
}

struct WithAliasEvidence;

impl WithAliasEvidence {
    fn run<T>(f: impl FnOnce() -> T) -> T {
        TL_ALIAS_EVIDENCE.with(|slot| {
            let prev = std::mem::take(&mut *slot.borrow_mut());
            let out = f();
            *slot.borrow_mut() = prev;
            out
        })
    }
}

impl InferCtx {
    pub fn fresh(&mut self) -> Ty {
        let v = self.next_var;
        self.next_var += 1;
        Ty::Var(v)
    }
}

#[derive(Debug, Default, Clone)]
struct ClassEnvIndex {
    /// method name -> scheme (overloaded function)
    methods_by_name: HashMap<String, Scheme>,
}

pub type Subst = HashMap<u32, Ty>;

pub fn unify(a: Ty, b: Ty) -> Result<Subst> {
    let mut subst = Subst::new();
    unify_in(&mut subst, a, b)?;
    Ok(subst)
}

fn unify_dbg(a: Ty, b: Ty, ctx: &str) -> Result<Subst> {
    unify(a.clone(), b.clone()).map_err(|e| {
        let (a_pretty, b_pretty) = pretty_ty_pair(&a, &b);
        let hint = TL_NAME_HINTS.with(|h| format_unify_name_hints(&a, &b, &h.borrow()));
        let mut evidence = TL_NAME_HINTS.with(|h| collect_unify_def_hints(&a, &b, &h.borrow()));
        TL_ALIAS_EVIDENCE.with(|slot| {
            for (unqual, qual) in slot.borrow().iter().cloned() {
                evidence.push(DefHint {
                    kind: DefHintKind::TypeAlias,
                    unqualified: unqual,
                    qualified: qual,
                });
            }
        });
        // Fallback: if either side is `[Char]`, emit evidence for the conventional `String`
        // alias when it is in scope. Alias nodes are erased from `Ty`, so without this, we
        // can miss the def-site note for common `String` mismatches.
        if ty_has_list_char(&a) || ty_has_list_char(&b) {
            TL_NAME_HINTS.with(|h| {
                let hints = h.borrow();
                let qual = hints
                    .type_alias
                    .get(&UnqualName("String".to_string()))
                    .cloned()
                    .unwrap_or_else(|| QualName {
                        module: ModuleName("Prelude".to_string()),
                        name: UnqualName("String".to_string()),
                    });
                evidence.push(DefHint {
                    kind: DefHintKind::TypeAlias,
                    unqualified: UnqualName("String".to_string()),
                    qualified: qual,
                });
            });
        }
        fn kind_rank(k: &DefHintKind) -> u8 {
            match k {
                DefHintKind::TypeCtor => 0,
                DefHintKind::TypeAlias => 2,
            }
        }
        evidence.sort_by(|a, b| {
            (
                kind_rank(&a.kind),
                a.unqualified.as_str(),
                a.qualified.as_key(),
            )
            .cmp(&(
                kind_rank(&b.kind),
                b.unqualified.as_str(),
                b.qualified.as_key(),
            ))
        });
        evidence.dedup();
        let (evidence_note, evidence_missing) = TL_DEF_EVIDENCE.with(|slot| {
            let binding = slot.borrow();
            let Some(ev) = binding.as_ref() else {
                return (String::new(), true);
            };
            let mut lines: Vec<String> = Vec::new();
            for h in evidence {
                let Some(site) = ev.site_for_hint(&h) else {
                    continue;
                };
                let Some(loc) = ev.def_loc(&site) else {
                    continue;
                };
                let kind = match h.kind {
                    DefHintKind::TypeCtor => "type ctor",
                    DefHintKind::TypeAlias => "type alias",
                };
                lines.push(format!(
                    "note: {kind} `{}` resolves to `{}` at {}:{}:{}",
                    h.unqualified,
                    h.qualified,
                    loc.path.display(),
                    loc.line,
                    loc.col
                ));
            }
            if lines.is_empty() {
                (String::new(), true)
            } else {
                (format!("\n{}", lines.join("\n")), false)
            }
        });
        if evidence_missing {
            // Fallback: if we cannot resolve alias def-sites via loader index (e.g. KSIF path or
            // qualified-name mismatch), still show *local* alias definitions in the current file.
            // This satisfies the “prefer A, and C if possible” requirement.
            let fallback = TL_DEF_EVIDENCE.with(|slot| {
                let binding = slot.borrow();
                let Some(ev) = binding.as_ref() else {
                    return String::new();
                };
                // Collect unique paths (we only need the entry file in practice).
                let mut paths: Vec<PathBuf> = ev.sources.keys().cloned().collect();
                paths.sort();
                paths.dedup();

                let mut lines: Vec<String> = Vec::new();
                for path in paths {
                    let Some(src) = ev.sources.get(&path) else {
                        continue;
                    };
                    for raw_line in src.lines() {
                        // Very small heuristic: capture `type <Name> = ...` definitions.
                        let line = raw_line.trim_start();
                        let Some(rest) = line.strip_prefix("type ") else {
                            continue;
                        };
                        let Some((name, _rhs)) = rest.split_once('=') else {
                            continue;
                        };
                        let name = name.trim();
                        if name.is_empty() {
                            continue;
                        }
                        // Only unqualified names.
                        if name.contains('.') {
                            continue;
                        }
                        // Column is best-effort (1-based) at the start of `type`.
                        let line_no = src
                            .lines()
                            .position(|l| l == raw_line)
                            .map(|i| i + 1)
                            .unwrap_or(1);
                        let col = raw_line.chars().take_while(|c| c.is_whitespace()).count() + 1;
                        lines.push(format!(
                            "note: type alias `{}` is defined locally at {}:{}:{}",
                            name,
                            path.display(),
                            line_no,
                            col
                        ));
                    }
                }

                if lines.is_empty() {
                    String::new()
                } else {
                    format!("\n{}", lines.join("\n"))
                }
            });

            // Additional (best-effort) C: if we saw `type Text = String`, also emit the def-site
            // evidence for `String`'s canonical alias (usually `Prelude.String`) when available.
            let chain_note = TL_NAME_HINTS.with(|h| {
                let hints = h.borrow();
                // Look for any local alias whose RHS is `String`.
                let mut out = String::new();
                for (lhs, rhs) in hints.type_alias_rhs_unqual.iter() {
                    if rhs.as_str() != "String" {
                        continue;
                    }
                    // If String is in the type_alias map, use that; otherwise default to Prelude.String
                    // (since Prelude is implicitly imported in most modules).
                    let q0 = hints.type_alias.get(&UnqualName("String".to_string())).cloned().unwrap_or_else(|| {
                        QualName {
                            module: ModuleName("Prelude".to_string()),
                            name: UnqualName("String".to_string()),
                        }
                    });
                    // Prefer canonical stdlib alias in notes.
                    let q = if q0.to_string() == "Main.String" {
                        QualName {
                            module: ModuleName("Prelude".to_string()),
                            name: UnqualName("String".to_string()),
                        }
                    } else {
                        q0.clone()
                    };
                    // Avoid duplicating if it was already printed.
                    out.push_str(&format!(
                        "\nnote: type alias `{}` expands to `{}`",
                        lhs,
                        q
                    ));

                    // If we also have def-evidence context, try to attach the canonical alias's
                    // definition location as concrete file:line:col.
                    let loc_note = TL_DEF_EVIDENCE.with(|slot| {
                        let binding = slot.borrow();
                        let Some(ev) = binding.as_ref() else {
                            return String::new();
                        };
                        let hint = DefHint {
                            kind: DefHintKind::TypeAlias,
                            unqualified: UnqualName("String".to_string()),
                            qualified: q.clone(),
                        };
                        let Some(site) = ev.site_for_hint(&hint) else {
                            return String::new();
                        };
                        let Some(loc) = ev.def_loc(&site) else {
                            return String::new();
                        };
                        format!(
                            "\nnote: type alias `{}` resolves to `{}` at {}:{}:{}",
                            hint.unqualified,
                            hint.qualified,
                            loc.path.display(),
                            loc.line,
                            loc.col
                        )
                    });
                    out.push_str(&loc_note);

                    // Last resort: scan loaded sources for `type String =` even if we couldn't
                    // resolve through `def_sites` (e.g. ksif import path mismatch).
                    if loc_note.is_empty() {
                        // Hard fallback: read stdlib Prelude directly.
                        let scan_note = (|| {
                            let prelude_path = stdlib_root().join("Prelude.ks");
                            let Ok(src) = std::fs::read_to_string(&prelude_path) else {
                                return String::new();
                            };
                            for (i, raw_line) in src.lines().enumerate() {
                                let line = raw_line.trim_start();
                                let Some(rest) = line.strip_prefix("type ") else {
                                    continue;
                                };
                                let Some((name, _rhs)) = rest.split_once('=') else {
                                    continue;
                                };
                                if name.trim() != "String" {
                                    continue;
                                }
                                let col = raw_line
                                    .chars()
                                    .take_while(|c| c.is_whitespace())
                                    .count()
                                    + 1;
                                return format!(
                                    "\nnote: type alias `String` resolves to `{}` at {}:{}:{}",
                                    q,
                                    prelude_path.display(),
                                    i + 1,
                                    col
                                );
                            }
                            String::new()
                        })();
                        out.push_str(&scan_note);
                    }
                }
                out
            });
            // Prefer structured evidence if present; otherwise append fallback.
            // evidence_note is empty here.
            if !fallback.is_empty() || !chain_note.is_empty() {
                return Error::msg(format!(
                    "{e} (unify goal: {ctx}: here = {a_pretty}, other = {b_pretty}){hint}{fallback}{chain_note}"
                ));
            }
        }
        if std::env::var("KSCR_DEBUG_ALIAS_EVIDENCE").ok().as_deref() == Some("1") {
            eprintln!(
                "[KSCR_DEBUG_ALIAS_EVIDENCE] unify_dbg evidence_note: {}",
                if evidence_note.is_empty() { "<empty>" } else { "<non-empty>" }
            );
        }
        // Always attach the concrete unification goal. This dramatically improves debugging
        // for import/name-resolution issues where the root cause is not near the reported span.
        // Keep this compact; callers add spans/contexts.
        Error::msg(format!(
            "{e} (unify goal: {ctx}: here = {a_pretty}, other = {b_pretty}){hint}{evidence_note}"
        ))
    })
}

fn ty_has_list_char(t: &Ty) -> bool {
    matches!(t, Ty::List(inner) if matches!(inner.as_ref(), Ty::Con(c) if c == "Char"))
}

fn collect_unify_def_hints(a: &Ty, b: &Ty, hints: &NameHints) -> Vec<DefHint> {
    let mut out: Vec<DefHint> = Vec::new();

    for t in [a, b] {
        if let Some(unqual) = ty_has_unqualified_type_ctor(t) {
            let un = UnqualName(unqual.to_string());
            if let Some(qual) = hints.type_ctor.get(&un) {
                out.push(DefHint {
                    kind: DefHintKind::TypeCtor,
                    unqualified: un,
                    qualified: qual.clone(),
                });
            }
        }

        if let Some(unqual) = ty_has_unqualified_type_alias(t) {
            let un = UnqualName(unqual.to_string());
            if let Some(qual) = hints.type_alias.get(&un) {
                out.push(DefHint {
                    kind: DefHintKind::TypeAlias,
                    unqualified: un,
                    qualified: qual.clone(),
                });
            }
        }
    }

    fn kind_rank(k: &DefHintKind) -> u8 {
        match k {
            DefHintKind::TypeCtor => 0,
            DefHintKind::TypeAlias => 2,
        }
    }

    out.sort_by(|a, b| {
        (
            kind_rank(&a.kind),
            a.unqualified.as_str(),
            a.qualified.as_key(),
        )
            .cmp(&(
                kind_rank(&b.kind),
                b.unqualified.as_str(),
                b.qualified.as_key(),
            ))
    });
    out.dedup();
    out
}

fn collect_unqualified_name_hints_from_imported(module: &ast::Module) -> NameHints {
    // After `.ksif` import forwarders are injected, imported modules are referenced as
    // qualified names (e.g. `Prelude.Maybe`) and unqualified forwarders may be emitted as
    // `type Maybe = Prelude.Maybe ...`. Collect both as lightweight “definition origin” hints.
    let mut out = NameHints::default();
    for it in &module.items {
        match it {
            ast::Item::TypeAlias(ta) => {
                // Only accept the simplest alias shape: `X = QUAL.X`.
                // (unqualified forwarders emitted for imported modules)
                if let ast::Type::Var(rhs) = &ta.ty {
                    if rhs.ends_with(&format!(".{}", ta.name)) {
                        if ta
                            .name
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_ascii_uppercase())
                        {
                            if let Some(q) = QualName::parse(rhs) {
                                out.type_ctor.insert(UnqualName(ta.name.clone()), q);
                            }
                        } else if let Some(q) = QualName::parse(rhs) {
                            out.type_alias.insert(UnqualName(ta.name.clone()), q);
                        }
                    }
                }

                // Track local alias chain: `type Text = String`.
                if let ast::Type::Var(rhs) = &ta.ty {
                    // Only keep unqualified RHS names.
                    if !rhs.contains('.') {
                        out.type_alias_rhs_unqual
                            .insert(UnqualName(ta.name.clone()), UnqualName(rhs.clone()));
                    }
                }

                // Type aliases declared in the module itself can be referenced unqualified.
                // Example: `type String = [Char]` in `Prelude` makes `String` resolve to
                // `Prelude.String` (not `Main.Prelude.String`).
                if ta
                    .name
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_uppercase())
                {
                    let mname = module.name.as_deref().unwrap_or("Main");
                    out.type_alias.insert(
                        UnqualName(ta.name.clone()),
                        QualName {
                            module: ModuleName(mname.to_string()),
                            name: UnqualName(ta.name.clone()),
                        },
                    );
                }
            }
            ast::Item::DataDecl(dd) => {
                // Data decls can be qualified (e.g. `data Prelude.Maybe a = ...`) or unqualified
                // (e.g. `data Maybe a = ...` inside `module Prelude`).
                // Collect `Maybe -> Prelude.Maybe` style hints.
                if let Some((qual, base)) = dd.name.rsplit_once('.') {
                    let base = base.to_string();
                    if base.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                        let k = UnqualName(base.clone());
                        let q = QualName {
                            module: ModuleName(qual.to_string()),
                            name: UnqualName(base.clone()),
                        };
                        if q.module.0 == "Prelude" {
                            out.type_ctor.insert(k, q);
                        } else {
                            out.type_ctor.entry(k).or_insert(q);
                        }
                    }
                } else if dd
                    .name
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_uppercase())
                {
                    let mname = module.name.as_deref().unwrap_or("Main");
                    let k = UnqualName(dd.name.clone());
                    let q = QualName {
                        module: ModuleName(mname.to_string()),
                        name: UnqualName(dd.name.clone()),
                    };
                    if mname == "Prelude" {
                        out.type_ctor.insert(k, q);
                    } else {
                        out.type_ctor.entry(k).or_insert(q);
                    }
                }

                // Constructors (values): map `Just` -> `Prelude.Just`.
                for ctor in &dd.ctors {
                    if let Some((qual, base)) = ctor.name.rsplit_once('.') {
                        if base.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                            out.value_ctor
                                .entry(UnqualName(base.to_string()))
                                .or_insert_with(|| QualName {
                                    module: ModuleName(qual.to_string()),
                                    name: UnqualName(base.to_string()),
                                });
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Also seed hints from stdlib Prelude so diagnostics can point to canonical names even when
    // Prelude isn't explicitly imported in the entry module.
    if let Ok(stdlib_root) = try_stdlib_root() {
        let prelude_path = stdlib_root.join("Prelude.ks");
        if let Ok(Some(prelude_mod)) = stdlib_cache::load_ast_stdlib_cached(&prelude_path) {
            for it in &prelude_mod.items {
                match it {
                    ast::Item::TypeAlias(ta) => {
                        if let ast::Type::Var(rhs) = &ta.ty {
                            if rhs.ends_with(&format!(".{}", ta.name)) {
                                if ta
                                    .name
                                    .chars()
                                    .next()
                                    .is_some_and(|c| c.is_ascii_uppercase())
                                {
                                    if let Some(q) = QualName::parse(rhs) {
                                        out.type_ctor
                                            .entry(UnqualName(ta.name.clone()))
                                            .or_insert(q);
                                    }
                                } else if let Some(q) = QualName::parse(rhs) {
                                    out.type_alias
                                        .entry(UnqualName(ta.name.clone()))
                                        .or_insert(q);
                                }
                            }
                        }
                    }
                    ast::Item::DataDecl(dd) => {
                        if let Some((qual, base)) = dd.name.rsplit_once('.') {
                            let base = base.to_string();
                            if base.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                                out.type_ctor
                                    .entry(UnqualName(base.clone()))
                                    .or_insert_with(|| QualName {
                                        module: ModuleName(qual.to_string()),
                                        name: UnqualName(base),
                                    });
                            }
                        } else if dd
                            .name
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_ascii_uppercase())
                        {
                            let mname = prelude_mod.name.as_deref().unwrap_or("Main");
                            out.type_ctor
                                .entry(UnqualName(dd.name.clone()))
                                .or_insert_with(|| QualName {
                                    module: ModuleName(mname.to_string()),
                                    name: UnqualName(dd.name.clone()),
                                });
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    out
}

fn ty_has_unqualified_type_ctor(t: &Ty) -> Option<&str> {
    match t {
        Ty::Con(name) => {
            if name.contains('.') {
                None
            } else if name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                Some(name.as_str())
            } else {
                None
            }
        }
        Ty::List(t) => ty_has_unqualified_type_ctor(t),
        Ty::Tuple(ts) => ts.iter().find_map(ty_has_unqualified_type_ctor),
        Ty::Record(fs) => fs.iter().find_map(|(_, t)| ty_has_unqualified_type_ctor(t)),
        Ty::RecordOpen(fs, rest) => fs
            .iter()
            .find_map(|(_, t)| ty_has_unqualified_type_ctor(t))
            .or_else(|| ty_has_unqualified_type_ctor(rest)),
        Ty::App { head, args } => ty_has_unqualified_type_ctor(head)
            .or_else(|| args.iter().find_map(ty_has_unqualified_type_ctor)),
        Ty::Func(a, b) => {
            ty_has_unqualified_type_ctor(a).or_else(|| ty_has_unqualified_type_ctor(b))
        }
        Ty::Var(_) => None,
    }
}

fn ty_has_unqualified_type_alias(_t: &Ty) -> Option<&str> {
    // NOTE: `Ty` currently does not represent aliases explicitly.
    // We'll emit alias evidence from places that *do* know alias usage (e.g. type lowering).
    None
}

fn format_unify_name_hints(a: &Ty, b: &Ty, hints: &NameHints) -> String {
    // Keep this short: show at most one ctor name hint.
    for name in [
        ty_has_unqualified_type_ctor(a),
        ty_has_unqualified_type_ctor(b),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(q) = hints.type_ctor.get(&UnqualName(name.to_string())) {
            return format!(" (hint: type ctor `{name}` resolves to `{}`)", q);
        }
    }
    String::new()
}

fn format_unknown_ctor_name_hint(name: &str, hints: &NameHints) -> String {
    // Known dotted names often fail due to missing qualified import/alias.
    if name.contains('.') {
        return " (hint: check qualified imports / aliasing)".to_string();
    }

    // For unqualified type ctor names, try to point to the resolved qualified name.
    if name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
        if let Some(q) = hints.type_ctor.get(&UnqualName(name.to_string())) {
            return format!(" (hint: `{name}` resolves to `{}`)", q);
        }
    }

    String::new()
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
        class: ast::ClassId,
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
            write!(f, "{} ", class.name)?;
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct EnvEntry {
    scheme: Scheme,
    def_site: Option<DefSite>,
}

type TypeEnv = HashMap<String, EnvEntry>;

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

fn ftv_env(env: &TypeEnv) -> HashSet<u32> {
    env.values().flat_map(|e| ftv_scheme(&e.scheme)).collect()
}

fn ftv_env_applied_from_ftv(subst: &Subst, env_ftv: &HashSet<u32>) -> HashSet<u32> {
    let mut out: HashSet<u32> = HashSet::new();
    for v in env_ftv {
        if let Some(t) = subst.get(v) {
            out.extend(ftv_ty(&apply(subst, t.clone())));
        } else {
            out.insert(*v);
        }
    }
    out
}

fn generalize_qual_with_env_ftv(
    env_ftv: &HashSet<u32>,
    constraints: Vec<Constraint>,
    ty: Ty,
) -> Scheme {
    let mut ftv = ftv_ty(&ty);
    for c in &constraints {
        ftv.extend(ftv_constraint(c));
    }
    let mut vars: Vec<u32> = ftv.difference(env_ftv).copied().collect();
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

#[allow(dead_code)]
fn apply_env(subst: &Subst, env: &TypeEnv) -> TypeEnv {
    env.iter()
        .map(|(k, v)| {
            (
                k.clone(),
                EnvEntry {
                    scheme: apply_scheme(subst, &v.scheme),
                    def_site: v.def_site.clone(),
                },
            )
        })
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

    let (s_e, _cs_e, t_e) = infer_expr_in(cx, data_env, subst, env, e.clone())?;
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
    let entry = env.get(name).ok_or_else(|| {
        let hint = TL_NAME_HINTS.with(|h| format_unknown_ctor_name_hint(name, &h.borrow()));
        Error::msg(format!("unknown constructor: {name}{hint}"))
    })?;
    let scheme = apply_scheme(subst, &entry.scheme);
    let mut ctor_ty = instantiate(cx, &scheme);

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
        PatternKind::Tuple(ps) => {
            infer_pat_tuple(cx, data_env, subst, env, ps, binds, seen, cs_out)
        }
        PatternKind::List(ps) => infer_pat_list(cx, data_env, subst, env, ps, binds, seen, cs_out),
        PatternKind::Record(fields) => {
            infer_pat_record(cx, data_env, subst, env, fields, binds, seen, cs_out)
        }
        PatternKind::RecordLoose(fields, rest_name) => infer_pat_record_loose(
            cx, data_env, subst, env, fields, rest_name, pat, binds, seen, cs_out,
        ),
        PatternKind::Cons(hd, tl) => {
            infer_pat_cons(cx, data_env, subst, env, hd, tl, binds, seen, cs_out)
        }
        PatternKind::Or(a, b) => infer_pat_or(cx, data_env, subst, env, a, b, binds, seen, cs_out),
        PatternKind::As(name, p) => {
            infer_pat_as(cx, data_env, subst, env, name, p, pat, binds, seen, cs_out)
        }
        PatternKind::View(p, e) => {
            infer_pat_view(cx, data_env, subst, env, p, e, binds, seen, cs_out)
        }
        PatternKind::Constructor { name, args } => infer_pat_constructor(
            cx,
            data_env,
            subst,
            env,
            &name.qualified_text(),
            args,
            binds,
            seen,
            cs_out,
        ),
    }
}

pub fn infer_expr(expr: ast::Expr) -> Result<Ty> {
    let mut cx = InferCtx::default();
    let data_env = DataEnv::new();
    let class_env = ClassEnv::default();
    let env = TypeEnv::new();
    let (s, cs, t) = infer_expr_in(&mut cx, &data_env, &Subst::new(), &env, expr)?;
    let _ = simplify_constraints(&data_env, &class_env, apply_constraints(&s, cs))?;
    Ok(apply(&s, t))
}

pub fn infer_in_module(module: &ast::Module, expr: ast::Expr) -> Result<Ty> {
    let mut cx = InferCtx::default();
    let data_env = collect_data_env(module);
    let class_env = ClassEnv::default();
    let env = collect_ctor_env(&mut cx, module)?;
    let (s, cs, t) = infer_expr_in(&mut cx, &data_env, &Subst::new(), &env, expr)?;
    let _ = simplify_constraints(&data_env, &class_env, apply_constraints(&s, cs))?;
    Ok(apply(&s, t))
}

pub fn infer_module(module: &ast::Module) -> Result<HashMap<String, Scheme>> {
    // Unit-test-friendly default: include stdlib class env so class methods like `show`
    // can be used as values (Haskell-like) without explicit Prelude wiring.
    let stdlib_class_env = load_stdlib_class_env()?;
    let mut cx = InferCtx::default();
    let class_index = build_class_method_scheme_index(&mut cx, &stdlib_class_env)?;
    infer_module_with_class_env(module, &stdlib_class_env, &class_index)
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
    collect_ctor_env_with_class_env(cx, module, &ClassEnv::default(), None)
}

fn add_minimal_prelude_types(cx: &mut InferCtx, env: &mut TypeEnv) {
    // Minimal prelude:
    //   IO :: forall a. a -> IO a
    // This lets `do` blocks typecheck without requiring an explicit `data IO a = ...` in every module.
    let Ty::Var(a) = cx.fresh() else {
        unreachable!()
    };
    env.insert(
        "IO".to_string(),
        EnvEntry {
            scheme: Scheme {
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
            def_site: None,
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
        EnvEntry {
            scheme: Scheme {
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
            def_site: None,
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
        EnvEntry {
            scheme: Scheme {
                vars: vec![a, b],
                constraints: vec![],
                ty: Ty::Func(
                    Box::new(io_a),
                    Box::new(Ty::Func(Box::new(io_b.clone()), Box::new(io_b))),
                ),
            },
            def_site: None,
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
        EnvEntry {
            scheme: Scheme {
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
            def_site: None,
        },
    );
}

fn add_integer_primitives(env: &mut TypeEnv) {
    // + :: Integer -> Integer -> Integer
    env.insert(
        "+".to_string(),
        EnvEntry {
            scheme: Scheme {
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
            def_site: None,
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
            EnvEntry {
                scheme: Scheme {
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
                def_site: None,
            },
        );
    }

    // Integer arithmetic builtins (used by Num Integer instance).
    // __builtin_Integer_add :: Integer -> Integer -> Integer
    // __builtin_Integer_mul :: Integer -> Integer -> Integer
    for name in ["__builtin_Integer_add", "__builtin_Integer_mul"] {
        env.insert(
            name.to_string(),
            EnvEntry {
                scheme: Scheme {
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
                def_site: None,
            },
        );
    }

    // - :: Integer -> Integer -> Integer
    env.insert(
        "-".to_string(),
        EnvEntry {
            scheme: Scheme {
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
            def_site: None,
        },
    );

    // * :: Integer -> Integer -> Integer
    env.insert(
        "*".to_string(),
        EnvEntry {
            scheme: Scheme {
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
            def_site: None,
        },
    );

    // / :: Integer -> Integer -> Integer
    env.insert(
        "/".to_string(),
        EnvEntry {
            scheme: Scheme {
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
            def_site: None,
        },
    );
}

fn add_bool_primitives(cx: &mut InferCtx, env: &mut TypeEnv) {
    // __primEq :: forall a. a -> a -> Bool
    // Structural equality primitive (used by derived Eq instances).
    let Ty::Var(v) = cx.fresh() else {
        unreachable!()
    };
    env.insert(
        "__primEq".to_string(),
        EnvEntry {
            scheme: Scheme {
                vars: vec![v],
                constraints: vec![],
                ty: Ty::Func(
                    Box::new(Ty::Var(v)),
                    Box::new(Ty::Func(
                        Box::new(Ty::Var(v)),
                        Box::new(Ty::Con("Bool".to_string())),
                    )),
                ),
            },
            def_site: None,
        },
    );

    // < :: Integer -> Integer -> Bool
    for name in ["<", "<=", ">", ">="] {
        env.insert(
            name.to_string(),
            EnvEntry {
                scheme: Scheme {
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
                def_site: None,
            },
        );
    }

    // Note: `==` and `/=` are provided by stdlib Prelude (overridable via Eq).

    // && :: Bool -> Bool -> Bool
    for name in ["&&", "||"] {
        env.insert(
            name.to_string(),
            EnvEntry {
                scheme: Scheme {
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
                def_site: None,
            },
        );
    }

    // not :: Bool -> Bool
    env.insert(
        "not".to_string(),
        EnvEntry {
            scheme: Scheme {
                vars: vec![],
                constraints: vec![],
                ty: Ty::Func(
                    Box::new(Ty::Con("Bool".to_string())),
                    Box::new(Ty::Con("Bool".to_string())),
                ),
            },
            def_site: None,
        },
    );
}

fn add_string_primitives(cx: &mut InferCtx, env: &mut TypeEnv) {
    let char_list = Ty::List(Box::new(Ty::Con("Char".to_string())));

    // __primShow :: forall a. a -> [Char]
    // Structural show primitive (used by derived Show instances).
    let Ty::Var(v) = cx.fresh() else {
        unreachable!()
    };
    env.insert(
        "__primShow".to_string(),
        EnvEntry {
            scheme: Scheme {
                vars: vec![v],
                constraints: vec![],
                ty: Ty::Func(Box::new(Ty::Var(v)), Box::new(char_list.clone())),
            },
            def_site: None,
        },
    );

    // intToString :: Integer -> [Char]
    env.insert(
        "intToString".to_string(),
        EnvEntry {
            scheme: Scheme {
                vars: vec![],
                constraints: vec![],
                ty: Ty::Func(
                    Box::new(Ty::Con("Integer".to_string())),
                    Box::new(char_list.clone()),
                ),
            },
            def_site: None,
        },
    );

    // boolToString :: Bool -> [Char]
    env.insert(
        "boolToString".to_string(),
        EnvEntry {
            scheme: Scheme {
                vars: vec![],
                constraints: vec![],
                ty: Ty::Func(
                    Box::new(Ty::Con("Bool".to_string())),
                    Box::new(char_list.clone()),
                ),
            },
            def_site: None,
        },
    );

    // ++ :: forall a. [a] -> [a] -> [a]
    let Ty::Var(v) = cx.fresh() else {
        unreachable!()
    };
    let list_a = Ty::List(Box::new(Ty::Var(v)));
    env.insert(
        "++".to_string(),
        EnvEntry {
            scheme: Scheme {
                vars: vec![v],
                constraints: vec![],
                ty: Ty::Func(
                    Box::new(list_a.clone()),
                    Box::new(Ty::Func(Box::new(list_a.clone()), Box::new(list_a))),
                ),
            },
            def_site: None,
        },
    );

    // Note: `show` / `toString` are provided by stdlib Prelude (overridable via Show).
}

fn add_io_primitives(cx: &mut InferCtx, env: &mut TypeEnv) {
    add_io_basic_primitives(env);
    add_io_exception_primitives(cx, env);
}

fn add_io_basic_primitives(env: &mut TypeEnv) {
    let char_list = Ty::List(Box::new(Ty::Con("Char".to_string())));

    // stdoutWrite :: [Char] -> IO Unit
    env.insert(
        "stdoutWrite".to_string(),
        EnvEntry {
            scheme: Scheme {
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
            def_site: None,
        },
    );

    // stdinReadLine :: IO [Char]
    env.insert(
        "stdinReadLine".to_string(),
        EnvEntry {
            scheme: Scheme {
                vars: vec![],
                constraints: vec![],
                ty: Ty::App {
                    head: Box::new(Ty::Con("IO".to_string())),
                    args: vec![char_list.clone()],
                },
            },
            def_site: None,
        },
    );

    // readLine :: IO [Char]
    env.insert(
        "readLine".to_string(),
        EnvEntry {
            scheme: Scheme {
                vars: vec![],
                constraints: vec![],
                ty: Ty::App {
                    head: Box::new(Ty::Con("IO".to_string())),
                    args: vec![char_list.clone()],
                },
            },
            def_site: None,
        },
    );

    // print :: [Char] -> IO Unit
    env.insert(
        "print".to_string(),
        EnvEntry {
            scheme: Scheme {
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
            def_site: None,
        },
    );

    // putStrLn :: [Char] -> IO Unit
    // Defined in stdlib Prelude, but required for `.ksif`-only compilation where stdlib
    // bindings may not be linked at runtime yet.
    env.insert(
        "putStrLn".to_string(),
        EnvEntry {
            scheme: Scheme {
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
            def_site: None,
        },
    );

    // getArgs :: IO [[Char]]
    let list_of_strings = Ty::List(Box::new(char_list.clone()));
    env.insert(
        "getArgs".to_string(),
        EnvEntry {
            scheme: Scheme {
                vars: vec![],
                constraints: vec![],
                ty: Ty::App {
                    head: Box::new(Ty::Con("IO".to_string())),
                    args: vec![list_of_strings],
                },
            },
            def_site: None,
        },
    );

    // readFile :: [Char] -> IO [Char]
    env.insert(
        "readFile".to_string(),
        EnvEntry {
            scheme: Scheme {
                vars: vec![],
                constraints: vec![],
                ty: Ty::Func(
                    Box::new(char_list.clone()),
                    Box::new(Ty::App {
                        head: Box::new(Ty::Con("IO".to_string())),
                        args: vec![char_list.clone()],
                    }),
                ),
            },
            def_site: None,
        },
    );

    // writeFile :: [Char] -> [Char] -> IO Unit
    env.insert(
        "writeFile".to_string(),
        EnvEntry {
            scheme: Scheme {
                vars: vec![],
                constraints: vec![],
                ty: Ty::Func(
                    Box::new(char_list.clone()),
                    Box::new(Ty::Func(
                        Box::new(char_list.clone()),
                        Box::new(Ty::App {
                            head: Box::new(Ty::Con("IO".to_string())),
                            args: vec![Ty::Con("Unit".to_string())],
                        }),
                    )),
                ),
            },
            def_site: None,
        },
    );
}

fn add_io_exception_primitives(cx: &mut InferCtx, env: &mut TypeEnv) {
    let char_list = Ty::List(Box::new(Ty::Con("Char".to_string())));

    // throw :: forall a. [Char] -> IO a
    let Ty::Var(a) = cx.fresh() else {
        unreachable!()
    };
    env.insert(
        "throw".to_string(),
        EnvEntry {
            scheme: Scheme {
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
            def_site: None,
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
        EnvEntry {
            scheme: Scheme {
                vars: vec![a],
                constraints: vec![],
                ty: Ty::Func(
                    Box::new(io_a.clone()),
                    Box::new(Ty::Func(Box::new(handler), Box::new(io_a))),
                ),
            },
            def_site: None,
        },
    );

    // try :: forall a. IO a -> IO (Either [Char] a)
    let Ty::Var(a) = cx.fresh() else {
        unreachable!()
    };
    let io_a = Ty::App {
        head: Box::new(Ty::Con("IO".to_string())),
        args: vec![Ty::Var(a)],
    };
    let either = Ty::App {
        head: Box::new(Ty::Con("Either".to_string())),
        args: vec![char_list, Ty::Var(a)],
    };
    env.insert(
        "try".to_string(),
        EnvEntry {
            scheme: Scheme {
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
            def_site: None,
        },
    );

    // exitWith :: forall a. Integer -> IO a
    // Note: exitWith never returns, so it can have any return type 'a'
    let Ty::Var(a) = cx.fresh() else {
        unreachable!()
    };
    env.insert(
        "exitWith".to_string(),
        EnvEntry {
            scheme: Scheme {
                vars: vec![a],
                constraints: vec![],
                ty: Ty::Func(
                    Box::new(Ty::Con("Integer".to_string())),
                    Box::new(Ty::App {
                        head: Box::new(Ty::Con("IO".to_string())),
                        args: vec![Ty::Var(a)],
                    }),
                ),
            },
            def_site: None,
        },
    );
}

fn add_misc_builtins(cx: &mut InferCtx, env: &mut TypeEnv) {
    let char_list = Ty::List(Box::new(Ty::Con("Char".to_string())));

    // error :: forall a. [Char] -> a
    let Ty::Var(a) = cx.fresh() else {
        unreachable!()
    };
    env.insert(
        "error".to_string(),
        EnvEntry {
            scheme: Scheme {
                vars: vec![a],
                constraints: vec![],
                ty: Ty::Func(Box::new(char_list.clone()), Box::new(Ty::Var(a))),
            },
            def_site: None,
        },
    );

    // __recordGet :: forall a b. { .. } -> [Char] -> b
    //
    // Used by the typeclass dict-passing rewrite to project a method field.
    // The runtime enforces the record-ness dynamically.
    let Ty::Var(a) = cx.fresh() else {
        unreachable!()
    };
    let Ty::Var(b) = cx.fresh() else {
        unreachable!()
    };
    env.insert(
        "__recordGet".to_string(),
        EnvEntry {
            scheme: Scheme {
                vars: vec![a, b],
                constraints: vec![],
                ty: Ty::Func(
                    Box::new(Ty::Var(a)),
                    Box::new(Ty::Func(Box::new(char_list), Box::new(Ty::Var(b)))),
                ),
            },
            def_site: None,
        },
    );
}

fn add_ffi_primitives(env: &mut TypeEnv) {
    // P6: unsafe-free "FFI" boundary scaffolding.
    // ffiAddI32 :: i32 -> i32 -> i32
    env.insert(
        "ffiAddI32".to_string(),
        EnvEntry {
            scheme: Scheme {
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
            def_site: None,
        },
    );

    // ffiAddF32 :: f32 -> f32 -> f32
    env.insert(
        "ffiAddF32".to_string(),
        EnvEntry {
            scheme: Scheme {
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
            def_site: None,
        },
    );

    // P9: real C ABI FFI (unsafe isolated; feature-gated).
    // ffiPuts :: [Char] -> IO i32
    // Note: in stdlib, `type String = [Char]`, and string literals are `[Char]`.
    #[cfg(feature = "unsafe_ffi")]
    env.insert(
        "ffiPuts".to_string(),
        EnvEntry {
            scheme: Scheme {
                vars: vec![],
                constraints: vec![],
                ty: Ty::Func(
                    Box::new(Ty::List(Box::new(Ty::Con("Char".to_string())))),
                    Box::new(Ty::App {
                        head: Box::new(Ty::Con("IO".to_string())),
                        args: vec![Ty::Con("i32".to_string())],
                    }),
                ),
            },
            def_site: None,
        },
    );
}

fn add_prelude_data_ctors(
    cx: &mut InferCtx,
    env: &mut TypeEnv,
    also_unqualified_maybe: bool,
    also_unqualified_either: bool,
) {
    // Hardcoded Prelude constructors for module-unit compilation with .ksif.
    // Always provide qualified ctor names (e.g. `Prelude.Just`).
    // Only provide unqualified ctor names (e.g. `Just`) when Prelude is imported unqualified.
    // Note: the REPL always includes `import Prelude`, so unqualified ctors should be available there.

    // Maybe: data Maybe a = Nothing | Just a
    let Ty::Var(a) = cx.fresh() else {
        unreachable!()
    };
    let maybe_a = Ty::App {
        head: Box::new(Ty::Con("Maybe".to_string())),
        args: vec![Ty::Var(a)],
    };

    let nothing_entry = EnvEntry {
        scheme: Scheme {
            vars: vec![a],
            constraints: vec![],
            ty: maybe_a.clone(),
        },
        def_site: None,
    };
    let just_entry = EnvEntry {
        scheme: Scheme {
            vars: vec![a],
            constraints: vec![],
            ty: Ty::Func(Box::new(Ty::Var(a)), Box::new(maybe_a)),
        },
        def_site: None,
    };

    env.insert("Prelude.Nothing".to_string(), nothing_entry.clone());
    env.insert("Prelude.Just".to_string(), just_entry.clone());
    if also_unqualified_maybe {
        env.insert("Nothing".to_string(), nothing_entry);
        env.insert("Just".to_string(), just_entry);
    }

    // Either: data Either a b = Left a | Right b
    let Ty::Var(a) = cx.fresh() else {
        unreachable!()
    };
    let Ty::Var(b) = cx.fresh() else {
        unreachable!()
    };
    let either_ab = Ty::App {
        head: Box::new(Ty::Con("Either".to_string())),
        args: vec![Ty::Var(a), Ty::Var(b)],
    };

    let left_entry = EnvEntry {
        scheme: Scheme {
            vars: vec![a, b],
            constraints: vec![],
            ty: Ty::Func(Box::new(Ty::Var(a)), Box::new(either_ab.clone())),
        },
        def_site: None,
    };
    let right_entry = EnvEntry {
        scheme: Scheme {
            vars: vec![a, b],
            constraints: vec![],
            ty: Ty::Func(Box::new(Ty::Var(b)), Box::new(either_ab)),
        },
        def_site: None,
    };

    env.insert("Prelude.Left".to_string(), left_entry.clone());
    env.insert("Prelude.Right".to_string(), right_entry.clone());
    if also_unqualified_either {
        env.insert("Left".to_string(), left_entry);
        env.insert("Right".to_string(), right_entry);
    }
}

fn collect_ctor_env_with_class_env(
    cx: &mut InferCtx,
    module: &ast::Module,
    class_env: &ClassEnv,
    module_path: Option<&Path>,
) -> Result<TypeEnv> {
    let mut env = TypeEnv::new();

    add_minimal_prelude_types(cx, &mut env);
    add_integer_primitives(&mut env);
    add_bool_primitives(cx, &mut env);
    add_string_primitives(cx, &mut env);
    add_io_primitives(cx, &mut env);
    add_misc_builtins(cx, &mut env);
    add_ffi_primitives(&mut env);
    let prelude_import: Option<&ast::ImportDecl> = module.items.iter().find_map(|it| {
        let ast::Item::Import(id) = it else {
            return None;
        };
        if id.module == "Prelude" && !id.qualified {
            Some(id)
        } else {
            None
        }
    });

    // Always provide qualified Prelude ctor names (Prelude.Just, Prelude.Nothing).
    // Provide unqualified ctor names (Just, Nothing, Left, Right) only if the Prelude import
    // actually brings them into scope (i.e. not hidden by an import spec).
    fn import_spec_allows_ctor(
        import_spec: &Option<ast::ImportSpec>,
        type_name: &str,
        ctor_name: &str,
    ) -> bool {
        match import_spec {
            None => true,
            Some(ast::ImportSpec::Only(specs)) => specs.iter().any(|s| match s {
                ast::ExportSpec::Name(n) => n == ctor_name,
                ast::ExportSpec::Type { name, ctors } if name == type_name => match ctors {
                    ast::ExportCtors::All => true,
                    ast::ExportCtors::Some(cs) => cs.iter().any(|c| c == ctor_name),
                },
                _ => false,
            }),
            Some(ast::ImportSpec::Hiding(specs)) => {
                let hidden = specs.iter().any(|s| match s {
                    ast::ExportSpec::Name(n) => n == ctor_name,
                    ast::ExportSpec::Type { name, ctors } if name == type_name => match ctors {
                        ast::ExportCtors::All => true,
                        ast::ExportCtors::Some(cs) => cs.iter().any(|c| c == ctor_name),
                    },
                    _ => false,
                });
                !hidden
            }
        }
    }

    let (also_unqualified_maybe, also_unqualified_either) = match prelude_import {
        Some(id) => (
            import_spec_allows_ctor(&id.import_spec, "Maybe", "Nothing")
                && import_spec_allows_ctor(&id.import_spec, "Maybe", "Just"),
            import_spec_allows_ctor(&id.import_spec, "Either", "Left")
                && import_spec_allows_ctor(&id.import_spec, "Either", "Right"),
        ),
        None => (false, false),
    };

    // REPL-style anonymous modules default to unqualified Prelude ctors.
    let (also_unqualified_maybe, also_unqualified_either) = if module.name.is_none() {
        (true, true)
    } else {
        (also_unqualified_maybe, also_unqualified_either)
    };

    add_prelude_data_ctors(
        cx,
        &mut env,
        also_unqualified_maybe,
        also_unqualified_either,
    );

    add_data_ctors_into_env(cx, module, module_path, &mut env);
    add_class_methods_into_env(cx, class_env, &mut env)?;

    Ok(env)
}

fn add_data_ctors_into_env(
    cx: &mut InferCtx,
    module: &ast::Module,
    module_path: Option<&Path>,
    env: &mut TypeEnv,
) {
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
                EnvEntry {
                    scheme: Scheme {
                        vars,
                        constraints: vec![],
                        ty,
                    },
                    def_site: module_path.map(|p| DefSite {
                        path: p.to_path_buf(),
                        span: ctor.span,
                    }),
                },
            );
        }
    }
}

fn add_class_methods_into_env(
    cx: &mut InferCtx,
    class_env: &ClassEnv,
    env: &mut TypeEnv,
) -> Result<()> {
    // Add class methods as overloaded functions.
    for ((class, method), qt) in &class_env.methods {
        // If the module defines a value with the same name, let it win.
        if env.contains_key(method) {
            continue;
        }
        let scheme = lower_class_method_scheme(cx, class_env, class, qt)?;
        env.insert(
            method.clone(),
            EnvEntry {
                scheme,
                def_site: None,
            },
        );
    }

    Ok(())
}

fn build_class_method_scheme_index(
    cx: &mut InferCtx,
    class_env: &ClassEnv,
) -> Result<ClassEnvIndex> {
    let mut idx = ClassEnvIndex::default();

    for ((class, method), qt) in &class_env.methods {
        if idx.methods_by_name.contains_key(method) {
            return Err(Error::msg(format!(
                "ambiguous method name: {method} (defined in multiple classes)"
            )));
        }

        let scheme = lower_class_method_scheme(cx, class_env, class, qt)?;
        idx.methods_by_name.insert(method.clone(), scheme);
    }
    Ok(idx)
}

fn lower_class_method_scheme(
    cx: &mut InferCtx,
    class_env: &ClassEnv,
    class: &ast::ClassId,
    qt: &ast::QualType,
) -> Result<Scheme> {
    let param = class_env
        .class_params
        .get(class)
        .ok_or_else(|| Error::msg("internal: missing class param"))?
        .clone();

    let mut holes: HashMap<String, Ty> = HashMap::new();
    let class_param_ty = holes.entry(param).or_insert_with(|| cx.fresh()).clone();

    let mut cs: Vec<Constraint> = Vec::new();

    // Built-in classes use specialized constraints.
    // This keeps inference/solver behavior consistent and avoids forcing all code paths
    // to handle fully-general class constraints.
    match class.name.rsplit('.').next().unwrap_or(class.name.as_str()) {
        "Show" => cs.push(Constraint::Show(class_param_ty)),
        "Eq" => cs.push(Constraint::Eq(class_param_ty)),
        "ShowRow" => cs.push(Constraint::ShowRow(class_param_ty)),
        "EqRow" => cs.push(Constraint::EqRow(class_param_ty)),
        _ => cs.push(Constraint::Class {
            class: class.clone(),
            ty: class_param_ty,
        }),
    }

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

    Ok(Scheme {
        vars,
        constraints: cs,
        ty,
    })
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
    /// class id -> parameter name (e.g. `class C a where` => (C, a))
    class_params: HashMap<ast::ClassId, String>,
    /// class id -> superclass predicates (Haskell-style)
    class_supers: HashMap<ast::ClassId, Vec<ast::Predicate>>,
    /// method name -> list of classes that define it
    method_classes: HashMap<String, Vec<ast::ClassId>>,
    /// (class, method) -> declared method type
    methods: HashMap<(ast::ClassId, String), ast::QualType>,
    /// (class, instance-head-type-key) -> dictionary binding name
    instances: HashMap<(ast::ClassId, String), String>,
    /// Non-ground instances (dictionary passing). These are selected by unification on the
    /// instance head pattern, and require dictionaries for their context predicates.
    poly_instances: Vec<PolyInstance>,

    // Type aliases in scope when this env was collected.
    // Used to compare method signatures modulo aliases across merges.
    aliases: HashMap<String, ast::TypeAlias>,
}

#[derive(Debug, Clone)]
struct PolyInstance {
    class: ast::ClassId,
    /// Instance head pattern as an internal type (may contain Ty::Var).
    head_pat: Ty,
    /// How many context dictionary arguments the instance dictionary expects.
    ctx_len: usize,
    /// Dictionary binding name (a value or a function if ctx_len > 0).
    dict_name: String,
}

// NOTE: local let/where constraint solving uses InferCtx.full_class_env.

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
        Ty::List(t) => {
            let mut out = "List".to_string();
            out.push('_');
            out.push_str(&instance_head_key_ty(t)?);
            out
        }
        Ty::Tuple(ts) => {
            let mut out = format!("Tuple{}", ts.len());
            for t in ts {
                out.push('_');
                out.push_str(&instance_head_key_ty(t)?);
            }
            out
        }
        Ty::Record(fields) => {
            // Key records by their sorted label set only (types are ignored for the key).
            // This is conservative and primarily avoids internal MVP errors.
            let mut labels: Vec<&str> = fields.iter().map(|(k, _)| k.as_str()).collect();
            labels.sort();
            labels.dedup();
            format!("Record{}", labels.join("_"))
        }
        Ty::RecordOpen(fields, _rest) => {
            let mut labels: Vec<&str> = fields.iter().map(|(k, _)| k.as_str()).collect();
            labels.sort();
            labels.dedup();
            format!("RecordOpen{}", labels.join("_"))
        }
        Ty::Func(a, b) => {
            // Avoid internal errors: functions can appear as ground types in constraints.
            // If there is no matching instance, callers will report “cannot satisfy constraint”.
            format!(
                "Func_{}_{}",
                instance_head_key_ty(a)?,
                instance_head_key_ty(b)?
            )
        }
        _ => {
            return Err(Error::msg(
                "MVP: class constraints support only constructor/app instance heads",
            ))
        }
    })
}

fn normalize_ty_for_instance_key(ty: &Ty) -> Ty {
    // Built-in surface alias: String = [Char].
    match ty {
        Ty::List(t) if matches!(t.as_ref(), Ty::Con(n) if n == "Char") => {
            Ty::Con("String".to_string())
        }
        _ => ty.clone(),
    }
}

fn mangle_ident(s: &str) -> String {
    // Map common operators to readable names to avoid collisions
    let special_names: &[(&str, &str)] = &[
        ("+", "plus"),
        ("-", "minus"),
        ("*", "times"),
        ("/", "div"),
        ("==", "eq"),
        ("/=", "ne"),
        ("<", "lt"),
        ("<=", "le"),
        (">", "gt"),
        (">=", "ge"),
        ("&&", "and"),
        ("||", "or"),
        ("++", "append"),
        (">>", "then"),
        (">>=", "bind"),
        ("<$>", "fmap"),
        ("<*>", "ap"),
        ("<*", "apLeft"),
        ("*>", "apRight"),
        (".", "compose"),
        ("$", "apply"),
        ("<>", "mappend"),
        ("<->", "diff"),
        ("=<<", "bindFlipped"),
        ("+^", "addOp"),
        ("-^", "subOp"),
        ("*^", "mulOp"),
        ("/^", "divOp"),
        ("&", "ampersand"),
    ];

    for (op, name) in special_names {
        if s == *op {
            return name.to_string();
        }
    }

    // Fallback: replace non-alphanumeric with underscore + hex code
    let mut out = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else {
            // Use hex encoding to ensure uniqueness
            out.push_str(&format!("_{:x}", ch as u32));
        }
    }
    out
}

fn desugar_typeclasses(module: &mut ast::Module) -> Result<ClassEnv> {
    desugar_typeclasses_with_strict(module, false)
}

/// Process classes and instances with optional strict canonicalization.
fn desugar_typeclasses_with_strict(module: &mut ast::Module, strict: bool) -> Result<ClassEnv> {
    if std::env::var("KSCR_DEBUG_DESUGAR").is_ok() {
        eprintln!(
            "[DESUGAR] Starting desugar_typeclasses for module: {:?}",
            module.name
        );
    }
    let mut env = ClassEnv {
        aliases: collect_type_aliases(module),
        ..Default::default()
    };

    // Expand deriving clauses BEFORE canonicalization so derived instance class ids get canonicalized
    expand_deriving_clauses(module)?;

    // Then canonicalize class references inside the module.
    // Use both import-based and def_module-based canonicalization to handle both
    // user-defined classes and imported/injected stdlib classes.
    canonicalize_class_names_in_module_combined(module, strict)?;

    // Then collect class decls with canonical ids.
    let (class_method_names, class_default_methods) = collect_class_decls(module, &mut env)?;
    reject_ambiguous_method_names(&mut env)?;
    validate_superclass_preds(&env)?;
    detect_superclass_cycles(&env)?;

    let instance_decls = collect_instance_decls(module);
    preregister_instance_dicts(&mut env, &instance_decls, module.name.as_deref())?;
    let extra_items = generate_instance_items(
        &env,
        &instance_decls,
        &class_method_names,
        &class_default_methods,
    )?;

    module.items = module
        .items
        .drain(..)
        .filter(|it| !matches!(it, ast::Item::ClassDecl(_) | ast::Item::InstanceDecl(_)))
        .chain(extra_items)
        .collect();

    Ok(env)
}

/// Collect only class declarations from a module, without processing instances.
/// Returns ClassEnv with class definitions, and metadata for later instance processing.
/// Skips superclass validation (should be done after all classes are collected).
fn collect_class_env_only(
    module: &mut ast::Module,
    strict: bool,
) -> Result<(ClassEnv, ClassDeclInfo)> {
    let mut env = ClassEnv {
        aliases: collect_type_aliases(module),
        ..Default::default()
    };

    // Expand deriving clauses BEFORE canonicalization so they get canonicalized too
    expand_deriving_clauses(module)?;

    // Canonicalize class references
    canonicalize_class_names_in_module_combined(module, strict)?;

    // Collect class decls only
    let class_decl_info = collect_class_decls(module, &mut env)?;
    reject_ambiguous_method_names(&mut env)?;
    // Note: Skip validate_superclass_preds and detect_superclass_cycles here.
    // They will be called after all classes are collected and merged.

    Ok((env, class_decl_info))
}

/// Process instances against an existing merged ClassEnv.
/// This assumes all classes have already been collected.
fn process_instances_with_env(
    module: &mut ast::Module,
    merged_env: &ClassEnv,
    class_method_names: &HashMap<ast::ClassId, Vec<String>>,
    class_default_methods: &HashMap<(ast::ClassId, String), ast::Expr>,
    strict: bool,
) -> Result<ClassEnv> {
    // Expand deriving clauses BEFORE canonicalization
    expand_deriving_clauses(module)?;

    // Canonicalize class references in instance declarations.
    // While building the stdlib ClassEnv, some stdlib modules intentionally avoid importing
    // Prelude to prevent cycles, but still reference Prelude classes (e.g. via deriving).
    // Use merged_env as an additional origin source to resolve those references.
    let mut extra_origin: HashMap<String, Vec<String>> = HashMap::new();
    for class_id in merged_env.class_params.keys() {
        if let Some((m, short)) = class_id.name.rsplit_once('.') {
            let v = extra_origin.entry(short.to_string()).or_default();
            if !v.iter().any(|x| x == m) {
                v.push(m.to_string());
            }
        }
    }
    canonicalize_class_names_in_module_combined_with_extra_origin(module, strict, &extra_origin)?;

    // Collect instances and generate dictionary items
    let instance_decls = collect_instance_decls(module);

    // Create a working env that combines merged classes with local instances
    let mut working_env = merged_env.clone();
    working_env.aliases = collect_type_aliases(module);

    let merged_poly_len = working_env.poly_instances.len();

    preregister_instance_dicts(&mut working_env, &instance_decls, module.name.as_deref())?;

    // Generate instance items using the working env (has both classes and instances)
    let extra_items = generate_instance_items(
        &working_env,
        &instance_decls,
        class_method_names,
        class_default_methods,
    )?;

    module.items = module
        .items
        .drain(..)
        .filter(|it| !matches!(it, ast::Item::ClassDecl(_) | ast::Item::InstanceDecl(_)))
        .chain(extra_items)
        .collect();

    // Return only the instance registrations (not the full working_env)
    let mut local_env = ClassEnv {
        aliases: collect_type_aliases(module),
        instances: working_env.instances.clone(),
        poly_instances: working_env.poly_instances[merged_poly_len..].to_vec(),
        ..Default::default()
    };

    // Filter to only instances that were added (not from merged_env)
    local_env
        .instances
        .retain(|k, _| !merged_env.instances.contains_key(k));

    Ok(local_env)
}

fn module_imported_exports(
    _module: &ast::Module,
    id: &ast::ImportDecl,
) -> Result<Option<(ast::Module, ExportTable)>> {
    // Only handle the common case where the import was already flattened into the module.
    // We can look up the imported module path via the loader-independent resolution function.
    let module_dir = Path::new(".");
    let rel = id.module.replace('.', "/");
    let local = module_dir.join(format!("{}.ks", rel));
    let stdlib_root = stdlib_cache::stdlib_root()?;
    let stdlib = stdlib_root.join(format!("{}.ks", rel));
    let path = std::fs::canonicalize(&local).or_else(|_| std::fs::canonicalize(&stdlib));
    let Ok(path) = path else {
        return Ok(None);
    };

    // Load via stdlib cache/parser directly (no recursive imports here; we just need exports).
    let src = std::fs::read_to_string(&path)?;
    let mut imported = parser::parse_module(&src)?;
    desugar_module_qualified_names(&mut imported)?;

    // Keep internal qualifier env consistent and allow canonical names.
    if imported.name.as_deref() != Some(&id.module) {
        // Best effort; module name mismatch will be caught later in ModuleLoader.
    }

    let exports = module_exported_names(&imported)?;
    Ok(Some((imported, exports)))
}

fn canonicalize_class_names_in_module_combined_with_extra_origin(
    module: &mut ast::Module,
    strict: bool,
    extra_origin: &HashMap<String, Vec<String>>,
) -> Result<()> {
    fn push_origin(map: &mut HashMap<String, Vec<String>>, name: &str, module: String) {
        let v = map.entry(name.to_string()).or_default();
        if !v.iter().any(|m| m == &module) {
            v.push(module);
        }
    }

    // Combined canonicalization: use both imports and def_module fields.
    // This handles both user-defined classes (from imports) and stdlib classes
    // that were flattened/injected (with def_module set).
    let mut class_origin: HashMap<String, Vec<String>> = HashMap::new();

    // First, gather class origins from unqualified imports (for user-defined classes).
    for it in &module.items {
        let ast::Item::Import(id) = it else {
            continue;
        };
        if id.qualified {
            continue;
        }
        let Some((imported, exports)) = module_imported_exports(module, id)? else {
            continue;
        };
        let imported_name = imported.name.clone().unwrap_or_else(|| id.module.clone());
        for (name, kind) in exports.entries.iter() {
            if *kind != SymbolKind::Class {
                continue;
            }
            push_origin(&mut class_origin, name, imported_name.clone());
        }
    }

    // Second, gather class origins from def_module fields (for stdlib/flattened classes).
    for it in &module.items {
        if let ast::Item::ClassDecl(c) = it {
            if let Some(ref def_mod) = c.def_module {
                push_origin(&mut class_origin, &c.name, def_mod.clone());
            }
        }
    }

    // Third, merge extra origins (e.g. merged stdlib ClassEnv) for cases where
    // a module references a stdlib class without importing its defining module.
    for (name, modules) in extra_origin {
        for m in modules {
            push_origin(&mut class_origin, name, m.clone());
        }
    }

    canonicalize_class_refs_in_module(module, &class_origin, strict)?;
    Ok(())
}

fn canonicalize_class_names_in_module_combined(
    module: &mut ast::Module,
    strict: bool,
) -> Result<()> {
    let extra_origin: HashMap<String, Vec<String>> = HashMap::new();
    canonicalize_class_names_in_module_combined_with_extra_origin(module, strict, &extra_origin)
}

fn canonicalize_class_names_in_merged_stdlib(module: &mut ast::Module) {
    fn push_origin(map: &mut HashMap<String, Vec<String>>, name: &str, module: String) {
        let v = map.entry(name.to_string()).or_default();
        if !v.iter().any(|m| m == &module) {
            v.push(module);
        }
    }

    // For a merged stdlib module (no imports, just all class/instance decls),
    // build class_origin from def_module fields of ClassDecls.
    let mut class_origin: HashMap<String, Vec<String>> = HashMap::new();

    for it in &module.items {
        if let ast::Item::ClassDecl(c) = it {
            if let Some(ref def_mod) = c.def_module {
                push_origin(&mut class_origin, &c.name, def_mod.clone());
            }
        }
    }

    // Use non-strict mode for backwards compatibility
    let _ = canonicalize_class_refs_in_module(module, &class_origin, false);
}

fn canonicalize_class_refs_in_module(
    module: &mut ast::Module,
    class_origin: &HashMap<String, Vec<String>>,
    strict: bool,
) -> Result<()> {
    fn dot_count(s: &str) -> usize {
        s.bytes().filter(|b| *b == b'.').count()
    }

    fn qualify_class_id(
        current_module: Option<&str>,
        id: &mut ast::ClassId,
        class_origin: &HashMap<String, Vec<String>>,
        strict: bool,
    ) -> Result<()> {
        if id.name.contains('.') {
            return Ok(());
        }

        let Some(candidates) = class_origin.get(&id.name) else {
            if strict {
                return Err(Error::msg(format!(
                    "Unresolved class reference: '{}'. \
                    This class is not imported or exported. \
                    Hint: Add an import statement for the module containing this class, \
                    or check for typos in the class name.",
                    id.name
                )));
            }
            // Best-effort: leave unqualified if not found
            return Ok(());
        };

        // 1) Same-module wins (e.g., a class defined locally).
        if let Some(cur) = current_module {
            if candidates.iter().any(|m| m == cur) {
                id.name = format!("{cur}.{}", id.name);
                return Ok(());
            }
        }

        if candidates.len() == 1 {
            let m = &candidates[0];
            id.name = format!("{m}.{}", id.name);
            return Ok(());
        }

        // 2) More specific qualifier wins (more dots in module path).
        let max_dots = candidates.iter().map(|m| dot_count(m)).max().unwrap_or(0);
        let mut best: Vec<&str> = candidates
            .iter()
            .filter_map(|m| {
                if dot_count(m) == max_dots {
                    Some(m.as_str())
                } else {
                    None
                }
            })
            .collect();

        if best.len() == 1 {
            let m = best[0];
            id.name = format!("{m}.{}", id.name);
            return Ok(());
        }

        // 3) Still ambiguous.
        best.sort();
        if strict {
            return Err(Error::msg(format!(
                "Ambiguous class reference: '{}'. Candidates: {}",
                id.name,
                best.join(", ")
            )));
        }

        // Best-effort for user code: pick deterministically.
        let m = best[0];
        id.name = format!("{m}.{}", id.name);
        Ok(())
    }

    let current_module = module.name.as_deref();
    for it in &mut module.items {
        match it {
            ast::Item::ClassDecl(c) => {
                for p in &mut c.supers {
                    if let ast::Predicate::Class { class, .. } = p {
                        qualify_class_id(current_module, class, class_origin, strict)?;
                    }
                }
            }
            ast::Item::InstanceDecl(inst) => {
                for p in &mut inst.preds {
                    if let ast::Predicate::Class { class, .. } = p {
                        qualify_class_id(current_module, class, class_origin, strict)?;
                    }
                }
                qualify_class_id(current_module, &mut inst.class, class_origin, strict)?;
            }
            _ => {}
        }
    }
    Ok(())
}

type ClassDeclInfo = (
    HashMap<ast::ClassId, Vec<String>>,
    HashMap<(ast::ClassId, String), ast::Expr>,
);

fn collect_class_decls(module: &ast::Module, env: &mut ClassEnv) -> Result<ClassDeclInfo> {
    // class name -> method names (declaration order)
    let mut class_method_names: HashMap<ast::ClassId, Vec<String>> = HashMap::new();
    // (class, method) -> default implementation expression
    let mut class_default_methods: HashMap<(ast::ClassId, String), ast::Expr> = HashMap::new();

    // Collect class method signatures + defaults.
    for it in &module.items {
        let ast::Item::ClassDecl(c) = it else {
            continue;
        };

        // Use canonical class name: def_module.name if available, else just name.
        let class_name = if let Some(ref m) = c.def_module {
            format!("{}.{}", m, c.name)
        } else {
            c.name.clone()
        };
        if std::env::var("KSCR_DEBUG_COLLECT_CLASS").is_ok() {
            eprintln!(
                "[COLLECT] Collecting class: {} (from name: {}, def_module: {:?})",
                class_name, c.name, c.def_module
            );
        }
        let class_id = ast::ClassId::dummy(class_name);

        if env.class_params.contains_key(&class_id) {
            return Err(Error::msg("duplicate class"));
        }
        env.class_params.insert(class_id.clone(), c.param.clone());
        env.class_supers.insert(class_id.clone(), c.supers.clone());

        for m in &c.methods {
            class_method_names
                .entry(class_id.clone())
                .or_default()
                .push(m.name.clone());
            env.method_classes
                .entry(m.name.clone())
                .or_default()
                .push(class_id.clone());
            env.methods
                .insert((class_id.clone(), m.name.clone()), m.ty.clone());
        }

        for b in &c.default_methods {
            let ast::PatternKind::Var(mname) = &b.pat.kind else {
                return Err(Error::msg(
                    "MVP: class default methods must be simple variable bindings",
                ));
            };
            let key = (class_id.clone(), mname.clone());
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

    Ok((class_method_names, class_default_methods))
}

fn reject_ambiguous_method_names(env: &mut ClassEnv) -> Result<()> {
    // MVP: avoid ambiguous unqualified method names.
    for (m, classes) in &env.method_classes {
        if classes.len() > 1 {
            return Err(Error::msg(format!(
                "ambiguous method name: {m} (defined in classes: {})",
                classes
                    .iter()
                    .map(|c| c.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
    }
    Ok(())
}

fn validate_superclass_preds(env: &ClassEnv) -> Result<()> {
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
                        eprintln!("[ERROR] Superclass {} not found in env", sup.name);
                        eprintln!("[ERROR] Available classes:");
                        for c in env.class_params.keys() {
                            eprintln!("[ERROR]   - {}", c.name);
                        }
                        return Err(Error::msg(format!(
                            "unknown superclass `{}` in class `{}`",
                            sup.name, class.name
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
    Ok(())
}

fn detect_superclass_cycles(env: &ClassEnv) -> Result<()> {
    // Detect cycles in the user-defined superclass graph.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mark {
        Temp,
        Perm,
    }

    fn dfs_cycle(
        env: &ClassEnv,
        node: &ast::ClassId,
        marks: &mut HashMap<ast::ClassId, Mark>,
        stack: &mut Vec<String>,
    ) -> Result<()> {
        if matches!(marks.get(node), Some(Mark::Perm)) {
            return Ok(());
        }
        if matches!(marks.get(node), Some(Mark::Temp)) {
            // Found a cycle; report a readable path.
            stack.push(node.name.clone());
            return Err(Error::msg(format!(
                "cyclic superclass constraints: {}",
                stack.join(" => ")
            )));
        }

        marks.insert(node.clone(), Mark::Temp);
        stack.push(node.name.clone());

        if let Some(supers) = env.class_supers.get(node) {
            for p in supers {
                if let ast::Predicate::Class { class: sup, .. } = p {
                    dfs_cycle(env, sup, marks, stack)?;
                }
            }
        }

        stack.pop();
        marks.insert(node.clone(), Mark::Perm);
        Ok(())
    }

    let mut marks: HashMap<ast::ClassId, Mark> = HashMap::new();
    for c in env.class_params.keys() {
        let mut stack: Vec<String> = Vec::new();
        dfs_cycle(env, c, &mut marks, &mut stack)?;
    }
    Ok(())
}

/// Expand `deriving` clauses into explicit instance declarations.
/// This ensures that derived instances are available via the normal instance mechanism,
/// which is needed for cross-module imports (KSIF doesn't carry deriving info).
///
/// Supported classes:
/// - Eq, Show: generate instance declarations backed by runtime primitives
/// - Semigroup, Monoid: generate actual method implementations
///
/// Restrictions for Semigroup/Monoid:
/// - Only single-constructor data types
/// - At most one type parameter (due to instance context limitations)
fn expand_deriving_clauses(module: &mut ast::Module) -> Result<()> {
    let mut synthetic_instances = Vec::new();

    // If the user already wrote an explicit instance, do not also generate a derived one.
    // This supports the common pattern: `data T deriving (Show)` plus `instance Show T where ...`.
    let mut explicit_instance_keys: HashSet<(String, String)> = HashSet::new();
    for item in &module.items {
        let ast::Item::InstanceDecl(inst) = item else {
            continue;
        };

        let class_unqualified = inst
            .class
            .name
            .rsplit('.')
            .next()
            .unwrap_or(inst.class.name.as_str())
            .to_string();

        if let Ok(ty_key) = instance_head_key_ast(&inst.ty) {
            explicit_instance_keys.insert((class_unqualified, ty_key));
        }
    }

    for item in &module.items {
        let ast::Item::DataDecl(d) = item else {
            continue;
        };

        // For each deriving clause, create a synthetic instance declaration
        for class_name in &d.deriving {
            let inst_ty = if d.params.is_empty() {
                ast::Type::Var(d.name.clone())
            } else {
                let head = Box::new(ast::Type::Var(d.name.clone()));
                let args = d.params.iter().map(|p| ast::Type::Var(p.clone())).collect();
                ast::Type::App { head, args }
            };

            if let Ok(ty_key) = instance_head_key_ast(&inst_ty) {
                if explicit_instance_keys.contains(&(class_name.clone(), ty_key)) {
                    continue;
                }
            }

            // Eq / Show: derive via runtime primitives (structural)
            if class_name == "Show" || class_name == "Eq" {
                // Note: structural deriving does not consult dictionaries.
                // Also, the current MVP instance context cannot carry multiple predicates of
                // the same class (e.g. `Eq a, Eq b`), so keep the context empty.
                let preds: Vec<ast::Predicate> = Vec::new();

                let (method_name, prim_name) = if class_name == "Show" {
                    ("show".to_string(), "__primShow".to_string())
                } else {
                    ("eq".to_string(), "__primEq".to_string())
                };

                let class_id_name = if class_name == "Show" {
                    "Prelude.Show".to_string()
                } else {
                    "Prelude.Eq".to_string()
                };

                let inst = ast::InstanceDecl {
                    preds,
                    class: ast::ClassId::dummy(class_id_name),
                    ty: inst_ty,
                    methods: vec![ast::Binding {
                        doc: None,
                        pat: ast::Pattern {
                            kind: ast::PatternKind::Var(method_name),
                            span: ast::dummy_span(),
                        },
                        expr: ast::Expr {
                            kind: ast::ExprKind::Var(prim_name),
                            span: ast::dummy_span(),
                        },
                        span: ast::dummy_span(),
                    }],
                };

                synthetic_instances.push(ast::Item::InstanceDecl(inst));
                continue;
            }

            // Generate instance for Semigroup or Monoid
            if class_name == "Semigroup" || class_name == "Monoid" {
                // Restriction: only single-constructor types
                if d.ctors.len() != 1 {
                    return Err(Error::msg(format!(
                        "deriving {}: only single-constructor data types are supported (type {} has {} constructors)",
                        class_name, d.name, d.ctors.len()
                    )));
                }

                // Restriction: at most one type parameter
                if d.params.len() > 1 {
                    return Err(Error::msg(format!(
                        "deriving {}: at most one type parameter is supported (type {} has {} parameters)",
                        class_name, d.name, d.params.len()
                    )));
                }

                let inst = generate_semigroup_monoid_instance(d, class_name)?;
                synthetic_instances.push(ast::Item::InstanceDecl(inst));
            } else {
                // Unknown deriving class - create empty instance as fallback
                let preds: Vec<ast::Predicate> = d
                    .params
                    .iter()
                    .map(|p| ast::Predicate::Class {
                        class: ast::ClassId::dummy(class_name.clone()),
                        ty: ast::Type::Var(p.clone()),
                    })
                    .collect();

                let class_id_name = if class_name == "Show" {
                    "Prelude.Show".to_string()
                } else {
                    "Prelude.Eq".to_string()
                };

                let inst = ast::InstanceDecl {
                    preds,
                    class: ast::ClassId::dummy(class_id_name),
                    ty: inst_ty,
                    methods: vec![],
                };

                synthetic_instances.push(ast::Item::InstanceDecl(inst));
            }
        }
    }

    // Add synthetic instances to the module
    module.items.extend(synthetic_instances);
    Ok(())
}

/// Generate Semigroup or Monoid instance for a single-constructor data type.
///
/// For Semigroup: implements (<>) by applying (<>) to each field
/// For Monoid: implements mempty by applying mempty to each field
fn generate_semigroup_monoid_instance(
    d: &ast::DataDecl,
    class_name: &str,
) -> Result<ast::InstanceDecl> {
    let ctor = &d.ctors[0]; // Already validated to have exactly one constructor
    let num_fields = ctor.args.len();

    // Create the instance type (apply type params if any)
    let inst_ty = if d.params.is_empty() {
        ast::Type::Var(d.name.clone())
    } else {
        let head = Box::new(ast::Type::Var(d.name.clone()));
        let args = d.params.iter().map(|p| ast::Type::Var(p.clone())).collect();
        ast::Type::App { head, args }
    };

    // Create predicates for type parameters
    let preds: Vec<ast::Predicate> = d
        .params
        .iter()
        .map(|p| ast::Predicate::Class {
            class: ast::ClassId::dummy(class_name.to_string()),
            ty: ast::Type::Var(p.clone()),
        })
        .collect();

    let mut methods = Vec::new();

    if class_name == "Semigroup" {
        // Generate (<>) implementation
        // \x y -> case (x, y) of (Ctor a1 ... an, Ctor b1 ... bn) -> Ctor (a1 <> b1) ... (an <> bn)

        let a_vars: Vec<String> = (0..num_fields).map(|i| format!("__a{}", i)).collect();
        let b_vars: Vec<String> = (0..num_fields).map(|i| format!("__b{}", i)).collect();

        let pat1 = ast::Pattern::dummy(ast::PatternKind::Constructor {
            name: ast::ResolvedName::unresolved(ctor.name.clone()),
            args: a_vars
                .iter()
                .map(|v| ast::Pattern::dummy(ast::PatternKind::Var(v.clone())))
                .collect(),
        });

        let pat2 = ast::Pattern::dummy(ast::PatternKind::Constructor {
            name: ast::ResolvedName::unresolved(ctor.name.clone()),
            args: b_vars
                .iter()
                .map(|v| ast::Pattern::dummy(ast::PatternKind::Var(v.clone())))
                .collect(),
        });

        let tuple_pat = ast::Pattern::dummy(ast::PatternKind::Tuple(vec![pat1, pat2]));

        // Build the result: Ctor (a1 <> b1) (a2 <> b2) ...
        let combined_fields: Vec<ast::Expr> = a_vars
            .iter()
            .zip(b_vars.iter())
            .map(|(a, b)| {
                // ((<>) a) b
                ast::Expr::dummy(ast::ExprKind::Apply {
                    func: Box::new(ast::Expr::dummy(ast::ExprKind::Apply {
                        func: Box::new(ast::Expr::dummy(ast::ExprKind::Var("<>".to_string()))),
                        args: vec![ast::Expr::dummy(ast::ExprKind::Var(a.clone()))],
                    })),
                    args: vec![ast::Expr::dummy(ast::ExprKind::Var(b.clone()))],
                })
            })
            .collect();

        let rhs = ast::Expr::dummy(ast::ExprKind::Apply {
            func: Box::new(ast::Expr::dummy(ast::ExprKind::Ctor(
                ast::ResolvedName::unresolved(ctor.name.clone()),
            ))),
            args: combined_fields,
        });

        // \x y -> case (x, y) of ...
        let case_expr = ast::Expr::dummy(ast::ExprKind::Case {
            expr: Box::new(ast::Expr::dummy(ast::ExprKind::Tuple(vec![
                ast::Expr::dummy(ast::ExprKind::Var("__x".to_string())),
                ast::Expr::dummy(ast::ExprKind::Var("__y".to_string())),
            ]))),
            arms: vec![ast::CaseArm {
                pat: tuple_pat,
                guard: None,
                body: rhs,
            }],
        });

        let body = ast::Expr::dummy(ast::ExprKind::Lambda {
            params: vec!["__x".to_string(), "__y".to_string()],
            body: Box::new(case_expr),
        });

        methods.push(ast::Binding {
            doc: None,
            pat: ast::Pattern::dummy(ast::PatternKind::Var("<>".to_string())),
            expr: body,
            span: ast::dummy_span(),
        });
    } else if class_name == "Monoid" {
        // Generate mempty implementation
        // Pattern: mempty = Ctor mempty mempty ... mempty

        let mempty_fields: Vec<ast::Expr> = (0..num_fields)
            .map(|_| ast::Expr::dummy(ast::ExprKind::Var("mempty".to_string())))
            .collect();

        let body = ast::Expr::dummy(ast::ExprKind::Apply {
            func: Box::new(ast::Expr::dummy(ast::ExprKind::Ctor(
                ast::ResolvedName::unresolved(ctor.name.clone()),
            ))),
            args: mempty_fields,
        });

        methods.push(ast::Binding {
            doc: None,
            pat: ast::Pattern::dummy(ast::PatternKind::Var("mempty".to_string())),
            expr: body,
            span: ast::dummy_span(),
        });

        // Also generate (<>) for Monoid (required by superclass)
        let a_vars: Vec<String> = (0..num_fields).map(|i| format!("__a{}", i)).collect();
        let b_vars: Vec<String> = (0..num_fields).map(|i| format!("__b{}", i)).collect();

        let pat1 = ast::Pattern::dummy(ast::PatternKind::Constructor {
            name: ast::ResolvedName::unresolved(ctor.name.clone()),
            args: a_vars
                .iter()
                .map(|v| ast::Pattern::dummy(ast::PatternKind::Var(v.clone())))
                .collect(),
        });

        let pat2 = ast::Pattern::dummy(ast::PatternKind::Constructor {
            name: ast::ResolvedName::unresolved(ctor.name.clone()),
            args: b_vars
                .iter()
                .map(|v| ast::Pattern::dummy(ast::PatternKind::Var(v.clone())))
                .collect(),
        });

        let tuple_pat = ast::Pattern::dummy(ast::PatternKind::Tuple(vec![pat1, pat2]));

        let combined_fields: Vec<ast::Expr> = a_vars
            .iter()
            .zip(b_vars.iter())
            .map(|(a, b)| {
                ast::Expr::dummy(ast::ExprKind::Apply {
                    func: Box::new(ast::Expr::dummy(ast::ExprKind::Apply {
                        func: Box::new(ast::Expr::dummy(ast::ExprKind::Var("<>".to_string()))),
                        args: vec![ast::Expr::dummy(ast::ExprKind::Var(a.clone()))],
                    })),
                    args: vec![ast::Expr::dummy(ast::ExprKind::Var(b.clone()))],
                })
            })
            .collect();

        let rhs = ast::Expr::dummy(ast::ExprKind::Apply {
            func: Box::new(ast::Expr::dummy(ast::ExprKind::Ctor(
                ast::ResolvedName::unresolved(ctor.name.clone()),
            ))),
            args: combined_fields,
        });

        let case_expr = ast::Expr::dummy(ast::ExprKind::Case {
            expr: Box::new(ast::Expr::dummy(ast::ExprKind::Tuple(vec![
                ast::Expr::dummy(ast::ExprKind::Var("__x".to_string())),
                ast::Expr::dummy(ast::ExprKind::Var("__y".to_string())),
            ]))),
            arms: vec![ast::CaseArm {
                pat: tuple_pat,
                guard: None,
                body: rhs,
            }],
        });

        let append_body = ast::Expr::dummy(ast::ExprKind::Lambda {
            params: vec!["__x".to_string(), "__y".to_string()],
            body: Box::new(case_expr),
        });

        methods.push(ast::Binding {
            doc: None,
            pat: ast::Pattern::dummy(ast::PatternKind::Var("<>".to_string())),
            expr: append_body,
            span: ast::dummy_span(),
        });
    }

    Ok(ast::InstanceDecl {
        preds,
        class: ast::ClassId::dummy(class_name.to_string()),
        ty: inst_ty,
        methods,
    })
}

fn collect_instance_decls(module: &ast::Module) -> Vec<ast::InstanceDecl> {
    module
        .items
        .iter()
        .filter_map(|it| match it {
            ast::Item::InstanceDecl(inst) => Some(inst.clone()),
            _ => None,
        })
        .collect()
}

fn preregister_instance_dicts(
    env: &mut ClassEnv,
    instance_decls: &[ast::InstanceDecl],
    _module_name: Option<&str>,
) -> Result<()> {
    // Phase 1: pre-register all instance dictionary names.
    // We also collect polymorphic instance metadata for later selection.
    // Dictionary names are unqualified here; they will be qualified during IR merging.
    let mut poly_to_register: Vec<PolyInstance> = Vec::new();

    fn poly_instance_head_key_ast(ty: &ast::Type) -> String {
        use ast::Type;

        fn is_lowercase_ident(s: &str) -> bool {
            s.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        }

        match ty {
            Type::Unit => "Unit".to_string(),
            Type::Integer => "Integer".to_string(),
            Type::Bool => "Bool".to_string(),
            Type::Float64 => "Float64".to_string(),
            Type::Char => "Char".to_string(),
            Type::String => "String".to_string(),
            Type::Hole(_) => "Hole".to_string(),
            Type::Var(name) => {
                if is_lowercase_ident(name) {
                    "poly".to_string()
                } else {
                    name.clone()
                }
            }
            Type::App { head, args } => {
                let mut out = poly_instance_head_key_ast(head);
                for a in args {
                    out.push('_');
                    out.push_str(&poly_instance_head_key_ast(a));
                }
                out
            }
            Type::List(t) => format!("List_{}", poly_instance_head_key_ast(t)),
            Type::Tuple(ts) => {
                let mut out = "Tuple".to_string();
                for t in ts {
                    out.push('_');
                    out.push_str(&poly_instance_head_key_ast(t));
                }
                out
            }
            Type::Record(_) => "Record".to_string(),
            Type::RecordOpen(_, _) => "RecordOpen".to_string(),
            Type::Func(a, b) => format!(
                "Func_{}_{}",
                poly_instance_head_key_ast(a),
                poly_instance_head_key_ast(b)
            ),
        }
    }
    for inst in instance_decls {
        // Use unqualified class name for dictionary names to avoid dots
        // E.g., "Prelude.Ring.Ring" -> "Ring"
        let unqualified_class = inst
            .class
            .name
            .rsplit('.')
            .next()
            .unwrap_or(&inst.class.name);

        match instance_head_key_ast(&inst.ty) {
            Ok(ty_key) => {
                let ty_mangled = mangle_ident(&ty_key);
                let dict_name = format!("__dict_{}_{}", unqualified_class, ty_mangled);

                let key = (inst.class.clone(), ty_key);
                if env.instances.contains_key(&key) {
                    return Err(Error::msg("duplicate instance"));
                }
                env.instances.insert(key, dict_name);
            }
            Err(_) => {
                // Polymorphic (non-ground) instance: pre-register a stable dictionary name.
                let poly_key = poly_instance_head_key_ast(&inst.ty);
                let dict_name = format!(
                    "__dict_{}_poly_{}",
                    unqualified_class,
                    mangle_ident(&poly_key)
                );

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
    Ok(())
}

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
        ast::Predicate::Class { class, .. } => Some(class.name.as_str()),
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

fn generate_instance_items(
    env: &ClassEnv,
    instance_decls: &[ast::InstanceDecl],
    class_method_names: &HashMap<ast::ClassId, Vec<String>>,
    class_default_methods: &HashMap<(ast::ClassId, String), ast::Expr>,
) -> Result<Vec<ast::Item>> {
    // Phase 2: generate impl bindings + dictionary records.
    let mut extra_items: Vec<ast::Item> = Vec::new();
    for inst in instance_decls {
        append_instance_items(
            env,
            inst,
            class_method_names,
            class_default_methods,
            &mut extra_items,
        )?;
    }
    Ok(extra_items)
}

fn append_instance_items(
    env: &ClassEnv,
    inst: &ast::InstanceDecl,
    class_method_names: &HashMap<ast::ClassId, Vec<String>>,
    class_default_methods: &HashMap<(ast::ClassId, String), ast::Expr>,
    extra_items: &mut Vec<ast::Item>,
) -> Result<()> {
    let (ty_key_opt, ty_mangled, dict_name) = resolve_instance_dict_name(env, inst)?;

    let Some(method_names) = class_method_names.get(&inst.class) else {
        return Err(Error::msg(format!(
            "unknown class in instance: {}",
            inst.class.name
        )));
    };

    let inst_methods = collect_instance_methods(inst)?;
    let direct_supers = collect_direct_supers(env, inst);
    let extra_param_names = build_extra_param_names(inst)?;
    let super_dict_names =
        resolve_super_dict_names(env, inst, ty_key_opt.as_ref(), &direct_supers)?;

    // Method impl bindings (instance overrides or class defaults).
    //
    // IMPORTANT (Plan A / Haskell-like imports): after import lowering we may have
    // instance decls whose method bodies refer to constructors unqualified (e.g. `Nothing`).
    // Under `import qualified Prelude as P`, those constructors are only available as
    // `P.Nothing` / `P.Just`, so we must qualify ctor references inside the generated
    // `__inst_*` bindings.
    let mut dict_fields: Vec<(String, ast::Expr)> = Vec::new();
    for mname in method_names {
        let expr = if let Some(e) = inst_methods.get(mname) {
            e.clone()
        } else if let Some(e) = class_default_methods.get(&(inst.class.clone(), mname.clone())) {
            e.clone()
        } else {
            return Err(Error::msg(format!(
                "missing method implementation for `{}` in instance {} {}",
                mname,
                inst.class.name,
                ty_key_opt.clone().unwrap_or_else(|| "<poly>".to_string())
            )));
        };

        // Use unqualified class name to avoid dots in instance method names
        let unqualified_class = inst
            .class
            .name
            .rsplit('.')
            .next()
            .unwrap_or(&inst.class.name);
        let impl_name = format!(
            "__inst_{}_{}_{}",
            unqualified_class,
            ty_mangled,
            mangle_ident(mname)
        );

        let expr = qualify_expr_ctors_for_instance_import(expr, inst);
        let expr = add_params_to_expr(ast::dummy_span(), expr, &extra_param_names);
        extra_items.push(ast::Item::Binding(ast::Binding {
            doc: None,
            pat: ast::Pattern::new(ast::dummy_span(), ast::PatternKind::Var(impl_name.clone())),
            expr,
            span: ast::dummy_span(),
        }));

        dict_fields.push((
            mname.clone(),
            ast::Expr::new(ast::dummy_span(), ast::ExprKind::Var(impl_name)),
        ));
    }

    for (sup, sup_dict_name) in direct_supers.into_iter().zip(super_dict_names.into_iter()) {
        dict_fields.push((
            super_field_name(&sup.name),
            ast::Expr::new(ast::dummy_span(), ast::ExprKind::Var(sup_dict_name)),
        ));
    }

    let ctx_params: Vec<String> = (0..inst.preds.len()).map(ctx_param_name).collect();
    let dict_expr = ast::Expr::new(ast::dummy_span(), ast::ExprKind::Record(dict_fields));
    let dict_expr = add_params_to_expr(ast::dummy_span(), dict_expr, &ctx_params);
    extra_items.push(ast::Item::Binding(ast::Binding {
        doc: None,
        pat: ast::Pattern::new(ast::dummy_span(), ast::PatternKind::Var(dict_name)),
        expr: dict_expr,
        span: ast::dummy_span(),
    }));

    Ok(())
}

fn qualify_expr_ctors_for_instance_import(expr: ast::Expr, inst: &ast::InstanceDecl) -> ast::Expr {
    // Only qualify constructors when the instance head type itself is qualified (imported
    // qualified / aliased). For local instances like `instance Applicative Maybe`, we must
    // not rewrite `Nothing` into `Maybe.Nothing`.
    fn instance_head_qual(inst: &ast::InstanceDecl) -> Option<&str> {
        let ast::Type::Var(h) = instance_head(inst)? else {
            return None;
        };
        // For an instance head like `P.Maybe` (alias-qualified import), constructors are
        // accessible as `P.Nothing` / `P.Just`.
        h.split_once('.').map(|(q, _)| q)
    }

    fn instance_head(inst: &ast::InstanceDecl) -> Option<&ast::Type> {
        match &inst.ty {
            ast::Type::App { head, .. } => Some(head),
            t => Some(t),
        }
    }

    let Some(qual) = instance_head_qual(inst) else {
        return expr;
    };
    if qual.is_empty() {
        return expr;
    }

    qualify_expr_ctors_recursive(expr, qual)
}

fn qualify_expr_ctors_recursive(mut e: ast::Expr, qual: &str) -> ast::Expr {
    match &mut e.kind {
        ast::ExprKind::Ctor(n) => {
            if n.is_unresolved_eq("Nothing") {
                *n = ast::ResolvedName::unresolved(format!("{qual}.Nothing"));
            }
            if n.is_unresolved_eq("Just") {
                *n = ast::ResolvedName::unresolved(format!("{qual}.Just"));
            }
        }
        ast::ExprKind::Lambda { body, .. } => {
            **body = qualify_expr_ctors_recursive((**body).clone(), qual);
        }
        ast::ExprKind::Apply { func, args } => {
            **func = qualify_expr_ctors_recursive((**func).clone(), qual);
            for a in args {
                *a = qualify_expr_ctors_recursive(a.clone(), qual);
            }
        }
        ast::ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            **cond = qualify_expr_ctors_recursive((**cond).clone(), qual);
            **then_branch = qualify_expr_ctors_recursive((**then_branch).clone(), qual);
            **else_branch = qualify_expr_ctors_recursive((**else_branch).clone(), qual);
        }
        ast::ExprKind::Let { bindings, body } => {
            for b in bindings {
                b.expr = qualify_expr_ctors_recursive(b.expr.clone(), qual);
            }
            **body = qualify_expr_ctors_recursive((**body).clone(), qual);
        }
        ast::ExprKind::Where { expr, bindings } => {
            for b in bindings {
                b.expr = qualify_expr_ctors_recursive(b.expr.clone(), qual);
            }
            **expr = qualify_expr_ctors_recursive((**expr).clone(), qual);
        }
        ast::ExprKind::Annot { expr, .. } => {
            **expr = qualify_expr_ctors_recursive((**expr).clone(), qual);
        }
        ast::ExprKind::Do(stmts) => {
            for s in stmts {
                match s {
                    ast::DoStmt::Bind { pat: _, expr } => {
                        *expr = qualify_expr_ctors_recursive(expr.clone(), qual);
                    }
                    ast::DoStmt::Expr(e) => {
                        *e = qualify_expr_ctors_recursive(e.clone(), qual);
                    }
                }
            }
        }
        ast::ExprKind::Case { expr, arms } => {
            **expr = qualify_expr_ctors_recursive((**expr).clone(), qual);
            for a in arms {
                if let Some(g) = &mut a.guard {
                    *g = qualify_expr_ctors_recursive(g.clone(), qual);
                }
                a.body = qualify_expr_ctors_recursive(a.body.clone(), qual);
            }
        }
        ast::ExprKind::Cons { head, tail } => {
            **head = qualify_expr_ctors_recursive((**head).clone(), qual);
            **tail = qualify_expr_ctors_recursive((**tail).clone(), qual);
        }
        ast::ExprKind::List(es) | ast::ExprKind::Tuple(es) => {
            for x in es {
                *x = qualify_expr_ctors_recursive(x.clone(), qual);
            }
        }
        ast::ExprKind::Record(fields) => {
            for (_, v) in fields {
                *v = qualify_expr_ctors_recursive(v.clone(), qual);
            }
        }
        _ => {}
    }
    e
}

fn resolve_instance_dict_name(
    env: &ClassEnv,
    inst: &ast::InstanceDecl,
) -> Result<(Option<String>, String, String)> {
    match instance_head_key_ast(&inst.ty) {
        Ok(ty_key) => {
            let ty_mangled = mangle_ident(&ty_key);
            let dict_key = (inst.class.clone(), ty_key.clone());
            let dict_name = env
                .instances
                .get(&dict_key)
                .cloned()
                .ok_or_else(|| Error::msg("internal: missing instance dict name"))?;
            Ok((Some(ty_key), ty_mangled, dict_name))
        }
        Err(_) => {
            let mut cx = InferCtx::default();
            let head_pat = lower_surface_type(&mut cx, &inst.ty, &mut HashMap::new());
            let Some(pi) = env.poly_instances.iter().find(|pi| {
                pi.class == inst.class && pi.head_pat == head_pat && pi.ctx_len == inst.preds.len()
            }) else {
                return Err(Error::msg("internal: missing poly instance"));
            };
            let ty_mangled = mangle_ident(&pi.dict_name);
            Ok((None, ty_mangled, pi.dict_name.clone()))
        }
    }
}

fn collect_instance_methods(inst: &ast::InstanceDecl) -> Result<HashMap<String, ast::Expr>> {
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
    Ok(inst_methods)
}

fn collect_direct_supers(env: &ClassEnv, inst: &ast::InstanceDecl) -> Vec<ast::ClassId> {
    let mut direct_supers: Vec<ast::ClassId> = Vec::new();
    if let Some(supers) = env.class_supers.get(&inst.class) {
        for p in supers {
            if let ast::Predicate::Class { class: sup, .. } = p {
                direct_supers.push(sup.clone());
            }
        }
    }
    direct_supers.sort();
    direct_supers.dedup();
    direct_supers
}

fn build_extra_param_names(inst: &ast::InstanceDecl) -> Result<Vec<String>> {
    let mut extra_param_names: Vec<String> = vec![dict_param_name(&inst.class.name)];
    let mut ctx_dict_by_class: HashMap<String, String> = HashMap::new();
    for (i, p) in inst.preds.iter().enumerate() {
        let Some(cls) = class_name_of_pred(p) else {
            return Err(Error::msg(
                "MVP: instance context supports only class predicates (C t)",
            ));
        };
        if ctx_dict_by_class.contains_key(cls) {
            return Err(Error::msg(
                "MVP: duplicate class in instance context is not supported",
            ));
        }
        let pname = ctx_param_name(i);
        extra_param_names.push(pname.clone());
        ctx_dict_by_class.insert(cls.to_string(), pname);
    }
    Ok(extra_param_names)
}

fn resolve_super_dict_names(
    env: &ClassEnv,
    inst: &ast::InstanceDecl,
    ty_key_opt: Option<&String>,
    direct_supers: &[ast::ClassId],
) -> Result<Vec<String>> {
    let mut ctx_dict_by_class: HashMap<String, String> = HashMap::new();
    for (i, p) in inst.preds.iter().enumerate() {
        if let Some(cls) = class_name_of_pred(p) {
            ctx_dict_by_class.insert(cls.to_string(), ctx_param_name(i));
        }
    }

    let mut super_dict_names: Vec<String> = Vec::new();
    for sup in direct_supers {
        if let Some(pname) = ctx_dict_by_class.get(&sup.name) {
            super_dict_names.push(pname.clone());
            continue;
        }

        let Some(ty_key) = ty_key_opt else {
            return Err(Error::msg(
                "MVP: superclass resolution for non-ground instance heads is not supported yet",
            ));
        };
        let sup_key = (sup.clone(), ty_key.clone());
        let Some(sup_dict_name) = env.instances.get(&sup_key) else {
            return Err(Error::msg(format!(
                "missing superclass instance required by `{}`: {} {}",
                inst.class.name, sup.name, ty_key
            )));
        };
        super_dict_names.push(sup_dict_name.clone());
    }
    Ok(super_dict_names)
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
        PatternKind::Constructor { name, .. } => out.push(format!(
            "ctor:{}",
            unqual_name_last_segment(&name.qualified_text())
        )),
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

    // If we got an unqualified head name but the stored constructors are qualified
    // (contain dots), we might be looking at stdlib metadata while the local module
    // defines its own ADT with the same unqualified name. Treat this as best-effort
    // and avoid false negatives.
    if !ty_name.contains('.') && d.ctors.iter().any(|c| c.name.contains('.')) {
        return Ok(Some(Ok(())));
    }

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
        ast::Predicate::Lacks { .. } => {
            unreachable!("internal error: Lacks predicate is not allowed in superclass constraints")
        }
    }
}

/// Check if a polymorphic instance head pattern matches a ground type.
/// Returns Some(()) if it matches (unifies), None otherwise.
fn unify_instance_head(pattern: &Ty, concrete: &Ty) -> Option<()> {
    use std::collections::HashMap;

    fn unify_helper(pat: &Ty, con: &Ty, subst: &mut HashMap<u32, Ty>) -> bool {
        match (pat, con) {
            (Ty::Var(v), _) => {
                // Pattern variable can match any concrete type
                if let Some(bound) = subst.get(v).cloned() {
                    // Already bound, check consistency
                    unify_helper(&bound, con, subst)
                } else {
                    // Bind the variable
                    subst.insert(*v, con.clone());
                    true
                }
            }
            (Ty::Con(p_name), Ty::Con(c_name)) => p_name == c_name,
            (Ty::List(p), Ty::List(c)) => unify_helper(p, c, subst),
            (Ty::Tuple(p_ts), Ty::Tuple(c_ts)) => {
                if p_ts.len() != c_ts.len() {
                    return false;
                }
                for (p_t, c_t) in p_ts.iter().zip(c_ts.iter()) {
                    if !unify_helper(p_t, c_t, subst) {
                        return false;
                    }
                }
                true
            }
            (Ty::Func(p_a, p_b), Ty::Func(c_a, c_b)) => {
                unify_helper(p_a, c_a, subst) && unify_helper(p_b, c_b, subst)
            }
            (Ty::Record(p_fields), Ty::Record(c_fields)) => {
                if p_fields.len() != c_fields.len() {
                    return false;
                }

                let mut p_sorted = p_fields.clone();
                let mut c_sorted = c_fields.clone();
                p_sorted.sort_by(|(a, _), (b, _)| a.cmp(b));
                c_sorted.sort_by(|(a, _), (b, _)| a.cmp(b));

                for ((p_k, p_t), (c_k, c_t)) in p_sorted.iter().zip(c_sorted.iter()) {
                    if p_k != c_k {
                        return false;
                    }
                    if !unify_helper(p_t, c_t, subst) {
                        return false;
                    }
                }
                true
            }
            (Ty::RecordOpen(p_req, p_rest), Ty::Record(c_fields)) => {
                // pat: { req..., ...rest }  con: { fields... }
                // Require all required fields to exist, and unify rest with the residual fields.
                let mut c_map: std::collections::BTreeMap<&str, &Ty> =
                    std::collections::BTreeMap::new();
                for (k, t) in c_fields {
                    c_map.insert(k.as_str(), t);
                }

                for (p_k, p_t) in p_req {
                    let Some(c_t) = c_map.get(p_k.as_str()) else {
                        return false;
                    };
                    if !unify_helper(p_t, c_t, subst) {
                        return false;
                    }
                }

                let mut residual: Vec<(String, Ty)> = Vec::new();
                for (k, t) in c_fields {
                    if !p_req.iter().any(|(pk, _)| pk == k) {
                        residual.push((k.clone(), t.clone()));
                    }
                }
                residual.sort_by(|(a, _), (b, _)| a.cmp(b));
                unify_helper(p_rest, &Ty::Record(residual), subst)
            }
            (Ty::RecordOpen(p_req, p_rest), Ty::RecordOpen(c_req, c_rest)) => {
                // Conservative structural match: required field set must align (up to order),
                // and residual row types must unify.
                if p_req.len() != c_req.len() {
                    return false;
                }
                let mut p_sorted = p_req.clone();
                let mut c_sorted = c_req.clone();
                p_sorted.sort_by(|(a, _), (b, _)| a.cmp(b));
                c_sorted.sort_by(|(a, _), (b, _)| a.cmp(b));
                for ((p_k, p_t), (c_k, c_t)) in p_sorted.iter().zip(c_sorted.iter()) {
                    if p_k != c_k {
                        return false;
                    }
                    if !unify_helper(p_t, c_t, subst) {
                        return false;
                    }
                }
                unify_helper(p_rest, c_rest, subst)
            }
            (
                Ty::App {
                    head: p_head,
                    args: p_args,
                },
                Ty::App {
                    head: c_head,
                    args: c_args,
                },
            ) => {
                if p_args.len() != c_args.len() {
                    return false;
                }
                if !unify_helper(p_head, c_head, subst) {
                    return false;
                }
                for (p_arg, c_arg) in p_args.iter().zip(c_args.iter()) {
                    if !unify_helper(p_arg, c_arg, subst) {
                        return false;
                    }
                }
                true
            }
            _ => false,
        }
    }

    let mut subst = HashMap::new();
    if unify_helper(pattern, concrete, &mut subst) {
        Some(())
    } else {
        None
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
    fn find_class_by_name(class_env: &ClassEnv, name: &str) -> Option<ast::ClassId> {
        // Prefer looking up by a distinctive method name when possible.
        // This is robust even if class names are qualified (e.g. `Prelude.Eq`).
        let method_hint = match name {
            "Eq" => Some("eq"),
            "Show" => Some("show"),
            _ => None,
        };
        if let Some(m) = method_hint {
            if let Some(classes) = class_env.method_classes.get(m) {
                if let Some(cid) = classes
                    .iter()
                    .find(|cid| cid.name == name || cid.name.ends_with(&format!(".{name}")))
                {
                    return Some(cid.clone());
                }
                if let Some(cid) = classes.first() {
                    return Some(cid.clone());
                }
            }
        }

        let mut fallback = None;
        for cid in class_env.class_params.keys() {
            if cid.name == name || cid.name.ends_with(&format!(".{name}")) {
                // Prefer resolved module ids.
                if cid.module.0 != 0 {
                    return Some(cid.clone());
                }
                fallback = Some(cid.clone());
            }
        }
        fallback
    }

    match c {
        Constraint::Show(t) => {
            if let Some(class) = find_class_by_name(class_env, "Show") {
                work.push_back(Constraint::Class { class, ty: t });
            } else {
                out.extend(entails_show(data_env, &t, in_progress)?);
            }
        }
        Constraint::ShowRow(t) => out.extend(entails_show_row(data_env, &t, in_progress)?),
        Constraint::Eq(t) => {
            if let Some(class) = find_class_by_name(class_env, "Eq") {
                work.push_back(Constraint::Class { class, ty: t });
            } else {
                out.extend(entails_eq(data_env, &t, in_progress)?);
            }
        }
        Constraint::EqRow(t) => out.extend(entails_eq_row(data_env, &t, in_progress)?),
        Constraint::Lacks { label, row } => out.extend(entails_lacks(&label, &row)?),
        Constraint::Class { class, ty } => {
            // Even with ordinary typeclasses, we keep the historical restriction that
            // `Show` cannot be satisfied for function types.
            // This catches cases like: `show (\\y -> y)` which would otherwise
            // generalize to an unsatisfiable/ambiguous constraint.
            if (class.name == "Show" || class.name.ends_with(".Show"))
                && matches!(ty, Ty::Func(_, _))
            {
                return Err(Error::msg("cannot satisfy constraint: Show (function)"));
            }

            let expand_key = format!("{}:{ty:?}", class.name);
            if expanded.insert(expand_key, ()).is_none() {
                if let Some(supers) = class_env.class_supers.get(&class) {
                    for p in supers {
                        work.push_back(lower_super_predicate_for_constraints(p, &ty));
                    }
                }
            }

            if !ftv_ty(&ty).is_empty() {
                // Keep the class constraint deferred, but still emit structural row
                // constraints for open records (MVP behavior).
                if matches!(ty, Ty::RecordOpen(_, _)) {
                    if class.name == "Show" || class.name.ends_with(".Show") {
                        for c2 in entails_show(data_env, &ty, in_progress)? {
                            work.push_back(c2);
                        }
                    }
                    if class.name == "Eq" || class.name.ends_with(".Eq") {
                        for c2 in entails_eq(data_env, &ty, in_progress)? {
                            work.push_back(c2);
                        }
                    }
                }

                out.push(Constraint::Class { class, ty });
            } else {
                let ty_norm = normalize_ty_for_instance_key(&ty);
                let key_ty = instance_head_key_ty(&ty_norm)?;
                let key = (class.clone(), key_ty.clone());

                // First check concrete instances
                let has_concrete = class_env.instances.contains_key(&key);

                // Also check polymorphic instances
                let has_poly = class_env.poly_instances.iter().any(|pi| {
                    pi.class == class && unify_instance_head(&pi.head_pat, &ty_norm).is_some()
                });

                if !has_concrete && !has_poly {
                    if std::env::var("KSCR_DEBUG_EQ_INTEGER").ok().as_deref() == Some("1")
                        && (class.name == "Prelude.Eq" || class.name.ends_with(".Eq"))
                    {
                        eprintln!(
                            "[KSCR_DEBUG_EQ_INTEGER] simplify failure: class={} module={:?} ty={:?} key_ty={}",
                            class.name,
                            class.module,
                            ty_norm,
                            key_ty
                        );
                        eprintln!(
                            "[KSCR_DEBUG_EQ_INTEGER] class_env.instances.len()={} class_env.class_params.len()={}",
                            class_env.instances.len(),
                            class_env.class_params.len()
                        );
                        let same_name: Vec<_> = class_env
                            .instances
                            .keys()
                            .filter(|(c, _)| c.name == class.name)
                            .map(|(c, t)| format!("{} module={:?} head={}", c.name, c.module, t))
                            .collect();
                        eprintln!(
                            "[KSCR_DEBUG_EQ_INTEGER] instances with same class name: {:?}",
                            same_name
                        );
                        let eqish: Vec<_> = class_env
                            .instances
                            .keys()
                            .filter(|(c, _)| c.name.contains("Eq"))
                            .take(30)
                            .map(|(c, t)| format!("{} module={:?} head={}", c.name, c.module, t))
                            .collect();
                        eprintln!(
                            "[KSCR_DEBUG_EQ_INTEGER] sample instances containing 'Eq': {:?}",
                            eqish
                        );
                    }
                    return Err(Error::msg(format!(
                        "cannot satisfy constraint: {} {ty}",
                        class.name
                    )));
                }
            }
        }
    }
    Ok(())
}

fn is_superclass_of_class_env(
    class_env: &ClassEnv,
    sub: &ast::ClassId,
    sup: &ast::ClassId,
) -> bool {
    use std::collections::{HashSet, VecDeque};

    if sub == sup {
        return false;
    }

    let mut seen: HashSet<ast::ClassId> = HashSet::new();
    let mut q: VecDeque<ast::ClassId> = VecDeque::new();
    q.push_back(sub.clone());

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

fn simplify_context_reduce_user_classes(
    class_env: &ClassEnv,
    cs: Vec<Constraint>,
) -> Vec<Constraint> {
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

fn stdlib_derives_show(ty_name: &str) -> bool {
    // Hardcoded list of stdlib types that derive Show.
    // TODO: Proper fix requires exporting deriving info in KSIF.
    matches!(ty_name, "Rational")
}

fn data_derives_eq(d: &ast::DataDecl) -> bool {
    d.deriving.iter().any(|c| c == "Eq")
}

fn stdlib_derives_eq(ty_name: &str) -> bool {
    // Hardcoded list of stdlib types that derive Eq.
    // TODO: Proper fix requires exporting deriving info in KSIF.
    matches!(ty_name, "Rational")
}

fn entails_show(data_env: &DataEnv, ty: &Ty, in_progress: &mut Vec<Ty>) -> Result<Vec<Constraint>> {
    Ok(match ty {
        Ty::Var(_) => vec![Constraint::Show(ty.clone())],
        Ty::Con(name) => {
            if show_primitives(name) {
                vec![]
            } else if stdlib_derives_show(name) {
                // Hardcoded list of stdlib types that derive Show but are in separate modules.
                // TODO: Proper fix requires exporting deriving info in KSIF.
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
            } else if stdlib_derives_eq(name) {
                // Hardcoded list of stdlib types that derive Eq but are in separate modules.
                // TODO: Proper fix requires exporting deriving info in KSIF.
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

fn rewrite_entry_main_apply_dicts(
    module: &mut ast::Module,
    class_env: &ClassEnv,
    main_cs: &[Constraint],
) -> Result<()> {
    use ast::{Expr, ExprKind, Item, Pattern, PatternKind};

    if main_cs.is_empty() {
        return Ok(());
    }

    let mut dict_args: Vec<Expr> = Vec::new();
    for c in main_cs {
        let Constraint::Class { class, ty } = c else {
            return Err(Error::msg("main must have type IO _"));
        };
        let ty_key = instance_head_key_ty_for_class(class_env, class, ty)?;
        let key = (class.clone(), ty_key);
        let Some(dict_name) = class_env.instances.get(&key) else {
            return Err(Error::msg("main must have type IO _"));
        };
        dict_args.push(Expr::dummy(ExprKind::Var(dict_name.clone())));
    }

    // Rename the original `main` binding and insert a wrapper that applies the
    // required dictionaries, so the runnable entrypoint has type `IO _`.
    let mut impl_name = "__main_impl".to_string();
    if module
        .items
        .iter()
        .any(|it| matches!(it, Item::Binding(b) if matches!(&b.pat.kind, PatternKind::Var(n) if n == &impl_name)))
    {
        let mut i = 0usize;
        loop {
            let candidate = format!("__main_impl{i}");
            if !module.items.iter().any(|it| {
                matches!(it, Item::Binding(b) if matches!(&b.pat.kind, PatternKind::Var(n) if n == &candidate))
            }) {
                impl_name = candidate;
                break;
            }
            i += 1;
        }
    }

    let mut found = false;
    for it in &mut module.items {
        let Item::Binding(b) = it else {
            continue;
        };
        let PatternKind::Var(name) = &mut b.pat.kind else {
            continue;
        };
        if name == "main" {
            *name = impl_name.clone();
            found = true;
            break;
        }
    }
    if !found {
        return Ok(());
    }

    let wrapper_expr = Expr::dummy(ExprKind::Apply {
        func: Box::new(Expr::dummy(ExprKind::Var(impl_name))),
        args: dict_args,
    });
    module.items.push(Item::Binding(ast::Binding {
        doc: None,
        pat: Pattern::dummy(PatternKind::Var("main".to_string())),
        expr: wrapper_expr,
        span: ast::dummy_span(),
    }));

    Ok(())
}

fn build_letrec_binding_metadata(
    bindings: &[ast::Binding],
) -> (Vec<String>, Vec<HashSet<String>>, HashMap<String, usize>) {
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

fn infer_one_letrec_binding(
    cxi: &mut LetrecInferCx<'_>,
    b: ast::Binding,
) -> Result<LetrecBindingInfer> {
    let mut binds: Vec<(String, Ty)> = Vec::new();
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

    let expr_span = b.expr.span;
    let pat_span = b.pat.span;

    let (s_rhs, cs_rhs, t_rhs) =
        infer_expr_in(cxi.cx, cxi.data_env, cxi.subst, cxi.env_scc, b.expr).map_err(|e| {
            let mut e = e;
            let needs_primary = e.span().is_none_or(|s| s.start == s.end);
            if needs_primary {
                e = e.push_span(expr_span);
            }
            e.push_secondary_span(pat_span)
                .with_context(format!("in {} binding {}", cxi.ctx_prefix, cxi.ctx_name))
        })?;
    *cxi.subst = compose(&s_rhs, cxi.subst);

    let s_pat = unify(apply(cxi.subst, t_rhs), apply(cxi.subst, pat_ty)).map_err(|e| {
        e.push_span(expr_span)
            .push_secondary_span(pat_span)
            .with_context(format!("in {} binding {}", cxi.ctx_prefix, cxi.ctx_name))
    })?;
    *cxi.subst = compose(&s_pat, cxi.subst);

    // Connect binder types to their placeholders so recursive references unify.
    for (name, t) in &binds {
        if let Some(ph) = cxi.placeholders.get(name).cloned() {
            let su = unify(apply(cxi.subst, t.clone()), apply(cxi.subst, ph)).map_err(|e| {
                e.with_context(format!("in {} binding {}", cxi.ctx_prefix, cxi.ctx_name))
            })?;
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
    let mut env_global_ftv = ftv_env(&env_global);

    let n = bindings.len();
    if n == 0 {
        return Ok((s, env_global));
    }

    let (ctx_names, defined_names, name_to_binding) = build_letrec_binding_metadata(&bindings);
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
                env_scc.insert(
                    name.clone(),
                    EnvEntry {
                        scheme: Scheme::mono(tv),
                        def_site: None,
                    },
                );
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

        let env_gen_ftv = ftv_env_applied_from_ftv(&s, &env_global_ftv);
        let mut new_schemes: Vec<(String, Scheme)> = Vec::new();
        for (binds, cs) in per_bind {
            for (name, t) in binds {
                let cs = simplify_constraints(
                    data_env,
                    &cx.full_class_env,
                    apply_constraints(&s, cs.clone()),
                )?;
                let scheme = generalize_qual_with_env_ftv(&env_gen_ftv, cs, apply(&s, t));
                new_schemes.push((name, scheme));
            }
        }

        for (name, scheme) in new_schemes {
            env_global_ftv.extend(ftv_scheme(&scheme));
            env_global.insert(
                name,
                EnvEntry {
                    scheme,
                    def_site: None,
                },
            );
        }
    }

    Ok((s, env_global))
}

fn infer_expr_in(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    subst_env: &Subst,
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
            // Haskell-like behavior: class methods are also available as ordinary values.
            //
            // We model methods as overloaded functions in the type environment
            // (`add_class_methods_into_env`). When import-forwarders don't expose a
            // method name as a value, allow falling back to the module-scope class env.
            let from_env = env.get(&name).map(|e| apply_scheme(subst_env, &e.scheme));
            let from_methods = cx
                .class_env
                .methods_by_name
                .get(&name)
                .map(|sch| apply_scheme(subst_env, sch));

            if std::env::var("KSCR_DEBUG_METHOD_VALUES").ok().as_deref() == Some("1")
                && from_env.is_none()
                && from_methods.is_some()
            {
                eprintln!("[KSCR_DEBUG_METHOD_VALUES] Var fallback hit: {name}");
            }

            let s = from_env
                .or(from_methods)
                .ok_or_else(|| Error::msg_with_span(format!("unbound variable: {name}"), span))?;
            let (cs, ty) = instantiate_qual(cx, &s);
            Ok((Subst::new(), cs, ty))
        }

        ExprKind::Ctor(name) => {
            let key = name.qualified_text();
            let entry = env.get(&key).ok_or_else(|| {
                let hint = TL_NAME_HINTS.with(|h| format_unknown_ctor_name_hint(&key, &h.borrow()));
                Error::msg_with_span(format!("unknown constructor: {key}{hint}"), span)
            })?;
            let s = apply_scheme(subst_env, &entry.scheme);
            let (cs, ty) = instantiate_qual(cx, &s);
            Ok((Subst::new(), cs, ty))
        }

        ExprKind::Lambda { params, body } => {
            infer_expr_lambda(cx, data_env, subst_env, env, span, params, *body)
        }

        ExprKind::Apply { func, args } => {
            infer_expr_apply(cx, data_env, subst_env, env, *func, args)
        }

        ExprKind::Annot { expr, ty } => infer_expr_annot(cx, data_env, subst_env, env, *expr, ty),

        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => infer_expr_if(
            cx,
            data_env,
            subst_env,
            env,
            *cond,
            *then_branch,
            *else_branch,
        ),

        ExprKind::Tuple(elems) => infer_expr_tuple(cx, data_env, subst_env, env, elems),

        ExprKind::Cons { head, tail } => {
            infer_expr_cons(cx, data_env, subst_env, env, *head, *tail)
        }

        ExprKind::List(elems) => infer_expr_list(cx, data_env, subst_env, env, elems),

        ExprKind::Record(fields) => infer_expr_record(cx, data_env, subst_env, env, fields),

        ExprKind::Let { bindings, body } => {
            let (s_bind, env2) = infer_local_letrec_bindings(cx, data_env, env, bindings, "let")?;
            let subst_body = compose(&s_bind, subst_env);
            let (s_body, cs_body, t_body) = infer_expr_in(cx, data_env, &subst_body, &env2, *body)
                .map_err(|e| e.with_context("in let body"))?;
            let s = compose(&s_body, &s_bind);
            Ok((s.clone(), apply_constraints(&s, cs_body), apply(&s, t_body)))
        }

        ExprKind::Where { expr, bindings } => {
            let (s_bind, env2) = infer_local_letrec_bindings(cx, data_env, env, bindings, "where")?;
            let subst_body = compose(&s_bind, subst_env);
            let (s_body, cs_body, t_body) = infer_expr_in(cx, data_env, &subst_body, &env2, *expr)
                .map_err(|e| e.with_context("in where body"))?;
            let s = compose(&s_body, &s_bind);
            Ok((s.clone(), apply_constraints(&s, cs_body), apply(&s, t_body)))
        }

        ExprKind::Case { expr, arms } => {
            infer_expr_case(cx, data_env, subst_env, env, span, *expr, arms)
        }

        ExprKind::Do(stmts) => infer_expr_do(cx, data_env, subst_env, env, span, stmts),
    }
}

fn infer_expr_lambda(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    subst_env: &Subst,
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
        env2.insert(
            p.clone(),
            EnvEntry {
                scheme: Scheme::mono(tv.clone()),
                def_site: None,
            },
        );
        param_tys.push(tv);
    }

    let (s_body, cs_body, body_ty) = infer_expr_in(cx, data_env, subst_env, &env2, body)?;
    let mut out = apply(&s_body, body_ty);
    for pty in param_tys.into_iter().rev() {
        out = Ty::Func(Box::new(apply(&s_body, pty)), Box::new(out));
    }

    Ok((s_body, cs_body, out))
}

fn infer_expr_apply(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    subst_env: &Subst,
    env: &TypeEnv,
    func: ast::Expr,
    args: Vec<ast::Expr>,
) -> Result<(Subst, Vec<Constraint>, Ty)> {
    let apply_span = crate::lexer::Span {
        start: func.span.start,
        end: args.last().map(|a| a.span.end).unwrap_or(func.span.end),
    };

    let (mut s, mut cs, mut fun_ty) = infer_expr_in(cx, data_env, subst_env, env, func)?;

    for arg in args {
        let subst2 = compose(&s, subst_env);
        let (s_arg, cs_arg, arg_ty) = infer_expr_in(cx, data_env, &subst2, env, arg)?;
        s = compose(&s_arg, &s);

        cs = apply_constraints(&s, cs);
        cs.extend(apply_constraints(&s, cs_arg));

        fun_ty = apply(&s, fun_ty);
        let res = cx.fresh();

        let s_unify = unify_dbg(
            fun_ty,
            Ty::Func(Box::new(apply(&s, arg_ty.clone())), Box::new(res.clone())),
            "infer_expr_apply",
        )
        .map_err(|e| e.push_span(apply_span).with_context("infer_expr_apply"))?;
        s = compose(&s_unify, &s);
        cs = apply_constraints(&s, cs);
        fun_ty = apply(&s, res);
    }

    Ok((s, cs, fun_ty))
}

fn infer_expr_annot(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    subst_env: &Subst,
    env: &TypeEnv,
    expr: ast::Expr,
    ty: ast::QualType,
) -> Result<(Subst, Vec<Constraint>, Ty)> {
    // `ExprKind::Annot { expr, ty }` currently stores only a single `Expr.span`.
    // Use that span both as primary and as the annotation-site secondary span.
    // Better locations can be added by callers (e.g. binding RHS/pattern spans).
    let annot_span = expr.span;
    let inner_expr_span = expr.span;
    let (s1, mut cs1, t1) = infer_expr_in(cx, data_env, subst_env, env, expr)?;
    let mut holes = HashMap::new();

    if std::env::var("KSCR_DEBUG_ALIAS_EVIDENCE").ok().as_deref() == Some("1") {
        TL_NAME_HINTS.with(|h| {
            eprintln!(
                "[KSCR_DEBUG_ALIAS_EVIDENCE] type_alias hints: {:?}",
                h.borrow().type_alias
            );
        });
    }

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
    let s2 = unify_dbg(
        apply(&s1, t1),
        apply(&s1, t_ann.clone()),
        "infer_expr_annot",
    )
    .map_err(|e| {
        // Primary: inner expression, Secondary: annotation site.
        // If spans are missing (0-length), other layers may attach better locations.
        e.push_span(inner_expr_span)
            .push_secondary_span(annot_span)
            .with_context("infer_expr_annot")
    })?;
    let s = compose(&s2, &s1);
    Ok((s.clone(), apply_constraints(&s, cs1), apply(&s, t_ann)))
}

fn infer_expr_if(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    subst_env: &Subst,
    env: &TypeEnv,
    cond: ast::Expr,
    then_branch: ast::Expr,
    else_branch: ast::Expr,
) -> Result<(Subst, Vec<Constraint>, Ty)> {
    let (s_cond, cs_cond, t_cond) = infer_expr_in(cx, data_env, subst_env, env, cond)
        .map_err(|e| e.with_context("in if cond"))?;
    let s_bool = unify_dbg(
        apply(&s_cond, t_cond),
        Ty::Con("Bool".to_string()),
        "infer_expr_if:cond",
    )
    .map_err(|e| e.with_context("in if cond"))?;
    let mut s = compose(&s_bool, &s_cond);
    let mut cs = apply_constraints(&s, cs_cond);

    let subst2 = compose(&s, subst_env);
    let (s_then, cs_then, t_then) = infer_expr_in(cx, data_env, &subst2, env, then_branch)
        .map_err(|e| e.with_context("in if then"))?;
    s = compose(&s_then, &s);
    cs = apply_constraints(&s, cs);
    cs.extend(apply_constraints(&s, cs_then));

    let subst3 = compose(&s, subst_env);
    let (s_else, cs_else, t_else) = infer_expr_in(cx, data_env, &subst3, env, else_branch)
        .map_err(|e| e.with_context("in if else"))?;
    s = compose(&s_else, &s);
    cs = apply_constraints(&s, cs);
    cs.extend(apply_constraints(&s, cs_else));

    let s_res = unify_dbg(
        apply(&s, t_then.clone()),
        apply(&s, t_else),
        "infer_expr_if:branches",
    )
    .map_err(|e| e.with_context("in if branches"))?;
    s = compose(&s_res, &s);
    cs = apply_constraints(&s, cs);
    Ok((s.clone(), cs, apply(&s, apply(&s, t_then))))
}

fn infer_expr_tuple(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    subst_env: &Subst,
    env: &TypeEnv,
    elems: Vec<ast::Expr>,
) -> Result<(Subst, Vec<Constraint>, Ty)> {
    let mut s = Subst::new();
    let mut cs: Vec<Constraint> = vec![];
    let mut ts = Vec::new();
    for e in elems {
        let subst2 = compose(&s, subst_env);
        let (s_e, cs_e, t_e) = infer_expr_in(cx, data_env, &subst2, env, e)?;
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
    subst_env: &Subst,
    env: &TypeEnv,
    head: ast::Expr,
    tail: ast::Expr,
) -> Result<(Subst, Vec<Constraint>, Ty)> {
    let (s_hd, cs_hd, t_hd) = infer_expr_in(cx, data_env, subst_env, env, head)?;
    let subst2 = compose(&s_hd, subst_env);
    let (s_tl, cs_tl, t_tl) = infer_expr_in(cx, data_env, &subst2, env, tail)?;
    let mut s = compose(&s_tl, &s_hd);
    let mut cs = apply_constraints(&s, cs_hd);
    cs.extend(apply_constraints(&s, cs_tl));

    let elem = cx.fresh();
    let su_tl = unify_dbg(
        apply(&s, t_tl),
        Ty::List(Box::new(elem.clone())),
        "infer_expr_cons:tail",
    )?;
    s = compose(&su_tl, &s);
    cs = apply_constraints(&s, cs);

    let su_hd = unify_dbg(
        apply(&s, t_hd),
        apply(&s, elem.clone()),
        "infer_expr_cons:head",
    )?;
    s = compose(&su_hd, &s);
    cs = apply_constraints(&s, cs);

    Ok((s.clone(), cs, Ty::List(Box::new(apply(&s, elem)))))
}

fn infer_expr_list(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    subst_env: &Subst,
    env: &TypeEnv,
    elems: Vec<ast::Expr>,
) -> Result<(Subst, Vec<Constraint>, Ty)> {
    if elems.is_empty() {
        return Ok((Subst::new(), vec![], Ty::List(Box::new(cx.fresh()))));
    }

    let (mut s, mut cs, first_ty) = infer_expr_in(cx, data_env, subst_env, env, elems[0].clone())?;
    let mut elem_ty = apply(&s, first_ty);

    for e in elems.into_iter().skip(1) {
        let subst2 = compose(&s, subst_env);
        let (s_e, cs_e, t_e) = infer_expr_in(cx, data_env, &subst2, env, e)?;
        s = compose(&s_e, &s);
        cs = apply_constraints(&s, cs);
        cs.extend(apply_constraints(&s, cs_e));

        let su = unify_dbg(
            apply(&s, elem_ty.clone()),
            apply(&s, t_e),
            "infer_expr_list:elem",
        )?;
        s = compose(&su, &s);
        cs = apply_constraints(&s, cs);
        elem_ty = apply(&s, elem_ty);
    }

    Ok((s.clone(), cs, Ty::List(Box::new(apply(&s, elem_ty)))))
}

fn infer_expr_record(
    cx: &mut InferCtx,
    data_env: &DataEnv,
    subst_env: &Subst,
    env: &TypeEnv,
    fields: Vec<(String, ast::Expr)>,
) -> Result<(Subst, Vec<Constraint>, Ty)> {
    let mut s = Subst::new();
    let mut cs: Vec<Constraint> = vec![];
    let mut out = Vec::new();
    for (name, e) in fields {
        let subst2 = compose(&s, subst_env);
        let (s_e, cs_e, t_e) = infer_expr_in(cx, data_env, &subst2, env, e)?;
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
    subst_env: &Subst,
    env: &TypeEnv,
    span: ast::Span,
    expr: ast::Expr,
    arms: Vec<ast::CaseArm>,
) -> Result<(Subst, Vec<Constraint>, Ty)> {
    if arms.is_empty() {
        return Err(Error::msg_with_span("empty case", span));
    }

    let (mut s, mut cs, scrut_ty) =
        infer_expr_in(cx, data_env, subst_env, env, expr).map_err(|e| {
            if std::env::var("KSCR_DEBUG_CASE_UNIFY").ok().as_deref() == Some("1") {
                eprintln!("[KSCR_DEBUG_CASE_UNIFY] scrutinee inference failed: {e}");
            }
            e.with_context("in case scrutinee")
        })?;
    let mut out_ty = cx.fresh();

    let mut pats_for_exhaustive_check: Vec<(ast::Pattern, bool)> = Vec::new();

    for (i, arm) in arms.into_iter().enumerate() {
        let arm_no = i + 1;
        let ast::CaseArm { pat, guard, body } = arm;

        pats_for_exhaustive_check.push((pat.clone(), guard.is_some()));

        let mut binds: Vec<(String, Ty)> = Vec::new();
        let mut seen = HashSet::new();
        let mut cs_pat = Vec::new();
        let pat_ty = infer_pat_in(
            cx,
            data_env,
            &mut s,
            env,
            &pat,
            &mut binds,
            &mut seen,
            &mut cs_pat,
        )
        .map_err(|e| e.with_context(format!("in case arm {arm_no}")))?;

        let pat_ty_applied = apply(&s, pat_ty);
        let scrut_ty_applied = apply(&s, scrut_ty.clone());
        let su_pat = unify_dbg(
            pat_ty_applied.clone(),
            scrut_ty_applied.clone(),
            &format!("infer_expr_case:arm {arm_no}:pat_vs_scrut"),
        )
        .map_err(|e| {
            if std::env::var("KSCR_DEBUG_CASE_UNIFY").ok().as_deref() == Some("1") {
                eprintln!("[KSCR_DEBUG_CASE_UNIFY] arm {arm_no}");
                eprintln!("  pat:   {pat_ty_applied}");
                eprintln!("  scrut: {scrut_ty_applied}");
            }
            e.with_context(format!("in case arm {arm_no}"))
        })?;
        s = compose(&su_pat, &s);
        cs = apply_constraints(&s, cs);
        cs.extend(apply_constraints(&s, cs_pat));

        let mut env_arm = env.clone();
        for (name, t) in binds {
            env_arm.insert(
                name,
                EnvEntry {
                    scheme: Scheme::mono(apply(&s, t)),
                    def_site: None,
                },
            );
        }

        let mut subst_arm = compose(&s, subst_env);

        if let Some(g) = guard {
            let (s_g, cs_g, t_g) = infer_expr_in(cx, data_env, &subst_arm, &env_arm, g)
                .map_err(|e| e.with_context(format!("in case arm {arm_no} guard")))?;
            s = compose(&s_g, &s);
            cs = apply_constraints(&s, cs);
            cs.extend(apply_constraints(&s, cs_g));

            let su_g = unify_dbg(
                apply(&s, t_g),
                Ty::Con("Bool".to_string()),
                &format!("infer_expr_case:arm {arm_no}:guard_bool"),
            )
            .map_err(|e| e.with_context(format!("in case arm {arm_no} guard")))?;
            s = compose(&su_g, &s);
            cs = apply_constraints(&s, cs);
            subst_arm = compose(&s, subst_env);
        }

        let (s_arm, cs_arm, arm_ty) = infer_expr_in(cx, data_env, &subst_arm, &env_arm, body)
            .map_err(|e| e.with_context(format!("in case arm {arm_no}")))?;
        s = compose(&s_arm, &s);
        cs = apply_constraints(&s, cs);
        cs.extend(apply_constraints(&s, cs_arm));

        let su_out = unify_dbg(
            apply(&s, out_ty.clone()),
            apply(&s, arm_ty),
            &format!("infer_expr_case:arm {arm_no}:out"),
        )
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
    subst_env: &Subst,
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
                let mut binds: Vec<(String, Ty)> = Vec::new();
                let mut seen = HashSet::new();
                let mut cs_pat = Vec::new();
                let pat_ty = infer_pat_in(
                    cx,
                    data_env,
                    &mut s,
                    &env2,
                    &pat,
                    &mut binds,
                    &mut seen,
                    &mut cs_pat,
                )
                .map_err(|e| e.with_context(format!("in do stmt {stmt_no} (<-)")))?;

                let subst_in = compose(&s, subst_env);
                let (s_e, cs_e, t_e) = infer_expr_in(cx, data_env, &subst_in, &env2, expr)
                    .map_err(|e| e.with_context(format!("in do stmt {stmt_no} (<-)")))?;
                s = compose(&s_e, &s);
                cs = apply_constraints(&s, cs);
                cs.extend(apply_constraints(&s, cs_e));

                let io_r = cx.fresh();
                let su = unify_dbg(
                    apply(&s, t_e),
                    Ty::App {
                        head: Box::new(Ty::Con("IO".to_string())),
                        args: vec![io_r.clone()],
                    },
                    &format!("infer_expr_do:stmt {stmt_no}:bind_io"),
                )
                .map_err(|e| e.with_context(format!("in do stmt {stmt_no} (<-)")))?;
                s = compose(&su, &s);
                cs = apply_constraints(&s, cs);

                let su_pat = unify_dbg(
                    apply(&s, pat_ty),
                    apply(&s, io_r.clone()),
                    &format!("infer_expr_do:stmt {stmt_no}:bind_pat"),
                )
                .map_err(|e| e.with_context(format!("in do stmt {stmt_no} (<-)")))?;
                s = compose(&su_pat, &s);
                cs = apply_constraints(&s, cs);
                cs.extend(apply_constraints(&s, cs_pat));

                for (name, t) in binds {
                    env2.insert(
                        name,
                        EnvEntry {
                            scheme: Scheme::mono(apply(&s, t)),
                            def_site: None,
                        },
                    );
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
                let subst_in = compose(&s, subst_env);
                let (s_e, cs_e, t_e) = infer_expr_in(cx, data_env, &subst_in, &env2, e)
                    .map_err(|e| e.with_context(format!("in do stmt {stmt_no}")))?;
                s = compose(&s_e, &s);
                cs = apply_constraints(&s, cs);
                cs.extend(apply_constraints(&s, cs_e));

                let io_r = cx.fresh();
                let su = unify_dbg(
                    apply(&s, t_e),
                    Ty::App {
                        head: Box::new(Ty::Con("IO".to_string())),
                        args: vec![io_r.clone()],
                    },
                    &format!("infer_expr_do:stmt {stmt_no}:expr_io"),
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
    let last_ty =
        last_ty.ok_or_else(|| Error::msg_with_span("do must end with expression", span))?;
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
        Type::String => {
            // `String` is represented as a surface-type token but is a stdlib type alias.
            // Record it as alias usage so unify failures can show def-site evidence.
            TL_NAME_HINTS.with(|h| {
                let hints = h.borrow();
                if let Some(qual) = hints
                    .type_alias
                    .get(&UnqualName("String".to_string()))
                    .cloned()
                {
                    TL_ALIAS_EVIDENCE.with(|slot| {
                        slot.borrow_mut()
                            .push((UnqualName("String".to_string()), qual));
                    });
                }
            });
            Ty::Con("String".to_string())
        }

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
                // Uppercase type names may be type aliases (e.g. `Text`, `String`).
                // We can't keep alias nodes in `Ty`, so record evidence here.
                TL_NAME_HINTS.with(|h| {
                    let hints = h.borrow();
                    if let Some(qual) = hints.type_alias.get(&UnqualName(name.clone())).cloned() {
                        TL_ALIAS_EVIDENCE.with(|slot| {
                            slot.borrow_mut().push((UnqualName(name.clone()), qual));
                        });
                    }
                });
                Ty::Con(name.clone())
            }
        }
    }
}

pub fn stdlib_root() -> PathBuf {
    stdlib_cache::stdlib_root().unwrap_or_else(|_| PathBuf::from("stdlib"))
}

pub fn try_stdlib_root() -> Result<PathBuf> {
    stdlib_cache::stdlib_root()
}

pub fn set_stdlib_dir_override(path: PathBuf) {
    stdlib_cache::set_stdlib_root_override(path);
}

/// Install the embedded stdlib into a user-writable location and return its path.
pub fn install_embedded_stdlib() -> Result<PathBuf> {
    stdlib_cache::install_embedded_stdlib()
}

/// Reinstall the embedded stdlib into a user-writable location and return its path.
pub fn reinstall_embedded_stdlib() -> Result<PathBuf> {
    stdlib_cache::reinstall_embedded_stdlib()
}

pub fn typecheck_file(entry: &Path) -> Result<TypedModule> {
    // Prelude is auto-imported unless the module has explicit imports.
    // Note: import-flattening is removed; `.ksif` artifacts provide imported schemes.
    let entry = std::fs::canonicalize(entry)?;

    let mut loader = ModuleLoader::new();
    loader.stack = vec![entry.clone()];

    // Load via ModuleLoader so stdlib cache + qualified-name desugaring stays consistent.
    let mut entry_mod = loader.load_ast(&entry)?;
    let entry_dir = module_search_root_from_entry(&entry, entry_mod.name.as_deref());

    // Do not inject implicit Prelude for stdlib files themselves.
    // Otherwise, opening e.g. `stdlib/Prelude/Functor.ks` causes an injected
    // `import Prelude`, and Prelude imports `Prelude.Functor`, creating a
    // spurious cyclic import.
    if !is_stdlib_path(&entry) {
        entry_mod = ensure_implicit_prelude_import(entry_mod);
        // Inject stdlib class declarations and their imports early, before loading .ksif schemes.
        // This ensures that imports needed by class default methods are available.
        inject_stdlib_class_decls(&mut entry_mod)?;
    }

    // Module-unit compilation: imports must be satisfied via `.ksif`.
    // Import-flattening is intentionally removed.
    // Clone entry_mod after injection so that the injected imports are included.
    let mut module = entry_mod.clone();

    let def_ctx = DefEvidenceCtx::from_loader(&loader);

    // Default: use `.ksif` for imports. (No opt-out; import-flattening is removed.)
    // This now includes imports injected by inject_stdlib_class_decls above.
    let imported = load_imported_ksif_schemes(&entry_mod, &entry_dir)?;
    inject_imported_ksif_forwarders(&mut module, &entry_mod, &imported, &entry_dir)?;
    WithDefEvidence::run(def_ctx, || {
        typecheck_with_stdlib_class_env_with_imported_with_entry_path(
            module,
            imported,
            Some(&entry),
        )
    })
}

/// Load all transitive imports for runtime linking.
/// Returns a map of module_name -> ast::Module for all imported modules.
/// Uses existing ModuleLoader cache to be cycle-safe.
pub fn load_transitive_imports_for_runtime(entry: &Path) -> Result<HashMap<String, ast::Module>> {
    if std::env::var("KSCR_DEBUG_RUNTIME_IMPORTS").is_ok() {
        eprintln!(
            "[RUNTIME] load_transitive_imports_for_runtime called for: {}",
            entry.display()
        );
    }
    let entry = std::fs::canonicalize(entry)?;

    let mut loader = ModuleLoader::new();
    loader.stack = vec![entry.clone()];

    // Load the entry module
    let mut entry_mod = loader.load_ast(&entry)?;
    let entry_dir = module_search_root_from_entry(&entry, entry_mod.name.as_deref());

    // Add implicit Prelude import if needed (same logic as typecheck_file)
    if !is_stdlib_path(&entry) {
        entry_mod = ensure_implicit_prelude_import(entry_mod);
    }

    // Inject stdlib class declarations and their imports (same logic as typecheck_file)
    // This ensures that imports needed by stdlib class default methods are included
    inject_stdlib_class_decls(&mut entry_mod)?;

    if std::env::var("KSCR_DEBUG_RUNTIME").ok().as_deref() == Some("1") {
        eprintln!("[KSCR_DEBUG_RUNTIME] Entry module imports after inject_stdlib_class_decls:");
        for it in &entry_mod.items {
            if let ast::Item::Import(id) = it {
                eprintln!("  - {}", id.module);
            }
        }
    }

    // Collect all transitive imports
    let mut result: HashMap<String, ast::Module> = HashMap::new();
    let mut to_visit: Vec<String> = Vec::new();

    // Start with entry module's imports
    for it in &entry_mod.items {
        let ast::Item::Import(id) = it else {
            continue;
        };
        to_visit.push(id.module.clone());
    }

    // BFS traversal to collect all transitive imports
    while let Some(module_name) = to_visit.pop() {
        if result.contains_key(&module_name) {
            continue;
        }

        // Resolve module path
        let module_path = resolve_module_path(&entry_dir, &module_name)?;

        // Load the module AST
        let module_ast = loader.load_ast(&module_path)?;

        // Add its imports to the queue
        for it in &module_ast.items {
            let ast::Item::Import(id) = it else {
                continue;
            };
            if !result.contains_key(&id.module) {
                to_visit.push(id.module.clone());
            }
        }

        result.insert(module_name, module_ast);
    }

    Ok(result)
}

pub(crate) fn resolve_module_path(entry_dir: &Path, module: &str) -> Result<PathBuf> {
    let rel = module.replace('.', "/");
    let local = entry_dir.join(format!("{}.ks", rel));
    let stdlib_root = stdlib_cache::stdlib_root()?;
    let stdlib = stdlib_root.join(format!("{}.ks", rel));

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

fn module_search_root_from_entry(entry: &Path, module_name: Option<&str>) -> PathBuf {
    let fallback_root = entry
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();

    let Some(module_name) = module_name else {
        return fallback_root;
    };

    let parts: Vec<&str> = module_name.split('.').collect();
    if parts.is_empty() || parts.iter().any(|part| part.is_empty()) {
        return fallback_root;
    }

    let mut candidate_root = fallback_root.clone();
    for _ in 1..parts.len() {
        let Some(parent) = candidate_root.parent() else {
            return fallback_root;
        };
        candidate_root = parent.to_path_buf();
    }

    let mut module_rel_path = PathBuf::new();
    for part in &parts {
        module_rel_path.push(part);
    }
    module_rel_path.set_extension("ks");

    if candidate_root.join(module_rel_path) == entry {
        candidate_root
    } else {
        fallback_root
    }
}

fn inject_imported_ksif_forwarders(
    module: &mut ast::Module,
    entry_mod: &ast::Module,
    imported: &HashMap<String, HashMap<String, Scheme>>,
    entry_dir: &Path,
) -> Result<()> {
    // Only inject *unqualified* forwarders (e.g. `x = A.x`).
    // Qualified names (`A.x` / `OM.x`) are resolved via imported `.ksif` schemes merged into
    // the inference environment; injecting extra qualified bindings can:
    // - break alias imports (`OM.x = A.x` where `A.x` is intentionally unavailable)
    // - introduce name-conflict masking that should be reported to the user.
    let mut defined: HashMap<String, String> = HashMap::new();
    for it in &module.items {
        let mut names = HashSet::new();
        item_defined_names(it, &mut names);
        for n in names {
            defined.insert(n.clone(), name_origin_hint(it, &n));
        }
    }

    let mut injected: Vec<ast::Item> = Vec::new();
    for it in &entry_mod.items {
        let ast::Item::Import(id) = it else {
            continue;
        };
        let Some(schemes) = imported.get(&id.module) else {
            continue;
        };

        let qual = id.as_name.as_deref().unwrap_or(&id.module);

        // Skip self-import forwarders (rare / would be self-recursive).
        // Note: `qual == id.module` is the normal case for `import Foo.Bar` (no alias),
        // and we *do* want to inject unqualified forwarders in that case.
        if module.name.as_deref() == Some(&id.module) {
            continue;
        }

        // Apply import spec filter to determine which names to forward
        let all_names: HashSet<String> = schemes.keys().cloned().collect();
        let filtered_names = match &id.import_spec {
            None => all_names, // No filter, import everything
            Some(ast::ImportSpec::Only(specs)) => {
                expand_import_spec_with_ctors(specs, &id.module, entry_dir)
            }
            Some(ast::ImportSpec::Hiding(specs)) => {
                let hidden = expand_import_spec_with_ctors(specs, &id.module, entry_dir);
                all_names
                    .into_iter()
                    .filter(|n| !hidden.contains(n))
                    .collect()
            }
        };

        for name in &filtered_names {
            // Only forward names that exist in the imported module's schemes
            if !schemes.contains_key(name) {
                continue;
            }

            let qual_name = format!("{qual}.{name}");

            // For unqualified imports, inject `x = <Qual>.x`.
            // If multiple imports try to provide the same unqualified name, first import wins.
            if !id.qualified {
                let origin = format!("import {qual}");
                if let Some(_prev) = defined.get(name) {
                    // Local definitions win silently.
                    // For import-vs-import conflicts: first import wins, skip silently.
                    continue;
                }

                defined.insert(name.clone(), origin);
                injected.push(ast::Item::Binding(ast::Binding {
                    doc: None,
                    pat: ast::Pattern {
                        kind: ast::PatternKind::Var(name.clone()),
                        span: ast::dummy_span(),
                    },
                    expr: ast::Expr {
                        kind: ast::ExprKind::Var(qual_name),
                        span: ast::dummy_span(),
                    },
                    span: ast::dummy_span(),
                }));
            }
        }
    }

    if injected.is_empty() {
        return Ok(());
    }

    let mut merged = Vec::new();
    merged.append(&mut injected);
    merged.append(&mut module.items);
    module.items = merged;
    Ok(())
}

/// Compute SHA256 hash of a file's contents and return as hex string.
pub(crate) fn compute_file_sha256(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let result = hasher.finalize();
    Ok(format!("{:x}", result))
}

/// Validate that all dependencies in a KSIF have matching hashes.
/// Returns Ok(true) if all hashes match, Ok(false) if any mismatch, Err on I/O or decode errors.
pub(crate) fn validate_ksif_dependencies(
    ksif_path: &Path,
    module_dir: &Path,
    default_artifact_dir: &Path,
) -> Result<bool> {
    // Load the KSIF to get its dependency manifest
    let ksif_bytes = std::fs::read(ksif_path)?;
    let ksif = crate::kir1::decode_ksif_module(&ksif_bytes).map_err(|e| {
        Error::msg(format!(
            "failed to decode ksif {}: {:?}",
            ksif_path.display(),
            e
        ))
    })?;

    // Check each dependency's hash
    for (dep_name, expected_hash) in &ksif.dependencies {
        // Find the dependency's .ksif file (same logic as load_imported_ksif_schemes_internal)
        let stdlib_root = stdlib_cache::stdlib_root()?;
        let stdlib_artifact_dir = stdlib_root
            .parent()
            .unwrap_or(stdlib_root.as_path())
            .join("target")
            .join("ksif");

        let candidates = [
            // Prefer stable cache locations over source-adjacent artifacts.
            default_artifact_dir.join(format!("{}.ksif", dep_name)),
            stdlib_artifact_dir.join(format!("{}.ksif", dep_name)),
            // Backwards-compat (older layouts)
            module_dir.join(format!("{}.ksif", dep_name.replace('.', "/"))),
            module_dir.join(format!("{}.ksif", dep_name)),
            module_dir.join(format!("ksif_{}.ksif", dep_name)),
        ];

        let mut dep_ksif_path: Option<PathBuf> = None;
        for p in &candidates {
            if p.exists() {
                dep_ksif_path = Some(p.clone());
                break;
            }
        }

        let Some(dep_path) = dep_ksif_path else {
            // Dependency .ksif not found - consider invalid
            return Ok(false);
        };

        // Compute current hash and compare
        let actual_hash = compute_file_sha256(&dep_path)?;
        if &actual_hash != expected_hash {
            // Hash mismatch - rebuild needed
            return Ok(false);
        }
    }

    // All hashes match
    Ok(true)
}

/// Ensure KSIF artifact exists for the given module by typechecking it if needed.
/// Recursively ensures KSIF for transitive imports.
///
/// NOTE: we must detect import cycles here; otherwise `typecheck_file` would silently accept
/// cyclic imports due to the old `visited` short-circuit.
fn ensure_ksif_for_module(
    module_name: &str,
    entry_dir: &Path,
    visiting: &mut HashSet<String>,
    done: &mut HashSet<String>,
) -> Result<()> {
    use std::path::PathBuf;

    if done.contains(module_name) {
        return Ok(());
    }
    if visiting.contains(module_name) {
        return Err(Error::msg(format!("cyclic imports: {module_name}")));
    }
    visiting.insert(module_name.to_string());

    // Resolve module .ks file
    let rel = module_name.replace('.', "/");
    let local = entry_dir.join(format!("{}.ks", rel));
    let stdlib_root = stdlib_cache::stdlib_root()?;
    let stdlib = stdlib_root.join(format!("{}.ks", rel));

    let module_path = std::fs::canonicalize(&local)
        .or_else(|_| std::fs::canonicalize(&stdlib))
        .map_err(|_| {
            Error::msg(format!(
                "cannot find module file for import {module_name} (tried: {}, {})",
                local.display(),
                stdlib.display()
            ))
        })?;

    let module_dir = module_path.parent().unwrap_or_else(|| Path::new("."));

    // Artifact dirs:
    // - stdlib artifacts go to a stable user-writable cache: <stdlib_root>/../target/ksif
    // - project artifacts (CLI `compile` default) go to: <entry_dir>/target/ksif
    let stdlib_artifact_dir = stdlib_root
        .parent()
        .unwrap_or(stdlib_root.as_path())
        .join("target")
        .join("ksif");
    let project_artifact_dir = entry_dir.join("target").join("ksif");
    let default_artifact_dir = if module_path.starts_with(&stdlib_root) {
        stdlib_artifact_dir
    } else {
        project_artifact_dir
    };
    let ksif_path = default_artifact_dir.join(format!("{}.ksif", module_name));

    let policy = get_ksif_rebuild_policy();

    // Check if KSIF already exists and validate dependency hashes.
    // IMPORTANT: this function is called in a shared DFS with `visiting`/`done`.
    // If we return early here, we must still mark the module as done; otherwise a later
    // import of the same module will look like a cycle.
    let needs_rebuild = if ksif_path.exists() {
        // If the source file is newer than the cached KSIF, rebuild.
        // This keeps local stdlib/project edits visible without requiring --ksif-rebuild.
        let src_is_newer = (|| {
            let src_m = std::fs::metadata(&module_path).ok()?.modified().ok()?;
            let ksif_m = std::fs::metadata(&ksif_path).ok()?.modified().ok()?;
            Some(src_m > ksif_m)
        })()
        .unwrap_or(false);

        if src_is_newer {
            true
        } else if policy.force_rebuild {
            // Force rebuild requested
            true
        } else if policy.suppress_recursive_rebuild {
            // When suppress_recursive_rebuild is true, trust existing ksif without validating
            // dependency hashes. This prevents cascading rebuilds when dependencies change.
            false
        } else {
            // Check dependency hashes
            match validate_ksif_dependencies(&ksif_path, module_dir, &default_artifact_dir) {
                Ok(valid) => !valid, // Rebuild if invalid
                Err(_) => true,      // Rebuild on validation error
            }
        }
    } else {
        // KSIF doesn't exist, need to build
        true
    };

    if !needs_rebuild {
        visiting.remove(module_name);
        done.insert(module_name.to_string());
        return Ok(());
    }

    // Load and parse module
    let src = std::fs::read_to_string(&module_path)?;
    let mut module_ast = parser::parse_module(&src)?;
    desugar_module_qualified_names(&mut module_ast)?;

    // Ensure module header matches the requested module name.
    if module_ast.name.as_deref() != Some(module_name) {
        return Err(Error::msg(format!(
            "module name mismatch: import {} but file declares module {}",
            module_name,
            module_ast.name.as_deref().unwrap_or("<missing>")
        )));
    }

    // Populate def_module for ClassDecls
    if let Some(name) = &module_ast.name {
        for it in &mut module_ast.items {
            if let ast::Item::ClassDecl(c) = it {
                c.def_module = Some(name.clone());
            }
        }
    }

    // Match file-based typechecking: auto-import Prelude for non-stdlib modules.
    // This keeps common operators (e.g. (==)) available when compiling dependencies into `.ksif`.
    if !is_stdlib_path(&module_path) {
        module_ast = ensure_implicit_prelude_import(module_ast);
    }

    // Recursively ensure KSIF for imports (unless suppressed)
    if !policy.suppress_recursive_rebuild {
        for it in &module_ast.items {
            let ast::Item::Import(id) = it else {
                continue;
            };
            ensure_ksif_for_module(&id.module, entry_dir, visiting, done)?;
        }
    } else {
        // When recursive rebuild is suppressed, we do not build dependencies.
        // But we must NOT silently accept missing dependency `.ksif` files, otherwise
        // we can write an incomplete dependency manifest.
        for it in &module_ast.items {
            let ast::Item::Import(id) = it else {
                continue;
            };

            let candidates = [
                default_artifact_dir.join(format!("{}.ksif", id.module)),
                // Backwards-compat
                module_dir.join(format!("{}.ksif", id.module.replace('.', "/"))),
                module_dir.join(format!("{}.ksif", id.module)),
                module_dir.join(format!("ksif_{}.ksif", id.module)),
            ];

            let dep_ksif_exists = candidates.iter().any(|p| p.exists());
            if !dep_ksif_exists {
                let dep_module_path = resolve_module_path(entry_dir, &id.module)?;
                if !is_stdlib_path(&dep_module_path) {
                    return Err(Error::msg(format!(
                        "missing dependency ksif for import {} (expected {}.ksif); build it first or run without --no-ksif-rebuild-deps",
                        id.module, id.module
                    )));
                }
            }

            // Mark as done to avoid cycle detection issues.
            done.insert(id.module.clone());
        }
    }

    // Compute dependency hashes after ensuring all KSIFs exist
    let mut dep_hashes: Vec<(String, String)> = Vec::new();
    for it in &module_ast.items {
        let ast::Item::Import(id) = it else {
            continue;
        };
        // Find the .ksif file for this dependency (same logic as load_imported_ksif_schemes_internal)
        let candidates = [
            default_artifact_dir.join(format!("{}.ksif", id.module)),
            // Backwards-compat
            module_dir.join(format!("{}.ksif", id.module.replace('.', "/"))),
            module_dir.join(format!("{}.ksif", id.module)),
            module_dir.join(format!("ksif_{}.ksif", id.module)),
        ];

        let mut dep_ksif_path: Option<PathBuf> = None;
        for p in &candidates {
            if p.exists() {
                dep_ksif_path = Some(p.clone());
                break;
            }
        }

        if let Some(dep_path) = dep_ksif_path {
            let hash = compute_file_sha256(&dep_path)?;
            dep_hashes.push((id.module.clone(), hash));
        } else if policy.suppress_recursive_rebuild {
            let dep_module_path = resolve_module_path(entry_dir, &id.module)?;
            if !is_stdlib_path(&dep_module_path) {
                return Err(Error::msg(format!(
                    "missing dependency ksif for import {} (expected {}.ksif); build it first or run without --no-ksif-rebuild-deps",
                    id.module, id.module
                )));
            }
        }
        // If no .ksif found, skip (stdlib modules might not have .ksif)
    }

    // Generate KSIF by typechecking the module (stdlib included).
    // This avoids placeholder schemes that break `.ksif`-only compilation.
    let imported = load_imported_ksif_schemes_internal(&module_ast, module_dir, &policy)?;

    // Inject forwarders for imported names
    let mut module_to_typecheck = ast::Module {
        name: module_ast.name.clone(),
        export_specs: module_ast.export_specs.clone(),
        items: module_ast.items.clone(),
    };
    inject_imported_ksif_forwarders(&mut module_to_typecheck, &module_ast, &imported, module_dir)?;

    let tm = typecheck_with_stdlib_class_env_with_imported_with_entry_path(
        module_to_typecheck,
        imported,
        Some(&module_path),
    )?;

    // Extract exported schemes
    let mut values: Vec<(String, Scheme)> =
        crate::cli_impl::filter_inferred_by_exports(&tm.module, tm.inferred.clone());

    // `.ksif` must also carry exported data constructors (they are values at term-level).
    // Without this, `import qualified A as M; M.Just` cannot typecheck in `.ksif` mode.
    let exports = module_exported_names(&tm.module)?;
    let mut seen: HashSet<String> = values.iter().map(|(n, _)| n.clone()).collect();

    let mut cx = InferCtx::default();
    let mut ctor_env = TypeEnv::new();
    add_data_ctors_into_env(&mut cx, &tm.module, Some(&module_path), &mut ctor_env);

    // Also add constructors from type alias re-exports (e.g., `type Maybe a = Prelude.Maybe a; export Maybe(..)`)
    // These constructors need to be in the KSIF so qualified imports work.
    for it in &tm.module.items {
        if let ast::Item::TypeAlias(ta) = it {
            if let Some(alias_ctors) = extract_aliased_type_ctors(&tm.module, ta) {
                if std::env::var("KSCR_DEBUG_IMPORTS").ok().is_some() {
                    eprintln!(
                        "[KSCR_DEBUG_IMPORTS] Type alias {} re-exports constructors: {:?}",
                        ta.name, alias_ctors
                    );
                }
                // Extract the qualified type name that the alias refers to
                // E.g., from `type Maybe a = Prelude.Maybe a`, extract "Prelude.Maybe"
                let target_ty_name = match &ta.ty {
                    ast::Type::Var(name) => Some(name.clone()),
                    ast::Type::App { head, .. } => match &**head {
                        ast::Type::Var(name) => Some(name.clone()),
                        _ => None,
                    },
                    _ => None,
                };

                if let Some(qual_type_name) = target_ty_name {
                    if std::env::var("KSCR_DEBUG_IMPORTS").ok().is_some() {
                        eprintln!(
                            "[KSCR_DEBUG_IMPORTS] Qualified type name: {}",
                            qual_type_name
                        );
                    }
                    // Look up each constructor with the qualified module prefix
                    // E.g., "Prelude.Just", "Prelude.Nothing"
                    for ctor_name in &alias_ctors {
                        let qual_ctor_name =
                            if let Some((module_part, _)) = qual_type_name.rsplit_once('.') {
                                format!("{}.{}", module_part, ctor_name)
                            } else {
                                ctor_name.clone()
                            };

                        if std::env::var("KSCR_DEBUG_IMPORTS").ok().is_some() {
                            eprintln!(
                                "[KSCR_DEBUG_IMPORTS] Looking for constructor: {} (qual: {})",
                                ctor_name, qual_ctor_name
                            );
                            eprintln!(
                                "[KSCR_DEBUG_IMPORTS] Found in tm.inferred: {}",
                                tm.inferred.contains_key(&qual_ctor_name)
                            );
                            if !tm.inferred.contains_key(&qual_ctor_name) {
                                eprintln!(
                                    "[KSCR_DEBUG_IMPORTS] Available keys matching {}: {:?}",
                                    ctor_name,
                                    tm.inferred
                                        .keys()
                                        .filter(|k| k.contains(ctor_name))
                                        .collect::<Vec<_>>()
                                );
                            }
                        }

                        // Try to find this constructor in the inferred schemes
                        // Try both qualified and unqualified names
                        let scheme = tm
                            .inferred
                            .get(&qual_ctor_name)
                            .or_else(|| tm.inferred.get(ctor_name));

                        if let Some(scheme) = scheme {
                            if std::env::var("KSCR_DEBUG_IMPORTS").ok().is_some() {
                                eprintln!("[KSCR_DEBUG_IMPORTS] Adding {} to ctor_env", ctor_name);
                            }
                            ctor_env.insert(
                                ctor_name.clone(),
                                EnvEntry {
                                    scheme: scheme.clone(),
                                    def_site: Some(DefSite {
                                        path: module_path.clone(),
                                        span: ta.span,
                                    }),
                                },
                            );
                        }
                    }
                }
            }
        }
    }

    for (name, entry) in ctor_env {
        if matches!(exports.entries.get(&name), Some(SymbolKind::Ctor)) && seen.insert(name.clone())
        {
            values.push((name, entry.scheme));
        }
    }

    let ksif = crate::kir1::KsifModule {
        module_name: module_name.to_string(),
        values,
        dependencies: dep_hashes,
    };
    let bytes = crate::kir1::encode_ksif_module(&ksif);

    fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        use std::time::{SystemTime, UNIX_EPOCH};

        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let file_name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "ksif.tmp".to_string());

        let pid = std::process::id();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);

        let tmp = parent.join(format!(".{file_name}.tmp.{pid}.{nanos}"));
        std::fs::write(&tmp, bytes)?;

        match std::fs::rename(&tmp, path) {
            Ok(()) => Ok(()),
            Err(e) => {
                // Windows can't rename over an existing file; treat "already written"
                // as success and clean up the temp file.
                if path.exists() {
                    let _ = std::fs::remove_file(&tmp);
                    Ok(())
                } else {
                    Err(e)
                }
            }
        }
    }

    // Write KSIF artifact (stable location + atomic write to avoid partial reads).
    if let Some(parent) = ksif_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_atomic(&ksif_path, &bytes)?;

    visiting.remove(module_name);
    done.insert(module_name.to_string());

    Ok(())
}

/// Internal helper that loads KSIF schemes from disk without ensuring they exist first.
/// Used by ensure_ksif_for_module to avoid infinite recursion.
fn load_imported_ksif_schemes_internal(
    module: &ast::Module,
    entry_dir: &Path,
    policy: &KsifRebuildPolicy,
) -> Result<HashMap<String, HashMap<String, Scheme>>> {
    use std::path::PathBuf;

    let default_artifact_dir = entry_dir.join("target").join("ksif");
    let stdlib_root = stdlib_cache::stdlib_root()?;
    let stdlib_artifact_dir = stdlib_root
        .parent()
        .unwrap_or(stdlib_root.as_path())
        .join("target")
        .join("ksif");

    let mut imported: HashMap<String, HashMap<String, Scheme>> = HashMap::new();
    'import_loop: for it in &module.items {
        let ast::Item::Import(id) = it else {
            continue;
        };

        let resolved_module_path = resolve_module_path(entry_dir, &id.module).ok();
        let is_stdlib_import = resolved_module_path
            .as_ref()
            .is_some_and(|p| p.starts_with(&stdlib_root));

        let candidates = if is_stdlib_import {
            [
                // For stdlib modules, prefer the stdlib cache first.
                stdlib_artifact_dir.join(format!("{}.ksif", id.module)),
                default_artifact_dir.join(format!("{}.ksif", id.module)),
                // Backwards-compat (older layouts)
                entry_dir.join(format!("{}.ksif", id.module.replace('.', "/"))),
                entry_dir.join(format!("{}.ksif", id.module)),
                entry_dir.join(format!("ksif_{}.ksif", id.module)),
            ]
        } else {
            [
                // Prefer stable cache locations first to avoid source-adjacent KSIF races.
                default_artifact_dir.join(format!("{}.ksif", id.module)),
                stdlib_artifact_dir.join(format!("{}.ksif", id.module)),
                // Backwards-compat (older layouts)
                entry_dir.join(format!("{}.ksif", id.module.replace('.', "/"))),
                entry_dir.join(format!("{}.ksif", id.module)),
                entry_dir.join(format!("ksif_{}.ksif", id.module)),
            ]
        };

        let read_first_existing = || -> Option<(PathBuf, Vec<u8>)> {
            for p in &candidates {
                if let Ok(b) = std::fs::read(p) {
                    return Some((p.clone(), b));
                }
            }
            None
        };

        // Stale KSIFs can happen in parallel test runs when another worker
        // updates transitive dependencies. Best effort: rebuild once and retry.
        let mut auto_rebuild_attempted = false;
        let (ksif_path, bytes) = loop {
            let Some((ksif_path, bytes)) = read_first_existing() else {
                // For stdlib modules, missing KSIF is tolerated (rely on stdlib cache/injection)
                continue 'import_loop;
            };

            // Safety: if a cached KSIF's dependency hashes don't match, rebuild once and retry.
            // However, skip this validation when suppress_recursive_rebuild is true.
            if !policy.suppress_recursive_rebuild {
                // Validate dependency hashes using the imported module's context.
                // Using `entry_dir` here can accidentally pick up unrelated project-local KSIFs
                // (e.g. /tmp/target/ksif/Prelude.*.ksif) and report false staleness.
                let (validate_module_dir, validate_default_artifact_dir) =
                    match resolved_module_path
                        .clone()
                        .or_else(|| resolve_module_path(entry_dir, &id.module).ok())
                    {
                        Some(module_path) => {
                            let module_dir =
                                module_path.parent().unwrap_or(entry_dir).to_path_buf();
                            let stdlib_root = stdlib_cache::stdlib_root()?;
                            let default_artifact_dir = if module_path.starts_with(&stdlib_root) {
                                stdlib_artifact_dir.clone()
                            } else {
                                default_artifact_dir.clone()
                            };
                            (module_dir, default_artifact_dir)
                        }
                        None => (entry_dir.to_path_buf(), default_artifact_dir.clone()),
                    };

                match validate_ksif_dependencies(
                    &ksif_path,
                    &validate_module_dir,
                    &validate_default_artifact_dir,
                ) {
                    Ok(true) => {}
                    Ok(false) => {
                        if !auto_rebuild_attempted {
                            auto_rebuild_attempted = true;
                            let mut visiting = HashSet::new();
                            let mut done = HashSet::new();
                            ensure_ksif_for_module(
                                &id.module,
                                entry_dir,
                                &mut visiting,
                                &mut done,
                            )?;
                            continue;
                        }
                        return Err(Error::msg(format!(
                            "stale ksif {} (dependencies changed); auto-rebuild attempted; re-run with --ksif-rebuild",
                            ksif_path.display()
                        )));
                    }
                    Err(e) => {
                        return Err(Error::msg(format!(
                            "failed to validate ksif dependencies for {}: {e}",
                            ksif_path.display()
                        )));
                    }
                }
            }

            break (ksif_path, bytes);
        };

        let ksif = crate::kir1::decode_ksif_module(&bytes).map_err(|e| {
            Error::msg(format!(
                "failed to decode ksif {}: {e:?}",
                ksif_path.display()
            ))
        })?;
        let mut m = HashMap::new();
        for (name, scheme) in ksif.values {
            m.insert(name, scheme);
        }

        imported.insert(id.module.clone(), m);
    }

    Ok(imported)
}

fn load_imported_ksif_schemes(
    module: &ast::Module,
    entry_dir: &Path,
) -> Result<HashMap<String, HashMap<String, Scheme>>> {
    // Ensure KSIF artifacts exist for all imports (including stdlib)
    let mut visiting = HashSet::new();
    let mut done = HashSet::new();
    for it in &module.items {
        let ast::Item::Import(id) = it else {
            continue;
        };
        ensure_ksif_for_module(&id.module, entry_dir, &mut visiting, &mut done)?;
    }

    // Load KSIF schemes
    let policy = get_ksif_rebuild_policy();
    load_imported_ksif_schemes_internal(module, entry_dir, &policy)
}

pub(crate) fn is_stdlib_path(path: &Path) -> bool {
    let Ok(path) = std::fs::canonicalize(path) else {
        return false;
    };
    path.starts_with(stdlib_root())
}

#[allow(dead_code)]
fn load_module_with_imports_ast_with_loader(
    loader: &mut ModuleLoader,
    entry: &Path,
    entry_dir: &Path,
    entry_mod: &ast::Module,
) -> Result<ast::Module> {
    let mut items = Vec::new();
    let mut defined: HashMap<String, String> = HashMap::new();

    let mut deps = Vec::new();
    loader.collect_imports(entry_mod, entry_dir, &mut deps)?;

    let debug_imports = std::env::var("KSCR_DEBUG_IMPORTS").ok().is_some();
    if debug_imports {
        eprintln!("[KSCR_DEBUG_IMPORTS] entry: {}", entry.display());
    }

    for it in deps {
        push_item_checked(&mut items, &mut defined, it)?;
    }

    for it in entry_mod.items.clone() {
        if matches!(it, ast::Item::Import(_)) {
            continue;
        }
        push_item_checked(&mut items, &mut defined, it)?;
    }

    Ok(ast::Module {
        name: entry_mod.name.clone(),
        export_specs: None,
        items,
    })
}

fn ensure_implicit_prelude_import(mut module: ast::Module) -> ast::Module {
    // Prelude itself should not implicitly import Prelude.
    if module.name.as_deref() == Some("Prelude") {
        return module;
    }

    // Check if Prelude is already imported (qualified or unqualified)
    let has_prelude_import = module
        .items
        .iter()
        .any(|it| matches!(it, ast::Item::Import(id) if id.module == "Prelude"));
    if has_prelude_import {
        return module;
    }

    // If the module already has imports, keep Prelude operators/values available but avoid
    // leaking Prelude data constructors that can interfere with ctor export restrictions.
    // (Users can explicitly `import Prelude` to opt into Prelude ctors unqualified.)
    let has_any_import = module
        .items
        .iter()
        .any(|it| matches!(it, ast::Item::Import(_)));

    let import_spec = if has_any_import {
        Some(ast::ImportSpec::Hiding(vec![
            ast::ExportSpec::Type {
                name: "Maybe".to_string(),
                ctors: ast::ExportCtors::All,
            },
            ast::ExportSpec::Type {
                name: "Either".to_string(),
                ctors: ast::ExportCtors::All,
            },
        ]))
    } else {
        None
    };

    let prelude_import = ast::Item::Import(ast::ImportDecl {
        module: "Prelude".to_string(),
        qualified: false,
        as_name: None,
        import_spec,
    });
    module.items.insert(0, prelude_import);
    module
}

#[allow(dead_code)]
fn typecheck_with_stdlib_class_env_with_entry_path(
    mut module: ast::Module,
    entry_path: Option<&Path>,
) -> Result<TypedModule> {
    let timing = std::env::var("KSCR_DEBUG_TIMING").ok().as_deref() == Some("1");

    let t0 = std::time::Instant::now();
    let stdlib_class_env = load_stdlib_class_env()?;
    if timing {
        eprintln!(
            "[KSCR_DEBUG_TIMING] load_stdlib_class_env: {:.3}s",
            t0.elapsed().as_secs_f64()
        );
    }

    let t0 = std::time::Instant::now();
    // Only inject stdlib class declarations for non-stdlib modules
    if entry_path.is_none_or(|p| !is_stdlib_path(p)) {
        inject_stdlib_class_decls(&mut module)?;
    }
    if timing {
        eprintln!(
            "[KSCR_DEBUG_TIMING] inject_stdlib_class_decls: {:.3}s",
            t0.elapsed().as_secs_f64()
        );
    }

    let t0 = std::time::Instant::now();
    // Only inject instance dict forwarders for non-stdlib modules
    if entry_path.is_none_or(|p| !is_stdlib_path(p)) {
        inject_stdlib_instance_dict_forwarders(&mut module)?;
    }
    if timing {
        eprintln!(
            "[KSCR_DEBUG_TIMING] inject_stdlib_instance_dict_forwarders: {:.3}s",
            t0.elapsed().as_secs_f64()
        );
    }

    let t0 = std::time::Instant::now();
    let out =
        typecheck_internal_core_with_entry_path(module, Some(&stdlib_class_env), None, entry_path);
    if timing {
        eprintln!(
            "[KSCR_DEBUG_TIMING] typecheck_internal: {:.3}s",
            t0.elapsed().as_secs_f64()
        );
    }
    out
}

fn typecheck_with_stdlib_class_env_with_imported_with_entry_path(
    mut module: ast::Module,
    imported: HashMap<String, HashMap<String, Scheme>>,
    entry_path: Option<&Path>,
) -> Result<TypedModule> {
    let timing = std::env::var("KSCR_DEBUG_TIMING").ok().as_deref() == Some("1");

    // Imported `.ksif` schemes will be merged into the inference env directly,
    // so we don't need to inject placeholder bindings.

    let t0 = std::time::Instant::now();
    let stdlib_class_env = load_stdlib_class_env()?;
    if timing {
        eprintln!(
            "[KSCR_DEBUG_TIMING] load_stdlib_class_env: {:.3}s",
            t0.elapsed().as_secs_f64()
        );
    }

    let t0 = std::time::Instant::now();
    // Inject stdlib class decls for user code and stdlib modules alike.
    // Stdlib modules contain instances (e.g. Prelude.Rational) that need their imported
    // class decls present during instance desugaring.
    let should_inject = true;
    if should_inject {
        inject_stdlib_class_decls(&mut module)?;
    }
    if timing {
        eprintln!(
            "[KSCR_DEBUG_TIMING] inject_stdlib_class_decls: {:.3}s",
            t0.elapsed().as_secs_f64()
        );
    }

    let t0 = std::time::Instant::now();
    // Only inject instance dict forwarders for non-stdlib modules
    if should_inject {
        inject_stdlib_instance_dict_forwarders(&mut module)?;
    }
    if timing {
        eprintln!(
            "[KSCR_DEBUG_TIMING] inject_stdlib_instance_dict_forwarders: {:.3}s",
            t0.elapsed().as_secs_f64()
        );
    }

    let t0 = std::time::Instant::now();
    let out = typecheck_internal_core_with_entry_path(
        module,
        Some(&stdlib_class_env),
        Some(imported),
        entry_path,
    );
    if timing {
        eprintln!(
            "[KSCR_DEBUG_TIMING] typecheck_internal: {:.3}s",
            t0.elapsed().as_secs_f64()
        );
    }
    out
}

fn inject_stdlib_instance_dict_forwarders(module: &mut ast::Module) -> Result<()> {
    // Collect all qualified stdlib dict bindings present in the module.
    let mut exports_map: HashMap<String, String> = HashMap::new();

    if std::env::var("KSCR_DEBUG_DICT").is_ok() {
        eprintln!(
            "[DICT] inject_stdlib_instance_dict_forwarders: checking {} import items",
            import_items(module).len()
        );
    }

    for it in import_items(module) {
        let ast::Item::Binding(b) = it else {
            continue;
        };
        let ast::PatternKind::Var(n) = b.pat.kind else {
            continue;
        };

        if std::env::var("KSCR_DEBUG_DICT").is_ok() && n.contains("__dict_") {
            eprintln!("[DICT]   Found binding: {}", n);
        }

        // Note: stdlib-provided dictionaries appear as qualified value bindings like
        // `Prelude.__dict_Monad_IO` or `Prelude.Num.__dict_Num_Integer`.
        // Later passes refer to the *unqualified* name like `__dict_Monad_IO` or `__dict_Num_Integer`,
        // so we need forwarders.
        if n.starts_with("Prelude.") {
            // Extract the unqualified dict name (last component that starts with __dict_)
            if let Some(unqualified) = n.split('.').next_back() {
                if unqualified.starts_with("__dict_") {
                    exports_map.insert(unqualified.to_string(), n.clone());
                }
            }
        }
    }

    if exports_map.is_empty() {
        return Ok(());
    }

    if std::env::var("KSCR_DEBUG_DICT").is_ok() {
        eprintln!("[DICT] Creating {} dict forwarders:", exports_map.len());
        for (unqualified, qualified) in &exports_map {
            eprintln!("[DICT]   {} = {}", unqualified, qualified);
        }
    }

    // Build unqualified forwarders like `__dict_Num_Integer = Prelude.Num.__dict_Num_Integer`.
    let mut forwarders: Vec<ast::Item> = Vec::new();
    for (unqualified, qualified) in exports_map {
        let span = ast::Span { start: 0, end: 0 };
        let forwarded = ast::Binding {
            doc: None,
            pat: ast::Pattern::new(span, ast::PatternKind::Var(unqualified)),
            expr: ast::Expr::new(span, ast::ExprKind::Var(qualified)),
            span,
        };
        forwarders.push(ast::Item::Binding(forwarded));
    }

    if forwarders.is_empty() {
        return Ok(());
    }

    let mut merged = Vec::new();
    merged.extend(forwarders);
    merged.append(&mut module.items);
    module.items = merged;
    Ok(())
}

fn inject_stdlib_instance_dict_forwarders_post_typecheck(module: &mut ast::Module) -> Result<()> {
    // Similar to inject_stdlib_instance_dict_forwarders, but runs after typecheck/desugaring
    // when instance dictionaries have been created.
    // Collects all qualified dict bindings (e.g., `Prelude.Num.__dict_Num_Integer`)
    // and creates unqualified forwarders (e.g., `__dict_Num_Integer = Prelude.Num.__dict_Num_Integer`).

    let mut exports_map: HashMap<String, String> = HashMap::new();

    if std::env::var("KSCR_DEBUG_DICT").is_ok() {
        eprintln!(
            "[DICT] inject_stdlib_instance_dict_forwarders_post_typecheck: checking {} items",
            module.items.len()
        );
        let mut dict_count = 0;
        for it in &module.items {
            if let ast::Item::Binding(b) = it {
                if let ast::PatternKind::Var(n) = &b.pat.kind {
                    if n.contains("__dict_") {
                        dict_count += 1;
                        if dict_count < 30 {
                            eprintln!("[DICT]   Item: {}", n);
                        }
                    }
                }
            }
        }
        eprintln!("[DICT]   Total dict-like bindings: {}", dict_count);
    }

    for it in &module.items {
        let ast::Item::Binding(b) = it else {
            continue;
        };
        let ast::PatternKind::Var(n) = &b.pat.kind else {
            continue;
        };

        if std::env::var("KSCR_DEBUG_DICT").is_ok() && n.contains("__dict_") {
            eprintln!("[DICT]   Found binding: {}", n);
        }

        // Look for qualified dict bindings from Prelude
        if n.starts_with("Prelude.")
            || n.starts_with("Prelude.Num.")
            || n.starts_with("Prelude.Ring.")
        {
            if let Some(unqualified) = n.split('.').next_back() {
                if unqualified.starts_with("__dict_") {
                    exports_map.insert(unqualified.to_string(), n.clone());
                }
            }
        }
    }

    if exports_map.is_empty() {
        if std::env::var("KSCR_DEBUG_DICT").is_ok() {
            eprintln!("[DICT] No forwarders needed (post-typecheck)");
        }
        return Ok(());
    }

    if std::env::var("KSCR_DEBUG_DICT").is_ok() {
        eprintln!(
            "[DICT] Creating {} dict forwarders (post-typecheck):",
            exports_map.len()
        );
        for (unqualified, qualified) in &exports_map {
            eprintln!("[DICT]   {} = {}", unqualified, qualified);
        }
    }

    let mut forwarders: Vec<ast::Item> = Vec::new();
    for (unqualified, qualified) in exports_map {
        let span = ast::Span { start: 0, end: 0 };
        let forwarded = ast::Binding {
            doc: None,
            pat: ast::Pattern::new(span, ast::PatternKind::Var(unqualified)),
            expr: ast::Expr::new(span, ast::ExprKind::Var(qualified)),
            span,
        };
        forwarders.push(ast::Item::Binding(forwarded));
    }

    let mut merged = Vec::new();
    merged.extend(forwarders);
    merged.append(&mut module.items);
    module.items = merged;
    Ok(())
}

fn inject_stdlib_class_decls(module: &mut ast::Module) -> Result<()> {
    let mut seen_classes: HashSet<String> = HashSet::new();

    for it in &module.items {
        if let ast::Item::ClassDecl(c) = it {
            seen_classes.insert(c.name.clone());
        }
    }

    let mut injected: Vec<ast::Item> = Vec::new();
    for it in load_stdlib_class_decl_items()? {
        if let ast::Item::ClassDecl(c) = &it {
            if seen_classes.insert(c.name.clone()) {
                injected.push(it);
            }
        }
    }

    if injected.is_empty() {
        return Ok(());
    }

    let mut merged = Vec::new();
    merged.append(&mut injected);
    merged.append(&mut module.items);
    module.items = merged;

    Ok(())
}

pub fn typecheck(mut module: ast::Module) -> Result<TypedModule> {
    // AST-only typechecking is widely used by unit tests. It still needs stdlib class
    // information (for method/value fallback like `show`).
    // NOTE: AST-only `typecheck` does not resolve imports. Keep it import-free and require
    // `typecheck_file` when imports are present.
    let stdlib_class_env = load_stdlib_class_env()?;
    // For unit tests and REPL-like AST-only callers, behave like a tiny file-based compilation
    // rooted at the project directory:
    // - bring Prelude values into scope
    // - load exported schemes from cached `.ksif`
    // - inject imported instance dictionaries so method calls can be dict-passed
    module = ensure_implicit_prelude_import(module);

    let entry_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let imported = load_imported_ksif_schemes(&module, &entry_dir)?;

    // Synthetic entry path so we can resolve stdlib modules and inject imported instances.
    let dummy_entry_path = entry_dir.join("__ast_typecheck__.ks");
    typecheck_internal_core_with_entry_path(
        module,
        Some(&stdlib_class_env),
        Some(imported),
        Some(&dummy_entry_path),
    )
}

#[allow(dead_code)]
fn typecheck_internal(
    module: ast::Module,
    stdlib_class_env: Option<&ClassEnv>,
) -> Result<TypedModule> {
    typecheck_internal_core(module, stdlib_class_env, None)
}

fn build_inferred_for_rewrite(
    module: &ast::Module,
    inferred: &HashMap<String, Scheme>,
    class_env: &ClassEnv,
    imported: Option<&HashMap<String, HashMap<String, Scheme>>>,
    entry_path: Option<&Path>,
) -> HashMap<String, Scheme> {
    let mut out = inferred.clone();
    let Some(imported) = imported else {
        return out;
    };

    let entry_dir = entry_path
        .and_then(|p| p.parent())
        .unwrap_or_else(|| Path::new("."));

    for it in &module.items {
        let ast::Item::Import(id) = it else {
            continue;
        };
        let Some(schemes) = imported.get(&id.module) else {
            continue;
        };

        // Apply import spec filter consistently with inference.
        let all_names: std::collections::HashSet<String> = schemes.keys().cloned().collect();
        let filtered_names: std::collections::HashSet<String> = match &id.import_spec {
            None => all_names,
            Some(ast::ImportSpec::Only(specs)) => {
                expand_import_spec_with_ctors(specs, &id.module, entry_dir)
            }
            Some(ast::ImportSpec::Hiding(specs)) => {
                let hidden = expand_import_spec_with_ctors(specs, &id.module, entry_dir);
                all_names
                    .into_iter()
                    .filter(|n| !hidden.contains(n))
                    .collect()
            }
        };

        for name in filtered_names {
            let Some(scheme) = schemes.get(&name) else {
                continue;
            };

            if let Some(as_name) = &id.as_name {
                // Aliased import: expose only alias-qualified names.
                out.entry(format!("{}.{}", as_name, name))
                    .or_insert_with(|| scheme.clone());
                continue;
            }

            if id.qualified {
                // Qualified import without alias: expose only Module.name.
                out.entry(format!("{}.{}", id.module, name))
                    .or_insert_with(|| scheme.clone());
            } else {
                // Unqualified import: allow both Module.name and name.
                out.entry(format!("{}.{}", id.module, name))
                    .or_insert_with(|| scheme.clone());
                out.entry(name).or_insert_with(|| scheme.clone());
            }
        }
    }

    // Mirror inference-time constructor re-exports that may not be present in `.ksif`.
    // This is needed for rewrite-time inference on qualified ctor uses like `M.Just`.
    if imported.contains_key("Data.Maybe") {
        if let Some(just) = out
            .get("Just")
            .cloned()
            .or_else(|| out.get("Prelude.Just").cloned())
        {
            out.entry("Data.Maybe.Just".to_string()).or_insert(just);
        }
        if let Some(nothing) = out
            .get("Nothing")
            .cloned()
            .or_else(|| out.get("Prelude.Nothing").cloned())
        {
            out.entry("Data.Maybe.Nothing".to_string())
                .or_insert(nothing);
        }

        for it in &module.items {
            let ast::Item::Import(id) = it else {
                continue;
            };
            if id.module != "Data.Maybe" {
                continue;
            }
            let Some(as_name) = &id.as_name else {
                continue;
            };
            if let Some(just) = out.get("Data.Maybe.Just").cloned() {
                out.entry(format!("{}.Just", as_name)).or_insert(just);
            }
            if let Some(nothing) = out.get("Data.Maybe.Nothing").cloned() {
                out.entry(format!("{}.Nothing", as_name)).or_insert(nothing);
            }
        }
    }

    // Ensure instance dictionary bindings referenced by name are available to rewrite-time inference.
    // `infer_in_module_with_class_env` only consults `inferred`, not module items, so injected
    // dict bindings like `Prelude.__dict_Show_Integer` must have schemes here.
    for dict_name in class_env.instances.values() {
        // Dict names are stored as qualified bindings (e.g. `Prelude.__dict_Show_Integer` or
        // `Prelude.Num.__dict_Num_Integer`). Imported scheme maps use the module-local name key
        // (e.g. `__dict_Show_Integer`).
        let (module_prefix, local_name) = match dict_name.rsplit_once('.') {
            Some(x) => x,
            None => continue,
        };
        let Some(schemes) = imported.get(module_prefix) else {
            continue;
        };
        let Some(scheme) = schemes.get(local_name) else {
            continue;
        };

        // Make both the qualified dict name and the unqualified forwarder name visible.
        out.entry(dict_name.clone())
            .or_insert_with(|| scheme.clone());
        out.entry(local_name.to_string())
            .or_insert_with(|| scheme.clone());
    }

    out
}

fn typecheck_internal_core_with_entry_path(
    mut module: ast::Module,
    stdlib_class_env: Option<&ClassEnv>,
    imported: Option<HashMap<String, HashMap<String, Scheme>>>,
    entry_path: Option<&Path>,
) -> Result<TypedModule> {
    // Collect docs from the source AST once. Later desugaring/rewrites may drop or rewrite items.
    let source_docs = collect_toplevel_docs(&module);

    WithAliasEvidence::run(|| {
        // Module-unit compilation: imports remain as syntax, but are satisfied via `.ksif`.

        // `typecheck` (AST-only) does not resolve imports; require `typecheck_file` for that.
        if imported.is_none()
            && module
                .items
                .iter()
                .any(|it| matches!(it, ast::Item::Import(_)))
        {
            return Err(Error::msg("imports require typecheck_file"));
        }

        // Try to hit the module-level typecheck cache.
        // NOTE: cache is only used when imported schemes are not provided.
        if imported.is_none() {
            if let Some(inferred) = stdlib_cache::check_module_typecheck_cache(&module) {
                return Ok(TypedModule {
                    module,
                    inferred,
                    docs: source_docs.clone(),
                });
            }
        }

        let name_hints = collect_unqualified_name_hints_from_imported(&module);
        TL_NAME_HINTS.with(|h| *h.borrow_mut() = name_hints);

        let aliases = collect_type_aliases(&module);
        let alias_def_sites = collect_type_alias_def_sites(&module, entry_path);
        // Make type alias def-sites available to unify evidence.
        TL_DEF_EVIDENCE.with(|slot| {
            if let Some(ev) = &mut *slot.borrow_mut() {
                for (name, site) in alias_def_sites {
                    if name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                        // Type names that start uppercase are type constructors.
                        ev.def_sites.type_ctor.insert(name, site);
                    } else {
                        ev.def_sites.type_alias.insert(name, site);
                    }
                }
            }
        });
        let items_in = std::mem::take(&mut module.items);
        module.items = items_in
            .into_iter()
            .map(|it| expand_item(it, &aliases))
            .collect::<Result<Vec<_>>>()?;

        fn filter_stdlib_env_for_prelude_import_spec(
            module: &ast::Module,
            stdlib_env: &ClassEnv,
        ) -> ClassEnv {
            use std::collections::HashSet;

            let mut hidden_names: HashSet<String> = HashSet::new();
            for it in &module.items {
                let ast::Item::Import(id) = it else {
                    continue;
                };
                if id.module != "Prelude" {
                    continue;
                }
                let Some(ast::ImportSpec::Hiding(specs)) = &id.import_spec else {
                    continue;
                };
                for s in specs {
                    if let ast::ExportSpec::Name(n) = s {
                        hidden_names.insert(n.clone());
                    }
                }
            }

            if hidden_names.is_empty() {
                return stdlib_env.clone();
            }

            let mut out = stdlib_env.clone();
            for name in hidden_names {
                if let Some(classes) = out.method_classes.get_mut(&name) {
                    classes.retain(|cid| !cid.name.starts_with("Prelude."));
                    if classes.is_empty() {
                        out.method_classes.remove(&name);
                    }
                }

                // Also remove the method type entries so we can build a unique-by-name
                // method scheme index for inference.
                out.methods
                    .retain(|(cid, m), _| !(m == &name && cid.name.starts_with("Prelude.")));
            }
            out
        }

        // Desugar typeclasses. For stdlib modules, use relaxed validation since
        // superclasses may be in other stdlib modules not yet in this module's env.
        let is_stdlib = entry_path.is_some_and(is_stdlib_path);
        let mut class_env = if is_stdlib {
            // For stdlib modules: collect classes and process instances without superclass validation
            // Validation will be done when user modules import these classes
            let (env, (class_method_names, class_default_methods)) =
                collect_class_env_only(&mut module, false)?;

            // Process instances
            let instance_decls = collect_instance_decls(&module);
            let mut working_env = env.clone();
            preregister_instance_dicts(&mut working_env, &instance_decls, module.name.as_deref())?;
            let extra_items = generate_instance_items(
                &working_env,
                &instance_decls,
                &class_method_names,
                &class_default_methods,
            )?;

            module.items = module
                .items
                .drain(..)
                .filter(|it| !matches!(it, ast::Item::ClassDecl(_) | ast::Item::InstanceDecl(_)))
                .chain(extra_items)
                .collect();

            env
        } else {
            desugar_typeclasses(&mut module)?
        };
        // Merge stdlib class env.
        // IMPORTANT: honor `import Prelude hiding (...)` for method-name disambiguation.
        if let Some(stdlib_env) = stdlib_class_env {
            let filtered = filter_stdlib_env_for_prelude_import_spec(&module, stdlib_env);
            merge_class_env(&mut class_env, &filtered)?;

            if std::env::var("KSCR_DEBUG_EQ_INTEGER").ok().as_deref() == Some("1") {
                let eq = class_env
                    .class_params
                    .keys()
                    .find(|cid| cid.name == "Prelude.Eq" || cid.name.ends_with(".Eq"))
                    .cloned();
                if let Some(eq) = eq {
                    let key = (eq.clone(), "Integer".to_string());
                    let in_filtered = filtered.instances.contains_key(&key);
                    let in_merged = class_env.instances.contains_key(&key);
                    eprintln!(
                        "[KSCR_DEBUG_EQ_INTEGER] module={:?} Eq class={} module_id={:?} in_filtered={} in_merged={} filtered_instances={} merged_instances={}",
                        module.name,
                        eq.name,
                        eq.module,
                        in_filtered,
                        in_merged,
                        filtered.instances.len(),
                        class_env.instances.len()
                    );
                    if !in_filtered {
                        let keys: Vec<_> = filtered
                            .instances
                            .keys()
                            .filter(|(c, _)| c.name.ends_with(".Eq"))
                            .map(|(c, t)| format!("{} {}", c.name, t))
                            .collect();
                        eprintln!(
                            "[KSCR_DEBUG_EQ_INTEGER] filtered Eq instance keys: {:?}",
                            keys
                        );
                    }
                } else {
                    eprintln!(
                        "[KSCR_DEBUG_EQ_INTEGER] module={:?} Prelude.Eq class not found in class_env",
                        module.name
                    );
                }
            }
        }

        // Load and merge class instances from imported user modules.
        // This allows Main to use instances defined in module A when Main imports A.
        if let Some(ep) = entry_path {
            let entry_dir = ep.parent().unwrap_or_else(|| Path::new("."));
            let (imported_instances, dict_bindings) =
                load_imported_instances(&module, entry_dir, entry_path)?;
            merge_class_env(&mut class_env, &imported_instances)?;
            // Inject dictionary bindings into the module so they're available at runtime
            module.items.extend(dict_bindings);
        }

        // If `>>=` and `>>` operators are available (typically from a Monad-like type class),
        // desugar `do`-notation into those operators. This allows `do` to work for any monad
        // (via type classes), not just IO.
        // If these operators are not available, do-notation remains as ExprKind::Do and is
        // lowered directly to IR IoBind/IoThen constructs (IO-only fallback).
        if class_env.method_classes.contains_key(">>=")
            && class_env.method_classes.contains_key(">>")
        {
            desugar_do_to_monad_ops_in_module(&mut module)?;
        }

        // Build method-name fallback index (Haskell-like: methods are values).
        // Use the merged class env if stdlib is provided.
        let mut cx_for_index = InferCtx::default();
        let class_index = build_class_method_scheme_index(&mut cx_for_index, &class_env)?;

        let inferred = if let Some(entry_path) = entry_path {
            // Best-effort: when typechecking from a file, we have a ModuleLoader in the caller
            // (typecheck_file) and can provide real file:line:col evidence.
            // If there is no loader context, this falls back to no evidence.
            infer_module_with_class_env_with_entry_path(
                &module,
                &class_env,
                &class_index,
                imported.as_ref(),
                Some(entry_path),
            )?
        } else {
            infer_module_with_class_env_with_entry_path(
                &module,
                &class_env,
                &class_index,
                imported.as_ref(),
                None,
            )?
        };

        let mut main_entry_class_constraints: Vec<Constraint> = Vec::new();
        if let Some(main) = inferred.get("main") {
            // Haskell-like: accept any `IO a` as an entry point.
            // If `main` is polymorphic (e.g. `forall m. Monad m => m Unit`), we accept it iff it
            // can be instantiated to `IO a` with all constraints solved.
            let mut cx = InferCtx::default();
            let (cs, ty) = instantiate_qual(&mut cx, main);
            let Ty::Var(a) = cx.fresh() else {
                unreachable!()
            };
            let io_a = Ty::App {
                head: Box::new(Ty::Con("IO".to_string())),
                args: vec![Ty::Var(a)],
            };
            let subst = unify(ty, io_a).map_err(|_| Error::msg("main must have type IO _"))?;

            let cs_raw = apply_constraints(&subst, cs);

            let data_env = collect_data_env(&module);
            let cs_simplified = simplify_constraints(&data_env, &class_env, cs_raw.clone())?;
            // Allow class constraints on `main` (e.g. `Show Box => IO Unit`) and discharge them
            // by inserting dictionary applications after dict-passing rewrite.
            // Any non-class constraints still reject entrypoints.
            if cs_simplified
                .iter()
                .any(|c| !matches!(c, Constraint::Class { .. }))
            {
                return Err(Error::msg("main must have type IO _"));
            }
            main_entry_class_constraints = cs_raw
                .into_iter()
                .filter(|c| matches!(c, Constraint::Class { .. }))
                .collect();
        }

        // Inject method value bindings early so dict-passing can thread dictionaries into them.
        // (e.g. `enumFromTo` becomes a function expecting `__dict_Enum`.)
        inject_class_method_value_bindings(&mut module, &class_env, &inferred);

        // Inject unqualified forwarders for stdlib instance dictionaries so dict-passing can reference them.
        // Instance dictionaries like `Prelude.Num.__dict_Num_Integer` need unqualified forwarders
        // like `__dict_Num_Integer = Prelude.Num.__dict_Num_Integer`.
        inject_stdlib_instance_dict_forwarders_post_typecheck(&mut module)?;

        // During rewrite we may need imported schemes (e.g. `M.fromMaybe`) that were available
        // during inference via `.ksif` imports but are not emitted as top-level bindings in `inferred`.
        let inferred_for_rewrite = build_inferred_for_rewrite(
            &module,
            &inferred,
            &class_env,
            imported.as_ref(),
            entry_path,
        );

        typeclass_dict_passing_common::rewrite_class_dict_passing_in_module(
            &mut module,
            &class_env,
            &inferred_for_rewrite,
        )?;

        rewrite_entry_main_apply_dicts(&mut module, &class_env, &main_entry_class_constraints)?;

        // Rewrite method calls/vars while dictionary bindings still exist.
        rewrite_class_method_calls_in_module(&mut module, &class_env, &inferred_for_rewrite)?;

        // Drop class / instance decls after desugaring.
        module
            .items
            .retain(|it| !matches!(it, ast::Item::ClassDecl(_) | ast::Item::InstanceDecl(_)));

        Ok(TypedModule {
            module,
            inferred,
            docs: source_docs,
        })
    })
}

#[allow(dead_code)]
#[allow(clippy::too_many_lines)]
fn typecheck_internal_core(
    module: ast::Module,
    stdlib_class_env: Option<&ClassEnv>,
    imported: Option<HashMap<String, HashMap<String, Scheme>>>,
) -> Result<TypedModule> {
    typecheck_internal_core_with_entry_path(module, stdlib_class_env, imported, None)
}

fn load_stdlib_class_env() -> Result<ClassEnv> {
    static CACHE: OnceLock<std::sync::Mutex<Option<ClassEnv>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(None));

    if let Ok(guard) = cache.lock() {
        if let Some(v) = guard.as_ref() {
            return Ok(v.clone());
        }
    }

    let v = load_stdlib_class_env_uncached()?;

    if let Ok(mut guard) = cache.lock() {
        if guard.is_none() {
            *guard = Some(v.clone());
        }
    }

    Ok(v)
}

fn load_stdlib_class_env_uncached() -> Result<ClassEnv> {
    // Approach B: Build per-module ClassEnv then merge, without constructing a single merged AST module.
    //
    // Important: CLI `run`/`typecheck` goes through `typecheck_file()` which flattens imports.
    // Some programs rely on typeclass instances from stdlib even without an explicit
    // `import Prelude` (e.g. `do`-notation requires `Monad IO`, arithmetic uses `Ring Integer`).
    // These must be present in the merged `ClassEnv` so `simplify_process_constraint` can
    // discharge ground constraints.
    let timing = std::env::var("KSCR_DEBUG_TIMING").ok().as_deref() == Some("1");
    let t_all = std::time::Instant::now();

    let stdlib = stdlib_root();

    // Important: stdlib modules are allowed to have imports.
    // Parsing them by raw text here bypasses the stdlib cache + import logic and can
    // yield confusing failures. Use ModuleLoader to keep behavior consistent.
    let mut loader = ModuleLoader::new();

    let t_scan = std::time::Instant::now();
    let mut module_paths = Vec::new();
    let mut stack: Vec<PathBuf> = vec![stdlib];
    while let Some(dir) = stack.pop() {
        for ent in std::fs::read_dir(&dir).map_err(Error::Io)? {
            let ent = ent.map_err(Error::Io)?;
            let path = ent.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|s| s.to_str()) != Some("ks") {
                continue;
            }
            module_paths.push(path);
        }
    }
    if timing {
        eprintln!(
            "[KSCR_DEBUG_TIMING] stdlib scan (class/instance): {:.3}s",
            t_scan.elapsed().as_secs_f64()
        );
    }

    // Build a merged ClassEnv by processing each module separately in two passes:
    // Pass 1: Collect all class declarations (creates merged class definitions)
    // Pass 2: Process all instances (now all classes are known)
    let mut merged_env = ClassEnv::default();
    let mut all_class_method_names: HashMap<ast::ClassId, Vec<String>> = HashMap::new();
    let mut all_class_default_methods: HashMap<(ast::ClassId, String), ast::Expr> = HashMap::new();
    let t_desugar = std::time::Instant::now();

    // Pass 1: Collect class declarations from all modules
    for path in &module_paths {
        // Load AST via the loader so stdlib cache and parsing stays consistent.
        let parsed = loader.load_ast(path).map_err(|e| {
            e.with_context(format!("while loading stdlib module {}", path.display()))
        })?;

        // Create a temporary module containing ClassDecl and Import items only.
        // We need Import items for canonicalization to work correctly.
        let mut tmp_module = ast::Module {
            name: parsed.name.clone(),
            export_specs: None,
            items: parsed
                .items
                .into_iter()
                .filter(|it| matches!(it, ast::Item::ClassDecl(_) | ast::Item::Import(_)))
                .collect(),
        };

        // Skip modules with no class declarations.
        if !tmp_module
            .items
            .iter()
            .any(|it| matches!(it, ast::Item::ClassDecl(_)))
        {
            continue;
        }

        // Collect class declarations only
        let (module_env, (class_method_names, class_default_methods)) =
            collect_class_env_only(&mut tmp_module, true).map_err(|e| {
                e.with_context(format!(
                    "while collecting class decls in stdlib module {}",
                    path.display()
                ))
            })?;

        // Merge into the global stdlib_env
        merge_class_env(&mut merged_env, &module_env).map_err(|e| {
            e.with_context(format!(
                "while merging ClassEnv from stdlib module {}",
                path.display()
            ))
        })?;

        // Accumulate class method metadata for Pass 2
        all_class_method_names.extend(class_method_names);
        all_class_default_methods.extend(class_default_methods);
    }

    // After all classes are collected, validate superclasses and check for cycles
    reject_ambiguous_method_names(&mut merged_env)?;
    validate_superclass_preds(&merged_env)?;
    detect_superclass_cycles(&merged_env)?;

    // Pass 2: Process instance declarations with the merged ClassEnv
    for path in &module_paths {
        // Re-load AST (cached by loader)
        let parsed = loader.load_ast(path).map_err(|e| {
            e.with_context(format!("while loading stdlib module {}", path.display()))
        })?;

        // Create a temporary module containing InstanceDecl, ClassDecl, Import, and DataDecl items.
        // DataDecl is needed so `deriving (...)` can expand into synthetic InstanceDecls.
        // We need ClassDecl items for canonicalization to resolve locally-defined classes.
        let mut tmp_module = ast::Module {
            name: parsed.name.clone(),
            export_specs: None,
            items: parsed
                .items
                .into_iter()
                .filter(|it| {
                    matches!(
                        it,
                        ast::Item::InstanceDecl(_)
                            | ast::Item::ClassDecl(_)
                            | ast::Item::Import(_)
                            | ast::Item::DataDecl(_)
                    )
                })
                .collect(),
        };

        // Skip modules with neither explicit instances nor deriving clauses.
        if !tmp_module.items.iter().any(|it| match it {
            ast::Item::InstanceDecl(_) => true,
            ast::Item::DataDecl(dd) => !dd.deriving.is_empty(),
            _ => false,
        }) {
            continue;
        }

        // Process instances against the merged ClassEnv
        let mut module_env = process_instances_with_env(
            &mut tmp_module,
            &merged_env,
            &all_class_method_names,
            &all_class_default_methods,
            true,
        )
        .map_err(|e| {
            e.with_context(format!(
                "while processing instances in stdlib module {}",
                path.display()
            ))
        })?;

        // Qualify instance dict names with the module name before merging
        if let Some(mod_name) = &tmp_module.name {
            qualify_instance_dict_names(&mut module_env, mod_name);
        }

        // Merge instance registrations into the global env
        merge_class_env(&mut merged_env, &module_env).map_err(|e| {
            e.with_context(format!(
                "while merging instances from stdlib module {}",
                path.display()
            ))
        })?;
    }

    if timing {
        eprintln!(
            "[KSCR_DEBUG_TIMING] desugar stdlib typeclasses: {:.3}s",
            t_desugar.elapsed().as_secs_f64()
        );
        eprintln!(
            "[KSCR_DEBUG_TIMING] load_stdlib_class_env total: {:.3}s",
            t_all.elapsed().as_secs_f64()
        );
    }

    // Safety net: ensure primitive Eq instances exist.
    // Some programs rely on `Prelude.(==)` which introduces an `Eq` constraint,
    // and must be dischargeable for primitive types like Integer.
    fn find_class(env: &ClassEnv, short: &str) -> Option<ast::ClassId> {
        let preferred = format!("Prelude.{short}");
        if let Some(cid) = env.class_params.keys().find(|cid| cid.name == preferred) {
            return Some(cid.clone());
        }
        env.class_params
            .keys()
            .find(|cid| cid.name == short || cid.name.ends_with(&format!(".{short}")))
            .cloned()
    }

    if let Some(eq_class) = find_class(&merged_env, "Eq") {
        for ty_key in ["Integer", "Bool", "Char", "Unit", "Float64"] {
            let key = (eq_class.clone(), ty_key.to_string());
            merged_env
                .instances
                .entry(key)
                .or_insert_with(|| "__builtinEqDict".to_string());
        }
    }

    Ok(merged_env)
}

fn load_stdlib_class_decl_items() -> Result<Vec<ast::Item>> {
    static CACHE: OnceLock<std::sync::Mutex<Option<Vec<ast::Item>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(None));

    if let Ok(guard) = cache.lock() {
        if let Some(v) = guard.as_ref() {
            return Ok(v.clone());
        }
    }

    let v = load_stdlib_class_decl_items_uncached()?;

    if let Ok(mut guard) = cache.lock() {
        if guard.is_none() {
            *guard = Some(v.clone());
        }
    }

    Ok(v)
}

fn load_stdlib_class_decl_items_uncached() -> Result<Vec<ast::Item>> {
    let timing = std::env::var("KSCR_DEBUG_TIMING").ok().as_deref() == Some("1");
    let t0 = std::time::Instant::now();

    // Collect ClassDecls over stdlib modules (Prelude + Prelude/*).
    let stdlib = stdlib_root();

    let mut loader = ModuleLoader::new();

    // Collect all class decls into a temporary merged module.
    let mut merged = ast::Module {
        name: Some("<stdlib-classes>".to_string()),
        export_specs: None,
        items: Vec::new(),
    };
    let mut seen: HashSet<String> = HashSet::new();

    let mut stack: Vec<PathBuf> = vec![stdlib];
    while let Some(dir) = stack.pop() {
        for ent in std::fs::read_dir(&dir).map_err(Error::Io)? {
            let ent = ent.map_err(Error::Io)?;
            let path = ent.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|s| s.to_str()) != Some("ks") {
                continue;
            }

            let parsed = loader.load_ast(&path).map_err(|e| {
                e.with_context(format!("while loading stdlib module {}", path.display()))
            })?;

            for it in parsed.items {
                if let ast::Item::ClassDecl(c) = &it {
                    if seen.insert(c.name.clone()) {
                        merged.items.push(it);
                    }
                }
            }
        }
    }

    // Canonicalize class names in the merged module using def_module fields.
    canonicalize_class_names_in_merged_stdlib(&mut merged);

    if timing {
        eprintln!(
            "[KSCR_DEBUG_TIMING] stdlib scan (class decls): {:.3}s",
            t0.elapsed().as_secs_f64()
        );
    }

    Ok(merged.items)
}

/// Load class instances from all imported user modules (non-stdlib).
/// This is needed so that instances defined in module A are available when Main imports A.
fn load_imported_instances(
    module: &ast::Module,
    entry_dir: &Path,
    entry_path: Option<&Path>,
) -> Result<(ClassEnv, Vec<ast::Item>)> {
    fn qualify_dict_refs_in_expr(expr: ast::Expr, module_prefix: &str) -> ast::Expr {
        use ast::{Expr, ExprKind};
        let span = expr.span;
        match expr.kind {
            ExprKind::Var(name) if name.starts_with("__dict_") || name.starts_with("__inst_") => {
                Expr::new(span, ExprKind::Var(format!("{}.{}", module_prefix, name)))
            }
            ExprKind::Lambda { params, body } => Expr::new(
                span,
                ExprKind::Lambda {
                    params,
                    body: Box::new(qualify_dict_refs_in_expr(*body, module_prefix)),
                },
            ),
            ExprKind::Apply { func, args } => Expr::new(
                span,
                ExprKind::Apply {
                    func: Box::new(qualify_dict_refs_in_expr(*func, module_prefix)),
                    args: args
                        .into_iter()
                        .map(|e| qualify_dict_refs_in_expr(e, module_prefix))
                        .collect(),
                },
            ),
            ExprKind::Record(fields) => Expr::new(
                span,
                ExprKind::Record(
                    fields
                        .into_iter()
                        .map(|(k, v)| (k, qualify_dict_refs_in_expr(v, module_prefix)))
                        .collect(),
                ),
            ),
            // For other expression kinds, return as-is
            _ => expr,
        }
    }

    let mut merged_env = ClassEnv::default();
    let mut dict_bindings: Vec<ast::Item> = Vec::new();
    let stdlib_root = stdlib_cache::stdlib_root()?;

    // Track module names to detect collisions
    let mut module_name_to_paths: std::collections::HashMap<String, Vec<std::path::PathBuf>> =
        std::collections::HashMap::new();

    // Register the current module being typechecked
    if let (Some(module_name), Some(ep)) = (&module.name, entry_path) {
        if let Ok(canonical_path) = std::fs::canonicalize(ep) {
            module_name_to_paths
                .entry(module_name.clone())
                .or_default()
                .push(canonical_path);
        }
    }

    for it in &module.items {
        let ast::Item::Import(id) = it else {
            continue;
        };

        // Skip stdlib modules - they're handled via load_stdlib_class_env.
        let rel = id.module.replace('.', "/");
        let local = entry_dir.join(format!("{}.ks", rel));

        // Only process if it's a local (non-stdlib) module
        let Ok(module_path) = std::fs::canonicalize(&local) else {
            continue;
        };

        if module_path.starts_with(&stdlib_root) {
            continue;
        }

        // Load and parse the imported module
        let src = std::fs::read_to_string(&module_path)?;
        let mut imported_ast = parser::parse_module(&src)?;
        desugar_module_qualified_names(&mut imported_ast)?;

        // Populate def_module for ClassDecls
        if let Some(name) = &imported_ast.name {
            for it in &mut imported_ast.items {
                if let ast::Item::ClassDecl(c) = it {
                    c.def_module = Some(name.clone());
                }
            }
        }

        // Track module names to detect collisions
        if let Some(module_name) = &imported_ast.name {
            let paths_for_module = module_name_to_paths.entry(module_name.clone()).or_default();

            // Only add if not already present (same module imported multiple times)
            if !paths_for_module.contains(&module_path) {
                paths_for_module.push(module_path.clone());
            }

            // Check for collision after adding this path
            if paths_for_module.len() > 1 {
                let mut msg = format!("module '{}' is defined in multiple files:", module_name);
                for p in paths_for_module {
                    msg.push_str(&format!("\n  - {}", p.display()));
                }
                return Err(Error::msg(msg));
            }
        }

        // Create a temporary module with only class and instance declarations
        let mut tmp_module = ast::Module {
            name: imported_ast.name.clone(),
            export_specs: None,
            items: imported_ast
                .items
                .into_iter()
                .filter(|it| {
                    matches!(
                        it,
                        ast::Item::ClassDecl(_)
                            | ast::Item::InstanceDecl(_)
                            | ast::Item::DataDecl(_)
                            | ast::Item::Import(_)
                    )
                })
                .collect(),
        };

        // Skip if no instances or class declarations
        if !tmp_module.items.iter().any(|it| {
            matches!(
                it,
                ast::Item::InstanceDecl(_) | ast::Item::ClassDecl(_) | ast::Item::DataDecl(_)
            )
        }) {
            continue;
        }

        // Canonicalize class names and desugar typeclasses to get instances

        // Bring stdlib class declarations into scope so `deriving (Show, Eq)` in imported
        // modules can resolve to stdlib classes (e.g. Prelude.Show) when we collect instances.
        inject_stdlib_class_decls(&mut tmp_module)?;
        let module_env = desugar_typeclasses(&mut tmp_module)?;

        // Collect dictionary bindings from the desugared module
        // These need to be injected into the importing module with qualified names
        for it in &tmp_module.items {
            if let ast::Item::Binding(b) = it {
                if let ast::PatternKind::Var(name) = &b.pat.kind {
                    if name.starts_with("__dict_") || name.starts_with("__inst_") {
                        // Qualify the binding name and all __dict_/__inst_ references in the expression
                        let qual_name = format!("{}.{}", id.module, name);
                        let qual_expr = qualify_dict_refs_in_expr(b.expr.clone(), &id.module);
                        let qual_binding = ast::Binding {
                            doc: b.doc.clone(),
                            pat: ast::Pattern::new(b.pat.span, ast::PatternKind::Var(qual_name)),
                            expr: qual_expr,
                            span: b.span,
                        };
                        dict_bindings.push(ast::Item::Binding(qual_binding));
                    }
                }
            }
        }

        // Don't qualify dict names here - they should remain unqualified within the module.
        // Qualification happens when merging into the importing module's class_env.

        // Merge this module's instances into the accumulated env
        merge_class_env_with_module_prefix(&mut merged_env, &module_env, &id.module)?;
    }

    Ok((merged_env, dict_bindings))
}

fn merge_class_env(dst: &mut ClassEnv, src: &ClassEnv) -> Result<()> {
    merge_class_definitions_only(dst, src)?;

    // Also merge instances
    for (k, v) in &src.instances {
        dst.instances.entry(k.clone()).or_insert_with(|| v.clone());
    }

    // Merge polymorphic instances too.
    // These are required for common stdlib constraints (e.g. `Show (Maybe a)`).
    for pi in &src.poly_instances {
        if !dst
            .poly_instances
            .iter()
            .any(|dpi| dpi.dict_name == pi.dict_name)
        {
            dst.poly_instances.push(pi.clone());
        }
    }

    Ok(())
}

/// Merge class env with module prefix qualification for instance dict names.
/// This is used when merging instances from imported user modules so they're accessible
/// with module-qualified names.
fn merge_class_env_with_module_prefix(
    dst: &mut ClassEnv,
    src: &ClassEnv,
    module_prefix: &str,
) -> Result<()> {
    merge_class_definitions_only(dst, src)?;

    // Merge instances with qualified dict names
    for (k, v) in &src.instances {
        let qualified_name = if v.contains('.') {
            v.clone()
        } else {
            format!("{}.{}", module_prefix, v)
        };
        dst.instances.entry(k.clone()).or_insert(qualified_name);
    }

    // Merge poly instances with qualified dict names
    for pi in &src.poly_instances {
        let qualified_dict_name = if pi.dict_name.contains('.') {
            pi.dict_name.clone()
        } else {
            format!("{}.{}", module_prefix, pi.dict_name)
        };
        let qualified_pi = PolyInstance {
            class: pi.class.clone(),
            head_pat: pi.head_pat.clone(),
            ctx_len: pi.ctx_len,
            dict_name: qualified_dict_name,
        };
        dst.poly_instances.push(qualified_pi);
    }

    Ok(())
}

/// Qualify instance dictionary names with a module prefix.
/// This is needed when merging instances from stdlib modules so that Main can reference them
/// with module-qualified names like `Prelude.Rational.__dict_Ring_Rational`.
fn qualify_instance_dict_names(env: &mut ClassEnv, module_name: &str) {
    // Qualify dict names in the instances map
    for (_, dict_name) in env.instances.iter_mut() {
        if !dict_name.contains('.') {
            *dict_name = format!("{}.{}", module_name, dict_name);
        }
    }

    // Qualify dict names in poly_instances
    for pi in env.poly_instances.iter_mut() {
        if !pi.dict_name.contains('.') {
            pi.dict_name = format!("{}.{}", module_name, pi.dict_name);
        }
    }
}

/// Merge only class definitions (class_params, class_supers, methods, method_classes)
/// without merging instances. Used when merging stdlib class env into stdlib modules
/// to avoid duplicate instances.
fn merge_class_definitions_only(dst: &mut ClassEnv, src: &ClassEnv) -> Result<()> {
    for (class, param) in &src.class_params {
        match dst.class_params.get(class) {
            Some(existing) if existing == param => {}
            Some(_) => {
                return Err(Error::msg(format!(
                    "conflicting class param for class {}",
                    class.name
                )))
            }
            None => {
                dst.class_params.insert(class.clone(), param.clone());
            }
        }
    }

    for (class, supers) in &src.class_supers {
        dst.class_supers
            .entry(class.clone())
            .or_insert_with(|| supers.clone());
    }

    for (m, classes) in &src.method_classes {
        let e = dst.method_classes.entry(m.clone()).or_default();
        e.extend(classes.iter().cloned());

        e.sort();
        e.dedup();
    }

    // Compare method signatures modulo type aliases.
    // Note: different sources may preserve aliases while others are already expanded.
    let normalize_qt = |qt: ast::QualType| -> Result<ast::QualType> {
        let mut aliases: HashMap<String, ast::TypeAlias> = HashMap::new();

        // Built-in surface alias: String = [Char].
        aliases.insert(
            "String".to_string(),
            ast::TypeAlias {
                doc: None,
                name: "String".to_string(),
                params: Vec::new(),
                ty: ast::Type::List(Box::new(ast::Type::Char)),
                span: ast::dummy_span(),
            },
        );

        // Also include aliases from both envs.
        // If there are name conflicts, prefer the destination env.
        aliases.extend(src.aliases.iter().map(|(k, v)| (k.clone(), v.clone())));
        for (k, v) in &dst.aliases {
            aliases.insert(k.clone(), v.clone());
        }

        expand_qual_type(qt, &aliases)
    };

    for ((class, method), qt) in &src.methods {
        let key = (class.clone(), method.clone());
        if let Some(existing) = dst.methods.get(&key) {
            let existing_n = normalize_qt(existing.clone())?;
            let qt_n = normalize_qt(qt.clone())?;
            if existing_n != qt_n {
                return Err(Error::msg(format!(
                    "conflicting method type for {}.{method}",
                    class.name
                )));
            }
        } else {
            dst.methods.insert(key, qt.clone());
        }
    }

    reject_ambiguous_method_names(dst)?;
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ImportQualKey {
    path: PathBuf,
    qual: String,
}

#[derive(Debug, Clone)]
struct ModuleIdInterner {
    by_canonical_name: HashMap<String, ast::ModuleId>,
    /// Next fresh module id.
    ///
    /// `ModuleId(0)` is reserved as a dummy/unresolved sentinel.
    next: u32,
}

impl Default for ModuleIdInterner {
    fn default() -> Self {
        Self {
            by_canonical_name: HashMap::new(),
            next: 1,
        }
    }
}

impl ModuleIdInterner {
    fn intern(&mut self, canonical_module: &str) -> ast::ModuleId {
        if let Some(id) = self.by_canonical_name.get(canonical_module) {
            return *id;
        }
        let id = ast::ModuleId(self.next);
        self.next += 1;
        self.by_canonical_name
            .insert(canonical_module.to_string(), id);
        id
    }
}

struct ModuleLoader {
    cache: HashMap<PathBuf, ast::Module>,
    sources: HashMap<PathBuf, String>,
    stack: Vec<PathBuf>,
    #[allow(dead_code)]
    emitted_qualified: HashSet<ImportQualKey>,
    #[allow(dead_code)]
    emitted_unqualified: HashSet<ImportQualKey>,
    def_sites: DefSiteIndex,
    module_ids: ModuleIdInterner,
    /// Track module names to their file paths to detect collisions
    module_name_to_paths: HashMap<String, Vec<PathBuf>>,
}

impl ModuleLoader {
    fn new() -> Self {
        let mut module_ids = ModuleIdInterner::default();
        // Reserve ModuleId(0) for the dummy/unresolved sentinel.
        let _ = module_ids.intern("<dummy>");
        Self {
            cache: HashMap::new(),
            sources: HashMap::new(),
            stack: Vec::new(),
            emitted_qualified: HashSet::new(),
            emitted_unqualified: HashSet::new(),
            def_sites: DefSiteIndex::default(),
            module_ids,
            module_name_to_paths: HashMap::new(),
        }
    }
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

    // Always allow the module's own canonical qualifier, so internal references like
    // `Prelude.Nothing` remain valid even after import lowering.
    if let Some(name) = module.name.as_ref() {
        env.allowed.insert(name.clone());
        env.local_to_module.insert(name.clone(), name.clone());
    }

    for it in &module.items {
        let ast::Item::Import(id) = it else {
            continue;
        };
        let local = id.as_name.clone().unwrap_or_else(|| id.module.clone());
        env.allowed.insert(local.clone());
        env.local_to_module.insert(local, id.module.clone());

        // Keep canonical module qualifier syntactically valid under `import qualified M as Q`.
        // This ensures errors surface as "unbound variable: M.x" (not "unknown qualifier").
        if id.qualified {
            env.allowed.insert(id.module.clone());
            env.local_to_module
                .insert(id.module.clone(), id.module.clone());
        }
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
    let Some((qual, member)) = name.rsplit_once('.') else {
        return Ok(name.to_string());
    };

    // Special case: if either qualifier or member is empty, this isn't actually a qualified name.
    // This handles operators like "." itself, which would split into ("", "").
    if qual.is_empty() || member.is_empty() {
        return Ok(name.to_string());
    }

    if !env.allowed.contains(qual) {
        let mut allowed: Vec<_> = env.allowed.iter().cloned().collect();
        allowed.sort();
        return Err(Error::msg(format!(
            "unknown qualifier {qual} in {name} (allowed: {})",
            allowed.join(", ")
        )));
    }

    Ok(name.to_string())
}

fn desugar_qualified_class_id(id: &ast::ClassId, env: &QualEnv) -> Result<ast::ClassId> {
    // Validate qualifier only. Actual module-id resolution is done later in
    // `resolve_class_names_to_module_ids` (after we have a ModuleIdInterner).
    let name = desugar_qualified_ref(&id.name, env)?;
    Ok(ast::ClassId {
        module: id.module,
        name,
    })
}

fn desugar_qualified_expr(expr: ast::Expr, env: &QualEnv) -> Result<ast::Expr> {
    use ast::ExprKind;
    let span = expr.span;
    let kind = match expr.kind {
        ExprKind::Var(n) => ExprKind::Var(desugar_qualified_ref(&n, env)?),
        // Constructors must preserve qualification under `import qualified`.
        // Desugaring `P.Nothing` into `Nothing` breaks Haskell-like semantics.
        ExprKind::Ctor(n) => ExprKind::Ctor(n),
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
        doc: b.doc,
        pat: desugar_qualified_pattern(b.pat, env)?,
        expr: desugar_qualified_expr(b.expr, env)?,
        span: b.span,
    })
}

fn desugar_qualified_pattern(p: ast::Pattern, env: &QualEnv) -> Result<ast::Pattern> {
    use ast::PatternKind;
    let span = p.span;
    let kind = match p.kind {
        PatternKind::Var(n) => {
            // Allow single-character operator names like "." that technically contain '.'
            // but aren't qualified names (qualified names have the form "Module.name")
            if n.contains('.')
                && n.rsplit_once('.')
                    .is_some_and(|(q, m)| !q.is_empty() && !m.is_empty())
            {
                return Err(Error::msg(format!(
                    "qualified name is not allowed in binder: {n}"
                )));
            }
            PatternKind::Var(n)
        }
        PatternKind::As(n, p) => {
            if n.contains('.')
                && n.rsplit_once('.')
                    .is_some_and(|(q, m)| !q.is_empty() && !m.is_empty())
            {
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
                if rest_name.contains('.')
                    && rest_name
                        .rsplit_once('.')
                        .is_some_and(|(q, m)| !q.is_empty() && !m.is_empty())
                {
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
            name: ast::ResolvedName::unresolved(desugar_qualified_ref(
                &name.qualified_text(),
                env,
            )?),
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
            class: desugar_qualified_class_id(&class, env)?,
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

#[allow(dead_code)]
fn resolve_class_names_to_module_ids(
    module: &mut ast::Module,
    env: &QualEnv,
    module_ids: &mut ModuleIdInterner,
) {
    fn resolve_class_id(id: &mut ast::ClassId, env: &QualEnv, module_ids: &mut ModuleIdInterner) {
        let Some((qual, name)) = id.name.rsplit_once('.') else {
            return;
        };
        let Some(canonical) = env.local_to_module.get(qual) else {
            return;
        };
        id.module = module_ids.intern(canonical);
        // Keep the canonical (unqualified) name for identity.
        id.name = name.to_string();
    }

    for it in &mut module.items {
        match it {
            ast::Item::ClassDecl(c) => {
                for p in &mut c.supers {
                    if let ast::Predicate::Class { class, .. } = p {
                        resolve_class_id(class, env, module_ids);
                    }
                }
            }
            ast::Item::InstanceDecl(inst) => {
                for p in &mut inst.preds {
                    if let ast::Predicate::Class { class, .. } = p {
                        resolve_class_id(class, env, module_ids);
                    }
                }
                resolve_class_id(&mut inst.class, env, module_ids);
            }
            _ => {}
        }
    }
}

fn resolve_ctor_names_to_module_ids(
    module: &mut ast::Module,
    module_ids: &mut ModuleIdInterner,
) -> Result<()> {
    fn resolve_name(
        n: ast::ResolvedName,
        env: &QualEnv,
        module_ids: &mut ModuleIdInterner,
    ) -> ast::ResolvedName {
        let ast::ResolvedName::Unresolved(s) = n else {
            return n;
        };
        let Some((qual, name)) = s.rsplit_once('.') else {
            return ast::ResolvedName::Unresolved(s);
        };
        let Some(canonical) = env.local_to_module.get(qual) else {
            // Unknown qualifier: keep syntactic name.
            return ast::ResolvedName::Unresolved(s);
        };
        let id = module_ids.intern(canonical);
        ast::ResolvedName::Resolved {
            module: id,
            // Keep the *local* qualifier for printing/diagnostics.
            module_name: qual.to_string(),
            name: name.to_string(),
        }
    }

    fn go_expr(e: ast::Expr, env: &QualEnv, module_ids: &mut ModuleIdInterner) -> ast::Expr {
        use ast::ExprKind;
        let span = e.span;
        let kind = match e.kind {
            ExprKind::Ctor(n) => ExprKind::Ctor(resolve_name(n, env, module_ids)),
            ExprKind::Lambda { params, body } => ExprKind::Lambda {
                params,
                body: Box::new(go_expr(*body, env, module_ids)),
            },
            ExprKind::Apply { func, args } => ExprKind::Apply {
                func: Box::new(go_expr(*func, env, module_ids)),
                args: args
                    .into_iter()
                    .map(|x| go_expr(x, env, module_ids))
                    .collect(),
            },
            ExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => ExprKind::If {
                cond: Box::new(go_expr(*cond, env, module_ids)),
                then_branch: Box::new(go_expr(*then_branch, env, module_ids)),
                else_branch: Box::new(go_expr(*else_branch, env, module_ids)),
            },
            ExprKind::Let { bindings, body } => ExprKind::Let {
                bindings: bindings
                    .into_iter()
                    .map(|b| ast::Binding {
                        doc: b.doc,
                        pat: go_pat(b.pat, env, module_ids),
                        expr: go_expr(b.expr, env, module_ids),
                        span: b.span,
                    })
                    .collect(),
                body: Box::new(go_expr(*body, env, module_ids)),
            },
            ExprKind::Where { expr, bindings } => ExprKind::Where {
                expr: Box::new(go_expr(*expr, env, module_ids)),
                bindings: bindings
                    .into_iter()
                    .map(|b| ast::Binding {
                        doc: b.doc,
                        pat: go_pat(b.pat, env, module_ids),
                        expr: go_expr(b.expr, env, module_ids),
                        span: b.span,
                    })
                    .collect(),
            },
            ExprKind::Annot { expr, ty } => ExprKind::Annot {
                expr: Box::new(go_expr(*expr, env, module_ids)),
                ty,
            },
            ExprKind::Do(stmts) => ExprKind::Do(
                stmts
                    .into_iter()
                    .map(|s| match s {
                        ast::DoStmt::Bind { pat, expr } => ast::DoStmt::Bind {
                            pat: go_pat(pat, env, module_ids),
                            expr: go_expr(expr, env, module_ids),
                        },
                        ast::DoStmt::Expr(x) => ast::DoStmt::Expr(go_expr(x, env, module_ids)),
                    })
                    .collect(),
            ),
            ExprKind::Case { expr, arms } => ExprKind::Case {
                expr: Box::new(go_expr(*expr, env, module_ids)),
                arms: arms
                    .into_iter()
                    .map(|a| ast::CaseArm {
                        pat: go_pat(a.pat, env, module_ids),
                        guard: a.guard.map(|g| go_expr(g, env, module_ids)),
                        body: go_expr(a.body, env, module_ids),
                    })
                    .collect(),
            },
            ExprKind::Cons { head, tail } => ExprKind::Cons {
                head: Box::new(go_expr(*head, env, module_ids)),
                tail: Box::new(go_expr(*tail, env, module_ids)),
            },
            ExprKind::List(es) => ExprKind::List(
                es.into_iter()
                    .map(|x| go_expr(x, env, module_ids))
                    .collect(),
            ),
            ExprKind::Tuple(es) => ExprKind::Tuple(
                es.into_iter()
                    .map(|x| go_expr(x, env, module_ids))
                    .collect(),
            ),
            ExprKind::Record(fs) => ExprKind::Record(
                fs.into_iter()
                    .map(|(l, x)| (l, go_expr(x, env, module_ids)))
                    .collect(),
            ),
            other => other,
        };
        ast::Expr::new(span, kind)
    }

    fn go_pat(p: ast::Pattern, env: &QualEnv, module_ids: &mut ModuleIdInterner) -> ast::Pattern {
        use ast::PatternKind;
        let span = p.span;
        let kind = match p.kind {
            PatternKind::Tuple(ps) => {
                PatternKind::Tuple(ps.into_iter().map(|x| go_pat(x, env, module_ids)).collect())
            }
            PatternKind::List(ps) => {
                PatternKind::List(ps.into_iter().map(|x| go_pat(x, env, module_ids)).collect())
            }
            PatternKind::Record(fs) => PatternKind::Record(
                fs.into_iter()
                    .map(|(l, x)| (l, go_pat(x, env, module_ids)))
                    .collect(),
            ),
            PatternKind::RecordLoose(fs, rest) => PatternKind::RecordLoose(
                fs.into_iter()
                    .map(|(l, x)| (l, go_pat(x, env, module_ids)))
                    .collect(),
                rest,
            ),
            PatternKind::Cons(a, b) => PatternKind::Cons(
                Box::new(go_pat(*a, env, module_ids)),
                Box::new(go_pat(*b, env, module_ids)),
            ),
            PatternKind::Or(a, b) => PatternKind::Or(
                Box::new(go_pat(*a, env, module_ids)),
                Box::new(go_pat(*b, env, module_ids)),
            ),
            PatternKind::View(pat, e) => PatternKind::View(
                Box::new(go_pat(*pat, env, module_ids)),
                Box::new(go_expr(*e, env, module_ids)),
            ),
            PatternKind::Constructor { name, args } => PatternKind::Constructor {
                name: resolve_name(name, env, module_ids),
                args: args
                    .into_iter()
                    .map(|x| go_pat(x, env, module_ids))
                    .collect(),
            },
            PatternKind::Literal(e) => PatternKind::Literal(go_expr(e, env, module_ids)),
            other => other,
        };
        ast::Pattern::new(span, kind)
    }

    let env = module_qual_env(module);

    module.items = module
        .items
        .clone()
        .into_iter()
        .map(|it| match it {
            ast::Item::Binding(b) => ast::Item::Binding(ast::Binding {
                doc: b.doc,
                pat: go_pat(b.pat, &env, module_ids),
                expr: go_expr(b.expr, &env, module_ids),
                span: b.span,
            }),
            ast::Item::ClassDecl(mut c) => {
                c.default_methods = c
                    .default_methods
                    .into_iter()
                    .map(|b| ast::Binding {
                        doc: b.doc,
                        pat: go_pat(b.pat, &env, module_ids),
                        expr: go_expr(b.expr, &env, module_ids),
                        span: b.span,
                    })
                    .collect();
                ast::Item::ClassDecl(c)
            }
            ast::Item::InstanceDecl(mut inst) => {
                inst.methods = inst
                    .methods
                    .into_iter()
                    .map(|b| ast::Binding {
                        doc: b.doc,
                        pat: go_pat(b.pat, &env, module_ids),
                        expr: go_expr(b.expr, &env, module_ids),
                        span: b.span,
                    })
                    .collect();
                ast::Item::InstanceDecl(inst)
            }
            other => other,
        })
        .collect();

    Ok(())
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
                    inst.class = desugar_qualified_class_id(&inst.class, &env)?;
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
    #[allow(dead_code)]
    fn debug_print_import(&self, enabled: bool, id: &ast::ImportDecl) {
        if !enabled {
            return;
        }
        eprintln!(
            "[KSCR_DEBUG_IMPORTS] saw import: module={} qualified={} as={:?}",
            id.module, id.qualified, id.as_name
        );
    }

    #[allow(dead_code)]
    fn resolve_import_path(&self, dir: &Path, module: &str) -> Result<std::path::PathBuf> {
        let rel = module.replace('.', "/");
        let local = dir.join(format!("{}.ks", rel));
        let stdlib_root = stdlib_cache::stdlib_root()?;
        let stdlib = stdlib_root.join(format!("{}.ks", rel));

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
        if let Some(mut m) = stdlib_cache::load_ast_stdlib_cached(path)? {
            // stdlib_cache does not have access to ModuleIdInterner; resolve module IDs here.

            // Populate def_module for ClassDecls if not already set.
            if let Some(module_name) = &m.name {
                for it in &mut m.items {
                    if let ast::Item::ClassDecl(c) = it {
                        if c.def_module.is_none() {
                            c.def_module = Some(module_name.clone());
                        }
                    }
                }
            }

            let env = module_qual_env(&m);
            // NOTE: keep ClassId.module as dummy ModuleId(0).
            // ClassEnv keying is currently based on the canonical (qualified) class name string,
            // and resolving module IDs here causes mismatches (e.g. stdlib instance processing).
            // resolve_class_names_to_module_ids(&mut m, &env, &mut self.module_ids);
            let _ = env;
            resolve_ctor_names_to_module_ids(&mut m, &mut self.module_ids)?;

            self.cache.insert(path.to_path_buf(), m.clone());

            // Track module names to detect collisions
            if let Some(module_name) = &m.name {
                self.module_name_to_paths
                    .entry(module_name.clone())
                    .or_default()
                    .push(path.to_path_buf());

                // Check for collision after adding this path
                let paths = &self.module_name_to_paths[module_name];
                if paths.len() > 1 {
                    let mut msg = format!("module '{}' is defined in multiple files:", module_name);
                    for p in paths {
                        msg.push_str(&format!("\n  - {}", p.display()));
                    }
                    return Err(Error::msg(msg));
                }
            }

            return Ok(m);
        }
        let src = std::fs::read_to_string(path)?;
        self.sources.insert(path.to_path_buf(), src.clone());
        let mut m = parser::parse_module(&src)?;
        desugar_module_qualified_names(&mut m)?;

        // Populate def_module for all ClassDecls with the module's name.
        if let Some(module_name) = &m.name {
            for it in &mut m.items {
                if let ast::Item::ClassDecl(c) = it {
                    c.def_module = Some(module_name.clone());
                }
            }
        }

        let env = module_qual_env(&m);
        // NOTE: keep ClassId.module as dummy ModuleId(0). See comment above.
        // resolve_class_names_to_module_ids(&mut m, &env, &mut self.module_ids);
        let _ = env;
        resolve_ctor_names_to_module_ids(&mut m, &mut self.module_ids)?;

        // Record definition sites for later diagnostics.
        // These spans are file-local offsets; path is canonical.
        for it in &m.items {
            match it {
                ast::Item::DataDecl(dd) => {
                    // Only index qualified type constructor names (`Module.T`).
                    if dd.name.contains('.') {
                        self.def_sites.type_ctor.insert(
                            dd.name.clone(),
                            DefSite {
                                path: path.to_path_buf(),
                                span: dd.span,
                            },
                        );
                    }
                    // Value constructors may also be qualified (`Module.C`).
                    for ctor in &dd.ctors {
                        if ctor.name.contains('.') {
                            self.def_sites.value_ctor.insert(
                                ctor.name.clone(),
                                DefSite {
                                    path: path.to_path_buf(),
                                    span: ctor.span,
                                },
                            );
                        }
                    }
                }
                ast::Item::TypeAlias(ta) => {
                    // Type aliases can act like “type-level forwarders” across modules.
                    // Index only qualified names to avoid ambiguity.
                    if ta.name.contains('.') {
                        self.def_sites.type_alias.insert(
                            ta.name.clone(),
                            DefSite {
                                path: path.to_path_buf(),
                                span: ta.span,
                            },
                        );
                    }
                }
                _ => {}
            }
        }
        self.cache.insert(path.to_path_buf(), m.clone());

        // Track module names to detect collisions
        if let Some(module_name) = &m.name {
            self.module_name_to_paths
                .entry(module_name.clone())
                .or_default()
                .push(path.to_path_buf());

            // Check for collision after adding this path
            let paths = &self.module_name_to_paths[module_name];
            if paths.len() > 1 {
                let mut msg = format!("module '{}' is defined in multiple files:", module_name);
                for p in paths {
                    msg.push_str(&format!("\n  - {}", p.display()));
                }
                return Err(Error::msg(msg));
            }
        }

        Ok(m)
    }

    #[allow(dead_code)]
    fn validate_import_cyclic(&self, p: &Path) -> Result<()> {
        if let Some(pos) = self.stack.iter().position(|x| x == p) {
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
        Ok(())
    }

    #[allow(dead_code)]
    fn validate_imported_module(
        &self,
        imported: &ast::Module,
        import_decl: &ast::ImportDecl,
    ) -> Result<()> {
        let Some(name) = &imported.name else {
            return Err(Error::msg(format!(
                "imported module {} must have a module header",
                import_decl.module
            )));
        };
        if name != &import_decl.module {
            return Err(Error::msg(format!(
                "module name mismatch: import {} but file declares module {}",
                import_decl.module, name
            )));
        }
        Ok(())
    }

    #[allow(dead_code)]
    fn debug_print_exports(
        &self,
        debug_imports: bool,
        id: &ast::ImportDecl,
        exports: &HashSet<String>,
    ) {
        if !debug_imports || (id.module != "Prelude" && id.module != "Prelude.List") {
            return;
        }
        let mut xs: Vec<&str> = exports.iter().map(|s| s.as_str()).collect();
        xs.sort_unstable();
        let has_from_to = exports.contains("enumFromTo");
        let has_from_then_to = exports.contains("enumFromThenTo");
        eprintln!(
            "[KSCR_DEBUG_IMPORTS] exports for module {}: count={} enumFromTo={} enumFromThenTo={}",
            id.module,
            exports.len(),
            has_from_to,
            has_from_then_to
        );
        if !has_from_to || !has_from_then_to {
            eprintln!(
                "[KSCR_DEBUG_IMPORTS]   first exports: {:?}",
                xs.into_iter().take(40).collect::<Vec<_>>()
            );
        }
    }

    #[allow(dead_code)]
    fn emit_qualified_imports(
        &mut self,
        p: &Path,
        id: &ast::ImportDecl,
        imported: &ast::Module,
        exports: &HashSet<String>,
        local_emitted_qualified: &mut HashSet<ImportQualKey>,
        out: &mut Vec<ast::Item>,
    ) -> Result<()> {
        let debug_imports = std::env::var("KSCR_DEBUG_IMPORTS").ok().is_some();
        let primary_qual = id.as_name.as_deref().unwrap_or(&id.module);
        let canonical_qual = id.module.as_str();

        let quals: Vec<&str> = if id.qualified {
            vec![primary_qual]
        } else if id.as_name.is_some() {
            vec![canonical_qual, primary_qual]
        } else {
            vec![canonical_qual]
        };

        for qual in quals {
            let key = ImportQualKey {
                path: p.to_path_buf(),
                qual: qual.to_string(),
            };
            if local_emitted_qualified.insert(key.clone()) && self.emitted_qualified.insert(key) {
                let items = import_qualified_items_for_decl(imported, qual, exports)?;
                if debug_imports {
                    self.debug_qualified_items(&items, &id.module, qual);
                }
                out.extend(items);
            }
        }
        Ok(())
    }

    #[allow(dead_code)]
    fn debug_qualified_items(&self, items: &[ast::Item], module: &str, qual: &str) {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for it in items {
            let mut names = HashSet::new();
            item_defined_names(it, &mut names);
            for n in names {
                *counts.entry(n).or_insert(0) += 1;
            }
        }
        if let Some(k) = counts.get("L.null") {
            eprintln!(
                "[KSCR_DEBUG_IMPORTS] import_qualified_items_for_decl produced L.null x{k} for module {module} as {qual}"
            );
            for it in items {
                let mut names = HashSet::new();
                item_defined_names(it, &mut names);
                if names.contains("L.null") {
                    eprintln!("[KSCR_DEBUG_IMPORTS]   L.null item: {it:?}");
                }
            }
        }
    }

    #[allow(dead_code)]
    fn collect_imports(
        &mut self,
        module: &ast::Module,
        dir: &Path,
        out: &mut Vec<ast::Item>,
    ) -> Result<()> {
        let debug_imports = std::env::var("KSCR_DEBUG_IMPORTS").ok().is_some();
        let mut local_emitted_qualified: HashSet<ImportQualKey> = HashSet::new();

        for it in &module.items {
            let ast::Item::Import(id) = it else {
                continue;
            };

            self.debug_print_import(debug_imports, id);

            let p = self.resolve_import_path(dir, &id.module)?;
            self.validate_import_cyclic(&p)?;

            let imported = self.load_ast(&p)?;
            self.validate_imported_module(&imported, id)?;

            self.stack.push(p.clone());
            let imported_dir = p.parent().unwrap_or(dir);
            self.collect_imports(&imported, imported_dir, out)?;
            self.stack.pop();

            let exports = module_exported_names(&imported)?;
            let export_names = exports.as_name_set();
            self.debug_print_exports(debug_imports, id, &export_names);

            self.emit_qualified_imports(
                &p,
                id,
                &imported,
                &export_names,
                &mut local_emitted_qualified,
                out,
            )?;

            // Emit unqualified forwarders for non-qualified imports
            if !id.qualified {
                let primary_qual = id.as_name.as_deref().unwrap_or(&id.module);
                let key = ImportQualKey {
                    path: p.clone(),
                    qual: primary_qual.to_string(),
                };
                if self.emitted_unqualified.insert(key) {
                    let fwd = import_unqualified_forwarders(
                        &imported,
                        primary_qual,
                        &export_names,
                        &id.import_spec,
                    )?;
                    if debug_imports && id.module == "Prelude" {
                        let mut defined_names = HashSet::new();
                        for it in &fwd {
                            item_defined_names(it, &mut defined_names);
                        }
                        eprintln!(
                            "[KSCR_DEBUG_IMPORTS] unqualified forwarders for Prelude emitted: enumFromTo={} enumFromThenTo={}",
                            defined_names.contains("enumFromTo"),
                            defined_names.contains("enumFromThenTo")
                        );
                        eprintln!(
                            "[KSCR_DEBUG_IMPORTS]   Prelude forwarder qual used: {}",
                            primary_qual
                        );
                    }
                    out.extend(fwd);
                }
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

/// Extract constructors from a type alias that re-exports a qualified type.
/// For example, if `type Maybe a = Prelude.Maybe a`, return ["Just", "Nothing"]
/// by looking up the imported Prelude.Maybe type.
fn extract_aliased_type_ctors(module: &ast::Module, ta: &ast::TypeAlias) -> Option<Vec<String>> {
    let debug = std::env::var("KSCR_DEBUG_IMPORTS").ok().is_some();

    // Extract the target type constructor from the RHS
    // This is a simple heuristic: if the RHS is an App or Var with a qualified name,
    // we try to resolve it
    let target_ty_name = match &ta.ty {
        ast::Type::Var(name) => Some(name.clone()),
        ast::Type::App { head, .. } => {
            // For applications like `Prelude.Maybe a`, extract the base constructor
            match &**head {
                ast::Type::Var(name) => Some(name.clone()),
                _ => None,
            }
        }
        _ => None,
    }?;

    if debug {
        eprintln!(
            "[KSCR_DEBUG_IMPORTS] extract_aliased_type_ctors: type alias {} -> target type {}",
            ta.name, target_ty_name
        );
    }

    // If the target type name is qualified (e.g., "Prelude.Maybe"), look it up in imports
    if let Some((module_part, type_name)) = target_ty_name.rsplit_once('.') {
        if debug {
            eprintln!(
                "[KSCR_DEBUG_IMPORTS] Qualified type: module={}, type={}",
                module_part, type_name
            );
        }
        // Find the import for this module
        for it in &module.items {
            let ast::Item::Import(id) = it else {
                continue;
            };
            if id.module != module_part {
                continue;
            }

            // Try to load the imported module and get its data declaration
            if let Ok(Some((imported, _))) = module_imported_exports(module, id) {
                // Look for the data declaration in the imported module
                for imp_it in &imported.items {
                    if let ast::Item::DataDecl(dd) = imp_it {
                        if dd.name == type_name {
                            let ctors: Vec<String> =
                                dd.ctors.iter().map(|c| c.name.clone()).collect();
                            if debug {
                                eprintln!("[KSCR_DEBUG_IMPORTS] Found data decl {} with constructors: {:?}", dd.name, ctors);
                            }
                            return Some(ctors);
                        }
                    }
                }
            }
        }
    } else {
        // Unqualified name - look for it in the current module
        for it in &module.items {
            if let ast::Item::DataDecl(dd) = it {
                if dd.name == target_ty_name {
                    return Some(dd.ctors.iter().map(|c| c.name.clone()).collect());
                }
            }
        }
    }

    if debug {
        eprintln!(
            "[KSCR_DEBUG_IMPORTS] No constructors found for type alias {}",
            ta.name
        );
    }
    None
}

/// Process a list of export specs and populate an export table.
/// This helper extracts the common logic used by both module header export specs
/// and legacy export declarations.
fn process_export_specs(
    specs: &[ast::ExportSpec],
    module: &ast::Module,
    exports: &mut ExportTable,
) -> Result<()> {
    for spec in specs {
        match spec {
            ast::ExportSpec::Name(n) => {
                // For now, classify unqualified names as values.
                // Later stages can refine this by looking up the actual item.
                let kind = classify_exported_name(module, n);
                exports.insert(n.clone(), kind);
            }
            ast::ExportSpec::Type { name, ctors } => {
                // Classify the type name (could be Type or Class)
                let kind = classify_type_export(module, name);
                exports.insert(name.clone(), kind);

                // Try to find a data declaration with this name
                let dd = module.items.iter().find_map(|it| match it {
                    ast::Item::DataDecl(d) if d.name == *name => Some(d),
                    _ => None,
                });

                if let Some(dd) = dd {
                    // Direct data declaration - export its constructors
                    match ctors {
                        ast::ExportCtors::All => {
                            exports.extend(
                                dd.ctors.iter().map(|c| (c.name.clone(), SymbolKind::Ctor)),
                            );
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
                                exports.insert(c.clone(), SymbolKind::Ctor);
                            }
                        }
                    }
                    continue;
                }

                // Check if it's a type alias that re-exports constructors
                let ta = module.items.iter().find_map(|it| match it {
                    ast::Item::TypeAlias(t) if t.name == *name => Some(t),
                    _ => None,
                });

                if let Some(ta) = ta {
                    // Type alias - try to resolve the constructors from the aliased type
                    // This handles cases like: type Maybe a = Prelude.Maybe a
                    // where we want to re-export Just and Nothing
                    if let ast::ExportCtors::All = ctors {
                        // Try to extract the target type name from the alias RHS
                        if let Some(target_ctors) = extract_aliased_type_ctors(module, ta) {
                            exports.extend(target_ctors.into_iter().map(|c| (c, SymbolKind::Ctor)));
                        }
                    }
                    continue;
                }

                // Type exports can refer to classes too (e.g. `Monad(..)`), which are not
                // `DataDecl`s or TypeAliases. For MVP export checking, allow these through.
            }
        }
    }
    Ok(())
}

fn module_exported_names(module: &ast::Module) -> Result<ExportTable> {
    let mut exports = ExportTable::new();

    // Priority 1: Check module header export_specs (module Foo (x, y) where ...)
    if let Some(specs) = &module.export_specs {
        process_export_specs(specs, module, &mut exports)?;
        return Ok(exports);
    }

    // Priority 2: Check legacy export declarations (deprecated but supported for backward compatibility)
    let mut has_export_decl = false;

    for it in &module.items {
        let ast::Item::Export(ed) = it else {
            continue;
        };
        has_export_decl = true;
        process_export_specs(&ed.specs, module, &mut exports)?;
    }

    if !has_export_decl {
        // No explicit export list: export everything
        for it in &module.items {
            match it {
                ast::Item::Binding(b) => {
                    let mut names = HashSet::new();
                    pat_defined_names(&b.pat, &mut names);
                    exports.extend(names.into_iter().map(|n| (n, SymbolKind::Value)));
                }
                ast::Item::TypeAlias(ta) => {
                    exports.insert(ta.name.clone(), SymbolKind::TypeAlias);
                }
                ast::Item::DataDecl(d) => {
                    exports.insert(d.name.clone(), SymbolKind::Type);
                    exports.extend(d.ctors.iter().map(|c| (c.name.clone(), SymbolKind::Ctor)));
                }
                ast::Item::Import(_)
                | ast::Item::Export(_)
                | ast::Item::Fixity(_)
                | ast::Item::ClassDecl(_)
                | ast::Item::InstanceDecl(_) => {}
            }
        }
        return Ok(exports);
    }

    Ok(exports)
}

/// Classify an exported name by looking it up in the module items.
fn classify_exported_name(module: &ast::Module, name: &str) -> SymbolKind {
    for it in &module.items {
        match it {
            ast::Item::Binding(b) => {
                if pattern_defines_name(&b.pat, name) {
                    return SymbolKind::Value;
                }
            }
            ast::Item::TypeAlias(ta) if ta.name == name => {
                return SymbolKind::TypeAlias;
            }
            ast::Item::DataDecl(d) => {
                if d.name == name {
                    return SymbolKind::Type;
                }
                if d.ctors.iter().any(|c| c.name == name) {
                    return SymbolKind::Ctor;
                }
            }
            ast::Item::ClassDecl(c) if c.name == name => {
                return SymbolKind::Class;
            }
            _ => {}
        }
    }
    // Default to Value if we can't find it (shouldn't happen in well-formed modules)
    SymbolKind::Value
}

/// Classify a type export (Type vs Class).
fn classify_type_export(module: &ast::Module, name: &str) -> SymbolKind {
    for it in &module.items {
        match it {
            ast::Item::DataDecl(d) if d.name == name => return SymbolKind::Type,
            ast::Item::ClassDecl(c) if c.name == name => return SymbolKind::Class,
            _ => {}
        }
    }
    // Default to Type if we can't find it
    SymbolKind::Type
}

/// Check if a pattern defines the given name.
fn pattern_defines_name(pat: &ast::Pattern, name: &str) -> bool {
    match &pat.kind {
        ast::PatternKind::Var(n) => n == name,
        ast::PatternKind::Wildcard | ast::PatternKind::Literal(_) | ast::PatternKind::Hole(_) => {
            false
        }
        ast::PatternKind::Constructor { args, .. } => {
            args.iter().any(|p| pattern_defines_name(p, name))
        }
        ast::PatternKind::Tuple(pats) => pats.iter().any(|p| pattern_defines_name(p, name)),
        ast::PatternKind::List(pats) => pats.iter().any(|p| pattern_defines_name(p, name)),
        ast::PatternKind::Record(fields) => {
            fields.iter().any(|(_, p)| pattern_defines_name(p, name))
        }
        ast::PatternKind::RecordLoose(fields, rest) => {
            fields.iter().any(|(_, p)| pattern_defines_name(p, name))
                || rest.as_ref().is_some_and(|r| r == name)
        }
        ast::PatternKind::Cons(p1, p2) => {
            pattern_defines_name(p1, name) || pattern_defines_name(p2, name)
        }
        ast::PatternKind::Or(p1, p2) => {
            pattern_defines_name(p1, name) || pattern_defines_name(p2, name)
        }
        ast::PatternKind::As(as_name, inner) => {
            as_name == name || pattern_defines_name(inner, name)
        }
        ast::PatternKind::View(inner, _) => pattern_defines_name(inner, name),
    }
}

#[allow(dead_code)]
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
    //
    // If the same module is imported multiple times under different qualifiers (e.g.
    // `import Prelude` and `import qualified Prelude as P`), we must not duplicate class/instance
    // decls; they are global and would trip `desugar_typeclasses` with "duplicate class".
    if qual == module.name.as_deref().unwrap_or("") {
        out.extend(qualify_class_instance_decls(module, qual, exports)?);
    }
    Ok(out)
}

#[allow(dead_code)]
fn merge_build_rename_map(from: &[String], to: &[String]) -> HashMap<String, String> {
    let mut map: HashMap<String, String> = HashMap::new();
    for (f, t) in from.iter().zip(to.iter()) {
        if f != t {
            map.insert(f.clone(), t.clone());
        }
    }
    map
}

#[allow(dead_code)]
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
        PatternKind::Tuple(ps) => {
            PatternKind::Tuple(ps.iter().map(|p| merge_subst_pat(p, map)).collect())
        }
        PatternKind::List(ps) => {
            PatternKind::List(ps.iter().map(|p| merge_subst_pat(p, map)).collect())
        }
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

#[allow(dead_code)]
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
        ExprKind::Tuple(es) => {
            ExprKind::Tuple(es.iter().map(|e| merge_subst_expr(e, map)).collect())
        }
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

#[allow(dead_code)]
fn merge_substitute_vars_in_expr(expr: &ast::Expr, from: &[String], to: &[String]) -> ast::Expr {
    let map = merge_build_rename_map(from, to);
    if map.is_empty() {
        return expr.clone();
    }
    merge_subst_expr(expr, &map)
}

#[allow(dead_code)]
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
            func: Box::new(Expr::new(
                expr.span,
                ExprKind::Lambda { params: rest, body },
            )),
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

#[allow(dead_code)]
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

#[allow(dead_code)]
fn merge_binding_name(b: &ast::Binding) -> Option<&str> {
    match &b.pat.kind {
        ast::PatternKind::Var(n) => Some(n.as_str()),
        _ => None,
    }
}

#[allow(dead_code)]
fn merge_rewrite_tuple_scrutinee_arms_to_cons(b_scrut: &ast::Expr, b_arms: &mut [ast::CaseArm]) {
    // If RHS was a tuple-scrutinee clause, collapsing to unary means we must
    // also rewrite its patterns to match on the single arg directly.
    if let ast::ExprKind::Tuple(_) = &b_scrut.kind {
        for arm in b_arms {
            if let ast::PatternKind::Tuple(ps) = &arm.pat.kind {
                if ps.len() == 3 {
                    // Expect (_, Constructor(:), _) shape.
                    if let ast::PatternKind::Constructor { name: ctor, args } = &ps[1].kind {
                        if ctor.is_unresolved_eq(":") && args.is_empty() {
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

#[allow(dead_code)]
fn merge_is_collapse_needed(expr: &ast::Expr) -> bool {
    match &expr.kind {
        ast::ExprKind::Lambda { params, .. } => params.len() > 1,
        _ => false,
    }
}

#[allow(dead_code)]
fn merge_rebuild_lambda_case(
    span: ast::Span,
    params: Vec<String>,
    scrut: ast::Expr,
    arms: Vec<ast::CaseArm>,
) -> ast::Expr {
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

#[allow(dead_code)]
fn merge_try_merge_lambda_clauses(
    prev: &mut ast::Binding,
    b: &ast::Binding,
    name: &str,
) -> Result<bool> {
    let (
        ast::ExprKind::Lambda {
            params: p_params,
            body: p_body,
        },
        ast::ExprKind::Lambda {
            params: b_params,
            body: b_body,
        },
    ) = (&prev.expr.kind, &b.expr.kind)
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

        let (
            ast::ExprKind::Lambda {
                params: p_params,
                body: p_body,
            },
            ast::ExprKind::Lambda {
                params: b_params,
                body: b_body,
            },
        ) = (&prev.expr.kind, &b_expr.kind)
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

#[allow(dead_code)]
fn merge_duplicate_bindings_for_names(
    items: Vec<ast::Item>,
    names: &[&str],
) -> Result<Vec<ast::Item>> {
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

/// Expand an ImportSpec into a set of names.
/// For filtering in infer_module, where we may not have imported module items.
fn expand_import_spec_to_names_simple(specs: &[ast::ExportSpec]) -> HashSet<String> {
    let mut names = HashSet::new();
    for spec in specs {
        match spec {
            ast::ExportSpec::Name(n) => {
                names.insert(n.clone());
            }
            ast::ExportSpec::Type { name, ctors } => {
                names.insert(name.clone());
                match ctors {
                    ast::ExportCtors::All => {
                        // Can't expand without module items, just include the type name
                        // The actual constructors will be handled by the type system
                    }
                    ast::ExportCtors::Some(ctor_list) => {
                        for ctor in ctor_list {
                            names.insert(ctor.clone());
                        }
                    }
                }
            }
        }
    }
    names
}

/// Expand an ImportSpec into a set of names with constructor resolution.
/// This loads the imported module source to expand Type(..) specs.
/// Falls back to expand_import_spec_to_names_simple if source is unavailable.
fn expand_import_spec_with_ctors(
    specs: &[ast::ExportSpec],
    module_name: &str,
    entry_dir: &Path,
) -> HashSet<String> {
    // Try to load the imported module source
    if let Ok(module_path) = resolve_module_path(entry_dir, module_name) {
        if let Ok(src) = std::fs::read_to_string(&module_path) {
            if let Ok(mut imported_mod) = parser::parse_module(&src) {
                let _ = desugar_module_qualified_names(&mut imported_mod);
                // Now we have module items, use the full expansion
                let mut names = HashSet::new();
                for spec in specs {
                    names.extend(expand_export_spec_to_names(spec, &imported_mod.items));
                }
                return names;
            }
        }
    }

    // Fallback: use simple expansion without constructor resolution
    expand_import_spec_to_names_simple(specs)
}

/// Expand an ImportSpec item (ExportSpec) into a set of names.
/// For `Name(n)`, returns {n}.
/// For `Type{name, ctors: All}`, returns {name} + all constructors of that type from module (best-effort).
/// For `Type{name, ctors: Some(list)}`, returns {name} + listed constructors.
fn expand_export_spec_to_names(
    spec: &ast::ExportSpec,
    module_items: &[ast::Item],
) -> HashSet<String> {
    let mut names = HashSet::new();
    match spec {
        ast::ExportSpec::Name(n) => {
            names.insert(n.clone());
        }
        ast::ExportSpec::Type { name, ctors } => {
            names.insert(name.clone());
            match ctors {
                ast::ExportCtors::All => {
                    // Find the DataDecl for this type and add all its constructors
                    for item in module_items {
                        if let ast::Item::DataDecl(dd) = item {
                            if &dd.name == name {
                                for ctor in &dd.ctors {
                                    names.insert(ctor.name.clone());
                                }
                                break;
                            }
                        }
                    }
                }
                ast::ExportCtors::Some(ctor_list) => {
                    for ctor in ctor_list {
                        names.insert(ctor.clone());
                    }
                }
            }
        }
    }
    names
}

/// Apply import spec filter to a set of exported names.
/// Returns the set of names that should be imported based on the import spec.
fn apply_import_spec_filter(
    exports: &HashSet<String>,
    import_spec: &Option<ast::ImportSpec>,
    module_items: &[ast::Item],
) -> HashSet<String> {
    match import_spec {
        None => exports.clone(), // No filter, import everything
        Some(ast::ImportSpec::Only(specs)) => {
            // Expand each ExportSpec to a set of names
            let mut allowed = HashSet::new();
            for spec in specs {
                allowed.extend(expand_export_spec_to_names(spec, module_items));
            }
            // Import only items that are both in exports and in the allowed set
            exports
                .iter()
                .filter(|n| allowed.contains(*n))
                .cloned()
                .collect()
        }
        Some(ast::ImportSpec::Hiding(specs)) => {
            // Expand each ExportSpec to a set of names to hide
            let mut hidden = HashSet::new();
            for spec in specs {
                hidden.extend(expand_export_spec_to_names(spec, module_items));
            }
            // Import everything except the hidden items
            exports
                .iter()
                .filter(|n| !hidden.contains(*n))
                .cloned()
                .collect()
        }
    }
}

fn import_unqualified_forwarders(
    module: &ast::Module,
    qual: &str,
    exports: &HashSet<String>,
    import_spec: &Option<ast::ImportSpec>,
) -> Result<Vec<ast::Item>> {
    // Bring unqualified exports as simple forwarders: `x = QUAL.x`.
    let mut out = Vec::new();

    let debug_imports = std::env::var("KSCR_DEBUG_IMPORTS").ok().as_deref() == Some("1");

    let mut values = HashSet::new();
    let mut type_aliases = HashMap::new();
    let mut data_decls: HashMap<String, ast::DataDecl> = HashMap::new();
    for it in import_items(module) {
        match it {
            ast::Item::Binding(b) => pat_defined_names(&b.pat, &mut values),
            ast::Item::TypeAlias(ta) => {
                type_aliases.insert(ta.name.clone(), ta);
            }
            ast::Item::DataDecl(d) => {
                values.extend(d.ctors.iter().map(|c| c.name.clone()));
                data_decls.insert(d.name.clone(), d);
            }
            ast::Item::Import(_)
            | ast::Item::Export(_)
            | ast::Item::Fixity(_)
            | ast::Item::ClassDecl(_)
            | ast::Item::InstanceDecl(_) => {}
        }
    }

    if debug_imports && module.name.as_deref() == Some("Prelude") {
        eprintln!(
            "[KSCR_DEBUG_IMPORTS] Prelude import_unqualified_forwarders: values_has_enumFromTo={} values_has_enumFromThenTo={}",
            values.contains("enumFromTo"),
            values.contains("enumFromThenTo")
        );
    }

    // Apply import spec filter to determine which names to import
    let filtered_exports = apply_import_spec_filter(exports, import_spec, &module.items);

    for n in filtered_exports.iter() {
        if values.contains(n) {
            out.push(ast::Item::Binding(ast::Binding {
                doc: None,
                pat: ast::Pattern::dummy(ast::PatternKind::Var(n.clone())),
                expr: ast::Expr::dummy(ast::ExprKind::Var(format!("{qual}.{n}"))),
                span: ast::dummy_span(),
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
                doc: None,
                name: ta.name.clone(),
                params: ta.params.clone(),
                ty,
                span: ast::dummy_span(),
            }));
        }

        // Data declarations also introduce a type constructor name that should be usable
        // unqualified in annotations (e.g. `Maybe a`). Provide an alias to the qualified name.
        if let Some(dd) = data_decls.get(n) {
            let head = ast::Type::Var(format!("{qual}.{}", dd.name));
            let ty = if dd.params.is_empty() {
                head
            } else {
                ast::Type::App {
                    head: Box::new(head),
                    args: dd.params.iter().cloned().map(ast::Type::Var).collect(),
                }
            };
            out.push(ast::Item::TypeAlias(ast::TypeAlias {
                doc: None,
                name: dd.name.clone(),
                params: dd.params.clone(),
                ty,
                span: ast::dummy_span(),
            }));
        }
    }

    Ok(out)
}

#[allow(dead_code)]
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

    let out: Vec<ast::Item> = module
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
        .collect::<Result<Vec<_>>>()?;

    Ok(out)
}

#[allow(dead_code)]
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
                // Also collect constructors from type alias re-exports
                // E.g., if we have `type Maybe a = Prelude.Maybe a` and export `Maybe(..)`
                if let Some(alias_ctors) = extract_aliased_type_ctors(module, ta) {
                    if std::env::var("KSCR_DEBUG_IMPORTS").ok().is_some() {
                        eprintln!(
                            "[KSCR_DEBUG_IMPORTS] Type alias {} re-exports constructors: {:?}",
                            ta.name, alias_ctors
                        );
                    }
                    ctors.extend(alias_ctors);
                }
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

    if std::env::var("KSCR_DEBUG_IMPORTS").ok().is_some() && qual == "P" {
        eprintln!(
            "[KSCR_DEBUG_IMPORTS] qualify_items qual={qual} ctor_map contains Nothing={} Just={}",
            ctor_map.contains_key("Nothing"),
            ctor_map.contains_key("Just")
        );
    }

    import_items(module)
        .into_iter()
        .map(|it| qualify_item(it, &val_map, &type_map, &ctor_map))
        .collect::<Result<Vec<_>>>()
}

#[allow(dead_code)]
fn qualify_item(
    it: ast::Item,
    val_map: &HashMap<String, String>,
    type_map: &HashMap<String, String>,
    ctor_map: &HashMap<String, String>,
) -> Result<ast::Item> {
    Ok(match it {
        ast::Item::Binding(b) => ast::Item::Binding(ast::Binding {
            doc: b.doc,
            pat: qualify_pat_binders(b.pat, val_map)?,
            expr: qualify_expr(b.expr, val_map, type_map, ctor_map)?,
            span: b.span,
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

#[allow(dead_code)]
fn qualify_ctor_if_imported(
    name: ast::ResolvedName,
    ctor_map: &HashMap<String, String>,
) -> ast::ResolvedName {
    // If a constructor name is unqualified (no dot) but it is exported by the imported
    // module, rewrite to the qualified ctor name. This is crucial for Haskell-like
    // `import qualified`, where unqualified names are not brought into scope.
    //
    // Keep `ResolvedName::Resolved` as-is so we don't lose `ModuleId` information gathered
    // during `load_ast`.
    match name {
        ast::ResolvedName::Unresolved(s) => {
            if !s.contains('.') {
                if let Some(q) = ctor_map.get(&s) {
                    if std::env::var("KSCR_DEBUG_IMPORTS").ok().is_some() {
                        eprintln!("[KSCR_DEBUG_IMPORTS] qualify_ctor_if_imported: {s} -> {q}");
                    }
                    return ast::ResolvedName::unresolved(q.clone());
                }
            }
            ast::ResolvedName::Unresolved(s)
        }
        ast::ResolvedName::Resolved { .. } => name,
    }
}

#[allow(dead_code)]
fn qualify_expr_boxed(
    expr: &ast::Expr,
    val_map: &HashMap<String, String>,
    type_map: &HashMap<String, String>,
    ctor_map: &HashMap<String, String>,
) -> Result<Box<ast::Expr>> {
    Ok(Box::new(qualify_expr(
        expr.clone(),
        val_map,
        type_map,
        ctor_map,
    )?))
}

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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
            let new_n = qualify_ctor_if_imported(n, ctor_map);
            Expr::new(span, ExprKind::Ctor(new_n))
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
        ExprKind::List(es) => Expr::new(
            span,
            ExprKind::List(qualify_expr_vec(es, val_map, type_map, ctor_map)?),
        ),
        ExprKind::Tuple(es) => Expr::new(
            span,
            ExprKind::Tuple(qualify_expr_vec(es, val_map, type_map, ctor_map)?),
        ),
        ExprKind::Record(fs) => Expr::new(
            span,
            ExprKind::Record(qualify_record_fields(fs, val_map, type_map, ctor_map)?),
        ),
        other => Expr::new(span, other),
    })
}

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
fn qualify_local_binding(
    b: ast::Binding,
    val_map: &HashMap<String, String>,
    type_map: &HashMap<String, String>,
    ctor_map: &HashMap<String, String>,
) -> Result<ast::Binding> {
    Ok(ast::Binding {
        doc: b.doc,
        pat: qualify_pat_nonbinders(b.pat, ctor_map, val_map, type_map)?,
        expr: qualify_expr(b.expr, val_map, type_map, ctor_map)?,
        span: b.span,
    })
}

#[allow(dead_code)]
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

#[allow(dead_code)]
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
                name: qualify_ctor_if_imported(name, ctor_map),
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

#[allow(dead_code)]
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
        Type::Var(n) => {
            // `ast::Type::Var` represents both type variables (lowercase) and type constructors
            // (uppercase). We only qualify constructors; qualifying type variables would break
            // polymorphism in signatures.
            if n.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                Type::Var(type_map.get(&n).cloned().unwrap_or(n))
            } else {
                Type::Var(n)
            }
        }
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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

fn infer_in_module_with_class_env(
    module: &ast::Module,
    class_env: &ClassEnv,
    inferred: &HashMap<String, Scheme>,
    expr: ast::Expr,
) -> Result<Ty> {
    let mut cx = InferCtx::default();
    let data_env = collect_data_env(module);
    let mut env = collect_ctor_env_with_class_env(&mut cx, module, class_env, None)?;
    // Add inferred binding types (module + imported forwarders). This is important for
    // inferring argument types during later desugaring passes.
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
    }
    let (s, cs, t) = infer_expr_in(&mut cx, &data_env, &Subst::new(), &env, expr)?;
    let _ = simplify_constraints(&data_env, class_env, apply_constraints(&s, cs))?;
    Ok(apply(&s, t))
}

// moved to `src/types/typeclass_dict_passing.rs`

struct RewriteClassMethodCallsCtx<'a> {
    module_snapshot: &'a ast::Module,
    class_env: &'a ClassEnv,
    inferred: &'a HashMap<String, Scheme>,
}

impl<'a> RewriteClassMethodCallsCtx<'a> {
    fn rewrite_expr(
        &self,
        dicts_in_scope: &HashSet<String>,
        known_dicts_in_scope: &HashMap<String, String>,
        expr: ast::Expr,
    ) -> Result<ast::Expr> {
        rewrite_expr(
            self.module_snapshot,
            self.class_env,
            self.inferred,
            dicts_in_scope,
            known_dicts_in_scope,
            expr,
        )
    }
}

fn class_is_higher_kinded(class_env: &ClassEnv, class_id: &ast::ClassId) -> bool {
    use ast::Type;

    fn has_app_head_var(ty: &Type, v: &str) -> bool {
        match ty {
            Type::App { head, args } => {
                matches!(head.as_ref(), Type::Var(name) if name == v)
                    || has_app_head_var(head, v)
                    || args.iter().any(|a| has_app_head_var(a, v))
            }
            Type::List(t) => has_app_head_var(t, v),
            Type::Tuple(ts) => ts.iter().any(|t| has_app_head_var(t, v)),
            Type::Record(fields) => fields.iter().any(|(_, t)| has_app_head_var(t, v)),
            Type::RecordOpen(fields, rest) => {
                fields.iter().any(|(_, t)| has_app_head_var(t, v)) || has_app_head_var(rest, v)
            }
            Type::Func(a, b) => has_app_head_var(a, v) || has_app_head_var(b, v),
            _ => false,
        }
    }

    let Some(param) = class_env.class_params.get(class_id) else {
        return false;
    };

    for ((cid, _), qt) in &class_env.methods {
        if cid == class_id && has_app_head_var(&qt.ty, param) {
            return true;
        }
    }
    false
}

fn instance_head_key_ty_for_class(
    class_env: &ClassEnv,
    class_id: &ast::ClassId,
    ty: &Ty,
) -> Result<String> {
    let ty = normalize_ty_for_instance_key(ty);
    // MVP: higher-kinded classes select instances by the type constructor head.
    // e.g. `Functor` instance is declared for `Maybe`, but call sites see `Maybe a`.
    if class_is_higher_kinded(class_env, class_id) {
        return Ok(match &ty {
            Ty::Con(name) => name.clone(),
            Ty::App { head, .. } => match head.as_ref() {
                Ty::Con(name) => name.clone(),
                _ => {
                    return Err(Error::msg(
                        "MVP: class constraints support only constructor/app instance heads",
                    ))
                }
            },
            _ => {
                return Err(Error::msg(
                    "MVP: class constraints support only constructor/app instance heads",
                ))
            }
        });
    }
    instance_head_key_ty(&ty)
}

struct ApplyRewriteCtx<'a> {
    module_snapshot: &'a ast::Module,
    class_env: &'a ClassEnv,
    inferred: &'a HashMap<String, Scheme>,
    dicts_in_scope: &'a HashSet<String>,
    known_dicts_in_scope: &'a HashMap<String, String>,
    span: ast::Span,
}

impl<'a> ApplyRewriteCtx<'a> {
    fn rewrite_expr(&self, expr: ast::Expr) -> Result<ast::Expr> {
        rewrite_expr(
            self.module_snapshot,
            self.class_env,
            self.inferred,
            self.dicts_in_scope,
            self.known_dicts_in_scope,
            expr,
        )
    }
}

fn find_super_path(class_env: &ClassEnv, from: &str, to: &str) -> Option<Vec<String>> {
    use std::collections::{HashMap, VecDeque};

    fn find_unique_class_id_by_name(class_env: &ClassEnv, name: &str) -> Option<ast::ClassId> {
        let mut found: Option<ast::ClassId> = None;
        for id in class_env.class_params.keys() {
            if id.name == name {
                if found.is_some() {
                    return None;
                }
                found = Some(id.clone());
            }
        }
        found
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

fn project_dict_along_path(span: ast::Span, mut base: ast::Expr, path: &[String]) -> ast::Expr {
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

fn rewrite_class_method_var(
    module_snapshot: &ast::Module,
    class_env: &ClassEnv,
    dicts_in_scope: &HashSet<String>,
    known_dicts_in_scope: &HashMap<String, String>,
    span: ast::Span,
    mname: String,
) -> Result<ast::Expr> {
    use ast::{Expr, ExprKind};

    // Check if this name is defined as a user function/value in the module.
    // If so, don't rewrite it as a typeclass method.
    // NOTE: We ignore injected import forwarders (dummy spans), because those are just
    // `x = M.x` re-exports and still need method rewriting.
    for item in &module_snapshot.items {
        if let ast::Item::Binding(b) = item {
            if let ast::PatternKind::Var(name) = &b.pat.kind {
                if name == &mname {
                    let injected_forwarder = b.span.start == 0
                        && b.span.end == 0
                        && b.expr.span.start == 0
                        && b.expr.span.end == 0;
                    if !injected_forwarder {
                        // This is a user-defined binding, not a typeclass method reference.
                        return Ok(Expr::new(span, ExprKind::Var(mname)));
                    }
                }
            }
        }
    }

    if let Some(classes) = class_env.method_classes.get(&mname) {
        if std::env::var("KSCR_DEBUG_METHOD_VALUES").ok().as_deref() == Some("1")
            && (mname == "enumFromTo" || mname == "enumFromThenTo")
        {
            eprintln!(
                    "[KSCR_DEBUG_METHOD_VALUES] rewrite_class_method_var hit: {mname} classes={classes:?} dicts_in_scope={:?} known_dicts={:?}",
                    dicts_in_scope,
                    known_dicts_in_scope
                );
        }
        let Some(class) = classes.first() else {
            return Err(Error::msg("internal: empty method class list"));
        };

        let dict_var = format!(
            "__dict_{}",
            class.name.rsplit('.').next().unwrap_or(&class.name)
        );

        let dict_expr: Option<ast::Expr> = if dicts_in_scope.contains(&dict_var) {
            Some(Expr::new(span, ExprKind::Var(dict_var.clone())))
        } else if let Some(d) = known_dicts_in_scope.get(&class.name) {
            Some(Expr::new(span, ExprKind::Var(d.clone())))
        } else {
            derive_dict_expr_from_candidates(
                span,
                class_env,
                &class.name,
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
            Expr::new(
                span,
                ExprKind::Apply {
                    func: Box::new(method_fn),
                    args: vec![dict_expr],
                },
            )
        };

        Ok(if let Some(dict_expr) = dict_expr {
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
        })
    } else {
        Ok(Expr::new(span, ExprKind::Var(mname)))
    }
}

fn rewrite_class_method_lambda(
    ctx: &ApplyRewriteCtx<'_>,
    params: Vec<String>,
    body: ast::Expr,
) -> Result<ast::Expr> {
    use ast::{Expr, ExprKind};

    let mut scope = ctx.dicts_in_scope.clone();
    for p in &params {
        if p.starts_with("__dict_") {
            scope.insert(p.clone());
        }
    }
    Ok(Expr::new(
        ctx.span,
        ExprKind::Lambda {
            params,
            body: Box::new(rewrite_expr(
                ctx.module_snapshot,
                ctx.class_env,
                ctx.inferred,
                &scope,
                ctx.known_dicts_in_scope,
                body,
            )?),
        },
    ))
}

/// Returns the name of the top-level binding that encloses `span`, if any.
/// Does NOT search nested `let`/`where`/lambda scopes.
fn find_enclosing_binding(module: &ast::Module, span: ast::Span) -> Option<String> {
    use ast::{Item, PatternKind};

    fn span_contains(outer: ast::Span, inner: ast::Span) -> bool {
        outer.start <= inner.start && inner.end <= outer.end
    }

    fn expr_contains_span(expr: &ast::Expr, target: ast::Span) -> bool {
        span_contains(expr.span, target)
    }

    for item in &module.items {
        if let Item::Binding(b) = item {
            if expr_contains_span(&b.expr, span) {
                if let PatternKind::Var(name) = &b.pat.kind {
                    return Some(name.clone());
                }
            }
        }
    }
    None
}

/// Extract the monad type from a function's return type.
/// Given a function type like `a -> b -> m c` and a polymorphic type like `m ()`,
/// try to unify them to determine what `m` should be.
fn extract_return_monad_type(func_ty: &Ty, poly_ty: &Ty) -> Option<Ty> {
    // Get the return type of the function (rightmost type in chain of ->)
    fn get_return_type(ty: &Ty) -> &Ty {
        match ty {
            Ty::Func(_, b) => get_return_type(b),
            _ => ty,
        }
    }

    // Simple unification: given poly_ty like `m ()` where m is a tyvar,
    // and ret_ty like `IO ()`, extract the IO part.
    fn extract_monad_constructor(poly: &Ty, concrete: &Ty) -> Option<Ty> {
        match (poly, concrete) {
            // Both are applications with same number of args
            (
                Ty::App {
                    head: poly_head,
                    args: poly_args,
                },
                Ty::App {
                    head: _conc_head,
                    args: conc_args,
                },
            ) if poly_args.len() == conc_args.len() => {
                // If poly head is a type variable, the concrete head is our monad constructor
                if matches!(poly_head.as_ref(), Ty::Var(_)) {
                    // Check if it's a single-arg application (typical for monads)
                    if poly_args.len() == 1 && conc_args.len() == 1 {
                        // Return the full concrete type (e.g., IO Unit)
                        return Some(concrete.clone());
                    }
                }
                None
            }
            _ => None,
        }
    }

    let ret_ty = get_return_type(func_ty);
    extract_monad_constructor(poly_ty, ret_ty)
}

/// Dictionary resolution order:
/// 1. Known dicts in scope (early exit if found)
/// 2. Argument-based selection (determined_by_args)
/// 3. Derive from candidates
/// 4. Enclosing binding fallback (for pattern vars)
/// 5. Error or defer
fn resolve_method_dict_expr(
    ctx: &ApplyRewriteCtx<'_>,
    class_id: &ast::ClassId,
    mname: &str,
    args: &[ast::Expr],
) -> Result<Option<(ast::Expr, Option<String>)>> {
    use ast::{Expr, ExprKind, Type};

    fn unqualified_class_name(class: &str) -> &str {
        class.rsplit('.').next().unwrap_or(class)
    }

    fn type_contains_var(ty: &Type, v: &str) -> bool {
        match ty {
            Type::Unit
            | Type::Integer
            | Type::Bool
            | Type::Float64
            | Type::Char
            | Type::String
            | Type::Hole(_) => false,
            Type::Var(name) => name == v,
            Type::List(t) => type_contains_var(t, v),
            Type::Tuple(ts) => ts.iter().any(|t| type_contains_var(t, v)),
            Type::Record(fields) => fields.iter().any(|(_, t)| type_contains_var(t, v)),
            Type::RecordOpen(fields, rest) => {
                fields.iter().any(|(_, t)| type_contains_var(t, v)) || type_contains_var(rest, v)
            }
            Type::App { head, args } => {
                type_contains_var(head, v) || args.iter().any(|t| type_contains_var(t, v))
            }
            Type::Func(a, b) => type_contains_var(a, v) || type_contains_var(b, v),
        }
    }

    fn method_class_param_determined_by_args(
        class_env: &ClassEnv,
        class_id: &ast::ClassId,
        mname: &str,
    ) -> bool {
        let Some(param) = class_env.class_params.get(class_id) else {
            return true;
        };
        let Some(qt) = class_env
            .methods
            .get(&(class_id.clone(), mname.to_string()))
        else {
            return true;
        };

        let mut cur = &qt.ty;
        while let Type::Func(a, b) = cur {
            if type_contains_var(a, param) {
                return true;
            }
            cur = b;
        }
        false
    }

    let class = class_id.name.as_str();

    // Prefer an in-scope dictionary param when available.
    let dict_var = format!("__dict_{class}");
    let dict_var_unqual = format!("__dict_{}", unqualified_class_name(class));
    if ctx.dicts_in_scope.contains(&dict_var) {
        return Ok(Some((
            Expr::new(ctx.span, ExprKind::Var(dict_var.clone())),
            Some(dict_var),
        )));
    }
    if ctx.dicts_in_scope.contains(&dict_var_unqual) {
        return Ok(Some((
            Expr::new(ctx.span, ExprKind::Var(dict_var_unqual.clone())),
            Some(dict_var_unqual),
        )));
    }

    // Prefer a dictionary already fixed by surrounding context.
    if let Some(d) = ctx
        .known_dicts_in_scope
        .get(class)
        .or_else(|| ctx.known_dicts_in_scope.get(unqualified_class_name(class)))
    {
        let chosen_name_for_known = Some(d.clone());
        return Ok(Some((
            Expr::new(ctx.span, ExprKind::Var(d.clone())),
            chosen_name_for_known,
        )));
    }

    let determined_by_args = method_class_param_determined_by_args(ctx.class_env, class_id, mname);

    // If the class parameter is not determined by argument types (e.g. `pure`, `return`),
    // try to use the inferred type of the whole application to pick an instance.
    if !determined_by_args {
        let app_expr = Expr::new(
            ctx.span,
            ExprKind::Apply {
                func: Box::new(Expr::new(ctx.span, ExprKind::Var(mname.to_string()))),
                args: args.to_vec(),
            },
        );
        if let Ok(app_ty) = infer_in_module_with_class_env(
            ctx.module_snapshot,
            ctx.class_env,
            ctx.inferred,
            app_expr,
        ) {
            if ftv_ty(&app_ty).is_empty() {
                if let Ok(head) = instance_head_key_ty_for_class(ctx.class_env, class_id, &app_ty) {
                    if let Some(d) = ctx.class_env.instances.get(&(class_id.clone(), head)) {
                        let chosen_name_for_known = Some(d.clone());
                        return Ok(Some((
                            Expr::new(ctx.span, ExprKind::Var(d.clone())),
                            chosen_name_for_known,
                        )));
                    }
                    return Err(Error::msg_with_span(
                        format!("no instance found for method call `{mname}`: {class} {app_ty}"),
                        ctx.span,
                    ));
                }
            } else {
                // If `app_ty` still has free variables, try to resolve them by looking at
                // the enclosing binding's inferred type. This helps when `return ()` appears
                // in one branch of a pattern match and the monad is determined by other branches.
                if let Some(binding_name) = find_enclosing_binding(ctx.module_snapshot, ctx.span) {
                    if let Some(scheme) = ctx.inferred.get(&binding_name) {
                        // Extract the return type from the function's scheme
                        if let Some(monad_ty) = extract_return_monad_type(&scheme.ty, &app_ty) {
                            if let Ok(head) =
                                instance_head_key_ty_for_class(ctx.class_env, class_id, &monad_ty)
                            {
                                if let Some(d) =
                                    ctx.class_env.instances.get(&(class_id.clone(), head))
                                {
                                    let chosen_name_for_known = Some(d.clone());
                                    return Ok(Some((
                                        Expr::new(ctx.span, ExprKind::Var(d.clone())),
                                        chosen_name_for_known,
                                    )));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let mut first_non_ground: Option<Ty> = None;
    let mut first_missing_instance: Option<Ty> = None;
    let mut dict_name: Option<String> = None;

    let hk_class = class_is_higher_kinded(ctx.class_env, class_id);

    if determined_by_args {
        for a in args {
            let Ok(a_ty) = infer_in_module_with_class_env(
                ctx.module_snapshot,
                ctx.class_env,
                ctx.inferred,
                a.clone(),
            ) else {
                continue;
            };

            // For first-order classes, instance selection needs a ground type key.
            if !hk_class && !ftv_ty(&a_ty).is_empty() {
                if first_non_ground.is_none() {
                    first_non_ground = Some(a_ty.clone());
                }
                continue;
            }

            let Ok(head) = instance_head_key_ty_for_class(ctx.class_env, class_id, &a_ty) else {
                continue;
            };

            let key = (class_id.clone(), head);
            if let Some(d) = ctx.class_env.instances.get(&key) {
                dict_name = Some(d.clone());
                break;
            }

            // Also try polymorphic instances when the argument type is ground.
            // We can only pick instances with no context dict args here.
            if ftv_ty(&a_ty).is_empty() {
                let a_ty_norm = normalize_ty_for_instance_key(&a_ty);
                let mut poly_candidates: Vec<&PolyInstance> = ctx
                    .class_env
                    .poly_instances
                    .iter()
                    .filter(|pi| pi.class == *class_id && pi.ctx_len == 0)
                    .filter(|pi| unify_instance_head(&pi.head_pat, &a_ty_norm).is_some())
                    .collect();

                if poly_candidates.len() == 1 {
                    let pi = poly_candidates.remove(0);
                    let dict_ref = if pi.dict_name.starts_with("Prelude.") {
                        pi.dict_name
                            .split('.')
                            .next_back()
                            .unwrap_or(pi.dict_name.as_str())
                            .to_string()
                    } else {
                        pi.dict_name.clone()
                    };
                    return Ok(Some((Expr::new(ctx.span, ExprKind::Var(dict_ref)), None)));
                }
                if poly_candidates.len() > 1 {
                    return Err(Error::msg_with_span(
                        format!(
                            "overlapping instances for `{}`: cannot choose for type {a_ty}",
                            class
                        ),
                        ctx.span,
                    ));
                }
            }

            if first_missing_instance.is_none() {
                first_missing_instance = Some(a_ty);
            }
        }
    }

    if let Some(dict_name) = dict_name {
        let chosen_name_for_known = Some(dict_name.clone());
        return Ok(Some((
            Expr::new(ctx.span, ExprKind::Var(dict_name)),
            chosen_name_for_known,
        )));
    }

    if let Some(d) = derive_dict_expr_from_candidates(
        ctx.span,
        ctx.class_env,
        class,
        ctx.dicts_in_scope,
        ctx.known_dicts_in_scope,
    ) {
        return Ok(Some((d, None)));
    }

    // Fallback: if argument types are not ground, try to pick an instance from the
    // enclosing binding's return type. This is important for `do`-desugared code
    // where `>>` / `>>=` may not have fully-ground argument types at this stage.
    if determined_by_args {
        if let Some(binding_name) = find_enclosing_binding(ctx.module_snapshot, ctx.span) {
            if let Some(scheme) = ctx.inferred.get(&binding_name) {
                fn return_ty(mut t: &Ty) -> &Ty {
                    while let Ty::Func(_, b) = t {
                        t = b;
                    }
                    t
                }

                let rt = return_ty(&scheme.ty);
                if ftv_ty(rt).is_empty() {
                    if let Ok(head) = instance_head_key_ty_for_class(ctx.class_env, class_id, rt) {
                        if let Some(d) = ctx.class_env.instances.get(&(class_id.clone(), head)) {
                            let chosen_name_for_known = Some(d.clone());
                            return Ok(Some((
                                Expr::new(ctx.span, ExprKind::Var(d.clone())),
                                chosen_name_for_known,
                            )));
                        }
                    }
                }
            }
        }
    }

    // If we saw only concrete (ground) argument types and still couldn't find an instance,
    // this is a real “missing instance” error.
    // If we saw any non-ground arg type, treat it as ambiguity and defer (non-strict mode).
    if determined_by_args && first_non_ground.is_none() {
        if let Some(ty) = first_missing_instance {
            return Err(Error::msg_with_span(
                format!("no instance found for method call `{mname}`: {class} {ty}"),
                ctx.span,
            ));
        }
    }

    // IMPORTANT: Dictionary choice failure means we don't have enough information at this
    // rewrite stage. Historically we kept such calls polymorphic by producing a dict-lambda.
    // This behaves like a fallback and can mask bugs, so allow opting into strict fail-fast.
    let failfast = std::env::var("KSCR_FAILFAST_METHOD_DICT").ok().as_deref() == Some("1");
    if failfast {
        eprintln!(
            "[KSCR_FAILFAST_METHOD_DICT] cannot choose dictionary for `{mname}` (class={class}) args_len={} span={:?}",
            args.len(),
            ctx.span
        );
        return Err(Error::msg_with_span(
            format!(
                "cannot choose dictionary for method call `{mname}`: {class} (insufficient information)"
            ),
            ctx.span,
        ));
    }

    // Non-strict mode: keep it polymorphic (caller will wrap in a dict-lambda).
    Ok(None)
}

fn build_method_call(
    ctx: &ApplyRewriteCtx<'_>,
    mname: &str,
    dict_expr: ast::Expr,
    new_args: Vec<ast::Expr>,
) -> ast::Expr {
    use ast::{Expr, ExprKind};

    let get = Expr::new(ctx.span, ExprKind::Var("__recordGet".to_string()));
    let method_fn = Expr::new(
        ctx.span,
        ExprKind::Apply {
            func: Box::new(get),
            args: vec![
                dict_expr.clone(),
                Expr::new(ctx.span, ExprKind::String(mname.to_string())),
            ],
        },
    );

    let mut call_args = Vec::with_capacity(1 + new_args.len());
    call_args.push(dict_expr);
    call_args.extend(new_args);

    Expr::new(
        ctx.span,
        ExprKind::Apply {
            func: Box::new(method_fn),
            args: call_args,
        },
    )
}

fn rewrite_args_with_known(
    ctx: &ApplyRewriteCtx<'_>,
    known_dicts_in_scope: &HashMap<String, String>,
    args: Vec<ast::Expr>,
) -> Result<Vec<ast::Expr>> {
    args.into_iter()
        .map(|a| {
            rewrite_expr(
                ctx.module_snapshot,
                ctx.class_env,
                ctx.inferred,
                ctx.dicts_in_scope,
                known_dicts_in_scope,
                a,
            )
        })
        .collect()
}

fn rewrite_class_method_apply(
    ctx: &ApplyRewriteCtx<'_>,
    func: ast::Expr,
    args: Vec<ast::Expr>,
) -> Result<ast::Expr> {
    use ast::{Expr, ExprKind};

    if let ExprKind::Var(mname) = &func.kind {
        // Check if this name is defined as a user function/value in the module.
        // If so, don't rewrite it as a typeclass method.
        let is_user_defined = ctx.module_snapshot.items.iter().any(|item| {
            if let ast::Item::Binding(b) = item {
                if let ast::PatternKind::Var(name) = &b.pat.kind {
                    if name != mname {
                        return false;
                    }
                    let injected_forwarder = b.span.start == 0
                        && b.span.end == 0
                        && b.expr.span.start == 0
                        && b.expr.span.end == 0;
                    return !injected_forwarder;
                }
            }
            false
        });

        if !is_user_defined {
            if let Some(classes) = ctx.class_env.method_classes.get(mname) {
                let Some(class) = classes.first() else {
                    return Err(Error::msg("internal: empty method class list"));
                };

                if let Some((dict_expr, chosen_name_for_known)) =
                    resolve_method_dict_expr(ctx, class, mname, &args)?
                {
                    let mut known = ctx.known_dicts_in_scope.clone();
                    if let Some(chosen) = chosen_name_for_known.clone() {
                        known.insert(class.name.clone(), chosen);
                    }

                    let new_args = rewrite_args_with_known(ctx, &known, args)?;
                    return Ok(build_method_call(ctx, mname, dict_expr, new_args));
                }

                // Polymorphic/ambiguous method application: keep it as a dictionary-taking function.
                // The runtime will auto-apply default dicts for specific classes (Num/Eq/Show) when needed.
                let dict_var = format!(
                    "__dict_{}",
                    class.name.rsplit('.').next().unwrap_or(&class.name)
                );
                let mut scope = ctx.dicts_in_scope.clone();
                scope.insert(dict_var.clone());

                let new_args: Vec<_> = args
                    .into_iter()
                    .map(|a| {
                        rewrite_expr(
                            ctx.module_snapshot,
                            ctx.class_env,
                            ctx.inferred,
                            &scope,
                            ctx.known_dicts_in_scope,
                            a,
                        )
                    })
                    .collect::<Result<Vec<_>>>()?;

                let dict_expr = Expr::new(ctx.span, ExprKind::Var(dict_var.clone()));
                let body = build_method_call(ctx, mname, dict_expr, new_args);
                return Ok(Expr::new(
                    ctx.span,
                    ExprKind::Lambda {
                        params: vec![dict_var],
                        body: Box::new(body),
                    },
                ));
            }
        }
    }

    let func = ctx.rewrite_expr(func)?;
    let args: Vec<_> = args
        .into_iter()
        .map(|a| ctx.rewrite_expr(a))
        .collect::<Result<Vec<_>>>()?;
    Ok(Expr::new(
        ctx.span,
        ExprKind::Apply {
            func: Box::new(func),
            args,
        },
    ))
}

fn rewrite_expr(
    module_snapshot: &ast::Module,
    class_env: &ClassEnv,
    inferred: &HashMap<String, Scheme>,
    dicts_in_scope: &HashSet<String>,
    known_dicts_in_scope: &HashMap<String, String>,
    expr: ast::Expr,
) -> Result<ast::Expr> {
    let span = expr.span;
    let rewrite_ctx = RewriteExprCtx::new(
        module_snapshot,
        class_env,
        inferred,
        dicts_in_scope,
        known_dicts_in_scope,
        span,
    );
    rewrite_expr_inner(&rewrite_ctx, expr, span)
}

struct RewriteExprCtx<'a> {
    apply_ctx: ApplyRewriteCtx<'a>,
}

impl<'a> RewriteExprCtx<'a> {
    fn new(
        module_snapshot: &'a ast::Module,
        class_env: &'a ClassEnv,
        inferred: &'a HashMap<String, Scheme>,
        dicts_in_scope: &'a HashSet<String>,
        known_dicts_in_scope: &'a HashMap<String, String>,
        span: ast::Span,
    ) -> Self {
        Self {
            apply_ctx: ApplyRewriteCtx {
                module_snapshot,
                class_env,
                inferred,
                dicts_in_scope,
                known_dicts_in_scope,
                span,
            },
        }
    }

    fn ctx(&self) -> &ApplyRewriteCtx<'a> {
        &self.apply_ctx
    }

    fn rewrite(&self, expr: ast::Expr) -> Result<ast::Expr> {
        rewrite_expr(
            self.apply_ctx.module_snapshot,
            self.apply_ctx.class_env,
            self.apply_ctx.inferred,
            self.apply_ctx.dicts_in_scope,
            self.apply_ctx.known_dicts_in_scope,
            expr,
        )
    }

    fn rewrite_bindings(&self, bindings: Vec<ast::Binding>) -> Result<Vec<ast::Binding>> {
        bindings
            .into_iter()
            .map(|b| {
                Ok(ast::Binding {
                    doc: b.doc,
                    pat: b.pat,
                    expr: self.rewrite(b.expr)?,
                    span: b.span,
                })
            })
            .collect::<Result<Vec<_>>>()
    }

    fn rewrite_do(&self, stmts: Vec<ast::DoStmt>) -> Result<Vec<ast::DoStmt>> {
        stmts
            .into_iter()
            .map(|s| {
                Ok(match s {
                    ast::DoStmt::Bind { pat, expr } => ast::DoStmt::Bind {
                        pat,
                        expr: self.rewrite(expr)?,
                    },
                    ast::DoStmt::Expr(e) => ast::DoStmt::Expr(self.rewrite(e)?),
                })
            })
            .collect::<Result<Vec<_>>>()
    }

    fn rewrite_case_arms(&self, arms: Vec<ast::CaseArm>) -> Result<Vec<ast::CaseArm>> {
        arms.into_iter()
            .map(|a| {
                Ok(ast::CaseArm {
                    pat: a.pat,
                    guard: a.guard.map(|g| self.rewrite(g)).transpose()?,
                    body: self.rewrite(a.body)?,
                })
            })
            .collect::<Result<Vec<_>>>()
    }

    fn rewrite_expr_list(&self, es: Vec<ast::Expr>) -> Result<Vec<ast::Expr>> {
        es.into_iter().map(|e| self.rewrite(e)).collect()
    }

    fn rewrite_record_fields(
        &self,
        fields: Vec<(String, ast::Expr)>,
    ) -> Result<Vec<(String, ast::Expr)>> {
        fields
            .into_iter()
            .map(|(k, v)| Ok((k, self.rewrite(v)?)))
            .collect::<Result<Vec<_>>>()
    }
}

fn rewrite_expr_inner(
    rewrite_ctx: &RewriteExprCtx<'_>,
    expr: ast::Expr,
    span: ast::Span,
) -> Result<ast::Expr> {
    use ast::{Expr, ExprKind};

    let apply_ctx = rewrite_ctx.ctx();
    Ok(match expr.kind {
        ExprKind::Var(mname) => rewrite_class_method_var(
            apply_ctx.module_snapshot,
            apply_ctx.class_env,
            apply_ctx.dicts_in_scope,
            apply_ctx.known_dicts_in_scope,
            span,
            mname,
        )?,
        ExprKind::Lambda { params, body } => rewrite_class_method_lambda(apply_ctx, params, *body)?,
        ExprKind::Apply { func, args } => rewrite_class_method_apply(apply_ctx, *func, args)?,
        ExprKind::Let { bindings, body } => Expr::new(
            span,
            ExprKind::Let {
                bindings: rewrite_ctx.rewrite_bindings(bindings)?,
                body: Box::new(rewrite_ctx.rewrite(*body)?),
            },
        ),
        ExprKind::Where { expr, bindings } => Expr::new(
            span,
            ExprKind::Where {
                expr: Box::new(rewrite_ctx.rewrite(*expr)?),
                bindings: rewrite_ctx.rewrite_bindings(bindings)?,
            },
        ),
        ExprKind::Annot { expr, ty } => Expr::new(
            span,
            ExprKind::Annot {
                expr: Box::new(rewrite_ctx.rewrite(*expr)?),
                ty,
            },
        ),
        ExprKind::Do(stmts) => Expr::new(span, ExprKind::Do(rewrite_ctx.rewrite_do(stmts)?)),
        ExprKind::Case { expr, arms } => Expr::new(
            span,
            ExprKind::Case {
                expr: Box::new(rewrite_ctx.rewrite(*expr)?),
                arms: rewrite_ctx.rewrite_case_arms(arms)?,
            },
        ),
        ExprKind::Cons { head, tail } => Expr::new(
            span,
            ExprKind::Cons {
                head: Box::new(rewrite_ctx.rewrite(*head)?),
                tail: Box::new(rewrite_ctx.rewrite(*tail)?),
            },
        ),
        ExprKind::List(es) => Expr::new(span, ExprKind::List(rewrite_ctx.rewrite_expr_list(es)?)),
        ExprKind::Tuple(es) => Expr::new(span, ExprKind::Tuple(rewrite_ctx.rewrite_expr_list(es)?)),
        ExprKind::Record(fields) => Expr::new(
            span,
            ExprKind::Record(rewrite_ctx.rewrite_record_fields(fields)?),
        ),
        other => Expr::new(span, other),
    })
}

fn rewrite_class_method_calls_in_module(
    module: &mut ast::Module,
    class_env: &ClassEnv,
    inferred: &HashMap<String, Scheme>,
) -> Result<()> {
    if std::env::var("KSCR_DEBUG_METHOD_VALUES").ok().as_deref() == Some("1") {
        let mut n = 0usize;
        for it in &module.items {
            let ast::Item::Binding(b) = it else {
                continue;
            };
            if expr_contains_var_any(&b.expr, &["enumFromTo", "enumFromThenTo"]) {
                n += 1;
            }
        }
        eprintln!(
            "[KSCR_DEBUG_METHOD_VALUES] pre-rewrite: bindings containing enumFromTo/enumFromThenTo = {n}"
        );
    }

    let snapshot = module.clone();
    let ctx = RewriteClassMethodCallsCtx {
        module_snapshot: &snapshot,
        class_env,
        inferred,
    };
    let empty_scope: HashSet<String> = HashSet::new();
    let empty_known: HashMap<String, String> = HashMap::new();
    module.items = module
        .items
        .drain(..)
        .map(|it| {
            Ok(match it {
                ast::Item::Binding(b) => ast::Item::Binding(ast::Binding {
                    doc: b.doc,
                    pat: b.pat,
                    expr: ctx.rewrite_expr(&empty_scope, &empty_known, b.expr)?,
                    span: b.span,
                }),
                other => other,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    if std::env::var("KSCR_DEBUG_METHOD_VALUES").ok().as_deref() == Some("1") {
        let mut n = 0usize;
        for it in &module.items {
            let ast::Item::Binding(b) = it else {
                continue;
            };
            if expr_contains_var_any(&b.expr, &["enumFromTo", "enumFromThenTo"]) {
                n += 1;
            }
        }
        eprintln!(
            "[KSCR_DEBUG_METHOD_VALUES] post-rewrite: bindings containing enumFromTo/enumFromThenTo = {n}"
        );
    }
    Ok(())
}

fn inject_class_method_value_bindings(
    module: &mut ast::Module,
    class_env: &ClassEnv,
    inferred: &HashMap<String, Scheme>,
) {
    let defined = collect_defined_names(module);
    let methods = collect_sorted_methods(class_env);
    let injected = create_method_bindings(methods, &defined, class_env);

    if std::env::var("KSCR_DEBUG_METHOD_VALUES").ok().as_deref() == Some("1") {
        debug_print_method_injection(class_env, inferred, &injected);
    }

    if !injected.is_empty() {
        // Prepend so later passes can treat them as ordinary globals.
        let mut all_items = injected;
        all_items.append(&mut module.items);
        module.items = all_items;
    }
}

fn collect_defined_names(module: &ast::Module) -> std::collections::HashSet<String> {
    let mut defined = std::collections::HashSet::new();
    for it in &module.items {
        let ast::Item::Binding(b) = it else {
            continue;
        };
        if let ast::PatternKind::Var(name) = &b.pat.kind {
            defined.insert(name.clone());
        }
    }
    defined
}

fn collect_sorted_methods(class_env: &ClassEnv) -> Vec<(String, String)> {
    let mut methods: Vec<(String, String)> = Vec::new();
    for (mname, classes) in &class_env.method_classes {
        let Some(class) = classes.first() else {
            continue;
        };
        methods.push((mname.clone(), class.name.clone()));
    }
    methods.sort_by(|(a, ca), (b, cb)| (a, ca).cmp(&(b, cb)));
    methods
}

fn create_method_bindings(
    methods: Vec<(String, String)>,
    defined: &std::collections::HashSet<String>,
    class_env: &ClassEnv,
) -> Vec<ast::Item> {
    use ast::{Binding, Expr, ExprKind, Pattern, PatternKind};

    let mut injected: Vec<ast::Item> = Vec::new();

    for (mname, class) in methods {
        // Check if class is Enum (unqualified or qualified like Prelude.Enum)
        let is_enum = class == "Enum" || class.ends_with(".Enum");
        if defined.contains(&mname) || !is_enum {
            continue;
        }

        // Look up the instance dict name for Enum Integer
        // Use the real ClassId from class_env instead of a dummy with ModuleId(0).
        let Some((class_id, _)) = class_env
            .class_params
            .iter()
            .find(|(cid, _)| cid.name == class)
        else {
            eprintln!(
                "[WARN] create_method_bindings: no ClassId found for class {}",
                class
            );
            continue;
        };

        let dict_key = (class_id.clone(), "Integer".to_string());
        let Some(inst_name) = class_env.instances.get(&dict_key).cloned() else {
            eprintln!("[WARN] create_method_bindings: no instance dict for Enum Integer");
            continue;
        };

        let method_fn = create_record_get_expr(&inst_name, &mname);
        let dict_arg = Expr::new(ast::dummy_span(), ExprKind::Var(inst_name.clone()));

        let (params, args) = match mname.as_str() {
            "enumFromTo" => {
                let a0 = "_a0".to_string();
                let a1 = "_a1".to_string();
                (
                    vec![a0.clone(), a1.clone()],
                    vec![
                        dict_arg,
                        Expr::new(ast::dummy_span(), ExprKind::Var(a0)),
                        Expr::new(ast::dummy_span(), ExprKind::Var(a1)),
                    ],
                )
            }
            "enumFromThenTo" => {
                let a0 = "_a0".to_string();
                let a1 = "_a1".to_string();
                let a2 = "_a2".to_string();
                (
                    vec![a0.clone(), a1.clone(), a2.clone()],
                    vec![
                        dict_arg,
                        Expr::new(ast::dummy_span(), ExprKind::Var(a0)),
                        Expr::new(ast::dummy_span(), ExprKind::Var(a1)),
                        Expr::new(ast::dummy_span(), ExprKind::Var(a2)),
                    ],
                )
            }
            _ => continue,
        };

        let body = Expr::new(
            ast::dummy_span(),
            ExprKind::Apply {
                func: Box::new(method_fn),
                args,
            },
        );

        let expr = Expr::new(
            ast::dummy_span(),
            ExprKind::Lambda {
                params,
                body: Box::new(body),
            },
        );

        injected.push(ast::Item::Binding(Binding {
            doc: None,
            pat: Pattern {
                kind: PatternKind::Var(mname),
                span: ast::dummy_span(),
            },
            expr,
            span: ast::dummy_span(),
        }));
    }

    injected
}

fn create_record_get_expr(inst_name: &str, field_name: &str) -> ast::Expr {
    use ast::{Expr, ExprKind};

    let inst = Expr::new(ast::dummy_span(), ExprKind::Var(inst_name.to_string()));
    let get = Expr::new(ast::dummy_span(), ExprKind::Var("__recordGet".to_string()));
    Expr::new(
        ast::dummy_span(),
        ExprKind::Apply {
            func: Box::new(get),
            args: vec![
                inst,
                Expr::new(ast::dummy_span(), ExprKind::String(field_name.to_string())),
            ],
        },
    )
}

fn debug_print_method_injection(
    class_env: &ClassEnv,
    inferred: &HashMap<String, Scheme>,
    injected: &[ast::Item],
) {
    eprintln!(
        "[KSCR_DEBUG_METHOD_VALUES] inject pass: method_classes keys sample: {:?}",
        class_env
            .method_classes
            .keys()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
    );
    eprintln!(
        "[KSCR_DEBUG_METHOD_VALUES] inject pass: inferred has enumFromTo={} enumFromThenTo={}",
        inferred.contains_key("enumFromTo"),
        inferred.contains_key("enumFromThenTo")
    );

    let mut has_from_to = false;
    let mut has_from_then_to = false;
    for it in injected {
        let ast::Item::Binding(b) = it else {
            continue;
        };
        if let ast::PatternKind::Var(n) = &b.pat.kind {
            if n == "enumFromTo" {
                has_from_to = true;
            }
            if n == "enumFromThenTo" {
                has_from_then_to = true;
            }
        }
    }
    eprintln!("[KSCR_DEBUG_METHOD_VALUES] injected method bindings: enumFromTo={has_from_to} enumFromThenTo={has_from_then_to} total={}", injected.len());
}

fn expr_contains_var(e: &ast::Expr, name: &str) -> bool {
    match &e.kind {
        ast::ExprKind::Var(v) => v == name,
        ast::ExprKind::Ctor(_) => false,
        ast::ExprKind::Unit => false,
        ast::ExprKind::Integer(_) => false,
        ast::ExprKind::Float64(_) => false,
        ast::ExprKind::Bool(_) => false,
        ast::ExprKind::String(_) => false,
        ast::ExprKind::Char(_) => false,
        ast::ExprKind::Lambda { body, .. } => expr_contains_var(body, name),
        ast::ExprKind::Apply { func, args } => {
            expr_contains_var(func, name) || args.iter().any(|a| expr_contains_var(a, name))
        }
        ast::ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            expr_contains_var(cond, name)
                || expr_contains_var(then_branch, name)
                || expr_contains_var(else_branch, name)
        }
        ast::ExprKind::Let { bindings, body } => {
            bindings.iter().any(|b| expr_contains_var(&b.expr, name))
                || expr_contains_var(body, name)
        }
        ast::ExprKind::Where { expr, bindings } => {
            expr_contains_var(expr, name)
                || bindings.iter().any(|b| expr_contains_var(&b.expr, name))
        }
        ast::ExprKind::Annot { expr, .. } => expr_contains_var(expr, name),
        ast::ExprKind::Do(stmts) => stmts.iter().any(|s| match s {
            ast::DoStmt::Bind { expr, .. } => expr_contains_var(expr, name),
            ast::DoStmt::Expr(e) => expr_contains_var(e, name),
        }),
        ast::ExprKind::Case { expr, arms } => {
            expr_contains_var(expr, name)
                || arms.iter().any(|a| {
                    a.guard.as_ref().is_some_and(|g| expr_contains_var(g, name))
                        || expr_contains_var(&a.body, name)
                })
        }
        ast::ExprKind::Cons { head, tail } => {
            expr_contains_var(head, name) || expr_contains_var(tail, name)
        }
        ast::ExprKind::List(es) | ast::ExprKind::Tuple(es) => {
            es.iter().any(|e| expr_contains_var(e, name))
        }
        ast::ExprKind::Record(fields) => fields.iter().any(|(_, v)| expr_contains_var(v, name)),
    }
}

fn expr_contains_var_any(e: &ast::Expr, names: &[&str]) -> bool {
    names.iter().any(|n| expr_contains_var(e, n))
}

fn infer_module_with_class_env(
    module: &ast::Module,
    class_env: &ClassEnv,
    class_index: &ClassEnvIndex,
) -> Result<HashMap<String, Scheme>> {
    // Order-independent top-level inference (Haskell-like): compute SCCs of top-level bindings,
    // then typecheck SCCs in dependency order, generalizing non-recursive groups.
    let mut cx = InferCtx {
        class_env: class_index.clone(),
        ..Default::default()
    };
    cx.full_class_env = std::sync::Arc::new(class_env.clone());
    let data_env = collect_data_env(module);
    let mut env_global = collect_ctor_env_with_class_env(&mut cx, module, class_env, None)?;
    let mut env_global_ftv = ftv_env(&env_global);

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
                EnvEntry {
                    scheme: Scheme {
                        vars: vec![],
                        constraints: vec![],
                        ty: Ty::Var(v),
                    },
                    def_site: None,
                },
            );
        }

        // Infer each binding in the SCC under the placeholder environment.
        let mut per_bind: Vec<BindingInfer> = Vec::new();
        for &bi in comp {
            let b = &bindings[bi];
            let ctx_name = &ctx_names[bi];

            let mut binds: Vec<(String, Ty)> = Vec::new();
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

            let (s_rhs, cs_rhs, t_rhs) =
                infer_expr_in(&mut cx, &data_env, &subst, &env_scc, b.expr.clone()).map_err(
                    |e| {
                        // If the inner error doesn't have a useful primary span,
                        // promote the binding RHS span as the primary location.
                        let mut e = e;
                        let needs_primary = e.span().is_none_or(|s| s.start == s.end);
                        if needs_primary {
                            e = e.push_span(b.expr.span);
                        }
                        e.push_secondary_span(b.pat.span)
                            .with_context(format!("in binding {ctx_name}"))
                    },
                )?;
            subst = compose(&s_rhs, &subst);

            let s_pat = unify(apply(&subst, t_rhs), apply(&subst, pat_ty)).map_err(|e| {
                e.push_span(b.expr.span)
                    .push_secondary_span(b.pat.span)
                    .with_context(format!("in binding {ctx_name}"))
            })?;
            subst = compose(&s_pat, &subst);

            let mut cs = cs_rhs;
            cs.extend(cs_pat);
            per_bind.push((binds, cs));
        }

        // Generalize all names in the SCC against the environment *outside* the SCC.
        let env_gen_ftv = ftv_env_applied_from_ftv(&subst, &env_global_ftv);
        let mut new_schemes: Vec<(String, Scheme)> = Vec::new();
        for (binds, cs) in per_bind {
            for (name, t) in binds {
                let cs = simplify_constraints(
                    &data_env,
                    class_env,
                    apply_constraints(&subst, cs.clone()),
                )?;
                let scheme = generalize_qual_with_env_ftv(&env_gen_ftv, cs, apply(&subst, t));
                new_schemes.push((name, scheme));
            }
        }

        for (name, scheme) in new_schemes {
            env_global_ftv.extend(ftv_scheme(&scheme));
            env_global.insert(
                name.clone(),
                EnvEntry {
                    scheme: scheme.clone(),
                    def_site: None,
                },
            );
            out.insert(name, scheme);
        }
    }

    Ok(out)
}

fn infer_module_with_class_env_with_entry_path(
    module: &ast::Module,
    class_env: &ClassEnv,
    class_index: &ClassEnvIndex,
    imported: Option<&HashMap<String, HashMap<String, Scheme>>>,
    entry_path: Option<&Path>,
) -> Result<HashMap<String, Scheme>> {
    // Order-independent top-level inference (Haskell-like): compute SCCs of top-level bindings,
    // then typecheck SCCs in dependency order, generalizing non-recursive groups.
    let mut cx = InferCtx {
        class_env: class_index.clone(),
        ..Default::default()
    };
    cx.full_class_env = std::sync::Arc::new(class_env.clone());
    let mut data_env = collect_data_env(module);

    // IMPORTANT: Also collect DataDecls from imported modules for deriving info.
    // KSIF doesn't carry deriving info, so we need to load source files.
    if let Some(entry_path) = entry_path {
        let entry_dir = entry_path.parent().unwrap_or_else(|| Path::new("."));
        for it in &module.items {
            let ast::Item::Import(id) = it else {
                continue;
            };
            // Try to load the imported module's source to get DataDecls with deriving info
            if let Ok(imported_path) = resolve_module_path(entry_dir, &id.module) {
                if let Ok(src) = std::fs::read_to_string(&imported_path) {
                    if let Ok(mut imported_mod) = parser::parse_module(&src) {
                        let _ = desugar_module_qualified_names(&mut imported_mod);
                        let imported_data_env = collect_data_env(&imported_mod);
                        data_env.extend(imported_data_env);
                    }
                }
            }
        }
    }

    let mut env_global = collect_ctor_env_with_class_env(&mut cx, module, class_env, entry_path)?;

    // Merge imported .ksif schemes into env_global so qualified names like `Data.Maybe.fromMaybe` can be resolved.
    if let Some(imported) = imported {
        if std::env::var("KSCR_DEBUG_IMPORTS").ok().is_some() {
            eprintln!(
                "[KSCR_DEBUG_IMPORTS] Merging imported schemes, {} modules",
                imported.len()
            );
        }
        // First pass: add all value schemes
        for (module_name, schemes) in imported {
            if std::env::var("KSCR_DEBUG_IMPORTS").ok().is_some() {
                eprintln!(
                    "[KSCR_DEBUG_IMPORTS] Module: {}, schemes: {}",
                    module_name,
                    schemes.len()
                );
            }

            // Collect aliases, qualified flag, and import_spec
            // Only apply import filtering if we're importing this module from another module,
            // not when processing the module itself
            let current_module = module.name.as_deref();
            let is_self_import = current_module == Some(module_name.as_str());

            let mut is_qualified = false;
            let mut aliases: Vec<String> = Vec::new();
            let mut import_spec: Option<ast::ImportSpec> = None;
            if !is_self_import {
                // Only look for import spec if this is not a self-import
                for it in &module.items {
                    if let ast::Item::Import(id) = it {
                        if id.module == *module_name {
                            is_qualified = id.qualified;
                            if let Some(ref as_name) = id.as_name {
                                aliases.push(as_name.clone());
                            }
                            import_spec = id.import_spec.clone();
                        }
                    }
                }
            }

            // Apply import spec filter (only if not a self-import)
            let filtered_names: std::collections::HashSet<String> = if is_self_import {
                schemes.keys().cloned().collect() // Self-import: import everything
            } else {
                let entry_dir = if let Some(ep) = entry_path {
                    ep.parent().unwrap_or_else(|| Path::new("."))
                } else {
                    Path::new(".")
                };
                match &import_spec {
                    None => schemes.keys().cloned().collect(), // No filter, import everything
                    Some(ast::ImportSpec::Only(specs)) => {
                        // Expand ExportSpecs to names with constructor resolution
                        expand_import_spec_with_ctors(specs, module_name, entry_dir)
                    }
                    Some(ast::ImportSpec::Hiding(specs)) => {
                        // Expand ExportSpecs to names to hide
                        let hidden = expand_import_spec_with_ctors(specs, module_name, entry_dir);
                        schemes
                            .keys()
                            .filter(|n| !hidden.contains(*n))
                            .cloned()
                            .collect()
                    }
                }
            };

            for (name, scheme) in schemes {
                // Skip if name is not in the filtered set
                if !filtered_names.contains(name) {
                    continue;
                }

                // For unqualified imports without aliases, expose both qualified and unqualified names
                if !is_qualified && aliases.is_empty() {
                    // Unqualified import: `import A` - expose both A.name and name
                    let qualified_name = format!("{}.{}", module_name, name);
                    if std::env::var("KSCR_DEBUG_IMPORTS").ok().is_some() {
                        eprintln!(
                            "[KSCR_DEBUG_IMPORTS] Adding unqualified: {} and {}",
                            name, qualified_name
                        );
                    }
                    env_global.insert(
                        qualified_name,
                        EnvEntry {
                            scheme: scheme.clone(),
                            def_site: None,
                        },
                    );
                    // Also expose unqualified name
                    env_global.insert(
                        name.clone(),
                        EnvEntry {
                            scheme: scheme.clone(),
                            def_site: None,
                        },
                    );
                } else if !aliases.is_empty() {
                    // Aliased import: `import A as M` or `import qualified A as M`
                    // Expose only alias-qualified names
                    for a in &aliases {
                        let qualified_name = format!("{}.{}", a, name);
                        env_global.insert(
                            qualified_name,
                            EnvEntry {
                                scheme: scheme.clone(),
                                def_site: None,
                            },
                        );
                    }
                } else {
                    // Qualified import without alias: `import qualified A` - expose only A.name
                    let qualified_name = format!("{}.{}", module_name, name);
                    env_global.insert(
                        qualified_name,
                        EnvEntry {
                            scheme: scheme.clone(),
                            def_site: None,
                        },
                    );
                }
            }
        }

        // Second pass: add constructor re-exports after all schemes are merged.
        // Hardcoded constructor re-export for Data.Maybe:
        // Data.Maybe exports `type Maybe = Prelude.Maybe` with `Maybe(..)`,
        // so qualified access like `M.Just` when `import qualified Data.Maybe as M`
        // should resolve to Prelude.Just.
        for module_name in imported.keys() {
            if module_name == "Data.Maybe" {
                if std::env::var("KSCR_DEBUG_IMPORTS").ok().is_some() {
                    eprintln!("[KSCR_DEBUG_IMPORTS] Handling Data.Maybe constructor re-exports");
                    eprintln!(
                        "[KSCR_DEBUG_IMPORTS] Just in env? {}",
                        env_global.contains_key("Just")
                    );
                    eprintln!(
                        "[KSCR_DEBUG_IMPORTS] Prelude.Just in env? {}",
                        env_global.contains_key("Prelude.Just")
                    );
                    eprintln!(
                        "[KSCR_DEBUG_IMPORTS] Nothing in env? {}",
                        env_global.contains_key("Nothing")
                    );
                }
                // Prelude constructors might be qualified; check both.
                if let Some(just_entry) = env_global
                    .get("Just")
                    .or_else(|| env_global.get("Prelude.Just"))
                {
                    env_global.insert("Data.Maybe.Just".to_string(), just_entry.clone());
                    if std::env::var("KSCR_DEBUG_IMPORTS").ok().is_some() {
                        eprintln!("[KSCR_DEBUG_IMPORTS] Added Data.Maybe.Just");
                    }
                }
                if let Some(nothing_entry) = env_global
                    .get("Nothing")
                    .or_else(|| env_global.get("Prelude.Nothing"))
                {
                    env_global.insert("Data.Maybe.Nothing".to_string(), nothing_entry.clone());
                    if std::env::var("KSCR_DEBUG_IMPORTS").ok().is_some() {
                        eprintln!("[KSCR_DEBUG_IMPORTS] Added Data.Maybe.Nothing");
                    }
                }
            }
        }

        // Handle import aliases: if `import qualified Data.Maybe as M`, add `M.Just` mapping to `Data.Maybe.Just`.
        if std::env::var("KSCR_DEBUG_IMPORTS").ok().is_some() {
            eprintln!(
                "[KSCR_DEBUG_IMPORTS] Handling import aliases, {} items in module",
                module.items.len()
            );
        }
        for it in &module.items {
            if let ast::Item::Import(id) = it {
                if std::env::var("KSCR_DEBUG_IMPORTS").ok().is_some() {
                    eprintln!(
                        "[KSCR_DEBUG_IMPORTS] Import: {} as {:?}",
                        id.module, id.as_name
                    );
                }
                if let Some(as_name) = &id.as_name {
                    // For each imported module with an alias, check if we've added qualified constructors
                    // and add them with the alias prefix too.
                    if id.module == "Data.Maybe" {
                        if let Some(maybe_just) = env_global.get("Data.Maybe.Just") {
                            let key = format!("{}.Just", as_name);
                            if std::env::var("KSCR_DEBUG_IMPORTS").ok().is_some() {
                                eprintln!("[KSCR_DEBUG_IMPORTS] Adding ctor mapping: {}", key);
                            }
                            env_global.insert(key, maybe_just.clone());
                        }
                        if let Some(maybe_nothing) = env_global.get("Data.Maybe.Nothing") {
                            let key = format!("{}.Nothing", as_name);
                            if std::env::var("KSCR_DEBUG_IMPORTS").ok().is_some() {
                                eprintln!("[KSCR_DEBUG_IMPORTS] Adding ctor mapping: {}", key);
                            }
                            env_global.insert(key, maybe_nothing.clone());
                        }
                    }
                }
            }
        }
    }

    let mut env_global_ftv = ftv_env(&env_global);

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
                EnvEntry {
                    scheme: Scheme {
                        vars: vec![],
                        constraints: vec![],
                        ty: Ty::Var(v),
                    },
                    def_site: entry_path.map(|p| DefSite {
                        path: p.to_path_buf(),
                        span: ast::dummy_span(),
                    }),
                },
            );
        }

        // Infer each binding in the SCC under the placeholder environment.
        let mut per_bind: Vec<BindingInfer> = Vec::new();
        for &bi in comp {
            let b = &bindings[bi];
            let ctx_name = &ctx_names[bi];

            let mut binds: Vec<(String, Ty)> = Vec::new();
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

            let (s_rhs, cs_rhs, t_rhs) =
                infer_expr_in(&mut cx, &data_env, &subst, &env_scc, b.expr.clone()).map_err(
                    |e| {
                        // If the inner error doesn't have a useful primary span,
                        // promote the binding RHS span as the primary location.
                        let mut e = e;
                        let needs_primary = e.span().is_none_or(|s| s.start == s.end);
                        if needs_primary {
                            e = e.push_span(b.expr.span);
                        }
                        e.push_secondary_span(b.pat.span)
                            .with_context(format!("in binding {ctx_name}"))
                    },
                )?;
            subst = compose(&s_rhs, &subst);

            let s_pat = unify(apply(&subst, t_rhs), apply(&subst, pat_ty)).map_err(|e| {
                e.push_span(b.expr.span)
                    .push_secondary_span(b.pat.span)
                    .with_context(format!("in binding {ctx_name}"))
            })?;
            subst = compose(&s_pat, &subst);

            let mut cs = cs_rhs;
            cs.extend(cs_pat);
            per_bind.push((binds, cs));
        }

        // Generalize all names in the SCC against the environment *outside* the SCC.
        let env_gen_ftv = ftv_env_applied_from_ftv(&subst, &env_global_ftv);
        let mut new_schemes: Vec<(String, Scheme)> = Vec::new();
        for (binds, cs) in per_bind {
            for (name, t) in binds {
                let cs = simplify_constraints(
                    &data_env,
                    class_env,
                    apply_constraints(&subst, cs.clone()),
                )?;
                let scheme = generalize_qual_with_env_ftv(&env_gen_ftv, cs, apply(&subst, t));
                new_schemes.push((name, scheme));
            }
        }

        for (name, scheme) in new_schemes {
            env_global_ftv.extend(ftv_scheme(&scheme));
            env_global.insert(
                name.clone(),
                EnvEntry {
                    scheme: scheme.clone(),
                    def_site: None,
                },
            );
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

fn infer_module_binding_scc_order(module: &ast::Module) -> Result<InferModuleBindingSccOrder> {
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

// (Old `typecheck` body removed; use `typecheck_internal`.)

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

fn desugar_monad_do_block(
    stmts: Vec<ast::DoStmt>,
    fresh: &mut usize,
    span: ast::Span,
) -> Result<ast::Expr> {
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
        ExprKind::Lambda { params, body } => desugar_monad_do_lambda(span, params, *body, fresh)?,
        ExprKind::Apply { func, args } => desugar_monad_do_apply(span, *func, args, fresh)?,
        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => desugar_monad_do_if(span, *cond, *then_branch, *else_branch, fresh)?,
        ExprKind::Let { bindings, body } => desugar_monad_do_let(span, bindings, *body, fresh)?,
        ExprKind::Where { expr, bindings } => desugar_monad_do_where(span, *expr, bindings, fresh)?,
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
                doc: b.doc,
                pat: b.pat,
                expr: desugar_monad_do_expr(b.expr, fresh)?,
                span: b.span,
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

fn collect_type_alias_def_sites(
    module: &ast::Module,
    module_path: Option<&std::path::Path>,
) -> HashMap<String, DefSite> {
    let Some(path) = module_path else {
        return HashMap::new();
    };
    module
        .items
        .iter()
        .filter_map(|it| match it {
            ast::Item::TypeAlias(ta) => {
                if ta.span.start == ta.span.end {
                    None
                } else {
                    Some((
                        ta.name.clone(),
                        DefSite {
                            path: path.to_path_buf(),
                            span: ta.span,
                        },
                    ))
                }
            }
            _ => None,
        })
        .collect()
}

fn expand_predicate(
    p: ast::Predicate,
    aliases: &HashMap<String, ast::TypeAlias>,
) -> Result<ast::Predicate> {
    Ok(match p {
        ast::Predicate::Show(t) => ast::Predicate::Show(expand_type(t, aliases, &mut Vec::new())?),
        ast::Predicate::ShowRow(t) => {
            ast::Predicate::ShowRow(expand_type(t, aliases, &mut Vec::new())?)
        }
        ast::Predicate::Eq(t) => ast::Predicate::Eq(expand_type(t, aliases, &mut Vec::new())?),
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
}

fn expand_bindings(
    bindings: Vec<ast::Binding>,
    aliases: &HashMap<String, ast::TypeAlias>,
) -> Result<Vec<ast::Binding>> {
    bindings
        .into_iter()
        .map(|b| {
            Ok(ast::Binding {
                doc: b.doc,
                pat: expand_pat(b.pat, aliases)?,
                expr: expand_expr(b.expr, aliases)?,
                span: b.span,
            })
        })
        .collect()
}

fn expand_do_stmts(
    stmts: Vec<ast::DoStmt>,
    aliases: &HashMap<String, ast::TypeAlias>,
) -> Result<Vec<ast::DoStmt>> {
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
        .collect()
}

fn expand_case_arms(
    arms: Vec<ast::CaseArm>,
    aliases: &HashMap<String, ast::TypeAlias>,
) -> Result<Vec<ast::CaseArm>> {
    arms.into_iter()
        .map(|a| {
            Ok(ast::CaseArm {
                pat: expand_pat(a.pat, aliases)?,
                guard: a.guard.map(|g| expand_expr(g, aliases)).transpose()?,
                body: expand_expr(a.body, aliases)?,
            })
        })
        .collect()
}

fn expand_expr_list(
    es: Vec<ast::Expr>,
    aliases: &HashMap<String, ast::TypeAlias>,
) -> Result<Vec<ast::Expr>> {
    es.into_iter().map(|e| expand_expr(e, aliases)).collect()
}

fn expand_data_ctors(
    ctors: Vec<ast::DataCtor>,
    aliases: &HashMap<String, ast::TypeAlias>,
) -> Result<Vec<ast::DataCtor>> {
    ctors
        .into_iter()
        .map(|c| {
            let mut span = c.span;
            if span.start > span.end {
                std::mem::swap(&mut span.start, &mut span.end);
            }
            Ok(ast::DataCtor {
                doc: c.doc,
                name: c.name,
                args: c
                    .args
                    .into_iter()
                    .map(|t| expand_type(t, aliases, &mut Vec::new()))
                    .collect::<Result<Vec<_>>>()?,
                span,
            })
        })
        .collect()
}

fn expand_item(item: ast::Item, aliases: &HashMap<String, ast::TypeAlias>) -> Result<ast::Item> {
    match item {
        ast::Item::Binding(b) => Ok(ast::Item::Binding(ast::Binding {
            doc: b.doc,
            pat: expand_pat(b.pat, aliases)?,
            expr: expand_expr(b.expr, aliases)?,
            span: b.span,
        })),
        ast::Item::TypeAlias(ta) => Ok(ast::Item::TypeAlias(ast::TypeAlias {
            doc: ta.doc,
            name: ta.name,
            params: ta.params,
            ty: expand_type(ta.ty, aliases, &mut Vec::new())?,
            span: ta.span,
        })),
        ast::Item::DataDecl(d) => Ok(ast::Item::DataDecl(ast::DataDecl {
            doc: d.doc,
            name: d.name,
            params: d.params,
            ctors: expand_data_ctors(d.ctors, aliases)?,
            deriving: d.deriving,
            span: d.span,
        })),
        ast::Item::ClassDecl(c) => Ok(ast::Item::ClassDecl(ast::ClassDecl {
            doc: c.doc,
            name: c.name,
            param: c.param,
            supers: c
                .supers
                .into_iter()
                .map(|p| expand_predicate(p, aliases))
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
                                .map(|p| expand_predicate(p, aliases))
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
                        doc: b.doc,
                        pat: expand_pat(b.pat, aliases)?,
                        expr: expand_expr(b.expr, aliases)?,
                        span: b.span,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            def_module: c.def_module,
        })),
        ast::Item::InstanceDecl(inst) => Ok(ast::Item::InstanceDecl(ast::InstanceDecl {
            preds: inst
                .preds
                .into_iter()
                .map(|p| expand_predicate(p, aliases))
                .collect::<Result<Vec<_>>>()?,
            class: inst.class,
            ty: expand_type(inst.ty, aliases, &mut Vec::new())?,
            methods: inst
                .methods
                .into_iter()
                .map(|b| {
                    Ok(ast::Binding {
                        doc: b.doc,
                        pat: expand_pat(b.pat, aliases)?,
                        expr: expand_expr(b.expr, aliases)?,
                        span: b.span,
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
                args: expand_expr_list(args, aliases)?,
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
                bindings: expand_bindings(bindings, aliases)?,
                body: Box::new(expand_expr(*body, aliases)?),
            },
        ),
        ExprKind::Where { expr, bindings } => Expr::new(
            span,
            ExprKind::Where {
                expr: Box::new(expand_expr(*expr, aliases)?),
                bindings: expand_bindings(bindings, aliases)?,
            },
        ),
        ExprKind::Annot { expr, ty } => Expr::new(
            span,
            ExprKind::Annot {
                expr: Box::new(expand_expr(*expr, aliases)?),
                ty: expand_qual_type(ty, aliases)?,
            },
        ),
        ExprKind::Do(stmts) => Expr::new(span, ExprKind::Do(expand_do_stmts(stmts, aliases)?)),
        ExprKind::Case { expr, arms } => Expr::new(
            span,
            ExprKind::Case {
                expr: Box::new(expand_expr(*expr, aliases)?),
                arms: expand_case_arms(arms, aliases)?,
            },
        ),
        ExprKind::Cons { head, tail } => Expr::new(
            span,
            ExprKind::Cons {
                head: Box::new(expand_expr(*head, aliases)?),
                tail: Box::new(expand_expr(*tail, aliases)?),
            },
        ),
        ExprKind::List(v) => Expr::new(span, ExprKind::List(expand_expr_list(v, aliases)?)),
        ExprKind::Tuple(v) => Expr::new(span, ExprKind::Tuple(expand_expr_list(v, aliases)?)),
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
                    // Record alias usage (for stdlib aliases like `String`).
                    // Resolve to qualified via import hints, so we can later show def-site evidence.
                    TL_NAME_HINTS.with(|h| {
                        let hints = h.borrow();
                        if let Some(qual) = hints.type_alias.get(&UnqualName(alias.name.clone())) {
                            TL_ALIAS_EVIDENCE.with(|slot| {
                                slot.borrow_mut()
                                    .push((UnqualName(alias.name.clone()), qual.clone()));
                            });
                        }
                    });
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

    // Record alias usage for later unify diagnostics.
    // We only know unqualified alias name here; resolve to qualified via current import hints.
    TL_NAME_HINTS.with(|h| {
        let hints = h.borrow();
        if let Some(qual) = hints.type_alias.get(&UnqualName(alias.name.clone())) {
            TL_ALIAS_EVIDENCE.with(|slot| {
                slot.borrow_mut()
                    .push((UnqualName(alias.name.clone()), qual.clone()));
            });
        }
    });

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
mod import_type_forwarder_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn type_annotation_can_use_unqualified_data_type_from_import() {
        // Regression test: importing Prelude should make `Maybe` usable in type annotations
        // (resolved to `Prelude.Maybe`), so this should typecheck.
        let tmp = std::env::temp_dir().join("kscr_import_type_forwarder_maybe.ks");
        std::fs::write(
            &tmp,
            r#"module ImportTypeForwarderMaybe where
  export Parser, pureP
  import Prelude

  type Parser a = String -> Maybe (a, String)

  pureP :: a -> Parser a
  pureP a = \s -> Just (a, s)
"#,
        )
        .unwrap();

        let typed = typecheck_file(Path::new(&tmp)).unwrap();
        assert!(typed.inferred.contains_key("pureP"));
        let _ = std::fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod class_ambiguity_resolution_tests {
    use super::*;
    use std::sync::Mutex;

    // These tests temporarily change the process-wide current working directory.
    // Serialize them and make the cwd restoration panic-safe.
    static CWD_MUTEX: Mutex<()> = Mutex::new(());

    struct CwdGuard {
        old: std::path::PathBuf,
    }

    impl CwdGuard {
        fn new(dir: &std::path::Path) -> Self {
            let old = std::env::current_dir().unwrap();
            std::env::set_current_dir(dir).unwrap();
            Self { old }
        }
    }

    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.old);
        }
    }

    fn write(path: &std::path::Path, body: &str) {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(path, body).unwrap();
    }

    #[test]
    fn prefers_more_specific_module_for_unqualified_class_ref() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_class_ambiguity_more_specific_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let _lock = CWD_MUTEX.lock().unwrap();
        {
            let _cwd = CwdGuard::new(&dir);

            write(
                &dir.join("A.ks"),
                "module A where\n  export C(..)\n  class C a where\n    c :: a -> a\n",
            );
            write(
                &dir.join("A/B.ks"),
                "module A.B where\n  export C(..)\n  class C a where\n    c :: a -> a\n",
            );

            let src = "module Main where\n  import A\n  import A.B\n  class C a => D a where\n    d :: a -> a\n";
            let mut m = parser::parse_module(src).unwrap();
            desugar_module_qualified_names(&mut m).unwrap();
            for it in &mut m.items {
                if let ast::Item::ClassDecl(c) = it {
                    c.def_module = Some("Main".to_string());
                }
            }

            canonicalize_class_names_in_module_combined(&mut m, true).unwrap();

            let d = m
                .items
                .iter()
                .find_map(|it| match it {
                    ast::Item::ClassDecl(c) if c.name == "D" => Some(c),
                    _ => None,
                })
                .unwrap();
            let class = match &d.supers[0] {
                ast::Predicate::Class { class, .. } => class,
                _ => panic!("expected class predicate"),
            };
            assert_eq!(class.name, "A.B.C");
        }

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn strict_mode_errors_on_ambiguous_same_specificity() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_class_ambiguity_strict_error_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let _lock = CWD_MUTEX.lock().unwrap();
        {
            let _cwd = CwdGuard::new(&dir);

            write(
                &dir.join("X.ks"),
                "module X where\n  export C(..)\n  class C a where\n    c :: a -> a\n",
            );
            write(
                &dir.join("Y.ks"),
                "module Y where\n  export C(..)\n  class C a where\n    c :: a -> a\n",
            );

            let src = "module Main where\n  import X\n  import Y\n  class C a => D a where\n    d :: a -> a\n";
            let mut m = parser::parse_module(src).unwrap();
            desugar_module_qualified_names(&mut m).unwrap();
            for it in &mut m.items {
                if let ast::Item::ClassDecl(c) = it {
                    c.def_module = Some("Main".to_string());
                }
            }

            let err = canonicalize_class_names_in_module_combined(&mut m, true).unwrap_err();
            assert!(err.to_string().contains("Ambiguous class reference: 'C'"));
        }

        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod inference_tests {
    use super::*;

    #[test]
    fn typecheck_rejects_imports() {
        let m = ast::Module {
            name: None,
            export_specs: None,
            items: vec![ast::Item::Import(ast::ImportDecl {
                module: "Foo".to_string(),
                qualified: false,
                as_name: None,
                import_spec: None,
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
    fn unknown_constructor_error_shows_type_ctor_resolution_hint() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_unknown_ctor_resolution_hint_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Provide a standard Prelude import so qualified names exist, but reference
        // an unqualified type ctor that is not in scope. The error message should
        // still hint the likely resolution.
        let main = dir.join("Main.ks");
        std::fs::write(
            &main,
            "module Main where\n  import Prelude.Read\n\n  bad : Maybe Integer = Nothing\n",
        )
        .unwrap();

        let e = typecheck_file(&main).unwrap_err();
        let msg = format!("{e}");
        assert!(msg.contains("unknown constructor: Maybe"));
        assert!(msg.contains("resolves to `Prelude.Maybe`"));

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
    fn typecheck_file_first_import_wins_for_name_conflicts() {
        // Test that when multiple imports provide the same unqualified name,
        // the first import wins (no error, just silently use first definition).
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

        // Should succeed (first import wins: x comes from A)
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
            "forall a. Prelude.Show a => a -> a"
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
                doc: None,
                pat: ast::Pattern::dummy(ast::PatternKind::Var("x".to_string())),
                expr: ast::Expr::dummy(ast::ExprKind::Var("y".to_string())),
                span: ast::dummy_span(),
            }],
            body: Box::new(ast::Expr::dummy(ast::ExprKind::Var("x".to_string()))),
        }))
        .unwrap_err();

        let e = infer_expr(ast::Expr::dummy(ast::ExprKind::Let {
            bindings: vec![ast::Binding {
                doc: None,
                pat: ast::Pattern::dummy(ast::PatternKind::Var("x".to_string())),
                expr: ast::Expr::dummy(ast::ExprKind::Var("y".to_string())),
                span: ast::dummy_span(),
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
                doc: None,
                pat: ast::Pattern::dummy(ast::PatternKind::Var("x".to_string())),
                expr: ast::Expr::dummy(ast::ExprKind::Var("y".to_string())),
                span: ast::dummy_span(),
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
            doc: None,
            pat: ast::Pattern::dummy(ast::PatternKind::Var("id".to_string())),
            expr: ast::Expr::dummy(ast::ExprKind::Lambda {
                params: vec!["x".to_string()],
                body: Box::new(ast::Expr::dummy(ast::ExprKind::Var("x".to_string()))),
            }),
            span: ast::dummy_span(),
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
            doc: None,
            pat: ast::Pattern::dummy(ast::PatternKind::Tuple(vec![
                ast::Pattern::dummy(ast::PatternKind::Var("a".to_string())),
                ast::Pattern::dummy(ast::PatternKind::Var("b".to_string())),
            ])),
            expr: ast::Expr::dummy(ast::ExprKind::Tuple(vec![
                ast::Expr::dummy(ast::ExprKind::Integer("1".to_string())),
                ast::Expr::dummy(ast::ExprKind::Bool(true)),
            ])),
            span: ast::dummy_span(),
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
            doc: None,
            pat: ast::Pattern::dummy(ast::PatternKind::Tuple(vec![
                ast::Pattern::dummy(ast::PatternKind::Var("x".to_string())),
                ast::Pattern::dummy(ast::PatternKind::Var("x".to_string())),
            ])),
            expr: ast::Expr::dummy(ast::ExprKind::Tuple(vec![
                ast::Expr::dummy(ast::ExprKind::Integer("1".to_string())),
                ast::Expr::dummy(ast::ExprKind::Integer("2".to_string())),
            ])),
            span: ast::dummy_span(),
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
            doc: None,
            pat: ast::Pattern::dummy(ast::PatternKind::List(vec![
                ast::Pattern::dummy(ast::PatternKind::Var("x".to_string())),
                ast::Pattern::dummy(ast::PatternKind::Var("y".to_string())),
            ])),
            expr: ast::Expr::dummy(ast::ExprKind::List(vec![
                ast::Expr::dummy(ast::ExprKind::Integer("1".to_string())),
                ast::Expr::dummy(ast::ExprKind::Integer("2".to_string())),
            ])),
            span: ast::dummy_span(),
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
            doc: None,
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
            span: ast::dummy_span(),
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
            doc: None,
            pat: ast::Pattern::dummy(ast::PatternKind::Record(vec![(
                "a".to_string(),
                ast::Pattern::dummy(ast::PatternKind::Wildcard),
            )])),
            expr: ast::Expr::dummy(ast::ExprKind::Record(vec![(
                "b".to_string(),
                ast::Expr::dummy(ast::ExprKind::Bool(true)),
            )])),
            span: ast::dummy_span(),
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
        let (class_name, t) = match &s.constraints[0] {
            Constraint::Class { class, ty } => (&class.name, ty),
            other => panic!("expected Class constraint, got {other:?}"),
        };
        assert!(
            class_name == "Show" || class_name.ends_with(".Show"),
            "expected Show class, got {class_name}"
        );

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
            .any(|c| matches!(c, Constraint::Class { class, .. } if class.name == "Show" || class.name.ends_with(".Show"))));
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

        assert!(s.constraints.iter().any(
            |c| matches!(c, Constraint::Class { class, .. } if class.name == "Show" || class.name.ends_with(".Show"))
        ));
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
            doc: None,
            pat: ast::Pattern::dummy(ast::PatternKind::Var("x".to_string())),
            expr: ast::Expr::dummy(ast::ExprKind::Integer("1".to_string())),
            span: ast::dummy_span(),
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
            doc: None,
            pat: ast::Pattern::dummy(ast::PatternKind::Var("x".to_string())),
            expr: ast::Expr::dummy(ast::ExprKind::Integer("1".to_string())),
            span: ast::dummy_span(),
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

#[cfg(test)]
mod export_table_tests {
    use super::*;

    #[test]
    fn export_table_as_name_set_works() {
        let mut table = ExportTable::new();
        table.insert("foo".to_string(), SymbolKind::Value);
        table.insert("Bar".to_string(), SymbolKind::Type);
        table.insert("Baz".to_string(), SymbolKind::Ctor);

        let names = table.as_name_set();
        assert_eq!(names.len(), 3);
        assert!(names.contains("foo"));
        assert!(names.contains("Bar"));
        assert!(names.contains("Baz"));
    }

    #[test]
    fn module_exported_names_no_export_decl() {
        let module = ast::Module {
            name: Some("Test".to_string()),
            export_specs: None,
            items: vec![
                ast::Item::Binding(ast::Binding {
                    doc: None,
                    pat: ast::Pattern::dummy(ast::PatternKind::Var("foo".to_string())),
                    expr: ast::Expr::dummy(ast::ExprKind::Integer("42".to_string())),
                    span: ast::dummy_span(),
                }),
                ast::Item::TypeAlias(ast::TypeAlias {
                    doc: None,
                    name: "MyInt".to_string(),
                    params: vec![],
                    ty: ast::Type::Var("Integer".to_string()),
                    span: ast::dummy_span(),
                }),
            ],
        };

        let table = module_exported_names(&module).unwrap();
        let names = table.as_name_set();
        assert_eq!(names.len(), 2);
        assert!(names.contains("foo"));
        assert!(names.contains("MyInt"));
    }

    #[test]
    fn module_exported_names_with_data_decl() {
        let module = ast::Module {
            name: Some("Test".to_string()),
            export_specs: None,
            items: vec![ast::Item::DataDecl(ast::DataDecl {
                doc: None,
                name: "Maybe".to_string(),
                params: vec!["a".to_string()],
                ctors: vec![
                    ast::DataCtor {
                        doc: None,
                        name: "Just".to_string(),
                        args: vec![ast::Type::Var("a".to_string())],
                        span: ast::dummy_span(),
                    },
                    ast::DataCtor {
                        doc: None,
                        name: "Nothing".to_string(),
                        args: vec![],
                        span: ast::dummy_span(),
                    },
                ],
                deriving: vec![],
                span: ast::dummy_span(),
            })],
        };

        let table = module_exported_names(&module).unwrap();
        let names = table.as_name_set();
        assert_eq!(names.len(), 3);
        assert!(names.contains("Maybe"));
        assert!(names.contains("Just"));
        assert!(names.contains("Nothing"));
    }

    #[test]
    fn module_exported_names_with_explicit_exports() {
        let module = ast::Module {
            name: Some("Test".to_string()),
            export_specs: None,
            items: vec![
                ast::Item::Binding(ast::Binding {
                    doc: None,
                    pat: ast::Pattern::dummy(ast::PatternKind::Var("foo".to_string())),
                    expr: ast::Expr::dummy(ast::ExprKind::Integer("42".to_string())),
                    span: ast::dummy_span(),
                }),
                ast::Item::Binding(ast::Binding {
                    doc: None,
                    pat: ast::Pattern::dummy(ast::PatternKind::Var("bar".to_string())),
                    expr: ast::Expr::dummy(ast::ExprKind::Integer("43".to_string())),
                    span: ast::dummy_span(),
                }),
                ast::Item::Export(ast::ExportDecl {
                    specs: vec![ast::ExportSpec::Name("foo".to_string())],
                }),
            ],
        };

        let table = module_exported_names(&module).unwrap();
        let names = table.as_name_set();
        // Only 'foo' should be exported, not 'bar'
        assert_eq!(names.len(), 1);
        assert!(names.contains("foo"));
        assert!(!names.contains("bar"));
    }

    #[test]
    fn module_exported_names_with_module_header_export_specs() {
        let module = ast::Module {
            name: Some("M".to_string()),
            export_specs: Some(vec![
                ast::ExportSpec::Name("foo".to_string()),
                ast::ExportSpec::Name("bar".to_string()),
            ]),
            items: vec![
                ast::Item::Binding(ast::Binding {
                    doc: None,
                    pat: ast::Pattern::dummy(ast::PatternKind::Var("foo".to_string())),
                    expr: ast::Expr::dummy(ast::ExprKind::Integer("1".to_string())),
                    span: ast::dummy_span(),
                }),
                ast::Item::Binding(ast::Binding {
                    doc: None,
                    pat: ast::Pattern::dummy(ast::PatternKind::Var("bar".to_string())),
                    expr: ast::Expr::dummy(ast::ExprKind::Integer("2".to_string())),
                    span: ast::dummy_span(),
                }),
                ast::Item::Binding(ast::Binding {
                    doc: None,
                    pat: ast::Pattern::dummy(ast::PatternKind::Var("baz".to_string())),
                    expr: ast::Expr::dummy(ast::ExprKind::Integer("3".to_string())),
                    span: ast::dummy_span(),
                }),
            ],
        };

        let table = module_exported_names(&module).unwrap();
        let names = table.as_name_set();
        assert_eq!(names.len(), 2, "Should export only foo and bar");
        assert!(names.contains("foo"));
        assert!(names.contains("bar"));
        assert!(!names.contains("baz"), "baz should NOT be exported");
    }
}
