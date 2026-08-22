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

/// Type check a program with `bag::` calls resolved against the bag.
///
/// This is how every caller checks — the prompt, a script, the language
/// server, the bag checking an entry — so a `bag::` call is typed the same
/// way everywhere. `scope` is what is already bound: the prompt's live
/// bindings, or nothing.
pub fn check_with_bag(
    pipes: &Program,
    scope: &std::collections::HashMap<String, types::BsType>,
) -> Result<check::Signature, BsError> {
    check::check_program(pipes, scope, &|name| crate::bag::signature(name))
}
