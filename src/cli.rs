use crate::{ast, ir, parser, types, Result};
use std::collections::HashSet;
use std::path::Path;

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

            let tm = types::typecheck_file(Path::new(&path))?;

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
            let tm = types::typecheck_file(Path::new(&path))?;
            let irm = ir::lower_to_ir(&tm.module)?;
            println!("{irm:#?}");
            Ok(())
        }
        "run" => {
            let path = args
                .next()
                .ok_or_else(|| crate::error::Error::msg("missing <file>"))?
                .into();
            let tm = types::typecheck_file(Path::new(&path))?;
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

    #[test]
    fn cli_run_do_smoke() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_do_smoke_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  main = do\n    print \"hello\"\n    print \"world\"\n",
        )
        .unwrap();
        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_run_import_prelude_from_stdlib_smoke() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_import_prelude_stdlib_smoke_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  import Prelude\n  inc = \\x -> x + 1\n  main = do\n    print (show (map inc [1, 2]))\n    print (show (filter (\\x -> x == 2) [1, 2, 3]))\n    print (show (concat [[1], [2, 3]]))\n",
        )
        .unwrap();
        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_run_import_data_case_smoke() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_cli_run_import_data_case_smoke_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("A.ks");
        std::fs::write(
            &a,
            "module A where\n  export Maybe\n  data Maybe a = Nothing | Just a\n",
        )
        .unwrap();

        let main = dir.join("Main.ks");
        std::fs::write(
            &main,
            "module Main where\n  import A\n  x = case Just 1 of\n    Just n -> n\n    Nothing -> 0\n  main = do\n    print (intToString x)\n",
        )
        .unwrap();

        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            main.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cli_run_transitive_import_qualified_smoke() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_cli_run_transitive_import_smoke_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("A.ks");
        std::fs::write(&a, "module A where\n  export x\n  x = 1\n").unwrap();

        let b = dir.join("B.ks");
        std::fs::write(
            &b,
            "module B where\n  export y\n  import A as OM\n  y = OM.x + 1\n",
        )
        .unwrap();

        let main = dir.join("Main.ks");
        std::fs::write(
            &main,
            "module Main where\n  import B\n  main = do\n    print (intToString y)\n",
        )
        .unwrap();

        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            main.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cli_run_import_as_qualified_in_list_comprehension_smoke() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_cli_run_import_as_qualified_in_list_comprehension_smoke_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("A.ks");
        std::fs::write(&a, "module A where\n  export x\n  x = 7\n").unwrap();

        let main = dir.join("Main.ks");
        std::fs::write(
            &main,
            "module Main where\n  import A as OM\n  xs = [OM.x | _ <- [1, 2], True]\n  main = do\n    print (show xs)\n",
        )
        .unwrap();

        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            main.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cli_run_imported_ctor_in_list_comprehension_smoke() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_cli_run_imported_ctor_in_list_comprehension_smoke_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("A.ks");
        std::fs::write(
            &a,
            "module A where\n  export Maybe, values\n  data Maybe a = Nothing | Just a\n  values = [Just 1, Nothing, Just 3]\n",
        )
        .unwrap();

        let main = dir.join("Main.ks");
        std::fs::write(
            &main,
            "module Main where\n  import A\n  xs = [a | Just a <- values]\n  main = do\n    print (show xs)\n",
        )
        .unwrap();

        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            main.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cli_run_import_as_qualified_ctor_in_list_comprehension_smoke() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_cli_run_import_as_qualified_ctor_in_list_comprehension_smoke_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("A.ks");
        std::fs::write(
            &a,
            "module A where\n  export Maybe, values\n  data Maybe a = Nothing | Just a\n  values = [Just 1, Nothing, Just 3]\n",
        )
        .unwrap();

        let main = dir.join("Main.ks");
        std::fs::write(
            &main,
            "module Main where\n  import A as OM\n  xs = [a | OM.Just a <- OM.values]\n  main = do\n    print (show xs)\n",
        )
        .unwrap();

        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            main.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cli_run_list_comprehension_skips_failed_pattern_smoke() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_list_comprehension_skips_failed_pattern_smoke_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  data Maybe a = Nothing | Just a\n  xs = [a | Just a <- [Just 1, Nothing, Just 3]]\n  main = do\n    print (show xs)\n",
        )
        .unwrap();

        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_run_list_comprehension_view_pattern_smoke() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_list_comprehension_view_pattern_smoke_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  data Maybe a = Nothing | Just a\n  id = \\x -> x\n  xs = [a | (Just a <- id) <- [Just 1, Nothing, Just 3]]\n  main = do\n    print (show xs)\n",
        )
        .unwrap();

        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_run_list_case_smoke() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_list_case_smoke_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  xs = [1, 2]\n  x = case xs of\n    [a, b] -> a + b\n    _ -> 0\n  main = do\n    print (intToString x)\n",
        )
        .unwrap();
        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_run_tuple_pattern_smoke() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_tuple_pattern_smoke_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  x = case (1, 2) of\n    (a, b) -> a + b\n  main = do\n    print (intToString x)\n",
        )
        .unwrap();
        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_run_tuple_pattern_does_not_force_fields_smoke() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_tuple_pattern_does_not_force_fields_smoke_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  bad = 1 / 0\n  x = case (bad, 2) of\n    (a, b) -> b\n  main = do\n    print (intToString x)\n",
        )
        .unwrap();
        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_run_case_forces_scrutinee_smoke() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_case_forces_scrutinee_smoke_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  x = case (1 / 0) of\n    _ -> 1\n  main = do\n    print (intToString x)\n",
        )
        .unwrap();
        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        let e = run(args.into_iter()).unwrap_err();
        assert!(format!("{e}").contains("division by zero"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_run_case_guard_smoke() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_case_guard_smoke_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  x = case 1 of\n    n | n == 1 -> n + 41\n    _ -> 0\n  main = do\n    print (intToString x)\n",
        )
        .unwrap();
        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_run_case_guard_fallthrough_smoke() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_case_guard_fallthrough_smoke_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  x = case 1 of\n    _ | 1 == 2 -> 0\n    _ -> 1\n  main = do\n    print (intToString x)\n",
        )
        .unwrap();
        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_run_short_circuit_bool_smoke() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_short_circuit_bool_smoke_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  bad = 1 / 0\n  a = False && (bad == 0)\n  b = True || (bad == 0)\n  main = do\n    print (boolToString a)\n    print (boolToString b)\n",
        )
        .unwrap();
        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_run_concat_map_smoke() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_concat_map_smoke_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  xs = concatMap (\\x -> [x, x]) [1, 2]\n  main = do\n    print (show xs)\n",
        )
        .unwrap();
        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_run_let_is_lazy_smoke() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_let_is_lazy_smoke_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  x = let y = 1 / 0 in 1\n  main = do\n    print (intToString x)\n",
        )
        .unwrap();
        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_run_case_on_list_does_not_force_elements_smoke() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_case_on_list_does_not_force_elements_smoke_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  bad = 1 / 0\n  xs = [bad]\n  x = case xs of\n    [] -> 0\n    _ -> 1\n  main = do\n    print (intToString x)\n",
        )
        .unwrap();
        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_run_if_is_lazy_smoke() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_if_is_lazy_smoke_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  bad = 1 / 0\n  x = if True then 1 else bad\n  y = if False then bad else 2\n  main = do\n    print (intToString (x + y))\n",
        )
        .unwrap();
        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_run_closure_curry_smoke() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_closure_curry_smoke_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  add = \\x -> \\y -> x + y\n  f = add 1\n  x = f 2\n  main = do\n    print (intToString x)\n",
        )
        .unwrap();
        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_run_unused_top_level_is_not_evaluated_smoke() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_unused_top_level_is_not_evaluated_smoke_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  x = 1 / 0\n  main = do\n    print \"ok\"\n",
        )
        .unwrap();
        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_run_list_comprehension_guard_smoke() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_list_comprehension_guard_smoke_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  xs = [x | x <- [1, 2, 3], x == 2]\n  main = do\n    print (show xs)\n",
        )
        .unwrap();
        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_run_list_comprehension_pattern_bind_smoke() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_list_comprehension_pattern_bind_smoke_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  xs = [a | (a, b) <- [(1, 2), (3, 4)]]\n  main = do\n    print (show xs)\n",
        )
        .unwrap();
        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_run_list_comprehension_bind_and_guard_smoke() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_list_comprehension_bind_and_guard_smoke_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  xs = [a | (a, b) <- [(1, 2), (3, 4)], a == 3]\n  main = do\n    print (show xs)\n",
        )
        .unwrap();
        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_run_do_bind_ctor_pattern_smoke() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_do_bind_ctor_pattern_smoke_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  data Maybe a = Nothing | Just a\n  main = do\n    Just n <- IO (Just 1)\n    print (intToString n)\n",
        )
        .unwrap();
        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_run_import_as_blocks_unqualified_smoke() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_cli_run_import_as_blocks_unqualified_smoke_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("A.ks");
        std::fs::write(&a, "module A where\n  export x\n  x = 1\n").unwrap();

        let main = dir.join("Main.ks");
        std::fs::write(
            &main,
            "module Main where\n  import A as OM\n  y = x + 1\n  main = IO ()\n",
        )
        .unwrap();

        let args = vec![
            "kscr".to_string(),
            "typecheck".to_string(),
            main.to_string_lossy().to_string(),
        ];
        let e = run(args.into_iter()).unwrap_err();
        assert!(format!("{e}").contains("unbound variable: x"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cli_import_as_blocks_unqualified_ctor_smoke() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_cli_import_as_blocks_unqualified_ctor_smoke_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("A.ks");
        std::fs::write(
            &a,
            "module A where\n  export Maybe, values\n  data Maybe a = Nothing | Just a\n  values = [Just 1, Nothing]\n",
        )
        .unwrap();

        let main = dir.join("Main.ks");
        std::fs::write(
            &main,
            "module Main where\n  import A as OM\n  xs = [a | Just a <- OM.values]\n  main = IO ()\n",
        )
        .unwrap();

        let args = vec![
            "kscr".to_string(),
            "typecheck".to_string(),
            main.to_string_lossy().to_string(),
        ];
        let e = run(args.into_iter()).unwrap_err();
        assert!(format!("{e}").contains("unknown constructor"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cli_import_as_allows_qualified_ctor_smoke() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_cli_import_as_allows_qualified_ctor_smoke_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("A.ks");
        std::fs::write(
            &a,
            "module A where\n  export Maybe, values\n  data Maybe a = Nothing | Just a\n  values = [Just 1, Nothing]\n",
        )
        .unwrap();

        let main = dir.join("Main.ks");
        std::fs::write(
            &main,
            "module Main where\n  import A as OM\n  xs = [a | OM.Just a <- OM.values]\n  main = IO ()\n",
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

    #[test]
    fn cli_import_as_blocks_module_qualifier_smoke() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_cli_import_as_blocks_module_qualifier_smoke_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("A.ks");
        std::fs::write(&a, "module A where\n  export x\n  x = 1\n").unwrap();

        let main = dir.join("Main.ks");
        std::fs::write(
            &main,
            "module Main where\n  import A as OM\n  y = A.x + 1\n  main = IO ()\n",
        )
        .unwrap();

        let args = vec![
            "kscr".to_string(),
            "typecheck".to_string(),
            main.to_string_lossy().to_string(),
        ];
        let e = run(args.into_iter()).unwrap_err();
        assert!(format!("{e}").contains("unknown qualifier A"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cli_import_does_not_import_unexported_ctor_smoke() {
        let dir = std::env::temp_dir().join(format!(
            "kscr_cli_import_does_not_import_unexported_ctor_smoke_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = dir.join("A.ks");
        std::fs::write(
            &a,
            "module A where\n  export values\n  data Maybe a = Nothing | Just a\n  values = [Just 1, Nothing]\n",
        )
        .unwrap();

        let main = dir.join("Main.ks");
        std::fs::write(
            &main,
            "module Main where\n  import A\n  xs = [a | Just a <- values]\n  main = IO ()\n",
        )
        .unwrap();

        let args = vec![
            "kscr".to_string(),
            "typecheck".to_string(),
            main.to_string_lossy().to_string(),
        ];
        let e = run(args.into_iter()).unwrap_err();
        assert!(format!("{e}").contains("unknown constructor"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cli_run_record_pattern_smoke() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_record_pattern_smoke_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  r = {a: 1, b: 2}\n  x = case r of\n    {b: y, a: z} -> y + z\n  main = do\n    print (intToString x)\n",
        )
        .unwrap();
        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_run_record_loose_pattern_smoke() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_record_loose_pattern_smoke_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  r = {a: 1, b: 2, c: 3}\n  x = case r of\n    {b: y, ...} -> y\n  main = do\n    print (intToString x)\n",
        )
        .unwrap();
        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_run_record_loose_rest_binding_smoke() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_record_loose_rest_binding_smoke_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  r = {a: 1, b: 2}\n  x = case r of\n    {a: _, ...rest} -> case rest of\n      {b: y} -> y\n  main = do\n    print (intToString x)\n",
        )
        .unwrap();
        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_run_view_pattern_smoke() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_view_pattern_smoke_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  data Maybe a = Nothing | Just a\n  id = \\x -> x\n  x = case Just 1 of\n    (Just n <- id) -> n\n    _ -> 0\n  main = do\n    print (intToString x)\n",
        )
        .unwrap();
        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        run(args.into_iter()).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_run_rejects_recursive_value_definition() {
        let path = std::env::temp_dir().join(format!(
            "kscr_cli_run_rejects_recursive_value_definition_{}.ks",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "module Main where\n  x = x\n  main = IO ()\n",
        )
        .unwrap();
        let args = vec![
            "kscr".to_string(),
            "run".to_string(),
            path.to_string_lossy().to_string(),
        ];
        let e = run(args.into_iter()).unwrap_err();
        assert!(format!("{e}").contains("unbound variable: x"));
        let _ = std::fs::remove_file(path);
    }
}
