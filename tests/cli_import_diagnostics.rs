use std::fs;
use std::process::Command;

use tempfile::TempDir;

#[test]
fn cli_typecheck_missing_import_shows_searched_paths() {
    let temp = TempDir::new().expect("create temp dir");
    let main = temp.path().join("Main.ks");
    fs::write(
        &main,
        "module Main where\n  import Missing.Module\n  main = IO ()\n",
    )
    .expect("write Main.ks");

    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .arg("typecheck")
        .arg(&main)
        .output()
        .expect("run kscr typecheck");

    assert!(!out.status.success(), "typecheck should fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cannot find module file for import Missing.Module"),
        "stderr was: {stderr}"
    );
    assert!(stderr.contains("tried:"), "stderr was: {stderr}");
    assert!(stderr.contains("Missing/Module.ks"), "stderr was: {stderr}");
    assert!(stderr.contains("stdlib"), "stderr was: {stderr}");
}

#[test]
fn cli_typecheck_import_cycle_shows_cycle_chain() {
    let temp = TempDir::new().expect("create temp dir");
    let a = temp.path().join("A.ks");
    let b = temp.path().join("B.ks");

    fs::write(&a, "module A where\n  import B\n  x = 1\n").expect("write A.ks");
    fs::write(&b, "module B where\n  import A\n  y = 2\n").expect("write B.ks");

    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .arg("typecheck")
        .arg(&a)
        .output()
        .expect("run kscr typecheck");

    assert!(!out.status.success(), "typecheck should fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cyclic imports"), "stderr was: {stderr}");
    assert!(stderr.contains("B -> A -> B"), "stderr was: {stderr}");
}
