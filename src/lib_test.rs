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
fn parser_binding_patterns() {
    let src = std::fs::read_to_string("tests/parser_binding_patterns.ks").unwrap();
    let m = crate::parser::parse_module(&src).unwrap();
    assert_eq!(m.items.len(), 3);

    use crate::ast::{Item, Pattern};

    match &m.items[1] {
        Item::Binding(b) => assert!(matches!(b.pat, Pattern::Tuple(_))),
        _ => panic!("expected binding"),
    }

    match &m.items[2] {
        Item::Binding(b) => assert!(matches!(b.pat, Pattern::Wildcard)),
        _ => panic!("expected binding"),
    }
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
                assert!(matches!(
                    bindings[0].pat,
                    crate::ast::Pattern::Var(ref s) if s == "x"
                ));
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
                assert!(matches!(
                    bindings[0].pat,
                    crate::ast::Pattern::Var(ref s) if s == "x"
                ));
                assert!(matches!(
                    bindings[1].pat,
                    crate::ast::Pattern::Var(ref s) if s == "y"
                ));
                assert!(matches!(**body, Expr::Apply { .. }));
            }
            _ => panic!("expected let"),
        },
        _ => panic!("expected binding"),
    }
}

#[test]
fn parser_golden_case_expr() {
    let src = std::fs::read_to_string("tests/parser_case.ks").unwrap();
    let module = crate::parser::parse_module(&src).unwrap();
    assert_eq!(module.items.len(), 2);

    use crate::ast::{Expr, Item, Pattern};

    match &module.items[0] {
        Item::Binding(b) => match &b.expr {
            Expr::Case { arms, .. } => {
                assert_eq!(arms.len(), 1);
                assert!(matches!(arms[0].0, Pattern::Wildcard));
            }
            _ => panic!("expected case"),
        },
        _ => panic!("expected binding"),
    }

    match &module.items[1] {
        Item::Binding(b) => match &b.expr {
            Expr::Case { arms, .. } => {
                assert_eq!(arms.len(), 2);
                assert!(matches!(arms[0].0, Pattern::Literal(Expr::Bool(true))));
                assert!(matches!(arms[1].0, Pattern::Literal(Expr::Bool(false))));
            }
            _ => panic!("expected case"),
        },
        _ => panic!("expected binding"),
    }
}

#[test]
fn parser_golden_where_expr() {
    let src = std::fs::read_to_string("tests/parser_where.ks").unwrap();
    let module = crate::parser::parse_module(&src).unwrap();
    assert_eq!(module.items.len(), 2);

    use crate::ast::{Expr, Item};

    match &module.items[0] {
        Item::Binding(b) => match &b.expr {
            Expr::Where { bindings, .. } => {
                assert_eq!(bindings.len(), 1);
                assert!(matches!(
                    bindings[0].pat,
                    crate::ast::Pattern::Var(ref s) if s == "x"
                ));
            }
            _ => panic!("expected where"),
        },
        _ => panic!("expected binding"),
    }

    match &module.items[1] {
        Item::Binding(b) => match &b.expr {
            Expr::Where { bindings, .. } => {
                assert_eq!(bindings.len(), 2);
                assert!(matches!(
                    bindings[0].pat,
                    crate::ast::Pattern::Var(ref s) if s == "x"
                ));
                assert!(matches!(
                    bindings[1].pat,
                    crate::ast::Pattern::Var(ref s) if s == "y"
                ));
            }
            _ => panic!("expected where"),
        },
        _ => panic!("expected binding"),
    }
}

#[test]
fn parser_case_patterns() {
    let src = std::fs::read_to_string("tests/parser_case_patterns.ks").unwrap();
    let module = crate::parser::parse_module(&src).unwrap();
    assert_eq!(module.items.len(), 1);

    use crate::ast::{Expr, Item, Pattern};

    let Item::Binding(b) = &module.items[0] else {
        panic!("expected binding");
    };

    let Expr::Case { arms, .. } = &b.expr else {
        panic!("expected case");
    };

    assert_eq!(arms.len(), 5);
    assert!(matches!(arms[0].0, Pattern::Literal(Expr::Unit)));
    assert!(matches!(arms[1].0, Pattern::Tuple(_)));
    assert!(matches!(arms[2].0, Pattern::List(_)));
    assert!(matches!(arms[3].0, Pattern::Record(_)));
    assert!(matches!(arms[4].0, Pattern::Constructor { .. }));
}

#[test]
fn parser_type_annotations() {
    let src = std::fs::read_to_string("tests/parser_annot.ks").unwrap();
    let module = crate::parser::parse_module(&src).unwrap();
    assert_eq!(module.items.len(), 3);

    use crate::ast::{Expr, Item, Type};

    let Item::Binding(b0) = &module.items[0] else {
        panic!("expected binding");
    };
    assert!(matches!(
        b0.expr,
        Expr::Annot {
            ty: Type::Var(ref s),
            ..
        } if s == "Integer"
    ));

    let Item::Binding(b1) = &module.items[1] else {
        panic!("expected binding");
    };
    assert!(matches!(
        b1.expr,
        Expr::Annot {
            ty: Type::Var(ref s),
            ..
        } if s == "Float64"
    ));

    let Item::Binding(b2) = &module.items[2] else {
        panic!("expected binding");
    };
    let Expr::List(v) = &b2.expr else {
        panic!("expected list");
    };
    assert!(matches!(
        v[0],
        Expr::Annot {
            ty: Type::Var(ref s),
            ..
        } if s == "Integer"
    ));
}

#[test]
fn parser_do_blocks() {
    let src = std::fs::read_to_string("tests/parser_do.ks").unwrap();
    let module = crate::parser::parse_module(&src).unwrap();
    assert_eq!(module.items.len(), 1);

    use crate::ast::{DoStmt, Expr, Item, Pattern};

    let Item::Binding(b) = &module.items[0] else {
        panic!("expected binding");
    };
    assert!(matches!(b.pat, Pattern::Var(ref s) if s == "main"));

    let Expr::Do(stmts) = &b.expr else {
        panic!("expected do");
    };

    assert_eq!(stmts.len(), 3);
    assert!(matches!(stmts[0], DoStmt::Bind { .. }));
    assert!(matches!(stmts[1], DoStmt::Bind { .. }));
    assert!(matches!(stmts[2], DoStmt::Expr(_)));
}

#[test]
fn parser_ctor_exprs() {
    let src = std::fs::read_to_string("tests/parser_ctor_expr.ks").unwrap();
    let module = crate::parser::parse_module(&src).unwrap();
    assert_eq!(module.items.len(), 2);

    use crate::ast::{Expr, Item};

    let Item::Binding(b0) = &module.items[0] else {
        panic!("expected binding");
    };
    assert!(matches!(
        b0.expr,
        Expr::Apply { ref func, ref args }
            if matches!(**func, Expr::Ctor(ref s) if s == "Just") && args.len() == 1
    ));

    let Item::Binding(b1) = &module.items[1] else {
        panic!("expected binding");
    };
    assert!(matches!(b1.expr, Expr::Ctor(ref s) if s == "Nothing"));
}

#[test]
fn parser_infix_backticks() {
    let src = std::fs::read_to_string("tests/parser_infix.ks").unwrap();
    let module = crate::parser::parse_module(&src).unwrap();
    assert_eq!(module.items.len(), 2);

    use crate::ast::{Expr, Item};

    let Item::Binding(b0) = &module.items[0] else {
        panic!("expected binding");
    };
    assert!(matches!(
        b0.expr,
        Expr::Apply { ref func, ref args }
            if matches!(**func, Expr::Var(ref s) if s == "f") && args.len() == 2
    ));

    let Item::Binding(b1) = &module.items[1] else {
        panic!("expected binding");
    };
    // left associative: (a `f` b) `g` c
    let Expr::Apply { func, args } = &b1.expr else {
        panic!("expected apply");
    };
    assert!(matches!(**func, Expr::Var(ref s) if s == "g"));
    assert_eq!(args.len(), 2);
    assert!(matches!(args[1], Expr::Var(ref s) if s == "c"));
    assert!(matches!(args[0], Expr::Apply { .. }));
}

#[test]
fn parser_symbol_ops() {
    let src = std::fs::read_to_string("tests/parser_ops.ks").unwrap();
    let module = crate::parser::parse_module(&src).unwrap();
    assert_eq!(module.items.len(), 2);

    use crate::ast::{Expr, Item};

    let Item::Binding(b0) = &module.items[0] else {
        panic!("expected binding");
    };
    // 1 + (2 * 3)
    let Expr::Apply { func, args } = &b0.expr else {
        panic!("expected apply");
    };
    assert!(matches!(**func, Expr::Var(ref s) if s == "+"));
    assert_eq!(args.len(), 2);
    assert!(matches!(args[0], Expr::Integer(ref s) if s == "1"));
    assert!(matches!(args[1], Expr::Apply { .. }));

    let Item::Binding(b1) = &module.items[1] else {
        panic!("expected binding");
    };
    // (10 / 2) - 1
    let Expr::Apply { func, args } = &b1.expr else {
        panic!("expected apply");
    };
    assert!(matches!(**func, Expr::Var(ref s) if s == "-"));
    assert_eq!(args.len(), 2);
    assert!(matches!(args[1], Expr::Integer(ref s) if s == "1"));
    assert!(matches!(args[0], Expr::Apply { .. }));
}

#[test]
fn parser_cmp_and_logic_ops() {
    let src = std::fs::read_to_string("tests/parser_cmp_logic.ks").unwrap();
    let module = crate::parser::parse_module(&src).unwrap();
    assert_eq!(module.items.len(), 3);

    use crate::ast::{Expr, Item};

    let Item::Binding(b0) = &module.items[0] else {
        panic!("expected binding");
    };
    // (1 + (2 * 3)) == 7
    let Expr::Apply { func, args } = &b0.expr else {
        panic!("expected apply");
    };
    assert!(matches!(**func, Expr::Var(ref s) if s == "=="));
    assert_eq!(args.len(), 2);

    let Item::Binding(b1) = &module.items[1] else {
        panic!("expected binding");
    };
    // (True && False) || True
    let Expr::Apply { func, args } = &b1.expr else {
        panic!("expected apply");
    };
    assert!(matches!(**func, Expr::Var(ref s) if s == "||"));
    assert_eq!(args.len(), 2);
    assert!(matches!(args[1], Expr::Bool(true)));

    let Item::Binding(b2) = &module.items[2] else {
        panic!("expected binding");
    };
    // (1 < 2) && (2 <= 3)
    let Expr::Apply { func, args } = &b2.expr else {
        panic!("expected apply");
    };
    assert!(matches!(**func, Expr::Var(ref s) if s == "&&"));
    assert_eq!(args.len(), 2);
}
