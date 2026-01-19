use std::fmt;

use crate::lexer::Span;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Msg(String),
    MsgWithSpan { msg: String, span: Span },
    MsgWithSpans { msg: String, spans: Vec<Span> },
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

    pub fn msg_with_spans(s: impl Into<String>, spans: Vec<Span>) -> Self {
        Self::MsgWithSpans {
            msg: s.into(),
            spans,
        }
    }

    pub fn push_span(self, span: Span) -> Self {
        match self {
            Error::MsgWithSpan { msg, span: s } => Error::MsgWithSpans {
                msg,
                spans: vec![span, s],
            },
            Error::MsgWithSpans { msg, mut spans } => {
                spans.insert(0, span);
                Error::MsgWithSpans { msg, spans }
            }
            other => other,
        }
    }

    pub fn span(&self) -> Option<Span> {
        match self {
            Error::MsgWithSpan { span, .. } => Some(*span),
            Error::MsgWithSpans { spans, .. } => spans.first().copied(),
            _ => None,
        }
    }

    pub fn spans(&self) -> Option<&[Span]> {
        match self {
            Error::MsgWithSpan { span, .. } => Some(std::slice::from_ref(span)),
            Error::MsgWithSpans { spans, .. } => Some(spans.as_slice()),
            _ => None,
        }
    }

    pub fn with_context(self, ctx: impl fmt::Display) -> Self {
        let old = self.to_string();
        let msg = format!("{ctx}: {old}");
        match self {
            Error::MsgWithSpan { span, .. } => Error::msg_with_span(msg, span),
            Error::MsgWithSpans { spans, .. } => Error::msg_with_spans(msg, spans),
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
            Error::MsgWithSpans { msg, .. } => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
