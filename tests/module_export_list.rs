// Test for Haskell-style module export lists

use kscr::parser_impl;

#[test]
fn test_module_export_list_simple() {
    let src = r#"
module Foo (x, y) where
    x = 1
    y = 2
    z = 3
"#;
    let module = parser_impl::parse_module(src).unwrap();

    assert_eq!(module.name, Some("Foo".to_string()));
    assert!(module.export_specs.is_some());

    let specs = module.export_specs.unwrap();
    assert_eq!(specs.len(), 2);

    match &specs[0] {
        kscr::ast::ExportSpec::Name(name) => assert_eq!(name, "x"),
        _ => panic!("Expected Name export spec"),
    }

    match &specs[1] {
        kscr::ast::ExportSpec::Name(name) => assert_eq!(name, "y"),
        _ => panic!("Expected Name export spec"),
    }
}

#[test]
fn test_module_export_list_with_newlines() {
    let src = r#"
module Foo (
    x,
    y,
    z
) where
    x = 1
    y = 2
    z = 3
"#;
    let module = parser_impl::parse_module(src).unwrap();

    assert_eq!(module.name, Some("Foo".to_string()));
    assert!(module.export_specs.is_some());

    let specs = module.export_specs.unwrap();
    assert_eq!(specs.len(), 3);
}

#[test]
fn test_module_export_list_with_type_all_ctors() {
    let src = r#"
module Foo (MyType(..)) where
    data MyType = A | B | C
"#;
    let module = parser_impl::parse_module(src).unwrap();

    assert_eq!(module.name, Some("Foo".to_string()));
    assert!(module.export_specs.is_some());

    let specs = module.export_specs.unwrap();
    assert_eq!(specs.len(), 1);

    match &specs[0] {
        kscr::ast::ExportSpec::Type { name, ctors } => {
            assert_eq!(name, "MyType");
            assert!(matches!(ctors, kscr::ast::ExportCtors::All));
        }
        _ => panic!("Expected Type export spec"),
    }
}

#[test]
fn test_module_export_list_with_type_some_ctors() {
    let src = r#"
module Foo (MyType(A, B)) where
    data MyType = A | B | C
"#;
    let module = parser_impl::parse_module(src).unwrap();

    assert_eq!(module.name, Some("Foo".to_string()));
    assert!(module.export_specs.is_some());

    let specs = module.export_specs.unwrap();
    assert_eq!(specs.len(), 1);

    match &specs[0] {
        kscr::ast::ExportSpec::Type { name, ctors } => {
            assert_eq!(name, "MyType");
            match ctors {
                kscr::ast::ExportCtors::Some(names) => {
                    assert_eq!(names.len(), 2);
                    assert_eq!(names[0], "A");
                    assert_eq!(names[1], "B");
                }
                _ => panic!("Expected Some ctors"),
            }
        }
        _ => panic!("Expected Type export spec"),
    }
}

#[test]
fn test_module_without_export_list() {
    let src = r#"
module Foo where
    x = 1
    y = 2
"#;
    let module = parser_impl::parse_module(src).unwrap();

    assert_eq!(module.name, Some("Foo".to_string()));
    assert!(module.export_specs.is_none());
}

#[test]
fn test_module_export_list_trailing_comma() {
    let src = r#"
module Foo (
    x,
    y,
) where
    x = 1
    y = 2
"#;
    let module = parser_impl::parse_module(src).unwrap();

    assert_eq!(module.name, Some("Foo".to_string()));
    assert!(module.export_specs.is_some());

    let specs = module.export_specs.unwrap();
    assert_eq!(specs.len(), 2);
}

#[test]
fn test_module_empty_export_list() {
    let src = r#"
module Foo () where
    x = 1
"#;
    let module = parser_impl::parse_module(src).unwrap();

    assert_eq!(module.name, Some("Foo".to_string()));
    assert!(module.export_specs.is_some());

    let specs = module.export_specs.unwrap();
    assert_eq!(specs.len(), 0);
}
