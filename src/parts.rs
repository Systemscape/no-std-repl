/// Whitespace-separated command parts, enabling slice pattern matching.
///
/// # Example
///
/// ```
/// use no_std_repl::Parts;
///
/// let parts = Parts::<8>::parse("ping 42");
/// match parts.as_slice() {
///     ["ping"] => println!("ping with no args"),
///     ["ping", n] => println!("ping with arg: {n}"),
///     _ => println!("unknown"),
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parts<'a, const N: usize = 8> {
    parts: [&'a str; N],
    count: usize,
    truncated: bool,
}

impl<'a, const N: usize> Parts<'a, N> {
    /// Parse a line into whitespace-separated parts.
    ///
    /// At most `N` parts are captured; any additional parts are ignored and
    /// reported via [`Parts::is_truncated`].
    pub fn parse(line: &'a str) -> Self {
        let mut parts = [""; N];
        let mut count = 0;
        let mut truncated = false;

        for part in line.split_whitespace() {
            if count >= N {
                truncated = true;
                break;
            }
            parts[count] = part;
            count += 1;
        }

        Self {
            parts,
            count,
            truncated,
        }
    }

    /// The captured parts as a slice for pattern matching.
    pub fn as_slice(&self) -> &[&'a str] {
        &self.parts[..self.count]
    }

    /// Number of captured parts.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Returns `true` when no parts were captured.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Returns `true` when at least one trailing part was discarded.
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_line() {
        let parts = Parts::<8>::parse("");
        assert!(parts.is_empty());
        assert!(!parts.is_truncated());
        assert_eq!(parts.as_slice(), &[] as &[&str]);
    }

    #[test]
    fn whitespace_only() {
        let parts = Parts::<8>::parse("   \t  ");
        assert!(parts.is_empty());
        assert!(!parts.is_truncated());
    }

    #[test]
    fn single_word() {
        let parts = Parts::<8>::parse("ping");
        assert_eq!(parts.as_slice(), &["ping"]);
        assert!(!parts.is_truncated());
    }

    #[test]
    fn multiple_words() {
        let parts = Parts::<8>::parse("ping 42 hello");
        assert_eq!(parts.as_slice(), &["ping", "42", "hello"]);
        assert!(!parts.is_truncated());
    }

    #[test]
    fn extra_whitespace() {
        let parts = Parts::<8>::parse("  ping   42  ");
        assert_eq!(parts.as_slice(), &["ping", "42"]);
        assert!(!parts.is_truncated());
    }

    #[test]
    fn max_parts_exceeded() {
        let parts = Parts::<2>::parse("a b c d e");
        assert_eq!(parts.as_slice(), &["a", "b"]);
        assert_eq!(parts.len(), 2);
        assert!(parts.is_truncated());
    }

    #[test]
    fn zero_capacity_is_truncated_for_non_empty_input() {
        let parts = Parts::<0>::parse("ping 42");
        assert!(parts.is_empty());
        assert!(parts.is_truncated());
    }

    #[test]
    fn pattern_matching() {
        let parts = Parts::<8>::parse("set key value");
        match parts.as_slice() {
            ["set", k, v] => {
                assert_eq!(*k, "key");
                assert_eq!(*v, "value");
            }
            _ => panic!("pattern should match"),
        }
    }
}
