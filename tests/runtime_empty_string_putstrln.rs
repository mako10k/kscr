use std::process::Command;

#[test]
fn empty_string_putstrln() {
    // Regression test for v0.3.3+: putStrLn "" and putStrLn [] should work
    // Previously failed with "error: expected String/[Char]"
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args(["run", "tests/runtime_empty_string_putstrln.ks"])
        .output()
        .expect("run kscr");

    assert!(out.status.success(), "exit code should be 0");
    let stdout = String::from_utf8_lossy(&out.stdout);

    // Should output 4 lines (2 empty, "hi", "a")
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 4, "should have 4 output lines");

    // Test: putStrLn "" outputs empty line
    assert_eq!(lines[0], "");

    // Test: putStrLn [] outputs empty line
    assert_eq!(lines[1], "");

    // Test: putStrLn "hi" outputs "hi"
    assert_eq!(lines[2], "hi");

    // Test: putStrLn ['a'] outputs "a"
    assert_eq!(lines[3], "a");
}
