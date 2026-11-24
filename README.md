# no-std-repl

Minimal `no_std` building blocks for serial REPLs on MCUs.

The crate turns input bytes into lines and command parts, and handles backspace
during input. Your application provides I/O and command dispatch.

## Usage

```rust,no_run
# use core::fmt;
# trait Serial: fmt::Write {
#     fn try_read(&mut self) -> Option<u8>;
#     fn write(&mut self, byte: u8);
# }
use no_std_repl::{LineBuffer, Outcome, Parts};

// Echo tabs as spaces so each byte uses at most one column
fn visible(byte: u8) -> u8 {
    if byte == b'\t' { b' ' } else { byte }
}

fn repl_loop(serial: &mut impl Serial) {
    let mut line = LineBuffer::<128>::new();

    loop {
        // Reserve one row (enough for this buffer at 66+ columns), return to
        // the prompt row, then save the cursor at the input start.
        write!(serial, "\n\x1b[A> \x1b7").ok();

        // Read through the newline and echo only bytes stored by the buffer
        let complete = loop {
            if let Some(byte) = serial.try_read() {
                match line.push(byte) {
                    Outcome::Stored => serial.write(visible(byte)),
                    Outcome::Erased => {
                        // Redraw from the saved cursor to handle wide characters
                        // and wrapped lines
                        write!(serial, "\x1b8\x1b[J").ok();
                        for &b in line.as_bytes() {
                            serial.write(visible(b));
                        }
                    }
                    Outcome::BufferFull => serial.write(0x07), // Ring the terminal bell.
                    Outcome::Ignored => {}
                    Outcome::LineComplete => break true,
                    Outcome::LineOverflow => break false,
                }
            }
        };
        write!(serial, "\r\n").ok();

        if !complete {
            write!(serial, "line too long\r\n").ok();
            continue;
        }

        let s = match line.as_str() {
            Some(s) => s,
            None => {
                write!(serial, "invalid input\r\n").ok();
                continue;
            }
        };

        // Dispatch on the parsed command parts.
        let parts = Parts::<8>::parse(s);
        if parts.is_truncated() {
            write!(serial, "too many arguments\r\n").ok();
            continue;
        }

        match parts.as_slice() {
            ["ping"] => {
                write!(serial, "pong\r\n").ok();
            }
            ["ping", n] => {
                if let Ok(v) = n.parse::<u32>() {
                    write!(serial, "ping {v}\r\n").ok();
                } else {
                    write!(serial, "bad number: {n}\r\n").ok();
                }
            }
            ["set", key, value] => {
                write!(serial, "setting {key} = {value}\r\n").ok();
            }
            ["help"] => {
                write!(serial, "commands: ping, set <key> <value>, help\r\n").ok();
            }
            [] => {}
            other => {
                write!(serial, "unknown: {other:?}\r\n").ok();
            }
        }
    }
}
# fn main() {}
```

## Core API

Three types, no dependencies, no allocations:

- **`LineBuffer<N>`:** Buffers input through a line ending; handles backspace,
  escape sequences, control bytes, and CR+LF coalescing.
- **`Parts<'a, N>`:** Splits a line on whitespace for slice pattern matching.
- **`Outcome`:** Reports whether input was stored, erased, ignored, completed a
  line, overflowed, or filled the buffer.

Match commands with ordinary Rust slice patterns such as `["cmd", arg1, arg2]`.

## Behavior

- **Line endings:** `\r`, `\n`, and `\r\n` terminate a line; the terminator is
  not stored. A completed line stays readable until the next byte starts a new
  line, so calling `LineBuffer::clear()` between lines is optional.
- **Echo and erase:** Echo bytes reported as `Stored`. Do not echo `Ignored` or
  `BufferFull`; a bell is a common signal for `BufferFull`, which is reported
  once per dropped character. On `Erased`, redraw from the saved cursor after
  the prompt: restore it (`"\x1b8"`), erase to the end of the screen
  (`"\x1b[J"`), then write the buffer. This handles wide characters,
  combining marks, and wrapped lines. For single-column input that never
  wraps, `"\x08 \x08"` is enough to erase a character.
- **Terminal rows:** The saved cursor is valid only while the screen does not
  scroll. Reserve enough rows below the prompt for the longest line. If tabs
  are echoed as spaces, each byte uses at most one column. For prompt width `P`,
  buffer capacity `N`, and terminal width `C`, reserve
  `ceil((P + N) / (C - 1)) - 1` extra rows. Move down with one `"\n"` per row,
  then back up with `"\x1b[<rows>A"` before writing the prompt. The example's
  128-byte buffer needs one extra row on terminals at least 66 columns wide.
- **Input:** Backspace (`0x08` or `0x7F`) removes one buffered UTF-8 character.
  ANSI CSI/SS3 sequences and Alt key combinations are consumed. Unsupported C0
  controls (such as Ctrl-C and NUL) are ignored; tab is stored, and CR/LF end
  the line.
- **Overflow:** A character is stored whole or dropped whole. `BufferFull` is
  reported on the first byte of each dropped character; its remaining bytes
  are ignored. Backspace removes dropped characters before buffered ones. At
  the line ending, any characters still dropped produce `LineOverflow`; if
  backspace removes them all, the line completes normally.
- **UTF-8:** `LineBuffer::as_str()` returns `None` when the buffered bytes are
  invalid UTF-8. `LineBuffer::as_bytes()` always exposes the raw bytes.
- **Part limit:** `Parts::<N>::parse()` captures at most `N` whitespace-
  separated parts. `Parts::is_truncated()` reports whether more were present.
- **Default capacities:** `LineBuffer` defaults to `128` bytes and `Parts` to
  `8` parts in type position (`line: LineBuffer`). Expressions must specify the
  capacity: `LineBuffer::<128>::new()` or `Parts::<8>::parse(line)`.

## Example

Run the host-side demo:

```text
cargo run --example host_repl
```

On a terminal, the demo uses `stty` for raw input and echoes bytes accepted by
`LineBuffer`. Backspace edits the line; arrow-key sequences are discarded. If
raw mode is unavailable or input is piped, the demo falls back to line input
without echo.

See [examples/host_repl.rs](https://github.com/Systemscape/no-std-repl/blob/main/examples/host_repl.rs)
for a complete byte-by-byte REPL loop.
