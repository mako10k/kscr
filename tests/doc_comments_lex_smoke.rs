use kscr::lexer::{self, TokenKind};

#[test]
fn lex_doc_comments_produce_tokens() {
    let src = "-- | Doc for foo.\nfoo = 1\n\n{-| Block doc\n-}\nbar = 2\n";
    let toks = lexer::lex(src).expect("lex");
    assert!(toks.iter().any(|t| matches!(t.kind, TokenKind::DocLine(_))));
    assert!(toks
        .iter()
        .any(|t| matches!(t.kind, TokenKind::DocBlock(_))));
}
