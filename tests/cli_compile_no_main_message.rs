use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn kscr_binary() -> String {
    // Get the path to the debug build
    let mut path = std::env::current_exe().expect("get current exe");
    path.pop(); // Remove test executable name
    path.pop(); // Remove deps directory
    path.push("kscr");
    path.to_str().expect("path to string").to_string()
}

#[test]
fn cli_compile_fails_without_main() {
    let temp = TempDir::new().expect("create temp dir");
    let temp_path = temp.path();

    let lib_content = r#"module Lib where
  export helper

  helper x = x + 1
"#;
    fs::write(temp_path.join("Lib.ks"), lib_content).expect("write Lib.ks");

    let kscr = kscr_binary();
    let output = Command::new(&kscr)
        .arg("compile")
        .arg(temp_path.join("Lib.ks"))
        .arg("-o")
        .arg(temp_path.join("lib.out"))
        .output()
        .expect("run kscr compile");

    assert!(
        !output.status.success(),
        "compile should fail without main"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("compile requires a `main` binding"),
        "stderr was: {stderr}"
    );
    assert!(
        !temp_path.join("lib.out").exists(),
        "output should not be created when compile fails"
    );
}

#[test]
fn cli_compile_allows_no_main_with_flag() {
    let temp = TempDir::new().expect("create temp dir");
    let temp_path = temp.path();

    let lib_content = r#"module Lib where
  export helper

  helper x = x + 1
"#;
    fs::write(temp_path.join("Lib.ks"), lib_content).expect("write Lib.ks");

    let kscr = kscr_binary();
    let output = Command::new(&kscr)
        .arg("compile")
        .arg(temp_path.join("Lib.ks"))
        .arg("--allow-no-main")
        .arg("-o")
        .arg(temp_path.join("lib.out"))
        .output()
        .expect("run kscr compile");

    assert!(
        output.status.success(),
        "compile should succeed with --allow-no-main: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        temp_path.join("lib.out").exists(),
        "output should be created when compile succeeds"
    );
}
