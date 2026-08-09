pub mod ast;
pub mod builtins;
pub mod check;
pub mod error;
pub mod interp;
pub mod lexer;
pub mod parser;
pub mod types;

use ast::Program;
use error::BsError;

/// Lex and parse a full source string into a program (a plain sequence of
/// pipes — the only shape BullScript source ever takes).
pub fn parse_source(src: &str) -> Result<Program, BsError> {
    let tokens = lexer::lex(src)?;
    parser::parse(&tokens)
}
