// SPDX-License-Identifier: MIT

//! Lexical helpers for locating module declarations in Rust source text.
//!
//! These read source text only: they find `mod` items, read the attributes and
//! visibility attached to them, and name the file each one resolves to. Deciding
//! whether a resolved file is *reachable* is `scan`'s job.

use std::path::{Path, PathBuf};

fn parent_of(path: &Path) -> PathBuf {
    path.parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// Remove comments so commented-out declarations do not count as reachable.
pub(super) fn strip_comments(text: &str) -> String {
    let mut stripped = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    let mut in_block = false;
    while let Some(character) = characters.next() {
        if in_block {
            if character == '*' && characters.peek() == Some(&'/') {
                characters.next();
                in_block = false;
            }
        } else if character == '/' && characters.peek() == Some(&'/') {
            for next in characters.by_ref() {
                if next == '\n' {
                    stripped.push('\n');
                    break;
                }
            }
        } else if character == '/' && characters.peek() == Some(&'*') {
            characters.next();
            in_block = true;
        } else {
            stripped.push(character);
        }
    }
    stripped
}

/// Module declarations as `(file, child directory)` pairs, plus `include!` targets.
pub(super) fn declarations(
    text: &str,
    directory: &Path,
    file_directory: &Path,
) -> Vec<(PathBuf, PathBuf)> {
    let mut found = Vec::new();
    let mut search = 0;
    while let Some(offset) = text[search..].find("mod") {
        let at = search + offset;
        search = at + 3;
        if !starts_word(text, at) || !text[at..].starts_with("mod") {
            continue;
        }
        let Some(declaration) = parse_declaration(text, at + 3) else {
            continue;
        };
        // Attributes precede the visibility (`#[path = "x.rs"] pub(super) mod x;`),
        // so the scan must start ahead of the whole declaration rather than at the
        // `mod` keyword; missing them marks an entire subtree unreachable.
        let declared = path_attributes(&attribute_text(text, declaration_start(text, at)));
        // A `#[path]` value is relative to the file carrying the attribute, and the
        // loaded module's children resolve beside the loaded file -- not inside a
        // directory named after it.
        let mut candidates: Vec<(PathBuf, PathBuf)> = if declared.is_empty() {
            declaration
                .candidates(directory)
                .into_iter()
                .map(|target| {
                    let child = directory.join(&declaration.name);
                    (target, child)
                })
                .collect()
        } else {
            declared
                .into_iter()
                .map(|relative| file_directory.join(relative))
                .map(|target| {
                    let child = parent_of(&target);
                    (target, child)
                })
                .collect()
        };
        if let Some(body) = declaration.body.as_deref() {
            candidates.extend(declarations(
                body,
                &directory.join(&declaration.name),
                file_directory,
            ));
        }
        found.extend(candidates);
    }
    found.extend(include_targets(text, directory, file_directory));
    found
}

struct Declaration {
    name: String,
    body: Option<String>,
    inline_directory: bool,
}

impl Declaration {
    /// rustc prefers `name.rs` and falls back to `name/mod.rs`. Both are offered
    /// here and the caller keeps whichever exists; offering both keeps the check
    /// conservative, since a missed candidate would read as a false orphan.
    fn candidates(&self, directory: &Path) -> Vec<PathBuf> {
        if self.inline_directory {
            return Vec::new();
        }
        vec![
            directory.join(format!("{}.rs", self.name)),
            directory.join(&self.name).join("mod.rs"),
        ]
    }
}

fn parse_declaration(text: &str, after: usize) -> Option<Declaration> {
    let rest = &text[after..];
    let trimmed = rest.trim_start();
    if trimmed.len() == rest.len() {
        return None;
    }
    let trimmed = trimmed.strip_prefix("r#").unwrap_or(trimmed);
    let length = trimmed
        .find(|character: char| !(character.is_alphanumeric() || character == '_'))
        .unwrap_or(trimmed.len());
    if length == 0 || trimmed.starts_with(|character: char| character.is_numeric()) {
        return None;
    }
    let name = trimmed[..length].to_owned();
    let tail = trimmed[length..].trim_start();
    if tail.starts_with(';') {
        return Some(Declaration {
            name,
            body: None,
            inline_directory: false,
        });
    }
    let body = tail.strip_prefix('{')?;
    Some(Declaration {
        name,
        body: Some(braced(body)),
        inline_directory: true,
    })
}

/// The text up to the brace closing an inline module body.
fn braced(text: &str) -> String {
    let mut depth = 1;
    for (index, character) in text.char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return text[..index].to_owned();
                }
            }
            _ => {}
        }
    }
    text.to_owned()
}

/// The contiguous attribute block immediately before `at`, including every
/// stacked group.
pub(super) fn attribute_text(text: &str, at: usize) -> String {
    let bytes = text.as_bytes();
    let mut end = skip_space_back(bytes, at);
    let mut start = end;
    loop {
        let probe = skip_space_back(bytes, end);
        if probe == 0 || bytes[probe - 1] != b']' {
            break;
        }
        let Some(opening) = attribute_opening(bytes, probe) else {
            break;
        };
        let before = skip_space_back(bytes, opening);
        if before == 0 || bytes[before - 1] != b'#' {
            break;
        }
        start = before - 1;
        end = start;
    }
    text[start..at].to_owned()
}

/// Index of the `[` matching the `]` at `close`.
fn attribute_opening(bytes: &[u8], close: usize) -> Option<usize> {
    matching_open(bytes, close, b'[', b']')
}

/// Index of the `open` byte matching the `close` byte at `close_index`.
fn matching_open(bytes: &[u8], close_index: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 0;
    let mut index = close_index;
    while index > 0 {
        index -= 1;
        match bytes[index] {
            byte if byte == close => depth += 1,
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

fn skip_space_back(bytes: &[u8], from: usize) -> usize {
    let mut index = from;
    while index > 0 && bytes[index - 1].is_ascii_whitespace() {
        index -= 1;
    }
    index
}

/// `path = "..."` values, including those nested in `cfg_attr(...)`.
pub(super) fn path_attributes(attributes: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut search = 0;
    while let Some(offset) = attributes[search..].find("path") {
        let at = search + offset;
        search = at + 4;
        if !starts_word(attributes, at) {
            continue;
        }
        let Some(rest) = attributes[at + 4..].trim_start().strip_prefix('=') else {
            continue;
        };
        if let Some(value) = quoted(rest.trim_start()) {
            found.push(PathBuf::from(value));
        }
    }
    found
}

fn include_targets(text: &str, directory: &Path, file_directory: &Path) -> Vec<(PathBuf, PathBuf)> {
    let mut found = Vec::new();
    let mut search = 0;
    while let Some(offset) = text[search..].find("include!") {
        let at = search + offset;
        search = at + 8;
        if !starts_word(text, at) {
            continue;
        }
        let Some(rest) = text[at + 8..].trim_start().strip_prefix('(') else {
            continue;
        };
        if let Some(value) = quoted(rest.trim_start()) {
            // `include!` shares the includer's child module directory.
            found.push((file_directory.join(value), directory.to_path_buf()));
        }
    }
    found
}

fn quoted(text: &str) -> Option<String> {
    let rest = text.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_owned())
}

fn starts_word(text: &str, at: usize) -> bool {
    !text[..at]
        .chars()
        .next_back()
        .is_some_and(|character| character.is_alphanumeric() || character == '_')
}

/// Walk back over an optional visibility (`pub`, `pub(crate)`, `pub(super)`) so the
/// attribute block preceding the whole declaration is found.
fn declaration_start(text: &str, mod_keyword: usize) -> usize {
    let bytes = text.as_bytes();
    let after_visibility = skip_space_back(bytes, mod_keyword);
    let Some(start) = visibility_start(text, after_visibility) else {
        return mod_keyword;
    };
    skip_space_back(bytes, start)
}

/// Index where a visibility restriction ending at `end` begins, if one is there.
fn visibility_start(text: &str, end: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if end == 0 {
        return None;
    }
    let keyword_end = if bytes[end - 1] == b')' {
        // The visibility parentheses are `(`/`)`, not the attribute brackets.
        let opening = matching_open(bytes, end, b'(', b')')?;
        skip_space_back(bytes, opening)
    } else {
        end
    };
    if keyword_end < 3 || &text[keyword_end - 3..keyword_end] != "pub" {
        return None;
    }
    if !starts_word(text, keyword_end - 3) {
        return None;
    }
    Some(keyword_end - 3)
}
