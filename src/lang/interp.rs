//! Tree-walk interpreter.
//!
//! No compile step, no stored binary: a `.busc` file is interpreted every
//! time it runs, whether that's `bullscript file.busc`, a line typed at the
//! prompt, or a `bag::name` call reached from inside another pipe.
//!
//! Everything here runs *after* `lang::check`, so types are already known to
//! line up. The runtime checks that remain are the ones a static pass cannot
//! make: integer overflow, division by zero, and I/O failure.
//!
//! A bag entry's prototype is never stored separately — it *is* the first
//! pipe's input list (parameter types) plus the last pipe's binding (return
//! type). Calling a bag script means: load its parsed program from the bag
//! cache, seed a fresh environment from the first pipe's declared names using
//! the caller's argument values, then run every pipe like any other program.

use std::collections::HashMap;

use super::ast::*;
use super::builtins;
use super::error::BsError;
use super::types::Value;
use crate::bag;

pub type Env = HashMap<String, Value>;

/// Guards against a bag entry (directly or transitively) calling itself.
const MAX_CALL_DEPTH: usize = 64;

/// Run a full program (already parsed and checked), threading `env` through
/// every pipe. Returns the last pipe's bound value, if any (a program ending
/// in a discard `{}` returns None).
pub fn run_program(pipes: &Program, env: &mut Env) -> Result<Option<Value>, BsError> {
    run_program_depth(pipes, env, 0)
}

fn run_program_depth(pipes: &Program, env: &mut Env, depth: usize) -> Result<Option<Value>, BsError> {
    let mut last: Option<Value> = None;
    for pipe in pipes {
        last = run_pipe(pipe, env, depth)?;
    }
    Ok(last)
}

/// Bind a program's parameters from a list of caller-supplied values.
///
/// Every slot in the first pipe's input list is a parameter, literal or not:
/// the caller supplies a value for each. A variable slot binds its value into
/// the environment under that name; a literal slot has no name, so its value
/// is consumed by the first pipe and never bound.
pub fn seed_params(pipes: &Program, args: &[Value]) -> Result<Env, BsError> {
    let mut env = Env::new();
    let Some(first) = pipes.first() else { return Ok(env) };

    if first.inputs.len() != args.len() {
        return Err(BsError::at(first.line, format!(
            "expected {} argument(s), got {}", first.inputs.len(), args.len()
        )));
    }

    for (input, arg) in first.inputs.iter().zip(args) {
        if arg.ty() != input.ty {
            let what = match &input.expr {
                InputExpr::Var(n) => format!("parameter '{}'", n),
                InputExpr::Lit(_) => "this argument".to_string(),
            };
            return Err(BsError::at(input.line, format!(
                "{} expects {}, got {}", what, input.ty, arg.ty()
            )));
        }
        if let InputExpr::Var(name) = &input.expr {
            env.insert(name.clone(), arg.clone());
        }
    }
    Ok(env)
}

/// Override the first pipe's literal slots with caller-supplied values, so a
/// literal position behaves exactly like a variable one for the pipe itself.
fn args_for_first_pipe(pipe: &Pipe, seeded: &[Value]) -> Vec<Value> {
    seeded.to_vec().into_iter().take(pipe.inputs.len()).collect()
}

fn run_pipe(pipe: &Pipe, env: &mut Env, depth: usize) -> Result<Option<Value>, BsError> {
    let mut args = Vec::with_capacity(pipe.inputs.len());
    for input in &pipe.inputs {
        args.push(resolve_input(input, env)?);
    }

    // A bag entry may legitimately produce nothing; the binding below decides
    // whether that is a problem.
    if let PipeVal::Call(Callee::Bag(name)) = &pipe.val {
        let produced = call_bag_entry_opt(name, &args, pipe.line, depth)?;
        return match (&pipe.binding, produced) {
            (Binding::Discard, _) => Ok(None),
            (Binding::Bound { name: b, .. }, Some(v)) => {
                env.insert(b.clone(), v.clone());
                Ok(Some(v))
            }
            (Binding::Bound { name: b, .. }, None) => Err(BsError::at(pipe.line, format!(
                "'bag::{}' ends in a discard '{{}}', so there is nothing for '{}' to hold",
                name, b
            ))),
        };
    }

    let result = match &pipe.val {
        PipeVal::Call(Callee::Builtin(name)) => {
            builtins::call(name, &args)
                .map_err(|e| BsError::at(pipe.line, e.message))?
        }
        PipeVal::Call(Callee::Bag(name)) => {
            call_bag_entry(name, &args, pipe.line, depth)?
        }
        PipeVal::Expr(expr) => {
            let local: Env = pipe.inputs.iter().zip(&args)
                .filter_map(|(i, v)| match &i.expr {
                    InputExpr::Var(n) => Some((n.clone(), v.clone())),
                    InputExpr::Lit(_) => None,
                })
                .collect();
            eval_expr(expr, &local, pipe.line)?
        }
    };

    match &pipe.binding {
        Binding::Discard => Ok(None),
        Binding::Bound { name, .. } => {
            env.insert(name.clone(), result.clone());
            Ok(Some(result))
        }
    }
}

fn resolve_input(input: &TypedInput, env: &Env) -> Result<Value, BsError> {
    match &input.expr {
        InputExpr::Lit(lit) => Ok(literal_value(lit)),
        InputExpr::Var(name) => env.get(name).cloned().ok_or_else(|| {
            BsError::at(input.line, format!("undefined variable '{}'", name))
        }),
    }
}

pub fn literal_value(lit: &Literal) -> Value {
    match lit {
        Literal::I64(n)  => Value::I64(*n),
        Literal::F64(x)  => Value::F64(*x),
        Literal::Bool(b) => Value::Bool(*b),
        Literal::Str(s)  => Value::Str(s.clone()),
    }
}

/// Run a bag entry and return whatever it produced.
///
/// `None` when the entry ends in a discard. That is not an error here: the
/// type checker has already established that the caller discards too, and an
/// entry whose purpose is a side effect — printing, writing a file — is a
/// perfectly ordinary thing to keep in a bag.
fn call_bag_entry_opt(name: &str, args: &[Value], line: usize, depth: usize) -> Result<Option<Value>, BsError> {
    if depth >= MAX_CALL_DEPTH {
        return Err(BsError::at(line, format!(
            "call depth exceeded calling bag::{} — likely a recursive cycle", name
        )));
    }

    let pipes = bag::program(name).map_err(|e| BsError::at(line, e))?;

    let mut callee_env = seed_params(&pipes, args)
        .map_err(|e| BsError::at(line, format!("bag::{}: {}", name, e.message)))?;

    // A literal slot in the first pipe is a parameter too, so the caller's
    // value must replace the literal when that pipe runs.
    let effective = args_for_first_pipe(&pipes[0], args);
    let result = run_first_and_rest(&pipes, &effective, &mut callee_env, depth + 1)?;

    Ok(result)
}

/// As above, for the one caller that needs a value: a bag entry used inside a
/// larger expression, where there is nowhere to put "nothing".
fn call_bag_entry(name: &str, args: &[Value], line: usize, depth: usize) -> Result<Value, BsError> {
    call_bag_entry_opt(name, args, line, depth)?.ok_or_else(|| BsError::at(line, format!(
        "'bag::{}' ends in a discard '{{}}', so it produces no value to use here", name
    )))
}

/// Run a program where the first pipe's inputs come from `args` rather than
/// from its own literals, then run the remaining pipes normally.
pub fn run_first_and_rest(
    pipes: &Program,
    args:  &[Value],
    env:   &mut Env,
    depth: usize,
) -> Result<Option<Value>, BsError> {
    let mut last = run_pipe_with_args(&pipes[0], args, env, depth)?;
    for pipe in &pipes[1..] {
        last = run_pipe(pipe, env, depth)?;
    }
    Ok(last)
}

fn run_pipe_with_args(
    pipe:  &Pipe,
    args:  &[Value],
    env:   &mut Env,
    depth: usize,
) -> Result<Option<Value>, BsError> {
    // Same as in run_pipe: a bag entry may produce nothing, and the binding
    // decides whether that matters. This path is the *first* pipe of a
    // program, which is where a script that only calls one bag entry lives.
    if let PipeVal::Call(Callee::Bag(name)) = &pipe.val {
        let produced = call_bag_entry_opt(name, args, pipe.line, depth)?;
        return match (&pipe.binding, produced) {
            (Binding::Discard, _) => Ok(None),
            (Binding::Bound { name: b, .. }, Some(v)) => {
                env.insert(b.clone(), v.clone());
                Ok(Some(v))
            }
            (Binding::Bound { name: b, .. }, None) => Err(BsError::at(pipe.line, format!(
                "'bag::{}' ends in a discard '{{}}', so there is nothing for '{}' to hold",
                name, b
            ))),
        };
    }

    let result = match &pipe.val {
        PipeVal::Call(Callee::Builtin(name)) => {
            builtins::call(name, args).map_err(|e| BsError::at(pipe.line, e.message))?
        }
        // Handled above.
        PipeVal::Call(Callee::Bag(_)) => unreachable!(),
        PipeVal::Expr(expr) => {
            let local: Env = pipe.inputs.iter().zip(args)
                .filter_map(|(i, v)| match &i.expr {
                    InputExpr::Var(n) => Some((n.clone(), v.clone())),
                    InputExpr::Lit(_) => None,
                })
                .collect();
            eval_expr(expr, &local, pipe.line)?
        }
    };

    match &pipe.binding {
        Binding::Discard => Ok(None),
        Binding::Bound { name, .. } => {
            env.insert(name.clone(), result.clone());
            Ok(Some(result))
        }
    }
}

// ── expression evaluation ─────────────────────────────────────────────────

fn eval_expr(expr: &Expr, env: &Env, line: usize) -> Result<Value, BsError> {
    match expr {
        Expr::Lit(lit) => Ok(literal_value(lit)),

        Expr::Var(name) => env.get(name).cloned().ok_or_else(|| {
            BsError::at(line, format!("undefined variable '{}'", name))
        }),

        Expr::Unary(op, inner) => {
            let v = eval_expr(inner, env, line)?;
            match (op, v) {
                (UnOp::Neg, Value::I64(n)) => n.checked_neg()
                    .map(Value::I64)
                    .ok_or_else(|| BsError::at(line, "integer overflow: cannot negate the smallest i64")),
                (UnOp::Neg, Value::F64(x)) => Ok(Value::F64(-x)),
                (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
                // The static pass rejects every other shape.
                (_, other) => Err(BsError::at(line, format!(
                    "unary operator cannot apply to {}", other.ty()
                ))),
            }
        }

        // `&&` and `||` short-circuit: the right operand is only evaluated
        // when it can still affect the result. This lets a guard like
        // `(n != 0) && (100 / n > 5)` work.
        Expr::Bin(BinOp::And, lhs, rhs) => {
            match eval_expr(lhs, env, line)? {
                Value::Bool(false) => Ok(Value::Bool(false)),
                Value::Bool(true)  => eval_expr(rhs, env, line),
                other => Err(BsError::at(line, format!("'&&' requires bool, found {}", other.ty()))),
            }
        }
        Expr::Bin(BinOp::Or, lhs, rhs) => {
            match eval_expr(lhs, env, line)? {
                Value::Bool(true)  => Ok(Value::Bool(true)),
                Value::Bool(false) => eval_expr(rhs, env, line),
                other => Err(BsError::at(line, format!("'||' requires bool, found {}", other.ty()))),
            }
        }

        Expr::Bin(op, lhs, rhs) => {
            let l = eval_expr(lhs, env, line)?;
            let r = eval_expr(rhs, env, line)?;
            eval_binop(*op, l, r, line)
        }
    }
}

fn eval_binop(op: BinOp, l: Value, r: Value, line: usize) -> Result<Value, BsError> {
    use BinOp::*;
    match op {
        Add | Sub | Mul | Div => arith(op, l, r, line),
        Eq => Ok(Value::Bool(l == r)),
        Ne => Ok(Value::Bool(l != r)),
        Lt | Gt | Le | Ge => compare(op, l, r, line),
        And | Or => unreachable!("handled by eval_expr for short-circuiting"),
    }
}

fn arith(op: BinOp, l: Value, r: Value, line: usize) -> Result<Value, BsError> {
    match (l, r) {
        (Value::I64(a), Value::I64(b)) => {
            let out = match op {
                BinOp::Add => a.checked_add(b),
                BinOp::Sub => a.checked_sub(b),
                BinOp::Mul => a.checked_mul(b),
                BinOp::Div => {
                    if b == 0 {
                        return Err(BsError::at(line, "division by zero"));
                    }
                    a.checked_div(b)
                }
                _ => unreachable!(),
            };
            out.map(Value::I64).ok_or_else(|| BsError::at(line, format!(
                "integer overflow: {} {} {} does not fit in an i64", a, op_symbol(op), b
            )))
        }
        (Value::F64(a), Value::F64(b)) => Ok(Value::F64(match op {
            BinOp::Add => a + b,
            BinOp::Sub => a - b,
            BinOp::Mul => a * b,
            BinOp::Div => a / b,
            _ => unreachable!(),
        })),
        (a, b) => Err(BsError::at(line, format!(
            "arithmetic requires two i64 or two f64 operands, found {} and {}", a.ty(), b.ty()
        ))),
    }
}

fn op_symbol(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+", BinOp::Sub => "-",
        BinOp::Mul => "*", BinOp::Div => "/",
        _ => "?",
    }
}

fn compare(op: BinOp, l: Value, r: Value, line: usize) -> Result<Value, BsError> {
    let ord = match (&l, &r) {
        (Value::I64(a), Value::I64(b)) => a.partial_cmp(b),
        (Value::F64(a), Value::F64(b)) => a.partial_cmp(b),
        (a, b) => return Err(BsError::at(line, format!(
            "comparison requires two i64 or two f64 operands, found {} and {}", a.ty(), b.ty()
        ))),
    };
    let Some(ord) = ord else {
        return Err(BsError::at(line, "comparison produced no ordering (NaN?)"));
    };
    use std::cmp::Ordering::*;
    Ok(Value::Bool(match op {
        BinOp::Lt => ord == Less,
        BinOp::Gt => ord == Greater,
        BinOp::Le => ord != Greater,
        BinOp::Ge => ord != Less,
        _ => unreachable!(),
    }))
}
