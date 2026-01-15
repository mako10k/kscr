use crate::{ir, Result};

/// Kept for back-compat: the LLVM IR lowering lives in `crates/kscr_llvm`.
///
/// This wrapper exists so older call sites can keep using `crate::llvm_backend`.
pub fn lower_ir_to_llvm_text(module: &ir::IrModule, module_name: &str) -> Result<String> {
    kscr_llvm::lower_ir_to_llvm_text(module, module_name).map_err(crate::error::Error::msg)
}
