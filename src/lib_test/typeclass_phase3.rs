#[test]
fn typecheck_accepts_instance_context_for_superclass_dicts() {
    let src = r#"
module Main where

    class C a where
        f :: a -> a

    class C a => D a where
        g :: a -> a

    -- Provide the superclass dictionary via instance context.
    instance C Integer => D Integer where
        g x = x

    main :: IO Unit
    main = IO ()
"#;

    let ast = crate::parser::parse_module(src).unwrap();
    crate::types::typecheck(ast).unwrap();
}

#[test]
fn typecheck_accepts_non_ground_instance_with_ctx_from_scope() {
    let src = r#"
module Main where

    data Maybe a = Nothing | Just a

    class C a where
        f :: a -> a

    instance C a => C (Maybe a) where
        f x = x

    -- Here, `C a` is in scope (as a dictionary arg), so `C (Maybe a)` can be built.
    use :: C a => Maybe a -> Maybe a
    use x = f x

    main :: IO Unit
    main = IO ()
"#;

    let ast = crate::parser::parse_module(src).unwrap();
    crate::types::typecheck(ast).unwrap();
}
