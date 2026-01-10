#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Ident(String),
    Integer(String),
    Float(String),
    String(String),
    Char(char),
    True,
    False,

    KwModule,
    KwWhere,
    KwImport,
    KwExport,
    KwLet,
    KwIn,
    KwCase,
    KwOf,
    KwDo,
    KwIf,
    KwThen,
    KwElse,
    KwType,
    KwData,
    KwInfix,
    KwInfixl,
    KwInfixr,

    Newline,
    Indent,
    Dedent,
    Comma,
    Dot,
    Backtick,
    Plus,
    PlusPlus,
    Minus,
    Star,
    Slash,
    EqEq,
    SlashEq,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,
    FatArrow,
    Eq,
    Pipe,
    Backslash,
    Arrow,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Colon,
    ColonColon,
    Ellipsis,
    At,
    Question,
    LeftArrow,
    Semicolon,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
}

pub fn lex(src: &str) -> crate::Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0usize;

    let mut indent_stack: Vec<usize> = vec![0];
    let mut bol = true;

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
        if bol {
            let mut col = 0usize;
            while i < bytes.len() {
                match bytes[i] {
                    b' ' => {
                        col += 1;
                        i += 1;
                    }
                    b'\t' => {
                        col += 4;
                        i += 1;
                    }
                    _ => break,
                }
            }

            // Do not change indentation on blank/comment-only lines.
            if i >= bytes.len()
                || bytes[i] == b'\n'
                || bytes[i..].starts_with(b"--")
                || bytes[i..].starts_with(b"{-")
            {
                bol = false;
            } else {
                let current = *indent_stack.last().unwrap_or(&0);
                match col.cmp(&current) {
                    std::cmp::Ordering::Greater => {
                        indent_stack.push(col);
                        tokens.push(Token {
                            kind: TokenKind::Indent,
                        });
                    }
                    std::cmp::Ordering::Less => {
                        while indent_stack.len() > 1 && col < *indent_stack.last().unwrap() {
                            indent_stack.pop();
                            tokens.push(Token {
                                kind: TokenKind::Dedent,
                            });
                        }
                        if col != *indent_stack.last().unwrap() {
                            return Err(crate::error::Error::msg("inconsistent indentation"));
                        }
                    }
                    std::cmp::Ordering::Equal => {}
                }
                bol = false;
            }
        }

        // Newline (statement separator)
        if bytes[i] == b'\n' {
            tokens.push(Token {
                kind: TokenKind::Newline,
            });
            i += 1;
            bol = true;
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

        // Punctuation (multi-char first)
        if bytes[i..].starts_with(b"->") {
            tokens.push(Token {
                kind: TokenKind::Arrow,
            });
            i += 2;
            continue;
        }
        if bytes[i..].starts_with(b"<-") {
            tokens.push(Token {
                kind: TokenKind::LeftArrow,
            });
            i += 2;
            continue;
        }
        if bytes[i..].starts_with(b"=>") {
            tokens.push(Token {
                kind: TokenKind::FatArrow,
            });
            i += 2;
            continue;
        }
        if bytes[i..].starts_with(b"==") {
            tokens.push(Token {
                kind: TokenKind::EqEq,
            });
            i += 2;
            continue;
        }
        if bytes[i..].starts_with(b"/=") {
            tokens.push(Token {
                kind: TokenKind::SlashEq,
            });
            i += 2;
            continue;
        }
        if bytes[i..].starts_with(b"<=") {
            tokens.push(Token {
                kind: TokenKind::Le,
            });
            i += 2;
            continue;
        }
        if bytes[i..].starts_with(b">=") {
            tokens.push(Token {
                kind: TokenKind::Ge,
            });
            i += 2;
            continue;
        }
        if bytes[i..].starts_with(b"&&") {
            tokens.push(Token {
                kind: TokenKind::AndAnd,
            });
            i += 2;
            continue;
        }
        if bytes[i..].starts_with(b"||") {
            tokens.push(Token {
                kind: TokenKind::OrOr,
            });
            i += 2;
            continue;
        }
        if bytes[i..].starts_with(b"++") {
            tokens.push(Token {
                kind: TokenKind::PlusPlus,
            });
            i += 2;
            continue;
        }
        if bytes[i..].starts_with(b"...") {
            tokens.push(Token {
                kind: TokenKind::Ellipsis,
            });
            i += 3;
            continue;
        }
        if bytes[i] == b';' {
            tokens.push(Token {
                kind: TokenKind::Semicolon,
            });
            i += 1;
            continue;
        }
        if bytes[i] == b'.' {
            tokens.push(Token {
                kind: TokenKind::Dot,
            });
            i += 1;
            continue;
        }

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
        if bytes[i] == b'<' {
            tokens.push(Token {
                kind: TokenKind::Lt,
            });
            i += 1;
            continue;
        }
        if bytes[i] == b'>' {
            tokens.push(Token {
                kind: TokenKind::Gt,
            });
            i += 1;
            continue;
        }
        if bytes[i] == b',' {
            tokens.push(Token {
                kind: TokenKind::Comma,
            });
            i += 1;
            continue;
        }
        if bytes[i] == b'`' {
            tokens.push(Token {
                kind: TokenKind::Backtick,
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
        if bytes[i] == b'+' {
            tokens.push(Token {
                kind: TokenKind::Plus,
            });
            i += 1;
            continue;
        }
        if bytes[i] == b'-' {
            tokens.push(Token {
                kind: TokenKind::Minus,
            });
            i += 1;
            continue;
        }
        if bytes[i] == b'*' {
            tokens.push(Token {
                kind: TokenKind::Star,
            });
            i += 1;
            continue;
        }
        if bytes[i] == b'/' {
            tokens.push(Token {
                kind: TokenKind::Slash,
            });
            i += 1;
            continue;
        }
        if bytes[i] == b'(' {
            tokens.push(Token {
                kind: TokenKind::LParen,
            });
            i += 1;
            continue;
        }
        if bytes[i] == b')' {
            tokens.push(Token {
                kind: TokenKind::RParen,
            });
            i += 1;
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
        if bytes[i] == b'{' {
            tokens.push(Token {
                kind: TokenKind::LBrace,
            });
            i += 1;
            continue;
        }
        if bytes[i] == b'}' {
            tokens.push(Token {
                kind: TokenKind::RBrace,
            });
            i += 1;
            continue;
        }
        if bytes[i..].starts_with(b"::") {
            tokens.push(Token {
                kind: TokenKind::ColonColon,
            });
            i += 2;
            continue;
        }
        if bytes[i] == b':' {
            tokens.push(Token {
                kind: TokenKind::Colon,
            });
            i += 1;
            continue;
        }
        if bytes[i] == b'@' {
            tokens.push(Token { kind: TokenKind::At });
            i += 1;
            continue;
        }
        if bytes[i] == b'?' {
            tokens.push(Token {
                kind: TokenKind::Question,
            });
            i += 1;
            continue;
        }

        // Char literal
        if bytes[i] == b'\'' {
            i += 1;
            if i >= bytes.len() {
                return Err(crate::error::Error::msg("unterminated char literal"));
            }

            let ch = if bytes[i] == b'\\' {
                i += 1;
                if i >= bytes.len() {
                    return Err(crate::error::Error::msg("unterminated char literal"));
                }
                let ch = match bytes[i] {
                    b'n' => '\n',
                    b't' => '\t',
                    b'r' => '\r',
                    b'\'' => '\'',
                    b'\\' => '\\',
                    other => other as char,
                };
                i += 1;
                ch
            } else {
                let s = &src[i..];
                let ch = s
                    .chars()
                    .next()
                    .ok_or_else(|| crate::error::Error::msg("unterminated char literal"))?;
                i += ch.len_utf8();
                ch
            };

            if i >= bytes.len() || bytes[i] != b'\'' {
                return Err(crate::error::Error::msg("unterminated char literal"));
            }
            i += 1;

            tokens.push(Token {
                kind: TokenKind::Char(ch),
            });
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
                "module" => TokenKind::KwModule,
                "where" => TokenKind::KwWhere,
                "import" => TokenKind::KwImport,
                "export" => TokenKind::KwExport,
                "let" => TokenKind::KwLet,
                "in" => TokenKind::KwIn,
                "case" => TokenKind::KwCase,
                "of" => TokenKind::KwOf,
                "do" => TokenKind::KwDo,
                "if" => TokenKind::KwIf,
                "then" => TokenKind::KwThen,
                "else" => TokenKind::KwElse,
                "type" => TokenKind::KwType,
                "data" => TokenKind::KwData,
                "infix" => TokenKind::KwInfix,
                "infixl" => TokenKind::KwInfixl,
                "infixr" => TokenKind::KwInfixr,
                _ => TokenKind::Ident(word.to_string()),
            };
            tokens.push(Token { kind });
            continue;
        }

        // Unknown byte: skip for now.
        i += 1;
    }

    // Close any remaining indentation at EOF.
    while indent_stack.len() > 1 {
        indent_stack.pop();
        tokens.push(Token {
            kind: TokenKind::Dedent,
        });
    }

    Ok(tokens)
}
