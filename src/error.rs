use std::fmt;

use crate::lexer::Span;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Msg(String),
    MsgWithSpan { msg: String, span: Span },
}

impl Error {
    pub fn msg(s: impl Into<String>) -> Self {
        Self::Msg(s.into())
    }

    pub fn msg_with_span(s: impl Into<String>, span: Span) -> Self {
        Self::MsgWithSpan {
            msg: s.into(),
            span,
        }
    }

    pub fn span(&self) -> Option<Span> {
        match self {
            Error::MsgWithSpan { span, .. } => Some(*span),
            _ => None,
        }
    }

    pub fn with_context(self, ctx: impl fmt::Display) -> Self {
        let old = self.to_string();
        let msg = format!("{ctx}: {old}");
        match self {
            Error::MsgWithSpan { span, .. } => Error::msg_with_span(msg, span),
            _ => Error::msg(msg),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Msg(s) => write!(f, "{s}"),
            Error::MsgWithSpan { msg, .. } => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
