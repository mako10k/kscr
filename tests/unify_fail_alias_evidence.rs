use std::process::Command;

#[test]
fn unify_fail_includes_type_alias_def_location_note() {
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args(["typecheck", "tests/unify_fail_alias_evidence.ks"])
        .output()
        .expect("run kscr typecheck");

    assert!(!out.status.success(), "expected typecheck to fail");
    let s = String::from_utf8_lossy(&out.stderr);

    // Keep existing evidence signal.
    assert!(s.contains("unify goal:"), "stderr was: {s}");

    // Prefer A: show the local alias `Text` definition location.
    assert!(
        s.contains("note: type alias `Text`"),
        "expected local type alias note in stderr, got: {s}"
    );
    assert!(
        s.contains("tests/unify_fail_alias_evidence.ks:"),
        "expected location to reference test file, got: {s}"
    );

    // C: also show where `Text` points to.
    assert!(
        s.contains("note: type alias `Text` expands to"),
        "expected alias expansion note in stderr, got: {s}"
    );
    assert!(
        s.contains("`Prelude.String`"),
        "expected canonical expansion target in stderr, got: {s}"
    );

    // Stronger C (A + B): include the canonical alias's def-site location.
    // Keep this robust: we only check for a stdlib path mention.
    assert!(
        s.contains("note: type alias `String` resolves to"),
        "expected canonical alias def-site note in stderr, got: {s}"
    );
    assert!(
        s.contains("stdlib/"),
        "expected stdlib path in canonical alias def-site note, got: {s}"
    );
}
