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
