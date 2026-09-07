// SPDX-License-Identifier: MIT

/// Parses the closed recovery timestamp form into the store's millisecond audit value. The
/// wire permits up to nanosecond precision; the durable store intentionally retains milliseconds,
/// so additional fractional digits are truncated after validating the complete timestamp.
pub(super) fn parse_timestamp_millis(value: &str) -> Option<u64> {
    let bytes = value.as_bytes();
    if !(20..=30).contains(&bytes.len())
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes.last() != Some(&b'Z')
    {
        return None;
    }
    let year = parse_digits(&bytes[0..4])? as i64;
    let month = parse_digits(&bytes[5..7])?;
    let day = parse_digits(&bytes[8..10])?;
    let hour = parse_digits(&bytes[11..13])?;
    let minute = parse_digits(&bytes[14..16])?;
    let second = parse_digits(&bytes[17..19])?;
    if !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let fraction = if bytes[19] == b'Z' {
        if bytes.len() != 20 {
            return None;
        }
        0
    } else {
        if bytes[19] != b'.' || bytes.len() < 22 {
            return None;
        }
        let digits = &bytes[20..bytes.len() - 1];
        if digits.is_empty() || digits.len() > 9 || !digits.iter().all(u8::is_ascii_digit) {
            return None;
        }
        let millis_digits = digits.len().min(3);
        let mut millis = parse_digits(&digits[..millis_digits])?;
        if millis_digits == 1 {
            millis *= 100;
        } else if millis_digits == 2 {
            millis *= 10;
        }
        millis
    };
    let days = days_from_civil(year, month, day)?;
    if days < 0 {
        return None;
    }
    let seconds = (days as u64)
        .checked_mul(86_400)?
        .checked_add((hour as u64) * 3_600)?
        .checked_add((minute as u64) * 60)?
        .checked_add(second as u64)?;
    seconds.checked_mul(1_000)?.checked_add(fraction as u64)
}

fn parse_digits(bytes: &[u8]) -> Option<u32> {
    (!bytes.is_empty() && bytes.iter().all(u8::is_ascii_digit)).then(|| {
        bytes
            .iter()
            .fold(0_u32, |value, digit| value * 10 + u32::from(digit - b'0'))
    })
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn days_from_civil(year: i64, month: u32, day: u32) -> Option<i64> {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = i64::from(month) + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era.checked_mul(146_097)?
        .checked_add(day_of_era)?
        .checked_sub(719_468)
}
