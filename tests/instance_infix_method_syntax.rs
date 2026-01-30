// Regression test: instance bodies accept infix method definitions
// Ensures syntax like `a =~= b = ...` works when method name is `(=~=)` in class.
// Also tests constructor patterns on both sides of infix operators.

use std::process::Command;

#[test]
fn instance_infix_method_definition() {
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args(["run", "tests/instance_infix_method_syntax.ks"])
        .output()
        .expect("run kscr");

    assert!(
        out.status.success(),
        "instance with infix method syntax should compile: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.trim().lines().collect();

    assert_eq!(lines.len(), 3, "should have 3 output lines");
    assert_eq!(lines[0], "True", "3 =~= 3 should be True");
    assert_eq!(lines[1], "False", "3 =~= 5 should be False");
    assert_eq!(lines[2], "Constructor pattern infix test passed");
}
