//! IR scaffolding.

use crate::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrModule {
    pub items: Vec<IrItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrItem {
    // Placeholder.
    Nop,
}

pub fn lower_to_ir() -> Result<IrModule> {
    // TODO: lower typed AST to IR; expand type aliases; insert checked casts at boundaries.
    Ok(IrModule { items: vec![] })
}
