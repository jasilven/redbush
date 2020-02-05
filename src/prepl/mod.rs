use crate::repl;
use crate::repl::Result;
use crate::repl::*;
use edn::parser::Parser;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::net::TcpStream;

pub struct PreplSender {
    #[allow(dead_code)]
    host: String,
    #[allow(dead_code)]
    port: String,
    pub session_id: String,
    request_cnt: usize,
    writer: BufWriter<TcpStream>,
}

impl PreplSender {
    fn write_and_flush(&mut self, buf: &[u8]) -> Result<()> {
        self.writer.write_all(buf)?;
        self.writer.flush()?;
        Ok(())
    }
}

impl PreplReceiver {
    fn parse_exception(&self, edn_s: &str) -> String {
        log::debug!("Parsing exception from pREPL response");

        let mut result = "".to_string();
        let mut parser = Parser::new(&edn_s);
        let edn_val = parser.read().unwrap();

        if let Ok(edn::Value::Map(map)) = edn_val {
            if let edn::Value::Vector(vec) =
                map.get(&edn::Value::Keyword("via".to_string())).unwrap()
            {
                for val in vec.iter() {
                    if let edn::Value::Map(m) = val {
                        if let Some(edn::Value::String(s)) =
                            m.get(&edn::Value::Keyword("message".to_string()))
                        {
                            result.push_str(s);
                            result.push('\n');
                        }
                    }
                }
            }
            result.push_str("\n-- Trace --\n");
            if let edn::Value::Vector(vec) =
                map.get(&edn::Value::Keyword("trace".to_string())).unwrap()
            {
                for val in vec.iter() {
                    if let edn::Value::Vector(v) = val {
                        for val in v.iter() {
                            if let edn::Value::Symbol(sym) = val {
                                result.push_str(sym);
                                result.push(' ');
                            } else if let edn::Value::String(s) = val {
                                result.push_str(s);
                                result.push(' ');
                            } else if let edn::Value::Integer(i) = val {
                                result.push_str(&i.to_string());
                                result.push(' ');
                            }
                        }

                        result = result.trim().to_string();
                        result.push('\n');
                    }
                }
            }
        }

        result
    }
}

pub struct PreplReceiver {
    #[allow(dead_code)]
    host: String,
    #[allow(dead_code)]
    port: String,
    pub session_id: String,
    #[allow(dead_code)]
    request_cnt: usize,
    reader: BufReader<TcpStream>,
}

pub fn new_sender_receiver(host: &str, port: &str) -> Result<(impl ReplSender, impl ReplReceiver)> {
    log::debug!("Connecting pREPL {}:{}", host, port);

    let stream = TcpStream::connect(format!("{}:{}", host, port))?;
    let stream2 = stream.try_clone()?;

    let mut sender = PreplSender {
        session_id: "prepl_default_session".to_string(),
        host: host.to_string(),
        port: port.to_string(),
        request_cnt: 0,
        writer: BufWriter::new(stream),
    };

    let mut receiver = PreplReceiver {
        session_id: "".to_string(),
        host: host.to_string(),
        port: port.to_string(),
        request_cnt: 0,
        reader: BufReader::new(stream2),
    };

    // Disable keyword namespaces (for edn parser)
    sender.write_and_flush(b"(set! *print-namespace-maps* false)\n")?;
    let mut _buf = "".to_string();
    receiver.reader.read_line(&mut _buf)?;

    Ok((sender, receiver))
}

impl ReplSender for PreplSender {
    fn send(&mut self, req: Request) -> Result<()> {
        log::debug!("Sending request to PREPL: {:?}", &req);

        match req {
            Request::Eval(params) => {
                let code = match params.get(&repl::Param::Str("code".into())) {
                    Some(repl::Param::Str(s)) => format!("{}\n", s),
                    _ => "".to_string(),
                };

                if !code.is_empty() {
                    log::debug!("Sending code to PREPL: {}", &code);

                    self.write_and_flush(code.as_bytes())?;
                    self.request_cnt += 1;
                }
            }
            Request::Exit() => {
                log::debug!("Sending exit to PREPL");
                self.write_and_flush(b":repl/quit\n")?;
            }
            _ => (),
        };

        Ok(())
    }

    fn session_id(&self) -> String {
        self.session_id.to_string()
    }
}

impl ReplReceiver for PreplReceiver {
    fn receive(&mut self) -> Result<Response> {
        log::debug!("Reading pREPL response");

        let mut resp = "".to_string();
        self.reader.read_line(&mut resp)?;

        let mut parser = Parser::new(&resp);
        let edn_val = parser.read();

        log::debug!("pREPL edn: {:?}", &edn_val);
        match edn_val {
            Some(Ok(edn::Value::Map(map))) => {
                let tag = map.get(&edn::Value::Keyword("tag".into())).unwrap();
                match tag {
                    edn::Value::Keyword(key) => match key.to_string().as_str() {
                        "ret" => {
                            let val = match map.get(&edn::Value::Keyword("val".into())) {
                                Some(edn::Value::String(s)) => s.to_owned(),
                                _ => "".to_string(),
                            };
                            let ns = match map.get(&edn::Value::Keyword("ns".into())) {
                                Some(edn::Value::String(s)) => s.to_owned(),
                                _ => "".to_string(),
                            };
                            let ms = match map.get(&edn::Value::Keyword("ms".into())) {
                                Some(edn::Value::Integer(i)) => *i,
                                _ => 0i64,
                            };
                            let form = match map.get(&edn::Value::Keyword("form".into())) {
                                Some(edn::Value::String(s)) => s.to_owned(),
                                _ => "".to_string(),
                            };
                            let exception = match map.get(&edn::Value::Keyword("exception".into()))
                            {
                                Some(edn::Value::Boolean(b)) => *b,
                                _ => false,
                            };
                            if exception {
                                log::debug!("EXCEPTION: {}", &val);
                                Ok(Response::Exception(self.parse_exception(&val)))
                            } else {
                                Ok(Response::Value(val, ns, ms as usize, form))
                            }
                        }
                        "out" => {
                            let out = match map.get(&edn::Value::Keyword("val".into())) {
                                Some(edn::Value::String(s)) => s.to_owned(),
                                _ => "".to_string(),
                            };

                            Ok(Response::Out(out))
                        }
                        "err" => {
                            let out = match map.get(&edn::Value::Keyword("val".into())) {
                                Some(edn::Value::String(s)) => s.to_owned(),
                                _ => "".to_string(),
                            };

                            Ok(Response::Out(out))
                        }
                        _ => Ok(Response::Other(key.to_string())),
                    },
                    _ => Ok(Response::Other("".to_string())),
                }
            }
            Some(Err(e)) => Err(ReplError::Error(format!("EDN parser Error: {:?}", e))),
            Some(x) => Err(ReplError::Error(format!(
                "EDN parser Error: unexpected response from pREPL: {:?}",
                x
            ))),
            None => Err(ReplError::Error(
                "EDN parser Error: trying to parse empty string".into(),
            )),
        }
    }
}
