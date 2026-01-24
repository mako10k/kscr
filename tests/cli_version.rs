use std::process::Command;

#[test]
fn cli_version_flag() {
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args(["--version"])
        .output()
        .expect("run kscr");

    assert!(out.status.success(), "kscr --version should succeed");
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("kscr 0.3.2"), "output should contain version: {s}");
    assert!(s.contains("git:"), "output should contain git SHA: {s}");
    assert!(s.contains("features:"), "output should contain features: {s}");
}

#[test]
fn cli_version_command() {
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args(["version"])
        .output()
        .expect("run kscr");

    assert!(out.status.success(), "kscr version should succeed");
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("kscr 0.3.2"), "output should contain version: {s}");
    assert!(s.contains("git:"), "output should contain git SHA: {s}");
    assert!(s.contains("features:"), "output should contain features: {s}");
}

#[test]
fn cli_version_short_flag() {
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args(["-v"])
        .output()
        .expect("run kscr");

    assert!(out.status.success(), "kscr -v should succeed");
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("kscr 0.3.2"), "output should contain version: {s}");
    assert!(s.contains("git:"), "output should contain git SHA: {s}");
    assert!(s.contains("features:"), "output should contain features: {s}");
}
