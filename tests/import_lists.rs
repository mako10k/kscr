// Test for Haskell-style import lists and hiding clauses

use kscr::{ast, parser_impl};

#[test]
fn test_import_simple() {
    let src = r#"
module Main where
    import Foo
    main = x
"#;
    let module = parser_impl::parse_module(src).unwrap();

    let import = match &module.items[0] {
        ast::Item::Import(id) => id,
        _ => panic!("Expected import"),
    };

    assert_eq!(import.module, "Foo");
    assert!(!import.qualified);
    assert_eq!(import.as_name, None);
    assert_eq!(import.import_spec, None);
}

#[test]
fn test_import_list_simple() {
    let src = r#"
module Main where
    import Foo (x, y, z)
    main = x
"#;
    let module = parser_impl::parse_module(src).unwrap();

    let import = match &module.items[0] {
        ast::Item::Import(id) => id,
        _ => panic!("Expected import"),
    };

    assert_eq!(import.module, "Foo");
    match &import.import_spec {
        Some(ast::ImportSpec::Only(items)) => {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0], ast::ExportSpec::Name("x".to_string()));
            assert_eq!(items[1], ast::ExportSpec::Name("y".to_string()));
            assert_eq!(items[2], ast::ExportSpec::Name("z".to_string()));
        }
        _ => panic!("Expected Only import spec"),
    }
}

#[test]
fn test_import_list_with_operator() {
    let src = r#"
module Main where
    import Foo (x, (++), y)
    main = x
"#;
    let module = parser_impl::parse_module(src).unwrap();

    let import = match &module.items[0] {
        ast::Item::Import(id) => id,
        _ => panic!("Expected import"),
    };

    match &import.import_spec {
        Some(ast::ImportSpec::Only(items)) => {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0], ast::ExportSpec::Name("x".to_string()));
            assert_eq!(items[1], ast::ExportSpec::Name("++".to_string()));
            assert_eq!(items[2], ast::ExportSpec::Name("y".to_string()));
        }
        _ => panic!("Expected Only import spec"),
    }
}

#[test]
fn test_import_hiding_simple() {
    let src = r#"
module Main where
    import Foo hiding (x, y)
    main = z
"#;
    let module = parser_impl::parse_module(src).unwrap();

    let import = match &module.items[0] {
        ast::Item::Import(id) => id,
        _ => panic!("Expected import"),
    };

    assert_eq!(import.module, "Foo");
    match &import.import_spec {
        Some(ast::ImportSpec::Hiding(items)) => {
            assert_eq!(items.len(), 2);
            assert_eq!(items[0], ast::ExportSpec::Name("x".to_string()));
            assert_eq!(items[1], ast::ExportSpec::Name("y".to_string()));
        }
        _ => panic!("Expected Hiding import spec"),
    }
}

#[test]
fn test_import_qualified_with_list() {
    let src = r#"
module Main where
    import qualified Foo as F (x, y)
    main = F.x
"#;
    let module = parser_impl::parse_module(src).unwrap();

    let import = match &module.items[0] {
        ast::Item::Import(id) => id,
        _ => panic!("Expected import"),
    };

    assert_eq!(import.module, "Foo");
    assert!(import.qualified);
    assert_eq!(import.as_name, Some("F".to_string()));
    match &import.import_spec {
        Some(ast::ImportSpec::Only(items)) => {
            assert_eq!(items.len(), 2);
            assert_eq!(items[0], ast::ExportSpec::Name("x".to_string()));
            assert_eq!(items[1], ast::ExportSpec::Name("y".to_string()));
        }
        _ => panic!("Expected Only import spec"),
    }
}

#[test]
fn test_import_list_with_newlines() {
    let src = r#"
module Main where
    import Foo (
        x,
        y,
        z
    )
    main = x
"#;
    let module = parser_impl::parse_module(src).unwrap();

    let import = match &module.items[0] {
        ast::Item::Import(id) => id,
        _ => panic!("Expected import"),
    };

    match &import.import_spec {
        Some(ast::ImportSpec::Only(items)) => {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0], ast::ExportSpec::Name("x".to_string()));
            assert_eq!(items[1], ast::ExportSpec::Name("y".to_string()));
            assert_eq!(items[2], ast::ExportSpec::Name("z".to_string()));
        }
        _ => panic!("Expected Only import spec"),
    }
}

#[test]
fn test_import_empty_list() {
    let src = r#"
module Main where
    import Foo ()
    main = x
"#;
    let module = parser_impl::parse_module(src).unwrap();

    let import = match &module.items[0] {
        ast::Item::Import(id) => id,
        _ => panic!("Expected import"),
    };

    match &import.import_spec {
        Some(ast::ImportSpec::Only(items)) => {
            assert_eq!(items.len(), 0);
        }
        _ => panic!("Expected Only import spec with empty list"),
    }
}

#[test]
fn test_import_hiding_with_trailing_comma() {
    let src = r#"
module Main where
    import Foo hiding (x, y,)
    main = z
"#;
    let module = parser_impl::parse_module(src).unwrap();

    let import = match &module.items[0] {
        ast::Item::Import(id) => id,
        _ => panic!("Expected import"),
    };

    match &import.import_spec {
        Some(ast::ImportSpec::Hiding(items)) => {
            assert_eq!(items.len(), 2);
            assert_eq!(items[0], ast::ExportSpec::Name("x".to_string()));
            assert_eq!(items[1], ast::ExportSpec::Name("y".to_string()));
        }
        _ => panic!("Expected Hiding import spec"),
    }
}

#[test]
fn test_import_as_before_spec() {
    // Test Haskell syntax: import Mod as M (x, y)
    let src = r#"
module Main where
    import Foo as F (x, y)
    main = x
"#;
    let module = parser_impl::parse_module(src).unwrap();

    let import = match &module.items[0] {
        ast::Item::Import(id) => id,
        _ => panic!("Expected import"),
    };

    assert_eq!(import.module, "Foo");
    assert!(!import.qualified);
    assert_eq!(import.as_name, Some("F".to_string()));
    match &import.import_spec {
        Some(ast::ImportSpec::Only(items)) => {
            assert_eq!(items.len(), 2);
            assert_eq!(items[0], ast::ExportSpec::Name("x".to_string()));
            assert_eq!(items[1], ast::ExportSpec::Name("y".to_string()));
        }
        _ => panic!("Expected Only import spec"),
    }
}

#[test]
fn test_import_qualified_as_hiding() {
    // Test: import qualified Mod as M hiding (x)
    let src = r#"
module Main where
    import qualified Foo as F hiding (x)
    main = F.y
"#;
    let module = parser_impl::parse_module(src).unwrap();

    let import = match &module.items[0] {
        ast::Item::Import(id) => id,
        _ => panic!("Expected import"),
    };

    assert_eq!(import.module, "Foo");
    assert!(import.qualified);
    assert_eq!(import.as_name, Some("F".to_string()));
    match &import.import_spec {
        Some(ast::ImportSpec::Hiding(items)) => {
            assert_eq!(items.len(), 1);
            assert_eq!(items[0], ast::ExportSpec::Name("x".to_string()));
        }
        _ => panic!("Expected Hiding import spec"),
    }
}

#[test]
fn test_import_type_with_all_ctors() {
    // Test: import Mod (Maybe(..))
    let src = r#"
module Main where
    import Prelude (Maybe(..))
    main = Just 42
"#;
    let module = parser_impl::parse_module(src).unwrap();

    let import = match &module.items[0] {
        ast::Item::Import(id) => id,
        _ => panic!("Expected import"),
    };

    assert_eq!(import.module, "Prelude");
    match &import.import_spec {
        Some(ast::ImportSpec::Only(items)) => {
            assert_eq!(items.len(), 1);
            match &items[0] {
                ast::ExportSpec::Type { name, ctors } => {
                    assert_eq!(name, "Maybe");
                    assert_eq!(ctors, &ast::ExportCtors::All);
                }
                _ => panic!("Expected Type with All constructors"),
            }
        }
        _ => panic!("Expected Only import spec"),
    }
}

#[test]
fn test_import_type_with_specific_ctors() {
    // Test: import Mod (Either(Left, Right))
    let src = r#"
module Main where
    import Prelude (Either(Left, Right))
    main = Left 42
"#;
    let module = parser_impl::parse_module(src).unwrap();

    let import = match &module.items[0] {
        ast::Item::Import(id) => id,
        _ => panic!("Expected import"),
    };

    assert_eq!(import.module, "Prelude");
    match &import.import_spec {
        Some(ast::ImportSpec::Only(items)) => {
            assert_eq!(items.len(), 1);
            match &items[0] {
                ast::ExportSpec::Type { name, ctors } => {
                    assert_eq!(name, "Either");
                    match ctors {
                        ast::ExportCtors::Some(list) => {
                            assert_eq!(list.len(), 2);
                            assert_eq!(list[0], "Left");
                            assert_eq!(list[1], "Right");
                        }
                        _ => panic!("Expected Some constructors"),
                    }
                }
                _ => panic!("Expected Type spec"),
            }
        }
        _ => panic!("Expected Only import spec"),
    }
}

#[test]
fn test_import_mixed_names_and_types() {
    // Test: import Mod (foo, Bar(..), baz)
    let src = r#"
module Main where
    import MyMod (foo, Bar(..), baz)
    main = foo
"#;
    let module = parser_impl::parse_module(src).unwrap();

    let import = match &module.items[0] {
        ast::Item::Import(id) => id,
        _ => panic!("Expected import"),
    };

    assert_eq!(import.module, "MyMod");
    match &import.import_spec {
        Some(ast::ImportSpec::Only(items)) => {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0], ast::ExportSpec::Name("foo".to_string()));
            match &items[1] {
                ast::ExportSpec::Type { name, ctors } => {
                    assert_eq!(name, "Bar");
                    assert_eq!(ctors, &ast::ExportCtors::All);
                }
                _ => panic!("Expected Type with All"),
            }
            assert_eq!(items[2], ast::ExportSpec::Name("baz".to_string()));
        }
        _ => panic!("Expected Only import spec"),
    }
}
