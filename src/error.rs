use std::fmt;
use std::path::PathBuf;

use crate::lexer::Span;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceSpan {
    pub path: PathBuf,
    pub span: Span,
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Msg(String),
    MsgWithSpan { msg: String, span: Span },
    MsgWithSpans { msg: String, spans: Vec<Span> },
    MsgWithSourceSpans { msg: String, spans: Vec<SourceSpan> },
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

    pub fn msg_with_source_spans(s: impl Into<String>, spans: Vec<SourceSpan>) -> Self {
        Self::MsgWithSourceSpans {
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
                spans.push(span);
                Error::MsgWithSpans { msg, spans }
            }
            Error::Msg(msg) => Error::MsgWithSpan { msg, span },
            other => other,
        }
    }

    pub fn push_secondary_span(self, span: Span) -> Self {
        match self {
            Error::MsgWithSpan { msg, span: primary } => Error::MsgWithSpans {
                msg,
                spans: vec![primary, span],
            },
            Error::MsgWithSpans { msg, mut spans } => {
                spans.push(span);
                Error::MsgWithSpans { msg, spans }
            }
            // If we don't have a primary span yet, treat this as a single secondary span.
            // This avoids overriding future primary span decisions.
            Error::Msg(msg) => Error::MsgWithSpans {
                msg,
                spans: vec![span],
            },
            other => other,
        }
    }

    pub fn push_secondary_source_span(self, span: SourceSpan) -> Self {
        match self {
            Error::MsgWithSourceSpans { msg, mut spans } => {
                spans.push(span);
                Error::MsgWithSourceSpans { msg, spans }
            }
            Error::MsgWithSpan { msg, span: primary } => Error::MsgWithSourceSpans {
                msg,
                spans: vec![
                    SourceSpan {
                        path: PathBuf::new(),
                        span: primary,
                    },
                    span,
                ],
            },
            Error::MsgWithSpans { msg, spans } => Error::MsgWithSourceSpans {
                msg,
                spans: spans
                    .into_iter()
                    .map(|s| SourceSpan {
                        path: PathBuf::new(),
                        span: s,
                    })
                    .chain(std::iter::once(span))
                    .collect(),
            },
            Error::Msg(msg) => Error::MsgWithSourceSpans {
                msg,
                spans: vec![span],
            },
            other => other,
        }
    }

    pub fn span(&self) -> Option<Span> {
        match self {
            Error::MsgWithSpan { span, .. } => Some(*span),
            Error::MsgWithSpans { spans, .. } => spans.first().copied(),
            Error::MsgWithSourceSpans { spans, .. } => spans.first().map(|s| s.span),
            _ => None,
        }
    }

    pub fn spans(&self) -> Option<&[Span]> {
        match self {
            Error::MsgWithSpan { span, .. } => Some(std::slice::from_ref(span)),
            Error::MsgWithSpans { spans, .. } => Some(spans.as_slice()),
            // Not representable as pure spans without discarding path information.
            Error::MsgWithSourceSpans { .. } => None,
            _ => None,
        }
    }

    pub fn source_spans(&self) -> Option<&[SourceSpan]> {
        match self {
            Error::MsgWithSourceSpans { spans, .. } => Some(spans.as_slice()),
            _ => None,
        }
    }

    pub fn with_context(self, ctx: impl fmt::Display) -> Self {
        match self {
            Error::Msg(old) => Error::msg(format!("{ctx}: {old}")),
            Error::MsgWithSpan { msg: old, span } => {
                Error::msg_with_span(format!("{ctx}: {old}"), span)
            }
            Error::MsgWithSpans { msg: old, spans } => {
                Error::msg_with_spans(format!("{ctx}: {old}"), spans)
            }
            Error::MsgWithSourceSpans { msg: old, spans } => {
                Error::msg_with_source_spans(format!("{ctx}: {old}"), spans)
            }
            other => Error::msg(format!("{ctx}: {other}")),
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
            Error::MsgWithSourceSpans { msg, .. } => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
