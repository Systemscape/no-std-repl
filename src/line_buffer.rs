/// Result of pushing a byte into a [`LineBuffer`].
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Byte consumed, keep reading.
    Continue,
    /// Line is complete, ready for parsing.
    LineComplete,
    /// Buffer at capacity, byte was dropped.
    BufferFull,
}

impl Outcome {
    /// Returns `true` when the pushed byte completed a line.
    pub fn is_complete(self) -> bool {
        matches!(self, Self::LineComplete)
    }
}

/// Accumulates bytes until a newline, suitable for line-by-line serial input.
///
/// Handles backspace (0x7F and 0x08), ignores bytes when buffer is full,
/// and coalesces `\r\n` into a single line-complete event.
pub struct LineBuffer<const N: usize = 128> {
    buf: [u8; N],
    len: usize,
    prev_cr: bool,
}

impl<const N: usize> LineBuffer<N> {
    /// Create an empty line buffer.
    pub const fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
            prev_cr: false,
        }
    }

    /// Push a byte into the buffer.
    ///
    /// Returns [`Outcome::LineComplete`] when a newline is received (`\r`, `\n`,
    /// or `\r\n` — the sequence is coalesced into one event).
    /// Returns [`Outcome::BufferFull`] if the byte was dropped due to capacity.
    /// Returns [`Outcome::Continue`] otherwise.
    pub fn push(&mut self, byte: u8) -> Outcome {
        match byte {
            b'\n' if self.prev_cr => {
                self.prev_cr = false;
                Outcome::Continue
            }
            b'\r' => {
                self.prev_cr = true;
                Outcome::LineComplete
            }
            b'\n' => Outcome::LineComplete,
            0x7F | 0x08 => {
                self.prev_cr = false;
                self.len = self.len.saturating_sub(1);
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

    /// Clear the buffered line and reset newline coalescing state.
    pub fn clear(&mut self) {
        self.len = 0;
        self.prev_cr = false;
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
    fn clear_resets_pending_cr_state() {
        let mut buf = LineBuffer::<64>::new();
        assert_eq!(buf.push(b'\r'), Outcome::LineComplete);
        buf.clear();
        assert_eq!(buf.push(b'\n'), Outcome::LineComplete);
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
}
