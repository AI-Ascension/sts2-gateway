// SPDX-License-Identifier: MIT

//! Byte-preserving Rust source masking for module-reachability scanning.
//!
//! `strip` blanks comments (always) and, on request, string and character
//! literal bodies, while preserving byte length so callers can parse structure
//! from masked text and read real attribute values from unmasked text at the
//! same indices.

pub(crate) fn strip(text: &str, blank_strings: bool) -> String {
    let bytes = text.as_bytes();
    let mask = |byte: u8| {
        if blank_strings && byte != b'\n' {
            b' '
        } else {
            byte
        }
    };
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
            while index < bytes.len() && bytes[index] != b'\n' {
                out.push(b' ');
                index += 1;
            }
        } else if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            let mut level = 1usize;
            out.extend_from_slice(b"  ");
            index += 2;
            while index < bytes.len() && level > 0 {
                if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
                    level += 1;
                    out.extend_from_slice(b"  ");
                    index += 2;
                } else if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
                    level -= 1;
                    out.extend_from_slice(b"  ");
                    index += 2;
                } else {
                    out.push(if bytes[index] == b'\n' { b'\n' } else { b' ' });
                    index += 1;
                }
            }
        } else if byte == b'"' {
            out.push(mask(byte));
            index += 1;
            while index < bytes.len() {
                if bytes[index] == b'\\' && index + 1 < bytes.len() {
                    out.push(mask(bytes[index]));
                    out.push(mask(bytes[index + 1]));
                    index += 2;
                } else if bytes[index] == b'"' {
                    out.push(mask(bytes[index]));
                    index += 1;
                    break;
                } else if bytes[index] == b'\n' {
                    break;
                } else {
                    out.push(mask(bytes[index]));
                    index += 1;
                }
            }
        } else if byte == b'\'' {
            let end = char_literal_end(bytes, index);
            let mut cursor = index;
            while cursor <= end {
                out.push(mask(bytes[cursor]));
                cursor += 1;
            }
            index = end + 1;
        } else if let Some((_, end)) = raw_string(bytes, index) {
            while index < end {
                out.push(mask(bytes[index]));
                index += 1;
            }
        } else {
            out.push(byte);
            index += 1;
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

/// Returns the byte index of the `'` closing a character literal at `start`, or
/// `start` itself when the token is a lifetime. A lone `'a` must not be treated
/// as a literal or a real identifier would be masked.
fn char_literal_end(bytes: &[u8], start: usize) -> usize {
    if bytes.get(start + 1) == Some(&b'\\') {
        let mut index = start + 2;
        while index < bytes.len() && index < start + 13 {
            if bytes[index] == b'\'' {
                return index;
            }
            index += 1;
        }
        return start;
    }
    if bytes
        .get(start + 1)
        .is_some_and(|byte| *byte != b'\'' && *byte != b'\n')
    {
        let limit = usize::min(start + 5, bytes.len() - 1);
        let mut cursor = start + 2;
        while cursor <= limit {
            if bytes[cursor] == b'\'' {
                return cursor;
            }
            if bytes[cursor] == b'\n' {
                break;
            }
            cursor += 1;
        }
    }
    start
}

/// Returns `(hash_count, end)` for a raw string starting at `start`, where `end`
/// is one past the closing `"` and its hashes. `r#ident` is not a raw string.
fn raw_string(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    if bytes.get(start) != Some(&b'r') {
        return None;
    }
    let mut index = start + 1;
    let mut hashes = 0usize;
    while bytes.get(index) == Some(&b'#') {
        hashes += 1;
        index += 1;
    }
    if bytes.get(index) != Some(&b'"') {
        return None;
    }
    index += 1;
    while index < bytes.len() {
        if bytes[index] == b'"'
            && (0..hashes).all(|offset| bytes.get(index + 1 + offset) == Some(&b'#'))
        {
            return Some((hashes, index + 1 + hashes));
        }
        index += 1;
    }
    None
}

pub(crate) fn skip_space(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    index
}

pub(crate) fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

pub(crate) fn is_ident_continue(byte: u8) -> bool {
    is_ident_start(byte) || byte.is_ascii_digit()
}
