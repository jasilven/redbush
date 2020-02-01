use chrono::{DateTime, Local};
use clap::{App, Arg};
use fern;
use log;
use neovim_lib;
use neovim_lib::{Neovim, NeovimApi, Session};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::net::TcpStream;
use std::thread;
mod error;
use error::MyError;

mod bencode;

type Result<T> = std::result::Result<T, MyError>;

struct NreplReceiver<R: BufRead> {
    reader: R,
}

struct NreplSender<W: Write> {
    session: String,
    writer: W,
}

impl<R: BufRead> NreplReceiver<R> {
    fn new(reader: R) -> Self {
        NreplReceiver { reader }
    }

    fn recv(&mut self) -> Result<bencode::Value> {
        log::debug!("Waiting for nREPL message");

        match bencode::parse_bencode(&mut self.reader) {
            Ok(Some(val)) => Ok(val),
            Ok(None) => {
                log::error!("None value from bencode");
                Err(MyError::from("Invalid bencode from nREPL"))
            }
            Err(e) => {
                log::error!("parse_bencode failed: {}", e.to_string());
                Err(e)
            }
        }
    }
}

impl<W: Write> NreplSender<W> {
    fn new(writer: W) -> Self {
        NreplSender {
            session: "<no session>".to_string(),
            writer,
        }
    }

    fn reset_session(&mut self, s: &str) {
        self.session = s.to_string();
    }

    fn send(&mut self, val: &bencode::Value) -> Result<()> {
        log::debug!("Sending request to nREPL: {}", val.to_string(true));

        self.writer.write_all(val.to_bencode().as_bytes())?;
        self.writer.flush()?;

        Ok(())
    }
}

fn connect_nvim_socket() -> Result<Neovim> {
    log::debug!("Connecting NVIM socket");

    let socket_path = std::env::var("NVIM_LISTEN_ADDRESS")?;
    let session = Session::new_unix_socket(socket_path)?;

    Ok(Neovim::new(session))
}

struct LogBuf {
    cursor_line: i64,
    bufno: i64,
    winid: i64,
    max_lines: i64,
}

impl LogBuf {
    fn new(nvim: &mut Neovim, max_lines: i64, path: &str) -> Result<Self> {
        log::debug!("Creating LogBuf: max_lines={}, path={}", max_lines, path);

        let buffers = nvim.list_bufs()?;
        match buffers
            .into_iter()
            .find(|b| path == b.get_name(nvim).unwrap_or_default())
        {
            Some(buf) => {
                let dt: DateTime<Local> = Local::now();
                let lines = vec![format!(
                    ";; Session {} ",
                    dt.format("%Y-%m-%d %H:%M:%S").to_string()
                )];
                let lines_cnt = lines.len() as i64;
                buf.set_lines(nvim, 0, lines_cnt, true, lines)?;
                Ok(LogBuf {
                    cursor_line: lines_cnt,
                    bufno: buf.get_number(nvim)?,
                    winid: nvim
                        .get_var("logbuf_winid")?
                        .as_i64()
                        .ok_or("Unable to get g:logbuf_winid variable from NVIM")?,
                    max_lines,
                })
            }
            None => Err(MyError::from("Logbuf not opened in NVIM")),
        }
    }

    fn get_prefix(&self, s: &str) -> &str {
        match s {
            "err" => ";✖ ",
            "nrepl.middleware.caught/throwable" => ";  ",
            "out" => ";",
            "value" => "",
            _ => ";",
        }
    }

    fn trimmer(&mut self, trim_cnt: i64) -> Vec<neovim_lib::Value> {
        let mut trim: Vec<neovim_lib::Value> = vec!["nvim_buf_set_lines".into()];
        let args: Vec<neovim_lib::Value> = vec![
            self.bufno.into(),
            0.into(),
            trim_cnt.into(),
            true.into(),
            neovim_lib::Value::Array(vec![]),
        ];

        trim.push(args.into());

        trim
    }

    fn get_appender(&mut self, lines: Vec<String>) -> Vec<neovim_lib::Value> {
        let mut append: Vec<neovim_lib::Value> = vec!["nvim_buf_set_lines".into()];
        let mut args: Vec<neovim_lib::Value> =
            vec![self.bufno.into(), (-1).into(), (-1).into(), true.into()];
        let lns: Vec<neovim_lib::Value> = lines.into_iter().map(|l| l.into()).collect();
        args.push(lns.into());
        append.push(args.into());

        append
    }

    fn append_lines(&mut self, nvim: &mut Neovim, lines: Vec<String>) -> Result<()> {
        log::debug!("Appending {} lines to NVIM log buffer", lines.len());

        let mut atom: Vec<neovim_lib::Value> = vec![];
        let lines_cnt = lines.len() as i64;

        if !lines.is_empty() {
            if self.cursor_line + lines_cnt > self.max_lines {
                let trim_cnt = self.max_lines / 2;
                atom.push(self.trimmer(trim_cnt).into());
                self.cursor_line = self.max_lines - trim_cnt + lines_cnt;
            } else {
                self.cursor_line += lines_cnt;
            }

            atom.push(self.get_appender(lines).into());

            let mut cursor: Vec<neovim_lib::Value> = vec!["nvim_win_set_cursor".into()];
            let mut args: Vec<neovim_lib::Value> = vec![self.winid.into()];
            let tuple: Vec<neovim_lib::Value> = vec![self.cursor_line.into(), 0i64.into()];
            args.push(tuple.into());

            cursor.push(args.into());

            atom.push(neovim_lib::Value::from(cursor));

            nvim.call_atomic(atom)?;
        }

        Ok(())
    }

    fn extract_lines(&self, val: &bencode::Value) -> Result<(Vec<String>, String)> {
        let mut lines: Vec<String> = vec![];
        let mut value = String::from("");

        if let bencode::Value::Map(hm) = val {
            for key in &["err", "nrepl.middleware.caught/throwable", "out", "value"] {
                if let Some(bencode::Value::Str(val)) = hm.get(&bencode::Value::from(*key)) {
                    val.lines()
                        .for_each(|l| lines.push(format!("{}{}", self.get_prefix(&key), l)));

                    if key.eq(&"value") {
                        value = val.to_string();
                    }
                }
            }
        } else {
            return Err(MyError::from("Unexpected bencode value"));
        }

        Ok((lines, value))
    }

    fn echo_eval_result(
        &self,
        nvim: &mut Neovim,
        val: &bencode::Value,
        eval_result: &str,
    ) -> Result<()> {
        if let bencode::Value::Map(hm) = val {
            if let Some(bencode::Value::List(list)) = hm.get(&bencode::Value::from("status")) {
                if !eval_result.is_empty() && list.contains(&bencode::Value::from("done")) {
                    log::debug!(
                        "Echoing eval result: '{}'..",
                        &eval_result[0..(std::cmp::min(70, eval_result.len()))]
                    );

                    nvim.out_write(&format!("{}\n", &eval_result))?;
                }
            } else if let Some(bencode::Value::Str(err)) = hm.get(&bencode::Value::from("err")) {
                log::debug!("Echoing eval error: '{}'..", &err);

                nvim.out_write(&format!("ERROR: {}\n", &err.replace('\n', " ")))?;
            }
        }

        Ok(())
    }
}

fn is_exit(val: &bencode::Value) -> bool {
    if let bencode::Value::Map(hm) = val {
        if let Some(bencode::Value::List(list)) = hm.get(&bencode::Value::from("status")) {
            return list.contains(&bencode::Value::from("session-closed"));
        }
    }

    false
}

fn nrepl_loop<R: BufRead>(recver: &mut NreplReceiver<R>, logbuf: &mut LogBuf) -> Result<()> {
    log::debug!("Nrep_loop starting NVIM event loop");

    let mut nvim = connect_nvim_socket()?;
    nvim.session.start_event_loop();

    let mut last_result = String::from("");

    loop {
        match recver.recv() {
            Ok(val) => {
                log::debug!("Got nREPL message: {}", val.to_string(true));

                if !is_exit(&val) {
                    let (lines, result) = logbuf.extract_lines(&val)?;
                    logbuf.append_lines(&mut nvim, lines)?;
                    logbuf.echo_eval_result(&mut nvim, &val, &last_result)?;
                    last_result = result;
                } else {
                    log::debug!("Got exit from nREPL : {}", val.to_string(true));
                    break;
                }
            }
            Err(e) => {
                log::error!("Failed to get message from nREPL: {}", e.to_string());
                return Err(e);
            }
        }
    }

    Ok(())
}

fn setup_logger() -> Result<()> {
    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "{}[{}][{}] {}",
                chrono::Local::now().format("[%H:%M:%S]"),
                record.target(),
                record.level(),
                message
            ))
        })
        .level(log::LevelFilter::Debug)
        .level_for("neovim_lib", log::LevelFilter::Error)
        .level_for("redbush::bencode", log::LevelFilter::Error)
        .chain(std::io::stderr())
        .chain(fern::DateBased::new("/tmp/redbush.log.", "%Y-%m-%d"))
        .apply()?;

    log::debug!("Logger ready");

    Ok(())
}

fn start_nrepl_session<W: Write, R: BufRead>(
    sender: &mut NreplSender<W>,
    recver: &mut NreplReceiver<R>,
) -> Result<String> {
    log::debug!("Starting new nREPL session");

    let mut m = HashMap::new();
    m.insert("op", "clone");

    sender.send(&bencode::Value::from(m))?;

    match recver.recv() {
        Ok(bencode::Value::Map(hm)) => {
            match hm.get(&bencode::Value::Str("new-session".to_string())) {
                Some(bencode::Value::Str(s)) => {
                    sender.reset_session(s);
                    log::debug!("New nREPL session OK");
                    Ok(s.to_string())
                }
                Some(x) => Err(MyError::Error(format!(
                    "Unexpected nREPL response for 'clone' request: {:?}",
                    x
                ))),
                None => Err(MyError::from("No nREPL response for 'clone' request")),
            }
        }
        Ok(x) => Err(MyError::Error(format!(
            "Unexpected nREPL response for 'clone' request: {:?}",
            x
        ))),
        Err(e) => Err(MyError::Error(format!(
            "Failed to get nREPL response: {}",
            e
        ))),
    }
}

fn connect_nrepl<'a, 'b>(
    host: &'a str,
    port: &'b str,
) -> Result<(
    NreplSender<BufWriter<TcpStream>>,
    NreplReceiver<BufReader<TcpStream>>,
)> {
    log::debug!("Opening TcpStream to nREPL");

    let stream = TcpStream::connect(format!("{}:{}", host, port))?;
    let stream2 = stream.try_clone()?;
    let writer = BufWriter::new(stream2);
    let reader = BufReader::new(stream);
    let sender = NreplSender::new(writer);
    let recver = NreplReceiver::new(reader);
    Ok((sender, recver))
}

fn get_args() -> Result<(String, String, i64)> {
    log::debug!("Parsing command line arguments:");

    let matches = App::new("Clojure nREPL plugin ")
        .author("jasilven <jasilven@gmail.com>")
        .about("Clojure nREPL plugin for neovim")
        .arg(
            Arg::with_name("port")
                .short("p")
                .long("port")
                .value_name("PORT")
                .help("nREPL port")
                .required(false),
        )
        .arg(
            Arg::with_name("log")
                .short("l")
                .long("log")
                .value_name("LOG")
                .help("nREPL eval-logfile path e.g. /tmp/redbush-log.clj")
                .required(true),
        )
        .arg(
            Arg::with_name("logsize")
                .short("s")
                .long("logsize")
                .value_name("LOG")
                .help("eval-logfile size in lines")
                .required(false),
        )
        .get_matches();
    let port = match matches.value_of("port") {
        Some(p) => p.to_string(),
        None => match std::fs::read_to_string(".nrepl-port") {
            Ok(p) => p,
            Err(e) => return Err(MyError::from(e)),
        },
    };

    let logpath = match matches.value_of("log") {
        Some(p) => p.to_string(),
        None => return Err(MyError::from("eval-logfile name not given")),
    };

    let logsize = matches.value_of("logsize").unwrap_or("100");

    Ok((port, logpath, logsize.parse::<i64>()?))
}

impl TryFrom<Vec<neovim_lib::Value>> for bencode::Value {
    type Error = &'static str;

    fn try_from(
        nvim_arg: Vec<neovim_lib::Value>,
    ) -> std::result::Result<bencode::Value, Self::Error> {
        log::debug!("Parsing NVIM message");

        if let Some(vals) = nvim_arg.iter().next().unwrap().as_map() {
            let mut hm = HashMap::new();
            for (k, v) in vals {
                if v.is_str() {
                    hm.insert(
                        bencode::Value::from(k.as_str().unwrap_or("")),
                        bencode::Value::from(v.as_str().unwrap_or("")),
                    );
                } else if v.is_i64() {
                    hm.insert(
                        bencode::Value::from(k.as_str().unwrap_or("")),
                        bencode::Value::Int(v.as_i64().unwrap_or(0) as i32),
                    );
                } else {
                    log::warn!("Unsupported value type: {}", v.to_string());
                    return Err("Failed to convert NVIM message to bencode::Value");
                }
            }
            return Ok(bencode::Value::from(hm));
        }

        Err("Unable to convert NVIM message to bencode::Value")
    }
}

fn insert_nrepl_session(val: &mut bencode::Value, session: &str) -> Result<()> {
    log::debug!("Inserting nREPL session key '{}' to request", session);

    match val {
        bencode::Value::Map(hm) => {
            hm.insert(
                bencode::Value::from("session"),
                bencode::Value::from(session),
            );
            Ok(())
        }
        _ => Err(MyError::from(
            "Invalid bencode, cannot insert session key to nREPL request",
        )),
    }
}

fn run() -> Result<()> {
    setup_logger()?;
    log::debug!("---------Starting---------");

    let (port, logfile, logsize) = get_args()?;
    let (mut sender, mut recver) = connect_nrepl("127.0.0.1", &port)?;
    let nrepl_session = start_nrepl_session(&mut sender, &mut recver)?;

    let nvim_session = Session::new_parent()?;
    let mut nvim = Neovim::new(nvim_session);
    let nvim_recver = nvim.session.start_event_loop_channel();

    let mut logbuf = LogBuf::new(&mut nvim, logsize, &logfile)?;

    let nrepl = thread::spawn(move || nrepl_loop(&mut recver, &mut logbuf));

    for (event, nvim_args) in nvim_recver {
        log::debug!("Got NVIM message: event={}", event);
        match event.as_str() {
            "nrepl" => {
                let mut bc_value = bencode::Value::try_from(nvim_args)?;
                log::debug!("Parsed NVIM message: {}", bc_value.to_string(true));

                insert_nrepl_session(&mut bc_value, &nrepl_session)?;
                sender.send(&bc_value)?;
            }
            "stop" | "exit" | _ => {
                let mut hm = HashMap::new();
                hm.insert("op", "close");
                sender.send(&bencode::Value::from(hm))?;
                break;
            }
        }
    }

    log::debug!("Waiting for nREPL thread");
    nrepl.join().expect("Thread join error!")?;

    Ok(())
}

fn main() {
    match run() {
        Ok(_) => log::debug!("Good exit"),
        Err(e) => {
            log::error!("ERROR: {}", e);
            std::process::exit(1);
        }
    }
}
