# Implementation Plan Summary: KSIF Dependency Hashing & Cyclic SCC Resolution

**Status:** Ready for Implementation  
**Estimated Duration:** 10-14 days  
**Full Plan:** See `plan.md`

## Quick Overview

This plan extends `.ksif` format to track dependency content hashes and implements cyclic dependency resolution through iterative fixpoint solving for Strongly Connected Components (SCCs).

## Core Requirements (Must Have)

1. ✅ **Dependency Hash Manifest:** `.ksif` includes list of (module name, content hash SHA-256)
2. ✅ **Hash Computation:** Compute/verify ksif hashes on read/write
3. ✅ **Cycle Detection:** Tarjan's algorithm for SCC detection in import graph
4. ✅ **Iterative SCC Solver:** Placeholder interfaces → iterate until fixpoint/max iterations
5. ✅ **No Flattening:** Maintain module boundaries throughout
6. ✅ **All Tests Pass:** No regression in existing 339 tests

## Implementation Phases

### Phase 1: Hash Infrastructure (1-2 days)
- Add `sha2` crate dependency
- Implement `compute_shape_hash()` for ModuleShape
- Extend `DependencySpec` with `content_hash: Option<String>`
- Hash validation on load

### Phase 2: Import Graph & SCC Detection (2-3 days)
- New file: `src/types/import_graph.rs`
- Implement Tarjan's algorithm for SCC detection
- Integrate into ModuleLoader
- Build complete import graph before processing

### Phase 3: Iterative SCC Solver (3-4 days)
- New file: `src/types/scc_solver.rs`
- Placeholder ModuleShape generation
- Iterative solving loop with convergence detection
- Max iterations (default: 10) with clear error messages

### Phase 4: Hash Integration (2 days)
- Update `emit_ksif()` to include dependency hashes
- Hash verification on load
- Debug logging (`KSCR_DEBUG_KSIF=1`)

### Phase 5: Testing & Documentation (2-3 days)
- Test cases for 2-way, 3-way, N-way cycles
- Integration tests for mixed acyclic/cyclic
- Performance benchmarks
- Update documentation

## Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| **Hash Algorithm** | SHA-256 (via `sha2` crate) | Collision resistance, stability, standard |
| **SCC Algorithm** | Tarjan's | O(V+E), single pass, well-understood |
| **Convergence** | JSON equality or hash equality | Simple to implement, debuggable |
| **Max Iterations** | 10 (configurable) | Reasonable for typical cycles |
| **Hash Format** | Hex string in JSON | Human-readable, debuggable |
| **Verification** | Warn by default, error with `--strict` | Graceful degradation |

## Files to Create

1. `src/types/import_graph.rs` - Import graph and Tarjan's algorithm
2. `src/types/scc_solver.rs` - Iterative fixpoint solver
3. `tests/cycle_simple/{A,B}.ks` - Simple 2-way cycle test
4. `tests/cycle_three/{A,B,C}.ks` - 3-way cycle test
5. `docs/ksif-hash-format.md` - Hash computation documentation

## Files to Modify

1. `Cargo.toml` - Add `sha2` dependency
2. `src/ksif.rs` - Hash computation, DependencySpec extension
3. `src/types.rs` - Integrate SCC detection into ModuleLoader
4. `src/cli/cli_compile.rs` - Update `emit_ksif()` with hashes
5. `docs/ksif-stage3-design.md` - Document SCC solving

## Example: Mutual Recursion

**Before (rejected):**
```
Error: cyclic imports: A.ks -> B.ks -> A.ks
```

**After (resolved):**
```
[SCC] Detected cycle: {A, B}
[SCC] Iteration 1: placeholder shapes
[SCC] Iteration 2: extracted signatures
[SCC] Converged after 2 iterations
Generated: A.ksif (hash: abc123...)
Generated: B.ksif (hash: def456...)
```

## Testing Strategy

- **Unit Tests:** Hash computation, Tarjan's algorithm, SCC solver
- **Integration Tests:** 2-way, 3-way, nested cycles
- **Regression Tests:** All 339 existing tests must pass
- **Performance Tests:** <100ms for 50-module project

## Success Criteria

**MVP (Minimum Viable Product):**
- Dependency hashes in `.ksif` ✓
- SCC detection ✓
- 2-module cycles work ✓
- All existing tests pass ✓

**Full Success:**
- MVP + N-way cycles ✓
- Nested SCCs ✓
- Performance targets met ✓
- Clear error messages ✓

## Risk Mitigation

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Non-convergence | Medium | High | Max iterations + clear error |
| Performance issues | Low | Medium | Tarjan's is O(V+E), cache SCCs |
| Hash collisions | Very Low | Low | SHA-256 (cryptographic) |
| Breaking workflows | Low | Medium | Graceful degradation, version tracking |

## Next Steps (Day 1)

1. Review and approve this plan
2. Add `sha2 = "0.10"` to `Cargo.toml`
3. Implement `compute_shape_hash()` in `src/ksif.rs`
4. Write hash stability unit tests
5. Start Tarjan's algorithm implementation

## Key Architectural Principles

✅ **No Flattening:** Modules remain separate, imports create scope links  
✅ **Iterative Solving:** Placeholder → refine → converge  
✅ **Content Addressing:** Hashes enable incremental compilation  
✅ **Fail Gracefully:** Clear errors for non-convergent cycles  
✅ **Observable:** Debug logging for troubleshooting

## Questions or Concerns?

- See full plan in `plan.md` for detailed algorithm descriptions
- Appendix A: Tarjan's algorithm pseudocode
- Appendix B: Example cyclic module scenario
- Appendix C: Complete testing checklist

**Ready to implement? Let's start with Phase 1!**
