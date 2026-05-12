use std::process::Command;

#[test]
fn newtype_smoke_typechecks_via_data_lowering() {
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args(["typecheck", "tests/newtype_smoke.ks"])
        .output()
        .expect("run kscr typecheck");

    assert!(
        out.status.success(),
        "stderr was: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
