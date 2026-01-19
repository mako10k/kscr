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
    let mut ksif_out_dir: Option<PathBuf> = None;
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
            "--ksif-out" => {
                let out = args
                    .next()
                    .ok_or_else(|| crate::error::Error::msg("missing <dir> for --ksif-out"))?
                    .into();
                ksif_out_dir = Some(PathBuf::from(out));
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

    // Stage 2 (MVP): emit an interface-only artifact (.ksif) that carries exported value schemes.
    // This is not yet consumed by the compiler pipeline; it is produced for upcoming work.
    emit_ksif(&input_path, &tm, ksif_out_dir.as_deref())?;

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

    // Default (stage 1): typecheck -> lower to IR -> embed KIR1 -> compile a tiny Rust runner
    // that decodes the IR and feeds it to the existing executor.
    let kir1 = crate::kir1::encode_kir1_module(&irm);
    compile_rust_runner(&input_path, &output_path, &kir1, release)
}

fn emit_ksif(
    input_path: &Path,
    tm: &crate::types::TypedModule,
    ksif_out_dir: Option<&Path>,
) -> Result<()> {
    use std::io::Write;

    let module_name = tm
        .module
        .name
        .clone()
        .unwrap_or_else(|| "Main".to_string());

    // Reuse CLI export filtering logic (keeps export surface consistent).
    let exported = crate::cli_impl::filter_inferred_by_exports(&tm.module, tm.inferred.clone());
    let values: Vec<(String, crate::types::Scheme)> = exported;

    let ksif = crate::kir1::KsifModule {
        module_name,
        values,
    };
    let bytes = crate::kir1::encode_ksif_module(&ksif);

    let out_path = match ksif_out_dir {
        Some(dir) => {
            std::fs::create_dir_all(dir)?;
            let file_stem = input_path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| crate::error::Error::msg("invalid input filename"))?;
            dir.join(format!("{file_stem}.ksif"))
        }
        None => default_ksif_output_path(input_path),
    };

    let mut f = std::fs::File::create(&out_path)?;
    f.write_all(&bytes)?;
    Ok(())
}

fn default_ksif_output_path(input_path: &Path) -> PathBuf {
    // Default: write under ./target/ksif/<file>.ksif
    // (keeps sources clean; artifact is treated as build output)
    let file_stem = input_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("module");
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("ksif")
        .join(format!("{file_stem}.ksif"))
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
    kir1_bytes: &[u8],
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

    let packed_path = build_dir.join("kir1.bin");
    {
        let mut f = std::fs::File::create(&packed_path)?;
        f.write_all(kir1_bytes)?;
    }

    let main_rs_path = build_dir.join("main.rs");
    {
        let mut f = std::fs::File::create(&main_rs_path)?;
        // Note: keep this runner minimal; it just decodes packed IR and runs main.
        writeln!(
            f,
            "use kscr::ir;\nuse kscr::kir1;\n\nfn main() {{\n    let bytes: &[u8] = include_bytes!(\"kir1.bin\");\n    let module = match kir1::decode_kir1_module(bytes) {{\n        Ok(m) => m,\n        Err(e) => {{ eprintln!(\"kscr: failed to decode KIR1: {{:?}}\", e); std::process::exit(1); }}\n    }};\n\n    if let Err(e) = ir::run_main(&module) {{\n        eprintln!(\"kscr: runtime error: {{}}\", e);\n        std::process::exit(1);\n    }}\n}}\n"
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
        "compiled {} -> {} (KIR1 embedded from {})",
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

    let llvm_ir =
        kscr_llvm::lower_ir_to_llvm_text(ir_module, "main").map_err(crate::error::Error::msg)?;

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
