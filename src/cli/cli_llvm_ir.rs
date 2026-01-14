use crate::Result;

pub fn cmd_llvm_ir<I, S>(mut args: I) -> Result<()>
where
    I: Iterator<Item = S>,
    S: Into<String>,
{
    let _path = args
        .next()
        .ok_or_else(|| crate::error::Error::msg("missing <file>"))?
        .into();

    #[cfg(feature = "llvm")]
    {
        // MVP: placeholder LLVM IR. Later this will lower kscr IR to LLVM IR.
        let llvm_ir = kscr_llvm::generate_llvm_ir_text("main").map_err(crate::error::Error::msg)?;
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
