use std::process::Command;

#[test]
fn eq_instance_can_override_builtin_behavior() {
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args([
            "run",
            "--ksif-rebuild",
            "tests/repro_eq_override_instance.ks",
        ])
        .output()
        .expect("run kscr");

    assert!(
        out.status.success(),
        "stderr was: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout, "EQ_OK\n", "stdout was: {stdout}");
}
