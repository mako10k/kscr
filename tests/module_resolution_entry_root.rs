use kscr::types;
use std::fs;
use tempfile::TempDir;

#[test]
fn typecheck_file_resolves_imports_from_entry_module_root() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    let poc_dir = root.join("Poc");
    fs::create_dir_all(&poc_dir).unwrap();

    let entry = poc_dir.join("ImportSibling.ks");
    fs::write(
        &entry,
        "module Poc.ImportSibling where\n  import Poc.Parser\n  x = Poc.Parser.answer\n",
    )
    .unwrap();

    let parser = poc_dir.join("Parser.ks");
    fs::write(&parser, "module Poc.Parser where\n  answer = 42\n").unwrap();

    let tm = types::typecheck_file(&entry).unwrap();
    assert_eq!(tm.module.name.as_deref(), Some("Poc.ImportSibling"));
}

#[test]
fn typecheck_file_keeps_entry_parent_fallback_when_layout_mismatches_module_path() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    let entry = root.join("Main.ks");
    fs::write(
        &entry,
        "module Foo.Main where\n  import Foo.Parser\n  x = Foo.Parser.answer\n",
    )
    .unwrap();

    let foo_dir = root.join("Foo");
    fs::create_dir_all(&foo_dir).unwrap();
    fs::write(
        foo_dir.join("Parser.ks"),
        "module Foo.Parser where\n  answer = 42\n",
    )
    .unwrap();

    let tm = types::typecheck_file(&entry).unwrap();
    assert_eq!(tm.module.name.as_deref(), Some("Foo.Main"));
}

#[test]
fn runtime_import_traversal_keeps_entry_parent_fallback_when_layout_mismatches_module_path() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    let entry = root.join("Main.ks");
    fs::write(
        &entry,
        "module Foo.Main where\n  import Foo.Parser\n  x = Foo.Parser.answer\n",
    )
    .unwrap();

    let foo_dir = root.join("Foo");
    fs::create_dir_all(&foo_dir).unwrap();
    fs::write(
        foo_dir.join("Parser.ks"),
        "module Foo.Parser where\n  answer = 42\n",
    )
    .unwrap();

    let imported = types::load_transitive_imports_for_runtime(&entry).unwrap();
    assert!(imported.contains_key("Foo.Parser"));
}
