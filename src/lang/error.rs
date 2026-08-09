use std::fmt;

#[derive(Debug, Clone)]
pub struct BsError {
    pub line:    Option<usize>,
    pub message: String,
}

impl BsError {
    pub fn new(message: impl Into<String>) -> Self {
        BsError { line: None, message: message.into() }
    }

    pub fn at(line: usize, message: impl Into<String>) -> Self {
        BsError { line: Some(line), message: message.into() }
    }
}

impl fmt::Display for BsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(l) => write!(f, "line {}: {}", l, self.message),
            None    => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for BsError {}
