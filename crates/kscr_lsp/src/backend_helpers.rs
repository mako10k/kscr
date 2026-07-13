use crate::vfs::Document;
use kscr::{error::Error as KscrError, lexer};
use tower_lsp::lsp_types::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QualifiedIdentParts {
    pub current_name: String,
    pub current_span: kscr::lexer::Span,
    pub full_name: String,
    pub full_span: kscr::lexer::Span,
    pub segment_index: usize,
    pub segments: Vec<String>,
}

fn token_symbol_name(kind: &lexer::TokenKind) -> Option<String> {
    use lexer::TokenKind;

    match kind {
        TokenKind::Ident(name) | TokenKind::Operator(name) => Some(name.clone()),
        TokenKind::Plus => Some("+".to_string()),
        TokenKind::PlusPlus => Some("++".to_string()),
        TokenKind::Minus => Some("-".to_string()),
        TokenKind::Star => Some("*".to_string()),
        TokenKind::Slash => Some("/".to_string()),
        TokenKind::EqEq => Some("==".to_string()),
        TokenKind::SlashEq => Some("/=".to_string()),
        TokenKind::Lt => Some("<".to_string()),
        TokenKind::Le => Some("<=".to_string()),
        TokenKind::Gt => Some(">".to_string()),
        TokenKind::Ge => Some(">=".to_string()),
        TokenKind::GtGt => Some(">>".to_string()),
        TokenKind::GtGtEq => Some(">>=".to_string()),
        TokenKind::AndAnd => Some("&&".to_string()),
        TokenKind::OrOr => Some("||".to_string()),
        TokenKind::Colon => Some(":".to_string()),
        _ => None,
    }
}

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
    let ident = qualified_ident_parts_at_offset(src, offset)?;
    Some((ident.full_name, ident.full_span))
}

pub(super) fn qualified_ident_parts_at_offset(
    src: &str,
    offset: usize,
) -> Option<QualifiedIdentParts> {
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

    let current_name = token_symbol_name(&toks[i].kind)?;

    if !matches!(toks[i].kind, TokenKind::Ident(_)) {
        return Some(QualifiedIdentParts {
            current_name: current_name.clone(),
            current_span: toks[i].span,
            full_name: current_name.clone(),
            full_span: toks[i].span,
            segment_index: 0,
            segments: vec![current_name],
        });
    }

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

    Some(QualifiedIdentParts {
        current_name,
        current_span: toks[i].span,
        full_name: parts.join("."),
        full_span: span,
        segment_index: (i - start) / 2,
        segments: parts,
    })
}

pub(super) fn contextual_ident_kind_at_offset(src: &str, offset: usize) -> Option<&'static str> {
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

    let TokenKind::Ident(_) = &toks[i].kind else {
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

    if start >= 1 && toks[start - 1].kind == TokenKind::KwModule {
        return Some("module");
    }
    if start >= 1 && toks[start - 1].kind == TokenKind::KwImport {
        return Some("module");
    }
    if start >= 2
        && matches!(&toks[start - 1].kind, TokenKind::Ident(s) if s == "qualified")
        && toks[start - 2].kind == TokenKind::KwImport
    {
        return Some("module");
    }

    if instance_head_class_token_index(&toks, i).is_some_and(|idx| idx == i) {
        return Some("class");
    }

    None
}

fn instance_head_class_token_index(toks: &[lexer::Token], cursor: usize) -> Option<usize> {
    use lexer::TokenKind;

    let mut inst = cursor;
    loop {
        if toks.get(inst)?.kind == TokenKind::KwInstance {
            break;
        }
        if matches!(
            toks.get(inst)?.kind,
            TokenKind::KwModule
                | TokenKind::KwImport
                | TokenKind::KwData
                | TokenKind::KwNewtype
                | TokenKind::KwType
                | TokenKind::KwClass
        ) {
            return None;
        }
        if inst == 0 {
            return None;
        }
        inst -= 1;
    }

    let mut end = inst + 1;
    let mut last_fat_arrow = None;
    while end < toks.len() {
        match toks[end].kind {
            TokenKind::KwWhere => break,
            TokenKind::FatArrow => last_fat_arrow = Some(end),
            TokenKind::Newline | TokenKind::Indent | TokenKind::Dedent => {}
            _ => {}
        }
        end += 1;
    }

    let scan_start = last_fat_arrow.map(|idx| idx + 1).unwrap_or(inst + 1);
    for (idx, tok) in toks.iter().enumerate().take(end).skip(scan_start) {
        if let TokenKind::Ident(name) = &tok.kind {
            if name
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_uppercase())
            {
                return Some(idx);
            }
        }
    }

    None
}

pub(super) fn find_decl_name_span(
    doc: &Document,
    kw: lexer::TokenKind,
    name: &str,
) -> Option<kscr::lexer::Span> {
    find_decl_name_span_any(doc, &[kw], name)
}

pub(super) fn find_decl_name_span_any(
    doc: &Document,
    kws: &[lexer::TokenKind],
    name: &str,
) -> Option<kscr::lexer::Span> {
    let toks = lexer::lex(&doc.text).ok()?;
    for w in toks.windows(2) {
        if kws.iter().any(|kw| w[0].kind == *kw) {
            if let lexer::TokenKind::Ident(n) = &w[1].kind {
                if n == name {
                    return Some(w[1].span);
                }
            }
        }
    }
    None
}

pub(super) fn find_ctor_name_span(
    doc: &Document,
    ctor_span: kscr::lexer::Span,
    name: &str,
) -> Option<kscr::lexer::Span> {
    let toks = lexer::lex(&doc.text).ok()?;
    for tok in toks {
        if tok.span.start < ctor_span.start {
            continue;
        }
        if tok.span.end > ctor_span.end {
            break;
        }

        match &tok.kind {
            lexer::TokenKind::Ident(n) if n == name => return Some(tok.span),
            lexer::TokenKind::Operator(n) if n == name => return Some(tok.span),
            _ => {}
        }
    }
    None
}
