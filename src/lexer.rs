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

    let mut tokens = Vec::new();
    for line in src.lines() {
        let line = line.trim();
        // 行コメント（--）を除去
        let line = match line.find("--") {
            Some(idx) => &line[..idx],
            None => line,
        };
        for word in line.split_whitespace() {
            let kind = match word {
                "True" => TokenKind::True,
                "False" => TokenKind::False,
                "type" => TokenKind::KwType,
                "data" => TokenKind::KwData,
                "module" => TokenKind::Ident("module".to_string()),
                "import" => TokenKind::Ident("import".to_string()),
                "export" => TokenKind::Ident("export".to_string()),
                "let" => TokenKind::Ident("let".to_string()),
                "in" => TokenKind::Ident("in".to_string()),
                "where" => TokenKind::Ident("where".to_string()),
                "case" => TokenKind::Ident("case".to_string()),
                "of" => TokenKind::Ident("of".to_string()),
                "if" => TokenKind::Ident("if".to_string()),
                "then" => TokenKind::Ident("then".to_string()),
                "else" => TokenKind::Ident("else".to_string()),
                "do" => TokenKind::Ident("do".to_string()),
                "=" => TokenKind::Eq,
                "|" => TokenKind::Pipe,
                _ if word.chars().all(|c| c.is_ascii_digit()) => TokenKind::Integer(word.to_string()),
                _ if word.parse::<f64>().is_ok() && word.contains('.') => TokenKind::Float(word.to_string()),
                _ if word.starts_with('"') && word.ends_with('"') => TokenKind::String(word.trim_matches('"').to_string()),
                _ if word.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => TokenKind::Ident(word.to_string()),
                _ => continue, // skip unknown for now
            };
            tokens.push(Token { kind });
        }
    }
    tokens
}
