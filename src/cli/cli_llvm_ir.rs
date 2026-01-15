use crate::Result;
#[cfg(feature = "llvm")]
use std::path::Path;

pub fn cmd_llvm_ir<I, S>(mut args: I) -> Result<()>
where
    I: Iterator<Item = S>,
    S: Into<String>,
{
    let _path: String = args
        .next()
        .ok_or_else(|| crate::error::Error::msg("missing <file>"))?
        .into();

    #[cfg(feature = "llvm")]
    {
        let tm = crate::types::typecheck_file(Path::new(&_path))?;
        let irm = crate::ir::lower_to_ir(&tm.module)?;

        // MVP: lower a small subset of IR to LLVM IR text.
        let llvm_ir =
            kscr_llvm::lower_ir_to_llvm_text(&irm, "main").map_err(crate::error::Error::msg)?;
        print!("{}", llvm_ir);
        Ok(())
    }

    #[cfg(not(feature = "llvm"))]
    {
        Err(crate::error::Error::msg(
            "llvm-ir command requires --features llvm",
        ))
    }
}
