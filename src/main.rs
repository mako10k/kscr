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

        if let Some(path) = file_arg {
            let src = std::fs::read_to_string(&path).ok();
            let src_s = src.as_deref();

            if let Some(spans) = e.spans() {
                if let Some(primary) = spans.first().copied() {
                    let (line, col, start_off, end_off) = span_to_loc(src_s, primary);
                    if src_s.is_some() {
                        eprintln!("error: {path}:{line}:{col}: {e} (span {start_off}..{end_off})");
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
