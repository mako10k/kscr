use kscr::ast::{DoStmt, ExprKind, Item, PatternKind};

#[test]
fn parser_do_braces_semicolons() {
    let module = kscr::parser::parse_module("x = do { y <- IO 1; IO y }\n").unwrap();

    let Item::Binding(b) = &module.items[0] else {
        panic!("expected binding");
    };

    let ExprKind::Do(stmts) = &b.expr.kind else {
        panic!("expected do");
    };

    assert_eq!(stmts.len(), 2);
    let DoStmt::Bind { pat, .. } = &stmts[0] else {
        panic!("expected bind");
    };
    assert!(matches!(&pat.kind, PatternKind::Var(s) if s == "y"));
    assert!(matches!(stmts[1], DoStmt::Expr(_)));
}

#[test]
fn parser_let_inline_semicolons() {
    let module = kscr::parser::parse_module("x = let a = 1; b = 2 in a\n").unwrap();

    let Item::Binding(b) = &module.items[0] else {
        panic!("expected binding");
    };

    let ExprKind::Let { bindings, body } = &b.expr.kind else {
        panic!("expected let");
    };

    assert_eq!(bindings.len(), 2);
    assert!(matches!(&bindings[0].pat.kind, PatternKind::Var(s) if s == "a"));
    assert!(matches!(&bindings[1].pat.kind, PatternKind::Var(s) if s == "b"));
    assert!(matches!(&body.as_ref().kind, ExprKind::Var(s) if s == "a"));
}

#[test]
fn parser_eq_rhs_indent_block_allows_multiline_let_in() {
    let src = "x =\n  let step = 2 in\n  step\n";
    let module = kscr::parser::parse_module(src).unwrap();

    let Item::Binding(b) = &module.items[0] else {
        panic!("expected binding");
    };

    let ExprKind::Let { .. } = &b.expr.kind else {
        panic!("expected let");
    };
}

#[test]
fn parser_if_then_else_allow_newline_and_indent() {
    let src = "x = if True then\n  1\nelse\n  2\n";
    let module = kscr::parser::parse_module(src).unwrap();

    let Item::Binding(b) = &module.items[0] else {
        panic!("expected binding");
    };

    let ExprKind::If { .. } = &b.expr.kind else {
        panic!("expected if");
    };
}

#[test]
fn parser_where_braces_semicolons() {
    let module = kscr::parser::parse_module("x = a where { a = 1; b = 2 }\n").unwrap();

    let Item::Binding(b) = &module.items[0] else {
        panic!("expected binding");
    };

    let ExprKind::Where { bindings, .. } = &b.expr.kind else {
        panic!("expected where");
    };

    assert_eq!(bindings.len(), 2);
    assert!(matches!(&bindings[0].pat.kind, PatternKind::Var(s) if s == "a"));
    assert!(matches!(&bindings[1].pat.kind, PatternKind::Var(s) if s == "b"));
}

#[test]
fn parser_case_arrow_allow_newline_and_indent() {
    let src = "f = \\xs -> case xs of\n  [] ->\n    1\n  _ -> 2\n";
    let module = kscr::parser::parse_module(src).unwrap();

    let Item::Binding(b) = &module.items[0] else {
        panic!("expected binding");
    };

    let ExprKind::Lambda { body, .. } = &b.expr.kind else {
        panic!("expected lambda");
    };

    let ExprKind::Case { arms, .. } = &body.kind else {
        panic!("expected case");
    };

    assert_eq!(arms.len(), 2);
    // First arm should have parsed with newline after ->
    assert!(matches!(&arms[0].body.kind, ExprKind::Integer(_)));
    // Second arm should work as before (no newline)
    assert!(matches!(&arms[1].body.kind, ExprKind::Integer(_)));
}

#[test]
fn parser_lambda_arrow_allow_newline() {
    let src = "f = \\x ->\n  x + 1\n";
    let module = kscr::parser::parse_module(src).unwrap();

    let Item::Binding(b) = &module.items[0] else {
        panic!("expected binding");
    };

    let ExprKind::Lambda { params, body } = &b.expr.kind else {
        panic!("expected lambda, got {:?}", b.expr.kind);
    };

    assert_eq!(params.len(), 1);
    assert_eq!(params[0], "x");

    // Body should be an application (x + 1 is desugared to Apply)
    assert!(matches!(&body.kind, ExprKind::Apply { .. }));
}

#[test]
fn parser_lambda_arrow_allow_newline_with_indent_block() {
    let src = "f = \\x ->\n  let y = x + 1 in\n  y * 2\n";
    let module = kscr::parser::parse_module(src).unwrap();

    let Item::Binding(b) = &module.items[0] else {
        panic!("expected binding");
    };

    let ExprKind::Lambda { params, body } = &b.expr.kind else {
        panic!("expected lambda");
    };

    assert_eq!(params.len(), 1);
    assert_eq!(params[0], "x");

    // Body should be a let expression
    assert!(matches!(&body.kind, ExprKind::Let { .. }));
}

#[test]
fn parser_lambda_multiple_params_newline() {
    let src = "f = \\x y z ->\n  x + y + z\n";
    let module = kscr::parser::parse_module(src).unwrap();

    let Item::Binding(b) = &module.items[0] else {
        panic!("expected binding");
    };

    let ExprKind::Lambda { params, body } = &b.expr.kind else {
        panic!("expected lambda");
    };

    assert_eq!(params.len(), 3);
    assert_eq!(params[0], "x");
    assert_eq!(params[1], "y");
    assert_eq!(params[2], "z");

    // Body should be an application
    assert!(matches!(&body.kind, ExprKind::Apply { .. }));
}

#[test]
fn parser_lambda_nested_newlines() {
    let src = "f = \\x ->\n  \\y ->\n    x + y\n";
    let module = kscr::parser::parse_module(src).unwrap();

    let Item::Binding(b) = &module.items[0] else {
        panic!("expected binding");
    };

    let ExprKind::Lambda { params, body } = &b.expr.kind else {
        panic!("expected lambda");
    };

    assert_eq!(params.len(), 1);
    assert_eq!(params[0], "x");

    // Body should be another lambda
    let ExprKind::Lambda {
        params: inner_params,
        body: inner_body,
    } = &body.kind
    else {
        panic!("expected nested lambda");
    };

    assert_eq!(inner_params.len(), 1);
    assert_eq!(inner_params[0], "y");

    // Inner body should be an application
    assert!(matches!(&inner_body.kind, ExprKind::Apply { .. }));
}

#[test]
fn parser_lambda_inline_still_works() {
    // Ensure that inline lambdas (without newlines) still work
    let src = "f = \\x -> x + 1\n";
    let module = kscr::parser::parse_module(src).unwrap();

    let Item::Binding(b) = &module.items[0] else {
        panic!("expected binding");
    };

    let ExprKind::Lambda { params, body } = &b.expr.kind else {
        panic!("expected lambda");
    };

    assert_eq!(params.len(), 1);
    assert_eq!(params[0], "x");

    assert!(matches!(&body.kind, ExprKind::Apply { .. }));
}
