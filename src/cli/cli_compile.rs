use crate::Result;
use std::path::{Path, PathBuf};

pub fn cmd_compile<I, S>(mut args: I) -> Result<()>
where
    I: Iterator<Item = S>,
    S: Into<String>,
{
    let input_path: PathBuf = args
        .next()
        .ok_or_else(|| crate::error::Error::msg("missing <file>"))?
        .into()
        .into();

    let mut output_path: Option<PathBuf> = None;
    let mut release: bool = false;
    let mut use_llvm: bool = false;
    while let Some(arg) = args.next() {
        let arg: String = arg.into();
        match arg.as_str() {
            "-o" | "--output" => {
                let out = args
                    .next()
                    .ok_or_else(|| crate::error::Error::msg("missing <output> for -o"))?
                    .into();
                output_path = Some(PathBuf::from(out));
            }
            "--release" => {
                release = true;
            }
            "--llvm" => {
                use_llvm = true;
            }
            _ => return Err(crate::error::Error::msg(format!("unknown arg: {arg}"))),
        }
    }

    let output_path = output_path.unwrap_or_else(|| default_output_path(&input_path));

    let tm = crate::types::typecheck_file(&input_path)?;
    let irm = crate::ir::lower_to_ir(&tm.module)?;

    if use_llvm {
        #[cfg(feature = "llvm")]
        {
            return compile_via_llvm(&input_path, &output_path, &irm, release);
        }

        #[cfg(not(feature = "llvm"))]
        {
            return Err(crate::error::Error::msg(
                "compile --llvm requires --features llvm",
            ));
        }
    }

    // Default (stage 1): typecheck -> lower to IR -> pack IR -> compile a tiny Rust runner
    // that decodes the IR and feeds it to the existing executor.
    let packed = crate::ir_pack::encode_ir_module(&irm);
    compile_rust_runner(&input_path, &output_path, &packed, release)
}

fn default_output_path(input_path: &Path) -> PathBuf {
    // Place output next to input: `foo.ks` -> `foo`
    let mut out = input_path.to_path_buf();
    out.set_extension("");
    out
}

fn compile_rust_runner(
    input_path: &Path,
    output_path: &Path,
    packed_ir: &[u8],
    release: bool,
) -> Result<()> {
    use std::io::Write;
    use std::process::Command;

    // Ensure we have a compiled `kscr` library artifact we can link against.
    let profile = if release { "release" } else { "debug" };
    let mut cargo = Command::new("cargo");
    cargo.arg("build").arg("-q").arg("--lib");
    if release {
        cargo.arg("--release");
    }
    cargo.current_dir(env!("CARGO_MANIFEST_DIR"));

    let status = cargo.status()?;
    if !status.success() {
        return Err(crate::error::Error::msg(format!(
            "cargo build --lib failed with status: {status}"
        )));
    }

    let deps_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join("deps");
    let kscr_rlib = find_latest_rlib(&deps_dir, "kscr")?;

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| crate::error::Error::msg(format!("time error: {e}")))?
        .as_nanos();
    let build_dir =
        std::env::temp_dir().join(format!("kscr_compile_{}_{}", std::process::id(), nanos));
    std::fs::create_dir_all(&build_dir)?;

    let packed_path = build_dir.join("packed_ir.bin");
    {
        let mut f = std::fs::File::create(&packed_path)?;
        f.write_all(packed_ir)?;
    }

    let main_rs_path = build_dir.join("main.rs");
    {
        let mut f = std::fs::File::create(&main_rs_path)?;
        // Note: keep this runner minimal; it just decodes packed IR and runs main.
        writeln!(
            f,
            "use kscr::ir;\nuse kscr::ir_pack;\n\nfn main() {{\n    let bytes: &[u8] = include_bytes!(\"packed_ir.bin\");\n    let module = match ir_pack::decode_ir_module(bytes) {{\n        Ok(m) => m,\n        Err(e) => {{ eprintln!(\"kscr: failed to decode packed IR: {{}}\", e); std::process::exit(1); }}\n    }};\n\n    match ir::run_main(&module) {{\n        Ok(v) => match v {{\n            ir::Value::Unit => println!(\"()\"),\n            other => println!(\"{{:#?}}\", other),\n        }},\n        Err(e) => {{ eprintln!(\"kscr: runtime error: {{}}\", e); std::process::exit(1); }}\n    }}\n}}\n"
        )?;
    }

    let mut rustc = Command::new("rustc");
    rustc
        .arg("--edition=2021")
        .arg(&main_rs_path)
        .arg("-o")
        .arg(output_path)
        .arg("-L")
        .arg(format!("dependency={}", deps_dir.display()))
        .arg("--extern")
        .arg(format!("kscr={}", kscr_rlib.display()));

    if release {
        rustc.arg("-O");
    }

    let status = rustc.status()?;
    if !status.success() {
        return Err(crate::error::Error::msg(format!(
            "rustc failed with status: {status}"
        )));
    }

    // Best-effort cleanup.
    let _ = std::fs::remove_dir_all(&build_dir);

    // Help users understand what's embedded.
    eprintln!(
        "compiled {} -> {} (packed IR from {})",
        input_path.display(),
        output_path.display(),
        input_path.display()
    );

    Ok(())
}

#[cfg(feature = "llvm")]
fn compile_via_llvm(
    input_path: &Path,
    output_path: &Path,
    ir_module: &crate::ir::IrModule,
    release: bool,
) -> Result<()> {
    use std::io::Write;
    use std::process::Command;

    let llvm_ir = kscr_llvm::lower_ir_to_llvm_text(ir_module, "main")
        .map_err(crate::error::Error::msg)?;

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| crate::error::Error::msg(format!("time error: {e}")))?
        .as_nanos();
    let build_dir = std::env::temp_dir().join(format!(
        "kscr_compile_llvm_{}_{}",
        std::process::id(),
        nanos
    ));
    std::fs::create_dir_all(&build_dir)?;

    let ll_path = build_dir.join("module.ll");
    {
        let mut f = std::fs::File::create(&ll_path)?;
        f.write_all(llvm_ir.as_bytes())?;
    }

    let mut clang = Command::new("clang");
    clang.arg(&ll_path).arg("-o").arg(output_path);
    if release {
        clang.arg("-O3");
    } else {
        clang.arg("-O0");
    }

    let status = clang.status()?;
    if !status.success() {
        return Err(crate::error::Error::msg(format!(
            "clang failed with status: {status}"
        )));
    }

    // Best-effort cleanup.
    let _ = std::fs::remove_dir_all(&build_dir);

    eprintln!(
        "compiled {} -> {} (LLVM backend, from {})",
        input_path.display(),
        output_path.display(),
        input_path.display()
    );

    Ok(())
}

fn find_latest_rlib(dir: &Path, crate_name: &str) -> Result<PathBuf> {
    let prefix = format!("lib{}-", crate_name);
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for ent in std::fs::read_dir(dir)? {
        let ent = ent?;
        let p = ent.path();
        if p.extension().and_then(|s| s.to_str()) != Some("rlib") {
            continue;
        }
        let file_name = match p.file_name().and_then(|s| s.to_str()) {
            Some(s) => s,
            None => continue,
        };
        if !file_name.starts_with(&prefix) {
            continue;
        }
        let mtime = ent.metadata()?.modified().unwrap_or(std::time::UNIX_EPOCH);
        match &best {
            None => best = Some((mtime, p)),
            Some((best_time, _)) if mtime > *best_time => best = Some((mtime, p)),
            _ => {}
        }
    }
    best.map(|(_, p)| p).ok_or_else(|| {
        crate::error::Error::msg(format!(
            "could not find {crate_name} rlib under {}",
            dir.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_output_path_strips_ks_extension() {
        let p = PathBuf::from("/tmp/hello.ks");
        assert_eq!(default_output_path(&p), PathBuf::from("/tmp/hello"));
    }
}
