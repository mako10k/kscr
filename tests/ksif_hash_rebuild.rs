//! Test .ksif hash validation and rebuild policy

use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

#[test]
fn test_ksif_rebuild_on_hash_mismatch() {
    // Create temporary directory
    let temp = TempDir::new().expect("create temp dir");
    let temp_path = temp.path();

    // Write module A (no dependencies)
    let a_content = r#"module A where
  export id
  
  id x = x
"#;
    fs::write(temp_path.join("A.ks"), a_content).expect("write A.ks");

    // Write module B (imports A)
    let b_content = r#"module B where
  import A
  
  export foo
  
  foo = A.id 42
"#;
    fs::write(temp_path.join("B.ks"), b_content).expect("write B.ks");

    // Write module Main (imports B, which imports A)
    let main_content = r#"module Main where
  import B
  
  export test
  
  test = B.foo
"#;
    fs::write(temp_path.join("Main.ks"), main_content).expect("write Main.ks");

    // Typecheck Main to generate .ksif files for A and B
    let result = kscr::types::typecheck_file(&temp_path.join("Main.ks"));
    assert!(result.is_ok(), "initial typecheck should succeed");

    // Check that A.ksif and B.ksif exist
    let a_ksif = temp_path.join("A.ksif");
    let b_ksif = temp_path.join("B.ksif");
    assert!(a_ksif.exists(), "A.ksif should exist");
    assert!(b_ksif.exists(), "B.ksif should exist");

    // Record B.ksif mtime
    let b_ksif_mtime_1 = fs::metadata(&b_ksif)
        .expect("read B.ksif metadata")
        .modified()
        .expect("get mtime");

    // Sleep to ensure mtime difference
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Typecheck Main again - should not rebuild (hashes match)
    let result = kscr::types::typecheck_file(&temp_path.join("Main.ks"));
    assert!(result.is_ok(), "second typecheck should succeed");

    let b_ksif_mtime_2 = fs::metadata(&b_ksif)
        .expect("read B.ksif metadata")
        .modified()
        .expect("get mtime");

    // B.ksif should NOT have been rebuilt (hash still valid)
    assert_eq!(
        b_ksif_mtime_1, b_ksif_mtime_2,
        "B.ksif should not be rebuilt when hashes match"
    );

    // Now modify A.ks and rebuild A.ksif
    let a_content_modified = r#"module A where
  export id, newFunc
  
  id x = x
  newFunc y = y
"#;
    fs::write(temp_path.join("A.ks"), a_content_modified).expect("write modified A.ks");

    // Force rebuild A.ksif without rebuilding its dependents
    kscr::types::set_ksif_rebuild_policy(kscr::types::KsifRebuildPolicy {
        force_rebuild: true,
        suppress_recursive_rebuild: true,
    });
    // Typecheck A directly to rebuild only A.ksif
    let result = kscr::types::typecheck_file(&temp_path.join("A.ks"));
    // This might fail because A.ks doesn't import itself, but what we care about is
    // that when we later check something that imports A, we see the A.ksif has been regenerated
    let _ = result; // Ignore result - we just want A.ksif to be regenerated if possible

    // Actually, since typecheck_file doesn't generate .ksif for the entry module,
    // we need a different approach. Let's create a dummy module that imports A to force A.ksif rebuild.
    let dummy_content = r#"module Dummy where
  import A
  
  export dummy
  
  dummy = A.id 1
"#;
    fs::write(temp_path.join("Dummy.ks"), dummy_content).expect("write Dummy.ks");
    kscr::types::set_ksif_rebuild_policy(kscr::types::KsifRebuildPolicy {
        force_rebuild: true,
        suppress_recursive_rebuild: true,
    });
    let result = kscr::types::typecheck_file(&temp_path.join("Dummy.ks"));
    assert!(result.is_ok(), "rebuild A should succeed");

    // Sleep again
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Now typecheck Main again - should rebuild B because A.ksif hash changed
    kscr::types::set_ksif_rebuild_policy(kscr::types::KsifRebuildPolicy::default());
    let result = kscr::types::typecheck_file(&temp_path.join("Main.ks"));
    assert!(result.is_ok(), "typecheck after hash change should succeed");

    let b_ksif_mtime_3 = fs::metadata(&b_ksif)
        .expect("read B.ksif metadata")
        .modified()
        .expect("get mtime");

    // B.ksif should have been rebuilt (hash mismatch detected)
    assert_ne!(
        b_ksif_mtime_2, b_ksif_mtime_3,
        "B.ksif should be rebuilt when dependency hash changes"
    );
}

#[test]
fn test_ksif_force_rebuild_flag() {
    let temp = TempDir::new().expect("create temp dir");
    let temp_path = temp.path();

    // Create module C
    let content_c = r#"module C where
  export val
  
  val = 123
"#;
    fs::write(temp_path.join("C.ks"), content_c).expect("write C.ks");

    // Create module Main that imports C (so C.ksif gets generated)
    let content_main = r#"module Main where
  import C
  
  export test
  
  test = C.val
"#;
    fs::write(temp_path.join("Main.ks"), content_main).expect("write Main.ks");

    // Initial build
    let result = kscr::types::typecheck_file(&temp_path.join("Main.ks"));
    assert!(result.is_ok(), "initial build failed: {:?}", result.err());

    // The .ksif file for C should be created next to the source file
    let c_ksif = temp_path.join("C.ksif");
    assert!(c_ksif.exists(), "C.ksif should exist at {}", c_ksif.display());

    let mtime_1 = fs::metadata(&c_ksif).unwrap().modified().unwrap();

    std::thread::sleep(std::time::Duration::from_millis(100));

    // Normal rebuild - should not rebuild
    kscr::types::set_ksif_rebuild_policy(kscr::types::KsifRebuildPolicy::default());
    let result = kscr::types::typecheck_file(&temp_path.join("Main.ks"));
    assert!(result.is_ok());

    let mtime_2 = fs::metadata(&c_ksif).unwrap().modified().unwrap();
    assert_eq!(mtime_1, mtime_2, "should not rebuild without changes");

    std::thread::sleep(std::time::Duration::from_millis(100));

    // Force rebuild
    kscr::types::set_ksif_rebuild_policy(kscr::types::KsifRebuildPolicy {
        force_rebuild: true,
        suppress_recursive_rebuild: false,
    });
    let result = kscr::types::typecheck_file(&temp_path.join("Main.ks"));
    assert!(result.is_ok());

    let mtime_3 = fs::metadata(&c_ksif).unwrap().modified().unwrap();
    assert_ne!(mtime_2, mtime_3, "force rebuild should rebuild");

    // Reset policy
    kscr::types::set_ksif_rebuild_policy(kscr::types::KsifRebuildPolicy::default());
}
