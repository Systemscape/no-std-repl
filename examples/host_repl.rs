use std::io::{self, Read, Write};
use std::process::{Command, Stdio};

use no_std_repl::{LineBuffer, Outcome, Parts};

const PROMPT: &str = "> ";
const LINE_CAPACITY: usize = 128;
const DEFAULT_COLUMNS: usize = 80;
const SAVE_CURSOR: &str = "\x1b7";
const RESTORE_CURSOR: &str = "\x1b8";
const ERASE_BELOW: &str = "\x1b[J";
const CTRL_C: u8 = 0x03;
const CTRL_D: u8 = 0x04;

/// Terminal raw-mode guard. Restores the previous settings when dropped.
struct RawMode {
    saved: String,
}

impl RawMode {
    /// Returns `None` if stdin is not a terminal or raw mode cannot be enabled.
    fn enable() -> Option<Self> {
        let saved = stty(&["-g"])?;
        stty(&["raw", "-echo"])?;
        Some(Self {
            saved: saved.trim().to_owned(),
        })
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = stty(&[&self.saved]);
    }
}

fn stty(args: &[&str]) -> Option<String> {
    let output = Command::new("stty")
        .args(args)
        .stdin(Stdio::inherit())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn terminal_columns() -> usize {
    stty(&["size"])
        .and_then(|size| size.split_whitespace().nth(1)?.parse().ok())
        .unwrap_or(DEFAULT_COLUMNS)
}

/// Extra rows to reserve below the prompt so a full line cannot scroll the
/// screen and invalidate the saved cursor position.
fn reserved_rows(columns: usize) -> usize {
    // Each byte uses at most one column, and leave one column for early wraps.
    let usable = columns.saturating_sub(1).max(1);
    (PROMPT.len() + LINE_CAPACITY)
        .div_ceil(usable)
        .saturating_sub(1)
}

/// Echo tabs as spaces so each byte uses at most one column.
fn visible(byte: u8) -> u8 {
    if byte == b'\t' { b' ' } else { byte }
}

fn write_prompt(out: &mut impl Write, echo: bool) -> io::Result<()> {
    if !echo {
        return write!(out, "{PROMPT}");
    }

    let rows = reserved_rows(terminal_columns());
    if rows > 0 {
        write!(out, "{}\x1b[{rows}A", "\n".repeat(rows))?;
    }
    // Backspace redraws the line from the saved cursor position.
    write!(out, "{PROMPT}{SAVE_CURSOR}")
}

fn handle_line(line: &str, out: &mut impl Write) -> io::Result<bool> {
    let parts = Parts::<8>::parse(line);
    if parts.is_truncated() {
        write!(out, "too many arguments\r\n")?;
        return Ok(true);
    }

    match parts.as_slice() {
        [] => {}
        ["help"] => {
            write!(out, "commands: help, ping [n], echo <words...>, quit\r\n")?;
        }
        ["ping"] => {
            write!(out, "pong\r\n")?;
        }
        ["ping", n] => {
            write!(out, "pong {n}\r\n")?;
        }
        ["echo", rest @ ..] => {
            write!(out, "{}\r\n", rest.join(" "))?;
        }
        ["quit"] | ["exit"] => {
            write!(out, "bye\r\n")?;
            return Ok(false);
        }
        other => {
            write!(out, "unknown command: {other:?}\r\n")?;
        }
    }

    Ok(true)
}

fn main() -> io::Result<()> {
    // In raw mode the terminal does not echo or edit, as on a serial port.
    // Otherwise, a terminal echoes in line mode; piped input arrives as-is.
    let raw_mode = RawMode::enable();
    let echo = raw_mode.is_some();

    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    let mut line = LineBuffer::<LINE_CAPACITY>::new();
    let mut buf = [0_u8; 1];

    write!(stdout, "host REPL demo\r\n")?;
    write!(stdout, "type `help` for commands\r\n")?;
    write_prompt(&mut stdout, echo)?;
    stdout.flush()?;

    loop {
        if stdin.read(&mut buf)? == 0 {
            break;
        }

        let byte = buf[0];
        if echo && matches!(byte, CTRL_C | CTRL_D) {
            write!(stdout, "\r\n")?;
            break;
        }

        match line.push(byte) {
            Outcome::Stored => {
                if echo {
                    stdout.write_all(&[visible(byte)])?;
                }
            }
            Outcome::Erased => {
                if echo {
                    // Redraw from the saved cursor to handle wide characters
                    // and wrapped lines.
                    write!(stdout, "{RESTORE_CURSOR}{ERASE_BELOW}")?;
                    for &b in line.as_bytes() {
                        stdout.write_all(&[visible(b)])?;
                    }
                }
            }
            Outcome::BufferFull => {
                if echo {
                    write!(stdout, "\x07")?;
                }
            }
            Outcome::Ignored => {}
            Outcome::LineComplete => {
                if echo {
                    write!(stdout, "\r\n")?;
                }

                match line.as_str() {
                    Some(s) => {
                        if !handle_line(s, &mut stdout)? {
                            break;
                        }
                    }
                    None => {
                        write!(stdout, "invalid UTF-8 input\r\n")?;
                    }
                }

                write_prompt(&mut stdout, echo)?;
            }
            Outcome::LineOverflow => {
                if echo {
                    write!(stdout, "\r\n")?;
                }
                write!(stdout, "line too long\r\n")?;
                write_prompt(&mut stdout, echo)?;
            }
        }
        stdout.flush()?;
    }

    Ok(())
}
