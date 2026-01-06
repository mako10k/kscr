use crate::{ast, parser, types, Result};
use std::collections::HashSet;

pub fn run<I, S>(mut args: I) -> Result<()>
where
    I: Iterator<Item = S>,
    S: Into<String>,
{
    let _exe = args.next();
    let cmd = args
        .next()
        .map(Into::into)
        .unwrap_or_else(|| "help".to_string());

    match cmd.as_str() {
        "help" | "-h" | "--help" => {
            print_help();
            Ok(())
        }
        "parse" => {
            let path = args
                .next()
                .ok_or_else(|| crate::error::Error::msg("missing <file>"))?;
            let src = std::fs::read_to_string(path.into())?;
            let ast = parser::parse_module(&src)?;
            println!("{ast:#?}");
            Ok(())
        }
        "lex" => {
            let path = args
                .next()
                .ok_or_else(|| crate::error::Error::msg("missing <file>"))?;
            let src = std::fs::read_to_string(path.into())?;
            let toks = crate::lexer::lex(&src)?;
            println!("{toks:#?}");
            Ok(())
        }
        "typecheck" => {
            let path = args
                .next()
                .ok_or_else(|| crate::error::Error::msg("missing <file>"))?;
            let src = std::fs::read_to_string(path.into())?;
            let ast = parser::parse_module(&src)?;
            let tm = types::typecheck(ast)?;

            let mut inferred = filter_inferred_by_exports(&tm.module, tm.inferred);
            inferred.sort_by(|(a, _), (b, _)| a.cmp(b));
            for (name, scheme) in inferred {
                println!("{name} : {scheme}");
            }
            Ok(())
        }
        _ => Err(crate::error::Error::msg(format!("unknown command: {cmd}"))),
    }
}

fn exported_names(module: &ast::Module) -> Option<HashSet<String>> {
    let mut out = HashSet::new();
    for it in &module.items {
        if let ast::Item::Export(ed) = it {
            out.extend(ed.names.iter().cloned());
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn filter_inferred_by_exports(
    module: &ast::Module,
    inferred: std::collections::HashMap<String, types::Scheme>,
) -> Vec<(String, types::Scheme)> {
    match exported_names(module) {
        None => inferred.into_iter().collect(),
        Some(exports) => inferred
            .into_iter()
            .filter(|(name, _)| exports.contains(name))
            .collect(),
    }
}

fn print_help() {
    eprintln!(
        "kscr - lazy functional scripting language (scaffold)\n\nUSAGE:\n  kscr <command> [args]\n\nCOMMANDS:\n  parse <file>      Parse source and print AST (debug)\n  lex <file>        Lex source and print tokens (debug)\n  typecheck <file>  Typecheck and print inferred schemes\n                   (if export decl exists, only exported names are shown)\n                   (imports are not supported yet)\n  help              Show this help\n"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typecheck_filters_exports() {
        let src = "export x\nx = 1\ny = 2\n";
        let ast = parser::parse_module(src).unwrap();
        let tm = types::typecheck(ast).unwrap();
        let filtered = filter_inferred_by_exports(&tm.module, tm.inferred);
        let names: HashSet<String> = filtered.into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["x".to_string()].into_iter().collect());
    }
}
