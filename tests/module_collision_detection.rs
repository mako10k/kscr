use kscr::types;
use std::fs;
use tempfile::TempDir;

#[test]
fn module_collision_detected() {
    // Create a temporary directory with two files declaring the same module
    let tmp = TempDir::new().unwrap();
    let base = tmp.path();

    // Create parser.ks declaring module Poc.Parser
    let parser_path = base.join("parser.ks");
    fs::write(
        &parser_path,
        "module Poc.Parser where\n  import Prelude\n  import Poc.Parser\n  foo = 42\n",
    )
    .unwrap();

    // Create Poc/Parser.ks also declaring module Poc.Parser
    let poc_dir = base.join("Poc");
    fs::create_dir(&poc_dir).unwrap();
    let poc_parser_path = poc_dir.join("Parser.ks");
    fs::write(
        &poc_parser_path,
        "module Poc.Parser where\n  import Prelude\n  bar = 100\n",
    )
    .unwrap();

    // Attempt to typecheck parser.ks which imports Poc.Parser (triggers collision)
    let result = types::typecheck_file(&parser_path);

    // Should fail with module collision error
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("module 'Poc.Parser' is defined in multiple files"),
        "Expected module collision error, got: {err_msg}"
    );
    assert!(
        err_msg.contains("parser.ks"),
        "Expected parser.ks path in error, got: {err_msg}"
    );
    assert!(
        err_msg.contains("Poc/Parser.ks") || err_msg.contains("Poc\\Parser.ks"),
        "Expected Poc/Parser.ks path in error, got: {err_msg}"
    );
}
