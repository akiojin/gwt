//! OSC 7 (current working directory) parser for terminal output.
//!
//! Shells like zsh/bash emit OSC 7 sequences to report the current working directory:
//!   ESC ] 7 ; file://hostname/path BEL
//!   ESC ] 7 ; file://hostname/path ESC \

/// Extract the current working directory from an OSC 7 escape sequence in a byte buffer.
///
/// Returns `Some(path)` if an OSC 7 sequence is found, `None` otherwise.
/// URL-encoded characters (e.g., `%20`) are decoded.
pub fn extract_osc7_cwd(buf: &[u8]) -> Option<String> {
    extract_osc7_cwd_with_consumed(buf).map(|(cwd, _)| cwd)
}

/// Extract the current working directory and consumed bytes from an OSC 7 escape sequence.
///
/// The returned `usize` is the byte count to consume from the beginning of `buf`
/// through the OSC 7 terminator.
pub fn extract_osc7_cwd_with_consumed(buf: &[u8]) -> Option<(String, usize)> {
    // Find ESC ] 7 ; (0x1b 0x5d 0x37 0x3b)
    let marker = b"\x1b]7;";
    let start = buf.windows(marker.len()).position(|w| w == marker)?;
    let after_marker = start + marker.len();
    if after_marker >= buf.len() {
        return None;
    }

    // Find the terminator: BEL (0x07) or ESC \ (0x1b 0x5c)
    let payload = &buf[after_marker..];
    let (end, terminator_len) = find_terminator(payload)?;
    let uri = &payload[..end];

    // Parse the file:// URI
    let uri_str = std::str::from_utf8(uri).ok()?;
    let path_part = strip_file_uri(uri_str)?;

    Some((url_decode(path_part), after_marker + end + terminator_len))
}

/// Find the position of the OSC terminator (BEL or ST).
fn find_terminator(buf: &[u8]) -> Option<(usize, usize)> {
    for i in 0..buf.len() {
        if buf[i] == 0x07 {
            return Some((i, 1));
        }
        if buf[i] == 0x1b && i + 1 < buf.len() && buf[i + 1] == b'\\' {
            return Some((i, 2));
        }
    }
    None
}

/// Strip the `file://[hostname]/path` prefix and return the path portion.
fn strip_file_uri(uri: &str) -> Option<&str> {
    let rest = uri.strip_prefix("file://")?;
    // The hostname may be empty (file:///path) or present (file://host/path).
    // In either case, the path starts at the first '/' after the hostname.
    if rest.starts_with('/') {
        // file:///path — hostname is empty
        Some(rest)
    } else {
        // file://hostname/path — skip hostname
        rest.find('/').map(|i| &rest[i..])
    }
}

/// Decode URL-encoded characters (%XX).
fn url_decode(s: &str) -> String {
    let mut decoded = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    decoded.push(byte);
                    i += 3;
                    continue;
                }
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    // T019-1: BEL terminated normal parse
    #[test]
    fn test_bel_terminated() {
        let buf = b"\x1b]7;file://localhost/Users/test\x07";
        let result = extract_osc7_cwd(buf);
        assert_eq!(result, Some("/Users/test".to_string()));
    }

    // T019-2: ESC \ terminated normal parse
    #[test]
    fn test_esc_backslash_terminated() {
        let buf = b"\x1b]7;file://localhost/Users/test\x1b\\";
        let result = extract_osc7_cwd(buf);
        assert_eq!(result, Some("/Users/test".to_string()));
    }

    // T019-3: hostname omitted (file:///path)
    #[test]
    fn test_hostname_omitted() {
        let buf = b"\x1b]7;file:///home/user/project\x07";
        let result = extract_osc7_cwd(buf);
        assert_eq!(result, Some("/home/user/project".to_string()));
    }

    // T019-4: URL encoded characters (%20 -> space)
    #[test]
    fn test_url_encoded() {
        let buf = b"\x1b]7;file:///home/user/my%20project\x07";
        let result = extract_osc7_cwd(buf);
        assert_eq!(result, Some("/home/user/my project".to_string()));
    }

    #[test]
    fn test_url_encoded_utf8_multibyte() {
        let buf = b"\x1b]7;file:///home/user/%E3%83%86%E3%82%B9%E3%83%88\x07";
        let result = extract_osc7_cwd(buf);
        assert_eq!(
            result,
            Some("/home/user/\u{30c6}\u{30b9}\u{30c8}".to_string())
        );
    }

    // T019-5: invalid input (no file://) returns None
    #[test]
    fn test_invalid_no_file_prefix() {
        let buf = b"\x1b]7;/Users/test\x07";
        let result = extract_osc7_cwd(buf);
        assert_eq!(result, None);
    }

    // T019-6: empty buffer returns None
    #[test]
    fn test_empty_buffer() {
        let buf: &[u8] = b"";
        let result = extract_osc7_cwd(buf);
        assert_eq!(result, None);
    }

    #[test]
    fn test_osc7_embedded_in_larger_output() {
        let buf = b"some output\x1b]7;file://host/tmp/dir\x07more output";
        let result = extract_osc7_cwd(buf);
        assert_eq!(result, Some("/tmp/dir".to_string()));
    }

    #[test]
    fn test_extract_with_consumed_returns_sequence_end() {
        let buf = b"prefix\x1b]7;file://host/tmp/dir\x07suffix";
        let result = extract_osc7_cwd_with_consumed(buf);
        assert_eq!(result, Some(("/tmp/dir".to_string(), 30)));
    }

    #[test]
    fn test_no_terminator_returns_none() {
        let buf = b"\x1b]7;file:///home/user";
        let result = extract_osc7_cwd(buf);
        assert_eq!(result, None);
    }
}
