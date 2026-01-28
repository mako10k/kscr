# Implementation Plan: KSIF Dependency Tracking and Cyclic SCC Resolution

## 1. Summary

Extend the `.ksif` format to include dependency content hashes and implement cyclic dependency resolution using iterative fixpoint solving for Strongly Connected Components (SCCs) in the module import graph. This enables incremental compilation by detecting when dependencies change and handling mutual recursion between modules.

**Key Requirements:**
- `.ksif` files remember dependency `.ksif` content hashes
- Detect cycles in module import graph at `.ksif`-generation time
- For cyclic dependency groups (SCCs), perform iterative solve:
  - Generate placeholder interfaces
  - Merge and re-run until fixpoint or max iterations
  - Write final `.ksif` with dependency hashes
- Maintain "no-flattening" principle
- Avoid ad-hoc hacks

**Out of Scope:**
- `.ksobj` format (not requested)
- Runtime optimization
- Cross-version compatibility (current salt strategy is sufficient)

## 2. Requirements and Acceptance Criteria

### Must Have (M)
- [M1] `.ksif` format includes a dependency manifest: list of (module name, ksif content hash)
- [M2] Compute and verify ksif content hash when reading/writing
- [M3] Detect cycles in module import graph during ksif generation
- [M4] For SCC groups, implement iterative fixpoint solving with placeholder interfaces
- [M5] Handle convergence: detect fixpoint or enforce max iteration limit
- [M6] Write final `.ksif` with computed dependency hashes
- [M7] No flattening: maintain module boundaries and import structure
- [M8] All existing tests continue to pass

### Should Have (S)
- [S1] Clear error messages when cycles cannot be resolved
- [S2] Debug logging for cycle detection and iterative solving
- [S3] Performance: avoid redundant hash computation
- [S4] Validate hash consistency when loading `.ksif`

### Could Have (C)
- [C1] Cache SCC computation results
- [C2] Visualize module dependency graph for debugging
- [C3] Detect and warn about deep recursion in SCC solving

## 3. Current State Analysis

### Existing KSIF Implementation (`src/ksif.rs`)
- **Format:** JSON-based (serde), version 1.0
- **Components:**
  - `KsifHeader`: version + salt (includes kscr version)
  - `ModuleShape`: interface-only (types, classes, values, instances)
  - `ModuleContent`: implementation payload (not used yet)
  - `DependencySpec`: name + version requirement (no content hash yet)
- **Capabilities:**
  - Serialize/deserialize to JSON
  - Validate header compatibility
  - Detect module collisions by salt
  - Extract shape from AST

**Missing:**
- Content hash computation
- Dependency hash tracking
- Cycle detection
- SCC-based iterative solving

### Module Loading (`src/types.rs`)
- **ModuleLoader:**
  - Maintains cache (PathBuf -> ast::Module)
  - Stack-based cycle detection (simple path check)
  - Import collection (recursive DFS)
  - Current error: "cyclic imports: A -> B -> A" (rejects all cycles)
- **Collision detection:** Only checks for duplicate module paths

**Missing:**
- Import graph construction
- Tarjan's algorithm or equivalent for SCC detection
- Iterative solving infrastructure
- Integration with ksif hash validation

### Hash Infrastructure
- **Available:** `HashMap`, `HashSet` (standard collections)
- **Need to add:** SHA-256 or similar for content hashing
  - Rust stdlib provides basic hashing via `std::hash`
  - For cryptographic/content-addressed hashing, consider `sha2` crate

## 4. Design Decisions

### 4.1 Hash Algorithm Choice

**Recommendation: SHA-256 via `sha2` crate**

Rationale:
- Collision resistance needed for content-addressed caching
- Stable across platforms and Rust versions
- Fast enough for module-sized inputs
- Standard choice for content hashing

Alternative considered: `std::collections::hash_map::DefaultHasher`
- Rejected: Not stable across runs, designed for HashMap not content addressing

### 4.2 Dependency Hash Manifest Format

**Extend `DependencySpec` in ksif.rs:**

```rust
pub struct DependencySpec {
    pub name: String,
    pub version_req: String,
    pub content_hash: Option<String>, // hex-encoded SHA-256
}
```

**Hash computation:**
- Hash the **serialized JSON of ModuleShape** (after removing `module_id`)
- Include in hash: canonical_path, values, types, classes, instances, dependencies
- Exclude: `module_id` (runtime-only field)

### 4.3 Cycle Detection Strategy

**Use Tarjan's Algorithm for SCC detection:**
- Single DFS pass, O(V+E) complexity
- Identifies all SCCs in topological order
- Well-understood algorithm with good Rust implementations

**When to detect:**
- During `collect_imports` phase in ModuleLoader
- Build complete import graph before processing
- Detect SCCs, then process in topological order

**Granularity:**
- SCC = set of modules that import each other (directly or transitively)
- Example: `{A, B}` if A imports B and B imports A
- Singleton SCCs (no cycle) processed normally

### 4.4 Iterative SCC Solving Strategy

**For each SCC with >1 module:**

1. **Initialize:** Generate placeholder ModuleShape for each module in SCC
   - Empty export tables (or forward-declared names)
   - Mark as "provisional"

2. **Iterate:**
   - For each module M in SCC:
     - Parse M's source
     - Resolve imports using current provisional shapes
     - Type-check M (may partially fail if types incomplete)
     - Extract new ModuleShape from typed AST
   - Compare new shapes with previous iteration
   - If **converged** (shapes unchanged): done
   - If **max iterations** reached: error or use partial result

3. **Finalize:**
   - Mark shapes as "final"
   - Compute content hashes
   - Write `.ksif` files with dependency hashes

**Convergence criteria:**
- Structural equality of ModuleShape (JSON serialization equality)
- Alternative: Hash equality (cheaper to compare)

**Max iterations:**
- Default: 10 iterations
- Configurable via environment variable `KSCR_MAX_SCC_ITERATIONS`
- Error message if not converged: suggest refactoring to break cycle

### 4.5 No-Flattening Guarantee

**Maintain throughout:**
- Do NOT merge module ASTs
- Keep per-module ClassEnv and scope separate
- Import resolution creates scope links, not AST duplication
- SCC solving operates on ModuleShape, not flattened AST

**Verification:**
- Each module's `.ksif` references dependencies by hash
- Loading reconstructs scope from ModuleShapes
- No combined "super-module" artifact

## 5. Implementation Plan

### Phase 1: Add Hash Infrastructure (1-2 days)

**Tasks:**
- [T1.1] Add `sha2` dependency to `Cargo.toml`
- [T1.2] Implement hash computation for `ModuleShape`:
  - Function: `compute_shape_hash(shape: &ModuleShape) -> String`
  - Serialize to JSON, compute SHA-256, return hex string
  - Add test: hash stability check
- [T1.3] Update `DependencySpec` to include `content_hash: Option<String>`
- [T1.4] Update `ModuleShape` serialization to include dependency hashes
- [T1.5] Add validation: when loading `.ksif`, optionally verify dependency hashes

**Files affected:**
- `Cargo.toml`
- `src/ksif.rs` (add hash functions, update DependencySpec)

**Tests:**
- `test_hash_computation_stable()`: same shape -> same hash
- `test_hash_computation_sensitive()`: changed shape -> different hash
- `test_dependency_hash_round_trip()`: serialize/deserialize with hashes

**Completion criteria:**
- Dependency hashes can be computed and stored in `.ksif`
- Loading validates hashes (with flag to skip for compatibility)

### Phase 2: Import Graph and SCC Detection (2-3 days)

**Tasks:**
- [T2.1] Create new module `src/types/import_graph.rs`:
  - `struct ImportGraph { nodes: Vec<ModulePath>, edges: Vec<(usize, usize)> }`
  - `fn build_import_graph(loader: &ModuleLoader, entry: &Path) -> Result<ImportGraph>`
  - Build complete graph before processing imports
- [T2.2] Implement Tarjan's algorithm in `import_graph.rs`:
  - `fn find_sccs(graph: &ImportGraph) -> Vec<Vec<usize>>`
  - Returns SCCs in reverse topological order
- [T2.3] Integrate SCC detection into `ModuleLoader::collect_imports`:
  - Build import graph first
  - Detect SCCs
  - For singleton SCCs: process as before
  - For multi-node SCCs: defer to iterative solver
- [T2.4] Update `validate_import_cyclic` to allow cycles but mark them:
  - Don't error on cycles (they're valid in new design)
  - Collect cycle info for SCC processing

**Files affected:**
- `src/types/import_graph.rs` (new file)
- `src/types.rs` (integrate SCC detection into ModuleLoader)
- `src/types/mod.rs` (if needed for module visibility)

**Tests:**
- `test_tarjan_no_cycle()`: acyclic graph -> singleton SCCs
- `test_tarjan_simple_cycle()`: A -> B -> A
- `test_tarjan_complex_cycle()`: A -> B -> C -> A, D -> C
- `test_import_graph_build()`: construct graph from test modules

**Completion criteria:**
- Import graph correctly represents module dependencies
- SCCs detected accurately
- No regression in existing acyclic module loading

### Phase 3: Iterative SCC Solver (3-4 days)

**Tasks:**
- [T3.1] Create new module `src/types/scc_solver.rs`:
  - `struct SccSolver { modules: Vec<PathBuf>, loader: &mut ModuleLoader, ... }`
  - `fn solve_scc(&mut self, scc: &[usize]) -> Result<Vec<ModuleShape>>`
- [T3.2] Implement placeholder shape generation:
  - `fn create_placeholder_shape(path: &Path) -> ModuleShape`
  - Empty exports, mark as provisional
- [T3.3] Implement iterative solving loop:
  - For iteration 1..MAX_ITERATIONS:
    - Parse each module with current provisional shapes
    - Type-check (may be partial)
    - Extract ModuleShape
    - Compare with previous iteration
    - If converged: break
  - Return final shapes or error
- [T3.4] Implement convergence detection:
  - `fn shapes_equal(a: &ModuleShape, b: &ModuleShape) -> bool`
  - Compare serialized JSON or hashes
- [T3.5] Handle convergence failure:
  - Error message with cycle participants
  - Suggest breaking cycle or increasing max iterations
- [T3.6] Integrate into ModuleLoader:
  - When SCC with >1 node detected, call `SccSolver::solve_scc`
  - Use resulting shapes for further processing

**Files affected:**
- `src/types/scc_solver.rs` (new file)
- `src/types.rs` (integrate solver into ModuleLoader)

**Tests:**
- `test_scc_solver_simple_cycle()`: A imports B, B imports A (converges)
- `test_scc_solver_convergence()`: verify iteration count and final shapes
- `test_scc_solver_max_iterations()`: cycle that doesn't converge -> error
- `test_scc_solver_partial_export()`: SCC with selective exports

**Completion criteria:**
- Cyclic imports no longer cause immediate error
- Simple mutual recursion (A <-> B) resolves correctly
- Non-converging cycles produce helpful error

### Phase 4: Hash Integration in KSIF Generation (2 days)

**Tasks:**
- [T4.1] Update `emit_ksif` in `src/cli/cli_compile.rs`:
  - Compute dependency hashes from loaded ModuleShapes
  - Populate `DependencySpec.content_hash` for each dependency
  - Write updated `.ksif` with hashes
- [T4.2] Update `ModuleShape::from_ast_module`:
  - Accept loaded dependency shapes as parameter
  - Compute and store hashes in dependencies list
- [T4.3] Implement hash verification on load:
  - When loading `.ksif`, check if referenced dependency hashes match
  - Warn or error if mismatch (indicates stale cache)
- [T4.4] Add debug logging:
  - Log hash computation for each module
  - Log SCC detection and solving progress
  - Controlled by `KSCR_DEBUG_KSIF=1`

**Files affected:**
- `src/cli/cli_compile.rs` (update emit_ksif)
- `src/ksif.rs` (hash verification in load)
- `src/types.rs` (thread dependency shapes through)

**Tests:**
- `test_ksif_dependency_hashes()`: generated ksif includes correct hashes
- `test_ksif_hash_verification()`: loading validates hashes
- `test_ksif_stale_dependency()`: changed dependency -> detected

**Completion criteria:**
- Generated `.ksif` files include dependency hashes
- Stale cache detected via hash mismatch
- Debug logging helps trace hash computation

### Phase 5: End-to-End Integration and Testing (2-3 days)

**Tasks:**
- [T5.1] Create test cases for cyclic modules:
  - `tests/cycle_simple/A.ks`: imports B, exports f using B's g
  - `tests/cycle_simple/B.ks`: imports A, exports g using A's f
  - Verify: both compile, `.ksif` files generated with hashes
- [T5.2] Create test case for 3-way cycle:
  - `tests/cycle_three/A.ks`, `B.ks`, `C.ks`: A->B->C->A
- [T5.3] Create test case for nested SCCs:
  - Module graph with multiple independent SCCs
- [T5.4] Update existing tests:
  - Tests that expected cycle errors now should pass
  - Verify no behavior change for acyclic imports
- [T5.5] Performance testing:
  - Large module graph (20+ modules)
  - Measure time for SCC detection and solving
  - Ensure <1s for typical project sizes
- [T5.6] Documentation:
  - Update `docs/ksif-stage3-design.md` with SCC solving
  - Document hash format and validation
  - Add examples of cyclic module patterns

**Files affected:**
- `tests/cycle_simple/` (new test modules)
- `tests/cycle_three/` (new test modules)
- `docs/ksif-stage3-design.md` (update docs)
- Existing test files (if cycle errors need updating)

**Tests:**
- Integration tests for all cyclic patterns
- Regression tests: all existing 339 tests still pass
- Performance benchmark: `cargo test --release -- --nocapture perf_scc`

**Completion criteria:**
- All test cases pass
- Documentation updated
- No performance regressions
- Cyclic imports work correctly

## 6. Affected Files and Functions

### New Files
1. `src/types/import_graph.rs`: Import graph construction and Tarjan's algorithm
2. `src/types/scc_solver.rs`: Iterative SCC solving logic

### Modified Files

#### `Cargo.toml`
- Add dependency: `sha2 = "0.10"`

#### `src/ksif.rs`
**Functions to add:**
- `pub fn compute_shape_hash(shape: &ModuleShape) -> String`
- `pub fn verify_dependency_hash(dep: &DependencySpec, actual_shape: &ModuleShape) -> Result<()>`

**Structures to modify:**
- `DependencySpec`: add `content_hash: Option<String>`

**Functions to modify:**
- `ModuleShape::from_ast_module()`: accept dependency shapes, compute hashes
- `ModuleShape::load_from_file()`: add hash verification option

#### `src/types.rs`
**Functions to modify:**
- `ModuleLoader::collect_imports()`: integrate SCC detection
- `ModuleLoader::validate_import_cyclic()`: allow cycles, collect info
- `load_module_with_imports_ast_with_loader()`: handle SCC-resolved shapes

**Functions to add:**
- `ModuleLoader::collect_import_graph()`: build ImportGraph
- `ModuleLoader::process_scc()`: handle multi-node SCC

#### `src/cli/cli_compile.rs`
**Functions to modify:**
- `emit_ksif()`: compute and include dependency hashes

#### New test modules
- `tests/cycle_simple/{A,B}.ks`
- `tests/cycle_three/{A,B,C}.ks`
- `tests/cycle_nested/{D,E,F,G}.ks`

#### Documentation
- `docs/ksif-stage3-design.md`: add SCC solving section
- `docs/ksif-hash-format.md` (new): document hash computation

## 7. Validation Strategy

### Unit Tests
- **Hash computation** (src/ksif.rs):
  - Stability: same input -> same hash
  - Sensitivity: different input -> different hash
  - Round-trip: hash in dependency spec survives serialization
  
- **Tarjan's algorithm** (src/types/import_graph.rs):
  - Acyclic graph: all singleton SCCs
  - Simple cycle: single SCC with 2 nodes
  - Complex graph: multiple SCCs in correct order
  - Edge cases: self-loop, disconnected components
  
- **SCC solver** (src/types/scc_solver.rs):
  - Convergence: simple mutual recursion converges
  - Non-convergence: error on max iterations
  - Placeholder handling: empty shape doesn't break typecheck

### Integration Tests
- **Cyclic modules**:
  - 2-module cycle (A <-> B)
  - 3-module cycle (A -> B -> C -> A)
  - Nested cycles: (A <-> B) and (C <-> D), with D -> A
  
- **Mixed acyclic/cyclic**:
  - Some modules in SCC, others acyclic
  - Verify acyclic modules unaffected
  
- **Hash validation**:
  - Generate `.ksif` for module A
  - Modify dependency B
  - Verify A's `.ksif` detects staleness

### Regression Tests
- Run all existing 339 tests
- Ensure no changes to acyclic module behavior
- Verify error messages still helpful

### Performance Tests
- Measure SCC detection time for N-module graph
- Measure iteration count for typical cycles
- Ensure <100ms for 50-module project

### Debug/Observability
- `KSCR_DEBUG_KSIF=1`: log hash computation and SCC solving
- `KSCR_DEBUG_SCC_SOLVE=1`: log each iteration of SCC solver
- `KSCR_MAX_SCC_ITERATIONS=N`: override default (10)

## 8. Risk Assessment and Mitigation

### Risk 1: Non-convergence in SCC Solver
**Likelihood:** Medium  
**Impact:** High (blocks compilation)

**Mitigation:**
- Set reasonable max iterations (10)
- Clear error message with cycle participants
- Document patterns that may not converge
- Provide escape hatch: `--max-scc-iterations=N`

### Risk 2: Performance degradation for large projects
**Likelihood:** Low  
**Impact:** Medium

**Mitigation:**
- Tarjan's algorithm is O(V+E), efficient
- Cache SCC computation results
- Only re-solve SCC if dependencies changed
- Profile with realistic project sizes

### Risk 3: Hash collisions
**Likelihood:** Very Low  
**Impact:** Low (false cache hit)

**Mitigation:**
- Use SHA-256 (cryptographically secure)
- Include module canonical path in hash input
- Hash collision virtually impossible in practice

### Risk 4: Breaking existing workflows
**Likelihood:** Low  
**Impact:** Medium

**Mitigation:**
- All changes additive to `.ksif` format
- Old `.ksif` files still loadable (content_hash is Option)
- Graceful degradation: missing hash -> skip verification
- Version in KsifHeader tracks format changes

### Risk 5: Complex debugging of SCC solver
**Likelihood:** Medium  
**Impact:** Medium

**Mitigation:**
- Extensive debug logging
- Visualize iteration progress
- Unit tests for each iteration step
- Document expected iteration counts

## 9. Open Questions and Decisions Needed

### Q1: Should we support cross-version hash compatibility?
**Current decision:** No, salt already forces recompilation on version change.  
**Rationale:** Hashes are content-addressed, version-independent. Salt provides version boundary.

### Q2: What if SCC solver produces partial types?
**Proposed:** Accept partial types if they're sufficient for dependents.  
**Alternative:** Require complete type information before finalizing.  
**Recommendation:** Start strict (complete types), relax if needed.

### Q3: Should placeholder shapes include forward declarations?
**Proposed:** Start with empty shapes, add minimal forward decls if needed.  
**Rationale:** Simplifies initial implementation, can enhance later.

### Q4: Hash format: hex string or binary?
**Decision:** Hex string in JSON (human-readable, debuggable).  
**Alternative:** Binary in future binary format (more compact).

### Q5: Verification strictness: error or warn on hash mismatch?
**Proposed:** Warn by default, error with `--strict` flag.  
**Rationale:** Allows graceful degradation during development.

## 10. Success Criteria

### Minimum Viable Product (MVP)
- ✅ Dependency hashes in `.ksif` format
- ✅ SCC detection using Tarjan's algorithm
- ✅ Iterative solving for 2-module cycles (A <-> B)
- ✅ Hash validation on load (warn on mismatch)
- ✅ All existing tests pass
- ✅ Documentation updated

### Full Success
- ✅ MVP criteria
- ✅ N-way cycles (3+ modules) work correctly
- ✅ Nested SCC handling
- ✅ Performance <100ms for 50-module project
- ✅ Clear error messages for non-convergent cycles
- ✅ Debug logging for troubleshooting
- ✅ Integration tests covering all cycle patterns

### Stretch Goals
- ⚡ Cache SCC results across compilations
- ⚡ Visualize module dependency graph
- ⚡ Detect and optimize common cycle patterns
- ⚡ Parallel SCC solving for independent SCCs

## 11. Timeline Estimate

| Phase | Tasks | Estimated Time | Dependencies |
|-------|-------|----------------|--------------|
| Phase 1: Hash Infrastructure | T1.1-T1.5 | 1-2 days | None |
| Phase 2: Import Graph & SCC | T2.1-T2.4 | 2-3 days | Phase 1 |
| Phase 3: Iterative Solver | T3.1-T3.6 | 3-4 days | Phase 2 |
| Phase 4: Hash Integration | T4.1-T4.4 | 2 days | Phase 1, 3 |
| Phase 5: Testing & Docs | T5.1-T5.6 | 2-3 days | All phases |
| **Total** | | **10-14 days** | |

**Parallel opportunities:**
- Phase 1 and Phase 2 can start simultaneously (different files)
- Phase 4 can start once Phase 1 complete (doesn't need Phase 3)
- Documentation can be written incrementally

## 12. Next Steps

### Immediate Actions (Day 1)
1. ✅ Review and approve this plan
2. Add `sha2` to `Cargo.toml`
3. Implement hash computation in `src/ksif.rs`
4. Write unit tests for hash stability

### Short-term (Week 1)
1. Implement Tarjan's algorithm in `import_graph.rs`
2. Integrate SCC detection into ModuleLoader
3. Create test cases for cyclic modules
4. Begin iterative solver implementation

### Medium-term (Week 2)
1. Complete SCC solver with convergence detection
2. Integrate hash validation in ksif load/save
3. Run full test suite, fix regressions
4. Update documentation

### Follow-up (Week 3+)
1. Performance profiling and optimization
2. Additional test coverage for edge cases
3. User-facing documentation and examples
4. Consider stretch goals (caching, visualization)

## 13. Dependencies

### External Crates
- `sha2 = "0.10"`: SHA-256 hashing for content addressing

### Internal Modules
- `src/ksif.rs`: Extend with hash computation
- `src/types.rs`: Integrate SCC detection
- `src/cli/cli_compile.rs`: Update ksif emission

### Prerequisite Knowledge
- Tarjan's algorithm for SCC detection
- Iterative fixpoint solving techniques
- Content-addressed storage principles

## 14. Rollback Plan

If implementation encounters critical issues:

### Rollback Steps
1. Revert commits from working branch
2. Restore ModuleLoader to reject cycles (current behavior)
3. Keep hash infrastructure (forward-compatible)
4. Document issues encountered for future attempt

### Partial Success Scenarios
- **Hash-only:** Ship dependency hashing without SCC solving
- **Detection-only:** Detect and report SCCs without solving
- **Simple-cycles-only:** Support 2-module cycles, reject N-way

### Compatibility Guarantee
- Old `.ksif` files remain loadable (Option<String> for hash)
- Version bump in KsifHeader if breaking changes needed
- Clear migration path documented

---

## Appendix A: Algorithm Details

### Tarjan's Algorithm for SCC Detection

**Input:** Directed graph G = (V, E)  
**Output:** List of SCCs in reverse topological order

**Pseudocode:**
```
function tarjan(v):
    index[v] = low[v] = next_index++
    stack.push(v)
    on_stack[v] = true
    
    for each edge (v, w):
        if index[w] is undefined:
            tarjan(w)
            low[v] = min(low[v], low[w])
        else if on_stack[w]:
            low[v] = min(low[v], index[w])
    
    if low[v] == index[v]:
        scc = []
        repeat:
            w = stack.pop()
            on_stack[w] = false
            scc.append(w)
        until w == v
        output scc
```

**Properties:**
- Single DFS traversal
- O(V + E) time complexity
- O(V) space for stack and index arrays

### Iterative Fixpoint Solving

**Input:** SCC with modules M1, M2, ..., Mn  
**Output:** Final ModuleShapes for each module

**Algorithm:**
```
shapes = [create_placeholder(m) for m in SCC]
for iteration in 1..MAX_ITERATIONS:
    new_shapes = []
    for i, module in enumerate(SCC):
        // Use current shapes for dependencies
        typed_ast = typecheck(module, current_shapes=shapes)
        new_shape = extract_shape(typed_ast)
        new_shapes.append(new_shape)
    
    if new_shapes == shapes:
        return new_shapes  // Converged
    
    shapes = new_shapes

error("Failed to converge after MAX_ITERATIONS")
```

**Convergence Criteria:**
- Structural equality: `serialize(new_shapes) == serialize(old_shapes)`
- Or hash equality: `hash(new_shapes) == hash(old_shapes)`

**Termination:**
- Guaranteed: max iterations reached
- Desired: convergence in <10 iterations for typical cases

## Appendix B: Example Cyclic Module Scenario

### Scenario: Mutual Recursion

**Module A (A.ks):**
```haskell
module A where
  import B
  
  isEven :: Integer -> Bool
  isEven 0 = True
  isEven n = B.isOdd (n - 1)
```

**Module B (B.ks):**
```haskell
module B where
  import A
  
  isOdd :: Integer -> Bool
  isOdd 0 = False
  isOdd n = A.isEven (n - 1)
```

**Expected Behavior:**
1. **Phase 2:** Detect SCC = {A, B}
2. **Phase 3:** 
   - Iteration 1: Placeholder shapes (empty exports)
   - Iteration 2: Extract `isEven : Integer -> Bool` and `isOdd : Integer -> Bool`
   - Iteration 3: No change (converged)
3. **Phase 4:** 
   - Compute hash(B.ksif) = "abc123..."
   - Write A.ksif with dependency: (B, "abc123...")
   - Compute hash(A.ksif) = "def456..."
   - Write B.ksif with dependency: (A, "def456...")

**Resulting A.ksif (JSON):**
```json
{
  "header": { "ksif_version": "1.0", "salt": "kscr-0.3.6" },
  "canonical_path": "A",
  "values": {
    "isEven": { "name": "isEven", "scheme": "Integer -> Bool" }
  },
  "dependencies": [
    { "name": "B", "version_req": "*", "content_hash": "abc123..." }
  ]
}
```

## Appendix C: Testing Checklist

### Unit Tests (src/ksif.rs)
- [ ] `test_compute_hash_stable()`
- [ ] `test_compute_hash_sensitive()`
- [ ] `test_dependency_hash_roundtrip()`
- [ ] `test_verify_dependency_hash_match()`
- [ ] `test_verify_dependency_hash_mismatch()`

### Unit Tests (src/types/import_graph.rs)
- [ ] `test_tarjan_single_node()`
- [ ] `test_tarjan_acyclic_graph()`
- [ ] `test_tarjan_simple_cycle()`
- [ ] `test_tarjan_two_sccs()`
- [ ] `test_tarjan_nested_sccs()`
- [ ] `test_tarjan_self_loop()`

### Unit Tests (src/types/scc_solver.rs)
- [ ] `test_placeholder_shape_creation()`
- [ ] `test_scc_solver_converges()`
- [ ] `test_scc_solver_max_iterations()`
- [ ] `test_shapes_equal()`

### Integration Tests
- [ ] `test_cycle_two_modules()`
- [ ] `test_cycle_three_modules()`
- [ ] `test_nested_cycles()`
- [ ] `test_acyclic_with_cyclic_deps()`
- [ ] `test_ksif_hash_validation()`
- [ ] `test_stale_dependency_detection()`

### Regression Tests
- [ ] All existing 339 tests pass
- [ ] No change to acyclic module behavior
- [ ] Error messages still clear

### Performance Tests
- [ ] `bench_tarjan_large_graph()`
- [ ] `bench_scc_solver_iterations()`
- [ ] `bench_hash_computation()`

---

**Plan Version:** 1.0  
**Last Updated:** 2024-01-28  
**Reviewers:** [To be assigned]  
**Status:** Ready for Review

---

## Appendix D: Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│                      KSIF Generation Pipeline                    │
└─────────────────────────────────────────────────────────────────┘

Entry Module (e.g., Main.ks)
         │
         ├─> Parse Source
         │
         ├─> Build Import Graph ──────┐
         │   (collect all imports)    │
         │                            │
         ├─> Detect SCCs ─────────────┤  Phase 2: SCC Detection
         │   (Tarjan's algorithm)     │  (src/types/import_graph.rs)
         │                            │
         ├─> Group by SCC ────────────┘
         │
         ├─> For each SCC:
         │   │
         │   ├─> If singleton (no cycle):
         │   │   └─> Process normally ────┐
         │   │                            │
         │   └─> If multi-node (cycle):   │  Phase 3: SCC Solving
         │       │                        │  (src/types/scc_solver.rs)
         │       ├─> Create placeholders  │
         │       ├─> Iterate:             │
         │       │   ├─> Parse modules    │
         │       │   ├─> Typecheck        │
         │       │   ├─> Extract shapes   │
         │       │   └─> Check convergence│
         │       └─> Finalize shapes ─────┘
         │
         ├─> Compute hashes ──────────────┐
         │   (SHA-256 of ModuleShape)     │  Phase 1: Hash Infrastructure
         │                                │  (src/ksif.rs)
         └─> Write .ksif with dep hashes ─┘
             (DependencySpec includes hashes)

┌─────────────────────────────────────────────────────────────────┐
│                      Data Flow Example                          │
└─────────────────────────────────────────────────────────────────┘

Module A.ks ───imports───> Module B.ks
     ▲                           │
     │                           │
     └────────imports─────────────┘
              (cycle!)

Step 1: Detect SCC = {A, B}

Step 2: Iterative Solving
   Iter 1: A.shape = {}, B.shape = {}
   Iter 2: A.shape = {isEven: Int->Bool}, B.shape = {isOdd: Int->Bool}
   Iter 3: No change → CONVERGED

Step 3: Hash Computation
   hash_A = SHA256(A.shape) = "abc123..."
   hash_B = SHA256(B.shape) = "def456..."

Step 4: Write .ksif
   A.ksif: { ..., dependencies: [{ name: "B", hash: "def456..." }] }
   B.ksif: { ..., dependencies: [{ name: "A", hash: "abc123..." }] }

┌─────────────────────────────────────────────────────────────────┐
│                      Module Structures                          │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────┐
│   ModuleShape       │  Interface-only, small
│─────────────────────│
│ + canonical_path    │
│ + values: {..}      │
│ + types: {..}       │
│ + classes: {..}     │
│ + instances: [..]   │
│ + dependencies: [   │  ← Extended with content_hash
│     DependencySpec  │
│   ]                 │
└─────────────────────┘

┌─────────────────────┐
│  DependencySpec     │  Tracks one dependency
│─────────────────────│
│ + name: String      │
│ + version_req: String│
│ + content_hash:     │  ← NEW: SHA-256 hex string
│   Option<String>    │
└─────────────────────┘

┌─────────────────────┐
│   ImportGraph       │  For SCC detection
│─────────────────────│
│ + nodes: Vec<Path>  │
│ + edges: Vec<(u,v)> │
│                     │
│ Methods:            │
│ + find_sccs()       │  ← Tarjan's algorithm
│ + is_cyclic()       │
└─────────────────────┘

┌─────────────────────┐
│   SccSolver         │  Iterative fixpoint
│─────────────────────│
│ + scc: Vec<ModId>   │
│ + shapes: Vec<Shape>│
│ + iteration: usize  │
│                     │
│ Methods:            │
│ + solve() -> Result │
│ + converged() -> bool│
└─────────────────────┘
```

---

## Appendix E: Decision Log

### Decision 1: Hash Algorithm (2024-01-28)
**Question:** Which hash algorithm for content addressing?  
**Options:**
- A) `std::hash` DefaultHasher (fast, unstable)
- B) SHA-256 via `sha2` crate (stable, secure)
- C) BLAKE3 (faster, newer)

**Decision:** SHA-256 (Option B)  
**Rationale:**
- Stability across platforms/versions required
- Industry standard for content addressing
- Good balance of speed and security
- Well-supported in Rust ecosystem

**Trade-offs:** BLAKE3 is faster but less mature tooling

---

### Decision 2: SCC Detection Algorithm (2024-01-28)
**Question:** Which algorithm for finding SCCs?  
**Options:**
- A) Kosaraju's algorithm (2 passes)
- B) Tarjan's algorithm (1 pass)
- C) Path-based strong component algorithm

**Decision:** Tarjan's algorithm (Option B)  
**Rationale:**
- Single DFS pass, O(V+E) time
- Well-understood, many reference implementations
- Returns SCCs in useful order (reverse topological)
- Minimal memory overhead

**Trade-offs:** Slightly more complex than Kosaraju's

---

### Decision 3: Convergence Criterion (2024-01-28)
**Question:** How to detect when SCC solving has converged?  
**Options:**
- A) JSON string equality
- B) Hash equality (re-hash shapes each iteration)
- C) Deep structural comparison

**Decision:** JSON string equality (Option A), with hash caching  
**Rationale:**
- Simple to implement and debug
- JSON serialization already available
- Can optimize with hash comparison later
- Clear semantics: converged when serialized forms identical

**Trade-offs:** JSON comparison slower than hash, but negligible for small shapes

---

### Decision 4: Non-convergence Handling (2024-01-28)
**Question:** What to do when SCC solver doesn't converge?  
**Options:**
- A) Error immediately
- B) Use partial result with warning
- C) Allow manual override

**Decision:** Error after max iterations (Option A)  
**Rationale:**
- Safer: prevents incorrect type inference
- Forces developer to address fundamental issue
- Can add Option B later if needed
- Max iterations configurable for experimentation

**Trade-offs:** Stricter but safer

---

### Decision 5: Hash Verification Strictness (2024-01-28)
**Question:** How strict should hash verification be on load?  
**Options:**
- A) Always error on mismatch
- B) Warn by default, error with flag
- C) Never verify (trust filesystem)

**Decision:** Warn by default, error with `--strict` (Option B)  
**Rationale:**
- Graceful degradation during development
- Allows manual cache invalidation
- Production builds can use --strict
- Matches existing kscr philosophy

**Trade-offs:** Potential for stale cache confusion

---

## Appendix F: Performance Estimates

### Hash Computation

**Input:** ModuleShape with ~50 exported values  
**JSON size:** ~10 KB  
**SHA-256 throughput:** ~500 MB/s (typical CPU)  
**Expected time:** <0.02 ms per module

**Conclusion:** Hash computation is negligible (<1% of total)

### Tarjan's Algorithm

**Input:** 50 modules, average 3 imports each → 150 edges  
**Complexity:** O(V + E) = O(50 + 150) = 200 operations  
**Expected time:** <0.1 ms

**Conclusion:** SCC detection is negligible

### Iterative SCC Solving

**Assumptions:**
- SCC size: 3 modules (worst case: 10 modules)
- Iterations to convergence: 3 (worst case: 10)
- Per-iteration cost: parse (5ms) + typecheck (20ms) + extract (1ms) = 26ms

**Expected time:**
- Best case (3 modules × 3 iterations): 234 ms
- Worst case (10 modules × 10 iterations): 2.6 s

**Mitigation:**
- Most projects have small/no cycles
- Parallel solving of independent SCCs (future)
- Caching of converged results

**Conclusion:** Acceptable for typical use, may need optimization for pathological cases

### Overall Impact

**Baseline (current):**
- Parse 50 modules: ~250 ms
- Typecheck 50 modules: ~1000 ms
- Total: ~1.25 s

**With SCC solving (estimated):**
- Parse: 250 ms (unchanged)
- Build import graph: 0.1 ms
- Detect SCCs: 0.1 ms
- Solve SCCs (assume 1 SCC with 3 modules): 234 ms
- Typecheck remaining: 800 ms
- Hash computation: 1 ms
- Total: ~1.29 s

**Overhead:** ~40 ms (~3% increase)

**Conclusion:** Minimal performance impact for typical projects

