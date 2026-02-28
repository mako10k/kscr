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

#[test]
fn lsp_semantic_tokens_range_returns_subset() {
    let src = r#"module Main where
  data Opt a = Some a | None
  answer = Some 42
"#;

    let uri = tower_lsp::lsp_types::Url::parse("file:///semantic_tokens_range_subset.ks").unwrap();
    let doc = vfs::Document::new(uri, src.to_string(), 5);

    let all = backend_semantic_tokens::semantic_tokens_in_doc(&doc)
        .expect("full semantic tokens should be available");
    let subset = backend_semantic_tokens::semantic_tokens_in_range(
        &doc,
        tower_lsp::lsp_types::Range {
            start: tower_lsp::lsp_types::Position {
                line: 2,
                character: 0,
            },
            end: tower_lsp::lsp_types::Position {
                line: 3,
                character: 0,
            },
        },
    )
    .expect("range semantic tokens should be available");

    assert!(
        !subset.data.is_empty(),
        "range semantic token stream should not be empty"
    );
    assert!(subset.data.len() <= all.data.len());
}

#[test]
fn lsp_semantic_tokens_full_delta_returns_tokens_result() {
    let src = "module Main where\n  x = 1\n";
    let uri = tower_lsp::lsp_types::Url::parse("file:///semantic_tokens_delta_result.ks").unwrap();
    let doc = vfs::Document::new(uri, src.to_string(), 9);

    let delta = tower_lsp::lsp_types::SemanticTokensFullDeltaResult::Tokens(
        backend_semantic_tokens::semantic_tokens_in_doc(&doc)
            .expect("semantic tokens should be available"),
    );

    match delta {
        tower_lsp::lsp_types::SemanticTokensFullDeltaResult::Tokens(tokens) => {
            assert_eq!(tokens.result_id.as_deref(), Some("9"));
            assert!(!tokens.data.is_empty());
        }
        tower_lsp::lsp_types::SemanticTokensFullDeltaResult::TokensDelta(_)
        | tower_lsp::lsp_types::SemanticTokensFullDeltaResult::PartialTokensDelta { .. } => {
            panic!("expected full token fallback variant")
        }
    }
}

#[test]
fn lsp_semantic_tokens_full_delta_returns_edits_with_previous() {
    let uri = tower_lsp::lsp_types::Url::parse("file:///semantic_tokens_delta_prev.ks").unwrap();
    let old_doc = vfs::Document::new(uri.clone(), "module Main where\n  x = 1\n".to_string(), 1);
    let new_doc = vfs::Document::new(uri, "module Main where\n  xyz = 1\n".to_string(), 2);

    let previous = backend_semantic_tokens::semantic_tokens_in_doc(&old_doc)
        .expect("previous semantic tokens should be available");
    let current = backend_semantic_tokens::semantic_tokens_in_doc(&new_doc)
        .expect("current semantic tokens should be available");
    let delta =
        backend_semantic_tokens::semantic_tokens_full_delta_from_previous(&previous, current);

    match delta {
        tower_lsp::lsp_types::SemanticTokensFullDeltaResult::TokensDelta(d) => {
            assert_eq!(d.result_id.as_deref(), Some("2"));
            assert!(!d.edits.is_empty());
        }
        _ => panic!("expected TokensDelta variant"),
    }
}
