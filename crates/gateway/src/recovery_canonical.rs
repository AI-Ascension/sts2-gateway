// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;

use serde_json::Value;

use super::recovery_types::{MAX_RECOVERY_ACTION_BYTES, RecoveryStoreError};

/// Validates and returns the exact RCJ-1 bytes accepted for a Runtime-v3 action.
pub fn canonicalize_recovery_action(bytes: &[u8]) -> Result<Vec<u8>, RecoveryStoreError> {
    if bytes.is_empty() || bytes.len() > MAX_RECOVERY_ACTION_BYTES {
        return Err(RecoveryStoreError::InvalidInput(
            "canonical action exceeds the recovery bound".to_owned(),
        ));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| RecoveryStoreError::InvalidInput("action is not UTF-8".to_owned()))?;
    if text.starts_with('\u{feff}') || bytes.iter().any(|byte| *byte >= 0x80) {
        return Err(RecoveryStoreError::InvalidInput(
            "RCJ-1 actions must contain ASCII only".to_owned(),
        ));
    }
    let mut scanner = Scanner::new(bytes);
    scanner.value()?;
    if scanner.position() != bytes.len() {
        return Err(invalid("action has trailing bytes"));
    }
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| invalid("action JSON is malformed"))?;
    validate_action(&value)?;
    let canonical = serde_json::to_vec(&value)
        .map_err(|_| invalid("action JSON could not be canonicalized"))?;
    if canonical != bytes {
        return Err(RecoveryStoreError::ContractMismatch(
            "action bytes are not RCJ-1 canonical".to_owned(),
        ));
    }
    Ok(canonical)
}

fn validate_action(value: &Value) -> Result<(), RecoveryStoreError> {
    let root = object(value, "legal action")?;
    require_keys(root, &["action", "action_id"], "legal action")?;
    let action_id = root
        .get("action_id")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("action_id must be a string"))?;
    validate_action_identity(action_id, "action_id")?;
    let payload = object(
        root.get("action")
            .ok_or_else(|| invalid("legal action has no action payload"))?,
        "action payload",
    )?;
    let kind = payload
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("action kind must be a string"))?;
    let identity_field = match kind {
        "start_run" => Some("character_id"),
        "select_map_node" => Some("node_id"),
        "choose_reward" => Some("reward_id"),
        "shop_purchase" => Some("item_id"),
        "shop_remove" | "smith" | "select_card" => Some("card_id"),
        "event_choice" => Some("choice_id"),
        "play_card" => Some("card_id"),
        "end_turn" | "skip_reward" | "rest" | "confirm_victory" | "save_quit" | "proceed"
        | "confirm_selection" | "cancel_selection" => None,
        _ => {
            return Err(invalid(
                "action kind is not in the frozen Runtime-v3 profile",
            ));
        }
    };
    if kind == "play_card" {
        require_keys(payload, &["card_id", "kind", "target_id"], "play_card")?;
        validate_string(payload, "card_id")?;
        if !payload
            .get("target_id")
            .is_some_and(|target| target.is_null() || target.is_string())
        {
            return Err(invalid("play_card target_id must be a string or null"));
        }
        if let Some(target) = payload.get("target_id").and_then(Value::as_str) {
            validate_action_identity(target, "target_id")?;
        }
    } else if let Some(field) = identity_field {
        require_keys(payload, &[field, "kind"], kind)?;
        validate_string(payload, field)?;
    } else {
        require_keys(payload, &["kind"], kind)?;
    }
    Ok(())
}

fn object<'a>(
    value: &'a Value,
    name: &str,
) -> Result<&'a serde_json::Map<String, Value>, RecoveryStoreError> {
    value
        .as_object()
        .ok_or_else(|| invalid(format!("{name} must be an object")))
}

fn require_keys(
    object: &serde_json::Map<String, Value>,
    expected: &[&str],
    name: &str,
) -> Result<(), RecoveryStoreError> {
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(invalid(format!(
            "{name} contains unknown or missing fields"
        )));
    }
    Ok(())
}

fn validate_string(
    object: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<(), RecoveryStoreError> {
    let value = object
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("{field} must be a string")))?;
    validate_action_identity(value, field)
}

fn validate_action_identity(value: &str, name: &str) -> Result<(), RecoveryStoreError> {
    if value.is_empty()
        || value.len() > 512
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:/".contains(&byte))
    {
        return Err(invalid(format!("{name} is not a Runtime-v3 identity")));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> RecoveryStoreError {
    RecoveryStoreError::InvalidInput(message.into())
}

struct Scanner<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> Scanner<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    const fn position(&self) -> usize {
        self.cursor
    }

    fn value(&mut self) -> Result<(), RecoveryStoreError> {
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'"') => self.string().map(|_| ()),
            Some(b'n') => self.literal(b"null"),
            Some(b't') => self.literal(b"true"),
            Some(b'f') => self.literal(b"false"),
            Some(b'[') => Err(invalid("arrays are not allowed in RCJ-1 actions")),
            Some(b'-' | b'0'..=b'9') => Err(invalid("numbers are not allowed in RCJ-1 actions")),
            _ => Err(invalid("action contains an invalid JSON value")),
        }
    }

    fn object(&mut self) -> Result<(), RecoveryStoreError> {
        self.expect(b'{')?;
        let mut keys = BTreeSet::new();
        if self.take(b'}') {
            return Ok(());
        }
        loop {
            let key = self.string()?;
            if !keys.insert(key) {
                return Err(invalid("duplicate JSON object member"));
            }
            self.expect(b':')?;
            self.value()?;
            if self.take(b'}') {
                return Ok(());
            }
            self.expect(b',')?;
        }
    }

    fn string(&mut self) -> Result<String, RecoveryStoreError> {
        self.expect(b'"')?;
        let start = self.cursor;
        while let Some(byte) = self.peek() {
            match byte {
                b'"' => {
                    let text = std::str::from_utf8(&self.bytes[start..self.cursor])
                        .map_err(|_| invalid("string is not UTF-8"))?;
                    self.cursor += 1;
                    return Ok(text.to_owned());
                }
                b'\\' => return Err(invalid("RCJ-1 does not permit string escapes")),
                0..=0x1f => return Err(invalid("control characters are not allowed")),
                _ => self.cursor += 1,
            }
        }
        Err(invalid("unterminated JSON string"))
    }

    fn literal(&mut self, literal: &[u8]) -> Result<(), RecoveryStoreError> {
        let end = self
            .cursor
            .checked_add(literal.len())
            .ok_or_else(|| invalid("JSON cursor overflow"))?;
        if self.bytes.get(self.cursor..end) != Some(literal) {
            return Err(invalid("invalid JSON literal"));
        }
        self.cursor = end;
        Ok(())
    }

    fn expect(&mut self, byte: u8) -> Result<(), RecoveryStoreError> {
        if self.take(byte) {
            Ok(())
        } else {
            Err(invalid("action JSON punctuation is invalid"))
        }
    }

    fn take(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.cursor).copied()
    }
}
