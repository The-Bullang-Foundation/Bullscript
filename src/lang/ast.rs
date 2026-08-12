//! AST for BullScript's pipe-only grammar.
//!
//! A program (a `.busc` file, or a run of lines typed at the BullScript
//! prompt) is just a sequence of pipes — there is no `let`, no functions,
//! no blocks. Every pipe has the shape:
//!
//!     ( <typed input>, ... ) : <callee-or-expr> -> <typed binding> ;
//!
//! Every input and every created binding always carries an explicit type.

use super::types::BsType;

#[derive(Debug, Clone)]
pub enum Literal {
    I64(i64),
    F64(f64),
    Bool(bool),
    Str(String),
}

impl Literal {
    pub fn ty(&self) -> BsType {
        match self {
            Literal::I64(_)  => BsType::I64,
            Literal::F64(_)  => BsType::F64,
            Literal::Bool(_) => BsType::Bool,
            Literal::Str(_)  => BsType::String,
        }
    }
}

/// One item in a pipe's input list: either a literal or a reference to an
/// already-bound variable. Always carries its own explicit type annotation.
///
/// In the *first* pipe of a script this list doubles as the parameter list.
/// Every slot is a parameter, literal or not: the caller must supply a value
/// for each one. A variable slot also binds its value into the environment
/// under that name; a literal slot has no name, so its value is used by the
/// first pipe and not bound.
#[derive(Debug, Clone)]
pub struct TypedInput {
    pub expr: InputExpr,
    pub ty:   BsType,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub enum InputExpr {
    Lit(Literal),
    Var(String),
    /// `data::prompt.audit` — a field of a stored JSON document.
    ///
    /// A value, not a call, which is why it lives here beside literals and
    /// variables rather than in `PipeVal` alongside `bag::` and `builtin::`.
    Data(DataRef),
}

/// A path into a stored JSON document: the entry name, then one or more
/// field names. `data::prompt.audit.system` is `{ entry: "prompt",
/// path: ["audit", "system"] }`.
#[derive(Debug, Clone, PartialEq)]
pub struct DataRef {
    pub entry: String,
    pub path:  Vec<PathSeg>,
}

/// One step of a data path.
///
/// `data::norm.c` is one `Field`; `data::norm[lang]` is one `Key`, where the
/// field name is whatever the variable `lang` holds when the pipe runs.
#[derive(Debug, Clone, PartialEq)]
pub enum PathSeg {
    /// A field name written in the source.
    Field(String),
    /// A variable holding the field name.
    Key(String),
}

impl std::fmt::Display for PathSeg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathSeg::Field(n) => write!(f, ".{}", n),
            PathSeg::Key(v)   => write!(f, "[{}]", v),
        }
    }
}

impl std::fmt::Display for DataRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "data::{}", self.entry)?;
        for part in &self.path {
            write!(f, "{}", part)?;
        }
        Ok(())
    }
}

/// What a callable is namespaced under. `builtin::` for the fixed, hardcoded
/// builtin table; `bag::` for a user script stored in the bag.
#[derive(Debug, Clone)]
pub enum Callee {
    Builtin(String),
    Bag(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add, Sub, Mul, Div,
    Eq, Ne, Lt, Gt, Le, Ge,
    And, Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

/// A freestanding arithmetic/comparison/logical expression, used when a
/// pipe's middle section is not a `builtin::`/`bag::` call — e.g. `a + b`.
/// Identifiers here must already be declared (typed) in the same pipe's
/// input list.
#[derive(Debug, Clone)]
pub enum Expr {
    Lit(Literal),
    Var(String),
    Unary(UnOp, Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
}

/// The middle section of a pipe, between `:` and `->`.
#[derive(Debug, Clone)]
pub enum PipeVal {
    Call(Callee),
    Expr(Expr),
}

/// The output binding of a pipe: `{}` discards the result, `{name: ty}`
/// creates or overwrites `name` with the computed value.
#[derive(Debug, Clone)]
pub enum Binding {
    Discard,
    Bound { name: String, ty: BsType },
    /// `-> {data::prompt.audit: String}` — write the pipe's value into a field
    /// of a stored document, rather than into a new variable.
    ///
    /// The write happens when the pipe runs, like `builtin::out`, so a program
    /// that fails halfway leaves the writes that already happened in place.
    Data { target: DataRef, ty: BsType },
}

#[derive(Debug, Clone)]
pub struct Pipe {
    pub inputs:  Vec<TypedInput>,
    pub val:     PipeVal,
    pub binding: Binding,
    pub line:    usize,
}

pub type Program = Vec<Pipe>;
