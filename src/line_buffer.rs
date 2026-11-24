/// Longest UTF-8 encoding of a single character, in bytes.
const UTF8_MAX_LEN: usize = 4;

/// Result of pushing a byte into a [`LineBuffer`].
///
/// The variants tell the caller when to echo, redraw, or handle a complete line.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Byte appended to the line. Echo it.
    Stored,
    /// Backspace removed one buffered character. Redraw from the cursor saved
    /// after the prompt: restore it (`"\x1b8"`), erase to the end of the screen
    /// (`"\x1b[J"`), then write the buffer. This handles wide characters and
    /// wrapped lines; see the crate docs for how many rows to reserve. For
    /// single-column input that never wraps, `"\x08 \x08"` is enough.
    Erased,
    /// Byte consumed without changing the buffered bytes: an escape sequence,
    /// ignored control byte, `\n` after `\r`, a trailing byte of a dropped
    /// character, or a backspace that erases no buffered character. Do not
    /// echo it.
    Ignored,
    /// Line is complete, ready for parsing.
    LineComplete,
    /// Line ended with overflow still outstanding. Any buffered bytes remain
    /// available; dropped characters are gone.
    LineOverflow,
    /// A character did not fit. Reported on its first byte; ignore its
    /// remaining bytes. Do not echo it.
    BufferFull,
}

impl Outcome {
    /// Returns `true` when the pushed byte completed a line.
    ///
    /// Overlong lines report [`Outcome::LineOverflow`], which is not a
    /// completed line.
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
    Utf8Tail,
}

/// Accumulates bytes until a newline for line-by-line serial input.
///
/// Backspace (`0x08` or `0x7F`) removes one UTF-8 character. ANSI escape
/// sequences are consumed; control bytes other than tab and line endings are
/// ignored. A `\r\n` pair completes one line, and overlong lines are reported
/// instead of silently truncated.
///
/// A finished line stays readable until the next byte is pushed; that byte
/// starts a new line, so calling [`LineBuffer::clear`] between lines is
/// optional.
///
/// The default capacity applies in type annotations (`line: LineBuffer`).
/// Expressions need an explicit capacity: `LineBuffer::<128>::new()`.
#[derive(Debug, Clone)]
pub struct LineBuffer<const N: usize = 128> {
    buf: [u8; N],
    len: usize,
    prev_cr: bool,
    terminated: bool,
    dropped: usize,
    esc: EscState,
}

impl<const N: usize> LineBuffer<N> {
    /// Create an empty line buffer.
    pub const fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
            prev_cr: false,
            terminated: false,
            dropped: 0,
            esc: EscState::None,
        }
    }

    /// Push a byte into the buffer.
    ///
    /// Returns [`Outcome::LineComplete`] on `\r` or `\n` unless characters
    /// were dropped and not restored with backspace; then it returns
    /// [`Outcome::LineOverflow`]. In a CRLF pair, the `\n` returns
    /// [`Outcome::Ignored`].
    ///
    /// Returns [`Outcome::BufferFull`] on the first byte of each character
    /// that does not fit; its remaining bytes are ignored. A character is
    /// stored or dropped whole. Other bytes return [`Outcome::Stored`],
    /// [`Outcome::Erased`], or [`Outcome::Ignored`].
    ///
    /// After a terminator, the next byte starts a new line and discards the
    /// previous one, except for the `\n` in a `\r\n` pair.
    pub fn push(&mut self, byte: u8) -> Outcome {
        if self.terminated {
            let coalesced = self.prev_cr && byte == b'\n';
            self.prev_cr = false;
            if coalesced {
                return Outcome::Ignored;
            }
            self.terminated = false;
            self.len = 0;
            self.dropped = 0;
        }

        if self.consume_escape(byte) {
            return Outcome::Ignored;
        }

        match byte {
            b'\r' | b'\n' => {
                self.prev_cr = byte == b'\r';
                self.terminated = true;
                if self.dropped > 0 {
                    Outcome::LineOverflow
                } else {
                    Outcome::LineComplete
                }
            }
            0x1B => {
                self.esc = EscState::Esc;
                Outcome::Ignored
            }
            0x7F | 0x08 => self.erase(),
            b'\t' | 0x20..=0xFF => self.store(byte),
            _ => Outcome::Ignored,
        }
    }

    /// Consumes escape-sequence bytes. Returns `true` when `byte` belongs to
    /// an escape sequence and should not be processed further.
    fn consume_escape(&mut self, byte: u8) -> bool {
        // Control bytes (< 0x20) abort the sequence and fall through to
        // `push`, so a stray ESC cannot consume a newline.
        let (next, consumed): (EscState, bool) = match (self.esc, byte) {
            (EscState::Esc, b'[') => (EscState::Csi, true),
            (EscState::Esc, b'O') => (EscState::Ss3, true),
            // For Alt plus a non-ASCII character, consume its UTF-8 tail too.
            (EscState::Esc | EscState::Ss3, 0xC0..=0xFF) => (EscState::Utf8Tail, true),
            (EscState::Esc | EscState::Ss3, 0x20..=0xBF) => (EscState::None, true),
            (EscState::Csi, 0x20..=0x3F) => (EscState::Csi, true),
            (EscState::Csi, 0x40..=0x7E) => (EscState::None, true),
            (EscState::Utf8Tail, 0x80..=0xBF) => (EscState::Utf8Tail, true),
            _ => (EscState::None, false),
        };
        self.esc = next;
        consumed
    }

    fn store(&mut self, byte: u8) -> Outcome {
        let is_continuation = byte & 0xC0 == 0x80;
        if self.dropped > 0 {
            if is_continuation {
                return Outcome::Ignored;
            }
            self.dropped = self.dropped.saturating_add(1);
            return Outcome::BufferFull;
        }

        // Reserve space for the full character so overflow cannot split it.
        let needed: usize = match byte {
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => UTF8_MAX_LEN,
            _ => 1,
        };
        if N - self.len < needed {
            self.dropped = 1;
            return Outcome::BufferFull;
        }

        self.buf[self.len] = byte;
        self.len += 1;
        Outcome::Stored
    }

    fn erase(&mut self) -> Outcome {
        if self.dropped > 0 {
            self.dropped -= 1;
            return Outcome::Ignored;
        }
        if self.len == 0 {
            return Outcome::Ignored;
        }

        // Remove one whole UTF-8 character, not just one byte.
        for _ in 0..UTF8_MAX_LEN {
            self.len -= 1;
            if self.len == 0 || self.buf[self.len] & 0xC0 != 0x80 {
                break;
            }
        }
        Outcome::Erased
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

    /// Clear the buffered bytes. CRLF coalescing and overflow tracking remain
    /// active. During overflow, backspace must remove all dropped characters
    /// before the line can complete normally.
    ///
    /// After a completed line, calling this is optional; the next byte starts
    /// a new line. See [`LineBuffer::push`].
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
        assert_eq!(buf.push(b'h'), Outcome::Stored);
        assert_eq!(buf.push(b'i'), Outcome::Stored);
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
        assert_eq!(buf.push(b'\n'), Outcome::Ignored);
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
    fn backspace_reports_whether_a_char_was_erased() {
        let mut buf = LineBuffer::<64>::new();
        let _ = buf.push(b'a');
        assert_eq!(buf.push(0x7F), Outcome::Erased);
        assert_eq!(buf.push(0x7F), Outcome::Ignored);
        assert_eq!(buf.push(0x08), Outcome::Ignored);
    }

    #[test]
    fn buffer_full() {
        let mut buf = LineBuffer::<4>::new();
        assert_eq!(buf.push(b'a'), Outcome::Stored);
        assert_eq!(buf.push(b'b'), Outcome::Stored);
        assert_eq!(buf.push(b'c'), Outcome::Stored);
        assert_eq!(buf.push(b'd'), Outcome::Stored);
        assert_eq!(buf.push(b'e'), Outcome::BufferFull);
        assert_eq!(buf.as_str(), Some("abcd"));
        assert_eq!(buf.len(), 4);
        assert_eq!(buf.capacity(), 4);
    }

    #[test]
    fn zero_capacity_never_accepts_data() {
        let mut buf = LineBuffer::<0>::new();
        assert_eq!(buf.push(b'a'), Outcome::BufferFull);
        assert!(buf.is_empty());
        assert_eq!(buf.as_bytes(), b"");
        assert_eq!(buf.push(b'\n'), Outcome::LineOverflow);
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
        assert_eq!(buf.push(b'\n'), Outcome::Ignored);
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
        assert_eq!(buf.push(b'\n'), Outcome::Ignored);
    }

    #[test]
    fn data_after_cr_is_not_swallowed() {
        let mut buf = LineBuffer::<64>::new();
        assert_eq!(buf.push(b'\r'), Outcome::LineComplete);
        assert_eq!(buf.push(b'x'), Outcome::Stored);
        assert_eq!(buf.as_str(), Some("x"));
    }

    #[test]
    fn next_line_starts_fresh_without_clear() {
        let mut buf = LineBuffer::<64>::new();
        let _ = buf.push(b'a');
        assert_eq!(buf.push(b'\n'), Outcome::LineComplete);
        assert_eq!(buf.as_str(), Some("a"));
        assert_eq!(buf.push(b'b'), Outcome::Stored);
        assert_eq!(buf.as_str(), Some("b"));
    }

    #[test]
    fn line_stays_readable_after_coalesced_lf() {
        let mut buf = LineBuffer::<64>::new();
        let _ = buf.push(b'a');
        assert_eq!(buf.push(b'\r'), Outcome::LineComplete);
        assert_eq!(buf.push(b'\n'), Outcome::Ignored);
        assert_eq!(buf.as_str(), Some("a"));

        // A second LF starts a new, empty line.
        assert_eq!(buf.push(b'\n'), Outcome::LineComplete);
        assert_eq!(buf.as_str(), Some(""));
    }

    #[test]
    fn overlong_line_does_not_wedge_is_complete_callers() {
        let mut buf = LineBuffer::<4>::new();
        let mut completed = 0;
        for &b in b"abcdef\nhi\n" {
            if buf.push(b).is_complete() {
                completed += 1;
                assert_eq!(buf.as_str(), Some("hi"));
            }
        }
        assert_eq!(completed, 1);
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
    fn clone_copies_buffered_line() {
        let mut buf = LineBuffer::<64>::new();
        let _ = buf.push(b'a');
        let copy = buf.clone();
        let _ = buf.push(b'b');
        assert_eq!(copy.as_str(), Some("a"));
        assert_eq!(buf.as_str(), Some("ab"));
    }

    #[test]
    fn default_is_empty() {
        let buf = LineBuffer::<64>::default();
        assert!(buf.is_empty());
    }

    #[test]
    fn backspace_removes_whole_utf8_char() {
        let mut buf = LineBuffer::<64>::new();
        for b in "aé€".bytes() {
            let _ = buf.push(b);
        }
        assert_eq!(buf.push(0x7F), Outcome::Erased);
        assert_eq!(buf.as_str(), Some("aé"));
        let _ = buf.push(0x08);
        assert_eq!(buf.as_str(), Some("a"));
        let _ = buf.push(0x7F);
        assert_eq!(buf.as_str(), Some(""));
    }

    #[test]
    fn backspace_is_bounded_on_stray_continuation_bytes() {
        let mut buf = LineBuffer::<64>::new();
        for &b in b"ab\x80\x80\x80\x80\x80\x80" {
            let _ = buf.push(b);
        }
        assert_eq!(buf.push(0x7F), Outcome::Erased);
        assert_eq!(buf.as_bytes(), b"ab\x80\x80");
    }

    #[test]
    fn control_bytes_are_ignored() {
        let mut buf = LineBuffer::<64>::new();
        for &b in b"pi" {
            assert_eq!(buf.push(b), Outcome::Stored);
        }
        // These control bytes are ignored: Ctrl-C, NUL, and Ctrl-U.
        for &b in b"\x03\x00\x15" {
            assert_eq!(buf.push(b), Outcome::Ignored);
        }
        for &b in b"ng" {
            assert_eq!(buf.push(b), Outcome::Stored);
        }
        assert_eq!(buf.as_str(), Some("ping"));
    }

    #[test]
    fn tab_is_stored() {
        let mut buf = LineBuffer::<64>::new();
        let _ = buf.push(b'a');
        assert_eq!(buf.push(b'\t'), Outcome::Stored);
        let _ = buf.push(b'b');
        assert_eq!(buf.as_str(), Some("a\tb"));

        let _ = buf.push(0x7F);
        assert_eq!(buf.push(0x7F), Outcome::Erased);
        assert_eq!(buf.as_str(), Some("a"));
    }

    #[test]
    fn escape_sequences_are_consumed() {
        let mut buf = LineBuffer::<64>::new();
        // Cursor up: CSI final byte
        for &b in b"\x1b[A" {
            assert_eq!(buf.push(b), Outcome::Ignored);
        }
        assert!(buf.is_empty());

        // CSI with parameter bytes
        for &b in b"\x1b[1;5D" {
            assert_eq!(buf.push(b), Outcome::Ignored);
        }
        assert!(buf.is_empty());

        // SS3 (application cursor mode)
        for &b in b"\x1bOA" {
            assert_eq!(buf.push(b), Outcome::Ignored);
        }
        assert!(buf.is_empty());
    }

    #[test]
    fn alt_key_is_consumed() {
        let mut buf = LineBuffer::<64>::new();
        assert_eq!(buf.push(0x1B), Outcome::Ignored);
        assert_eq!(buf.push(b'x'), Outcome::Ignored);
        assert!(buf.is_empty());
    }

    #[test]
    fn alt_non_ascii_key_is_consumed() {
        let mut buf = LineBuffer::<64>::new();
        let _ = buf.push(b'a');
        for b in "\x1bé\x1b€\x1b😀".bytes() {
            assert_eq!(buf.push(b), Outcome::Ignored);
        }
        assert_eq!(buf.push(b'b'), Outcome::Stored);
        assert_eq!(buf.as_str(), Some("ab"));
    }

    #[test]
    fn ss3_non_ascii_byte_is_consumed() {
        let mut buf = LineBuffer::<64>::new();
        for b in "\x1bOé".bytes() {
            assert_eq!(buf.push(b), Outcome::Ignored);
        }
        assert_eq!(buf.push(b'a'), Outcome::Stored);
        assert_eq!(buf.as_str(), Some("a"));
    }

    #[test]
    fn backspace_aborts_csi_sequence() {
        let mut buf = LineBuffer::<64>::new();
        let _ = buf.push(b'a');
        assert_eq!(buf.push(0x1B), Outcome::Ignored);
        assert_eq!(buf.push(b'['), Outcome::Ignored);
        assert_eq!(buf.push(0x7F), Outcome::Erased);
        assert!(buf.is_empty());
    }

    #[test]
    fn newline_aborts_escape_sequence() {
        let mut buf = LineBuffer::<64>::new();
        let _ = buf.push(b'a');
        assert_eq!(buf.push(0x1B), Outcome::Ignored);
        assert_eq!(buf.push(b'\r'), Outcome::LineComplete);
        assert_eq!(buf.as_str(), Some("a"));

        let mut buf = LineBuffer::<64>::new();
        assert_eq!(buf.push(0x1B), Outcome::Ignored);
        assert_eq!(buf.push(b'['), Outcome::Ignored);
        assert_eq!(buf.push(b'\n'), Outcome::LineComplete);

        let mut buf = LineBuffer::<64>::new();
        assert_eq!(buf.push(0x1B), Outcome::Ignored);
        assert_eq!(buf.push(b'O'), Outcome::Ignored);
        assert_eq!(buf.push(b'\r'), Outcome::LineComplete);
    }

    #[test]
    fn overlong_line_is_discarded_until_terminator() {
        let mut buf = LineBuffer::<4>::new();
        for &b in b"abcd" {
            assert_eq!(buf.push(b), Outcome::Stored);
        }
        assert_eq!(buf.push(b'e'), Outcome::BufferFull);
        assert_eq!(buf.push(b'f'), Outcome::BufferFull);
        assert_eq!(buf.push(b'\n'), Outcome::LineOverflow);
        assert_eq!(buf.as_str(), Some("abcd"));

        // next line is processed normally again
        buf.clear();
        assert_eq!(buf.push(b'x'), Outcome::Stored);
        assert_eq!(buf.push(b'\n'), Outcome::LineComplete);
        assert_eq!(buf.as_str(), Some("x"));
    }

    #[test]
    fn overflowed_crlf_is_coalesced() {
        let mut buf = LineBuffer::<1>::new();
        let _ = buf.push(b'a');
        assert_eq!(buf.push(b'b'), Outcome::BufferFull);
        assert_eq!(buf.push(b'\r'), Outcome::LineOverflow);
        assert_eq!(buf.push(b'\n'), Outcome::Ignored);
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
    fn backspace_after_clear_takes_dropped_chars_back() {
        let mut buf = LineBuffer::<1>::new();
        let _ = buf.push(b'a');
        assert_eq!(buf.push(b'b'), Outcome::BufferFull);
        buf.clear();
        assert_eq!(buf.push(0x7F), Outcome::Ignored);
        assert_eq!(buf.push(b'\n'), Outcome::LineComplete);
        assert_eq!(buf.as_str(), Some(""));
    }

    #[test]
    fn backspace_recovers_from_overflow() {
        let mut buf = LineBuffer::<4>::new();
        for &b in b"abcdef" {
            let _ = buf.push(b);
        }
        // Backspace removes the two dropped characters before the stored one.
        assert_eq!(buf.push(0x7F), Outcome::Ignored);
        assert_eq!(buf.push(0x7F), Outcome::Ignored);
        assert_eq!(buf.push(0x7F), Outcome::Erased);
        assert_eq!(buf.push(b'x'), Outcome::Stored);
        assert_eq!(buf.push(b'\n'), Outcome::LineComplete);
        assert_eq!(buf.as_str(), Some("abcx"));
    }

    #[test]
    fn partial_backspace_keeps_line_overflowed() {
        let mut buf = LineBuffer::<4>::new();
        for &b in b"abcdef" {
            let _ = buf.push(b);
        }
        assert_eq!(buf.push(0x7F), Outcome::Ignored);
        assert_eq!(buf.push(b'\n'), Outcome::LineOverflow);
        assert_eq!(buf.as_str(), Some("abcd"));
    }

    #[test]
    fn overflow_never_splits_utf8_char() {
        let mut buf = LineBuffer::<4>::new();
        for &b in b"abc" {
            assert_eq!(buf.push(b), Outcome::Stored);
        }
        assert_eq!(buf.push(0xC3), Outcome::BufferFull);
        assert_eq!(buf.push(0xA9), Outcome::Ignored);
        assert_eq!(buf.push(b'\n'), Outcome::LineOverflow);
        assert_eq!(buf.as_str(), Some("abc"));
    }

    #[test]
    fn dropped_char_reports_buffer_full_once() {
        let mut buf = LineBuffer::<4>::new();
        for &b in b"abc" {
            let _ = buf.push(b);
        }
        for c in ["é", "€", "😀"] {
            let mut bytes = c.bytes();
            assert_eq!(bytes.next().map(|b| buf.push(b)), Some(Outcome::BufferFull));
            for b in bytes {
                assert_eq!(buf.push(b), Outcome::Ignored);
            }
        }
        assert_eq!(buf.push(b'x'), Outcome::BufferFull);
        assert_eq!(buf.push(b'\n'), Outcome::LineOverflow);
    }

    #[test]
    fn dropped_multibyte_char_is_taken_back_by_one_backspace() {
        let mut buf = LineBuffer::<4>::new();
        for b in "abc€".bytes() {
            let _ = buf.push(b);
        }
        assert_eq!(buf.push(0x7F), Outcome::Ignored);
        assert_eq!(buf.push(b'\n'), Outcome::LineComplete);
        assert_eq!(buf.as_str(), Some("abc"));
    }

    #[test]
    fn escape_sequence_during_overflow_is_ignored() {
        let mut buf = LineBuffer::<1>::new();
        let _ = buf.push(b'a');
        assert_eq!(buf.push(b'b'), Outcome::BufferFull);
        for &b in b"\x1b[A" {
            assert_eq!(buf.push(b), Outcome::Ignored);
        }
        assert_eq!(buf.push(b'\n'), Outcome::LineOverflow);
    }
}
