//! Hand-written lexer.
//!
//! BullScript's grammar is small and extremely regular (a fixed sequence of
//! pipes), so a hand-rolled lexer/parser is simpler than pulling in a
//! parser-generator for it — pest stays in Bullang, where the grammar is
//! large enough to earn it.

use super::error::BsError;

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    LParen, RParen, LBrace, RBrace,
    Colon, DoubleColon, Comma, Semicolon, Arrow, Dot,

    Plus, Minus, Star, Slash,
    EqEq, NotEq, Lt, Gt, Le, Ge, AndAnd, OrOr, Bang,

    Ident(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),

    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok:  Tok,
    pub line: usize,
}

pub fn lex(src: &str) -> Result<Vec<Token>, BsError> {
    let mut out = Vec::new();
    let mut line = 1usize;
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0usize;

    macro_rules! push {
        ($t:expr) => { out.push(Token { tok: $t, line }) };
    }

    while i < chars.len() {
        let c = chars[i];

        match c {
            ' ' | '\t' | '\r' => { i += 1; }
            '\n' => { line += 1; i += 1; }

            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' { i += 1; }
            }

            '(' => { push!(Tok::LParen); i += 1; }
            ')' => { push!(Tok::RParen); i += 1; }
            '{' => { push!(Tok::LBrace); i += 1; }
            '}' => { push!(Tok::RBrace); i += 1; }
            ',' => { push!(Tok::Comma); i += 1; }
            ';' => { push!(Tok::Semicolon); i += 1; }

            // `.` only ever separates a data entry from its field. It is not
            // a general operator: BullScript has no structs and no methods.
            '.' => { push!(Tok::Dot); i += 1; }
            ':' if chars.get(i + 1) == Some(&':') => { push!(Tok::DoubleColon); i += 2; }
            ':' => { push!(Tok::Colon); i += 1; }

            '-' if chars.get(i + 1) == Some(&'>') => { push!(Tok::Arrow); i += 2; }
            '-' => { push!(Tok::Minus); i += 1; }
            '+' => { push!(Tok::Plus); i += 1; }
            '*' => { push!(Tok::Star); i += 1; }
            '/' => { push!(Tok::Slash); i += 1; }

            '=' if chars.get(i + 1) == Some(&'=') => { push!(Tok::EqEq); i += 2; }
            '=' => return Err(BsError::at(line, "unexpected '=' — did you mean '=='?")),
            '!' if chars.get(i + 1) == Some(&'=') => { push!(Tok::NotEq); i += 2; }
            '!' => { push!(Tok::Bang); i += 1; }
            '<' if chars.get(i + 1) == Some(&'=') => { push!(Tok::Le); i += 2; }
            '<' => { push!(Tok::Lt); i += 1; }
            '>' if chars.get(i + 1) == Some(&'=') => { push!(Tok::Ge); i += 2; }
            '>' => { push!(Tok::Gt); i += 1; }
            '&' if chars.get(i + 1) == Some(&'&') => { push!(Tok::AndAnd); i += 2; }
            '&' => return Err(BsError::at(line, "unexpected '&' — BullScript has no bitwise operators; did you mean '&&'?")),
            '|' if chars.get(i + 1) == Some(&'|') => { push!(Tok::OrOr); i += 2; }
            '|' => return Err(BsError::at(line, "unexpected '|' — BullScript has no bitwise operators; did you mean '||'?")),

            '"' => {
                // A string literal may not span lines: an unterminated quote
                // would otherwise swallow the rest of the file and report the
                // failure far from the actual mistake. `\n` embeds a newline.
                let open_line = line;
                let mut s = String::new();
                i += 1;
                loop {
                    match chars.get(i) {
                        None => return Err(BsError::at(
                            open_line, "unterminated string literal (opened here, never closed)"
                        )),
                        Some('\n') => return Err(BsError::at(
                            open_line,
                            "unterminated string literal (opened here) — a string cannot span \
                             lines; use '\\n' to embed a newline"
                        )),
                        Some('"') => { i += 1; break; }
                        Some('\\') => {
                            i += 1;
                            match chars.get(i) {
                                Some('n')  => s.push('\n'),
                                Some('t')  => s.push('\t'),
                                Some('r')  => s.push('\r'),
                                Some('0')  => s.push('\0'),
                                Some('"')  => s.push('"'),
                                Some('\\') => s.push('\\'),
                                Some(other) => return Err(BsError::at(
                                    line, format!(
                                        "unknown escape sequence '\\{}' — valid escapes are \
                                         \\n \\t \\r \\0 \\\" \\\\", other
                                    )
                                )),
                                None => return Err(BsError::at(
                                    open_line, "unterminated string literal (opened here, never closed)"
                                )),
                            }
                            i += 1;
                        }
                        Some(c) => { s.push(*c); i += 1; }
                    }
                }
                push!(Tok::Str(s));
            }

            c if c.is_ascii_digit() => {
                let start = i;
                let mut is_float = false;
                while i < chars.len() && chars[i].is_ascii_digit() { i += 1; }
                if chars.get(i) == Some(&'.')
                    && chars.get(i + 1).map_or(false, |d| d.is_ascii_digit())
                {
                    is_float = true;
                    i += 1;
                    while i < chars.len() && chars[i].is_ascii_digit() { i += 1; }
                }
                let text: String = chars[start..i].iter().collect();
                if is_float {
                    let f: f64 = text.parse()
                        .map_err(|_| BsError::at(line, format!("invalid float literal '{}'", text)))?;
                    push!(Tok::Float(f));
                } else {
                    let n: i64 = text.parse()
                        .map_err(|_| BsError::at(line, format!(
                            "integer literal '{}' does not fit in an i64", text
                        )))?;
                    push!(Tok::Int(n));
                }
            }

            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                // `true` / `false` are reserved: they lex as bool literals, not
                // identifiers, so they can never be used as a binding name.
                match text.as_str() {
                    "true"  => push!(Tok::Bool(true)),
                    "false" => push!(Tok::Bool(false)),
                    _       => push!(Tok::Ident(text)),
                }
            }

            other => return Err(BsError::at(line, format!("unexpected character '{}'", other))),
        }
    }

    out.push(Token { tok: Tok::Eof, line });
    Ok(out)
}
