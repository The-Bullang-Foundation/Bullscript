//! `bullscript <file.busc> [args...]` — run a script non-interactively.
//!
//! The file is parsed, type checked in full, and only then executed, so a
//! type error can never surface after a `builtin::out` or `builtin::run` has
//! already taken effect.
//!
//! The first pipe's **named** slots are the script's parameters. A literal
//! slot is a value the script already carries, so it keeps it.
//!
//! Every slot used to be a parameter, literals included. That is right when a
//! bag entry is called from a pipe — the caller is filling the slots, and a
//! literal fd being a slot is what lets one entry write to stdout or stderr
//! depending on who calls it. On the command line there is no caller, and the
//! rule meant a script with nothing missing still refused to run:
//!
//! ```text
//! (1: i64, "Hello, world!\n": String) : builtin::out -> {ok: bool};
//!
//! $ bullscript hello.busc
//! hello.busc expects 2 argument(s), got 0
//! ```
//!
//! You had to type the script's own literals back at it. Worse, doing so
//! *overwrote* them — `bullscript hello.busc 2 "x"` printed `x` to stderr,
//! silently substituting values the script had written into itself.
//!
//! So a literal is never taken from the command line and never overridden by
//! it. `bag::` calls are untouched: they do not come through here.

use std::collections::HashMap;
use std::process;

use crate::bag;
use crate::lang;
use crate::lang::interp::{self, Env};
use crate::lang::types::Value;

pub fn run(path: &str, args: &[String]) {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => fail(&format!("could not read '{}': {}", path, e)),
    };

    let program = match lang::parse_source(&src) {
        Ok(p) => p,
        Err(e) => fail(&format!("{}: {}", path, e)),
    };

    if program.is_empty() {
        fail(&format!("{}: the script contains no pipes", path));
    }

    // Static pass: everything checkable is checked before anything runs.
    if let Err(e) = lang::check::check_program(&program, &HashMap::new(), &|n| bag::signature(n)) {
        fail(&format!("{}: {}", path, e));
    }

    let expected = &program[0].inputs;

    // Only the named slots are parameters.
    let params: Vec<&lang::ast::TypedInput> = expected.iter()
        // Named slots only. A literal and a data field are values the script
        // already carries, so neither is a command-line parameter.
        .filter(|i| matches!(i.expr, lang::ast::InputExpr::Var(_)))
        .collect();

    if args.len() != params.len() {
        let names: Vec<String> = params.iter()
            .map(|i| match &i.expr {
                lang::ast::InputExpr::Var(n) => format!("<{}: {}>", n, i.ty),
                lang::ast::InputExpr::Lit(_) | lang::ast::InputExpr::Data(_) =>
                unreachable!("filtered above"),
            })
            .collect();
        let usage = if names.is_empty() {
            format!("usage: bullscript {}   (it takes no arguments)", path)
        } else {
            format!("usage: bullscript {} {}", path, names.join(" "))
        };
        fail(&format!(
            "{} expects {} argument(s), got {}\n  {}",
            path, params.len(), args.len(), usage
        ));
    }

    // Slot by slot: a literal supplies itself, a named slot takes the next
    // argument. The order of `values` still matches the input list, so
    // everything downstream is unchanged.
    let mut values = Vec::with_capacity(expected.len());
    let mut next_arg = args.iter();
    for input in expected {
        match &input.expr {
            lang::ast::InputExpr::Lit(lit) => {
                values.push(lang::interp::literal_value(lit));
            }
            // Read at the point the pipe runs, like any other data access.
            lang::ast::InputExpr::Data(r) => {
                match crate::data::read_field(r) {
                    Ok(v)  => values.push(v),
                    Err(e) => fail(&format!("{}: {}", path, e)),
                }
            }
            lang::ast::InputExpr::Var(_) => {
                let text = next_arg.next().expect("counts checked above");
                match Value::parse_as(text, input.ty) {
                    Ok(v)  => values.push(v),
                    Err(e) => fail(&format!("{}: argument {}", path, e)),
                }
            }
        }
    }

    let mut env: Env = match interp::seed_params(&program, &values) {
        Ok(e) => e,
        Err(e) => fail(&format!("{}: {}", path, e)),
    };

    let result = match interp::run_first_and_rest(&program, &values, &mut env, 0) {
        Ok(r) => r,
        Err(e) => {
            lang::builtins::close_all_fds();
            fail(&format!("{}: {}", path, e));
        }
    };

    lang::builtins::close_all_fds();

    match result {
        Some(v) => println!("Script executed. Returned value: {}", v),
        None    => println!("Script executed. No returned value."),
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("{}", msg);
    lang::builtins::close_all_fds();
    process::exit(1);
}
