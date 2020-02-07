use crate::MyError;
use crate::Result;
use chrono::Local;
use neovim_lib::neovim_api::Buffer;
use neovim_lib::{Neovim, NeovimApi};

const DATEFMT: &str = "%H:%M:%S %b %d %Y";

pub struct LogBuf {
    buf: Buffer,
    bufno: i64,
    winid: i64,
    max_lines: i64,
}

impl LogBuf {
    pub fn new(nvim: &mut Neovim, max_lines: i64, path: &str) -> Result<Self> {
        log::debug!("Creating LogBuf: max_lines={}, path={}", max_lines, path);

        let buffers = nvim.list_bufs()?;
        match buffers
            .into_iter()
            .find(|b| path == b.get_name(nvim).unwrap_or_default())
        {
            Some(buf) => Ok(LogBuf {
                bufno: buf.get_number(nvim)?,
                buf,
                winid: nvim
                    .get_var("logbuf_winid")?
                    .as_i64()
                    .ok_or("Unable to get 'g:logbuf_winid' variable from NVIM")?,
                max_lines,
            }),
            None => Err(MyError::from("Logbuf not opened in NVIM")),
        }
    }

    pub fn message(&self, nvim: &mut Neovim, msg: &str) -> Result<()> {
        log::debug!("Showing logbuf welcome");
        let lines = vec![format!(";; [{}] {}", Local::now().format(DATEFMT), msg)];
        let start = if self.buf.line_count(nvim)? > 1 {
            -1
        } else {
            0
        };

        self.buf.set_lines(nvim, start, -1, true, lines)?;
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

    fn cursor_setter(&mut self, cursor_line: i64) -> Vec<neovim_lib::Value> {
        let mut cursor: Vec<neovim_lib::Value> = vec!["nvim_win_set_cursor".into()];
        let mut args: Vec<neovim_lib::Value> = vec![self.winid.into()];
        let tuple: Vec<neovim_lib::Value> = vec![cursor_line.into(), 0i64.into()];
        args.push(tuple.into());
        cursor.push(args.into());

        cursor
    }

    pub fn append_lines(&mut self, nvim: &mut Neovim, lines: Vec<String>) -> Result<()> {
        let lines_cnt = lines.len() as i64;
        log::debug!("Appending to NVIM log buffer: {} lines", lines_cnt);
        let mut cursor_line = self.buf.line_count(nvim)?;

        let mut atom: Vec<neovim_lib::Value> = vec![];
        if !lines.is_empty() {
            if cursor_line + lines_cnt > self.max_lines {
                let trim_cnt = self.max_lines / 2;

                atom.push(self.trimmer(trim_cnt).into());

                cursor_line = self.max_lines - trim_cnt + lines_cnt;
            } else {
                cursor_line += lines_cnt;
            }

            atom.push(self.appender(lines).into());
            atom.push(self.cursor_setter(cursor_line).into());
            nvim.call_atomic(atom)?;
        }

        Ok(())
    }

    pub fn show(&mut self, nvim: &mut Neovim, prefix: &str, content: &str) -> Result<()> {
        self.append_lines(
            nvim,
            content
                .lines()
                .map(|s| format!("{}{}", prefix, s))
                .collect(),
        )?;

        Ok(())
    }
}
