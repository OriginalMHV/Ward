use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

/// Everything except the RFC 3986 unreserved characters (`A-Z a-z 0-9 - . _ ~`).
const NOT_UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Percent-encode a single path segment (e.g. an environment name), which may
/// legally contain characters such as `/`, spaces, or other symbols that must
/// not be interpreted as path separators or otherwise misparsed by the API.
///
/// Every non-alphanumeric byte is encoded, including `-`, `.`, `_` and `~`.
pub(crate) fn encode_path_segment(segment: &str) -> String {
    utf8_percent_encode(segment, NON_ALPHANUMERIC).to_string()
}

/// Percent-encode a branch name or user-supplied segment, leaving the RFC 3986
/// unreserved characters (`A-Z a-z 0-9 - . _ ~`) as they are.
pub(crate) fn encode_unreserved(segment: &str) -> String {
    utf8_percent_encode(segment, NOT_UNRESERVED).to_string()
}

#[cfg(test)]
mod tests {
    use super::{encode_path_segment, encode_unreserved};

    #[test]
    fn path_segment_encoding_keeps_only_alphanumerics() {
        assert_eq!(
            encode_path_segment("staging/eu west-1_a.b~c"),
            "staging%2Feu%20west%2D1%5Fa%2Eb%7Ec"
        );
        assert_eq!(encode_path_segment("ü☃"), "%C3%BC%E2%98%83");
    }

    #[test]
    fn unreserved_encoding_keeps_unreserved_characters() {
        assert_eq!(
            encode_unreserved("feature/ü branch-1_a.b~c"),
            "feature%2F%C3%BC%20branch-1_a.b~c"
        );
        assert_eq!(encode_unreserved("a+b%c#d?e"), "a%2Bb%25c%23d%3Fe");
    }
}
