use kscr::{parser, types};

#[test]
fn doc_comments_attach_and_reachable_from_typedmodule_docs() {
    let src = include_str!("doc_comments_attach_smoke.ks");
    let m = parser::parse_module(src).expect("parse");

    // Parser attaches docs to AST nodes.
    let mut found = std::collections::HashMap::<String, String>::new();
    for it in &m.items {
        match it {
            kscr::ast::Item::Binding(b) => {
                if let kscr::ast::PatternKind::Var(name) = &b.pat.kind {
                    if let Some(doc) = &b.doc {
                        found.insert(name.clone(), doc.clone());
                    }
                }
            }
            kscr::ast::Item::DataDecl(d) => {
                if let Some(doc) = &d.doc {
                    found.insert(d.name.clone(), doc.clone());
                }
            }
            kscr::ast::Item::TypeAlias(ta) => {
                if let Some(doc) = &ta.doc {
                    found.insert(ta.name.clone(), doc.clone());
                }
            }
            kscr::ast::Item::ClassDecl(c) => {
                if let Some(doc) = &c.doc {
                    found.insert(c.name.clone(), doc.clone());
                }
            }
            _ => {}
        }
    }

    assert_eq!(found.get("foo").unwrap(), "Doc for foo.");
    assert_eq!(found.get("Bar").unwrap(), "Doc for Bar.");
    assert_eq!(
        found.get("Baz").unwrap(),
        "Block doc for Baz.\nSecond line."
    );
    assert_eq!(found.get("Qux").unwrap(), "Doc for Qux.");

    // TypedModule collects docs into an index.
    let tm = types::typecheck(m).expect("typecheck");
    assert_eq!(tm.docs.get("foo").unwrap(), "Doc for foo.");
    assert_eq!(tm.docs.get("Bar").unwrap(), "Doc for Bar.");
    assert_eq!(
        tm.docs.get("Baz").unwrap(),
        "Block doc for Baz.\nSecond line."
    );
    assert_eq!(tm.docs.get("Qux").unwrap(), "Doc for Qux.");
}
