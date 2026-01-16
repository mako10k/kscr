# Test Execution Performance Optimization Strategy

## Executive Summary

Test execution time is currently too long due to repeated parsing and type checking operations. This document outlines a comprehensive caching strategy to speed up individual test execution by caching AST and type inference results per file/module/phase.

**Current Performance Characteristics:**
- Single test execution: ~5 seconds
- Full test suite: ~5+ minutes (282 tests)
- Main bottlenecks: stdlib ClassEnv loading, module import flattening, type inference

**Expected Performance After Optimization:**
- Single test execution: ~0.5-1 seconds (5-10x improvement)
- Full test suite: ~1-2 minutes (3-5x improvement)

## Current Architecture

### Test Execution Flow

```
Test → typecheck_file(path)
  ├─ ModuleLoader (empty per-call)
  │   ├─ load_ast(path)
  │   │   ├─ Check per-call cache (HashMap<Path, Module>)
  │   │   ├─ Check global stdlib AST cache [✓ CACHED]
  │   │   └─ Parse from disk [SLOW for non-stdlib]
  │   └─ collect_imports() [❌ REBUILDS every call]
  │       └─ Recursively load & qualify all imports
  │
  └─ typecheck_internal()
      ├─ Check module typecheck cache [✓ BY AST HASH]
      ├─ Expand type aliases
      ├─ Desugar typeclasses
      ├─ load_stdlib_class_env() [❌ NO CACHE - CRITICAL]
      │   └─ Walk stdlib/ + parse all .ks files
      ├─ Merge class envs
      ├─ Infer bindings (SCC-ordered)
      ├─ Simplify constraints
      ├─ Rewrite dict passing
      └─ Cache result [✓ CACHED]
```

### Existing Caching Mechanisms

The codebase already has some caching in `src/types/stdlib_cache.rs`:

| Cache | Scope | Key | Validation | Effectiveness |
|-------|-------|-----|-----------|--------------|
| **Stdlib AST Cache** | Global | File path | mtime + size | ✓ Good |
| **Module Typecheck Cache** | Global | AST hash | Hash comparison | ✓ Good |
| **ModuleLoader.cache** | Per-call | File path | None | ⚠️ Limited |

**Key Issue:** The `load_stdlib_class_env()` function is called on EVERY `typecheck_file()` invocation and walks the entire stdlib directory, parsing all `.ks` files to collect class and instance declarations. This is the primary performance bottleneck.

## Performance Bottlenecks (Prioritized)

### 1. Stdlib ClassEnv Loading - CRITICAL 🔴

**Location:** `src/types.rs:4824-4875` (`load_stdlib_class_env()`)

**Problem:**
- Called on every `typecheck_file()` invocation
- Walks `stdlib/` directory recursively
- Parses every `.ks` file in stdlib (even if already in AST cache)
- Collects all class/instance declarations
- Time complexity: O(F × C) where F = file count, C = avg declarations per file

**Current Code:**
```rust
fn load_stdlib_class_env() -> Result<ClassEnvIndex> {
    let stdlib = stdlib_root();
    let mut entries = Vec::new();
    for entry in walkdir::WalkDir::new(&stdlib) {
        let entry = entry?;
        if entry.file_type().is_file() && entry.path().extension() == Some("ks".as_ref()) {
            entries.push(entry.path().to_path_buf());
        }
    }
    entries.sort();
    
    let mut class_env = ClassEnvIndex::default();
    for p in entries {
        // Parses EVERY time, even if AST cached
        let src = std::fs::read_to_string(&p)?;
        let m = parser::parse_module(&src)?;
        // Extract classes/instances...
    }
    Ok(class_env)
}
```

**Impact:** Estimated 50-80% of test execution time

### 2. Module Typecheck Cache Hashing - HIGH 🟡

**Location:** `src/types/stdlib_cache.rs:23-33` (`hash_module_ast()`)

**Problem:**
```rust
pub(super) fn hash_module_ast(module: &ast::Module) -> u64 {
    let mut hasher = DefaultHasher::new();
    let module_str = format!("{:?}", module); // Formats entire AST!
    module_str.hash(&mut hasher);
    hasher.finish()
}
```

- Formats entire AST as debug string: O(AST size)
- Creates temporary string allocation
- Hash lookup happens on every typecheck call

**Impact:** Estimated 10-20% of cache lookup time

### 3. Import Flattening Not Cached - MEDIUM 🟡

**Location:** `src/types.rs:4538-4570` (`load_module_with_imports_ast_with_loader()`)

**Problem:**
- Creates new `ModuleLoader` with empty cache for each `typecheck_file()` call
- `collect_imports()` recursively processes all imports
- `qualify_items()` walks AST to qualify names with module prefixes
- Result is not cached between test runs

**Impact:** Estimated 10-30% for multi-module projects

### 4. Constraint Simplification - LOW 🟢

**Location:** `src/types.rs` (constraint solving)

**Problem:**
- `simplify_constraints()` processes Show/Eq/Class constraints
- Traverses data declarations recursively
- Happens once per typecheck but can be expensive for large modules

**Impact:** Estimated 5-15% for complex type hierarchies

## Proposed Caching Strategy

### Phase 1: Stdlib ClassEnv Caching (Priority 1) 🔴

**Goal:** Cache the stdlib ClassEnv globally and invalidate only on stdlib changes.

#### Implementation Approach

**1. Add Stdlib Content Hash**

Create a hash of all stdlib file contents:

```rust
// In src/types/stdlib_cache.rs

#[derive(Clone, Debug)]
struct CachedStdlibClassEnv {
    content_hash: u64,
    class_env: ClassEnvIndex,
}

static STDLIB_CLASS_ENV_CACHE: OnceLock<Mutex<Option<CachedStdlibClassEnv>>> = OnceLock::new();

fn stdlib_class_env_cache() -> &'static Mutex<Option<CachedStdlibClassEnv>> {
    STDLIB_CLASS_ENV_CACHE.get_or_init(|| Mutex::new(None))
}
```

**2. Compute Content Hash**

Hash all stdlib files efficiently:

```rust
pub(super) fn compute_stdlib_content_hash() -> Result<u64> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    
    let stdlib = stdlib_root();
    let mut entries = Vec::new();
    
    for entry in walkdir::WalkDir::new(&stdlib)
        .follow_links(false)
        .sort_by_file_name()
    {
        let entry = entry?;
        if entry.file_type().is_file() 
           && entry.path().extension() == Some("ks".as_ref()) 
        {
            entries.push(entry.path().to_path_buf());
        }
    }
    
    let mut hasher = DefaultHasher::new();
    for path in entries {
        // Hash path for structure
        path.hash(&mut hasher);
        
        // Hash metadata for quick validation
        if let Ok(meta) = std::fs::metadata(&path) {
            if let Ok(modified) = meta.modified() {
                modified.hash(&mut hasher);
            }
            meta.len().hash(&mut hasher);
        }
    }
    
    Ok(hasher.finish())
}
```

**3. Cache ClassEnv Globally**

```rust
pub(super) fn load_stdlib_class_env_cached() -> Result<ClassEnvIndex> {
    let current_hash = compute_stdlib_content_hash()?;
    
    if let Ok(mut cache) = stdlib_class_env_cache().lock() {
        if let Some(cached) = &*cache {
            if cached.content_hash == current_hash {
                return Ok(cached.class_env.clone());
            }
        }
        
        // Cache miss or invalid - rebuild
        drop(cache); // Release lock during expensive operation
        let class_env = load_stdlib_class_env_uncached()?;
        
        if let Ok(mut cache) = stdlib_class_env_cache().lock() {
            *cache = Some(CachedStdlibClassEnv {
                content_hash: current_hash,
                class_env: class_env.clone(),
            });
        }
        
        Ok(class_env)
    } else {
        // Fallback if lock fails
        load_stdlib_class_env_uncached()
    }
}

fn load_stdlib_class_env_uncached() -> Result<ClassEnvIndex> {
    // Current implementation from load_stdlib_class_env()
    // ... (lines 4824-4875)
}
```

**4. Update Call Sites**

In `src/types.rs:4590-4601` (`typecheck_with_stdlib_class_env()`):

```rust
fn typecheck_with_stdlib_class_env(mut module: ast::Module) -> Result<TypedModule> {
    // OLD: let stdlib_class_env = load_stdlib_class_env()?;
    let stdlib_class_env = stdlib_cache::load_stdlib_class_env_cached()?;
    
    inject_stdlib_class_decls(&mut module)?;
    inject_stdlib_instance_dict_forwarders(&mut module)?;
    typecheck_internal(module, Some(&stdlib_class_env))
}
```

#### Cache Invalidation Strategy

The stdlib ClassEnv cache is invalidated when:

1. **Any stdlib file changes** (detected via content hash)
2. **Stdlib files added/removed** (detected via directory walk + path hashing)
3. **Process restarts** (cache is in-memory via `OnceLock`)

For persistent cross-process caching (future enhancement):
- Serialize cache to `target/.kscr_cache/stdlib_class_env.bincode`
- Include stdlib hash in filename for validation
- Deserialize on first access if hash matches

### Phase 2: Optimize Module Typecheck Hash (Priority 2) 🟡

**Goal:** Use structural AST hashing instead of debug formatting.

#### Implementation Approach

**Option A: Implement `Hash` for AST types**

Add `#[derive(Hash)]` to AST types that need it:

```rust
// In src/ast.rs
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Module {
    pub name: Option<String>,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Item {
    // ... existing variants
}
```

Then simplify hashing:

```rust
// In src/types/stdlib_cache.rs
pub(super) fn hash_module_ast(module: &ast::Module) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    
    let mut hasher = DefaultHasher::new();
    module.hash(&mut hasher);
    hasher.finish()
}
```

**Option B: Use faster hash algorithm**

If `Hash` derivation is not feasible, use a faster hash for debug strings:

```rust
use std::hash::{BuildHasher, Hasher};

pub(super) fn hash_module_ast(module: &ast::Module) -> u64 {
    // Use FxHash (faster than DefaultHasher for strings)
    let mut hasher = rustc_hash::FxHasher::default();
    format!("{:?}", module).hash(&mut hasher);
    hasher.finish()
}
```

#### Trade-offs

| Approach | Speed | Stability | Implementation |
|----------|-------|-----------|----------------|
| Derive Hash | Fast | Stable | Requires Hash on all AST types |
| FxHash | Medium | Stable | Minimal change |
| Current | Slow | Stable | No change needed |

**Recommendation:** Start with Option B (FxHash) for quick wins, then migrate to Option A for maximum performance.

### Phase 3: Cache Import Flattening (Priority 3) 🟡

**Goal:** Memoize import-flattened modules to avoid redundant qualification.

#### Implementation Approach

**1. Add Import Flattening Cache**

```rust
// In src/types/stdlib_cache.rs

#[derive(Clone)]
struct CachedFlattenedImport {
    /// Hash of the source module + import declarations
    source_hash: u64,
    /// The flattened items with qualified names
    items: Vec<ast::Item>,
}

static IMPORT_FLATTEN_CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedFlattenedImport>>> = 
    OnceLock::new();

fn import_flatten_cache() -> &'static Mutex<HashMap<PathBuf, CachedFlattenedImport>> {
    IMPORT_FLATTEN_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}
```

**2. Cache Key Generation**

Hash the module AST + import declarations:

```rust
fn hash_module_with_imports(module: &ast::Module) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    
    let mut hasher = DefaultHasher::new();
    
    // Hash module identity
    module.name.hash(&mut hasher);
    
    // Hash import declarations
    for item in &module.items {
        if let ast::Item::Import(import) = item {
            import.module.hash(&mut hasher);
            import.qualified.hash(&mut hasher);
            import.as_name.hash(&mut hasher);
        }
    }
    
    // Hash non-import items efficiently
    // Note: If Item implements Hash, this can be simplified to just hash each item directly
    for item in &module.items {
        if !matches!(item, ast::Item::Import(_)) {
            // TODO: Implement Hash for ast::Item or use structural hashing
            format!("{:?}", item).hash(&mut hasher);
        }
    }
    
    hasher.finish()
}
```

**3. Check and Update Cache**

```rust
pub(super) fn check_import_flatten_cache(
    path: &Path,
    module: &ast::Module,
) -> Option<Vec<ast::Item>> {
    let hash = hash_module_with_imports(module);
    
    if let Ok(cache) = import_flatten_cache().lock() {
        if let Some(cached) = cache.get(path) {
            if cached.source_hash == hash {
                return Some(cached.items.clone());
            }
        }
    }
    None
}

pub(super) fn store_import_flatten_cache(
    path: &Path,
    module: &ast::Module,
    items: &[ast::Item],
) {
    let hash = hash_module_with_imports(module);
    
    if let Ok(mut cache) = import_flatten_cache().lock() {
        cache.insert(
            path.to_path_buf(),
            CachedFlattenedImport {
                source_hash: hash,
                items: items.to_vec(),
            },
        );
    }
}
```

**4. Integrate with ModuleLoader**

In `src/types.rs` (`load_module_with_imports_ast_with_loader()`):

```rust
fn load_module_with_imports_ast_with_loader(
    loader: &mut ModuleLoader,
    entry: &Path,
    entry_dir: &Path,
    entry_mod: &ast::Module,
) -> Result<ast::Module> {
    // Check cache first
    if let Some(cached_items) = stdlib_cache::check_import_flatten_cache(entry, entry_mod) {
        return Ok(ast::Module {
            name: entry_mod.name.clone(),
            items: cached_items,
        });
    }
    
    // Original logic...
    let mut items = Vec::new();
    let mut defined: HashMap<String, String> = HashMap::new();
    let mut deps = Vec::new();
    loader.collect_imports(entry_mod, entry_dir, &mut deps)?;
    
    for it in deps {
        push_item_checked(&mut items, &mut defined, it)?;
    }
    
    for it in entry_mod.items.clone() {
        if matches!(it, ast::Item::Import(_)) {
            continue;
        }
        push_item_checked(&mut items, &mut defined, it)?;
    }
    
    // Store result
    stdlib_cache::store_import_flatten_cache(entry, entry_mod, &items);
    
    Ok(ast::Module {
        name: entry_mod.name.clone(),
        items,
    })
}
```

#### Cache Invalidation Strategy

The import flattening cache is invalidated when:

1. **Source module changes** (detected via hash)
2. **Import declarations change** (detected via hash)
3. **Imported modules change** (detected via transitive hash)

### Phase 4: Cache Constraint Simplification (Priority 4) 🟢

**Goal:** Memoize derived instance constraints (Show, Eq, Ord).

#### Implementation Approach

This is a lower-priority optimization. The basic approach:

```rust
// In src/types/stdlib_cache.rs

#[derive(Clone)]
struct CachedDataEnvDerivations {
    data_hash: u64,
    derived_constraints: Vec<(String, Vec<types::Constraint>)>,
}

static DATA_ENV_CACHE: OnceLock<Mutex<HashMap<String, CachedDataEnvDerivations>>> = 
    OnceLock::new();
```

Cache key: hash of data type definition
Cache value: pre-computed Show/Eq/Ord constraints

## Implementation Roadmap

### Step 1: Measure Baseline ⏱️

Before implementing any caching, establish baseline metrics:

```bash
# Run a subset of tests with timing
cargo test --lib cli_impl::tests -- --nocapture 2>&1 | tee test_baseline.log

# Count test execution time
time cargo test --lib cli_impl::tests::cli_run_command_smoke

# Profile with cargo-flamegraph (if available)
cargo flamegraph --test kscr -- cli_impl::tests::cli_run_command_smoke
```

Expected baseline:
- Single test: 3-5 seconds
- 50 CLI tests: 150-250 seconds

### Step 2: Implement Phase 1 (Stdlib ClassEnv Cache) 🔴

**Files to modify:**
1. `src/types/stdlib_cache.rs` - Add `CachedStdlibClassEnv`, hash functions
2. `src/types.rs` - Update `typecheck_with_stdlib_class_env()` call site

**Testing approach:**
```rust
#[test]
fn stdlib_class_env_cache_works() {
    let env1 = stdlib_cache::load_stdlib_class_env_cached().unwrap();
    let env2 = stdlib_cache::load_stdlib_class_env_cached().unwrap();
    // Second call should be instant (cached)
    assert_eq!(env1, env2);
}

#[test]
fn stdlib_class_env_cache_invalidates_on_change() {
    let env1 = stdlib_cache::load_stdlib_class_env_cached().unwrap();
    
    // Simulate stdlib change (in test only)
    // ... modify a stdlib file ...
    
    let env2 = stdlib_cache::load_stdlib_class_env_cached().unwrap();
    // Should detect change and reload
}
```

**Expected improvement:** 3-5x speedup (single test: 1-1.5 seconds)

### Step 3: Implement Phase 2 (Optimize Hash) 🟡

**Files to modify:**
1. `src/types/stdlib_cache.rs` - Update `hash_module_ast()`
2. Optionally `src/ast.rs` - Add `#[derive(Hash)]`

**Testing approach:**
```rust
#[test]
fn module_hash_is_stable() {
    let src = "module Main where\n  x = 1\n";
    let m1 = parser::parse_module(src).unwrap();
    let m2 = parser::parse_module(src).unwrap();
    assert_eq!(hash_module_ast(&m1), hash_module_ast(&m2));
}

#[test]
fn module_hash_changes_on_content_change() {
    let m1 = parser::parse_module("x = 1").unwrap();
    let m2 = parser::parse_module("x = 2").unwrap();
    assert_ne!(hash_module_ast(&m1), hash_module_ast(&m2));
}
```

**Expected improvement:** Additional 10-20% speedup

### Step 4: Implement Phase 3 (Import Flattening) 🟡

**Files to modify:**
1. `src/types/stdlib_cache.rs` - Add import flatten cache
2. `src/types.rs` - Update `load_module_with_imports_ast_with_loader()`

**Testing approach:**
```rust
#[test]
fn import_flatten_cache_works() {
    // Create temp module with imports
    let dir = temp_dir().join("kscr_test_import_cache");
    std::fs::create_dir_all(&dir).unwrap();
    
    std::fs::write(dir.join("A.ks"), "module A where\n  x = 1").unwrap();
    std::fs::write(
        dir.join("Main.ks"),
        "module Main where\n  import A\n  main = IO ()"
    ).unwrap();
    
    // First typecheck - populate cache
    let t1 = std::time::Instant::now();
    typecheck_file(&dir.join("Main.ks")).unwrap();
    let d1 = t1.elapsed();
    
    // Second typecheck - should use cache
    let t2 = std::time::Instant::now();
    typecheck_file(&dir.join("Main.ks")).unwrap();
    let d2 = t2.elapsed();
    
    // Second should be faster
    assert!(d2 < d1);
}
```

**Expected improvement:** Additional 10-30% for multi-module tests

### Step 5: Measure Final Performance ⏱️

Run the same benchmarks from Step 1:

```bash
time cargo test --lib cli_impl::tests::cli_run_command_smoke
```

Expected final performance:
- Single test: 0.5-1 second (5-10x improvement)
- 50 CLI tests: 25-50 seconds (3-5x improvement)
- 282 total tests: 1-2 minutes (3-5x improvement)

## Cache Management

### Memory Usage

Estimated memory footprint per cache:

| Cache | Size per entry | Max entries | Total |
|-------|---------------|-------------|-------|
| Stdlib AST | ~50-500 KB | ~50 files | 2-25 MB |
| Stdlib ClassEnv | ~100-500 KB | 1 | 100-500 KB |
| Module Typecheck | ~10-100 KB | ~100 modules | 1-10 MB |
| Import Flatten | ~50-200 KB | ~50 modules | 2-10 MB |

**Total estimated:** 5-45 MB (acceptable for development)

### Cache Clearing

For development, add CLI command to clear caches:

```rust
// In src/cli_impl.rs
"clear-cache" => {
    types::stdlib_cache::clear_all_caches();
    println!("All caches cleared.");
    Ok(())
}
```

```rust
// In src/types/stdlib_cache.rs
pub fn clear_all_caches() {
    if let Ok(mut cache) = stdlib_ast_cache().lock() {
        cache.clear();
    }
    if let Ok(mut cache) = module_typecheck_cache().lock() {
        cache.clear();
    }
    if let Ok(mut cache) = stdlib_class_env_cache().lock() {
        *cache = None;
    }
    // ... clear other caches
}
```

### Environment Variables

Add debugging environment variables:

```bash
# Disable all caching
KSCR_NO_CACHE=1 cargo test

# Show cache statistics
KSCR_CACHE_STATS=1 cargo test

# Clear cache before run
KSCR_CLEAR_CACHE=1 cargo test
```

## Testing Strategy

### Unit Tests

Each cache implementation should have:

1. **Cache hit test** - Verify cached results match fresh computation
2. **Cache miss test** - Verify cache correctly detects changes
3. **Cache invalidation test** - Verify cache invalidates on relevant changes
4. **Concurrency test** - Verify cache works with parallel test execution

### Integration Tests

Run existing test suite with caching enabled:

```bash
# Run all tests - should pass with caching
cargo test

# Run with cache disabled - should still pass
KSCR_NO_CACHE=1 cargo test

# Run with cache stats - measure hit rate
KSCR_CACHE_STATS=1 cargo test 2>&1 | tee cache_stats.log
```

### Performance Tests

Create dedicated performance benchmarks:

```rust
#[test]
fn bench_typecheck_file_cold_cache() {
    stdlib_cache::clear_all_caches();
    let start = std::time::Instant::now();
    typecheck_file("tests/stdlib_classes_smoke.ks").unwrap();
    println!("Cold cache: {:?}", start.elapsed());
}

#[test]
fn bench_typecheck_file_warm_cache() {
    // Pre-warm cache
    typecheck_file("tests/stdlib_classes_smoke.ks").unwrap();
    
    let start = std::time::Instant::now();
    typecheck_file("tests/stdlib_classes_smoke.ks").unwrap();
    println!("Warm cache: {:?}", start.elapsed());
}
```

## Risks and Mitigations

### Risk 1: Cache Invalidation Bugs

**Risk:** Cache not invalidated when source changes, leading to stale results.

**Mitigation:**
- Use file metadata (mtime + size) for validation
- Add content hashing for critical caches
- Provide `KSCR_NO_CACHE` escape hatch
- Add cache version number to detect format changes

### Risk 2: Memory Exhaustion

**Risk:** Caches grow unbounded in long-running processes.

**Mitigation:**
- Implement LRU eviction for large caches
- Set maximum cache size limits
- Provide manual cache clearing command
- Cache size is acceptable for CLI/test usage (~5-45 MB)

### Risk 3: Hash Collisions

**Risk:** Different ASTs hash to same value, causing incorrect cache hits.

**Mitigation:**
- Use 64-bit hashes (collision probability ~50% after 2^32 entries per birthday paradox)
- For critical caches, validate content after hash match
- Use cryptographic hash (SHA-256) for stdlib ClassEnv if needed

### Risk 4: Breaking Existing Tests

**Risk:** Caching changes break existing test behavior.

**Mitigation:**
- All tests must pass with `KSCR_NO_CACHE=1`
- All tests must pass with caching enabled
- Add cache-specific tests
- Review test output carefully for differences

### Risk 5: Platform-Specific Issues

**Risk:** Cache invalidation behaves differently on Windows/Linux/macOS.

**Mitigation:**
- Use `std::fs::metadata()` which is cross-platform
- Test on multiple platforms in CI
- Handle platform-specific errors gracefully
- Use canonical paths consistently

## Future Enhancements

### Persistent Cache (Disk-Based)

For very large projects, serialize caches to disk:

```rust
// In src/types/stdlib_cache.rs
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct PersistentCache {
    version: u32,
    stdlib_class_env: Option<CachedStdlibClassEnv>,
    module_typecheck: HashMap<u64, CachedModuleTypecheck>,
}

fn cache_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("target/.kscr_cache")
}

fn load_persistent_cache() -> Option<PersistentCache> {
    let path = cache_dir().join("cache.bincode");
    let bytes = std::fs::read(&path).ok()?;
    bincode::deserialize(&bytes).ok()
}

fn save_persistent_cache(cache: &PersistentCache) {
    let path = cache_dir().join("cache.bincode");
    std::fs::create_dir_all(cache_dir()).ok();
    if let Ok(bytes) = bincode::serialize(cache) {
        let _ = std::fs::write(&path, bytes);
    }
}
```

### Incremental Compilation

Track dependencies between modules:

```rust
struct ModuleDependencies {
    imports: Vec<PathBuf>,
    hash: u64,
}

fn compute_transitive_hash(path: &Path, deps: &HashMap<PathBuf, ModuleDependencies>) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    
    if let Some(dep) = deps.get(path) {
        dep.hash.hash(&mut hasher);
        for import in &dep.imports {
            compute_transitive_hash(import, deps).hash(&mut hasher);
        }
    }
    
    hasher.finish()
}
```

Only recompile modules whose transitive dependencies changed.

### Parallel Typecheck

Process independent modules in parallel:

```rust
use rayon::prelude::*;

fn typecheck_modules_parallel(modules: Vec<PathBuf>) -> Vec<Result<TypedModule>> {
    modules.par_iter()
        .map(|path| typecheck_file(path))
        .collect()
}
```

Requires thread-safe caches (already using `Mutex`).

### Watch Mode

Implement file watching for development:

```bash
kscr watch tests/
# Automatically re-runs tests when files change
# Uses caching for unchanged modules
```

## Appendix A: Profiling Commands

```bash
# Install profiling tools
cargo install flamegraph
cargo install cargo-instruments  # macOS only

# Profile single test
cargo flamegraph --test kscr -- cli_impl::tests::cli_run_command_smoke

# Profile full test suite
cargo build --tests
perf record --call-graph dwarf ./target/debug/deps/kscr-<hash>
perf report

# Memory profiling (requires valgrind)
cargo build --tests
valgrind --tool=massif ./target/debug/deps/kscr-<hash>
ms_print massif.out.*
```

## Appendix B: Cache Statistics

Example implementation of cache statistics:

```rust
// In src/types/stdlib_cache.rs

#[derive(Default)]
struct CacheStats {
    stdlib_ast_hits: AtomicUsize,
    stdlib_ast_misses: AtomicUsize,
    stdlib_class_env_hits: AtomicUsize,
    stdlib_class_env_misses: AtomicUsize,
    module_typecheck_hits: AtomicUsize,
    module_typecheck_misses: AtomicUsize,
}

static CACHE_STATS: OnceLock<CacheStats> = OnceLock::new();

pub fn cache_stats() -> &'static CacheStats {
    CACHE_STATS.get_or_init(Default::default)
}

pub fn print_cache_stats() {
    let stats = cache_stats();
    eprintln!("Cache Statistics:");
    eprintln!("  Stdlib AST: {} hits, {} misses", 
        stats.stdlib_ast_hits.load(Ordering::Relaxed),
        stats.stdlib_ast_misses.load(Ordering::Relaxed));
    eprintln!("  Stdlib ClassEnv: {} hits, {} misses", 
        stats.stdlib_class_env_hits.load(Ordering::Relaxed),
        stats.stdlib_class_env_misses.load(Ordering::Relaxed));
    eprintln!("  Module Typecheck: {} hits, {} misses", 
        stats.module_typecheck_hits.load(Ordering::Relaxed),
        stats.module_typecheck_misses.load(Ordering::Relaxed));
}
```

Register atexit handler to print stats:

```rust
// In src/lib.rs
#[cfg(test)]
#[dtor]
fn print_cache_stats_on_exit() {
    if std::env::var("KSCR_CACHE_STATS").is_ok() {
        types::stdlib_cache::print_cache_stats();
    }
}
```

## Appendix C: References

- **Haskell GHC Interface Files:** `.hi` files cache type signatures and inlining info
- **Rust Incremental Compilation:** `target/debug/incremental/` directory structure
- **OCaml Compiled Modules:** `.cmi` interface files, `.cmo` bytecode
- **Language Server Protocol:** Maintains in-memory AST and type caches

## Conclusion

This caching strategy provides a systematic approach to optimize test execution time by:

1. **Caching stdlib ClassEnv globally** (50-80% speedup) - Priority 1
2. **Optimizing hash computation** (10-20% speedup) - Priority 2
3. **Caching import flattening** (10-30% speedup) - Priority 3
4. **Caching constraint simplification** (5-15% speedup) - Priority 4

Expected overall improvement: **5-10x faster individual test execution**, **3-5x faster full test suite**.

The strategy is conservative, incremental, and maintains correctness through careful cache invalidation. Each phase can be implemented and validated independently.
