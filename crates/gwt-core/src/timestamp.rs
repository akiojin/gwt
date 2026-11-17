//! UTC timestamps for persisted and wire data.

use std::{fmt, str::FromStr, time::SystemTime};

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use time::{Date, Duration, Month, OffsetDateTime, UtcOffset};

const NANOS_PER_SECOND: u32 = 1_000_000_000;
const SECONDS_PER_DAY: i64 = 86_400;
const LEGACY_YEARS: std::ops::RangeInclusive<i32> = -262_143..=262_142;

/// A UTC instant with nanosecond precision, including legacy leap seconds.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UtcTimestamp {
    seconds: i64,
    nanoseconds: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct InvalidTimestamp;

impl fmt::Display for InvalidTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid date and time")
    }
}

impl std::error::Error for InvalidTimestamp {}

impl UtcTimestamp {
    pub const UNIX_EPOCH: Self = Self {
        seconds: 0,
        nanoseconds: 0,
    };

    pub fn now() -> Self {
        SystemTime::now().into()
    }

    fn from_timestamp(seconds: i64, nanoseconds: u32) -> Option<Self> {
        if nanoseconds >= 2 * NANOS_PER_SECOND
            || (nanoseconds >= NANOS_PER_SECOND && seconds.rem_euclid(60) != 59)
        {
            return None;
        }
        let year = OffsetDateTime::from_unix_timestamp(seconds).ok()?.year();
        if !LEGACY_YEARS.contains(&year) {
            return None;
        }
        Some(Self {
            seconds,
            nanoseconds,
        })
    }

    /// Whole seconds since the Unix epoch, using the preceding second for a
    /// leap second.
    pub fn timestamp(self) -> i64 {
        self.seconds
    }

    pub fn whole_seconds_since(self, earlier: Self) -> i64 {
        let mut seconds = self.seconds - earlier.seconds;
        // Legacy arithmetic counts the stored leap second when it comes
        // between the two times-of-day.
        let this_clock = self.seconds.rem_euclid(SECONDS_PER_DAY);
        let earlier_clock = earlier.seconds.rem_euclid(SECONDS_PER_DAY);
        if this_clock > earlier_clock && earlier.nanoseconds >= NANOS_PER_SECOND {
            seconds += 1;
        } else if this_clock < earlier_clock && self.nanoseconds >= NANOS_PER_SECOND {
            seconds -= 1;
        }
        let nanos = self.nanoseconds as i32 - earlier.nanoseconds as i32;
        Duration::new(seconds, nanos).whole_seconds()
    }

    /// Format UTC with the numeric offset used by existing user-facing output.
    pub fn to_rfc3339(self) -> String {
        let mut output = String::new();
        self.format(&mut output, 'T', "+00:00")
            .expect("format valid UTC timestamp");
        output
    }

    /// Format UTC at whole-second precision for legacy status views.
    pub fn to_rfc3339_seconds(mut self) -> String {
        self.nanoseconds -= self.nanoseconds % NANOS_PER_SECOND;
        let mut output = String::new();
        self.format(&mut output, 'T', "Z")
            .expect("format valid UTC timestamp");
        output
    }

    fn format(self, f: &mut impl fmt::Write, separator: char, offset: &str) -> fmt::Result {
        let now = OffsetDateTime::from_unix_timestamp(self.seconds).map_err(|_| fmt::Error)?;
        let year = now.year();
        if (0..=9999).contains(&year) {
            write!(f, "{year:04}")?;
        } else {
            write!(f, "{year:+05}")?;
        }
        write!(
            f,
            "-{:02}-{:02}{separator}{:02}:{:02}:{:02}",
            now.month() as u8,
            now.day(),
            now.hour(),
            now.minute(),
            now.second() + u8::from(self.nanoseconds >= NANOS_PER_SECOND)
        )?;
        match self.nanoseconds % NANOS_PER_SECOND {
            0 => {}
            nanos if nanos.is_multiple_of(1_000_000) => write!(f, ".{:03}", nanos / 1_000_000)?,
            nanos if nanos.is_multiple_of(1_000) => write!(f, ".{:06}", nanos / 1_000)?,
            nanos => write!(f, ".{nanos:09}")?,
        }
        f.write_str(offset)
    }
}

impl From<SystemTime> for UtcTimestamp {
    fn from(value: SystemTime) -> Self {
        let at = OffsetDateTime::from(value);
        Self::from_timestamp(at.unix_timestamp(), at.nanosecond())
            .expect("system time is outside the supported UTC timestamp range")
    }
}

impl From<UtcTimestamp> for SystemTime {
    fn from(value: UtcTimestamp) -> Self {
        let seconds = std::time::Duration::from_secs(value.seconds.unsigned_abs());
        let nanos = std::time::Duration::from_nanos(u64::from(value.nanoseconds));
        if value.seconds < 0 {
            Self::UNIX_EPOCH - seconds + nanos
        } else {
            Self::UNIX_EPOCH + seconds + nanos
        }
    }
}

// These conversions are used at the remaining Chrono API boundaries while
// the crates that own those boundaries migrate. SystemTime would lose the
// extra nanoseconds that identify a persisted leap second.
impl From<chrono::DateTime<chrono::Utc>> for UtcTimestamp {
    fn from(value: chrono::DateTime<chrono::Utc>) -> Self {
        Self {
            seconds: value.timestamp(),
            nanoseconds: value.timestamp_subsec_nanos(),
        }
    }
}

impl From<UtcTimestamp> for chrono::DateTime<chrono::Utc> {
    fn from(value: UtcTimestamp) -> Self {
        Self::from_timestamp(value.seconds, value.nanoseconds).expect("valid legacy UTC timestamp")
    }
}

impl fmt::Debug for UtcTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.format(f, 'T', "Z")
    }
}

impl fmt::Display for UtcTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.format(f, ' ', " UTC")
    }
}

impl Serialize for UtcTimestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        struct WireTimestamp(UtcTimestamp);

        impl fmt::Display for WireTimestamp {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.format(f, 'T', "Z")
            }
        }

        serializer.collect_str(&WireTimestamp(*self))
    }
}

impl<'de> Deserialize<'de> for UtcTimestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct TimestampVisitor;

        impl de::Visitor<'_> for TimestampVisitor {
            type Value = UtcTimestamp;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an RFC 3339 formatted date and time string")
            }

            fn visit_str<E: de::Error>(self, input: &str) -> Result<UtcTimestamp, E> {
                input.parse().map_err(E::custom)
            }
        }

        deserializer.deserialize_str(TimestampVisitor)
    }
}

impl FromStr for UtcTimestamp {
    type Err = InvalidTimestamp;

    fn from_str(input: &str) -> Result<Self, InvalidTimestamp> {
        parse_legacy_datetime(input).ok_or(InvalidTimestamp)
    }
}

fn take_digits<'a>(input: &mut &'a str, max: usize) -> Option<&'a str> {
    let end = input
        .bytes()
        .take(max)
        .take_while(u8::is_ascii_digit)
        .count();
    if end == 0 {
        return None;
    }
    let (digits, remaining) = input.split_at(end);
    *input = remaining;
    Some(digits)
}

fn take_component(input: &mut &str) -> Option<u8> {
    *input = input.trim_start();
    take_digits(input, 2)?.parse().ok()
}

fn parse_legacy_datetime(mut input: &str) -> Option<UtcTimestamp> {
    input = input.trim_start();
    let (sign, year_digits) = match input.as_bytes().first()? {
        b'+' => (1, usize::MAX),
        b'-' => (-1, usize::MAX),
        _ => (1, 4),
    };
    if year_digits == usize::MAX {
        input = &input[1..];
    }
    let year = take_digits(&mut input, year_digits)?.parse::<i32>().ok()? * sign;
    if !LEGACY_YEARS.contains(&year) {
        return None;
    }
    input = input.trim_start().strip_prefix('-')?;
    let month = Month::try_from(take_component(&mut input)?).ok()?;
    input = input.trim_start().strip_prefix('-')?;
    let day = take_component(&mut input)?;
    if !matches!(input.as_bytes().first()?, b'T' | b't' | b' ') {
        return None;
    }
    input = &input[1..];
    let hour = take_component(&mut input)?;
    input = input.trim_start().strip_prefix(':')?;
    let minute = take_component(&mut input)?;
    input = input.trim_start().strip_prefix(':')?;
    let mut second = take_component(&mut input)?;
    let mut nanoseconds = 0;
    if let Some(fraction) = input.strip_prefix('.') {
        input = fraction;
        let digits = take_digits(&mut input, usize::MAX)?;
        let kept = &digits[..digits.len().min(9)];
        nanoseconds = kept.parse::<u32>().ok()? * 10_u32.pow(9 - kept.len() as u32);
    }
    if second == 60 {
        second = 59;
        nanoseconds += NANOS_PER_SECOND;
    }
    let offset = parse_offset(input.trim_start())?;
    let at = Date::from_calendar_date(year, month, day)
        .ok()?
        .with_hms_nano(hour, minute, second, nanoseconds % NANOS_PER_SECOND)
        .ok()?
        .assume_offset(offset);
    UtcTimestamp::from_timestamp(at.unix_timestamp(), nanoseconds)
}

fn parse_offset(mut input: &str) -> Option<UtcOffset> {
    match input.trim_end() {
        "z" | "Z" => return Some(UtcOffset::UTC),
        utc if utc.eq_ignore_ascii_case("UTC") => return Some(UtcOffset::UTC),
        _ => {}
    }
    let sign = input.chars().next()?;
    let multiplier = match sign {
        '+' => 1,
        '-' | '−' => -1,
        _ => return None,
    };
    input = &input[sign.len_utf8()..];
    let hours = take_digits(&mut input, 2)?;
    if hours.len() != 2 {
        return None;
    }
    input = input.trim_start_matches(|ch: char| ch == ':' || ch.is_whitespace());
    let minutes = take_digits(&mut input, 2)?;
    if minutes.len() != 2 || !input.trim_end().is_empty() {
        return None;
    }
    let hours = hours.parse::<i8>().ok()?;
    if hours > 23 {
        return None;
    }
    UtcOffset::from_hms(
        hours * multiplier,
        minutes.parse::<i8>().ok()? * multiplier,
        0,
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_matches_chrono_records() {
        for input in [
            "2026-11-18T00:00:00Z",
            "2026-11-18T00:00:00.007Z",
            "2026-11-18T00:00:00.007980-03:30",
            "2026-11-18T00:00:00.007987654+05:30",
            "2026-11-18 00:00:00.012345678910−07:00",
            "2026-11-18t00:00:00z",
            "2016-12-31T23:59:60.125Z",
            "2016-12-31T00:00:60.125+07:00",
            "0000-01-01T00:00:00+23:59",
            "9999-12-31T23:59:59-23:59",
            "2026-11-18Q00:00:00Z",
            "2026-11-18T00:00:00+0000",
            "2026-11-18T00:00:00+24:00",
            "2026-11-18T00:00:00+00:60",
            "2026-11-18T00:00:00",
            "2026-11-18T00:00:00Z ",
            "2026-11-18T00:00:00.Z",
            "2026−11-18T00:00:00Z",
            "+10000-01-01T00:00:00Z",
            "-0001-12-30T00:00:00Z",
            "2012-  12-12T12:  12:12Z",
            " 2012-1-2 3:4:5 UTC",
            "2012-1-2 3:4:5+01 : : 00\t",
        ] {
            let legacy = input.parse::<chrono::DateTime<chrono::Utc>>();
            let candidate = input.parse::<UtcTimestamp>();
            assert_eq!(candidate.is_ok(), legacy.is_ok(), "{input}");
            if let (Ok(candidate), Ok(legacy)) = (candidate, legacy) {
                assert_eq!(chrono::DateTime::from(candidate), legacy, "{input}");
                assert_eq!(UtcTimestamp::from(legacy), candidate, "{input}");
                assert_eq!(candidate.to_rfc3339(), legacy.to_rfc3339(), "{input}");
                assert_eq!(
                    candidate.to_rfc3339_seconds(),
                    legacy.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    "{input}"
                );
                assert_eq!(
                    serde_json::to_string(&candidate).unwrap(),
                    serde_json::to_string(&legacy).unwrap(),
                    "{input}"
                );
                assert_eq!(format!("{candidate:?}"), format!("{legacy:?}"), "{input}");
                assert_eq!(candidate.to_string(), legacy.to_string(), "{input}");
                assert_eq!(
                    serde_json::from_str::<UtcTimestamp>(&serde_json::to_string(input).unwrap())
                        .unwrap(),
                    candidate,
                    "{input}"
                );
            }
        }
    }

    #[test]
    fn system_time_conversion_preserves_runtime_gc_ages() {
        for input in ["1969-12-31T23:59:59.125Z", "2016-12-31T23:59:60.125Z"] {
            let old: chrono::DateTime<chrono::Utc> = input.parse().unwrap();
            let new: UtcTimestamp = input.parse().unwrap();
            assert_eq!(SystemTime::from(new), SystemTime::from(old));
        }
    }

    #[test]
    fn keeps_legacy_whole_seconds_when_crossing_a_leap_second() {
        let before = "2016-12-31T23:59:58.575Z"
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap();
        let leap = "2016-12-31T23:59:60.125Z"
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap();
        let after = "2017-01-01T00:00:01.875Z"
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap();
        for (left, right) in [(before, leap), (leap, before), (leap, after), (after, leap)] {
            assert_eq!(
                UtcTimestamp::from(left).whole_seconds_since(UtcTimestamp::from(right)),
                (left - right).num_seconds()
            );
        }
    }
}
