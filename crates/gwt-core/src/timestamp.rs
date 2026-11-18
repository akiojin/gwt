//! UTC timestamps for persisted and wire data.

use std::{
    borrow::Cow,
    fmt,
    ops::{Add, AddAssign, Sub, SubAssign},
    str::FromStr,
    time::SystemTime,
};

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use time::{
    format_description::well_known::Rfc3339, Date, Duration, Month, OffsetDateTime, UtcOffset,
};

const NANOS_PER_SECOND: u32 = 1_000_000_000;
const SECONDS_PER_DAY: i64 = 86_400;
const LEGACY_YEARS: std::ops::RangeInclusive<i32> = -262_143..=262_142;

/// A UTC instant with nanosecond precision, including legacy leap seconds.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UtcTimestamp {
    seconds: i64,
    nanoseconds: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct InvalidTimestamp;

#[derive(Clone, Copy)]
enum FractionalSeconds {
    Automatic,
    Milliseconds,
    Nanoseconds,
}

impl fmt::Display for InvalidTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid date and time")
    }
}

impl std::error::Error for InvalidTimestamp {}

impl UtcTimestamp {
    /// The endpoints of the UTC range supported by persisted records.
    pub const MIN: Self = Self {
        seconds: time::macros::datetime!(-262143-01-01 0:00 UTC).unix_timestamp(),
        nanoseconds: 0,
    };

    pub const MAX: Self = Self {
        seconds: time::macros::datetime!(+262142-12-31 23:59:59 UTC).unix_timestamp(),
        nanoseconds: NANOS_PER_SECOND - 1,
    };

    pub const UNIX_EPOCH: Self = Self {
        seconds: 0,
        nanoseconds: 0,
    };

    pub fn now() -> Self {
        SystemTime::now().into()
    }

    pub fn from_timestamp(seconds: i64, nanoseconds: u32) -> Option<Self> {
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

    /// Construct a UTC timestamp from a whole number of Unix seconds.
    pub fn from_unix_seconds(seconds: i64) -> Option<Self> {
        Self::from_timestamp(seconds, 0)
    }

    pub fn from_ymd_and_hms(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> Option<Self> {
        let month = Month::try_from(u8::try_from(month).ok()?).ok()?;
        let date = Date::from_calendar_date(year, month, day.try_into().ok()?).ok()?;
        let at = date
            .with_hms(
                hour.try_into().ok()?,
                minute.try_into().ok()?,
                second.try_into().ok()?,
            )
            .ok()?
            .assume_utc();
        Self::from_unix_seconds(at.unix_timestamp())
    }

    /// Add a signed duration, returning no value if the result is outside
    /// the supported calendar range.
    pub fn checked_add(self, duration: Duration) -> Option<Self> {
        self.checked_shift(duration.whole_nanoseconds())
    }

    /// Parse a wire timestamp with the same separator and leap-second rules
    /// as the previous parser.
    pub fn parse_rfc3339(input: &str) -> Option<Self> {
        Self::parse_with_offset(input).map(|(at, _)| at)
    }

    /// Parse a wire timestamp and return its original numeric offset.
    pub fn parse_with_offset(input: &str) -> Option<(Self, UtcOffset)> {
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
        Some((
            Self {
                seconds: parsed.unix_timestamp(),
                nanoseconds: parsed.nanosecond() + u32::from(leap_second) * NANOS_PER_SECOND,
            },
            parsed.offset(),
        ))
    }

    pub fn parse_from_rfc3339(input: &str) -> Result<Self, InvalidTimestamp> {
        Self::parse_rfc3339(input).ok_or(InvalidTimestamp)
    }

    /// Whole seconds since the Unix epoch, using the preceding second for a
    /// leap second.
    pub fn timestamp(self) -> i64 {
        self.seconds
    }

    /// Milliseconds since the Unix epoch, truncating fractional milliseconds.
    pub fn timestamp_millis(self) -> i64 {
        self.seconds * 1_000 + i64::from(self.nanoseconds / 1_000_000)
    }

    pub fn timestamp_nanos_opt(self) -> Option<i64> {
        let nanos =
            i128::from(self.seconds) * i128::from(NANOS_PER_SECOND) + i128::from(self.nanoseconds);
        nanos.try_into().ok()
    }

    pub fn timestamp_subsec_nanos(self) -> u32 {
        self.nanoseconds
    }

    pub fn utc_datetime(self) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(self.seconds)
            .and_then(|value| value.replace_nanosecond(self.nanoseconds % NANOS_PER_SECOND))
            .expect("UTC timestamp has a valid date and nanoseconds")
    }

    pub fn local_datetime(self) -> OffsetDateTime {
        let utc = self.utc_datetime();
        let offset = UtcOffset::local_offset_at(utc).unwrap_or(UtcOffset::UTC);
        utc.to_offset(offset)
    }

    pub fn now_local_rfc3339() -> String {
        let mut output = String::new();
        crate::logging::fmt_layer::write_log_timestamp(&mut output, Self::now().local_datetime())
            .expect("format valid local timestamp");
        output
    }

    /// The calendar date of this timestamp in UTC.
    pub fn utc_date(self) -> Date {
        OffsetDateTime::from_unix_timestamp(self.seconds)
            .expect("valid UTC timestamp")
            .date()
    }

    pub fn whole_seconds_since(self, earlier: Self) -> i64 {
        self.signed_duration_since(earlier).whole_seconds()
    }

    pub fn signed_duration_since(self, earlier: Self) -> Duration {
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
        Duration::new(seconds, nanos)
    }

    /// Format UTC with the numeric offset used by existing user-facing output.
    pub fn to_rfc3339(self) -> String {
        let mut output = String::new();
        self.format(&mut output, 'T', "+00:00", FractionalSeconds::Automatic)
            .expect("format valid UTC timestamp");
        output
    }

    /// Format the instant using a numeric offset and the legacy precision.
    pub fn to_rfc3339_with_offset(self, offset: UtcOffset) -> String {
        self.format_at_offset(offset, false)
            .expect("format valid timestamp at offset")
    }

    /// Format UTC with exactly three digits, truncating sub-millisecond precision.
    pub fn to_rfc3339_millis(self) -> String {
        let mut output = String::new();
        self.format(&mut output, 'T', "Z", FractionalSeconds::Milliseconds)
            .expect("format valid UTC timestamp");
        output
    }

    /// Format UTC with exactly nine fractional digits.
    pub fn to_rfc3339_nanos(self) -> String {
        let mut output = String::new();
        self.format(&mut output, 'T', "Z", FractionalSeconds::Nanoseconds)
            .expect("format valid UTC timestamp");
        output
    }

    fn checked_shift(self, delta_nanos: i128) -> Option<Self> {
        let nanos_per_second = i128::from(NANOS_PER_SECOND);
        let mut nanos = i128::from(self.nanoseconds) + delta_nanos;
        if self.nanoseconds >= NANOS_PER_SECOND {
            if delta_nanos >= nanos_per_second || nanos >= 2 * nanos_per_second {
                // A positive shift across the known leap second first spends
                // one second there before advancing the Unix timestamp.
                nanos -= nanos_per_second;
            } else if delta_nanos > -nanos_per_second {
                // A fractional shift inside the leap second or its preceding
                // second can keep the same Unix second.
                return Self::from_timestamp(self.seconds, nanos.try_into().ok()?);
            }
        }
        let seconds = i128::from(self.seconds) + nanos.div_euclid(nanos_per_second);
        Self::from_timestamp(
            seconds.try_into().ok()?,
            nanos.rem_euclid(nanos_per_second) as u32,
        )
    }

    /// Format UTC at whole-second precision for legacy status views.
    pub fn to_rfc3339_seconds(mut self) -> String {
        self.nanoseconds -= self.nanoseconds % NANOS_PER_SECOND;
        let mut output = String::new();
        self.format(&mut output, 'T', "Z", FractionalSeconds::Automatic)
            .expect("format valid UTC timestamp");
        output
    }

    fn format(
        self,
        f: &mut impl fmt::Write,
        separator: char,
        offset: &str,
        precision: FractionalSeconds,
    ) -> fmt::Result {
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
        let nanos = self.nanoseconds % NANOS_PER_SECOND;
        match precision {
            FractionalSeconds::Milliseconds => write!(f, ".{:03}", nanos / 1_000_000)?,
            FractionalSeconds::Nanoseconds => write!(f, ".{nanos:09}")?,
            FractionalSeconds::Automatic => match nanos {
                0 => {}
                nanos if nanos.is_multiple_of(1_000_000) => write!(f, ".{:03}", nanos / 1_000_000)?,
                nanos if nanos.is_multiple_of(1_000) => write!(f, ".{:06}", nanos / 1_000)?,
                nanos => write!(f, ".{nanos:09}")?,
            },
        }
        f.write_str(offset)
    }

    fn format_at_offset(
        mut self,
        offset: UtcOffset,
        whole_seconds: bool,
    ) -> Result<String, fmt::Error> {
        let offset_seconds = offset.whole_seconds();
        self.seconds += i64::from(offset_seconds);
        if whole_seconds {
            self.nanoseconds -= self.nanoseconds % NANOS_PER_SECOND;
        }
        let minutes = (offset_seconds.unsigned_abs() + 30) / 60;
        let sign = if offset_seconds < 0 { '-' } else { '+' };
        let offset = if offset_seconds == 0 && whole_seconds {
            "Z".to_string()
        } else {
            format!("{sign}{:02}:{:02}", minutes / 60, minutes % 60)
        };
        let mut output = String::new();
        self.format(&mut output, 'T', &offset, FractionalSeconds::Automatic)?;
        Ok(output)
    }
}

impl Add<Duration> for UtcTimestamp {
    type Output = Self;

    fn add(self, rhs: Duration) -> Self {
        self.checked_shift(rhs.whole_nanoseconds())
            .expect("overflow adding duration to UTC timestamp")
    }
}

impl AddAssign<Duration> for UtcTimestamp {
    fn add_assign(&mut self, rhs: Duration) {
        *self = *self + rhs;
    }
}

impl Sub<Duration> for UtcTimestamp {
    type Output = Self;

    fn sub(self, rhs: Duration) -> Self {
        self.checked_shift(-rhs.whole_nanoseconds())
            .expect("overflow subtracting duration from UTC timestamp")
    }
}

impl SubAssign<Duration> for UtcTimestamp {
    fn sub_assign(&mut self, rhs: Duration) {
        *self = *self - rhs;
    }
}

/// Shift an RFC3339 value while retaining the source's numeric UTC offset.
pub fn shift_rfc3339(input: &str, duration: Duration) -> Option<String> {
    format_shifted_rfc3339(input, duration, false)
}

/// Shift an RFC3339 value and render whole seconds, using Z for UTC.
pub fn shift_rfc3339_seconds(input: &str, duration: Duration) -> Option<String> {
    format_shifted_rfc3339(input, duration, true)
}

fn format_shifted_rfc3339(input: &str, duration: Duration, whole_seconds: bool) -> Option<String> {
    let (instant, offset) = UtcTimestamp::parse_with_offset(input)?;
    instant
        .checked_add(duration)?
        .format_at_offset(offset, whole_seconds)
        .ok()
}

impl Sub for UtcTimestamp {
    type Output = Duration;

    fn sub(self, rhs: Self) -> Duration {
        self.signed_duration_since(rhs)
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

impl fmt::Debug for UtcTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.format(f, 'T', "Z", FractionalSeconds::Automatic)
    }
}

impl fmt::Display for UtcTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.format(f, ' ', " UTC", FractionalSeconds::Automatic)
    }
}

impl Serialize for UtcTimestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        struct WireTimestamp(UtcTimestamp);

        impl fmt::Display for WireTimestamp {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.format(f, 'T', "Z", FractionalSeconds::Automatic)
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

/// Parse the UTC calendar date used by daily file names.
pub fn parse_utc_date(input: &str) -> Option<Date> {
    let (date, remaining) = parse_date_prefix(input, |remaining| remaining)?;
    remaining.is_empty().then_some(date)
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

fn parse_date_prefix(
    mut input: &str,
    trim_before_separator: fn(&str) -> &str,
) -> Option<(Date, &str)> {
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
    input = trim_before_separator(input).strip_prefix('-')?;
    let month = Month::try_from(take_component(&mut input)?).ok()?;
    input = trim_before_separator(input).strip_prefix('-')?;
    let day = take_component(&mut input)?;
    let date = Date::from_calendar_date(year, month, day).ok()?;
    Some((date, input))
}

fn parse_legacy_datetime(input: &str) -> Option<UtcTimestamp> {
    let (date, mut input) = parse_date_prefix(input, str::trim_start)?;
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
    let at = date
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
    fn serde_matches_legacy_records() {
        for (input, expected) in [
            ("2026-11-18T00:00:00Z", Some("2026-11-18T00:00:00Z")),
            ("2026-11-18T00:00:00.007Z", Some("2026-11-18T00:00:00.007Z")),
            (
                "2026-11-18T00:00:00.007980-03:30",
                Some("2026-11-18T03:30:00.007980Z"),
            ),
            (
                "2026-11-18T00:00:00.007987654+05:30",
                Some("2026-11-17T18:30:00.007987654Z"),
            ),
            (
                "2026-11-18 00:00:00.012345678910−07:00",
                Some("2026-11-18T07:00:00.012345678Z"),
            ),
            ("2026-11-18t00:00:00z", Some("2026-11-18T00:00:00Z")),
            ("2016-12-31T23:59:60.125Z", Some("2016-12-31T23:59:60.125Z")),
            (
                "2016-12-31T00:00:60.125+07:00",
                Some("2016-12-30T17:00:60.125Z"),
            ),
            ("0000-01-01T00:00:00+23:59", Some("-0001-12-31T00:01:00Z")),
            ("9999-12-31T23:59:59-23:59", Some("+10000-01-01T23:58:59Z")),
            ("2026-11-18Q00:00:00Z", None),
            ("2026-11-18T00:00:00+0000", Some("2026-11-18T00:00:00Z")),
            ("2026-11-18T00:00:00+24:00", None),
            ("2026-11-18T00:00:00+00:60", None),
            ("2026-11-18T00:00:00", None),
            ("2026-11-18T00:00:00Z ", Some("2026-11-18T00:00:00Z")),
            ("2026-11-18T00:00:00.Z", None),
            ("2026−11-18T00:00:00Z", None),
            ("+10000-01-01T00:00:00Z", Some("+10000-01-01T00:00:00Z")),
            ("-0001-12-30T00:00:00Z", Some("-0001-12-30T00:00:00Z")),
            ("2012-  12-12T12:  12:12Z", Some("2012-12-12T12:12:12Z")),
            (" 2012-1-2 3:4:5 UTC", Some("2012-01-02T03:04:05Z")),
            ("2012-1-2 3:4:5+01 : : 00\t", Some("2012-01-02T02:04:05Z")),
        ] {
            let candidate = serde_json::from_value::<UtcTimestamp>(serde_json::json!(input));
            assert_eq!(candidate.is_ok(), expected.is_some(), "{input}");
            if let Some(expected) = expected {
                let actual = candidate.unwrap();
                assert_eq!(
                    serde_json::to_value(actual).unwrap(),
                    serde_json::json!(expected)
                );
            }
        }
    }

    #[test]
    fn strict_rfc3339_matches_previous_wire_parser() {
        for (input, expected) in [
            (
                "1970-01-01t00:00:00.000001200−00:30",
                Some("1970-01-01T00:30:00.000001200Z"),
            ),
            (
                "2017-01-01 00:59:60.1250000005+01:00",
                Some("2016-12-31T23:59:60.125Z"),
            ),
            ("0000-01-01T00:00:00+23:59", Some("-0001-12-31T00:01:00Z")),
            ("9999-12-31T23:59:59-23:59", Some("+10000-01-01T23:58:59Z")),
            ("2026-11-18Q00:00:00Z", None),
            ("2026-11-18T00:00:00+0000", None),
            ("2026-11-18T00:00:00+24:00", None),
            ("+10000-01-01T00:00:00Z", None),
        ] {
            let actual = UtcTimestamp::parse_rfc3339(input).map(|value| format!("{value:?}"));
            assert_eq!(actual.as_deref(), expected, "{input}");
        }
    }

    #[test]
    fn system_time_conversion_preserves_runtime_gc_ages() {
        let before: UtcTimestamp = "1969-12-31T23:59:59.125Z".parse().unwrap();
        let leap: UtcTimestamp = "2016-12-31T23:59:60.125Z".parse().unwrap();
        assert_eq!(
            SystemTime::from(before),
            SystemTime::UNIX_EPOCH - std::time::Duration::from_millis(875)
        );
        assert_eq!(
            SystemTime::from(leap),
            SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(1_483_228_800_125)
        );
    }

    #[test]
    fn keeps_legacy_whole_seconds_when_crossing_a_leap_second() {
        let [before, leap, after]: [UtcTimestamp; 3] = [
            "2016-12-31T23:59:58.575Z",
            "2016-12-31T23:59:60.125Z",
            "2017-01-01T00:00:01.875Z",
        ]
        .map(|input| input.parse().unwrap());
        for (left, right, expected) in [
            (before, leap, -1),
            (leap, before, 1),
            (leap, after, -1),
            (after, leap, 1),
        ] {
            assert_eq!(left.whole_seconds_since(right), expected);
        }
    }

    #[test]
    fn fixed_precision_uses_legacy_wire_spelling() {
        let timestamp: UtcTimestamp = "2026-11-18T12:34:56.012987Z".parse().unwrap();
        assert_eq!(timestamp.to_rfc3339(), "2026-11-18T12:34:56.012987+00:00");
        assert_eq!(timestamp.to_rfc3339_seconds(), "2026-11-18T12:34:56Z");
        assert_eq!(timestamp.to_rfc3339_millis(), "2026-11-18T12:34:56.012Z");
        assert_eq!(
            timestamp.to_rfc3339_nanos(),
            "2026-11-18T12:34:56.012987000Z"
        );
    }

    #[test]
    fn shifted_values_preserve_wire_offsets_and_precision() {
        let input = "2026-11-18T00:59:59.002+05:30";
        let delay = Duration::seconds(2);
        assert_eq!(
            shift_rfc3339(input, delay).as_deref(),
            Some("2026-11-18T01:00:01.002+05:30")
        );
        assert_eq!(
            shift_rfc3339_seconds(input, delay).as_deref(),
            Some("2026-11-18T01:00:01+05:30")
        );
    }

    #[test]
    fn signed_shifts_preserve_legacy_leap_seconds() {
        let timestamp: UtcTimestamp = "2016-12-31T23:59:60.125Z".parse().unwrap();
        for (millis, plus, minus) in [
            (
                -2000,
                "2016-12-31T23:59:58.125Z",
                "2017-01-01T00:00:01.125Z",
            ),
            (
                -1000,
                "2016-12-31T23:59:59.125Z",
                "2017-01-01T00:00:00.125Z",
            ),
            (-500, "2016-12-31T23:59:59.625Z", "2016-12-31T23:59:60.625Z"),
            (0, "2016-12-31T23:59:60.125Z", "2016-12-31T23:59:60.125Z"),
            (500, "2016-12-31T23:59:60.625Z", "2016-12-31T23:59:59.625Z"),
            (875, "2017-01-01T00:00:00Z", "2016-12-31T23:59:59.250Z"),
            (1000, "2017-01-01T00:00:00.125Z", "2016-12-31T23:59:59.125Z"),
            (2000, "2017-01-01T00:00:01.125Z", "2016-12-31T23:59:58.125Z"),
        ] {
            let shift = Duration::milliseconds(millis);
            assert_eq!(format!("{:?}", timestamp + shift), plus);
            assert_eq!(format!("{:?}", timestamp - shift), minus);
        }
    }
}
