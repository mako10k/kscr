pub mod ast;
pub mod cli;
pub mod error;
pub mod ir;
pub mod lexer;
pub mod parser;
pub mod types;

pub type Result<T> = std::result::Result<T, error::Error>;

#[cfg(test)]
mod lib_test;
