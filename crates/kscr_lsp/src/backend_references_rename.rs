use crate::backend_goto_completion::goto_definition_in_doc;
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

fn span_start_pos(doc: &Document, span: lexer::Span) -> Option<Position> {
    let (line, character) = doc.offset_to_position(span.start)?;
    Some(Position { line, character })
}

fn same_location(a: &Location, b: &Location) -> bool {
    a.uri == b.uri
        && a.range.start.line == b.range.start.line
        && a.range.start.character == b.range.start.character
        && a.range.end.line == b.range.end.line
        && a.range.end.character == b.range.end.character
}

fn location_from_span(doc: &Document, span: lexer::Span) -> Option<Location> {
    let range = span_to_range(doc, span)?;
    Some(Location {
        uri: doc.uri.clone(),
        range,
    })
}

pub(super) fn references_in_vfs(vfs: &Vfs, doc: &Document, pos: Position) -> Vec<Location> {
    let Some(name) = ident_at_pos(doc, pos) else {
        return Vec::new();
    };

    let anchor_decl = goto_definition_in_doc(doc, pos);

    let mut out = Vec::new();
    for d in vfs.iter_documents() {
        for span in collect_ident_spans(d, &name) {
            let Some(loc) = location_from_span(d, span) else {
                continue;
            };

            let include = if let Some(anchor) = &anchor_decl {
                if same_location(&loc, anchor) {
                    true
                } else if let Some(candidate_pos) = span_start_pos(d, span) {
                    goto_definition_in_doc(d, candidate_pos)
                        .as_ref()
                        .is_some_and(|decl| same_location(decl, anchor))
                } else {
                    false
                }
            } else {
                true
            };

            if include {
                out.push(loc);
            }
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
    let anchor_decl = goto_definition_in_doc(doc, pos);

    let mut changes: std::collections::HashMap<Url, Vec<TextEdit>> =
        std::collections::HashMap::new();

    for d in vfs.iter_documents() {
        let edits: Vec<TextEdit> = collect_ident_spans(d, &name)
            .into_iter()
            .filter_map(|span| {
                let range = span_to_range(d, span)?;

                if let Some(anchor) = &anchor_decl {
                    let loc = Location {
                        uri: d.uri.clone(),
                        range,
                    };

                    let include = if same_location(&loc, anchor) {
                        true
                    } else if let Some(candidate_pos) = span_start_pos(d, span) {
                        goto_definition_in_doc(d, candidate_pos)
                            .as_ref()
                            .is_some_and(|decl| same_location(decl, anchor))
                    } else {
                        false
                    };

                    if !include {
                        return None;
                    }
                }

                Some(TextEdit {
                    range: span_to_range(d, span)?,
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

    #[test]
    fn references_and_rename_exclude_unrelated_same_name_symbols() {
        let mut vfs = Vfs::new();

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("kscr-lsp-refs-rename-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();

        let a_path = dir.join("A.ks");
        let b_path = dir.join("B.ks");
        let c_path = dir.join("C.ks");

        let src_a = "module A where\n  foo = 1\n";
        let src_b = "module B where\n  import A\n  bar = foo\n";
        let src_c = "module C where\n  foo = 99\n  z = foo\n";

        std::fs::write(&a_path, src_a).unwrap();
        std::fs::write(&b_path, src_b).unwrap();
        std::fs::write(&c_path, src_c).unwrap();

        let uri_a = Url::from_file_path(&a_path).unwrap();
        let uri_b = Url::from_file_path(&b_path).unwrap();
        let uri_c = Url::from_file_path(&c_path).unwrap();

        vfs.insert(uri_a.clone(), src_a.to_string(), 1);
        vfs.insert(uri_b.clone(), src_b.to_string(), 1);
        vfs.insert(uri_c.clone(), src_c.to_string(), 1);

        let doc_b = vfs.get(&uri_b).unwrap();
        let pos = Position {
            line: 2,
            character: 8,
        };

        let refs = references_in_vfs(&vfs, doc_b, pos);
        assert_eq!(refs.len(), 2, "refs = {refs:?}");
        assert!(refs.iter().any(|l| l.uri == uri_a));
        assert!(refs.iter().any(|l| l.uri == uri_b));
        assert!(!refs.iter().any(|l| l.uri == uri_c));

        let edit = rename_in_vfs(&vfs, doc_b, pos, "qux").unwrap();
        let changes = edit.changes.unwrap();
        assert_eq!(changes.len(), 2);
        assert!(changes.contains_key(&uri_a));
        assert!(changes.contains_key(&uri_b));
        assert!(!changes.contains_key(&uri_c));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
