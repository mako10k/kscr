use std::fs;
use std::process::Command;
use tempfile::TempDir;

#[test]
fn cli_run_typeclass_transitive_deep_nested_reexported() {
    let temp = TempDir::new().expect("create temp dir");
    let root = temp.path();

    fs::write(
        root.join("A.ks"),
        "module A where\n  export Inc(..)\n  class Inc a where\n    inc :: a -> a\n  instance Inc Integer where\n    inc x = x + 1\n",
    )
    .expect("write A.ks");

    fs::write(
        root.join("B.ks"),
        "module B where\n  export applyInc\n  import A\n  applyInc = inc\n",
    )
    .expect("write B.ks");

    fs::write(
        root.join("C.ks"),
        "module C where\n  export useInc\n  import qualified B as BX\n  useInc = BX.applyInc\n",
    )
    .expect("write C.ks");

    fs::write(
        root.join("D.ks"),
        "module D where\n  export callInc\n  import C\n  callInc = useInc\n",
    )
    .expect("write D.ks");

    let main = root.join("Main.ks");
    fs::write(
        &main,
        "module Main where\n  import qualified D as DX\n  main = do\n    stdoutWrite (show (DX.callInc 1))\n    IO ()\n",
    )
    .expect("write Main.ks");

    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .arg("run")
        .arg(&main)
        .output()
        .expect("run kscr");

    assert!(
        out.status.success(),
        "kscr run should succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains('2'), "stdout should contain 2: {stdout}");
}
