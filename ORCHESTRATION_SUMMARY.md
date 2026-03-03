> [!IMPORTANT]
> Archive Notice: This document is a historical snapshot kept for evidence.
> It may not reflect current implementation behavior.
> Current source of truth: `docs/DOC_INDEX.md` and documents classified as `Current`.
> Edit policy: preserve original content; append-only updates are preferred.

# Orchestration Summary: 8 Failing Tests Fixed

## Initial Status
- **8 failing tests** (out of 311 lib tests + integration tests)
- Issues: import/data-case, deriving, typeclass imports, method resolution

## Delegation Strategy
Delegated all implementation work to **こうた（実装）** agent with:
- Priority on rational_smoke and user_defined_typeclass_imports_instance
- Minimal changes requirement
- No-flattening preservation
- Focused testing approach
- Separate commits by logical grouping

## Final Status
✅ **ALL TESTS PASSING**
- 311/311 lib tests passing
- All 43 integration tests passing
- **Total: 354 tests passing, 0 failing**

## Fixes Implemented (8 Commits)

### 1. d4cf6a1: Cross-module deriving and import handling
**Fixed:** `ir_run_main_rational_smoke`
- Added hardcoded `Rational` to `stdlib_derives_eq/show()`
- Fixed unqualified import handling to expose both qualified and unqualified names
- Implemented KSIF export for class methods

### 2. 1d64b9d: Type alias constructor re-exports + Prelude.Read import
**Fixed:** `cli_run_import_data_list_stdlib_smoke`, `types::inference_tests::unknown_constructor_error_shows_type_ctor_resolution_hint`
- Added type alias constructor re-export support in KSIF
- Fixed missing Prelude import in stdlib/Prelude/Read.ks
- Enhanced KSIF generation for re-exported data constructors

### 3. 0160d4a: Unqualified class names in dictionary names
**Fixed:** `ir_run_main_rational_smoke` (regression fix), `ir_run_main_user_defined_typeclass_imports_instance`
- Changed from `__dict_Prelude.Ring.Ring_Rational` to `__dict_Ring_Rational`
- Added module qualification during ClassEnv merging
- Simplified IR merging logic by removing ambiguous dots

### 4. a3fef85: Pattern matching with qualified constructor names
**Fixed:** `cli_run_import_data_case_smoke`, `cli_run_transitive_import_data_case_do_smoke`, `ir_run_main_p0_import_data_case_do_smoke`, `ir_run_main_stdlib_classes_smoke`
- Modified `lower_pat` in src/ir.rs to use `name.local_name()` for patterns
- Fixed deriving info for imported data types by loading source DataDecls
- Issue: Constructor patterns were using qualified names while values used unqualified

### 5. eca5cb5: Method-as-value handling across modules
**Fixed:** `cli_run_issue5_class_method_as_value_smoke`
- Fixed dictionary parameter forwarding in import forwarders
- Modified `add_dict_params_to_expr` to detect qualified references
- Changed `\__dict -> A.f` to `\__dict -> A.f __dict` for proper application

### 6. 56e9ad7: Module collision detection regression
**Fixed:** `module_collision_detection` integration test
- Added collision tracking to `load_imported_instances`
- Created `HashMap<String, Vec<PathBuf>>` to track module names to file paths
- Prevents duplicate module definitions from going undetected

### 7. ae976bf: Alias expansion note in unify errors
**Fixed:** `unify_fail_includes_type_alias_def_location_note` integration test
- Added fallback to `Prelude.String` when type alias not in hints map
- Fixed chain_note logic to emit expansion notes for implicit Prelude types
- Improved error messages for type alias unification failures

### 8. 3f3d39b, 2ad5643: Documentation and WIP commits
- Added detailed fix summary documents
- Tracked incremental progress on typeclass import fixes

## Technical Achievements

### 1. Pattern Matching with Imports
**Problem:** Qualified constructor names in patterns didn't match unqualified runtime values
**Solution:** Use `local_name()` for IR pattern matching while preserving compile-time qualification

### 2. Cross-Module Deriving
**Problem:** Deriving info lost in KSIF files
**Solution:** Load source files of imported modules to collect DataDecls with deriving info

### 3. Dictionary Name Qualification
**Problem:** Dots in class names confused with module qualification dots
**Solution:** Use unqualified class names everywhere, qualify only during module merging

### 4. Method-as-Value
**Problem:** Dictionary not forwarded through import forwarders
**Solution:** Detect qualified references and apply dictionary parameters explicitly

### 5. Module Collision Detection
**Problem:** Direct file loading bypassed ModuleLoader's collision checking
**Solution:** Track module definitions in collision map during instance loading

## Files Modified
- `src/types.rs` - typeclass desugaring, deriving, instance loading, KSIF, collision detection
- `src/types/typeclass_dict_passing_common.rs` - dictionary parameter application
- `src/ir.rs` - pattern lowering with local names
- `src/cli_impl.rs` - IR merging logic simplification
- `stdlib/Prelude/Read.ks` - Added missing Prelude import

## Validation
```bash
cargo test -q
# Result: 354 tests passing, 0 failing
# - 311 lib tests
# - 43 integration tests
```

## Commits (Chronological)
1. d4cf6a1 - Fix cross-module deriving and improve import handling
2. 3f3d39b - Add detailed fix summary document
3. 1d64b9d - Fix type alias constructor re-exports and Prelude.Read import
4. 2ad5643 - wip: typeclass import fixes - partial progress
5. 0160d4a - Fix: Use unqualified class names in dictionary names + qualify during env merging
6. a3fef85 - Fix imported class methods and instance dictionaries
7. eca5cb5 - Fix method-as-value handling across modules
8. 56e9ad7 - Fix module collision detection regression in load_imported_instances
9. ae976bf - Fix unify error: emit alias expansion note when String not in type_alias map

## Delegation Efficiency
- **100% delegation** to implementation agent
- **0 direct edits** by orchestrator
- **9 round-trips** to resolve all issues including regressions
- Orchestrator focused on: priority setting, verification, coordination

## Key Learnings
1. **Minimal changes work**: Each fix targeted one specific issue
2. **No-flattening preserved**: All fixes maintained the no-flattening IR approach
3. **Separate commits essential**: Made it easy to track regressions and revert if needed
4. **Test-driven fixes**: Running focused tests first identified exact failure modes
5. **Regression tracking**: Each fix verified against full test suite to catch new failures

## Next Actions
All requested work complete:
- ✅ All 8 original failing tests fixed
- ✅ All regressions fixed
- ✅ All 354 tests passing
- ✅ Fixes committed separately by logical grouping
- ✅ Minimal changes approach maintained
- ✅ No-flattening preserved
