#![forbid(unsafe_code)]

pub mod ast;
// `src/cli.rs` is intentionally compiled as a private module (`cli_impl`) for now,
// while we gradually split it into the `src/cli/` module tree.
pub mod cli;
mod ctor_reexport;
pub mod debug;
pub mod error;
pub mod ir;
pub mod ir_pack;
pub mod kir1;
pub mod ksif;
pub mod lexer;
pub mod parser;

// Build `src/parser_impl.rs` as a public module for testing.
#[path = "parser_impl.rs"]
pub mod parser_impl;
#[cfg(not(feature = "unsafe_bigint"))]
mod safe_bigint;
pub mod types;

#[path = "cli_impl.rs"]
mod cli_impl;

#[cfg(feature = "llvm")]
pub mod llvm_backend;

pub type Result<T> = std::result::Result<T, error::Error>;

#[cfg(test)]
mod lib_test;
