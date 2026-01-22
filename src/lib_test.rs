mod parser_typehole_alias_do;
mod typeclass_phase3;

#[test]
fn scaffold_parser_accepts_binding() {
    let m = crate::parser::parse_module("x = 1").unwrap();
    assert_eq!(m.items.len(), 1);
}

#[test]
fn typecheck_rejects_non_exhaustive_case_on_integer() {
    let src = "module Main where\n  a x = case x of\n    1 -> \"1\"\n  main = IO ()\n";
    let ast = crate::parser::parse_module(src).unwrap();
    let e = crate::types::typecheck(ast).unwrap_err();
    assert!(format!("{e}").contains("non-exhaustive case"));
}

#[test]
fn typecheck_rejects_non_exhaustive_fun_clauses_for_adt() {
    let src = "module Main where\n  data OneTwoThree = One | Two | Three\n  a One = 1\n  a Two = 2\n  main = IO ()\n";
    let ast = crate::parser::parse_module(src).unwrap();
    let e = crate::types::typecheck(ast).unwrap_err();
    assert!(format!("{e}").contains("missing constructors"));
}

#[test]
fn ksif_default_output_and_import_search_path_smoke() {
    use std::path::PathBuf;

    // Emit `target/ksif/ksif_A.ksif`, then typecheck B with KSIF enabled.
    // Avoid spawning `cargo run` from tests (fragile in CI / sandbox).
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let a = repo.join("tests").join("ksif_A.ks");
    let b = repo.join("tests").join("ksif_B.ks");

    // 1) Typecheck A and write its exported schemes as KSIF under target/ksif.
    let tm_a = crate::types::typecheck_file(&a).expect("typecheck A");
    let module_name = tm_a
        .module
        .name
        .clone()
        .unwrap_or_else(|| "Main".to_string());
    let exported = crate::cli_impl::filter_inferred_by_exports(&tm_a.module, tm_a.inferred.clone());
    let ksif = crate::kir1::KsifModule {
        module_name,
        values: exported,
    };
    let bytes = crate::kir1::encode_ksif_module(&ksif);
    let ksif_dir = repo.join("target").join("ksif");
    std::fs::create_dir_all(&ksif_dir).expect("create target/ksif");
    let ksif_path = ksif_dir.join("ksif_A.ksif");
    std::fs::write(&ksif_path, bytes).expect("write ksif");

    assert!(ksif_path.is_file(), "missing ksif: {}", ksif_path.display());

    // 2) Typecheck B with KSIF enabled (succeeds if search path works).
    // Note: KSIF is default-on; keep this explicit to avoid regressions if defaults change.
    std::env::set_var("KSCR_USE_KSIF", "1");
    let _tm_b = crate::types::typecheck_file(&b).expect("typecheck B with ksif");

    // 3) Opt-out should still work.
    std::env::set_var("KSCR_USE_KSIF", "0");
    let _tm_b_no = crate::types::typecheck_file(&b).expect("typecheck B without ksif");
}

#[test]
fn parser_module_basic() {
    let src = std::fs::read_to_string("tests/module_basic.ks").unwrap();
    let m = crate::parser::parse_module(&src).unwrap();
    assert_eq!(m.name.as_deref(), Some("Main"));
    assert_eq!(m.items.len(), 2);
}

#[test]
fn parser_module_hierarchical_name() {
    let m = crate::parser::parse_module("module A.B where\n  x = 1\n").unwrap();
    assert_eq!(m.name.as_deref(), Some("A.B"));
}

#[test]
fn parser_import_hierarchical_name() {
    let m = crate::parser::parse_module("import A.B\nx = 1\n").unwrap();
    match &m.items[0] {
        crate::ast::Item::Import(id) => assert_eq!(id.module, "A.B"),
        _ => panic!("expected import"),
    }
}

#[test]
fn parser_error_has_span() {
    let src = "x = )";
    let e = crate::parser::parse_module(src).unwrap_err();
    assert_eq!(e.span(), Some(crate::lexer::Span { start: 4, end: 5 }));
}

#[test]
fn type_error_has_span() {
    let src = "module Main where\n  x = zzz\n  main = IO ()\n";
    let ast = crate::parser::parse_module(src).unwrap();
    let e = crate::types::typecheck(ast).unwrap_err();
    let s = e.span().expect("type error should have a span");
    // The exact span may widen as we attach higher-level context; it must at least include `zzz`.
    assert!(s.start <= 24, "span.start too large: {s:?}");
    assert!(s.end >= 27, "span.end too small: {s:?}");
}

#[test]
fn parser_class_and_instance_basic() {
    let src = r#"
class C a where
    f :: a -> a

instance C Integer where
    f x = x
"#;

    let m = crate::parser::parse_module(src).unwrap();
    assert!(m
        .items
        .iter()
        .any(|it| matches!(it, crate::ast::Item::ClassDecl(_))));
    assert!(m
        .items
        .iter()
        .any(|it| matches!(it, crate::ast::Item::InstanceDecl(_))));
}

#[test]
fn parser_class_superclass_context_parens() {
    let src = r#"
class (Eq a) => Ord a where
"#;

    let m = crate::parser::parse_module(src).unwrap();
    let cls = m
        .items
        .iter()
        .find_map(|it| match it {
            crate::ast::Item::ClassDecl(c) => Some(c),
            _ => None,
        })
        .expect("expected class decl");

    assert_eq!(cls.name, "Ord");
    assert_eq!(cls.param, "a");
    assert_eq!(
        cls.supers,
        vec![crate::ast::Predicate::Eq(crate::ast::Type::Var(
            "a".to_string()
        ))]
    );
}

#[test]
fn parser_class_superclass_context_single_pred() {
    let src = r#"
class Eq a => Ord a where
"#;

    let m = crate::parser::parse_module(src).unwrap();
    let cls = m
        .items
        .iter()
        .find_map(|it| match it {
            crate::ast::Item::ClassDecl(c) => Some(c),
            _ => None,
        })
        .expect("expected class decl");

    assert_eq!(cls.name, "Ord");
    assert_eq!(cls.param, "a");
    assert_eq!(
        cls.supers,
        vec![crate::ast::Predicate::Eq(crate::ast::Type::Var(
            "a".to_string()
        ))]
    );
}

#[test]
fn parser_instance_context_parens() {
    let src = r#"
class C a where
    f :: a -> a

instance (C a) => C (Maybe a) where
    f x = x
"#;

    let m = crate::parser::parse_module(src).unwrap();
    let inst = m
        .items
        .iter()
        .find_map(|it| match it {
            crate::ast::Item::InstanceDecl(i) => Some(i),
            _ => None,
        })
        .expect("expected instance decl");

    assert_eq!(inst.class.name, "C");
    assert_eq!(inst.preds.len(), 1);
}

#[test]
fn parser_instance_context_single_pred() {
    let src = r#"
class C a where
    f :: a -> a

instance C a => C (Maybe a) where
    f x = x
"#;

    let m = crate::parser::parse_module(src).unwrap();
    let inst = m
        .items
        .iter()
        .find_map(|it| match it {
            crate::ast::Item::InstanceDecl(i) => Some(i),
            _ => None,
        })
        .expect("expected instance decl");

    assert_eq!(inst.class.name, "C");
    assert_eq!(inst.preds.len(), 1);
}

#[test]
fn typecheck_rejects_bad_superclass_predicate_shape() {
    let src = r#"
class Inc a where
    inc :: a -> a

class (Inc Integer) => C a where
    c :: a -> a
"#;

    let m = crate::parser::parse_module(src).unwrap();
    let err = crate::types::typecheck(m).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("superclass") && msg.contains("MVP: superclass constraints"),
        "unexpected error: {msg}"
    );
}

#[test]
fn parser_list_range_sugar_desugars_to_enum_from_to() {
    let m = crate::parser::parse_module("xs = [1..3]\n").unwrap();
    let crate::ast::Item::Binding(b) = &m.items[0] else {
        panic!("expected binding");
    };

    match &b.expr.kind {
        crate::ast::ExprKind::Apply { func, args } => {
            assert!(matches!(&func.kind, crate::ast::ExprKind::Var(s) if s == "enumFromTo"));
            assert_eq!(args.len(), 2);
        }
        other => panic!("expected apply, got {other:?}"),
    }
}

#[test]
fn parser_list_range_sugar_desugars_to_enum_from() {
    let m = crate::parser::parse_module("xs = [1..]\n").unwrap();
    let crate::ast::Item::Binding(b) = &m.items[0] else {
        panic!("expected binding");
    };

    match &b.expr.kind {
        crate::ast::ExprKind::Apply { func, args } => {
            assert!(matches!(&func.kind, crate::ast::ExprKind::Var(s) if s == "enumFrom"));
            assert_eq!(args.len(), 1);
        }
        other => panic!("expected apply, got {other:?}"),
    }
}

#[test]
fn parser_list_range_sugar_desugars_to_enum_from_then_to() {
    let m = crate::parser::parse_module("xs = [1,3..10]\n").unwrap();
    let crate::ast::Item::Binding(b) = &m.items[0] else {
        panic!("expected binding");
    };

    match &b.expr.kind {
        crate::ast::ExprKind::Apply { func, args } => {
            assert!(matches!(&func.kind, crate::ast::ExprKind::Var(s) if s == "enumFromThenTo"));
            assert_eq!(args.len(), 3);
        }
        other => panic!("expected apply, got {other:?}"),
    }
}

#[test]
fn parser_list_range_sugar_desugars_to_enum_from_then() {
    let m = crate::parser::parse_module("xs = [1,3..]\n").unwrap();
    let crate::ast::Item::Binding(b) = &m.items[0] else {
        panic!("expected binding");
    };

    match &b.expr.kind {
        crate::ast::ExprKind::Apply { func, args } => {
            assert!(matches!(&func.kind, crate::ast::ExprKind::Var(s) if s == "enumFromThen"));
            assert_eq!(args.len(), 2);
        }
        other => panic!("expected apply, got {other:?}"),
    }
}

#[test]
fn parser_binding_patterns() {
    let src = std::fs::read_to_string("tests/parser_binding_patterns.ks").unwrap();
    let m = crate::parser::parse_module(&src).unwrap();
    assert_eq!(m.items.len(), 3);

    use crate::ast::{Item, PatternKind};

    match &m.items[1] {
        Item::Binding(b) => assert!(matches!(&b.pat.kind, PatternKind::Tuple(_))),
        _ => panic!("expected binding"),
    }

    match &m.items[2] {
        Item::Binding(b) => assert!(matches!(&b.pat.kind, PatternKind::Wildcard)),
        _ => panic!("expected binding"),
    }
}

#[test]
fn parser_fun_clause_desugar_single_clause_multi_args() {
    let m = crate::parser::parse_module("f x y = x\n").unwrap();
    assert_eq!(m.items.len(), 1);

    use crate::ast::{ExprKind, Item, PatternKind};

    let Item::Binding(b) = &m.items[0] else {
        panic!("expected binding");
    };
    assert!(matches!(&b.pat.kind, PatternKind::Var(name) if name == "f"));

    let ExprKind::Lambda { params, body } = &b.expr.kind else {
        panic!("expected lambda");
    };
    assert_eq!(params.len(), 2);

    let ExprKind::Case { arms, .. } = &body.as_ref().kind else {
        panic!("expected case");
    };
    assert_eq!(arms.len(), 1);

    assert!(matches!(
        &arms[0].pat.kind,
        PatternKind::Tuple(ps)
            if matches!(
                &ps[..],
                [
                    crate::ast::Pattern {
                        kind: PatternKind::Var(x),
                        ..
                    },
                    crate::ast::Pattern {
                        kind: PatternKind::Var(y),
                        ..
                    }
                ] if x == "x" && y == "y"
            )
    ));
    assert!(matches!(&arms[0].body.kind, ExprKind::Var(s) if s == "x"));
}

#[test]
fn parser_binding_operator_name_parens() {
    let m = crate::parser::parse_module("(++) = concat\n").unwrap();
    assert_eq!(m.items.len(), 1);

    use crate::ast::{ExprKind, Item, PatternKind};

    let Item::Binding(b) = &m.items[0] else {
        panic!("expected binding");
    };
    assert!(matches!(&b.pat.kind, PatternKind::Var(name) if name == "++"));
    assert!(matches!(&b.expr.kind, ExprKind::Var(s) if s == "concat"));
}

#[test]
fn parser_binding_operator_name_dot_parens() {
    let m = crate::parser::parse_module("(.) = f\n").unwrap();
    assert_eq!(m.items.len(), 1);

    use crate::ast::{ExprKind, Item, PatternKind};

    let Item::Binding(b) = &m.items[0] else {
        panic!("expected binding");
    };
    assert!(matches!(&b.pat.kind, PatternKind::Var(name) if name == "."));
    assert!(matches!(&b.expr.kind, ExprKind::Var(s) if s == "f"));
}

#[test]
fn parser_fun_clause_operator_name_parens() {
    let m = crate::parser::parse_module("(++) a b = concat a b\n").unwrap();
    assert_eq!(m.items.len(), 1);

    use crate::ast::{ExprKind, Item, PatternKind};

    let Item::Binding(b) = &m.items[0] else {
        panic!("expected binding");
    };
    assert!(matches!(&b.pat.kind, PatternKind::Var(name) if name == "++"));

    let ExprKind::Lambda { params, body } = &b.expr.kind else {
        panic!("expected lambda");
    };
    assert_eq!(params.len(), 2);

    let ExprKind::Case { arms, .. } = &body.as_ref().kind else {
        panic!("expected case");
    };
    assert_eq!(arms.len(), 1);

    assert!(matches!(
        &arms[0].pat.kind,
        PatternKind::Tuple(ps)
            if matches!(
                &ps[..],
                [
                    crate::ast::Pattern {
                        kind: PatternKind::Var(a),
                        ..
                    },
                    crate::ast::Pattern {
                        kind: PatternKind::Var(b),
                        ..
                    }
                ] if a == "a" && b == "b"
            )
    ));

    let ExprKind::Apply { func, args } = &arms[0].body.kind else {
        panic!("expected apply");
    };
    assert!(matches!(&func.as_ref().kind, ExprKind::Var(s) if s == "concat"));
    assert!(matches!(
        &args[..],
        [
            crate::ast::Expr {
                kind: ExprKind::Var(a),
                ..
            },
            crate::ast::Expr {
                kind: ExprKind::Var(b),
                ..
            }
        ] if a == "a" && b == "b"
    ));
}

#[test]
fn parser_case_allows_inline_of_arms_with_semicolons() {
    let src = "x = case a of Just y -> y; _ -> 0\n";
    let m = crate::parser::parse_module(src).unwrap();
    assert_eq!(m.items.len(), 1);
}

#[test]
fn parser_where_allows_inline_bindings_with_semicolons() {
    let src = "x = 1 where y = 2; z = 3\n";
    let m = crate::parser::parse_module(src).unwrap();
    assert_eq!(m.items.len(), 1);
}

#[test]
fn parser_binding_operator_name_arbitrary() {
    let m = crate::parser::parse_module("(<+>) = f\n").unwrap();
    assert_eq!(m.items.len(), 1);

    use crate::ast::{ExprKind, Item, PatternKind};

    let Item::Binding(b) = &m.items[0] else {
        panic!("expected binding");
    };
    assert!(matches!(&b.pat.kind, PatternKind::Var(name) if name == "<+>"));
    assert!(matches!(&b.expr.kind, ExprKind::Var(s) if s == "f"));
}

#[test]
fn parser_fun_clause_operator_name_infix_arbitrary() {
    let m = crate::parser::parse_module("a <+> b = a\n").unwrap();
    assert_eq!(m.items.len(), 1);

    use crate::ast::{ExprKind, Item};
    let Item::Binding(b) = &m.items[0] else {
        panic!("expected binding");
    };

    // Desugars to a lambda + case; just sanity check it parsed.
    assert!(matches!(&b.expr.kind, ExprKind::Lambda { .. }));
}

#[test]
fn parser_rejects_operator_starting_with_colon() {
    let err = crate::parser::parse_module("(:+) a b = a\n").unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("operators starting with ':'"),
        "unexpected error: {msg}"
    );
}

#[test]
fn parser_fun_clause_operator_name_infix() {
    let m = crate::parser::parse_module("a ++ b = concat a b\n").unwrap();
    assert_eq!(m.items.len(), 1);

    use crate::ast::{ExprKind, Item, PatternKind};

    let Item::Binding(b) = &m.items[0] else {
        panic!("expected binding");
    };
    assert!(matches!(&b.pat.kind, PatternKind::Var(name) if name == "++"));

    let ExprKind::Lambda { params, body } = &b.expr.kind else {
        panic!("expected lambda");
    };
    assert_eq!(params.len(), 2);

    let ExprKind::Case { arms, .. } = &body.as_ref().kind else {
        panic!("expected case");
    };
    assert_eq!(arms.len(), 1);

    assert!(matches!(
        &arms[0].pat.kind,
        PatternKind::Tuple(ps)
            if matches!(
                &ps[..],
                [
                    crate::ast::Pattern {
                        kind: PatternKind::Var(a),
                        ..
                    },
                    crate::ast::Pattern {
                        kind: PatternKind::Var(b),
                        ..
                    }
                ] if a == "a" && b == "b"
            )
    ));

    let ExprKind::Apply { func, args } = &arms[0].body.kind else {
        panic!("expected apply");
    };
    assert!(matches!(&func.as_ref().kind, ExprKind::Var(s) if s == "concat"));
    assert!(matches!(
        &args[..],
        [
            crate::ast::Expr {
                kind: ExprKind::Var(a),
                ..
            },
            crate::ast::Expr {
                kind: ExprKind::Var(b),
                ..
            }
        ] if a == "a" && b == "b"
    ));
}

#[test]
fn ir_run_main_fun_clauses_multi_clause_and_multi_args() {
    let src = concat!(
        "f 0 y = y\n",
        "f x 0 = x\n",
        "f _ _ = 9\n",
        "main = case (f 0 5) of\n",
        "  5 -> case (f 7 0) of\n",
        "    7 -> case (f 1 2) of\n",
        "      9 -> IO ()\n",
        "      _ -> throw \"assert failed\"\n",
        "    _ -> throw \"assert failed\"\n",
        "  _ -> throw \"assert failed\"\n",
    );
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_guarded_fun_clause_top_level() {
    let src = concat!(
        "f x | x == 0 = 1\n",
        "f _ = 2\n",
        "main = case (f 0) of\n",
        "  1 -> case (f 3) of\n",
        "    2 -> IO ()\n",
        "    _ -> throw \"assert failed\"\n",
        "  _ -> throw \"assert failed\"\n",
    );
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_fun_clauses_in_let_and_where() {
    let src = concat!(
        "main = let\n",
        "  f 0 = 1\n",
        "  f _ = 2\n",
        "in case (f 0) of\n",
        "  1 -> case (f 9) of\n",
        "    2 -> case (g 0) of\n",
        "      1 -> case (g 9) of\n",
        "        2 -> IO ()\n",
        "        _ -> throw \"assert failed\"\n",
        "      _ -> throw \"assert failed\"\n",
        "    _ -> throw \"assert failed\"\n",
        "  _ -> throw \"assert failed\"\n",
        "where\n",
        "  g 0 = 1\n",
        "  g _ = 2\n",
    );
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn parser_qualified_names_desugar() {
    let m = crate::parser::parse_module("y = A.B.OM.x\n").unwrap();
    let crate::ast::Item::Binding(b) = &m.items[0] else {
        panic!("expected binding");
    };
    assert!(matches!(&b.expr.kind, crate::ast::ExprKind::Var(s) if s == "A.B.OM.x"));

    let m = crate::parser::parse_module("x = 1 :: OM.Integer\n").unwrap();
    let crate::ast::Item::Binding(b) = &m.items[0] else {
        panic!("expected binding");
    };
    let crate::ast::Expr {
        kind: crate::ast::ExprKind::Annot { ty, .. },
        ..
    } = &b.expr
    else {
        panic!("expected annotation");
    };
    assert!(matches!(ty.ty, crate::ast::Type::Integer));
}

#[test]
fn parser_module_import_export() {
    let src = std::fs::read_to_string("tests/module_import_export.ks").unwrap();
    let m = crate::parser::parse_module(&src).unwrap();
    assert_eq!(m.name.as_deref(), Some("Main"));
    assert_eq!(m.items.len(), 5);

    use crate::ast::{ExportSpec, Item};

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
        Item::Export(e) => assert_eq!(
            e.specs,
            vec![
                ExportSpec::Name("x".to_string()),
                ExportSpec::Name("y".to_string())
            ]
        ),
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
    assert_eq!(m.items.len(), 10);

    use crate::ast::{Item, Type};

    assert!(matches!(m.items[0], Item::DataDecl(_)));

    match &m.items[1] {
        Item::TypeAlias(ta) => assert_eq!(ta.ty, Type::List(Box::new(Type::Char))),
        _ => panic!("expected type alias"),
    }

    match &m.items[2] {
        Item::DataDecl(dd) => {
            assert_eq!(dd.name, "Pair");
            assert_eq!(dd.params, vec!["a".to_string(), "b".to_string()]);
            assert_eq!(dd.ctors.len(), 1);
            assert_eq!(dd.ctors[0].name, ":*:");
            assert_eq!(
                dd.ctors[0].args,
                vec![Type::Var("a".to_string()), Type::Var("b".to_string())]
            );
        }
        _ => panic!("expected data decl"),
    }
    assert!(matches!(m.items[3], Item::Binding(_)));
    assert!(matches!(m.items[4], Item::Binding(_)));
    assert!(matches!(m.items[5], Item::Binding(_)));
    assert!(matches!(m.items[6], Item::Binding(_)));
    assert!(matches!(m.items[7], Item::Binding(_)));
    assert!(matches!(m.items[8], Item::Binding(_)));
    assert!(matches!(m.items[9], Item::Binding(_)));
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
    assert!(matches!(
        tokens[5].kind,
        crate::lexer::TokenKind::Char('\n')
    ));
}

#[test]
fn parser_char_literal() {
    let m = crate::parser::parse_module("x = 'a'\n").unwrap();
    use crate::ast::{ExprKind, Item};
    match &m.items[0] {
        Item::Binding(b) => assert!(matches!(b.expr.kind, ExprKind::Char('a'))),
        _ => panic!("expected binding"),
    }
}

#[test]
fn parser_cons_pattern() {
    let m = crate::parser::parse_module("x:xs = ys\n").unwrap();
    use crate::ast::{Item, PatternKind};
    match &m.items[0] {
        Item::Binding(b) => {
            assert!(matches!(&b.pat.kind, PatternKind::Cons(_, _)))
        }
        _ => panic!("expected binding"),
    }
}

#[test]
fn parser_cons_expr() {
    let m = crate::parser::parse_module("xs = 1:[]\n").unwrap();
    use crate::ast::{ExprKind, Item};
    match &m.items[0] {
        Item::Binding(b) => assert!(matches!(&b.expr.kind, ExprKind::Cons { .. })),
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
    use crate::ast::{Item, PatternKind};
    match &m.items[0] {
        Item::Binding(b) => assert!(matches!(&b.pat.kind, PatternKind::As(_, _))),
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
    use crate::ast::{Item, PatternKind};
    match &m.items[0] {
        Item::Binding(b) => assert!(matches!(&b.pat.kind, PatternKind::Hole(None))),
        _ => panic!("expected binding"),
    }
    match &m.items[1] {
        Item::Binding(b) => {
            assert!(matches!(&b.pat.kind, PatternKind::Hole(Some(name)) if name == "x"))
        }
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
    use crate::ast::{Item, PatternKind};
    match &m.items[0] {
        Item::Binding(b) => assert!(matches!(&b.pat.kind, PatternKind::RecordLoose(_, _))),
        _ => panic!("expected binding"),
    }
}

#[test]
fn parser_record_loose_pattern_with_rest() {
    let m = crate::parser::parse_module("{x: a, ...r} = r0\n").unwrap();
    use crate::ast::{Item, PatternKind};
    match &m.items[0] {
        Item::Binding(b) => {
            assert!(matches!(&b.pat.kind, PatternKind::RecordLoose(_, Some(rest)) if rest == "r"))
        }
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
fn typecheck_record_loose_pattern_binds_rest() {
    let m = crate::parser::parse_module("{x: a, ...r} = {x: 1, y: 2}\n").unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    assert_eq!(tm.inferred["a"].to_string(), "Integer");
    assert_eq!(tm.inferred["r"].to_string(), "{y: Integer}");
}

#[test]
fn parser_view_pattern() {
    let m = crate::parser::parse_module("(Just n <- id) = x\n").unwrap();
    use crate::ast::{Item, PatternKind};
    match &m.items[0] {
        Item::Binding(b) => assert!(matches!(&b.pat.kind, PatternKind::View(_, _))),
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
    let src =
        "main = case [x | x <- [1, 2]] of\n  [1, 2] -> IO ()\n  _ -> throw \"assert failed\"\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_list_range_sugar() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("kscr-enum-range-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();

    let main_path = dir.join("Main.ks");
    let src = "module Main where\n  import Prelude\n  main = case [1..3] of\n    1:2:3:[] -> IO ()\n    _ -> throw \"assert failed\"\n";
    std::fs::write(&main_path, src).unwrap();

    let tm = crate::types::typecheck_file(&main_path).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ir_run_main_list_range_sugar_infinite() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("kscr-enum-range-inf-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();

    let main_path = dir.join("Main.ks");
    let src = "module Main where\n  import Prelude\n  main = case [1..] of\n    1:2:3:_ -> IO ()\n    _ -> throw \"assert failed\"\n";
    std::fs::write(&main_path, src).unwrap();

    let tm = crate::types::typecheck_file(&main_path).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ir_run_main_list_range_sugar_step_finite() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("kscr-enum-range-step-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();

    let main_path = dir.join("Main.ks");
    let src = "module Main where\n  import Prelude\n  main = case [1,3..10] of\n    1:3:5:7:9:[] -> IO ()\n    _ -> throw \"assert failed\"\n";
    std::fs::write(&main_path, src).unwrap();

    let tm = crate::types::typecheck_file(&main_path).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ir_run_main_list_range_sugar_step_infinite() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("kscr-enum-range-stepinf-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();

    let main_path = dir.join("Main.ks");
    let src = "module Main where\n  import Prelude\n  main = case [1,3..] of\n    1:3:5:_ -> IO ()\n    _ -> throw \"assert failed\"\n";
    std::fs::write(&main_path, src).unwrap();

    let tm = crate::types::typecheck_file(&main_path).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ir_run_main_plus_operator() {
    let src = "main = case (1 + 2) of\n  3 -> IO ()\n  _ -> throw \"assert failed\"\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_eqeq_operator() {
    let src = "main = case (1 == 1) of\n  True -> IO ()\n  False -> throw \"assert failed\"\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_minus_operator() {
    let src = "main = case (3 - 2) of\n  1 -> IO ()\n  _ -> throw \"assert failed\"\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_mul_operator() {
    let src = "main = case (2 * 3) of\n  6 -> IO ()\n  _ -> throw \"assert failed\"\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_div_operator() {
    let src = "main = case (6 / 2) of\n  3 -> IO ()\n  _ -> throw \"assert failed\"\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_lt_le_operators() {
    let src = "main = case (1 < 2) of\n  True -> case (2 <= 2) of\n    True -> IO ()\n    False -> throw \"assert failed\"\n  False -> throw \"assert failed\"\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_gt_ge_ne_operators() {
    let src = "main = case (2 > 1) of\n  True -> case (2 >= 2) of\n    True -> case (1 /= 2) of\n      True -> IO ()\n      False -> throw \"assert failed\"\n    False -> throw \"assert failed\"\n  False -> throw \"assert failed\"\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_and_or_short_circuit() {
    let src = "main = let\n  bad = error \"boom\"\nin case (False && bad) of\n  False -> case (True || bad) of\n    True -> IO ()\n    False -> throw \"assert failed\"\n  True -> throw \"assert failed\"\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_not_builtin() {
    let src = "main = case (not False) of\n  True -> IO ()\n  False -> throw \"assert failed\"\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_int_bool_to_string() {
    let src = "main = do\n  stdoutWrite (intToString (1 + 2))\n  stdoutWrite (boolToString (1 == 1))\n  IO ()\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_large_integer_ok() {
    let src = "main = case (999999999999999999999999999999 + 1) of\n  1000000000000000000000000000000 -> IO ()\n  _ -> throw \"assert failed\"\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_checked_cast_i32_ok() {
    let src = "main = case (1 :: i32) of\n  1 -> IO ()\n  _ -> throw \"assert failed\"\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_checked_cast_i32_overflow_is_error() {
    let src = "main = case (2147483648 :: i32) of\n  0 -> IO ()\n  _ -> throw \"expected i32 overflow\"\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let err = crate::ir::run_main(&ir).unwrap_err();
    assert!(err.to_string().contains("integer out of range for i32"));
}

#[test]
fn ir_run_main_checked_cast_i32_negative_overflow_is_error() {
    let src = "main = case ((0 - 2147483649) :: i32) of\n  0 -> IO ()\n  _ -> throw \"expected i32 overflow\"\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let err = crate::ir::run_main(&ir).unwrap_err();
    assert!(err.to_string().contains("integer out of range for i32"));
}

#[test]
fn ir_run_main_integer_div_trunc_toward_zero() {
    let src = "main = case (((0 - 7) / 2) == (0 - 3)) of\n  True -> IO ()\n  False -> throw \"assert failed\"\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_integer_div_round_trip_big() {
    let src = "a = 100000000000000000000\nmain = case (((a * a) / a) == a) of\n  True -> IO ()\n  False -> throw \"assert failed\"\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_ffi_add_i32_ok() {
    let src = "main = case (ffiAddI32 1 2) of\n  3 -> IO ()\n  _ -> throw \"assert failed\"\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_ffi_add_i32_arg_out_of_range_is_error() {
    let src = "main = case (ffiAddI32 2147483648 0) of\n  0 -> IO ()\n  _ -> throw \"expected ffiAddI32 out-of-range\"\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let err = crate::ir::run_main(&ir).unwrap_err();
    assert!(err
        .to_string()
        .contains("ffiAddI32: integer out of range for i32"));
}

#[test]
fn ir_run_main_ffi_add_i32_overflow_is_error() {
    let src = "main = case (ffiAddI32 2147483647 1) of\n  0 -> IO ()\n  _ -> throw \"expected ffiAddI32 overflow\"\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let err = crate::ir::run_main(&ir).unwrap_err();
    assert!(err.to_string().contains("ffiAddI32: i32 overflow"));
}

#[test]
fn ir_run_main_ffi_add_f32_overflow_is_error() {
    let src = "main = case (ffiAddF32 1e39 1.0) of\n  0.0 -> IO ()\n  _ -> throw \"expected ffiAddF32 overflow\"\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let err = crate::ir::run_main(&ir).unwrap_err();
    assert!(err.to_string().contains("ffiAddF32"));
}

#[cfg(feature = "unsafe_ffi")]
#[test]
fn ir_run_main_ffi_puts_ok() {
    let src = "main = do\n  ffiPuts \"hello from kscr\\n\"\n  IO ()\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_show_to_string() {
    let src =
        "main = do\n  stdoutWrite (show (1 + 2))\n  stdoutWrite (toString (1 == 1))\n  IO ()\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_user_defined_typeclass_method() {
    let src = r#"
class Inc a where
    inc :: a -> a

instance Inc Integer where
    inc x = x + 1

main = do
    stdoutWrite (show (inc 1))
    IO ()
"#;

    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_user_defined_typeclass_polymorphic_dict_passing() {
    let src = r#"
class Inc a where
    inc :: a -> a

instance Inc Integer where
    inc x = x + 1

f x = inc x

main = do
    stdoutWrite (show (f 1))
    IO ()
"#;

    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_user_defined_typeclass_deferred_dict_higher_order() {
    let src = r#"
class Inc a where
    inc :: a -> a

instance Inc Integer where
    inc x = x + 1

f x = inc x

use h x = h x

g x = use f x

main = do
    stdoutWrite (show (g 1))
    IO ()
"#;

    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_user_defined_typeclass_deferred_dict_let() {
    let src = r#"
class Inc a where
    inc :: a -> a

instance Inc Integer where
    inc x = x + 1

f x = inc x
use h x = h x

g x = let
    h = \y -> use f y
in h x

main = do
    stdoutWrite (show (g 1))
    IO ()
"#;

    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_user_defined_typeclass_deferred_dict_where() {
    let src = r#"
class Inc a where
    inc :: a -> a

instance Inc Integer where
    inc x = x + 1

f x = inc x
use h x = h x

g x = h x where
    h = \y -> use f y

main = do
    stdoutWrite (show (g 1))
    IO ()
"#;

    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_user_defined_typeclass_deferred_dict_let_transitive() {
    let src = r#"
class Inc a where
    inc :: a -> a

instance Inc Integer where
    inc x = x + 1

main = do
    stdoutWrite (show (let
        k = \y -> inc y
        h = \y -> k y
    in h 1))
    IO ()
"#;

    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_user_defined_typeclass_deferred_dict_where_transitive() {
    let src = r#"
class Inc a where
    inc :: a -> a

instance Inc Integer where
    inc x = x + 1

x = h 1 where
    k = \y -> inc y
    h = \y -> k y

main = do
    stdoutWrite (show x)
    IO ()
"#;

    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_user_defined_typeclass_deferred_dict_local_fn_as_value() {
    let src = r#"
class Inc a where
    inc :: a -> a

instance Inc Integer where
    inc x = x + 1

use h x = h x

g x = let
    k = \y -> inc y
in use k x

main = do
    stdoutWrite (show (g 1))
    IO ()
"#;

    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_user_defined_typeclass_deferred_dict_local_fn_as_value_callsite_ground() {
    let src = r#"
class Inc a where
    inc :: a -> a

instance Inc Integer where
    inc x = x + 1

use h x = h x

g = let
    k = \y -> inc y
in use k 1

main = do
    stdoutWrite (show g)
    IO ()
"#;

    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_user_defined_typeclass_multi_constraints_callsite() {
    let src = r#"
class Inc a where
    inc :: a -> a

class Dec a where
    dec :: a -> a

instance Inc Integer where
    inc x = x + 1

instance Dec Integer where
    dec x = x - 1

f x = dec (inc x)

main = do
    stdoutWrite (show (f 1))
    IO ()
"#;

    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_user_defined_typeclass_multi_constraints_deferred_dict_higher_order() {
    let src = r#"
class Inc a where
    inc :: a -> a

class Dec a where
    dec :: a -> a

instance Inc Integer where
    inc x = x + 1

instance Dec Integer where
    dec x = x - 1

f x = dec (inc x)

use h x = h x

g x = use f x

main = do
    stdoutWrite (show (g 1))
    IO ()
"#;

    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_user_defined_typeclass_multi_constraints_local_fn_as_value_callsite_ground() {
    let src = r#"
class Inc a where
    inc :: a -> a

class Dec a where
    dec :: a -> a

instance Inc Integer where
    inc x = x + 1

instance Dec Integer where
    dec x = x - 1

use h x = h x

g = let
    k = \y -> dec (inc y)
in use k 1

main = do
    stdoutWrite (show g)
    IO ()
"#;

    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_user_defined_typeclass_multi_constraints_partial_scope_plus_callsite() {
    let src = r#"
class Inc a where
    inc :: a -> a

class Dec a where
    dec :: a -> a

instance Inc Integer where
    inc x = x + 1

instance Dec Integer where
    dec x = x - 1

f x = dec (inc x)
use h x = h x

-- `g` needs only `Inc`, so only `__dict_Inc` is in scope.
-- Passing `f` as a value to `use` should still resolve the missing `Dec` dict
-- from the callsite ground argument `1`.
g x = let
    y = inc x
in case y of
    z -> use f 1

main = do
    stdoutWrite (show (g 1))
    IO ()
"#;

    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn typecheck_user_defined_typeclass_multi_constraints_missing_instance_fails() {
    let src = r#"
class Inc a where
    inc :: a -> a

class Dec a where
    dec :: a -> a

instance Inc Integer where
    inc x = x + 1

-- NOTE: missing `instance Dec Integer` on purpose.

f x = dec (inc x)

main = do
    stdoutWrite (show (f 1))
    IO ()
"#;

    let m = crate::parser::parse_module(src).unwrap();
    let err = crate::types::typecheck(m).unwrap_err();
    let msg = format!("{err}");

    assert!(
        msg.contains("Dec Integer")
            && (msg.contains("cannot satisfy constraint")
                || msg.contains("no instance found for method call `dec`")
                || msg.contains("no instance found for dictionary argument")),
        "unexpected error: {msg}"
    );
}

#[test]
fn typecheck_user_defined_typeclass_multi_constraints_missing_instance_higher_order_fails() {
    let src = r#"
class Inc a where
    inc :: a -> a

class Dec a where
    dec :: a -> a

instance Inc Integer where
    inc x = x + 1

-- NOTE: missing `instance Dec Integer` on purpose.

f x = dec (inc x)
use h x = h x
g x = use f x

main = do
    stdoutWrite (show (g 1))
    IO ()
"#;

    let m = crate::parser::parse_module(src).unwrap();
    let err = crate::types::typecheck(m).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("Dec Integer")
            && (msg.contains("cannot satisfy constraint")
                || msg.contains("no instance found for method call `dec`")
                || msg.contains("no instance found for dictionary argument")),
        "unexpected error: {msg}"
    );
}

#[test]
fn typecheck_user_defined_typeclass_multi_constraints_missing_instance_local_fn_value_callsite_ground_fails(
) {
    let src = r#"
class Inc a where
    inc :: a -> a

class Dec a where
    dec :: a -> a

instance Inc Integer where
    inc x = x + 1

-- NOTE: missing `instance Dec Integer` on purpose.

use h x = h x

g = let
    k = \y -> dec (inc y)
in use k 1

main = do
    stdoutWrite (show g)
    IO ()
"#;

    let m = crate::parser::parse_module(src).unwrap();
    let err = crate::types::typecheck(m).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("Dec Integer")
            && (msg.contains("cannot satisfy constraint")
                || msg.contains("no instance found for method call `dec`")
                || msg.contains("no instance found for dictionary argument")),
        "unexpected error: {msg}"
    );
}

#[test]
fn ir_run_main_user_defined_typeclass_imports_instance() {
    let tm = crate::types::typecheck_file(std::path::Path::new("tests/typeclass_import_main.ks"))
        .unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_stdlib_classes_smoke() {
    let tm = crate::types::typecheck_file(std::path::Path::new("tests/stdlib_classes_smoke.ks"))
        .unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_rational_smoke() {
    let tm = crate::types::typecheck_file(std::path::Path::new("tests/rational_smoke.ks")).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_p0_import_data_case_do_smoke() {
    let tm = crate::types::typecheck_file(std::path::Path::new("tests/P0/Main.ks")).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn typecheck_file_do_generalizes_to_monad() {
    let _tm = crate::types::typecheck_file(std::path::Path::new("tests/do_monad.ks")).unwrap();
}

#[test]
fn ir_run_main_show_composites() {
    let src = "main = do\n  stdoutWrite (show [1, 2])\n  stdoutWrite (show (1, True))\n  stdoutWrite (show {a: 1, b: True})\n  IO ()\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn typeclass_operator_method_and_class_default_impl() {
    // - class defines an operator method via `(+) :: ...`
    // - class provides a default implementation using infix operator syntax
    // - instance omits the method, so the default is used
    let src = r#"
class Add1 a where
    (+) :: a -> a -> a
    x + y = x

instance Add1 Integer where

main = do
    stdoutWrite (intToString (1 + 2))
    IO ()
"#;

    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_user_defined_typeclass_superclass_dict_projection() {
    let src = r#"
class A a where
    foo :: a -> a

class (A a) => B a where
    bar :: a -> a

instance A Integer where
    foo x = x + 1

instance B Integer where
    bar x = foo x

useB = (\x -> foo x) :: (B a) => a -> a

main = do
    stdoutWrite (show (bar 1))
    stdoutWrite (show (useB 1))
    IO ()
"#;

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
    assert!(matches!(
        arms[0].pat,
        crate::ir::IrPattern::RecordLoose(_, _)
    ));
}

#[test]
fn ir_lowering_view_pattern() {
    let src = "id = \\x -> x\nx = case 1 of\n  (n <- id) -> n\n  _ -> 0\n";
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
    assert!(matches!(expr, crate::ir::IrExpr::Let { .. }));
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
    assert!(matches!(expr, crate::ir::IrExpr::Let { .. }));
}

#[test]
fn ir_lowering_top_level_pattern_binding() {
    let src = "(a, b) = (1, 2)\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    assert!(ir
        .items
        .iter()
        .any(|it| matches!(it, crate::ir::IrItem::Binding { name, .. } if name == "a")));
    assert!(ir
        .items
        .iter()
        .any(|it| matches!(it, crate::ir::IrItem::Binding { name, .. } if name == "b")));
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
    let src = "main = case [1, 2] of\n  x:xs -> IO ()\n  [] -> throw \"assert failed\"\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_run_main_cons_expr_and_pattern() {
    let src = "main = case (1:[]) of\n  x:xs -> IO ()\n  [] -> throw \"assert failed\"\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_cons_expr_head_is_lazy() {
    let src =
        "main = let\n  bad = error \"boom\"\nin case (bad:[]) of\n  _:xs -> IO ()\n  [] -> throw \"assert failed\"\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_cons_expr_tail_is_lazy() {
    let src =
        "main = let\n  bad = error \"boom\"\nin case (1:bad) of\n  x:xs -> IO ()\n  [] -> throw \"assert failed\"\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_cons_pattern_is_lazy_in_tail() {
    let src = "main = let\n  bad = error \"boom\"\n  xs = [1, bad]\nin case xs of\n  x:xt -> IO ()\n  [] -> throw \"assert failed\"\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_let_is_lazy() {
    let src = "main = let\n  x = error \"boom\"\nin IO ()\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_apply_args_are_lazy() {
    let src =
        "main = let\n  bad = error \"boom\"\n  f = \\a b -> a\nin do\n  IO (f 1 bad)\n  IO ()\n";
    let m = crate::parser::parse_module(src).unwrap();
    let tm = crate::types::typecheck(m).unwrap();
    let ir = crate::ir::lower_to_ir(&tm.module).unwrap();
    let v = crate::ir::run_main(&ir).unwrap();
    assert!(matches!(v, crate::ir::Value::Unit));
}

#[test]
fn ir_tuple_elems_are_lazy() {
    let src = "main = let\n  bad = error \"boom\"\n  x = (1, bad)\n  first = case x of\n    (a, b) -> a\nin do\n  IO first\n  IO ()\n";
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

    use crate::ast::{ExprKind, Item};

    match &module.items[0] {
        Item::Binding(b) => assert!(matches!(&b.expr.kind, ExprKind::Lambda { .. })),
        _ => panic!("expected binding"),
    }

    match &module.items[1] {
        Item::Binding(b) => assert!(matches!(&b.expr.kind, ExprKind::Lambda { .. })),
        _ => panic!("expected binding"),
    }

    match &module.items[2] {
        Item::Binding(b) => assert!(matches!(&b.expr.kind, ExprKind::If { .. })),
        _ => panic!("expected binding"),
    }

    match &module.items[3] {
        Item::Binding(b) => assert!(matches!(&b.expr.kind, ExprKind::Apply { .. })),
        _ => panic!("expected binding"),
    }

    match &module.items[4] {
        Item::Binding(b) => assert!(matches!(&b.expr.kind, ExprKind::Lambda { .. })),
        _ => panic!("expected binding"),
    }
}

#[test]
fn parser_golden_list_expr() {
    let src = std::fs::read_to_string("tests/parser_list.ks").unwrap();
    let module = crate::parser::parse_module(&src).unwrap();
    assert_eq!(module.items.len(), 2);

    use crate::ast::{ExprKind, Item};

    match &module.items[0] {
        Item::Binding(b) => assert!(matches!(&b.expr.kind, ExprKind::List(v) if v.is_empty())),
        _ => panic!("expected binding"),
    }

    match &module.items[1] {
        Item::Binding(b) => match &b.expr.kind {
            ExprKind::List(v) => {
                assert_eq!(v.len(), 3);
                assert!(matches!(&v[0].kind, ExprKind::Integer(s) if s == "1"));
                assert!(matches!(&v[1].kind, ExprKind::Integer(s) if s == "2"));
                assert!(matches!(&v[2].kind, ExprKind::Apply { .. }));
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

    use crate::ast::{ExprKind, Item};

    match &module.items[0] {
        Item::Binding(b) => assert!(matches!(&b.expr.kind, ExprKind::Unit)),
        _ => panic!("expected binding"),
    }

    match &module.items[1] {
        Item::Binding(b) => assert!(matches!(&b.expr.kind, ExprKind::Apply { .. })),
        _ => panic!("expected binding"),
    }

    match &module.items[2] {
        Item::Binding(b) => match &b.expr.kind {
            ExprKind::Tuple(v) => {
                assert_eq!(v.len(), 3);
                assert!(matches!(&v[0].kind, ExprKind::Integer(s) if s == "1"));
                assert!(matches!(&v[1].kind, ExprKind::Integer(s) if s == "2"));
                assert!(matches!(&v[2].kind, ExprKind::Apply { .. }));
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

    use crate::ast::{ExprKind, Item};

    match &module.items[0] {
        Item::Binding(b) => assert!(matches!(&b.expr.kind, ExprKind::Record(v) if v.is_empty())),
        _ => panic!("expected binding"),
    }

    match &module.items[1] {
        Item::Binding(b) => match &b.expr.kind {
            ExprKind::Record(v) => {
                assert_eq!(v.len(), 2);
                assert_eq!(v[0].0, "a");
                assert!(matches!(&v[0].1.kind, ExprKind::Integer(s) if s == "1"));
                assert_eq!(v[1].0, "b");
                assert!(matches!(&v[1].1.kind, ExprKind::Apply { .. }));
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

    use crate::ast::{ExprKind, Item, PatternKind};

    match &module.items[0] {
        Item::Binding(b) => match &b.expr.kind {
            ExprKind::Let { bindings, body } => {
                assert_eq!(bindings.len(), 1);
                assert!(matches!(
                    &bindings[0].pat.kind,
                    PatternKind::Var(s) if s == "x"
                ));
                assert!(matches!(&bindings[0].expr.kind, ExprKind::Integer(s) if s == "1"));
                assert!(matches!(&body.as_ref().kind, ExprKind::Var(s) if s == "x"));
            }
            _ => panic!("expected let"),
        },
        _ => panic!("expected binding"),
    }

    match &module.items[1] {
        Item::Binding(b) => match &b.expr.kind {
            ExprKind::Let { bindings, body } => {
                assert_eq!(bindings.len(), 2);
                assert!(matches!(
                    &bindings[0].pat.kind,
                    PatternKind::Var(s) if s == "x"
                ));
                assert!(matches!(
                    &bindings[1].pat.kind,
                    PatternKind::Var(s) if s == "y"
                ));
                assert!(matches!(&body.as_ref().kind, ExprKind::Apply { .. }));
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

    use crate::ast::{ExprKind, Item, PatternKind};

    match &module.items[0] {
        Item::Binding(b) => match &b.expr.kind {
            ExprKind::Case { arms, .. } => {
                assert_eq!(arms.len(), 1);
                assert!(matches!(&arms[0].pat.kind, PatternKind::Wildcard));
            }
            _ => panic!("expected case"),
        },
        _ => panic!("expected binding"),
    }

    match &module.items[1] {
        Item::Binding(b) => match &b.expr.kind {
            ExprKind::Case { arms, .. } => {
                assert_eq!(arms.len(), 2);
                assert!(
                    matches!(&arms[0].pat.kind, PatternKind::Literal(e) if matches!(&e.kind, ExprKind::Bool(true)))
                );
                assert!(
                    matches!(&arms[1].pat.kind, PatternKind::Literal(e) if matches!(&e.kind, ExprKind::Bool(false)))
                );
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

    use crate::ast::{ExprKind, Item, PatternKind};

    match &module.items[0] {
        Item::Binding(b) => match &b.expr.kind {
            ExprKind::Where { bindings, .. } => {
                assert_eq!(bindings.len(), 1);
                assert!(matches!(
                    &bindings[0].pat.kind,
                    PatternKind::Var(s) if s == "x"
                ));
            }
            _ => panic!("expected where"),
        },
        _ => panic!("expected binding"),
    }

    match &module.items[1] {
        Item::Binding(b) => match &b.expr.kind {
            ExprKind::Where { bindings, .. } => {
                assert_eq!(bindings.len(), 2);
                assert!(matches!(
                    &bindings[0].pat.kind,
                    PatternKind::Var(s) if s == "x"
                ));
                assert!(matches!(
                    &bindings[1].pat.kind,
                    PatternKind::Var(s) if s == "y"
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

    use crate::ast::{ExprKind, Item, PatternKind};

    let Item::Binding(b) = &module.items[0] else {
        panic!("expected binding");
    };

    let ExprKind::Case { arms, .. } = &b.expr.kind else {
        panic!("expected case");
    };

    assert_eq!(arms.len(), 7);
    assert!(
        matches!(&arms[0].pat.kind, PatternKind::Literal(e) if matches!(&e.kind, ExprKind::Unit))
    );
    assert!(matches!(&arms[1].pat.kind, PatternKind::Tuple(_)));
    assert!(matches!(&arms[2].pat.kind, PatternKind::List(_)));
    assert!(matches!(&arms[3].pat.kind, PatternKind::Record(_)));
    assert!(matches!(&arms[4].pat.kind, PatternKind::Constructor { .. }));
    assert!(matches!(&arms[5].pat.kind, PatternKind::Wildcard));
    assert!(arms[5].guard.is_some());
    assert!(matches!(&arms[6].pat.kind, PatternKind::Or(_, _)));
}

#[test]
fn parser_type_annotations() {
    let src = std::fs::read_to_string("tests/parser_annot.ks").unwrap();
    let module = crate::parser::parse_module(&src).unwrap();
    assert_eq!(module.items.len(), 3);

    use crate::ast::{ExprKind, Item, QualType, Type};

    let Item::Binding(b0) = &module.items[0] else {
        panic!("expected binding");
    };
    assert!(matches!(
        &b0.expr.kind,
        ExprKind::Annot {
            ty: QualType {
                ty: Type::Integer,
                ..
            },
            ..
        }
    ));

    let Item::Binding(b1) = &module.items[1] else {
        panic!("expected binding");
    };
    assert!(matches!(
        &b1.expr.kind,
        ExprKind::Annot {
            ty: QualType {
                ty: Type::Float64,
                ..
            },
            ..
        }
    ));

    let Item::Binding(b2) = &module.items[2] else {
        panic!("expected binding");
    };
    let ExprKind::List(v) = &b2.expr.kind else {
        panic!("expected list");
    };
    assert!(matches!(
        &v[0].kind,
        ExprKind::Annot {
            ty: QualType {
                ty: Type::Integer,
                ..
            },
            ..
        }
    ));
}

#[test]
fn parser_type_exprs() {
    let src = std::fs::read_to_string("tests/parser_type_expr.ks").unwrap();
    let module = crate::parser::parse_module(&src).unwrap();
    assert_eq!(module.items.len(), 5);

    use crate::ast::{ExprKind, Item, QualType, Type};

    let Item::Binding(b0) = &module.items[0] else {
        panic!("expected binding");
    };
    assert!(matches!(
        &b0.expr.kind,
        ExprKind::Annot {
            ty: QualType {
                ty: Type::List(_),
                ..
            },
            ..
        }
    ));

    let Item::Binding(b1) = &module.items[1] else {
        panic!("expected binding");
    };
    assert!(matches!(
        &b1.expr.kind,
        ExprKind::Annot {
            ty: QualType {
                ty: Type::Tuple(_),
                ..
            },
            ..
        }
    ));

    let Item::Binding(b2) = &module.items[2] else {
        panic!("expected binding");
    };
    assert!(matches!(
        &b2.expr.kind,
        ExprKind::Annot {
            ty: QualType {
                ty: Type::Record(_),
                ..
            },
            ..
        }
    ));

    let Item::Binding(b3) = &module.items[3] else {
        panic!("expected binding");
    };
    assert!(matches!(
        &b3.expr.kind,
        ExprKind::Annot {
            ty: QualType {
                ty: Type::App { .. },
                ..
            },
            ..
        }
    ));

    let Item::Binding(b4) = &module.items[4] else {
        panic!("expected binding");
    };
    assert!(matches!(
        &b4.expr.kind,
        ExprKind::Annot {
            ty: QualType {
                ty: Type::Func(_, _),
                ..
            },
            ..
        }
    ));
}

#[test]
fn parser_type_holes() {
    crate::lib_test::parser_typehole_alias_do::parser_type_holes();
}

#[test]
fn typecheck_expands_type_aliases() {
    crate::lib_test::parser_typehole_alias_do::typecheck_expands_type_aliases();
}

#[test]
fn parser_do_blocks() {
    crate::lib_test::parser_typehole_alias_do::parser_do_blocks();
}

// ============================================================================
// IR Optimization Tests
// ============================================================================

#[test]
fn ir_optimize_constant_folding_preserves_semantics() {
    let src = r#"
module Main where
  testIf = if True then 42 else 0
  testIf2 = if False then 0 else 99
  main = IO ()
"#;
    let ast = crate::parser::parse_module(src).unwrap();
    let typed = crate::types::typecheck(ast).unwrap();
    let ir = crate::ir::lower_to_ir(&typed.module).unwrap();

    // Apply constant folding
    use kscr_ir::optimize::{ConstantFolding, OptimizationPass};
    let pass = ConstantFolding;
    let optimized = pass.optimize_module(&ir);

    // Both should have the same bindings
    assert_eq!(ir.items.len(), optimized.items.len());
}

#[test]
fn ir_optimize_dead_code_elimination_removes_unused() {
    let src = r#"
module Main where
  used = 100
  unused = 999
  testUsed = used
  main = IO ()
"#;
    let ast = crate::parser::parse_module(src).unwrap();
    let typed = crate::types::typecheck(ast).unwrap();
    let ir = crate::ir::lower_to_ir(&typed.module).unwrap();

    let original_count = ir.items.len();

    // Apply dead code elimination
    use kscr_ir::optimize::{DeadCodeElimination, OptimizationPass};
    let pass = DeadCodeElimination;
    let optimized = pass.optimize_module(&ir);

    // Should have fewer items (unused is removed)
    assert!(optimized.items.len() < original_count);

    // Should still have main
    assert!(optimized.items.iter().any(|item| match item {
        kscr_ir::ir::IrItem::Binding { name, .. } => name == "main",
    }));
}

#[test]
fn ir_optimize_case_simplification_simplifies_trivial() {
    let src = r#"
module Main where
  testCase = case 42 of
    x -> x
  main = IO ()
"#;
    let ast = crate::parser::parse_module(src).unwrap();
    let typed = crate::types::typecheck(ast).unwrap();
    let ir = crate::ir::lower_to_ir(&typed.module).unwrap();

    // Apply case simplification
    use kscr_ir::optimize::{CaseSimplification, OptimizationPass};
    let pass = CaseSimplification;
    let optimized = pass.optimize_module(&ir);

    // Both should have the same number of items
    assert_eq!(ir.items.len(), optimized.items.len());
}

#[test]
fn ir_optimize_pipeline_preserves_execution() {
    let src = r#"
module Main where
  f x = if True then x else 0
  unused = 999
  result = f 42
  main = IO ()
"#;
    let ast = crate::parser::parse_module(src).unwrap();
    let typed = crate::types::typecheck(ast).unwrap();
    let ir = crate::ir::lower_to_ir(&typed.module).unwrap();

    // Apply optimization pipeline
    use kscr_ir::optimize::{
        run_passes, CaseSimplification, ConstantFolding, DeadCodeElimination, OptimizationPass,
    };
    let passes: Vec<Box<dyn OptimizationPass>> = vec![
        Box::new(ConstantFolding),
        Box::new(CaseSimplification),
        Box::new(DeadCodeElimination),
    ];
    let optimized = run_passes(&ir, &passes);

    // Both should run without error
    let result_orig = crate::ir::run_main(&ir);
    let result_opt = crate::ir::run_main(&optimized);

    assert!(result_orig.is_ok());
    assert!(result_opt.is_ok());
}

#[test]
fn ir_optimize_lazy_semantics_preserved() {
    // Test that optimization preserves lazy evaluation
    let src = r#"
module Main where
  inf = inf  -- infinite loop
  testLazy = case () of
    _ -> 42
  main = IO ()
"#;
    let ast = crate::parser::parse_module(src).unwrap();
    let typed = crate::types::typecheck(ast).unwrap();
    let ir = crate::ir::lower_to_ir(&typed.module).unwrap();

    // Apply optimizations
    use kscr_ir::optimize::{run_passes, CaseSimplification, ConstantFolding, OptimizationPass};
    let passes: Vec<Box<dyn OptimizationPass>> =
        vec![Box::new(ConstantFolding), Box::new(CaseSimplification)];
    let optimized = run_passes(&ir, &passes);

    // Both should run (not diverge on unused 'inf')
    let result_orig = crate::ir::run_main(&ir);
    let result_opt = crate::ir::run_main(&optimized);

    assert!(result_orig.is_ok());
    assert!(result_opt.is_ok());
}

#[test]
fn ir_optimize_api_example() {
    // Example showing how to use the optimization API
    let src = r#"
module Main where
  unused1 = 100
  unused2 = 200
  used = 42
  result = if True then used else 0
  main = IO ()
"#;
    let ast = crate::parser::parse_module(src).unwrap();
    let typed = crate::types::typecheck(ast).unwrap();
    let ir = crate::ir::lower_to_ir(&typed.module).unwrap();

    // Count items before optimization
    let before_count = ir.items.len();

    // Apply default optimization passes
    let optimized = crate::ir::optimize_ir(&ir);

    // Count items after optimization
    let after_count = optimized.items.len();

    // Should have removed unused bindings
    assert!(
        after_count < before_count,
        "Expected optimization to remove unused code"
    );

    // Both should execute successfully
    assert!(crate::ir::run_main(&ir).is_ok());
    assert!(crate::ir::run_main(&optimized).is_ok());
}
