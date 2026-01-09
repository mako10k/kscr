#![forbid(unsafe_code)]

pub mod ast;
pub mod cli;
pub mod debug;
pub mod error;
pub mod ir;
pub mod lexer;
pub mod parser;
#[cfg(not(feature = "unsafe_bigint"))]
mod safe_bigint;
pub mod types;

pub type Result<T> = std::result::Result<T, error::Error>;

#[cfg(test)]
mod lib_test;
