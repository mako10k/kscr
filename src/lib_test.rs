#[test]
fn scaffold_parser_accepts_binding() {
    let m = crate::parser::parse_module("x = 1").unwrap();
    assert_eq!(m.items.len(), 1);
}

#[test]
fn parser_module_basic() {
    let src = std::fs::read_to_string("tests/module_basic.ks").unwrap();
    let m = crate::parser::parse_module(&src).unwrap();
    assert_eq!(m.name.as_deref(), Some("Main"));
    assert_eq!(m.items.len(), 2);
}

#[test]
fn parser_module_import_export() {
    let src = std::fs::read_to_string("tests/module_import_export.ks").unwrap();
    let m = crate::parser::parse_module(&src).unwrap();
    assert_eq!(m.name.as_deref(), Some("Main"));
    assert_eq!(m.items.len(), 5);

    use crate::ast::Item;

    match &m.items[0] {
        Item::Import(i) => {
            assert_eq!(i.module, "Foo");
            assert_eq!(i.as_name, None);
        }
        _ => panic!("expected import"),
    }

    match &m.items[1] {
        Item::Import(i) => {
            assert_eq!(i.module, "Bar");
            assert_eq!(i.as_name.as_deref(), Some("B"));
        }
        _ => panic!("expected import"),
    }

    match &m.items[2] {
        Item::Export(e) => assert_eq!(e.names, vec!["x", "y"]),
        _ => panic!("expected export"),
    }
}

#[test]
fn parser_golden_basic() {
    let src = std::fs::read_to_string("tests/basic.ks").unwrap();
    let m = crate::parser::parse_module(&src).unwrap();
    assert_eq!(m.items.len(), 3);
    // Optionally: check item names/types
}

#[test]
fn parser_golden_decl() {
    let src = std::fs::read_to_string("tests/parser_decl.ks").unwrap();
    let m = crate::parser::parse_module(&src).unwrap();
    assert_eq!(m.items.len(), 5);
    // 1: DataDecl, 2: TypeAlias, 3-5: Binding
    use crate::ast::Item;
    matches!(m.items[0], Item::DataDecl(_));
    matches!(m.items[1], Item::TypeAlias(_));
    matches!(m.items[2], Item::Binding(_));
    matches!(m.items[3], Item::Binding(_));
    matches!(m.items[4], Item::Binding(_));
}

#[test]
fn lexer_golden_basic() {
    let src = std::fs::read_to_string("tests/basic.ks").unwrap();
    let tokens = crate::lexer::lex(&src).unwrap();
    let tokens: Vec<_> = tokens
        .into_iter()
        .filter(|t| t.kind != crate::lexer::TokenKind::Newline)
        .collect();
    // 3 bindings: x = 1, flag = True, msg = "hello"
    // 9 tokens expected: Ident, Eq, Integer, Ident, Eq, True, Ident, Eq, String
    assert_eq!(tokens.len(), 9);
    assert!(matches!(tokens[0].kind, crate::lexer::TokenKind::Ident(_)));
    assert!(matches!(tokens[1].kind, crate::lexer::TokenKind::Eq));
    assert!(matches!(
        tokens[2].kind,
        crate::lexer::TokenKind::Integer(_)
    ));
    assert!(matches!(tokens[3].kind, crate::lexer::TokenKind::Ident(_)));
    assert!(matches!(tokens[4].kind, crate::lexer::TokenKind::Eq));
    assert!(matches!(tokens[5].kind, crate::lexer::TokenKind::True));
    assert!(matches!(tokens[6].kind, crate::lexer::TokenKind::Ident(_)));
    assert!(matches!(tokens[7].kind, crate::lexer::TokenKind::Eq));
    assert!(matches!(tokens[8].kind, crate::lexer::TokenKind::String(_)));
}

#[test]
fn lexer_golden_ext() {
    let src = std::fs::read_to_string("tests/lexer_ext.ks").unwrap();
    let tokens = crate::lexer::lex(&src).unwrap();
    let tokens: Vec<_> = tokens
        .into_iter()
        .filter(|t| t.kind != crate::lexer::TokenKind::Newline)
        .collect();
    // x = 1.23, flag = True, msg = "hello", module = "main"
    // コメント行は無視される
    // 12 tokens expected: Ident, Eq, Float, Ident, Eq, True, Ident, Eq, String, Ident, Eq, String
    assert_eq!(tokens.len(), 12);
    assert!(matches!(tokens[2].kind, crate::lexer::TokenKind::Float(_)));
    assert!(matches!(tokens[5].kind, crate::lexer::TokenKind::True));
    assert!(matches!(tokens[8].kind, crate::lexer::TokenKind::String(_)));
    assert!(matches!(
        tokens[11].kind,
        crate::lexer::TokenKind::String(_)
    ));
}

#[test]
fn lexer_strips_nested_block_comments() {
    let src = "x = 1 {- a {- b -} c -} y = 2";
    let tokens = crate::lexer::lex(src).unwrap();
    let tokens: Vec<_> = tokens
        .into_iter()
        .filter(|t| t.kind != crate::lexer::TokenKind::Newline)
        .collect();
    assert_eq!(tokens.len(), 6);
}

#[test]
fn lexer_skips_shebang() {
    let src = "#!/usr/bin/env kscr\nx = 1";
    let tokens = crate::lexer::lex(src).unwrap();
    let tokens: Vec<_> = tokens
        .into_iter()
        .filter(|t| t.kind != crate::lexer::TokenKind::Newline)
        .collect();
    assert_eq!(tokens.len(), 3);
}

#[test]
fn parser_golden_expr() {
    let src = std::fs::read_to_string("tests/parser_expr.ks").unwrap();
    let module = crate::parser::parse_module(&src).unwrap();
    assert_eq!(module.items.len(), 5);

    use crate::ast::{Expr, Item};

    match &module.items[0] {
        Item::Binding(b) => assert!(matches!(b.expr, Expr::Lambda { .. })),
        _ => panic!("expected binding"),
    }

    match &module.items[1] {
        Item::Binding(b) => assert!(matches!(b.expr, Expr::Lambda { .. })),
        _ => panic!("expected binding"),
    }

    match &module.items[2] {
        Item::Binding(b) => assert!(matches!(b.expr, Expr::If { .. })),
        _ => panic!("expected binding"),
    }

    match &module.items[3] {
        Item::Binding(b) => assert!(matches!(b.expr, Expr::Apply { .. })),
        _ => panic!("expected binding"),
    }

    match &module.items[4] {
        Item::Binding(b) => assert!(matches!(b.expr, Expr::Lambda { .. })),
        _ => panic!("expected binding"),
    }
}

#[test]
fn parser_golden_list_expr() {
    let src = std::fs::read_to_string("tests/parser_list.ks").unwrap();
    let module = crate::parser::parse_module(&src).unwrap();
    assert_eq!(module.items.len(), 2);

    use crate::ast::{Expr, Item};

    match &module.items[0] {
        Item::Binding(b) => assert!(matches!(&b.expr, Expr::List(v) if v.is_empty())),
        _ => panic!("expected binding"),
    }

    match &module.items[1] {
        Item::Binding(b) => match &b.expr {
            Expr::List(v) => {
                assert_eq!(v.len(), 3);
                assert!(matches!(&v[0], Expr::Integer(s) if s == "1"));
                assert!(matches!(&v[1], Expr::Integer(s) if s == "2"));
                assert!(matches!(&v[2], Expr::Apply { .. }));
            }
            _ => panic!("expected list"),
        },
        _ => panic!("expected binding"),
    }
}

#[test]
fn parser_golden_tuple_expr() {
    let src = std::fs::read_to_string("tests/parser_tuple.ks").unwrap();
    let module = crate::parser::parse_module(&src).unwrap();
    assert_eq!(module.items.len(), 3);

    use crate::ast::{Expr, Item};

    match &module.items[0] {
        Item::Binding(b) => assert!(matches!(&b.expr, Expr::Unit)),
        _ => panic!("expected binding"),
    }

    match &module.items[1] {
        Item::Binding(b) => assert!(matches!(&b.expr, Expr::Apply { .. })),
        _ => panic!("expected binding"),
    }

    match &module.items[2] {
        Item::Binding(b) => match &b.expr {
            Expr::Tuple(v) => {
                assert_eq!(v.len(), 3);
                assert!(matches!(&v[0], Expr::Integer(s) if s == "1"));
                assert!(matches!(&v[1], Expr::Integer(s) if s == "2"));
                assert!(matches!(&v[2], Expr::Apply { .. }));
            }
            _ => panic!("expected tuple"),
        },
        _ => panic!("expected binding"),
    }
}

#[test]
fn parser_golden_record_expr() {
    let src = std::fs::read_to_string("tests/parser_record.ks").unwrap();
    let module = crate::parser::parse_module(&src).unwrap();
    assert_eq!(module.items.len(), 2);

    use crate::ast::{Expr, Item};

    match &module.items[0] {
        Item::Binding(b) => assert!(matches!(&b.expr, Expr::Record(v) if v.is_empty())),
        _ => panic!("expected binding"),
    }

    match &module.items[1] {
        Item::Binding(b) => match &b.expr {
            Expr::Record(v) => {
                assert_eq!(v.len(), 2);
                assert_eq!(v[0].0, "a");
                assert!(matches!(&v[0].1, Expr::Integer(s) if s == "1"));
                assert_eq!(v[1].0, "b");
                assert!(matches!(&v[1].1, Expr::Apply { .. }));
            }
            _ => panic!("expected record"),
        },
        _ => panic!("expected binding"),
    }
}

#[test]
fn parser_golden_let_expr() {
    let src = std::fs::read_to_string("tests/parser_let.ks").unwrap();
    let module = crate::parser::parse_module(&src).unwrap();
    assert_eq!(module.items.len(), 2);

    use crate::ast::{Expr, Item};

    match &module.items[0] {
        Item::Binding(b) => match &b.expr {
            Expr::Let { bindings, body } => {
                assert_eq!(bindings.len(), 1);
                assert_eq!(bindings[0].name, "x");
                assert!(matches!(bindings[0].expr, Expr::Integer(ref s) if s == "1"));
                assert!(matches!(**body, Expr::Var(ref s) if s == "x"));
            }
            _ => panic!("expected let"),
        },
        _ => panic!("expected binding"),
    }

    match &module.items[1] {
        Item::Binding(b) => match &b.expr {
            Expr::Let { bindings, body } => {
                assert_eq!(bindings.len(), 2);
                assert_eq!(bindings[0].name, "x");
                assert_eq!(bindings[1].name, "y");
                assert!(matches!(**body, Expr::Apply { .. }));
            }
            _ => panic!("expected let"),
        },
        _ => panic!("expected binding"),
    }
}
