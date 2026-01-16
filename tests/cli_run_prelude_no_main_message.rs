use std::process::Command;

#[test]
fn cli_run_stdlib_prelude_no_main_message() {
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args(["run", "stdlib/Prelude.ks"])
        .output()
        .expect("run kscr");

    // This should fail (no `main` in Prelude), and the message should be explicit.
    assert!(!out.status.success());
    let s = String::from_utf8_lossy(&out.stderr);
    assert!(s.contains("main does not exist"), "stderr was: {s}");
}
