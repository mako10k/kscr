# Implementation Checklist: KSIF Dependency Hashing & Cyclic SCC Resolution

Track progress through each phase. Check off items as completed.

---

## Pre-Implementation Setup

- [ ] Review and approve `plan.md`
- [ ] Review and approve `PLAN_SUMMARY.md`
- [ ] Assign implementation owner
- [ ] Create feature branch: `feature/ksif-scc-resolution`
- [ ] Set up tracking issue or project board

---

## Phase 1: Hash Infrastructure (1-2 days)

### T1.1: Add sha2 Dependency
- [ ] Edit `Cargo.toml`
- [ ] Add `sha2 = "0.10"` to `[dependencies]`
- [ ] Run `cargo build` to verify
- [ ] Commit: "Add sha2 dependency for content hashing"

### T1.2: Implement Hash Computation
- [ ] Open `src/ksif.rs`
- [ ] Add `use sha2::{Sha256, Digest};`
- [ ] Implement `pub fn compute_shape_hash(shape: &ModuleShape) -> String`
  - [ ] Serialize ModuleShape to JSON
  - [ ] Compute SHA-256 digest
  - [ ] Return hex-encoded string
- [ ] Add test `test_hash_computation_stable()`
- [ ] Add test `test_hash_computation_sensitive()`
- [ ] Run `cargo test` and verify tests pass
- [ ] Commit: "Implement ModuleShape hash computation"

### T1.3: Update DependencySpec
- [ ] In `src/ksif.rs`, find `struct DependencySpec`
- [ ] Add field: `pub content_hash: Option<String>`
- [ ] Update `Serialize` and `Deserialize` derives
- [ ] Run `cargo test` to check for breakage
- [ ] Commit: "Add content_hash field to DependencySpec"

### T1.4: Update ModuleShape Serialization
- [ ] Verify `ModuleShape` includes `dependencies: Vec<DependencySpec>`
- [ ] Update tests to include example with content_hash
- [ ] Add test `test_dependency_hash_roundtrip()`
- [ ] Commit: "Update ModuleShape serialization with dependency hashes"

### T1.5: Add Hash Validation
- [ ] Implement `pub fn verify_dependency_hash(...) -> Result<()>`
- [ ] Add test `test_verify_dependency_hash_match()`
- [ ] Add test `test_verify_dependency_hash_mismatch()`
- [ ] Commit: "Add dependency hash verification"

### Phase 1 Completion
- [ ] All Phase 1 tests pass
- [ ] Code review completed
- [ ] Merge to feature branch

---

## Phase 2: Import Graph & SCC Detection (2-3 days)

### T2.1: Create ImportGraph Module
- [ ] Create file `src/types/import_graph.rs`
- [ ] Define `pub struct ImportGraph { ... }`
- [ ] Implement `pub fn new() -> Self`
- [ ] Implement `pub fn add_node(...)`
- [ ] Implement `pub fn add_edge(...)`
- [ ] Add basic test `test_import_graph_creation()`
- [ ] Update `src/types/mod.rs` to include new module
- [ ] Commit: "Add ImportGraph structure"

### T2.2: Implement Tarjan's Algorithm
- [ ] In `import_graph.rs`, add Tarjan helper structs
- [ ] Implement `pub fn find_sccs(&self) -> Vec<Vec<usize>>`
- [ ] Add test `test_tarjan_single_node()`
- [ ] Add test `test_tarjan_acyclic_graph()`
- [ ] Add test `test_tarjan_simple_cycle()`
- [ ] Add test `test_tarjan_two_sccs()`
- [ ] Add test `test_tarjan_self_loop()`
- [ ] Commit: "Implement Tarjan's algorithm for SCC detection"

### T2.3: Build Import Graph in ModuleLoader
- [ ] Open `src/types.rs`, find `ModuleLoader`
- [ ] Add method `fn build_import_graph(...) -> Result<ImportGraph>`
- [ ] Implement graph construction from module imports
- [ ] Add test `test_build_import_graph_simple()`
- [ ] Commit: "Add import graph construction to ModuleLoader"

### T2.4: Integrate SCC Detection
- [ ] Update `collect_imports()` to use SCC detection
- [ ] Modify `validate_import_cyclic()` to allow cycles
- [ ] Add debug logging for detected SCCs
- [ ] Add test `test_scc_detection_no_cycle()`
- [ ] Add test `test_scc_detection_simple_cycle()`
- [ ] Commit: "Integrate SCC detection into import collection"

### Phase 2 Completion
- [ ] All Phase 2 tests pass
- [ ] SCC detection working correctly
- [ ] Code review completed
- [ ] Merge to feature branch

---

## Phase 3: Iterative SCC Solver (3-4 days)

### T3.1: Create SccSolver Module
- [ ] Create file `src/types/scc_solver.rs`
- [ ] Define `pub struct SccSolver { ... }`
- [ ] Implement `pub fn new(...) -> Self`
- [ ] Update `src/types/mod.rs` to include module
- [ ] Commit: "Add SccSolver structure"

### T3.2: Implement Placeholder Generation
- [ ] Implement `fn create_placeholder_shape(...) -> ModuleShape`
- [ ] Add test `test_placeholder_shape_creation()`
- [ ] Commit: "Implement placeholder ModuleShape generation"

### T3.3: Implement Iterative Solving Loop
- [ ] Implement `pub fn solve_scc(...) -> Result<Vec<ModuleShape>>`
- [ ] Add iteration loop with max_iterations check
- [ ] Add parsing and typechecking within loop
- [ ] Add shape extraction
- [ ] Add test `test_scc_solver_basic()`
- [ ] Commit: "Implement iterative SCC solving loop"

### T3.4: Implement Convergence Detection
- [ ] Implement `fn shapes_equal(...) -> bool`
- [ ] Use JSON serialization equality
- [ ] Add test `test_shapes_equal()`
- [ ] Add convergence check in solve loop
- [ ] Add test `test_scc_solver_converges()`
- [ ] Commit: "Add convergence detection to SCC solver"

### T3.5: Handle Convergence Failure
- [ ] Add error handling for max iterations reached
- [ ] Format helpful error message with cycle info
- [ ] Add test `test_scc_solver_max_iterations()`
- [ ] Add environment variable `KSCR_MAX_SCC_ITERATIONS`
- [ ] Commit: "Add convergence failure handling"

### T3.6: Integrate into ModuleLoader
- [ ] Update `collect_imports()` to call SccSolver for multi-node SCCs
- [ ] Thread solved shapes through loader
- [ ] Add integration test with simple cycle
- [ ] Commit: "Integrate SCC solver into ModuleLoader"

### Phase 3 Completion
- [ ] All Phase 3 tests pass
- [ ] Simple cycles (A <-> B) resolve correctly
- [ ] Code review completed
- [ ] Merge to feature branch

---

## Phase 4: Hash Integration (2 days)

### T4.1: Update emit_ksif
- [ ] Open `src/cli/cli_compile.rs`
- [ ] Find `fn emit_ksif(...)`
- [ ] Add logic to compute dependency hashes
- [ ] Populate `DependencySpec.content_hash`
- [ ] Commit: "Include dependency hashes in emitted .ksif files"

### T4.2: Update ModuleShape::from_ast_module
- [ ] Open `src/ksif.rs`
- [ ] Update `from_ast_module()` signature to accept dependency shapes
- [ ] Compute hashes for each dependency
- [ ] Store in dependencies list
- [ ] Commit: "Compute dependency hashes when extracting ModuleShape"

### T4.3: Implement Hash Verification on Load
- [ ] Update `ModuleShape::load_from_file()` to verify hashes
- [ ] Add flag/env var to control verification strictness
- [ ] Add warning log on hash mismatch
- [ ] Add test `test_ksif_stale_dependency()`
- [ ] Commit: "Add hash verification when loading .ksif files"

### T4.4: Add Debug Logging
- [ ] Add logging for hash computation (controlled by `KSCR_DEBUG_KSIF`)
- [ ] Add logging for SCC detection progress
- [ ] Add logging for iteration count
- [ ] Test logging output manually
- [ ] Commit: "Add debug logging for KSIF operations"

### Phase 4 Completion
- [ ] Generated .ksif files include hashes
- [ ] Hash verification working
- [ ] Debug logging helpful
- [ ] Code review completed
- [ ] Merge to feature branch

---

## Phase 5: Testing & Documentation (2-3 days)

### T5.1: Create Cyclic Test Cases
- [ ] Create directory `tests/cycle_simple/`
- [ ] Write `tests/cycle_simple/A.ks` (imports B, uses B.g)
- [ ] Write `tests/cycle_simple/B.ks` (imports A, uses A.f)
- [ ] Add test in `tests/` to compile both
- [ ] Verify .ksif files generated with correct hashes
- [ ] Commit: "Add test case for simple 2-module cycle"

### T5.2: Create 3-Way Cycle Test
- [ ] Create directory `tests/cycle_three/`
- [ ] Write `A.ks`, `B.ks`, `C.ks` with A→B→C→A
- [ ] Add test to verify compilation succeeds
- [ ] Verify convergence within 10 iterations
- [ ] Commit: "Add test case for 3-way cycle"

### T5.3: Create Nested SCC Test
- [ ] Create directory `tests/cycle_nested/`
- [ ] Write modules with multiple independent SCCs
- [ ] Add test to verify all SCCs resolved correctly
- [ ] Commit: "Add test case for nested/multiple SCCs"

### T5.4: Update Existing Tests
- [ ] Review tests that expected cycle errors
- [ ] Update expected behavior if needed
- [ ] Run full test suite: `cargo test`
- [ ] Verify all 339+ tests pass
- [ ] Fix any regressions
- [ ] Commit: "Update existing tests for cycle support"

### T5.5: Performance Testing
- [ ] Create benchmark test with 20+ modules
- [ ] Measure time for SCC detection
- [ ] Measure time for iterative solving
- [ ] Verify <1s for typical projects
- [ ] Document results
- [ ] Commit: "Add performance benchmarks"

### T5.6: Documentation
- [ ] Update `docs/ksif-stage3-design.md` with SCC solving
- [ ] Create `docs/ksif-hash-format.md` (hash computation spec)
- [ ] Add examples of cyclic module patterns
- [ ] Update README if needed
- [ ] Commit: "Update documentation for SCC resolution"

### Phase 5 Completion
- [ ] All integration tests pass
- [ ] Performance acceptable
- [ ] Documentation complete
- [ ] Code review completed

---

## Final Integration & Release

### Pre-Merge Checklist
- [ ] All 5 phases completed
- [ ] Full test suite passes: `cargo test`
- [ ] No compiler warnings: `cargo clippy`
- [ ] Code formatted: `cargo fmt`
- [ ] Documentation built successfully: `cargo doc`
- [ ] Manual testing with example projects
- [ ] Performance regression check

### Merge & Release
- [ ] Create pull request from feature branch
- [ ] Address review comments
- [ ] Squash/rebase as appropriate
- [ ] Merge to main branch
- [ ] Tag release (e.g., v0.4.0)
- [ ] Update CHANGELOG
- [ ] Announce feature in release notes

### Post-Release
- [ ] Monitor for issues
- [ ] Respond to user feedback
- [ ] Consider stretch goals:
  - [ ] Cache SCC results
  - [ ] Visualize dependency graph
  - [ ] Optimize common patterns

---

## Quick Status Summary

**Overall Progress:** ____ / 85 tasks completed

**Phase 1:** ____ / 13 completed  
**Phase 2:** ____ / 13 completed  
**Phase 3:** ____ / 15 completed  
**Phase 4:** ____ / 10 completed  
**Phase 5:** ____ / 18 completed  
**Final:** ____ / 16 completed

**Current Phase:** _____________  
**Blockers:** _____________  
**Next Action:** _____________

---

**Last Updated:** [Date]  
**Implementer:** [Name]  
**Reviewer:** [Name]
