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

pub fn lex(src: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    for word in src.split_whitespace() {
        let kind = match word {
            "True" => TokenKind::True,
            "False" => TokenKind::False,
            "type" => TokenKind::KwType,
            "data" => TokenKind::KwData,
            "=" => TokenKind::Eq,
            "|" => TokenKind::Pipe,
            _ if word.chars().all(|c| c.is_ascii_digit()) => TokenKind::Integer(word.to_string()),
            _ if word.starts_with('"') && word.ends_with('"') => TokenKind::String(word.trim_matches('"').to_string()),
            _ if word.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => TokenKind::Ident(word.to_string()),
            _ => continue, // skip unknown for now
        };
        tokens.push(Token { kind });
    }
    tokens
}
