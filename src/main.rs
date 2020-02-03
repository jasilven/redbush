use clap::{App, Arg};
use fern;
use log;
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
mod logbuf;
use logbuf::LogBuf;

type Result<T> = std::result::Result<T, MyError>;

fn connect_nvim_socket() -> Result<Neovim> {
    log::debug!("Connecting NVIM socket");

    let socket_path = std::env::var("NVIM_LISTEN_ADDRESS")?;
    let mut session = Session::new_unix_socket(socket_path)?;
    session.start_event_loop();

    Ok(Neovim::new(session))
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

fn get_nrepl_session_id(
    writer: &mut BufWriter<TcpStream>,
    reader: &mut BufReader<TcpStream>,
) -> Result<String> {
    log::debug!("Starting nREPL session");

    let mut m = HashMap::new();
    m.insert("op", "clone");
    let val = bc::Value::from(m);
    write_and_flush(writer, val.to_bencode().as_bytes())?;

    match bc::parse_bencode(reader) {
        Ok(Some(bc::Value::Map(hm))) => match hm.get(&bc::Value::Str("new-session".to_string())) {
            Some(bc::Value::Str(s)) => {
                log::debug!("nREPL session id: {}", s);
                Ok(s.to_string())
            }
            Some(x) => Err(MyError::Error(format!(
                "Unexpected nREPL response for 'clone' request: {:?}",
                x
            ))),
            None => Err(MyError::from("No nREPL response for 'clone' request")),
        },
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
    log::debug!("Connecting nREPL port {}", port);

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
            Arg::with_name("filepath")
                .short("f")
                .long("file")
                .value_name("FILE")
                .help("file path e.g. /tmp/redbush-eval.clj")
                .required(true),
        )
        .arg(
            Arg::with_name("filesize")
                .short("s")
                .long("size")
                .value_name("LOG")
                .help("file size in lines")
                .required(false),
        )
        .get_matches();
    let port = match matches.value_of("port") {
        Some(x) => x.to_string(),
        None => match std::fs::read_to_string(".nrepl-port") {
            Ok(x) => x,
            Err(e) => return Err(MyError::from(e)),
        },
    };

    let filepath = match matches.value_of("filepath") {
        Some(x) => x.to_string(),
        None => return Err(MyError::from("file name not given")),
    };

    let filesize = matches.value_of("filesize").unwrap_or("1000");

    Ok((port, filepath, filesize.parse::<i64>()?))
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

    let (port, filepath, filesize) = get_args()?;
    let (mut writer, mut reader) = connect_nrepl("127.0.0.1", &port)?;
    let session_id = get_nrepl_session_id(&mut writer, &mut reader)?;

    let nvim_session = Session::new_parent()?;
    let mut nvim = Neovim::new(nvim_session);
    let nvim_channel = nvim.session.start_event_loop_channel();

    log::debug!("Setting redbush_nrepl_session_id");
    nvim.set_var(
        "redbush_nrepl_session_id",
        neovim_lib::Value::from(session_id.as_str()),
    )?;

    let mut logbuf = LogBuf::new(&mut nvim, filesize, &filepath)?;
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
            log::error!("{}", e);
            std::process::exit(1);
        }
    }
}
