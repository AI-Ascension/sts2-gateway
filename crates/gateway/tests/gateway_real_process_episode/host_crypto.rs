// SPDX-License-Identifier: MIT

//! HCJ1 canonicalization and HMAC-SHA256 proofs for the signed host fake.
//!
//! The gateway authenticates a host lease-control acknowledgment with
//! `HMAC-SHA256(key, domain || 0x00 || HCJ1(frame without auth.proof))`, so the
//! fake has to reproduce both the canonicalization and the MAC exactly. Both are
//! re-derived here from the contract rather than borrowed from the gateway
//! binary, whose implementations are bin-private.

use serde_json::Value;
use sha2::{Digest, Sha256};

/// `HMAC-SHA256(key, domain || 0x00 || HCJ1(frame))` as lower hex.
pub(crate) fn proof_for(frame: &Value, domain: &str, key: &[u8]) -> String {
    let mut protected = frame.clone();
    if let Some(auth) = protected["auth"].as_object_mut() {
        auth.remove("proof");
    }
    let canonical = canonical_hcj1(&protected);
    let mut message = Vec::with_capacity(domain.len() + 1 + canonical.len());
    message.extend_from_slice(domain.as_bytes());
    message.push(0);
    message.extend_from_slice(&canonical);
    hex(&hmac_sha256(key, &message))
}

/// HCJ1: object members emit in ascending byte order of the raw key, numbers are
/// unsigned integers, and strings escape only the forms the contract can carry.
pub(crate) fn canonical_hcj1(value: &Value) -> Vec<u8> {
    let mut output = Vec::new();
    write_canonical(value, &mut output);
    output
}

fn write_canonical(value: &Value, output: &mut Vec<u8>) {
    match value {
        Value::Null => output.extend_from_slice(b"null"),
        Value::Bool(true) => output.extend_from_slice(b"true"),
        Value::Bool(false) => output.extend_from_slice(b"false"),
        Value::Number(number) => {
            let text = number.to_string();
            output.extend_from_slice(text.as_bytes());
        }
        Value::String(text) => write_string(text, output),
        Value::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_canonical(value, output);
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
                write_string(key, output);
                output.push(b':');
                write_canonical(&values[*key], output);
            }
            output.push(b'}');
        }
    }
}

fn write_string(value: &str, output: &mut Vec<u8>) {
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
            control if control <= '\u{1f}' => {
                let code = control as u32;
                output.extend_from_slice(br#"\u00"#);
                output.push(b"0123456789abcdef"[((code >> 4) & 0x0f) as usize]);
                output.push(b"0123456789abcdef"[(code & 0x0f) as usize]);
            }
            printable => {
                let mut buffer = [0_u8; 4];
                output.extend_from_slice(printable.encode_utf8(&mut buffer).as_bytes());
            }
        }
    }
    output.push(b'"');
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut normalized = [0_u8; 64];
    let key = &key[..key.len().min(64)];
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

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(b"0123456789abcdef"[(byte >> 4) as usize]));
        text.push(char::from(b"0123456789abcdef"[(byte & 0x0f) as usize]));
    }
    text
}
