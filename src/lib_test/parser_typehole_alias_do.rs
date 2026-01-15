pub(crate) fn parser_type_holes() {
    let src = std::fs::read_to_string("tests/parser_type_holes.ks").unwrap();
    let module = crate::parser::parse_module(&src).unwrap();
    assert_eq!(module.items.len(), 4);

    use crate::ast::{ExprKind, Item, Type};

    let Item::Binding(b0) = &module.items[0] else {
        panic!("expected binding");
    };
    let ExprKind::Annot { ty, .. } = &b0.expr.kind else {
        panic!("expected annotation");
    };
    assert_eq!(&ty.ty, &Type::Hole(None));

    let Item::Binding(b1) = &module.items[1] else {
        panic!("expected binding");
    };
    let ExprKind::Annot { ty, .. } = &b1.expr.kind else {
        panic!("expected annotation");
    };
    assert_eq!(&ty.ty, &Type::Hole(Some("t".to_string())));

    let Item::Binding(b2) = &module.items[2] else {
        panic!("expected binding");
    };
    let ExprKind::Annot { ty, .. } = &b2.expr.kind else {
        panic!("expected annotation");
    };
    assert_eq!(&ty.ty, &Type::List(Box::new(Type::Hole(None))));

    let Item::Binding(b3) = &module.items[3] else {
        panic!("expected binding");
    };
    let ExprKind::Annot { ty, .. } = &b3.expr.kind else {
        panic!("expected annotation");
    };
    assert_eq!(
        &ty.ty,
        &Type::Tuple(vec![Type::Hole(Some("a".to_string())), Type::Hole(None)])
    );
}

pub(crate) fn typecheck_expands_type_aliases() {
    let src = std::fs::read_to_string("tests/type_alias_expand.ks").unwrap();
    let module = crate::parser::parse_module(&src).unwrap();
    let tm = crate::types::typecheck(module).unwrap();

    use crate::ast::{ExprKind, Item, PatternKind, Type};

    let find_binding = |name: &str| -> &crate::ast::Binding {
        tm.module
            .items
            .iter()
            .find_map(|it| match it {
                Item::Binding(b) if matches!(&b.pat.kind, PatternKind::Var(n) if n == name) => {
                    Some(b)
                }
                _ => None,
            })
            .unwrap()
    };

    let b0 = find_binding("x");
    let ExprKind::Annot { ty, .. } = &b0.expr.kind else {
        panic!("expected annotation");
    };
    assert_eq!(&ty.ty, &Type::List(Box::new(Type::Char)));

    let b1 = find_binding("z");
    let ExprKind::Annot { ty, .. } = &b1.expr.kind else {
        panic!("expected annotation");
    };
    assert_eq!(&ty.ty, &Type::Tuple(vec![Type::Integer, Type::Bool]));

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

pub(crate) fn parser_do_blocks() {
    let src = std::fs::read_to_string("tests/parser_do.ks").unwrap();
    let module = crate::parser::parse_module(&src).unwrap();
    assert_eq!(module.items.len(), 1);

    use crate::ast::{DoStmt, ExprKind, Item, PatternKind};

    let Item::Binding(b) = &module.items[0] else {
        panic!("expected binding");
    };
    assert!(matches!(&b.pat.kind, PatternKind::Var(s) if s == "main"));

    let ExprKind::Do(stmts) = &b.expr.kind else {
        panic!("expected do");
    };

    assert_eq!(stmts.len(), 3);
    assert!(matches!(stmts[0], DoStmt::Bind { .. }));
    assert!(matches!(stmts[1], DoStmt::Bind { .. }));
    assert!(matches!(stmts[2], DoStmt::Expr(_)));
}
