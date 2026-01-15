//! LLVM IR generation for kscr
//!
//! This crate provides conversion from kscr's IR to LLVM IR text format.
//! It generates LLVM IR as a string that can be compiled with `llc` or `clang`.
//!
//! Usage:
//! ```ignore
//! use kscr_llvm::LLVMIRGenerator;
//! 
//! let mut gen = LLVMIRGenerator::new("my_module");
//! gen.generate_placeholder_main();
//! println!("{}", gen.to_string());
//! ```

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;

use kscr_ir::ir;

type Result<T> = std::result::Result<T, String>;

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
    Err("no `main` binding found".to_string())
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
                return Err(
                    "LLVM backend MVP supports only calls to IO/stdoutWrite/print".to_string(),
                );
            };

            if name == "IO" {
                if args.len() == 1 {
                    return Ok(());
                }
                return Err("LLVM backend: IO expects 1 arg".to_string());
            }

            if name == "stdoutWrite" || name == "print" {
                if args.len() != 1 {
                    return Err("LLVM backend: stdoutWrite expects 1 arg".to_string());
                }
                let s = eval_const_string(bindings, visiting, &args[0])?;
                out.push(Action::StdoutWrite(s));
                return Ok(());
            }

            Err(
                "LLVM backend MVP supports only IO (), stdoutWrite/print <string>, and IoThen"
                    .to_string(),
            )
        }
        _ => Err("LLVM backend MVP expects main to be an IO expression".to_string()),
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
                        return Err(
                            "LLVM backend: list literal must be a [Char] constant".to_string(),
                        )
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
                return Err(
                    "LLVM backend MVP supports only calls to built-in conversion functions"
                        .to_string(),
                );
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
                _ => Err(
                    "LLVM backend MVP supports only intToString/boolToString on constants"
                        .to_string(),
                ),
            }
        }
        ir::IrExpr::Var(name) => {
            let Some(e) = bindings.get(name) else {
                return Err(format!("LLVM backend: unknown variable in const string: {name}"));
            };
            if visiting.get(name).copied().unwrap_or(false) {
                return Err("LLVM backend: cyclic const string".to_string());
            }
            visiting.insert(name.clone(), true);
            let v = eval_const_string(bindings, visiting, e);
            visiting.insert(name.clone(), false);
            v
        }
        _ => Err("LLVM backend MVP expects string argument to be a constant".to_string()),
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
            return Err("LLVM backend: cons head must be a Char constant".to_string())
        }
    }
    match tail {
        ir::IrExpr::List(es) => {
            for e in es {
                match e {
                    ir::IrExpr::Char(c) => out.push(*c),
                    _ => {
                        return Err(
                            "LLVM backend: list literal must be a [Char] constant".to_string(),
                        )
                    }
                }
            }
            Ok(())
        }
        ir::IrExpr::Cons {
            head: h,
            tail: t,
        } => append_const_char_list(bindings, visiting, h, t, out),
        ir::IrExpr::Var(_) => {
            let s = eval_const_string(bindings, visiting, tail)?;
            out.push_str(&s);
            Ok(())
        }
        _ => Err("LLVM backend: cons tail must be a [Char] constant".to_string()),
    }
}

fn collect_apply(expr: &ir::IrExpr) -> (&ir::IrExpr, Vec<&ir::IrExpr>) {
    let mut head = expr;
    let mut args_acc: Vec<&ir::IrExpr> = Vec::new();
    loop {
        match head {
            ir::IrExpr::Apply { func, args } => {
                for a in args.iter().rev() {
                    args_acc.push(a);
                }
                head = func;
            }
            _ => break,
        }
    }
    args_acc.reverse();
    (head, args_acc)
}

fn eval_const_i64(
    bindings: &HashMap<String, ir::IrExpr>,
    visiting: &mut HashMap<String, bool>,
    expr: &ir::IrExpr,
) -> Result<i64> {
    match expr {
        ir::IrExpr::Integer(s) => s
            .parse::<i64>()
            .map_err(|e| format!("LLVM backend: invalid i64 literal: {e}")),
        ir::IrExpr::Var(name) => {
            let Some(e) = bindings.get(name) else {
                return Err(format!("LLVM backend: unknown variable in const int: {name}"));
            };
            if visiting.get(name).copied().unwrap_or(false) {
                return Err("LLVM backend: cyclic const int".to_string());
            }
            visiting.insert(name.clone(), true);
            let v = eval_const_i64(bindings, visiting, e);
            visiting.insert(name.clone(), false);
            v
        }
        _ => Err(
            "LLVM backend MVP expects intToString argument to be an Integer constant".to_string(),
        ),
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
                return Err(format!("LLVM backend: unknown variable in const bool: {name}"));
            };
            if visiting.get(name).copied().unwrap_or(false) {
                return Err("LLVM backend: cyclic const bool".to_string());
            }
            visiting.insert(name.clone(), true);
            let v = eval_const_bool(bindings, visiting, e);
            visiting.insert(name.clone(), false);
            v
        }
        _ => Err(
            "LLVM backend MVP expects boolToString argument to be a Bool constant".to_string(),
        ),
    }
}

fn escape_llvm_bytes(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        match *b {
            b'\\' => out.push_str("\\5C"),
            b'\n' => out.push_str("\\0A"),
            b'\r' => out.push_str("\\0D"),
            b'\t' => out.push_str("\\09"),
            b'\"' => out.push_str("\\22"),
            0x20..=0x7E => out.push(*b as char),
            _ => {
                write!(&mut out, "\\{:02X}", b).unwrap();
            }
        }
    }
    out
}

/// LLVM IR generator that produces textual LLVM IR
pub struct LLVMIRGenerator {
    module_name: String,
    output: String,
    #[allow(dead_code)] // Reserved for future IR generation
    counter: usize,
}

impl LLVMIRGenerator {
    /// Create a new LLVM IR generator
    pub fn new(module_name: &str) -> Self {
        let mut gen = LLVMIRGenerator {
            module_name: module_name.to_string(),
            output: String::new(),
            counter: 0,
        };
        gen.emit_header();
        gen
    }

    /// Emit LLVM IR module header
    fn emit_header(&mut self) {
        writeln!(
            &mut self.output,
            "; ModuleID = '{}'",
            self.module_name
        )
        .unwrap();
        writeln!(&mut self.output, "source_filename = \"{}\"", self.module_name).unwrap();
        writeln!(&mut self.output, "target datalayout = \"e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-f80:128-n8:16:32:64-S128\"").unwrap();
        writeln!(&mut self.output, "target triple = \"x86_64-unknown-linux-gnu\"").unwrap();
        writeln!(&mut self.output).unwrap();
    }

    /// Generate a new unique label/register
    /// Reserved for future expression lowering
    #[allow(dead_code)]
    fn gen_label(&mut self, prefix: &str) -> String {
        let id = self.counter;
        self.counter += 1;
        format!("{}{}", prefix, id)
    }

    /// Emit runtime function declarations
    fn emit_runtime_declarations(&mut self) {
        writeln!(&mut self.output, "; Runtime function declarations").unwrap();
        writeln!(&mut self.output).unwrap();

        // Thunk structure type: { i8*, i8*, i8*, i32 }
        writeln!(
            &mut self.output,
            "%struct.kscr_thunk = type {{ i8*, i8*, i8*, i32 }}"
        )
        .unwrap();
        writeln!(&mut self.output).unwrap();

        // Value structure type (tagged union): { i32, i8* }
        writeln!(&mut self.output, "%struct.kscr_value = type {{ i32, i8* }}").unwrap();
        writeln!(&mut self.output).unwrap();

        // Force thunk runtime function
        writeln!(
            &mut self.output,
            "declare %struct.kscr_value @kscr_force_thunk(%struct.kscr_thunk*)"
        )
        .unwrap();
        writeln!(&mut self.output).unwrap();

        // IO action executor
        writeln!(
            &mut self.output,
            "declare %struct.kscr_value @kscr_execute_io(%struct.kscr_value*)"
        )
        .unwrap();
        writeln!(&mut self.output).unwrap();

        // Memory allocation
        writeln!(&mut self.output, "declare i8* @malloc(i64)").unwrap();
        writeln!(&mut self.output).unwrap();

        // Standard C library functions
        writeln!(&mut self.output, "declare i32 @puts(i8*)").unwrap();
        writeln!(&mut self.output, "declare i32 @printf(i8*, ...)").unwrap();
        writeln!(&mut self.output).unwrap();
    }

    /// Generate a placeholder main function
    pub fn generate_placeholder_main(&mut self) {
        self.emit_runtime_declarations();

        // String constant for message
        let msg = "LLVM IR generated by kscr";
        writeln!(
            &mut self.output,
            "@.str = private unnamed_addr constant [{} x i8] c\"{}\\00\", align 1",
            msg.len() + 1,
            msg
        )
        .unwrap();
        writeln!(&mut self.output).unwrap();

        // Main function
        writeln!(&mut self.output, "define i32 @main() {{").unwrap();
        writeln!(&mut self.output, "entry:").unwrap();
        writeln!(
            &mut self.output,
            "  %0 = getelementptr inbounds [{} x i8], [{} x i8]* @.str, i64 0, i64 0",
            msg.len() + 1,
            msg.len() + 1
        )
        .unwrap();
        writeln!(&mut self.output, "  %1 = call i32 @puts(i8* %0)").unwrap();
        writeln!(&mut self.output, "  ret i32 0").unwrap();
        writeln!(&mut self.output, "}}").unwrap();
        writeln!(&mut self.output).unwrap();
    }

    /// Generate LLVM IR for integer arithmetic
    pub fn generate_integer_add_function(&mut self) {
        writeln!(&mut self.output, "; Integer addition function").unwrap();
        writeln!(
            &mut self.output,
            "define i64 @kscr_add_i64(i64 %a, i64 %b) {{"
        )
        .unwrap();
        writeln!(&mut self.output, "entry:").unwrap();
        writeln!(&mut self.output, "  %result = add i64 %a, %b").unwrap();
        writeln!(&mut self.output, "  ret i64 %result").unwrap();
        writeln!(&mut self.output, "}}").unwrap();
        writeln!(&mut self.output).unwrap();
    }

    /// Get the generated LLVM IR as a string
    pub fn to_string(&self) -> String {
        self.output.clone()
    }
}

/// Generate LLVM IR text for a module (simplified version)
/// This generates a placeholder main function as a proof of concept
pub fn generate_llvm_ir_text(module_name: &str) -> Result<String, String> {
    let mut gen = LLVMIRGenerator::new(module_name);
    gen.generate_placeholder_main();
    gen.generate_integer_add_function();
    Ok(gen.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_basic_llvm_ir() {
        let ir = generate_llvm_ir_text("test_module").unwrap();
        assert!(ir.contains("define i32 @main()"));
        assert!(ir.contains("LLVM IR generated by kscr"));
        assert!(ir.contains("ret i32 0"));
    }

    #[test]
    fn test_generator_has_runtime_declarations() {
        let mut gen = LLVMIRGenerator::new("test");
        gen.emit_runtime_declarations();
        let ir = gen.to_string();
        
        assert!(ir.contains("kscr_force_thunk"));
        assert!(ir.contains("kscr_execute_io"));
        assert!(ir.contains("malloc"));
        assert!(ir.contains("puts"));
    }

    #[test]
    fn test_generator_has_thunk_and_value_types() {
        let mut gen = LLVMIRGenerator::new("test");
        gen.emit_runtime_declarations();
        let ir = gen.to_string();
        
        assert!(ir.contains("%struct.kscr_thunk"));
        assert!(ir.contains("%struct.kscr_value"));
    }

    #[test]
    fn test_integer_add_function() {
        let mut gen = LLVMIRGenerator::new("test");
        gen.generate_integer_add_function();
        let ir = gen.to_string();
        
        assert!(ir.contains("define i64 @kscr_add_i64(i64 %a, i64 %b)"));
        assert!(ir.contains("add i64 %a, %b"));
    }

    #[test]
    fn test_lower_ir_to_llvm_text_mvp_print() {
        let module = ir::IrModule {
            items: vec![ir::IrItem::Binding {
                name: "main".to_string(),
                expr: ir::IrExpr::IoThen {
                    first: Box::new(ir::IrExpr::Apply {
                        func: Box::new(ir::IrExpr::Var("print".to_string())),
                        args: vec![ir::IrExpr::String("hello".to_string())],
                    }),
                    then_expr: Box::new(ir::IrExpr::Apply {
                        func: Box::new(ir::IrExpr::Var("IO".to_string())),
                        args: vec![ir::IrExpr::Unit],
                    }),
                },
            }],
        };

        let text = lower_ir_to_llvm_text(&module, "test").unwrap();
        assert!(text.contains("define i32 @main()"));
        assert!(text.contains("declare i32 @printf(i8*, ...)"));
        assert!(text.contains("@.fmt"));
        assert!(text.contains("hello"));
    }
}
