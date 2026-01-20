use std::process::Command;

#[test]
fn unify_fail_msg2_includes_def_location_note() {
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args(["typecheck", "tests/unify_fail_msg2.ks"])
        .output()
        .expect("run kscr typecheck");

    assert!(!out.status.success(), "expected typecheck to fail");
    let s = String::from_utf8_lossy(&out.stderr);

    // Always keep the existing strong evidence.
    assert!(s.contains("unify goal:"), "stderr was: {s}");

    // New evidence: real file:line:col location note.
    assert!(s.contains("note:"), "stderr was: {s}");
    assert!(s.contains(":") && s.split(':').count() >= 4, "stderr was: {s}");
}
