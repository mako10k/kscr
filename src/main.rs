#![forbid(unsafe_code)]

use std::env;

fn main() {
    if let Err(e) = kscr::cli::run(env::args()) {
        // Best-effort location rendering for commands that take a single <file> argument.
        // Fallback to message-only when the file is unknown.
        let mut it = env::args();
        let _exe = it.next();
        let cmd = it.next().unwrap_or_default();
        let file_arg = match cmd.as_str() {
            "parse" | "lex" | "typecheck" | "ir" | "llvm-ir" | "compile" | "run" => it.next(),
            _ => None,
        };

        fn span_to_loc(src: Option<&str>, span: kscr::lexer::Span) -> (usize, usize, usize, usize) {
            let len = src.map(|s| s.len()).unwrap_or(usize::MAX);
            let start_off = span.start.min(len);
            let mut end_off = span.end.min(len);
            if end_off < start_off {
                end_off = start_off;
            }

            if let Some(s) = src {
                let mut line: usize = 1;
                let mut last_nl: usize = 0;
                for (i, ch) in s.char_indices() {
                    if i >= start_off {
                        break;
                    }
                    if ch == '\n' {
                        line += 1;
                        last_nl = i + 1;
                    }
                }
                let col = s[last_nl..start_off].chars().count() + 1;
                (line, col, start_off, end_off)
            } else {
                (1, 1, start_off, end_off)
            }
        }

        fn render_snippet(src: &str, start_off: usize, end_off: usize) -> Option<String> {
            if src.is_empty() {
                return None;
            }

            let start_off = start_off.min(src.len());
            let end_off = end_off.min(src.len());
            let line_start = src[..start_off].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let line_end = src[start_off..]
                .find('\n')
                .map(|i| start_off + i)
                .unwrap_or(src.len());
            let line_text = &src[line_start..line_end];
            if line_text.is_empty() {
                return None;
            }

            let col = src[line_start..start_off].chars().count();
            let mut span_len = end_off.saturating_sub(start_off);
            if span_len == 0 {
                span_len = 1;
            }
            let max_len = line_text.chars().count().saturating_sub(col).max(1);
            let span_len = span_len.min(max_len);

            let caret = if span_len <= 1 {
                "^".to_string()
            } else {
                format!("^{}", "~".repeat(span_len - 1))
            };
            let pad = " ".repeat(col);
            Some(format!("  {line_text}\n  {pad}{caret}"))
        }

        if let Some(path) = file_arg {
            let src = std::fs::read_to_string(&path).ok();
            let src_s = src.as_deref();

            if let Some(spans) = e.spans() {
                let primary = spans
                    .iter()
                    .copied()
                    .find(|s| s.start < s.end)
                    .or_else(|| spans.first().copied());

                if let Some(primary) = primary {
                    let (line, col, start_off, end_off) = span_to_loc(src_s, primary);
                    if src_s.is_some() {
                        eprintln!("error: {path}:{line}:{col}: {e} (span {start_off}..{end_off})");
                        if let Some(src) = src_s {
                            if let Some(snippet) = render_snippet(src, start_off, end_off) {
                                eprintln!("{snippet}");
                            }
                        }
                    } else {
                        eprintln!("error: {path}: {e} (span {start_off}..{end_off})");
                    }

                    let mut prev = primary;
                    for s2 in spans.iter().skip(1).copied() {
                        if s2 == prev {
                            continue;
                        }
                        prev = s2;
                        let (l2, c2, so2, eo2) = span_to_loc(src_s, s2);
                        if src_s.is_some() {
                            eprintln!(
                                "note: {path}:{l2}:{c2}: related location (span {so2}..{eo2})"
                            );
                            if let Some(src) = src_s {
                                if let Some(snippet) = render_snippet(src, so2, eo2) {
                                    eprintln!("{snippet}");
                                }
                            }
                        } else {
                            eprintln!("note: {path}: related location (span {so2}..{eo2})");
                        }
                    }

                    std::process::exit(1);
                }
            }
        }

        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
