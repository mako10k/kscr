// Integration tests for module export restrictions
// Validates that export lists correctly restrict visibility of functions, types, and constructors

use std::process::Command;

/// Test 1: Basic export restriction - only exported function is accessible
#[test]
fn basic_export_restriction() {
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args(["run", "tests/export_restriction_basic.ks"])
        .output()
        .expect("run kscr");

    assert!(out.status.success(), "exit code should be 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.trim(), "6", "publicFunc should return 6");
}

/// Test 2: Type export restriction - only exported constructor is accessible
#[test]
fn type_export_restriction() {
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args(["run", "tests/export_restriction_type.ks"])
        .output()
        .expect("run kscr");

    assert!(out.status.success(), "exit code should be 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.trim(),
        "OK",
        "exported constructor A should be accessible"
    );
}

/// Test 3: No explicit export - everything is accessible by default
#[test]
fn no_explicit_export() {
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args(["run", "tests/export_restriction_empty.ks"])
        .output()
        .expect("run kscr");

    assert!(out.status.success(), "exit code should be 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.trim(),
        "42",
        "secret should be accessible by default"
    );
}

/// Test 4: Multiple modules with different export restrictions
#[test]
fn multiple_modules_export_restriction() {
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args(["run", "tests/export_restriction_multiple.ks"])
        .output()
        .expect("run kscr");

    assert!(out.status.success(), "exit code should be 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.trim().lines().collect();

    assert_eq!(lines.len(), 3, "should have 3 output lines");
    assert_eq!(lines[0], "15", "MA.funcA should return 15");
    assert_eq!(lines[1], "10", "MB.funcB should return 10");
    assert_eq!(lines[2], "All exports work correctly");
}

/// Negative test 1: Non-exported function should cause compile error
#[test]
fn basic_export_restriction_negative() {
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args(["run", "tests/export_restriction_basic_negative.ks"])
        .output()
        .expect("run kscr");

    assert!(!out.status.success(), "should fail to compile");
    let stderr = String::from_utf8_lossy(&out.stderr);
    // Check that error message mentions the non-exported function
    assert!(
        stderr.contains("privateFunc") || stderr.contains("unbound"),
        "error should mention privateFunc or unbound: {}",
        stderr
    );
}

/// Negative test 2: Non-exported constructor should cause compile error
#[test]
fn type_export_restriction_negative() {
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args(["run", "tests/export_restriction_type_negative.ks"])
        .output()
        .expect("run kscr");

    assert!(!out.status.success(), "should fail to compile");
    let stderr = String::from_utf8_lossy(&out.stderr);
    // Check that error message mentions the non-exported constructor
    assert!(
        stderr.contains("T.B") || stderr.contains("unknown constructor"),
        "error should mention constructor B or unknown constructor: {}",
        stderr
    );
}
