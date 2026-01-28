# KSIF Dependency Tracking & Cyclic SCC Resolution - Planning Index

This directory contains the complete implementation plan for adding dependency content hashing and cyclic dependency resolution to the KSIF (KScript Intermediate Format) system.

## 📚 Documents Overview

### 1. **plan.md** (Main Document)
**Size:** 1063 lines (36 KB)  
**Purpose:** Complete technical specification and implementation plan

**Contents:**
- Executive summary and scope
- Detailed requirements (Must/Should/Could have)
- Current state analysis of existing codebase
- 5 key design decisions with rationales
- 5-phase implementation plan (10-14 days)
- Affected files and functions
- Comprehensive validation strategy
- Risk assessment and mitigation
- Timeline estimates
- Rollback plan

**Appendices:**
- A: Algorithm Details (Tarjan's, Fixpoint solving)
- B: Example Cyclic Module Scenario
- C: Testing Checklist
- D: Architecture Diagrams
- E: Decision Log
- F: Performance Estimates

**Target Audience:** Implementers, technical reviewers

---

### 2. **PLAN_SUMMARY.md** (Executive Summary)
**Size:** 148 lines (5.2 KB)  
**Purpose:** Quick reference and decision-maker overview

**Contents:**
- Quick overview of the feature
- Core requirements (6 Must-Haves)
- Implementation phases summary
- Key design decisions table
- Files to create/modify
- Example before/after comparison
- Success criteria
- Risk mitigation table
- Next steps

**Target Audience:** Project managers, stakeholders, reviewers

---

### 3. **IMPLEMENTATION_CHECKLIST.md** (Task Tracker)
**Size:** 301 lines (9.9 KB)  
**Purpose:** Day-to-day implementation tracking

**Contents:**
- Pre-implementation setup (5 tasks)
- Phase 1: Hash Infrastructure (13 tasks)
- Phase 2: Import Graph & SCC Detection (13 tasks)
- Phase 3: Iterative SCC Solver (15 tasks)
- Phase 4: Hash Integration (10 tasks)
- Phase 5: Testing & Documentation (18 tasks)
- Final Integration & Release (16 tasks)
- Progress tracking template

**Total:** 85 actionable checklist items

**Target Audience:** Implementer (daily tracking)

---

## 🎯 How to Use These Documents

### For Project Approval
1. Read **PLAN_SUMMARY.md** (5 minutes)
2. Review success criteria and risks
3. Check timeline estimate (10-14 days)
4. Approve or request changes

### For Implementation
1. Read **plan.md** in full (30-45 minutes)
2. Review architecture diagrams (Appendix D)
3. Understand algorithm details (Appendix A)
4. Use **IMPLEMENTATION_CHECKLIST.md** to track progress
5. Check off tasks as completed
6. Refer back to plan.md for technical details

### For Code Review
1. Use **PLAN_SUMMARY.md** to understand goals
2. Check **IMPLEMENTATION_CHECKLIST.md** for completion
3. Refer to plan.md Section 6 (Affected Files) for context
4. Verify against acceptance criteria (Section 2)

---

## 🏗️ Implementation Phases

| Phase | Duration | Key Deliverables |
|-------|----------|------------------|
| **Phase 1: Hash Infrastructure** | 1-2 days | SHA-256 hashing, DependencySpec extension |
| **Phase 2: Import Graph & SCC** | 2-3 days | Tarjan's algorithm, SCC detection |
| **Phase 3: Iterative Solver** | 3-4 days | Fixpoint solving, convergence detection |
| **Phase 4: Hash Integration** | 2 days | .ksif generation with hashes |
| **Phase 5: Testing & Docs** | 2-3 days | Test suites, documentation |

**Total Estimate:** 10-14 days

---

## 📦 New Files to Create

1. `src/types/import_graph.rs` - Import graph data structure and Tarjan's algorithm
2. `src/types/scc_solver.rs` - Iterative fixpoint solver for cyclic SCCs
3. `tests/cycle_simple/A.ks` - Test case: 2-module cycle (A ↔ B)
4. `tests/cycle_simple/B.ks` - Test case: 2-module cycle
5. `tests/cycle_three/A.ks` - Test case: 3-module cycle (A → B → C → A)
6. `tests/cycle_three/B.ks` - Test case: 3-module cycle
7. `tests/cycle_three/C.ks` - Test case: 3-module cycle
8. `docs/ksif-hash-format.md` - Hash computation specification

---

## 🔧 Files to Modify

1. **Cargo.toml**
   - Add dependency: `sha2 = "0.10"`

2. **src/ksif.rs**
   - Add `compute_shape_hash()` function
   - Extend `DependencySpec` with `content_hash` field
   - Add hash verification logic

3. **src/types.rs**
   - Integrate SCC detection into `ModuleLoader`
   - Update `collect_imports()` to build import graph
   - Allow cycles (remove immediate error)

4. **src/cli/cli_compile.rs**
   - Update `emit_ksif()` to compute and include hashes

5. **docs/ksif-stage3-design.md**
   - Add section on SCC resolution
   - Document hash computation

---

## ✅ Success Criteria

### Minimum Viable Product (MVP)
- ✓ Dependency hashes included in `.ksif` files
- ✓ SCC detection working with Tarjan's algorithm
- ✓ Simple 2-module cycles (A ↔ B) resolve correctly
- ✓ All existing 339 tests continue to pass

### Full Success
- ✓ MVP criteria met
- ✓ N-way cycles (3+ modules) work correctly
- ✓ Nested/multiple SCCs handled properly
- ✓ Performance: <100ms for 50-module project
- ✓ Clear error messages for non-convergent cycles

---

## 🧪 Testing Strategy

- **Unit Tests:** 15+ tests for hash, Tarjan, SCC solver
- **Integration Tests:** 5+ scenarios for cyclic modules
- **Regression Tests:** All 339 existing tests must pass
- **Performance Tests:** Benchmark with 20-50 module projects

---

## 🚨 Risks and Mitigation

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Non-convergence in solver | Medium | High | Max iterations (10) + clear error |
| Performance degradation | Low | Medium | Tarjan is O(V+E), efficient |
| Hash collisions | Very Low | Low | SHA-256 (cryptographic) |
| Breaking existing workflows | Low | Medium | Graceful degradation |

---

## 🔄 Next Steps

### Immediate (Day 1)
1. ✅ Review and approve plan.md
2. Add `sha2` to Cargo.toml
3. Implement `compute_shape_hash()` in src/ksif.rs
4. Write hash stability tests
5. Begin Tarjan's algorithm

### Short-term (Week 1)
- Complete Phase 1 (Hash Infrastructure)
- Complete Phase 2 (SCC Detection)
- Start Phase 3 (Iterative Solver)

### Medium-term (Week 2)
- Complete Phase 3 (Iterative Solver)
- Complete Phase 4 (Hash Integration)
- Complete Phase 5 (Testing & Docs)

---

## 📖 Quick Reference

- **Main spec:** plan.md
- **Quick overview:** PLAN_SUMMARY.md
- **Task tracking:** IMPLEMENTATION_CHECKLIST.md
- **This index:** PLANNING_INDEX.md

---

## 🤝 Team Roles

- **Implementer:** Follows IMPLEMENTATION_CHECKLIST.md daily
- **Reviewer:** Uses plan.md Section 6-7 for context
- **Project Manager:** Tracks via IMPLEMENTATION_CHECKLIST.md progress
- **Stakeholder:** Reviews PLAN_SUMMARY.md for decisions

---

## 📞 Questions?

- Technical details: See plan.md
- Quick summary: See PLAN_SUMMARY.md
- Task status: See IMPLEMENTATION_CHECKLIST.md
- Algorithms: See plan.md Appendix A
- Examples: See plan.md Appendix B

---

**Status:** Planning Complete ✅  
**Ready for:** Implementation Phase 1  
**Last Updated:** 2024-01-28  
**Version:** 1.0
