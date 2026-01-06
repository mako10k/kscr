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
    for line in src.lines() {
        let line = line.trim();
        // 行コメント（--）を除去
        let line = match line.find("--") {
            Some(idx) => &line[..idx],
            None => line,
        };
        for word in line.split_whitespace() {
            let kind = if word == "True" {
                Some(TokenKind::True)
            } else if word == "False" {
                Some(TokenKind::False)
            } else if word == "type" {
                Some(TokenKind::KwType)
            } else if word == "data" {
                Some(TokenKind::KwData)
            } else if word == "module" {
                Some(TokenKind::Ident("module".to_string()))
            } else if word == "import" {
                Some(TokenKind::Ident("import".to_string()))
            } else if word == "export" {
                Some(TokenKind::Ident("export".to_string()))
            } else if word == "let" {
                Some(TokenKind::Ident("let".to_string()))
            } else if word == "in" {
                Some(TokenKind::Ident("in".to_string()))
            } else if word == "where" {
                Some(TokenKind::Ident("where".to_string()))
            } else if word == "case" {
                Some(TokenKind::Ident("case".to_string()))
            } else if word == "of" {
                Some(TokenKind::Ident("of".to_string()))
            } else if word == "if" {
                Some(TokenKind::Ident("if".to_string()))
            } else if word == "then" {
                Some(TokenKind::Ident("then".to_string()))
            } else if word == "else" {
                Some(TokenKind::Ident("else".to_string()))
            } else if word == "do" {
                Some(TokenKind::Ident("do".to_string()))
            } else if word == "=" {
                Some(TokenKind::Eq)
            } else if word == "|" {
                Some(TokenKind::Pipe)
            } else if word.chars().all(|c| c.is_ascii_digit()) {
                Some(TokenKind::Integer(word.to_string()))
            } else if word.parse::<f64>().is_ok() && word.contains('.') {
                Some(TokenKind::Float(word.to_string()))
            } else if word.starts_with('"') && word.ends_with('"') {
                Some(TokenKind::String(word.trim_matches('"').to_string()))
            } else if word.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                Some(TokenKind::Ident(word.to_string()))
            } else {
                None
            };
            if let Some(kind) = kind {
                tokens.push(Token { kind });
            }
        }
    }
    tokens
}
