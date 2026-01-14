use crate::{error::Error, ir, Result};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
    StdoutWrite(String),
}

struct LlvmTextGen {
    module_name: String,
    out: String,
    str_pool: BTreeMap<String, String>,
    next_str_id: usize,
    next_tmp_id: usize,
}

impl LlvmTextGen {
    fn new(module_name: &str) -> Self {
        let mut this = Self {
            module_name: module_name.to_string(),
            out: String::new(),
            str_pool: BTreeMap::new(),
            next_str_id: 0,
            next_tmp_id: 0,
        };
        this.emit_header();
        this
    }

    fn emit_header(&mut self) {
        writeln!(&mut self.out, "; ModuleID = '{}'", self.module_name).unwrap();
        writeln!(&mut self.out, "source_filename = \"{}\"", self.module_name).unwrap();
        writeln!(
            &mut self.out,
            "target triple = \"x86_64-unknown-linux-gnu\""
        )
        .unwrap();
        writeln!(&mut self.out).unwrap();
    }

    fn declare_runtime(&mut self) {
        writeln!(&mut self.out, "; External declarations").unwrap();
        writeln!(&mut self.out, "declare i32 @printf(i8*, ...)").unwrap();
        writeln!(&mut self.out).unwrap();
    }

    fn intern_string(&mut self, s: &str) -> String {
        if let Some(name) = self.str_pool.get(s) {
            return name.clone();
        }
        let name = format!("@.str{}", self.next_str_id);
        self.next_str_id += 1;
        self.str_pool.insert(s.to_string(), name.clone());
        name
    }

    fn emit_string_constants(&mut self) {
        // printf format: "%s"
        writeln!(
            &mut self.out,
            "@.fmt = private unnamed_addr constant [3 x i8] c\"%s\\00\", align 1"
        )
        .unwrap();

        for (s, name) in &self.str_pool {
            let bytes = escape_llvm_bytes(s);
            let len = s.len() + 1;
            writeln!(
                &mut self.out,
                "{name} = private unnamed_addr constant [{len} x i8] c\"{bytes}\\00\", align 1"
            )
            .unwrap();
        }

        writeln!(&mut self.out).unwrap();
    }

    fn emit_main(&mut self, actions: Vec<Action>) {
        writeln!(&mut self.out, "define i32 @main() {{").unwrap();
        writeln!(&mut self.out, "entry:").unwrap();
        writeln!(
            &mut self.out,
            "  %fmt = getelementptr inbounds [3 x i8], [3 x i8]* @.fmt, i64 0, i64 0"
        )
        .unwrap();

        for action in actions {
            match action {
                Action::StdoutWrite(s) => {
                    let g = self.intern_string(&s);
                    let len = s.len() + 1;
                    let tmp = self.next_tmp_id;
                    self.next_tmp_id += 1;
                    writeln!(
                        &mut self.out,
                        "  %s{tmp} = getelementptr inbounds [{len} x i8], [{len} x i8]* {g}, i64 0, i64 0"
                    )
                    .unwrap();
                    writeln!(
                        &mut self.out,
                        "  call i32 (i8*, ...) @printf(i8* %fmt, i8* %s{tmp})"
                    )
                    .unwrap();
                }
            }
        }

        writeln!(&mut self.out, "  ret i32 0").unwrap();
        writeln!(&mut self.out, "}}").unwrap();
        writeln!(&mut self.out).unwrap();
    }
}

/// IR -> LLVM IR text (MVP)
///
/// Supported subset:
/// - `main` binding exists.
/// - `IrExpr::IoThen` sequencing.
/// - `stdoutWrite <string-constant>` and `print <string-constant>`.
/// - terminal `IO ()` (treated as no-op).
///
/// Anything else returns an error.
pub fn lower_ir_to_llvm_text(module: &ir::IrModule, module_name: &str) -> Result<String> {
    let main = find_main_expr(module)?;
    let bindings = collect_bindings(module);

    let mut actions = Vec::new();
    collect_io(&bindings, &mut HashMap::new(), main, &mut actions)?;

    let mut gen = LlvmTextGen::new(module_name);
    gen.declare_runtime();

    // Pre-intern globals for deterministic output.
    for a in &actions {
        let Action::StdoutWrite(s) = a;
        gen.intern_string(s);
    }

    gen.emit_string_constants();
    gen.emit_main(actions);

    Ok(gen.out)
}

fn collect_bindings(module: &ir::IrModule) -> HashMap<String, ir::IrExpr> {
    let mut out = HashMap::new();
    for it in &module.items {
        match it {
            ir::IrItem::Binding { name, expr } => {
                out.insert(name.clone(), expr.clone());
            }
        }
    }
    out
}

fn find_main_expr(module: &ir::IrModule) -> Result<&ir::IrExpr> {
    for it in &module.items {
        match it {
            ir::IrItem::Binding { name, expr } if name == "main" => return Ok(expr),
            _ => {}
        }
    }
    Err(Error::msg("no `main` binding found"))
}

fn collect_io(
    bindings: &HashMap<String, ir::IrExpr>,
    visiting: &mut HashMap<String, bool>,
    expr: &ir::IrExpr,
    out: &mut Vec<Action>,
) -> Result<()> {
    match expr {
        ir::IrExpr::IoThen { first, then_expr } => {
            collect_io(bindings, visiting, first, out)?;
            collect_io(bindings, visiting, then_expr, out)
        }
        ir::IrExpr::Apply { func, args } => {
            let ir::IrExpr::Var(name) = &**func else {
                return Err(Error::msg(
                    "LLVM backend MVP supports only calls to IO/stdoutWrite/print",
                ));
            };

            if name == "IO" {
                if args.len() == 1 {
                    return Ok(());
                }
                return Err(Error::msg("LLVM backend: IO expects 1 arg"));
            }

            if name == "stdoutWrite" || name == "print" {
                if args.len() != 1 {
                    return Err(Error::msg("LLVM backend: stdoutWrite expects 1 arg"));
                }
                let s = eval_const_string(bindings, visiting, &args[0])?;
                out.push(Action::StdoutWrite(s));
                return Ok(());
            }

            Err(Error::msg(
                "LLVM backend MVP supports only IO (), stdoutWrite/print <string>, and IoThen",
            ))
        }
        _ => Err(Error::msg(
            "LLVM backend MVP expects main to be an IO expression",
        )),
    }
}

fn eval_const_string(
    bindings: &HashMap<String, ir::IrExpr>,
    visiting: &mut HashMap<String, bool>,
    expr: &ir::IrExpr,
) -> Result<String> {
    match expr {
        ir::IrExpr::String(s) => Ok(s.clone()),
        ir::IrExpr::List(es) => {
            let mut out = String::new();
            for e in es {
                match e {
                    ir::IrExpr::Char(c) => out.push(*c),
                    _ => {
                        return Err(Error::msg(
                            "LLVM backend: list literal must be a [Char] constant",
                        ))
                    }
                }
            }
            Ok(out)
        }
        ir::IrExpr::Cons { head, tail } => {
            let mut out = String::new();
            append_const_char_list(bindings, visiting, head, tail, &mut out)?;
            Ok(out)
        }
        ir::IrExpr::Apply { .. } => {
            let (head, args) = collect_apply(expr);
            let ir::IrExpr::Var(name) = head else {
                return Err(Error::msg(
                    "LLVM backend MVP supports only calls to built-in conversion functions",
                ));
            };

            match (name.as_str(), args.as_slice()) {
                ("intToString", [a]) => {
                    let v = eval_const_i64(bindings, visiting, a)?;
                    Ok(v.to_string())
                }
                ("boolToString", [a]) => {
                    let v = eval_const_bool(bindings, visiting, a)?;
                    Ok(if v { "True" } else { "False" }.to_string())
                }
                _ => Err(Error::msg(
                    "LLVM backend MVP supports only intToString/boolToString on constants",
                )),
            }
        }
        ir::IrExpr::Var(name) => {
            let Some(e) = bindings.get(name) else {
                return Err(Error::msg(format!(
                    "LLVM backend: unbound var in string constant: {name}"
                )));
            };
            if visiting.get(name).copied().unwrap_or(false) {
                return Err(Error::msg(format!(
                    "LLVM backend: cyclic constant definition: {name}"
                )));
            }
            visiting.insert(name.clone(), true);
            let r = eval_const_string(bindings, visiting, e);
            visiting.insert(name.clone(), false);
            r
        }
        _ => Err(Error::msg(
            "LLVM backend MVP supports only constant strings",
        )),
    }
}

fn collect_apply(expr: &ir::IrExpr) -> (&ir::IrExpr, Vec<&ir::IrExpr>) {
    fn go<'a>(e: &'a ir::IrExpr, out: &mut Vec<&'a ir::IrExpr>) -> &'a ir::IrExpr {
        match e {
            ir::IrExpr::Apply { func, args } => {
                let head = go(func, out);
                out.extend(args.iter());
                head
            }
            other => other,
        }
    }

    let mut args = Vec::new();
    let head = go(expr, &mut args);
    (head, args)
}

fn eval_const_i64(
    bindings: &HashMap<String, ir::IrExpr>,
    visiting: &mut HashMap<String, bool>,
    expr: &ir::IrExpr,
) -> Result<i64> {
    match expr {
        ir::IrExpr::Integer(s) => s
            .parse::<i64>()
            .map_err(|_| Error::msg("LLVM backend MVP supports only i64 integer constants")),
        ir::IrExpr::CheckedCast { expr, .. } => eval_const_i64(bindings, visiting, expr),
        ir::IrExpr::Var(name) => {
            let Some(e) = bindings.get(name) else {
                return Err(Error::msg(format!(
                    "LLVM backend: unbound var in integer constant: {name}"
                )));
            };
            if visiting.get(name).copied().unwrap_or(false) {
                return Err(Error::msg(format!(
                    "LLVM backend: cyclic constant definition: {name}"
                )));
            }
            visiting.insert(name.clone(), true);
            let r = eval_const_i64(bindings, visiting, e);
            visiting.insert(name.clone(), false);
            r
        }
        ir::IrExpr::Apply { .. } => {
            let (head, args) = collect_apply(expr);
            let ir::IrExpr::Var(name) = head else {
                return Err(Error::msg(
                    "LLVM backend MVP supports only built-in arithmetic on constants",
                ));
            };
            match (name.as_str(), args.as_slice()) {
                ("+", [a, b]) => Ok(eval_const_i64(bindings, visiting, a)?
                    .checked_add(eval_const_i64(bindings, visiting, b)?)
                    .ok_or_else(|| Error::msg("LLVM backend: i64 overflow in +"))?),
                ("-", [a, b]) => Ok(eval_const_i64(bindings, visiting, a)?
                    .checked_sub(eval_const_i64(bindings, visiting, b)?)
                    .ok_or_else(|| Error::msg("LLVM backend: i64 overflow in -"))?),
                ("*", [a, b]) => Ok(eval_const_i64(bindings, visiting, a)?
                    .checked_mul(eval_const_i64(bindings, visiting, b)?)
                    .ok_or_else(|| Error::msg("LLVM backend: i64 overflow in *"))?),
                ("/", [a, b]) => {
                    let aa = eval_const_i64(bindings, visiting, a)?;
                    let bb = eval_const_i64(bindings, visiting, b)?;
                    if bb == 0 {
                        return Err(Error::msg("LLVM backend: division by zero"));
                    }
                    Ok(aa / bb)
                }
                _ => Err(Error::msg(
                    "LLVM backend MVP supports only + - * / on constant i64",
                )),
            }
        }
        _ => Err(Error::msg(
            "LLVM backend MVP supports only integer constants",
        )),
    }
}

fn eval_const_bool(
    bindings: &HashMap<String, ir::IrExpr>,
    visiting: &mut HashMap<String, bool>,
    expr: &ir::IrExpr,
) -> Result<bool> {
    match expr {
        ir::IrExpr::Bool(b) => Ok(*b),
        ir::IrExpr::Var(name) => {
            let Some(e) = bindings.get(name) else {
                return Err(Error::msg(format!(
                    "LLVM backend: unbound var in bool constant: {name}"
                )));
            };
            if visiting.get(name).copied().unwrap_or(false) {
                return Err(Error::msg(format!(
                    "LLVM backend: cyclic constant definition: {name}"
                )));
            }
            visiting.insert(name.clone(), true);
            let r = eval_const_bool(bindings, visiting, e);
            visiting.insert(name.clone(), false);
            r
        }
        ir::IrExpr::Apply { .. } => {
            let (head, args) = collect_apply(expr);
            let ir::IrExpr::Var(name) = head else {
                return Err(Error::msg(
                    "LLVM backend MVP supports only built-in boolean ops on constants",
                ));
            };
            match (name.as_str(), args.as_slice()) {
                ("==", [a, b]) => Ok(eval_const_i64(bindings, visiting, a)?
                    == eval_const_i64(bindings, visiting, b)?),
                ("__eq", [dict, a, b]) => {
                    // Typechecker lowers `(==)` to `__eq <dict> a b`.
                    match dict {
                        ir::IrExpr::Var(d) if d == "__builtinEqDict" => {}
                        _ => {
                            return Err(Error::msg(
                                "LLVM backend MVP supports only __eq __builtinEqDict on constants",
                            ))
                        }
                    }
                    Ok(eval_const_i64(bindings, visiting, a)?
                        == eval_const_i64(bindings, visiting, b)?)
                }
                ("<", [a, b]) => Ok(eval_const_i64(bindings, visiting, a)?
                    < eval_const_i64(bindings, visiting, b)?),
                ("<=", [a, b]) => Ok(eval_const_i64(bindings, visiting, a)?
                    <= eval_const_i64(bindings, visiting, b)?),
                (">", [a, b]) => Ok(eval_const_i64(bindings, visiting, a)?
                    > eval_const_i64(bindings, visiting, b)?),
                (">=", [a, b]) => Ok(eval_const_i64(bindings, visiting, a)?
                    >= eval_const_i64(bindings, visiting, b)?),
                ("not", [a]) => Ok(!eval_const_bool(bindings, visiting, a)?),
                _ => Err(Error::msg(
                    "LLVM backend MVP supports only comparisons/not on constants",
                )),
            }
        }
        _ => Err(Error::msg(
            "LLVM backend MVP supports only bool constants",
        )),
    }
}

fn append_const_char_list(
    bindings: &HashMap<String, ir::IrExpr>,
    visiting: &mut HashMap<String, bool>,
    head: &ir::IrExpr,
    tail: &ir::IrExpr,
    out: &mut String,
) -> Result<()> {
    match head {
        ir::IrExpr::Char(c) => out.push(*c),
        _ => {
            return Err(Error::msg(
                "LLVM backend: cons head must be a Char constant",
            ))
        }
    }

    match tail {
        ir::IrExpr::List(es) => {
            for e in es {
                match e {
                    ir::IrExpr::Char(c) => out.push(*c),
                    _ => {
                        return Err(Error::msg(
                            "LLVM backend: cons tail list must be a [Char] constant",
                        ))
                    }
                }
            }
            Ok(())
        }
        ir::IrExpr::Cons { head, tail } => append_const_char_list(bindings, visiting, head, tail, out),
        ir::IrExpr::Var(name) => {
            let Some(e) = bindings.get(name) else {
                return Err(Error::msg(format!(
                    "LLVM backend: unbound var in char list: {name}"
                )));
            };
            if visiting.get(name).copied().unwrap_or(false) {
                return Err(Error::msg(format!(
                    "LLVM backend: cyclic char list definition: {name}"
                )));
            }
            visiting.insert(name.clone(), true);
            let s = eval_const_string(bindings, visiting, e)?;
            visiting.insert(name.clone(), false);
            out.push_str(&s);
            Ok(())
        }
        _ => Err(Error::msg(
            "LLVM backend: cons tail must be a constant [Char]",
        )),
    }
}

fn escape_llvm_bytes(s: &str) -> String {
    // LLVM IR: c"...". We emit UTF-8 bytes, escaping non-printables as \XX.
    let mut out = String::new();
    for &b in s.as_bytes() {
        match b {
            b'\\' => out.push_str("\\5C"),
            b'\n' => out.push_str("\\0A"),
            b'\r' => out.push_str("\\0D"),
            b'\t' => out.push_str("\\09"),
            b'"' => out.push_str("\\22"),
            0x20..=0x7e => out.push(b as char),
            _ => out.push_str(&format!("\\{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llvm_backend_emits_printf_and_string_global() {
        let module = ir::IrModule {
            items: vec![ir::IrItem::Binding {
                name: "main".to_string(),
                expr: ir::IrExpr::IoThen {
                    first: Box::new(ir::IrExpr::Apply {
                        func: Box::new(ir::IrExpr::Var("stdoutWrite".to_string())),
                        args: vec![ir::IrExpr::String("hi\n".to_string())],
                    }),
                    then_expr: Box::new(ir::IrExpr::Apply {
                        func: Box::new(ir::IrExpr::Var("IO".to_string())),
                        args: vec![ir::IrExpr::Unit],
                    }),
                },
            }],
        };

        let text = lower_ir_to_llvm_text(&module, "test").unwrap();
        assert!(text.contains("declare i32 @printf"));
        assert!(text.contains("define i32 @main"));
        assert!(text.contains("@.str0"));
        assert!(text.contains("hi"));
        assert!(text.contains("\\0A"));
    }

    #[test]
    fn llvm_backend_constant_folds_int_to_string() {
        let module = ir::IrModule {
            items: vec![ir::IrItem::Binding {
                name: "main".to_string(),
                expr: ir::IrExpr::IoThen {
                    first: Box::new(ir::IrExpr::Apply {
                        func: Box::new(ir::IrExpr::Var("stdoutWrite".to_string())),
                        args: vec![ir::IrExpr::Apply {
                            func: Box::new(ir::IrExpr::Var("intToString".to_string())),
                            args: vec![ir::IrExpr::Apply {
                                func: Box::new(ir::IrExpr::Var("+".to_string())),
                                args: vec![
                                    ir::IrExpr::Integer("1".to_string()),
                                    ir::IrExpr::Integer("2".to_string()),
                                ],
                            }],
                        }],
                    }),
                    then_expr: Box::new(ir::IrExpr::Apply {
                        func: Box::new(ir::IrExpr::Var("IO".to_string())),
                        args: vec![ir::IrExpr::Unit],
                    }),
                },
            }],
        };

        let text = lower_ir_to_llvm_text(&module, "test").unwrap();
        assert!(text.contains("@.str0"));
        assert!(text.contains("\"3\\00\""));
    }
}
