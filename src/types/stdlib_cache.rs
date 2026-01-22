use crate::{ast, parser, Result};
use dirs;
use include_dir::{include_dir, Dir, DirEntry};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

#[derive(Clone)]
struct CachedAst {
    modified: Option<SystemTime>,
    len: u64,
    module: ast::Module,
}

#[derive(Clone)]
struct CachedModuleTypecheck {
    schemes: HashMap<String, super::Scheme>,
}

static STDLIB_AST_CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedAst>>> = OnceLock::new();
static MODULE_TYPECHECK_CACHE: OnceLock<Mutex<HashMap<u64, CachedModuleTypecheck>>> =
    OnceLock::new();

static EMBED_STDLIB: Dir<'_> = include_dir!("stdlib");

#[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
pub(super) fn hash_module_ast(module: &ast::Module) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    // Fast structural hash; avoids allocating a giant debug string every time.
    fn hash_str(h: &mut DefaultHasher, s: &str) {
        s.hash(h);
    }

    fn hash_resolved_name(h: &mut DefaultHasher, n: &ast::ResolvedName) {
        match n {
            ast::ResolvedName::Unresolved(s) => {
                0u8.hash(h);
                hash_str(h, s);
            }
            ast::ResolvedName::Resolved {
                module,
                module_name,
                name,
            } => {
                1u8.hash(h);
                module.hash(h);
                hash_str(h, module_name);
                hash_str(h, name);
            }
        }
    }

    fn hash_type(h: &mut DefaultHasher, t: &ast::Type) {
        use ast::Type;
        match t {
            Type::Unit => 0u8.hash(h),
            Type::Integer => 1u8.hash(h),
            Type::Bool => 2u8.hash(h),
            Type::Float64 => 3u8.hash(h),
            Type::Char => 4u8.hash(h),
            Type::String => 5u8.hash(h),
            Type::List(a) => {
                6u8.hash(h);
                hash_type(h, a);
            }
            Type::Tuple(ts) => {
                7u8.hash(h);
                ts.len().hash(h);
                for x in ts {
                    hash_type(h, x);
                }
            }
            Type::Record(fs) => {
                8u8.hash(h);
                fs.len().hash(h);
                for (l, x) in fs {
                    hash_str(h, l);
                    hash_type(h, x);
                }
            }
            Type::RecordOpen(fs, rest) => {
                9u8.hash(h);
                fs.len().hash(h);
                for (l, x) in fs {
                    hash_str(h, l);
                    hash_type(h, x);
                }
                hash_type(h, rest);
            }
            Type::Hole(n) => {
                10u8.hash(h);
                n.as_deref().hash(h);
            }
            Type::Var(s) => {
                11u8.hash(h);
                hash_str(h, s);
            }
            Type::App { head, args } => {
                12u8.hash(h);
                hash_type(h, head);
                args.len().hash(h);
                for a in args {
                    hash_type(h, a);
                }
            }
            Type::Func(a, b) => {
                13u8.hash(h);
                hash_type(h, a);
                hash_type(h, b);
            }
        }
    }

    fn hash_pred(h: &mut DefaultHasher, p: &ast::Predicate) {
        use ast::Predicate;
        match p {
            Predicate::Show(t) => {
                0u8.hash(h);
                hash_type(h, t);
            }
            Predicate::ShowRow(t) => {
                1u8.hash(h);
                hash_type(h, t);
            }
            Predicate::Eq(t) => {
                2u8.hash(h);
                hash_type(h, t);
            }
            Predicate::EqRow(t) => {
                3u8.hash(h);
                hash_type(h, t);
            }
            Predicate::Class { class, ty } => {
                4u8.hash(h);
                hash_str(h, class);
                hash_type(h, ty);
            }
            Predicate::Lacks { label, row } => {
                5u8.hash(h);
                hash_str(h, label);
                hash_type(h, row);
            }
        }
    }

    fn hash_qual_type(h: &mut DefaultHasher, qt: &ast::QualType) {
        qt.preds.len().hash(h);
        for p in &qt.preds {
            hash_pred(h, p);
        }
        hash_type(h, &qt.ty);
    }

    #[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
    fn hash_expr(h: &mut DefaultHasher, e: &ast::Expr) {
        use ast::ExprKind;
        match &e.kind {
            ExprKind::Unit => 0u8.hash(h),
            ExprKind::Integer(s) => {
                1u8.hash(h);
                hash_str(h, s);
            }
            ExprKind::Float64(s) => {
                2u8.hash(h);
                hash_str(h, s);
            }
            ExprKind::Bool(b) => {
                3u8.hash(h);
                b.hash(h);
            }
            ExprKind::String(s) => {
                4u8.hash(h);
                hash_str(h, s);
            }
            ExprKind::Char(c) => {
                5u8.hash(h);
                c.hash(h);
            }
            ExprKind::Var(s) => {
                6u8.hash(h);
                hash_str(h, s);
            }
            ExprKind::Ctor(n) => {
                7u8.hash(h);
                hash_resolved_name(h, n);
            }
            ExprKind::Lambda { params, body } => {
                8u8.hash(h);
                params.len().hash(h);
                for p in params {
                    hash_str(h, p);
                }
                hash_expr(h, body);
            }
            ExprKind::Apply { func, args } => {
                9u8.hash(h);
                hash_expr(h, func);
                args.len().hash(h);
                for a in args {
                    hash_expr(h, a);
                }
            }
            ExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                10u8.hash(h);
                hash_expr(h, cond);
                hash_expr(h, then_branch);
                hash_expr(h, else_branch);
            }
            ExprKind::Let { bindings, body } => {
                11u8.hash(h);
                bindings.len().hash(h);
                for b in bindings {
                    hash_pat(h, &b.pat);
                    hash_expr(h, &b.expr);
                }
                hash_expr(h, body);
            }
            ExprKind::Where { expr, bindings } => {
                12u8.hash(h);
                hash_expr(h, expr);
                bindings.len().hash(h);
                for b in bindings {
                    hash_pat(h, &b.pat);
                    hash_expr(h, &b.expr);
                }
            }
            ExprKind::Annot { expr, ty } => {
                13u8.hash(h);
                hash_expr(h, expr);
                hash_qual_type(h, ty);
            }
            ExprKind::Do(stmts) => {
                14u8.hash(h);
                stmts.len().hash(h);
                for s in stmts {
                    match s {
                        ast::DoStmt::Bind { pat, expr } => {
                            0u8.hash(h);
                            hash_pat(h, pat);
                            hash_expr(h, expr);
                        }
                        ast::DoStmt::Expr(e) => {
                            1u8.hash(h);
                            hash_expr(h, e);
                        }
                    }
                }
            }
            ExprKind::Case { expr, arms } => {
                15u8.hash(h);
                hash_expr(h, expr);
                arms.len().hash(h);
                for a in arms {
                    hash_pat(h, &a.pat);
                    if let Some(g) = &a.guard {
                        1u8.hash(h);
                        hash_expr(h, g);
                    } else {
                        0u8.hash(h);
                    }
                    hash_expr(h, &a.body);
                }
            }
            ExprKind::Cons { head, tail } => {
                16u8.hash(h);
                hash_expr(h, head);
                hash_expr(h, tail);
            }
            ExprKind::List(es) => {
                17u8.hash(h);
                es.len().hash(h);
                for x in es {
                    hash_expr(h, x);
                }
            }
            ExprKind::Tuple(es) => {
                18u8.hash(h);
                es.len().hash(h);
                for x in es {
                    hash_expr(h, x);
                }
            }
            ExprKind::Record(fs) => {
                19u8.hash(h);
                fs.len().hash(h);
                for (l, x) in fs {
                    hash_str(h, l);
                    hash_expr(h, x);
                }
            }
        }
    }

    fn hash_pat(h: &mut DefaultHasher, p: &ast::Pattern) {
        use ast::PatternKind;
        match &p.kind {
            PatternKind::Var(s) => {
                0u8.hash(h);
                hash_str(h, s);
            }
            PatternKind::Wildcard => 1u8.hash(h),
            PatternKind::Hole(n) => {
                2u8.hash(h);
                n.as_deref().hash(h);
            }
            PatternKind::Literal(e) => {
                3u8.hash(h);
                hash_expr(h, e);
            }
            PatternKind::Tuple(ps) => {
                4u8.hash(h);
                ps.len().hash(h);
                for x in ps {
                    hash_pat(h, x);
                }
            }
            PatternKind::List(ps) => {
                5u8.hash(h);
                ps.len().hash(h);
                for x in ps {
                    hash_pat(h, x);
                }
            }
            PatternKind::Record(fs) => {
                6u8.hash(h);
                fs.len().hash(h);
                for (l, x) in fs {
                    hash_str(h, l);
                    hash_pat(h, x);
                }
            }
            PatternKind::RecordLoose(fs, rest) => {
                7u8.hash(h);
                fs.len().hash(h);
                for (l, x) in fs {
                    hash_str(h, l);
                    hash_pat(h, x);
                }
                rest.as_deref().hash(h);
            }
            PatternKind::Cons(a, b) => {
                8u8.hash(h);
                hash_pat(h, a);
                hash_pat(h, b);
            }
            PatternKind::Or(a, b) => {
                9u8.hash(h);
                hash_pat(h, a);
                hash_pat(h, b);
            }
            PatternKind::As(s, p) => {
                10u8.hash(h);
                hash_str(h, s);
                hash_pat(h, p);
            }
            PatternKind::View(p, e) => {
                11u8.hash(h);
                hash_pat(h, p);
                hash_expr(h, e);
            }
            PatternKind::Constructor { name, args } => {
                12u8.hash(h);
                hash_resolved_name(h, name);
                args.len().hash(h);
                for a in args {
                    hash_pat(h, a);
                }
            }
        }
    }

    let mut hasher = DefaultHasher::new();
    module.name.as_deref().hash(&mut hasher);
    module.items.len().hash(&mut hasher);
    for it in &module.items {
        use ast::Item;
        match it {
            Item::Import(i) => {
                0u8.hash(&mut hasher);
                hash_str(&mut hasher, &i.module);
                i.qualified.hash(&mut hasher);
                i.as_name.as_deref().hash(&mut hasher);
            }
            Item::Export(e) => {
                1u8.hash(&mut hasher);
                e.specs.len().hash(&mut hasher);
                for s in &e.specs {
                    match s {
                        ast::ExportSpec::Name(name) => {
                            0u8.hash(&mut hasher);
                            hash_str(&mut hasher, name);
                        }
                        ast::ExportSpec::Type { name, ctors } => {
                            1u8.hash(&mut hasher);
                            hash_str(&mut hasher, name);
                            match ctors {
                                ast::ExportCtors::All => 0u8.hash(&mut hasher),
                                ast::ExportCtors::Some(cs) => {
                                    1u8.hash(&mut hasher);
                                    cs.len().hash(&mut hasher);
                                    for c in cs {
                                        hash_str(&mut hasher, c);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Item::Fixity(f) => {
                2u8.hash(&mut hasher);
                std::mem::discriminant(&f.assoc).hash(&mut hasher);
                f.prec.hash(&mut hasher);
                f.ops.len().hash(&mut hasher);
                for op in &f.ops {
                    hash_str(&mut hasher, op);
                }
            }
            Item::Binding(b) => {
                3u8.hash(&mut hasher);
                hash_pat(&mut hasher, &b.pat);
                hash_expr(&mut hasher, &b.expr);
            }
            Item::TypeAlias(ta) => {
                4u8.hash(&mut hasher);
                hash_str(&mut hasher, &ta.name);
                ta.params.len().hash(&mut hasher);
                for p in &ta.params {
                    hash_str(&mut hasher, p);
                }
                hash_type(&mut hasher, &ta.ty);
            }
            Item::DataDecl(d) => {
                5u8.hash(&mut hasher);
                hash_str(&mut hasher, &d.name);
                d.params.len().hash(&mut hasher);
                for p in &d.params {
                    hash_str(&mut hasher, p);
                }
                d.ctors.len().hash(&mut hasher);
                for c in &d.ctors {
                    hash_str(&mut hasher, &c.name);
                    c.args.len().hash(&mut hasher);
                    for a in &c.args {
                        hash_type(&mut hasher, a);
                    }
                }
                d.deriving.len().hash(&mut hasher);
                for x in &d.deriving {
                    hash_str(&mut hasher, x);
                }
            }
            Item::ClassDecl(c) => {
                6u8.hash(&mut hasher);
                hash_str(&mut hasher, &c.name);
                hash_str(&mut hasher, &c.param);
                c.supers.len().hash(&mut hasher);
                for p in &c.supers {
                    hash_pred(&mut hasher, p);
                }
                c.methods.len().hash(&mut hasher);
                for m in &c.methods {
                    hash_str(&mut hasher, &m.name);
                    hash_qual_type(&mut hasher, &m.ty);
                }
                c.default_methods.len().hash(&mut hasher);
                for b in &c.default_methods {
                    hash_pat(&mut hasher, &b.pat);
                    hash_expr(&mut hasher, &b.expr);
                }
            }
            Item::InstanceDecl(i) => {
                7u8.hash(&mut hasher);
                i.preds.len().hash(&mut hasher);
                for p in &i.preds {
                    hash_pred(&mut hasher, p);
                }
                hash_str(&mut hasher, &i.class);
                hash_type(&mut hasher, &i.ty);
                i.methods.len().hash(&mut hasher);
                for b in &i.methods {
                    hash_pat(&mut hasher, &b.pat);
                    hash_expr(&mut hasher, &b.expr);
                }
            }
        }
    }

    hasher.finish()
}

pub(super) fn check_module_typecheck_cache(
    module: &ast::Module,
) -> Option<HashMap<String, super::Scheme>> {
    let hash = hash_module_ast(module);
    if let Ok(cache) = module_typecheck_cache().lock() {
        if let Some(cached) = cache.get(&hash) {
            return Some(cached.schemes.clone());
        }
    }
    None
}

#[allow(dead_code)]
pub(super) fn store_module_typecheck_cache(
    module: &ast::Module,
    schemes: &HashMap<String, super::Scheme>,
) {
    let hash = hash_module_ast(module);
    if let Ok(mut cache) = module_typecheck_cache().lock() {
        cache.insert(
            hash,
            CachedModuleTypecheck {
                schemes: schemes.clone(),
            },
        );
    }
}

fn stdlib_ast_cache() -> &'static Mutex<HashMap<PathBuf, CachedAst>> {
    STDLIB_AST_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn module_typecheck_cache() -> &'static Mutex<HashMap<u64, CachedModuleTypecheck>> {
    MODULE_TYPECHECK_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn stdlib_root() -> Result<PathBuf> {
    resolve_stdlib_root()
}

pub(super) fn set_stdlib_root_override(path: PathBuf) {
    let slot = STDLIB_ROOT_OVERRIDE.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = slot.lock() {
        *g = Some(path);
    }
}

fn stdlib_root_override() -> Option<PathBuf> {
    let slot = STDLIB_ROOT_OVERRIDE.get_or_init(|| Mutex::new(None));
    slot.lock().ok().and_then(|g| g.clone())
}

static STDLIB_ROOT_OVERRIDE: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

fn extract_embedded_to(dest: &Path) -> std::io::Result<()> {
    for entry in EMBED_STDLIB.entries() {
        match entry {
            DirEntry::Dir(d) => {
                let dir_path = dest.join(d.path());
                fs::create_dir_all(&dir_path)?;
                // Recurse into sub-entries
                for sub in d.entries() {
                    match sub {
                        DirEntry::Dir(sd) => {
                            let sub_dir = dest.join(sd.path());
                            fs::create_dir_all(&sub_dir)?;
                        }
                        DirEntry::File(f) => {
                            let path = dest.join(f.path());
                            if let Some(parent) = path.parent() {
                                fs::create_dir_all(parent)?;
                            }
                            fs::write(path, f.contents())?;
                        }
                    }
                }
            }
            DirEntry::File(f) => {
                let path = dest.join(f.path());
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(path, f.contents())?;
            }
        }
    }
    Ok(())
}

pub(super) fn install_embedded_stdlib() -> Result<PathBuf> {
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    let dest = base.join("kscr").join("stdlib");

    if is_valid_stdlib_root(&dest) {
        return Ok(dest);
    }

    if let Err(e) = extract_embedded_to(&dest) {
        return Err(crate::error::Error::msg(format!(
            "failed to extract embedded stdlib: {}",
            e
        )));
    }

    if is_valid_stdlib_root(&dest) {
        Ok(dest)
    } else {
        Err(crate::error::Error::msg(
            "embedded stdlib extraction did not produce a valid stdlib".to_string(),
        ))
    }
}

fn resolve_stdlib_root() -> Result<PathBuf> {
    let mut tried: Vec<PathBuf> = Vec::new();

    // 1) CLI override: --stdlib-dir <path>
    if let Some(p) = stdlib_root_override() {
        tried.push(p.clone());
        if is_valid_stdlib_root(&p) {
            return Ok(p);
        }
    }

    // 2) Env: KSCR_STDLIB_DIR
    if let Ok(s) = std::env::var("KSCR_STDLIB_DIR") {
        if !s.trim().is_empty() {
            let p = PathBuf::from(s);
            tried.push(p.clone());
            if is_valid_stdlib_root(&p) {
                return Ok(p);
            }
        }
    }

    // 3) $EXE_DIR/stdlib
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("stdlib");
            tried.push(p.clone());
            if is_valid_stdlib_root(&p) {
                return Ok(p);
            }
        }
    }

    // 4) (dev/test only) CARGO_MANIFEST_DIR/stdlib
    if cfg!(any(test, debug_assertions)) {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("stdlib");
        tried.push(p.clone());
        if is_valid_stdlib_root(&p) {
            return Ok(p);
        }
    }

    // 5) Embedded stdlib: extract to user data dir (e.g. $XDG_DATA_HOME/kscr/stdlib)
    if let Ok(p) = install_embedded_stdlib() {
        tried.push(p.clone());
        if is_valid_stdlib_root(&p) {
            return Ok(p);
        }
    }

    Err(crate::error::Error::msg(format!(
        "cannot find stdlib root (tried: {}). Hint: pass --stdlib-dir <path>, set KSCR_STDLIB_DIR, or place stdlib next to the kscr executable ($EXE_DIR/stdlib).",
        tried
            .into_iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

fn is_valid_stdlib_root(dir: &Path) -> bool {
    dir.join("Prelude.ks").is_file()
}

pub(super) fn is_stdlib_path(path: &Path) -> bool {
    stdlib_root()
        .ok()
        .is_some_and(|root| path.starts_with(root))
}

pub(super) fn load_ast_stdlib_cached(path: &Path) -> Result<Option<ast::Module>> {
    if !is_stdlib_path(path) {
        return Ok(None);
    }

    let meta = std::fs::metadata(path)?;
    let fingerprint_modified = meta.modified().ok();
    let fingerprint_len = meta.len();

    if let Ok(cache) = stdlib_ast_cache().lock() {
        if let Some(cached) = cache.get(path) {
            if cached.len == fingerprint_len && cached.modified == fingerprint_modified {
                return Ok(Some(cached.module.clone()));
            }
        }
    }

    // Parse outside the lock.
    let src = std::fs::read_to_string(path)?;
    let mut m = parser::parse_module(&src)?;
    super::desugar_module_qualified_names(&mut m)?;

    if let Ok(mut cache) = stdlib_ast_cache().lock() {
        cache.insert(
            path.to_path_buf(),
            CachedAst {
                modified: fingerprint_modified,
                len: fingerprint_len,
                module: m.clone(),
            },
        );
    }

    Ok(Some(m))
}
