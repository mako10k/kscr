use std::fs;
use std::process::Command;
use tempfile::TempDir;

#[test]
fn imported_class_default_method_supports_empty_instance() {
    let temp = TempDir::new().expect("create temp dir");
    let root = temp.path();

    fs::write(
        root.join("A.ks"),
        "module A where\n  export C(..)\n  class C a where\n    f :: a -> Integer\n    f _ = 42\n",
    )
    .expect("write A.ks");

    fs::write(
        root.join("Main.ks"),
        "module Main where\n  import A\n  instance C Bool where\n  main = stdoutWrite (show (f True))\n",
    )
    .expect("write Main.ks");

    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .arg("run")
        .arg(root.join("Main.ks"))
        .output()
        .expect("run kscr");

    assert!(
        out.status.success(),
        "imported class default should work: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "42");
}

#[test]
fn minimal_definition_rejects_empty_instance() {
    let temp = TempDir::new().expect("create temp dir");
    let root = temp.path();

    fs::write(
        root.join("Main.ks"),
        "module Main where\n  class C a where\n    f :: a -> Integer\n    g :: a -> Integer\n    minimal f | g\n    f x = g x\n    g x = f x\n  instance C Bool where\n  main = IO ()\n",
    )
    .expect("write Main.ks");

    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .arg("typecheck")
        .arg("--all")
        .arg(root.join("Main.ks"))
        .output()
        .expect("typecheck kscr");

    assert!(!out.status.success(), "empty instance should fail minimal definition");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("does not satisfy minimal definition"),
        "stderr should mention minimal definition: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn minimal_definition_accepts_one_alternative() {
    let temp = TempDir::new().expect("create temp dir");
    let root = temp.path();

    fs::write(
        root.join("Main.ks"),
        "module Main where\n  class C a where\n    f :: a -> Integer\n    g :: a -> Integer\n    minimal f | g\n    f x = g x\n    g x = f x\n  instance C Bool where\n    f _ = 7\n  main = stdoutWrite (show (f True))\n",
    )
    .expect("write Main.ks");

    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .arg("run")
        .arg(root.join("Main.ks"))
        .output()
        .expect("run kscr");

    assert!(
        out.status.success(),
        "instance implementing one minimal alternative should run: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "7");
}