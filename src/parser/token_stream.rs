use crate::{error::Error, lexer, lexer::TokenKind, Result};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Stop {
    Then,
    Else,
    Of,
    LetBind,
    SemiOrRBrace,
    Pattern,
    Arrow,
    LineEnd,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Assoc {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Fixity {
    pub(crate) prec: u8,
    pub(crate) assoc: Assoc,
}

pub(crate) fn default_fixity(op: &str) -> Fixity {
    match op {
        "." => Fixity {
            // Haskell-like: function composition is high precedence, right associative.
            prec: 90,
            assoc: Assoc::Right,
        },
        "*" | "/" => Fixity {
            prec: 70,
            assoc: Assoc::Left,
        },
        "+" | "-" => Fixity {
            prec: 60,
            assoc: Assoc::Left,
        },
        "++" => Fixity {
            // Haskell-like: (++) is right associative and slightly lower precedence than (+).
            prec: 55,
            assoc: Assoc::Right,
        },
        "==" | "/=" | "<" | "<=" | ">" | ">=" => Fixity {
            prec: 50,
            assoc: Assoc::Left,
        },
        "&&" => Fixity {
            prec: 40,
            assoc: Assoc::Left,
        },
        "||" => Fixity {
            prec: 30,
            assoc: Assoc::Left,
        },
        ">>=" | ">>" => Fixity {
            // Haskell-like: sequencing is very low precedence.
            prec: 10,
            assoc: Assoc::Left,
        },
        "$" => Fixity {
            // Haskell-like: application is very low precedence, right associative.
            prec: 0,
            assoc: Assoc::Right,
        },
        _ => Fixity {
            // Default used for backtick infix, and for any other operator-like names.
            prec: 60,
            assoc: Assoc::Left,
        },
    }
}

pub(crate) fn compute_line_starts(src: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (i, b) in src.as_bytes().iter().enumerate() {
        if *b == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

fn line_col(line_starts: &[usize], offset: usize) -> (usize, usize) {
    // line_starts contains the byte offset of the first byte of each line.
    // Line numbers are 1-based.
    let line_idx = match line_starts.binary_search(&offset) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    };
    let line_start = *line_starts.get(line_idx).unwrap_or(&0);
    (line_idx + 1, offset.saturating_sub(line_start) + 1)
}

pub(crate) struct TokenStream {
    pub(crate) tokens: Vec<lexer::Token>,
    pub(crate) i: usize,
    pub(crate) last_span_end: usize,
    pub(crate) gensym: u32,
    pub(crate) fixities: HashMap<String, Fixity>,
    pub(crate) line_starts: Vec<usize>,
}

impl TokenStream {
    pub(crate) fn new(
        tokens: Vec<lexer::Token>,
        fixities: HashMap<String, Fixity>,
        line_starts: Vec<usize>,
    ) -> Self {
        Self {
            tokens,
            i: 0,
            last_span_end: 0,
            gensym: 0,
            fixities,
            line_starts,
        }
    }

    pub(crate) fn peek_span(&self) -> Option<lexer::Span> {
        self.tokens.get(self.i).map(|t| t.span)
    }

    pub(crate) fn span_from(&self, start: usize) -> lexer::Span {
        lexer::Span {
            start,
            end: self.last_span_end.max(start),
        }
    }

    pub(crate) fn pos_str_at(&self, offset: usize) -> String {
        let (line, col) = line_col(&self.line_starts, offset);
        format!("{line}:{col}")
    }

    pub(crate) fn pos_str_here(&self) -> String {
        let offset = self
            .tokens
            .get(self.i)
            .map(|t| t.span.start)
            .unwrap_or_else(|| self.tokens.last().map(|t| t.span.end).unwrap_or(0));
        self.pos_str_at(offset)
    }

    pub(crate) fn err_here(&self, msg: impl std::fmt::Display) -> Error {
        let span = self.peek_span().unwrap_or(lexer::Span {
            start: self.last_span_end,
            end: self.last_span_end,
        });
        Error::msg_with_span(format!("{msg} at {}", self.pos_str_here()), span)
    }

    pub(crate) fn fixity(&self, op: &str) -> Fixity {
        self.fixities
            .get(op)
            .copied()
            .unwrap_or_else(|| default_fixity(op))
    }

    pub(crate) fn fresh_name(&mut self, prefix: &str) -> String {
        let n = self.gensym;
        self.gensym += 1;
        format!("{prefix}{n}")
    }

    pub(crate) fn is_eof(&self) -> bool {
        self.i >= self.tokens.len()
    }

    pub(crate) fn peek_kind(&self) -> Option<&TokenKind> {
        self.tokens.get(self.i).map(|t| &t.kind)
    }

    pub(crate) fn bump(&mut self) -> Option<TokenKind> {
        let tok = self.tokens.get(self.i)?;
        self.last_span_end = tok.span.end;
        let t = tok.kind.clone();
        self.i += 1;
        Some(t)
    }

    pub(crate) fn expect(&mut self, kind: TokenKind) -> Result<()> {
        let pos = self.pos_str_here();
        let span = self.peek_span().unwrap_or(lexer::Span {
            start: self.last_span_end,
            end: self.last_span_end,
        });
        let got = self.bump().ok_or_else(|| {
            Error::msg_with_span(
                format!("unexpected EOF at {pos}, expected {kind:?}"),
                lexer::Span {
                    start: self.last_span_end,
                    end: self.last_span_end,
                },
            )
        })?;

        if got == kind {
            Ok(())
        } else {
            Err(Error::msg_with_span(
                format!("unexpected token {got:?} at {pos}, expected {kind:?}"),
                span,
            ))
        }
    }

    pub(crate) fn expect_ident(&mut self) -> Result<String> {
        let pos = self.pos_str_here();
        let span = self.peek_span().unwrap_or(lexer::Span {
            start: self.last_span_end,
            end: self.last_span_end,
        });
        match self.bump() {
            Some(TokenKind::Ident(s)) => Ok(s),
            Some(got) => Err(Error::msg_with_span(
                format!("expected identifier at {pos}, got {got:?}"),
                span,
            )),
            None => Err(Error::msg_with_span(
                format!("unexpected EOF at {pos}, expected identifier"),
                lexer::Span {
                    start: self.last_span_end,
                    end: self.last_span_end,
                },
            )),
        }
    }

    pub(crate) fn skip_newlines(&mut self) {
        while matches!(self.peek_kind(), Some(TokenKind::Newline)) {
            self.i += 1;
        }
    }

    pub(crate) fn consume_line_end(&mut self) {
        while matches!(self.peek_kind(), Some(TokenKind::Newline)) {
            self.i += 1;
        }
    }

    pub(crate) fn can_continue_expr(&self, stop: Stop) -> bool {
        match (stop, self.peek_kind()) {
            (_, None) => false,
            (_, Some(TokenKind::Newline)) => false,
            (Stop::Then, Some(TokenKind::KwThen)) => false,
            (Stop::Else, Some(TokenKind::KwElse)) => false,
            (Stop::Of, Some(TokenKind::KwOf)) => false,
            (Stop::LetBind, Some(TokenKind::Semicolon | TokenKind::KwIn)) => false,
            (Stop::SemiOrRBrace, Some(TokenKind::Semicolon | TokenKind::RBrace)) => false,
            (Stop::Arrow, Some(TokenKind::Arrow)) => false,
            (
                Stop::Pattern,
                Some(TokenKind::Arrow | TokenKind::Eq | TokenKind::Comma | TokenKind::Pipe),
            ) => false,
            // Case guards should accept normal operators like `==`. Do not treat `==` as a pattern
            // terminator; it's part of the guard expression.
            (Stop::Pattern, Some(TokenKind::EqEq)) => true,
            (Stop::Pattern, Some(TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace)) => {
                false
            }
            (Stop::Pattern, Some(TokenKind::Operator(op))) if op == ".." => false,
            (Stop::Pattern, Some(TokenKind::Dedent)) => false,
            (Stop::LineEnd, _) => true,
            _ => true,
        }
    }

    pub(crate) fn can_continue_pattern(&self) -> bool {
        !matches!(
            self.peek_kind(),
            None | Some(TokenKind::Newline)
                | Some(TokenKind::Dedent)
                | Some(TokenKind::Arrow)
                | Some(TokenKind::LeftArrow)
                | Some(TokenKind::Eq)
                | Some(TokenKind::ColonColon)
                | Some(TokenKind::Comma)
                | Some(TokenKind::Colon)
                | Some(TokenKind::Pipe)
                | Some(TokenKind::At)
                | Some(TokenKind::Ellipsis)
                | Some(TokenKind::RParen)
                | Some(TokenKind::RBracket)
                | Some(TokenKind::RBrace)
                // Stop at any operator token (to allow infix function clauses)
                | Some(TokenKind::Operator(_))
                | Some(TokenKind::Backtick)
                | Some(TokenKind::Plus)
                | Some(TokenKind::PlusPlus)
                | Some(TokenKind::Minus)
                | Some(TokenKind::Star)
                | Some(TokenKind::Slash)
                | Some(TokenKind::EqEq)
                | Some(TokenKind::SlashEq)
                | Some(TokenKind::Lt)
                | Some(TokenKind::Le)
                | Some(TokenKind::Gt)
                | Some(TokenKind::Ge)
                | Some(TokenKind::GtGt)
                | Some(TokenKind::GtGtEq)
                | Some(TokenKind::AndAnd)
                | Some(TokenKind::OrOr)
        )
    }
}
