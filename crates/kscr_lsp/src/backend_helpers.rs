use crate::vfs::Document;
use kscr::{error::Error as KscrError, lexer};
use tower_lsp::lsp_types::*;

pub(super) fn span_to_range(doc: &Document, span: kscr::lexer::Span) -> Option<Range> {
    let len = doc.text.len();
    let start_off = span.start.min(len);
    let mut end_off = span.end.min(len);

    if end_off < start_off {
        end_off = start_off;
    }
    if end_off == start_off && end_off < len {
        end_off += 1;
    }

    let (sl, sc) = doc.offset_to_position(start_off)?;
    let (el, ec) = doc.offset_to_position(end_off)?;

    Some(Range {
        start: Position {
            line: sl,
            character: sc,
        },
        end: Position {
            line: el,
            character: ec,
        },
    })
}

pub(super) fn create_diagnostic(
    doc: &Document,
    err: &KscrError,
    severity: DiagnosticSeverity,
) -> Diagnostic {
    let primary_span = err.spans().and_then(|spans| {
        spans
            .iter()
            .copied()
            .find(|s| s.start < s.end)
            .or_else(|| spans.first().copied())
    });

    let range = primary_span
        .and_then(|s| span_to_range(doc, s))
        .unwrap_or(Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 0,
                character: 0,
            },
        });

    let zero_range = Range {
        start: Position {
            line: 0,
            character: 0,
        },
        end: Position {
            line: 0,
            character: 0,
        },
    };

    let related_information = if let Some(spans) = err.spans() {
        let mut out: Vec<DiagnosticRelatedInformation> = Vec::new();
        for s in spans.iter().skip(1).copied() {
            if let Some(p) = primary_span {
                if s == p {
                    continue;
                }
            }
            let Some(range) = span_to_range(doc, s) else {
                continue;
            };
            if out.iter().any(|ri| {
                ri.location.range.start == range.start && ri.location.range.end == range.end
            }) {
                continue;
            }
            out.push(DiagnosticRelatedInformation {
                location: Location {
                    uri: doc.uri.clone(),
                    range,
                },
                message: "related location".to_string(),
            });
        }
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    } else if let Some(spans) = err.source_spans() {
        let mut out: Vec<DiagnosticRelatedInformation> = Vec::new();
        for ss in spans.iter().skip(1) {
            let Ok(uri) = Url::from_file_path(&ss.path) else {
                continue;
            };
            // Best-effort: without the other document in the VFS, we can't map offsets to line/col.
            // Still provide the URI; clients can open the file.
            out.push(DiagnosticRelatedInformation {
                location: Location {
                    uri,
                    range: zero_range,
                },
                message: "related location".to_string(),
            });
        }
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    } else {
        None
    };

    Diagnostic {
        range,
        severity: Some(severity),
        code: None,
        code_description: None,
        source: Some("kscr".to_string()),
        message: err.to_string(),
        related_information,
        tags: None,
        data: None,
    }
}

pub(super) fn qualified_ident_at_offset(
    src: &str,
    offset: usize,
) -> Option<(String, kscr::lexer::Span)> {
    use lexer::TokenKind;

    let toks = lexer::lex(src).ok()?;
    let i = toks
        .iter()
        .position(|t| t.span.start <= offset && offset < t.span.end && t.span.end > t.span.start)
        .or_else(|| {
            toks.iter().position(|t| {
                t.span.start < offset && offset <= t.span.end && t.span.end > t.span.start
            })
        })?;

    let lexer::Token {
        kind: TokenKind::Ident(_),
        ..
    } = &toks[i]
    else {
        return None;
    };

    let mut start = i;
    while start >= 2 {
        if toks[start - 1].kind == TokenKind::Dot
            && matches!(toks[start - 2].kind, TokenKind::Ident(_))
        {
            start -= 2;
            continue;
        }
        break;
    }

    let mut end = i;
    while end + 2 < toks.len() {
        if toks[end + 1].kind == TokenKind::Dot && matches!(toks[end + 2].kind, TokenKind::Ident(_))
        {
            end += 2;
            continue;
        }
        break;
    }

    let mut parts = Vec::new();
    let mut span = toks[start].span;
    let mut j = start;
    while j <= end {
        if let TokenKind::Ident(s) = &toks[j].kind {
            parts.push(s.clone());
            span.end = toks[j].span.end;
        }
        j += 2;
    }

    Some((parts.join("."), span))
}

pub(super) fn find_decl_name_span(
    doc: &Document,
    kw: lexer::TokenKind,
    name: &str,
) -> Option<kscr::lexer::Span> {
    let toks = lexer::lex(&doc.text).ok()?;
    for w in toks.windows(2) {
        if w[0].kind == kw {
            if let lexer::TokenKind::Ident(n) = &w[1].kind {
                if n == name {
                    return Some(w[1].span);
                }
            }
        }
    }
    None
}
