use crate::{parser, Result};

pub fn run<I, S>(mut args: I) -> Result<()>
where
    I: Iterator<Item = S>,
    S: Into<String>,
{
    let _exe = args.next();
    let cmd = args.next().map(Into::into).unwrap_or_else(|| "help".to_string());

    match cmd.as_str() {
        "help" | "-h" | "--help" => {
            print_help();
            Ok(())
        }
        "parse" => {
            let path = args.next().ok_or_else(|| crate::error::Error::msg("missing <file>"))?;
            let src = std::fs::read_to_string(path.into())?;
            let ast = parser::parse_module(&src)?;
            println!("{ast:#?}");
            Ok(())
        }
        _ => Err(crate::error::Error::msg(format!("unknown command: {cmd}"))),
    }
}

fn print_help() {
    eprintln!(
        "kscr - lazy functional scripting language (scaffold)\n\nUSAGE:\n  kscr <command> [args]\n\nCOMMANDS:\n  parse <file>   Parse source and print AST (debug)\n  help           Show this help\n"
    );
}
