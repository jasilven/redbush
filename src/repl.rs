use std::collections::HashMap;
use std::fmt::{self, Display};

pub type Result<T> = std::result::Result<T, ReplError>;

#[derive(Debug)]
pub enum ReplError {
    Error(String),
    Io(std::io::Error),
}

impl From<&str> for ReplError {
    fn from(s: &str) -> ReplError {
        ReplError::Error(s.to_string())
    }
}

impl From<std::io::Error> for ReplError {
    fn from(err: std::io::Error) -> ReplError {
        ReplError::Io(err)
    }
}

impl Display for ReplError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReplError::Error(s) => write!(f, "Repl Error: {} ", s),
            ReplError::Io(e) => write!(f, "Repl Io: {}", e),
        }
    }
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub enum Param {
    Str(String),
    Int(i32),
}

impl From<&str> for Param {
    fn from(s: &str) -> Param {
        Param::Str(s.to_string())
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum Request {
    Eval(HashMap<Param, Param>),
    Interrupt(HashMap<Param, Param>),
    NewSession(),
    Exit(),
}

#[derive(Debug, Eq, PartialEq)]
pub enum Response {
    //    value   ns      ms     form
    Value(String, String, usize, String),
    Err(String),
    Out(String),
    Exception(String),
    Status(Vec<String>),
    NewSession(String),
    Eof(),
    Other(String),
}

pub trait ReplSender {
    fn session_id(&self) -> String;
    fn send(&mut self, req: Request) -> Result<()>;
}

pub trait ReplReceiver: Send + Sync + 'static {
    fn receive(&mut self) -> Result<Response>;
}
