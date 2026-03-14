use std::io::{self, Read, Write};

use no_std_repl::{LineBuffer, Outcome, Parts};

fn handle_line(line: &str, out: &mut impl Write) -> io::Result<bool> {
    let parts = Parts::<8>::parse(line);
    if parts.is_truncated() {
        writeln!(out, "too many arguments")?;
        return Ok(true);
    }

    match parts.as_slice() {
        [] => {}
        ["help"] => {
            writeln!(out, "commands: help, ping [n], echo <words...>, quit")?;
        }
        ["ping"] => {
            writeln!(out, "pong")?;
        }
        ["ping", n] => {
            writeln!(out, "pong {n}")?;
        }
        ["echo", rest @ ..] => {
            writeln!(out, "{}", rest.join(" "))?;
        }
        ["quit"] | ["exit"] => {
            writeln!(out, "bye")?;
            return Ok(false);
        }
        other => {
            writeln!(out, "unknown command: {other:?}")?;
        }
    }

    Ok(true)
}

fn main() -> io::Result<()> {
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    let mut line = LineBuffer::<128>::new();
    let mut buf = [0_u8; 1];

    writeln!(stdout, "host REPL demo")?;
    writeln!(stdout, "type `help` for commands")?;
    write!(stdout, "> ")?;
    stdout.flush()?;

    loop {
        if stdin.read(&mut buf)? == 0 {
            break;
        }

        let byte = buf[0];
        if !matches!(byte, b'\r' | b'\n') {
            stdout.write_all(&[byte])?;
            stdout.flush()?;
        }

        match line.push(byte) {
            Outcome::Continue => {}
            Outcome::LineComplete => {
                writeln!(stdout)?;

                match line.as_str() {
                    Some(s) => {
                        if !handle_line(s, &mut stdout)? {
                            break;
                        }
                    }
                    None => {
                        writeln!(stdout, "invalid UTF-8 input")?;
                    }
                }

                line.clear();
                write!(stdout, "> ")?;
                stdout.flush()?;
            }
            Outcome::BufferFull => {
                writeln!(stdout, "\nline too long")?;
                line.clear();
                write!(stdout, "> ")?;
                stdout.flush()?;
            }
        }
    }

    Ok(())
}
