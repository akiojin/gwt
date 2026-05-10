//! Ring-buffer scrollback storage for terminal lines.
//!
//! Stores up to `capacity` lines (default 10,000) per pane.
//! Memory-efficient: each entry is text + basic attributes.

/// A single stored line with text content and optional attributes.
///
/// `formatted` carries the SGR-bearing escape stream (`\x1b[...m...`) that
/// reproduces the original cell foreground / background color, bold, italic,
/// underline, and inverse attributes when written verbatim into a terminal
/// emulator (e.g. xterm.js). `text` keeps the plain text rendering for
/// search / contents export paths that do not understand escape codes.
/// SPEC-1919 FR-003j / SC-005b require both representations so scrollback
/// replay does not collapse colored history into default-color text.
#[derive(Debug, Clone, Default)]
pub struct ScrollbackLine {
    /// Plain text content of the line (formatting stripped).
    pub text: String,
    /// SGR-bearing byte stream suitable for replay into a terminal emulator.
    /// Empty when no formatting information is available.
    pub formatted: Vec<u8>,
    /// Whether the line was wrapped (soft wrap) vs. explicit newline.
    pub wrapped: bool,
}

/// Ring-buffer scrollback storage.
///
/// Stores the most recent `capacity` lines, discarding oldest when full.
pub struct ScrollbackStorage {
    lines: Vec<Option<ScrollbackLine>>,
    capacity: usize,
    /// Index where the next line will be written.
    head: usize,
    /// Total number of lines stored (capped at capacity).
    len: usize,
}

impl ScrollbackStorage {
    /// Default maximum lines per pane.
    pub const DEFAULT_CAPACITY: usize = 10_000;

    /// Create a new scrollback storage with the given capacity.
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        let mut lines = Vec::with_capacity(capacity);
        lines.resize_with(capacity, || None);
        Self {
            lines,
            capacity,
            head: 0,
            len: 0,
        }
    }

    /// Push a line into the ring buffer, evicting the oldest if full.
    pub fn push_line(&mut self, line: ScrollbackLine) {
        self.lines[self.head] = Some(line);
        self.head = (self.head + 1) % self.capacity;
        if self.len < self.capacity {
            self.len += 1;
        }
    }

    /// Get `count` lines starting from logical index `start` (0 = oldest stored line).
    ///
    /// Returns fewer lines if the range extends beyond what is stored.
    pub fn get_lines(&self, start: usize, count: usize) -> Vec<&ScrollbackLine> {
        if start >= self.len || count == 0 {
            return Vec::new();
        }
        let actual_count = count.min(self.len - start);
        let mut result = Vec::with_capacity(actual_count);

        // The oldest stored line is at physical index:
        //   if len < capacity: 0
        //   else: head (because head points to the next write slot, which is the oldest)
        let oldest_physical = if self.len < self.capacity {
            0
        } else {
            self.head
        };

        for i in 0..actual_count {
            let logical = start + i;
            let physical = (oldest_physical + logical) % self.capacity;
            if let Some(ref line) = self.lines[physical] {
                result.push(line);
            }
        }
        result
    }

    /// Number of lines currently stored.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the storage is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The maximum capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Clear all stored lines.
    pub fn clear(&mut self) {
        for slot in &mut self.lines {
            *slot = None;
        }
        self.head = 0;
        self.len = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_storage_is_empty() {
        let s = ScrollbackStorage::new(100);
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
        assert_eq!(s.capacity(), 100);
    }

    #[test]
    fn test_push_and_get_single_line() {
        let mut s = ScrollbackStorage::new(10);
        s.push_line(ScrollbackLine {
            text: "hello".to_string(),
            wrapped: false,
            ..Default::default()
        });
        assert_eq!(s.len(), 1);
        let lines = s.get_lines(0, 1);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "hello");
        assert!(!lines[0].wrapped);
    }

    #[test]
    fn test_push_multiple_and_read_all() {
        let mut s = ScrollbackStorage::new(10);
        for i in 0..5 {
            s.push_line(ScrollbackLine {
                text: format!("line-{i}"),
                wrapped: false,
                ..Default::default()
            });
        }
        assert_eq!(s.len(), 5);
        let lines = s.get_lines(0, 5);
        assert_eq!(lines.len(), 5);
        for (i, line) in lines.iter().enumerate() {
            assert_eq!(line.text, format!("line-{i}"));
        }
    }

    #[test]
    fn test_ring_buffer_wraps_and_evicts_oldest() {
        let mut s = ScrollbackStorage::new(3);
        for i in 0..5 {
            s.push_line(ScrollbackLine {
                text: format!("line-{i}"),
                wrapped: false,
                ..Default::default()
            });
        }
        // Capacity 3, pushed 5 -> oldest 2 evicted, remaining: line-2, line-3, line-4
        assert_eq!(s.len(), 3);
        let lines = s.get_lines(0, 3);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text, "line-2");
        assert_eq!(lines[1].text, "line-3");
        assert_eq!(lines[2].text, "line-4");
    }

    #[test]
    fn test_get_lines_partial_range() {
        let mut s = ScrollbackStorage::new(10);
        for i in 0..5 {
            s.push_line(ScrollbackLine {
                text: format!("line-{i}"),
                wrapped: false,
                ..Default::default()
            });
        }
        let lines = s.get_lines(2, 2);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "line-2");
        assert_eq!(lines[1].text, "line-3");
    }

    #[test]
    fn test_get_lines_beyond_range_returns_available() {
        let mut s = ScrollbackStorage::new(10);
        for i in 0..3 {
            s.push_line(ScrollbackLine {
                text: format!("line-{i}"),
                wrapped: false,
                ..Default::default()
            });
        }
        let lines = s.get_lines(1, 100);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "line-1");
        assert_eq!(lines[1].text, "line-2");
    }

    #[test]
    fn test_get_lines_start_beyond_len_returns_empty() {
        let mut s = ScrollbackStorage::new(10);
        s.push_line(ScrollbackLine {
            text: "a".to_string(),
            wrapped: false,
            ..Default::default()
        });
        let lines = s.get_lines(5, 1);
        assert!(lines.is_empty());
    }

    #[test]
    fn test_get_lines_zero_count_returns_empty() {
        let mut s = ScrollbackStorage::new(10);
        s.push_line(ScrollbackLine {
            text: "a".to_string(),
            wrapped: false,
            ..Default::default()
        });
        let lines = s.get_lines(0, 0);
        assert!(lines.is_empty());
    }

    #[test]
    fn test_clear_resets_storage() {
        let mut s = ScrollbackStorage::new(10);
        for i in 0..5 {
            s.push_line(ScrollbackLine {
                text: format!("line-{i}"),
                wrapped: false,
                ..Default::default()
            });
        }
        assert_eq!(s.len(), 5);
        s.clear();
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
        let lines = s.get_lines(0, 10);
        assert!(lines.is_empty());
    }

    #[test]
    fn test_capacity_minimum_is_one() {
        let s = ScrollbackStorage::new(0);
        assert_eq!(s.capacity(), 1);
    }

    #[test]
    fn test_default_capacity_constant() {
        assert_eq!(ScrollbackStorage::DEFAULT_CAPACITY, 10_000);
    }

    #[test]
    fn test_large_capacity_push_and_wrap() {
        let cap = ScrollbackStorage::DEFAULT_CAPACITY;
        let mut s = ScrollbackStorage::new(cap);
        // Push cap + 100 lines
        for i in 0..(cap + 100) {
            s.push_line(ScrollbackLine {
                text: format!("L{i}"),
                wrapped: false,
                ..Default::default()
            });
        }
        assert_eq!(s.len(), cap);
        // Oldest should be line 100
        let lines = s.get_lines(0, 1);
        assert_eq!(lines[0].text, "L100");
        // Newest should be line cap+99
        let lines = s.get_lines(cap - 1, 1);
        assert_eq!(lines[0].text, format!("L{}", cap + 99));
    }

    #[test]
    fn test_wrapped_attribute_preserved() {
        let mut s = ScrollbackStorage::new(10);
        s.push_line(ScrollbackLine {
            text: "wrapped line".to_string(),
            wrapped: true,
            ..Default::default()
        });
        let lines = s.get_lines(0, 1);
        assert!(lines[0].wrapped);
    }

    #[test]
    fn test_sgr_attributes_preserved_in_formatted_bytes() {
        // SPEC-1919 FR-003j / SC-005b: scrollback push_line must retain the
        // SGR-bearing escape stream so a later replay can reconstruct the
        // original foreground / background / bold / italic / underline /
        // inverse cell attributes; storing only plain text + wrap flag would
        // lose color on scroll-back through xterm.js.
        let mut s = ScrollbackStorage::new(10);
        let formatted = b"\x1b[31;1mALERT\x1b[0m normal".to_vec();
        s.push_line(ScrollbackLine {
            text: "ALERT normal".to_string(),
            formatted: formatted.clone(),
            wrapped: false,
        });
        let lines = s.get_lines(0, 1);
        assert_eq!(lines[0].text, "ALERT normal");
        assert_eq!(
            lines[0].formatted, formatted,
            "expected SGR-bearing formatted bytes to round-trip through scrollback"
        );
        // CSI introducer + final 'm' byte must be present so the line can be
        // replayed verbatim into xterm.js without losing color/style.
        let has_csi_m = lines[0].formatted.windows(2).enumerate().any(|(idx, win)| {
            win == [0x1b, b'['] && {
                let tail = &lines[0].formatted[idx + 2..];
                tail.iter().take(16).any(|b| *b == b'm')
            }
        });
        assert!(
            has_csi_m,
            "scrollback line should retain at least one CSI ... m SGR escape; got {:?}",
            lines[0].formatted
        );
    }
}
