//! Static type checking.
//!
//! Every input and every binding in BullScript carries an explicit type, so
//! checking a program needs no inference at all — it is a straight walk over
//! the pipes, tracking which names are in scope and what type each holds.
//!
//! This runs *before* anything executes. Without it a script could call
//! `builtin::out` or `builtin::run` — writing files, spawning processes —
//! and only then hit a type error that was visible in the source all along.

use std::collections::HashMap;

use super::ast::*;
use super::builtins;
use super::error::BsError;
use super::types::BsType;

/// Type of a program's parameter list and its result: the first pipe's
/// declared input types, and the last pipe's binding type (None when the
/// program ends in a discard).
pub struct Signature {
    pub params: Vec<BsType>,
    pub ret:    Option<BsType>,
}

/// Resolve a `bag::` callee to its signature. The interpreter supplies a
/// closure that loads and checks the entry; the checker itself never touches
/// the filesystem.
pub type BagResolver<'a> = &'a dyn Fn(&str) -> Result<Signature, String>;

/// Check a whole program, starting from an empty scope.
///
/// `seed` names any variables already bound before the program runs — the
/// REPL passes its live environment here so a line can refer to bindings
/// made by earlier lines.
pub fn check_program(
    pipes:   &Program,
    seed:    &HashMap<String, BsType>,
    resolve: BagResolver,
) -> Result<Signature, BsError> {
    let mut scope: HashMap<String, BsType> = seed.clone();
    let mut last_ret: Option<BsType> = None;

    // The first pipe's input list is also the program's parameter list, so a
    // named slot there *declares* that name rather than referring to one that
    // already exists. Anything already in `scope` (the REPL's live bindings)
    // still has to agree with the annotation, which check_pipe verifies.
    if let Some(first) = pipes.first() {
        for input in &first.inputs {
            if let InputExpr::Var(name) = &input.expr {
                scope.entry(name.clone()).or_insert(input.ty);
            }
        }
    }

    for pipe in pipes {
        last_ret = check_pipe(pipe, &mut scope, resolve)?;
    }

    Ok(Signature {
        params: pipes.first()
            .map(|p| p.inputs.iter().map(|i| i.ty).collect())
            .unwrap_or_default(),
        ret: last_ret,
    })
}

/// Check one pipe. Returns the type it produces, or None for a discard.
fn check_pipe(
    pipe:    &Pipe,
    scope:   &mut HashMap<String, BsType>,
    resolve: BagResolver,
) -> Result<Option<BsType>, BsError> {
    // ── inputs ────────────────────────────────────────────────────────────
    let mut arg_tys = Vec::with_capacity(pipe.inputs.len());
    for input in &pipe.inputs {
        match &input.expr {
            InputExpr::Lit(lit) => {
                if lit.ty() != input.ty {
                    return Err(BsError::at(input.line, format!(
                        "literal is annotated as {} but is actually {}", input.ty, lit.ty()
                    )));
                }
            }
            InputExpr::Var(name) => match scope.get(name) {
                Some(actual) if *actual == input.ty => {}
                Some(actual) => return Err(BsError::at(input.line, format!(
                    "'{}' is annotated as {} but holds {}", name, input.ty, actual
                ))),
                None => return Err(BsError::at(input.line, format!(
                    "undefined variable '{}'", name
                ))),
            },
        }
        arg_tys.push(input.ty);
    }

    // ── middle section ────────────────────────────────────────────────────
    let produced = match &pipe.val {
        PipeVal::Call(Callee::Builtin(name)) => {
            let (params, ret) = builtins::prototype(name).ok_or_else(|| {
                BsError::at(pipe.line, format!(
                    "unknown builtin 'builtin::{}' — available builtins are: {}",
                    name, builtins::NAMES.join(", ")
                ))
            })?;
            check_args("builtin", name, &params, &arg_tys, pipe.line)?;
            ret
        }
        PipeVal::Call(Callee::Bag(name)) => {
            let sig = resolve(name).map_err(|e| BsError::at(pipe.line, e))?;
            check_args("bag entry", name, &sig.params, &arg_tys, pipe.line)?;
            match (sig.ret, &pipe.binding) {
                (Some(ret), _) => ret,

                // The entry produces nothing and the caller asked for nothing.
                //
                // This used to be rejected outright, before the binding was
                // even looked at — so a script that exists to print something
                // could be put in a bag and then never called from a pipe,
                // which is most of what a bag is for.
                (None, Binding::Discard) => return Ok(None),

                (None, Binding::Bound { name: binding, .. }) => {
                    return Err(BsError::at(pipe.line, format!(
                        "'bag::{}' ends in a discard '{{}}', so it produces no value \
                         and there is nothing for '{}' to hold. Call it with '-> {{}}'.",
                        name, binding
                    )));
                }
            }
        }
        PipeVal::Expr(expr) => {
            // A bare expression may only mention this pipe's own inputs.
            let local: HashMap<String, BsType> = pipe.inputs.iter()
                .filter_map(|i| match &i.expr {
                    InputExpr::Var(n) => Some((n.clone(), i.ty)),
                    InputExpr::Lit(_) => None,
                })
                .collect();
            check_expr(expr, &local, pipe.line)
                .map_err(|e| name_hint(expr, &local, pipe.line, resolve).unwrap_or(e))?
        }
    };

    // ── binding ───────────────────────────────────────────────────────────
    match &pipe.binding {
        Binding::Discard => Ok(None),
        Binding::Bound { name, ty } => {
            if produced != *ty {
                return Err(BsError::at(pipe.line, format!(
                    "binding '{}' is declared as {} but the pipe produces {}",
                    name, ty, produced
                )));
            }
            scope.insert(name.clone(), *ty);
            Ok(Some(*ty))
        }
    }
}

fn check_args(
    kind: &str,
    name: &str,
    params: &[BsType],
    args: &[BsType],
    line: usize,
) -> Result<(), BsError> {
    if params.len() != args.len() {
        return Err(BsError::at(line, format!(
            "{} '{}' expects {} argument(s), got {}",
            kind, name, params.len(), args.len()
        )));
    }
    for (i, (expected, got)) in params.iter().zip(args).enumerate() {
        if expected != got {
            return Err(BsError::at(line, format!(
                "{} '{}' argument {} expects {}, got {}",
                kind, name, i + 1, expected, got
            )));
        }
    }
    Ok(())
}

/// A better error for `(4, 5) : print_add -> {};`
///
/// A bare name in the value position is a *variable*, so an unknown one is
/// reported as not being one of the pipe's inputs — which is true, and
/// unhelpful when the name is plainly a script the user has in their bag or a
/// builtin they know exists. Calling something in BullScript means saying
/// where it comes from: `bag::print_add`, `builtin::trim`.
///
/// This is where BullScript and Bullang differ, and the difference is easy to
/// trip over: in Bullang `(a, b) : add -> {sum};` calls `add`, because a
/// Bullang folder has one namespace and the inventory says what is in it.
/// BullScript has two — your bag and the builtins — so it asks which.
///
/// Returns None when the name is nothing recognisable, leaving the original
/// error, which is then the right one.
fn name_hint(
    expr:    &Expr,
    local:   &HashMap<String, BsType>,
    line:    usize,
    resolve: BagResolver,
) -> Option<BsError> {
    let Expr::Var(name) = expr else { return None };
    if local.contains_key(name) {
        return None;
    }

    if resolve(name).is_ok() {
        return Some(BsError::at(line, format!(
            "'{}' is a script in your bag. Call it with 'bag::{}'.",
            name, name
        )));
    }

    if super::builtins::prototype(name).is_some() {
        return Some(BsError::at(line, format!(
            "'{}' is a builtin. Call it with 'builtin::{}'.",
            name, name
        )));
    }

    None
}

// ── expressions ───────────────────────────────────────────────────────────

fn check_expr(
    expr:  &Expr,
    scope: &HashMap<String, BsType>,
    line:  usize,
) -> Result<BsType, BsError> {
    match expr {
        Expr::Lit(lit) => Ok(lit.ty()),

        Expr::Var(name) => scope.get(name).copied().ok_or_else(|| {
            BsError::at(line, format!(
                "'{}' is not one of this pipe's inputs — a bare expression may only use \
                 the names in its own '( ... )' list", name
            ))
        }),

        Expr::Unary(op, inner) => {
            let t = check_expr(inner, scope, line)?;
            match (op, t) {
                (UnOp::Neg, BsType::I64) => Ok(BsType::I64),
                (UnOp::Neg, BsType::F64) => Ok(BsType::F64),
                (UnOp::Neg, other) => Err(BsError::at(line, format!(
                    "unary '-' requires i64 or f64, found {}", other
                ))),
                (UnOp::Not, BsType::Bool) => Ok(BsType::Bool),
                (UnOp::Not, other) => Err(BsError::at(line, format!(
                    "unary '!' requires bool, found {}", other
                ))),
            }
        }

        Expr::Bin(op, lhs, rhs) => {
            let l = check_expr(lhs, scope, line)?;
            let r = check_expr(rhs, scope, line)?;
            check_binop(*op, l, r, line)
        }
    }
}

fn check_binop(op: BinOp, l: BsType, r: BsType, line: usize) -> Result<BsType, BsError> {
    use BinOp::*;
    match op {
        Add | Sub | Mul | Div => match (l, r) {
            (BsType::I64, BsType::I64) => Ok(BsType::I64),
            (BsType::F64, BsType::F64) => Ok(BsType::F64),
            _ => Err(BsError::at(line, format!(
                "arithmetic requires two i64 or two f64 operands, found {} and {}", l, r
            ))),
        },

        // `==` and `!=` are as type-strict as the ordering comparisons. A
        // cross-type equality can never be true, so accepting it silently
        // would only ever hide a mistake.
        Eq | Ne => {
            if l == r {
                Ok(BsType::Bool)
            } else {
                Err(BsError::at(line, format!(
                    "'==' and '!=' require operands of the same type, found {} and {}", l, r
                )))
            }
        }

        Lt | Gt | Le | Ge => match (l, r) {
            (BsType::I64, BsType::I64) | (BsType::F64, BsType::F64) => Ok(BsType::Bool),
            _ => Err(BsError::at(line, format!(
                "comparison requires two i64 or two f64 operands, found {} and {}", l, r
            ))),
        },

        And | Or => match (l, r) {
            (BsType::Bool, BsType::Bool) => Ok(BsType::Bool),
            _ => Err(BsError::at(line, format!(
                "'&&' and '||' require two bool operands, found {} and {}", l, r
            ))),
        },
    }
}
