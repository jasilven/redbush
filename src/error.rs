use crate::repl;
use std::fmt;

#[derive(Debug)]
pub enum MyError {
    Error(String),
    Log(log::SetLoggerError),
    Env(std::env::VarError),
    Utf8(std::string::FromUtf8Error),
    Io(std::io::Error),
    ParseInt(std::num::ParseIntError),
    ParseFloat(std::num::ParseFloatError),
    Nvim(neovim_lib::CallError),
    Repl(repl::ReplError),
}

impl fmt::Display for MyError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            MyError::Env(ref err) => err.fmt(f),
            MyError::Log(ref err) => err.fmt(f),
            MyError::Utf8(ref err) => err.fmt(f),
            MyError::Io(ref err) => err.fmt(f),
            MyError::ParseInt(ref err) => err.fmt(f),
            MyError::ParseFloat(ref err) => err.fmt(f),
            MyError::Nvim(ref err) => err.fmt(f),
            MyError::Repl(ref s) => write!(f, "{}", s),
            MyError::Error(ref s) => write!(f, "{}", s),
        }
    }
}

impl From<&str> for MyError {
    fn from(s: &str) -> MyError {
        MyError::Error(String::from(s))
    }
}

impl From<String> for MyError {
    fn from(s: String) -> MyError {
        MyError::Error(s)
    }
}

impl From<log::SetLoggerError> for MyError {
    fn from(err: log::SetLoggerError) -> MyError {
        MyError::Log(err)
    }
}

impl From<repl::ReplError> for MyError {
    fn from(err: repl::ReplError) -> MyError {
        MyError::Repl(err)
    }
}

impl From<std::io::Error> for MyError {
    fn from(err: std::io::Error) -> MyError {
        MyError::Io(err)
    }
}

impl From<std::env::VarError> for MyError {
    fn from(err: std::env::VarError) -> MyError {
        MyError::Env(err)
    }
}

impl From<std::string::FromUtf8Error> for MyError {
    fn from(err: std::string::FromUtf8Error) -> MyError {
        MyError::Utf8(err)
    }
}

impl From<std::num::ParseIntError> for MyError {
    fn from(err: std::num::ParseIntError) -> MyError {
        MyError::ParseInt(err)
    }
}

impl From<std::num::ParseFloatError> for MyError {
    fn from(err: std::num::ParseFloatError) -> MyError {
        MyError::ParseFloat(err)
    }
}

impl From<neovim_lib::CallError> for MyError {
    fn from(err: neovim_lib::CallError) -> MyError {
        MyError::Nvim(err)
    }
}
