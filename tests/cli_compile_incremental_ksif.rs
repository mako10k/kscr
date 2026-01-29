//! Test CLI compile command incremental KSIF emission

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
fn test_cli_compile_skips_ksif_emission_when_hashes_match() {
    let temp = TempDir::new().expect("create temp dir");
    let temp_path = temp.path();

    // Create a dependency module
    let lib_content = r#"module Lib where
  export helper
  
  helper x = x + 1
"#;
    fs::write(temp_path.join("Lib.ks"), lib_content).expect("write Lib.ks");

    // Create a main module that imports Lib
    let main_content = r#"module Main where
  import Prelude
  import Lib
  
  main = do
    putStrLn "Hello"
"#;
    fs::write(temp_path.join("Main.ks"), main_content).expect("write Main.ks");

    // First, generate Lib.ksif by typechecking it through a dummy module
    let dummy_content = r#"module Dummy where
  import Lib
  
  export test
  
  test = Lib.helper 1
"#;
    fs::write(temp_path.join("Dummy.ks"), dummy_content).expect("write Dummy.ks");
    let result = kscr::types::typecheck_file(&temp_path.join("Dummy.ks"));
    assert!(
        result.is_ok(),
        "generate Lib.ksif failed: {:?}",
        result.err()
    );

    let lib_ksif = temp_path.join("Lib.ksif");
    assert!(lib_ksif.exists(), "Lib.ksif should exist");

    // Now compile Main.ks to generate Main executable and Main.ksif
    let kscr = kscr_binary();
    let output = Command::new(&kscr)
        .arg("compile")
        .arg(temp_path.join("Main.ks"))
        .arg("--ksif-out")
        .arg(temp_path)
        .output()
        .expect("run kscr compile");

    assert!(
        output.status.success(),
        "first compile should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let main_ksif = temp_path.join("Main.ksif");
    assert!(main_ksif.exists(), "Main.ksif should exist after compile");

    let mtime_1 = fs::metadata(&main_ksif)
        .expect("read Main.ksif metadata")
        .modified()
        .expect("get mtime");

    // Sleep to ensure mtime difference if file is written
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Compile again - should skip KSIF emission since hashes match
    let output = Command::new(&kscr)
        .arg("compile")
        .arg(temp_path.join("Main.ks"))
        .arg("--ksif-out")
        .arg(temp_path)
        .output()
        .expect("run kscr compile");

    assert!(
        output.status.success(),
        "second compile should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mtime_2 = fs::metadata(&main_ksif)
        .expect("read Main.ksif metadata")
        .modified()
        .expect("get mtime");

    // Main.ksif should NOT have been rewritten (incremental rebuild skipped)
    assert_eq!(
        mtime_1, mtime_2,
        "Main.ksif should not be rewritten when dependency hashes match"
    );
}

#[test]
fn test_cli_compile_rebuilds_ksif_when_dependency_changes() {
    let temp = TempDir::new().expect("create temp dir");
    let temp_path = temp.path();

    // Create a dependency module
    let lib_content = r#"module Lib where
  export helper
  
  helper x = x + 1
"#;
    fs::write(temp_path.join("Lib.ks"), lib_content).expect("write Lib.ks");

    // Create a main module that imports Lib
    let main_content = r#"module Main where
  import Prelude
  import Lib
  
  main = do
    putStrLn "Hello"
"#;
    fs::write(temp_path.join("Main.ks"), main_content).expect("write Main.ks");

    // First, generate Lib.ksif
    let dummy_content = r#"module Dummy where
  import Lib
  
  export test
  
  test = Lib.helper 1
"#;
    fs::write(temp_path.join("Dummy.ks"), dummy_content).expect("write Dummy.ks");
    let result = kscr::types::typecheck_file(&temp_path.join("Dummy.ks"));
    assert!(
        result.is_ok(),
        "generate Lib.ksif failed: {:?}",
        result.err()
    );

    // Compile Main.ks
    let kscr = kscr_binary();
    let output = Command::new(&kscr)
        .arg("compile")
        .arg(temp_path.join("Main.ks"))
        .arg("--ksif-out")
        .arg(temp_path)
        .output()
        .expect("run kscr compile");

    assert!(
        output.status.success(),
        "first compile should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let main_ksif = temp_path.join("Main.ksif");
    let mtime_1 = fs::metadata(&main_ksif)
        .expect("read Main.ksif metadata")
        .modified()
        .expect("get mtime");

    std::thread::sleep(std::time::Duration::from_millis(100));

    // Modify Lib.ks and rebuild Lib.ksif
    let lib_content_modified = r#"module Lib where
  export helper, newHelper
  
  helper x = x + 1
  newHelper x = x + 2
"#;
    fs::write(temp_path.join("Lib.ks"), lib_content_modified).expect("write modified Lib.ks");

    // Delete Lib.ksif to force rebuild
    let lib_ksif = temp_path.join("Lib.ksif");
    fs::remove_file(&lib_ksif).expect("remove Lib.ksif");

    // Rebuild Lib.ksif by re-typechecking Dummy
    let result = kscr::types::typecheck_file(&temp_path.join("Dummy.ks"));
    assert!(
        result.is_ok(),
        "rebuild Lib.ksif failed: {:?}",
        result.err()
    );

    // Verify that Lib.ksif exists again
    assert!(lib_ksif.exists(), "Lib.ksif should be rebuilt");

    std::thread::sleep(std::time::Duration::from_millis(100));

    // Compile Main.ks again - should rebuild Main.ksif due to Lib.ksif hash change
    let output = Command::new(&kscr)
        .arg("compile")
        .arg(temp_path.join("Main.ks"))
        .arg("--ksif-out")
        .arg(temp_path)
        .output()
        .expect("run kscr compile");

    assert!(
        output.status.success(),
        "second compile should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mtime_2 = fs::metadata(&main_ksif)
        .expect("read Main.ksif metadata")
        .modified()
        .expect("get mtime");

    // Main.ksif SHOULD have been rewritten (hash mismatch detected)
    assert_ne!(
        mtime_1, mtime_2,
        "Main.ksif should be rewritten when dependency hash changes"
    );
}

#[test]
fn test_cli_compile_respects_default_policy() {
    let temp = TempDir::new().expect("create temp dir");
    let temp_path = temp.path();

    // Create a simple main module with no dependencies
    let main_content = r#"module Main where
  import Prelude
  
  main = do
    putStrLn "Hello"
"#;
    fs::write(temp_path.join("Main.ks"), main_content).expect("write Main.ks");

    // Compile to generate Main.ksif
    let kscr = kscr_binary();
    let output = Command::new(&kscr)
        .arg("compile")
        .arg(temp_path.join("Main.ks"))
        .arg("--ksif-out")
        .arg(temp_path)
        .output()
        .expect("run kscr compile");

    assert!(
        output.status.success(),
        "first compile should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let main_ksif = temp_path.join("Main.ksif");
    let mtime_1 = fs::metadata(&main_ksif)
        .expect("read Main.ksif metadata")
        .modified()
        .expect("get mtime");

    std::thread::sleep(std::time::Duration::from_millis(100));

    // Compile without force - should skip (default policy)
    let output = Command::new(&kscr)
        .arg("compile")
        .arg(temp_path.join("Main.ks"))
        .arg("--ksif-out")
        .arg(temp_path)
        .output()
        .expect("run kscr compile");

    assert!(
        output.status.success(),
        "second compile should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mtime_2 = fs::metadata(&main_ksif)
        .expect("read Main.ksif metadata")
        .modified()
        .expect("get mtime");

    // Should skip since no changes
    assert_eq!(
        mtime_1, mtime_2,
        "Main.ksif should not be rewritten when no changes and no force_rebuild"
    );
}
