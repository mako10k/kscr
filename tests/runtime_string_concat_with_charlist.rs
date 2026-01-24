use std::process::Command;

#[test]
fn string_literal_concat_with_charlist() {
    // Test that string literals can be concatenated with [Char] lists
    // This validates the fix for: "++ fails when mixing string literal and [Char]"
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args(["run", "tests/runtime_string_concat_with_charlist.ks"])
        .output()
        .expect("run kscr");

    assert!(out.status.success(), "exit code should be 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    
    // Each test case should output one line
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines.len(), 5, "should have 5 output lines");
    
    // Test: "" ++ ['a'] = "a"
    assert_eq!(lines[0], "a");
    
    // Test: ['b'] ++ "" = "b"
    assert_eq!(lines[1], "b");
    
    // Test: "x" ++ ['y'] = "xy"
    assert_eq!(lines[2], "xy");
    
    // Test: ['c'] ++ "d" = "cd"
    assert_eq!(lines[3], "cd");
    
    // Test: [] ++ ['e'] = "e"
    assert_eq!(lines[4], "e");
}
