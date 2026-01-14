use crate::{ast, parser, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

#[derive(Clone)]
struct CachedAst {
    modified: Option<SystemTime>,
    len: u64,
    module: ast::Module,
}

static STDLIB_AST_CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedAst>>> = OnceLock::new();

fn stdlib_ast_cache() -> &'static Mutex<HashMap<PathBuf, CachedAst>> {
    STDLIB_AST_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn stdlib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("stdlib")
}

pub(super) fn is_stdlib_path(path: &Path) -> bool {
    path.starts_with(stdlib_root())
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
