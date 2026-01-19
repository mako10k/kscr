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

        if let Some(path) = file_arg {
            let src = std::fs::read_to_string(&path).ok();
            if let Some(span) = e.span() {
                let out = {
                    let src_s = src.as_deref();
                    let start_off = span.start.min(src_s.map(|s| s.len()).unwrap_or(span.start));
                    let mut end_off = span.end;
                    if let Some(s) = src_s {
                        end_off = end_off.min(s.len());
                        if end_off < start_off {
                            end_off = start_off;
                        }
                    }

                    if let Some(s) = src_s {
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
                        format!("error: {path}:{line}:{col}: {e} (span {start_off}..{end_off})")
                    } else {
                        format!("error: {path}: {e} (span {start_off}..{end_off})")
                    }
                };

                eprintln!("{out}");
                std::process::exit(1);
            }
        }

        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
