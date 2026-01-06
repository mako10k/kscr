# Binary Toolchain Design

This document summarizes the design of a binary toolchain for a Haskell-style lazy functional language.

---

## 1. List of Tools and Their Roles

### 1.1 Compiler
- Converts source code to IR (Intermediate Representation), performs type checking and optimization.
- Lowers surface numeric types to LLVM-aligned backend types for literals/FFI; inserts checked casts for potentially lossy conversions.
- Handles conversion from IR to LLVM IR or custom VM bytecode.
- Serves as the foundation for static binary generation and JIT execution.

### 1.2 Interpreter
- Directly executes source code or IR.
- Mainly used for development, debugging, and REPL purposes.

### 1.3 IR Executor
- Interpreter for the custom IR.
- Includes an LLVM JIT executor (IR → LLVM IR → JIT execution).

### 1.4 IR-to-LLVM IR Converter
- Converts custom IR to LLVM IR.
- Used for JIT and static binary generation.

### 1.5 Formatter
- Automatically formats source code.
- Ensures consistent style and readability.

### 1.6 Linter
- Performs static analysis, style checks, and warnings.
- Detects coding standard violations and potential bugs, excluding type checking.

---

## 2. Extension and Optional Tools

### 2.1 Linker
- Combines multiple modules or binaries.
- Required for split compilation of LLVM IR or bytecode.
- Can be omitted for small-scale or scripting use cases.

### 2.2 FFI (Foreign Function Interface)
- Calls external functions such as C libraries.
- Essential for practical use in the future.
- Easy C ABI integration via LLVM IR.

---

## 3. Others
- Package manager (needed in the future)
- Debugger (can be integrated into REPL or interpreter)

---

## 4. Data Flow Between Tools (Example)

```
[Source] 
    | 
    v
[Formatter] --+--> [Linter]
    |           |
    v           v
[Compiler] --> [IR] --> [IR Interpreter | IR→LLVM IR Converter]
                                            |                |
                                            v                v
                                [JIT Executor]      [LLVM Toolchain]
                                            |                |
                                            v                v
                                    [Execution/Binary Generation]
```

---

## 5. Notes
- Tools can be implemented as independent CLI applications or integrated as libraries.
- FFI and linker should be considered for future expansion from the design stage.
- Detailed design and interface specifications for each tool will be defined separately.

