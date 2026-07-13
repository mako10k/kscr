use std::process::Command;

#[test]
fn unify_fail_nonstring_alias_uses_rhs_type() {
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args(["typecheck", "tests/unify_fail_alias_nonstring.ks"])
        .output()
        .expect("run kscr typecheck");

    assert!(!out.status.success(), "expected typecheck to fail");
    let s = String::from_utf8_lossy(&out.stderr);

    assert!(s.contains("other = Integer"), "stderr was: {s}");
    assert!(s.contains("type alias `Age`"), "stderr was: {s}");
}
