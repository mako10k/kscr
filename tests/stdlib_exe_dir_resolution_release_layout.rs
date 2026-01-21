use std::fs;
use std::process::Command;

#[test]
fn stdlib_is_found_next_to_exe_without_overrides() {
    // Arrange a pseudo-release layout:
    // <tmp>/bin/kscr (copied from built test binary)
    // <tmp>/bin/stdlib/Prelude.ks
    // <tmp>/main.ks (imports Prelude)
    let tmp = tempfile::tempdir().expect("tempdir");
    let bin_dir = tmp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("mkdir bin");

    // Use the currently running test binary as a stand-in for a release `kscr` executable.
    // This keeps the test robust even when Cargo doesn't export `CARGO_BIN_EXE_kscr`.
    let kscr_bin = std::env::current_exe().expect("current_exe");
    let dst = bin_dir.join("kscr");
    fs::copy(&kscr_bin, &dst).expect("copy kscr");

    let stdlib_dir = bin_dir.join("stdlib");
    fs::create_dir_all(&stdlib_dir).expect("mkdir stdlib");
    fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("stdlib/Prelude.ks"),
        stdlib_dir.join("Prelude.ks"),
    )
    .expect("copy Prelude.ks");

    let main_path = tmp.path().join("Main.ks");
    fs::write(&main_path, "module Main\n\nimport Prelude\n\nmain = 0\n").expect("write Main.ks");

    // Ensure no environment override is involved.
    let out = Command::new(&dst)
        .arg("typecheck")
        .arg(&main_path)
        .env_remove("KSCR_STDLIB_DIR")
        .output()
        .expect("run kscr");

    assert!(
        out.status.success(),
        "expected success, got status={:?}\nstdout:\n{}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
