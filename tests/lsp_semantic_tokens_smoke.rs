use std::collections::HashSet;

#[allow(dead_code)]
#[path = "../crates/kscr_lsp/src/vfs.rs"]
mod vfs;

#[allow(dead_code)]
#[path = "../crates/kscr_lsp/src/backend_helpers.rs"]
mod backend_helpers;

#[allow(dead_code)]
#[path = "../crates/kscr_lsp/src/backend_semantic_tokens.rs"]
mod backend_semantic_tokens;

#[test]
fn lsp_semantic_tokens_non_empty_for_sample_module() {
    let src = r#"module Main where
  data Opt a = Some a | None
  class ShowLike a where
    showLike :: a -> String
  answer = Some 42
"#;

    let uri = tower_lsp::lsp_types::Url::parse("file:///semantic_tokens_sample.ks").unwrap();
    let doc = vfs::Document::new(uri, src.to_string(), 1);

    let tokens = backend_semantic_tokens::semantic_tokens_in_doc(&doc)
        .expect("semantic tokens should be available for parseable source");

    assert!(
        !tokens.data.is_empty(),
        "semantic token stream should not be empty"
    );
}

#[test]
fn lsp_semantic_tokens_shape_and_types_are_reasonable() {
    let src = r#"module Main where
  data Opt a = Some a | None
  class ShowLike a where
    showLike :: a -> String
  answer = Some 42
"#;

    let uri = tower_lsp::lsp_types::Url::parse("file:///semantic_tokens_shape.ks").unwrap();
    let doc = vfs::Document::new(uri, src.to_string(), 1);

    let tokens = backend_semantic_tokens::semantic_tokens_in_doc(&doc)
        .expect("semantic tokens should be available for parseable source");

    let mut seen_token_types: HashSet<u32> = HashSet::new();
    for token in &tokens.data {
        assert!(token.length > 0, "token length must be > 0: {token:?}");
        seen_token_types.insert(token.token_type);
    }

    assert!(
        seen_token_types.contains(&0),
        "expected function token type (0)"
    );
    assert!(
        seen_token_types.contains(&1),
        "expected type token type (1)"
    );
    assert!(
        seen_token_types.contains(&2),
        "expected class token type (2)"
    );
    assert!(
        seen_token_types.contains(&4),
        "expected enum member token type (4)"
    );
}
