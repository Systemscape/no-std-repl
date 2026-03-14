# no-std-repl

Minimal `no_std` building blocks for serial REPL interfaces on MCUs.

This crate does not provide I/O, command dispatch, or editing beyond single-byte
backspace handling. It focuses on the byte-to-line and line-to-parts steps that
are awkward to rewrite in every embedded project.

## Usage

```rust,no_run
# use core::fmt;
# trait Serial: fmt::Write {
#     fn try_read(&mut self) -> Option<u8>;
#     fn write(&mut self, byte: u8);
# }
use no_std_repl::{LineBuffer, Outcome, Parts};

fn repl_loop(serial: &mut impl Serial) {
    let mut line = LineBuffer::<128>::new();

    loop {
        write!(serial, "> ").ok();

        // Read until newline
        loop {
            if let Some(byte) = serial.try_read() {
                serial.write(byte); // echo
                match line.push(byte) {
                    Outcome::Continue => {}
                    Outcome::LineComplete => {
                        writeln!(serial).ok();
                        break;
                    }
                    Outcome::BufferFull => {
                        writeln!(serial, "\r\nline too long").ok();
                        line.clear();
                        break;
                    }
                }
            }
        }

        let s = match line.as_str() {
            Some(s) => s,
            None => {
                writeln!(serial, "invalid input").ok();
                line.clear();
                continue;
            }
        };

        // Pattern match on command parts
        let parts = Parts::<8>::parse(s);
        if parts.is_truncated() {
            writeln!(serial, "too many arguments").ok();
            line.clear();
            continue;
        }

        match parts.as_slice() {
            ["ping"] => {
                writeln!(serial, "pong").ok();
            }
            ["ping", n] => {
                if let Ok(v) = n.parse::<u32>() {
                    writeln!(serial, "ping {v}").ok();
                } else {
                    writeln!(serial, "bad number: {n}").ok();
                }
            }
            ["set", key, value] => {
                writeln!(serial, "setting {key} = {value}").ok();
            }
            ["help"] => {
                writeln!(serial, "commands: ping, set <key> <value>, help").ok();
            }
            [] => {}
            other => {
                writeln!(serial, "unknown: {other:?}").ok();
            }
        }

        line.clear();
    }
}
# fn main() {}
```

## Design

Three types, zero dependencies, zero allocations:

- **`LineBuffer<N>`** - Accumulates bytes until newline, handles backspace and CR+LF coalescing
- **`Parts<'a, N>`** - Parses a line into parts for slice pattern matching
- **`Outcome`** - Push result: `Continue`, `LineComplete`, or `BufferFull`

The pattern matching syntax (`["cmd", arg1, arg2]`) is standard Rust slice patterns.

## Behavior Notes

- Line endings: `\r`, `\n`, and `\r\n` all terminate the line. The terminator is
  not stored in the buffer.
- Backspace: both `0x08` and `0x7F` remove one buffered byte if present.
- UTF-8: `LineBuffer::as_str()` returns `None` for invalid UTF-8, but
  `LineBuffer::as_bytes()` always exposes the raw bytes.
- Part limit: `Parts::<N>::parse()` keeps at most `N` whitespace-separated parts.
  If additional parts were present, `Parts::is_truncated()` returns `true`.

## Example

Run the host-side demo:

```text
cargo run --example host_repl
```

See [examples/host_repl.rs](examples/host_repl.rs) for a complete byte-by-byte REPL loop.
