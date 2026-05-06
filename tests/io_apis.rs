use std::process::Command;

#[test]
fn test_getargs_api() {
    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args(["run", "tests/test_getargs.ks", "arg1", "arg2", "arg3"])
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
fn test_readfile_two_qualified_imports_regression() {
    let dir = std::env::temp_dir().join(format!(
        "kscr_test_readfile_two_qualified_imports_regression_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");

    let program = dir.join("Program.ks");
    let program_src = "module Main where\n  import Prelude\n  import qualified Foo as Foo\n  import qualified Bar as Bar\n\n  main = do\n    putStrLn Foo.hello\n    putStrLn Bar.msg\n";
    std::fs::write(&program, program_src).expect("write Program.ks");

    let main = dir.join("Main.ks");
    std::fs::write(
        &main,
        "module Main where\n  import Prelude\n\n  countChars = \\xs -> case xs of\n    [] -> 0\n    _ : rest -> 1 + countChars rest\n\n  main = do\n    content <- readFile \"Program.ks\"\n    print (show (countChars content))\n",
    )
    .expect("write Main.ks");

    let out = Command::new(env!("CARGO_BIN_EXE_kscr"))
        .args(["run", "Main.ks"])
        .current_dir(&dir)
        .output()
        .expect("run kscr");

    assert!(
        out.status.success(),
        "expected success, got status {:?}, stdout={}, stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let expected = format!("\"{}\"", program_src.chars().count());
    assert_eq!(stdout.trim(), expected);

    let _ = std::fs::remove_dir_all(dir);
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
