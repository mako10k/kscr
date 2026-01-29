// Integration tests for import lists and hiding with type constructors

use kscr::types;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn test_import_code(src: &str, test_name: &str) -> Result<(), kscr::error::Error> {
    // Write to a file in the tests directory so it can find TestDataTypes.ks
    // Use a counter to ensure unique filenames
    let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let test_file = format!(
        "tests/test_import_{}_{}_{}.ks",
        test_name,
        std::process::id(),
        counter
    );
    fs::write(&test_file, src)?;
    let result = types::typecheck_file(std::path::Path::new(&test_file));
    let _ = fs::remove_file(&test_file);
    result.map(|_| ())
}

#[test]
fn test_import_type_all_constructors() {
    // Import a type with all constructors using (..)
    let src = r#"
module Main where
  import TestDataTypes (Color(..))
  main = pure Red
"#;
    let result = test_import_code(src, "test");
    assert!(result.is_ok(), "Should compile: {:?}", result.err());
}

#[test]
fn test_import_type_specific_constructors() {
    // Import a type with specific constructors
    let src = r#"
module Main where
  import TestDataTypes (Result(Ok))
  main = pure (Ok 42)
"#;
    let result = test_import_code(src, "test");
    assert!(result.is_ok(), "Should compile: {:?}", result.err());
}

#[test]
fn test_import_type_specific_ctor_reject_other() {
    // Import only Ok, but try to use Err (should fail)
    let src = r#"
module Main where
  import TestDataTypes (Result(Ok))
  main = pure (Err "error")
"#;
    let result = test_import_code(src, "test");
    assert!(result.is_err(), "Should fail: Err not imported");
}

#[test]
fn test_import_hiding_type_constructors() {
    // Hide a type's constructors
    let src = r#"
module Main where
  import TestDataTypes hiding (Color(..))
  main = pure foo
"#;
    let result = test_import_code(src, "test");
    assert!(result.is_ok(), "Should compile: {:?}", result.err());
}

#[test]
fn test_import_hiding_reject_hidden() {
    // Try to use a hidden name
    let src = r#"
module Main where
  import TestDataTypes hiding (foo)
  main = pure foo
"#;
    let result = test_import_code(src, "test");
    assert!(result.is_err(), "Should fail: foo is hidden");
}

#[test]
fn test_import_list_basic_filtering() {
    // Import only specific names
    let src = r#"
module Main where
  import TestDataTypes (foo)
  main = pure foo
"#;
    let result = test_import_code(src, "test");
    assert!(result.is_ok(), "Should compile: {:?}", result.err());
}

#[test]
fn test_import_list_reject_unlisted() {
    // Try to use a name not in the import list
    let src = r#"
module Main where
  import TestDataTypes (foo, bar)
  main = pure baz
"#;
    let result = test_import_code(src, "test");
    assert!(result.is_err(), "Should fail: baz not in import list");
}

#[test]
fn test_import_qualified_with_list() {
    // Qualified import with list should still filter
    let src = r#"
module Main where
  import qualified TestDataTypes as TD (foo)
  main = pure TD.foo
"#;
    let result = test_import_code(src, "test");
    assert!(result.is_ok(), "Should compile: {:?}", result.err());
}

#[test]
fn test_import_qualified_list_reject_unlisted() {
    // Qualified import, try to use unlisted name
    let src = r#"
module Main where
  import qualified TestDataTypes as TD (foo)
  main = pure TD.bar
"#;
    let result = test_import_code(src, "test");
    assert!(result.is_err(), "Should fail: TD.bar not in import list");
}

#[test]
fn test_import_prelude_maybe_all_ctors() {
    // Import Maybe with all constructors from Prelude
    let src = r#"
module Main where
  import Prelude (Maybe(..))
  main = pure (Just 42)
"#;
    let result = test_import_code(src, "test");
    assert!(result.is_ok(), "Should compile: {:?}", result.err());
}

#[test]
fn test_import_as_before_spec_syntax() {
    // Test that "as" comes before import spec
    let src = r#"
module Main where
  import TestDataTypes as TD (foo)
  main = pure foo
"#;
    let result = test_import_code(src, "test");
    assert!(result.is_ok(), "Should compile: {:?}", result.err());
}
