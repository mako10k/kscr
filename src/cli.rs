use crate::{ast, ir, parser, types, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

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
            let mut show_all = false;
            let arg1 = args
                .next()
                .ok_or_else(|| crate::error::Error::msg("missing <file>"))?;
            let path = match arg1.into().as_str() {
                "--all" => {
                    show_all = true;
                    args.next()
                        .ok_or_else(|| crate::error::Error::msg("missing <file>"))?
                        .into()
                }
                other => other.to_string(),
            };

            let module = load_module_with_imports(Path::new(&path))?;
            let tm = types::typecheck(module)?;

            print!(
                "{}",
                render_typecheck_report(&tm.module, tm.inferred, show_all)
            );
            Ok(())
        }
        "ir" => {
            let path = args
                .next()
                .ok_or_else(|| crate::error::Error::msg("missing <file>"))?
                .into();
            let module = load_module_with_imports(Path::new(&path))?;
            let tm = types::typecheck(module)?;
            let irm = ir::lower_to_ir(&tm.module)?;
            println!("{irm:#?}");
            Ok(())
        }
        "run" => {
            let path = args
                .next()
                .ok_or_else(|| crate::error::Error::msg("missing <file>"))?
                .into();
            let module = load_module_with_imports(Path::new(&path))?;
            let tm = types::typecheck(module)?;
            let irm = ir::lower_to_ir(&tm.module)?;
            let v = ir::run_main(&irm)?;
            match v {
                ir::Value::Unit => println!("()"),
                other => println!("{other:#?}"),
            }
            Ok(())
        }
        _ => Err(crate::error::Error::msg(format!("unknown command: {cmd}"))),
    }
}

fn load_module_with_imports(entry: &Path) -> Result<ast::Module> {
    let entry = std::fs::canonicalize(entry)?;
    let entry_dir = entry.parent().unwrap_or_else(|| Path::new("."));

    let mut loader = ModuleLoader {
        cache: HashMap::new(),
        stack: Vec::new(),
        emitted: HashSet::new(),
    };

    let entry_mod = loader.load_ast(&entry)?;

    let mut items = Vec::new();
    let mut defined = HashSet::new();

    let mut deps = Vec::new();
    loader.collect_imports(&entry_mod, entry_dir, &mut deps)?;

    for it in deps {
        push_item_checked(&mut items, &mut defined, it)?;
    }

    for it in entry_mod.items {
        if matches!(it, ast::Item::Import(_)) {
            continue;
        }
        push_item_checked(&mut items, &mut defined, it)?;
    }

    Ok(ast::Module {
        name: entry_mod.name,
        items,
    })
}

struct ModuleLoader {
    cache: HashMap<PathBuf, ast::Module>,
    stack: Vec<PathBuf>,
    emitted: HashSet<PathBuf>,
}

impl ModuleLoader {
    fn load_ast(&mut self, path: &Path) -> Result<ast::Module> {
        if let Some(m) = self.cache.get(path) {
            return Ok(m.clone());
        }

        if self.stack.iter().any(|p| p == path) {
            return Err(crate::error::Error::msg("cyclic imports"));
        }

        self.stack.push(path.to_path_buf());
        let src = std::fs::read_to_string(path)?;
        let m = parser::parse_module(&src)?;
        self.stack.pop();

        self.cache.insert(path.to_path_buf(), m.clone());
        Ok(m)
    }

    fn collect_imports(
        &mut self,
        module: &ast::Module,
        dir: &Path,
        out: &mut Vec<ast::Item>,
    ) -> Result<()> {
        for it in &module.items {
            let ast::Item::Import(id) = it else {
                continue;
            };

            if id.as_name.is_some() {
                return Err(crate::error::Error::msg(
                    "qualified imports are not supported yet (import ... as ...)",
                ));
            }

            let p = std::fs::canonicalize(dir.join(format!("{}.ks", id.module))).map_err(|_| {
                crate::error::Error::msg(format!("cannot find module file for import {}", id.module))
            })?;

            let imported = self.load_ast(&p)?;
            let Some(name) = &imported.name else {
                return Err(crate::error::Error::msg(format!(
                    "imported module {} must have a module header",
                    id.module
                )));
            };
            if name != &id.module {
                return Err(crate::error::Error::msg(format!(
                    "module name mismatch: import {} but file declares module {}",
                    id.module, name
                )));
            }

            let imported_dir = p.parent().unwrap_or(dir);
            self.collect_imports(&imported, imported_dir, out)?;

            if self.emitted.insert(p) {
                out.extend(public_items(&imported));
            }
        }
        Ok(())
    }
}

fn public_items(module: &ast::Module) -> Vec<ast::Item> {
    let exports = exported_names(module);
    module
        .items
        .iter()
        .filter_map(|it| match it {
            ast::Item::Import(_) | ast::Item::Export(_) => None,
            ast::Item::Binding(b) => {
                if let Some(exports) = &exports {
                    let mut names = HashSet::new();
                    pat_defined_names(&b.pat, &mut names);
                    if !names.iter().any(|n| exports.contains(n)) {
                        return None;
                    }
                }
                Some(ast::Item::Binding(b.clone()))
            }
            ast::Item::TypeAlias(ta) => {
                if let Some(exports) = &exports {
                    if !exports.contains(&ta.name) {
                        return None;
                    }
                }
                Some(ast::Item::TypeAlias(ta.clone()))
            }
            ast::Item::DataDecl(d) => {
                if let Some(exports) = &exports {
                    let any_ctor = d.ctors.iter().any(|c| exports.contains(&c.name));
                    if !exports.contains(&d.name) && !any_ctor {
                        return None;
                    }
                }
                Some(ast::Item::DataDecl(d.clone()))
            }
        })
        .collect()
}

fn push_item_checked(
    items: &mut Vec<ast::Item>,
    defined: &mut HashSet<String>,
    it: ast::Item,
) -> Result<()> {
    let mut names = HashSet::new();
    item_defined_names(&it, &mut names);
    for n in names {
        if !defined.insert(n.clone()) {
            return Err(crate::error::Error::msg(format!("name conflict: {n}")));
        }
    }
    items.push(it);
    Ok(())
}

fn item_defined_names(it: &ast::Item, out: &mut HashSet<String>) {
    match it {
        ast::Item::Binding(b) => pat_defined_names(&b.pat, out),
        ast::Item::TypeAlias(ta) => {
            out.insert(ta.name.clone());
        }
        ast::Item::DataDecl(d) => {
            out.insert(d.name.clone());
            out.extend(d.ctors.iter().map(|c| c.name.clone()));
        }
        ast::Item::Import(_) | ast::Item::Export(_) => {}
    }
}

fn pat_defined_names(p: &ast::Pattern, out: &mut HashSet<String>) {
    use ast::Pattern;
    match p {
        Pattern::Var(n) => {
            out.insert(n.clone());
        }
        Pattern::As(n, p) => {
            out.insert(n.clone());
            pat_defined_names(p, out);
        }
        Pattern::Tuple(ps) | Pattern::List(ps) => {
            for p in ps {
                pat_defined_names(p, out);
            }
        }
        Pattern::Record(fs) | Pattern::RecordLoose(fs, _) => {
            for (_, p) in fs {
                pat_defined_names(p, out);
            }
            if let Pattern::RecordLoose(_, Some(rest)) = p {
                out.insert(rest.clone());
            }
        }
        Pattern::Cons(a, b) | Pattern::Or(a, b) => {
            pat_defined_names(a, out);
            pat_defined_names(b, out);
        }
        Pattern::View(p, _) => pat_defined_names(p, out),
        Pattern::Constructor { args, .. } => {
            for p in args {
                pat_defined_names(p, out);
            }
        }
        Pattern::Wildcard | Pattern::Hole(_) | Pattern::Literal(_) => {}
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

fn render_typecheck_report(
    module: &ast::Module,
    inferred: std::collections::HashMap<String, types::Scheme>,
    show_all: bool,
) -> String {
    let mut out = String::new();

    if let Some(name) = &module.name {
        out.push_str(&format!("module {name}\n"));
    }

    if let Some(exports) = exported_names(module) {
        let mut names: Vec<_> = exports.into_iter().collect();
        names.sort();
        out.push_str("export ");
        out.push_str(&names.join(", "));
        out.push('\n');
    }

    let mut inferred = if show_all {
        inferred.into_iter().collect()
    } else {
        filter_inferred_by_exports(module, inferred)
    };
    inferred.sort_by(|(a, _), (b, _)| a.cmp(b));
    for (name, scheme) in inferred {
        out.push_str(&format!("{name} : {scheme}\n"));
    }

    out
}

fn print_help() {
    eprintln!(
        "kscr - lazy functional scripting language (scaffold)\n\nUSAGE:\n  kscr <command> [args]\n\nCOMMANDS:\n  parse <file>      Parse source and print AST (debug)\n  lex <file>        Lex source and print tokens (debug)\n  typecheck <file>  Typecheck and print inferred schemes\n                   (if export decl exists, only exported names are shown)\n  ir <file>         Typecheck then lower to IR (debug)\n  run <file>        Typecheck, lower to IR, then run main (minimal)\n  help              Show this help\n"
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

    #[test]
    fn typecheck_report_includes_module_and_export() {
        let src = "module Main where\n  export x\n  x = 1\n  y = 2\n";
        let ast = parser::parse_module(src).unwrap();
        let tm = types::typecheck(ast).unwrap();
        let report = render_typecheck_report(&tm.module, tm.inferred, false);
        assert!(report.starts_with("module Main\nexport x\n"));
        assert!(report.contains("x : Integer\n"));
        assert!(!report.contains("y : Integer\n"));
    }

    #[test]
    fn typecheck_report_all_includes_nonexported() {
        let src = "module Main where\n  export x\n  x = 1\n  y = 2\n";
        let ast = parser::parse_module(src).unwrap();
        let tm = types::typecheck(ast).unwrap();
        let report = render_typecheck_report(&tm.module, tm.inferred, true);
        assert!(report.contains("y : Integer\n"));
    }

    #[test]
    fn cli_run_command_smoke() {
        let path = std::env::temp_dir().join("kscr_cli_run_command_smoke.ks");
        std::fs::write(&path, "main = IO ()\n").unwrap();
        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_typecheck_imports_smoke() {
        let dir = std::env::temp_dir().join(format!("kscr_cli_import_smoke_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("A.ks");
        std::fs::write(&a, "module A where\n  x = 1\n").unwrap();

        let main = dir.join("Main.ks");
        std::fs::write(
            &main,
            "module Main where\n  import A\n  y = x + 1\n  main = IO ()\n",
        )
        .unwrap();

        let args = vec![
            "kscr".to_string(),
            "typecheck".to_string(),
            main.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();

        let _ = std::fs::remove_dir_all(dir);
    }
}
