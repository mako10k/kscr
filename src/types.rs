//! Type checking and elaboration scaffolding.
//!
//! Policy (docs):
//! - Surface numeric types: Integer (arbitrary precision) and Float64.
//! - Backend/IR numeric types are LLVM-aligned (i32/i64/f32/f64...).
//! - Pure IR subtyping allows only integer widening (iN <: iM); float widening is NOT subtyping.
//! - Potentially lossy conversions happen only at boundaries as checked casts.

use crate::{ast, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedModule {
    pub module: ast::Module,
}

pub fn typecheck(module: ast::Module) -> Result<TypedModule> {
    // TODO: HM + contextual overloading + alias expansion
    Ok(TypedModule { module })
}
