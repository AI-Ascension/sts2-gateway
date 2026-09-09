// SPDX-License-Identifier: MIT

use sha2::{Digest, Sha256};

/// The native game mode selected for a seeded launch.
#[derive(Clone, Copy, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SeededRunGameMode {
    Standard,
}

/// The player character selected for a seeded launch.
#[derive(Clone, Copy, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SeededRunCharacter {
    Ironclad,
}

/// Whether the selected profile is a pristine baseline or an existing profile.
#[derive(Clone, Copy, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SeededRunProfileKind {
    Fresh,
    Existing,
}

/// Native save behavior for the launched run.
#[derive(Clone, Copy, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SeededRunSavePolicy {
    Disabled,
    Enabled,
}

/// A build or profile identity paired with its content digest.
#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SeededRunIdentityDigest {
    pub identity: String,
    pub digest: String,
}

impl SeededRunIdentityDigest {
    pub(super) fn validate(
        &self,
        error: super::SeededRunValidationError,
    ) -> Result<(), super::SeededRunValidationError> {
        validate_context_text(&self.identity, SEEDED_RUN_MAX_CONTEXT_ID_BYTES, error)?;
        if !is_context_digest(&self.digest) {
            return Err(super::SeededRunValidationError::ContextDigest);
        }
        Ok(())
    }
}

/// A concrete native context bound to one seeded launch operation.
#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SeededRunSelectionContext {
    /// Stable context identity retained across request and host readback.
    pub context_id: String,
    /// Native game mode, deliberately separate from the operation's run mode.
    pub game_mode: SeededRunGameMode,
    /// Character selected in the native standard lobby.
    pub character: SeededRunCharacter,
    /// Ascension level selected in the native lobby.
    pub ascension: u8,
    /// Explicit modifiers in stable order; an empty list means no modifiers.
    pub modifiers: Vec<String>,
    /// Acts included by the selection policy, in native selection order.
    pub acts: Vec<String>,
    /// Policy that gives the meaning of the acts list.
    pub selection_policy: String,
    /// Profile baseline used to make the native context reproducible.
    pub profile_baseline: SeededRunProfileBaseline,
    /// Whether the native run may write its save state.
    pub save_policy: SeededRunSavePolicy,
    /// Game and mod compatibility identities required for this context.
    pub compatibility: SeededRunCompatibility,
    /// SHA-256 of the canonical context fields excluding this member.
    pub context_digest: String,
}

/// Profile identity and content digest used as the launch baseline.
#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SeededRunProfileBaseline {
    pub kind: SeededRunProfileKind,
    pub identity: String,
    pub digest: String,
}

/// Compatibility identities for the game and the installed mod.
#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SeededRunCompatibility {
    pub game: SeededRunIdentityDigest,
    #[serde(rename = "mod")]
    pub mod_: SeededRunIdentityDigest,
}

#[derive(serde::Serialize)]
struct SeededRunContextDigestInput<'a> {
    context_id: &'a str,
    game_mode: SeededRunGameMode,
    character: SeededRunCharacter,
    ascension: u8,
    modifiers: &'a [String],
    acts: &'a [String],
    selection_policy: &'a str,
    profile_baseline: &'a SeededRunProfileBaseline,
    save_policy: SeededRunSavePolicy,
    compatibility: &'a SeededRunCompatibility,
}

impl SeededRunSelectionContext {
    /// Validates the concrete native selection and its content-addressed identity.
    pub fn validate(&self) -> Result<(), super::SeededRunValidationError> {
        validate_context_text(
            &self.context_id,
            SEEDED_RUN_MAX_CONTEXT_ID_BYTES,
            super::SeededRunValidationError::InvalidContext,
        )?;
        if self.ascension > 20 {
            return Err(super::SeededRunValidationError::ContextBounds);
        }
        if self.modifiers.len() > SEEDED_RUN_MAX_MODIFIERS
            || self.acts.is_empty()
            || self.acts.len() > SEEDED_RUN_MAX_ACTS
        {
            return Err(super::SeededRunValidationError::ContextBounds);
        }
        for modifier in &self.modifiers {
            validate_context_text(
                modifier,
                SEEDED_RUN_MAX_CONTEXT_TEXT_BYTES,
                super::SeededRunValidationError::InvalidContext,
            )?;
        }
        for act in &self.acts {
            validate_context_text(
                act,
                SEEDED_RUN_MAX_CONTEXT_TEXT_BYTES,
                super::SeededRunValidationError::InvalidContext,
            )?;
        }
        if self
            .modifiers
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
            || self
                .acts
                .iter()
                .enumerate()
                .any(|(index, act)| self.acts[..index].contains(act))
        {
            return Err(super::SeededRunValidationError::ContextOrdering);
        }
        validate_context_text(
            &self.selection_policy,
            SEEDED_RUN_MAX_CONTEXT_TEXT_BYTES,
            super::SeededRunValidationError::InvalidContext,
        )?;
        self.profile_baseline.validate()?;
        self.compatibility.game.validate(
            super::SeededRunValidationError::InvalidCompatibility,
        )?;
        self.compatibility.mod_.validate(
            super::SeededRunValidationError::InvalidCompatibility,
        )?;
        if !is_context_digest(&self.context_digest) {
            return Err(super::SeededRunValidationError::ContextDigest);
        }
        if self.canonical_digest()? != self.context_digest {
            return Err(super::SeededRunValidationError::ContextDigestMismatch);
        }
        Ok(())
    }

    /// Returns the digest of this context's canonical, digest-independent fields.
    pub fn canonical_digest(&self) -> Result<String, super::SeededRunValidationError> {
        let input = SeededRunContextDigestInput {
            context_id: &self.context_id,
            game_mode: self.game_mode,
            character: self.character,
            ascension: self.ascension,
            modifiers: &self.modifiers,
            acts: &self.acts,
            selection_policy: &self.selection_policy,
            profile_baseline: &self.profile_baseline,
            save_policy: self.save_policy,
            compatibility: &self.compatibility,
        };
        let bytes = serde_json::to_vec(&input)
            .map_err(|_| super::SeededRunValidationError::ContextSerialization)?;
        Ok(sha256_hex(&bytes))
    }
}

impl SeededRunProfileBaseline {
    fn validate(&self) -> Result<(), super::SeededRunValidationError> {
        validate_context_text(
            &self.identity,
            SEEDED_RUN_MAX_CONTEXT_ID_BYTES,
            super::SeededRunValidationError::InvalidContext,
        )?;
        if !is_context_digest(&self.digest) {
            return Err(super::SeededRunValidationError::ContextDigest);
        }
        Ok(())
    }
}

fn validate_context_text(
    value: &str,
    maximum: usize,
    error: super::SeededRunValidationError,
) -> Result<(), super::SeededRunValidationError> {
    if value.is_empty()
        || value.len() > maximum
        || value.chars().any(char::is_control)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
    {
        return Err(error);
    }
    Ok(())
}

fn is_context_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}
