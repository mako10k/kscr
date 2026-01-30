use kscr::lexer::{self, TokenKind};

#[test]
fn lex_dot_as_qualification_no_spaces() {
    // a.b and A.B should lex as Ident, Dot, Ident (qualification/field access)
    let src = "a.b";
    let toks = lexer::lex(src).expect("lex");
    let kinds: Vec<_> = toks.iter().map(|t| &t.kind).collect();
    assert_eq!(kinds.len(), 3);
    assert!(matches!(kinds[0], TokenKind::Ident(_)));
    assert!(matches!(kinds[1], TokenKind::Dot));
    assert!(matches!(kinds[2], TokenKind::Ident(_)));

    let src = "A.B.C";
    let toks = lexer::lex(src).expect("lex");
    let kinds: Vec<_> = toks.iter().map(|t| &t.kind).collect();
    assert_eq!(kinds.len(), 5);
    assert!(matches!(kinds[0], TokenKind::Ident(_)));
    assert!(matches!(kinds[1], TokenKind::Dot));
    assert!(matches!(kinds[2], TokenKind::Ident(_)));
    assert!(matches!(kinds[3], TokenKind::Dot));
    assert!(matches!(kinds[4], TokenKind::Ident(_)));
}

#[test]
fn lex_dot_as_operator_with_spaces() {
    // "a . b" should lex as Ident, Operator("."), Ident
    let src = "a . b";
    let toks = lexer::lex(src).expect("lex");
    let kinds: Vec<_> = toks.iter().map(|t| &t.kind).collect();
    assert_eq!(kinds.len(), 3);
    assert!(matches!(kinds[0], TokenKind::Ident(_)));
    assert!(matches!(kinds[1], TokenKind::Operator(s) if s == "."));
    assert!(matches!(kinds[2], TokenKind::Ident(_)));
}

#[test]
fn lex_dot_sections() {
    // ".f" should lex as Operator("."), Ident
    let src = ".f";
    let toks = lexer::lex(src).expect("lex");
    let kinds: Vec<_> = toks.iter().map(|t| &t.kind).collect();
    assert_eq!(kinds.len(), 2);
    assert!(matches!(kinds[0], TokenKind::Operator(s) if s == "."));
    assert!(matches!(kinds[1], TokenKind::Ident(_)));

    // "f." should lex as Ident, Operator(".")
    let src = "f.";
    let toks = lexer::lex(src).expect("lex");
    let kinds: Vec<_> = toks.iter().map(|t| &t.kind).collect();
    assert_eq!(kinds.len(), 2);
    assert!(matches!(kinds[0], TokenKind::Ident(_)));
    assert!(matches!(kinds[1], TokenKind::Operator(s) if s == "."));
}

#[test]
fn lex_dollar_as_operator() {
    // "f $ x" should lex as Ident, Operator("$"), Ident
    let src = "f $ x";
    let toks = lexer::lex(src).expect("lex");
    let kinds: Vec<_> = toks.iter().map(|t| &t.kind).collect();
    assert_eq!(kinds.len(), 3);
    assert!(matches!(kinds[0], TokenKind::Ident(_)));
    assert!(matches!(kinds[1], TokenKind::Operator(s) if s == "$"));
    assert!(matches!(kinds[2], TokenKind::Ident(_)));
}

#[test]
fn lex_float_unchanged() {
    // "1.23" should still lex as Float
    let src = "1.23";
    let toks = lexer::lex(src).expect("lex");
    let kinds: Vec<_> = toks.iter().map(|t| &t.kind).collect();
    assert_eq!(kinds.len(), 1);
    assert!(matches!(kinds[0], TokenKind::Float(_)));
}
