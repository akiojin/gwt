use crate::timestamp::UtcTimestamp;

// Retry-After previously accepted the RFC 2822 date forms in addition to
// the standard HTTP date. Keep obsolete years, named offsets, and leap
// seconds intact while deriving the calendar instant with time.
pub(super) fn parse_http_retry_date(mut input: &str) -> Option<UtcTimestamp> {
    const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    input = input.trim_start();
    let weekday = input.get(..3).and_then(|word| {
        WEEKDAYS
            .iter()
            .position(|name| name.eq_ignore_ascii_case(word))
    });
    if weekday.is_some() {
        input = input.get(3..)?.strip_prefix(',')?.trim_start();
    }
    let day = http_date_digits(&mut input, 1, 2)?.parse().ok()?;
    http_date_space(&mut input)?;
    let month = MONTHS
        .iter()
        .position(|name| name.eq_ignore_ascii_case(input.get(..3).unwrap_or_default()))?;
    let month = time::Month::try_from((month + 1) as u8).ok()?;
    input = input.get(3..)?;
    http_date_space(&mut input)?;

    let year_digits = http_date_digits(&mut input, 2, usize::MAX)?;
    let mut year: i32 = year_digits.parse().ok()?;
    year = match (year_digits.len(), year) {
        (2, 0..=49) => year + 2000,
        (2 | 3, _) => year + 1900,
        _ => year,
    };
    if year > 262_142 {
        return None;
    }
    http_date_space(&mut input)?;

    let hour = http_date_digits(&mut input, 2, 2)?.parse().ok()?;
    input = input.trim_start().strip_prefix(':')?.trim_start();
    let minute = http_date_digits(&mut input, 2, 2)?.parse().ok()?;
    let second: u8 = if let Some(rest) = input.trim_start().strip_prefix(':') {
        input = rest;
        http_date_digits(&mut input, 2, 2)?.parse().ok()?
    } else {
        0
    };
    http_date_space(&mut input)?;
    let iso_offset = http_date_offset(&mut input)?;
    while !input.is_empty() {
        input = strip_http_date_comment(input.trim_start())?;
    }

    let date = time::Date::from_calendar_date(year, month, day).ok()?;
    if weekday.is_some_and(|expected| expected != date.weekday().number_days_from_monday() as usize)
    {
        return None;
    }
    let (second, nanoseconds) = if second == 60 {
        (59, 1_000_000_000)
    } else {
        (second, 0)
    };
    let local = date.with_hms(hour, minute, second).ok()?;
    UtcTimestamp::from_timestamp(
        local.assume_offset(iso_offset).unix_timestamp(),
        nanoseconds,
    )
}

fn http_date_digits<'a>(input: &mut &'a str, min: usize, max: usize) -> Option<&'a str> {
    let len = input
        .bytes()
        .take(max)
        .take_while(u8::is_ascii_digit)
        .count();
    if len < min {
        return None;
    }
    let (digits, rest) = input.split_at(len);
    *input = rest;
    Some(digits)
}

fn http_date_space(input: &mut &str) -> Option<()> {
    let trimmed = input.trim_start();
    if trimmed.len() == input.len() {
        return None;
    }
    *input = trimmed;
    Some(())
}

fn http_date_offset(input: &mut &str) -> Option<time::UtcOffset> {
    let name_len = input.bytes().take_while(u8::is_ascii_alphabetic).count();
    if name_len != 0 {
        let (name, rest) = input.split_at(name_len);
        *input = rest;
        let hours = match name.to_ascii_uppercase().as_str() {
            "GMT" | "UT" | "Z" => 0,
            "EDT" => -4,
            "EST" | "CDT" => -5,
            "CST" | "MDT" => -6,
            "MST" | "PDT" => -7,
            "PST" => -8,
            _ if name.len() == 1 && !name.eq_ignore_ascii_case("J") => 0,
            _ => return None,
        };
        return time::UtcOffset::from_hms(hours, 0, 0).ok();
    }

    let sign = if let Some(rest) = input.strip_prefix('+') {
        *input = rest;
        1
    } else {
        *input = input.strip_prefix('-')?;
        -1
    };
    let hour: i8 = http_date_digits(input, 2, 2)?.parse().ok()?;
    let minute: i8 = http_date_digits(input, 2, 2)?.parse().ok()?;
    if hour > 23 {
        return None;
    }
    time::UtcOffset::from_hms(sign * hour, sign * minute, 0).ok()
}

fn strip_http_date_comment(input: &str) -> Option<&str> {
    let input = input.strip_prefix('(')?;
    let mut depth = 1;
    let mut escaped = false;
    for (index, byte) in input.bytes().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match byte {
            b'\\' => escaped = true,
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return input.get(index + 1..);
                }
            }
            _ => {}
        }
    }
    None
}
