use std::process::Command;

#[test]
fn stdlib_read_int_smoke() {
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["run", "--ksif-rebuild", "tests/stdlib_read_int_smoke.ks"])
        .output()
        .expect("run kscr");

    assert!(
        out.status.success(),
        "exit code should be 0 (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines, ["0", "-42", "Nothing"]);
}
