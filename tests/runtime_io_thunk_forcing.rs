use std::process::Command;

#[test]
fn runtime_forces_thunks_in_main() {
    // Test that main's result is forced before checking it's an IO action.
    // This validates the fix for: "Runtime should force thunks when an IO action is expected"
    // Repro 1: main calls a function that returns IO action via do-block with multiple statements
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args(["run", "tests/repro_io_thunk_main.ks"])
        .output()
        .expect("run kscr");

    assert!(out.status.success(), "exit code should be 0, stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.trim().lines().collect();
    
    assert_eq!(lines.len(), 3, "should have 3 output lines");
    assert_eq!(lines[0], "a");
    assert_eq!(lines[1], "b");
    assert_eq!(lines[2], "done");
}

#[test]
fn runtime_forces_thunks_in_io_bind() {
    // Test that __ioBind's continuation result is forced before checking it's an IO action.
    // Repro 2: bind result calls a function that returns IO action via do-block
    let mut child = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args(["run", "tests/repro_io_thunk_bind.ks"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn kscr");
    
    use std::io::Write;
    child.stdin.as_mut().unwrap().write_all(b"test\n").expect("write to stdin");
    let out = child.wait_with_output().expect("wait for kscr");

    assert!(out.status.success(), "exit code should be 0, stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.trim().lines().collect();
    
    assert_eq!(lines.len(), 2, "should have 2 output lines");
    assert_eq!(lines[0], "test");
    assert_eq!(lines[1], "done");
}
