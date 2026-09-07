// SPDX-License-Identifier: MIT

//! Canonicalization, proof, and scalar validation for host lease-control frames.

use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use sts2_gateway::MAX_WIRE_INTEGER;

use super::host_lease_control::HostLeaseFrameError;

pub(super) fn grant_digest(grant: &Value) -> Result<String, HostLeaseFrameError> {
    let canonical = canonical_hcj1(grant)?;
    Ok(hex_digest(&canonical))
}

pub(super) fn canonical_hcj1(value: &Value) -> Result<Vec<u8>, HostLeaseFrameError> {
    let mut output = Vec::new();
    write_canonical(value, &mut output)?;
    Ok(output)
}

pub(super) fn proof_for_frame(frame: &Value, domain: &str, secret: &[u8]) -> String {
    let mut protected = frame.clone();
    if let Some(auth) = protected["auth"].as_object_mut() {
        auth.remove("proof");
    }
    let canonical = canonical_hcj1(&protected).unwrap_or_default();
    let mut message = Vec::with_capacity(domain.len() + 1 + canonical.len());
    message.extend_from_slice(domain.as_bytes());
    message.push(0);
    message.extend_from_slice(&canonical);
    hex_digest(&hmac_sha256(secret, &message))
}

pub(super) fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut normalized = [0_u8; 64];
    normalized[..key.len()].copy_from_slice(key);
    let mut inner = [0_u8; 64];
    let mut outer = [0_u8; 64];
    for index in 0..64 {
        inner[index] = normalized[index] ^ 0x36;
        outer[index] = normalized[index] ^ 0x5c;
    }
    let mut inner_hasher = Sha256::new();
    inner_hasher.update(inner);
    inner_hasher.update(message);
    let inner_digest = inner_hasher.finalize();
    let mut outer_hasher = Sha256::new();
    outer_hasher.update(outer);
    outer_hasher.update(inner_digest);
    let digest = outer_hasher.finalize();
    let mut result = [0_u8; 32];
    result.copy_from_slice(&digest);
    result
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(char::from(b"0123456789abcdef"[(byte >> 4) as usize]));
        result.push(char::from(b"0123456789abcdef"[(byte & 0x0f) as usize]));
    }
    result
}

fn write_canonical(value: &Value, output: &mut Vec<u8>) -> Result<(), HostLeaseFrameError> {
    match value {
        Value::Null => output.extend_from_slice(b"null"),
        Value::Bool(value) => output.extend_from_slice(if *value { b"true" } else { b"false" }),
        Value::Number(number) => {
            let value = number.as_u64().ok_or(HostLeaseFrameError::Invalid)?;
            if value > MAX_WIRE_INTEGER {
                return Err(HostLeaseFrameError::Invalid);
            }
            output.extend_from_slice(value.to_string().as_bytes());
        }
        Value::String(value) => write_string(value, output)?,
        Value::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_canonical(value, output)?;
            }
            output.push(b']');
        }
        Value::Object(values) => {
            let mut keys: Vec<&String> = values.keys().collect();
            keys.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
            output.push(b'{');
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_string(key, output)?;
                output.push(b':');
                write_canonical(&values[*key], output)?;
            }
            output.push(b'}');
        }
    }
    Ok(())
}

fn write_string(value: &str, output: &mut Vec<u8>) -> Result<(), HostLeaseFrameError> {
    output.push(b'"');
    for character in value.chars() {
        match character {
            '"' => output.extend_from_slice(br#"\""#),
            '\\' => output.extend_from_slice(br#"\\"#),
            '\u{08}' => output.extend_from_slice(br#"\b"#),
            '\u{0c}' => output.extend_from_slice(br#"\f"#),
            '\n' => output.extend_from_slice(br#"\n"#),
            '\r' => output.extend_from_slice(br#"\r"#),
            '\t' => output.extend_from_slice(br#"\t"#),
            character if character <= '\u{1f}' => {
                let value = character as u32;
                output.extend_from_slice(br#"\u00"#);
                output.push(hex_digit((value >> 4) as u8));
                output.push(hex_digit(value as u8));
            }
            character => output.extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes()),
        }
    }
    output.push(b'"');
    Ok(())
}

fn hex_digit(value: u8) -> u8 {
    b"0123456789abcdef"[(value & 0x0f) as usize]
}

pub(super) fn valid_token(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

pub(super) fn valid_uuid(value: &str) -> bool {
    Uuid::parse_str(value).ok().is_some_and(|id| {
        id.hyphenated().to_string() == value && id.get_variant() == uuid::Variant::RFC4122
    })
}

pub(super) fn valid_uuid_v4(value: &str) -> bool {
    Uuid::parse_str(value).ok().is_some_and(|id| {
        id.hyphenated().to_string() == value
            && id.get_variant() == uuid::Variant::RFC4122
            && id.get_version_num() == 4
    })
}

pub(super) fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub(super) fn valid_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    (20..=30).contains(&bytes.len())
        && bytes.get(4) == Some(&b'-')
        && bytes.get(7) == Some(&b'-')
        && bytes.get(10) == Some(&b'T')
        && bytes.get(13) == Some(&b':')
        && bytes.get(16) == Some(&b':')
        && bytes.last() == Some(&b'Z')
        && bytes.iter().enumerate().all(|(index, byte)| {
            if matches!(index, 4 | 7 | 10 | 13 | 16) || index == bytes.len() - 1 {
                true
            } else {
                byte.is_ascii_digit() || (*byte == b'.' && (19..bytes.len() - 1).contains(&index))
            }
        })
}

pub(super) fn parse_timestamp_millis(value: &str) -> Option<u64> {
    if !valid_timestamp(value) {
        return None;
    }
    let bytes = value.as_bytes();
    let year = value.get(0..4)?.parse::<u64>().ok()?;
    let month = value.get(5..7)?.parse::<u64>().ok()?;
    let day = value.get(8..10)?.parse::<u64>().ok()?;
    let hour = value.get(11..13)?.parse::<u64>().ok()?;
    let minute = value.get(14..16)?.parse::<u64>().ok()?;
    let second = value.get(17..19)?.parse::<u64>().ok()?;
    if !(1..=12).contains(&month) || day == 0 || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let max_day = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if day > max_day {
        return None;
    }
    let fraction = if bytes.len() == 20 {
        0
    } else {
        let fraction = value.get(20..bytes.len() - 1)?;
        let mut millis = fraction.parse::<u64>().ok()?;
        for _ in fraction.len()..3 {
            millis = millis.checked_mul(10)?;
        }
        if fraction.len() > 3 {
            millis /= 10_u64.pow((fraction.len() - 3) as u32);
        }
        millis
    };
    let days = days_from_civil(year, month, day)?;
    days.checked_mul(86_400_000)?
        .checked_add(hour * 3_600_000)?
        .checked_add(minute * 60_000)?
        .checked_add(second * 1_000)?
        .checked_add(fraction)
}

fn days_from_civil(year: u64, month: u64, day: u64) -> Option<u64> {
    let year = i64::try_from(year).ok()? - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i64::try_from(month).ok()?;
    let day = i64::try_from(day).ok()?;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    u64::try_from(days).ok()
}

#[allow(dead_code)]
pub(super) fn timestamp_for_grant(millis: u64) -> String {
    super::recovery_frame::timestamp_from_millis(millis)
}
