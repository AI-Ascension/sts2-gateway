// SPDX-License-Identifier: MIT

//! Rust source scanning for module reachability: `mod` declarations (with their
//! attributes and inline nesting) and `include!` targets.

use crate::modules::Declaration;
use crate::rust_text::{is_ident_continue, is_ident_start, skip_space, strip};

/// Every file module declaration in `text`, each carrying the inline module
/// names that enclose it and any `#[path = "..."]` it names.
pub(crate) fn declarations(text: &str) -> Vec<Declaration> {
    let code = strip(text, true);
    let kept = strip(text, false);
    let bytes = code.as_bytes();
    let mut found = Vec::new();
    let mut depth = 0usize;
    let mut stack: Vec<(usize, String)> = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'{' => {
                depth += 1;
                index += 1;
            }
            b'}' => {
                if stack.last().is_some_and(|(level, _)| *level == depth) {
                    stack.pop();
                }
                depth = depth.saturating_sub(1);
                index += 1;
            }
            byte if is_ident_start(byte) => {
                let start = index;
                while index < bytes.len() && is_ident_continue(bytes[index]) {
                    index += 1;
                }
                if &code[start..index] != "mod" {
                    continue;
                }
                let Some((name, body, next)) = read_module(&code, index) else {
                    continue;
                };
                let attributes = attribute_text(&kept, start);
                if body {
                    stack.push((depth + 1, name.clone()));
                }
                found.push(Declaration {
                    name,
                    inline: stack.iter().map(|(_, name)| name.clone()).collect(),
                    paths: attribute_paths(&attributes),
                    conditional: attributes.contains("cfg_attr"),
                });
                index = next;
            }
            _ => index += 1,
        }
    }
    found
}

/// Every string-literal `include!("...")` target named in `text`.
pub(crate) fn include_paths(text: &str) -> Vec<String> {
    let text = strip(text, false);
    let bytes = text.as_bytes();
    let mut paths = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if !is_ident_start(bytes[index]) {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && is_ident_continue(bytes[index]) {
            index += 1;
        }
        if &text[start..index] != "include" {
            continue;
        }
        let mut cursor = skip_space(bytes, index);
        if bytes.get(cursor) != Some(&b'!') {
            continue;
        }
        cursor = skip_space(bytes, cursor + 1);
        if bytes.get(cursor) != Some(&b'(') {
            continue;
        }
        cursor = skip_space(bytes, cursor + 1);
        if bytes.get(cursor) != Some(&b'"') {
            continue;
        }
        let value = cursor + 1;
        let mut close = value;
        while close < bytes.len() && bytes[close] != b'"' && bytes[close] != b'\n' {
            close += 1;
        }
        if bytes.get(close) == Some(&b'"') {
            paths.push(text[value..close].to_owned());
            index = close + 1;
        }
    }
    paths
}

/// Reads `mod NAME;` or `mod NAME {` starting after the `mod` keyword.
fn read_module(code: &str, mut index: usize) -> Option<(String, bool, usize)> {
    let bytes = code.as_bytes();
    index = skip_space(bytes, index);
    if !bytes.get(index).is_some_and(|byte| is_ident_start(*byte)) {
        return None;
    }
    let start = index;
    while index < bytes.len() && is_ident_continue(bytes[index]) {
        index += 1;
    }
    let name = code[start..index].to_owned();
    index = skip_space(bytes, index);
    match bytes.get(index) {
        Some(b'{') => Some((name, true, index)),
        Some(b';') => Some((name, false, index)),
        _ => None,
    }
}

/// Collects the attribute groups immediately preceding `end`, skipping any
/// `pub(...)` or `unsafe` qualifiers between them and the item.
fn attribute_text(code: &str, mut end: usize) -> String {
    let bytes = code.as_bytes();
    let mut groups = Vec::new();
    loop {
        while end > 0 && bytes[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
        if end == 0 {
            break;
        }
        if bytes[end - 1] == b']' {
            let Some(open) = matching(code, end - 1, b'[', b']') else {
                break;
            };
            groups.push(code[open + 1..end - 1].to_owned());
            end = open;
        } else if bytes[end - 1] == b')' {
            let Some(open) = matching(code, end - 1, b'(', b')') else {
                break;
            };
            let mut word = open;
            while word > 0 && bytes[word - 1].is_ascii_whitespace() {
                word -= 1;
            }
            let keyword = word;
            while word > 0 && is_ident_continue(bytes[word - 1]) {
                word -= 1;
            }
            if &code[word..keyword] != "pub" {
                break;
            }
            end = word;
        } else {
            let mut word = end;
            while word > 0 && is_ident_continue(bytes[word - 1]) {
                word -= 1;
            }
            if matches!(&code[word..end], "pub" | "unsafe") {
                end = word;
            } else {
                break;
            }
        }
    }
    groups.reverse();
    groups.join("\n")
}

/// Finds the `[`/`(` matching the closer at `close`.
fn matching(code: &str, close: usize, open: u8, shut: u8) -> Option<usize> {
    let bytes = code.as_bytes();
    let mut depth = 0usize;
    let mut index = close + 1;
    while index > 0 {
        index -= 1;
        match bytes[index] {
            byte if byte == shut => depth += 1,
            byte if byte == open => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

/// Every `path = "..."` value in an attribute group, including one nested in
/// `#[cfg_attr(..., path = "...")]`.
fn attribute_paths(attributes: &str) -> Vec<String> {
    let bytes = attributes.as_bytes();
    let mut paths = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if !is_ident_start(bytes[index]) {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && is_ident_continue(bytes[index]) {
            index += 1;
        }
        if &attributes[start..index] != "path" {
            continue;
        }
        let mut cursor = skip_space(bytes, index);
        if bytes.get(cursor) != Some(&b'=') {
            continue;
        }
        cursor = skip_space(bytes, cursor + 1);
        if bytes.get(cursor) != Some(&b'"') {
            continue;
        }
        let value = cursor + 1;
        let mut close = value;
        while close < bytes.len() && bytes[close] != b'"' {
            close += 1;
        }
        if bytes.get(close) == Some(&b'"') {
            paths.push(attributes[value..close].to_owned());
            index = close + 1;
        }
    }
    paths
}
