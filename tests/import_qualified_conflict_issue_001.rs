use std::path::Path;

use kscr::types;

#[test]
fn issue_001_alias_imports_do_not_conflict() {
    let path = Path::new("tests/import_qualified_conflict_issue_001_minrepro.ks");
    let result = types::typecheck_file(path);
    assert!(
        result.is_ok(),
        "qualified alias imports should not trigger a name conflict: {:?}",
        result.err()
    );
}

#[test]
fn issue_001_alias_imports_do_not_leak_unqualified_names() {
    let path = Path::new("tests/import_qualified_conflict_issue_001_unqualified_leak.ks");
    let result = types::typecheck_file(path);
    let err = result.expect_err("qualified alias imports must not leak unqualified names");
    let msg = err.to_string();
    assert!(
        msg.contains("unbound variable: map"),
        "unexpected diagnostic for alias-import leakage: {msg}"
    );
}
