use crate::backend_helpers::{qualified_ident_at_offset, span_to_range};
use crate::vfs::{Document, Vfs};
use kscr::lexer;
use tower_lsp::lsp_types::*;

fn ident_at_pos(doc: &Document, pos: Position) -> Option<String> {
    let off = doc.position_to_offset(pos.line, pos.character)?;
    let (name, _span) = qualified_ident_at_offset(&doc.text, off)?;
    if name.contains('.') {
        return None;
    }
    Some(name)
}

fn collect_ident_spans(doc: &Document, name: &str) -> Vec<lexer::Span> {
    let toks = match lexer::lex(&doc.text) {
        Ok(toks) => toks,
        Err(_) => return Vec::new(),
    };

    toks.iter()
        .filter_map(|t| match &t.kind {
            lexer::TokenKind::Ident(s) if s == name => Some(t.span),
            _ => None,
        })
        .collect()
}

pub(super) fn references_in_vfs(vfs: &Vfs, doc: &Document, pos: Position) -> Vec<Location> {
    let Some(name) = ident_at_pos(doc, pos) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for d in vfs.iter_documents() {
        for span in collect_ident_spans(d, &name) {
            let Some(range) = span_to_range(d, span) else {
                continue;
            };
            out.push(Location {
                uri: d.uri.clone(),
                range,
            });
        }
    }
    out
}

pub(super) fn rename_in_vfs(
    vfs: &Vfs,
    doc: &Document,
    pos: Position,
    new_name: &str,
) -> Option<WorkspaceEdit> {
    let name = ident_at_pos(doc, pos)?;

    let mut changes: std::collections::HashMap<Url, Vec<TextEdit>> =
        std::collections::HashMap::new();

    for d in vfs.iter_documents() {
        let edits: Vec<TextEdit> = collect_ident_spans(d, &name)
            .into_iter()
            .filter_map(|span| {
                let range = span_to_range(d, span)?;
                Some(TextEdit {
                    range,
                    new_text: new_name.to_string(),
                })
            })
            .collect();

        if !edits.is_empty() {
            changes.insert(d.uri.clone(), edits);
        }
    }

    if changes.is_empty() {
        return None;
    }

    Some(WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_and_rename_are_vfs_scoped() {
        let mut vfs = Vfs::new();
        let uri_a = Url::parse("file:///A.ks").unwrap();
        let uri_b = Url::parse("file:///B.ks").unwrap();

        vfs.insert(uri_a.clone(), "module A where\n  foo = 1\n".to_string(), 1);
        vfs.insert(
            uri_b.clone(),
            "module B where\n  bar = foo\n  baz = foo\n".to_string(),
            1,
        );

        let doc_b = vfs.get(&uri_b).unwrap();
        let pos = Position {
            line: 1,
            character: 8,
        };

        let refs = references_in_vfs(&vfs, doc_b, pos);
        assert_eq!(refs.len(), 3);

        let edit = rename_in_vfs(&vfs, doc_b, pos, "qux").unwrap();
        let changes = edit.changes.unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes.get(&uri_a).unwrap().len(), 1);
        assert_eq!(changes.get(&uri_b).unwrap().len(), 2);
    }
}
