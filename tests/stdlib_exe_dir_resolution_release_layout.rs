use std::fs;
use std::process::Command;

fn kscr_binary() -> std::path::PathBuf {
    let mut path = std::env::current_exe().expect("get current exe");
    path.pop(); // test exe
    path.pop(); // deps
    path.push("kscr");
    path
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) {
    fs::create_dir_all(dst).expect("mkdir recursive dst");
    for entry in fs::read_dir(src).expect("read_dir src") {
        let entry = entry.expect("dir entry");
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path);
        } else {
            fs::copy(&src_path, &dst_path).expect("copy file");
        }
    }
}

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
    let kscr_bin = kscr_binary();
    let dst = bin_dir.join("kscr");
    fs::copy(&kscr_bin, &dst).expect("copy kscr");

    let stdlib_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("stdlib");
    let stdlib_dir = bin_dir.join("stdlib");
    copy_dir_recursive(&stdlib_src, &stdlib_dir);

    let main_path = tmp.path().join("Main.ks");
    fs::write(
        &main_path,
        "module Main where\n  import Prelude\n  main = IO ()\n",
    )
    .expect("write Main.ks");

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

#[test]
fn run_uses_writable_runtime_ksif_cache_path() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let bin_dir = tmp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("mkdir bin");

    let kscr_bin = kscr_binary();
    let dst = bin_dir.join("kscr");
    fs::copy(&kscr_bin, &dst).expect("copy kscr");

    let stdlib_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("stdlib");
    let stdlib_dir = bin_dir.join("stdlib");
    copy_dir_recursive(&stdlib_src, &stdlib_dir);

    let main_path = tmp.path().join("Main.ks");
    fs::write(
        &main_path,
        "module Main where\n  import Prelude\n  main = IO ()\n",
    )
    .expect("write Main.ks");

    let out = Command::new(&dst)
        .arg("run")
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

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("/home/runner/work/"),
        "stderr should not reference build-time CI paths:\n{stderr}"
    );
    assert!(
        !stderr.contains("Permission denied"),
        "stderr should not contain permission denied:\n{stderr}"
    );
}
