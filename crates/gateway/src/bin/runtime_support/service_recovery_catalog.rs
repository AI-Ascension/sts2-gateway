// SPDX-License-Identifier: MIT

use serde_json::{Value, value::RawValue};
use std::collections::BTreeMap;
use sts2_gateway::{RecoveryEffectWitness, RecoveryLease, RecoveryLeaseProof, sha256_hex};

use super::{RuntimeService, RuntimeV3GameplayRoute};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RecoveryCatalogKey {
    pub(super) instance_id: String,
    pub(super) instance_incarnation: String,
    pub(super) session_id: String,
    pub(super) lease_id: String,
    pub(super) lease_epoch: u64,
    pub(super) state_id: String,
    pub(super) gameplay_generation: u64,
}

#[derive(Debug, Default)]
pub(super) struct RecoveryCatalogCache {
    current: Option<RecoveryCatalog>,
    observed: Option<RecoveryCatalogKey>,
    // Consumed boundary awaiting a fresh authoritative observation.
    invalidated: Option<RecoveryCatalogKey>,
}

#[derive(Debug)]
struct RecoveryCatalog {
    key: RecoveryCatalogKey,
    raw_legal_actions: Vec<u8>,
    digest: String,
    actions: Vec<Value>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RecoveryCatalogAdmission {
    Missing,
    Stale,
    ActionNotCurrent,
}

impl RecoveryCatalogCache {
    pub(super) fn capture(&mut self, key: RecoveryCatalogKey, body: &[u8]) -> bool {
        let Some((response, raw, actions)) = extract_catalog(body) else {
            return false;
        };
        if response["kind"] != RuntimeV3GameplayRoute::LegalActions.response_kind()
            || !response_matches_key(&response, &key)
            || response["state_id"].as_str() != Some(key.state_id.as_str())
            || response["generation"].as_u64() != Some(key.gameplay_generation)
            || self.invalidated.as_ref() == Some(&key)
        {
            return false;
        }
        if !self.advance_observation(&key) {
            return false;
        }
        let digest = sha256_hex(&raw);
        self.current = Some(RecoveryCatalog {
            key,
            raw_legal_actions: raw,
            digest,
            actions,
        });
        true
    }

    pub(super) fn observe(&mut self, key: RecoveryCatalogKey) -> bool {
        let accepted = self.advance_observation(&key);
        if accepted && self.invalidated.as_ref() == Some(&key) {
            self.invalidated = None;
        }
        accepted
    }

    pub(super) fn invalidate_current(&mut self) {
        self.invalidated = self.current.take().map(|c| c.key).or(self.observed.clone());
    }

    pub(super) fn admission(
        &self,
        key: &RecoveryCatalogKey,
        action: &Value,
    ) -> Result<&str, RecoveryCatalogAdmission> {
        let Some(catalog) = self.current.as_ref() else {
            return Err(RecoveryCatalogAdmission::Missing);
        };
        if catalog.key != *key {
            return Err(RecoveryCatalogAdmission::Stale);
        }
        if !catalog.actions.iter().any(|candidate| candidate == action) {
            return Err(RecoveryCatalogAdmission::ActionNotCurrent);
        }
        debug_assert_eq!(catalog.digest, sha256_hex(&catalog.raw_legal_actions));
        Ok(catalog.digest.as_str())
    }

    #[cfg(test)]
    pub(super) fn current(&self) -> Option<(&RecoveryCatalogKey, &[u8], &str)> {
        self.current.as_ref().map(|catalog| {
            (
                &catalog.key,
                catalog.raw_legal_actions.as_slice(),
                catalog.digest.as_str(),
            )
        })
    }

    fn advance_observation(&mut self, key: &RecoveryCatalogKey) -> bool {
        if let Some(observed) = self.observed.as_ref() {
            if !same_authority_context(observed, key) {
                self.current = None;
            } else if key.gameplay_generation < observed.gameplay_generation
                || (key.gameplay_generation == observed.gameplay_generation
                    && key.state_id != observed.state_id)
            {
                return false;
            } else if key.gameplay_generation > observed.gameplay_generation {
                self.current = None;
            }
        }

        self.observed = Some(key.clone());
        true
    }
}

impl RuntimeService {
    pub(super) fn invalidate_recovery_catalog(&mut self) {
        self.recovery_catalog.invalidate_current();
    }

    pub(super) fn observe_recovery_witness(
        &mut self,
        proof: &RecoveryLeaseProof,
        witness: &RecoveryEffectWitness,
    ) -> bool {
        let Some(lease) = self.recovery_lease.as_ref() else {
            return false;
        };
        if lease.proof() != *proof {
            return false;
        }
        self.recovery_catalog.observe(RecoveryCatalogKey {
            instance_id: self.config.instance_id.clone(),
            instance_incarnation: proof.instance_incarnation.clone(),
            session_id: self.config.session_id.clone(),
            lease_id: proof.lease_id.clone(),
            lease_epoch: proof.lease_epoch,
            state_id: witness.state_id.clone(),
            gameplay_generation: witness.generation,
        })
    }

    pub(super) fn capture_recovery_catalog(
        &mut self,
        lease: &RecoveryLease,
        status: u16,
        body: &[u8],
    ) -> bool {
        if status != 200 {
            return false;
        }
        let Some(response) = super::super::strict_json::parse(body).ok() else {
            return false;
        };
        let Some(key) = self.recovery_catalog_key(lease, &response) else {
            return false;
        };
        self.recovery_catalog.capture(key, body)
    }

    pub(super) fn observe_recovery_catalog(
        &mut self,
        lease: &RecoveryLease,
        route: RuntimeV3GameplayRoute,
        status: u16,
        body: &[u8],
    ) -> bool {
        if status != 200 {
            return true;
        }
        let Some(response) = super::super::strict_json::parse(body).ok() else {
            return false;
        };
        if !authoritative_observation(route, &response) {
            return true;
        }
        let Some(key) = self.recovery_catalog_key(lease, &response) else {
            return false;
        };
        self.recovery_catalog.observe(key)
    }

    pub(super) fn recovery_catalog_admission(
        &self,
        lease: &RecoveryLease,
        envelope: &Value,
    ) -> Result<String, RecoveryCatalogAdmission> {
        let Some(key) = self.recovery_catalog_key(lease, envelope) else {
            return Err(RecoveryCatalogAdmission::Missing);
        };
        let action = &envelope["action"];
        if !action.is_object() {
            return Err(RecoveryCatalogAdmission::ActionNotCurrent);
        }
        self.recovery_catalog
            .admission(&key, action)
            .map(str::to_owned)
    }

    fn recovery_catalog_key(
        &self,
        lease: &RecoveryLease,
        value: &Value,
    ) -> Option<RecoveryCatalogKey> {
        let state_id = value["state_id"].as_str()?.to_owned();
        let gameplay_generation = value["generation"].as_u64()?;
        (value["instance_id"].as_str() == Some(self.config.instance_id.as_str())
            && value["session_id"].as_str() == Some(self.config.session_id.as_str())
            && value["lease_id"].as_str() == Some(lease.lease_id.as_str())
            && value["lease_epoch"].as_u64() == Some(lease.lease_epoch)
            && lease.instance_id == self.config.instance_id)
            .then_some(RecoveryCatalogKey {
                instance_id: self.config.instance_id.clone(),
                instance_incarnation: lease.instance_incarnation.clone(),
                session_id: self.config.session_id.clone(),
                lease_id: lease.lease_id.clone(),
                lease_epoch: lease.lease_epoch,
                state_id,
                gameplay_generation,
            })
    }
}

fn response_matches_key(response: &Value, key: &RecoveryCatalogKey) -> bool {
    response["instance_id"].as_str() == Some(key.instance_id.as_str())
        && response["session_id"].as_str() == Some(key.session_id.as_str())
        && response["lease_id"].as_str() == Some(key.lease_id.as_str())
        && response["lease_epoch"].as_u64() == Some(key.lease_epoch)
}

fn same_authority_context(left: &RecoveryCatalogKey, right: &RecoveryCatalogKey) -> bool {
    left.instance_id == right.instance_id
        && left.instance_incarnation == right.instance_incarnation
        && left.session_id == right.session_id
        && left.lease_id == right.lease_id
        && left.lease_epoch == right.lease_epoch
}

fn authoritative_observation(route: RuntimeV3GameplayRoute, response: &Value) -> bool {
    if !response["observation"].is_object() {
        return false;
    }
    match route {
        RuntimeV3GameplayRoute::State | RuntimeV3GameplayRoute::Reobserve => true,
        RuntimeV3GameplayRoute::DispatchAction
        | RuntimeV3GameplayRoute::WaitForTransition
        | RuntimeV3GameplayRoute::Recover => response["status"].as_str() == Some("settled"),
        RuntimeV3GameplayRoute::LegalActions => false,
    }
}

/// Extract the value bytes without reserializing through `serde_json::Value`.
/// The strict parse first rejects duplicate members and trailing bytes; the
/// `RawValue` map then preserves the host's exact UTF-8 representation.
fn extract_catalog(body: &[u8]) -> Option<(Value, Vec<u8>, Vec<Value>)> {
    let response = super::super::strict_json::parse(body).ok()?;
    let fields = serde_json::from_slice::<BTreeMap<String, Box<RawValue>>>(body).ok()?;
    let raw = fields.get("legal_actions")?;
    let raw_bytes = exact_array_bytes(raw.get())?;
    let actions = serde_json::from_slice::<Vec<Value>>(raw_bytes).ok()?;
    (response["legal_actions"].as_array()?.len() == actions.len()).then_some((
        response,
        raw_bytes.to_vec(),
        actions,
    ))
}

/// Return the exact UTF-8 byte range from the opening array bracket through its
/// matching closing bracket.  `RawValue` normally excludes surrounding JSON
/// whitespace, but doing this explicitly keeps the contract true if the
/// deserializer's representation changes while preserving all inner bytes.
fn exact_array_bytes(raw: &str) -> Option<&[u8]> {
    let bytes = raw.as_bytes();
    let start = bytes.iter().position(|byte| !byte.is_ascii_whitespace())?;
    if bytes[start] != b'[' {
        return None;
    }
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut end = None;
    for (index, byte) in bytes.iter().copied().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'[' => depth = depth.checked_add(1)?,
            b']' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    end = Some(index);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end?;
    bytes[end + 1..]
        .iter()
        .all(u8::is_ascii_whitespace)
        .then_some(&bytes[start..=end])
}
