/// Result of pushing a byte into a [`LineBuffer`].
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Byte consumed, keep reading.
    Continue,
    /// Line is complete, ready for parsing.
    LineComplete,
    /// Line terminator received for an overlong line. The buffer holds the
    /// truncated prefix; the dropped bytes are gone.
    LineOverflow,
    /// Buffer at capacity, byte was dropped.
    BufferFull,
}

impl Outcome {
    /// Returns `true` when the pushed byte completed a line.
    pub fn is_complete(self) -> bool {
        matches!(self, Self::LineComplete)
    }
}

/// State of the ANSI escape sequence consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EscState {
    None,
    Esc,
    Csi,
    Ss3,
}

/// Accumulates bytes until a newline, suitable for line-by-line serial input.
///
/// Handles backspace (0x7F and 0x08) at UTF-8 character granularity, consumes
/// ANSI escape sequences (arrow keys etc.) without storing them, coalesces
/// `\r\n` into a single line-complete event, and discards the remainder of
/// overlong lines until their terminator arrives.
pub struct LineBuffer<const N: usize = 128> {
    buf: [u8; N],
    len: usize,
    prev_cr: bool,
    overflowed: bool,
    esc: EscState,
}

impl<const N: usize> LineBuffer<N> {
    /// Create an empty line buffer.
    pub const fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
            prev_cr: false,
            overflowed: false,
            esc: EscState::None,
        }
    }

    /// Push a byte into the buffer.
    ///
    /// Returns [`Outcome::LineComplete`] when a newline is received (`\r`, `\n`,
    /// or `\r\n` — the sequence is coalesced into one event), or
    /// [`Outcome::LineOverflow`] instead if bytes of the line were dropped
    /// due to capacity.
    /// Returns [`Outcome::BufferFull`] if the byte was dropped due to capacity.
    /// Returns [`Outcome::Continue`] otherwise.
    pub fn push(&mut self, byte: u8) -> Outcome {
        // Control bytes (< 0x20) abort an escape sequence and are processed
        // normally below, so a newline is never swallowed by a stray ESC.
        match self.esc {
            EscState::Esc => match byte {
                b'[' => {
                    self.esc = EscState::Csi;
                    return Outcome::Continue;
                }
                b'O' => {
                    self.esc = EscState::Ss3;
                    return Outcome::Continue;
                }
                0x20..=0xFF => {
                    self.esc = EscState::None;
                    return Outcome::Continue;
                }
                _ => self.esc = EscState::None,
            },
            EscState::Csi => match byte {
                0x20..=0x3F => return Outcome::Continue,
                0x40..=0x7E => {
                    self.esc = EscState::None;
                    return Outcome::Continue;
                }
                _ => self.esc = EscState::None,
            },
            EscState::Ss3 => match byte {
                0x20..=0xFF => {
                    self.esc = EscState::None;
                    return Outcome::Continue;
                }
                _ => self.esc = EscState::None,
            },
            EscState::None => {}
        }

        match byte {
            b'\n' if self.prev_cr => {
                self.prev_cr = false;
                Outcome::Continue
            }
            b'\r' | b'\n' => {
                self.prev_cr = byte == b'\r';
                if self.overflowed {
                    self.overflowed = false;
                    Outcome::LineOverflow
                } else {
                    Outcome::LineComplete
                }
            }
            0x1B => {
                self.prev_cr = false;
                self.esc = EscState::Esc;
                Outcome::Continue
            }
            _ if self.overflowed => {
                self.prev_cr = false;
                Outcome::BufferFull
            }
            0x7F | 0x08 => {
                self.prev_cr = false;
                // Remove one whole UTF-8 character, not just one byte.
                while self.len > 0 {
                    self.len -= 1;
                    if self.buf[self.len] & 0xC0 != 0x80 {
                        break;
                    }
                }
                Outcome::Continue
            }
            _ if self.len < N => {
                self.prev_cr = false;
                self.buf[self.len] = byte;
                self.len += 1;
                Outcome::Continue
            }
            _ => {
                self.prev_cr = false;
                self.overflowed = true;
                Outcome::BufferFull
            }
        }
    }

    /// Current buffer contents as a string slice, or `None` if not valid UTF-8.
    ///
    /// Line terminators are never included in the returned slice.
    pub fn as_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.buf[..self.len]).ok()
    }

    /// Current buffer contents as raw bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    /// Clear the buffered line. Newline coalescing and overflow state are
    /// preserved: a `\n` immediately following a `\r` is still ignored, and
    /// an overflowed line keeps discarding bytes until its terminator
    /// arrives, even if `clear()` is called in between.
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Returns `true` when the buffer currently holds no bytes.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Current number of buffered bytes.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Maximum number of bytes the buffer can hold.
    pub const fn capacity(&self) -> usize {
        N
    }
}

impl<const N: usize> Default for LineBuffer<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_buffer() {
        let buf = LineBuffer::<64>::new();
        assert!(buf.is_empty());
        assert_eq!(buf.as_str(), Some(""));
    }

    #[test]
    fn push_chars() {
        let mut buf = LineBuffer::<64>::new();
        assert_eq!(buf.push(b'h'), Outcome::Continue);
        assert_eq!(buf.push(b'i'), Outcome::Continue);
        assert_eq!(buf.as_str(), Some("hi"));
        assert_eq!(buf.len(), 2);
    }

    #[test]
    fn newline_lf() {
        let mut buf = LineBuffer::<64>::new();
        let _ = buf.push(b'a');
        assert_eq!(buf.push(b'\n'), Outcome::LineComplete);
    }

    #[test]
    fn newline_cr() {
        let mut buf = LineBuffer::<64>::new();
        let _ = buf.push(b'a');
        assert_eq!(buf.push(b'\r'), Outcome::LineComplete);
    }

    #[test]
    fn crlf_coalesced() {
        let mut buf = LineBuffer::<64>::new();
        let _ = buf.push(b'a');
        assert_eq!(buf.push(b'\r'), Outcome::LineComplete);
        assert_eq!(buf.push(b'\n'), Outcome::Continue);
    }

    #[test]
    fn bare_lf_after_normal_byte() {
        let mut buf = LineBuffer::<64>::new();
        let _ = buf.push(b'a');
        buf.clear();
        assert_eq!(buf.push(b'\n'), Outcome::LineComplete);
    }

    #[test]
    fn backspace() {
        let mut buf = LineBuffer::<64>::new();
        let _ = buf.push(b'a');
        let _ = buf.push(b'b');
        let _ = buf.push(0x7F);
        assert_eq!(buf.as_str(), Some("a"));

        let _ = buf.push(0x08);
        assert_eq!(buf.as_str(), Some(""));

        // backspace on empty buffer is harmless
        let _ = buf.push(0x7F);
        assert!(buf.is_empty());
    }

    #[test]
    fn buffer_full() {
        let mut buf = LineBuffer::<4>::new();
        assert_eq!(buf.push(b'a'), Outcome::Continue);
        assert_eq!(buf.push(b'b'), Outcome::Continue);
        assert_eq!(buf.push(b'c'), Outcome::Continue);
        assert_eq!(buf.push(b'd'), Outcome::Continue);
        assert_eq!(buf.push(b'e'), Outcome::BufferFull);
        assert_eq!(buf.as_str(), Some("abcd"));
        assert_eq!(buf.len(), 4);
    }

    #[test]
    fn zero_capacity_never_accepts_data() {
        let mut buf = LineBuffer::<0>::new();
        assert_eq!(buf.push(b'a'), Outcome::BufferFull);
        assert!(buf.is_empty());
        assert_eq!(buf.as_bytes(), b"");
    }

    #[test]
    fn clear() {
        let mut buf = LineBuffer::<64>::new();
        let _ = buf.push(b'x');
        buf.clear();
        assert!(buf.is_empty());
    }

    #[test]
    fn clear_preserves_pending_cr_state() {
        let mut buf = LineBuffer::<64>::new();
        assert_eq!(buf.push(b'\r'), Outcome::LineComplete);
        buf.clear();
        assert_eq!(buf.push(b'\n'), Outcome::Continue);
    }

    #[test]
    fn repeated_carriage_returns_each_complete_a_line() {
        let mut buf = LineBuffer::<64>::new();
        assert_eq!(buf.push(b'\r'), Outcome::LineComplete);
        assert_eq!(buf.push(b'\r'), Outcome::LineComplete);
    }

    #[test]
    fn lf_cr_are_two_line_completions() {
        let mut buf = LineBuffer::<64>::new();
        assert_eq!(buf.push(b'\n'), Outcome::LineComplete);
        assert_eq!(buf.push(b'\r'), Outcome::LineComplete);
        assert_eq!(buf.push(b'\n'), Outcome::Continue);
    }

    #[test]
    fn data_after_cr_is_not_swallowed() {
        let mut buf = LineBuffer::<64>::new();
        assert_eq!(buf.push(b'\r'), Outcome::LineComplete);
        assert_eq!(buf.push(b'x'), Outcome::Continue);
        assert_eq!(buf.as_str(), Some("x"));
    }

    #[test]
    fn as_bytes() {
        let mut buf = LineBuffer::<64>::new();
        let _ = buf.push(b'h');
        let _ = buf.push(b'i');
        assert_eq!(buf.as_bytes(), b"hi");
    }

    #[test]
    fn invalid_utf8_returns_none() {
        let mut buf = LineBuffer::<64>::new();
        let _ = buf.push(0xFF);
        assert_eq!(buf.as_str(), None);
        assert_eq!(buf.as_bytes(), &[0xFF]);
    }

    #[test]
    fn backspace_removes_whole_utf8_char() {
        let mut buf = LineBuffer::<64>::new();
        for b in "aé€".bytes() {
            let _ = buf.push(b);
        }
        let _ = buf.push(0x7F);
        assert_eq!(buf.as_str(), Some("aé"));
        let _ = buf.push(0x08);
        assert_eq!(buf.as_str(), Some("a"));
        let _ = buf.push(0x7F);
        assert_eq!(buf.as_str(), Some(""));
    }

    #[test]
    fn escape_sequences_are_consumed() {
        let mut buf = LineBuffer::<64>::new();
        // Cursor up: CSI final byte
        for &b in b"\x1b[A" {
            assert_eq!(buf.push(b), Outcome::Continue);
        }
        assert!(buf.is_empty());

        // CSI with parameter bytes
        for &b in b"\x1b[1;5D" {
            assert_eq!(buf.push(b), Outcome::Continue);
        }
        assert!(buf.is_empty());

        // SS3 (application cursor mode)
        for &b in b"\x1bOA" {
            assert_eq!(buf.push(b), Outcome::Continue);
        }
        assert!(buf.is_empty());
    }

    #[test]
    fn alt_key_is_consumed() {
        let mut buf = LineBuffer::<64>::new();
        assert_eq!(buf.push(0x1B), Outcome::Continue);
        assert_eq!(buf.push(b'x'), Outcome::Continue);
        assert!(buf.is_empty());
    }

    #[test]
    fn newline_aborts_escape_sequence() {
        let mut buf = LineBuffer::<64>::new();
        let _ = buf.push(b'a');
        assert_eq!(buf.push(0x1B), Outcome::Continue);
        assert_eq!(buf.push(b'\r'), Outcome::LineComplete);
        assert_eq!(buf.as_str(), Some("a"));

        let mut buf = LineBuffer::<64>::new();
        assert_eq!(buf.push(0x1B), Outcome::Continue);
        assert_eq!(buf.push(b'['), Outcome::Continue);
        assert_eq!(buf.push(b'\n'), Outcome::LineComplete);
    }

    #[test]
    fn overlong_line_is_discarded_until_terminator() {
        let mut buf = LineBuffer::<4>::new();
        for &b in b"abcd" {
            assert_eq!(buf.push(b), Outcome::Continue);
        }
        assert_eq!(buf.push(b'e'), Outcome::BufferFull);
        assert_eq!(buf.push(b'f'), Outcome::BufferFull);
        assert_eq!(buf.push(b'\n'), Outcome::LineOverflow);
        assert_eq!(buf.as_str(), Some("abcd"));

        // next line is processed normally again
        buf.clear();
        assert_eq!(buf.push(b'x'), Outcome::Continue);
        assert_eq!(buf.push(b'\n'), Outcome::LineComplete);
        assert_eq!(buf.as_str(), Some("x"));
    }

    #[test]
    fn overflowed_crlf_is_coalesced() {
        let mut buf = LineBuffer::<1>::new();
        let _ = buf.push(b'a');
        assert_eq!(buf.push(b'b'), Outcome::BufferFull);
        assert_eq!(buf.push(b'\r'), Outcome::LineOverflow);
        assert_eq!(buf.push(b'\n'), Outcome::Continue);
    }

    #[test]
    fn clear_preserves_overflow_state() {
        let mut buf = LineBuffer::<1>::new();
        let _ = buf.push(b'a');
        assert_eq!(buf.push(b'b'), Outcome::BufferFull);
        buf.clear();
        assert_eq!(buf.push(b'c'), Outcome::BufferFull);
        assert_eq!(buf.push(b'\n'), Outcome::LineOverflow);
    }

    #[test]
    fn backspace_during_overflow_is_ignored() {
        let mut buf = LineBuffer::<1>::new();
        let _ = buf.push(b'a');
        assert_eq!(buf.push(b'b'), Outcome::BufferFull);
        assert_eq!(buf.push(0x7F), Outcome::BufferFull);
        assert_eq!(buf.as_str(), Some("a"));
    }
}
