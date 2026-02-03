// Test: infix operators with constructor patterns on both sides
// Ensures patterns like `P x y ++ P a b = ...` work correctly.

use std::process::Command;

#[test]
fn infix_constructor_patterns() {
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args(["run", "tests/infix_constructor_patterns.ks"])
        .output()
        .expect("run kscr");

    assert!(
        out.status.success(),
        "infix operators with constructor patterns should compile and run: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.trim(), "All constructor pattern infix tests passed");
}
