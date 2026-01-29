//! Test .ksif hash validation and rebuild policy

use std::fs;
use std::sync::Mutex;
use tempfile::TempDir;

// Ksif rebuild policy is global mutable state; these tests must not run in parallel.
static KSIF_POLICY_MUTEX: Mutex<()> = Mutex::new(());

struct PolicyResetGuard;

impl Drop for PolicyResetGuard {
    fn drop(&mut self) {
        kscr::types::set_ksif_rebuild_policy(kscr::types::KsifRebuildPolicy::default());
    }
}

#[test]
fn test_ksif_rebuild_on_hash_mismatch() {
    let _lock = KSIF_POLICY_MUTEX.lock().unwrap();
    let _policy_guard = PolicyResetGuard;

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
    let _lock = KSIF_POLICY_MUTEX.lock().unwrap();
    let _policy_guard = PolicyResetGuard;

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
    assert!(
        c_ksif.exists(),
        "C.ksif should exist at {}",
        c_ksif.display()
    );

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

#[test]
fn test_suppress_recursive_rebuild_errors_on_missing_dependency_ksif() {
    let _lock = KSIF_POLICY_MUTEX.lock().unwrap();
    let _policy_guard = PolicyResetGuard;

    let temp = TempDir::new().expect("create temp dir");
    let temp_path = temp.path();

    // Create module A (no dependencies)
    let a_content = r#"module A where
  export id

  id x = x
"#;
    fs::write(temp_path.join("A.ks"), a_content).expect("write A.ks");

    // Create module B that imports A
    let b_content = r#"module B where
  import A

  export foo

  foo = A.id 1
"#;
    fs::write(temp_path.join("B.ks"), b_content).expect("write B.ks");

    // Create Main that imports B, so building B.ksif is required.
    let main_content = r#"module Main where
  import B

  export test

  test = B.foo
"#;
    fs::write(temp_path.join("Main.ks"), main_content).expect("write Main.ks");

    // With dependency rebuild suppressed, we should NOT silently proceed if A.ksif is missing.
    kscr::types::set_ksif_rebuild_policy(kscr::types::KsifRebuildPolicy {
        force_rebuild: true,
        suppress_recursive_rebuild: true,
    });

    let result = kscr::types::typecheck_file(&temp_path.join("Main.ks"));
    assert!(
        result.is_err(),
        "typecheck should fail due to missing A.ksif"
    );
    let msg = format!("{:?}", result.err().unwrap());
    assert!(msg.contains("A.ksif"), "error should mention A.ksif: {msg}");

    // Create A.ksif and retry (should succeed)
    let dummy_content = r#"module DummyA where
  import A

  export dummy

  dummy = A.id 1
"#;
    fs::write(temp_path.join("DummyA.ks"), dummy_content).expect("write DummyA.ks");

    kscr::types::set_ksif_rebuild_policy(kscr::types::KsifRebuildPolicy::default());
    let result = kscr::types::typecheck_file(&temp_path.join("DummyA.ks"));
    assert!(result.is_ok(), "creating A.ksif should succeed");

    kscr::types::set_ksif_rebuild_policy(kscr::types::KsifRebuildPolicy {
        force_rebuild: true,
        suppress_recursive_rebuild: true,
    });
    let result = kscr::types::typecheck_file(&temp_path.join("Main.ks"));
    assert!(
        result.is_ok(),
        "typecheck should succeed once A.ksif exists"
    );
}

#[test]
fn test_suppress_recursive_rebuild_skips_dependency_validation() {
    let _lock = KSIF_POLICY_MUTEX.lock().unwrap();
    let _policy_guard = PolicyResetGuard;

    let temp = TempDir::new().expect("create temp dir");
    let temp_path = temp.path();

    // Create module D (leaf dependency)
    let d_content = r#"module D where
  export value
  
  value = 100
"#;
    fs::write(temp_path.join("D.ks"), d_content).expect("write D.ks");

    // Create module E (imports D)
    let e_content = r#"module E where
  import D
  
  export result
  
  result = D.value
"#;
    fs::write(temp_path.join("E.ks"), e_content).expect("write E.ks");

    // Create module F (imports E, which imports D)
    let f_content = r#"module F where
  import E
  
  export test
  
  test = E.result
"#;
    fs::write(temp_path.join("F.ks"), f_content).expect("write F.ks");

    // Initial build - generate all ksif files
    kscr::types::set_ksif_rebuild_policy(kscr::types::KsifRebuildPolicy::default());
    let result = kscr::types::typecheck_file(&temp_path.join("F.ks"));
    assert!(
        result.is_ok(),
        "initial typecheck should succeed: {:?}",
        result.err()
    );

    let d_ksif = temp_path.join("D.ksif");
    let e_ksif = temp_path.join("E.ksif");
    assert!(d_ksif.exists(), "D.ksif should exist");
    assert!(e_ksif.exists(), "E.ksif should exist");

    let e_ksif_mtime_1 = fs::metadata(&e_ksif).unwrap().modified().unwrap();

    std::thread::sleep(std::time::Duration::from_millis(100));

    // Modify D.ks and force rebuild only D.ksif
    let d_content_modified = r#"module D where
  export value, newValue
  
  value = 100
  newValue = 200
"#;
    fs::write(temp_path.join("D.ks"), d_content_modified).expect("write modified D.ks");

    // Force rebuild with suppress_recursive_rebuild=true
    // This should rebuild D.ksif but not E.ksif
    kscr::types::set_ksif_rebuild_policy(kscr::types::KsifRebuildPolicy {
        force_rebuild: true,
        suppress_recursive_rebuild: true,
    });

    // Typecheck a module that imports D to force D.ksif rebuild
    let dummy_content = r#"module DummyD where
  import D
  
  export dummy
  
  dummy = D.value
"#;
    fs::write(temp_path.join("DummyD.ks"), dummy_content).expect("write DummyD.ks");
    let result = kscr::types::typecheck_file(&temp_path.join("DummyD.ks"));
    assert!(
        result.is_ok(),
        "rebuild D should succeed: {:?}",
        result.err()
    );

    std::thread::sleep(std::time::Duration::from_millis(100));

    // Now typecheck F with suppress_recursive_rebuild=true
    // Even though D.ksif hash changed, E.ksif should NOT be rebuilt
    // because suppress_recursive_rebuild skips dependency validation
    kscr::types::set_ksif_rebuild_policy(kscr::types::KsifRebuildPolicy {
        force_rebuild: false,
        suppress_recursive_rebuild: true,
    });
    let result = kscr::types::typecheck_file(&temp_path.join("F.ks"));
    assert!(
        result.is_ok(),
        "typecheck with suppress should succeed: {:?}",
        result.err()
    );

    let e_ksif_mtime_2 = fs::metadata(&e_ksif).unwrap().modified().unwrap();

    // E.ksif should NOT have been rebuilt (dependency validation was skipped)
    assert_eq!(
        e_ksif_mtime_1, e_ksif_mtime_2,
        "E.ksif should not be rebuilt when suppress_recursive_rebuild=true, even with D.ksif hash mismatch"
    );

    // Reset policy and verify normal rebuild behavior
    kscr::types::set_ksif_rebuild_policy(kscr::types::KsifRebuildPolicy::default());

    std::thread::sleep(std::time::Duration::from_millis(100));

    let result = kscr::types::typecheck_file(&temp_path.join("F.ks"));
    assert!(
        result.is_ok(),
        "final typecheck should succeed: {:?}",
        result.err()
    );

    let e_ksif_mtime_3 = fs::metadata(&e_ksif).unwrap().modified().unwrap();

    // Now E.ksif SHOULD be rebuilt (dependency validation detects hash mismatch)
    assert_ne!(
        e_ksif_mtime_2, e_ksif_mtime_3,
        "E.ksif should be rebuilt when dependency hash validation is enabled"
    );
}
