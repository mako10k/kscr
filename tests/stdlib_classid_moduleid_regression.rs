use std::path::Path;

// Regression: typechecking a small program should succeed after ModuleId(0) sentinel reservation.
//
// NOTE: The stdlib class env is intentionally *module-agnostic* today (classes are identified by
// unqualified name), so we must not assert on `ModuleId` here.
#[test]
fn stdlib_typecheck_smoke() {
    let path = Path::new("tests/stdlib_duplicate_class_id_sentinel_regression.ks");
    kscr::types::typecheck_file(path).expect("typecheck_file");
}

#[test]
fn string_eq_alias_typechecks() {
    let path = Path::new("tests/alias_string_eq_charlist_method_merge.ks");
    kscr::types::typecheck_file(path).expect("typecheck_file");
}
