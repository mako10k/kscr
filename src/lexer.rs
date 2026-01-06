#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Ident(String),
    Integer(String),
    Float(String),
    String(String),
    True,
    False,
    Eq,
    KwType,
    KwData,
    Pipe,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
}

pub fn lex(_src: &str) -> Vec<Token> {
    // TODO: real lexer
    Vec::new()
}
