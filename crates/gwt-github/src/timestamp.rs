use std::borrow::Cow;

use time::{format_description::well_known::Rfc3339, Duration, OffsetDateTime};

const NANOS_PER_SECOND: u32 = 1_000_000_000;
const SECONDS_PER_DAY: i64 = 86_400;

/// A UTC instant parsed from GitHub's RFC3339 wire format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rfc3339Timestamp {
    unix_seconds: i64,
    nanoseconds: u32,
}

impl Rfc3339Timestamp {
    /// Parse a wire timestamp with the same separator and leap-second rules
    /// as the previous parser.
    pub fn parse(input: &str) -> Option<Self> {
        if !matches!(input.as_bytes().get(10), Some(b'T' | b't' | b' ')) {
            return None;
        }
        let leap_second = input.get(17..19) == Some("60");
        let input = if leap_second || input.contains('−') {
            let mut normalized = input.replace('−', "-");
            if leap_second {
                normalized.replace_range(17..19, "59");
            }
            Cow::Owned(normalized)
        } else {
            Cow::Borrowed(input)
        };
        let parsed = OffsetDateTime::parse(&input, &Rfc3339).ok()?;
        Some(Self {
            unix_seconds: parsed.unix_timestamp(),
            nanoseconds: parsed.nanosecond() + u32::from(leap_second) * NANOS_PER_SECOND,
        })
    }

    /// Seconds and subsecond nanoseconds, with a leap second represented by
    /// nanoseconds in the range 1,000,000,000 through 1,999,999,999.
    pub fn unix_parts(self) -> (i64, u32) {
        (self.unix_seconds, self.nanoseconds)
    }

    pub(crate) fn age_at(self, now: OffsetDateTime) -> Duration {
        let now_seconds = now.unix_timestamp();
        // Preserve Chrono's subtraction of UTC time-of-day values: the stored
        // leap second adds a second only when it precedes today's clock time.
        let add_leap_second = self.nanoseconds >= NANOS_PER_SECOND
            && now_seconds.rem_euclid(SECONDS_PER_DAY)
                > self.unix_seconds.rem_euclid(SECONDS_PER_DAY);
        Duration::seconds(now_seconds - self.unix_seconds + i64::from(add_leap_second))
            + Duration::nanoseconds(i64::from(now.nanosecond()) - i64::from(self.nanoseconds))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn accepts_legacy_separators_offsets_and_leap_seconds() {
        for (input, parts) in [
            ("1970-01-01t00:00:00.000001200−00:30", (1_800, 1_200)),
            (
                "2017-01-01 00:59:60.1250000005+01:00",
                (1_483_228_799, 1_125_000_000),
            ),
            ("0000-01-01T00:00:00+23:59", (-62_167_305_540, 0)),
            ("9999-12-31T23:59:59-23:59", (253_402_387_139, 0)),
        ] {
            assert_eq!(Rfc3339Timestamp::parse(input).unwrap().unix_parts(), parts);
        }
        for input in [
            "2026-11-18Q00:00:00Z",
            "2026-11-18T00:00:00+0000",
            "2026-11-18T00:00:00+24:00",
            "+10000-01-01T00:00:00Z",
        ] {
            assert_eq!(Rfc3339Timestamp::parse(input), None, "{input}");
        }
    }

    #[test]
    fn keeps_legacy_leap_second_age_at_and_across_midnight() {
        let before_midnight = Rfc3339Timestamp::parse("2016-12-31T23:59:60.125Z").unwrap();
        assert_eq!(
            before_midnight.age_at(datetime!(2017-01-01 00:00 UTC)),
            Duration::milliseconds(-125)
        );
        let after_midnight = Rfc3339Timestamp::parse("2017-01-01T00:00:60.125Z").unwrap();
        assert_eq!(
            after_midnight.age_at(datetime!(2017-01-01 00:01 UTC)),
            Duration::milliseconds(875)
        );
    }
}
