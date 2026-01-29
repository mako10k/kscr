// Test for deprecated export declaration warning

use kscr::parser_impl;

#[test]
fn test_export_decl_deprecated_warning() {
    // Test that export declaration still works but is marked as deprecated
    let src = r#"
x = 1
y = 2
export x, y
"#;
    
    // Capture stderr to verify the warning is emitted
    // Note: In practice, the warning goes to stderr via eprintln!
    // This test just ensures parsing still works
    let module = parser_impl::parse_module(src).unwrap();
    
    // Find the export declaration
    let has_export = module.items.iter().any(|item| {
        matches!(item, kscr::ast::Item::Export(_))
    });
    
    assert!(has_export, "Export declaration should still be parsed");
}

#[test]
fn test_export_decl_functionality() {
    // Test that export declarations continue to work correctly
    let src = r#"
export foo, Bar(..)

foo = 42

data Bar = Baz | Qux
"#;
    
    let module = parser_impl::parse_module(src).unwrap();
    
    // Find the export item
    let export_item = module.items.iter().find(|item| {
        matches!(item, kscr::ast::Item::Export(_))
    });
    
    assert!(export_item.is_some(), "Export item should be present");
    
    if let Some(kscr::ast::Item::Export(export_decl)) = export_item {
        assert_eq!(export_decl.specs.len(), 2);
        
        match &export_decl.specs[0] {
            kscr::ast::ExportSpec::Name(name) => assert_eq!(name, "foo"),
            _ => panic!("Expected Name export spec"),
        }
        
        match &export_decl.specs[1] {
            kscr::ast::ExportSpec::Type { name, ctors } => {
                assert_eq!(name, "Bar");
                assert!(matches!(ctors, kscr::ast::ExportCtors::All));
            }
            _ => panic!("Expected Type export spec"),
        }
    }
}
