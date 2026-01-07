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

    use crate::ast::{Item, Type};

    assert!(matches!(m.items[0], Item::DataDecl(_)));

    match &m.items[1] {
        Item::TypeAlias(ta) => assert_eq!(ta.ty, Type::List(Box::new(Type::Char))),
        _ => panic!("expected type alias"),
    }

    assert!(matches!(m.items[2], Item::Binding(_)));
    assert!(matches!(m.items[3], Item::Binding(_)));
    assert!(matches!(m.items[4], Item::Binding(_)));
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
fn lexer_char_literal() {
    let tokens = crate::lexer::lex("x = 'a'\ny = '\\n'\n").unwrap();
    let tokens: Vec<_> = tokens
        .into_iter()
        .filter(|t| t.kind != crate::lexer::TokenKind::Newline)
        .collect();

    assert!(matches!(tokens[2].kind, crate::lexer::TokenKind::Char('a')));
    assert!(matches!(tokens[5].kind, crate::lexer::TokenKind::Char('\n')));
}

#[test]
fn parser_char_literal() {
    let m = crate::parser::parse_module("x = 'a'\n").unwrap();
    use crate::ast::{Expr, Item};
    match &m.items[0] {
        Item::Binding(b) => assert!(matches!(b.expr, Expr::Char('a'))),
        _ => panic!("expected binding"),
    }
}

#[test]
fn parser_cons_pattern() {
    let m = crate::parser::parse_module("x:xs = ys\n").unwrap();
    use crate::ast::{Item, Pattern};
    match &m.items[0] {
        Item::Binding(b) => assert!(matches!(&b.pat, Pattern::Cons(_, _))),
        _ => panic!("expected binding"),
    }
}

#[test]
fn parser_cons_expr() {
    let m = crate::parser::parse_module("xs = 1:[]\n").unwrap();
    use crate::ast::{Expr, Item};
    match &m.items[0] {
        Item::Binding(b) => assert!(matches!(&b.expr, Expr::Cons { .. })),
        _ => panic!("expected binding"),
    }
}

#[test]
fn typecheck_cons_pattern_binds() {
    let src = "x:xs = [1, 2]\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();

    let mut names: Vec<_> = tm.inferred.keys().cloned().collect();
    names.sort();
    assert_eq!(names, vec!["x".to_string(), "xs".to_string()]);

    assert_eq!(tm.inferred["x"].to_string(), "Integer");
    assert_eq!(tm.inferred["xs"].to_string(), "[Integer]");
}

#[test]
fn typecheck_cons_expr() {
    let m = crate::parser::parse_module("xs = 1:[]\n").unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    assert_eq!(tm.inferred["xs"].to_string(), "[Integer]");
}

#[test]
fn parser_as_pattern() {
    let m = crate::parser::parse_module("xs@_ = [1]\n").unwrap();
    use crate::ast::{Item, Pattern};
    match &m.items[0] {
        Item::Binding(b) => assert!(matches!(&b.pat, Pattern::As(_, _))),
        _ => panic!("expected binding"),
    }
}

#[test]
fn typecheck_as_pattern_binds() {
    let src = "xs@_ = [1]\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    assert_eq!(tm.inferred["xs"].to_string(), "[Integer]");
}

#[test]
fn typecheck_as_pattern_with_cons_binds_all() {
    let src = "xs@(x:xt) = [1, 2]\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();

    assert_eq!(tm.inferred["xs"].to_string(), "[Integer]");
    assert_eq!(tm.inferred["x"].to_string(), "Integer");
    assert_eq!(tm.inferred["xt"].to_string(), "[Integer]");
}

#[test]
fn parser_hole_pattern() {
    let m = crate::parser::parse_module("? = 1\n?x = 2\n").unwrap();
    use crate::ast::{Item, Pattern};
    match &m.items[0] {
        Item::Binding(b) => assert!(matches!(&b.pat, Pattern::Hole(None))),
        _ => panic!("expected binding"),
    }
    match &m.items[1] {
        Item::Binding(b) => assert!(matches!(&b.pat, Pattern::Hole(Some(name)) if name == "x")),
        _ => panic!("expected binding"),
    }
}

#[test]
fn typecheck_hole_pattern_binds_nothing() {
    let m = crate::parser::parse_module("? = [1]\n").unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    assert!(tm.inferred.is_empty());
}

#[test]
fn parser_record_loose_pattern() {
    let m = crate::parser::parse_module("{x: a, ...} = r\n").unwrap();
    use crate::ast::{Item, Pattern};
    match &m.items[0] {
        Item::Binding(b) => assert!(matches!(&b.pat, Pattern::RecordLoose(_))),
        _ => panic!("expected binding"),
    }
}

#[test]
fn typecheck_record_loose_pattern_binds_fields() {
    let m = crate::parser::parse_module("{x: a, ...} = {x: 1, y: 2}\n").unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    assert_eq!(tm.inferred["a"].to_string(), "Integer");
}

#[test]
fn parser_view_pattern() {
    let m = crate::parser::parse_module("(Just n <- id) = x\n").unwrap();
    use crate::ast::{Item, Pattern};
    match &m.items[0] {
        Item::Binding(b) => assert!(matches!(&b.pat, Pattern::View(_, _))),
        _ => panic!("expected binding"),
    }
}

#[test]
fn typecheck_view_pattern_binds() {
    let src = "data Maybe a = Nothing | Just a\n\
id = \\x -> x\n\
(Just n <- id) = Just 1\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    assert_eq!(tm.inferred["n"].to_string(), "Integer");
}

#[test]
fn typecheck_or_pattern_case_arm() {
    let src = "f = \\x -> case x of\n  0 | 1 -> 1\n  _ -> 0\n";
    let m = crate::parser::parse_module(src).unwrap();
    crate::types::typecheck(m).unwrap();
}

#[test]
fn typecheck_or_pattern_requires_same_binds() {
    let src = "f = \\x -> case x of\n  0 | y -> 1\n";
    let m = crate::parser::parse_module(src).unwrap();
    assert!(crate::types::typecheck(m).is_err());
}

#[test]
fn typecheck_case_guard_bool() {
    let src = "f = \\x -> case x of\n  _ | if True then True else False -> 1\n  _ -> 0\n";
    let m = crate::parser::parse_module(src).unwrap();
    crate::types::typecheck(m).unwrap();
}

#[test]
fn typecheck_case_guard_must_be_bool() {
    let src = "f = \\x -> case x of\n  _ | let y = 1 in y -> 1\n";
    let m = crate::parser::parse_module(src).unwrap();
    assert!(crate::types::typecheck(m).is_err());
}

#[test]
fn typecheck_main_must_be_io_unit() {
    let m_ok = crate::parser::parse_module("main = IO ()\n").unwrap();
    crate::types::typecheck(m_ok).unwrap();

    let m_bad = crate::parser::parse_module("main = 1\n").unwrap();
    assert!(crate::types::typecheck(m_bad).is_err());

    let m_bad2 = crate::parser::parse_module("main = IO 1\n").unwrap();
    assert!(crate::types::typecheck(m_bad2).is_err());
}

#[test]
fn typecheck_list_comprehension_simple() {
    let m = crate::parser::parse_module("xs = [x | x <- [1, 2]]\n").unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    assert_eq!(tm.inferred["xs"].to_string(), "[Integer]");
}

#[test]
fn ir_run_main_list_comprehension() {
    let src = "main = case [x | x <- [1, 2]] of\n  [1, 2] -> IO ()\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_plus_operator() {
    let src = "main = case (1 + 2) of\n  3 -> IO ()\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_eqeq_operator() {
    let src = "main = case (1 == 1) of\n  True -> IO ()\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_minus_operator() {
    let src = "main = case (3 - 2) of\n  1 -> IO ()\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_mul_operator() {
    let src = "main = case (2 * 3) of\n  6 -> IO ()\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_lt_le_operators() {
    let src = "main = case (1 < 2) of\n  True -> case (2 <= 2) of\n    True -> IO ()\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_and_or_short_circuit() {
    let src = "main = let\n  bad = case True of\n    False -> True\nin case (False && bad) of\n  False -> case (True || bad) of\n    True -> IO ()\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn typecheck_list_comprehension_with_guard() {
    let m = crate::parser::parse_module("xs = [x | x <- [1, 2], True]\n").unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    assert_eq!(tm.inferred["xs"].to_string(), "[Integer]");
}

#[test]
fn typecheck_list_comprehension_with_pattern_bind() {
    let m = crate::parser::parse_module("xs = [a | (a, b) <- [(1, 2)]]\n").unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    assert_eq!(tm.inferred["xs"].to_string(), "[Integer]");
}

#[test]
fn ir_lowering_basic_binding() {
    let m = crate::parser::parse_module("x = if True then 1 else 2\n").unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    assert!(matches!(
        &ir.items[..],
        [crate::ir::IrItem::Binding { name, .. }] if name == "x"
    ));
}

#[test]
fn ir_lowering_case_expr() {
    let m = crate::parser::parse_module("x = case 1 of\n  0 -> 1\n  _ -> 2\n").unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let [crate::ir::IrItem::Binding { expr, .. }] = &ir.items[..] else {
        panic!("expected single binding");
    };
    assert!(matches!(expr, crate::ir::IrExpr::Case { .. }));
}

#[test]
fn ir_lowering_where_expr() {
    let src = "x = 1 where\n  y = 2\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let [crate::ir::IrItem::Binding { expr, .. }] = &ir.items[..] else {
        panic!("expected single binding");
    };
    assert!(matches!(expr, crate::ir::IrExpr::Let { .. }));
}

#[test]
fn ir_lowering_as_pattern() {
    let src = "x = case 1 of\n  n @ _ -> n\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let [crate::ir::IrItem::Binding { expr, .. }] = &ir.items[..] else {
        panic!("expected single binding");
    };
    let crate::ir::IrExpr::Case { arms, .. } = expr else {
        panic!("expected case");
    };
    assert!(matches!(arms[0].pat, crate::ir::IrPattern::As(_, _)));
}

#[test]
fn ir_lowering_loose_record_pattern() {
    let src = "x = case {a: 1, b: 2} of\n  {a: n, ...} -> n\n  _ -> 0\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let [crate::ir::IrItem::Binding { expr, .. }] = &ir.items[..] else {
        panic!("expected single binding");
    };
    let crate::ir::IrExpr::Case { arms, .. } = expr else {
        panic!("expected case");
    };
    assert!(matches!(arms[0].pat, crate::ir::IrPattern::RecordLoose(_)));
}

#[test]
fn ir_lowering_view_pattern() {
    let src = "id = \\x -> x\nx = case 1 of\n  (n <- id) -> n\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let [_, crate::ir::IrItem::Binding { expr, .. }] = &ir.items[..] else {
        panic!("expected two bindings");
    };
    let crate::ir::IrExpr::Case { arms, .. } = expr else {
        panic!("expected case");
    };
    assert!(matches!(arms[0].pat, crate::ir::IrPattern::View(_, _)));
}

#[test]
fn ir_lowering_hole_pattern() {
    let src = "x = case 1 of\n  ? -> 0\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let [crate::ir::IrItem::Binding { expr, .. }] = &ir.items[..] else {
        panic!("expected single binding");
    };
    let crate::ir::IrExpr::Case { arms, .. } = expr else {
        panic!("expected case");
    };
    assert!(matches!(arms[0].pat, crate::ir::IrPattern::Wildcard));
}

#[test]
fn ir_lowering_do_expr() {
    let src = "x = do\n  y <- IO 1\n  IO y\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let [crate::ir::IrItem::Binding { expr, .. }] = &ir.items[..] else {
        panic!("expected single binding");
    };
    assert!(matches!(expr, crate::ir::IrExpr::IoBind { .. }));
}

#[test]
fn ir_lowering_do_then() {
    let src = "x = do\n  IO 1\n  IO 2\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let [crate::ir::IrItem::Binding { expr, .. }] = &ir.items[..] else {
        panic!("expected single binding");
    };
    assert!(matches!(expr, crate::ir::IrExpr::IoThen { .. }));
}

#[test]
fn ir_lowering_pattern_let() {
    let src = "x = let (a, b) = (1, 2) in a\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let [crate::ir::IrItem::Binding { expr, .. }] = &ir.items[..] else {
        panic!("expected single binding");
    };
    assert!(matches!(expr, crate::ir::IrExpr::Case { .. }));
}

#[test]
fn ir_lowering_pattern_where() {
    let src = "x = a where\n  (a, b) = (1, 2)\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let [crate::ir::IrItem::Binding { expr, .. }] = &ir.items[..] else {
        panic!("expected single binding");
    };
    assert!(matches!(expr, crate::ir::IrExpr::Case { .. }));
}

#[test]
fn ir_lowering_top_level_pattern_binding() {
    let src = "(a, b) = (1, 2)\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    assert!(ir.items.iter().any(|it| matches!(it, crate::ir::IrItem::Binding { name, .. } if name == "a")));
    assert!(ir.items.iter().any(|it| matches!(it, crate::ir::IrItem::Binding { name, .. } if name == "b")));
}

#[test]
fn ir_run_main_io_unit() {
    let src = "main = do\n  IO 1\n  IO ()\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_cons_pattern_matches() {
    let src = "main = case [1, 2] of\n  x:xs -> IO ()\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_cons_expr_and_pattern() {
    let src = "main = case (1:[]) of\n  x:xs -> IO ()\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_cons_expr_head_is_lazy() {
    let src = "main = let\n  bad = case True of\n    False -> 0\nin case (bad:[]) of\n  _:xs -> IO ()\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_cons_expr_tail_is_lazy() {
    let src = "main = let\n  bad = case True of\n    False -> []\nin case (1:bad) of\n  x:xs -> IO ()\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_cons_pattern_is_lazy_in_tail() {
    let src = "main = let\n  bad = case True of\n    False -> 0\n  xs = [1, bad]\nin case xs of\n  x:xt -> IO ()\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_let_is_lazy() {
    let src = "main = let\n  x = case True of\n    False -> ()\nin IO ()\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_apply_args_are_lazy() {
    let src = "main = let\n  bad = case True of\n    False -> ()\n  f = \\a b -> a\nin do\n  IO (f 1 bad)\n  IO ()\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_tuple_elems_are_lazy() {
    let src = "main = let\n  bad = case True of\n    False -> ()\n  x = (1, bad)\n  first = case x of\n    (a, b) -> a\nin do\n  IO first\n  IO ()\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_with_print() {
    let src = "main = do\n  print \"hi\"\n  IO ()\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_with_stdout_write() {
    let src = "main = do\n  stdoutWrite \"hi\"\n  IO ()\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_typechecks_read_line() {
    let src = "main = do\n  x <- readLine\n  stdoutWrite x\n  IO ()\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let _ir = crate::ir::lower_to_ir(&tm.module).unwrap();
}

#[test]
fn ir_run_main_detects_cycle() {
    use crate::ir::{IrExpr, IrItem, IrModule};
    let ir = IrModule {
        items: vec![IrItem::Binding {
            name: "main".to_string(),
            expr: IrExpr::Var("main".to_string()),
        }],
    };
    assert!(crate::ir::run_main(&ir).is_err());
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
                assert!(matches!(arms[0].pat, Pattern::Wildcard));
            }
            _ => panic!("expected case"),
        },
        _ => panic!("expected binding"),
    }

    match &module.items[1] {
        Item::Binding(b) => match &b.expr {
            Expr::Case { arms, .. } => {
                assert_eq!(arms.len(), 2);
                assert!(matches!(arms[0].pat, Pattern::Literal(Expr::Bool(true))));
                assert!(matches!(arms[1].pat, Pattern::Literal(Expr::Bool(false))));
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

    assert_eq!(arms.len(), 7);
    assert!(matches!(arms[0].pat, Pattern::Literal(Expr::Unit)));
    assert!(matches!(arms[1].pat, Pattern::Tuple(_)));
    assert!(matches!(arms[2].pat, Pattern::List(_)));
    assert!(matches!(arms[3].pat, Pattern::Record(_)));
    assert!(matches!(arms[4].pat, Pattern::Constructor { .. }));
    assert!(matches!(arms[5].pat, Pattern::Wildcard));
    assert!(arms[5].guard.is_some());
    assert!(matches!(arms[6].pat, Pattern::Or(_, _)));
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
            ty: Type::Integer,
            ..
        }
    ));

    let Item::Binding(b1) = &module.items[1] else {
        panic!("expected binding");
    };
    assert!(matches!(
        b1.expr,
        Expr::Annot {
            ty: Type::Float64,
            ..
        }
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
            ty: Type::Integer,
            ..
        }
    ));
}

#[test]
fn parser_type_exprs() {
    let src = std::fs::read_to_string("tests/parser_type_expr.ks").unwrap();
    let module = crate::parser::parse_module(&src).unwrap();
    assert_eq!(module.items.len(), 5);

    use crate::ast::{Expr, Item, Type};

    let Item::Binding(b0) = &module.items[0] else {
        panic!("expected binding");
    };
    assert!(matches!(
        b0.expr,
        Expr::Annot {
            ty: Type::List(_),
            ..
        }
    ));

    let Item::Binding(b1) = &module.items[1] else {
        panic!("expected binding");
    };
    assert!(matches!(
        b1.expr,
        Expr::Annot {
            ty: Type::Tuple(_),
            ..
        }
    ));

    let Item::Binding(b2) = &module.items[2] else {
        panic!("expected binding");
    };
    assert!(matches!(
        b2.expr,
        Expr::Annot {
            ty: Type::Record(_),
            ..
        }
    ));

    let Item::Binding(b3) = &module.items[3] else {
        panic!("expected binding");
    };
    assert!(matches!(
        b3.expr,
        Expr::Annot {
            ty: Type::App { .. },
            ..
        }
    ));

    let Item::Binding(b4) = &module.items[4] else {
        panic!("expected binding");
    };
    assert!(matches!(
        b4.expr,
        Expr::Annot {
            ty: Type::Func(_, _),
            ..
        }
    ));
}

#[test]
fn parser_type_holes() {
    let src = std::fs::read_to_string("tests/parser_type_holes.ks").unwrap();
    let module = crate::parser::parse_module(&src).unwrap();
    assert_eq!(module.items.len(), 4);

    use crate::ast::{Expr, Item, Type};

    let Item::Binding(b0) = &module.items[0] else {
        panic!("expected binding");
    };
    let Expr::Annot { ty, .. } = &b0.expr else {
        panic!("expected annotation");
    };
    assert_eq!(ty, &Type::Hole(None));

    let Item::Binding(b1) = &module.items[1] else {
        panic!("expected binding");
    };
    let Expr::Annot { ty, .. } = &b1.expr else {
        panic!("expected annotation");
    };
    assert_eq!(ty, &Type::Hole(Some("t".to_string())));

    let Item::Binding(b2) = &module.items[2] else {
        panic!("expected binding");
    };
    let Expr::Annot { ty, .. } = &b2.expr else {
        panic!("expected annotation");
    };
    assert_eq!(ty, &Type::List(Box::new(Type::Hole(None))));

    let Item::Binding(b3) = &module.items[3] else {
        panic!("expected binding");
    };
    let Expr::Annot { ty, .. } = &b3.expr else {
        panic!("expected annotation");
    };
    assert_eq!(
        ty,
        &Type::Tuple(vec![Type::Hole(Some("a".to_string())), Type::Hole(None)])
    );
}

#[test]
fn typecheck_expands_type_aliases() {
    let src = std::fs::read_to_string("tests/type_alias_expand.ks").unwrap();
    let module = crate::parser::parse_module(&src).unwrap();
    let tm = crate::types::typecheck(module).unwrap();

    use crate::ast::{Expr, Item, Pattern, Type};

    let find_binding = |name: &str| -> &crate::ast::Binding {
        tm.module
            .items
            .iter()
            .find_map(|it| match it {
                Item::Binding(b) if matches!(&b.pat, Pattern::Var(n) if n == name) => Some(b),
                _ => None,
            })
            .unwrap()
    };

    let b0 = find_binding("x");
    let Expr::Annot { ty, .. } = &b0.expr else {
        panic!("expected annotation");
    };
    assert_eq!(ty, &Type::List(Box::new(Type::Char)));

    let b1 = find_binding("z");
    let Expr::Annot { ty, .. } = &b1.expr else {
        panic!("expected annotation");
    };
    assert_eq!(ty, &Type::Tuple(vec![Type::Integer, Type::Bool]));

    use crate::types::{Scheme, Ty};
    assert_eq!(
        tm.inferred.get("x").unwrap(),
        &Scheme::mono(Ty::List(Box::new(Ty::Con("Char".to_string()))))
    );
    assert_eq!(
        tm.inferred.get("z").unwrap(),
        &Scheme::mono(Ty::Tuple(vec![
            Ty::Con("Integer".to_string()),
            Ty::Con("Bool".to_string())
        ]))
    );
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
