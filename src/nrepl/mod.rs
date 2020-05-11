use crate::repl::{
    parse_exception, Param, ReplError, ReplReceiver, ReplSender, Request, Response, Result,
};
use bencode_rs as bc;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::io::{BufReader, BufWriter, Write};
use std::net::TcpStream;

pub struct NreplSender {
    #[allow(dead_code)]
    host: String,
    #[allow(dead_code)]
    port: String,
    pub session_id: String,
    request_cnt: usize,
    writer: BufWriter<TcpStream>,
}

impl NreplSender {
    fn write_and_flush(&mut self, buf: &[u8]) -> Result<()> {
        self.writer.write_all(buf)?;
        self.writer.flush()?;
        Ok(())
    }
}

pub struct NreplReceiver {
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
    log::debug!("Connecting nREPL {}:{}", host, port);

    let stream = TcpStream::connect(format!("{}:{}", host, port))?;
    let stream2 = stream.try_clone()?;

    let mut sender = NreplSender {
        session_id: "".to_string(),
        host: host.to_string(),
        port: port.to_string(),
        request_cnt: 0,
        writer: BufWriter::new(stream),
    };

    let mut receiver = NreplReceiver {
        session_id: "".to_string(),
        host: host.to_string(),
        port: port.to_string(),
        request_cnt: 0,
        reader: BufReader::new(stream2),
    };

    sender.send(Request::NewSession())?;

    match receiver.receive() {
        Ok(Response::NewSession(session_id)) => {
            sender.session_id = session_id.to_string();
            receiver.session_id = session_id;

            sender.send(Request::DisableNsMaps())?;
            match receiver.receive() {
                Ok(Response::Value(_, _, _, _)) => match receiver.receive() {
                    Ok(Response::Status(vec)) => {
                        if vec.contains(&"done".to_string()) {
                            Ok((sender, receiver))
                        } else {
                            log::debug!("Unable to disable ns-maps");
                            Err(ReplError::from("Unable to disable ns-maps"))
                        }
                    }
                    Ok(_) => {
                        log::debug!("Unexpected response when trying to disable ns-maps");
                        Err(ReplError::Error(
                            "Unexpected response when trying to disable ns-maps".to_string(),
                        ))
                    }
                    Err(e) => {
                        log::debug!("Failed to disable ns-maps");
                        Err(ReplError::Error(e.to_string()))
                    }
                },

                Err(_) | Ok(_) => {
                    log::debug!("Failed to disable ns-maps");
                    Err(ReplError::Error("Failed to disable ns-maps".to_string()))
                }
            }
        }
        Ok(x) => {
            log::debug!(
                "Got unexpected nREPL response for 'new session' request: {:?}",
                x
            );
            Err(ReplError::from(
                "Unexpected nREPL response for 'new session'",
            ))
        }
        Err(e) => {
            log::debug!(
                "Failed to get nREPL response for 'new session' request: {}",
                e
            );
            Err(e)
        }
    }
}

fn build_bc_value(hm: HashMap<Param, Param>) -> bc::Value {
    let mut bcmap = HashMap::<bc::Value, bc::Value>::new();
    for (k, v) in hm.iter() {
        let key = match k {
            Param::Str(s) => bc::Value::Str(s.to_string()),
            Param::Int(i) => bc::Value::Int(*i),
        };
        let val = match v {
            Param::Str(s) => bc::Value::Str(s.to_string()),
            Param::Int(i) => bc::Value::Int(*i),
        };
        bcmap.insert(key, val);
    }
    bc::Value::from(bcmap)
}

impl ReplSender for NreplSender {
    fn send(&mut self, req: Request) -> Result<()> {
        let mut params = match req {
            Request::NewSession() => {
                let mut params = HashMap::new();
                params.insert(Param::from("op"), Param::from("clone"));
                params
            }
            Request::DisableNsMaps() => {
                let mut params = HashMap::new();
                params.insert(Param::from("op"), Param::from("eval"));
                params.insert(
                    Param::from("code"),
                    Param::from("(set! *print-namespace-maps* false)"),
                );
                params
            }
            Request::Eval(mut params) => {
                params.insert(Param::from("op"), Param::from("eval"));
                params
            }
            Request::Exit() => {
                let mut params = HashMap::new();
                params.insert(Param::from("op"), Param::from("close"));
                params
            }
            Request::Interrupt(mut params) => {
                params.insert(Param::from("op"), Param::from("interrupt"));
                params.insert(
                    Param::from("interrupt-id"),
                    Param::from(self.request_cnt.to_string().as_str()),
                );
                params
            }
        };
        if !self.session_id.is_empty() {
            params.insert(
                Param::from("session"),
                Param::from(self.session_id.as_str()),
            );
        }
        params.insert(
            Param::from("id"),
            Param::from(self.request_cnt.to_string().as_str()),
        );

        log::debug!("Sending request to NREPL: {:?}", &params);

        self.write_and_flush(build_bc_value(params).to_bencode().as_bytes())?;
        self.request_cnt += 1;

        Ok(())
    }

    fn session_id(&self) -> String {
        self.session_id.to_string()
    }
}

impl ReplReceiver for NreplReceiver {
    fn receive(&mut self) -> Result<Response> {
        match bc::parse_bencode(&mut self.reader) {
            Ok(Some(bencode_val)) => {
                log::debug!("Got nREPL message: {}", &bencode_val);
                let resp = Response::try_from(bencode_val)?;
                Ok(resp)
            }
            Ok(None) => {
                log::debug!("Got None/Nil from nREPL");
                Ok(Response::Other("None/Nil Response from nREPL".into()))
            }
            Err(e) => match e {
                bc::BencodeError::Eof() => Ok(Response::Eof()),
                _ => Err(ReplError::Error(format!("BencodeError: {}", e))),
            },
        }
    }
}

impl TryFrom<bc::Value> for Response {
    type Error = ReplError;

    fn try_from(val: bc::Value) -> Result<Self> {
        match val {
            bc::Value::Map(hm) => {
                if let Some(bc::Value::Str(s)) = hm.get(&bc::Value::Str("new-session".into())) {
                    log::debug!("nREPL session: {}", s);
                    return Ok(Response::NewSession(s.to_string()));
                }
                if let Some(bc::Value::Str(s)) = hm.get(&bc::Value::Str("err".into())) {
                    log::debug!("nREPL err : {}", s);
                    return Ok(Response::Err(s.to_string()));
                }
                if let Some(bc::Value::Str(s)) = hm.get(&bc::Value::Str("out".into())) {
                    log::debug!("nREPL out: {}", s);
                    return Ok(Response::Out(s.to_string()));
                }
                if let Some(bc::Value::Str(s)) = hm.get(&bc::Value::Str("ex".into())) {
                    if let Some(bc::Value::Str(s)) =
                        hm.get(&bc::Value::Str("nrepl.middleware.caught/throwable".into()))
                    {
                        log::debug!("nREPL throwable: {}", s);
                        let (trace, _) = parse_exception(&s.replace("#error ", ""));
                        return Ok(Response::Exception(trace, "".to_string()));
                    } else {
                        log::debug!("nREPL ex: {}", s);
                        return Ok(Response::Exception(s.to_string(), "".to_string()));
                    }
                }
                if let Some(bc::Value::List(list)) = hm.get(&bc::Value::Str("status".into())) {
                    log::debug!("nREPL status: {:?}", list);

                    let mut vec: Vec<String> = vec![];
                    for val in list {
                        if let bc::Value::Str(s) = val {
                            vec.push(s.to_string());
                        }
                    }
                    return Ok(Response::Status(vec));
                }
                if let Some(bc::Value::Str(value)) = hm.get(&bc::Value::Str("value".into())) {
                    log::debug!("nREPL value: {}", &value);
                    if let Some(bc::Value::Str(ns)) = hm.get(&bc::Value::Str("ns".into())) {
                        Ok(Response::Value(
                            value.to_string(),
                            ns.to_string(),
                            0,
                            "".into(),
                        ))
                    } else {
                        Err(ReplError::Error(format!(
                            "Missing 'ns' in nREPL value-response: {}",
                            value
                        )))
                    }
                } else {
                    Err(ReplError::Error(format!(
                        "Unsupported nREPL response: {:?}",
                        hm
                    )))
                }
            }
            _ => Err(ReplError::Error(format!(
                "Unexpected nREPL response: {:?}",
                val
            ))),
        }
    }
}
