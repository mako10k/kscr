use std::process::Command;

#[test]
fn deriving_eq_show_works_for_user_defined_type() {
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["run", "--ksif-rebuild", "tests/repro_deriving_eq_show.ks"])
        .output()
        .expect("run kscr");

    assert!(
        out.status.success(),
        "stderr was: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Person"), "stdout was: {stdout}");
    assert!(stdout.contains("EQ_OK"), "stdout was: {stdout}");
}
