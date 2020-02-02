use chrono::Local;
use clap::{App, Arg};
use fern;
use log;
use neovim_lib::neovim_api::Buffer;
use neovim_lib::{Neovim, NeovimApi, Session};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::convert::TryInto;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::net::TcpStream;
use std::thread;
mod error;
use error::MyError;

mod bencode;
use bencode as bc;

type Result<T> = std::result::Result<T, MyError>;

const DATEFMT: &'static str = "%H:%M:%S %b %d %Y";

fn connect_nvim_socket() -> Result<Neovim> {
    log::debug!("Connecting NVIM socket");

    let socket_path = std::env::var("NVIM_LISTEN_ADDRESS")?;
    let mut session = Session::new_unix_socket(socket_path)?;
    session.start_event_loop();

    Ok(Neovim::new(session))
}

struct LogBuf<'a> {
    buf: Buffer,
    cursor_line: i64,
    bufno: i64,
    winid: i64,
    max_lines: i64,
    prefix: HashMap<&'a str, &'a str>,
}

impl LogBuf<'_> {
    fn new(nvim: &mut Neovim, max_lines: i64, path: &str) -> Result<Self> {
        log::debug!("Creating LogBuf: max_lines={}, path={}", max_lines, path);

        let buffers = nvim.list_bufs()?;
        match buffers
            .into_iter()
            .find(|b| path == b.get_name(nvim).unwrap_or_default())
        {
            Some(buf) => Ok(LogBuf {
                cursor_line: 0,
                bufno: buf.get_number(nvim)?,
                buf: buf,
                winid: nvim
                    .get_var("logbuf_winid")?
                    .as_i64()
                    .ok_or("Unable to get 'g:logbuf_winid' variable from NVIM")?,
                max_lines,
                prefix: [
                    ("err", ";✖ "),
                    ("nrepl.middleware.caught/throwable", ";  "),
                    ("out", ";"),
                    ("value", ""),
                ]
                .iter()
                .cloned()
                .collect(),
            }),
            None => Err(MyError::from("Logbuf not opened in NVIM")),
        }
    }

    fn wellcome(&mut self, nvim: &mut Neovim) -> Result<()> {
        let lines = vec![format!(";; Start {} ", Local::now().format(DATEFMT))];
        let lines_len = lines.len() as i64;
        self.buf.set_lines(nvim, 0, -lines_len, true, lines)?;
        self.cursor_line = lines_len;
        Ok(())
    }

    fn goodbye(&mut self, nvim: &mut Neovim) -> Result<()> {
        let lines = vec![format!(";; End   {} ", Local::now().format(DATEFMT))];
        self.append_lines(nvim, lines)?;
        Ok(())
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

    fn appender(&mut self, lines: Vec<String>) -> Vec<neovim_lib::Value> {
        let mut append: Vec<neovim_lib::Value> = vec!["nvim_buf_set_lines".into()];
        let mut args: Vec<neovim_lib::Value> =
            vec![self.bufno.into(), (-1).into(), (-1).into(), true.into()];
        let lns: Vec<neovim_lib::Value> = lines.into_iter().map(|l| l.into()).collect();
        args.push(lns.into());
        append.push(args.into());

        append
    }

    fn cursor_setter(&mut self) -> Vec<neovim_lib::Value> {
        let mut cursor: Vec<neovim_lib::Value> = vec!["nvim_win_set_cursor".into()];
        let mut args: Vec<neovim_lib::Value> = vec![self.winid.into()];
        let tuple: Vec<neovim_lib::Value> = vec![self.cursor_line.into(), 0i64.into()];
        args.push(tuple.into());
        cursor.push(args.into());

        cursor
    }

    fn append_lines(&mut self, nvim: &mut Neovim, lines: Vec<String>) -> Result<()> {
        let lines_cnt = lines.len() as i64;
        log::debug!("Appending to NVIM log buffer: {} lines", lines_cnt);

        let mut atom: Vec<neovim_lib::Value> = vec![];

        if !lines.is_empty() {
            if self.cursor_line + lines_cnt > self.max_lines {
                let trim_cnt = self.max_lines / 2;

                atom.push(self.trimmer(trim_cnt).into());

                self.cursor_line = self.max_lines - trim_cnt + lines_cnt;
            } else {
                self.cursor_line += lines_cnt;
            }

            atom.push(self.appender(lines).into());
            atom.push(self.cursor_setter().into());
            nvim.call_atomic(atom)?;
        }

        Ok(())
    }
}

fn is_exit(val: &bc::Value) -> bool {
    if let bc::Value::Map(hm) = val {
        if let Some(bc::Value::List(list)) = hm.get(&bc::Value::from("status")) {
            return list.contains(&bc::Value::from("session-closed"));
        }
    }

    false
}

fn nrepl_loop<R: BufRead>(reader: &mut R, logbuf: &mut LogBuf) -> Result<()> {
    log::debug!("Nrep_loop starting NVIM event loop");
    let mut nvim = connect_nvim_socket()?;

    logbuf.wellcome(&mut nvim)?;

    let mut last_result = String::from("");

    loop {
        match bc::parse_bencode(reader) {
            Ok(Some(val)) => {
                log::debug!("Got nREPL message: {}", &val);

                if !is_exit(&val) {
                    let nrepl_map: HashMap<String, String> = val.try_into()?;

                    if let Some(key) = nrepl_map
                        .keys()
                        .find(|k| logbuf.prefix.get(k.as_str()).is_some())
                    {
                        let val: Vec<String> = nrepl_map
                            .get(key)
                            .unwrap()
                            .lines()
                            .map(|s| format!("{}{}", logbuf.prefix.get(key.as_str()).unwrap(), s))
                            .collect();
                        logbuf.append_lines(&mut nvim, val)?;
                    }

                    if let Some(s) = nrepl_map.get("status") {
                        if s.contains("done") && !last_result.is_empty() {
                            nvim.out_write(&format!("{}\n", &last_result))?;
                        }
                    } else if let Some(s) = nrepl_map.get("err") {
                        nvim.out_write(&format!("ERROR: {}\n", &s.replace('\n', " ")))?;
                        last_result = "".into();
                    } else if let Some(s) = nrepl_map.get("value") {
                        last_result = s.to_string();
                    }
                } else {
                    break;
                }
            }
            Ok(None) => {
                log::error!("Got None/empty response from nREPL");
                return Err(MyError::from("Got None/empty response from nREPL"));
            }
            Err(e) => {
                log::error!("Failed to get message from nREPL: {}", e);
                return Err(e.into());
            }
        }
    }

    logbuf.goodbye(&mut nvim)?;

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

fn start_nrepl_session(
    writer: &mut BufWriter<TcpStream>,
    reader: &mut BufReader<TcpStream>,
) -> Result<String> {
    log::debug!("Starting new nREPL session");

    let mut m = HashMap::new();
    m.insert("op", "clone");
    let val = bc::Value::from(m);
    write_and_flush(writer, val.to_bencode().as_bytes())?;

    match bc::parse_bencode(reader) {
        Ok(Some(bc::Value::Map(hm))) => {
            match hm.get(&bc::Value::Str("new-session".to_string())) {
                Some(bc::Value::Str(s)) => {
                    // sender.reset_session(s);
                    log::debug!("New nREPL session: {}", s);
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
) -> Result<(BufWriter<TcpStream>, BufReader<TcpStream>)> {
    log::debug!("Opening TcpStream to nREPL");

    let stream = TcpStream::connect(format!("{}:{}", host, port))?;
    let stream2 = stream.try_clone()?;
    let writer = BufWriter::new(stream2);
    let reader = BufReader::new(stream);
    Ok((writer, reader))
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

impl TryFrom<Vec<neovim_lib::Value>> for bc::Value {
    type Error = &'static str;

    fn try_from(nvim_arg: Vec<neovim_lib::Value>) -> std::result::Result<bc::Value, Self::Error> {
        log::debug!("Parsing NVIM message");

        if let Some(vals) = nvim_arg.iter().next().unwrap().as_map() {
            let mut hm = HashMap::new();
            for (k, v) in vals {
                if v.is_str() {
                    hm.insert(
                        bc::Value::from(k.as_str().unwrap_or("")),
                        bc::Value::from(v.as_str().unwrap_or("")),
                    );
                } else if v.is_i64() {
                    hm.insert(
                        bc::Value::from(k.as_str().unwrap_or("")),
                        bc::Value::Int(v.as_i64().unwrap_or(0) as i32),
                    );
                } else {
                    log::warn!("Unsupported value type: {}", v.to_string());
                    return Err("Failed to convert NVIM message to bc::Value");
                }
            }
            return Ok(bc::Value::from(hm));
        }

        Err("Unable to convert NVIM message to bc::Value")
    }
}

fn write_and_flush(writer: &mut BufWriter<TcpStream>, buf: &[u8]) -> Result<()> {
    writer.write_all(buf)?;
    writer.flush()?;
    Ok(())
}

fn run() -> Result<()> {
    setup_logger()?;
    log::debug!("---------Starting---------");

    let (port, logfile, logsize) = get_args()?;
    let (mut writer, mut reader) = connect_nrepl("127.0.0.1", &port)?;
    let session_id = start_nrepl_session(&mut writer, &mut reader)?;

    let nvim_session = Session::new_parent()?;
    let mut nvim = Neovim::new(nvim_session);
    let nvim_channel = nvim.session.start_event_loop_channel();
    nvim.set_var(
        "redbush_nrepl_session_id",
        neovim_lib::Value::from(session_id.as_str()),
    )?;

    let mut logbuf = LogBuf::new(&mut nvim, logsize, &logfile)?;
    let nrepl = thread::spawn(move || nrepl_loop(&mut reader, &mut logbuf));

    for (event, nvim_args) in nvim_channel {
        log::debug!("Got NVIM message: event={}", event);
        match event.as_str() {
            "nrepl" => {
                let bc = bc::Value::try_from(nvim_args)?;
                log::debug!("nREPL message from NVIM: {}", &bc);
                write_and_flush(&mut writer, bc.to_bencode().as_bytes())?;
            }
            "stop" | "exit" | _ => {
                let mut hm = HashMap::new();
                hm.insert("op", "close");
                write_and_flush(&mut writer, &bc::Value::from(hm).to_bencode().as_bytes())?;
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
