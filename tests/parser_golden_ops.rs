use kscr::ast::{ExprKind, Item};

#[test]
fn parser_ctor_exprs() {
    let src = std::fs::read_to_string("tests/parser_ctor_expr.ks").unwrap();
    let module = kscr::parser::parse_module(&src).unwrap();
    assert_eq!(module.items.len(), 2);

    let Item::Binding(b0) = &module.items[0] else {
        panic!("expected binding");
    };
        assert!(matches!(
            &b0.expr.kind,
            ExprKind::Apply { func, args }
                if matches!(&func.as_ref().kind, ExprKind::Ctor(kscr::ast::ResolvedName::Unresolved(s)) if s == "Just") && args.len() == 1
        ));

    let Item::Binding(b1) = &module.items[1] else {
        panic!("expected binding");
    };
        assert!(matches!(&b1.expr.kind, ExprKind::Ctor(kscr::ast::ResolvedName::Unresolved(s)) if s == "Nothing"));
}

#[test]
fn parser_infix_backticks() {
    let src = std::fs::read_to_string("tests/parser_infix.ks").unwrap();
    let module = kscr::parser::parse_module(&src).unwrap();
    assert_eq!(module.items.len(), 2);

    let Item::Binding(b0) = &module.items[0] else {
        panic!("expected binding");
    };
    assert!(matches!(
        &b0.expr.kind,
        ExprKind::Apply { func, args }
            if matches!(&func.as_ref().kind, ExprKind::Var(s) if s == "f") && args.len() == 2
    ));

    let Item::Binding(b1) = &module.items[1] else {
        panic!("expected binding");
    };

    // left associative: (a `f` b) `g` c
    let ExprKind::Apply { func, args } = &b1.expr.kind else {
        panic!("expected apply");
    };
    assert!(matches!(&func.as_ref().kind, ExprKind::Var(s) if s == "g"));
    assert_eq!(args.len(), 2);
    assert!(matches!(&args[1].kind, ExprKind::Var(s) if s == "c"));
    assert!(matches!(&args[0].kind, ExprKind::Apply { .. }));
}

#[test]
fn parser_symbol_ops() {
    let src = std::fs::read_to_string("tests/parser_ops.ks").unwrap();
    let module = kscr::parser::parse_module(&src).unwrap();
    assert_eq!(module.items.len(), 2);

    let Item::Binding(b0) = &module.items[0] else {
        panic!("expected binding");
    };
    // 1 + (2 * 3)
    let ExprKind::Apply { func, args } = &b0.expr.kind else {
        panic!("expected apply");
    };
    assert!(matches!(&func.as_ref().kind, ExprKind::Var(s) if s == "+"));
    assert_eq!(args.len(), 2);
    assert!(matches!(&args[0].kind, ExprKind::Integer(s) if s == "1"));
    assert!(matches!(&args[1].kind, ExprKind::Apply { .. }));

    let Item::Binding(b1) = &module.items[1] else {
        panic!("expected binding");
    };
    // (10 / 2) - 1
    let ExprKind::Apply { func, args } = &b1.expr.kind else {
        panic!("expected apply");
    };
    assert!(matches!(&func.as_ref().kind, ExprKind::Var(s) if s == "-"));
    assert_eq!(args.len(), 2);
    assert!(matches!(&args[1].kind, ExprKind::Integer(s) if s == "1"));
    assert!(matches!(&args[0].kind, ExprKind::Apply { .. }));
}

#[test]
fn parser_cmp_and_logic_ops() {
    let src = std::fs::read_to_string("tests/parser_cmp_logic.ks").unwrap();
    let module = kscr::parser::parse_module(&src).unwrap();
    assert_eq!(module.items.len(), 3);

    let Item::Binding(b0) = &module.items[0] else {
        panic!("expected binding");
    };
    // (1 + (2 * 3)) == 7
    let ExprKind::Apply { func, args } = &b0.expr.kind else {
        panic!("expected apply");
    };
    assert!(matches!(&func.as_ref().kind, ExprKind::Var(s) if s == "=="));
    assert_eq!(args.len(), 2);

    let Item::Binding(b1) = &module.items[1] else {
        panic!("expected binding");
    };
    // (True && False) || True
    let ExprKind::Apply { func, args } = &b1.expr.kind else {
        panic!("expected apply");
    };
    assert!(matches!(&func.as_ref().kind, ExprKind::Var(s) if s == "||"));
    assert_eq!(args.len(), 2);
    assert!(matches!(&args[1].kind, ExprKind::Bool(true)));

    let Item::Binding(b2) = &module.items[2] else {
        panic!("expected binding");
    };
    // (1 < 2) && (2 <= 3)
    let ExprKind::Apply { func, args } = &b2.expr.kind else {
        panic!("expected apply");
    };
    assert!(matches!(&func.as_ref().kind, ExprKind::Var(s) if s == "&&"));
    assert_eq!(args.len(), 2);
}
