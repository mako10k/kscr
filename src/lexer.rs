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

fn is_ident_start_byte(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_continue_byte(b: u8) -> bool {
    is_ident_start_byte(b) || b.is_ascii_digit()
}

fn is_operator_byte(b: u8) -> bool {
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
            | b'.'
            | b'<'
            | b'='
            | b'>'
            | b'^'
            | b'|'
            | b'~'
            | b':'
    )
}

fn push_token(tokens: &mut Vec<Token>, kind: TokenKind, start: usize, end: usize) {
    tokens.push(Token {
        kind,
        span: Span { start, end },
    });
}

fn skip_shebang(bytes: &[u8], i: &mut usize) {
    if bytes.starts_with(b"#!") {
        while *i < bytes.len() && bytes[*i] != b'\n' {
            *i += 1;
        }
        if *i < bytes.len() && bytes[*i] == b'\n' {
            *i += 1;
        }
    }
}

fn handle_bol_indentation(
    bytes: &[u8],
    i: &mut usize,
    indent_stack: &mut Vec<usize>,
    tokens: &mut Vec<Token>,
) -> crate::Result<bool> {
    let mut col = 0usize;
    while *i < bytes.len() {
        match bytes[*i] {
            b' ' => {
                col += 1;
                *i += 1;
            }
            b'\t' => {
                col += 4;
                *i += 1;
            }
            _ => break,
        }
    }

    // Do not change indentation on blank/comment-only lines.
    if *i >= bytes.len() || bytes[*i] == b'\n' || bytes[*i..].starts_with(b"--") || bytes[*i..].starts_with(b"{-") {
        return Ok(false);
    }

    let current = *indent_stack.last().unwrap_or(&0);
    match col.cmp(&current) {
        std::cmp::Ordering::Greater => {
            indent_stack.push(col);
            push_token(tokens, TokenKind::Indent, *i, *i);
        }
        std::cmp::Ordering::Less => {
            while indent_stack.len() > 1 && col < *indent_stack.last().unwrap() {
                indent_stack.pop();
                push_token(tokens, TokenKind::Dedent, *i, *i);
            }
            if col != *indent_stack.last().unwrap() {
                return Err(crate::error::Error::msg_with_span(
                    "inconsistent indentation",
                    Span { start: *i, end: *i },
                ));
            }
        }
        std::cmp::Ordering::Equal => {}
    }

    Ok(false)
}

fn skip_line_comment(bytes: &[u8], i: &mut usize) {
    while *i < bytes.len() && bytes[*i] != b'\n' {
        *i += 1;
    }
}

fn skip_block_comment(bytes: &[u8], i: &mut usize) {
    *i += 2;
    let mut depth = 1usize;
    while *i < bytes.len() && depth > 0 {
        if bytes[*i..].starts_with(b"{-") {
            depth += 1;
            *i += 2;
        } else if bytes[*i..].starts_with(b"-}") {
            depth -= 1;
            *i += 2;
        } else {
            *i += 1;
        }
    }
}

fn dot_is_qualification(bytes: &[u8], i: usize) -> bool {
    if bytes.get(i) != Some(&b'.') {
        return false;
    }
    let prev = if i > 0 { bytes[i - 1] } else { b' ' };
    let next = if i + 1 < bytes.len() { bytes[i + 1] } else { b' ' };
    is_ident_continue_byte(prev) && is_ident_start_byte(next)
}

fn operator_token_kind(op: &str) -> TokenKind {
    match op {
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
        "..." => TokenKind::Ellipsis,
        ":" => TokenKind::Colon,
        "+" => TokenKind::Plus,
        "-" => TokenKind::Minus,
        "*" => TokenKind::Star,
        "/" => TokenKind::Slash,
        "<" => TokenKind::Lt,
        ">" => TokenKind::Gt,
        "=" => TokenKind::Eq,
        "|" => TokenKind::Pipe,
        _ => TokenKind::Operator(op.to_string()),
    }
}

fn lex_operator_run(src: &str, bytes: &[u8], i: &mut usize) -> Option<(TokenKind, usize, usize)> {
    if *i >= bytes.len() {
        return None;
    }
    if bytes[*i] == b'.' && dot_is_qualification(bytes, *i) {
        let start = *i;
        *i += 1;
        return Some((TokenKind::Dot, start, *i));
    }
    if !is_operator_byte(bytes[*i]) {
        return None;
    }

    let start = *i;
    *i += 1;
    while *i < bytes.len() && is_operator_byte(bytes[*i]) {
        *i += 1;
    }
    let op = &src[start..*i];
    Some((operator_token_kind(op), start, *i))
}

fn lex_char_literal(src: &str, bytes: &[u8], i: &mut usize) -> crate::Result<Option<(TokenKind, usize, usize)>> {
    if *i >= bytes.len() || bytes[*i] != b'\'' {
        return Ok(None);
    }
    let start = *i;
    *i += 1;
    if *i >= bytes.len() {
        return Err(crate::error::Error::msg_with_span(
            "unterminated char literal",
            Span { start, end: *i },
        ));
    }

    let ch = if bytes[*i] == b'\\' {
        *i += 1;
        if *i >= bytes.len() {
            return Err(crate::error::Error::msg_with_span(
                "unterminated char literal",
                Span { start, end: *i },
            ));
        }
        let ch = match bytes[*i] {
            b'n' => '\n',
            b't' => '\t',
            b'r' => '\r',
            b'\'' => '\'',
            b'\\' => '\\',
            other => other as char,
        };
        *i += 1;
        ch
    } else {
        let s = &src[*i..];
        let ch = s.chars().next().ok_or_else(|| {
            crate::error::Error::msg_with_span(
                "unterminated char literal",
                Span { start, end: *i },
            )
        })?;
        *i += ch.len_utf8();
        ch
    };

    if *i >= bytes.len() || bytes[*i] != b'\'' {
        return Err(crate::error::Error::msg_with_span(
            "unterminated char literal",
            Span { start, end: *i },
        ));
    }
    *i += 1;

    Ok(Some((TokenKind::Char(ch), start, *i)))
}

fn lex_string_literal(bytes: &[u8], i: &mut usize) -> Option<(TokenKind, usize, usize)> {
    if *i >= bytes.len() || bytes[*i] != b'"' {
        return None;
    }
    let start = *i;
    *i += 1;
    let mut s = String::new();
    while *i < bytes.len() {
        match bytes[*i] {
            b'"' => break,
            b'\\' => {
                *i += 1;
                if *i >= bytes.len() {
                    break;
                }
                let ch = match bytes[*i] {
                    b'n' => '\n',
                    b't' => '\t',
                    b'r' => '\r',
                    b'"' => '"',
                    b'\\' => '\\',
                    other => other as char,
                };
                s.push(ch);
                *i += 1;
            }
            other => {
                s.push(other as char);
                *i += 1;
            }
        }
    }
    if *i < bytes.len() && bytes[*i] == b'"' {
        *i += 1;
    }
    Some((TokenKind::String(s), start, *i))
}

fn lex_number_literal(src: &str, bytes: &[u8], i: &mut usize) -> Option<(TokenKind, usize, usize)> {
    if *i >= bytes.len() || !bytes[*i].is_ascii_digit() {
        return None;
    }
    let start = *i;
    while *i < bytes.len() && bytes[*i].is_ascii_digit() {
        *i += 1;
    }

    let mut is_float = false;
    if *i + 1 < bytes.len() && bytes[*i] == b'.' && bytes[*i + 1].is_ascii_digit() {
        is_float = true;
        *i += 1;
        while *i < bytes.len() && bytes[*i].is_ascii_digit() {
            *i += 1;
        }
    }

    if *i < bytes.len() && (bytes[*i] == b'e' || bytes[*i] == b'E') {
        is_float = true;
        let exp_start = *i;
        *i += 1;
        if *i < bytes.len() && (bytes[*i] == b'+' || bytes[*i] == b'-') {
            *i += 1;
        }
        let digits_start = *i;
        while *i < bytes.len() && bytes[*i].is_ascii_digit() {
            *i += 1;
        }
        if digits_start == *i {
            *i = exp_start;
        }
    }

    let text = &src[start..*i];
    let kind = if is_float {
        TokenKind::Float(text.to_string())
    } else {
        TokenKind::Integer(text.to_string())
    };
    Some((kind, start, *i))
}

fn keyword_or_ident(word: &str) -> TokenKind {
    match word {
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
    }
}

fn lex_ident_or_keyword(src: &str, bytes: &[u8], i: &mut usize) -> Option<(TokenKind, usize, usize)> {
    if *i >= bytes.len() {
        return None;
    }
    let c = bytes[*i] as char;
    if !(c.is_ascii_alphabetic() || c == '_') {
        return None;
    }
    let start = *i;
    *i += 1;
    while *i < bytes.len() {
        let ch = bytes[*i] as char;
        if ch.is_ascii_alphanumeric() || ch == '_' {
            *i += 1;
        } else {
            break;
        }
    }
    let word = &src[start..*i];
    Some((keyword_or_ident(word), start, *i))
}

fn lex_punctuation(bytes: &[u8], i: &mut usize) -> Option<(TokenKind, usize, usize)> {
    if *i >= bytes.len() {
        return None;
    }

    let start = *i;

    if bytes[*i..].starts_with(b"->") {
        *i += 2;
        return Some((TokenKind::Arrow, start, *i));
    }
    if bytes[*i..].starts_with(b"<-") {
        *i += 2;
        return Some((TokenKind::LeftArrow, start, *i));
    }
    if bytes[*i..].starts_with(b"=>") {
        *i += 2;
        return Some((TokenKind::FatArrow, start, *i));
    }
    if bytes[*i..].starts_with(b"==") {
        *i += 2;
        return Some((TokenKind::EqEq, start, *i));
    }
    if bytes[*i..].starts_with(b"/=") {
        *i += 2;
        return Some((TokenKind::SlashEq, start, *i));
    }
    if bytes[*i..].starts_with(b"<=") {
        *i += 2;
        return Some((TokenKind::Le, start, *i));
    }
    if bytes[*i..].starts_with(b">=") {
        *i += 2;
        return Some((TokenKind::Ge, start, *i));
    }
    if bytes[*i..].starts_with(b">>=") {
        *i += 3;
        return Some((TokenKind::GtGtEq, start, *i));
    }
    if bytes[*i..].starts_with(b">>") {
        *i += 2;
        return Some((TokenKind::GtGt, start, *i));
    }
    if bytes[*i..].starts_with(b"&&") {
        *i += 2;
        return Some((TokenKind::AndAnd, start, *i));
    }
    if bytes[*i..].starts_with(b"||") {
        *i += 2;
        return Some((TokenKind::OrOr, start, *i));
    }
    if bytes[*i..].starts_with(b"++") {
        *i += 2;
        return Some((TokenKind::PlusPlus, start, *i));
    }
    if bytes[*i..].starts_with(b"...") {
        *i += 3;
        return Some((TokenKind::Ellipsis, start, *i));
    }

    let kind = match bytes[*i] {
        b';' => TokenKind::Semicolon,
        b'=' => TokenKind::Eq,
        b'|' => TokenKind::Pipe,
        b'<' => TokenKind::Lt,
        b'>' => TokenKind::Gt,
        b',' => TokenKind::Comma,
        b'`' => TokenKind::Backtick,
        b'\\' => TokenKind::Backslash,
        b'+' => TokenKind::Plus,
        b'-' => TokenKind::Minus,
        b'*' => TokenKind::Star,
        b'/' => TokenKind::Slash,
        b'(' => TokenKind::LParen,
        b')' => TokenKind::RParen,
        b'[' => TokenKind::LBracket,
        b']' => TokenKind::RBracket,
        b'{' => TokenKind::LBrace,
        b'}' => TokenKind::RBrace,
        b':' => TokenKind::Colon,
        b'@' => TokenKind::At,
        b'?' => TokenKind::Question,
        _ => return None,
    };
    *i += 1;
    Some((kind, start, *i))
}

pub fn lex(src: &str) -> crate::Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0usize;

    let mut indent_stack: Vec<usize> = vec![0];
    let mut bol = true;

    skip_shebang(bytes, &mut i);

    while i < bytes.len() {
        if bol {
            bol = handle_bol_indentation(bytes, &mut i, &mut indent_stack, &mut tokens)?;
        }

        if i >= bytes.len() {
            break;
        }

        if bytes[i] == b'\n' {
            let start = i;
            i += 1;
            push_token(&mut tokens, TokenKind::Newline, start, i);
            bol = true;
            continue;
        }

        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }

        if bytes[i..].starts_with(b"--") {
            skip_line_comment(bytes, &mut i);
            continue;
        }

        if bytes[i..].starts_with(b"{-") {
            skip_block_comment(bytes, &mut i);
            continue;
        }

        if let Some((kind, start, end)) = lex_operator_run(src, bytes, &mut i) {
            push_token(&mut tokens, kind, start, end);
            continue;
        }

        if let Some((kind, start, end)) = lex_punctuation(bytes, &mut i) {
            push_token(&mut tokens, kind, start, end);
            continue;
        }

        if let Some((kind, start, end)) = lex_char_literal(src, bytes, &mut i)? {
            push_token(&mut tokens, kind, start, end);
            continue;
        }

        if let Some((kind, start, end)) = lex_string_literal(bytes, &mut i) {
            push_token(&mut tokens, kind, start, end);
            continue;
        }

        if let Some((kind, start, end)) = lex_number_literal(src, bytes, &mut i) {
            push_token(&mut tokens, kind, start, end);
            continue;
        }

        if let Some((kind, start, end)) = lex_ident_or_keyword(src, bytes, &mut i) {
            push_token(&mut tokens, kind, start, end);
            continue;
        }

        i += 1;
    }

    while indent_stack.len() > 1 {
        indent_stack.pop();
        push_token(&mut tokens, TokenKind::Dedent, i, i);
    }

    Ok(tokens)
}
