use edn::parser::Parser;
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
    DisableNsMaps(),
    Exit(),
}

#[derive(Debug, Eq, PartialEq)]
pub enum Response {
    //    value   ns      ms     form
    Value(String, String, usize, String),
    Err(String),
    Out(String),
    Exception(String, String),
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

pub fn parse_exception(edn_s: &str) -> (String, String) {
    log::debug!("Parsing exception: {}", &edn_s);

    let mut errmsg = "".to_string();
    let mut trace = "".to_string();
    let mut parser = Parser::new(&edn_s);
    let edn_val = parser.read().unwrap();

    if let Ok(edn::Value::Map(map)) = edn_val {
        if let edn::Value::Vector(vec) = map.get(&edn::Value::Keyword("via".to_string())).unwrap() {
            for val in vec.iter() {
                if let edn::Value::Map(m) = val {
                    if let Some(edn::Value::String(s)) =
                        m.get(&edn::Value::Keyword("message".to_string()))
                    {
                        errmsg.push_str(s);
                        errmsg.push('\n');
                    }
                }
            }
        }
        if let edn::Value::Vector(vec) = map.get(&edn::Value::Keyword("trace".to_string())).unwrap()
        {
            for val in vec.iter() {
                if let edn::Value::Vector(v) = val {
                    for val in v.iter() {
                        if let edn::Value::Symbol(sym) = val {
                            trace.push_str(sym);
                            trace.push(' ');
                        } else if let edn::Value::String(s) = val {
                            trace.push_str(s);
                            trace.push(' ');
                        } else if let edn::Value::Integer(i) = val {
                            trace.push_str(&i.to_string());
                            trace.push(' ');
                        }
                    }

                    trace = trace.trim().to_string();
                    trace.push('\n');
                }
            }
        }
    }

    (trace, errmsg)
}
