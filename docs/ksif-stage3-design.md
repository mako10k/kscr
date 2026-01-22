# KSIF vNext Stage 3: Implementation Summary

## Overview

This document summarizes the Stage 3 implementation of KSIF (KScript Intermediate Format) for the module system redesign (Issue #31).

## What Was Implemented

### 1. KSIF Module Structure (`src/ksif.rs`)

#### ModuleShape (Interface-Only)
- Contains exported declarations: values, types, classes, instances
- Includes dependency specifications
- Serializable to JSON for caching
- No implementation details (bodies, method definitions)

**Purpose**: Enable fast dependency resolution and type checking without parsing full modules.

#### ModuleContent (Payload)
- Contains implementation details: value definitions, instance methods
- Separate from shape to enable incremental compilation
- Only loaded when execution is needed

**Purpose**: Keep implementation separate from interface.

#### KsifHeader (Version + Salt)
- Schema version: "1.0"
- Salt: Includes kscr version (`kscr-0.2.0`)
- Prevents cross-version cache corruption

**Purpose**: Automatic cache invalidation across interpreter versions.

### 2. Module Identity Integration

#### ModuleIdInterner Enhancements (`src/types.rs`)
- `get_canonical_name()`: Reverse lookup from ModuleId to canonical path
- `get_or_intern()`: Get or create ModuleId for canonical path
- `contains()`: Check if module is already interned

**Purpose**: Support KSIF loading with proper module identity resolution.

#### ClassId Resolution
- Updated `stdlib_cache.rs` comment to clarify resolution is deferred
- Resolution happens in `ModuleLoader.load_ast()` after cache load
- Ensures all ClassIds have proper ModuleIds, not dummy `ModuleId(0)`

**Purpose**: Fix known limitation where stdlib cache couldn't resolve ClassIds.

### 3. Module Collision Detection

#### `detect_collision()` Function
- Checks multiple candidates for the same canonical module path
- Same salt → OK (acceptable duplication)
- Different salt → Error (ambiguous/unsafe)

#### `ModuleCollision` Type
- Captures collision details: canonical path, all candidates
- `error_message()`: Generates helpful diagnostic with:
  - Import site
  - File paths of all conflicting candidates
  - Salt/version for each
  - Suggested fixes

**Purpose**: Clear error reporting when multiple incompatible versions exist in search path.

### 4. File I/O Support

#### ModuleShape
- `from_ast_module()`: Extract shape from parsed AST
- `save_to_file()`: Serialize to JSON file
- `load_from_file()`: Deserialize from JSON with validation

#### ModuleContent
- `save_to_file()`: Serialize to JSON
- `load_from_file()`: Deserialize from JSON

**Purpose**: Enable persistent caching of module shapes for dependency resolution.

### 5. Serialization Format

#### Current Implementation: JSON via serde

**Choice rationale**:
- JSON is human-readable for debugging during development
- Easy to inspect and validate manually
- Widely supported tooling

**Important note**: This is **not a permanent format choice**. The serialization format may migrate to more efficient alternatives in the future, such as:
- **Protocol Buffers (protobuf)**: Better performance, smaller size, backwards compatibility
- **Cap'n Proto**: Zero-copy deserialization, efficient
- **MessagePack**: Binary JSON alternative
- **Custom binary format**: Optimized for kscr-specific needs

**Migration strategy**:
- The `ksif_version` field in `KsifHeader` tracks the format version
- When changing formats, increment the version and provide migration tools
- Keep serialization logic isolated to enable easy format changes
- Core types (`ModuleShape`, `ModuleContent`) remain format-agnostic

## Testing

All existing tests pass (339 tests total):
- `test_ksif_header_compatibility`: Version/salt validation
- `test_module_shape_serialization`: Shape JSON roundtrip
- `test_module_content_serialization`: Content JSON roundtrip
- `test_collision_detection_no_collision`: Single candidate OK
- `test_collision_detection_same_salt_ok`: Multiple candidates with same salt OK
- `test_collision_detection_different_salt_error`: Multiple candidates with different salt → error

## Current Status

### Completed
- ✅ KSIF schema design (shape vs content)
- ✅ Version/salt header for cache safety
- ✅ ModuleIdInterner enhancements
- ✅ Collision detection with helpful errors
- ✅ File I/O for shapes and content
- ✅ AST extraction to ModuleShape
- ✅ All tests passing

### Remaining Work (Future PRs)
- ⏳ Integrate KSIF loading into ModuleLoader
- ⏳ Shape-first module loading path
- ⏳ Package resolution with version requirements
- ⏳ KSIF-based incremental compilation
- ⏳ Lockfile support (optional)

## Design Decisions

### 1. Canonical Module Identity
- Module identity is based on **canonical path** (e.g., "Data.List")
- Interned to `ModuleId` (u32) for efficient comparison
- ClassId is `(ModuleId, String)` pair

### 2. Collision Policy
- Multiple candidates with **same salt**: acceptable (pick any deterministically)
- Multiple candidates with **different salt**: error (ambiguous)
- Rationale: Same salt means same version, so they should be identical

### 3. Separation of Shape vs Content
- Shape is small, fast to load, sufficient for type checking
- Content is larger, only needed for execution
- Enables "compile once, type check dependents many times"

### 4. Salt Strategy
- Include interpreter version in salt
- Automatic invalidation when upgrading kscr
- Future: Could include content hash for more precise invalidation

## Integration with Existing Code

### No Breaking Changes
- All existing functionality preserved
- KSIF is additive, not replacing current AST-based loading
- ModuleIdInterner helpers are compatible with existing usage

### Stdlib Cache Fix
- Comment updated to clarify ClassId resolution is deferred
- No code changes needed; resolution already works correctly

## Next Steps (Stage 4+)

1. **Loader Integration**
   - Add KSIF cache alongside AST cache
   - Check for `.ksif` files before parsing `.ks`
   - Load shape first, then content only if needed

2. **Package Resolution**
   - Implement version requirement matching
   - Search multiple roots (KSIF_PATH)
   - Detect and report collisions

3. **Incremental Compilation**
   - Cache KSIF shapes per module
   - Invalidate only affected modules when source changes
   - Skip recompiling unchanged dependencies

4. **Lockfile Support** (optional)
   - Record resolved versions
   - Ensure reproducible builds
   - Warn when resolution would differ from lockfile

## References

- Issue: mako10k/kscr#31 (Design: module objects + interned IDs + local package resolution + KSIF vNext)
- Related: mako10k/kscr#34 (Stage 2: ClassId migration - completed)
- Design doc in issue comments by @mako10k
