//! Recursive-descent parser.
//!
//! Grammar (informal):
//!
//!   program     := pipe*
//!   pipe        := "(" input_list? ")" ":" pipe_val "->" binding ";"
//!   input_list  := input ("," input)*
//!   input       := (literal | ident) ":" type
//!   pipe_val    := call | expr
//!   call        := ("builtin" | "bag") "::" ident
//!   binding     := "{" "}" | "{" ident ":" type "}"
//!
//!   expr        := logic_or
//!   logic_or    := logic_and ("||" logic_and)*
//!   logic_and   := equality ("&&" equality)*
//!   equality    := comparison (("=="|"!=") comparison)*
//!   comparison  := term (("<"|">"|"<="|">=") term)*
//!   term        := factor (("+"|"-") factor)*
//!   factor      := unary (("*"|"/") unary)*
//!   unary       := ("-"|"!")? primary
//!   primary     := int | float | bool | string | ident | "(" expr ")"
//!
//! Standard precedence is used (arithmetic binds tighter than comparison,
//! which binds tighter than logical) rather than mirroring Bullang's own
//! flat left-to-right chaining — this is simpler and less surprising, and
//! is the one deliberate deviation worth flagging.

use super::ast::*;
use super::error::BsError;
use super::lexer::{Tok, Token};
use super::types::BsType;

pub fn parse(tokens: &[Token]) -> Result<Program, BsError> {
    let mut p = Parser { toks: tokens, pos: 0 };
    let mut pipes = Vec::new();
    while !p.check(&Tok::Eof) {
        pipes.push(p.parse_pipe()?);
    }
    Ok(pipes)
}

struct Parser<'a> {
    toks: &'a [Token],
    pos:  usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> &Tok { &self.toks[self.pos].tok }
    fn line(&self) -> usize { self.toks[self.pos].line }

    fn check(&self, t: &Tok) -> bool { self.peek() == t }

    fn advance(&mut self) -> Tok {
        let t = self.toks[self.pos].tok.clone();
        if self.pos < self.toks.len() - 1 { self.pos += 1; }
        t
    }

    fn expect(&mut self, t: &Tok, what: &str) -> Result<(), BsError> {
        if self.check(t) {
            self.advance();
            Ok(())
        } else {
            Err(BsError::at(self.line(), format!("expected {}, found {}", what, describe(self.peek()))))
        }
    }

    fn expect_ident(&mut self, what: &str) -> Result<String, BsError> {
        match self.peek().clone() {
            Tok::Ident(s) => { self.advance(); Ok(s) }
            other => Err(BsError::at(self.line(), format!("expected {}, found {}", what, describe(&other)))),
        }
    }

    fn expect_type(&mut self) -> Result<BsType, BsError> {
        let line = self.line();
        let name = self.expect_ident("a type")?;
        BsType::parse(&name).ok_or_else(|| {
            BsError::at(line, format!(
                "unknown type '{}' — BullScript only has i64, f64, bool, String", name
            ))
        })
    }

    // ── pipe ─────────────────────────────────────────────────────────────

    fn parse_pipe(&mut self) -> Result<Pipe, BsError> {
        let line = self.line();
        self.expect(&Tok::LParen, "'('")?;
        let inputs = self.parse_input_list()?;
        self.expect(&Tok::RParen, "')'")?;
        self.expect(&Tok::Colon, "':'")?;
        let val = self.parse_pipe_val()?;
        self.expect(&Tok::Arrow, "'->'")?;
        let binding = self.parse_binding()?;
        self.expect(&Tok::Semicolon, "';'")?;
        Ok(Pipe { inputs, val, binding, line })
    }

    fn parse_input_list(&mut self) -> Result<Vec<TypedInput>, BsError> {
        let mut inputs = Vec::new();
        if self.check(&Tok::RParen) {
            return Ok(inputs);
        }
        loop {
            inputs.push(self.parse_input()?);
            if self.check(&Tok::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        Ok(inputs)
    }

    fn parse_input(&mut self) -> Result<TypedInput, BsError> {
        let line = self.line();
        let expr = match self.peek().clone() {
            Tok::Int(n)      => { self.advance(); InputExpr::Lit(Literal::I64(n)) }
            Tok::Float(x)    => { self.advance(); InputExpr::Lit(Literal::F64(x)) }
            Tok::Bool(b)     => { self.advance(); InputExpr::Lit(Literal::Bool(b)) }
            Tok::Str(s)      => { self.advance(); InputExpr::Lit(Literal::Str(s)) }
            // `data::entry.field` — a value read out of a stored document.
            Tok::Ident(ref n) if n == "data"
                && self.toks.get(self.pos + 1).map(|t| &t.tok) == Some(&Tok::DoubleColon) =>
            {
                InputExpr::Data(self.parse_data_ref()?)
            }
            Tok::Ident(name) => { self.advance(); InputExpr::Var(name) }
            other => return Err(BsError::at(line, format!(
                "expected a literal, a variable name or a data field in the input list, found {}", describe(&other)
            ))),
        };
        self.expect(&Tok::Colon, "':' (every input needs an explicit type)")?;
        let ty = self.expect_type()?;
        Ok(TypedInput { expr, ty, line })
    }

    fn parse_pipe_val(&mut self) -> Result<PipeVal, BsError> {
        // A call is `builtin::name` or `bag::name` — recognisable by an
        // ident immediately followed by '::'.
        if let Tok::Ident(name) = self.peek().clone() {
            if (name == "builtin" || name == "bag")
                && self.toks.get(self.pos + 1).map(|t| &t.tok) == Some(&Tok::DoubleColon)
            {
                self.advance(); // namespace
                self.advance(); // '::'
                let entry = self.expect_ident("a callee name")?;
                let callee = if name == "builtin" {
                    Callee::Builtin(entry)
                } else {
                    Callee::Bag(entry)
                };
                return Ok(PipeVal::Call(callee));
            }
        }
        Ok(PipeVal::Expr(self.parse_expr()?))
    }

    fn parse_binding(&mut self) -> Result<Binding, BsError> {
        self.expect(&Tok::LBrace, "'{'")?;
        if self.check(&Tok::RBrace) {
            self.advance();
            return Ok(Binding::Discard);
        }
        // `-> {data::entry.field: T}` writes into a document instead of
        // declaring a new binding.
        if matches!(self.peek(), Tok::Ident(n) if n == "data")
            && self.toks.get(self.pos + 1).map(|t| &t.tok) == Some(&Tok::DoubleColon)
        {
            let target = self.parse_data_ref()?;
            self.expect(&Tok::Colon, "':' (every binding needs an explicit type)")?;
            let ty = self.expect_type()?;
            self.expect(&Tok::RBrace, "'}'")?;
            return Ok(Binding::Data { target, ty });
        }

        let name = self.expect_ident("a binding name")?;
        self.expect(&Tok::Colon, "':' (every binding needs an explicit type)")?;
        let ty = self.expect_type()?;
        self.expect(&Tok::RBrace, "'}'")?;
        Ok(Binding::Bound { name, ty })
    }

    /// `data::entry.field` or `data::entry.field.subfield`.
    ///
    /// At least one field is required: a bare `data::prompt` is the whole
    /// document, which is an object and not one of BullScript's four types.
    fn parse_data_ref(&mut self) -> Result<DataRef, BsError> {
        let line = self.line();
        self.advance(); // 'data'
        self.advance(); // '::'
        let entry = self.expect_ident("a data entry name")?;

        let mut path = Vec::new();
        loop {
            if self.check(&Tok::Dot) {
                self.advance();
                path.push(PathSeg::Field(self.expect_ident("a field name after '.'")?));
            } else if self.check(&Tok::LBracket) {
                // `[key]` — the field name comes from a variable.
                self.advance();
                let key = self.expect_ident("a variable name inside '[ ]'")?;
                self.expect(&Tok::RBracket, "']'")?;
                path.push(PathSeg::Key(key));
            } else {
                break;
            }
        }

        if path.is_empty() {
            return Err(BsError::at(line, format!(
                "'data::{}' is a whole document, not a value. Name a field of it, \
                 like 'data::{}.audit', or select one with 'data::{}[key]'.",
                entry, entry, entry
            )));
        }
        Ok(DataRef { entry, path })
    }

    // ── expressions (precedence climbing) ───────────────────────────────

    fn parse_expr(&mut self) -> Result<Expr, BsError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, BsError> {
        let mut lhs = self.parse_and()?;
        while self.check(&Tok::OrOr) {
            self.advance();
            let rhs = self.parse_and()?;
            lhs = Expr::Bin(BinOp::Or, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<Expr, BsError> {
        let mut lhs = self.parse_equality()?;
        while self.check(&Tok::AndAnd) {
            self.advance();
            let rhs = self.parse_equality()?;
            lhs = Expr::Bin(BinOp::And, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_equality(&mut self) -> Result<Expr, BsError> {
        let mut lhs = self.parse_comparison()?;
        loop {
            let op = match self.peek() {
                Tok::EqEq  => BinOp::Eq,
                Tok::NotEq => BinOp::Ne,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_comparison()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_comparison(&mut self) -> Result<Expr, BsError> {
        let mut lhs = self.parse_term()?;
        loop {
            let op = match self.peek() {
                Tok::Lt => BinOp::Lt,
                Tok::Gt => BinOp::Gt,
                Tok::Le => BinOp::Le,
                Tok::Ge => BinOp::Ge,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_term()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_term(&mut self) -> Result<Expr, BsError> {
        let mut lhs = self.parse_factor()?;
        loop {
            let op = match self.peek() {
                Tok::Plus  => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_factor()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_factor(&mut self) -> Result<Expr, BsError> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Tok::Star  => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_unary()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, BsError> {
        match self.peek() {
            Tok::Minus => { self.advance(); Ok(Expr::Unary(UnOp::Neg, Box::new(self.parse_unary()?))) }
            Tok::Bang  => { self.advance(); Ok(Expr::Unary(UnOp::Not, Box::new(self.parse_unary()?))) }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, BsError> {
        let line = self.line();
        match self.peek().clone() {
            Tok::Int(n)   => { self.advance(); Ok(Expr::Lit(Literal::I64(n))) }
            Tok::Float(x) => { self.advance(); Ok(Expr::Lit(Literal::F64(x))) }
            Tok::Bool(b)  => { self.advance(); Ok(Expr::Lit(Literal::Bool(b))) }
            Tok::Str(s)   => { self.advance(); Ok(Expr::Lit(Literal::Str(s))) }
            Tok::Ident(name) => { self.advance(); Ok(Expr::Var(name)) }
            Tok::LParen => {
                self.advance();
                let e = self.parse_expr()?;
                self.expect(&Tok::RParen, "')'")?;
                Ok(e)
            }
            other => Err(BsError::at(line, format!("expected an expression, found {}", describe(&other)))),
        }
    }
}

/// Human-readable token description for error messages — `{:?}` on the raw
/// token leaks Rust enum spelling at the user.
fn describe(t: &Tok) -> String {
    match t {
        Tok::LParen => "'('".into(),
        Tok::RParen => "')'".into(),
        Tok::LBrace => "'{'".into(),
        Tok::RBrace => "'}'".into(),
        Tok::Colon => "':'".into(),
        Tok::DoubleColon => "'::'".into(),
        Tok::Dot => "'.'".into(),
        Tok::LBracket => "'['".into(),
        Tok::RBracket => "']'".into(),
        Tok::Comma => "','".into(),
        Tok::Semicolon => "';'".into(),
        Tok::Arrow => "'->'".into(),
        Tok::Plus => "'+'".into(),
        Tok::Minus => "'-'".into(),
        Tok::Star => "'*'".into(),
        Tok::Slash => "'/'".into(),
        Tok::EqEq => "'=='".into(),
        Tok::NotEq => "'!='".into(),
        Tok::Lt => "'<'".into(),
        Tok::Gt => "'>'".into(),
        Tok::Le => "'<='".into(),
        Tok::Ge => "'>='".into(),
        Tok::AndAnd => "'&&'".into(),
        Tok::OrOr => "'||'".into(),
        Tok::Bang => "'!'".into(),
        Tok::Ident(s) => format!("'{}'", s),
        Tok::Int(n) => format!("the number {}", n),
        Tok::Float(x) => format!("the number {}", x),
        Tok::Bool(b) => format!("'{}'", b),
        Tok::Str(_) => "a string literal".into(),
        Tok::Eof => "end of input".into(),
    }
}
