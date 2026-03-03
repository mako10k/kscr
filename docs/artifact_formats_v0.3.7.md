> [!IMPORTANT]
> Archive Notice: This document is a historical snapshot kept for evidence.
> It may not reflect current implementation behavior.
> Current source of truth: `docs/DOC_INDEX.md` and documents classified as `Current`.
> Edit policy: preserve original content; append-only updates are preferred.

# Artifact Formats Status: v0.3.7

This document describes the actual implementation status of binary artifact formats in kscr v0.3.7 as of commit 9b21a20.

**Last Updated**: 2025-01-28  
**Target Version**: v0.3.7  
**Context**: Issue #61 (KSIF dependency hashing completion)

---

## Summary Table

| Format  | Status            | File Extension | Implementation Location | Purpose |
|---------|-------------------|----------------|-------------------------|---------|
| KSIF    | **Implemented**   | `.ksif`        | `src/kir1.rs`           | Interface-only artifact (types + dependencies) |
| KIR1    | **Implemented**   | `.kir`         | `src/kir1.rs`           | Binary IR container (whole-program) |
| KSCM    | **Proposal only** | `.kscm`        | None                    | Compiled module artifact (interface + implementation) |
| KSOBJ   | **Mentioned only**| (undefined)    | None                    | Object file format (out-of-scope) |
| KSM     | **N/A (see note)**| N/A            | N/A                     | See "KSM Clarification" below |

---

## 1. KSIF (KScript Interface Format)

### Status: **Implemented** ✅

KSIF is the primary interface artifact format for separate compilation, currently implemented in kscr v0.3.7.

### Location
- **Source**: `src/kir1.rs` (lines 23-31, 33-94, 96-170)
- **Related commits**:
  - `9b21a20` – KSIF dependencies + cache scoping (Issue #61 AC2)
  - `e1ddcbf` – Plan documentation for dependency hashing

### Current Capabilities

**Data Structure** (`KsifModule`):
```rust
pub struct KsifModule {
    pub module_name: String,
    pub values: Vec<(String, Scheme)>,  // Exported value schemes
    pub dependencies: Vec<(String, String)>,  // (module_name, sha256_hash)
}
```

**Container Format**: KIR1 binary container with:
- Magic bytes: `"KIR1"`
- Version: `0.1`
- Sections:
  - `SECTION_STRINGS (1)`: String interning table
  - `SECTION_INTERFACE (4)`: Exported schemes + dependency manifest

**Encoding**: `encode_ksif_module()` at line 33  
**Decoding**: `decode_ksif_module()` at line 96

### Producers
- `cargo run -- compile <file.ks>` writes `.ksif` to `./target/ksif/<Module>.ksif`
- CLI flag: `--ksif-out <dir>` to override output directory

### Consumers
- Dependency hash validation for incremental builds (Issue #61 AC2 complete)
- Cache invalidation when dependencies change (scoped by dependency hash)

### Limitations
- No compression support yet (planned in KIR1 proposal)
- No data/type definitions yet (only value schemes)
- Dependency hashes are stored but not yet used for full rebuild decisions

---

## 2. KIR1 (Binary IR Container)

### Status: **Implemented** ✅

KIR1 is the underlying binary container format used by KSIF. It can also store whole-program IR modules.

### Location
- **Source**: `src/kir1.rs` (entire file)
- **Related docs**: `docs/BinaryIRFormat.md` (proposal for future extensions)

### Current Capabilities

**File Header** (lines 15-17):
```rust
const MAGIC: [u8; 4] = *b"KIR1";
const VERSION_MAJOR: u16 = 0;
const VERSION_MINOR: u16 = 1;
```

**Section IDs** (lines 19-21):
```rust
const SECTION_STRINGS: u32 = 1;    // String interning
const SECTION_INTERFACE: u32 = 4;  // KSIF interface
const SECTION_IR: u32 = 5;         // Full IR module bodies
```

**Functions**:
- `encode_kir1_module(module: &IrModule)` – Packs whole-program IR (line 217)
- `decode_kir1_module(input: &[u8])` – Unpacks whole-program IR (not shown, but analogous)
- `encode_ksif_module(module: &KsifModule)` – Packs interface-only (line 33)
- `decode_ksif_module(input: &[u8])` – Unpacks interface-only (line 96)

### Producers
- `kscr compile` with `--emit kir` (planned, not yet implemented in CLI)
- Currently used internally for KSIF generation

### Consumers
- `decode_ksif_module()` reads KSIF artifacts for dependency tracking
- Whole-program IR decoding (Stage 1 complete, per `docs/BinaryIRFormat.md` line 13)

### Future Extensions (Proposed)
See `docs/BinaryIRFormat.md` for planned sections:
- `SECTION_SYMBOLS (2)`: Qualified name table
- `SECTION_TYPES (3)`: Type graph + schemes + constraints
- `SECTION_DEPGRAPH (6)`: Dependency hashes (partially implemented in KSIF)
- `SECTION_BUILDINFO (7)`: Compiler version, feature flags
- Compression support (`COMPRESSED_ZSTD` flag)

---

## 3. KSCM (KSC Module)

### Status: **Proposal Only** 📝

KSCM is a **proposed** artifact format for compiled modules containing both interface and implementation. It is documented but **not implemented** in v0.3.7.

### Location
- **Documentation**: `docs/BinaryIRFormat.md` (lines 109-115, 447-472)
- **Implementation**: None (no code in `src/` or `crates/`)

### Proposed Design

From `docs/BinaryIRFormat.md` (lines 109-115):

> - `.ksif` (interface-only): contains `STRINGS + SYMBOLS + TYPES + INTERFACE (+ DEPGRAPH, BUILDINFO)`.
> - `.kscm` (module implementation): contains everything in `.ksif` plus `IR` (and optionally debug sections).
> - (Optional) `.kir` (whole-program bundle): a single `KIR1` with all modules merged; convenient for distribution.

**Intended Purpose**: Separate compilation artifact with full implementation (line 449):
```
A `.kscm` file is a `KIR1` container that **must** contain:
- STRINGS
- SYMBOLS
- TYPES
- INTERFACE

and **may** contain:
- IR (when distributing implementation)
- DEPGRAPH
- BUILDINFO
```

### Proposed CLI Integration (line 123)
```bash
# Planned but not yet implemented
kscr compile --emit kscm <file.ks>  # writes ./target/kscr/<Module>.kscm
```

### Rationale for Non-Implementation

KSCM depends on several sections not yet implemented in KIR1:
- `SECTION_SYMBOLS` (qualified names)
- `SECTION_TYPES` (type graph serialization)
- Full dependency graph tracking

Current v0.3.7 provides KSIF as a minimal interface-only artifact, which is sufficient for dependency hash tracking (Issue #61 scope).

---

## 4. KSOBJ (Object File Format)

### Status: **Mentioned, Out-of-Scope** ❌

KSOBJ is briefly mentioned in planning documents but has no design, implementation, or active plans.

### Location
- **Documentation**: `docs/plans/plan.md` (line 18)

From `docs/plans/plan.md` (line 18):
```markdown
**Out of Scope:**
- `.ksobj` format (not requested)
```

### Context
KSOBJ appears to have been considered as a native object file format (similar to `.o` files in C/C++ compilation), but was explicitly excluded from Issue #31 (module system redesign) and Issue #61 (KSIF dependency hashing).

### Status Determination
**Out-of-scope**: No requirements, no design, no implementation planned for v0.3.x series.

---

## 5. KSM Clarification

### Decision: **KSM is a typographical error** ⚠️

**Evidence**:
1. No occurrences of "KSM" found in:
   - Source code (`src/`, `crates/`)
   - Documentation (`docs/*.md`)
   - README.md
   - Issue tracker references

2. Likely confusion sources:
   - **KSIF** (interface format) ← "KS-I-F"
   - **KSCM** (proposed compiled module) ← "KS-C-M"
   - **KSM** appears to be a misremembered abbreviation

3. Naming pattern in the codebase:
   - `KIR1` = KScript IR version 1
   - `KSIF` = KScript Interface Format
   - `KSCM` = KScript Compiled Module
   - `KSM` breaks this pattern (no "C" for compiled, no descriptor)

### Recommendation
Where "KSM" appears in discussions:
- If referring to **interface artifacts**, use **KSIF**
- If referring to **compiled modules with IR**, use **KSCM** (proposal) or **KIR1** (current implementation)
- If found in documentation, replace with the correct term based on context

---

## Implementation Roadmap

### Completed (v0.3.7)
- ✅ KIR1 binary container format (header + section table)
- ✅ KSIF structure with dependency hashing
- ✅ String interning (`SECTION_STRINGS`)
- ✅ Interface section (`SECTION_INTERFACE`)
- ✅ Dependency manifest: `Vec<(String, String)>` with SHA-256 hashes
- ✅ Cache scoping by dependency hash (Issue #61 AC2)

### In Progress
- 🚧 Dependency hash validation in build pipeline (Issue #61 AC3)
- 🚧 Documentation of hash computation algorithm (Issue #61 AC1)

### Planned (Future)
- ⏳ `SECTION_TYPES` for type graph serialization
- ⏳ `SECTION_SYMBOLS` for qualified name resolution
- ⏳ KSCM implementation (interface + IR in single artifact)
- ⏳ Compression support (zstd)
- ⏳ Full incremental build system using KSIF hashes

---

## References

### Source Files
- `src/kir1.rs`: KIR1 + KSIF implementation
- `src/ir_pack.rs`: Legacy IR packing (pre-KIR1, still in use)

### Documentation
- `docs/BinaryIRFormat.md`: KIR1/KSCM/KSIF proposal (design doc)
- `docs/ksif-stage3-design.md`: KSIF v1 design (JSON-based, now superseded by KIR1-based KSIF)
- `docs/plans/plan.md`: Issue #61 implementation plan

### Commits
- `9b21a20`: KSIF dependencies + cache scoping ✅
- `e1ddcbf`: KSIF dependency hashing plan docs ✅

### Issues
- Issue #61: KSIF dependency hashing and cache invalidation
- Issue #31: Module system redesign + KSIF vNext

---

## Glossary

| Term     | Definition |
|----------|------------|
| **KSIF** | KScript Interface Format – binary artifact containing exported types and dependency hashes |
| **KIR1** | KScript IR version 1 – binary container format with sections (underlies KSIF) |
| **KSCM** | KScript Compiled Module – proposed artifact format with interface + implementation |
| **KSOBJ**| KScript Object file – mentioned, out-of-scope, no design |
| **KSM**  | Typographical error; use KSIF or KSCM instead |
| **IR**   | Intermediate Representation – kscr's AST-based intermediate language |

---

## Maintenance

This document should be updated when:
- New artifact formats are implemented
- KSCM implementation begins
- KIR1 section support expands
- Dependency hashing algorithm changes
- Version number increments (v0.4.0+)

**Document Owner**: Issue #61 tracking  
**Next Review**: v0.4.0 release prep
