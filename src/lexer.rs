#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Ident(String),
    Operator(String),
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
    KwClass,
    KwInstance,
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
    GtGt,
    GtGtEq,
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
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

pub fn lex(src: &str) -> crate::Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0usize;

    let is_operator_byte = |b: u8| -> bool {
        matches!(
            b,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'*'
                | b'+'
                | b'-'
                | b'/'
                | b'<'
                | b'='
                | b'>'
                | b'^'
                | b'|'
                | b'~'
                | b':'
        )
    };

    let mut push = |kind: TokenKind, start: usize, end: usize| {
        tokens.push(Token {
            kind,
            span: Span { start, end },
        });
    };

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
                        push(TokenKind::Indent, i, i);
                    }
                    std::cmp::Ordering::Less => {
                        while indent_stack.len() > 1 && col < *indent_stack.last().unwrap() {
                            indent_stack.pop();
                            push(TokenKind::Dedent, i, i);
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
            let start = i;
            i += 1;
            push(TokenKind::Newline, start, i);
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

        // Operator-like runs (Haskell-like): allow arbitrary symbol sequences,
        // except those starting with ':' (reserved for constructors).
        //
        // Note: this runs before punctuation matching so that operators like "+>" are
        // lexed as a single token instead of "+" then ">".
        if is_operator_byte(bytes[i]) {
            let start = i;
            i += 1;
            while i < bytes.len() && is_operator_byte(bytes[i]) {
                i += 1;
            }
            let op = &src[start..i];

            // Keep existing token kinds for well-known operators / punctuation-like symbols.
            let kind = match op {
                "->" => TokenKind::Arrow,
                "<-" => TokenKind::LeftArrow,
                "=>" => TokenKind::FatArrow,
                "==" => TokenKind::EqEq,
                "/=" => TokenKind::SlashEq,
                "<=" => TokenKind::Le,
                ">=" => TokenKind::Ge,
                ">>=" => TokenKind::GtGtEq,
                ">>" => TokenKind::GtGt,
                "&&" => TokenKind::AndAnd,
                "||" => TokenKind::OrOr,
                "++" => TokenKind::PlusPlus,
                "::" => TokenKind::ColonColon,
                ":" => TokenKind::Colon,
                "+" => TokenKind::Plus,
                "-" => TokenKind::Minus,
                "*" => TokenKind::Star,
                "/" => TokenKind::Slash,
                "<" => TokenKind::Lt,
                ">" => TokenKind::Gt,
                "=" => TokenKind::Eq,
                "|" => TokenKind::Pipe,
                _ => {
                    TokenKind::Operator(op.to_string())
                }
            };

            push(kind, start, i);
            continue;
        }

        // Punctuation (multi-char first)
        if bytes[i..].starts_with(b"->") {
            let start = i;
            i += 2;
            push(TokenKind::Arrow, start, i);
            continue;
        }
        if bytes[i..].starts_with(b"<-") {
            let start = i;
            i += 2;
            push(TokenKind::LeftArrow, start, i);
            continue;
        }
        if bytes[i..].starts_with(b"=>") {
            let start = i;
            i += 2;
            push(TokenKind::FatArrow, start, i);
            continue;
        }
        if bytes[i..].starts_with(b"==") {
            let start = i;
            i += 2;
            push(TokenKind::EqEq, start, i);
            continue;
        }
        if bytes[i..].starts_with(b"/=") {
            let start = i;
            i += 2;
            push(TokenKind::SlashEq, start, i);
            continue;
        }
        if bytes[i..].starts_with(b"<=") {
            let start = i;
            i += 2;
            push(TokenKind::Le, start, i);
            continue;
        }
        if bytes[i..].starts_with(b">=") {
            let start = i;
            i += 2;
            push(TokenKind::Ge, start, i);
            continue;
        }
        if bytes[i..].starts_with(b">>=") {
            let start = i;
            i += 3;
            push(TokenKind::GtGtEq, start, i);
            continue;
        }
        if bytes[i..].starts_with(b">>") {
            let start = i;
            i += 2;
            push(TokenKind::GtGt, start, i);
            continue;
        }
        if bytes[i..].starts_with(b"&&") {
            let start = i;
            i += 2;
            push(TokenKind::AndAnd, start, i);
            continue;
        }
        if bytes[i..].starts_with(b"||") {
            let start = i;
            i += 2;
            push(TokenKind::OrOr, start, i);
            continue;
        }
        if bytes[i..].starts_with(b"++") {
            let start = i;
            i += 2;
            push(TokenKind::PlusPlus, start, i);
            continue;
        }
        if bytes[i..].starts_with(b"...") {
            let start = i;
            i += 3;
            push(TokenKind::Ellipsis, start, i);
            continue;
        }
        if bytes[i] == b';' {
            let start = i;
            i += 1;
            push(TokenKind::Semicolon, start, i);
            continue;
        }
        if bytes[i] == b'.' {
            let start = i;
            i += 1;
            push(TokenKind::Dot, start, i);
            continue;
        }

        if bytes[i] == b'=' {
            let start = i;
            i += 1;
            push(TokenKind::Eq, start, i);
            continue;
        }
        if bytes[i] == b'|' {
            let start = i;
            i += 1;
            push(TokenKind::Pipe, start, i);
            continue;
        }
        if bytes[i] == b'<' {
            let start = i;
            i += 1;
            push(TokenKind::Lt, start, i);
            continue;
        }
        if bytes[i] == b'>' {
            let start = i;
            i += 1;
            push(TokenKind::Gt, start, i);
            continue;
        }
        if bytes[i] == b',' {
            let start = i;
            i += 1;
            push(TokenKind::Comma, start, i);
            continue;
        }
        if bytes[i] == b'`' {
            let start = i;
            i += 1;
            push(TokenKind::Backtick, start, i);
            continue;
        }
        if bytes[i] == b'\\' {
            let start = i;
            i += 1;
            push(TokenKind::Backslash, start, i);
            continue;
        }
        if bytes[i] == b'+' {
            let start = i;
            i += 1;
            push(TokenKind::Plus, start, i);
            continue;
        }
        if bytes[i] == b'-' {
            let start = i;
            i += 1;
            push(TokenKind::Minus, start, i);
            continue;
        }
        if bytes[i] == b'*' {
            let start = i;
            i += 1;
            push(TokenKind::Star, start, i);
            continue;
        }
        if bytes[i] == b'/' {
            let start = i;
            i += 1;
            push(TokenKind::Slash, start, i);
            continue;
        }
        if bytes[i] == b'(' {
            let start = i;
            i += 1;
            push(TokenKind::LParen, start, i);
            continue;
        }
        if bytes[i] == b')' {
            let start = i;
            i += 1;
            push(TokenKind::RParen, start, i);
            continue;
        }
        if bytes[i] == b'[' {
            let start = i;
            i += 1;
            push(TokenKind::LBracket, start, i);
            continue;
        }
        if bytes[i] == b']' {
            let start = i;
            i += 1;
            push(TokenKind::RBracket, start, i);
            continue;
        }
        if bytes[i] == b'{' {
            let start = i;
            i += 1;
            push(TokenKind::LBrace, start, i);
            continue;
        }
        if bytes[i] == b'}' {
            let start = i;
            i += 1;
            push(TokenKind::RBrace, start, i);
            continue;
        }
        if bytes[i..].starts_with(b"::") {
            let start = i;
            i += 2;
            push(TokenKind::ColonColon, start, i);
            continue;
        }
        if bytes[i] == b':' {
            let start = i;
            i += 1;
            push(TokenKind::Colon, start, i);
            continue;
        }
        if bytes[i] == b'@' {
            let start = i;
            i += 1;
            push(TokenKind::At, start, i);
            continue;
        }
        if bytes[i] == b'?' {
            let start = i;
            i += 1;
            push(TokenKind::Question, start, i);
            continue;
        }

        // Char literal
        if bytes[i] == b'\'' {
            let start = i;
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

            push(TokenKind::Char(ch), start, i);
            continue;
        }

        // String literal
        if bytes[i] == b'"' {
            let start = i;
            i += 1;
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
            push(TokenKind::String(s), start, i);
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
            let kind = if is_float {
                TokenKind::Float(text.to_string())
            } else {
                TokenKind::Integer(text.to_string())
            };
            push(kind, start, i);
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
                "class" => TokenKind::KwClass,
                "instance" => TokenKind::KwInstance,
                "infix" => TokenKind::KwInfix,
                "infixl" => TokenKind::KwInfixl,
                "infixr" => TokenKind::KwInfixr,
                _ => TokenKind::Ident(word.to_string()),
            };
            push(kind, start, i);
            continue;
        }

        // Unknown byte: skip for now.
        i += 1;
    }

    // Close any remaining indentation at EOF.
    while indent_stack.len() > 1 {
        indent_stack.pop();
        push(TokenKind::Dedent, i, i);
    }

    Ok(tokens)
}
