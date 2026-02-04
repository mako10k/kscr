use std::process::Command;

#[test]
fn show_override_print_custom() {
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args(["run", "tests/runtime_show_override_custom.ks"])
        .output()
        .expect("run kscr");

    assert!(
        out.status.success(),
        "exit code should be 0 (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines, ["CUSTOM"]);
}
