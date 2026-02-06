use kscr::lexer::{self, TokenKind};

#[test]
fn lex_dot_qualification_a_dot_b() {
    // Case 1: A.B should yield Ident("A"), Dot, Ident("B")
    let src = "A.B";
    let toks = lexer::lex(src).expect("lex");

    let token_kinds: Vec<_> = toks.iter().map(|t| &t.kind).collect();

    assert_eq!(
        token_kinds.len(),
        3,
        "Expected 3 tokens for 'A.B', got: {:?}",
        token_kinds
    );
    assert!(
        matches!(token_kinds[0], TokenKind::Ident(ref s) if s == "A"),
        "First token should be Ident(A), got: {:?}",
        token_kinds[0]
    );
    assert!(
        matches!(token_kinds[1], TokenKind::Dot),
        "Second token should be Dot, got: {:?}",
        token_kinds[1]
    );
    assert!(
        matches!(token_kinds[2], TokenKind::Ident(ref s) if s == "B"),
        "Third token should be Ident(B), got: {:?}",
        token_kinds[2]
    );
}

#[test]
fn lex_dot_operator_a_space_dot_space_b() {
    // Case 2: a . b should yield Ident("a"), Operator("."), Ident("b")
    let src = "a . b";
    let toks = lexer::lex(src).expect("lex");

    let token_kinds: Vec<_> = toks.iter().map(|t| &t.kind).collect();

    assert_eq!(
        token_kinds.len(),
        3,
        "Expected 3 tokens for 'a . b', got: {:?}",
        token_kinds
    );
    assert!(
        matches!(token_kinds[0], TokenKind::Ident(ref s) if s == "a"),
        "First token should be Ident(a), got: {:?}",
        token_kinds[0]
    );
    assert!(
        matches!(token_kinds[1], TokenKind::Operator(ref s) if s == "."),
        "Second token should be Operator(.), got: {:?}",
        token_kinds[1]
    );
    assert!(
        matches!(token_kinds[2], TokenKind::Ident(ref s) if s == "b"),
        "Third token should be Ident(b), got: {:?}",
        token_kinds[2]
    );
}

#[test]
fn lex_dot_operator_dot_f() {
    // Case 3: .f should yield Operator("."), Ident("f")
    let src = ".f";
    let toks = lexer::lex(src).expect("lex");

    let token_kinds: Vec<_> = toks.iter().map(|t| &t.kind).collect();

    assert_eq!(
        token_kinds.len(),
        2,
        "Expected 2 tokens for '.f', got: {:?}",
        token_kinds
    );
    assert!(
        matches!(token_kinds[0], TokenKind::Operator(ref s) if s == "."),
        "First token should be Operator(.), got: {:?}",
        token_kinds[0]
    );
    assert!(
        matches!(token_kinds[1], TokenKind::Ident(ref s) if s == "f"),
        "Second token should be Ident(f), got: {:?}",
        token_kinds[1]
    );
}

#[test]
fn lex_dot_operator_f_dot() {
    // Case 4: f. should yield Ident("f"), Operator(".")
    let src = "f.";
    let toks = lexer::lex(src).expect("lex");

    let token_kinds: Vec<_> = toks.iter().map(|t| &t.kind).collect();

    assert_eq!(
        token_kinds.len(),
        2,
        "Expected 2 tokens for 'f.', got: {:?}",
        token_kinds
    );
    assert!(
        matches!(token_kinds[0], TokenKind::Ident(ref s) if s == "f"),
        "First token should be Ident(f), got: {:?}",
        token_kinds[0]
    );
    assert!(
        matches!(token_kinds[1], TokenKind::Operator(ref s) if s == "."),
        "Second token should be Operator(.), got: {:?}",
        token_kinds[1]
    );
}

#[test]
fn lex_float_1_23() {
    // Case 5: 1.23 should yield Float("1.23")
    let src = "1.23";
    let toks = lexer::lex(src).expect("lex");

    let token_kinds: Vec<_> = toks.iter().map(|t| &t.kind).collect();

    assert_eq!(
        token_kinds.len(),
        1,
        "Expected 1 token for '1.23', got: {:?}",
        token_kinds
    );
    assert!(
        matches!(token_kinds[0], TokenKind::Float(ref s) if s == "1.23"),
        "Token should be Float(1.23), got: {:?}",
        token_kinds[0]
    );
}
