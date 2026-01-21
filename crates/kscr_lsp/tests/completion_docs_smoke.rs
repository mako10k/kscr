use kscr::types;
use kscr_lsp::backend_goto_completion::completion_items_in_doc;
use kscr_lsp::vfs::Document;
use tower_lsp::lsp_types::{MarkupKind, Position};

#[test]
fn completion_includes_doc_comments() {
    // Keep this file small and stable.
    // We rely on the compiler pipeline (parse -> typecheck) to populate TypedModule.docs.
    let src = r#"-- | Adds two numbers.
add x y = x + y

-- | Another binding.
adjust = 0
"#;

    let uri = tower_lsp::lsp_types::Url::parse("file:///completion_docs_smoke.ks").unwrap();
    let doc = Document::new(uri, src.to_string());

    let module = kscr::parser::parse_module(src).unwrap();
    let tm = types::typecheck_module(&module, vec![]).unwrap();

    // Completion at end-of-file with prefix "ad" should include `add` and `adjust`.
    let items = completion_items_in_doc(&doc, Position::new(5, 2), &tm).unwrap();

    let add = items.iter().find(|i| i.label == "add").unwrap();
    let add_doc = add.documentation.as_ref().unwrap();
    match add_doc {
        tower_lsp::lsp_types::Documentation::MarkupContent(mc) => {
            assert_eq!(mc.kind, MarkupKind::Markdown);
            assert!(mc.value.contains("Adds two numbers."));
        }
        other => panic!("unexpected documentation: {other:?}"),
    }
}
