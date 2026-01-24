use std::process::Command;

#[test]
fn test_getargs_api() {
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args([
            "run",
            "tests/test_getargs.ks",
            "arg1",
            "arg2",
            "arg3",
        ])
        .output()
        .expect("run kscr");

    assert!(out.status.success(), "exit code should be 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let output = stdout.trim();

    // getArgs should return all arguments including the binary path
    assert!(output.contains("arg1"));
    assert!(output.contains("arg2"));
    assert!(output.contains("arg3"));
}

#[test]
fn test_readfile_writefile_api() {
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args(["run", "tests/test_read_write_file.ks"])
        .output()
        .expect("run kscr");

    assert!(out.status.success(), "exit code should be 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.trim(), "Hello, World!");
}

#[test]
fn test_exitwith_api() {
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args(["run", "tests/test_exitwith.ks"])
        .output()
        .expect("run kscr");

    // exitWith 42 should set the exit code to 42
    assert_eq!(out.status.code(), Some(42));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("About to exit with code 42"));
    assert!(!stdout.contains("This should not be printed"));
}
