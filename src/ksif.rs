//! KSIF vNext: KScript Intermediate Format
//!
//! ## Overview
//!
//! Stage 3 of the module system design introduces KSIF (KScript Intermediate Format),
//! a serialized representation that separates module **shape** (interface) from
//! **content** (implementation payload).
//!
//! ## Key Concepts
//!
//! ### 1. ModuleShape (Interface)
//!
//! Contains everything needed for type checking and name resolution of dependent modules:
//! - Exported types, classes, values, instances
//! - Type signatures and class method signatures
//! - Dependency specifications
//! - **No implementation details**
//!
//! Benefits:
//! - Fast dependency scanning without parsing full modules
//! - Enables incremental compilation
//! - Clear separation of interface from implementation
//!
//! ### 2. ModuleContent (Payload)
//!
//! Contains the actual implementation:
//! - Value definitions (IR or AST)
//! - Instance method implementations
//! - Only needed for execution, not for type checking dependents
//!
//! ### 3. KSIF Header and Salt
//!
//! Every KSIF file includes a header with:
//! - **Version**: KSIF schema version (currently "1.0")
//! - **Salt**: Includes kscr interpreter version for cache safety
//!
//! The salt prevents accidental usage of incompatible cached modules across
//! different interpreter versions.
//!
//! ### 4. Module Collision Detection
//!
//! When multiple candidates provide the same canonical module path:
//! - If all have matching salt → acceptable duplication (pick any)
//! - If salts differ → error with detailed diagnostics
//!
//! Error messages include:
//! - Import site
//! - All conflicting candidates with file paths
//! - Salt/version for each candidate
//! - Suggested fixes
//!
//! ## Usage Example
//!
//! ```rust,ignore
//! use kscr::ksif::ModuleShape;
//!
//! // Extract shape from AST
//! let shape = ModuleShape::from_ast_module(&module, "Data.List".to_string());
//!
//! // Save to file
//! shape.save_to_file(Path::new("Data.List.ksif"))?;
//!
//! // Load later for dependency resolution
//! let loaded = ModuleShape::load_from_file(Path::new("Data.List.ksif"))?;
//! ```
//!
//! ## Design Goals
//!
//! - **Local-first**: Start with local package resolution
//! - **Registry-ready**: Metadata compatible with future central registry
//! - **Incremental**: Support incremental compilation
//! - **Safe**: Version-aware caching with automatic invalidation
//!
//! ## Serialization Format
//!
//! **Current implementation**: JSON via serde
//!
//! The current serialization uses JSON for simplicity and debuggability during development.
//! However, this is **not a permanent choice**. Future versions may migrate to more efficient
//! formats such as:
//! - Protocol Buffers (protobuf)
//! - Cap'n Proto
//! - MessagePack
//! - Custom binary format
//!
//! The serialization format is versioned via `KsifHeader.ksif_version` to support migration.
//! When changing formats, update the version and provide migration tools.
//!
//! **Design principle**: Keep serialization logic isolated to enable easy format changes.
//! The core types (`ModuleShape`, `ModuleContent`) should remain format-agnostic.
//!
//! ## Non-goals (for now)
//!
//! - Central registry integration
//! - Signature verification
//! - Lockfiles (future: Stage 2 of package resolution)

// Current serialization: serde + JSON
// Note: This is not permanent. Future versions may use Protocol Buffers,
// Cap'n Proto, MessagePack, or other formats for better performance.
// The ksif_version tracks format changes to support migration.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use crate::ast::ModuleId;

/// KSIF format version and salt.
///
/// The salt includes the kscr version to prevent accidental cross-version usage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KsifHeader {
    /// KSIF schema version (e.g., "1.0")
    pub ksif_version: String,
    /// Salt includes interpreter version for cache safety
    pub salt: String,
}

impl KsifHeader {
    pub fn current() -> Self {
        Self {
            ksif_version: "1.0".to_string(),
            // Include kscr version from Cargo.toml in the salt
            salt: format!("kscr-{}", env!("CARGO_PKG_VERSION")),
        }
    }

    pub fn is_compatible(&self) -> bool {
        self.ksif_version == "1.0" && self.salt == format!("kscr-{}", env!("CARGO_PKG_VERSION"))
    }
}

/// ModuleShape: interface-only representation.
///
/// Contains everything needed for name resolution and type checking
/// of modules that depend on this one, without the implementation details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleShape {
    /// KSIF header with version and salt
    pub header: KsifHeader,

    /// Canonical module path (e.g., "Data.List")
    pub canonical_path: String,

    /// Module identity (interned ID, established during loading)
    #[serde(skip)]
    pub module_id: Option<ModuleId>,

    /// Exported value declarations
    pub values: HashMap<String, ValueExport>,

    /// Exported type declarations
    pub types: HashMap<String, TypeExport>,

    /// Exported class declarations
    pub classes: HashMap<String, ClassExport>,

    /// Exported instance declarations
    pub instances: Vec<InstanceExport>,

    /// Dependencies (for package resolution)
    pub dependencies: Vec<DependencySpec>,

    /// Export specifications (module header exports)
    /// Optional for backward compatibility with existing .ksif files
    pub export_specs: Option<Vec<crate::ast::ExportSpec>>,
}

/// Exported value declaration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValueExport {
    pub name: String,
    /// Type scheme (e.g., "forall a. a -> a")
    pub scheme: String,
    // Future: could add a hash ID here
}

/// Exported type declaration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeExport {
    pub name: String,
    /// Kind (e.g., "*", "* -> *")
    pub kind: String,
    /// Type parameters arity
    pub arity: usize,
    /// Constructor names (for ADTs)
    pub constructors: Vec<String>,
}

/// Exported class declaration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassExport {
    pub name: String,
    /// Class parameter name
    pub param: String,
    /// Method signatures
    pub methods: Vec<ClassMethodSig>,
    /// Superclass constraints
    pub supers: Vec<String>, // Serialized predicate strings
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassMethodSig {
    pub name: String,
    pub scheme: String,
}

/// Exported instance declaration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceExport {
    /// Class name (for matching)
    pub class_name: String,
    /// Instance head type (serialized)
    pub instance_type: String,
    /// Context predicates (serialized)
    pub context: Vec<String>,
}

/// Dependency specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencySpec {
    pub name: String,
    pub version_req: String, // e.g., "^1.0.0"
}

/// ModuleContent: implementation payload.
///
/// Contains the actual implementation details needed for execution,
/// separate from the interface in ModuleShape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleContent {
    /// KSIF header
    pub header: KsifHeader,

    /// Canonical module path (must match the shape)
    pub canonical_path: String,

    /// Value definitions (IR or AST)
    pub value_defs: HashMap<String, String>, // Placeholder: serialized IR

    /// Instance method implementations
    pub instance_methods: Vec<InstanceImpl>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceImpl {
    pub class_name: String,
    pub instance_type: String,
    pub methods: HashMap<String, String>, // method name -> serialized implementation
}

impl ModuleShape {
    /// Create an empty shape for a given canonical path
    pub fn new(canonical_path: String) -> Self {
        Self {
            header: KsifHeader::current(),
            canonical_path,
            module_id: None,
            values: HashMap::new(),
            types: HashMap::new(),
            classes: HashMap::new(),
            instances: Vec::new(),
            dependencies: Vec::new(),
            export_specs: None,
        }
    }

    /// Build a ModuleShape from an AST Module.
    ///
    /// This extracts the interface information (types, classes, instances)
    /// without the implementation details.
    pub fn from_ast_module(module: &crate::ast::Module, canonical_path: String) -> Self {
        let mut shape = Self::new(canonical_path);

        // Preserve export specifications from module header
        shape.export_specs = module.export_specs.clone();

        for item in &module.items {
            match item {
                crate::ast::Item::Binding(b) => {
                    // For now, we don't track value exports in the shape
                    // (would need type inference results)
                    let _ = b;
                }
                crate::ast::Item::DataDecl(dd) => {
                    shape.types.insert(
                        dd.name.clone(),
                        TypeExport {
                            name: dd.name.clone(),
                            kind: "*".to_string(), // Simplified: would need kind inference
                            arity: dd.params.len(),
                            constructors: dd.ctors.iter().map(|c| c.name.clone()).collect(),
                        },
                    );
                }
                crate::ast::Item::ClassDecl(cd) => {
                    shape.classes.insert(
                        cd.name.clone(),
                        ClassExport {
                            name: cd.name.clone(),
                            param: cd.param.clone(),
                            methods: cd
                                .methods
                                .iter()
                                .map(|m| ClassMethodSig {
                                    name: m.name.clone(),
                                    scheme: format!("{:?}", m.ty), // Simplified: would serialize properly
                                })
                                .collect(),
                            supers: cd
                                .supers
                                .iter()
                                .map(|p| format!("{:?}", p)) // Simplified
                                .collect(),
                        },
                    );
                }
                crate::ast::Item::InstanceDecl(inst) => {
                    shape.instances.push(InstanceExport {
                        class_name: inst.class.name.clone(),
                        instance_type: format!("{:?}", inst.ty), // Simplified
                        context: inst
                            .preds
                            .iter()
                            .map(|p| format!("{:?}", p)) // Simplified
                            .collect(),
                    });
                }
                crate::ast::Item::Import(imp) => {
                    // Track dependencies from imports
                    if !shape.dependencies.iter().any(|d| d.name == imp.module) {
                        shape.dependencies.push(DependencySpec {
                            name: imp.module.clone(),
                            version_req: "*".to_string(), // Default: any version
                        });
                    }
                }
                _ => {}
            }
        }

        shape
    }

    /// Serialize to JSON.
    ///
    /// **Note**: JSON is the current serialization format for development convenience,
    /// but this may change in future versions (e.g., to Protocol Buffers, Cap'n Proto,
    /// or other formats). The `ksif_version` field in the header tracks format changes.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize from JSON.
    ///
    /// **Note**: This expects JSON format matching the current `ksif_version`.
    /// Future versions may support multiple formats or provide migration tools.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Validate header compatibility
    pub fn validate_header(&self) -> Result<(), String> {
        if !self.header.is_compatible() {
            return Err(format!(
                "Incompatible KSIF version or salt: expected {}, got {}",
                KsifHeader::current().salt,
                self.header.salt
            ));
        }
        Ok(())
    }

    /// Save ModuleShape to a file as JSON.
    pub fn save_to_file(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let json = self.to_json()?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load ModuleShape from a file.
    pub fn load_from_file(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let json = std::fs::read_to_string(path)?;
        let shape = Self::from_json(&json)?;
        shape.validate_header()?;
        Ok(shape)
    }
}

impl ModuleContent {
    /// Create empty content for a given canonical path
    pub fn new(canonical_path: String) -> Self {
        Self {
            header: KsifHeader::current(),
            canonical_path,
            value_defs: HashMap::new(),
            instance_methods: Vec::new(),
        }
    }

    /// Serialize to JSON.
    ///
    /// **Note**: JSON is the current serialization format for development convenience,
    /// but this may change in future versions (e.g., to Protocol Buffers, Cap'n Proto,
    /// or other formats). The `ksif_version` field in the header tracks format changes.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize from JSON.
    ///
    /// **Note**: This expects JSON format matching the current `ksif_version`.
    /// Future versions may support multiple formats or provide migration tools.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Save ModuleContent to a file as JSON.
    pub fn save_to_file(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let json = self.to_json()?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load ModuleContent from a file.
    pub fn load_from_file(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let json = std::fs::read_to_string(path)?;
        let content = Self::from_json(&json)?;
        // Validate header compatibility
        if !content.header.is_compatible() {
            return Err(format!(
                "Incompatible KSIF content version or salt: expected {}, got {}",
                KsifHeader::current().salt,
                content.header.salt
            )
            .into());
        }
        Ok(content)
    }
}

/// Module collision information for error reporting.
#[derive(Debug, Clone)]
pub struct ModuleCollision {
    pub canonical_path: String,
    pub candidates: Vec<ModuleCandidate>,
}

#[derive(Debug, Clone)]
pub struct ModuleCandidate {
    pub file_path: std::path::PathBuf,
    pub header_salt: String,
}

impl ModuleCollision {
    /// Format a helpful error message for module collision.
    pub fn error_message(&self, import_site: &str) -> String {
        let mut msg = format!(
            "Module collision detected for '{}' at import site: {}\n",
            self.canonical_path, import_site
        );
        msg.push_str("Multiple candidates found:\n");
        for (idx, candidate) in self.candidates.iter().enumerate() {
            msg.push_str(&format!(
                "  [{}] {}\n      (salt: {})\n",
                idx + 1,
                candidate.file_path.display(),
                candidate.header_salt
            ));
        }
        msg.push_str("\nSuggested fix:\n");
        msg.push_str("  - Tighten version constraints in package metadata\n");
        msg.push_str("  - Remove conflicting dependencies from search path\n");
        msg.push_str("  - Ensure all candidates have matching version/salt\n");
        msg
    }
}

/// Detect if multiple module candidates have conflicting salts.
///
/// Returns None if no collision, or Some(ModuleCollision) if candidates conflict.
pub fn detect_collision(
    canonical_path: &str,
    candidates: &[(std::path::PathBuf, ModuleShape)],
) -> Option<ModuleCollision> {
    if candidates.len() <= 1 {
        return None;
    }

    // Check if all candidates have the same salt
    let first_salt = &candidates[0].1.header.salt;
    let all_match = candidates
        .iter()
        .all(|(_, shape)| &shape.header.salt == first_salt);

    if all_match {
        // All candidates have identical salt - this is acceptable duplication
        None
    } else {
        // Conflicting salts - report collision
        Some(ModuleCollision {
            canonical_path: canonical_path.to_string(),
            candidates: candidates
                .iter()
                .map(|(path, shape)| ModuleCandidate {
                    file_path: path.clone(),
                    header_salt: shape.header.salt.clone(),
                })
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ksif_header_compatibility() {
        let header = KsifHeader::current();
        assert!(header.is_compatible());

        let incompatible = KsifHeader {
            ksif_version: "0.9".to_string(),
            salt: "old".to_string(),
        };
        assert!(!incompatible.is_compatible());
    }

    #[test]
    fn test_module_shape_serialization() {
        let mut shape = ModuleShape::new("Data.List".to_string());
        shape.values.insert(
            "map".to_string(),
            ValueExport {
                name: "map".to_string(),
                scheme: "forall a b. (a -> b) -> [a] -> [b]".to_string(),
            },
        );

        let json = shape.to_json().expect("serialization failed");
        let deserialized = ModuleShape::from_json(&json).expect("deserialization failed");

        assert_eq!(shape.canonical_path, deserialized.canonical_path);
        assert_eq!(shape.values.len(), deserialized.values.len());
        assert!(deserialized.validate_header().is_ok());
    }

    #[test]
    fn test_module_content_serialization() {
        let content = ModuleContent::new("Data.List".to_string());
        let json = content.to_json().expect("serialization failed");
        let deserialized = ModuleContent::from_json(&json).expect("deserialization failed");

        assert_eq!(content.canonical_path, deserialized.canonical_path);
    }

    #[test]
    fn test_module_content_header_validation() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        // Create a content with incompatible header
        let mut content = ModuleContent::new("Test.Module".to_string());
        content.header.salt = "incompatible-version".to_string();

        let json = content.to_json().expect("serialization failed");

        // Write to temp file
        let mut file = NamedTempFile::new().expect("failed to create temp file");
        file.write_all(json.as_bytes())
            .expect("failed to write temp file");

        // Loading should fail due to incompatible header
        let result = ModuleContent::load_from_file(file.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_collision_detection_no_collision() {
        let shape1 = ModuleShape::new("Data.List".to_string());
        let candidates = vec![(std::path::PathBuf::from("/a/Data.List.ks"), shape1)];
        assert!(detect_collision("Data.List", &candidates).is_none());
    }

    #[test]
    fn test_collision_detection_same_salt_ok() {
        let shape1 = ModuleShape::new("Data.List".to_string());
        let shape2 = ModuleShape::new("Data.List".to_string());
        let candidates = vec![
            (std::path::PathBuf::from("/a/Data.List.ks"), shape1),
            (std::path::PathBuf::from("/b/Data.List.ks"), shape2),
        ];
        // Same salt - no collision
        assert!(detect_collision("Data.List", &candidates).is_none());
    }

    #[test]
    fn test_collision_detection_different_salt_error() {
        let mut shape1 = ModuleShape::new("Data.List".to_string());
        shape1.header.salt = "v0.1.0".to_string();

        let mut shape2 = ModuleShape::new("Data.List".to_string());
        shape2.header.salt = "v0.2.0".to_string();

        let candidates = vec![
            (std::path::PathBuf::from("/a/Data.List.ks"), shape1),
            (std::path::PathBuf::from("/b/Data.List.ks"), shape2),
        ];

        let collision = detect_collision("Data.List", &candidates);
        assert!(collision.is_some());

        let collision = collision.unwrap();
        assert_eq!(collision.canonical_path, "Data.List");
        assert_eq!(collision.candidates.len(), 2);
    }
}
