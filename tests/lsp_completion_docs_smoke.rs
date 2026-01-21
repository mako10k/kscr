use tower_lsp::lsp_types::{MarkupKind, Position};

#[path = "../crates/kscr_lsp/src/vfs.rs"]
mod vfs;

#[path = "../crates/kscr_lsp/src/backend_helpers.rs"]
mod backend_helpers;

#[path = "../crates/kscr_lsp/src/backend_goto_completion.rs"]
mod backend_goto_completion;

#[test]
fn lsp_completion_includes_doc_comments() {
    let src_typed = r#"add x y = x + y

-- | A simple data type.
type Box = Integer

-- | A small sum type.
data Opt a = {-| some ctor doc -} Some a | None

-- | Another binding.
adjust = 0
"#;

    // Document text can be incomplete; completion logic should still work.
    let src_doc = format!("{src_typed}\nad");

    let tmp_dir = std::env::temp_dir().join("kscr_tests");
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let path = tmp_dir.join("completion_docs_smoke.ks");
    std::fs::write(&path, src_typed).unwrap();

    let uri = tower_lsp::lsp_types::Url::from_file_path(&path).unwrap();
    let doc = vfs::Document::new(uri, src_doc, 0);

    let tm = kscr::types::typecheck_file(&path).unwrap();

    assert!(
        tm.docs
            .get("adjust")
            .is_some_and(|d| d.contains("Another binding.")),
        "TypedModule.docs missing for `adjust`: {:?}",
        tm.docs
    );

    assert!(
        tm.docs
            .get("Box")
            .is_some_and(|d| d.contains("A simple data type.")),
        "TypedModule.docs missing for `Box`: {:?}",
        tm.docs
    );

    assert!(
        tm.docs
            .get("Some")
            .is_some_and(|d| d.contains("some ctor doc")),
        "TypedModule.docs missing for `Some`: {:?}",
        tm.docs
    );


    // Completion on a fresh line with prefix "ad" should include `adjust`.
    // Lines are 0-based.
    // `ad` is on the last line.
    let items = backend_goto_completion::completion_items_in_doc(&doc, Position::new(11, 2), &tm)
        .unwrap();

    let adjust = items
        .iter()
        .find(|i| i.label == "adjust")
        .unwrap_or_else(|| {
            panic!(
                "missing `adjust` in completion: {:?}",
                items.iter().map(|i| &i.label).collect::<Vec<_>>()
            )
        });
    let adjust_doc = adjust
        .documentation
        .as_ref()
        .expect("missing documentation for `adjust`");
    match adjust_doc {
        tower_lsp::lsp_types::Documentation::MarkupContent(mc) => {
            assert_eq!(mc.kind, MarkupKind::Markdown);
            assert!(mc.value.contains("Another binding."));
        }
        other => panic!("unexpected documentation: {other:?}"),
    }

    // Completion with prefix "B" should include `Box` with docs.
    let (doc2_uri, doc2_text) = {
        let src_doc2 = format!("{src_typed}\nB");
        (doc.uri.clone(), src_doc2)
    };
    let doc2 = vfs::Document::new(doc2_uri, doc2_text, 0);
    // `B` is appended after the typed source.
    let items2 = backend_goto_completion::completion_items_in_doc(&doc2, Position::new(11, 1), &tm)
        .unwrap();

    let b = items2
        .iter()
        .find(|i| i.label == "Box")
        .unwrap_or_else(|| {
            panic!(
                "missing `Box` in completion: {:?}",
                items2.iter().map(|i| &i.label).collect::<Vec<_>>()
            )
        });
    assert_eq!(
        b.kind,
        Some(tower_lsp::lsp_types::CompletionItemKind::TYPE_PARAMETER)
    );
    let b_doc = b
        .documentation
        .as_ref()
        .expect("missing documentation for `Box`");
    match b_doc {
        tower_lsp::lsp_types::Documentation::MarkupContent(mc) => {
            assert_eq!(mc.kind, MarkupKind::Markdown);
            assert!(mc.value.contains("A simple data type."));
        }
        other => panic!("unexpected documentation: {other:?}"),
    }

    // Completion with prefix "So" should include the constructor `Some` with docs.
    let (doc3_uri, doc3_text) = {
        let src_doc3 = format!("{src_typed}\nSo");
        (doc.uri.clone(), src_doc3)
    };
    let doc3 = vfs::Document::new(doc3_uri, doc3_text, 0);
    // `So` is appended after the typed source.
    let items3 = backend_goto_completion::completion_items_in_doc(&doc3, Position::new(11, 2), &tm)
        .unwrap();

    let some = items3
        .iter()
        .find(|i| i.label == "Some")
        .unwrap_or_else(|| {
            panic!(
                "missing `Some` in completion: {:?}",
                items3.iter().map(|i| &i.label).collect::<Vec<_>>()
            )
        });
    assert_eq!(
        some.kind,
        Some(tower_lsp::lsp_types::CompletionItemKind::CONSTRUCTOR)
    );
    let some_doc = some
        .documentation
        .as_ref()
        .expect("missing documentation for `Some`");
    match some_doc {
        tower_lsp::lsp_types::Documentation::MarkupContent(mc) => {
            assert_eq!(mc.kind, MarkupKind::Markdown);
            assert!(mc.value.contains("some ctor doc"));
        }
        other => panic!("unexpected documentation: {other:?}"),
    }
}
