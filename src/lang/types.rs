//! The BullScript type pool: exactly four types, flat, no variants.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BsType {
    I64,
    F64,
    Bool,
    String,
}

impl BsType {
    /// Parse a type name as it appears in source (`i64`, `f64`, `bool`, `String`).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "i64"    => Some(BsType::I64),
            "f64"    => Some(BsType::F64),
            "bool"   => Some(BsType::Bool),
            "String" => Some(BsType::String),
            _        => None,
        }
    }
}

impl fmt::Display for BsType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            BsType::I64    => "i64",
            BsType::F64    => "f64",
            BsType::Bool   => "bool",
            BsType::String => "String",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    I64(i64),
    F64(f64),
    Bool(bool),
    Str(String),
}

impl Value {
    pub fn ty(&self) -> BsType {
        match self {
            Value::I64(_)  => BsType::I64,
            Value::F64(_)  => BsType::F64,
            Value::Bool(_) => BsType::Bool,
            Value::Str(_)  => BsType::String,
        }
    }

    /// Parse a command-line argument string into `ty`.
    ///
    /// Used when seeding a script's parameters from `bullscript file.busc a b c`:
    /// arguments arrive as text and must be turned into typed values before the
    /// first pipe runs.
    pub fn parse_as(text: &str, ty: BsType) -> Result<Value, String> {
        match ty {
            BsType::String => Ok(Value::Str(text.to_string())),
            BsType::I64 => text.trim().parse::<i64>()
                .map(Value::I64)
                .map_err(|_| format!("'{}' is not a valid i64", text)),
            BsType::F64 => text.trim().parse::<f64>()
                .map(Value::F64)
                .map_err(|_| format!("'{}' is not a valid f64", text)),
            BsType::Bool => match text.trim() {
                "true"  => Ok(Value::Bool(true)),
                "false" => Ok(Value::Bool(false)),
                _ => Err(format!("'{}' is not a valid bool (use 'true' or 'false')", text)),
            },
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::I64(n)  => write!(f, "{}", n),
            Value::F64(x)  => write!(f, "{}", x),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Str(s)  => write!(f, "{}", s),
        }
    }
}
