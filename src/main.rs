use clap::{App, Arg};
use fern;
use log;
use neovim_lib::{Neovim, NeovimApi, Session};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::thread;
mod error;
use error::MyError;

mod logbuf;
mod nrepl;
mod prepl;
mod repl;
use repl::{ReplReceiver, ReplSender};

type Result<T> = std::result::Result<T, MyError>;

fn connect_nvim_socket() -> Result<Neovim> {
    log::debug!("Connecting NVIM socket");

    let socket_path = std::env::var("NVIM_LISTEN_ADDRESS")?;
    let mut session = Session::new_unix_socket(socket_path)?;
    session.start_event_loop();

    Ok(Neovim::new(session))
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
        .level_for("redbush::repl::bencode", log::LevelFilter::Error)
        .chain(fern::DateBased::new("/tmp/redbush.log.", "%Y-%m-%d"))
        .apply()?;

    log::debug!("Logger ready");

    Ok(())
}

fn get_args() -> Result<(String, String, String, i64)> {
    log::debug!("Parsing command line arguments:");

    let matches = App::new("Clojure xREPL plugin ")
        .author("jasilven <jasilven@gmail.com>")
        .about("Clojure xREPL plugin for neovim")
        .arg(
            Arg::with_name("host")
                .short("h")
                .long("host")
                .value_name("HOST")
                .help("xREPL host")
                .required(false),
        )
        .arg(
            Arg::with_name("port")
                .short("p")
                .long("port")
                .value_name("PORT")
                .help("xREPL port")
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

    let host = matches.value_of("host").unwrap_or("127.0.0.1");

    let port = match matches.value_of("port") {
        Some(p) => p.to_string(),
        None => match std::fs::read_to_string(".nrepl-port") {
            Ok(p) => p,
            Err(_) => match std::fs::read_to_string(".prepl-port") {
                Ok(p) => p,
                Err(e) => {
                    log::debug!("REPL port missing");
                    return Err(MyError::from(format!(
                    "No '-port'-parameter given and .nrepl-port/.prepl-port files not found: {}",
                    e
                )));
                }
            },
        },
    };

    let filepath = match matches.value_of("filepath") {
        Some(x) => x.to_string(),
        None => return Err(MyError::from("file name not given")),
    };

    let filesize = matches.value_of("filesize").unwrap_or("1000");

    Ok((host.to_string(), port, filepath, filesize.parse::<i64>()?))
}

fn to_params(nvim_args: Vec<neovim_lib::Value>) -> Result<HashMap<repl::Param, repl::Param>> {
    log::debug!("Parsing NVIM message");

    if let Some(vals) = nvim_args.iter().next().unwrap().as_map() {
        let mut hm = HashMap::new();

        for (k, v) in vals {
            if v.is_str() {
                hm.insert(
                    repl::Param::Str(k.as_str().unwrap_or("").into()),
                    repl::Param::Str(v.as_str().unwrap_or("").into()),
                );
            } else if v.is_i64() {
                hm.insert(
                    repl::Param::Str(k.as_str().unwrap_or("").into()),
                    repl::Param::Int(v.as_i64().unwrap_or(0) as i32),
                );
            } else {
                log::warn!("Unsupported NVIM value type: {}", v.to_string());
                return Err("Failed to convert NVIM message to bc::Value".into());
            }
        }

        return Ok(hm);
    }

    Err("Unable to convert NVIM message".into())
}

fn repl_loop(mut receiver: impl ReplReceiver, logbuf: &mut logbuf::LogBuf) -> Result<()> {
    log::debug!("repl_loop starting NVIM event loop");
    let mut nvim = connect_nvim_socket()?;

    let mut prefix = HashMap::<String, String>::new();
    prefix.insert("err".into(), ";✖ ".into());
    prefix.insert("exc".into(), ";  ".into());
    prefix.insert("out".into(), ";".into());
    prefix.insert("ns".into(), ";=> ".into());
    prefix.insert("status".into(), ";; Status: ".into());
    prefix.insert("value".into(), "".into());

    logbuf.message(&mut nvim, "Start")?;

    loop {
        match receiver.receive() {
            Ok(repl::Response::Value(value, ns, ms, form)) => {
                log::debug!(
                    "Got VALUE response from REPL: value: {}, ns: {}, ms: {}, form: {}",
                    value,
                    ns,
                    ms,
                    form
                );
                logbuf.show(
                    &mut nvim,
                    prefix.get("value").unwrap_or(&"".to_string()),
                    &value,
                )?;
                logbuf.show(&mut nvim, prefix.get("ns").unwrap_or(&"".to_string()), &ns)?;
                nvim.out_write(&format!("{}\n", &value))?;
            }
            Ok(repl::Response::Err(s)) => {
                log::debug!("Got ERR response from REPL: {}", s);
                logbuf.show(&mut nvim, prefix.get("err").unwrap_or(&"".to_string()), &s)?;
                nvim.out_write(&format!("ERROR: {}\n", &s))?;
            }
            Ok(repl::Response::Out(s)) => {
                log::debug!("Got OUT response from REPL: {}", s);
                logbuf.show(&mut nvim, prefix.get("out").unwrap_or(&"".to_string()), &s)?;
            }
            Ok(repl::Response::Exception(s)) => {
                log::debug!("Got EXCEPTION response from REPL: {}", s);
                logbuf.show(&mut nvim, prefix.get("exc").unwrap_or(&"".to_string()), &s)?;
            }
            Ok(repl::Response::Other(s)) => {
                log::debug!("Got OTHER response from REPL: {}", s);
            }
            Ok(repl::Response::NewSession(s)) => {
                log::debug!("Got NEWSESSION response from REPL: {}", s);
            }
            Ok(repl::Response::Status(v)) => {
                log::debug!("Got STATUS response from REPL: {:?}", &v);

                let mut status = "".to_string();
                v.iter().for_each(|s| status.push_str(&format!("{} ", s)));

                logbuf.show(
                    &mut nvim,
                    prefix.get("status").unwrap_or(&"".to_string()),
                    &status,
                )?;

                if v.contains(&"session-closed".to_string()) {
                    break;
                }
            }
            Ok(repl::Response::Eof()) => {
                log::debug!("Got EOF response from REPL");
                nvim.command("RedBushStop")?;
                logbuf.message(&mut nvim, "REPL died?")?;
                panic!("Got EOF from REPL");
            }
            Err(e) => {
                log::debug!("Got Error from REPL: {}", &e);
                nvim.command("RedBushStop")?;
                logbuf.message(&mut nvim, "REPL died?")?;
                panic!(format!("Failed to get REPL message (REPL died?): {}", e));
            }
        }
    }

    logbuf.message(&mut nvim, "End")?;

    Ok(())
}

fn run(
    mut sender: impl ReplSender,
    receiver: impl ReplReceiver,
    filesize: i64,
    filepath: &str,
) -> Result<()> {
    let nvim_session = Session::new_parent()?;
    let mut nvim = Neovim::new(nvim_session);
    let nvim_channel = nvim.session.start_event_loop_channel();

    let mut logbuf = logbuf::LogBuf::new(&mut nvim, filesize, &filepath)?;
    let nrepl_t = thread::spawn(move || repl_loop(receiver, &mut logbuf));

    log::debug!("Setting NVIM 'g:redbush_repl_session_id'");
    nvim.set_var(
        "redbush_repl_session_id",
        neovim_lib::Value::from(sender.session_id().as_str()),
    )?;

    for (event, nvim_args) in nvim_channel {
        log::debug!("Got NVIM event: {}", event);

        match event.as_str() {
            "eval" => {
                let params = to_params(nvim_args)?;
                log::debug!("EVAL-message from NVIM, params: {:?}", &params);
                sender.send(repl::Request::Eval(params))?;
            }

            "interrupt" => {
                let params = to_params(nvim_args)?;
                log::debug!("INTERRUPT-message from NVIM, params: {:?}", &params);
                sender.send(repl::Request::Interrupt(params))?;
            }

            "stop" | "exit" | _ => {
                log::debug!("EXIT-message from NVIM");
                sender.send(repl::Request::Exit())?;
                break;
            }
        }
    }

    log::debug!("Waiting for nREPL thread");

    match nrepl_t.join() {
        Err(e) => Err(MyError::from(format!("Error from nREPL thread: {:?}", e))),
        _ => Ok(()),
    }
}

fn main() -> Result<()> {
    setup_logger().unwrap();
    log::debug!("---------------- Starting ---------------- ");

    let (host, port, filepath, filesize) = get_args()?;

    log::debug!("Connecting REPL");
    let mut stream = TcpStream::connect(format!("{}:{}", host, port))?;

    log::debug!("Handshaking with REPL");
    let _ = stream.write(b"d4:code7:(+ 1 1)2:op4:evale\n")?;
    stream.flush()?;

    let mut buf = [0u8; 1];

    match stream.read_exact(&mut buf) {
        Ok(_) => {
            if buf[0] == 123 {
                log::debug!("pREPL is available");
                let (sender, receiver) = prepl::new_sender_receiver(&host, &port)?;
                run(sender, receiver, filesize, &filepath)
            } else if buf[0] == 100 {
                log::debug!("nREPL is available");
                let (sender, receiver) = nrepl::new_sender_receiver(&host, &port)?;
                run(sender, receiver, filesize, &filepath)
            } else {
                log::debug!(
                    "Unexpected response from nREPL or pREPL at {}:{}",
                    host,
                    port
                );
                std::process::exit(1)
            }
        }
        Err(_) => {
            log::debug!(
                "Neither nREPL or pREPL is not available at {}:{}",
                host,
                port
            );
            std::process::exit(1)
        }
    }
}
