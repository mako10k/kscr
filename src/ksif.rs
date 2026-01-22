//! KSIF vNext: KScript Intermediate Format
//!
//! Stage 3: Separate module shape (interface) from content (payload).
//! This enables efficient cross-module references and dependency resolution.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
        }
    }

    /// Serialize to JSON
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize from JSON
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

    /// Serialize to JSON
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize from JSON
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
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
}
