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

use crate::lang;
use crate::lang::ast::{InputExpr, TypedInput};
use crate::lang::interp::{self, Env};
use crate::lang::types::Value;

/// Run the script at `path`, print its result, and exit.
///
/// Descriptors the script opened are closed exactly once, whether it
/// finished or failed, and before the process exits — `process::exit` runs
/// no destructors, so this cannot be left to a guard.
pub fn run(path: &str, args: &[String]) -> ! {
    let outcome = execute(path, args);
    lang::builtins::close_all_fds();
    match outcome {
        // The value alone, so `$(bullscript x.busc)` is the value. A script
        // that ends in a discard prints nothing.
        Ok(Some(v)) => { println!("{}", v); process::exit(0) }
        Ok(None)    => process::exit(0),
        Err(e)      => { eprintln!("{}: {}", path, e); process::exit(1) }
    }
}

fn execute(path: &str, args: &[String]) -> Result<Option<Value>, String> {
    let src = std::fs::read_to_string(path)
        .map_err(|e| format!("could not read: {}", e))?;

    let program = lang::parse_source(&src).map_err(|e| e.to_string())?;
    if program.is_empty() {
        return Err("the script contains no pipes".to_string());
    }

    // Static pass: everything checkable is checked before anything runs.
    lang::check_with_bag(&program, &HashMap::new()).map_err(|e| e.to_string())?;

    let expected = &program[0].inputs;

    // Named slots only. A literal and a data field are values the script
    // already carries, so neither is a command-line parameter.
    let params: Vec<&TypedInput> = expected.iter()
        .filter(|i| matches!(i.expr, InputExpr::Var(_)))
        .collect();

    if args.len() != params.len() {
        let names: Vec<String> = params.iter()
            .map(|i| match &i.expr {
                InputExpr::Var(n) => format!("<{}: {}>", n, i.ty),
                InputExpr::Lit(_) | InputExpr::Data(_) => unreachable!("filtered above"),
            })
            .collect();
        let usage = if names.is_empty() {
            format!("usage: bullscript {}   (it takes no arguments)", path)
        } else {
            format!("usage: bullscript {} {}", path, names.join(" "))
        };
        return Err(format!(
            "expects {} argument(s), got {}\n  {}", params.len(), args.len(), usage
        ));
    }

    // Slot by slot: a literal supplies itself, a named slot takes the next
    // argument. The order of `values` still matches the input list, so
    // everything downstream is unchanged.
    let mut values = Vec::with_capacity(expected.len());
    let mut next_arg = args.iter();
    for input in expected {
        match &input.expr {
            InputExpr::Lit(lit) => values.push(interp::literal_value(lit)),
            // Read at the point the pipe runs, like any other data access.
            // This is the first pipe, before any binding exists, so a `[key]`
            // here has nothing to resolve against — a dynamic key is for
            // later pipes, once the parameter it reads is in scope.
            InputExpr::Data(r) => values.push(crate::data::read_field(r, &Env::new())?),
            InputExpr::Var(_) => {
                let text = next_arg.next().expect("counts checked above");
                values.push(Value::parse_as(text, input.ty)
                    .map_err(|e| format!("argument {}", e))?);
            }
        }
    }

    let mut env: Env = interp::seed_params(&program, &values).map_err(|e| e.to_string())?;
    interp::run_first_and_rest(&program, &values, &mut env, 0).map_err(|e| e.to_string())
}
