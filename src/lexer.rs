#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Ident(String),
    Integer(String),
    Float(String),
    String(String),
    True,
    False,

    KwIf,
    KwThen,
    KwElse,
    KwType,
    KwData,

    Newline,
    Eq,
    Pipe,
    Backslash,
    Arrow,
    LBracket,
    RBracket,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
}

pub fn lex(src: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0usize;

    // Shebang handling: ignore first line if it starts with "#!".
    if bytes.starts_with(b"#!") {
        while i < bytes.len() && bytes[i] != b'\n' {
            i += 1;
        }
        if i < bytes.len() && bytes[i] == b'\n' {
            i += 1;
        }
    }

    while i < bytes.len() {
        // Newline (statement separator)
        if bytes[i] == b'\n' {
            tokens.push(Token {
                kind: TokenKind::Newline,
            });
            i += 1;
            continue;
        }

        // Whitespace
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Line comment: -- ... \n
        if bytes[i..].starts_with(b"--") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // Nested block comment: {- ... -}
        if bytes[i..].starts_with(b"{-") {
            i += 2;
            let mut depth = 1usize;
            while i < bytes.len() && depth > 0 {
                if bytes[i..].starts_with(b"{-") {
                    depth += 1;
                    i += 2;
                } else if bytes[i..].starts_with(b"-}") {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }

        // Punctuation
        if bytes[i] == b'=' {
            tokens.push(Token {
                kind: TokenKind::Eq,
            });
            i += 1;
            continue;
        }
        if bytes[i] == b'|' {
            tokens.push(Token {
                kind: TokenKind::Pipe,
            });
            i += 1;
            continue;
        }
        if bytes[i] == b'\\' {
            tokens.push(Token {
                kind: TokenKind::Backslash,
            });
            i += 1;
            continue;
        }
        if bytes[i..].starts_with(b"->") {
            tokens.push(Token {
                kind: TokenKind::Arrow,
            });
            i += 2;
            continue;
        }
        if bytes[i] == b'[' {
            tokens.push(Token {
                kind: TokenKind::LBracket,
            });
            i += 1;
            continue;
        }
        if bytes[i] == b']' {
            tokens.push(Token {
                kind: TokenKind::RBracket,
            });
            i += 1;
            continue;
        }

        // String literal
        if bytes[i] == b'"' {
            i += 1;
            let start = i;
            let mut s = String::new();
            while i < bytes.len() {
                match bytes[i] {
                    b'"' => break,
                    b'\\' => {
                        i += 1;
                        if i >= bytes.len() {
                            break;
                        }
                        let ch = match bytes[i] {
                            b'n' => '\n',
                            b't' => '\t',
                            b'r' => '\r',
                            b'"' => '"',
                            b'\\' => '\\',
                            other => other as char,
                        };
                        s.push(ch);
                        i += 1;
                    }
                    other => {
                        s.push(other as char);
                        i += 1;
                    }
                }
            }
            if i == start {
                // empty string
            }
            if i < bytes.len() && bytes[i] == b'"' {
                i += 1;
            }
            tokens.push(Token {
                kind: TokenKind::String(s),
            });
            continue;
        }

        // Number literal: integer or float
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }

            let mut is_float = false;

            // fractional part
            if i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
                is_float = true;
                i += 1; // '.'
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
            }

            // exponent
            if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
                is_float = true;
                let exp_start = i;
                i += 1;
                if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                    i += 1;
                }
                let digits_start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if digits_start == i {
                    // invalid exponent; roll back to before 'e'
                    i = exp_start;
                }
            }

            let text = &src[start..i];
            tokens.push(Token {
                kind: if is_float {
                    TokenKind::Float(text.to_string())
                } else {
                    TokenKind::Integer(text.to_string())
                },
            });
            continue;
        }

        // Identifier / keyword
        let c = bytes[i] as char;
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            i += 1;
            while i < bytes.len() {
                let ch = bytes[i] as char;
                if ch.is_ascii_alphanumeric() || ch == '_' {
                    i += 1;
                } else {
                    break;
                }
            }
            let word = &src[start..i];
            let kind = match word {
                "True" => TokenKind::True,
                "False" => TokenKind::False,
                "if" => TokenKind::KwIf,
                "then" => TokenKind::KwThen,
                "else" => TokenKind::KwElse,
                "type" => TokenKind::KwType,
                "data" => TokenKind::KwData,
                _ => TokenKind::Ident(word.to_string()),
            };
            tokens.push(Token { kind });
            continue;
        }

        // Unknown byte: skip for now.
        i += 1;
    }

    tokens
}
